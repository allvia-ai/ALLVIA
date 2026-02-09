use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticCheckIssue {
    pub file: String,
    pub reason: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticCheckResult {
    pub ok: bool,
    pub issues: Vec<StaticCheckIssue>,
    pub reason: String,
    pub template: String,
}

pub fn run_static_checks(workdir: &Path, max_files: usize) -> StaticCheckResult {
    let mut issues = Vec::new();
    let mut scanned = 0usize;

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

            if !is_check_target(&path) {
                continue;
            }
            scanned += 1;

            if !path.exists() {
                issues.push(StaticCheckIssue {
                    file: display_path(workdir, &path),
                    reason: "File missing".to_string(),
                    severity: "high".to_string(),
                });
                continue;
            }

            if let Ok(meta) = fs::metadata(&path) {
                if meta.len() == 0 && !is_allowed_empty(&path) {
                    issues.push(StaticCheckIssue {
                        file: display_path(workdir, &path),
                        reason: "Empty file (not allowed)".to_string(),
                        severity: "high".to_string(),
                    });
                    continue;
                }
            }

            if has_hidden_chars(&path) {
                issues.push(StaticCheckIssue {
                    file: display_path(workdir, &path),
                    reason: "Hidden control characters detected".to_string(),
                    severity: "medium".to_string(),
                });
            }
        }
    }

    let ok = issues.is_empty();
    let reason = if ok {
        "Static checks passed".to_string()
    } else {
        format!("{} static check issues found", issues.len())
    };

    StaticCheckResult {
        ok,
        issues,
        reason,
        template: "static_checks".to_string(),
    }
}

fn is_check_target(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "rs" | "py"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
    )
}

fn is_allowed_empty(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    matches!(name, "__init__.py" | ".gitkeep" | ".keep" | ".empty")
}

fn has_hidden_chars(path: &Path) -> bool {
    let content = match fs::read(path) {
        Ok(content) => content,
        Err(_) => return false,
    };
    content
        .iter()
        .any(|b| matches!(b, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F))
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
            | ".cache"
    )
}

fn display_path(root: &Path, path: &PathBuf) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}
