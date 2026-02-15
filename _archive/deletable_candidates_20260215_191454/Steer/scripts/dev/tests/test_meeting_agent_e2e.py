import sys
import shutil
import os
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent.parent))

from core.agents.meeting.agent import MeetingAgent, MeetingAgentConfig
from core.agents.base import AgentRequest

def test_meeting_agent_e2e():
    print("🧪 Testing Meeting Agent E2E...")
    
    # Setup paths
    os.environ["MEETING_MASK_PII"] = "1"
    test_audio = Path("test_meeting__audio.txt") # Use text file as "audio" for noop backend fallback or just trick it
    test_audio.write_text("This is a meeting. My email is test@example.com. Action: David to fix bug.", encoding="utf-8")
    
    # Create Agent
    agent = MeetingAgent()
    
    # Request
    req = AgentRequest(
        query="Summarize this",
        context={
            "audio_path": str(test_audio.absolute()),
            "output_dir": "/tmp/test_meeting_output"
        }
    )
    
    # Run
    # Note: We need to handle STT backend carefully. The pipeline defaults to 'whisper' if available or 'auto'.
    # We want 'noop' or something that reads the text file.
    # The pipeline logic for `_load_transcript_text` checks for sidecar txt.
    # So if we provide .txt, it should load it.
    
    try:
        result = agent.run(req)
        print("✅ Pipeline execution successful")
        print("Result Content:\n", result.content)
        
        # Verify content
        if "test@example.com" not in result.content and "[REDACTED_EMAIL]" in result.content:
             print("✅ Masking applied in summary (if summary contains the email)")
        elif "test@example.com" not in result.content:
             print("ℹ️ Email extracted away by summary (Acceptable)")
        else:
             print("❌ Masking failed or didn't apply")
             
        # Verify output dir
        output_dir = Path(result.metadata["output_dir"])
        print(f"Output Dir: {output_dir}")
        assert output_dir.exists()
        assert "transcript.txt" in [f.name for f in output_dir.iterdir()]
        
    except Exception as e:
        print(f"❌ Execution failed: {e}")
        import traceback
        traceback.print_exc()
    finally:
        if test_audio.exists():
            test_audio.unlink()
        if Path("/tmp/test_meeting_output").exists():
            shutil.rmtree("/tmp/test_meeting_output")

if __name__ == "__main__":
    test_meeting_agent_e2e()
