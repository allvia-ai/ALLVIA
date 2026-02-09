use crate::llm_gateway::LLMClient;
use crate::runtime_verification::RuntimeVerifyResult;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeReviewInput {
    pub goal_achieved: Option<bool>,
    pub api_compatible: Option<bool>,
    pub issues: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub overall: f64,
    pub breakdown: HashMap<String, f64>,
    pub issues: Vec<String>,
    pub strengths: Vec<String>,
    pub recommendation: String,
    pub summary: String,
}

pub fn score_quality(
    runtime: Option<&RuntimeVerifyResult>,
    code_review: Option<&CodeReviewInput>,
) -> QualityScore {
    let mut breakdown = HashMap::new();
    let mut issues = Vec::new();
    let mut strengths = Vec::new();

    // Functionality (3.0)
    let mut func_score = 3.0;
    if let Some(rt) = runtime {
        if !rt.backend_started {
            func_score -= 1.5;
            issues.push("Backend failed to start".to_string());
        } else if !rt.backend_health {
            func_score -= 0.5;
            issues.push("Backend health check failed".to_string());
        } else {
            strengths.push("Backend healthy".to_string());
        }

        if !rt.frontend_started {
            func_score -= 1.0;
            issues.push("Frontend failed to start".to_string());
        } else if !rt.frontend_health {
            func_score -= 0.5;
            issues.push("Frontend health check failed".to_string());
        } else {
            strengths.push("Frontend healthy".to_string());
        }
    } else {
        func_score = 1.5;
        issues.push("Runtime verification not provided".to_string());
    }
    func_score = clamp(func_score, 0.0, 3.0);
    breakdown.insert("functionality".to_string(), func_score);

    // UI/UX (3.0)
    let mut ui_score = 1.5;
    if let Some(rt) = runtime {
        if rt.frontend_health {
            ui_score += 1.0;
        }
        if rt.e2e_passed == Some(true) {
            ui_score += 0.5;
            strengths.push("E2E scenario passed".to_string());
        } else if rt.e2e_passed == Some(false) {
            issues.push("E2E scenario failed".to_string());
        } else {
            issues.push("E2E verification not run".to_string());
        }
    } else {
        issues.push("Visual/UX verification not run".to_string());
    }
    ui_score = clamp(ui_score, 0.0, 3.0);
    breakdown.insert("ui_ux".to_string(), ui_score);

    // Code Quality (2.0)
    let mut code_score = 1.0;
    if let Some(review) = code_review {
        code_score = 2.0;
        if review.goal_achieved == Some(false) {
            code_score -= 1.0;
            issues.push("Goal requirements not met".to_string());
        }
        if let Some(issue_list) = &review.issues {
            let penalty = (issue_list.len() as f64) * 0.2;
            code_score -= penalty.min(0.6);
            issues.extend(issue_list.iter().take(3).cloned());
        }
        if review.goal_achieved == Some(true) {
            strengths.push("Goal requirements met".to_string());
        }
    } else {
        issues.push("Code review not provided".to_string());
    }
    code_score = clamp(code_score, 0.0, 2.0);
    breakdown.insert("code_quality".to_string(), code_score);

    // API Compatibility (2.0)
    let mut api_score = 1.0;
    if let Some(review) = code_review {
        if review.api_compatible == Some(true) {
            api_score = 2.0;
            strengths.push("API compatibility confirmed".to_string());
        } else if review.api_compatible == Some(false) {
            api_score = 0.5;
            issues.push("API compatibility issues".to_string());
        }
    } else if let Some(rt) = runtime {
        api_score = 0.0;
        if rt.backend_health {
            api_score += 1.0;
        }
        if rt.frontend_health {
            api_score += 0.5;
        }
        if rt.e2e_passed == Some(true) {
            api_score += 0.5;
        }
    }
    api_score = clamp(api_score, 0.0, 2.0);
    breakdown.insert("api_compatibility".to_string(), api_score);

    let overall = (func_score + ui_score + code_score + api_score).max(0.0);
    let recommendation = if overall < 5.0 {
        "replanning"
    } else if overall < 8.0 {
        "fix"
    } else {
        "done"
    };

    let summary = if issues.is_empty() {
        "Quality checks passed".to_string()
    } else {
        format!(
            "Issues: {}",
            issues
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        )
    };

    QualityScore {
        overall: round2(overall),
        breakdown,
        issues,
        strengths,
        recommendation: recommendation.to_string(),
        summary,
    }
}

pub async fn score_quality_with_llm(
    llm: &dyn LLMClient,
    goal: Option<&str>,
    runtime: Option<&RuntimeVerifyResult>,
    code_review: Option<&CodeReviewInput>,
) -> Result<QualityScore> {
    let payload = serde_json::json!({
        "goal": goal.unwrap_or(""),
        "runtime": runtime,
        "code_review": code_review,
    });

    let system_prompt = r#"
You are a software quality reviewer. Evaluate overall quality on a 0-10 scale.
Return JSON with fields:
{
  "overall": number,
  "breakdown": {"functionality": number, "ui_ux": number, "code_quality": number, "api_compatibility": number},
  "issues": [string],
  "strengths": [string],
  "recommendation": "done" | "fix" | "replanning",
  "summary": string
}
Be concise. Use evidence from runtime/code review. Output JSON only.
"#;

    let response = llm.score_quality(&system_prompt, &payload).await?;
    let parsed: QualityScore = serde_json::from_str(&response)?;
    Ok(parsed)
}

fn clamp(val: f64, min: f64, max: f64) -> f64 {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

fn round2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}
