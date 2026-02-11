use anyhow::Result;
use local_os_agent::collector_pipeline;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct Args {
    config: String,
    start: String,
    end: String,
    days: f64,
    n_min: usize,
    n_max: usize,
    min_support: i64,
    max_patterns: usize,
    max_evidence: usize,
    use_state: bool,
    dry_run: bool,
}

fn main() -> Result<()> {
    let args = parse_args();
    let config_path = PathBuf::from(&args.config);
    let db_path = collector_pipeline::resolve_db_path(Some(&config_path));
    let conn = collector_pipeline::open_connection(&db_path)?;

    collector_pipeline::ensure_pipeline_tables(&conn)?;

    let mut start_ts = empty_to_none(&args.start);
    let end_ts = empty_to_none(&args.end);

    if args.days > 0.0 && start_ts.is_none() {
        start_ts = Some(collector_pipeline::iso_now_minus_days(args.days));
    }

    let latest_end_ts = collector_pipeline::fetch_latest_session_end_ts(&conn)?;

    if args.use_state {
        if let (Some(last_routine_ts), Some(latest)) = (
            collector_pipeline::get_state(&conn, "last_routine_ts")?,
            latest_end_ts.clone(),
        ) {
            if let (Some(last_parsed), Some(latest_parsed)) = (
                collector_pipeline::parse_iso_ts(&last_routine_ts),
                collector_pipeline::parse_iso_ts(&latest),
            ) {
                if latest_parsed <= last_parsed {
                    println!("routine_candidates_skipped=unchanged");
                    return Ok(());
                }
            }
        }
    }

    let sessions =
        collector_pipeline::fetch_sessions(&conn, start_ts.as_deref(), end_ts.as_deref())?;
    let candidates = collector_pipeline::build_routine_candidates(
        &sessions,
        args.n_min,
        args.n_max,
        args.min_support,
        args.max_patterns,
        args.max_evidence,
    );

    if args.dry_run {
        println!("routine_candidates_ready={} dry_run=true", candidates.len());
        return Ok(());
    }

    collector_pipeline::clear_routine_candidates(&conn)?;
    for candidate in &candidates {
        collector_pipeline::insert_routine_candidate(&conn, candidate)?;
    }

    if args.use_state {
        if let Some(latest) = latest_end_ts {
            collector_pipeline::set_state(&conn, "last_routine_ts", &latest)?;
        }
    }

    println!("routine_candidates_inserted={}", candidates.len());
    Ok(())
}

fn parse_args() -> Args {
    let mut args = Args {
        config: "configs/config.yaml".to_string(),
        start: String::new(),
        end: String::new(),
        days: 7.0,
        n_min: 2,
        n_max: 5,
        min_support: 2,
        max_patterns: 100,
        max_evidence: 10,
        use_state: false,
        dry_run: false,
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
            "--start" => {
                if let Some(v) = argv.get(i + 1) {
                    args.start = v.clone();
                    i += 1;
                }
            }
            "--end" => {
                if let Some(v) = argv.get(i + 1) {
                    args.end = v.clone();
                    i += 1;
                }
            }
            "--days" => {
                if let Some(v) = argv.get(i + 1) {
                    args.days = v.parse::<f64>().unwrap_or(7.0);
                    i += 1;
                }
            }
            "--n-min" => {
                if let Some(v) = argv.get(i + 1) {
                    args.n_min = v.parse::<usize>().unwrap_or(2);
                    i += 1;
                }
            }
            "--n-max" => {
                if let Some(v) = argv.get(i + 1) {
                    args.n_max = v.parse::<usize>().unwrap_or(5);
                    i += 1;
                }
            }
            "--min-support" => {
                if let Some(v) = argv.get(i + 1) {
                    args.min_support = v.parse::<i64>().unwrap_or(2);
                    i += 1;
                }
            }
            "--max-patterns" => {
                if let Some(v) = argv.get(i + 1) {
                    args.max_patterns = v.parse::<usize>().unwrap_or(100);
                    i += 1;
                }
            }
            "--max-evidence" => {
                if let Some(v) = argv.get(i + 1) {
                    args.max_evidence = v.parse::<usize>().unwrap_or(10);
                    i += 1;
                }
            }
            "--use-state" => {
                args.use_state = true;
            }
            "--dry-run" => {
                args.dry_run = true;
            }
            _ => {}
        }
        i += 1;
    }

    args
}

fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
