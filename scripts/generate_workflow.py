import os
import argparse
import datetime
import json
import logging
import sqlite3
from pathlib import Path
from typing import List, Dict, Any

try:
    from dotenv import load_dotenv
except ImportError:
    load_dotenv = None

try:
    from openai import OpenAI
except ImportError:
    OpenAI = None

# Configure logging
logging.basicConfig(level=logging.INFO, format="%(asctime)s - %(levelname)s - %(message)s")
logger = logging.getLogger(__name__)

# Load environment variables when python-dotenv is available.
if load_dotenv:
    load_dotenv()

# JSON Workflow Schema
WORKFLOW_SCHEMA = """
{
  "workflow_name": "string - 워크플로우 이름",
  "description": "string - 워크플로우 설명",
  "created_at": "string - ISO8601 타임스탬프",
  "steps": [
    {
      "step_number": "int - 순서",
      "action_type": "string - click | type | open | close | wait | scroll",
      "target": {
        "app": "string - 대상 앱 이름",
        "window_title": "string - 창 제목 패턴",
        "control_type": "string - Button | Edit | MenuItem | TreeItem 등",
        "element_name": "string - 요소 이름",
        "automation_id": "string (optional) - 자동화 ID"
      },
      "value": "string (optional) - 입력할 값",
      "description": "string - 이 단계 설명"
    }
  ],
  "metadata": {
    "source_events_count": "int - 원본 이벤트 수",
    "estimated_duration_seconds": "int - 예상 소요 시간"
  }
}
"""

def connect_db(db_path: Path) -> sqlite3.Connection:
    if not db_path.exists():
        raise FileNotFoundError(f"Database not found at {db_path}")
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row
    return conn

def fetch_latest_session_events(conn: sqlite3.Connection, gap_minutes: int = 15) -> List[Dict[str, Any]]:
    cutoff = (datetime.datetime.utcnow() - datetime.timedelta(hours=24)).isoformat()
    query = """
    SELECT ts, app, event_type, payload_json 
    FROM events 
    WHERE ts >= ? 
    ORDER BY ts ASC
    """
    rows = conn.execute(query, (cutoff,)).fetchall()
    
    events = []
    for row in rows:
        events.append({
            "ts": datetime.datetime.fromisoformat(row["ts"].replace("Z", "+00:00")),
            "app": row["app"],
            "event_type": row["event_type"],
            "payload": json.loads(row["payload_json"] or "{}")
        })
        
    if not events:
        return []

    # finding the last session
    current_session = []
    last_session = []
    last_ts = None
    
    for ev in events:
        if last_ts and (ev["ts"] - last_ts).total_seconds() > (gap_minutes * 60):
            if current_session:
                last_session = current_session
            current_session = []
        current_session.append(ev)
        last_ts = ev["ts"]
        
    if current_session:
        last_session = current_session
        
    return last_session

def construct_prompt(session: List[Dict[str, Any]]) -> str:
    lines = []
    lines.append("I have collected a log of user actions. Please analyze these actions and generate a structured JSON workflow.")
    lines.append("\n### Output Format (JSON Schema):")
    lines.append(WORKFLOW_SCHEMA)
    lines.append("\n### User Actions Log:")
    
    for ev in session:
        ts = ev["ts"].strftime("%H:%M:%S")
        app = ev["app"]
        p = ev["payload"]
        
        detail = ""
        if ev["event_type"] == "user.interaction":
            ctl = p.get("control_type", "")
            name = p.get("element_name", "")
            val = p.get("element_value", "")
            win = p.get("window_title", "")
            auto_id = p.get("automation_id", "")
            detail = f"ControlType: {ctl}, Name: '{name}'"
            if val: detail += f", Value: '{val}'"
            if win: detail += f", Window: '{win}'"
            if auto_id: detail += f", AutomationId: '{auto_id}'"
            
        lines.append(f"- [{ts}] App: {app} | {detail}")
        
    lines.append("\n### Instructions:")
    lines.append("1. Analyze the sequence of user actions above.")
    lines.append("2. Group similar/repeated actions into logical steps.")
    lines.append("3. Generate a JSON workflow following the exact schema provided.")
    lines.append("4. Return ONLY valid JSON. No markdown, no explanation, no code blocks.")
    return "\n".join(lines)

