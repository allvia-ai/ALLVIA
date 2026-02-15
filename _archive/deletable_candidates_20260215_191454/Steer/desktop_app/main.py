
import sys
import os
import signal
from PySide6.QtWidgets import QApplication
from PySide6.QtCore import QThread

# Ensure project root is in sys.path
current_dir = os.path.dirname(os.path.abspath(__file__))
project_root = os.path.dirname(current_dir)
if project_root not in sys.path:
    sys.path.insert(0, project_root)

from desktop_app.ui import LauncherWindow
from desktop_app.backend import LNPBackend

def exception_hook(exctype, value, traceback):
    print(f"[CRITICAL_ERROR] {exctype.__name__}: {value}")
    sys.__excepthook__(exctype, value, traceback)

sys.excepthook = exception_hook

def main():
    signal.signal(signal.SIGINT, signal.SIG_DFL)

    app = QApplication(sys.argv)
    app.setQuitOnLastWindowClosed(False)
    
    # Theme Setup
    try:
        import qdarktheme
        if hasattr(qdarktheme, "setup_theme"):
            qdarktheme.setup_theme("dark")
        elif hasattr(qdarktheme, "load_stylesheet"):
            app.setStyleSheet(qdarktheme.load_stylesheet("dark"))
    except Exception:
        pass

    # --- Backend Setup (QThread) ---
    backend_thread = QThread()
    backend_worker = LNPBackend()
    backend_worker.moveToThread(backend_thread)
    
    # Connect signals
    backend_thread.started.connect(backend_worker.initialize)
    
    # Start thread
    backend_thread.start()

    # --- UI Setup ---
    # Pass backend_worker to Window so it can connect signals
    window = LauncherWindow(backend=backend_worker)
    window.show()

    # System Tray
    from PySide6.QtWidgets import QSystemTrayIcon, QMenu
    from PySide6.QtGui import QIcon
    
    tray_icon = QSystemTrayIcon(app)
    icon_path = os.path.join(current_dir, "assets", "logo.png")
    if os.path.exists(icon_path):
        tray_icon.setIcon(QIcon(icon_path))
    else:
        from PySide6.QtWidgets import QStyle
        tray_icon.setIcon(app.style().standardIcon(QStyle.SP_ComputerIcon))
    
    tray_menu = QMenu()
    action_show = tray_menu.addAction("열기")
    action_quit = tray_menu.addAction("종료")
    
    action_show.triggered.connect(window.show_and_activate)
    action_quit.triggered.connect(app.quit)
    
    tray_icon.setContextMenu(tray_menu)
    tray_icon.show()
    
    tray_icon.activated.connect(lambda reason: window.show_and_activate() if reason == QSystemTrayIcon.Trigger else None)

    # Cleanup on exit
    def cleanup():
        print("[App] Shutting down backend thread...")
        backend_thread.quit()
        backend_thread.wait()
        
    app.aboutToQuit.connect(cleanup)

    sys.exit(app.exec())

if __name__ == "__main__":
    main()
