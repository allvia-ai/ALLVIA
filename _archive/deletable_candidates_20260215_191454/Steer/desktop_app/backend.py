
# desktop_app/backend.py
"""
Backend logic for the Desktop App, running in a separate QThread.
Directly imports core modules instead of using subprocess IPC.
"""
import sys
import os
from pathlib import Path
from PySide6.QtCore import QObject, Signal, Slot
from typing import Optional

# Core Imports
# Ensure core is in path
current_dir = os.path.dirname(os.path.abspath(__file__))
project_root = os.path.dirname(current_dir)
if project_root not in sys.path:
    sys.path.insert(0, project_root)

from core.conversation.lnp_chat import LNPChat
from core.conversation.retrieval_strategy import init_llm_client

class LNPBackend(QObject):
    """
    Worker that runs in a background thread.
    Handles Model Loading and Chat Interactions.
    """
    # Signals
    ready = Signal()                # Backend initialized
    response_ready = Signal(str)    # Final response text
    stream_update = Signal(str)     # Streaming chunk (if supported later)
    error_occurred = Signal(str)    # Error message
    status_msg = Signal(str)        # "Loading model...", "Thinking..."

    def __init__(self):
        super().__init__()
        self.chat: Optional[LNPChat] = None
        self._is_loading = False

    @Slot()
    def initialize(self):
        """Initializes the heavy LNPChat components."""
        if self.chat:
            self.ready.emit()
            return

        self._is_loading = True
        self.status_msg.emit("Initializing AI Core...")
        
        try:
            # 1. Initialize LLM (Ensure GPU is used via env var or default)
            # Fix: Explicitly ensure GPU layers setting here if needed, 
            # though lnp_chat.py now defaults to -1.
            
            # 2. Create LNPChat Instance
            # Use absolute paths based on project_root
            project_path = Path(project_root)
            self.chat = LNPChat(
                model_path=project_path / "data/topic_model.joblib",
                corpus_path=project_path / "data/corpus.parquet",
                cache_dir=project_path / "data/cache",
                llm_model=str(project_path / "models/gguf/gemma-3-4b-it-Q4_K_M.gguf")
            )
            
            # 3. Build Retriever & LLM
            # LNPChat.__post_init__ calls _reset_llm_client but we might want explicit control.
            # Using .ready() triggers retriever build
            self.status_msg.emit("Loading Search Index...")
            self.chat.ready(rebuild=False)
            
            self.status_msg.emit("Ready")
            self.ready.emit()
            
        except Exception as e:
            self.error_occurred.emit(f"Initialization Failed: {str(e)}")
            import traceback
            traceback.print_exc()
        finally:
            self._is_loading = False

    @Slot(str)
    def handle_query(self, query: str):
        """Process a user query."""
        if not self.chat:
            self.error_occurred.emit("Backend not initialized.")
            return

        try:
            self.status_msg.emit("Thinking...")
            
            # Using chat.ask() which is the public API we verified
            response_dict = self.chat.ask(query)
            
            # Extract text answer
            answer = response_dict.get("answer", "")
            
            # Extract document hits for clickable links
            hits = response_dict.get("hits", [])
            if hits:
                answer += "\n\n📎 참조 문서:"
                for hit in hits[:5]:  # Limit to top 5
                    path = hit.get("path", hit.get("file_path", ""))
                    title = hit.get("title", hit.get("filename", path.split("/")[-1] if path else "Unknown"))
                    if path:
                        answer += f"\n• {title}"
                        # Format: [FILE_LINK:path] for UI to parse
                        answer += f" [FILE_LINK:{path}]"
            
            # Handle suggestions if present
            suggestions = response_dict.get("suggestions", [])
            if suggestions:
                answer += "\n\n(Tip: " + ", ".join(suggestions) + ")"
            
            self.response_ready.emit(answer)
            self.status_msg.emit("Ready")
            
        except Exception as e:
            self.error_occurred.emit(f"Error: {str(e)}")
            self.status_msg.emit("Error")
            import traceback
            traceback.print_exc()
