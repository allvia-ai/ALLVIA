use crate::db;
use crate::llm_gateway;
use crate::memory::MemoryStore; // Added for RAG
use crate::notifier;
use crate::pattern_detector::PatternDetector;
use crate::recommendation::TemplateMatcher;
use crate::schema::EventEnvelope;
use crate::session::Sessionizer;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub fn spawn(
    mut log_rx: mpsc::Receiver<String>,
    #[allow(unused)] // LLM might be unused if we rely solely on patterns for now
    llm_client: Arc<dyn llm_gateway::LLMClient>,
) {
    tokio::spawn(async move {
        // Buffers
        let mut session_buffer: Vec<EventEnvelope> = Vec::new();
        let sessionizer = Sessionizer::new(15 * 60); // 15 min idle gap
        let detector = PatternDetector::new();
        let matcher = TemplateMatcher::new();

        let batch_size = 50;
        let mut last_process_at = Instant::now();
        let max_buffer_age = Duration::from_secs(60); // Process at least every minute

        // [Privacy] Initialize Guard with Salt (Env or Default)
        let salt = std::env::var("PRIVACY_SALT").unwrap_or_else(|_| {
            eprintln!("⚠️ [Privacy] PRIVACY_SALT not set, using default. Set PRIVACY_SALT env var for production.");
            "default_salt".to_string()
        });
        let guard = crate::privacy::PrivacyGuard::new(salt);

        // [Memory] Initialize Vector DB (Absolute Path)
        let mem_path = if let Some(mut path) = dirs::data_local_dir() {
            path.push("steer");
            path.push("steer_mem");
            path.to_string_lossy().to_string()
        } else {
            "steer_mem".to_string() // Fallback
        };
        let memory = match MemoryStore::new(&mem_path, llm_client.clone()).await {
            Ok(m) => {
                println!("🧠 [Memory] Visual Cortex Online at: {}", mem_path);
                Some(m)
            }
            Err(e) => {
                eprintln!("⚠️ [Memory] Failed to init Vector DB: {}", e);
                None
            }
        };

        while let Some(log_json) = log_rx.recv().await {
            // [Pipeline Upgrade] Parse -> Sanitize -> Store V2
            // 1. Parse Event
            if let Ok(event) = serde_json::from_str::<EventEnvelope>(&log_json) {
                // 2. Apply Privacy Guard
                if let Some(masked_event) = guard.apply(event) {
                    // [RAG] Ingest File System Changes (Active Watcher)
                    if masked_event.source == "filesystem"
                        && (masked_event.event_type == "file_created"
                            || masked_event.event_type == "file_modified")
                    {
                        if let Some(path) =
                            masked_event.payload.get("path").and_then(|v| v.as_str())
                        {
                            // Filter extensions
                            let p = std::path::Path::new(path);
                            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                                if ["md", "txt", "rs", "py", "ts", "tsx", "js", "json"]
                                    .contains(&ext)
                                {
                                    if let Some(mem) = &memory {
                                        // Read file content
                                        match tokio::fs::read_to_string(path).await {
                                            Ok(content) => {
                                                let meta = serde_json::json!({
                                                    "source": "file_watcher",
                                                    "path": path,
                                                    "timestamp": masked_event.ts
                                                });
                                                println!(
                                                    "📄 [Analyzer] Ingesting file update: {}",
                                                    path
                                                );
                                                if let Err(e) = mem.add(&content, meta).await {
                                                    if e.to_string().contains("RATE_LIMITED_QUOTA")
                                                    {
                                                        eprintln!("⚠️ [Analyzer] Mem Ingest Paused (Quota)");
                                                    } else {
                                                        eprintln!(
                                                            "⚠️ [Analyzer] Mem Ingest Failed: {}",
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                            Err(e) => eprintln!(
                                                "⚠️ [Analyzer] Failed to read file {}: {}",
                                                path, e
                                            ),
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 3. Persist to V2 Table
                    if let Err(e) = db::insert_event_v2(&masked_event) {
                        eprintln!("⚠️ [Analyzer] DB Insert Error: {}", e);
                    }

                    // 4. Buffer Sanitized Event for Intelligence
                    let is_idle = masked_event.event_type.contains("idle");
                    session_buffer.push(masked_event);

                    // 5. Trigger Check
                    if session_buffer.len() >= batch_size
                        || last_process_at.elapsed() >= max_buffer_age
                        || is_idle
                    {
                        process_buffer(
                            &mut session_buffer,
                            &sessionizer,
                            &detector,
                            &matcher,
                            &memory,
                            &llm_client,
                        )
                        .await;
                        last_process_at = Instant::now();
                    }
                } else {
                    // Dropped by Privacy Guard
                }
            } else {
                eprintln!(
                    "⚠️ [Analyzer] Failed to parse log as EventEnvelope: {}",
                    log_json
                );
            }
        }
    });
}

/// Core Intelligence Loop
async fn process_buffer(
    buffer: &mut Vec<EventEnvelope>,
    sessionizer: &Sessionizer,
    detector: &PatternDetector,
    matcher: &TemplateMatcher,
    memory: &Option<MemoryStore>,
    llm: &Arc<dyn llm_gateway::LLMClient>,
) {
    if buffer.is_empty() {
        return;
    }

    // A. Sessionize (Cut valid sessions from buffer)
    let sessions = sessionizer.sessionize(buffer);
    for session in &sessions {
        if let Err(e) = db::insert_session(session) {
            eprintln!("⚠️ [Analyzer] Failed to save session: {}", e);
        }

        // [Memory] Ingest Session Context
        if let Some(mem) = memory {
            let summary = format!(
                "User Activity Session: Used {} for {} events. Key Context: {:?} Resources: {:?}",
                session.summary.top_app,
                session.summary.event_count,
                session.summary.key_events,
                session.summary.resources
            );

            let meta = serde_json::json!({
                "source": "session",
                "session_id": session.session_id,
                "timestamp": session.start_ts
            });

            if let Err(e) = mem.add(&summary, meta).await {
                if e.to_string().contains("RATE_LIMITED_QUOTA") {
                    eprintln!("⚠️ [Memory] Ingestion Paused (Quota)");
                } else {
                    eprintln!("⚠️ [Memory] Ingestion Failed: {}", e);
                }
            }
        }
    }

    // B. Detect Patterns
    // Use async analysis (includes vector fuzzy matching)
    let patterns = detector.analyze_async().await;

    if !patterns.is_empty() {
        println!(
            "🧠 [Analyzer] Detected {} patterns in recent activity.",
            patterns.len()
        );
    }

    // C. Match & Recommend (rate-limited)
    let max_per_day: i64 = env_i64("REC_MAX_PER_DAY", 3);
    let min_confidence: f64 = env_f64("REC_MIN_CONFIDENCE", 0.8);
    let cooldown_hours: i64 = env_i64("REC_PATTERN_COOLDOWN_HOURS", 72);
    let mut remaining_budget = match db::count_recent_recommendations(24) {
        Ok(count) => {
            if count >= max_per_day {
                0
            } else {
                (max_per_day - count) as usize
            }
        }
        Err(_) => max_per_day as usize,
    };

    for pattern in patterns {
        if !pattern_is_recommendable(&pattern) {
            continue;
        }

        if cooldown_hours > 0 {
            if let Ok(true) =
                db::has_recent_pattern_recommendation(&pattern.pattern_id, cooldown_hours)
            {
                continue;
            }
        }

        if let Err(e) = db::insert_routine_candidate(&pattern) {
            eprintln!("⚠️ [Analyzer] Failed to save routine candidate: {}", e);
        }

        if remaining_budget == 0 {
            break;
        }

        // 1. Try Template Match (Fast, High Trust, Zero Cost)
        if let Some(proposal) = matcher.match_pattern(&pattern) {
            if proposal.confidence >= min_confidence {
                println!("✨ [Analyzer] Matched Template: {}", proposal.title);
                if let Err(e) = db::insert_recommendation(&proposal) {
                    eprintln!("⚠️ [Analyzer] DB Error: {}", e);
                } else {
                    let _ = notifier::send("AllvIa", &format!("💡 New Idea: {}", proposal.title));
                    remaining_budget -= 1;
                }
                continue; // Skip LLM if template matched
            }
        }

        // 2. Hybrid Intelligence (Router)
        // Rule: Use AI if budget exists and pattern is strong.
        let has_pii = false;
        let (use_local, preferred_model) = llm.route_task(&pattern.description, has_pii);

        println!(
            "🤖 [Analyzer] Hybrid Intelligence: Routing to {} (Model: {})",
            if use_local {
                "Local (Ollama)"
            } else {
                "Cloud (OpenAI)"
            },
            preferred_model
        );

        let proposal_result = if use_local {
            // Local Inference (Ollama) - Not yet implemented
            println!("   -> Local route selected but unavailable; falling back to Cloud path.");
            llm.generate_recommendation_from_pattern(&pattern.description, &pattern.sample_events)
                .await
        } else {
            // Cloud Inference (OpenAI)
            // Re-use existing method
            // We need a method that takes a Pattern and returns Proposal.
            // Existing `propose_workflow` takes `logs`.
            // Let's use `generate_recommendation_from_pattern` which we should have...
            // Wait, we added `generate_recommendation_from_pattern` in llm_gateway?
            // I checked llm_gateway.rs, yes, on line 648! Valid.
            llm.generate_recommendation_from_pattern(&pattern.description, &pattern.sample_events)
                .await
        };

        match proposal_result {
            Ok(mut proposal) => {
                proposal.pattern_id = Some(pattern.pattern_id.clone());
                if proposal.evidence.is_empty() {
                    proposal.evidence = build_evidence(&pattern);
                }
                if proposal.confidence >= min_confidence {
                    println!("✨ [Analyzer] AI Generated Idea: {}", proposal.title);
                    if let Err(e) = db::insert_recommendation(&proposal) {
                        eprintln!("⚠️ [Analyzer] DB Error: {}", e);
                    } else {
                        let _ = notifier::send(
                            "AllvIa",
                            &format!("✨ New Idea (AI): {}", proposal.title),
                        );
                        remaining_budget -= 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️ [Analyzer] AI Recommendation Failed: {}", e);
            }
        }
    }

    // Clear buffer after processing
    buffer.clear();
}

fn env_i64(key: &str, default_val: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_val)
}

fn env_u32(key: &str, default_val: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_val)
}

fn env_f64(key: &str, default_val: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_val)
}

fn pattern_is_recommendable(pattern: &crate::pattern_detector::DetectedPattern) -> bool {
    use crate::pattern_detector::PatternType::*;
    let (min_occ, min_sim) = match pattern.pattern_type {
        AppSequence => (
            env_u32("REC_MIN_OCCURRENCES_APP", 4),
            env_f64("REC_MIN_SIMILARITY_APP", 0.8),
        ),
        KeywordRepeat => (
            env_u32("REC_MIN_OCCURRENCES_KEYWORD", 5),
            env_f64("REC_MIN_SIMILARITY_KEYWORD", 0.85),
        ),
        FilePattern => (
            env_u32("REC_MIN_OCCURRENCES_FILE", 4),
            env_f64("REC_MIN_SIMILARITY_FILE", 0.85),
        ),
        TimeBasedAction => (
            env_u32("REC_MIN_OCCURRENCES_TIME", 4),
            env_f64("REC_MIN_SIMILARITY_TIME", 0.8),
        ),
    };
    pattern.occurrences >= min_occ && pattern.similarity_score >= min_sim
}

fn build_evidence(pattern: &crate::pattern_detector::DetectedPattern) -> Vec<String> {
    let mut evidence = vec![
        format!("Pattern: {}", pattern.description),
        format!("Frequency: Found {} occurrences", pattern.occurrences),
    ];
    if let Some(sample) = pattern.sample_events.first() {
        let snippet = if sample.len() > 140 {
            format!("{}...", &sample[..140])
        } else {
            sample.clone()
        };
        evidence.push(format!("Sample: {}", snippet));
    }
    evidence
}
