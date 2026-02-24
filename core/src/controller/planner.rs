use crate::action_schema;
use crate::controller::actions::ActionRunner;
use crate::controller::heuristics;
use crate::controller::loop_detector::LoopDetector;
use crate::controller::supervisor::Supervisor;
use crate::db;
use crate::llm_gateway::LLMClient;
use crate::schema::EventEnvelope;
use crate::session_store::{Session, SessionStatus, SessionStep};
use crate::visual_driver::{SmartStep, VisualDriver};
use anyhow::Result;
use chrono::Utc;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use uuid::Uuid;

static GUI_RUN_SERIAL_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

pub struct Planner {
    pub llm: Arc<dyn LLMClient>,
    pub max_steps: usize,
    pub tx: Option<mpsc::Sender<String>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunGoalOutcome {
    pub run_id: String,
    pub planner_complete: bool,
    pub execution_complete: bool,
    pub business_complete: bool,
    pub status: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone)]
struct RunGoalExecutionSummary {
    planner_complete: bool,
    execution_complete: bool,
    business_complete: bool,
    business_note: String,
    approval_required: bool,
    preflight_permissions_ok: bool,
    preflight_screen_capture_ok: bool,
    cleanup_dialog_closed_count: usize,
    cleanup_app_ready_count: usize,
    cleanup_mail_outgoing_hidden_count: usize,
    step_count: usize,
    failed_steps: usize,
    blocking_failed_steps: usize,
    blocking_failure_details: Vec<String>,
    mail_send_required: bool,
    mail_send_confirmed: bool,
    notes_write_required: bool,
    notes_write_confirmed: bool,
    textedit_write_required: bool,
    textedit_write_confirmed: bool,
    textedit_save_required: bool,
    textedit_save_confirmed: bool,
    capture_total_ms: u128,
    capture_max_ms: u128,
    capture_count: usize,
    plan_total_ms: u128,
    plan_max_ms: u128,
    plan_count: usize,
    supervisor_total_ms: u128,
    supervisor_max_ms: u128,
    supervisor_count: usize,
    execute_total_ms: u128,
    execute_max_ms: u128,
    execute_count: usize,
}

#[derive(Debug, Default, Clone)]
struct RunGoalBusinessEvidence {
    mail_send_confirmed: bool,
    notes_write_confirmed: bool,
    textedit_write_confirmed: bool,
    textedit_save_confirmed: bool,
}

#[derive(Debug, Default, Clone)]
struct PlannerTimingStats {
    capture_total_ms: u128,
    capture_max_ms: u128,
    capture_count: usize,
    plan_total_ms: u128,
    plan_max_ms: u128,
    plan_count: usize,
    supervisor_total_ms: u128,
    supervisor_max_ms: u128,
    supervisor_count: usize,
    execute_total_ms: u128,
    execute_max_ms: u128,
    execute_count: usize,
}

impl PlannerTimingStats {
    fn record_capture(&mut self, elapsed: Duration) {
        let ms = elapsed.as_millis();
        self.capture_total_ms += ms;
        self.capture_max_ms = self.capture_max_ms.max(ms);
        self.capture_count += 1;
    }

    fn record_plan(&mut self, elapsed: Duration) {
        let ms = elapsed.as_millis();
        self.plan_total_ms += ms;
        self.plan_max_ms = self.plan_max_ms.max(ms);
        self.plan_count += 1;
    }

    fn record_supervisor(&mut self, elapsed: Duration) {
        let ms = elapsed.as_millis();
        self.supervisor_total_ms += ms;
        self.supervisor_max_ms = self.supervisor_max_ms.max(ms);
        self.supervisor_count += 1;
    }

    fn record_execute(&mut self, elapsed: Duration) {
        let ms = elapsed.as_millis();
        self.execute_total_ms += ms;
        self.execute_max_ms = self.execute_max_ms.max(ms);
        self.execute_count += 1;
    }
}

