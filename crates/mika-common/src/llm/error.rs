use std::borrow::Cow;

use thiserror::Error;

/// The wire-format error-class vocabulary, defined **once** (mika#2331 D3/AC5).
///
/// These seven strings are a wire format, not labels: the operator runs
/// `GROUP BY` over them on `audit_events.callback_delivery_failed` (mika#2179),
/// and the `llm_call_attempt` event (mika#2331 §3.2) carries them too. A second
/// spelling anywhere would split one population in two without saying so — the
/// defect the `FILTER_*` constants of mika#2131 had to close once already.
///
/// Two mappings consume them and only two: [`LlmError::error_class`] and
/// `ClaudeApiError::error_class` (`crate::claude`). The second exists because
/// `ClaudeApiError` is a foreign enum that cannot share an *implementation* —
/// which is why this module offers a site of **definition** rather than one of
/// implementation.
pub mod error_class {
    /// A transport failure whose cause chain names a timeout — the shape the
    /// mika#2179 founding incident was made of (19 in four hours).
    pub const TRANSPORT_TIMEOUT: &str = "transport_timeout";
    /// Any other transport failure: refused connection, DNS, TLS, reset.
    pub const TRANSPORT: &str = "transport";
    /// The bytes arrived and did not parse.
    pub const PARSE: &str = "parse";
    /// The provider answered, and answered something we cannot use.
    pub const PROVIDER: &str = "provider";
    /// The request asked for a capability this provider does not have.
    pub const UNSUPPORTED: &str = "unsupported";
    /// Not an LLM failure at all (the classifier was handed something else).
    pub const OTHER: &str = "other";

    /// `http_<status>` — the seventh class, the only one carrying a value.
    ///
    /// Keeping `429` distinguishable from `500` is the difference between
    /// "we are being rate-limited" and "the provider is down", and triage
    /// needs both.
    #[must_use]
    pub fn http(status: u16) -> String {
        format!("http_{status}")
    }
}

/// Split a transport failure's rendered cause chain into the two transport
/// classes.
///
/// The one place a string is consulted rather than a variant: neither
/// [`LlmError::Transport`] nor the ported `ClaudeApiError::BodyRead` models the
/// timeout-versus-refusal distinction, and `From<reqwest::Error>` flattens the
/// `reqwest::Error` into a `String` (see the impl below), losing `is_timeout()`.
#[must_use]
pub fn classify_transport_message(message: &str) -> &'static str {
    if message.to_lowercase().contains("timed out") {
        error_class::TRANSPORT_TIMEOUT
    } else {
        error_class::TRANSPORT
    }
}

/// A response body made only of whitespace is an EMPTY response, therefore
/// transitory — never a malformed JSON (mika#1781).
///
/// Returns `Some(Transport(…))` when `body.trim()` is empty, `None` otherwise.
/// `None` is what lets a non-blank body **traverse** to `serde_json`, keeping
/// mika#2015's population intact: a body that is invalid without being blank is
/// an error, not something to sew back together.
///
/// # Why a function rather than an `if` at the call site
///
/// `send_once` performs a real HTTP POST, while every test of `openai.rs` is a
/// synchronous `#[test]` over pure functions — so the rule has to live somewhere
/// testable without standing up a server. It is also the one site where this
/// message is worded, and the message decides the wire class: duplicating it on
/// a second rail is how the [`error_class`] divergence gets programmed.
///
/// The message deliberately contains no `timed out`, so
/// [`classify_transport_message`] files it under [`error_class::TRANSPORT`] and
/// not [`error_class::TRANSPORT_TIMEOUT`]. That is a decision, not a side
/// effect: the observed shape *suggests* an upstream timeout (padding emitted
/// while a backend expires) but nothing in the body attests it, and `transport`
/// is precisely the "any other transport failure" class.
#[must_use]
pub fn blank_response_body(body: &str) -> Option<LlmError> {
    if body.trim().is_empty() {
        Some(LlmError::Transport(format!(
            "empty (whitespace-only) response body, {} bytes",
            body.len()
        )))
    } else {
        None
    }
}

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

    /// The wire-format error class of this failure (mika#2331 D3).
    ///
    /// Moved here verbatim from `classify_delivery_error`
    /// (`mika-agent::task_engine::dispatcher`, mika#2179), which now delegates:
    /// the classification had to become readable from the LLM client so the
    /// per-attempt `llm_call_attempt` event could carry it, and writing a
    /// second copy there would have been the divergence [`error_class`]
    /// exists to prevent.
    ///
    /// Reads the **variant**, never the rendered message — with the one
    /// documented exception inside `Transport`, see
    /// [`classify_transport_message`].
    #[must_use]
    pub fn error_class(&self) -> Cow<'static, str> {
        match self {
            LlmError::Transport(msg) => Cow::Borrowed(classify_transport_message(msg)),
            LlmError::HttpError { status, .. } => Cow::Owned(error_class::http(*status)),
            LlmError::ParseError(_) => Cow::Borrowed(error_class::PARSE),
            LlmError::ProviderError(_) => Cow::Borrowed(error_class::PROVIDER),
            LlmError::UnsupportedFeature(_) => Cow::Borrowed(error_class::UNSUPPORTED),
        }
    }
}

