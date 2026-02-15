
import sys
from pathlib import Path

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from core.conversation.prompting import MemoryStore, ChatTurn

def test_memory_history():
    mem = MemoryStore(capacity=5)
    mem.add_turn("user", "Hello")
    mem.add_turn("assistant", "Hi there")
    
    history = mem.build_prompt_history()
    print("History:\n", history)
    
    assert "User: Hello" in history
    assert "Assistant: Hi there" in history
    
    print("Test Passed!")

if __name__ == "__main__":
    test_memory_history()
