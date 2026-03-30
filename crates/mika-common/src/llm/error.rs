use thiserror::Error;

/// Provider-agnostic LLM error type.
///
/// Each provider maps its native errors into this enum. The `retryable` field
/// on `HttpError` lets the caller decide retry strategy without knowing
/// provider-specific status codes.
#[derive(Debug, Clone, Error)]
pub enum LlmError {
    #[error("LLM HTTP error (status {status}): {message}")]
    HttpError {
        status: u16,
        message: String,
        retryable: bool,
    },

    #[error("LLM transport error: {0}")]
    Transport(String),

    #[error("LLM response parse error: {0}")]
    ParseError(String),

    #[error("LLM provider error: {0}")]
    ProviderError(String),

    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
}

impl LlmError {
    /// Whether this error is transient and the request should be retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            LlmError::HttpError { retryable, .. } => *retryable,
            LlmError::Transport(_) => true,
            _ => false,
        }
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Transport(e.to_string())
    }
}
