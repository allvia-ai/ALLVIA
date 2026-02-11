"""
Data Aggregation Script - 데이터 집약 및 일일 요약 생성.

원시 이벤트를 시간 단위로 집약하고 일일 요약을 생성합니다.
"""

import argparse
import json
import logging
import sqlite3
from collections import defaultdict
from datetime import datetime, timedelta
from typing import Dict, List, Any, Optional

logger = logging.getLogger(__name__)


def get_db_connection(db_path: str) -> sqlite3.Connection:
    """SQLite 연결 생성."""
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    return conn


def ensure_aggregation_tables(conn: sqlite3.Connection):
    """집약 테이블 생성 (없으면)."""
    cursor = conn.cursor()
    
    # 시간 단위 집약 테이블
    cursor.execute("""
        CREATE TABLE IF NOT EXISTS hourly_aggregates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT NOT NULL,
            hour INTEGER NOT NULL,
            app TEXT NOT NULL,
            event_count INTEGER DEFAULT 0,
            unique_elements INTEGER DEFAULT 0,
            top_actions TEXT,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(date, hour, app)
        )
    """)
    
    # 일일 요약 테이블
    cursor.execute("""
        CREATE TABLE IF NOT EXISTS daily_summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date TEXT UNIQUE,
            total_events INTEGER DEFAULT 0,
            total_apps INTEGER DEFAULT 0,
            active_hours INTEGER DEFAULT 0,
            app_usage_json TEXT,
            top_actions_json TEXT,
            summary_text TEXT,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        )
    """)
    
    conn.commit()


def fetch_events_for_date(conn: sqlite3.Connection, target_date: str) -> List[Dict]:
    """특정 날짜의 이벤트 조회."""
    cursor = conn.cursor()
    
    # events 테이블 존재 확인
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='events'")
    if not cursor.fetchone():
        logger.warning("events table not found")
        return []
    
    cursor.execute("""
        SELECT * FROM events 
        WHERE date(ts) = ?
        ORDER BY ts
    """, (target_date,))
    
    rows = cursor.fetchall()
    return [dict(row) for row in rows]


def aggregate_hourly(events: List[Dict]) -> Dict[str, Dict[int, Dict]]:
    """이벤트를 앱별, 시간대별로 집약."""
    hourly_data = defaultdict(lambda: defaultdict(lambda: {
        "event_count": 0,
        "elements": set(),
        "actions": defaultdict(int)
    }))
    
    for event in events:
        try:
            ts = event.get("ts", "")
            app = event.get("app", "unknown")
            
            # 시간 파싱
            if "T" in ts:
                hour = int(ts.split("T")[1].split(":")[0])
            else:
                hour = 0
            
            payload = event.get("payload_json", event.get("payload", {}))
            if isinstance(payload, str):
                try:
                    payload = json.loads(payload)
                except Exception:
                    payload = {}
            
            element_name = payload.get("element_name", "")
            control_type = payload.get("control_type", "")
            
            hourly_data[app][hour]["event_count"] += 1
            if element_name:
                hourly_data[app][hour]["elements"].add(element_name)
            if control_type and element_name:
                action = f"{control_type}:{element_name}"
                hourly_data[app][hour]["actions"][action] += 1
                
        except Exception as e:
            logger.warning(f"Error processing event: {e}")
    
    return hourly_data


def save_hourly_aggregates(conn: sqlite3.Connection, target_date: str, hourly_data: Dict):
    """시간 단위 집약 데이터 저장."""
    cursor = conn.cursor()
    
    for app, hours in hourly_data.items():
        for hour, data in hours.items():
            # 상위 액션 추출
            top_actions = sorted(
                data["actions"].items(), 
                key=lambda x: x[1], 
                reverse=True
            )[:5]
            top_actions_json = json.dumps(top_actions, ensure_ascii=False)
            
            cursor.execute("""
                INSERT OR REPLACE INTO hourly_aggregates 
                (date, hour, app, event_count, unique_elements, top_actions)
                VALUES (?, ?, ?, ?, ?, ?)
            """, (
                target_date,
                hour,
                app,
                data["event_count"],
                len(data["elements"]),
                top_actions_json
            ))
    
    conn.commit()


