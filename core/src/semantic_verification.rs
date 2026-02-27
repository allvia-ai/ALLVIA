use crate::static_checks;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticIssue {
    pub file: String,
    pub reason: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticVerificationResult {
    pub ok: bool,
    pub issues: Vec<SemanticIssue>,
    pub reason: String,
    pub template: String,
}

pub fn semantic_consistency(workdir: &Path, max_files: usize) -> SemanticVerificationResult {
    let mut issues = Vec::new();
    let mut scanned = 0usize;
    let todo_strict = env_truthy("STEER_SEMANTIC_TODO_STRICT");
    let fail_on_medium = env_truthy("STEER_SEMANTIC_FAIL_ON_MEDIUM");

    let mut stack = vec![workdir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if scanned >= max_files {
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            if scanned >= max_files {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with('.') || is_ignored_dir(name) {
                    continue;
                }
                stack.push(path);
                continue;
            }

            if !is_code_file(&path) {
                continue;
            }
            scanned += 1;
            if let Ok(content) = fs::read_to_string(&path) {
                if todo_strict
                    && (content.contains("TODO")
                        || content.contains("FIXME")
                        || content.contains("XXX"))
                {
                    issues.push(SemanticIssue {
                        file: display_path(workdir, &path),
                        reason: "TODO/FIXME marker present".to_string(),
                        severity: "low".to_string(),
                    });
                }
                if content.contains("unimplemented!") || content.contains("panic!(\"TODO") {
                    issues.push(SemanticIssue {
                        file: display_path(workdir, &path),
                        reason: "Unimplemented code path found".to_string(),
                        severity: "high".to_string(),
                    });
                }
                if content.contains("raise NotImplementedError") {
                    issues.push(SemanticIssue {
                        file: display_path(workdir, &path),
                        reason: "NotImplementedError raised".to_string(),
                        severity: "high".to_string(),
                    });
                }
            }
        }
    }

    let ok = !issues.iter().any(|issue| {
        issue.severity.eq_ignore_ascii_case("high")
            || (fail_on_medium && issue.severity.eq_ignore_ascii_case("medium"))
    });
    let reason = if ok {
        "Semantic check passed".to_string()
    } else {
        format!("{} semantic issues found", issues.len())
    };
    let mut merged_ok = ok;
    let mut merged_issues = issues;
    let mut reason_parts = vec![reason];

    let static_result = static_checks::run_static_checks(workdir, max_files);
    if !static_result.ok {
        merged_ok = false;
        reason_parts.push(static_result.reason.clone());
        for issue in static_result.issues {
            merged_issues.push(SemanticIssue {
                file: issue.file,
                reason: issue.reason,
                severity: issue.severity,
            });
        }
    }

    SemanticVerificationResult {
        ok: merged_ok,
        issues: merged_issues,
        reason: reason_parts.join(" | "),
        template: "semantic_consistency".to_string(),
    }
}

fn is_code_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "go" | "java" | "kt" | "swift"
    )
}

fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | "target"
            | "dist"
            | "build"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".git"
            | ".next"
    )
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES") | Ok("on") | Ok("ON")
    )
}
