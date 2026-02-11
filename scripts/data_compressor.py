"""
Data Compressor - payload JSON 압축 및 오래된 데이터 정리.

- payload를 gzip 압축하여 저장 공간 절약
- retention 정책에 따라 오래된 원시 이벤트 삭제
"""

import argparse
import gzip
import json
import logging
import sqlite3
from datetime import datetime, timedelta

logger = logging.getLogger(__name__)


def get_db_connection(db_path: str) -> sqlite3.Connection:
    """SQLite 연결 생성."""
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    return conn


def compress_payload(payload: dict) -> bytes:
    """payload를 gzip 압축."""
    json_str = json.dumps(payload, ensure_ascii=False)
    return gzip.compress(json_str.encode("utf-8"))


def decompress_payload(compressed: bytes) -> dict:
    """압축된 payload 복원."""
    json_str = gzip.decompress(compressed).decode("utf-8")
    return json.loads(json_str)


def ensure_compressed_table(conn: sqlite3.Connection):
    """압축 저장용 테이블 생성."""
    cursor = conn.cursor()
    
    cursor.execute("""
        CREATE TABLE IF NOT EXISTS events_compressed (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT UNIQUE NOT NULL,
            ts TEXT NOT NULL,
            source TEXT,
            app TEXT,
            event_type TEXT,
            priority TEXT,
            resource_type TEXT,
            resource_id TEXT,
            payload_compressed BLOB,
            window_id TEXT,
            pid INTEGER,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP
        )
    """)
    
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_ec_ts ON events_compressed(ts)")
    cursor.execute("CREATE INDEX IF NOT EXISTS idx_ec_app ON events_compressed(app)")
    
    conn.commit()


def compress_events(conn: sqlite3.Connection, days_old: int = 3):
    """
    일정 기간 지난 이벤트를 압축 테이블로 이동.
    
    Args:
        days_old: 이 기간 이전의 이벤트를 압축
    """
    cursor = conn.cursor()
    
    # events 테이블 확인
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='events'")
    if not cursor.fetchone():
        logger.warning("events table not found")
        return 0
    
    ensure_compressed_table(conn)
    
    cutoff_date = (datetime.now() - timedelta(days=days_old)).strftime("%Y-%m-%d")
    
    # 대상 이벤트 조회
    cursor.execute("""
        SELECT * FROM events 
        WHERE date(ts) < ?
        AND event_id NOT IN (SELECT event_id FROM events_compressed)
    """, (cutoff_date,))
    
    events = cursor.fetchall()
    compressed_count = 0
    
    for event in events:
        try:
            event_dict = dict(event)
            payload = event_dict.get("payload_json", event_dict.get("payload", {}))
            
            if isinstance(payload, str):
                try:
                    payload = json.loads(payload)
                except Exception:
                    payload = {}
            
            compressed_payload = compress_payload(payload)
            
            cursor.execute("""
                INSERT OR IGNORE INTO events_compressed
                (event_id, ts, source, app, event_type, priority, 
                 resource_type, resource_id, payload_compressed, window_id, pid)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """, (
                event_dict.get("event_id"),
                event_dict.get("ts"),
                event_dict.get("source"),
                event_dict.get("app"),
                event_dict.get("event_type"),
                event_dict.get("priority"),
                event_dict.get("resource_type"),
                event_dict.get("resource_id"),
                compressed_payload,
                event_dict.get("window_id"),
                event_dict.get("pid")
            ))
            
            compressed_count += 1
            
        except Exception as e:
            logger.warning(f"Error compressing event: {e}")
    
    conn.commit()
    logger.info(f"Compressed {compressed_count} events older than {cutoff_date}")
    
    return compressed_count


