//! Shared format-prefix constants for GitHub webhook event text emitted by
//! `mika-gateway::github::format_event_text` and consumed by
//! `mika-agent::webhook_dispatch`. Single source of truth — drift between
//! producer and consumer is a contract violation; the cross-crate test in
//! `mika-gateway::github::tests` enforces the contract at CI time.

/// Prefix emitted by `format_event_text` for `issues.labeled` events
/// where the label name is `ready`. The trailing space is significant —
/// the consumer parses `<repo>#<n>` immediately after the prefix.
///
/// Producer: `mika_gateway::github::format_event_text` (`issues.labeled`
/// arm; constructs via `format!`, asserted to match this prefix in test
/// `test_format_event_text_issue_labeled_extracts_label_name`).
///
/// Consumers: `mika_agent::webhook_dispatch` (`is_ready_label_dispatch_marker`,
/// `is_unauthorized_webhook_dispatch`); `mika_agent::agent`
/// (`parse_ready_label_location`).
pub const READY_LABEL_DISPATCH_MARKER: &str = "[GitHub] Issue labeled ready on ";
