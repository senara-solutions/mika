use thiserror::Error;

use crate::types::TaskState;

/// Errors from A2A protocol operations.
#[derive(Debug, Error)]
pub enum A2aError {
    #[error("invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: TaskState, to: TaskState },

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task not cancelable: {0}")]
    TaskNotCancelable(String),

    #[error("invalid JSON-RPC request: {0}")]
    InvalidJsonRpc(String),

    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
