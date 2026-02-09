use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetric {
    pub name: String,
    pub value: f64,
    pub threshold: f64,
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceVerificationResult {
    pub ok: bool,
    pub metrics: Vec<PerformanceMetric>,
    pub reason: String,
    pub template: String,
}

pub fn performance_baseline(workdir: &Path, max_files: usize) -> PerformanceVerificationResult {
    let mut metrics = Vec::new();

    let file_count = count_files(workdir, max_files);
    let max_files_threshold = env_u64("PERF_MAX_FILES", 300) as f64;
    metrics.push(metric("file_count", file_count as f64, max_files_threshold));

    let total_bytes = total_code_bytes(workdir, max_files);
    let max_bytes_threshold = env_u64("PERF_MAX_CODE_BYTES", 5_000_000) as f64;
    metrics.push(metric(
        "code_bytes",
        total_bytes as f64,
        max_bytes_threshold,
    ));

    let dep_count = dependency_count(workdir);
    let max_deps_threshold = env_u64("PERF_MAX_DEPS", 200) as f64;
    metrics.push(metric(
        "dependency_count",
        dep_count as f64,
        max_deps_threshold,
    ));

    let ok = metrics.iter().all(|m| m.ok);
    let reason = if ok {
        "All performance metrics within thresholds".to_string()
    } else {
        "Some performance metrics exceeded thresholds".to_string()
    };

    PerformanceVerificationResult {
        ok,
        metrics,
        reason,
        template: "performance_baseline".to_string(),
    }
}

fn metric(name: &str, value: f64, threshold: f64) -> PerformanceMetric {
    PerformanceMetric {
        name: name.to_string(),
        value,
        threshold,
        ok: value <= threshold,
    }
}

fn count_files(root: &Path, max_files: usize) -> usize {
    let mut count = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if count >= max_files {
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if count >= max_files {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name.starts_with('.') || is_ignored_dir(name) {
                    continue;
                }
                stack.push(path);
            } else {
                count += 1;
            }
        }
    }
    count
}

fn total_code_bytes(root: &Path, max_files: usize) -> u64 {
    let mut bytes = 0u64;
    let mut scanned = 0usize;
    let mut stack = vec![root.to_path_buf()];
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
            } else if is_code_file(&path) {
                if let Ok(meta) = fs::metadata(&path) {
                    bytes += meta.len();
                }
                scanned += 1;
            }
        }
    }
    bytes
}

fn dependency_count(root: &Path) -> usize {
    let mut count = 0usize;
    let package = root.join("package.json");
    if let Ok(content) = fs::read_to_string(&package) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) {
                count += deps.len();
            }
            if let Some(dev) = json.get("devDependencies").and_then(|v| v.as_object()) {
                count += dev.len();
            }
        }
    }

    let req = root.join("requirements.txt");
    if let Ok(content) = fs::read_to_string(&req) {
        count += content
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
            .count();
    }

    count
}

fn is_code_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "go" | "java" | "kt" | "swift" | "css" | "scss"
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

fn env_u64(key: &str, default_val: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default_val)
}
