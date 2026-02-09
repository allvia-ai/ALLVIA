use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationProposal {
    pub title: String,
    pub summary: String,
    pub trigger: String,
    pub actions: Vec<String>,
    pub confidence: f64,
    pub n8n_prompt: String,
    // [Explainability]
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub pattern_id: Option<String>,
}

impl Default for AutomationProposal {
    fn default() -> Self {
        Self {
            title: "No recommendation".to_string(),
            summary: "No details.".to_string(),
            trigger: "none".to_string(),
            actions: vec![],
            confidence: 0.0,
            n8n_prompt: "".to_string(),
            evidence: vec![],
            pattern_id: None,
        }
    }
}

impl AutomationProposal {
    pub fn fingerprint(&self) -> String {
        format!(
            "{}::{}",
            self.title.trim().to_lowercase(),
            self.trigger.trim().to_lowercase()
        )
    }
}

// --- Template Engine ---

pub struct Template {
    #[allow(dead_code)]
    pub id: &'static str,
    pub title: &'static str,
    pub trigger_type: crate::pattern_detector::PatternType, // Filter by pattern type
    pub required_keywords: Vec<&'static str>,               // Apps or Keywords
    pub min_keyword_matches: usize,
    pub n8n_prompt: &'static str,
    pub base_confidence: f64,
}

pub struct TemplateMatcher {
    templates: Vec<Template>,
}

impl TemplateMatcher {
    pub fn new() -> Self {
        use crate::pattern_detector::PatternType::*;

        Self {
            templates: vec![
                // T1. Daily App Report
                Template {
                    id: "T1",
                    title: "Daily App Usage Report",
                    trigger_type: TimeBasedAction,
                    required_keywords: vec!["daily", "usage"], 
                    min_keyword_matches: 2,
                    n8n_prompt: "Create a workflow that sends a daily app usage report to Telegram at 11 PM.",
                    base_confidence: 0.9,
                },
                // T2. Downloads Cleanup
                Template {
                    id: "T2",
                    title: "Downloads Folder Cleanup",
                    trigger_type: FilePattern,
                    required_keywords: vec!["zip", "dmg", "pdf", "screenshot"], 
                    min_keyword_matches: 1,
                    n8n_prompt: "Watch Downloads folder for new files. If file is older than 7 days, move it to Archive folder.",
                    base_confidence: 0.85,
                },
                // T3. Work Start Routine
                Template {
                    id: "T3",
                    title: "Work Start Checklist",
                    trigger_type: AppSequence,
                    required_keywords: vec!["Slack", "Chrome", "Notion"], 
                    min_keyword_matches: 2,
                    n8n_prompt: "When I open Slack and Chrome in the morning, send me my daily checklist.",
                    base_confidence: 0.8,
                },
                // T4. Meeting Prep
                Template {
                    id: "T4",
                    title: "Meeting Prep Assistant",
                    trigger_type: AppSequence, // Or TimeBased
                    required_keywords: vec!["Zoom", "Notion", "Calendar"],
                    min_keyword_matches: 2,
                    n8n_prompt: "When a Calendar event starts, create a meeting note in Notion.",
                    base_confidence: 0.85,
                },
                // T5. Email Follow-Up
                Template {
                    id: "T5",
                    title: "Email Follow-Up Reminder",
                    trigger_type: KeywordRepeat,
                    required_keywords: vec!["invoice", "follow-up", "urgent"],
                    min_keyword_matches: 1,
                    n8n_prompt: "If I send an email with 'invoice', set a reminder to check for payment in 3 days.",
                    base_confidence: 0.9,
                },
                // T6. Daily Agenda
                Template {
                    id: "T6",
                    title: "Daily Agenda Notification",
                    trigger_type: AppSequence,
                    required_keywords: vec!["Calendar", "Todoist", "Reminders"],
                    min_keyword_matches: 2,
                    n8n_prompt: "Every morning at 8 AM, fetch my calendar events and send them to me via Telegram.",
                    base_confidence: 0.9,
                },
                // T7. Focus Block
                Template {
                    id: "T7",
                    title: "Focus Mode Activator",
                    trigger_type: AppSequence,
                    required_keywords: vec!["VSCode", "Xcode", "Terminal"], // Coding apps
                    min_keyword_matches: 2,
                    n8n_prompt: "If I use VSCode for more than 30 minutes, set Slack status to 'Focusing'.",
                    base_confidence: 0.85,
                },
                // T8. Slack Summary
                Template {
                    id: "T8",
                    title: "Slack Digest",
                    trigger_type: AppSequence,
                    required_keywords: vec!["Slack", "Gmail"],
                    min_keyword_matches: 2,
                    n8n_prompt: "Summarize unread Slack messages every 4 hours and email me the digest.",
                    base_confidence: 0.75,
                },
                // T9. File Backup
                Template {
                    id: "T9",
                    title: "Document Auto-Backup",
                    trigger_type: FilePattern,
                    required_keywords: vec!["docx", "pptx", "xlsx"],
                    min_keyword_matches: 1,
                    n8n_prompt: "When a document is saved, upload it to Google Drive / Backup folder.",
                    base_confidence: 0.9,
                },
                // T10. Weekly Report
                Template {
                    id: "T10",
                    title: "Weekly Productivity Report",
                    trigger_type: TimeBasedAction,
                    required_keywords: vec!["week", "report"],
                    min_keyword_matches: 2,
                    n8n_prompt: "Every Friday at 5 PM, generate a weekly productivity report based on my screen time.",
                    base_confidence: 0.8,
                },
            ]
        }
    }