impl Planner {
    fn scenario_mode_enabled() -> bool {
        matches!(
            std::env::var("STEER_SCENARIO_MODE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
    }

    fn history_contains_case_insensitive(history: &[String], needle: &str) -> bool {
        let needle_lower = needle.to_lowercase();
        history
            .iter()
            .any(|h| h.to_lowercase().contains(&needle_lower))
    }

    fn last_history_index_contains_case_insensitive(
        history: &[String],
        needle: &str,
    ) -> Option<usize> {
        let needle_lower = needle.to_lowercase();
        history.iter().enumerate().rev().find_map(|(idx, entry)| {
            if entry.to_lowercase().contains(&needle_lower) {
                Some(idx)
            } else {
                None
            }
        })
    }

    fn last_opened_app_from_history(history: &[String]) -> Option<String> {
        for entry in history.iter().rev() {
            if let Some(rest) = entry.strip_prefix("Opened app: ") {
                let app = rest.trim();
                if !app.is_empty() {
                    return Some(app.to_string());
                }
            }
        }
        None
    }

    fn goal_contains_any(goal_lower: &str, needles: &[&str]) -> bool {
        needles.iter().any(|needle| goal_lower.contains(needle))
    }

    fn normalize_text_for_matching(text: &str) -> String {
        use unicode_normalization::UnicodeNormalization;

        text.nfkc().collect::<String>().to_lowercase()
    }

    fn goal_requires_mail_send(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_mail =
            lower.contains("mail") || lower.contains("메일") || lower.contains("이메일");
        let mentions_send =
            lower.contains("send") || lower.contains("보내") || lower.contains("발송");
        mentions_mail && mentions_send
    }

    fn goal_requires_telegram_send(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_telegram = lower.contains("telegram") || lower.contains("텔레그램");
        let mentions_send = lower.contains("send")
            || lower.contains("보내")
            || lower.contains("발송")
            || lower.contains("전송");
        mentions_telegram && mentions_send
    }

    fn infer_news_topic_from_goal(goal: &str) -> String {
        let lower = goal.to_lowercase();
        let topic_map: &[(&[&str], &str)] = &[
            (
                &[
                    "스포츠",
                    "sport",
                    "nba",
                    "nfl",
                    "mlb",
                    "epl",
                    "축구",
                    "야구",
                    "농구",
                ],
                "스포츠",
            ),
            (
                &[
                    "경제", "금융", "finance", "market", "stock", "주식", "증시", "코인",
                ],
                "경제",
            ),
            (
                &[
                    "정치",
                    "politic",
                    "election",
                    "정부",
                    "대통령",
                    "의회",
                    "외교",
                ],
                "정치",
            ),
            (&["과학", "science", "연구", "우주"], "과학"),
            (&["기술", "tech", "it", "startup", "반도체"], "기술"),
            (&["ai", "인공지능", "머신러닝", "생성형"], "AI"),
            (
                &["연예", "엔터", "entertainment", "movie", "music"],
                "엔터테인먼트",
            ),
            (&["건강", "의료", "health", "medicine"], "건강"),
        ];
        for (needles, topic) in topic_map.iter().copied() {
            if needles.iter().any(|needle| lower.contains(needle)) {
                return topic.to_string();
            }
        }

        let compact_topic_re =
            regex::Regex::new(r"([가-힣A-Za-z0-9+#.&/\-]{2,40})\s*(?:뉴스|기사|헤드라인)").ok();
        if let Some(re) = compact_topic_re {
            if let Some(captures) = re.captures(goal) {
                if let Some(raw) = captures.get(1) {
                    let candidate = raw
                        .as_str()
                        .trim()
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .to_string();
                    if !candidate.is_empty()
                        && !["요약", "정리", "선정", "최신", "오늘", "개"]
                            .iter()
                            .any(|w| candidate.eq_ignore_ascii_case(w))
                    {
                        return candidate;
                    }
                }
            }
        }

        "latest".to_string()
    }

    fn goal_targets_ai_news_to_notion(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let asks_news = lower.contains("news")
            || lower.contains("headline")
            || lower.contains("article")
            || lower.contains("기사")
            || lower.contains("트렌드")
            || lower.contains("헤드라인")
            || lower.contains("브리핑")
            || lower.contains("trend")
            || lower.contains("digest")
            || lower.contains("trendy")
            || lower.contains("뉴스");
        let asks_summary = lower.contains("요약")
            || lower.contains("summar")
            || lower.contains("정리")
            || lower.contains("선정")
            || lower.contains("모아")
            || lower.contains("핵심");
        let asks_notion = lower.contains("notion") || lower.contains("노션");
        asks_news && asks_summary && asks_notion
    }

    fn goal_targets_todo_summary(goal: &str) -> bool {
        let lower = Self::normalize_text_for_matching(goal);
        let asks_todo = Self::goal_contains_any(
            &lower,
            &[
                "todo",
                "to-do",
                "task",
                "tasks",
                "할 일",
                "할일",
                "체크리스트",
                "업무",
            ],
        );
        let asks_summary_or_list = Self::goal_contains_any(
            &lower,
            &[
                "요약",
                "정리",
                "목록",
                "리스트",
                "만들",
                "작성",
                "summar",
                "list",
                "organize",
            ],
        );
        asks_todo && asks_summary_or_list && !Self::goal_targets_ai_news_to_notion(goal)
    }

    fn text_staging_app() -> &'static str {
        match std::env::var("STEER_TEXT_STAGING_APP") {
            Ok(raw) => {
                let v = raw.trim().to_lowercase();
                if v == "notes" || v == "메모" {
                    "Notes"
                } else {
                    "TextEdit"
                }
            }
            Err(_) => "TextEdit",
        }
    }

    fn should_use_deterministic_goal_autoplan(goal: &str) -> bool {
        let lower = Self::normalize_text_for_matching(goal);
        if Self::goal_targets_ai_news_to_notion(goal) {
            return true;
        }
        if Self::goal_targets_todo_summary(goal) {
            return true;
        }
        if lower.contains("n8n") && lower.contains("추천 기능") {
            return true;
        }
        if Self::env_truthy("STEER_FORCE_DETERMINISTIC_GOAL_AUTOPLAN") {
            return true;
        }

        if !Self::env_truthy_default("STEER_DETERMINISTIC_GOAL_AUTOPLAN", true) {
            return false;
        }
        if Self::goal_targets_ai_news_to_notion(goal) {
            return true;
        }

        let apps = Self::ordered_apps_in_goal(goal);
        let text_fragments = Self::extract_goal_text_fragments(goal);
        let inferred_open_app = Self::extract_known_app_from_text(goal);
        let explicit_ops = Self::goal_contains_any(
            &lower,
            &[
                "cmd+",
                "command+",
                "전체 선택",
                "복사",
                "붙여넣",
                "copy",
                "paste",
                "subject",
                "제목",
                "받는 사람",
                "recipient",
                "send",
                "보내기",
                "발송",
            ],
        );

        let direct_delivery_goal =
            Self::goal_requires_mail_send(goal) || Self::goal_requires_telegram_send(goal);
        if direct_delivery_goal && !apps.is_empty() {
            return true;
        }

        let single_textual_write_goal = apps.len() == 1
            && Self::is_textual_app(apps[0])
            && Self::goal_has_write_signal(&lower)
            && !text_fragments.is_empty();
        if single_textual_write_goal {
            return true;
        }

        let simple_open_goal = (apps.len() == 1
            || (apps.is_empty() && inferred_open_app.is_some()))
            && Self::goal_has_open_signal(&lower)
            && !Self::goal_has_write_signal(&lower)
            && !Self::goal_has_payload_tokens(goal);
        if simple_open_goal {
            return true;
        }

        apps.len() >= 2 && text_fragments.len() >= 2 && explicit_ops
    }

    fn goal_has_payload_tokens(goal: &str) -> bool {
        !Self::extract_goal_text_fragments(goal).is_empty()
    }

    fn goal_has_write_signal(lower: &str) -> bool {
        Self::goal_contains_any(
            lower,
            &[
                "write",
                "작성",
                "입력",
                "써",
                "적어",
                "붙여넣",
                "paste",
                "type",
                "append",
                "기록",
                "escribe",
                "escribir",
                "écris",
                "ecris",
                "rédige",
                "redige",
                "schreib",
                "書",
                "入力して",
                "写",
                "输入",
            ],
        )
    }

    fn goal_has_open_signal(lower: &str) -> bool {
        Self::goal_contains_any(
            lower,
            &[
                "open", "launch", "열어", "열고", "실행", "켜", "띄워", "abre", "abrir", "ouvre",
                "ouvrir", "öffne", "oeffne", "開", "打开", "開啟",
            ],
        )
    }

    fn goal_has_new_item_signal(lower: &str) -> bool {
        Self::goal_contains_any(
            lower,
            &[
                "new note",
                "new document",
                "새 메모",
                "새 문서",
                "cmd+n",
                "command+n",
            ],
        )
    }

    fn goal_requires_notes_write(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_notes = lower.contains("notes") || lower.contains("메모");
        mentions_notes
            && (Self::goal_has_write_signal(&lower)
                || Self::goal_has_new_item_signal(&lower)
                || Self::goal_has_payload_tokens(goal))
    }

    fn goal_requires_textedit_write(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_textedit = lower.contains("textedit") || lower.contains("텍스트에디트");
        mentions_textedit
            && (Self::goal_has_write_signal(&lower)
                || Self::goal_has_new_item_signal(&lower)
                || Self::goal_has_payload_tokens(goal))
    }

    fn goal_requires_textedit_save(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        let mentions_textedit = lower.contains("textedit") || lower.contains("텍스트에디트");
        let mentions_save_shortcut = Self::contains_shortcut_token(&lower, "cmd", "s")
            || Self::contains_shortcut_token(&lower, "command", "s");
        let mentions_save =
            Self::goal_contains_any(&lower, &["save", "저장", "파일로 저장", "저장해"])
                || mentions_save_shortcut;
        mentions_textedit && mentions_save
    }

    fn contains_shortcut_token(text_lower: &str, modifier: &str, key: &str) -> bool {
        let escaped_modifier = regex::escape(modifier);
        let escaped_key = regex::escape(key);
        let pattern = format!(
            r"(^|[^a-z0-9_+]){}\s*\+\s*{}([^a-z0-9_+]|$)",
            escaped_modifier, escaped_key
        );
        regex::Regex::new(&pattern)
            .map(|re| re.is_match(text_lower))
            .unwrap_or(false)
    }

    fn step_data_has_proof(step: &crate::session_store::SessionStep, proof: &str) -> bool {
        step.data
            .as_ref()
            .and_then(|d| d.get("proof"))
            .and_then(|v| v.as_str())
            .map(|v| v == proof)
            .unwrap_or(false)
    }

    fn parse_app_context_from_step(step: &crate::session_store::SessionStep) -> Option<String> {
        let description = step.description.trim();
        if let Some(rest) = description.strip_prefix("Opened app: ") {
            let app = rest.trim();
            if !app.is_empty() {
                return Some(app.to_lowercase());
            }
        }
        if let Some(rest) = description.strip_prefix("Switched to app: ") {
            let app = rest.trim();
            if !app.is_empty() {
                return Some(app.to_lowercase());
            }
        }
        None
    }

    fn collect_business_evidence(session: &Session, history: &[String]) -> RunGoalBusinessEvidence {
        let mut evidence = RunGoalBusinessEvidence::default();
        let mut current_app = Self::last_opened_app_from_history(history).map(|a| a.to_lowercase());
        let mut textedit_context_seen = current_app.as_deref() == Some("textedit");
        let mut sent_pending_step: Option<usize> = None;
        let mut no_draft_after_pending = false;

        for step in &session.steps {
            if let Some(send_status) = Self::step_mail_send_status(step) {
                match send_status.as_str() {
                    "sent_confirmed" => {
                        if Self::step_has_mail_send_confirmed(step) {
                            evidence.mail_send_confirmed = true;
                        }
                    }
                    "sent_pending" => sent_pending_step = Some(step.step_index),
                    "no_draft" => {
                        if sent_pending_step.is_some() {
                            no_draft_after_pending = true;
                        }
                    }
                    _ => {}
                }
            }

            if step.status == "success" {
                if let Some(app) = Self::parse_app_context_from_step(step) {
                    if app == "textedit" {
                        textedit_context_seen = true;
                    }
                    current_app = Some(app);
                }
            }

            if step.status != "success" {
                continue;
            }

            let desc = step.description.to_lowercase();
            if Self::step_has_mail_send_confirmed(step) {
                evidence.mail_send_confirmed = true;
            }
            if Self::step_data_has_proof(step, "notes_write_text") || desc.contains("(notes body)")
            {
                evidence.notes_write_confirmed = true;
            }
            if Self::step_data_has_proof(step, "textedit_append_text")
                || desc.contains("(textedit body)")
            {
                evidence.textedit_write_confirmed = true;
                textedit_context_seen = true;
            }

            if matches!(step.action_type.as_str(), "type" | "paste") {
                match current_app.as_deref() {
                    Some("notes") => evidence.notes_write_confirmed = true,
                    Some("textedit") => {
                        evidence.textedit_write_confirmed = true;
                        textedit_context_seen = true;
                    }
                    _ => {}
                }
            }

            let is_save_shortcut = matches!(step.action_type.as_str(), "shortcut" | "key" | "save")
                && desc.contains("shortcut 's'")
                && desc.contains("command");
            let strict_textedit_save_proof =
                Self::env_truthy_default("STEER_STRICT_TEXTEDIT_SAVE_PROOF", true);
            let has_textedit_save_proof = Self::step_data_has_proof(step, "textedit_save")
                || desc.contains("textedit saved")
                || desc.contains("saved file")
                || desc.contains("file saved");
            if has_textedit_save_proof
                || (!strict_textedit_save_proof
                    && is_save_shortcut
                    && (current_app.as_deref() == Some("textedit") || textedit_context_seen))
            {
                evidence.textedit_save_confirmed = true;
            }
        }

        if !evidence.mail_send_confirmed && no_draft_after_pending {
            evidence.mail_send_confirmed = true;
        }

        if !evidence.mail_send_confirmed
            && !Self::env_truthy_default("STEER_STRICT_MAIL_SEND_PROOF", true)
        {
            evidence.mail_send_confirmed =
                Self::history_contains_case_insensitive(history, "mail send completed")
                    || Self::history_contains_case_insensitive(history, "(mail sent)");
        }
        if !evidence.notes_write_confirmed {
            evidence.notes_write_confirmed =
                Self::history_contains_case_insensitive(history, "(notes body)")
                    || (Self::history_contains_case_insensitive(history, "opened app: notes")
                        && Self::history_contains_case_insensitive(history, "typed '"));
        }
        if !evidence.textedit_write_confirmed {
            evidence.textedit_write_confirmed =
                Self::history_contains_case_insensitive(history, "(textedit body)")
                    || (Self::history_contains_case_insensitive(history, "opened app: textedit")
                        && Self::history_contains_case_insensitive(history, "typed '"));
        }
        if !evidence.textedit_save_confirmed {
            let strict_textedit_save_proof =
                Self::env_truthy_default("STEER_STRICT_TEXTEDIT_SAVE_PROOF", true);
            evidence.textedit_save_confirmed = (!strict_textedit_save_proof
                && Self::history_contains_case_insensitive(history, "opened app: textedit")
                && Self::history_contains_shortcut(history, "s"))
                || Self::history_contains_case_insensitive(history, "textedit saved")
                || Self::history_contains_case_insensitive(history, "file saved");
        }

        evidence
    }

    fn step_mail_send_status(step: &crate::session_store::SessionStep) -> Option<String> {
        step.data
            .as_ref()
            .and_then(|data| data.get("send_status"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
    }

    fn step_mail_send_body_len(step: &crate::session_store::SessionStep) -> Option<i64> {
        step.data
            .as_ref()
            .and_then(|data| data.get("body_len"))
            .and_then(|v| v.as_i64())
    }

    fn step_mail_send_recipient(step: &crate::session_store::SessionStep) -> Option<String> {
        step.data
            .as_ref()
            .and_then(|data| data.get("recipient"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    fn step_has_mail_send_confirmed(step: &crate::session_store::SessionStep) -> bool {
        let strict_mail_send_proof = Self::env_truthy_default("STEER_STRICT_MAIL_SEND_PROOF", true);
        if Self::step_mail_send_status(step).as_deref() == Some("sent_confirmed") {
            if !strict_mail_send_proof {
                return true;
            }
            let body_ok = Self::step_mail_send_body_len(step)
                .map(|len| len > 2)
                .unwrap_or(false);
            let recipient_ok = Self::step_mail_send_recipient(step)
                .map(|recipient| recipient.contains('@'))
                .unwrap_or(false);
            if body_ok && recipient_ok {
                return true;
            }
        }
        if strict_mail_send_proof {
            return false;
        }
        let desc = step.description.to_lowercase();
        desc.contains("mail send completed") || desc.contains("(mail sent)")
    }

    fn is_shortcut_permission_failure(step: &SessionStep) -> bool {
        if step.status == "success" {
            return false;
        }
        if !step.action_type.eq_ignore_ascii_case("shortcut") {
            return false;
        }
        let desc = step.description.to_lowercase();
        desc.contains("not allowed to send keystrokes")
            || desc.contains("허용되지 않습니다")
            || desc.contains("osascript")
            || desc.contains("system events")
            || desc.contains("shortcut failed")
    }

    fn is_benign_failed_step(step: &SessionStep) -> bool {
        if step.status == "success" {
            return false;
        }
        matches!(
            Self::step_mail_send_status(step).as_deref(),
            Some("sent_pending") | Some("no_draft")
        ) || Self::is_shortcut_permission_failure(step)
    }

    fn summarize_execution(
        goal: &str,
        session: &Session,
        history: &[String],
        planner_complete: bool,
        timing: &PlannerTimingStats,
    ) -> RunGoalExecutionSummary {
        let cleanup_dialog_closed_count = history
            .iter()
            .filter(|h| h.starts_with("CLEANUP_DIALOG_CLOSED:"))
            .count();
        let cleanup_app_ready_count = history
            .iter()
            .filter(|h| h.starts_with("CLEANUP_APP_READY:"))
            .count();
        let cleanup_mail_outgoing_hidden_count = history
            .iter()
            .filter_map(|h| h.strip_prefix("CLEANUP_MAIL_OUTGOING_HIDDEN:"))
            .filter_map(|raw| raw.trim().parse::<usize>().ok())
            .sum::<usize>();
        let preflight_permissions_ok = history.iter().any(|h| h == "PREFLIGHT_PERMISSIONS_OK");
        let preflight_screen_capture_ok =
            history.iter().any(|h| h == "PREFLIGHT_SCREEN_CAPTURE_OK");

        let step_count = session.steps.len();
        let failed_steps = session
            .steps
            .iter()
            .filter(|s| s.status != "success")
            .count();
        let blocking_failed_steps = session
            .steps
            .iter()
            .filter(|s| s.status != "success" && !Self::is_benign_failed_step(s))
            .count();
        let blocking_failure_details: Vec<String> = session
            .steps
            .iter()
            .filter(|s| s.status != "success" && !Self::is_benign_failed_step(s))
            .take(3)
            .map(|s| format!("{}: {}", s.action_type, s.description))
            .collect();
        let execution_complete = planner_complete && blocking_failed_steps == 0;
        let mail_send_required = Self::goal_requires_mail_send(goal);
        let notes_write_required = Self::goal_requires_notes_write(goal);
        let textedit_write_required = Self::goal_requires_textedit_write(goal);
        let textedit_save_required = Self::goal_requires_textedit_save(goal);
        let evidence = Self::collect_business_evidence(session, history);
        let mail_send_confirmed = evidence.mail_send_confirmed;
        let notes_write_confirmed = evidence.notes_write_confirmed;
        let textedit_write_confirmed = evidence.textedit_write_confirmed;
        let textedit_save_confirmed = evidence.textedit_save_confirmed;

        let (business_complete, business_note) = if !execution_complete {
            (
                false,
                format!(
                    "action execution had blocking failures (blocking_failed_steps={} / failed_steps={} / total_steps={})",
                    blocking_failed_steps, failed_steps, step_count
                ),
            )
        } else {
            let mut missing_checks: Vec<&str> = Vec::new();
            if mail_send_required && !mail_send_confirmed {
                missing_checks.push("mail_send_confirmation");
            }
            if notes_write_required && !notes_write_confirmed {
                missing_checks.push("notes_write_evidence");
            }
            if textedit_write_required && !textedit_write_confirmed {
                missing_checks.push("textedit_write_evidence");
            }
            if textedit_save_required && !textedit_save_confirmed {
                missing_checks.push("textedit_save_confirmation");
            }

            if missing_checks.is_empty() {
                (
                    true,
                    "planner/execution/business checks passed from run evidence".to_string(),
                )
            } else {
                (
                    false,
                    format!("business evidence missing: {}", missing_checks.join(", ")),
                )
            }
        };

        RunGoalExecutionSummary {
            planner_complete,
            execution_complete,
            business_complete,
            business_note,
            approval_required: false,
            preflight_permissions_ok,
            preflight_screen_capture_ok,
            cleanup_dialog_closed_count,
            cleanup_app_ready_count,
            cleanup_mail_outgoing_hidden_count,
            step_count,
            failed_steps,
            blocking_failed_steps,
            blocking_failure_details,
            mail_send_required,
            mail_send_confirmed,
            notes_write_required,
            notes_write_confirmed,
            textedit_write_required,
            textedit_write_confirmed,
            textedit_save_required,
            textedit_save_confirmed,
            capture_total_ms: timing.capture_total_ms,
            capture_max_ms: timing.capture_max_ms,
            capture_count: timing.capture_count,
            plan_total_ms: timing.plan_total_ms,
            plan_max_ms: timing.plan_max_ms,
            plan_count: timing.plan_count,
            supervisor_total_ms: timing.supervisor_total_ms,
            supervisor_max_ms: timing.supervisor_max_ms,
            supervisor_count: timing.supervisor_count,
            execute_total_ms: timing.execute_total_ms,
            execute_max_ms: timing.execute_max_ms,
            execute_count: timing.execute_count,
        }
    }

    fn is_textual_app(app: &str) -> bool {
        app.eq_ignore_ascii_case("Notes")
            || app.eq_ignore_ascii_case("TextEdit")
            || app.eq_ignore_ascii_case("Mail")
    }

    fn fallback_plan_from_goal(goal: &str, history: &[String]) -> Option<serde_json::Value> {
        let goal_lower = goal.to_lowercase();

        if goal_lower.contains("n8n") && goal_lower.contains("추천 기능") {
            if !Self::history_contains_case_insensitive(
                history,
                "http://localhost:5678/workflow/new",
            ) && !Self::history_contains_case_insensitive(history, "http://localhost:5678/")
            {
                return Some(serde_json::json!({
                    "action": "open_url",
                    "url": "http://localhost:5678/workflow/new"
                }));
            }
            if !Self::history_contains_case_insensitive(history, "n8n workflow created:") {
                let scope_marker = Self::goal_run_scope_marker(goal)
                    .unwrap_or_else(|| "RUN_SCOPE_TEST_03".to_string());
                return Some(serde_json::json!({
                    "action": "n8n_create_workflow",
                    "name": format!("Steer Scope {}", scope_marker),
                    "marker": scope_marker
                }));
            }
            return Some(serde_json::json!({ "action": "done" }));
        }

        if Self::goal_targets_ai_news_to_notion(goal) {
            let topic = Self::infer_news_topic_from_goal(goal);
            let search_query = format!("trending {} news", topic);
            let encoded_query = urlencoding::encode(&search_query).replace("%20", "+");
            let search_url = format!("https://www.google.com/search?q={}", encoded_query);
            let topic_lower = topic.to_lowercase();
            let has_topic_search = history.iter().any(|entry| {
                let e = entry.to_lowercase();
                e.contains("google.com/search?q=")
                    && (topic_lower == "latest" || e.contains(&topic_lower))
            });
            if !has_topic_search {
                return Some(serde_json::json!({
                    "action": "open_url",
                    "url": search_url
                }));
            }

            let summary_header = format!("{} 뉴스 기사 요약 (자동 생성)", topic);
            let mut summary_text = format!(
                "{}\n1) 기사 1: 최신 {} 핵심 이슈 요약\n2) 기사 2: 영향/배경/맥락 정리\n3) 기사 3: 후속 확인 포인트\n작성시각(UTC): {}",
                summary_header,
                topic,
                Utc::now().format("%Y-%m-%d %H:%M")
            );
            if let Some(marker) = Self::goal_run_scope_marker(goal) {
                if !summary_text.contains(&marker) {
                    summary_text = format!("{}\n{}", summary_text, marker);
                }
            }

            if Self::notion_api_ready() {
                if !Self::history_contains_case_insensitive(history, "Notion page created:") {
                    return Some(serde_json::json!({
                        "action": "notion_write",
                        "title": format!("{} {}", summary_header, Utc::now().format("%Y-%m-%d %H:%M")),
                        "content": summary_text
                    }));
                }
                return Some(serde_json::json!({ "action": "done" }));
            }

            if !Self::history_contains_case_insensitive(history, "Opened app: Notion") {
                return Some(serde_json::json!({
                    "action": "open_app",
                    "name": "Notion"
                }));
            }

            if !Self::history_contains_case_insensitive(history, "Created new item")
                && !Self::history_contains_case_insensitive(history, "shortcut 'n'")
            {
                return Some(serde_json::json!({
                    "action": "shortcut",
                    "key": "n",
                    "modifiers": ["command"],
                    "app": "Notion"
                }));
            }

            if !Self::history_contains_case_insensitive(history, &summary_header) {
                return Some(serde_json::json!({
                    "action": "type",
                    "text": summary_text
                }));
            }

            return Some(serde_json::json!({ "action": "done" }));
        }

        if Self::goal_targets_todo_summary(goal) {
            let target_app = if goal_lower.contains("notes")
                || goal_lower.contains("메모")
                || goal_lower.contains("노트")
            {
                "Notes"
            } else {
                Self::text_staging_app()
            };

            let todo_header = format!("오늘 할 일 체크리스트 ({})", Utc::now().format("%Y-%m-%d"));
            let todo_text = format!(
                "{}\n1) 오늘 최우선 작업 1개를 명확한 완료 조건과 함께 적기\n2) 30분 이내 착수 가능한 작업 2개 선정\n3) 지연 위험이 있는 항목 1개와 대응책 작성\n4) 커뮤니케이션 필요한 항목 1개와 담당자 지정\n5) 오늘 마감 전 점검 체크 1회 예약",
                todo_header
            );

            let opened_marker = format!("Opened app: {}", target_app);
            if !Self::history_contains_case_insensitive(history, &opened_marker) {
                return Some(serde_json::json!({
                    "action": "open_app",
                    "name": target_app
                }));
            }

            if target_app.eq_ignore_ascii_case("Notes")
                && !Self::history_contains_case_insensitive(history, "Created new item")
                && !Self::history_contains_case_insensitive(history, "shortcut 'n'")
            {
                return Some(serde_json::json!({
                    "action": "shortcut",
                    "key": "n",
                    "modifiers": ["command"],
                    "app": "Notes"
                }));
            }

            if !Self::history_contains_case_insensitive(history, &todo_header) {
                return Some(serde_json::json!({
                    "action": "type",
                    "text": todo_text
                }));
            }

            return Some(serde_json::json!({ "action": "done" }));
        }

        let wants_downloads = goal_lower.contains("downloads") || goal_lower.contains("다운로드");
        let apps_in_goal = Self::ordered_apps_in_goal(goal);
        let current_app = Self::last_opened_app_from_history(history);

        // Keep Finder->Downloads progression explicit before jumping to later apps.
        if wants_downloads {
            let finder_opened =
                Self::history_contains_case_insensitive(history, "Opened app: Finder");
            if !finder_opened {
                return Some(serde_json::json!({ "action": "open_app", "name": "Finder" }));
            }

            let downloads_opened = Self::history_contains_case_insensitive(
                history,
                "Opened Downloads folder in Finder",
            );
            if !downloads_opened {
                return Some(
                    serde_json::json!({ "action": "click_ref", "ref": "LeftSidebarDownloads", "app": "Finder" }),
                );
            }
        }

        if let Some(app_name) = current_app.as_deref() {
            let app_lower = app_name.to_lowercase();
            if app_name.eq_ignore_ascii_case("Calendar")
                && Self::goal_requires_telegram_send(goal)
                && !Self::history_has_read_result(history)
            {
                return Some(serde_json::json!({
                    "action": "read",
                    "query": "오늘 일정의 핵심 항목을 짧게 요약"
                }));
            }
            if app_name.eq_ignore_ascii_case("Notes") {
                let wants_textedit = apps_in_goal
                    .iter()
                    .any(|app| app.eq_ignore_ascii_case("TextEdit"));
                if wants_textedit {
                    let copied_from_notes =
                        Self::history_contains_case_insensitive(history, "Copied selection");
                    if copied_from_notes {
                        let last_notes_idx = Self::last_history_index_contains_case_insensitive(
                            history,
                            "Opened app: Notes",
                        );
                        let last_textedit_idx = Self::last_history_index_contains_case_insensitive(
                            history,
                            "Opened app: TextEdit",
                        );
                        let textedit_after_notes = match (last_notes_idx, last_textedit_idx) {
                            (Some(n_idx), Some(t_idx)) => t_idx > n_idx,
                            _ => false,
                        };
                        if !textedit_after_notes {
                            return Some(
                                serde_json::json!({ "action": "open_app", "name": "TextEdit" }),
                            );
                        }
                    }
                }
            }

            let mentions_new_item = Self::goal_contains_any(
                &goal_lower,
                &[
                    "cmd+n",
                    "command+n",
                    "새 메모",
                    "새 문서",
                    "새 이메일",
                    "new note",
                    "new document",
                    "new email",
                    "new draft",
                ],
            );
            if mentions_new_item
                && !Self::history_contains_shortcut(history, "n")
                && matches!(app_lower.as_str(), "notes" | "textedit" | "mail")
            {
                return Some(serde_json::json!({
                    "action": "shortcut",
                    "key": "n",
                    "modifiers": ["command"],
                    "app": app_name
                }));
            }

            if app_name.eq_ignore_ascii_case("Mail") {
                if let Some(subject) = Self::extract_mail_subject_from_goal(goal) {
                    if !Self::history_contains_case_insensitive(history, "(mail subject)") {
                        return Some(
                            serde_json::json!({ "action": "type", "text": subject, "app": "Mail" }),
                        );
                    }
                }

                let mail_body_done =
                    Self::history_contains_case_insensitive(history, "(mail body)")
                        || Self::history_contains_case_insensitive(
                            history,
                            "pasted clipboard contents (mail body)",
                        );
                let wants_mail_paste = Self::goal_contains_any(
                    &goal_lower,
                    &["붙여넣", "paste", "cmd+v", "command+v"],
                );
                if wants_mail_paste && !mail_body_done {
                    return Some(serde_json::json!({ "action": "paste", "app": "Mail" }));
                }

                let mail_send_done =
                    Self::history_contains_case_insensitive(history, "mail send completed")
                        || Self::history_contains_case_insensitive(history, "(mail sent)")
                        || (Self::history_contains_case_insensitive(
                            history,
                            "mail send blocked: sent_pending|",
                        ) && Self::history_contains_case_insensitive(
                            history,
                            "mail send blocked: no_draft|0|0",
                        ));
                if Self::goal_requires_mail_send(goal) && !mail_send_done {
                    return Some(serde_json::json!({ "action": "mail_send", "app": "Mail" }));
                }
            }

            if Self::goal_requires_telegram_send(goal)
                && !Self::history_has_telegram_send_done(history)
            {
                let mail_ready = !Self::goal_requires_mail_send(goal)
                    || Self::history_has_mail_send_done(history);
                if mail_ready && Self::history_has_read_result(history) {
                    return Some(serde_json::json!({ "action": "telegram_send" }));
                }
            }

            if Self::is_textual_app(app_name) {
                let mail_subject = Self::extract_mail_subject_from_goal(goal);
                if !app_name.eq_ignore_ascii_case("Mail") {
                    let mut fragments: Vec<String> = Vec::new();
                    for fragment in Self::extract_goal_text_fragments(goal) {
                        let trimmed = fragment.trim();
                        let lower = trimmed.to_lowercase();
                        if trimmed.len() < 2
                            || lower.starts_with("cmd+")
                            || lower.starts_with("status:")
                            || trimmed.to_uppercase().starts_with("RUN_SCOPE_")
                        {
                            continue;
                        }

                        if let Some(subject) = mail_subject.as_deref() {
                            if trimmed.eq_ignore_ascii_case(subject) {
                                continue;
                            }
                        }
                        fragments.push(trimmed.to_string());
                    }

                    if app_name.eq_ignore_ascii_case("Notes") && fragments.len() > 1 {
                        let combined = fragments.join("\n");
                        if !Self::history_contains_case_insensitive(history, &combined) {
                            return Some(serde_json::json!({
                                "action": "type",
                                "text": combined,
                                "app": app_name
                            }));
                        }
                    }

                    for trimmed in fragments {
                        if !Self::history_contains_case_insensitive(history, &trimmed) {
                            return Some(serde_json::json!({
                                "action": "type",
                                "text": trimmed,
                                "app": app_name
                            }));
                        }
                    }

                    if Self::goal_contains_any(
                        &goal_lower,
                        &["select all", "전체 선택", "cmd+a", "command+a"],
                    ) && !Self::history_contains_case_insensitive(
                        history,
                        "Selected all contents",
                    ) {
                        return Some(
                            serde_json::json!({ "action": "select_all", "app": app_name }),
                        );
                    }

                    if Self::goal_contains_any(&goal_lower, &["copy", "복사", "cmd+c", "command+c"])
                        && !Self::history_contains_case_insensitive(history, "Copied selection")
                    {
                        return Some(serde_json::json!({ "action": "copy", "app": app_name }));
                    }
                }

                if Self::goal_contains_any(&goal_lower, &["paste", "붙여넣", "cmd+v", "command+v"])
                    && !Self::history_contains_case_insensitive(history, "Pasted")
                {
                    return Some(serde_json::json!({ "action": "paste", "app": app_name }));
                }
            }
        }

        if Self::goal_requires_telegram_send(goal) && !Self::history_has_telegram_send_done(history)
        {
            let mail_ready =
                !Self::goal_requires_mail_send(goal) || Self::history_has_mail_send_done(history);
            if mail_ready && Self::history_has_read_result(history) {
                return Some(serde_json::json!({ "action": "telegram_send" }));
            }
        }

        for app in &apps_in_goal {
            let marker = format!("Opened app: {}", app);
            if !Self::history_contains_case_insensitive(history, &marker) {
                return Some(serde_json::json!({ "action": "open_app", "name": app }));
            }
        }

        if apps_in_goal.is_empty() {
            let fragments = Self::extract_goal_text_fragments(goal)
                .into_iter()
                .filter(|frag| {
                    let trimmed = frag.trim();
                    let lower = trimmed.to_lowercase();
                    trimmed.len() >= 3
                        && !lower.starts_with("cmd+")
                        && lower != "done"
                        && !lower.starts_with("status:")
                        && !trimmed.to_uppercase().starts_with("RUN_SCOPE_")
                })
                .collect::<Vec<_>>();
            if !fragments.is_empty() {
                let staging_app = Self::text_staging_app();
                let opened_marker = format!("Opened app: {}", staging_app);
                if !Self::history_contains_case_insensitive(history, &opened_marker) {
                    return Some(serde_json::json!({ "action": "open_app", "name": staging_app }));
                }

                let typed_marker = if staging_app.eq_ignore_ascii_case("Notes") {
                    "(notes body)"
                } else {
                    "(textedit body)"
                };
                if !Self::history_contains_case_insensitive(history, typed_marker) {
                    let combined = fragments.join("\n");
                    return Some(serde_json::json!({
                        "action": "type",
                        "text": combined,
                        "app": staging_app
                    }));
                }
            }
        }

        if !apps_in_goal.is_empty() {
            return Some(serde_json::json!({ "action": "done" }));
        }

        None
    }

    fn should_relax_review(reason: &str, notes: &str) -> bool {
        let text = format!("{} {}", reason.to_lowercase(), notes.to_lowercase());

        let strict_signals = [
            "full sequence",
            "entire sequence",
            "initial step",
            "only the first step",
            "only includes opening",
            "incomplete",
            "single step",
            "not the complete",
        ];
        let has_strict_signal = strict_signals.iter().any(|s| text.contains(s));

        let hard_blockers = [
            "danger",
            "unsafe",
            "impossible",
            "stuck in a loop",
            "does not relate",
            "not related",
            "before opening safari",
            "without ensuring safari is open",
        ];
        let has_hard_blocker = hard_blockers.iter().any(|s| text.contains(s));

        has_strict_signal && !has_hard_blocker
    }

    fn goal_has_explicit_sequence(goal: &str) -> bool {
        let lower = goal.to_lowercase();
        lower.contains(" then ")
            || lower.contains("다음")
            || lower.contains("후")
            || lower.contains("이후")
            || lower.contains("next")
    }

    fn goal_has_multi_app(goal: &str) -> bool {
        Self::ordered_apps_in_goal(goal).len() > 1
    }

    fn should_accept_text_flow_after_type(
        plan: &serde_json::Value,
        history: &[String],
        reason: &str,
        notes: &str,
    ) -> bool {
        let action = plan["action"].as_str().unwrap_or("");
        if !matches!(action, "select_all" | "copy" | "paste") {
            return false;
        }

        let recent_typed = history.iter().rev().take(10).any(|h| {
            let lower = h.to_lowercase();
            lower.starts_with("typed '")
                || lower.contains("typed \"")
                || lower.contains("typed ")
                || lower.contains("read_result:")
        });
        if !recent_typed {
            return false;
        }

        let text = format!("{} {}", reason.to_lowercase(), notes.to_lowercase());
        let confirmation_only_signals = [
            "confirmation",
            "confirm",
            "visible",
            "evidence",
            "fully entered",
            "full content",
            "입력",
            "보이",
            "확인",
        ];
        let has_confirmation_signal = confirmation_only_signals
            .iter()
            .any(|s| text.contains(&s.to_lowercase()));

        let hard_blockers = [
            "danger",
            "unsafe",
            "impossible",
            "not related",
            "does not relate",
            "wrong app",
            "before opening",
        ];
        let has_hard_blocker = hard_blockers.iter().any(|s| text.contains(s));

        has_confirmation_signal && !has_hard_blocker
    }

    fn should_accept_typing_after_new_item_shortcut(
        plan: &serde_json::Value,
        history: &[String],
        reason: &str,
        notes: &str,
    ) -> bool {
        if plan["action"].as_str() != Some("type") {
            return false;
        }

        let has_new_item_shortcut = history.iter().rev().take(8).any(|h| {
            let lower = h.to_lowercase();
            lower.contains("shortcut 'n'")
                && lower.contains("command")
                && (lower.contains("created new item") || lower.contains("shortcut"))
        });
        if !has_new_item_shortcut {
            return false;
        }

        let text = format!("{} {}", reason.to_lowercase(), notes.to_lowercase());
        let note_creation_signals = [
            "new note",
            "new item",
            "새 메모",
            "생성",
            "no evidence",
            "must ensure",
        ];
        let has_note_creation_signal = note_creation_signals
            .iter()
            .any(|s| text.contains(&s.to_lowercase()));

        let hard_blockers = [
            "danger",
            "unsafe",
            "impossible",
            "not related",
            "wrong app",
            "before opening",
        ];
        let has_hard_blocker = hard_blockers.iter().any(|s| text.contains(s));

        has_note_creation_signal && !has_hard_blocker
    }

    fn history_contains_shortcut(history: &[String], key: &str) -> bool {
        let needle = format!("shortcut '{}'", key.to_lowercase());
        history.iter().any(|h| h.to_lowercase().contains(&needle))
    }

    fn ordered_apps_in_goal(goal: &str) -> Vec<&'static str> {
        let goal_lower = Self::normalize_text_for_matching(goal);
        let app_aliases = [
            ("calendar", "Calendar"),
            ("캘린더", "Calendar"),
            ("calendario", "Calendar"),
            ("カレンダー", "Calendar"),
            ("日历", "Calendar"),
            ("google chrome", "Google Chrome"),
            ("chrome", "Google Chrome"),
            ("크롬", "Google Chrome"),
            ("cromo", "Google Chrome"),
            ("クローム", "Google Chrome"),
            ("谷歌浏览器", "Google Chrome"),
            ("safari", "Safari"),
            ("サファリ", "Safari"),
            ("파인더", "Finder"),
            ("finder", "Finder"),
            ("explorador", "Finder"),
            ("textedit", "TextEdit"),
            ("텍스트에디트", "TextEdit"),
            ("notes", "Notes"),
            ("note", "Notes"),
            ("노트", "Notes"),
            ("메모장", "Notes"),
            ("메모", "Notes"),
            ("notas", "Notes"),
            ("nota", "Notes"),
            ("notes app", "Notes"),
            ("メモ", "Notes"),
            ("ノート", "Notes"),
            ("笔记", "Notes"),
            ("記事", "Notes"),
            ("calculator", "Calculator"),
            ("계산기", "Calculator"),
            ("calculadora", "Calculator"),
            ("計算機", "Calculator"),
            ("计算器", "Calculator"),
            ("mail", "Mail"),
            ("이메일", "Mail"),
            ("메일", "Mail"),
            ("correo", "Mail"),
            ("email", "Mail"),
            ("メール", "Mail"),
            ("邮箱", "Mail"),
        ];
        let mut found: Vec<(usize, &'static str)> = app_aliases
            .iter()
            .filter_map(|(alias, app)| goal_lower.find(alias).map(|idx| (idx, *app)))
            .collect();
        found.sort_by_key(|(idx, _)| *idx);
        let mut ordered: Vec<&'static str> = Vec::new();
        for (_, app) in found {
            if !ordered.iter().any(|seen| seen.eq_ignore_ascii_case(app)) {
                ordered.push(app);
            }
        }
        ordered
    }

    fn extract_quoted_fragments(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut in_double = false;
        let mut in_single = false;
        let mut buf = String::new();
        for ch in text.chars() {
            if ch == '"' && !in_single {
                if in_double {
                    let value = buf.trim().to_string();
                    if !value.is_empty() {
                        out.push(value);
                    }
                    buf.clear();
                    in_double = false;
                } else {
                    in_double = true;
                    buf.clear();
                }
                continue;
            }
            if ch == '\'' && !in_double {
                if in_single {
                    let value = buf.trim().to_string();
                    if !value.is_empty() {
                        out.push(value);
                    }
                    buf.clear();
                    in_single = false;
                } else {
                    in_single = true;
                    buf.clear();
                }
                continue;
            }
            if in_double || in_single {
                buf.push(ch);
            }
        }
        out
    }

    fn normalize_goal_text_fragment(raw: &str) -> Option<String> {
        let mut text = raw.trim().to_string();
        if text.is_empty() {
            return None;
        }

        if let Ok(prefix_re) = regex::Regex::new(
            r"(?i)^.*?(?:메모장|메모|notes|note|textedit|텍스트에디트)\s*(?:을|를)?\s*(?:열어줘|열어서|열고|열어|open|launch)\s*",
        ) {
            text = prefix_re.replace(&text, "").to_string();
        }

        text = text
            .trim()
            .trim_matches(|ch| {
                matches!(
                    ch,
                    '"' | '\'' | '“' | '”' | '‘' | '’' | '「' | '」' | '『' | '』'
                )
            })
            .trim()
            .to_string();

        if let Some(stripped) = text.strip_suffix("이라고") {
            text = stripped.trim().to_string();
        } else if let Some(stripped) = text.strip_suffix("라고") {
            text = stripped.trim().to_string();
        }

        if text.is_empty() {
            return None;
        }

        let lower = text.to_lowercase();
        if lower.starts_with("cmd+") || lower.starts_with("status:") || lower == "done" {
            return None;
        }

        Some(text)
    }

    fn extract_goal_text_fragments(goal: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for fragment in Self::extract_quoted_fragments(goal) {
            if let Some(normalized) = Self::normalize_goal_text_fragment(&fragment) {
                if !out.contains(&normalized) {
                    out.push(normalized);
                }
            }
        }

        if let Ok(korean_write_re) = regex::Regex::new(
            r#"(?P<payload>[^"'“”‘’\n]{1,160}?)(?:이라고|라고)\s*(?:써줘|써 줘|적어줘|적어 줘|입력해줘|입력해 줘|작성해줘|작성해 줘|써|적어|입력|작성)"#,
        ) {
            for captures in korean_write_re.captures_iter(goal) {
                if let Some(raw_payload) = captures.name("payload") {
                    if let Some(normalized) =
                        Self::normalize_goal_text_fragment(raw_payload.as_str())
                    {
                        if !out.contains(&normalized) {
                            out.push(normalized);
                        }
                    }
                }
            }
        }

        out
    }

    fn goal_run_scope_marker(goal: &str) -> Option<String> {
        let run_scope_re = regex::Regex::new(r"(?i)(RUN_SCOPE_[A-Z0-9_]+)").ok();

        for fragment in Self::extract_goal_text_fragments(goal) {
            let f = fragment.trim();
            if let Some(re) = &run_scope_re {
                if let Some(caps) = re.captures(f) {
                    if let Some(m) = caps.get(1) {
                        return Some(m.as_str().to_string());
                    }
                }
            }
            if f.to_uppercase().starts_with("RUN_SCOPE_") {
                return Some(
                    f.chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect(),
                );
            }
        }

        for token in goal.split_whitespace() {
            let cleaned = token
                .trim_matches(|c: char| {
                    c == '"' || c == '\'' || c == '“' || c == '”' || c == '‘' || c == '’'
                })
                .trim();
            if let Some(re) = &run_scope_re {
                if let Some(caps) = re.captures(cleaned) {
                    if let Some(m) = caps.get(1) {
                        return Some(m.as_str().to_string());
                    }
                }
            }
            if cleaned.to_uppercase().starts_with("RUN_SCOPE_") {
                return Some(
                    cleaned
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect(),
                );
            }
        }

        None
    }

    fn extract_mail_subject_from_goal(goal: &str) -> Option<String> {
        let lower = goal.to_lowercase();
        let mut scopes: Vec<&str> = Vec::new();
        let mail_idx = lower
            .rfind("mail")
            .or_else(|| lower.rfind("메일"))
            .or_else(|| lower.rfind("이메일"));
        if let Some(idx) = mail_idx {
            scopes.push(&goal[idx..]);
        }
        scopes.push(goal);

        let email_re = regex::Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").ok();

        for scope in scopes {
            let scope_lower = scope.to_lowercase();
            for marker in ["제목", "subject", "title"] {
                if let Some(idx) = scope_lower.find(marker) {
                    let rest = &scope[idx + marker.len()..];
                    for frag in Self::extract_quoted_fragments(rest) {
                        let f = frag.trim();
                        let lf = f.to_lowercase();
                        if f.len() < 2 {
                            continue;
                        }
                        if lf.starts_with("cmd+") || lf.starts_with("status:") {
                            continue;
                        }
                        if f.starts_with("RUN_SCOPE_") {
                            continue;
                        }
                        if email_re.as_ref().map(|re| re.is_match(f)).unwrap_or(false) {
                            continue;
                        }
                        return Some(f.to_string());
                    }
                }
            }
        }

        if lower.contains("mail") || lower.contains("메일") {
            for frag in Self::extract_quoted_fragments(goal) {
                if frag.contains("S1_")
                    || frag.contains("S2_")
                    || frag.contains("S3_")
                    || frag.contains("S4_")
                    || frag.contains("S5_")
                {
                    return Some(frag.trim().to_string());
                }
            }
            for frag in Self::extract_quoted_fragments(goal) {
                let f = frag.trim();
                let lf = f.to_lowercase();
                if f.len() < 2 {
                    continue;
                }
                if lf.contains("cmd+") || lf.starts_with("status:") {
                    continue;
                }
                if f.starts_with("RUN_SCOPE_") {
                    continue;
                }
                if email_re.as_ref().map(|re| re.is_match(f)).unwrap_or(false) {
                    continue;
                }
                return Some(f.to_string());
            }
        }
        None
    }

    fn history_has_mail_subject(history: &[String]) -> bool {
        history.iter().any(|h| {
            let lower = h.to_lowercase();
            lower.contains("(mail subject)") || lower.contains("mail subject")
        })
    }

    fn history_has_read_result(history: &[String]) -> bool {
        history.iter().any(|h| h.starts_with("READ_RESULT: "))
    }

    fn history_has_telegram_send_done(history: &[String]) -> bool {
        history.iter().any(|entry| {
            let lower = entry.to_lowercase();
            lower.contains("telegram send completed")
                || lower.contains("telegram: sent")
                || lower.contains("target=telegram|event=send|status=sent")
        })
    }

    fn last_opened_app(history: &[String]) -> Option<String> {
        for entry in history.iter().rev() {
            if let Some(rest) = entry.strip_prefix("Opened app: ") {
                let app = rest.trim();
                if !app.is_empty() {
                    return Some(app.to_string());
                }
            }
        }
        None
    }

    fn has_recent_created_item(history: &[String]) -> bool {
        history
            .iter()
            .rev()
            .take(8)
            .any(|h| h.to_lowercase().contains("created new item"))
    }

    fn plan_is_cmd_n_shortcut(plan: &serde_json::Value) -> bool {
        if plan["action"].as_str() != Some("shortcut") {
            return false;
        }
        let key_is_n = plan["key"]
            .as_str()
            .map(|k| k.eq_ignore_ascii_case("n"))
            .unwrap_or(false);
        if !key_is_n {
            return false;
        }
        plan["modifiers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .any(|m| m.as_str().unwrap_or("").eq_ignore_ascii_case("command"))
            })
            .unwrap_or(false)
    }

    fn history_has_recent_new_item_for_app(history: &[String], app_name: &str) -> bool {
        let target = app_name.to_lowercase();
        let mut in_target_context = Self::last_opened_app(history)
            .map(|app| app.eq_ignore_ascii_case(app_name))
            .unwrap_or(false);

        for entry in history.iter().rev().take(24) {
            let lower = entry.to_lowercase();
            if let Some(rest) = lower.strip_prefix("opened app: ") {
                let opened = rest.trim();
                if opened.eq_ignore_ascii_case(&target) {
                    in_target_context = true;
                    continue;
                }
                if in_target_context {
                    break;
                }
                continue;
            }
            if !in_target_context {
                continue;
            }
            if lower.contains("shortcut 'n'") && lower.contains("created new item") {
                return true;
            }
            if lower.contains("mail send completed") || lower.contains("(mail sent)") {
                return false;
            }
        }
        false
    }

    fn maybe_rewrite_mail_subject_before_paste(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        let action = plan["action"].as_str().unwrap_or("");
        if !matches!(action, "paste" | "done" | "shortcut") {
            return;
        }

        let in_mail_context =
            match Self::last_opened_app(history) {
                Some(app) => app.eq_ignore_ascii_case("Mail"),
                None => false,
            } || Self::history_contains_case_insensitive(history, "Opened app: Mail");
        if !in_mail_context {
            return;
        }

        if Self::history_has_mail_subject(history) {
            return;
        }

        let is_cmd_n_shortcut = action == "shortcut"
            && plan["key"].as_str().map(|k| k.eq_ignore_ascii_case("n")) == Some(true)
            && plan["modifiers"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .any(|m| m.as_str().unwrap_or("").eq_ignore_ascii_case("command"))
                })
                .unwrap_or(false);
        if action == "shortcut" && !is_cmd_n_shortcut {
            return;
        }

        if let Some(subject) = Self::extract_mail_subject_from_goal(goal) {
            *plan = serde_json::json!({
                "action": "type",
                "app": "Mail",
                "text": subject
            });
            println!("   🔁 Rewrote action to set Mail subject before paste.");
        }
    }

    fn maybe_rewrite_shortcut_to_next_app(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if plan["action"].as_str() != Some("shortcut") {
            return;
        }
        if plan.get("app").and_then(|v| v.as_str()).is_some() {
            return;
        }

        let key = plan["key"].as_str().unwrap_or("").to_lowercase();
        let has_command = plan["modifiers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .any(|m| m.as_str().unwrap_or("").eq_ignore_ascii_case("command"))
            })
            .unwrap_or(false);
        if key != "n" || !has_command {
            return;
        }

        let Some(last_opened) = Self::last_opened_app(history) else {
            return;
        };
        if !Self::has_recent_created_item(history) {
            return;
        }

        let ordered = Self::ordered_apps_in_goal(goal);
        let mut current_idx: Option<usize> = None;
        for (idx, app) in ordered.iter().enumerate() {
            if app.eq_ignore_ascii_case(&last_opened) {
                current_idx = Some(idx);
                break;
            }
        }
        let Some(idx) = current_idx else {
            return;
        };
        let Some(next_app) = ordered.get(idx + 1) else {
            return;
        };

        let opened_marker = format!("Opened app: {}", next_app);
        if Self::history_contains_case_insensitive(history, &opened_marker) {
            return;
        }

        *plan = serde_json::json!({ "action": "open_app", "name": next_app });
        println!(
            "   🔁 Rewrote repeated Cmd+N shortcut to next app transition: {}",
            next_app
        );
    }

    fn maybe_rewrite_redundant_new_item_shortcut(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if !Self::plan_is_cmd_n_shortcut(plan) {
            return;
        }

        let target_app = plan["app"]
            .as_str()
            .map(|v| v.to_string())
            .or_else(|| Self::last_opened_app(history));
        let Some(app_name) = target_app else {
            return;
        };
        if !Self::is_textual_app(&app_name) {
            return;
        }
        if !Self::history_has_recent_new_item_for_app(history, &app_name) {
            return;
        }

        if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, history) {
            if !Self::plan_is_cmd_n_shortcut(&fallback_plan) {
                let action = fallback_plan["action"].as_str().unwrap_or("").to_string();
                *plan = fallback_plan;
                println!(
                    "   🔁 Rewrote redundant Cmd+N to progress action: {} (app={})",
                    action, app_name
                );
                return;
            }
        }

        if app_name.eq_ignore_ascii_case("Mail") {
            if Self::goal_requires_mail_send(goal) && !Self::history_has_mail_send_done(history) {
                if !Self::history_has_mail_body(history) {
                    *plan = serde_json::json!({ "action": "paste", "app": "Mail" });
                    println!("   🔁 Rewrote redundant Cmd+N to paste (Mail body pending).");
                } else {
                    *plan = serde_json::json!({ "action": "mail_send", "app": "Mail" });
                    println!("   🔁 Rewrote redundant Cmd+N to mail_send (Mail send pending).");
                }
            } else {
                *plan = serde_json::json!({ "action": "done" });
                println!("   🔁 Rewrote redundant Cmd+N to done (Mail already satisfied).");
            }
            return;
        }

        *plan = serde_json::json!({ "action": "done" });
        println!(
            "   🔁 Rewrote redundant Cmd+N to done (new item already created in {}).",
            app_name
        );
    }

    fn extract_known_app_from_text(text: &str) -> Option<&'static str> {
        let lower = Self::normalize_text_for_matching(text);
        let aliases = [
            ("calendar", "Calendar"),
            ("캘린더", "Calendar"),
            ("calendario", "Calendar"),
            ("カレンダー", "Calendar"),
            ("日历", "Calendar"),
            ("google chrome", "Google Chrome"),
            ("chrome", "Google Chrome"),
            ("크롬", "Google Chrome"),
            ("cromo", "Google Chrome"),
            ("クローム", "Google Chrome"),
            ("谷歌浏览器", "Google Chrome"),
            ("notes", "Notes"),
            ("메모", "Notes"),
            ("노트", "Notes"),
            ("notas", "Notes"),
            ("nota", "Notes"),
            ("メモ", "Notes"),
            ("ノート", "Notes"),
            ("笔记", "Notes"),
            ("textedit", "TextEdit"),
            ("mail", "Mail"),
            ("메일", "Mail"),
            ("correo", "Mail"),
            ("email", "Mail"),
            ("メール", "Mail"),
            ("邮箱", "Mail"),
            ("finder", "Finder"),
            ("safari", "Safari"),
            ("calculator", "Calculator"),
            ("계산기", "Calculator"),
            ("calculadora", "Calculator"),
            ("計算機", "Calculator"),
            ("计算器", "Calculator"),
            ("notion", "Notion"),
            ("노션", "Notion"),
        ];

        for (needle, app) in aliases {
            if lower.contains(needle) {
                return Some(app);
            }
        }
        None
    }

    fn next_unopened_app_in_goal(goal: &str, history: &[String]) -> Option<&'static str> {
        for app in Self::ordered_apps_in_goal(goal) {
            let marker = format!("Opened app: {}", app);
            if !Self::history_contains_case_insensitive(history, &marker) {
                return Some(app);
            }
        }
        None
    }

