use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendEndpoint {
    pub path: String,
    pub method: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendCall {
    pub path: String,
    pub method: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyIssue {
    pub path: String,
    pub reason: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyCheckResult {
    pub ok: bool,
    pub issues: Vec<ConsistencyIssue>,
    pub backend_paths: Vec<String>,
    pub frontend_calls: Vec<FrontendCall>,
    pub summary: String,
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyCheckRequest {
    pub workdir: Option<String>,
}

pub fn run_consistency_check(req: ConsistencyCheckRequest) -> ConsistencyCheckResult {
    let workdir = resolve_workdir(req.workdir.as_deref());
    let backend_file = workdir.join("core/src/api_server.rs");
    let backend = scan_backend_routes(&backend_file);
    let backend_paths = backend.iter().map(|b| b.path.clone()).collect::<Vec<_>>();

    let frontend_calls = scan_frontend_calls(&workdir);

    let backend_set: HashSet<String> = backend
        .iter()
        .map(|b| normalize_backend_path(&b.path))
        .filter(|p| !p.is_empty())
        .collect();

    let mut issues = Vec::new();
    for call in &frontend_calls {
        let normalized = call.path.clone();
        if !backend_set.iter().any(|b| paths_match(&normalized, b)) {
            issues.push(ConsistencyIssue {
                path: normalized,
                reason: "Frontend call has no matching backend route".to_string(),
                source: call.source.clone(),
            });
        }
    }

    let ok = issues.is_empty();
    let summary = format!(
        "Backend routes: {}, frontend calls: {}, mismatches: {}",
        backend_paths.len(),
        frontend_calls.len(),
        issues.len()
    );

    ConsistencyCheckResult {
        ok,
        issues,
        backend_paths,
        frontend_calls,
        summary,
        template: "consistency_check".to_string(),
    }
}

fn scan_backend_routes(path: &Path) -> Vec<BackendEndpoint> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let source = path.to_string_lossy().to_string();

    let route_re = Regex::new(r#"\.route\(\"([^\"]+)\"\s*,\s*([a-zA-Z_]+)"#).unwrap();
    let mut endpoints = Vec::new();

    for cap in route_re.captures_iter(&content) {
        let path = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }
        let method = cap.get(2).map(|m| m.as_str().to_uppercase());
        endpoints.push(BackendEndpoint {
            path,
            method,
            source: source.clone(),
        });
    }

    endpoints
}

fn scan_frontend_calls(workdir: &Path) -> Vec<FrontendCall> {
    let mut calls = Vec::new();
    let base_prefix = detect_api_base_prefix(workdir);

    let web_dir = workdir.join("web/src");
    if !web_dir.exists() {
        return calls;
    }

    let files = collect_files(&web_dir, &["ts", "tsx", "js", "jsx"]);

    let axios_re =
        Regex::new(r#"\b(?:api|axios)\.(get|post|put|patch|delete)\(\s*[\"'`]([^\"'`]+)[\"'`]"#)
            .unwrap();
    let fetch_re = Regex::new(r#"\bfetch\(\s*[\"'`]([^\"'`]+)[\"'`]"#).unwrap();

    for file in files {
        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(_) => continue,
        };

        for cap in axios_re.captures_iter(&content) {
            let method = cap.get(1).map(|m| m.as_str().to_uppercase());
            let raw = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            if let Some(path) = normalize_frontend_path(raw, base_prefix.as_deref()) {
                calls.push(FrontendCall {
                    path,
                    method,
                    source: file.to_string_lossy().to_string(),
                });
            }
        }

        for cap in fetch_re.captures_iter(&content) {
            let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if let Some(path) = normalize_frontend_path(raw, base_prefix.as_deref()) {
                calls.push(FrontendCall {
                    path,
                    method: None,
                    source: file.to_string_lossy().to_string(),
                });
            }
        }
    }

    calls
}

fn detect_api_base_prefix(workdir: &Path) -> Option<String> {
    let api_file = workdir.join("web/src/lib/api.ts");
    let content = fs::read_to_string(api_file).ok()?;
    let base_re = Regex::new(r#"API_BASE_URL\s*=\s*\"([^\"]+)\""#).unwrap();
    if let Some(cap) = base_re.captures(&content) {
        return extract_base_path(cap.get(1)?.as_str());
    }

    let fallback_re = Regex::new(r#"baseURL\s*:\s*\"([^\"]+)\""#).unwrap();
    if let Some(cap) = fallback_re.captures(&content) {
        return extract_base_path(cap.get(1)?.as_str());
    }

    None
}

fn collect_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if is_ignored_dir(&path) {
                    continue;
                }
                stack.push(path);
            } else if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                if extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                    files.push(path);
                }
            }
        }
    }
    files
}

