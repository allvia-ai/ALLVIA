use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectScanResult {
    pub files: Vec<String>,
    pub key_files: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    React,
    Vite,
    Node,
    Python,
    Rust,
    Vanilla,
    Unknown,
}

impl ProjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectType::React => "react",
            ProjectType::Vite => "vite",
            ProjectType::Node => "node",
            ProjectType::Python => "python",
            ProjectType::Rust => "rust",
            ProjectType::Vanilla => "vanilla",
            ProjectType::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectScanner {
    root: PathBuf,
    max_files: usize,
    max_file_size: usize,
    ignored_dirs: HashSet<String>,
    key_files: HashSet<String>,
}

impl ProjectScanner {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let max_files = env::var("PROJECT_SCAN_MAX_FILES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(200);
        let max_file_size = env::var("PROJECT_SCAN_MAX_FILE_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(20_000);
        let ignored_dirs = parse_list(&env::var("PROJECT_SCAN_IGNORED_DIRS").unwrap_or_default());
        let key_files = parse_list(&env::var("KEY_FILE_NAMES").unwrap_or_default());

        let ignored_dirs = if ignored_dirs.is_empty() {
            [
                "node_modules",
                ".git",
                "target",
                "dist",
                "build",
                "__pycache__",
                ".venv",
                "venv",
                ".next",
                ".turbo",
                ".idea",
                ".vscode",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        } else {
            ignored_dirs.into_iter().collect()
        };

        let key_files = if key_files.is_empty() {
            [
                "package.json",
                "README.md",
                "requirements.txt",
                "pyproject.toml",
                "tsconfig.json",
                "vite.config.ts",
                "Cargo.toml",
                ".env.example",
                "Dockerfile",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect()
        } else {
            key_files.into_iter().collect()
        };

        Self {
            root,
            max_files,
            max_file_size,
            ignored_dirs,
            key_files,
        }
    }

    pub fn scan(&self, max_files_override: Option<usize>) -> ProjectScanResult {
        let max_files = max_files_override.unwrap_or(self.max_files);
        let mut files = Vec::new();
        let mut key_files = HashMap::new();

        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            if files.len() >= max_files {
                break;
            }
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                if files.len() >= max_files {
                    break;
                }
                let path = entry.path();
                let file_name = match path.file_name().and_then(|s| s.to_str()) {
                    Some(name) => name,
                    None => continue,
                };

                if path.is_dir() {
                    if file_name.starts_with('.') {
                        continue;
                    }
                    if self.ignored_dirs.contains(file_name) {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }

                if file_name.starts_with('.') && file_name != ".env.example" {
                    continue;
                }

                let rel_path = match path.strip_prefix(&self.root) {
                    Ok(p) => p.to_string_lossy().to_string(),
                    Err(_) => continue,
                };
                files.push(rel_path.clone());

                if self.key_files.contains(file_name) {
                    if let Some(content) = read_file_limited(&path, self.max_file_size) {
                        key_files.insert(rel_path, content);
                    }
                }
            }
        }

        ProjectScanResult { files, key_files }
    }

    pub fn get_project_type(&self) -> ProjectType {
        let package_json = self.root.join("package.json");
        if package_json.exists() {
            if let Ok(content) = fs::read_to_string(&package_json) {
                let lower = content.to_lowercase();
                if lower.contains("\"vite\"") {
                    return ProjectType::Vite;
                }
                if lower.contains("\"react\"") {
                    return ProjectType::React;
                }
                return ProjectType::Node;
            }
            return ProjectType::Node;
        }

        if self.root.join("Cargo.toml").exists() {
            return ProjectType::Rust;
        }

        if self.root.join("requirements.txt").exists() || self.root.join("pyproject.toml").exists()
        {
            return ProjectType::Python;
        }

        if self.root.join("index.html").exists() || self.root.join("public/index.html").exists() {
            return ProjectType::Vanilla;
        }

        ProjectType::Unknown
    }

    pub fn compute_state_hash(&self, max_files_override: Option<usize>) -> Option<String> {
        let scan = self.scan(max_files_override);
        let mut hasher = Sha256::new();
        for file in scan.files.iter() {
            hasher.update(file.as_bytes());
            hasher.update(b"|");
        }
        let mut keys: Vec<_> = scan.key_files.into_iter().collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, content) in keys {
            hasher.update(path.as_bytes());
            hasher.update(b":");
            hasher.update(content.as_bytes());
            hasher.update(b"|");
        }
        let digest = hasher.finalize();
        Some(format!("{:x}", digest))
    }
}

fn read_file_limited(path: &Path, max_size: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    if content.len() <= max_size {
        return Some(content);
    }
    let mut trimmed = content.chars().take(max_size).collect::<String>();
    trimmed.push_str("\n... (truncated)");
    Some(trimmed)
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
