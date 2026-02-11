import sqlite3
import uuid
import json
import datetime
import argparse

def get_utc_now_str(offset_seconds=0):
    ts = datetime.datetime.utcnow() + datetime.timedelta(seconds=offset_seconds)
    return ts.isoformat() + "Z"

def seed_data(db_path="collector.db"):
    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    
    base_time = datetime.datetime.utcnow() - datetime.timedelta(minutes=30)
    
    events = []
    
    # Session 1: Outlook Email
    events.append({
        "app": "Outlook.exe",
        "event_type": "user.interaction",
        "ts": (base_time + datetime.timedelta(seconds=10)).isoformat() + "Z",
        "payload": {
            "window_title": "Inbox - Outlook",
            "control_type": "Button",
            "element_name": "New Email",
            "element_value": ""
        }
    })
    events.append({
        "app": "Outlook.exe",
        "event_type": "user.interaction",
        "ts": (base_time + datetime.timedelta(seconds=15)).isoformat() + "Z",
        "payload": {
            "window_title": "Untitled - Message (HTML)",
            "control_type": "Edit",
            "element_name": "To",
            "element_value": "boss@example.com"
        }
    })
    events.append({
        "app": "Outlook.exe",
        "event_type": "user.interaction",
        "ts": (base_time + datetime.timedelta(seconds=30)).isoformat() + "Z",
        "payload": {
            "window_title": "Untitled - Message (HTML)",
            "control_type": "Button",
            "element_name": "Send",
            "element_value": ""
        }
    })

    # Session 2: Excel Work (after 20 mins)
    base_time2 = base_time + datetime.timedelta(minutes=20)
    events.append({
        "app": "Excel.exe",
        "event_type": "user.interaction",
        "ts": (base_time2 + datetime.timedelta(seconds=5)).isoformat() + "Z",
        "payload": {
            "window_title": "Report.xlsx - Excel",
            "control_type": "Pane",
            "element_name": "Grid",
            "element_value": "1000"
        }
    })

    print(f"Inserting {len(events)} events...")
    
    for ev in events:
        cursor.execute("""
            INSERT INTO events (
                schema_version, event_id, ts, source, app, event_type, priority,
                resource_type, resource_id, payload_json, privacy_json, raw_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """, (
            "1.0",
            str(uuid.uuid4()),
            ev["ts"],
            "ui_automation",
            ev["app"],
            ev["event_type"],
            "P2",
            "ui_element",
            "dummy_id",
            json.dumps(ev["payload"]),
            "{}",
            "{}"
        ))
    
    conn.commit()
    conn.close()
    print("Done.")

def parse_args():
    parser = argparse.ArgumentParser(description="Seed dummy collector events")
    parser.add_argument("--db", default="collector.db", help="Path to collector database")
    return parser.parse_args()


if __name__ == "__main__":
    args = parse_args()
    seed_data(args.db)
