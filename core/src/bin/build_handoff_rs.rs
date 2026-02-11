use anyhow::Result;
use local_os_agent::collector_pipeline;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct Args {
    config: String,
    max_size_kb: usize,
    sessions: usize,
    routines: usize,
    resources: usize,
    evidence: usize,
    redaction_scan: usize,
    dry_run: bool,
    skip_unchanged: bool,
    keep_latest_pending: bool,
}

fn main() -> Result<()> {
    let args = parse_args();
    let config_path = PathBuf::from(&args.config);
    let db_path = collector_pipeline::resolve_db_path(Some(&config_path));
    let privacy_rules_path = collector_pipeline::resolve_privacy_rules_path(Some(&config_path));

    let conn = collector_pipeline::open_connection(&db_path)?;
    collector_pipeline::ensure_pipeline_tables(&conn)?;

    let rules = collector_pipeline::load_handoff_privacy_rules(&privacy_rules_path);
    let options = collector_pipeline::HandoffBuildOptions {
        max_size_bytes: args.max_size_kb.saturating_mul(1024),
        recent_sessions: args.sessions,
        recent_routines: args.routines,
        max_resources: args.resources,
        max_evidence: args.evidence,
        redaction_scan_limit: args.redaction_scan,
    };

    let payload = collector_pipeline::build_handoff_with_size_guard(&conn, &rules, &options)?;
    let last_event_ts = payload
        .payload
        .get("device_context")
        .and_then(|v| v.get("last_event_ts"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    if args.skip_unchanged {
        if let Some(previous) = collector_pipeline::fetch_latest_pending_handoff_payload(&conn)? {
            let prev_ts = previous
                .get("device_context")
                .and_then(|v| v.get("last_event_ts"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !prev_ts.is_empty() && prev_ts == last_event_ts {
                println!("handoff_skipped=unchanged");
                return Ok(());
            }
        }
    }

    if args.dry_run {
        println!("handoff_ready size_bytes={}", payload.size_bytes);
        return Ok(());
    }

    if args.keep_latest_pending {
        collector_pipeline::clear_pending_handoff(&conn)?;
    }

    collector_pipeline::enqueue_handoff(&conn, &payload)?;
    println!("handoff_enqueued size_bytes={}", payload.size_bytes);
    Ok(())
}

fn parse_args() -> Args {
    let mut args = Args {
        config: "configs/config.yaml".to_string(),
        max_size_kb: 50,
        sessions: 3,
        routines: 10,
        resources: 10,
        evidence: 5,
        redaction_scan: 200,
        dry_run: false,
        skip_unchanged: false,
        keep_latest_pending: false,
    };

    let argv: Vec<String> = env::args().collect();
    let mut i = 1usize;
    while i < argv.len() {
        match argv[i].as_str() {
            "--config" => {
                if let Some(v) = argv.get(i + 1) {
                    args.config = v.clone();
                    i += 1;
                }
            }
            "--max-size-kb" => {
                if let Some(v) = argv.get(i + 1) {
                    args.max_size_kb = v.parse::<usize>().unwrap_or(50);
                    i += 1;
                }
            }
            "--sessions" => {
                if let Some(v) = argv.get(i + 1) {
                    args.sessions = v.parse::<usize>().unwrap_or(3);
                    i += 1;
                }
            }
            "--routines" => {
                if let Some(v) = argv.get(i + 1) {
                    args.routines = v.parse::<usize>().unwrap_or(10);
                    i += 1;
                }
            }
            "--resources" => {
                if let Some(v) = argv.get(i + 1) {
                    args.resources = v.parse::<usize>().unwrap_or(10);
                    i += 1;
                }
            }
            "--evidence" => {
                if let Some(v) = argv.get(i + 1) {
                    args.evidence = v.parse::<usize>().unwrap_or(5);
                    i += 1;
                }
            }
            "--redaction-scan" => {
                if let Some(v) = argv.get(i + 1) {
                    args.redaction_scan = v.parse::<usize>().unwrap_or(200);
                    i += 1;
                }
            }
            "--dry-run" => {
                args.dry_run = true;
            }
            "--skip-unchanged" => {
                args.skip_unchanged = true;
            }
            "--keep-latest-pending" => {
                args.keep_latest_pending = true;
            }
            _ => {}
        }
        i += 1;
    }

    args
}
