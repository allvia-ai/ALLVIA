use crate::project_scanner::ProjectScanner;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration};

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeVerifyOptions {
    pub workdir: Option<String>,
    pub run_backend: Option<bool>,
    pub run_frontend: Option<bool>,
    pub run_e2e: Option<bool>,
    pub run_build_checks: Option<bool>,
    pub backend_port: Option<u16>,
    pub frontend_port: Option<u16>,
    pub backend_health_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeVerifyResult {
    pub backend_started: bool,
    pub backend_health: bool,
    pub backend_build_ok: Option<bool>,
    pub frontend_started: bool,
    pub frontend_health: bool,
    pub frontend_build_ok: Option<bool>,
    pub e2e_passed: Option<bool>,
    pub issues: Vec<String>,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone)]
enum BackendTarget {
    Python { dir: PathBuf, module: String },
    Rust { dir: PathBuf },
}

#[derive(Debug, Clone)]
struct FrontendTarget {
    dir: PathBuf,
}

pub async fn run_runtime_verification(options: RuntimeVerifyOptions) -> RuntimeVerifyResult {
    let workdir = options
        .workdir
        .as_ref()
        .map(|s| PathBuf::from(s))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let run_backend = options.run_backend.unwrap_or(true);
    let run_frontend = options.run_frontend.unwrap_or(true);
    let run_e2e = options.run_e2e.unwrap_or(false);
    let run_build_checks = options.run_build_checks.unwrap_or(false);
    let backend_port = options.backend_port.unwrap_or(5123);
    let frontend_port = options.frontend_port.unwrap_or(5173);
    let backend_health_path = options
        .backend_health_path
        .unwrap_or_else(|| "/health".to_string());

    let mut issues = Vec::new();
    let mut logs = Vec::new();

    let scanner = ProjectScanner::new(&workdir);
    let project_type = scanner.get_project_type();
    logs.push(format!("Project type: {}", project_type.as_str()));

    let backend_target = detect_backend_target(&workdir);
    let frontend_target = detect_frontend_target(&workdir);

    let mut backend_started = false;
    let mut backend_health = false;
    let mut backend_build_ok = None;
    let mut frontend_started = false;
    let mut frontend_health = false;
    let mut frontend_build_ok = None;
    let mut e2e_passed = None;

    if run_build_checks {
        if let Some(target) = backend_target.as_ref() {
            backend_build_ok = Some(run_backend_build_check(target, &mut logs).await);
            if backend_build_ok == Some(false) {
                issues.push("Backend build check failed".to_string());
            }
        }
        if let Some(target) = frontend_target.as_ref() {
            frontend_build_ok = Some(run_frontend_build_check(target, &mut logs).await);
            if frontend_build_ok == Some(false) {
                issues.push("Frontend build check failed".to_string());
            }
        }
    }

    if run_backend {
        if let Some(target) = backend_target.as_ref() {
            match start_backend(target, backend_port, &backend_health_path, &mut logs).await {
                Ok(health_ok) => {
                    backend_started = true;
                    backend_health = health_ok;
                    if !health_ok {
                        issues.push("Backend health check failed".to_string());
                    }
                }
                Err(e) => {
                    issues.push(format!("Backend failed to start: {}", e));
                }
            }
        } else {
            issues.push("No backend target detected".to_string());
        }
    }

    if run_frontend {
        if let Some(target) = frontend_target.as_ref() {
            match start_frontend(target, frontend_port, &mut logs).await {
                Ok(health_ok) => {
                    frontend_started = true;
                    frontend_health = health_ok;
                    if !health_ok {
                        issues.push("Frontend health check failed".to_string());
                    }
                }
                Err(e) => {
                    issues.push(format!("Frontend failed to start: {}", e));
                }
            }
        } else {
            issues.push("No frontend target detected".to_string());
        }
    }

    if run_e2e {
        if let Some(target) = frontend_target.as_ref() {
            let base_url = format!("http://127.0.0.1:{}", frontend_port);
            match run_e2e_tests(target, &base_url, &mut logs).await {
                Ok(passed) => {
                    e2e_passed = Some(passed);
                    if !passed {
                        issues.push("E2E tests failed".to_string());
                    }
                }
                Err(err) => {
                    e2e_passed = Some(false);
                    issues.push(format!("E2E tests error: {}", err));
                }
            }
        } else {
            issues.push("E2E requested but no frontend target detected".to_string());
        }
    }

    RuntimeVerifyResult {
        backend_started,
        backend_health,
        backend_build_ok,
        frontend_started,
        frontend_health,
        frontend_build_ok,
        e2e_passed,
        issues,
        logs,
    }
}

