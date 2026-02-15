#!/usr/bin/env python3
import sys
import shutil
from pathlib import Path

# Add project root to path
sys.path.insert(0, str(Path(__file__).parent.parent))

# Ensure user site packages are in path
import site
import os
try:
    user_site = site.getusersitepackages()
    if user_site not in sys.path:
        sys.path.append(user_site)
except Exception:
    pass

try:
    import jsonschema
    print(f"✅ jsonschema found at {jsonschema.__file__}")
except ImportError as e:
    print(f"❌ jsonschema not found: {e}")
    print(f"sys.path: {sys.path}")

from core.policy.registry import SmartFolderRegistry
from core.policy.engine import PolicyEngine

def test_policy_system():
    print("🧪 Testing Smart Folder Policy System...")
    
    # Setup test path
    config_path = Path("test_smart_folders.json")
    if config_path.exists():
        config_path.unlink()
        
    registry = SmartFolderRegistry(config_path=config_path)
    
    # 1. Test Add
    test_folder = Path("/tmp/test_smart_folder").resolve()
    registry.add_folder(test_folder, label="Test Folder", folder_type="general")
    print("✅ Added folder")
    
    # 2. Verify Persistence
    registry2 = SmartFolderRegistry(config_path=config_path)
    folders = registry2.list_folders()
    assert len(folders) == 1
    assert folders[0]["path"] == str(test_folder)
    print("✅ Persistence verified")
    
    # 3. Test Policy Engine Enforcement
    engine = PolicyEngine.from_file(config_path)
    
    # Check allowed path (inside smart folder)
    allowed_file = test_folder / "doc.txt"
    is_allowed = engine.allows(allowed_file, agent="meeting")
    # By default, new folders allow 'meeting' in our registry default logic
    assert is_allowed, "Should allow file in smart folder"
    print("✅ Inside scope allowed")
    
    # Check denied path (outside)
    outside_file = Path("/tmp/other/doc.txt").resolve()
    is_allowed = engine.allows(outside_file, agent="meeting")
    assert not is_allowed, "Should deny file outside smart folder"
    print("✅ Outside scope denied")
    
    # 4. Test Remove
    registry.remove_folder(test_folder)
    assert len(registry.list_folders()) == 0
    print("✅ Removed folder")
    
    # Cleanup
    if config_path.exists():
        config_path.unlink()
    print("🎉 All tests passed!")

if __name__ == "__main__":
    test_policy_system()
