import sys
import os
from pathlib import Path
import shutil

# Ensure we can import core
project_root = Path(".").resolve()
sys.path.insert(0, str(project_root))

from core.agents.meeting.agent import MeetingAgent
from core.agents import AgentRequest

def setup_test_files():
    base_dir = Path("~/Desktop/AI Summary/녹음").expanduser()
    sensitive_dir = base_dir / "민감"
    
    base_dir.mkdir(parents=True, exist_ok=True)
    sensitive_dir.mkdir(parents=True, exist_ok=True)
    
    # 1. Allowed File
    allowed_file = base_dir / "test_normal_meeting.txt"
    allowed_file.write_text("This is a normal meeting transcript.\nParticipant A: Hello.\nParticipant B: Hi.", encoding="utf-8")
    
    # 2. Denied File
    denied_file = sensitive_dir / "test_secret_meeting.txt"
    denied_file.write_text("This is a TOP SECRET meeting.\nDO NOT PROCESS.", encoding="utf-8")
    
    return allowed_file, denied_file

def run_test():
    print("🚀 Starting Policy Verification Test...")
    allowed_file, denied_file = setup_test_files()
    
    agent = MeetingAgent()
    agent.prepare() # Load policies
    
    # Test 1: Allowed
    print(f"\n[Test 1] Processing Allowed File: {allowed_file.name}")
    try:
        req = AgentRequest(query="summarize", context={"audio_path": str(allowed_file)})
        # We expect this to fail later at 'pipeline' stage because it's a txt file pretending to be audio 
        # OR proceed if pipeline handles sidecar text.
        # But importantly, it should PASS the policy check.
        try:
            agent.run(req)
            print("✅ PASSED: Policy allowed access.")
        except Exception as e:
            if "PermissionError" in str(e):
                print(f"❌ FAILED: Policy incorrectly blocked access! Error: {e}")
            else:
                # If it failed for other reasons (e.g. pipeline error), that's fine for this test
                print(f"✅ PASSED: Policy allowed access (failed later as expected: {type(e).__name__})")
                
    except Exception as e:
        print(f"⚠️ Unexpected error: {e}")

    # Test 2: Denied
    print(f"\n[Test 2] Processing Denied File: {denied_file.name}")
    try:
        req = AgentRequest(query="summarize", context={"audio_path": str(denied_file)})
        agent.run(req)
        print("❌ FAILED: Agent processed a sensitive file! Policy ignoring it.")
    except PermissionError as e:
        print(f"✅ PASSED: Policy correctly blocked access.\n   Error message: {e}")
    except Exception as e:
        print(f"❌ FAILED: Expected PermissionError, got {type(e).__name__}: {e}")

    # Cleanup
    if allowed_file.exists(): allowed_file.unlink()
    if denied_file.exists(): denied_file.unlink()
    print("\nDone.")

if __name__ == "__main__":
    run_test()
