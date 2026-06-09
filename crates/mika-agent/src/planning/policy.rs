//! Agent-loop policy constants — step budgets, timeouts, byte/char caps,
//! staleness thresholds. Per Foundation §6, `planning/` owns the rules
//! governing what the agent loop is allowed to do (budgets/timeouts/caps).
//!
//! Consumed by `crate::agent::RunMode::max_steps`,
//! `crate::agent::SilentTrigger::max_steps`, and other loop sites.
//! The impl methods that read these constants stay near their enums in
//! `agent.rs` (agent_loop/ #1452 domain); only the constants relocate.

pub const MAX_TOOL_STEPS: usize = 20;

pub const MAX_CALLBACK_TOOL_STEPS: usize = 20;

pub const MAX_TEAM_TOOL_STEPS: usize = 20;

pub const TOOL_TIMEOUT_SECS: u64 = 30;

pub const AGENT_TOTAL_TIMEOUT_SECS: u64 = 300;

/// Maximum bytes for callback results injected into the system prompt via
/// `format_callback_framing()`. Results exceeding this are truncated to prevent
/// oversized prompts from consuming the agent timeout during serialization.
/// Full results remain available in task logs.
pub const CALLBACK_RESULT_MAX_BYTES: usize = 10_240;

/// Per-agent timeout for team sub-agents (matches AGENT_TOTAL_TIMEOUT_SECS).
/// Since team agents run in parallel, the constraint is fitting within the global
/// team run budget (max of agent times, not sum).
pub const TEAM_AGENT_TIMEOUT_SECS: u64 = 300;

/// Timeout for the continuation API call after max tool steps are exceeded.
/// Longer than TOOL_TIMEOUT_SECS because this is a full generation call, not a tool.
pub const CONTINUATION_TIMEOUT_SECS: u64 = 60;

/// Maximum total base64 image bytes across all tool results in a single agent step.
/// Prevents memory spikes when multiple tools return images in one step.
/// 5 images at 5 MB each ≈ 33 MB base64 — this caps at ~20 MB to stay within
/// container memory limits (256 MB target).
pub const MAX_IMAGE_BYTES_PER_STEP: usize = 20 * 1024 * 1024;

/// Maximum age (in minutes) for a failed callback to be delivered to the agent.
/// Failed callbacks older than this are silently marked as delivered to prevent
/// flooding the conversation with stale failures (e.g., after an upgrade).
pub const STALE_FAILED_CALLBACK_MINUTES: i64 = 5;

/// Maximum total characters for serialized tool call metadata.
pub const TOOL_METADATA_MAX: usize = 4000;

/// Maximum characters for tool input summary in metadata.
pub const INPUT_SUMMARY_MAX: usize = 200;

/// Maximum characters for tool output summary in metadata.
pub const OUTPUT_SUMMARY_MAX: usize = 300;

/// Maximum characters of conversation/memory digest injected into the reflection prompt.
/// ~12,500 tokens at 4 chars/token -- keeps total prompt well within Claude's context.
pub const MAX_REFLECTION_DIGEST_CHARS: usize = 50_000;
