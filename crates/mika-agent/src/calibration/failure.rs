//! Failure classification taxonomy for calibration scenarios.
//!
//! Maps directly to known regression classes:
//! - mika#1168 = Refusal
//! - mika#1166 = ContractViolation
//! - mika#1173 = ContractViolation

use serde::{Deserialize, Serialize};

/// Classification of a scenario failure for aggregation in calibration reports.
///
/// The taxonomy is intentionally small and additive — new classes are added
/// when a new regression appears that doesn't fit existing variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "class", content = "detail")]
pub enum FailureClass {
    /// Refusal regex match (e.g., "Prompt injection. Rejected.")
    Refusal,
    /// Tool call to non-existent tool, or cite to non-existent file
    Fabrication,
    /// No text output and no tool calls
    EmptyResponse,
    /// Output budget consumed by internal reasoning tokens before any visible
    /// content was emitted (reasoning-mode models hitting the `max_tokens` cap).
    /// Distinguished from `EmptyResponse` by nonzero output tokens + a
    /// length-capped finish reason. Remediation is raising `max_tokens`, not
    /// treating the model as a regression (mika#1665).
    ReasoningBudgetExhausted,
    /// Exceeded per-scenario latency budget (default 60s)
    Timeout,
    /// Authentication failure (OAuth, API key, etc.)
    AuthenticationError,
    /// Provider returned non-200 / network error
    TransportError,
    /// Skill output contract failed (suffix line, finding list, disposition keyword, etc.)
    ContractViolation,
    /// Catch-all for failures not matching other classes
    Other(String),
}

impl std::fmt::Display for FailureClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refusal => write!(f, "Refusal"),
            Self::Fabrication => write!(f, "Fabrication"),
            Self::EmptyResponse => write!(f, "EmptyResponse"),
            Self::ReasoningBudgetExhausted => write!(f, "ReasoningBudgetExhausted"),
            Self::Timeout => write!(f, "Timeout"),
            Self::AuthenticationError => write!(f, "AuthenticationError"),
            Self::TransportError => write!(f, "TransportError"),
            Self::ContractViolation => write!(f, "ContractViolation"),
            Self::Other(detail) => write!(f, "Other({})", detail),
        }
    }
}

/// Known refusal patterns from production incidents.
/// Anchored on mika#1168's actual refusal strings.
const REFUSAL_PATTERNS: &[&str] = &[
    "prompt injection. rejected.",
    "i can't help with that",
    "i cannot help with that",
    "i'm not able to help with that",
    "i am not able to assist",
    "i cannot assist with",
    "i'm unable to",
    "as an ai, i cannot",
];

