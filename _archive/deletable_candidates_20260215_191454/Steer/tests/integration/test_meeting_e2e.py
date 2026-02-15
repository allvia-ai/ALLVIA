import pytest
import json
from pathlib import Path
from unittest.mock import MagicMock, patch
from core.agents.meeting.pipeline import MeetingPipeline, MeetingJobConfig
from core.agents.meeting.models import MeetingTranscriptionResult, MeetingSummary
from core.tasks.store import TaskStore

@pytest.fixture
def mock_stt_backend():
    backend = MagicMock()
    # Mock behavior: return a fixed transcript with Korean text
    # The pipeline expects a TranscriptionPayload-like object (or whatever stt.transcribe returns)
    # Let's ensure all fields are concrete types
    payload = MagicMock()
    payload.text = "이것은 테스트 회의입니다. 중요한 결정이 있었습니다. 데이비드는 문서를 완료해야 합니다."
    payload.language = "ko"
    payload.duration_seconds = 6.0
    payload.segments = [
        {"start": 0.0, "end": 2.0, "speaker": "A", "text": "이것은 테스트 회의입니다."},
        {"start": 2.0, "end": 4.0, "speaker": "B", "text": "중요한 결정이 있었습니다."},
        {"start": 4.0, "end": 6.0, "speaker": "A", "text": "데이비드는 문서를 완료해야 합니다."},
    ]
    backend.transcribe.return_value = payload
    return backend

@pytest.fixture
def mock_summariser():
    summariser = MagicMock()
    # Mock summary output in Korean structure
    summary_text = """
    ## Highlights
    - 테스트 회의 진행

    ## Decisions
    - 결정 사항 없음

    ## Action Items
    - [David] 문서 완료 (Due: TBD)

    ## Summary
    이것은 테스트 회의 요약입니다.
    """
    summariser.summarise.return_value = summary_text
    return summariser

@pytest.fixture
def mock_policy_engine_permissive():
    engine = MagicMock()
    engine.allows.return_value = True
    # check returns (allowed: bool, reason: str)
    engine.check.return_value = (True, "")
    return engine

@pytest.mark.smoke
def test_meeting_pipeline_e2e(tmp_path, mock_stt_backend, mock_summariser):
    # Setup paths
    audio_path = tmp_path / "test_audio.mp3"
    audio_path.touch()
    output_dir = tmp_path / "output"
    output_dir.mkdir()
    
    # Setup job config
    job = MeetingJobConfig(
        audio_path=audio_path,
        output_dir=output_dir,
        language="ko",
        policy_tag="internal"
    )
    
    # Mock dependencies
    with patch("core.agents.meeting.pipeline.create_stt_backend", return_value=mock_stt_backend), \
         patch("core.agents.meeting.pipeline.create_summary_backend", return_value=mock_summariser), \
         patch("core.agents.meeting.pipeline.load_provider_config", return_value=None), \
         patch("core.agents.meeting.pipeline.MeetingContextStore") as MockStore, \
         patch("core.agents.meeting.pipeline.MeetingAnalyticsRecorder") as MockRecorder, \
         patch("core.agents.meeting.pipeline.MeetingAuditLogger") as MockLogger:

        MockStore.from_env.return_value.is_enabled.return_value = False
        MockRecorder.return_value = MagicMock() # Ensure recorder is a mock that doesn't crash on serialization if it were to save something, but here it shouldn't matter as much as the result
        MockLogger.from_env.return_value.is_enabled.return_value = False
        
        # Initialize Pipeline
        pipeline = MeetingPipeline(stt_backend="mock", summary_backend="mock")
        pipeline._stt = mock_stt_backend
        pipeline._summariser = mock_summariser
        
        # Run Pipeline
        summary = pipeline.run(job)
        
        # Verify STT called
        mock_stt_backend.transcribe.assert_called()
        
        # Verify Summariser called
        mock_summariser.summarise.assert_called()
        
        # Verify Output Files
        assert (output_dir / "transcript.txt").exists()
        assert (output_dir / "summary.md").exists()
        
        assert summary.structured_summary["action_items"]
        assert "David" in str(summary.structured_summary["action_items"])
        
@pytest.mark.smoke
def test_agent_saves_tasks(tmp_path, mock_stt_backend, mock_summariser, mock_policy_engine_permissive):
    from core.agents.meeting.agent import MeetingAgent
    from core.agents.base import AgentRequest
    
    audio_path = tmp_path / "agent_test.mp3"
    audio_path.touch()
    
    # We need to mock TaskStore to avoid touching real DB
    with patch("core.agents.meeting.agent.TaskStore") as MockTaskStoreClass, \
         patch("core.agents.meeting.pipeline.create_stt_backend", return_value=mock_stt_backend), \
         patch("core.agents.meeting.pipeline.create_summary_backend", return_value=mock_summariser):
         
        mock_db = MockTaskStoreClass.return_value
        
        # Inject policy engine to avoid permission errors
        agent = MeetingAgent(policy_engine=mock_policy_engine_permissive)
        
        req = AgentRequest(query="요약해줘", context={"audio_path": str(audio_path), "output_dir": str(tmp_path)})
        
        result = agent.run(req)
        
        # Check if TaskStore.add_task was called
        items_count = mock_db.add_task.call_count
        assert items_count >= 1, "Agent should verify action items and save to store"
