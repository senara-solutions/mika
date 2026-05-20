//! Shared predicates for webhook dispatch gating (mika#933).
//!
//! Both `agent.rs` (INTENT_GUARDS post-hoc) and `skills/executor.rs`
//! (tool-boundary pre-hoc) consume these predicates. Single source of truth
//! prevents drift between the two guard layers.

/// Marker prefix emitted by `mika_gateway::github::format_event_text` for
/// `issues.labeled` events where the label name is `ready`. Re-exported
/// from `mika_common::github_event_format` for cross-crate single-source-of-
/// truth coupling. See mika#852.
pub(crate) use mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER;

/// True when the message is a `[GitHub]` webhook event in the
/// **Webhook Fallthrough** domain — i.e., a turn that MUST NOT call
/// `run_claude_pilot`. The fallthrough domain is the complement of:
/// (a) the authorized ready-label dispatch marker, and (b) the qa/ci
/// handler-skill territory (PR events, check suites).
///
/// Allowlist rationale: the gateway emits `[GitHub] PR ...` and `[GitHub]
/// Check suite ...` prefixes specifically for events that `self-dev-webhook-qa`
/// and `self-dev-webhook-ci` activate on. Those skills own legitimate
/// `run_claude_pilot` dispatch flows (CI-fix iteration, QA hold retries) and
/// must not be blocked. The fallthrough rejection scope is exactly the
/// `[GitHub] Issue ...` / `[GitHub] New comment on ...` / unknown-catchall
/// surface where no handler skill activates — the same scope the self-dev
/// prompt's Webhook Fallthrough section governs.
///
/// Mutually exclusive with `is_ready_label_dispatch_marker` on the
/// `[GitHub]` domain (mika#910).
pub(crate) fn is_unauthorized_webhook_dispatch(msg: &str) -> bool {
    if !msg.starts_with("[GitHub]") {
        return false;
    }
    if msg.starts_with(READY_LABEL_DISPATCH_MARKER) {
        return false;
    }
    // qa skill territory (Phase 0 prefix surface rows E, F).
    if msg.starts_with("[GitHub] PR ") {
        return false;
    }
    // ci skill territory (Phase 0 prefix surface row G).
    if msg.starts_with("[GitHub] Check suite ") {
        return false;
    }
    // Everything else in [GitHub] domain (rows B, C, D, H) is fallthrough.
    true
}

/// True when the message matches the ready-label dispatch marker prefix.
pub(crate) fn is_ready_label_dispatch_marker(msg: &str) -> bool {
    msg.starts_with(READY_LABEL_DISPATCH_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exhaustive matrix mapped to the Phase 0 prefix surface table in the
    /// plan (docs/plans/2026-05-13-002-fix-933-webhook-fallthrough-readygate-plan.md).
    #[test]
    fn test_is_unauthorized_webhook_dispatch_predicate() {
        // Row A — authorized ready-label dispatch → false
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"
            ),
            "Row A: ready-label dispatch must be allowed"
        );

        // Row B — non-ready label → true (fallthrough)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue labeled bug on senara-solutions/mika#999"
            ),
            "Row B: non-ready label must be rejected"
        );
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue labeled p1-important on senara-solutions/mika#999"
            ),
            "Row B: non-ready label must be rejected"
        );

        // Row C — issue actions → true (fallthrough)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue opened: senara-solutions/mika#100 — title"
            ),
            "Row C: issue opened must be rejected"
        );
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] Issue assigned: senara-solutions/mika#100 — title"
            ),
            "Row C: issue assigned must be rejected"
        );

        // Row D — issue comments → true (the mika#932 incident class)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko"
            ),
            "Row D: issue comment must be rejected"
        );

        // Row E — PR events → false (qa skill territory)
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR opened: senara-solutions/mika#1000 — title (branch: foo)"
            ),
            "Row E: PR opened must be allowed (qa skill territory)"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)"
            ),
            "Row E: PR closed must be allowed (qa skill territory)"
        );

        // Row F — PR reviews → false (qa skill territory)
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer"
            ),
            "Row F: PR review approved must be allowed (qa skill territory)"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] PR review (changes_requested) on senara-solutions/mika#1000 (title) by @reviewer"
            ),
            "Row F: PR review changes_requested must be allowed (qa skill territory)"
        );

        // Row G — check suites → false (ci skill territory)
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)"
            ),
            "Row G: check suite failure must be allowed (ci skill territory)"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(
                "[GitHub] Check suite success on senara-solutions/mika (branch: main)"
            ),
            "Row G: check suite success must be allowed (ci skill territory)"
        );

        // Row H — unknown event catchall → true (fail-closed)
        assert!(
            is_unauthorized_webhook_dispatch(
                "[GitHub] discussion.created on senara-solutions/mika"
            ),
            "Row H: unknown event type must be rejected (fail-closed)"
        );

        // Non-domain — not a [GitHub] prefix → false
        assert!(
            !is_unauthorized_webhook_dispatch("[claude-pilot] callback ..."),
            "Non-domain: claude-pilot prefix must not be caught"
        );
        assert!(
            !is_unauthorized_webhook_dispatch(""),
            "Non-domain: empty string must not be caught"
        );
        assert!(
            !is_unauthorized_webhook_dispatch("Implement mika#933"),
            "Non-domain: direct mika ask prompt must not be caught"
        );
    }

    #[test]
    fn test_is_ready_label_dispatch_marker() {
        assert!(is_ready_label_dispatch_marker(
            "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"
        ));
        assert!(!is_ready_label_dispatch_marker(
            "[GitHub] Issue labeled bug on senara-solutions/mika#933"
        ));
        assert!(!is_ready_label_dispatch_marker(
            "[GitHub] New comment on senara-solutions/mika#933"
        ));
        assert!(!is_ready_label_dispatch_marker("not a github event"));
    }
}