    pub fn match_pattern(
        &self,
        pattern: &crate::pattern_detector::DetectedPattern,
    ) -> Option<AutomationProposal> {
        // 1. Filter by type
        let candidates: Vec<&Template> = self
            .templates
            .iter()
            .filter(|t| t.trigger_type == pattern.pattern_type)
            .collect();

        // 2. Score match
        let tokens = extract_tokens_from_pattern(pattern);
        for tmpl in candidates {
            let match_count = tmpl
                .required_keywords
                .iter()
                .filter(|k| tokens.contains(&k.to_lowercase()))
                .count();

            if match_count >= tmpl.min_keyword_matches {
                // Found a match!
                // Boost confidence based on pattern strength + keyword matches
                let match_bonus = (0.1 * match_count as f64).min(0.3);
                let final_confidence =
                    (tmpl.base_confidence * (0.7 + match_bonus)) + (pattern.similarity_score * 0.2);
                let matched_keywords: Vec<String> = tmpl
                    .required_keywords
                    .iter()
                    .filter(|k| tokens.contains(&k.to_lowercase()))
                    .map(|k| k.to_string())
                    .collect();

                // Construct Evidence (The Trust UX part)
                let evidence = vec![
                    format!("Pattern: {}", pattern.description),
                    format!("Frequency: Found {} occurrences", pattern.occurrences),
                    format!("Matched: {}", matched_keywords.join(", ")),
                ];

                return Some(AutomationProposal {
                    title: tmpl.title.to_string(),
                    summary: format!("Based on your activity: {}", pattern.description),
                    trigger: format!("Pattern Detected ({})", pattern.pattern_type.as_str()),
                    actions: vec!["n8n Workflow".to_string()],
                    confidence: final_confidence,
                    n8n_prompt: tmpl.n8n_prompt.to_string(),
                    evidence,
                    pattern_id: Some(pattern.pattern_id.clone()),
                });
            }
        }

        None
    }
}

fn extract_tokens_from_pattern(
    pattern: &crate::pattern_detector::DetectedPattern,
) -> HashSet<String> {
    let mut tokens = HashSet::new();

    // 1) Description tokens
    for part in pattern.description.split(|c: char| !c.is_alphanumeric()) {
        if part.len() >= 3 {
            tokens.insert(part.to_lowercase());
        }
    }

    // 2) Sample event tokens
    for ev in &pattern.sample_events {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(ev) {
            // app
            if let Some(app) = val
                .get("payload")
                .or_else(|| val.get("data"))
                .and_then(|p| p.get("app").and_then(|v| v.as_str()))
                .or_else(|| val.get("app").and_then(|v| v.as_str()))
            {
                tokens.insert(app.to_lowercase());
            }

            // keyword text
            if let Some(text) = val
                .get("payload")
                .or_else(|| val.get("data"))
                .and_then(|p| p.get("text").and_then(|v| v.as_str()))
            {
                for word in text.split_whitespace() {
                    if word.len() >= 3 {
                        tokens.insert(word.to_lowercase());
                    }
                }
            }

            // file extension
            if let Some(path) = val
                .get("payload")
                .or_else(|| val.get("data"))
                .and_then(|p| {
                    p.get("path")
                        .and_then(|v| v.as_str())
                        .or_else(|| p.as_str())
                })
            {
                if let Some(ext) = std::path::Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                {
                    tokens.insert(ext.to_lowercase());
                }
            }
        }
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern_detector::{DetectedPattern, PatternType};
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn test_token_extraction() {
        let pattern = DetectedPattern {
            pattern_id: "test".to_string(),
            pattern_type: PatternType::KeywordRepeat,
            description: "Repeated keyword: 'invoice'".to_string(),
            occurrences: 5,
            similarity_score: 1.0,
            sample_events: vec![
                json!({"type": "key_input", "data": {"text": "check invoice"}}).to_string(),
            ],
            detected_at: Utc::now(),
        };

        let tokens = extract_tokens_from_pattern(&pattern);
        assert!(tokens.contains("invoice"));
        assert!(tokens.contains("check"));
        assert!(tokens.contains("repeated")); // from description
    }

    #[test]
    fn test_template_matching_logic() {
        let matcher = TemplateMatcher::new();

        // Scenario 1: Exact Match for "Email Follow-Up" (Requires "invoice", "urgent", etc.)
        // T5 required: invoice, follow-up, urgent. min_match: 1? No, T5 in code has min_match=1?
        // Let's check T5 in the actual file. It has ["invoice", "follow-up", "urgent"].
        // This test assumes T5 Logic.

        let pattern = DetectedPattern {
            pattern_id: "p1".to_string(),
            pattern_type: PatternType::KeywordRepeat,
            description: "Repeated usage of 'invoice'".to_string(),
            occurrences: 10,
            similarity_score: 0.9,
            sample_events: vec![
                json!({"type": "ui.type", "data": {"text": "sending invoice"}}).to_string(),
            ],
            detected_at: Utc::now(),
        };

        let proposal = matcher.match_pattern(&pattern);
        assert!(proposal.is_some());
        let p = proposal.unwrap();
        assert_eq!(p.title, "Email Follow-Up Reminder");
        assert!(p.evidence.len() > 0);
    }
}
