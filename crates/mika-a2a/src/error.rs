use thiserror::Error;

use crate::types::TaskState;

/// Errors from A2A protocol operations.
#[derive(Debug, Error)]
pub enum A2aError {
    #[error("invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: TaskState, to: TaskState },

    #[error("invalid JSON-RPC request: {0}")]
    InvalidJsonRpc(String),

    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}

/// How a transport-level exchange actually failed.
///
/// `reqwest::Error` collapses several very different situations into one type,
/// and [`A2aError::ClientError`] used to collapse them one step further into a
/// single `connection error` string at the CLI boundary. That string cost a real
/// diagnosis: it was read as "the server is down" while `/health` answered in
/// 0.5 ms and the response the caller wanted had already been generated and
/// persisted server-side (mika#2036).
///
/// The distinction that matters most to a caller is [`Self::request_was_sent`]:
/// a request that never left the client cannot have produced work, while one
/// that did may have a finished answer waiting to be reclaimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportFailure {
    /// The server was never reached — connection refused, DNS failure, TLS
    /// handshake failure, or a request that could not be built at all.
    Unreachable,
    /// The request was sent and the client stopped waiting for the response.
    TimedOut,
    /// The server answered with a non-success HTTP status.
    HttpStatus(u16),
    /// A response arrived but could not be read or decoded.
    Undecodable,
    /// The request was sent and the exchange broke before a usable response
    /// arrived — the socket closed mid-flight, for instance.
    Interrupted,
}

impl TransportFailure {
    /// Classify a `reqwest` failure.
    ///
    /// The order is load-bearing. A connect timeout reports both `is_connect`
    /// and `is_timeout`, and it means the server was never reached, so
    /// `is_connect` is tested first — misreading it as [`Self::TimedOut`] would
    /// send a caller looking for work that was never started.
    pub fn classify(err: &reqwest::Error) -> Self {
        if err.is_connect() || err.is_builder() {
            Self::Unreachable
        } else if err.is_timeout() {
            Self::TimedOut
        } else if err.is_status() {
            Self::HttpStatus(err.status().map_or(0, |s| s.as_u16()))
        } else if err.is_decode() || err.is_body() {
            Self::Undecodable
        } else {
            Self::Interrupted
        }
    }

    /// Whether the request reached the server, so that work may exist on the
    /// other side despite the failure.
    ///
    /// Only [`Self::Unreachable`] answers `false`. This is the guard against a
    /// phantom recovery: a caller that never reached the server must not go
    /// looking for a task it cannot have created.
    pub fn request_was_sent(self) -> bool {
        !matches!(self, Self::Unreachable)
    }

    /// One-line description naming the endpoint — and, for a timeout, the
    /// waiting budget that was actually spent.
    ///
    /// Every variant must produce a distinct sentence; that difference is the
    /// whole point of the type, and `transport_failure_descriptions_are_distinct`
    /// holds it.
    pub fn describe(self, url: &str, timeout: std::time::Duration) -> String {
        match self {
            Self::Unreachable => format!("unreachable: no request reached {url}"),
            Self::TimedOut => format!(
                "timed out: waited {}s for {url} and gave up",
                timeout.as_secs()
            ),
            Self::HttpStatus(code) => format!("HTTP {code} from {url}"),
            Self::Undecodable => format!("unreadable response from {url}"),
            Self::Interrupted => {
                format!("interrupted after send: {url} returned no complete response")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const ALL: [TransportFailure; 5] = [
        TransportFailure::Unreachable,
        TransportFailure::TimedOut,
        TransportFailure::HttpStatus(503),
        TransportFailure::Undecodable,
        TransportFailure::Interrupted,
    ];

    /// AC1's anti-vacuity clause. The founding defect was not a missing message
    /// but a *shared* one: four failures rendered the same sentence. Comparing
    /// every pair fails the moment two of them collapse again.
    #[test]
    fn transport_failure_descriptions_are_distinct() {
        let url = "http://127.0.0.1:9/a2a/mika-arch";
        let rendered: Vec<String> = ALL
            .iter()
            .map(|f| f.describe(url, Duration::from_secs(300)))
            .collect();

        for (i, a) in rendered.iter().enumerate() {
            for (j, b) in rendered.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "{:?} and {:?} render the same text", ALL[i], ALL[j]);
                }
            }
        }
    }

    #[test]
    fn every_description_names_the_endpoint() {
        let url = "http://127.0.0.1:9/a2a/mika-arch";
        for f in ALL {
            let text = f.describe(url, Duration::from_secs(300));
            assert!(
                text.contains(url),
                "{f:?} does not name the endpoint: {text}"
            );
        }
    }

    /// The timeout message must carry the budget that was actually spent —
    /// "j'ai attendu N secondes" is the half of AC1 a bare "connection error"
    /// could never say.
    #[test]
    fn timeout_description_names_the_budget_it_spent() {
        let text = TransportFailure::TimedOut.describe("http://x/a2a/a", Duration::from_secs(300));
        assert!(text.contains("300s"), "budget missing from: {text}");

        // And it is the real budget, not a constant baked into the sentence.
        let short = TransportFailure::TimedOut.describe("http://x/a2a/a", Duration::from_secs(7));
        assert!(
            short.contains("7s"),
            "budget not taken from the argument: {short}"
        );
    }

    /// The recovery gate. Only an unreachable server proves no work exists;
    /// every other failure happened after the bytes left, so an answer may be
    /// sitting on the other side.
    #[test]
    fn only_an_unreachable_server_proves_no_work_exists() {
        assert!(!TransportFailure::Unreachable.request_was_sent());
        for f in ALL.iter().filter(|f| **f != TransportFailure::Unreachable) {
            assert!(
                f.request_was_sent(),
                "{f:?} should allow a recovery attempt"
            );
        }
    }
}
