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

    /// Whether this error is a network-transport failure (mika#1744).
    ///
    /// Transport failures (DNS, connection refused, TLS handshake, socket
    /// reset) resolve in seconds — much faster than HTTP-status errors that
    /// consume the full per-request timeout. The retry loop uses this to
    /// pick a smaller deadline-remaining threshold before allowing a
    /// retry, which is the primary substrate fix for the z.ai transport
    /// wedge that killed mika-qa's 2026-07-07 turn.
    pub fn is_transport(&self) -> bool {
        matches!(self, LlmError::Transport(_))
    }
}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Transport(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_transport_only_true_for_transport_variant() {
        assert!(LlmError::Transport("network down".into()).is_transport());
        assert!(
            !LlmError::HttpError {
                status: 500,
                message: "server error".into(),
                retryable: true,
            }
            .is_transport()
        );
        assert!(!LlmError::ParseError("bad json".into()).is_transport());
        assert!(!LlmError::ProviderError("provider bad".into()).is_transport());
        assert!(!LlmError::UnsupportedFeature("no vision".into()).is_transport());
    }

    /// mika#1744 guardrail: `is_transport()` and `is_retryable()` are
    /// independent classifiers. All transport errors are retryable, but
    /// not all retryable errors are transport (e.g., HTTP 429 / 500).
    #[test]
    fn all_transport_errors_are_retryable() {
        let e = LlmError::Transport("connection reset".into());
        assert!(e.is_transport() && e.is_retryable());
    }

    #[test]
    fn http_500_retryable_but_not_transport() {
        let e = LlmError::HttpError {
            status: 500,
            message: "upstream".into(),
            retryable: true,
        };
        assert!(e.is_retryable());
        assert!(!e.is_transport());
    }
}