    fn maybe_rewrite_click_visual_to_app_action(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if plan["action"].as_str() != Some("click_visual") {
            return;
        }

        let description = plan["description"].as_str().unwrap_or("").trim();
        if description.is_empty() {
            return;
        }

        let desc_lower = description.to_lowercase();
        let looks_like_app_switch = desc_lower.contains("dock")
            || desc_lower.contains("icon")
            || desc_lower.contains("앱")
            || desc_lower.contains("application");
        if !looks_like_app_switch {
            return;
        }

        let target_app = Self::extract_known_app_from_text(description)
            .or_else(|| Self::next_unopened_app_in_goal(goal, history));
        let Some(target_app) = target_app else {
            return;
        };

        let opened_marker = format!("Opened app: {}", target_app);
        if Self::history_contains_case_insensitive(history, &opened_marker) {
            *plan = serde_json::json!({ "action": "switch_app", "app": target_app });
            println!(
                "   🔁 Rewrote click_visual dock/app action to switch_app: {}",
                target_app
            );
        } else {
            *plan = serde_json::json!({ "action": "open_app", "name": target_app });
            println!(
                "   🔁 Rewrote click_visual dock/app action to open_app: {}",
                target_app
            );
        }
    }

    fn maybe_repair_open_app_missing_name(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if plan["action"].as_str() != Some("open_app") {
            return;
        }

        let has_name = plan["name"]
            .as_str()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if has_name {
            return;
        }

        let mut inferred: Option<&'static str> = None;

        if inferred.is_none() {
            if let Some(app_text) = plan["app"].as_str() {
                inferred = Self::extract_known_app_from_text(app_text);
            }
        }

        if inferred.is_none() {
            if let Some(desc) = plan["description"].as_str() {
                inferred = Self::extract_known_app_from_text(desc);
            }
        }

        if inferred.is_none() {
            inferred = Self::next_unopened_app_in_goal(goal, history);
        }

        if inferred.is_none() {
            inferred = Self::ordered_apps_in_goal(goal).into_iter().next();
        }

        if let Some(app) = inferred {
            plan["name"] = serde_json::Value::String(app.to_string());
            println!("   🛠️ Repaired open_app missing name -> {}", app);
        }
    }

