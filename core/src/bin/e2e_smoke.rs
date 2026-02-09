use chrono::Utc;
use local_os_agent::{db, pattern_detector, recommendation, schema};
use recommendation::TemplateMatcher;
use schema::{EventEnvelope, ResourceContext};

fn main() {
    if let Err(e) = db::init() {
        eprintln!("DB init failed: {}", e);
        return;
    }

    let events = build_sample_events();
    for ev in &events {
        if let Err(e) = db::insert_event_v2(ev) {
            eprintln!("Insert event failed: {}", e);
        }
    }

    let logs: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap_or_default())
        .collect();

    let detector = pattern_detector::PatternDetector::new();
    let matcher = TemplateMatcher::new();
    let patterns = detector.analyze_with_events(&logs);

    let mut inserted = 0;
    for pattern in patterns {
        if !detector.should_recommend(&pattern) {
            continue;
        }
        if let Some(proposal) = matcher.match_pattern(&pattern) {
            if let Ok(true) = db::insert_recommendation(&proposal) {
                inserted += 1;
            }
        }
    }

    let recs = db::get_recommendations_with_filter(Some("all")).unwrap_or_default();
    let total = recs.len();
    let latest: Vec<_> = recs.iter().take(5).collect();

    println!(
        "E2E Smoke: inserted {} recommendations (total: {})",
        inserted, total
    );
    for r in latest {
        println!(
            "- [{}] {} ({:.0}%)",
            r.status,
            r.title,
            r.confidence * 100.0
        );
    }
}

fn build_sample_events() -> Vec<EventEnvelope> {
    let now = Utc::now().to_rfc3339();
    let mut events = Vec::new();

    // App switch flow (Slack <-> Chrome) repeated to trigger AppSequence
    for _ in 0..5 {
        events.push(EventEnvelope {
            schema_version: "1.0".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            ts: now.clone(),
            source: "e2e_smoke".to_string(),
            app: "Slack".to_string(),
            event_type: "app_switch".to_string(),
            priority: "P2".to_string(),
            resource: Some(ResourceContext {
                resource_type: "app".to_string(),
                id: "Slack".to_string(),
            }),
            payload: serde_json::json!({"app":"Slack","window_title":"Inbox","browser_url":"https://mail.google.com"}),
            privacy: None,
            pid: None,
            window_id: None,
            window_title: None,
            browser_url: Some("https://mail.google.com".to_string()),
            raw: None,
        });
        events.push(EventEnvelope {
            schema_version: "1.0".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            ts: now.clone(),
            source: "e2e_smoke".to_string(),
            app: "Chrome".to_string(),
            event_type: "app_switch".to_string(),
            priority: "P2".to_string(),
            resource: Some(ResourceContext {
                resource_type: "app".to_string(),
                id: "Chrome".to_string(),
            }),
            payload: serde_json::json!({"app":"Chrome","window_title":"Docs","browser_url":"https://docs.google.com"}),
            privacy: None,
            pid: None,
            window_id: None,
            window_title: None,
            browser_url: Some("https://docs.google.com".to_string()),
            raw: None,
        });
    }

    // File pattern (3 pdfs)
    for i in 1..=3 {
        events.push(EventEnvelope {
            schema_version: "1.0".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            ts: now.clone(),
            source: "e2e_smoke".to_string(),
            app: "Finder".to_string(),
            event_type: "file_created".to_string(),
            priority: "P2".to_string(),
            resource: Some(ResourceContext {
                resource_type: "file".to_string(),
                id: format!("/Users/test/Downloads/report{}.pdf", i),
            }),
            payload: serde_json::json!({"path":format!("/Users/test/Downloads/report{}.pdf", i),"filename":format!("report{}.pdf", i)}),
            privacy: None,
            pid: None,
            window_id: None,
            window_title: None,
            browser_url: None,
            raw: None,
        });
    }

    // Keyword repeat (5 occurrences)
    for _ in 0..5 {
        events.push(EventEnvelope {
            schema_version: "1.0".to_string(),
            event_id: uuid::Uuid::new_v4().to_string(),
            ts: now.clone(),
            source: "e2e_smoke".to_string(),
            app: "Mail".to_string(),
            event_type: "key_input".to_string(),
            priority: "P2".to_string(),
            resource: Some(ResourceContext {
                resource_type: "input".to_string(),
                id: "keyboard".to_string(),
            }),
            payload: serde_json::json!({"text":"invoice follow-up"}),
            privacy: None,
            pid: None,
            window_id: None,
            window_title: None,
            browser_url: None,
            raw: None,
        });
    }

    events
}
