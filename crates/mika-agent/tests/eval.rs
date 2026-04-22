//! Integration tests for the agent eval harness.
//!
//! Run with: `cargo test -p mika-agent --test eval`

#[allow(dead_code)]
mod eval {
    pub mod assertions;
    pub mod harness;
    pub mod trace;

    mod test_basic_conversation;
    mod test_callback_turn;
    mod test_completion_claim_guard;
    mod test_di_builders;
    mod test_error_handling;
    mod test_intent_precondition_guard;
    mod test_internal_tagging;
    mod test_multi_step;
    mod test_persistence_eval_guard;
    mod test_phantom_retry_guard;
    mod test_pr_review_idempotency;
    mod test_required_tools_gate;
    mod test_self_knowledge_kg;
    mod test_task_not_found_retry;
    mod test_tool_calling;
    mod test_verdict_handler;
    mod test_webhook_queue;
    mod test_webhook_zero_tools_guard;
}
