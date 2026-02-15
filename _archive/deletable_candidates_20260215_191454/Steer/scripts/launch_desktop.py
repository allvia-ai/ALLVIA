import os
import subprocess
import time
import threading
import sys
import webbrowser

try:
    import webview  # type: ignore
except Exception:  # pragma: no cover - optional dependency
    webview = None

streamlit_process = None


def run_tkinter_app() -> None:
    """Launch the CustomTkinter desktop app directly (main-thread safe)."""
    scripts_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(scripts_dir)
    sys.path.insert(0, project_root)

    try:
        import customtkinter as ctk  # type: ignore
        ctk.set_appearance_mode("System")
        ctk.set_default_color_theme("dark-blue")
    except ModuleNotFoundError:
        print("customtkinter 패키지를 찾을 수 없습니다. `bash scripts/setup_env.sh` 실행 후 다시 시도하세요.")
        raise
    except Exception:
        pass

    try:
        from ui.app import App  # pylint: disable=import-error
    except ModuleNotFoundError as exc:
        if exc.name == "customtkinter":
            print("customtkinter 패키지를 찾을 수 없습니다. `bash scripts/setup_env.sh` 실행 후 다시 시도하세요.")
        raise

    app = App()
    app.mainloop()


def run_streamlit() -> None:
    """Runs the Streamlit app in a subprocess."""
    global streamlit_process
    scripts_dir = os.path.dirname(os.path.abspath(__file__))
    project_root = os.path.dirname(scripts_dir)
    app_path = os.path.join(project_root, "ui", "app.py")
    command = ["streamlit", "run", app_path, "--server.headless", "true"]

    creationflags = 0
    if sys.platform == "win32":
        creationflags = subprocess.CREATE_NO_WINDOW

    streamlit_process = subprocess.Popen(
        command,
        cwd=project_root,
        creationflags=creationflags,
        start_new_session=(sys.platform != "win32"),
    )

def kill_streamlit():
    """Terminates the Streamlit server process."""
    global streamlit_process
    if streamlit_process and streamlit_process.poll() is None:
        print("Terminating Streamlit server...")
        if sys.platform == "win32":
            # Use taskkill on Windows to forcefully terminate the process tree
            # Redirect output to DEVNULL to suppress success messages
            subprocess.call(
                ['taskkill', '/F', '/T', '/PID', str(streamlit_process.pid)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
        else:
            import signal
            try:
                os.killpg(os.getpgid(streamlit_process.pid), signal.SIGTERM)
            except ProcessLookupError:
                pass
        streamlit_process = None
        print("Streamlit server terminated.")

if __name__ == '__main__':
    if sys.platform == "darwin" and os.environ.get("FORCE_STREAMLIT") != "1":
        run_tkinter_app()
    else:
        streamlit_thread = threading.Thread(target=run_streamlit, daemon=True)
        streamlit_thread.start()

        print("Starting Streamlit server, please wait...")
        time.sleep(8)

        target_url = 'http://localhost:8501'

        if webview:
            print("Opening application window.")
            webview.create_window(
                'InfoPilot',
                target_url,
                width=1280,
                height=800,
            )
            try:
                webview.start()
            finally:
                kill_streamlit()
        else:
            print(f"Open your browser to {target_url}. Press Ctrl+C to stop.")
            webbrowser.open(target_url, new=2, autoraise=True)
            try:
                while streamlit_process and streamlit_process.poll() is None:
                    time.sleep(1)
            except KeyboardInterrupt:
                pass
            finally:
                kill_streamlit()
