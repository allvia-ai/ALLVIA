#[derive(Debug, Clone)]
pub struct ShellCommandAnalysis {
    pub segments: Vec<String>,
    pub has_composites: bool,
    #[allow(dead_code)]
    pub has_pipe: bool,
    #[allow(dead_code)]
    pub has_redirection: bool,
    pub has_substitution: bool,
}

pub fn analyze_shell_command(command: &str) -> ShellCommandAnalysis {
    let has_and = command.contains("&&");
    let has_or = command.contains("||");
    let has_semicolon = command.contains(';');
    let has_pipe = command.contains('|');
    let has_redirection = command.contains('>') || command.contains('<');
    let has_substitution = command.contains('`') || command.contains("$(");
    let has_composites = has_and || has_or || has_semicolon || has_pipe;

    let segments = split_segments(command);

    ShellCommandAnalysis {
        segments,
        has_composites,
        has_pipe,
        has_redirection,
        has_substitution,
    }
}

fn split_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut buffer = String::new();
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '&' && chars.peek() == Some(&'&') {
            chars.next();
            push_segment(&mut segments, &mut buffer);
            continue;
        }
        if ch == '|' {
            if chars.peek() == Some(&'|') {
                chars.next();
            }
            push_segment(&mut segments, &mut buffer);
            continue;
        }
        if ch == ';' {
            push_segment(&mut segments, &mut buffer);
            continue;
        }
        buffer.push(ch);
    }

    push_segment(&mut segments, &mut buffer);
    segments
}

fn push_segment(segments: &mut Vec<String>, buffer: &mut String) {
    let trimmed = buffer.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    buffer.clear();
}