/// The wire-format error class of an `anyhow` error that may wrap an
/// [`LlmError`] (mika#2289 A2).
///
/// `downcast_ref` walks the whole `anyhow` cause chain, so this reads the
/// **variant** wherever in the chain it sits — never a `contains()` on a
/// rendered message. Anything that is not an [`LlmError`] is
/// [`error_class::OTHER`], which is a statement ("not an LLM failure"), not a
/// fallback.
///
/// **One definition, two consumers.** `mika-agent` calls it from
/// `task_engine::dispatcher::classify_delivery_error` (mika#2179, the callback
/// delivery ladder) and from `server::handlers` (mika#2289, the engine-side
/// `hold[review]` net). Those two write into the same operator vocabulary —
/// `audit_events.callback_delivery_failed` and the `error_class` field of
/// `qa_deadline_verdict` — and a second copy of these four lines is exactly how
/// one population splits into two spellings without saying so.
#[must_use]
pub fn classify_anyhow_error(err: &anyhow::Error) -> Cow<'static, str> {
    err.downcast_ref::<LlmError>()
        .map_or(Cow::Borrowed(error_class::OTHER), LlmError::error_class)
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

    // ── error_class (mika#2331 D3/AC5) ──

    /// The founding incident's exact error text, verbatim from
    /// `/var/log/mika/server.log` on the night of 2026-09-03/04 — the same
    /// constant `dispatcher.rs` pins on the consumer side.
    const INCIDENT_TRANSPORT_TIMEOUT: &str = "failed to read response body: error decoding response body: \
         request or response body error: operation timed out";

    #[test]
    fn error_class_covers_every_variant() {
        let cases: Vec<(LlmError, &str)> = vec![
            (
                LlmError::Transport(INCIDENT_TRANSPORT_TIMEOUT.into()),
                "transport_timeout",
            ),
            (
                LlmError::Transport("connection refused".into()),
                "transport",
            ),
            (
                LlmError::HttpError {
                    status: 429,
                    message: "slow down".into(),
                    retryable: true,
                },
                "http_429",
            ),
            (
                LlmError::HttpError {
                    status: 500,
                    message: "upstream".into(),
                    retryable: true,
                },
                "http_500",
            ),
            (LlmError::ParseError("bad json".into()), "parse"),
            (LlmError::ProviderError("upstream".into()), "provider"),
            (LlmError::UnsupportedFeature("vision".into()), "unsupported"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.error_class(), expected, "class for {err:?}");
        }
    }

    /// A rate-limited provider and a provider that is down must not collapse
    /// into one bucket — the reason the seventh class carries a value.
    #[test]
    fn error_class_keeps_the_http_status() {
        assert_ne!(
            LlmError::HttpError {
                status: 429,
                message: String::new(),
                retryable: true,
            }
            .error_class(),
            LlmError::HttpError {
                status: 500,
                message: String::new(),
                retryable: true,
            }
            .error_class()
        );
    }

    /// The vocabulary is a wire format: these exact spellings land in
    /// `audit_events.after_value` and operators `GROUP BY` them. A rename here
    /// silently cuts a population in two.
    #[test]
    fn mika2331_class_names_are_a_wire_format() {
        assert_eq!(error_class::TRANSPORT_TIMEOUT, "transport_timeout");
        assert_eq!(error_class::TRANSPORT, "transport");
        assert_eq!(error_class::PARSE, "parse");
        assert_eq!(error_class::PROVIDER, "provider");
        assert_eq!(error_class::UNSUPPORTED, "unsupported");
        assert_eq!(error_class::OTHER, "other");
        assert_eq!(error_class::http(429), "http_429");
    }

    // ── blank_response_body (mika#1781) ──

    /// The measured body of 2026-08-28T20:53:23Z, provider `openrouter`:
    /// **1320 bytes, 240 newlines, not one JSON character** — which is why serde
    /// reported `EOF while parsing a value at line 241 column 0`.
    ///
    /// Built rather than captured: mika#2015 fixed the posture (capped excerpt,
    /// on failure only), and a body of whitespace carries no customer content to
    /// protect — which is exactly what makes it reproducible here without
    /// raising a privacy question.
    fn measured_blank_body() -> String {
        // 240 lines totalling 1080 spaces + 240 '\n' = 1320 bytes. The split
        // (120 lines of 4 spaces, 120 of 5) is what makes both numbers exact.
        let mut body = String::new();
        for i in 0..240 {
            body.push_str(if i % 2 == 0 { "    " } else { "     " });
            body.push('\n');
        }
        body
    }

    /// V1a — the measured shape is claimed, routed to `Transport`, and retryable.
    ///
    /// **Seen red before the production rule existed**, per the plan's DoD: on
    /// `main` a blank body reaches no classifier at all, so a test that passed
    /// first try would not attest that it measures anything.
    #[test]
    fn mika1781_the_measured_blank_body_is_retryable_transport() {
        let body = measured_blank_body();
        assert_eq!(body.len(), 1320, "the fixture must be the measured size");
        assert_eq!(
            body.matches('\n').count(),
            240,
            "240 newlines — serde reported line 241"
        );
        assert!(
            !body.contains(|c: char| !c.is_whitespace()),
            "not one JSON character"
        );

        let err = blank_response_body(&body).expect("a whitespace-only body must be claimed");
        assert!(
            matches!(err, LlmError::Transport(_)),
            "must be Transport, never ParseError: {err:?}"
        );
        assert!(err.is_retryable(), "the whole point of mika#1781");
        assert!(err.is_transport(), "and it takes the mika#1744 fast retry");
        assert_eq!(
            err.error_class(),
            error_class::TRANSPORT,
            "transport, not transport_timeout — nothing in the body attests a timeout"
        );
        assert!(
            err.to_string().contains("1320"),
            "the message must name the byte size (AC1): {err}"
        );
    }

    /// V1b — negative control, and the frontier of the whole fix: a malformed
    /// JSON is **not claimed**, so it traverses to serde and keeps its
    /// `ParseError` plus mika#2015's full diagnostic (AC3).
    ///
    /// Without this test the fix could eat #2015's population.
    #[test]
    fn mika1781_a_malformed_json_traverses_the_classifier() {
        assert!(
            blank_response_body("{bad").is_none(),
            "a non-blank body stays serde's business"
        );
        // And the error it then produces is the terminal one, unchanged.
        let parse = LlmError::ParseError("failed to parse response: expected value".into());
        assert!(!parse.is_retryable());
        assert_eq!(parse.error_class(), error_class::PARSE);
    }

    /// V1c — negative control: a valid response body is not claimed either.
    /// Guards against a predicate that bites too wide.
    #[test]
    fn mika1781_a_valid_body_is_not_claimed() {
        let valid = r#"{"id":"x","object":"chat.completion","choices":[]}"#;
        assert!(blank_response_body(valid).is_none());
        // An empty body is blank too — `trim()` of "" is empty. Stated rather
        // than left to inference: a zero-byte 200 is the same class of failure.
        assert!(blank_response_body("").is_some());
    }

    /// Case-insensitive, because the substring is all that separates a timeout
    /// from a refused connection and provider wording is not ours to control.
    #[test]
    fn classify_transport_message_is_case_insensitive() {
        assert_eq!(
            classify_transport_message("Operation TIMED OUT"),
            error_class::TRANSPORT_TIMEOUT
        );
        assert_eq!(
            classify_transport_message("connection refused"),
            error_class::TRANSPORT
        );
    }
}
