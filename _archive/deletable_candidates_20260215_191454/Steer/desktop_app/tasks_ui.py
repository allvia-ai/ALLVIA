"""Task Management UI."""
import sys
from pathlib import Path

from PySide6.QtCore import Qt, QTimer, QThread, Signal
from PySide6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QHBoxLayout,
    QPushButton,
    QTableWidget,
    QTableWidgetItem,
    QHeaderView,
    QMenu,
    QMessageBox,
    QWidget,
    QLabel,
    QCheckBox,
    QApplication
)
from PySide6.QtGui import QAction, QColor, QFont

from core.tasks.store import TaskStore, Task, TaskStatus

class TaskLoader(QThread):
    """Background thread to load tasks from DB."""
    tasks_loaded = Signal(list)

    def __init__(self, store):
        super().__init__()
        self.store = store

    def run(self):
        # Allow some time for UI loop to spin if needed, 
        # but mostly just run the query off-thread.
        tasks = self.store.list_tasks()
        # Sort: Pending first
        tasks.sort(key=lambda t: 0 if t.status == TaskStatus.PENDING else 1)
        self.tasks_loaded.emit(tasks)

class TaskManagerWindow(QDialog):
    """Window for managing tasks extracted from meetings."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Task Center")
        self.resize(800, 500)
        self.store = TaskStore()
        # Ensure indexes exist (in case DB was created before update)
        self.store._init_db() 
        self._setup_ui()
        self.refresh_tasks()

    def _setup_ui(self):
        layout = QVBoxLayout(self)
        
        # Header
        header_layout = QHBoxLayout()
        title = QLabel("📝 Action Items & Tasks")
        title.setFont(QFont("Arial", 16, QFont.Weight.Bold))
        header_layout.addWidget(title)
        
        refresh_btn = QPushButton("Refresh")
        refresh_btn.clicked.connect(self.refresh_tasks)
        header_layout.addWidget(refresh_btn)
        
        header_layout.addStretch()
        layout.addLayout(header_layout)

        # Loading Indicator (hidden by default)
        self.loading_label = QLabel("Loading tasks...")
        self.loading_label.setAlignment(Qt.AlignCenter)
        self.loading_label.setStyleSheet("color: gray; font-style: italic;")
        self.loading_label.hide()
        layout.addWidget(self.loading_label)

        # Table
        self.table = QTableWidget()
        self.table.setColumnCount(5)
        self.table.setHorizontalHeaderLabels(["Done", "Source", "Task", "Owner", "Due Date"])
        # PySide6 compatibility for ResizeMode
        header = self.table.horizontalHeader()
        header.setSectionResizeMode(2, QHeaderView.Stretch)
        self.table.setSelectionBehavior(QTableWidget.SelectRows)
        self.table.setContextMenuPolicy(Qt.CustomContextMenu)
        self.table.customContextMenuRequested.connect(self._show_context_menu)
        
        layout.addWidget(self.table)
        
        # Help text
        layout.addWidget(QLabel("Right-click a task to delete."))

    def refresh_tasks(self):
        """Reload tasks from DB in background."""
        self.table.setEnabled(False)
        self.loading_label.show()
        
        self.loader = TaskLoader(self.store)
        self.loader.tasks_loaded.connect(self._on_tasks_loaded)
        self.loader.finished.connect(self.loader.deleteLater)
        self.loader.start()

    def _on_tasks_loaded(self, tasks):
        """Update UI with loaded tasks."""
        self.loading_label.hide()
        self.table.setEnabled(True)
        self.table.setRowCount(0)
        
        self.table.setRowCount(len(tasks))
        for row, task in enumerate(tasks):
            # Checkbox for status
            chk_widget = QWidget()
            chk_layout = QHBoxLayout(chk_widget)
            chk_layout.setContentsMargins(0, 0, 0, 0)
            chk_layout.setAlignment(Qt.AlignCenter)
            checkbox = QCheckBox()
            checkbox.setChecked(task.status == TaskStatus.COMPLETED)
            checkbox.stateChanged.connect(lambda state, t=task: self._toggle_status(t, state))
            chk_layout.addWidget(checkbox)
            self.table.setCellWidget(row, 0, chk_widget)
            
            # Source
            self.table.setItem(row, 1, QTableWidgetItem(task.source_meeting_id or ""))
            
            # Content
            content_item = QTableWidgetItem(task.content)
            if task.status == TaskStatus.COMPLETED:
                font = content_item.font()
                font.setStrikeOut(True)
                content_item.setFont(font)
                content_item.setForeground(QColor("gray"))
            self.table.setItem(row, 2, content_item)
            
            # Owner
            self.table.setItem(row, 3, QTableWidgetItem(task.owner or ""))
            
            # Due Date
            self.table.setItem(row, 4, QTableWidgetItem(task.due_date or ""))
            
            # Store ID in hidden role
            content_item.setData(Qt.UserRole, task.id)

    def _toggle_status(self, task: Task, state: int):
        # 2 is Checked in Qt
        new_status = TaskStatus.COMPLETED if state == 2 else TaskStatus.PENDING 
        self.store.update_task_status(task.id, new_status)
        self.refresh_tasks()

    def _show_context_menu(self, position):
        row = self.table.rowAt(position.y())
        if row < 0:
            return
            
        menu = QMenu()
        delete_action = QAction("Delete Task", self)
        delete_action.triggered.connect(lambda: self._delete_task(row))
        menu.addAction(delete_action)
        menu.exec(self.table.viewport().mapToGlobal(position))

    def _delete_task(self, row):
        task_id = self.table.item(row, 2).data(Qt.UserRole)
        confirm = QMessageBox.question(
            self, "Confirm Delete", "Are you sure you want to delete this task?",
            QMessageBox.Yes | QMessageBox.No
        )
        if confirm == QMessageBox.Yes:
            self.store.delete_task(task_id)
            self.refresh_tasks()

if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = TaskManagerWindow()
    window.show()
    sys.exit(app.exec())
