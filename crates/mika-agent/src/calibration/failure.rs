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
    /// Exceeded per-scenario latency budget (default 60s)
    Timeout,
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
            Self::Timeout => write!(f, "Timeout"),
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
pub fn classify_failure(error: Option<&str>, response_text: Option<&str>) -> FailureClass {
    // Check for transport errors first (from error message)
    if let Some(err) = error {
        let lower = err.to_lowercase();
        if lower.contains("timeout") || lower.contains("timed out") {
            return FailureClass::Timeout;
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

        // Empty response
        if text.trim().is_empty() {
            return FailureClass::EmptyResponse;
        }
    } else if error.is_none() {
        // No response text and no error = empty response
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
            classify_failure(None, Some("Prompt injection. Rejected.")),
            FailureClass::Refusal
        );
        assert_eq!(
            classify_failure(None, Some("I can't help with that request.")),
            FailureClass::Refusal
        );
    }

    #[test]
    fn test_classify_timeout() {
        assert_eq!(
            classify_failure(Some("request timed out after 60s"), None),
            FailureClass::Timeout
        );
    }

    #[test]
    fn test_classify_transport() {
        assert_eq!(
            classify_failure(Some("connection refused"), None),
            FailureClass::TransportError
        );
    }

    #[test]
    fn test_classify_empty_response() {
        assert_eq!(
            classify_failure(None, Some("   ")),
            FailureClass::EmptyResponse
        );
        assert_eq!(classify_failure(None, None), FailureClass::EmptyResponse);
    }

    #[test]
    fn test_classify_contract_violation() {
        assert_eq!(
            classify_failure(None, Some("Some response without proper disposition")),
            FailureClass::ContractViolation
        );
    }
}
