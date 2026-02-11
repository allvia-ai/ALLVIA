import argparse
import datetime
import json
import logging
import sqlite3
import re
from pathlib import Path
from typing import List, Dict, Any, Tuple

# Configure logging
logging.basicConfig(level=logging.INFO, format="%(asctime)s - %(levelname)s - %(message)s")
logger = logging.getLogger(__name__)

def connect_db(db_path: Path) -> sqlite3.Connection:
    if not db_path.exists():
        raise FileNotFoundError(f"Database not found at {db_path}")
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    return conn

def fetch_events(conn: sqlite3.Connection, hours: int = 24) -> List[sqlite3.Row]:
    cutoff = (datetime.datetime.utcnow() - datetime.timedelta(hours=hours)).isoformat()
    query = """
    SELECT ts, app, event_type, payload_json 
    FROM events 
    WHERE ts >= ? 
    ORDER BY ts ASC
    """
    return conn.execute(query, (cutoff,)).fetchall()

def parse_payload(payload_json: str) -> Dict[str, Any]:
    try:
        return json.loads(payload_json)
    except json.JSONDecodeError:
        return {}

def group_by_session(events: List[sqlite3.Row], gap_minutes: int = 15) -> List[List[Dict[str, Any]]]:
    sessions = []
    current_session = []
    last_ts = None
    
    for row in events:
        ts_str = row["ts"]
        ts = datetime.datetime.fromisoformat(ts_str.replace("Z", "+00:00"))
        
        if last_ts and (ts - last_ts).total_seconds() > (gap_minutes * 60):
            if current_session:
                sessions.append(current_session)
            current_session = []
            
        current_session.append({
            "ts": ts,
            "app": row["app"],
            "event_type": row["event_type"],
            "payload": parse_payload(row["payload_json"])
        })
        last_ts = ts
        
    if current_session:
        sessions.append(current_session)
        
    return sessions

def print_session_timeline(session: List[Dict[str, Any]]):
    if not session:
        return
    
    start = session[0]["ts"]
    end = session[-1]["ts"]
    duration = (end - start).total_seconds()
    
    print(f"\n=== Session: {start.strftime('%Y-%m-%d %H:%M:%S')} (Duration: {duration:.1f}s) ===")
    
    for event in session:
        ts_str = event["ts"].strftime("%H:%M:%S")
        app = event["app"]
        etype = event["event_type"]
        payload = event["payload"]
        
        # Visualize "user.interaction" nicely
        if etype == "user.interaction":
            element = payload.get("element_name", "")
            control = payload.get("control_type", "")
            value = payload.get("element_value", "")
            window = payload.get("window_title", "")
            
            detail = f"[{control}] '{element}'"
            if value:
                detail += f" = '{value}'"
            if window and window != app:
                detail += f" (in {window})"
                
            print(f"  {ts_str} [{app}] {detail}")
            
        elif etype == "os.foreground_changed":
            title = payload.get("window_title", "")
            print(f"  {ts_str} [SWITCH] -> {app} ({title})")
        else:
            # Generic fallback
            print(f"  {ts_str} [{app}] {etype}")

def main():
    parser = argparse.ArgumentParser(description="Analyze User Patterns from Collector DB")
    parser.add_argument("--db", default="collector.db", help="Path to collector.db")
    parser.add_argument("--hours", type=int, default=24, help="Analyze last N hours")
    parser.add_argument("--gap", type=int, default=15, help="Session gap in minutes")
    
    args = parser.parse_args()
    
    db_path = Path(args.db)
    try:
        conn = connect_db(db_path)
        logger.info(f"Connected to {db_path}")
        
        events = fetch_events(conn, args.hours)
        logger.info(f"Fetched {len(events)} events")
        
        sessions = group_by_session(events, args.gap)
        logger.info(f"Grouped into {len(sessions)} sessions")
        
        for session in sessions:
            print_session_timeline(session)
            
    except Exception as e:
        logger.error(f"Analysis failed: {e}")

if __name__ == "__main__":
    main()