/// Classify a failure from an error message and optional response text.
///
/// `output_tokens` and `finish_reason_is_length` sharpen the empty-content
/// case: a reasoning-mode model can burn its entire output budget on internal
/// reasoning tokens before emitting visible content, producing an empty
/// response with nonzero output tokens and a length-capped finish reason. That
/// is `ReasoningBudgetExhausted` (raise `max_tokens`), not `EmptyResponse`
/// (model regression). Callers without this signal pass `None`/`false` and get
/// the original behavior (mika#1665).
pub fn classify_failure(
    error: Option<&str>,
    response_text: Option<&str>,
    output_tokens: Option<u64>,
    finish_reason_is_length: bool,
) -> FailureClass {
    // Check for transport/auth errors first (from error message)
    if let Some(err) = error {
        let lower = err.to_lowercase();
        if lower.contains("timeout") || lower.contains("timed out") {
            return FailureClass::Timeout;
        }
        // Authentication errors (OAuth, API key) before transport
        if lower.contains("oauth")
            || lower.contains("authentication failed")
            || lower.contains("invalid x-api-key")
            || lower.contains("invalid api key")
        {
            return FailureClass::AuthenticationError;
        }
        if lower.contains("connection")
            || lower.contains("network")
            || lower.contains("status: 5")
            || lower.contains("status: 429")
        {
            return FailureClass::TransportError;
        }
    }

    // Check response text for refusal patterns
    if let Some(text) = response_text {
        let lower = text.to_lowercase();

        // Check refusals
        for pattern in REFUSAL_PATTERNS {
            if lower.contains(pattern) {
                return FailureClass::Refusal;
            }
        }

        // Empty response: distinguish reasoning-budget exhaustion from a
        // genuine empty/refusal so the remediation (raise max_tokens) is clear.
        if text.trim().is_empty() {
            if finish_reason_is_length && output_tokens.is_some_and(|t| t > 0) {
                return FailureClass::ReasoningBudgetExhausted;
            }
            return FailureClass::EmptyResponse;
        }
    } else if error.is_none() {
        // No response text and no error.
        if finish_reason_is_length && output_tokens.is_some_and(|t| t > 0) {
            return FailureClass::ReasoningBudgetExhausted;
        }
        return FailureClass::EmptyResponse;
    }

    // Default to Other with the error message
    if let Some(err) = error {
        FailureClass::Other(err.to_string())
    } else {
        FailureClass::ContractViolation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_refusal() {
        assert_eq!(
            classify_failure(None, Some("Prompt injection. Rejected."), None, false),
            FailureClass::Refusal
        );
        assert_eq!(
            classify_failure(None, Some("I can't help with that request."), None, false),
            FailureClass::Refusal
        );
    }

    #[test]
    fn test_classify_timeout() {
        assert_eq!(
            classify_failure(Some("request timed out after 60s"), None, None, false),
            FailureClass::Timeout
        );
    }

    #[test]
    fn test_classify_transport() {
        assert_eq!(
            classify_failure(Some("connection refused"), None, None, false),
            FailureClass::TransportError
        );
    }

    #[test]
    fn test_classify_empty_response() {
        assert_eq!(
            classify_failure(None, Some("   "), None, false),
            FailureClass::EmptyResponse
        );
        assert_eq!(
            classify_failure(None, None, None, false),
            FailureClass::EmptyResponse
        );
    }

    #[test]
    fn test_classify_reasoning_budget_exhausted() {
        // Empty visible content, but the model burned output tokens and stopped
        // on length → reasoning-budget exhaustion, not a genuine empty response.
        assert_eq!(
            classify_failure(None, Some(""), Some(1000), true),
            FailureClass::ReasoningBudgetExhausted
        );
        assert_eq!(
            classify_failure(None, Some("   "), Some(2000), true),
            FailureClass::ReasoningBudgetExhausted
        );
        // No-response-text variant (e.g. content stripped upstream).
        assert_eq!(
            classify_failure(None, None, Some(1500), true),
            FailureClass::ReasoningBudgetExhausted
        );
    }

    #[test]
    fn test_reasoning_budget_requires_both_signals() {
        // Length-capped but zero output tokens → still EmptyResponse.
        assert_eq!(
            classify_failure(None, Some(""), Some(0), true),
            FailureClass::EmptyResponse
        );
        // Output tokens present but not length-capped → still EmptyResponse.
        assert_eq!(
            classify_failure(None, Some(""), Some(1000), false),
            FailureClass::EmptyResponse
        );
        // Nonempty content is never reclassified, even with the signals set.
        assert_eq!(
            classify_failure(None, Some("here is my plan"), Some(2000), true),
            FailureClass::ContractViolation
        );
    }

    #[test]
    fn test_classify_oauth_error() {
        assert_eq!(
            classify_failure(
                Some("OAuth token resolution failed: ..."),
                None,
                None,
                false
            ),
            FailureClass::AuthenticationError
        );
    }

    #[test]
    fn test_classify_auth_failed() {
        assert_eq!(
            classify_failure(
                Some("Authentication failed. Check API key."),
                None,
                None,
                false
            ),
            FailureClass::AuthenticationError
        );
    }

    #[test]
    fn test_classify_invalid_api_key() {
        assert_eq!(
            classify_failure(Some("invalid x-api-key"), None, None, false),
            FailureClass::AuthenticationError
        );
        assert_eq!(
            classify_failure(Some("invalid api key provided"), None, None, false),
            FailureClass::AuthenticationError
        );
    }

    #[test]
    fn test_classify_contract_violation() {
        assert_eq!(
            classify_failure(
                None,
                Some("Some response without proper disposition"),
                None,
                false
            ),
            FailureClass::ContractViolation
        );
    }

    #[test]
    fn test_reasoning_budget_display() {
        assert_eq!(
            FailureClass::ReasoningBudgetExhausted.to_string(),
            "ReasoningBudgetExhausted"
        );
    }
}
