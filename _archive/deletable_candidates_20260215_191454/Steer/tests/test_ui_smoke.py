
import pytest
import sys
from pathlib import Path

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

try:
    from PySide6.QtWidgets import QApplication
    from desktop_app.tasks_ui import TaskManagerWindow
except ImportError:
    # If PySide6 is not installed or headless limitations prevent import
    pytest.skip("PySide6 not available or headless", allow_module_level=True)

@pytest.mark.skipif(sys.platform == "linux", reason="Requires Wayland/X11 on Linux")
def test_ui_instantiation():
    """Verify TaskManagerWindow can be instantiated without crashing."""
    # We need a qApp instance
    app = QApplication.instance() or QApplication(sys.argv)
    
    # Mocking storage to avoid filesystem issues during test? 
    # Actually TaskStore uses SQLite, which is fine.
    
    try:
        window = TaskManagerWindow()
        assert window is not None
        assert window.windowTitle() == "Task Center"
        window.close()
    except Exception as e:
        pytest.fail(f"UI Instantiation failed: {e}")
