import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent.parent))

from core.agents.meeting.masking_patterns import mask_content

def test_masking():
    sample_text = """
    Please contact me at test.user@example.com or call 010-1234-5678.
    My ID is 900101-1234567.
    Billing: 1234-5678-1234-5678.
    """
    
    masked = mask_content(sample_text)
    print("Original:\n", sample_text)
    print("\nMasked:\n", masked)
    
    assert "[EMAIL_MASKED]" in masked
    assert "test.user@example.com" not in masked
    assert "[PHONE_MASKED]" in masked
    assert "010-1234-5678" not in masked
    assert "[RRN_MASKED]" in masked
    assert "900101-1234567" not in masked
    assert "[CREDIT_CARD_MASKED]" in masked
    assert "1234-5678-1234-5678" not in masked
    
    print("\n✅ All masking patterns verified successfully.")

if __name__ == "__main__":
    test_masking()
