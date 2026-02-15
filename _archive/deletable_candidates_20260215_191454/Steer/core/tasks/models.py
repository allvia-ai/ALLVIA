"""Data models for the Task Center."""
from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from typing import Optional

class TaskStatus(str, Enum):
    PENDING = "pending"
    COMPLETED = "completed"
    DELETED = "deleted"

@dataclass
class Task:
    content: str
    id: str = field(default_factory=lambda: str(uuid.uuid4()))
    source_meeting_id: Optional[str] = None
    status: TaskStatus = TaskStatus.PENDING
    owner: Optional[str] = None
    due_date: Optional[str] = None
    created_at: datetime = field(default_factory=datetime.now)
    updated_at: datetime = field(default_factory=datetime.now)

    def to_dict(self) -> dict:
        return {
            "id": self.id,
            "content": self.content,
            "source_meeting_id": self.source_meeting_id,
            "status": self.status.value,
            "owner": self.owner,
            "due_date": self.due_date,
            "created_at": self.created_at.isoformat(),
            "updated_at": self.updated_at.isoformat(),
        }

    @classmethod
    def from_dict(cls, data: dict) -> Task:
        return cls(
            id=data["id"],
            content=data["content"],
            source_meeting_id=data.get("source_meeting_id"),
            status=TaskStatus(data.get("status", "pending")),
            owner=data.get("owner"),
            due_date=data.get("due_date"),
            created_at=datetime.fromisoformat(data["created_at"]),
            updated_at=datetime.fromisoformat(data["updated_at"]),
        )
