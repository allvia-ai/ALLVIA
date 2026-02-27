use std::path::Path;

use crate::shell_analysis;

#[derive(Debug, Clone)]
pub struct ShellAction {
    pub instruction: String,
    pub targets: Vec<String>,
    pub verify: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct VerifyVerdict {
    pub ok: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub success: bool,
    pub verdicts: Vec<VerifyVerdict>,
}

const FILES_TXT_VERIFICATION: &[&str] = &[
    "files_exist:files.txt",
    "files_not_empty:files.txt",
    "files_no_hidden:files.txt",
    "files_match_listing:files.txt",
];

const SAFE_FILES_TXT_COMMAND: &str =
    "find . -maxdepth 1 -mindepth 1 -not -name 'files.txt' -not -name '.*' | sed 's|^./||' | sort > files.txt";

pub fn sanitize_shell_action(mut action: ShellAction, workdir: &str) -> ShellAction {
    let mut instr = normalize_ls_command(&action.instruction);
    instr = augment_files_txt(instr);

    if mentions_files_txt(&instr) {
        action.verify = merge_verify(&action.verify, FILES_TXT_VERIFICATION);
    } else {
        let inferred = infer_verify_steps(&instr);
        action.verify = merge_verify(
            &action.verify,
            &inferred.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
    }

    if needs_missing_file_guard(&instr) {
        let missing = action
            .targets
            .iter()
            .filter(|t| !exists_in_workdir(workdir, t))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            instr = format!(
                "IMPORTANT: The following file(s) do NOT exist and must be CREATED first: {}. Original instruction: {}",
                missing.join(", "),
                instr
            );
        }
    }

    action.instruction = instr;
    action
}

pub fn verify_shell_action(action: &ShellAction, result: &str, workdir: &str) -> VerifyResult {
    let mut verdicts = Vec::new();
    let result_lower = result.to_lowercase();

    if result_lower.contains("error") || result_lower.contains("exception") {
        verdicts.push(VerifyVerdict {
            ok: false,
            reason: "result contains error".to_string(),
        });
        return VerifyResult {
            success: false,
            verdicts,
        };
    }

    for item in &action.verify {
        if item == "tests_pass" {
            let ok = !result_lower.contains("fail") && !result_lower.contains("error");
            verdicts.push(VerifyVerdict {
                ok,
                reason: "tests_pass".to_string(),
            });
        } else if item == "lint_pass" {
            let ok = result_lower.contains("lint")
                && (result_lower.contains("pass") || result_lower.contains("no issues"));
            verdicts.push(VerifyVerdict {
                ok,
                reason: "lint_pass".to_string(),
            });
        } else if item == "build_success" {
            let ok = !result_lower.contains("build failed") && !result_lower.contains("error");
            verdicts.push(VerifyVerdict {
                ok,
                reason: "build_success".to_string(),
            });
        } else if item.starts_with("files_exist:") {
            let path = item.split_once(':').map(|x| x.1).unwrap_or("");
            verdicts.push(VerifyVerdict {
                ok: files_exist(workdir, path),
                reason: format!("files_exist:{}", path),
            });
        } else if item.starts_with("files_not_empty:") {
            let path = item.split_once(':').map(|x| x.1).unwrap_or("");
            verdicts.push(VerifyVerdict {
                ok: files_not_empty(workdir, path),
                reason: format!("files_not_empty:{}", path),
            });
        } else if item.starts_with("files_no_hidden:") {
            let path = item.split_once(':').map(|x| x.1).unwrap_or("");
            verdicts.push(VerifyVerdict {
                ok: files_no_hidden(workdir, path),
                reason: format!("files_no_hidden:{}", path),
            });
        } else if item.starts_with("files_match_listing:") {
            let path = item.split_once(':').map(|x| x.1).unwrap_or("");
            verdicts.push(VerifyVerdict {
                ok: files_match_listing(workdir, path),
                reason: format!("files_match_listing:{}", path),
            });
        }
    }

    let success = verdicts.iter().all(|v| v.ok);
    VerifyResult { success, verdicts }
}

pub fn infer_verify_steps(instruction: &str) -> Vec<String> {
    let lower = instruction.to_lowercase();
    let mut verify = Vec::new();

    if lower.contains("pytest") || lower.contains("cargo test") || lower.contains("npm test") {
        verify.push("tests_pass".to_string());
    }
    if lower.contains("lint") || lower.contains("ruff") || lower.contains("eslint") {
        verify.push("lint_pass".to_string());
    }
    if lower.contains("cargo build") || lower.contains("npm run build") {
        verify.push("build_success".to_string());
    }

    verify
}

fn normalize_ls_command(instr: &str) -> String {
    let mut out = instr.to_string();
    out = regex::Regex::new(r"ls -[aA]*l[aA]*")
        .unwrap()
        .replace_all(&out, "ls -1")
        .to_string();
    out = regex::Regex::new(r"ls -[aA]*")
        .unwrap()
        .replace_all(&out, "ls -1")
        .to_string();
    out.replace("ls -A", "ls -1")
}

fn augment_files_txt(instr: String) -> String {
    if instr.contains("files.txt") {
        let lower = instr.to_lowercase();
        if lower.contains("list") || lower.contains("ls") || lower.contains("rg --files") {
            return SAFE_FILES_TXT_COMMAND.to_string();
        }
    }
    instr
}

fn mentions_files_txt(instr: &str) -> bool {
    instr.contains("files.txt")
}

fn merge_verify(existing: &[String], extra: &[&str]) -> Vec<String> {
    let mut merged = existing.to_vec();
    for item in extra {
        if !merged.iter().any(|v| v == item) {
            merged.push(item.to_string());
        }
    }
    merged
}

fn needs_missing_file_guard(instr: &str) -> bool {
    let lower = instr.to_lowercase();
    ["modify", "update", "edit", "add", "append", "change"]
        .iter()
        .any(|k| lower.contains(k))
}

fn exists_in_workdir(workdir: &str, target: &str) -> bool {
    let base = Path::new(workdir);
    let path = if Path::new(target).is_absolute() {
        Path::new(target).to_path_buf()
    } else {
        base.join(target)
    };
    path.exists()
}

fn files_exist(workdir: &str, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let full = resolve_path(workdir, path);
    full.exists()
}

fn files_not_empty(workdir: &str, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let full = resolve_path(workdir, path);
    full.exists() && full.metadata().map(|m| m.len() > 0).unwrap_or(false)
}

fn files_no_hidden(workdir: &str, path: &str) -> bool {
    let full = resolve_path(workdir, path);
    let content = match std::fs::read_to_string(full) {
        Ok(c) => c,
        Err(_) => return false,
    };
    !content
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn files_match_listing(workdir: &str, path: &str) -> bool {
    let full = resolve_path(workdir, path);
    let content = match std::fs::read_to_string(&full) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let mut listed: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && *l != path)
        .map(|s| s.to_string())
        .collect();
    listed.sort();

    let mut expected: Vec<String> = match std::fs::read_dir(workdir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| !name.starts_with('.') && name != path)
            .collect(),
        Err(_) => return false,
    };
    expected.sort();
    listed == expected
}

fn resolve_path(workdir: &str, path: &str) -> std::path::PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(workdir).join(path)
    }
}

#[allow(dead_code)]
pub fn should_block_shell(command: &str) -> Option<&'static str> {
    let analysis = shell_analysis::analyze_shell_command(command);
    if analysis.has_substitution {
        return Some("command substitution is blocked");
    }
    if analysis.has_composites {
        return Some("composite commands are blocked");
    }
    None
}
