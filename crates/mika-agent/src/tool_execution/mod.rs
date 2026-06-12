//! Tool execution — dispatch, MCP integration, exec handlers, dispatch gates.
//!
//! Operational responsibility (per Foundation §6): routes tool-use blocks from
//! LLM responses through the three-tier dispatch chain (builtin → skill → MCP),
//! manages per-tool timeouts, per-turn dedup, image-budget accounting, and
//! persists tool-call metadata to SQLite.

pub(crate) mod dispatch;
pub mod types;

// Re-export primary public types for ergonomic use from agent.rs
pub(crate) use dispatch::process_tool_calls;
pub use types::{
    ToolCallSummary, format_tool_summary_block, has_non_zero_exit_prefix, tool_calls_metadata_json,
    truncate_summary,
};
