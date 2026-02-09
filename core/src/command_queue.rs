use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub type CommandResult = Result<String>;
type CommandTask = Box<dyn FnOnce() -> CommandResult + Send + 'static>;

struct QueueEntry {
    task: CommandTask,
    tx: oneshot::Sender<CommandResult>,
    enqueued_at: Instant,
    warn_after: Duration,
}

struct LaneState {
    queue: VecDeque<QueueEntry>,
    active: usize,
    max_concurrent: usize,
    draining: bool,
}

impl LaneState {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            active: 0,
            max_concurrent: 1,
            draining: false,
        }
    }
}

struct CommandQueue {
    lanes: Mutex<HashMap<String, LaneState>>,
}

impl CommandQueue {
    fn new() -> Self {
        Self {
            lanes: Mutex::new(HashMap::new()),
        }
    }

    async fn enqueue_in_lane(
        &self,
        lane: &str,
        task: CommandTask,
        warn_after_ms: u64,
    ) -> CommandResult {
        let (tx, rx) = oneshot::channel();
        let entry = QueueEntry {
            task,
            tx,
            enqueued_at: Instant::now(),
            warn_after: Duration::from_millis(warn_after_ms),
        };

        let should_pump = {
            let mut lanes = self.lanes.lock().expect("command queue lock poisoned");
            let state = lanes.entry(lane.to_string()).or_insert_with(LaneState::new);
            state.queue.push_back(entry);
            if state.draining {
                false
            } else {
                state.draining = true;
                true
            }
        };

        if should_pump {
            let lane_name = lane.to_string();
            let queue = Arc::clone(&COMMAND_QUEUE);
            queue.pump_lane(lane_name);
        }

        match rx.await {
            Ok(result) => result,
            Err(_) => Err(anyhow!("Command queue dropped before returning a result")),
        }
    }

    fn pump_lane(self: Arc<Self>, lane: String) {
        loop {
            let entry = {
                let mut lanes = self.lanes.lock().expect("command queue lock poisoned");
                let state = lanes.entry(lane.clone()).or_insert_with(LaneState::new);
                if state.active >= state.max_concurrent || state.queue.is_empty() {
                    state.draining = false;
                    return;
                }
                state.active += 1;
                state.queue.pop_front()
            };

            let Some(entry) = entry else { continue };
            let waited = entry.enqueued_at.elapsed();
            if waited >= entry.warn_after {
                eprintln!(
                    "⚠️ Command queue wait exceeded: lane={} waited_ms={}",
                    lane,
                    waited.as_millis()
                );
            }

            let queue = Arc::clone(&COMMAND_QUEUE);
            let lane_name = lane.clone();
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || (entry.task)())
                    .await
                    .map_err(|e| anyhow!("Command task join error: {}", e))
                    .and_then(|r| r);
                let _ = entry.tx.send(result);
                queue.on_task_complete(lane_name);
            });
        }
    }

    fn on_task_complete(&self, lane: String) {
        let should_pump = {
            let mut lanes = self.lanes.lock().expect("command queue lock poisoned");
            let state = lanes.entry(lane.clone()).or_insert_with(LaneState::new);
            if state.active > 0 {
                state.active -= 1;
            }
            if !state.queue.is_empty() && !state.draining {
                state.draining = true;
                true
            } else {
                false
            }
        };

        if should_pump {
            let queue = Arc::clone(&COMMAND_QUEUE);
            queue.pump_lane(lane);
        }
    }

    #[allow(dead_code)]
    async fn set_lane_concurrency(&self, lane: &str, max_concurrent: usize) {
        let mut lanes = self.lanes.lock().expect("command queue lock poisoned");
        let state = lanes.entry(lane.to_string()).or_insert_with(LaneState::new);
        state.max_concurrent = std::cmp::max(1, max_concurrent);
    }

    #[allow(dead_code)]
    async fn get_lane_size(&self, lane: &str) -> usize {
        let lanes = self.lanes.lock().expect("command queue lock poisoned");
        lanes
            .get(lane)
            .map(|s| s.queue.len() + s.active)
            .unwrap_or(0)
    }
}

lazy_static! {
    static ref COMMAND_QUEUE: Arc<CommandQueue> = Arc::new(CommandQueue::new());
}

#[allow(dead_code)]
pub async fn enqueue_command(task: CommandTask) -> CommandResult {
    enqueue_command_in_lane("main", task, None).await
}

pub async fn enqueue_command_in_lane(
    lane: &str,
    task: CommandTask,
    warn_after_ms: Option<u64>,
) -> CommandResult {
    let warn_after = warn_after_ms.unwrap_or(2_000);
    COMMAND_QUEUE.enqueue_in_lane(lane, task, warn_after).await
}

#[allow(dead_code)]
pub async fn set_lane_concurrency(lane: &str, max_concurrent: usize) {
    COMMAND_QUEUE
        .set_lane_concurrency(lane, max_concurrent)
        .await
}

#[allow(dead_code)]
pub async fn get_lane_size(lane: &str) -> usize {
    COMMAND_QUEUE.get_lane_size(lane).await
}

/// [Phase 25] Cancel all pending tasks in a lane
#[allow(dead_code)]
pub async fn cancel_lane(lane: &str) {
    let mut lanes = COMMAND_QUEUE
        .lanes
        .lock()
        .expect("command queue lock poisoned");
    if let Some(state) = lanes.get_mut(lane) {
        let count = state.queue.len();
        state.queue.clear();
        if count > 0 {
            eprintln!("⚠️ Cancelled {} pending tasks in lane '{}'", count, lane);
        }
    }
}
