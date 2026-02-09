use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct Dependency {
    pub name: String,
    pub check_cmd: String,
    pub install_cmd: String,
    pub is_critical: bool,
    pub is_missing: bool, // Added field to indicate status explicitly in JSON
}

impl Dependency {
    pub fn new(name: &str, check_cmd: &str, install_cmd: &str, critical: bool) -> Self {
        Self {
            name: name.to_string(),
            check_cmd: check_cmd.to_string(),
            install_cmd: install_cmd.to_string(),
            is_critical: critical,
            is_missing: false,
        }
    }

    pub fn check(&mut self) -> bool {
        let parts: Vec<&str> = self.check_cmd.split_whitespace().collect();
        if parts.is_empty() {
            return false;
        }

        let success = Command::new(parts[0])
            .args(&parts[1..])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        self.is_missing = !success;
        success
    }
}

#[derive(Serialize)]
pub struct SystemHealth {
    pub missing_deps: Vec<Dependency>,
}

impl SystemHealth {
    pub fn check_all() -> Self {
        let deps = vec![
            Dependency::new("Homebrew", "which brew", "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"", true),
            Dependency::new("cliclick", "which cliclick", "brew install cliclick", false),
            Dependency::new("n8n", "which n8n", "npm install -g n8n", true),
        ];

        let mut missing = Vec::new();
        for mut dep in deps {
            if !dep.check() {
                missing.push(dep);
            }
        }

        Self {
            missing_deps: missing,
        }
    }

    pub fn print_report(&self) {
        if self.missing_deps.is_empty() {
            println!("✅ All system dependencies are satisfied.");
            return;
        }

        println!("⚠️  MISSING DEPENDENCIES DETECTED:");
        for dep in &self.missing_deps {
            println!("   - ❌ {} (Install: `{}`)", dep.name, dep.install_cmd);
        }
        println!("\nPlease install these tools for full functionality.\n");
    }
}
