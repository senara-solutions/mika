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

    // Distribution Doctrine regression: public-promo suppression scenarios (mika#1814)
    pub mod doctrine_regressions;

    // KG provider evaluation matrix: provider comparison for extraction + resolution (#762)
    pub mod kg_provider_eval;

    // Per-skill eval scenarios — output-shape contracts validated through the
    // agent loop with synthetic skills + mock LLM (mika#879 Unit 1 onwards).
    pub mod skills;

    // v26->v27 KG migration invariant tests: coalesce per-agent data (#787)
    mod kg_v27_migration;

    mod test_auto_groom_dispatch;
    mod test_basic_conversation;
    mod test_callback_milestone_advance;
    mod test_callback_terminal_action;
    mod test_callback_turn;
    mod test_completion_claim_guard;
    mod test_context_summary_inject;
    mod test_correction_message_classifier_guard;
    mod test_deadline_in_flight_llm_call;
    mod test_deferred_dispatch_idempotent_ack;
    mod test_di_builders;
    mod test_dispatch_no_grooming_marker_guard;
    mod test_dispatch_task_has_open_pr_guard;
    mod test_error_handling;
    mod test_intent_precondition_guard;
    mod test_internal_tagging;
    mod test_kg_budget_757;
    mod test_max_steps_continuation;
    mod test_multi_step;
    mod test_multi_turn_persistence;
    mod test_per_corpus_fairness_927;
    mod test_per_skill_provider_override;
    mod test_persistence_eval_guard;
    mod test_phantom_retry_guard;
    mod test_phantom_task_row_sweep;
    mod test_pr_review_idempotency;
    mod test_ready_label_grooming_guard;
    mod test_real_provider_matrix;
    mod test_request_wellformedness;
    mod test_required_tools_gate;
    mod test_schema_divergence;
    mod test_self_knowledge_kg;
    mod test_task_not_found_retry;
    mod test_tool_call_secret_redaction;
    mod test_tool_call_stream_emission;
    mod test_tool_calling;
    mod test_unauthorized_webhook_dispatch_tool_boundary;
    mod test_verdict_handler;
    mod test_webhook_no_unauthorized_dispatch_guard;
    mod test_webhook_queue;
    mod test_webhook_zero_tools_guard;

    // qa-review skill-scoped run_gh validator wiring test (mika#1196)
    mod test_qa_review_run_gh_scope_validator;

    // Self-dev-callback engine consistency: documented callback-handler branches
    // produce tool calls the engine accepts or defers (mika#806).
    mod test_self_dev_callback_engine_consistency;

    // Send-message turn boundary guard: prevents write tools after
    // send_message in conversation mode (#771).
    mod test_send_message_boundary;

    // Multi-agent corpus parity: regression guard for #1155 search_content gap
    mod kg_multi_agent_corpus_parity;

    // Compact provider gate: MikaModel request shape regression (mika#1491)
    mod test_compact_provider_gate;
}
