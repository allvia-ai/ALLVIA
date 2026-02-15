"""Photo Gallery UI - Thumbnail grid viewer for photo analysis results."""
from __future__ import annotations

import os
import subprocess
from pathlib import Path
from typing import List, Optional, Set

from PySide6.QtCore import Qt, QSize, Signal
from PySide6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QGridLayout, QPushButton, 
    QLabel, QScrollArea, QWidget, QFileDialog, QFrame, QLineEdit
)
from PySide6.QtGui import QPixmap, QFont, QCursor


class PhotoThumbnail(QFrame):
    """Clickable photo thumbnail with status icons."""
    
    clicked = Signal(Path)
    
    THUMBNAIL_SIZE = 120
    
    def __init__(
        self, 
        path: Path, 
        is_best: bool = False, 
        is_duplicate: bool = False,
        parent=None
    ):
        super().__init__(parent)
        self.path = path
        self.is_best = is_best
        self.is_duplicate = is_duplicate
        
        self.setup_ui()
        self.setCursor(QCursor(Qt.PointingHandCursor))
        
    def setup_ui(self):
        layout = QVBoxLayout(self)
        layout.setContentsMargins(4, 4, 4, 4)
        layout.setSpacing(2)
        
        # Thumbnail container
        thumb_container = QFrame()
        thumb_container.setFixedSize(self.THUMBNAIL_SIZE, self.THUMBNAIL_SIZE)
        thumb_layout = QVBoxLayout(thumb_container)
        thumb_layout.setContentsMargins(0, 0, 0, 0)
        
        # Load and scale image
        self.thumb_label = QLabel()
        self.thumb_label.setFixedSize(self.THUMBNAIL_SIZE, self.THUMBNAIL_SIZE)
        self.thumb_label.setAlignment(Qt.AlignCenter)
        self.thumb_label.setStyleSheet("""
            QLabel {
                background-color: #2a2a2a;
                border-radius: 8px;
            }
        """)
        
        if self.path.exists():
            pixmap = QPixmap(str(self.path))
            if not pixmap.isNull():
                scaled = pixmap.scaled(
                    self.THUMBNAIL_SIZE - 8, 
                    self.THUMBNAIL_SIZE - 8,
                    Qt.KeepAspectRatio,
                    Qt.SmoothTransformation
                )
                self.thumb_label.setPixmap(scaled)
        
        thumb_layout.addWidget(self.thumb_label)
        
        # Status icon overlay
        if self.is_best or self.is_duplicate:
            icon = "⭐" if self.is_best else "🔄"
            icon_label = QLabel(icon)
            icon_label.setStyleSheet("""
                QLabel {
                    background-color: rgba(0, 0, 0, 0.6);
                    border-radius: 10px;
                    padding: 2px 6px;
                    font-size: 14px;
                }
            """)
            icon_label.setParent(thumb_container)
            icon_label.move(4, 4)
        
        layout.addWidget(thumb_container)
        
        # Filename label
        name_label = QLabel(self.path.name[:15] + "..." if len(self.path.name) > 15 else self.path.name)
        name_label.setAlignment(Qt.AlignCenter)
        name_label.setStyleSheet("font-size: 11px; color: #aaa;")
        layout.addWidget(name_label)
        
        # Frame styling
        self.setStyleSheet("""
            PhotoThumbnail {
                background-color: transparent;
                border-radius: 8px;
            }
            PhotoThumbnail:hover {
                background-color: rgba(255, 255, 255, 0.05);
            }
        """)
        
    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.clicked.emit(self.path)
        super().mousePressEvent(event)


