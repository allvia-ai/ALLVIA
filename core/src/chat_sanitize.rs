#[derive(Debug, Clone)]
pub struct SanitizedChat {
    pub text: String,
    pub flags: Vec<String>,
}

pub fn sanitize_chat_input(input: &str) -> SanitizedChat {
    let mut flags = Vec::new();

    let mut text = input.replace('\0', "");
    if text.len() > 4000 {
        text.truncate(4000);
        flags.push("truncated".to_string());
    }

    let cleaned: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    if cleaned != text {
        flags.push("control_stripped".to_string());
    }

    let normalized = cleaned.replace("\r\n", "\n").replace('\r', "\n");
    let stripped = strip_envelope_and_message_id(&normalized);
    if stripped != normalized {
        flags.push("envelope_stripped".to_string());
    }

    let lower = stripped.to_lowercase();
    let suspicious = [
        "ignore previous",
        "system prompt",
        "developer message",
        "hidden instruction",
        "jailbreak",
    ];
    if suspicious.iter().any(|k| lower.contains(k)) {
        flags.push("prompt_injection".to_string());
    }

    SanitizedChat {
        text: stripped,
        flags,
    }
}

fn strip_envelope_and_message_id(text: &str) -> String {
    let mut trimmed = text.to_string();

    if let Some(stripped) = strip_envelope_prefix(&trimmed) {
        trimmed = stripped;
    }

    let lines: Vec<&str> = trimmed.lines().collect();
    let filtered: Vec<&str> = lines
        .into_iter()
        .filter(|line| !is_message_id_line(line))
        .collect();
    filtered.join("\n")
}

fn strip_envelope_prefix(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"^\[([^\]]+)\]\s*").ok()?;
    let caps = re.captures(text)?;
    let header = caps.get(1)?.as_str().trim();
    if !looks_like_envelope_header(header) {
        return None;
    }
    let prefix = caps.get(0)?.as_str();
    Some(text[prefix.len()..].to_string())
}

fn looks_like_envelope_header(header: &str) -> bool {
    let header_lower = header.to_lowercase();
    let channels = [
        "webchat",
        "whatsapp",
        "telegram",
        "signal",
        "slack",
        "discord",
        "google chat",
        "imessage",
        "teams",
        "matrix",
        "zalo",
        "zalo personal",
        "bluebubbles",
    ];

    if regex::Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}Z")
        .unwrap()
        .is_match(&header_lower)
    {
        return true;
    }
    if regex::Regex::new(r"\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}")
        .unwrap()
        .is_match(&header_lower)
    {
        return true;
    }
    channels.iter().any(|c| header_lower.starts_with(c))
}

fn is_message_id_line(line: &str) -> bool {
    let re = regex::Regex::new(r"^\s*\[message_id:\s*[^\]]+\]\s*$").unwrap();
    re.is_match(line)
}