def cleanup_old_events(conn: sqlite3.Connection, raw_days: int = 7, compressed_days: int = 30):
    """
    오래된 이벤트 삭제.
    
    Args:
        raw_days: 원시 이벤트 보존 기간 (기본 7일)
        compressed_days: 압축 이벤트 보존 기간 (기본 30일)
    """
    cursor = conn.cursor()
    
    # 원시 이벤트 삭제 (압축 완료된 것만)
    raw_cutoff = (datetime.now() - timedelta(days=raw_days)).strftime("%Y-%m-%d")
    
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='events'")
    if cursor.fetchone():
        cursor.execute("""
            DELETE FROM events 
            WHERE date(ts) < ?
            AND event_id IN (SELECT event_id FROM events_compressed)
        """, (raw_cutoff,))
        deleted_raw = cursor.rowcount
        logger.info(f"Deleted {deleted_raw} raw events older than {raw_cutoff}")
    
    # 압축 이벤트 삭제
    compressed_cutoff = (datetime.now() - timedelta(days=compressed_days)).strftime("%Y-%m-%d")
    
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='events_compressed'")
    if cursor.fetchone():
        cursor.execute("""
            DELETE FROM events_compressed 
            WHERE date(ts) < ?
        """, (compressed_cutoff,))
        deleted_compressed = cursor.rowcount
        logger.info(f"Deleted {deleted_compressed} compressed events older than {compressed_cutoff}")
    
    conn.commit()
    
    # VACUUM으로 공간 회수
    logger.info("Running VACUUM to reclaim space...")
    conn.execute("VACUUM")


def get_storage_stats(conn: sqlite3.Connection) -> dict:
    """저장 공간 통계 조회."""
    cursor = conn.cursor()
    stats = {}
    
    # 원시 이벤트
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='events'")
    if cursor.fetchone():
        cursor.execute("SELECT COUNT(*) as cnt FROM events")
        stats["raw_events"] = cursor.fetchone()["cnt"]
    else:
        stats["raw_events"] = 0
    
    # 압축 이벤트
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='events_compressed'")
    if cursor.fetchone():
        cursor.execute("SELECT COUNT(*) as cnt FROM events_compressed")
        stats["compressed_events"] = cursor.fetchone()["cnt"]
    else:
        stats["compressed_events"] = 0
    
    # 집약 데이터
    cursor.execute("SELECT name FROM sqlite_master WHERE type='table' AND name='daily_summaries'")
    if cursor.fetchone():
        cursor.execute("SELECT COUNT(*) as cnt FROM daily_summaries")
        stats["daily_summaries"] = cursor.fetchone()["cnt"]
    else:
        stats["daily_summaries"] = 0
    
    return stats


def run_maintenance(db_path: str, compress_days: int = 3, raw_days: int = 7, compressed_days: int = 30):
    """전체 유지보수 실행."""
    logger.info(f"Running maintenance on {db_path}")
    
    conn = get_db_connection(db_path)
    
    try:
        # 1. 통계 출력 (전)
        stats_before = get_storage_stats(conn)
        logger.info(f"Before: {stats_before}")
        
        # 2. 압축
        compress_events(conn, compress_days)
        
        # 3. 정리
        cleanup_old_events(conn, raw_days, compressed_days)
        
        # 4. 통계 출력 (후)
        stats_after = get_storage_stats(conn)
        logger.info(f"After: {stats_after}")
        
        print("\n" + "=" * 50)
        print("Storage Maintenance Complete")
        print("=" * 50)
        print(f"Raw events: {stats_before['raw_events']} → {stats_after['raw_events']}")
        print(f"Compressed: {stats_before['compressed_events']} → {stats_after['compressed_events']}")
        print(f"Daily summaries: {stats_after['daily_summaries']}")
        print("=" * 50)
        
    finally:
        conn.close()


def parse_args():
    parser = argparse.ArgumentParser(description="Data Compressor & Retention Cleaner")
    parser.add_argument(
        "--db",
        default="collector.db",
        help="Path to collector database"
    )
    parser.add_argument(
        "--compress-days",
        type=int,
        default=3,
        help="Compress events older than N days (default: 3)"
    )
    parser.add_argument(
        "--raw-days",
        type=int,
        default=7,
        help="Keep raw events for N days (default: 7)"
    )
    parser.add_argument(
        "--compressed-days",
        type=int,
        default=30,
        help="Keep compressed events for N days (default: 30)"
    )
    return parser.parse_args()


def main():
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s"
    )
    args = parse_args()
    run_maintenance(
        args.db,
        args.compress_days,
        args.raw_days,
        args.compressed_days
    )


if __name__ == "__main__":
    main()
