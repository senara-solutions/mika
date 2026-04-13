//! Integration tests for the agent eval harness.
//!
//! Run with: `cargo test -p mika-agent --test eval`

#[allow(dead_code)]
mod eval {
    pub mod assertions;
    pub mod harness;
    pub mod trace;

    mod test_basic_conversation;
    mod test_completion_claim_guard;
    mod test_error_handling;
    mod test_multi_step;
    mod test_tool_calling;
    mod test_verdict_handler;
}
