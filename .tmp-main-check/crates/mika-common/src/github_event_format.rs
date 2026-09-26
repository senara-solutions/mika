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

/// Prefix of the **last** line `format_event_text` appends to an
/// `issues.labeled` event, naming the GitHub identity that applied the label
/// (mika#2323). Full line shape: `Labeled by: @samidarko`.
///
/// # Why this exists
///
/// mika#2323 asked whether the ready-label handler carries an actor filter.
/// It does not, and it could not: `GitHubWebhookEvent.sender` was deserialized
/// by the gateway and **never emitted**, so the agent had no identity to filter
/// on even had it wanted to. This line closes that gap so the question is
/// answerable next time without re-running the investigation.
///
/// # Hard invariant — informational only
///
/// **No refusal decision reads this value.** It is a log field and an audit
/// field; wiring it into a predicate would create exactly the actor filter
/// mika#2323 established does not exist, and which that ticket places out of
/// scope. Enforced by the source scan
/// `mika2323_no_gate_predicate_reads_the_actor` in
/// `mika_agent::server::ready_label_handler`.
///
/// # Why appending here is safe for the ready-label parse
///
/// The line is appended **after** the existing text, so
/// `starts_with(READY_LABEL_DISPATCH_MARKER)` is untouched, and
/// `parse_ready_label_location` bounds its `<repo>#<n>` token at the first
/// whitespace after the marker — a trailing line cannot reach it. Pinned by
/// `mika2323_actor_line_preserves_the_marker_and_the_parse`.
///
/// Producer: `mika_gateway::github::format_event_text` (`issues.labeled` arm).
/// Consumer: `mika_agent::server::ready_label_handler::parse_event_actor`.
pub const LABELED_BY_LINE_PREFIX: &str = "Labeled by: @";