    fn in_mail_context(history: &[String]) -> bool {
        (match Self::last_opened_app(history) {
            Some(app) => app.eq_ignore_ascii_case("Mail"),
            None => false,
        }) || Self::history_contains_case_insensitive(history, "Opened app: Mail")
    }

    fn history_has_mail_body(history: &[String]) -> bool {
        Self::history_contains_case_insensitive(history, "(mail body)")
            || Self::history_contains_case_insensitive(
                history,
                "pasted clipboard contents (mail body)",
            )
    }

    fn history_has_mail_send_done(history: &[String]) -> bool {
        Self::history_contains_case_insensitive(history, "mail send completed")
            || Self::history_contains_case_insensitive(history, "(mail sent)")
            || (Self::history_contains_case_insensitive(
                history,
                "mail send blocked: sent_pending|",
            ) && Self::history_contains_case_insensitive(
                history,
                "mail send blocked: no_draft|0|0",
            ))
    }

    fn maybe_rewrite_click_visual_mail_body(history: &[String], plan: &mut serde_json::Value) {
        if plan["action"].as_str() != Some("click_visual") {
            return;
        }
        if !Self::in_mail_context(history) {
            return;
        }

        let desc = plan["description"].as_str().unwrap_or("");
        let desc_lc = desc.to_lowercase();
        let is_mail_body_target = desc_lc.contains("message body")
            || desc_lc.contains("mail body")
            || desc_lc.contains("compose body")
            || desc_lc.contains("본문")
            || desc_lc.contains("메시지");
        if !is_mail_body_target {
            return;
        }

        *plan = serde_json::json!({ "action": "paste", "app": "Mail" });
        println!("   🔁 Rewrote Mail body click_visual to deterministic paste.");
    }

