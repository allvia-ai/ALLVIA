use regex::Regex;

fn bool_env_with_default(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn normalize_text(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn csv_has_exact(raw_csv: &str, value: &str) -> bool {
    let target = normalize_text(value);
    if target.is_empty() {
        return false;
    }
    raw_csv
        .split(',')
        .map(normalize_text)
        .filter(|item| !item.is_empty())
        .any(|item| item == target)
}

fn parse_usize_env_with_default(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn extract_emails(raw: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}") else {
        return Vec::new();
    };
    re.find_iter(raw)
        .map(|m| normalize_email(m.as_str()))
        .collect()
}

pub fn enforce_mail_send_policy(
    goal: Option<&str>,
    recipient_field: &str,
    subject: &str,
    body_len: Option<i64>,
    status: &str,
) -> Result<(), String> {
    let strict_mode = bool_env_with_default("STEER_OUTBOUND_MAIL_STRICT", true);
    if !strict_mode {
        return Ok(());
    }

    if bool_env_with_default("STEER_OUTBOUND_MAIL_REQUIRE_SENT_CONFIRMED", true)
        && !status.eq_ignore_ascii_case("sent_confirmed")
    {
        return Err(format!(
            "mail_send policy: non-confirmed send status ({})",
            status
        ));
    }

    if bool_env_with_default("STEER_OUTBOUND_MAIL_REQUIRE_BODY", true)
        && body_len.unwrap_or_default() <= 0
    {
        return Err("mail_send policy: body_len must be > 0".to_string());
    }

    if bool_env_with_default("STEER_OUTBOUND_MAIL_REQUIRE_SUBJECT", true)
        && subject.trim().is_empty()
    {
        return Err("mail_send policy: subject is empty".to_string());
    }

    let actual_recipients = extract_emails(recipient_field);
    if actual_recipients.is_empty() {
        return Err(format!(
            "mail_send policy: recipient is invalid ({})",
            recipient_field
        ));
    }

    if bool_env_with_default("STEER_OUTBOUND_MAIL_REQUIRE_SINGLE_RECIPIENT", true)
        && actual_recipients.len() != 1
    {
        return Err(format!(
            "mail_send policy: expected single recipient, got {}",
            actual_recipients.len()
        ));
    }

    if bool_env_with_default("STEER_OUTBOUND_MAIL_REQUIRE_GOAL_TARGET_MATCH", true) {
        let expected: Vec<String> = goal
            .map(crate::semantic_contract::extract_expected_recipients)
            .unwrap_or_default()
            .into_iter()
            .map(|v| normalize_email(&v))
            .collect();
        if !expected.is_empty() {
            let actual = &actual_recipients[0];
            if !expected.iter().any(|v| v == actual) {
                return Err(format!(
                    "mail_send policy: recipient mismatch (actual={}, expected={})",
                    actual,
                    expected.join(",")
                ));
            }
        }
    }

    Ok(())
}

pub fn enforce_telegram_send_policy(chat_id: &str, text: &str) -> Result<(), String> {
    let strict_mode = bool_env_with_default("STEER_OUTBOUND_TELEGRAM_STRICT", true);
    if !strict_mode {
        return Ok(());
    }

    let chat = chat_id.trim();
    if chat.is_empty() {
        return Err("telegram_send policy: chat_id is empty".to_string());
    }
    let chat_is_numeric = chat
        .chars()
        .enumerate()
        .all(|(idx, ch)| ch.is_ascii_digit() || (idx == 0 && ch == '-'));
    if !chat_is_numeric {
        return Err(format!(
            "telegram_send policy: chat_id must be numeric-like ({})",
            chat
        ));
    }

    let deny_targets = std::env::var("STEER_OUTBOUND_TELEGRAM_DENY_TARGET_IDS").unwrap_or_default();
    if csv_has_exact(&deny_targets, chat) {
        return Err(format!("telegram_send policy: denied target id ({})", chat));
    }
    let allow_targets =
        std::env::var("STEER_OUTBOUND_TELEGRAM_ALLOW_TARGET_IDS").unwrap_or_default();
    if !allow_targets.trim().is_empty() && !csv_has_exact(&allow_targets, chat) {
        return Err(format!(
            "telegram_send policy: target id not in allowlist ({})",
            chat
        ));
    }

    let body = text.trim();
    if bool_env_with_default("STEER_OUTBOUND_TELEGRAM_REQUIRE_TEXT", true) && body.is_empty() {
        return Err("telegram_send policy: text is empty".to_string());
    }

    let max_chars = parse_usize_env_with_default(
        "STEER_OUTBOUND_TELEGRAM_MAX_MESSAGE_CHARS",
        120_000,
        64,
        400_000,
    );
    if body.chars().count() > max_chars {
        return Err(format!(
            "telegram_send policy: message too long ({})",
            body.chars().count()
        ));
    }

    if bool_env_with_default("STEER_OUTBOUND_TELEGRAM_REQUIRE_REPORT_SHAPE", false)
        && !(body.contains("상태:") && body.contains("근거:"))
    {
        return Err(
            "telegram_send policy: report shape invalid (requires '상태:' and '근거:')".to_string(),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn blocks_invalid_recipient() {
        let out = enforce_mail_send_policy(
            Some("qed4950@gmail.com 으로 메일 보내"),
            "not-an-email",
            "subject",
            Some(10),
            "sent_confirmed",
        );
        assert!(out.is_err());
    }

    #[test]
    fn blocks_goal_target_mismatch() {
        let out = enforce_mail_send_policy(
            Some("qed4950@gmail.com 으로 메일 보내"),
            "other@example.com",
            "subject",
            Some(10),
            "sent_confirmed",
        );
        assert!(out.is_err());
    }

    #[test]
    fn accepts_matching_goal_target() {
        let out = enforce_mail_send_policy(
            Some("qed4950@gmail.com 으로 메일 보내"),
            "qed4950@gmail.com",
            "subject",
            Some(10),
            "sent_confirmed",
        );
        assert!(out.is_ok());
    }

    #[test]
    #[serial]
    fn blocks_empty_subject_by_default() {
        std::env::remove_var("STEER_OUTBOUND_MAIL_REQUIRE_SUBJECT");
        let out = enforce_mail_send_policy(
            Some("qed4950@gmail.com 으로 메일 보내"),
            "qed4950@gmail.com",
            "",
            Some(10),
            "sent_confirmed",
        );
        assert!(out.is_err());
    }

    #[test]
    fn telegram_blocks_invalid_chat_id() {
        let out = enforce_telegram_send_policy("chat-abc", "hello");
        assert!(out.is_err());
    }

    #[test]
    #[serial]
    fn telegram_respects_deny_target_list() {
        std::env::set_var("STEER_OUTBOUND_TELEGRAM_DENY_TARGET_IDS", "-1001,-1002");
        let out = enforce_telegram_send_policy("-1002", "hello");
        std::env::remove_var("STEER_OUTBOUND_TELEGRAM_DENY_TARGET_IDS");
        assert!(out.is_err());
    }

    #[test]
    #[serial]
    fn telegram_respects_allow_target_list() {
        std::env::set_var("STEER_OUTBOUND_TELEGRAM_ALLOW_TARGET_IDS", "-1001,-1002");
        let blocked = enforce_telegram_send_policy("-2000", "hello");
        let allowed = enforce_telegram_send_policy("-1001", "hello");
        std::env::remove_var("STEER_OUTBOUND_TELEGRAM_ALLOW_TARGET_IDS");
        assert!(blocked.is_err());
        assert!(allowed.is_ok());
    }

    #[test]
    #[serial]
    fn telegram_report_shape_check_can_be_enabled() {
        std::env::set_var("STEER_OUTBOUND_TELEGRAM_REQUIRE_REPORT_SHAPE", "1");
        let blocked = enforce_telegram_send_policy("-1001", "요약만 있음");
        let allowed = enforce_telegram_send_policy("-1001", "상태: ✅ 성공\n근거:\n- line");
        std::env::remove_var("STEER_OUTBOUND_TELEGRAM_REQUIRE_REPORT_SHAPE");
        assert!(blocked.is_err());
        assert!(allowed.is_ok());
    }
}
