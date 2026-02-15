from datetime import datetime
from pathlib import Path

def test_directory_logic():
    print("🧪 Testing Directory Logic...")
    
    audio_path = Path("/Users/david/Downloads/Marketing Meeting 2025.mp3")
    output_root = Path("/tmp/ami_outputs")
    
    # Logic extracted from agent.py
    date_str = datetime.now().strftime("%Y-%m-%d")
    safe_title = "".join(c for c in audio_path.stem if c.isalnum() or c in (' ', '_', '-')).strip().replace(' ', '_')
    output_dir = output_root / f"{date_str}_{safe_title}"
    
    print(f"Generated Path: {output_dir}")
    
    expected_start = f"/tmp/ami_outputs/{date_str}_Marketing_Meeting_2025"
    assert str(output_dir) == expected_start
    
    print("✅ Directory logic verified.")

if __name__ == "__main__":
    test_directory_logic()
