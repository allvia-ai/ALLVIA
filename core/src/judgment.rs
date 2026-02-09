use crate::db;
use crate::performance_verification::PerformanceVerificationResult;
use crate::project_scanner::ProjectScanner;
use crate::quality_scorer::QualityScore;
use crate::runtime_verification::RuntimeVerifyResult;
use crate::semantic_verification::SemanticVerificationResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct JudgmentRequest {
    pub workdir: Option<String>,
    pub runtime: Option<RuntimeVerifyResult>,
    pub quality: Option<QualityScore>,
    pub semantic: Option<SemanticVerificationResult>,
    pub performance: Option<PerformanceVerificationResult>,
    pub max_files: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JudgmentResponse {
    pub status: String,
    pub reasons: Vec<String>,
    pub no_progress: bool,
    pub project_hash: Option<String>,
    pub consecutive_no_progress: i64,
}

pub fn evaluate_judgment(req: JudgmentRequest) -> JudgmentResponse {
    let mut reasons = Vec::new();

    if let Some(runtime) = &req.runtime {
        if !runtime.backend_started || !runtime.frontend_started {
            reasons.push("runtime_failed".to_string());
        }
        if !runtime.backend_health || !runtime.frontend_health {
            reasons.push("health_check_failed".to_string());
        }
        if runtime.e2e_passed == Some(false) {
            reasons.push("e2e_failed".to_string());
        }
    }

    if let Some(semantic) = &req.semantic {
        if !semantic.ok {
            reasons.push("semantic_failed".to_string());
        }
    }

    if let Some(perf) = &req.performance {
        if !perf.ok {
            reasons.push("performance_failed".to_string());
        }
    }

    if let Some(quality) = &req.quality {
        if quality.recommendation == "replanning" {
            reasons.push("quality_replan".to_string());
        }
    }

    let failure = !reasons.is_empty();

    let (project_hash, no_progress, consecutive_no_progress) =
        detect_no_progress(req.workdir.as_deref(), req.max_files, failure);

    let status = if no_progress && consecutive_no_progress >= 2 {
        "stop"
    } else if failure {
        "replan"
    } else {
        "ok"
    };

    JudgmentResponse {
        status: status.to_string(),
        reasons,
        no_progress,
        project_hash,
        consecutive_no_progress,
    }
}

fn detect_no_progress(
    workdir: Option<&str>,
    max_files: Option<usize>,
    failure: bool,
) -> (Option<String>, bool, i64) {
    let workdir = workdir.unwrap_or(".");
    let scanner = ProjectScanner::new(workdir);
    let hash = scanner.compute_state_hash(max_files);

    let mut consecutive = 0;
    let last_hash = db::get_judgment_state()
        .ok()
        .flatten()
        .and_then(|s| s.last_hash);
    let no_progress = if failure {
        match (&hash, &last_hash) {
            (Some(current), Some(prev)) => current == prev,
            _ => false,
        }
    } else {
        false
    };

    if no_progress {
        consecutive = db::get_judgment_state()
            .ok()
            .flatten()
            .map(|s| s.consecutive_no_progress + 1)
            .unwrap_or(1);
    } else if failure {
        consecutive = 0;
    }

    let _ = db::upsert_judgment_state(hash.as_deref(), consecutive);
    (hash, no_progress, consecutive)
}
