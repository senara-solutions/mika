//! Post-condition guard registry for the agent loop (#771).
//!
//! Guards that fire at EndTurn time with the reject-and-reprompt pattern.
//! The registry provides label-driven dispatch, retry tracking, and structured
//! logging. Guard evaluation logic lives in `agent.rs` (dispatched via `match
//! guard.label`) because some evaluators need `async` + DB access.
//!
//! The send_message turn boundary guard is NOT in this registry — its enforcement
//! is upstream of EndTurn (intra-step gating + inter-step break). See agent.rs.

use std::collections::HashSet;

use crate::agent::ToolCallSummary;
use crate::tools::ToolRegistry;
use mika_common::llm::LlmStopReason;

/// A post-condition guard evaluated at EndTurn.
pub struct PostConditionGuard {
    /// Unique label for retry-tracking and structured logging.
    pub label: &'static str,
    /// Human-readable description for debug output.
    pub description: &'static str,
}

/// Decision from a post-condition guard evaluation.
pub enum GuardDecision {
    /// No violation detected.
    Pass,
    /// Reject EndTurn and re-prompt with a correction message.
    RejectEndTurn { correction: String },
}

/// Context available to post-condition guard evaluators.
pub struct PostConditionContext<'a> {
    pub text: &'a str,
    pub all_tool_summaries: &'a [ToolCallSummary],
    pub tools_called: &'a HashSet<String>,
    pub tools: &'a ToolRegistry,
    pub stop_reason: &'a LlmStopReason,
}

/// Registry of post-condition guards evaluated at EndTurn.
///
/// Currently contains only the completion-claim guard (#483). Other inline
/// guards remain inline — their shapes are diverse enough that premature
/// extraction would add complexity without benefit. The registry establishes
/// the pattern; future guards can opt in incrementally.
pub const POST_CONDITION_GUARDS: &[PostConditionGuard] = &[PostConditionGuard {
    label: "completion_claim",
    description: "Reject EndTurn when agent claims completion without update_task_status",
}];
