//! `AuthBoundaryError` — the shape a cross-boundary authentication failure
//! takes on the mika-manager write path (mika#1949, Porte 3).
//!
//! # Why this type exists
//!
//! Four env-var-backed tokens guard that path — `MIKA_INTERNAL_TOKEN`,
//! `INTERNAL_TOKEN`, `CM_FULL_ACCESS_TOKEN`, `MIKA_MANAGER_DELIVERY_TOKEN`.
//! Before this type, a failure at any of them was indistinguishable from a
//! network error: the caller saw a transport error or an opaque
//! `{"error": "unauthorized"}`, and the operator had no way to tell *which*
//! credential had gone bad. mika#2013 is the measured precedent for the class —
//! a frozen installation token cycled `auth_class=401` sixteen times in one
//! night without naming itself.
//!
//! This type is the observation layer. It names the token, the boundary, and
//! the failure kind, and nothing else.
//!
//! # The one invariant
//!
//! **A token NAME, never a token VALUE.** No field of this struct holds, or is
//! permitted to hold, a secret — not the presented token, not the expected
//! one, not a prefix, not a length. `token_name` carries the *env-var name*
//! (`"MIKA_INTERNAL_TOKEN"`), which is public information already written in
//! the deployment docs. `never_renders_a_value_it_was_not_given` pins this.
//!
//! # Status codes are deliberately not part of this shape
//!
//! cm answers `403` on a token refusal (a house convention recorded at
//! `control-monitor/backend/crates/cm-api/src/routes/permission_events.rs:104`)
//! and mika answers `401`. mika#1949 KTD2 arbitrated that divergence rather
//! than retrofitting it: what this work makes uniform is the *body*, not the
//! status line. A `status` field here would invite a reader to believe
//! otherwise.

use serde::{Deserialize, Serialize};
use std::fmt;

/// How the authentication attempt failed.
///
/// The first two kinds are configuration faults observable *before* any
/// request leaves the process; the last three are outcomes of one that did.
/// Keeping them in one enum is deliberate — the operator's question is "which
/// token, and what is wrong with it", and an unset token and a rejected token
/// are two answers to the same question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthBoundaryKind {
    /// The token is not configured at all — the env var is unset.
    Missing,
    /// The env var is set but holds an empty (or whitespace-only) value.
    /// Distinct from `Missing` because the two have different operator fixes:
    /// a missing line versus a line whose value was lost in an edit.
    Empty,
    /// The token is present but structurally malformed — fails the shape check
    /// the boundary applies before presenting it (e.g. the gateway's hex
    /// validation on `MIKA_INTERNAL_TOKEN`).
    Invalid,
    /// The token was presented and the far side refused it.
    Rejected,
    /// The far side could not be reached, so no authentication verdict exists.
    /// Named here rather than left as a plain transport error because the
    /// operator's first hypothesis on a silent manager is "bad token", and
    /// ruling that out is the point of the ledger.
    Unreachable,
}

impl AuthBoundaryKind {
    /// Stable wire/log spelling. Matches the serde representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Empty => "empty",
            Self::Invalid => "invalid",
            Self::Rejected => "rejected",
            Self::Unreachable => "unreachable",
        }
    }

    /// Every kind, in declaration order. Lets callers and tests enumerate the
    /// set without restating it.
    pub const ALL: [AuthBoundaryKind; 5] = [
        Self::Missing,
        Self::Empty,
        Self::Invalid,
        Self::Rejected,
        Self::Unreachable,
    ];
}

impl fmt::Display for AuthBoundaryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One authentication failure at one boundary, named.
///
/// `from` and `to` are entity names — `"gateway"`, `"spirit"`, `"cm"`,
/// `"manager"` — and together form the `target_key` grammar `<from>_to_<to>`
/// used by the `auth_boundary` audit row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthBoundaryError {
    /// The **name** of the env var that guards this boundary. Never a value.
    pub token_name: String,
    /// The entity that presented (or should have presented) the token.
    pub from: String,
    /// The entity that was asked to accept it.
    pub to: String,
    /// What went wrong.
    pub kind: AuthBoundaryKind,
}

impl AuthBoundaryError {
    pub fn new(
        token_name: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        kind: AuthBoundaryKind,
    ) -> Self {
        Self {
            token_name: token_name.into(),
            from: from.into(),
            to: to.into(),
            kind,
        }
    }

    /// The `<from>_to_<to>` boundary key. This is the exact string the
    /// `auth_boundary` audit row carries as `target_key`, so the two cannot
    /// drift: callers must not re-format it by hand.
    pub fn boundary_key(&self) -> String {
        format!("{}_to_{}", self.from, self.to)
    }
}

impl fmt::Display for AuthBoundaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}->{} boundary (token: {})",
            self.kind, self.from, self.to, self.token_name
        )
    }
}

impl std::error::Error for AuthBoundaryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: AuthBoundaryKind) -> AuthBoundaryError {
        AuthBoundaryError::new("MIKA_INTERNAL_TOKEN", "gateway", "spirit", kind)
    }

    #[test]
    fn round_trips_every_kind() {
        for kind in AuthBoundaryKind::ALL {
            let err = sample(kind);
            let json = serde_json::to_string(&err).expect("serialize");
            let back: AuthBoundaryError = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(err, back, "round-trip lost information for {kind}");
            assert!(
                json.contains(kind.as_str()),
                "wire form must spell the kind as `{}`, got {json}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn display_names_kind_boundary_and_token() {
        let err = sample(AuthBoundaryKind::Rejected);
        assert_eq!(
            err.to_string(),
            "rejected at gateway->spirit boundary (token: MIKA_INTERNAL_TOKEN)"
        );
    }

    #[test]
    fn boundary_key_is_the_audit_target_key_grammar() {
        assert_eq!(
            sample(AuthBoundaryKind::Missing).boundary_key(),
            "gateway_to_spirit"
        );
        let cm = AuthBoundaryError::new("INTERNAL_TOKEN", "cm", "spirit", AuthBoundaryKind::Empty);
        assert_eq!(cm.boundary_key(), "cm_to_spirit");
    }

    /// Negative control for the one invariant: an instance built with a token
    /// *name* that is itself a plausible secret renders the name and nothing
    /// else. If a future field ever carried a value, this test is what goes
    /// red — the struct has no constructor path that accepts one.
    #[test]
    fn never_renders_a_value_it_was_not_given() {
        // A 64-hex-shaped string used as the *name*, so any leak of a
        // value-shaped field would be visible. There is no field to put a
        // real value in; this asserts that stays true.
        let secret_shaped = "a".repeat(64);
        let err = AuthBoundaryError::new(
            secret_shaped.clone(),
            "manager",
            "delivery",
            AuthBoundaryKind::Rejected,
        );
        let rendered = err.to_string();
        let json = serde_json::to_string(&err).expect("serialize");

        // Exactly one occurrence in each surface: the name slot. More than one
        // would mean some other field had picked the value up.
        assert_eq!(rendered.matches(&secret_shaped).count(), 1, "{rendered}");
        assert_eq!(json.matches(&secret_shaped).count(), 1, "{json}");

        // And the struct has exactly the four documented fields — a fifth,
        // added later to carry "just the prefix for debugging", goes red here.
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        let obj = value.as_object().expect("object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["from", "kind", "to", "token_name"]);
    }
}
