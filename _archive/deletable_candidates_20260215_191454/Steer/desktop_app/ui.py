from PySide6.QtCore import Qt, QSize, Signal, QThread, QTimer, QEvent, QUrl
from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QPushButton, QTextEdit, QListWidget, 
    QListWidgetItem, QLabel, QFrame, QGraphicsDropShadowEffect, QSizePolicy,
    QDialog, QScrollArea, QMessageBox, QFileDialog
)
from PySide6.QtGui import QFont, QColor, QDragEnterEvent, QDropEvent, QDesktopServices
import qdarktheme
import os
import subprocess
from pathlib import Path

# Import Policy Registry
from core.policy.registry import SmartFolderRegistry
from desktop_app.tasks_ui import TaskManagerWindow
from desktop_app.gallery_ui import PhotoGalleryDialog

class EnhancedInput(QTextEdit):
    submit = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setAcceptRichText(False)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        # Default height for approx 1 line
        self.setFixedHeight(50) 
        self.textChanged.connect(self.adjust_height)

    def adjust_height(self):
        # Auto-resize logic (simple version)
        doc_height = self.document().size().height()
        # Clamp height between 50 and 150
        new_height = max(50, min(150, doc_height + 10)) 
        self.setFixedHeight(int(new_height))

    def insertFromMimeData(self, source):
        if source.hasImage():
            return  # Ignore images
        super().insertFromMimeData(source)

    def keyPressEvent(self, event):
        if event.key() == Qt.Key_Return:
            if event.modifiers() & Qt.ShiftModifier:
                # Shift+Enter -> New Line
                self.insertPlainText("\n")
                event.accept()
            else:
                # Enter -> Submit
                event.accept()
                self.submit.emit()
        else:
            # Other keys -> Default behavior
            super().keyPressEvent(event)


# QueryWorker removed (Using QThread Backend)

class SmartFolderManagerDialog(QDialog):
    def __init__(self, registry, parent=None):
        super().__init__(parent)
        self.registry = registry
        self.setWindowTitle("Smart Folders")
        self.resize(500, 400)
        self.setWindowFlags(Qt.Dialog | Qt.CustomizeWindowHint | Qt.WindowTitleHint | Qt.WindowCloseButtonHint)
        
        layout = QVBoxLayout(self)
        layout.setSpacing(10)
        
        # Title
        title = QLabel("Manage Smart Folders")
        title.setStyleSheet("font-size: 18px; font-weight: bold; color: #fff;")
        layout.addWidget(title)
        
        # List Area
        self.scroll_area = QScrollArea()
        self.scroll_area.setWidgetResizable(True)
        self.scroll_area.setStyleSheet("background: transparent; border: none;")
        self.scroll_content = QWidget()
        self.scroll_layout = QVBoxLayout(self.scroll_content)
        self.scroll_layout.setAlignment(Qt.AlignTop)
        self.scroll_area.setWidget(self.scroll_content)
        layout.addWidget(self.scroll_area)
        
        self.refresh_list()
        
        # Close Button
        btn_close = QPushButton("Close")
        btn_close.clicked.connect(self.accept)
        btn_close.setStyleSheet("""
            QPushButton {
                background-color: #333; color: white; border-radius: 6px; padding: 8px;
            }
            QPushButton:hover { background-color: #444; }
        """)
        layout.addWidget(btn_close)
        
        # Apply Dark Theme to dialog as well (manually or inherit)
        self.setStyleSheet("background-color: #1e1e1e; color: #fff;")

    def refresh_list(self):
        # Clear existing
        for i in reversed(range(self.scroll_layout.count())):
            self.scroll_layout.itemAt(i).widget().setParent(None)
            
        folders = self.registry.list_folders()
        if not folders:
            lbl = QLabel("No smart folders registered.\nDrag and drop a folder to the main window to add.")
            lbl.setStyleSheet("color: #888; padding: 20px;")
            lbl.setAlignment(Qt.AlignCenter)
            self.scroll_layout.addWidget(lbl)
            return

        for folder in folders:
            row = QFrame()
            row.setStyleSheet("background-color: #2a2a2a; border-radius: 8px; padding: 8px;")
            row_layout = QHBoxLayout(row)
            
            info_layout = QVBoxLayout()
            lbl_name = QLabel(folder.get('label', 'Unknown'))
            lbl_name.setStyleSheet("font-weight: bold; font-size: 14px;")
            lbl_path = QLabel(folder.get('path', ''))
            lbl_path.setStyleSheet("color: #aaa; font-size: 12px;")
            
            info_layout.addWidget(lbl_name)
            info_layout.addWidget(lbl_path)
            
            btn_remove = QPushButton("Remove")
            btn_remove.setCursor(Qt.PointingHandCursor)
            btn_remove.setStyleSheet("background-color: #8B0000; color: white; border-radius: 4px; padding: 4px 8px;")
            btn_remove.clicked.connect(lambda checked=False, p=folder.get('path'): self.remove_folder(p))
            
            row_layout.addLayout(info_layout, stretch=1)
            row_layout.addWidget(btn_remove, stretch=0)
            
            self.scroll_layout.addWidget(row)

    def remove_folder(self, path):
        if self.registry.remove_folder(Path(path)):
            self.refresh_list()