def generate_workflow(prompt: str) -> Dict[str, Any]:
    api_key = os.getenv("LLM_API_KEY")
    
    if not api_key or "YOUR" in api_key:
        logger.warning("No valid API Key found in .env. Falling back to mock generation.")
        return mock_llm_generate(prompt)
    if OpenAI is None:
        logger.warning("openai package not installed. Falling back to mock generation.")
        return mock_llm_generate(prompt)
        
    try:
        client = OpenAI(api_key=api_key)
        
        response = client.chat.completions.create(
            model="gpt-4o-mini",
            messages=[
                {"role": "system", "content": "You are a workflow automation expert. Analyze user actions and generate structured JSON workflows. Return ONLY valid JSON."},
                {"role": "user", "content": prompt}
            ],
            temperature=0.0
        )
        
        content = response.choices[0].message.content
        
        # Clean up markdown code blocks if present
        if content.startswith("```json"):
            content = content.replace("```json", "", 1)
        if content.startswith("```"):
            content = content.replace("```", "", 1)
        if content.endswith("```"):
            content = content.rsplit("```", 1)[0]
        
        content = content.strip()
        
        # Parse JSON
        try:
            workflow = json.loads(content)
            return workflow
        except json.JSONDecodeError as e:
            logger.error(f"Failed to parse JSON: {e}")
            return {"error": "Invalid JSON from LLM", "raw_content": content}
            
    except Exception as e:
        logger.error(f"OpenAI API Error: {e}")
        return {"error": str(e)}

def mock_llm_generate(prompt: str) -> Dict[str, Any]:
    """Fallback mock generation for testing without API key"""
    return {
        "workflow_name": "Sample Workflow",
        "description": "Mock workflow generated without API key",
        "created_at": datetime.datetime.now().isoformat(),
        "steps": [
            {
                "step_number": 1,
                "action_type": "open",
                "target": {
                    "app": "Excel",
                    "window_title": "Report.xlsx",
                    "control_type": "Window",
                    "element_name": "",
                    "automation_id": ""
                },
                "value": "",
                "description": "Excel 파일 열기"
            },
            {
                "step_number": 2,
                "action_type": "click",
                "target": {
                    "app": "Excel",
                    "window_title": "Report.xlsx",
                    "control_type": "Button",
                    "element_name": "저장",
                    "automation_id": "SaveButton"
                },
                "value": "",
                "description": "저장 버튼 클릭"
            }
        ],
        "metadata": {
            "source_events_count": 2,
            "estimated_duration_seconds": 10
        }
    }

def main():
    parser = argparse.ArgumentParser(description="Generate Workflow from Session")
    parser.add_argument("--db", default="collector.db")
    parser.add_argument("--out", default="generated_workflow.json")
    
    args = parser.parse_args()
    
    conn = connect_db(Path(args.db))
    session = fetch_latest_session_events(conn)
    
    if not session:
        logger.warning("No recent session found.")
        return
        
    logger.info(f"Found session with {len(session)} events.")
    
    prompt = construct_prompt(session)
    print("\n--- Generated Prompt ---\n")
    print(prompt)
    print("\n------------------------\n")
    
    workflow = generate_workflow(prompt)
    
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(workflow, f, ensure_ascii=False, indent=2)
        
    logger.info(f"Generated workflow saved to {args.out}")
    print("\n--- Generated Workflow (JSON) ---\n")
    print(json.dumps(workflow, ensure_ascii=False, indent=2))

if __name__ == "__main__":
    main()