fn detect_backend_target(workdir: &Path) -> Option<BackendTarget> {
    let backend_dir = workdir.join("backend");
    let root_dir = workdir.to_path_buf();

    let (python_dir, module) = if backend_dir.join("main.py").exists() {
        (backend_dir.clone(), "main".to_string())
    } else if backend_dir.join("app").join("main.py").exists() {
        (backend_dir.clone(), "app.main".to_string())
    } else if root_dir.join("main.py").exists() {
        (root_dir.clone(), "main".to_string())
    } else if root_dir.join("app").join("main.py").exists() {
        (root_dir.clone(), "app.main".to_string())
    } else {
        (PathBuf::new(), String::new())
    };

    if !module.is_empty() {
        return Some(BackendTarget::Python {
            dir: python_dir,
            module,
        });
    }

    if root_dir.join("Cargo.toml").exists() {
        return Some(BackendTarget::Rust { dir: root_dir });
    }

    None
}

fn detect_frontend_target(workdir: &Path) -> Option<FrontendTarget> {
    let frontend_dir = workdir.join("frontend");
    if frontend_dir.join("package.json").exists() {
        return Some(FrontendTarget { dir: frontend_dir });
    }
    let web_dir = workdir.join("web");
    if web_dir.join("package.json").exists() {
        return Some(FrontendTarget { dir: web_dir });
    }
    if workdir.join("package.json").exists() {
        return Some(FrontendTarget {
            dir: workdir.to_path_buf(),
        });
    }
    None
}

async fn start_backend(
    target: &BackendTarget,
    port: u16,
    health_path: &str,
    logs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    match target {
        BackendTarget::Python { dir, module } => {
            if !command_exists("python") {
                return Err(anyhow::anyhow!("python not found in PATH"));
            }
            let mut child = Command::new("python")
                .arg("-m")
                .arg("uvicorn")
                .arg(format!("{}:app", module))
                .arg("--host")
                .arg("127.0.0.1")
                .arg("--port")
                .arg(port.to_string())
                .current_dir(dir)
                .spawn()?;

            let ok = wait_for_health(port, health_path, logs).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            Ok(ok)
        }
        BackendTarget::Rust { dir } => {
            if !command_exists("cargo") {
                return Err(anyhow::anyhow!("cargo not found in PATH"));
            }
            let output =
                run_command("cargo", &["check", "-q"], dir, Duration::from_secs(120)).await?;
            logs.push(output);
            Ok(true)
        }
    }
}

