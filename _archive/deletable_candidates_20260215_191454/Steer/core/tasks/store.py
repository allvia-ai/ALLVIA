"""SQLite storage for the Task Center."""
from __future__ import annotations

import sqlite3
import logging
from pathlib import Path
from typing import List, Optional

from core.config.paths import DATA_DIR
from .models import Task, TaskStatus

LOGGER = logging.getLogger(__name__)

class TaskStore:
    def __init__(self, db_path: Optional[Path] = None) -> None:
        if db_path is None:
            db_path = DATA_DIR / "tasks.db"
        self._db_path = db_path
        self._init_db()

    def _init_db(self) -> None:
        """Initialize the database schema."""
        self._db_path.parent.mkdir(parents=True, exist_ok=True)
        with sqlite3.connect(self._db_path) as conn:
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS tasks (
                    id TEXT PRIMARY KEY,
                    content TEXT NOT NULL,
                    source_meeting_id TEXT,
                    status TEXT NOT NULL,
                    owner TEXT,
                    due_date TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )
                """
            )
            # Add indexes for performance
            conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status)")
            conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_created_at ON tasks(created_at)")
            conn.commit()

    def add_task(self, task: Task) -> None:
        """Persist a new task."""
        with sqlite3.connect(self._db_path) as conn:
            conn.execute(
                """
                INSERT INTO tasks (id, content, source_meeting_id, status, owner, due_date, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    task.id,
                    task.content,
                    task.source_meeting_id,
                    task.status.value,
                    task.owner,
                    task.due_date,
                    task.created_at.isoformat(),
                    task.updated_at.isoformat(),
                ),
            )
        LOGGER.debug("Added task %s", task.id)

    def list_tasks(self, status: Optional[TaskStatus] = None) -> List[Task]:
        """Retrieve tasks, optionally filtered by status."""
        query = "SELECT id, content, source_meeting_id, status, owner, due_date, created_at, updated_at FROM tasks"
        params = []
        if status:
            query += " WHERE status = ?"
            params.append(status.value)
        
        query += " ORDER BY created_at DESC"
        
        tasks = []
        with sqlite3.connect(self._db_path) as conn:
            cursor = conn.execute(query, params)
            for row in cursor:
                tasks.append(
                    Task.from_dict({
                        "id": row[0],
                        "content": row[1],
                        "source_meeting_id": row[2],
                        "status": row[3],
                        "owner": row[4],
                        "due_date": row[5],
                        "created_at": row[6],
                        "updated_at": row[7],
                    })
                )
        return tasks

    def update_task_status(self, task_id: str, status: TaskStatus) -> bool:
        """Update the status of a task."""
        with sqlite3.connect(self._db_path) as conn:
            cur = conn.execute(
                "UPDATE tasks SET status = ?, updated_at = datetime('now') WHERE id = ?",
                (status.value, task_id),
            )
            return cur.rowcount > 0

    def delete_task(self, task_id: str) -> bool:
        """Hard delete a task."""
        with sqlite3.connect(self._db_path) as conn:
            cur = conn.execute("DELETE FROM tasks WHERE id = ?", (task_id,))
            return cur.rowcount > 0