class PhotoGalleryDialog(QDialog):
    """Gallery dialog for viewing photo analysis results."""
    
    COLUMNS = 5
    
    def __init__(
        self,
        folder_path: Optional[Path] = None,
        best_shots: Optional[Set[Path]] = None,
        duplicates: Optional[Set[Path]] = None,
        parent=None
    ):
        super().__init__(parent)
        self.folder_path = folder_path
        self.best_shots = best_shots or set()
        self.duplicates = duplicates or set()
        self.photos: List[Path] = []
        
        self.setWindowTitle("📸 Photo Gallery")
        self.setMinimumSize(800, 600)
        self.setup_ui()
        
        if folder_path and folder_path.exists():
            self.load_photos(folder_path)
    
    def setup_ui(self):
        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 16, 16, 16)
        layout.setSpacing(12)
        
        # Header
        header = QHBoxLayout()
        
        title = QLabel("📸 Photo Gallery")
        title.setStyleSheet("font-size: 20px; font-weight: bold; color: #fff;")
        header.addWidget(title)
        
        header.addStretch()
        
        # Folder selection button
        btn_folder = QPushButton("📁 폴더 선택")
        btn_folder.setStyleSheet("""
            QPushButton {
                background-color: #10a37f;
                color: white;
                border: none;
                border-radius: 8px;
                padding: 8px 16px;
                font-size: 14px;
            }
            QPushButton:hover {
                background-color: #0d8a6a;
            }
        """)
        btn_folder.clicked.connect(self.select_folder)
        header.addWidget(btn_folder)
        
        layout.addLayout(header)
        
        # Search bar
        search_layout = QHBoxLayout()
        
        self.search_input = QLineEdit()
        self.search_input.setPlaceholderText("🔍 검색: 작년 겨울 후쿠오카 혼자 사진...")
        self.search_input.setStyleSheet("""
            QLineEdit {
                background-color: #2a2a2a;
                border: 1px solid #444;
                border-radius: 8px;
                padding: 10px 16px;
                font-size: 14px;
                color: #fff;
            }
            QLineEdit:focus {
                border-color: #10a37f;
            }
        """)
        self.search_input.returnPressed.connect(self.run_search)
        search_layout.addWidget(self.search_input)
        
        btn_search = QPushButton("🔍 검색")
        btn_search.setStyleSheet("""
            QPushButton {
                background-color: #3b82f6;
                color: white;
                border: none;
                border-radius: 8px;
                padding: 10px 20px;
                font-size: 14px;
            }
            QPushButton:hover {
                background-color: #2563eb;
            }
        """)
        btn_search.clicked.connect(self.run_search)
        search_layout.addWidget(btn_search)
        
        layout.addLayout(search_layout)
        
        # Scroll area for grid
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setStyleSheet("QScrollArea { border: none; background: transparent; }")
        
        self.grid_container = QWidget()
        self.grid_layout = QGridLayout(self.grid_container)
        self.grid_layout.setSpacing(8)
        self.grid_layout.setContentsMargins(0, 0, 0, 0)
        
        scroll.setWidget(self.grid_container)
        layout.addWidget(scroll)
        
        # Status bar
        self.status_label = QLabel("폴더를 선택하세요...")
        self.status_label.setStyleSheet("color: #888; font-size: 12px;")
        layout.addWidget(self.status_label)
        
        # Dialog styling
        self.setStyleSheet("""
            QDialog {
                background-color: #1e1e1e;
            }
        """)
    
    def select_folder(self):
        """Open folder selection dialog."""
        folder = QFileDialog.getExistingDirectory(
            self,
            "사진 폴더 선택",
            str(Path.home()),
            QFileDialog.ShowDirsOnly
        )
        if folder:
            self.load_photos(Path(folder))
    
    def load_photos(self, folder: Path):
        """Load photos from folder."""
        self.folder_path = folder
        self.photos.clear()
        
        # Clear existing grid
        while self.grid_layout.count():
            item = self.grid_layout.takeAt(0)
            if item.widget():
                item.widget().deleteLater()
        
        # Find image files
        extensions = {'.jpg', '.jpeg', '.png', '.heic', '.gif', '.webp'}
        for path in sorted(folder.rglob('*')):
            if path.suffix.lower() in extensions:
                self.photos.append(path)
        
        # Populate grid
        for idx, photo in enumerate(self.photos):
            row = idx // self.COLUMNS
            col = idx % self.COLUMNS
            
            is_best = photo in self.best_shots
            is_dup = photo in self.duplicates
            
            thumb = PhotoThumbnail(photo, is_best=is_best, is_duplicate=is_dup)
            thumb.clicked.connect(self.open_photo)
            self.grid_layout.addWidget(thumb, row, col)
        
        # Update status
        best_count = sum(1 for p in self.photos if p in self.best_shots)
        dup_count = sum(1 for p in self.photos if p in self.duplicates)
        self.status_label.setText(
            f"📁 {folder.name} | 총: {len(self.photos)}장 | ⭐ 베스트: {best_count} | 🔄 중복: {dup_count}"
        )
    
    def open_photo(self, path: Path):
        """Open photo with system viewer."""
        if not path.exists():
            return
        
        try:
            if os.name == 'nt':  # Windows
                os.startfile(str(path))
            elif os.name == 'posix':  # macOS/Linux
                subprocess.run(['open', str(path)], check=True)
        except Exception as e:
            print(f"Failed to open photo: {e}")
    
    def set_analysis_results(self, best_shots: List[Path], duplicates: List[Path]):
        """Set analysis results and reload if folder is loaded."""
        self.best_shots = set(best_shots)
        self.duplicates = set(duplicates)
        
        if self.folder_path:
            self.load_photos(self.folder_path)
    
    def run_search(self):
        """Run natural language photo search."""
        query = self.search_input.text().strip()
        if not query:
            return
        
        if not self.folder_path:
            self.status_label.setText("❌ 먼저 폴더를 선택하세요")
            return
        
        self.status_label.setText(f"🔍 검색 중: {query}...")
        
        try:
            from core.agents.photo.photo_search import search_photos_in_folder
            
            # Run search
            results = search_photos_in_folder(
                self.folder_path,
                query,
                run_detection=True
            )
            
            # Display results
            self.display_search_results(results)
            
        except Exception as e:
            self.status_label.setText(f"❌ 검색 오류: {e}")
    
    def display_search_results(self, results):
        """Display search results in grid."""
        # Clear existing grid
        while self.grid_layout.count():
            item = self.grid_layout.takeAt(0)
            if item.widget():
                item.widget().deleteLater()
        
        # Populate grid with results
        for idx, asset in enumerate(results):
            row = idx // self.COLUMNS
            col = idx % self.COLUMNS
            
            is_best = asset.path in self.best_shots
            is_dup = asset.path in self.duplicates
            
            thumb = PhotoThumbnail(asset.path, is_best=is_best, is_duplicate=is_dup)
            thumb.clicked.connect(self.open_photo)
            self.grid_layout.addWidget(thumb, row, col)
        
        # Update status
        self.status_label.setText(
            f"🔍 검색 결과: {len(results)}장 ('{self.search_input.text()}')"
        )


def show_gallery(
    folder_path: Optional[Path] = None,
    best_shots: Optional[List[Path]] = None,
    duplicates: Optional[List[Path]] = None,
    parent=None
) -> PhotoGalleryDialog:
    """Convenience function to show gallery dialog."""
    dialog = PhotoGalleryDialog(
        folder_path=folder_path,
        best_shots=set(best_shots) if best_shots else None,
        duplicates=set(duplicates) if duplicates else None,
        parent=parent
    )
    dialog.exec()
    return dialog
