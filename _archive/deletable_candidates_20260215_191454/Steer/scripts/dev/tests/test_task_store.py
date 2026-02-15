import sys
from pathlib import Path
import os
sys.path.insert(0, str(Path(__file__).parent.parent))

from core.tasks.store import TaskStore, Task, TaskStatus

def test_task_store():
    print("🧪 Testing Task Store...")
    
    # Use temp db
    test_db = Path("/tmp/test_tasks.db")
    if test_db.exists():
        test_db.unlink()
        
    store = TaskStore(db_path=test_db)
    
    # 1. Add Task
    t1 = Task(content="Fix the bug", owner="David", due_date="2025-12-25")
    store.add_task(t1)
    print(f"Added task: {t1.id}")
    
    # 2. List Tasks
    tasks = store.list_tasks()
    assert len(tasks) == 1
    assert tasks[0].content == "Fix the bug"
    assert tasks[0].status == TaskStatus.PENDING
    print("Listing passed.")
    
    # 3. Update Status
    store.update_task_status(t1.id, TaskStatus.COMPLETED)
    tasks = store.list_tasks(status=TaskStatus.COMPLETED)
    assert len(tasks) == 1
    assert tasks[0].status == TaskStatus.COMPLETED
    print("Update status passed.")
    
    # 4. Delete
    store.delete_task(t1.id)
    tasks = store.list_tasks()
    assert len(tasks) == 0
    print("Delete passed.")
    
    print("✅ Task Store verified.")
    
    if test_db.exists():
        test_db.unlink()

if __name__ == "__main__":
    test_task_store()
