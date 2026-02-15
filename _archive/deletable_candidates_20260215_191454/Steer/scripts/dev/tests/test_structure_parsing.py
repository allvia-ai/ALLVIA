import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent.parent))

from core.agents.meeting.pipeline import MeetingPipeline

def test_structure_parsing():
    print("🧪 Testing Structure Parsing...")
    
    pipeline = MeetingPipeline(stt_backend="noop", summary_backend="noop")
    
    mock_llm_output = """
    Here is the summary.
    
    ## Highlights
    - Key point A
    - Key point B
    
    ## Action Items
    - [David] Fix the bug (Due: 2025-12-25)
    - [Team] Deploy to prod
    
    ## Decisions
    - Approved budget
    """
    
    parsed = pipeline._parse_and_merge_structure(mock_llm_output)
    
    print("Parsed Result:", parsed)
    
    assert len(parsed["highlights"]) == 2
    assert parsed["highlights"][0]["text"] == "Key point A"
    
    assert len(parsed["action_items"]) == 2
    assert parsed["action_items"][0]["text"] == "[David] Fix the bug (Due: 2025-12-25)"
    
    assert len(parsed["decisions"]) == 1
    assert parsed["decisions"][0]["text"] == "Approved budget"
    
    print("✅ Structure parsing verified.")

if __name__ == "__main__":
    test_structure_parsing()
