use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let current = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("failed to resolve current executable path: {err}");
            return ExitCode::from(1);
        }
    };

    let Some(bin_dir) = current.parent() else {
        eprintln!("failed to resolve executable parent directory");
        return ExitCode::from(1);
    };

    let target = bin_dir.join("local_os_agent");
    let mut cmd = Command::new(&target);
    cmd.args(std::env::args().skip(1));

    let status = match cmd.status() {
        Ok(status) => status,
        Err(err) => {
            eprintln!(
                "failed to launch local_os_agent from {}: {err}",
                target.display()
            );
            return ExitCode::from(1);
        }
    };

    match status.code() {
        Some(code) if (0..=255).contains(&code) => ExitCode::from(code as u8),
        _ => ExitCode::from(1),
    }
}
