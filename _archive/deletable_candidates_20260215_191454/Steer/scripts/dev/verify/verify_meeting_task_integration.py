import sys
import shutil
import os
from pathlib import Path
from datetime import datetime

# Set isolated DATA_DIR before importing core modules
TEST_DATA_DIR = Path("/tmp/task_test_data")
if TEST_DATA_DIR.exists():
    shutil.rmtree(TEST_DATA_DIR)
TEST_DATA_DIR.mkdir()
os.environ["INFOPILOT_DATA_DIR"] = str(TEST_DATA_DIR)

sys.path.insert(0, str(Path(__file__).parent.parent))

from core.agents.meeting.agent import MeetingAgent
from core.agents.base import AgentRequest
from core.tasks.store import TaskStore, TaskStatus

def test_integration():
    print("🧪 Testing Meeting -> Task Integration...")
    print(f"Using isolated DATA_DIR: {TEST_DATA_DIR}")

    # 1. Setup Dummy Audio (Text fallback)
    # Ensure content has Action Item format: [Owner] Task (Due: Date)
    test_audio = Path("test_action_audio.txt")
    test_audio.write_text(
        "This is a strategy meeting.\n\n"
        "Results: We need to ship.\n\n"
        "Action: [Alice] Update the website banner (Due: 2025-01-15)\n\n"
        "Action: [Bob] Deploy the backend fix\n\n",
        encoding="utf-8"
    )

    # 2. Run Meeting Agent
    # Enable PII masking via env to prevent warnings, though not strictly needed for this test
    os.environ["MEETING_MASK_PII"] = "1" 
    
    agent = MeetingAgent()
    req = AgentRequest(
        query="Summarize",
        context={
            "audio_path": str(test_audio.absolute()),
            "output_dir": str(TEST_DATA_DIR / "outputs"),
            "speaker_count": 2
        }
    )
    
    try:
        result = agent.run(req)
        print("✅ Meeting Wrapper Finished")
        # print("Result Content:\n", result.content)
        
        # 3. Verify Task Store
        # TaskStore uses DATA_DIR defined at import time. 
        # Since we set env var before import, it should point to TEST_DATA_DIR.
        store = TaskStore()
        tasks = store.list_tasks()
        
        print(f"Found {len(tasks)} tasks in DB.")
        for t in tasks:
            print(f" - [Status: {t.status}] {t.content}")

        # We expect 2 tasks: Alice's and Bob's
        # Note: 'Action:' prefix might be stripped or kept depending on summarizer output.
        # The 'heuristic' summarizer uses regex to find lines starting with Action/Task strings.
        # Let's see what the heuristic summarizer produces.
        
        alice_task = next((t for t in tasks if "Alice" in (t.owner or "")), None)
        bob_task = next((t for t in tasks if "Bob" in (t.owner or "")), None)
        
        if alice_task:
            print(f"✅ Found Alice's Task: {alice_task.content} | Status: {alice_task.status}")
            assert alice_task.owner == "Alice"
            assert alice_task.due_date == "2025-01-15"
        else:
            print("❌ Alice's task not found.")
            
        if bob_task:
             print(f"✅ Found Bob's Task: {bob_task.content}")
             assert bob_task.owner == "Bob"
        else:
             print("❌ Bob's task not found.")
             
        assert len(tasks) >= 2

    except Exception as e:
        print(f"❌ Test Failed: {e}")
        import traceback
        traceback.print_exc()
    finally:
        if test_audio.exists():
            test_audio.unlink()
        # Cleanup
        if TEST_DATA_DIR.exists():
            shutil.rmtree(TEST_DATA_DIR)

if __name__ == "__main__":
    test_integration()
