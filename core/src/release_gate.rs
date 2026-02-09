use crate::{
    consistency_check, db, performance_verification, quality_scorer, semantic_verification,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseBaseline {
    pub created_at: String,
    pub consistency: Option<consistency_check::ConsistencyCheckResult>,
    pub semantic: Option<semantic_verification::SemanticVerificationResult>,
    pub performance: Option<performance_verification::PerformanceVerificationResult>,
    pub quality: Option<quality_scorer::QualityScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseGateResult {
    pub ok: bool,
    pub regressions: Vec<String>,
    pub warnings: Vec<String>,
    pub baseline: Option<ReleaseBaseline>,
    pub current: ReleaseBaseline,
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseGateRequest {
    pub workdir: Option<String>,
    pub max_files: Option<usize>,
    pub consistency: Option<consistency_check::ConsistencyCheckResult>,
    pub semantic: Option<semantic_verification::SemanticVerificationResult>,
    pub performance: Option<performance_verification::PerformanceVerificationResult>,
    pub quality: Option<quality_scorer::QualityScore>,
    pub perf_regression_pct: Option<f64>,
    pub quality_drop: Option<f64>,
}

pub type ReleaseBaselineRequest = ReleaseGateRequest;

pub fn build_baseline(req: ReleaseBaselineRequest) -> ReleaseBaseline {
    let workdir = resolve_workdir(req.workdir.as_deref());
    let semantic_max = req.max_files.unwrap_or(200);
    let performance_max = req.max_files.unwrap_or(300);

    let semantic = req.semantic.or_else(|| {
        Some(semantic_verification::semantic_consistency(
            &workdir,
            semantic_max,
        ))
    });
    let performance = req.performance.or_else(|| {
        Some(performance_verification::performance_baseline(
            &workdir,
            performance_max,
        ))
    });
    let consistency = req.consistency.or_else(|| {
        Some(consistency_check::run_consistency_check(
            consistency_check::ConsistencyCheckRequest {
                workdir: Some(workdir.to_string_lossy().to_string()),
            },
        ))
    });

    ReleaseBaseline {
        created_at: chrono::Utc::now().to_rfc3339(),
        consistency,
        semantic,
        performance,
        quality: req.quality,
    }
}

pub fn save_baseline(baseline: &ReleaseBaseline) {
    if let Ok(json) = serde_json::to_string(baseline) {
        let _ = db::upsert_release_baseline_json(&baseline.created_at, &json);
    }
}

pub fn run_release_gate(req: ReleaseGateRequest) -> ReleaseGateResult {
    let perf_override = req.perf_regression_pct;
    let quality_override = req.quality_drop;
    let current = build_baseline(req);
    let baseline = load_baseline();
    evaluate_release_gate(current, baseline, perf_override, quality_override)
}

fn load_baseline() -> Option<ReleaseBaseline> {
    db::get_release_baseline_json()
        .ok()
        .and_then(|record| record)
        .and_then(|record| serde_json::from_str::<ReleaseBaseline>(&record.baseline_json).ok())
}

fn evaluate_release_gate(
    current: ReleaseBaseline,
    baseline: Option<ReleaseBaseline>,
    perf_override: Option<f64>,
    quality_override: Option<f64>,
) -> ReleaseGateResult {
    let mut regressions = Vec::new();
    let mut warnings = Vec::new();

    let Some(base) = baseline.clone() else {
        warnings.push("No release baseline stored".to_string());
        return ReleaseGateResult {
            ok: true,
            regressions,
            warnings,
            baseline: None,
            current,
            template: "release_gate".to_string(),
        };
    };

    compare_semantic(&base, &current, &mut regressions, &mut warnings);
    compare_performance(
        &base,
        &current,
        &mut regressions,
        &mut warnings,
        perf_override,
    );
    compare_quality(
        &base,
        &current,
        &mut regressions,
        &mut warnings,
        quality_override,
    );
    compare_consistency(&base, &current, &mut regressions, &mut warnings);

    ReleaseGateResult {
        ok: regressions.is_empty(),
        regressions,
        warnings,
        baseline: Some(base),
        current,
        template: "release_gate".to_string(),
    }
}

fn compare_semantic(
    baseline: &ReleaseBaseline,
    current: &ReleaseBaseline,
    regressions: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Some(base_sem) = &baseline.semantic else {
        warnings.push("Baseline semantic check missing".to_string());
        return;
    };
    let Some(cur_sem) = &current.semantic else {
        warnings.push("Current semantic check missing".to_string());
        return;
    };

    if base_sem.ok && !cur_sem.ok {
        regressions.push("Semantic verification regressed from ok to failing".to_string());
    }
    if cur_sem.issues.len() > base_sem.issues.len() {
        regressions.push(format!(
            "Semantic issues increased ({} -> {})",
            base_sem.issues.len(),
            cur_sem.issues.len()
        ));
    }

    let base_high = count_severity(&base_sem.issues, "high");
    let cur_high = count_severity(&cur_sem.issues, "high");
    if cur_high > base_high {
        regressions.push(format!(
            "High severity semantic issues increased ({} -> {})",
            base_high, cur_high
        ));
    }
}

fn compare_performance(
    baseline: &ReleaseBaseline,
    current: &ReleaseBaseline,
    regressions: &mut Vec<String>,
    warnings: &mut Vec<String>,
    perf_override: Option<f64>,
) {
    let Some(base_perf) = &baseline.performance else {
        warnings.push("Baseline performance check missing".to_string());
        return;
    };
    let Some(cur_perf) = &current.performance else {
        warnings.push("Current performance check missing".to_string());
        return;
    };

    if base_perf.ok && !cur_perf.ok {
        regressions.push("Performance verification regressed from ok to failing".to_string());
    }

    let delta = perf_override.unwrap_or_else(|| env_f64("RELEASE_PERF_REGRESSION_PCT", 0.1));
    for base_metric in &base_perf.metrics {
        let Some(cur_metric) = cur_perf.metrics.iter().find(|m| m.name == base_metric.name) else {
            warnings.push(format!(
                "Current performance metric missing: {}",
                base_metric.name
            ));
            continue;
        };
        if base_metric.value > 0.0 {
            let allowed = base_metric.value * (1.0 + delta);
            if cur_metric.value > allowed {
                regressions.push(format!(
                    "Performance metric {} regressed ({} -> {})",
                    base_metric.name, base_metric.value, cur_metric.value
                ));
            }
        } else if cur_metric.value > 0.0 {
            regressions.push(format!(
                "Performance metric {} increased from baseline 0 to {}",
                base_metric.name, cur_metric.value
            ));
        }
    }
}

fn compare_quality(
    baseline: &ReleaseBaseline,
    current: &ReleaseBaseline,
    regressions: &mut Vec<String>,
    warnings: &mut Vec<String>,
    quality_override: Option<f64>,
) {
    let Some(base_quality) = &baseline.quality else {
        warnings.push("Baseline quality score missing".to_string());
        return;
    };
    let Some(cur_quality) = &current.quality else {
        warnings.push("Current quality score missing".to_string());
        return;
    };

    let drop = quality_override.unwrap_or_else(|| env_f64("RELEASE_QUALITY_DROP", 0.3));
    if cur_quality.overall + drop < base_quality.overall {
        regressions.push(format!(
            "Quality score dropped ({} -> {})",
            base_quality.overall, cur_quality.overall
        ));
    }
}

fn compare_consistency(
    baseline: &ReleaseBaseline,
    current: &ReleaseBaseline,
    regressions: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let Some(base_consistency) = &baseline.consistency else {
        warnings.push("Baseline consistency check missing".to_string());
        return;
    };
    let Some(cur_consistency) = &current.consistency else {
        warnings.push("Current consistency check missing".to_string());
        return;
    };

    if base_consistency.ok && !cur_consistency.ok {
        regressions.push("API consistency regressed from ok to failing".to_string());
    }
    if cur_consistency.issues.len() > base_consistency.issues.len() {
        regressions.push(format!(
            "API consistency issues increased ({} -> {})",
            base_consistency.issues.len(),
            cur_consistency.issues.len()
        ));
    }
}

fn count_severity(issues: &[semantic_verification::SemanticIssue], severity: &str) -> usize {
    let target = severity.to_lowercase();
    issues
        .iter()
        .filter(|issue| issue.severity.to_lowercase() == target)
        .count()
}

fn resolve_workdir(workdir: Option<&str>) -> PathBuf {
    workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

fn env_f64(key: &str, default_val: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default_val)
}

#[allow(dead_code)]
pub fn load_baseline_from_path(path: &Path) -> Option<ReleaseBaseline> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}