fn is_ignored_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    matches!(
        name,
        "node_modules" | "dist" | "build" | ".next" | ".git" | "target"
    )
}

fn normalize_backend_path(path: &str) -> String {
    let mut normalized = path.trim().to_string();
    if normalized.is_empty() {
        return normalized;
    }
    if let Some(idx) = normalized.find('?') {
        normalized.truncate(idx);
    }
    normalized = Regex::new(r":([A-Za-z0-9_]+)")
        .unwrap()
        .replace_all(&normalized, ":param")
        .to_string();
    normalized = trim_trailing_slash(&normalized);
    if !normalized.starts_with('/') {
        normalized = format!("/{}", normalized);
    }
    normalized
}

fn normalize_frontend_path(raw: &str, base_prefix: Option<&str>) -> Option<String> {
    let mut path = raw.trim().to_string();
    if path.is_empty() {
        return None;
    }

    if let Some(extracted) = extract_url_path(&path) {
        path = extracted;
    }

    if let Some(idx) = path.find('?') {
        path.truncate(idx);
    }
    if let Some(idx) = path.find('#') {
        path.truncate(idx);
    }

    path = Regex::new(r"\$\{[^}]+\}")
        .unwrap()
        .replace_all(&path, ":param")
        .to_string();

    if let Some(prefix) = base_prefix {
        if path.starts_with('/') && !path.starts_with(prefix) {
            path = format!("{}{}", prefix.trim_end_matches('/'), path);
        }
    }

    path = trim_trailing_slash(&path);
    if !path.starts_with('/') {
        path = format!("/{}", path);
    }
    Some(path)
}

fn extract_url_path(url: &str) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let stripped = url.split("//").nth(1)?;
        let mut parts = stripped.splitn(2, '/');
        let _host = parts.next()?;
        let path = parts.next().unwrap_or("");
        return Some(format!("/{}", path));
    }
    if url.starts_with("//") {
        let stripped = &url[2..];
        let mut parts = stripped.splitn(2, '/');
        let _host = parts.next()?;
        let path = parts.next().unwrap_or("");
        return Some(format!("/{}", path));
    }
    None
}

fn extract_base_path(url: &str) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        let stripped = url.split("//").nth(1)?;
        let mut parts = stripped.splitn(2, '/');
        let _host = parts.next()?;
        let path = parts.next().unwrap_or("");
        let normalized = format!("/{}", path.trim_end_matches('/'));
        return Some(normalized);
    }
    if url.starts_with('/') {
        return Some(url.trim_end_matches('/').to_string());
    }
    None
}

fn trim_trailing_slash(path: &str) -> String {
    if path.len() > 1 {
        path.trim_end_matches('/').to_string()
    } else {
        path.to_string()
    }
}

fn paths_match(front: &str, backend: &str) -> bool {
    if front == backend {
        return true;
    }

    let front_segments = split_path(front);
    let back_segments = split_path(backend);
    if front_segments.len() != back_segments.len() {
        return false;
    }

    for (f, b) in front_segments.iter().zip(back_segments.iter()) {
        if b == ":param" {
            continue;
        }
        if f != b {
            return false;
        }
    }
    true
}

fn split_path(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn resolve_workdir(workdir: Option<&str>) -> PathBuf {
    workdir
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_match_param() {
        assert!(paths_match("/api/routines/123", "/api/routines/:param"));
        assert!(!paths_match(
            "/api/routines/123/abc",
            "/api/routines/:param"
        ));
    }

    #[test]
    fn test_normalize_frontend_with_base() {
        let path = normalize_frontend_path("/status", Some("/api")).unwrap();
        assert_eq!(path, "/api/status");
    }
}
