//! Integration tests for the agent eval harness.
//!
//! Run with: `cargo test -p mika-agent --test eval`

#[allow(dead_code)]
mod eval {
    pub mod assertions;
    pub mod calibration;
    pub mod harness;
    pub mod providers;
    pub mod scenarios;
    pub mod trace;

    // Golden dataset: 25 curated scenarios for end-to-end quality testing (#339)
    pub mod golden;

    // KG fixture helpers: shared seeding for KG eval scenarios (#740, #741)
    pub mod kg_fixtures;

    // KG self-knowledge: 7 scenarios for KG-backed self-knowledge (#740)
    pub mod kg_self_knowledge;

    // Grounding assertion helpers: shared for fabrication-detection scenarios (#741)
    pub mod grounding_assertions;

    // Grounding + fabrication regression: 5 scenarios from KG retrospective (#741)
    pub mod grounding_regressions;

    mod test_basic_conversation;
    mod test_callback_turn;
    mod test_completion_claim_guard;
    mod test_di_builders;
    mod test_error_handling;
    mod test_intent_precondition_guard;
    mod test_internal_tagging;
    mod test_kg_budget_757;
    mod test_max_steps_continuation;
    mod test_multi_step;
    mod test_multi_turn_persistence;
    mod test_per_skill_provider_override;
    mod test_persistence_eval_guard;
    mod test_phantom_retry_guard;
    mod test_pr_review_idempotency;
    mod test_real_provider_matrix;
    mod test_request_wellformedness;
    mod test_required_tools_gate;
    mod test_schema_divergence;
    mod test_self_knowledge_kg;
    mod test_task_not_found_retry;
    mod test_tool_calling;
    mod test_verdict_handler;
    mod test_webhook_queue;
    mod test_webhook_zero_tools_guard;
}
