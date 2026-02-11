use anyhow::Result;
use local_os_agent::collector_pipeline;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct Args {
    config: String,
    start: String,
    end: String,
    since_hours: f64,
    gap_minutes: i64,
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

    if args.since_hours > 0.0 && start_ts.is_none() {
        start_ts = Some(collector_pipeline::iso_now_minus_hours(args.since_hours));
    }

    if args.use_state && start_ts.is_none() && args.since_hours <= 0.0 {
        if let Some(last) = collector_pipeline::get_state(&conn, "last_sessionized_ts")? {
            if let Some(next) = collector_pipeline::plus_one_microsecond_iso(&last) {
                start_ts = Some(next);
            }
        }
    }

    let events = collector_pipeline::fetch_events(&conn, start_ts.as_deref(), end_ts.as_deref())?;
    let sessions = collector_pipeline::sessionize_events(&events, args.gap_minutes.max(0) * 60);
    let records = collector_pipeline::build_session_records(&sessions);

    if args.dry_run {
        println!("sessions_ready={} dry_run=true", records.len());
        return Ok(());
    }

    for record in &records {
        collector_pipeline::insert_session_record(&conn, record)?;
    }

    if args.use_state {
        if let Some(last) = records.last() {
            collector_pipeline::set_state(&conn, "last_sessionized_ts", &last.end_ts)?;
        }
    }

    println!("sessions_inserted={}", records.len());
    Ok(())
}

fn parse_args() -> Args {
    let mut args = Args {
        config: "configs/config.yaml".to_string(),
        start: String::new(),
        end: String::new(),
        since_hours: 0.0,
        gap_minutes: 15,
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
            "--since-hours" => {
                if let Some(v) = argv.get(i + 1) {
                    args.since_hours = v.parse::<f64>().unwrap_or(0.0);
                    i += 1;
                }
            }
            "--gap-minutes" => {
                if let Some(v) = argv.get(i + 1) {
                    args.gap_minutes = v.parse::<i64>().unwrap_or(15);
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