class LauncherWindow(QWidget):
    query_requested = Signal(str) # Signal to send query to backend

    def __init__(self, backend=None):
        super().__init__()
        
        self.backend = backend
        
        # Connect Signals if backend provided
        if self.backend:
            # UI -> Backend
            self.query_requested.connect(self.backend.handle_query)
            
            # Backend -> UI
            self.backend.response_ready.connect(self.handle_response)
            self.backend.status_msg.connect(self.update_status_msg)
            self.backend.error_occurred.connect(self.handle_error)
        
        # 1. Window Setup
        self.setWindowFlags(
            Qt.FramelessWindowHint | 
            Qt.WindowStaysOnTopHint
            # Qt.Tool removed because it can cause auto-hiding on macOS
        )
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.setAcceptDrops(True)  # Enable Drag & Drop
        
        # Initial geometry
        self.resize(600, 140) 
        
        # 2. UI Layout & Styling
        self.setup_ui()
        self.setup_styles()
        
        # 3. Logic: Connected via Signals
        self.policy_registry = SmartFolderRegistry()
        
        # 4. Animation Timer
        self.thinking_timer = QTimer(self)
        self.thinking_timer.timeout.connect(self.update_thinking_text)
        self.thinking_dots = 0
        
        # Dragging variables
        self.old_pos = None
        self.streaming_item = None  # Track the current streaming message

    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.old_pos = event.globalPos()
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event):
        if self.old_pos and event.buttons() == Qt.LeftButton:
            delta = event.globalPos() - self.old_pos
            self.move(self.pos() + delta)
            self.old_pos = event.globalPos()
        super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.old_pos = None
        super().mouseReleaseEvent(event)

    def setup_ui(self):
        # Main Layout (Padding around the window)
        self.layout = QVBoxLayout(self)
        self.layout.setContentsMargins(10, 10, 10, 10)
        
        # Container Frame (The main visual element)
        self.container = QFrame()
        self.container.setObjectName("Container")
        self.container_layout = QVBoxLayout(self.container)
        # More padding for that "spacious" premium feel
        self.container_layout.setContentsMargins(24, 24, 24, 16)
        self.container_layout.setSpacing(12)
        
        # Result Area (Top)
        self.result_list = QListWidget()
        self.result_list.setObjectName("ResultList")
        self.result_list.setVisible(False)
        # Let list grow, but minimum size
        self.result_list.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Expanding)
        # Enable word wrap for long messages
        self.result_list.setWordWrap(True)
        self.result_list.setTextElideMode(Qt.ElideNone)
        # Enable clicking on items to open documents
        self.result_list.itemClicked.connect(self.on_result_item_clicked)
        
        # Input Field (EnhancedTextEdit)
        self.input_field = EnhancedInput()
        self.input_field.setPlaceholderText("무엇이든 부탁하세요")
        self.input_field.setObjectName("InputField")
        self.input_field.submit.connect(self.on_submit)
        
        # Toolbar (Icons)
        self.toolbar_layout = QHBoxLayout()
        self.toolbar_layout.setSpacing(18) # Space between icons
        self.toolbar_layout.setAlignment(Qt.AlignLeft)
        
        def create_tool_btn(icon_text, tooltip, callback, object_name="ToolBtn"):
            btn = QPushButton(icon_text)
            btn.setToolTip(tooltip)
            btn.setFlat(True)
            btn.setCursor(Qt.PointingHandCursor)
            btn.setObjectName(object_name)
            btn.clicked.connect(callback)
            return btn
            
        # Define actions with slightly different icons to match reference closer
        self.btn_add = create_tool_btn("➕", "첨부", lambda: self.input_field.setPlainText(self.input_field.toPlainText() + " [첨부] "))
        self.btn_web = create_tool_btn("🌐", "웹 검색", lambda: self.input_field.setPlainText("@검색 "))
        self.btn_photo = create_tool_btn("📸", "사진 폴더 분석", self.open_photo_dialog)
        self.btn_mic = create_tool_btn("🎙️", "회의 전사", self.open_meeting_dialog)
        self.btn_settings = create_tool_btn("⚙️", "설정 (스마트 폴더)", self.open_settings)
        self.btn_tasks = create_tool_btn("📋", "Task Center", self.open_tasks)
        
        self.toolbar_layout.addWidget(self.btn_add)
        self.toolbar_layout.addWidget(self.btn_web)
        self.toolbar_layout.addWidget(self.btn_photo)
        self.toolbar_layout.addWidget(self.btn_mic)
        self.toolbar_layout.addStretch()
        self.toolbar_layout.addWidget(self.btn_tasks)
        self.toolbar_layout.addWidget(self.btn_settings)
        
        # Status Label (Bottom Right, very subtle)
        self.status_label = QLabel("Ready")
        self.status_label.setObjectName("StatusLabel")
        self.status_label.setAlignment(Qt.AlignRight)
        
        # [Layout Order]:
        # 1. Result List (expands)
        # 2. Input Field (auto-height)
        # 3. Toolbar
        # 4. Status
        self.container_layout.addWidget(self.result_list, stretch=1) 
        self.container_layout.addWidget(self.input_field, stretch=0)
        self.container_layout.addLayout(self.toolbar_layout)
        self.container_layout.addWidget(self.status_label, alignment=Qt.AlignRight)
        
        # Shadow Effect (Stronger, softer shadow)
        shadow = QGraphicsDropShadowEffect(self)
        shadow.setBlurRadius(40)
        shadow.setColor(QColor(0, 0, 0, 160))
        shadow.setOffset(0, 10)
        self.container.setGraphicsEffect(shadow)
        
        self.layout.addWidget(self.container)

    def setup_styles(self):
        # Cycle 2: Premium Visuals (Deep Dark + Glassmorphism-ish)
        self.setStyleSheet("""
            QWidget {
                font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
                font-size: 15px;
                color: #FFFFFF;
                selection-background-color: #10a37f; /* OpenAI Green for selection */
                selection-color: #FFFFFF;
            }
            
            /* Main Container: Deep Dark with slight transparency */
            #Container {
                background-color: rgba(30, 30, 30, 0.96); 
                border-radius: 16px;
                border: 1px solid rgba(255, 255, 255, 0.08);
            }
            
            /* Input: Large, Clean, No Border */
            QTextEdit#InputField {
                background-color: transparent;
                border: none;
                font-size: 16px; /* Optimized for readability */
                line-height: 1.5;
                font-weight: 400;
                color: #ECECEC;
                padding-top: 8px;
            }
            
            /* Result List: Chat Bubble style separation */
            QListWidget#ResultList {
                background-color: transparent;
                border: none;
                outline: none;
                margin-bottom: 12px;
            }
            QListWidget::item {
                padding: 4px 0px; 
                border: none;
            }
            /* We will use HTML formatting in add_message for better looking bubbles later,
               but for now, clean list items */
               
            /* Toolbar Buttons: Minimalist */
            QPushButton {
                background: transparent;
                border: none;
                border-radius: 8px;
                padding: 6px;
                font-size: 18px; /* Emoji/Icon size */
                color: #8E8E93;
            }
            QPushButton:hover {
                background-color: rgba(255, 255, 255, 0.08); /* Subtle hover */
                color: #FFFFFF;
            }
            QPushButton:pressed {
                background-color: rgba(255, 255, 255, 0.12);
            }
            
            /* Status Label */
            QLabel#StatusLabel {
                font-size: 12px;
                color: #6e6e73; 
                margin-top: 4px;
                font-weight: 500;
            }
            
            /* Scrollbar Styling (Hidden or Minimal) */
            QScrollBar:vertical {
                border: none;
                background: transparent;
                width: 8px;
                margin: 0px;
            }
            QScrollBar::handle:vertical {
                background: rgba(255, 255, 255, 0.2);
                min-height: 20px;
                border-radius: 4px;
            }
            QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
                height: 0px;
            }
        """)

    def on_submit(self):
        query = self.input_field.toPlainText().strip()
        if not query:
            return
            
        self.input_field.clear()
        self.input_field.setFixedHeight(50) # Reset height
        self.expand_window()
        self.add_message("Me", query) # Changed 'User' to 'Me' for cleaner look
        
        # Start Thinking
        self.thinking_dots = 0
        self.status_label.setText("Thinking")
        self.status_label.setStyleSheet("color: #10a37f;") # Green pulse
        self.thinking_timer.start(500)
        
        # Reset current streaming item
        self.streaming_item = None
        
        # Send Query via Signal (Thread-safe)
        self.query_requested.emit(query)

    def update_thinking_text(self):
        self.thinking_dots = (self.thinking_dots + 1) % 4
        dots = "·" * (self.thinking_dots + 1)
        # Bumping text to creating a visual pulsing effect via text implementation
        self.status_label.setText(f"Thinking {dots}")

    def handle_stream_update(self, chunk):
        # Called when a new chunk of text arrives from backend
        self.status_label.setText("Typing...")
        
        if self.streaming_item is None:
            self.streaming_item = QListWidgetItem("AI: ")
            self.result_list.addItem(self.streaming_item)
            self.result_list.scrollToBottom()
            
        current_text = self.streaming_item.text()
        self.streaming_item.setText(current_text + chunk)
        self.result_list.scrollToBottom()

    def handle_response(self, response):
        self.thinking_timer.stop()
        self.status_label.setText("Ready")
        self.status_label.setStyleSheet("color: #6e6e73;")
        
        # Parse file links from response
        import re
        file_link_pattern = r'\[FILE_LINK:([^\]]+)\]'
        file_links = re.findall(file_link_pattern, response)
        # Remove file link markers from display text
        clean_response = re.sub(file_link_pattern, '', response)
        
        # If we were streaming, the item already exists and has content.
        # Just ensure the final text is correct and we are done.
        if self.streaming_item:
            self.streaming_item.setText(f"AI: {clean_response}")
            self.result_list.scrollToBottom()
            self.streaming_item = None # Reset
            # Add clickable file links
            for file_path in file_links:
                self.add_message("📄", os.path.basename(file_path), file_path=file_path)
            return

        # Start Typewriter Effect (Fallback for non-streaming responses)
        self.typewriter_text = clean_response
        self.typewriter_file_links = file_links  # Store for after typewriter
        self.typewriter_index = 0
        self.typewriter_item = QListWidgetItem("AI: ") # Placeholder
        self.result_list.addItem(self.typewriter_item)
        self.result_list.scrollToBottom()
        
        self.typewriter_timer = QTimer(self)
        self.typewriter_timer.timeout.connect(self.update_typewriter)
        self.typewriter_timer.start(15) # Fast typing speed

    def update_typewriter(self):
        if self.typewriter_index < len(self.typewriter_text):
            chunk_size = 3 # Add few chars at once for speed
            chunk = self.typewriter_text[self.typewriter_index:self.typewriter_index+chunk_size]
            current_text = self.typewriter_item.text()
            self.typewriter_item.setText(current_text + chunk)
            self.typewriter_index += chunk_size
            self.result_list.scrollToBottom()
        else:
            self.typewriter_timer.stop()
            # Add clickable file links after typewriter finishes
            if hasattr(self, 'typewriter_file_links'):
                for file_path in self.typewriter_file_links:
                    self.add_message("📄", os.path.basename(file_path), file_path=file_path)
                self.typewriter_file_links = []
        
    def expand_window(self):
        if not self.result_list.isVisible():
            self.result_list.setVisible(True)
            self.resize(600, 500)
            
    def update_status_msg(self, msg):
        self.status_label.setText(msg)
        if "Error" in msg:
            self.status_label.setStyleSheet("color: red;")
        elif "Thinking" in msg or "Loading" in msg:
            self.status_label.setStyleSheet("color: #10a37f;")
        else:
            self.status_label.setStyleSheet("color: #6e6e73;")
            
    def handle_error(self, msg):
        self.thinking_timer.stop()
        self.status_label.setText("Error")
        self.add_message("System", f"Error: {msg}")

    def add_message(self, sender, text, file_path=None):
        """Add a message to the result list. If file_path is provided, item is clickable."""
        formatted_text = f"{sender}: {text}"
        item = QListWidgetItem(formatted_text)
        if file_path:
            item.setData(Qt.UserRole, file_path)  # Store path for click handler
            item.setToolTip(f"Click to open: {file_path}")
        self.result_list.addItem(item)
        self.result_list.scrollToBottom()

    def on_result_item_clicked(self, item):
        """Handle click on result items to open documents."""
        file_path = item.data(Qt.UserRole)
        if file_path and os.path.exists(file_path):
            # Open file with default application
            if os.name == 'nt':  # Windows
                os.startfile(file_path)
            elif os.name == 'posix':  # macOS/Linux
                subprocess.run(["open", file_path], check=False)

    def cleanup(self):
        """Called when application is quitting"""
        # Backend thread is cleaned up in main.py
        pass

    def keyPressEvent(self, event):
        if event.key() == Qt.Key_Escape:
            self.hide() # Cycle 5: Hide instead of close
        elif (event.modifiers() == Qt.ControlModifier) and (event.key() == Qt.Key_C):
             # Optional: Allow Ctrl+C inside window to copy if text encoded?
             # For now let default handle or pass
             super().keyPressEvent(event)
        else:
            super().keyPressEvent(event)



    def closeEvent(self, event):
        # Cycle 5: Minimize to tray on close
        event.ignore()
        self.hide()

    def show_and_activate(self):
        self.show()
        self.raise_()
        self.activateWindow()
        self.input_field.setFocus()

    def dragEnterEvent(self, event: QDragEnterEvent):
        if event.mimeData().hasUrls():
            event.acceptProposedAction()

    def dropEvent(self, event: QDropEvent):
        urls = event.mimeData().urls()
        for url in urls:
            path_str = url.toLocalFile()
            path = Path(path_str)
            if path.is_dir():
                # Register as Smart Folder
                self.policy_registry.add_folder(path)
                self.status_label.setText(f"Active Folder: {path.name}")
                self.status_label.setStyleSheet("color: #10a37f;")
                QTimer.singleShot(3000, lambda: self.status_label.setText("Ready"))
                # Ideally show a dialog confirmation or visual feedback
                self.open_settings() # Auto open settings to show it added
            elif path.is_file():
                # Handle file drop (transcribe etc - existing functionality could go here)
                self.input_field.setPlainText(self.input_field.toPlainText() + f' "{path_str}" ')

    def open_settings(self):
        dlg = SmartFolderManagerDialog(self.policy_registry, self)
        dlg.exec()

    def open_tasks(self):
        dlg = TaskManagerWindow(self)
        dlg.exec()

    def open_meeting_dialog(self):
        """Open file dialog to select audio file for meeting transcription."""
        file_path, _ = QFileDialog.getOpenFileName(
            self,
            "오디오 파일 선택",
            str(Path.home()),
            "Audio Files (*.mp3 *.wav *.m4a *.ogg *.flac *.webm);;All Files (*)"
        )
        if file_path:
            self.input_field.setPlainText(f'/meeting "{file_path}"')
            self.on_submit()

    def open_photo_dialog(self):
        """Open photo gallery dialog."""
        dialog = PhotoGalleryDialog(parent=self)
        dialog.exec()