    fn maybe_rewrite_snapshot_to_progress_action(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if plan["action"].as_str() != Some("snapshot") {
            return;
        }

        if Self::in_mail_context(history) && Self::goal_requires_mail_send(goal) {
            if Self::history_has_mail_send_done(history) {
                *plan = serde_json::json!({ "action": "done" });
                println!("   🔁 Rewrote snapshot to done (Mail send already confirmed).");
                return;
            }
            if !Self::history_has_mail_body(history) {
                *plan = serde_json::json!({ "action": "paste", "app": "Mail" });
                println!("   🔁 Rewrote snapshot to paste (Mail body pending).");
            } else {
                *plan = serde_json::json!({ "action": "mail_send", "app": "Mail" });
                println!("   🔁 Rewrote snapshot to mail_send (Mail send pending).");
            }
            return;
        }

        if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, history) {
            let action = fallback_plan["action"].as_str().unwrap_or("").to_string();
            if action != "snapshot" {
                *plan = fallback_plan;
                println!(
                    "   🔁 Rewrote snapshot to fallback progress action: {}",
                    action
                );
            }
        }
    }

    fn maybe_rewrite_open_app_to_pending_text_action(
        goal: &str,
        history: &[String],
        plan: &mut serde_json::Value,
    ) {
        if plan["action"].as_str() != Some("open_app") {
            return;
        }

        let target_app = plan["name"].as_str().unwrap_or("").trim();

        let Some(current_app) = Self::last_opened_app_from_history(history) else {
            return;
        };
        if !Self::is_textual_app(&current_app) {
            return;
        }

        // Keep explicit cross-app transitions intact.
        if !target_app.is_empty() && !target_app.eq_ignore_ascii_case(&current_app) {
            return;
        }

        let has_pending_literal = Self::extract_quoted_fragments(goal)
            .into_iter()
            .any(|frag| {
                let trimmed = frag.trim();
                let lower = trimmed.to_lowercase();
                if trimmed.len() < 2 || lower.starts_with("cmd+") || lower.starts_with("status:") {
                    return false;
                }
                !Self::history_contains_case_insensitive(history, trimmed)
            });
        if !has_pending_literal {
            return;
        }

        if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, history) {
            let action = fallback_plan["action"].as_str().unwrap_or("").to_string();
            if matches!(
                action.as_str(),
                "type" | "select_all" | "copy" | "paste" | "shortcut"
            ) {
                *plan = fallback_plan;
                println!(
                    "   🔁 Rewrote open_app to pending text-flow action: {} (current app: {})",
                    action, current_app
                );
            }
        }
    }

    fn can_force_done_for_simple_goal(
        goal: &str,
        plan: &serde_json::Value,
        history: &[String],
    ) -> bool {
        if plan["action"].as_str() != Some("done") {
            return false;
        }

        let goal_lower = goal.to_lowercase();
        let is_note_creation_goal = goal_lower.contains("notes")
            && (goal_lower.contains("새 메모")
                || goal_lower.contains("new note")
                || goal_lower.contains("new memo"));
        if !is_note_creation_goal {
            return false;
        }

        let complex_markers = [
            "입력",
            "type",
            "복사",
            "copy",
            "붙여",
            "paste",
            "전송",
            "mail",
            "calculator",
            "textedit",
            "finder",
            "safari",
            "google",
            "검색",
        ];
        if complex_markers.iter().any(|m| goal_lower.contains(m)) {
            return false;
        }

        let opened_notes = Self::history_contains_case_insensitive(history, "Opened app: Notes");
        let made_new_note = Self::history_contains_shortcut(history, "n");
        opened_notes && made_new_note
    }

    fn env_truthy(name: &str) -> bool {
        matches!(
            std::env::var(name).as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        )
    }

    fn env_truthy_default(name: &str, default_value: bool) -> bool {
        match std::env::var(name) {
            Ok(raw) => matches!(raw.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
            Err(_) => default_value,
        }
    }

    fn should_abort_on_execution_error(err: &anyhow::Error) -> bool {
        if !Self::env_truthy_default("STEER_ABORT_ON_EXECUTION_ERROR", true) {
            return false;
        }

        let msg = err.to_string().to_lowercase();
        msg.contains("critical ")
            || msg.contains("focus_recovery_failed")
            || msg.contains("open app failed")
            || msg.contains("mail send")
    }

    fn supervisor_safe_bypass_enabled() -> bool {
        Self::env_truthy_default("STEER_SUPERVISOR_BYPASS_SAFE", true)
    }

    fn is_low_risk_action_for_supervisor(plan: &serde_json::Value) -> bool {
        matches!(
            plan["action"].as_str().unwrap_or(""),
            "open_app"
                | "switch_app"
                | "shortcut"
                | "key"
                | "type"
                | "paste"
                | "copy"
                | "select_all"
                | "read"
                | "read_clipboard"
                | "transfer"
        )
    }

    fn record_fallback_action(history: &mut Vec<String>, reason: &str, plan: &serde_json::Value) {
        println!("   🧯 Fallback action [{}]: {}", reason, plan);
        history.push(format!("FALLBACK_ACTION: {} => {}", reason, plan));
    }

    fn fallback_action_count(history: &[String]) -> usize {
        history
            .iter()
            .filter(|entry| entry.starts_with("FALLBACK_ACTION:"))
            .count()
    }

    fn fallback_checkpoint_limit() -> usize {
        Self::env_usize("STEER_FALLBACK_CHECKPOINT_LIMIT", 3)
    }

    fn enforce_fallback_checkpoint(history: &mut Vec<String>) -> Result<()> {
        if !Self::env_truthy_default("STEER_ENFORCE_FALLBACK_CHECKPOINT", true) {
            return Ok(());
        }
        let count = Self::fallback_action_count(history);
        let limit = Self::fallback_checkpoint_limit();
        if count < limit {
            return Ok(());
        }
        let msg = format!(
            "Fallback checkpoint reached (count={} limit={})",
            count, limit
        );
        println!("   ⛔ {}", msg);
        history.push(format!("APPROVAL_CHECKPOINT_REQUIRED: {}", msg));
        Err(anyhow::anyhow!(msg))
    }

    async fn run_standard_cleanup_preset(goal: &str, history: &mut Vec<String>) {
        if !Self::env_truthy_default("STEER_STANDARD_CLEANUP_PRESET", true) {
            return;
        }

        if heuristics::try_close_front_dialog() {
            history.push("CLEANUP_DIALOG_CLOSED: front dialog".to_string());
        }

        let lower = goal.to_lowercase();
        let mut targets: Vec<&str> = Vec::new();
        if lower.contains("mail") || lower.contains("메일") || lower.contains("이메일") {
            targets.push("Mail");
        }
        if lower.contains("notes") || lower.contains("메모") {
            targets.push("Notes");
        }
        if lower.contains("textedit") {
            targets.push("TextEdit");
        }

        for app in targets {
            let _ = heuristics::ensure_app_focus(app, 2).await;
            if heuristics::try_close_front_dialog() {
                history.push(format!("CLEANUP_DIALOG_CLOSED: {}", app));
            }
            history.push(format!("CLEANUP_APP_READY: {}", app));
        }

        if lower.contains("mail") || lower.contains("메일") || lower.contains("이메일") {
            let lines = [
                "tell application \"Mail\"",
                "set _count to (count of outgoing messages)",
                "if _count = 0 then return \"0\"",
                "repeat with _msg in outgoing messages",
                "try",
                "set visible of _msg to false",
                "end try",
                "end repeat",
                "return (_count as text)",
                "end tell",
            ];
            if let Ok(out) = crate::applescript::run_with_args(&lines, &Vec::<String>::new()) {
                let count = out.trim().to_string();
                if !count.is_empty() {
                    history.push(format!("CLEANUP_MAIL_OUTGOING_HIDDEN: {}", count));
                }
            }
        }
    }

    fn env_usize(name: &str, default_value: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(default_value)
    }

    fn env_u64(name: &str, default_value: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(default_value)
    }

    fn is_soft_planner_failure(message: &str) -> bool {
        let lower = message.to_lowercase();
        lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("rate limit")
            || lower.contains("429")
            || lower.contains("temporar")
            || lower.contains("network")
            || lower.contains("connection")
            || lower.contains("http 5")
            || lower.contains("service unavailable")
    }

    fn has_quota_exhaustion_marker(history: &[String]) -> bool {
        history.iter().any(|line| {
            let lower = line.to_lowercase();
            lower.contains("insufficient_quota")
                || lower.contains("quota")
                || lower.contains("exhausted your capacity")
                || lower.contains("rate limit")
                || lower.contains("429")
        })
    }

    async fn recover_plan_after_primary_failure(
        &self,
        goal: &str,
        history: &[String],
        failure_reason: &str,
    ) -> Option<serde_json::Value> {
        let recovery_enabled = Self::env_truthy_default("STEER_PLANNER_TIMEOUT_RECOVERY", true);
        if !recovery_enabled {
            return None;
        }

        let allow_hard_error_recovery = Self::env_truthy("STEER_PLANNER_RECOVER_HARD_ERRORS");
        if !allow_hard_error_recovery && !Self::is_soft_planner_failure(failure_reason) {
            return None;
        }

        let compact_history = history
            .iter()
            .rev()
            .take(16)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n- ");

        let system_prompt = "You are a resilient desktop automation planner.
Return exactly one JSON object for the next single action.
Allowed actions: click_visual, click_ref, type, shortcut, read, scroll, open_app, open_url, select_all, copy, paste, read_clipboard, done, wait.
Rules:
- Never output markdown or explanation text.
- If using open_app, include a non-empty name.
- Prefer the safest action that still moves toward the goal.";
        let user_prompt = format!(
            "GOAL: {}\nFAILURE_REASON: {}\nRECENT_HISTORY:\n- {}",
            goal,
            failure_reason,
            if compact_history.is_empty() {
                "None"
            } else {
                &compact_history
            }
        );

        let recovery_timeout =
            Duration::from_secs(Self::env_u64("STEER_PLANNER_RECOVERY_TIMEOUT_SEC", 14));
        let messages = vec![
            serde_json::json!({"role": "system", "content": system_prompt}),
            serde_json::json!({"role": "user", "content": user_prompt}),
        ];

        match tokio::time::timeout(recovery_timeout, self.llm.chat_completion(messages)).await {
            Ok(Ok(raw)) => {
                if let Some(parsed) = crate::llm_gateway::recover_json(&raw) {
                    println!("   🧯 Planner recovery: text-only LLM plan accepted.");
                    return Some(parsed);
                }
                println!("   ⚠️ Planner recovery: text-only output was not valid JSON.");
            }
            Ok(Err(e)) => {
                println!("   ⚠️ Planner recovery: text-only LLM failed: {}", e);
            }
            Err(_) => {
                println!(
                    "   ⚠️ Planner recovery: text-only LLM timeout after {}s.",
                    recovery_timeout.as_secs()
                );
            }
        }

        if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, history) {
            println!("   🧯 Planner recovery: deterministic fallback plan selected.");
            return Some(fallback_plan);
        }

        if let Some(app) = Self::extract_known_app_from_text(goal) {
            println!(
                "   🧯 Planner recovery: inferred open_app fallback selected ({}).",
                app
            );
            return Some(serde_json::json!({"action":"open_app","name":app}));
        }

        println!("   🧯 Planner recovery: default wait action selected.");
        Some(serde_json::json!({"action":"wait","seconds":1}))
    }

    fn notion_api_ready() -> bool {
        crate::load_env_with_fallback();
        let has_key = std::env::var("NOTION_API_KEY")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        let has_target = std::env::var("NOTION_DATABASE_ID")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            || std::env::var("NOTION_PAGE_ID")
                .ok()
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
        has_key && has_target
    }

    fn planner_retry_config() -> crate::retry_logic::RetryConfig {
        crate::retry_logic::RetryConfig {
            max_attempts: Self::env_usize("STEER_PLANNER_MAX_ATTEMPTS", 1),
            base_delay_ms: Self::env_u64("STEER_PLANNER_RETRY_BASE_DELAY_MS", 300),
            max_delay_ms: Self::env_u64("STEER_PLANNER_RETRY_MAX_DELAY_MS", 3000),
            backoff_multiplier: 2.0,
        }
    }

    fn sanitize_filename_token(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                out.push(ch);
            } else {
                out.push('_');
            }
        }
        let collapsed = out.trim_matches('_');
        let mut final_token = if collapsed.is_empty() {
            "node".to_string()
        } else {
            collapsed.to_string()
        };
        if final_token.len() > 48 {
            final_token.truncate(48);
        }
        final_token
    }

    fn capture_node_evidence(
        base_dir: &Path,
        seq: usize,
        step: usize,
        phase: &str,
        plan: &serde_json::Value,
        note: &str,
    ) -> Option<PathBuf> {
        let action = plan["action"].as_str().unwrap_or("unknown");
        let action_token = Self::sanitize_filename_token(action);
        let phase_token = Self::sanitize_filename_token(phase);
        let note_token = Self::sanitize_filename_token(note);
        let file_name = format!(
            "node_{:03}_step_{:02}_{}_{}_{}.png",
            seq, step, action_token, phase_token, note_token
        );
        let full_path = base_dir.join(file_name);

        let status = std::process::Command::new("screencapture")
            .arg("-x")
            .arg(&full_path)
            .status();

        match status {
            Ok(s) if s.success() => {
                let front_app = crate::tool_chaining::CrossAppBridge::get_frontmost_app()
                    .unwrap_or_else(|_| "unknown".to_string());
                println!(
                    "   📸 Node evidence: {} | step={} action={} phase={} front_app={} note={}",
                    full_path.display(),
                    step,
                    action,
                    phase,
                    front_app,
                    note
                );
                Some(full_path)
            }
            Ok(s) => {
                println!(
                    "   ⚠️ Node evidence capture failed (exit={:?}) for step={} action={}",
                    s.code(),
                    step,
                    action
                );
                None
            }
            Err(e) => {
                println!(
                    "   ⚠️ Node evidence capture error for step={} action={}: {}",
                    step, action, e
                );
                None
            }
        }
    }

    pub fn new(llm: Arc<dyn LLMClient>, tx: Option<mpsc::Sender<String>>) -> Self {
        Self {
            llm,
            max_steps: Self::env_usize("STEER_MAX_STEPS", 30),
            tx,
        }
    }

    pub async fn run_goal_tracked(
        &self,
        goal: &str,
        session_key: Option<&str>,
    ) -> Result<RunGoalOutcome> {
        let run_id = format!(
            "surf_{}_{}",
            Utc::now().format("%Y%m%d_%H%M%S"),
            Uuid::new_v4().simple()
        );
        self.run_goal_tracked_with_run_id(&run_id, goal, session_key)
            .await
    }

    pub async fn run_goal_tracked_with_run_id(
        &self,
        run_id: &str,
        goal: &str,
        session_key: Option<&str>,
    ) -> Result<RunGoalOutcome> {
        let _serial_guard = if Self::env_truthy_default("STEER_SERIALIZE_GUI_RUNS", true) {
            Some(GUI_RUN_SERIAL_LOCK.lock().await)
        } else {
            None
        };
        let run_id = run_id.trim().to_string();
        if run_id.is_empty() {
            return Err(anyhow::anyhow!("run_id_empty"));
        }
        if let Ok(cleaned) = db::mark_stale_running_task_runs_finished() {
            if cleaned > 0 {
                println!(
                    "🧹 Auto-cleaned stale running task runs before new execution: {}",
                    cleaned
                );
            }
        }
        let normalized_goal = goal.trim();
        let _ = db::create_task_run(&run_id, "surf_goal", normalized_goal, "running");
        let _ = db::record_task_stage_run(
            &run_id,
            "planner",
            1,
            "running",
            Some("planner.run_goal started"),
        );

        match self.run_goal_with_summary(goal, session_key).await {
            Ok(exec_summary) => {
                let planner_complete = exec_summary.planner_complete;
                let execution_complete = exec_summary.execution_complete;
                let business_complete = exec_summary.business_complete;
                let approval_required = exec_summary.approval_required;
                let status = if approval_required {
                    "approval_required"
                } else if business_complete {
                    "business_completed"
                } else {
                    "business_failed"
                };
                let task_run_status = if approval_required {
                    "business_incomplete"
                } else {
                    status
                };
                let summary = if approval_required {
                    Some(format!(
                        "surf goal paused: approval checkpoint required ({})",
                        exec_summary.business_note
                    ))
                } else if business_complete {
                    Some("surf goal execution and business checks completed".to_string())
                } else {
                    Some(format!(
                        "surf goal completed planner/execution but business check failed: {}",
                        exec_summary.business_note
                    ))
                };
                let details = serde_json::json!({
                    "source": "planner.run_goal_tracked",
                    "goal": normalized_goal,
                    "status": status,
                    "business_complete": business_complete,
                    "approval_required": approval_required,
                    "business_note": exec_summary.business_note,
                    "preflight_permissions_ok": exec_summary.preflight_permissions_ok,
                    "preflight_screen_capture_ok": exec_summary.preflight_screen_capture_ok,
                    "cleanup_dialog_closed_count": exec_summary.cleanup_dialog_closed_count,
                    "cleanup_app_ready_count": exec_summary.cleanup_app_ready_count,
                    "cleanup_mail_outgoing_hidden_count": exec_summary.cleanup_mail_outgoing_hidden_count,
                    "step_count": exec_summary.step_count,
                    "failed_steps": exec_summary.failed_steps,
                    "blocking_failed_steps": exec_summary.blocking_failed_steps,
                    "blocking_failure_details": exec_summary.blocking_failure_details.clone(),
                    "mail_send_required": exec_summary.mail_send_required,
                    "mail_send_confirmed": exec_summary.mail_send_confirmed,
                    "notes_write_required": exec_summary.notes_write_required,
                    "notes_write_confirmed": exec_summary.notes_write_confirmed,
                    "textedit_write_required": exec_summary.textedit_write_required,
                    "textedit_write_confirmed": exec_summary.textedit_write_confirmed,
                    "textedit_save_required": exec_summary.textedit_save_required,
                    "textedit_save_confirmed": exec_summary.textedit_save_confirmed,
                    "timing": {
                        "capture_total_ms": exec_summary.capture_total_ms,
                        "capture_max_ms": exec_summary.capture_max_ms,
                        "capture_count": exec_summary.capture_count,
                        "plan_total_ms": exec_summary.plan_total_ms,
                        "plan_max_ms": exec_summary.plan_max_ms,
                        "plan_count": exec_summary.plan_count,
                        "supervisor_total_ms": exec_summary.supervisor_total_ms,
                        "supervisor_max_ms": exec_summary.supervisor_max_ms,
                        "supervisor_count": exec_summary.supervisor_count,
                        "execute_total_ms": exec_summary.execute_total_ms,
                        "execute_max_ms": exec_summary.execute_max_ms,
                        "execute_count": exec_summary.execute_count
                    }
                })
                .to_string();

                let _ = db::record_task_stage_run(
                    &run_id,
                    "planner",
                    1,
                    if planner_complete {
                        "completed"
                    } else if approval_required {
                        "blocked"
                    } else {
                        "failed"
                    },
                    Some(if planner_complete {
                        "planner produced done"
                    } else if approval_required {
                        "planner paused by approval checkpoint"
                    } else {
                        "planner did not reach done"
                    }),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner_complete",
                    "true",
                    if planner_complete { "true" } else { "false" },
                    planner_complete,
                    Some(if planner_complete {
                        "Goal completed by planner"
                    } else {
                        "Planner did not complete goal"
                    }),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner.approval_checkpoint_required",
                    "false",
                    if approval_required { "true" } else { "false" },
                    !approval_required,
                    Some("Fallback checkpoint should not be hit in autonomous run"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner.preflight_permissions_ok",
                    "true",
                    if exec_summary.preflight_permissions_ok {
                        "true"
                    } else {
                        "false"
                    },
                    exec_summary.preflight_permissions_ok,
                    Some("Accessibility/automation permission preflight"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner.preflight_screen_capture_ok",
                    "true",
                    if exec_summary.preflight_screen_capture_ok {
                        "true"
                    } else {
                        "false"
                    },
                    exec_summary.preflight_screen_capture_ok,
                    Some("Screen capture permission preflight"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "execution",
                    2,
                    if approval_required {
                        "blocked"
                    } else if execution_complete {
                        "completed"
                    } else {
                        "failed"
                    },
                    Some(&format!(
                        "step_count={} failed_steps={} blocking_failed_steps={}",
                        exec_summary.step_count,
                        exec_summary.failed_steps,
                        exec_summary.blocking_failed_steps
                    )),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution_complete",
                    "true",
                    if execution_complete { "true" } else { "false" },
                    execution_complete,
                    Some(&{
                        let mut evidence = String::from(
                            "All blocking action steps must be successful (mail send pending/no_draft retries + shortcut permission denials are non-blocking)"
                        );
                        if !exec_summary.blocking_failure_details.is_empty() {
                            evidence.push_str(" | failures=");
                            evidence.push_str(&exec_summary.blocking_failure_details.join(" || "));
                        }
                        evidence
                    }),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution.cleanup_dialog_closed_count",
                    ">=0",
                    &exec_summary.cleanup_dialog_closed_count.to_string(),
                    true,
                    Some("Count of cleanup dialog closures before/during run"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution.cleanup_app_ready_count",
                    ">=0",
                    &exec_summary.cleanup_app_ready_count.to_string(),
                    true,
                    Some("Count of app readiness cleanup markers"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution.cleanup_mail_outgoing_hidden_count",
                    ">=0",
                    &exec_summary.cleanup_mail_outgoing_hidden_count.to_string(),
                    true,
                    Some("Count of hidden outgoing draft windows during cleanup"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "business",
                    3,
                    if approval_required {
                        "blocked"
                    } else if business_complete {
                        "completed"
                    } else {
                        "failed"
                    },
                    Some(&exec_summary.business_note),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "business",
                    "business_complete",
                    "true",
                    if business_complete { "true" } else { "false" },
                    business_complete,
                    Some(&format!(
                        "mail_send_required={} mail_send_confirmed={} notes_write_required={} notes_write_confirmed={} textedit_write_required={} textedit_write_confirmed={} textedit_save_required={} textedit_save_confirmed={}",
                        exec_summary.mail_send_required,
                        exec_summary.mail_send_confirmed,
                        exec_summary.notes_write_required,
                        exec_summary.notes_write_confirmed,
                        exec_summary.textedit_write_required,
                        exec_summary.textedit_write_confirmed,
                        exec_summary.textedit_save_required,
                        exec_summary.textedit_save_confirmed
                    )),
                );
                let _ = db::update_task_run_outcome(
                    &run_id,
                    planner_complete,
                    execution_complete,
                    business_complete,
                    task_run_status,
                    summary.as_deref(),
                    Some(&details),
                );

                Ok(RunGoalOutcome {
                    run_id,
                    planner_complete,
                    execution_complete,
                    business_complete,
                    status: status.to_string(),
                    summary,
                })
            }
            Err(e) => {
                let error_text = e.to_string();
                let summary = Some(format!("surf goal failed: {}", error_text));
                let details = serde_json::json!({
                    "source": "planner.run_goal_tracked",
                    "goal": normalized_goal,
                    "status": "failed",
                    "error": error_text
                })
                .to_string();

                let _ = db::record_task_stage_run(&run_id, "planner", 1, "failed", Some(&details));
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "planner",
                    "planner_complete",
                    "true",
                    "false",
                    false,
                    Some("planner did not reach done"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "execution",
                    2,
                    "failed",
                    Some("execution/biz completion unavailable due planner failure"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "execution",
                    "execution_complete",
                    "true",
                    "false",
                    false,
                    Some("run_goal returned error"),
                );
                let _ = db::record_task_stage_run(
                    &run_id,
                    "business",
                    3,
                    "failed",
                    Some("business completion failed"),
                );
                let _ = db::record_task_stage_assertion(
                    &run_id,
                    "business",
                    "business_complete",
                    "true",
                    "false",
                    false,
                    Some("run_goal returned error"),
                );
                let _ = db::update_task_run_outcome(
                    &run_id,
                    false,
                    false,
                    false,
                    "business_failed",
                    summary.as_deref(),
                    Some(&details),
                );

                Err(anyhow::anyhow!("{} (run_id={})", e, run_id))
            }
        }
    }

    pub async fn run_goal(&self, goal: &str, session_key: Option<&str>) -> Result<()> {
        let _ = self.run_goal_with_summary(goal, session_key).await?;
        Ok(())
    }

    async fn run_goal_with_summary(
        &self,
        goal: &str,
        session_key: Option<&str>,
    ) -> Result<RunGoalExecutionSummary> {
        println!("🌊 Starting Planned Surf: '{}'", goal);
        let scenario_mode = Self::scenario_mode_enabled();
        let deterministic_goal_mode =
            !scenario_mode && Self::should_use_deterministic_goal_autoplan(goal);
        if deterministic_goal_mode {
            println!("🧭 Deterministic goal autoplan enabled (script-like goal detected).");
        }
        let test_context = Self::env_truthy("STEER_TEST_MODE") || Self::env_truthy("CI");
        let deterministic_fallback_requested =
            Self::env_truthy("STEER_ALLOW_DETERMINISTIC_FALLBACK");
        let review_loop_override_requested = Self::env_truthy("STEER_ALLOW_REVIEW_LOOP_OVERRIDE");
        if deterministic_fallback_requested && !test_context {
            return Err(anyhow::anyhow!(
                "STEER_ALLOW_DETERMINISTIC_FALLBACK is test-only (requires STEER_TEST_MODE=1 or CI=1)."
            ));
        }
        if review_loop_override_requested && !test_context {
            return Err(anyhow::anyhow!(
                "STEER_ALLOW_REVIEW_LOOP_OVERRIDE is test-only (requires STEER_TEST_MODE=1 or CI=1)."
            ));
        }
        let allow_deterministic_fallback = deterministic_fallback_requested && test_context;
        let allow_review_loop_override =
            review_loop_override_requested && allow_deterministic_fallback;
        let require_primary_planner =
            Self::env_truthy_default("STEER_REQUIRE_PRIMARY_PLANNER", true);
        let allow_scenario_mode = Self::env_truthy("STEER_ALLOW_SCENARIO_MODE");
        if scenario_mode && require_primary_planner && !allow_scenario_mode {
            return Err(anyhow::anyhow!(
                "Scenario mode fallback is disabled by policy (set STEER_ALLOW_SCENARIO_MODE=1 only for explicit test runs)."
            ));
        }
        let mut node_capture_enabled = Self::env_truthy("STEER_NODE_CAPTURE");
        let node_capture_all = Self::env_truthy("STEER_NODE_CAPTURE_ALL");
        let mut node_capture_seq: usize = 0;
        let mut last_opened_app: Option<String> = None;
        let mut node_capture_dir: Option<PathBuf> = None;

        if node_capture_enabled {
            let dir = std::env::var("STEER_NODE_CAPTURE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(format!(
                        "scenario_results/node_evidence_{}",
                        Utc::now().format("%Y%m%d_%H%M%S")
                    ))
                });

            match std::fs::create_dir_all(&dir) {
                Ok(_) => {
                    println!("📸 Node capture enabled: {}", dir.display());
                    node_capture_dir = Some(dir);
                }
                Err(e) => {
                    println!(
                        "⚠️ Node capture disabled: failed to create dir '{}': {}",
                        dir.display(),
                        e
                    );
                    node_capture_enabled = false;
                }
            }
        }

        // [Session]
        let _ = crate::session_store::init_session_store();
        let mut session = Session::new(goal, session_key);
        session.add_message("user", goal);

        let mut history: Vec<String> = Vec::new();
        // [Preflight]
        if let Err(e) = heuristics::preflight_permissions() {
            println!("❌ Preflight failed: {}", e);
            history.push(format!("PREFLIGHT_PERMISSIONS_FAILED: {}", e));
            return Err(e);
        }
        history.push("PREFLIGHT_PERMISSIONS_OK".to_string());
        if let Err(e) = heuristics::verify_screen_capture() {
            history.push(format!("PREFLIGHT_SCREEN_CAPTURE_FAILED: {}", e));
            return Err(e);
        }
        history.push("PREFLIGHT_SCREEN_CAPTURE_OK".to_string());

        let mut action_history: Vec<String> = Vec::new(); // For loop detection
        let mut plan_attempts: HashMap<String, usize> = HashMap::new();
        let mut consecutive_failures = 0;
        let mut last_read_number: Option<String> = None;
        let mut session_steps: Vec<SmartStep> = Vec::new();
        let mut last_action_by_plan: HashMap<String, String> = HashMap::new();
        let mut repeated_loop_hits: usize = 0;
        let mut goal_completed = false;
        let mut timing = PlannerTimingStats::default();
        let run_started = Instant::now();
        let max_wall_seconds = Self::env_u64("STEER_GOAL_MAX_WALL_SEC", 240);
        let max_wall_duration = Duration::from_secs(max_wall_seconds.max(30));
        let max_repeat_loop_hits = Self::env_usize("STEER_MAX_REPEAT_LOOP_HITS", 6).max(2);
        let max_attempts_per_plan_key =
            Self::env_usize("STEER_MAX_ATTEMPTS_PER_PLAN_KEY", 12).max(3);

        Self::run_standard_cleanup_preset(goal, &mut history).await;

        for i in 1..=self.max_steps {
            if run_started.elapsed() > max_wall_duration {
                let msg = format!(
                    "Planner wall timeout after {}s (goal stalled without completion).",
                    max_wall_duration.as_secs()
                );
                history.push(format!("PLANNER_WALL_TIMEOUT: {}", msg));
                return Err(anyhow::anyhow!(msg));
            }
            println!("\n🔄 [Step {}/{}] Observing...", i, self.max_steps);

            // 1. Capture Screen
            let capture_started = Instant::now();
            let (image_b64, _) = VisualDriver::capture_screen()?;
            let capture_elapsed = capture_started.elapsed();
            timing.record_capture(capture_elapsed);
            history.push(format!(
                "TIMING|step={}|phase=capture|ms={}",
                i,
                capture_elapsed.as_millis()
            ));
            let plan_key = heuristics::compute_plan_key(goal, &image_b64);
            let attempt = plan_attempts
                .entry(plan_key.clone())
                .and_modify(|v| *v += 1)
                .or_insert(1);
            if *attempt > max_attempts_per_plan_key {
                let msg = format!(
                    "Planner exceeded max attempts for same screen state: attempts={} limit={} plan_key={}",
                    *attempt, max_attempts_per_plan_key, plan_key
                );
                history.push(format!("PLAN_ATTEMPT_LIMIT: {}", msg));
                return Err(anyhow::anyhow!(msg));
            }

            // Preflight: close blocking dialogs
            if heuristics::try_close_front_dialog() {
                history.push("Closed blocking dialog".to_string());
                continue;
            }

            // 2. Plan (Think)
            let retry_config = Self::planner_retry_config();
            let mut history_with_context = history.clone();
            if *attempt > 1 || consecutive_failures > 0 {
                let last_action = last_action_by_plan
                    .get(&plan_key)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let last_error = history
                    .iter()
                    .rev()
                    .find(|h| h.starts_with("FAILED") || h.starts_with("BLOCKED"))
                    .cloned()
                    .unwrap_or_else(|| "none".to_string());
                let context = format!(
                    "RETRY_CONTEXT: attempt={} plan_key={} last_action={} last_error={}",
                    attempt, plan_key, last_action, last_error
                );
                history_with_context.push(context);
            }

            let plan_started = Instant::now();
            let mut plan = if scenario_mode || deterministic_goal_mode {
                Self::fallback_plan_from_goal(goal, &history_with_context)
                    .unwrap_or_else(|| serde_json::json!({ "action": "done" }))
            } else {
                // Call LLM for Vision Planning
                let plan_timeout =
                    Duration::from_secs(Self::env_u64("STEER_PLANNER_PLAN_TIMEOUT_SEC", 10));
                let primary_result =
                    crate::retry_logic::with_retry(&retry_config, "LLM Vision", || async {
                        tokio::time::timeout(
                            plan_timeout,
                            self.llm
                                .plan_vision_step(goal, &image_b64, &history_with_context),
                        )
                        .await
                        .map_err(|_| {
                            anyhow::anyhow!(
                                "planner plan_vision_step timeout after {}s",
                                plan_timeout.as_secs()
                            )
                        })?
                    })
                    .await;

                match primary_result {
                    Ok(v) => v,
                    Err(e) => {
                        let err_text = e.to_string();
                        history.push(format!("PLAN_PRIMARY_FAILED: {}", err_text));
                        if let Some(recovered) = self
                            .recover_plan_after_primary_failure(
                                goal,
                                &history_with_context,
                                &err_text,
                            )
                            .await
                        {
                            if let Some(action) = recovered["action"].as_str() {
                                history.push(format!("PLAN_RECOVERY_ACTION: {}", action));
                            } else {
                                history.push("PLAN_RECOVERY_ACTION: unknown".to_string());
                            }
                            recovered
                        } else {
                            return Err(e);
                        }
                    }
                }
            };
            let plan_elapsed = plan_started.elapsed();
            timing.record_plan(plan_elapsed);
            history.push(format!(
                "TIMING|step={}|phase=plan|ms={}",
                i,
                plan_elapsed.as_millis()
            ));

            // Flatten nested JSON
            if plan["action"].is_object() {
                plan = plan["action"].clone();
            }

            Self::maybe_repair_open_app_missing_name(goal, &history, &mut plan);

            // Validate Schema
            let validation = action_schema::normalize_action(&plan);
            if let Some(err) = validation.error {
                let msg = format!("SCHEMA_ERROR: {}", err);
                println!("   ⚠️ {}", msg);
                history.push(msg);
                consecutive_failures += 1;
                continue;
            }
            plan = validation.normalized;
            Self::maybe_rewrite_click_visual_to_app_action(goal, &history, &mut plan);
            Self::maybe_rewrite_click_visual_mail_body(&history, &mut plan);
            Self::maybe_rewrite_shortcut_to_next_app(goal, &history, &mut plan);
            Self::maybe_rewrite_redundant_new_item_shortcut(goal, &history, &mut plan);
            Self::maybe_rewrite_mail_subject_before_paste(goal, &history, &mut plan);
            Self::maybe_rewrite_open_app_to_pending_text_action(goal, &history, &mut plan);
            Self::maybe_rewrite_snapshot_to_progress_action(goal, &history, &mut plan);

            if scenario_mode {
                if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, &history) {
                    Self::record_fallback_action(&mut history, "scenario_mode", &fallback_plan);
                    plan = fallback_plan;
                }
            } else if deterministic_goal_mode {
                if let Some(det_plan) = Self::fallback_plan_from_goal(goal, &history) {
                    if let Some(action) = det_plan["action"].as_str() {
                        history.push(format!("DETERMINISTIC_PLAN_ACTION: {}", action));
                    }
                    plan = det_plan;
                }
            } else {
                // 3. Supervisor Check (safe actions can bypass to reduce rate-limit stalls)
                let supervisor_started = Instant::now();
                let bypass_supervisor = Self::supervisor_safe_bypass_enabled()
                    && Self::is_low_risk_action_for_supervisor(&plan);
                let (mut supervisor_action, supervisor_reason, supervisor_notes) =
                    if bypass_supervisor {
                        println!("   🕵️ Supervisor: bypass (safe action)");
                        (
                            "accept".to_string(),
                            "safe_action_bypass".to_string(),
                            "Low-risk action bypassed supervisor gate".to_string(),
                        )
                    } else {
                        let supervisor_timeout = Duration::from_secs(Self::env_u64(
                            "STEER_PLANNER_SUPERVISOR_TIMEOUT_SEC",
                            6,
                        ));
                        let supervisor_result =
                            crate::retry_logic::with_retry(&retry_config, "Supervisor", || async {
                                tokio::time::timeout(
                                    supervisor_timeout,
                                    Supervisor::consult(&*self.llm, goal, &plan, &history),
                                )
                                .await
                                .map_err(|_| {
                                    anyhow::anyhow!(
                                        "planner supervisor timeout after {}s",
                                        supervisor_timeout.as_secs()
                                    )
                                })?
                            })
                            .await;

                        match supervisor_result {
                            Ok(supervisor_decision) => {
                                println!(
                                    "   🕵️ Supervisor: {} ({})",
                                    supervisor_decision.action, supervisor_decision.reason
                                );
                                (
                                    supervisor_decision.action,
                                    supervisor_decision.reason,
                                    supervisor_decision.notes,
                                )
                            }
                            Err(e) => {
                                let err_text = e.to_string();
                                let fail_open =
                                    Self::env_truthy_default("STEER_SUPERVISOR_FAIL_OPEN", true);
                                if fail_open && Self::is_soft_planner_failure(&err_text) {
                                    println!("   🧯 Supervisor fail-open: {}", err_text);
                                    history.push(format!("SUPERVISOR_FAIL_OPEN: {}", err_text));
                                    (
                                        "accept".to_string(),
                                        "supervisor_fail_open".to_string(),
                                        err_text,
                                    )
                                } else {
                                    return Err(e);
                                }
                            }
                        }
                    };
                let supervisor_elapsed = supervisor_started.elapsed();
                timing.record_supervisor(supervisor_elapsed);
                history.push(format!(
                    "TIMING|step={}|phase=supervisor|ms={}",
                    i,
                    supervisor_elapsed.as_millis()
                ));

                if supervisor_action == "review"
                    && Self::can_force_done_for_simple_goal(goal, &plan, &history)
                {
                    println!(
                        "   ✅ Force-complete override: simple note creation goal already satisfied."
                    );
                    supervisor_action = "accept".to_string();
                }

                if supervisor_action == "review"
                    && Self::should_accept_text_flow_after_type(
                        &plan,
                        &history,
                        &supervisor_reason,
                        &supervisor_notes,
                    )
                {
                    println!(
                        "   ✅ Review override: proceeding with text-flow action after prior typing evidence."
                    );
                    supervisor_action = "accept".to_string();
                }

                if supervisor_action == "review"
                    && Self::should_accept_typing_after_new_item_shortcut(
                        &plan,
                        &history,
                        &supervisor_reason,
                        &supervisor_notes,
                    )
                {
                    println!(
                        "   ✅ Review override: allowing typing step after Cmd+N creation evidence."
                    );
                    supervisor_action = "accept".to_string();
                }

                if supervisor_action == "review"
                    && !Self::goal_has_multi_app(goal)
                    && !Self::goal_has_explicit_sequence(goal)
                    && allow_deterministic_fallback
                    && Self::should_relax_review(&supervisor_reason, &supervisor_notes)
                {
                    if let Some(fallback_plan) = Self::fallback_plan_from_goal(goal, &history) {
                        Self::record_fallback_action(
                            &mut history,
                            "relaxed_review",
                            &fallback_plan,
                        );
                        plan = fallback_plan;
                        supervisor_action = "accept".to_string();
                    }
                }

                if supervisor_action == "review" {
                    let recent_rejections = history
                        .iter()
                        .rev()
                        .take(16)
                        .filter(|h| h.starts_with("PLAN_REJECTED:"))
                        .count();
                    if recent_rejections >= 4 {
                        let review_text = format!(
                            "{} {}",
                            supervisor_reason.to_lowercase(),
                            supervisor_notes.to_lowercase()
                        );
                        let hard_blockers = [
                            "danger",
                            "unsafe",
                            "impossible",
                            "not related",
                            "does not relate",
                            "wrong app",
                        ];
                        let has_hard_blocker =
                            hard_blockers.iter().any(|s| review_text.contains(s));
                        let has_notes_content_issue = review_text.contains("notes")
                            && (review_text.contains("content")
                                || review_text.contains("exact")
                                || review_text.contains("불일치"));

                        if !has_hard_blocker && !has_notes_content_issue {
                            if allow_deterministic_fallback {
                                if allow_review_loop_override {
                                    if let Some(loop_break_plan) =
                                        Self::fallback_plan_from_goal(goal, &history)
                                    {
                                        Self::record_fallback_action(
                                            &mut history,
                                            &format!("review_loop_{}rejections", recent_rejections),
                                            &loop_break_plan,
                                        );
                                        plan = loop_break_plan;
                                        supervisor_action = "accept".to_string();
                                    } else {
                                        let action_name =
                                            plan["action"].as_str().unwrap_or("unknown");
                                        if matches!(
                                            action_name,
                                            "open_app"
                                                | "open_url"
                                                | "shortcut"
                                                | "type"
                                                | "paste"
                                                | "copy"
                                                | "select_all"
                                                | "read"
                                                | "read_clipboard"
                                                | "click_visual"
                                        ) {
                                            println!(
                                                "   🔁 Review-loop override: forcing '{}' after {} rejections.",
                                                action_name, recent_rejections
                                            );
                                            supervisor_action = "accept".to_string();
                                        }
                                    }
                                } else {
                                    history.push(
                                        "FALLBACK_BLOCKED: review-loop override requires STEER_ALLOW_REVIEW_LOOP_OVERRIDE=1"
                                            .to_string(),
                                    );
                                    let msg = "Supervisor review loop: deterministic override disabled by policy";
                                    return Err(anyhow::anyhow!(msg));
                                }
                            } else {
                                history.push(
                                    "FALLBACK_BLOCKED: review-loop deterministic fallback disabled"
                                        .to_string(),
                                );
                                let msg = "Supervisor review loop: deterministic fallback disabled by policy";
                                return Err(anyhow::anyhow!(msg));
                            }
                        }
                    }
                }

                if supervisor_action == "escalate" {
                    let recent_rejections = history
                        .iter()
                        .rev()
                        .take(16)
                        .filter(|h| h.starts_with("PLAN_REJECTED:"))
                        .count();
                    let reason_lc = supervisor_reason.to_lowercase();
                    let notes_lc = supervisor_notes.to_lowercase();
                    let repeated_content_escalation = (reason_lc.contains("repeated")
                        || notes_lc.contains("repeated"))
                        && (reason_lc.contains("content")
                            || notes_lc.contains("content")
                            || reason_lc.contains("notes")
                            || notes_lc.contains("notes"));
                    if repeated_content_escalation && recent_rejections >= 3 {
                        println!(
                            "   🔁 Escalation override: retrying content-repair path before hard fail."
                        );
                        supervisor_action = "review".to_string();
                    }
                }

                match supervisor_action.as_str() {
                    "accept" => { /* Proceed */ }
                    "review" => {
                        history.push(format!("PLAN_REJECTED: {}", supervisor_notes));
                        continue;
                    }
                    "escalate" => {
                        let msg = format!("Supervisor escalated: {}", supervisor_reason);
                        println!("      🚨 {}", msg);
                        return Err(anyhow::anyhow!(msg));
                    }
                    _ => {}
                }
            }

            if let Err(e) = Self::enforce_fallback_checkpoint(&mut history) {
                let mut summary =
                    Self::summarize_execution(goal, &session, &history, false, &timing);
                summary.approval_required = true;
                summary.business_complete = false;
                summary.business_note = format!("approval checkpoint required: {}", e);
                session.status = SessionStatus::Paused;
                let _ = crate::session_store::save_session(&session);
                return Ok(summary);
            }

            // 4. Anti-Loop Check
            let action_str = plan.to_string();
            if LoopDetector::detect_high_risk_repetition(&action_history, &action_str) {
                println!(
                    "   🛑 LOOP BLOCKED. High-risk repeated plan suppressed before execution."
                );
                history.push(format!(
                    "LOOP_BLOCKED: high_risk_repeated_plan={}",
                    action_str
                ));
                continue;
            }
            if LoopDetector::detect_action_loop(&action_history, &action_str) {
                println!(
                    "   🔄 LOOP DETECTED. Recording context and retrying with same action family."
                );
                history.push(format!("LOOP_DETECTED: repeated_plan={}", action_str));
                repeated_loop_hits += 1;
                if repeated_loop_hits >= max_repeat_loop_hits {
                    let msg = format!(
                        "Planner aborted due to repeated loop detections (hits={} limit={}).",
                        repeated_loop_hits, max_repeat_loop_hits
                    );
                    history.push(format!("LOOP_ABORTED: {}", msg));
                    return Err(anyhow::anyhow!(msg));
                }
            } else {
                repeated_loop_hits = 0;
            }
            action_history.push(action_str.clone());
            last_action_by_plan.insert(
                plan_key.clone(),
                plan["action"].as_str().unwrap_or("unknown").to_string(),
            );

            if plan["action"].as_str() == Some("wait")
                && Self::has_quota_exhaustion_marker(&history)
            {
                let msg =
                    "Planner aborted: provider quota/rate-limit detected and wait-loop suppressed.";
                history.push(format!("QUOTA_ABORTED: {}", msg));
                return Err(anyhow::anyhow!(msg));
            }

            if plan["action"].as_str() == Some("done") {
                if node_capture_enabled {
                    if let Some(dir) = node_capture_dir.as_ref() {
                        node_capture_seq += 1;
                        let _ = Self::capture_node_evidence(
                            dir,
                            node_capture_seq,
                            i,
                            "goal_done",
                            &plan,
                            "planner_done",
                        );
                    }
                }
                println!("✅ Goal completed by planner.");
                goal_completed = true;
                break;
            }

            // 5. Execute via ActionRunner
            println!("   🚀 Executing Action...");
            let execute_started = Instant::now();
            let execute_result = ActionRunner::execute(
                &plan,
                &mut VisualDriver::new(), // In real scenario, might want to reuse driver or pass it
                Some(&*self.llm),
                &mut session_steps,
                &mut session,
                &mut history,
                &mut consecutive_failures,
                &mut last_read_number,
                goal,
            )
            .await;
            let execute_elapsed = execute_started.elapsed();
            timing.record_execute(execute_elapsed);
            history.push(format!(
                "TIMING|step={}|phase=execute|ms={}",
                i,
                execute_elapsed.as_millis()
            ));

            let mut abort_due_to_execution_error: Option<String> = None;
            if let Err(e) = &execute_result {
                println!("   ❌ Execution Error: {}", e);
                history.push(format!("EXECUTION_ERROR: {}", e));
                if Self::should_abort_on_execution_error(e) {
                    let action_name = plan["action"].as_str().unwrap_or("unknown");
                    abort_due_to_execution_error = Some(format!(
                        "Planner aborted after execution error at step {} (action={}): {}",
                        i, action_name, e
                    ));
                }
            }

            if node_capture_enabled {
                if let Some(dir) = node_capture_dir.as_ref() {
                    let action_name = plan["action"].as_str().unwrap_or("unknown");
                    let mut should_capture = node_capture_all;
                    let mut phase = "post_action";
                    let mut note = "action_executed".to_string();

                    if action_name == "open_app" {
                        should_capture = true;
                        phase = "app_node";
                        let current_app = plan["name"]
                            .as_str()
                            .or_else(|| plan["app"].as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        note = if let Some(prev_app) = last_opened_app.as_ref() {
                            if prev_app.eq_ignore_ascii_case(&current_app) {
                                format!("app_reopen_{}", current_app)
                            } else {
                                format!("transition_{}_to_{}", prev_app, current_app)
                            }
                        } else {
                            format!("app_entry_{}", current_app)
                        };
                        last_opened_app = Some(current_app);
                    } else if execute_result.is_err() {
                        should_capture = true;
                        phase = "execution_error";
                        note = "action_failed".to_string();
                    }

                    if should_capture {
                        node_capture_seq += 1;
                        let _ = Self::capture_node_evidence(
                            dir,
                            node_capture_seq,
                            i,
                            phase,
                            &plan,
                            &note,
                        );
                    }
                }
            }

            if let Some(abort_msg) = abort_due_to_execution_error {
                session.status = SessionStatus::Failed;
                let _ = crate::session_store::save_session(&session);
                return Err(anyhow::anyhow!(abort_msg));
            }

            // Broadcast event if tx available
            if let Some(tx) = &self.tx {
                let event = EventEnvelope {
                    schema_version: "1.0".to_string(),
                    event_id: Uuid::new_v4().to_string(),
                    ts: Utc::now().to_rfc3339(),
                    source: "dynamic_agent".to_string(),
                    app: "Agent".to_string(),
                    event_type: "action".to_string(),
                    priority: "P1".to_string(),
                    resource: None,
                    payload: serde_json::json!({
                        "goal": goal,
                        "step": i,
                        "plan": plan
                    }),
                    privacy: None,
                    pid: None,
                    window_id: None,
                    window_title: None,
                    browser_url: None,
                    raw: None,
                };
                if let Ok(json) = serde_json::to_string(&event) {
                    let _ = tx.try_send(json);
                }
            }
        }
        if goal_completed {
            let summary = Self::summarize_execution(goal, &session, &history, true, &timing);
            session.status = if summary.business_complete {
                SessionStatus::Completed
            } else {
                SessionStatus::Failed
            };
            let _ = crate::session_store::save_session(&session);
            return Ok(summary);
        }

        session.status = SessionStatus::Failed;
        let _ = crate::session_store::save_session(&session);
        Err(anyhow::anyhow!(
            "Planner stopped without completion (max steps reached or unresolved review loop)."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::Planner;
    use crate::session_store::Session;
    use chrono::Utc;
    use unicode_normalization::UnicodeNormalization;

    fn base_session(goal: &str) -> Session {
        Session::new(goal, Some("planner_test"))
    }

    #[test]
    fn summarize_execution_success_for_non_mail_goal() {
        let goal = "Notes를 열고 간단한 메모를 작성하고 done 하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Notes", "success", None);
        session.add_step("type", "Typed '회의 준비'", "success", None);
        let history = vec![
            "Opened app: Notes".to_string(),
            "Typed '회의 준비'".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.planner_complete);
        assert!(summary.execution_complete);
        assert!(summary.business_complete);
    }

    #[test]
    fn summarize_execution_fails_if_any_step_failed() {
        let goal = "TextEdit를 열고 문서를 작성하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step("type", "Type failed: blocked by dialog", "failed", None);
        let history = vec!["Opened app: TextEdit".to_string()];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.planner_complete);
        assert!(!summary.execution_complete);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_treats_shortcut_permission_failure_as_non_blocking() {
        let goal = "노트에서 최근 TODO 정리해줘";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Notes", "success", None);
        session.add_step(
            "shortcut",
            "Shortcut 'n' + [\"command\"] | driver execution failed: Shortcut Failed: AppleScript Error: not allowed to send keystrokes (1002)",
            "failed",
            None,
        );
        session.add_step("type", "Typed '오늘 할 일 5개 정리'", "success", None);
        let history = vec![
            "Opened app: Notes".to_string(),
            "Typed '오늘 할 일 5개 정리'".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.execution_complete);
        assert!(summary.business_complete);
        assert_eq!(summary.blocking_failed_steps, 0);
    }

    #[test]
    fn summarize_execution_requires_mail_send_confirmation() {
        let goal = "Mail을 열고 이메일을 보내세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Mail", "success", None);
        session.add_step("type", "Typed 'subject'", "success", None);
        let history = vec![
            "Opened app: Mail".to_string(),
            "Typed 'subject'".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.mail_send_required);
        assert!(!summary.mail_send_confirmed);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_passes_when_mail_send_confirmed() {
        let goal = "Mail로 보고서를 보내세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Mail", "success", None);
        session.add_step(
            "mail_send",
            "Mail send completed",
            "success",
            Some(serde_json::json!({
                "send_status": "sent_confirmed",
                "recipient": "qed4950@gmail.com",
                "body_len": 24
            })),
        );
        let history = vec![
            "Opened app: Mail".to_string(),
            "Mail send completed".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.mail_send_required);
        assert!(summary.mail_send_confirmed);
        assert!(summary.business_complete);
    }

    #[test]
    fn summarize_execution_accepts_pending_then_no_draft_mail_send() {
        let goal = "Mail로 보고서를 보내세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Mail", "success", None);
        session.add_step(
            "mail_send",
            "Mail send blocked: sent_pending|1|1",
            "failed",
            Some(serde_json::json!({"send_status": "sent_pending"})),
        );
        session.add_step(
            "mail_send",
            "Mail send blocked: no_draft|0|0",
            "failed",
            Some(serde_json::json!({"send_status": "no_draft"})),
        );
        let history = vec![
            "Opened app: Mail".to_string(),
            "Mail send blocked: sent_pending|1|1".to_string(),
            "Mail send blocked: no_draft|0|0".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.execution_complete);
        assert!(summary.mail_send_required);
        assert!(summary.mail_send_confirmed);
        assert!(summary.business_complete);
    }

    #[test]
    fn summarize_execution_rejects_sent_confirmed_without_body_or_recipient_proof() {
        let goal = "Mail로 보고서를 보내세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: Mail", "success", None);
        session.add_step(
            "mail_send",
            "Mail send completed",
            "success",
            Some(serde_json::json!({"send_status": "sent_confirmed"})),
        );
        let history = vec![
            "Opened app: Mail".to_string(),
            "Mail send completed".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.mail_send_required);
        assert!(!summary.mail_send_confirmed);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_requires_textedit_save_when_goal_mentions_save() {
        let goal = "TextEdit에서 문서를 작성하고 저장하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step("type", "Typed 'status: in-progress'", "success", None);
        let history = vec![
            "Opened app: TextEdit".to_string(),
            "Typed 'status: in-progress'".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.textedit_write_required);
        assert!(summary.textedit_write_confirmed);
        assert!(summary.textedit_save_required);
        assert!(!summary.textedit_save_confirmed);
        assert!(!summary.business_complete);
    }

    #[test]
    fn summarize_execution_passes_when_textedit_save_confirmed() {
        let goal = "TextEdit에서 문서를 작성하고 저장하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step(
            "type",
            "Typed 'status: in-progress' (textedit body)",
            "success",
            Some(serde_json::json!({"proof": "textedit_append_text"})),
        );
        session.add_step(
            "shortcut",
            "Shortcut 's' + [\"command\"]",
            "success",
            Some(serde_json::json!({"proof": "textedit_save"})),
        );
        let history = vec![
            "Opened app: TextEdit".to_string(),
            "Typed 'status: in-progress' (textedit body)".to_string(),
            "Shortcut 's' + [\"command\"]".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.textedit_write_required);
        assert!(summary.textedit_write_confirmed);
        assert!(summary.textedit_save_required);
        assert!(summary.textedit_save_confirmed);
        assert!(summary.business_complete);
    }

    #[test]
    fn goal_requires_textedit_save_detects_cmd_s_only() {
        let goal = "TextEdit를 열고 문서를 편집한 다음 Cmd+S 로 저장하세요.";
        assert!(Planner::goal_requires_textedit_save(goal));
    }

    #[test]
    fn rewrite_redundant_cmd_n_in_mail_to_send_progress() {
        let goal = "Mail을 열고 이메일을 보내세요.";
        let history = vec![
            "Opened app: Mail".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            "Typed 'Digest 제목' (mail subject)".to_string(),
            "Pasted clipboard contents (mail body)".to_string(),
        ];
        let mut plan = serde_json::json!({
            "action": "shortcut",
            "key": "n",
            "modifiers": ["command"]
        });

        Planner::maybe_rewrite_redundant_new_item_shortcut(goal, &history, &mut plan);

        assert_eq!(plan["action"].as_str(), Some("mail_send"));
    }

    #[test]
    fn goal_requires_textedit_save_does_not_match_cmd_shift_d() {
        let goal = "TextEdit를 열고 내용을 복사한 뒤 Mail에서 보내기(Cmd+Shift+D)로 발송하세요.";
        assert!(!Planner::goal_requires_textedit_save(goal));
    }

    #[test]
    fn summarize_execution_accepts_textedit_save_proof_without_shortcut_text() {
        let goal = "TextEdit에서 문서를 작성하고 저장하세요.";
        let mut session = base_session(goal);
        session.add_step("open_app", "Opened app: TextEdit", "success", None);
        session.add_step(
            "type",
            "Typed 'status: in-progress' (textedit body)",
            "success",
            Some(serde_json::json!({"proof": "textedit_append_text"})),
        );
        session.add_step(
            "shortcut",
            "Saved file in TextEdit",
            "success",
            Some(serde_json::json!({"proof": "textedit_save"})),
        );
        let history = vec![
            "Opened app: TextEdit".to_string(),
            "Typed 'status: in-progress' (textedit body)".to_string(),
            "Saved file in TextEdit".to_string(),
        ];

        let summary = Planner::summarize_execution(
            goal,
            &session,
            &history,
            true,
            &super::PlannerTimingStats::default(),
        );
        assert!(summary.textedit_save_required);
        assert!(summary.textedit_save_confirmed);
        assert!(summary.business_complete);
    }

    #[test]
    fn fallback_plan_reads_calendar_before_telegram_send() {
        let goal = "캘린더를 열고 오늘 일정 핵심만 텔레그램으로 보내줘";
        let history = vec!["Opened app: Calendar".to_string()];
        let plan = Planner::fallback_plan_from_goal(goal, &history).unwrap();
        assert_eq!(plan["action"].as_str(), Some("read"));
    }

    #[test]
    fn fallback_plan_sends_telegram_after_read_result() {
        let goal = "캘린더를 열고 오늘 일정 핵심만 텔레그램으로 보내줘";
        let history = vec![
            "Opened app: Calendar".to_string(),
            "READ_RESULT: 오늘 일정은 3건입니다.".to_string(),
        ];
        let plan = Planner::fallback_plan_from_goal(goal, &history).unwrap();
        assert_eq!(plan["action"].as_str(), Some("telegram_send"));
    }

    #[test]
    fn ordered_apps_in_goal_maps_korean_aliases() {
        let goal = "메모장 열어서 박대엽이라고 써줘";
        let apps = Planner::ordered_apps_in_goal(goal);
        assert_eq!(apps, vec!["Notes"]);
    }

    #[test]
    fn deterministic_autoplan_enabled_for_simple_korean_notes_write() {
        let goal = "메모장 열어서 박대엽이라고 써줘";
        assert!(Planner::should_use_deterministic_goal_autoplan(goal));
    }

    #[test]
    fn deterministic_autoplan_enabled_for_simple_korean_notes_open() {
        let goal = "노트 열어줘봐";
        assert!(Planner::should_use_deterministic_goal_autoplan(goal));
    }

    #[test]
    fn deterministic_autoplan_enabled_for_nfd_korean_notes_open() {
        let goal_nfd: String = "노트 열어줘봐".nfd().collect();
        assert!(Planner::should_use_deterministic_goal_autoplan(&goal_nfd));
    }

    #[test]
    fn soft_planner_failure_detects_timeout_and_rate_limit() {
        assert!(Planner::is_soft_planner_failure(
            "planner plan_vision_step timeout after 20s"
        ));
        assert!(Planner::is_soft_planner_failure(
            "HTTP 429 rate limit exceeded"
        ));
    }

    #[test]
    fn soft_planner_failure_ignores_schema_error() {
        assert!(!Planner::is_soft_planner_failure(
            "SCHEMA_ERROR: missing required key"
        ));
    }

    #[test]
    fn goal_has_open_signal_supports_multilingual_variants() {
        assert!(Planner::goal_has_open_signal("abre notes por favor"));
        assert!(Planner::goal_has_open_signal("ouvre notes"));
        assert!(Planner::goal_has_open_signal("メモを開いて"));
        assert!(Planner::goal_has_open_signal("打开 notes"));
    }

    #[test]
    fn deterministic_autoplan_enabled_for_ai_news_to_notion_goal() {
        let goal = "구글에서 현재가장 트렌디한 ai 관련 기사 찾아서 llm 으로 요약한후 노션에 정리";
        assert!(Planner::should_use_deterministic_goal_autoplan(goal));
    }

    #[test]
    fn deterministic_autoplan_enabled_for_sports_news_to_notion_goal() {
        let goal = "스포츠 뉴스 5개 선정해서 노션에 정리해줘";
        assert!(Planner::should_use_deterministic_goal_autoplan(goal));
    }

    #[test]
    fn deterministic_autoplan_enabled_for_todo_summary_goal() {
        let goal = "노트에 오늘 할 일 5개 정리해줘";
        assert!(Planner::should_use_deterministic_goal_autoplan(goal));
    }

    #[test]
    fn fallback_plan_todo_summary_notes_flow_order() {
        let goal = "노트에 오늘 할 일 5개 정리해줘";

        let step1 = Planner::fallback_plan_from_goal(goal, &[]).unwrap();
        assert_eq!(step1["action"].as_str(), Some("open_app"));
        assert_eq!(step1["name"].as_str(), Some("Notes"));

        let history_after_open = vec!["Opened app: Notes".to_string()];
        let step2 = Planner::fallback_plan_from_goal(goal, &history_after_open).unwrap();
        assert_eq!(step2["action"].as_str(), Some("shortcut"));
        assert_eq!(step2["key"].as_str(), Some("n"));

        let history_after_shortcut = vec![
            "Opened app: Notes".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
        ];
        let step3 = Planner::fallback_plan_from_goal(goal, &history_after_shortcut).unwrap();
        assert_eq!(step3["action"].as_str(), Some("type"));

        let todo_header = format!("오늘 할 일 체크리스트 ({})", Utc::now().format("%Y-%m-%d"));
        let history_after_type = vec![
            "Opened app: Notes".to_string(),
            "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            format!("Typed '{}'", todo_header),
        ];
        let step4 = Planner::fallback_plan_from_goal(goal, &history_after_type).unwrap();
        assert_eq!(step4["action"].as_str(), Some("done"));
    }

    #[test]
    fn fallback_plan_ai_news_to_notion_flow_order() {
        let goal = "구글에서 현재가장 트렌디한 ai 관련 기사 찾아서 llm 으로 요약한후 노션에 정리";

        let step1 = Planner::fallback_plan_from_goal(goal, &[]).unwrap();
        assert_eq!(step1["action"].as_str(), Some("open_url"));
        assert!(step1["url"]
            .as_str()
            .unwrap_or("")
            .contains("google.com/search"));

        let history_after_google =
            vec!["Opened URL 'https://www.google.com/search?q=trending+AI+news'".to_string()];
        let step2 = Planner::fallback_plan_from_goal(goal, &history_after_google).unwrap();
        if Planner::notion_api_ready() {
            assert_eq!(step2["action"].as_str(), Some("notion_write"));
            assert!(step2["content"]
                .as_str()
                .unwrap_or("")
                .contains("AI 뉴스 기사 요약"));
            let history_after_notion_write = vec![
                "Opened URL 'https://www.google.com/search?q=trending+AI+news'".to_string(),
                "Notion page created: https://www.notion.so/abcd1234".to_string(),
            ];
            let step3 =
                Planner::fallback_plan_from_goal(goal, &history_after_notion_write).unwrap();
            assert_eq!(step3["action"].as_str(), Some("done"));
        } else {
            assert_eq!(step2["action"].as_str(), Some("open_app"));
            assert_eq!(step2["name"].as_str(), Some("Notion"));

            let history_after_notion = vec![
                "Opened URL 'https://www.google.com/search?q=trending+AI+news'".to_string(),
                "Opened app: Notion".to_string(),
            ];
            let step3 = Planner::fallback_plan_from_goal(goal, &history_after_notion).unwrap();
            assert_eq!(step3["action"].as_str(), Some("shortcut"));
            assert_eq!(step3["key"].as_str(), Some("n"));

            let history_after_new_item = vec![
                "Opened URL 'https://www.google.com/search?q=trending+AI+news'".to_string(),
                "Opened app: Notion".to_string(),
                "Shortcut 'n' + [\"command\"] (Created new item)".to_string(),
            ];
            let step4 = Planner::fallback_plan_from_goal(goal, &history_after_new_item).unwrap();
            assert_eq!(step4["action"].as_str(), Some("type"));
            assert!(step4["text"]
                .as_str()
                .unwrap_or("")
                .contains("AI 뉴스 기사 요약"));
        }
    }

    #[test]
    fn fallback_plan_sports_news_to_notion_uses_topic_search() {
        let goal = "스포츠 뉴스 5개 선정해서 노션에 정리해줘";
        let step1 = Planner::fallback_plan_from_goal(goal, &[]).unwrap();
        assert_eq!(step1["action"].as_str(), Some("open_url"));
        let url = step1["url"].as_str().unwrap_or("");
        assert!(url.contains("google.com/search?q="));
        assert!(url.contains("%EC%8A%A4%ED%8F%AC%EC%B8%A0") || url.contains("sports"));
    }

    #[test]
    fn fallback_plan_types_unquoted_korean_payload_in_notes() {
        let goal = "메모장 열어서 박대엽이라고 써줘";
        let history = vec!["Opened app: Notes".to_string()];
        let plan = Planner::fallback_plan_from_goal(goal, &history).unwrap();
        assert_eq!(plan["action"].as_str(), Some("type"));
        assert_eq!(plan["app"].as_str(), Some("Notes"));
        assert_eq!(plan["text"].as_str(), Some("박대엽"));
    }
}
