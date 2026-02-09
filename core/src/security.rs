pub enum SafetyLevel {
    Safe,
    Warning,
    Critical,
}

pub struct CommandClassifier;

impl CommandClassifier {
    pub fn classify(cmd: &str) -> SafetyLevel {
        let cmd = cmd.trim();
        // Normalize: Collapse multiple spaces to one used for pattern matching
        let normalized: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
        let check_target = if normalized.is_empty() {
            cmd
        } else {
            &normalized
        };

        // 1. Critical Commands (High Risk)
        // Fork bombs, filesystem wipe, root escalation
        if check_target.contains("sudo")
            || check_target.contains("rm -rf")
            || check_target.contains("dd if=")
            || check_target.contains("mkfs")
            || check_target.contains(":(){ :|:& };:")
        {
            return SafetyLevel::Critical;
        }

        // 2. Warning Commands (Medium Risk)
        // File deletion, modification, network requests
        if check_target.starts_with("rm")
            || check_target.starts_with("mv")
            || check_target.starts_with("curl")
            || check_target.starts_with("wget")
            || check_target.starts_with("chmod")
            || check_target.starts_with("chown")
            || check_target.contains(">")
        {
            // Redirection could overwrite files
            return SafetyLevel::Warning;
        }

        // 3. Safe Commands (Low Risk)
        // Read-only or harmless operations
        SafetyLevel::Safe
    }
}