def generate_daily_summary(conn: sqlite3.Connection, target_date: str, hourly_data: Dict) -> Dict:
    """일일 요약 생성."""
    total_events = 0
    app_usage = defaultdict(int)
    all_actions = defaultdict(int)
    active_hours = set()
    
    for app, hours in hourly_data.items():
        for hour, data in hours.items():
            total_events += data["event_count"]
            app_usage[app] += data["event_count"]
            active_hours.add(hour)
            for action, count in data["actions"].items():
                all_actions[action] += count
    
    # 상위 앱
    top_apps = sorted(app_usage.items(), key=lambda x: x[1], reverse=True)[:10]
    
    # 상위 액션
    top_actions = sorted(all_actions.items(), key=lambda x: x[1], reverse=True)[:20]
    
    # 요약 텍스트 생성
    summary_parts = [f"날짜: {target_date}"]
    summary_parts.append(f"총 이벤트: {total_events}건")
    summary_parts.append(f"사용 앱: {len(app_usage)}개")
    summary_parts.append(f"활동 시간대: {len(active_hours)}시간")
    
    if top_apps:
        summary_parts.append("\n주요 앱:")
        for app, count in top_apps[:5]:
            # 앱 이름 정리 (창 제목에서 앱 이름 추출)
            app_short = app.split(" - ")[-1] if " - " in app else app
            summary_parts.append(f"  - {app_short}: {count}건")
    
    summary = {
        "date": target_date,
        "total_events": total_events,
        "total_apps": len(app_usage),
        "active_hours": len(active_hours),
        "app_usage": dict(top_apps),
        "top_actions": dict(top_actions),
        "summary_text": "\n".join(summary_parts)
    }
    
    return summary


def save_daily_summary(conn: sqlite3.Connection, summary: Dict):
    """일일 요약 저장."""
    cursor = conn.cursor()
    
    cursor.execute("""
        INSERT OR REPLACE INTO daily_summaries 
        (date, total_events, total_apps, active_hours, app_usage_json, top_actions_json, summary_text)
        VALUES (?, ?, ?, ?, ?, ?, ?)
    """, (
        summary["date"],
        summary["total_events"],
        summary["total_apps"],
        summary["active_hours"],
        json.dumps(summary["app_usage"], ensure_ascii=False),
        json.dumps(summary["top_actions"], ensure_ascii=False),
        summary["summary_text"]
    ))
    
    conn.commit()


def run_aggregation(db_path: str, target_date: Optional[str] = None):
    """집약 실행."""
    if target_date is None:
        # 어제 날짜
        target_date = (datetime.now() - timedelta(days=1)).strftime("%Y-%m-%d")
    
    logger.info(f"Running aggregation for {target_date}")
    
    conn = get_db_connection(db_path)
    
    try:
        # 테이블 생성
        ensure_aggregation_tables(conn)
        
        # 이벤트 조회
        events = fetch_events_for_date(conn, target_date)
        logger.info(f"Found {len(events)} events for {target_date}")
        
        if not events:
            logger.info("No events to aggregate")
            return
        
        # 시간 단위 집약
        hourly_data = aggregate_hourly(events)
        save_hourly_aggregates(conn, target_date, hourly_data)
        logger.info(f"Saved hourly aggregates for {len(hourly_data)} apps")
        
        # 일일 요약
        summary = generate_daily_summary(conn, target_date, hourly_data)
        save_daily_summary(conn, summary)
        logger.info(f"Saved daily summary: {summary['total_events']} events")
        
        print("\n" + "=" * 50)
        print(summary["summary_text"])
        print("=" * 50)
        
    finally:
        conn.close()


def parse_args():
    parser = argparse.ArgumentParser(description="Data Aggregation Script")
    parser.add_argument(
        "--db",
        default="collector.db",
        help="Path to collector database"
    )
    parser.add_argument(
        "--date",
        help="Target date (YYYY-MM-DD). Default: yesterday"
    )
    return parser.parse_args()


def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s"
    )
    args = parse_args()
    run_aggregation(args.db, args.date)


if __name__ == "__main__":
    main()
