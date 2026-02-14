use regex::Regex;
use std::collections::HashSet;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SemanticContract {
    pub tokens: Vec<String>,
    pub recipients: Vec<String>,
}

pub fn parse_contract(source: &str) -> SemanticContract {
    SemanticContract {
        tokens: extract_expected_tokens(source),
        recipients: extract_expected_recipients(source),
    }
}

pub fn extract_expected_tokens(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let text = source.trim();
    if text.is_empty() {
        return out;
    }

    let quoted_patterns = [
        r#""([^"]+)""#,
        r#"'([^']+)'"#,
        r#"“([^”]+)”"#,
        r#"‘([^’]+)’"#,
        r#"`([^`]+)`"#,
    ];
    for pat in quoted_patterns {
        if let Ok(re) = Regex::new(pat) {
            for cap in re.captures_iter(text) {
                if let Some(m) = cap.get(1) {
                    push_token(&mut out, &mut seen, m.as_str());
                }
            }
        }
    }

    if let Ok(re_semantic_list) =
        Regex::new(r#"(?i)(?:semantic[_ -]?tokens?|의미(?:검증)?(?:토큰)?)\s*[:=]\s*\[([^\]]+)\]"#)
    {
        for cap in re_semantic_list.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                for part in m.as_str().split([',', '|']) {
                    push_token(&mut out, &mut seen, part);
                }
            }
        }
    }

    if let Ok(re_key_value) = Regex::new(
        r#"([A-Za-z가-힣][A-Za-z가-힣0-9 _-]{1,24})\s*[:=]\s*([A-Za-z가-힣0-9._:@#/\- _]{3,96})"#,
    ) {
        for cap in re_key_value.captures_iter(text) {
            if let Some(v) = cap.get(2) {
                push_token(&mut out, &mut seen, v.as_str());
            }
            if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                let key = normalize_text(k.as_str());
                let value = normalize_text(v.as_str());
                push_token(&mut out, &mut seen, &format!("{key}: {value}"));
            }
        }
    }

    if let Ok(re_status) =
        Regex::new(r#"(?i)(status|상태)\s*(?:는|은|:|=)?\s*([A-Za-z0-9._-]{3,48})"#)
    {
        for cap in re_status.captures_iter(text) {
            if let (Some(k), Some(v)) = (cap.get(1), cap.get(2)) {
                let key = normalize_text(k.as_str()).to_lowercase();
                let value = normalize_text(v.as_str());
                push_token(&mut out, &mut seen, &format!("{key}: {value}"));
                push_token(&mut out, &mut seen, &value);
            }
        }
    }

    if let Ok(re_imperative) = Regex::new(
        r#"(?i)(?:입력|작성|기입|붙여넣기|기록|설정)\s*(?:은|는|을|를)?\s*([A-Za-z가-힣0-9._:@#/\- _]{3,96})"#,
    ) {
        for cap in re_imperative.captures_iter(text) {
            if let Some(v) = cap.get(1) {
                push_token(&mut out, &mut seen, v.as_str());
            }
        }
    }

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || (trimmed.chars().next().is_some_and(|c| c.is_ascii_digit())
                && (trimmed.contains(". ") || trimmed.contains(") ")))
        {
            let mut s = trimmed
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .to_string();
            if let Some(idx) = s.find(". ") {
                if s[..idx].chars().all(|c| c.is_ascii_digit()) {
                    s = s[idx + 2..].to_string();
                }
            } else if let Some(idx) = s.find(") ") {
                if s[..idx].chars().all(|c| c.is_ascii_digit()) {
                    s = s[idx + 2..].to_string();
                }
            }
            push_token(&mut out, &mut seen, &s);
        }
    }

    out
}

pub fn extract_expected_recipients(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let text = source.trim();
    if text.is_empty() {
        return out;
    }

    let Ok(re) = Regex::new(r"(?i)[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}") else {
        return out;
    };
    for m in re.find_iter(text) {
        let candidate = normalize_email_candidate(m.as_str());
        if candidate.is_empty() {
            continue;
        }
        if seen.insert(candidate.clone()) {
            out.push(candidate);
        }
    }
    out
}

fn normalize_email_candidate(raw: &str) -> String {
    let mut s = normalize_text(raw).to_lowercase();
    s = s
        .trim_matches(|c: char| "<>()[]{}\"'`“”‘’,;:.".contains(c))
        .to_string();
    for suffix in ["를", "을", "은", "는", "이", "가", "께", "에게"] {
        if s.ends_with(suffix) {
            s = s.trim_end_matches(suffix).to_string();
            break;
        }
    }
    s.trim().to_string()
}

fn normalize_text(raw: &str) -> String {
    raw.replace(['\r', '\n', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .trim_matches(|c: char| "\"'`“”‘’".contains(c))
        .to_string()
}

fn push_token(out: &mut Vec<String>, seen: &mut HashSet<String>, candidate: &str) {
    let token = normalize_text(candidate);
    if token.len() < 3 || token.len() > 120 {
        return;
    }
    if is_noise_token(&token) {
        return;
    }
    if seen.insert(token.clone()) {
        out.push(token);
    }
}

fn is_noise_token(token: &str) -> bool {
    let lower = token.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return true;
    }
    if ["cmd+", "command+", "shortcut"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    [
        "열고",
        "열어",
        "붙여넣",
        "복사",
        "입력하",
        "작성하",
        "보내기",
        "발송",
        "하세요",
        "해라",
        "실행해",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipients_strip_korean_particles() {
        let source = r#"받는 사람에 "qed4950@gmail.com"를 입력하고 보내"#;
        let recipients = extract_expected_recipients(source);
        assert_eq!(recipients, vec!["qed4950@gmail.com".to_string()]);
    }

    #[test]
    fn tokens_extract_status_and_quotes() {
        let source = r#"다음 줄에 "status: in-progress"를 입력하고 "Done"도 추가"#;
        let tokens = extract_expected_tokens(source);
        assert!(tokens
            .iter()
            .any(|t| t.eq_ignore_ascii_case("status: in-progress")));
        assert!(tokens.iter().any(|t| t == "in-progress"));
        assert!(tokens.iter().any(|t| t == "Done"));
    }
}
