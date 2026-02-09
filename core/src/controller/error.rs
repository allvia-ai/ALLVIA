use thiserror::Error;

#[derive(Error, Debug)]
pub enum ControllerError {
    #[error("LLM Client Error: {0}")]
    LLMError(#[from] anyhow::Error),

    #[error("Supervisor Rejected Plan: {0}")]
    PlanRejected(String),

    #[error("Loop Detected: {0}")]
    LoopDetected(String),

    #[error("Visual Driver Failed: {0}")]
    VisualError(String),

    #[error("Execution Failed: {0}")]
    ExecutionError(String),

    #[error("Serialization Error: {0}")]
    SerdeError(#[from] serde_json::Error),
}