async fn start_frontend(
    target: &FrontendTarget,
    port: u16,
    logs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    if !command_exists("npm") {
        return Err(anyhow::anyhow!("npm not found in PATH"));
    }
    let mut child = Command::new("npm")
        .arg("run")
        .arg("dev")
        .current_dir(&target.dir)
        .env("PORT", port.to_string())
        .spawn()?;

    let ok = wait_for_frontend(port).await;
    if !ok {
        logs.push("Frontend health check failed".to_string());
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
    Ok(ok)
}

async fn wait_for_health(port: u16, health_path: &str, logs: &mut Vec<String>) -> bool {
    let client = Client::new();
    let base = format!("http://127.0.0.1:{}", port);
    let paths = [health_path, "/api/health", "/health", "/"];

    for _ in 0..10 {
        for path in paths.iter() {
            let url = format!("{}{}", base, path);
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() || resp.status().is_redirection() {
                    logs.push(format!("Backend health ok: {}", url));
                    return true;
                }
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn wait_for_frontend(port: u16) -> bool {
    let client = Client::new();
    let url = format!("http://127.0.0.1:{}", port);
    for _ in 0..10 {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() || resp.status().is_redirection() {
                return true;
            }
        }
        sleep(Duration::from_millis(600)).await;
    }
    false
}

async fn run_backend_build_check(target: &BackendTarget, logs: &mut Vec<String>) -> bool {
    match target {
        BackendTarget::Python { dir, module } => {
            if !command_exists("python") {
                logs.push("python not found for backend build check".to_string());
                return false;
            }
            let target_file = module_to_file(dir, module);
            if target_file.is_none() {
                logs.push("python main file not found for build check".to_string());
                return false;
            }
            let file_path = target_file.unwrap();
            let file_arg = file_path.to_string_lossy().to_string();
            let output = run_command(
                "python",
                &["-m", "py_compile", &file_arg],
                dir,
                Duration::from_secs(60),
            )
            .await;
            match output {
                Ok(out) => {
                    logs.push(out);
                    true
                }
                Err(err) => {
                    logs.push(err.to_string());
                    false
                }
            }
        }
        BackendTarget::Rust { dir } => {
            if !command_exists("cargo") {
                logs.push("cargo not found for backend build check".to_string());
                return false;
            }
            let output =
                run_command("cargo", &["check", "-q"], dir, Duration::from_secs(120)).await;
            match output {
                Ok(out) => {
                    logs.push(out);
                    true
                }
                Err(err) => {
                    logs.push(err.to_string());
                    false
                }
            }
        }
    }
}

async fn run_frontend_build_check(target: &FrontendTarget, logs: &mut Vec<String>) -> bool {
    if !command_exists("npm") {
        logs.push("npm not found for frontend build check".to_string());
        return false;
    }
    let output = run_command(
        "npm",
        &["run", "build"],
        &target.dir,
        Duration::from_secs(180),
    )
    .await;
    match output {
        Ok(out) => {
            logs.push(out);
            true
        }
        Err(err) => {
            logs.push(err.to_string());
            false
        }
    }
}

async fn run_e2e_tests(
    target: &FrontendTarget,
    base_url: &str,
    logs: &mut Vec<String>,
) -> anyhow::Result<bool> {
    if !command_exists("npx") {
        return Err(anyhow::anyhow!("npx not found in PATH"));
    }
    let spec = target
        .dir
        .join("e2e")
        .join("generated_verification.spec.ts");
    if !spec.exists() {
        return Err(anyhow::anyhow!("E2E spec not found at {}", spec.display()));
    }
    let mut cmd = Command::new("npx");
    cmd.arg("playwright")
        .arg("test")
        .arg(spec.to_string_lossy().to_string())
        .current_dir(&target.dir)
        .env("PLAYWRIGHT_TEST_BASE_URL", base_url)
        .env("CI", "true");

    let output = run_command_with_cmd(cmd, Duration::from_secs(180)).await?;
    logs.push(output.clone());
    Ok(true)
}

async fn run_command(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout_duration: Duration,
) -> anyhow::Result<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd);
    run_command_with_cmd(cmd, timeout_duration).await
}

async fn run_command_with_cmd(
    mut cmd: Command,
    timeout_duration: Duration,
) -> anyhow::Result<String> {
    let output = timeout(timeout_duration, cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("Command timed out"))??;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(anyhow::anyhow!(stderr))
    }
}

fn command_exists(cmd: &str) -> bool {
    let path = match env::var_os("PATH") {
        Some(path) => path,
        None => return false,
    };
    env::split_paths(&path).any(|dir| {
        let full = dir.join(cmd);
        if cfg!(windows) {
            full.with_extension("exe").exists() || full.exists()
        } else {
            full.exists()
        }
    })
}

fn module_to_file(dir: &Path, module: &str) -> Option<PathBuf> {
    if module == "main" {
        let candidate = dir.join("main.py");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if module == "app.main" {
        let candidate = dir.join("app").join("main.py");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
