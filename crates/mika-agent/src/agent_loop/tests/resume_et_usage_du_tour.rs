//! Résumé plafonné en mode silencieux et usage déclaré du tour.
//!
//! `load_gated_summary` et son budget mode-conditionnel (mika#1021), la porte
//! du provider compact (mika#1491), `build_turn_usage_fields` (mika#1889), la
//! taille que le tour déclare y compris quand il échoue (mika#2331) et le
//! classifieur d'issue de délégation (mika#1671).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- load_gated_summary tests (Axis 3 — mika#1021) --

/// Seed a summary into the async DB by saving enough messages and compacting.
async fn seed_summary(db: &AsyncDatabase, content: &str) {
    // Save enough messages so replace_with_summary has something to compact.
    for i in 0..5 {
        db.save_message("test-session", "user", &format!("msg {i}"), None)
            .await
            .unwrap();
    }
    let old = db.load_messages_before_window(3).await.unwrap();
    let highest_id = old.last().unwrap().id;
    db.replace_with_summary(content, highest_id).await.unwrap();
}

fn make_callback_trigger() -> SilentTrigger {
    SilentTrigger::Callback {
        task_id: "test-task".to_string(),
        label: "test".to_string(),
        result: "done".to_string(),
        failed: false,
        parent_task_id: None,
    }
}

#[tokio::test]
async fn gated_summary_no_cap_silent_returns_full() {
    // max_tokens = None, silent turn, summary present → returns full summary (regression).
    let db = test_async_db();
    seed_summary(&db, "Full summary content here").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: None,
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, Some("Full summary content here".to_string()));
}

#[tokio::test]
async fn gated_summary_zero_sentinel_silent_returns_none() {
    // max_tokens = Some(0), silent turn → returns None (load-omit sentinel).
    let db = test_async_db();
    seed_summary(&db, "Should be omitted").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(0),
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn gated_summary_zero_sentinel_non_silent_returns_full() {
    // max_tokens = Some(0), non-silent turn → returns full (cap gated by mode).
    let db = test_async_db();
    seed_summary(&db, "Full content non-silent").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(0),
    };
    let result = load_gated_summary(&db, &config, None).await.unwrap();
    assert_eq!(result, Some("Full content non-silent".to_string()));
}

#[tokio::test]
async fn gated_summary_cap_truncates_in_silent() {
    // max_tokens = Some(n), silent turn, summary > n tokens → truncated.
    let db = test_async_db();
    let long_summary = "word ".repeat(500); // 2500 chars, way over budget
    seed_summary(&db, &long_summary).await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(100), // 400 char budget
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    let content = result.unwrap();
    assert!(content.contains("[… summary truncated to fit silent-mode budget …]"));
    // Total truncated content (before marker) should be ≤ 400 chars
    let before_marker = content
        .strip_suffix("\n[… summary truncated to fit silent-mode budget …]")
        .unwrap();
    assert!(before_marker.len() <= 400);
}

#[tokio::test]
async fn gated_summary_cap_under_budget_returns_full() {
    // max_tokens = Some(n), silent turn, summary < n tokens → full summary.
    let db = test_async_db();
    seed_summary(&db, "Short summary").await;

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: Some(1000), // 4000 char budget, summary is tiny
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, Some("Short summary".to_string()));
}

#[tokio::test]
async fn gated_summary_axis4_wins_over_axis3() {
    // Axis 4 inject = false + any max_tokens → returns None without DB call.
    let db = test_async_db();
    // Don't even seed a summary — Axis 4 should short-circuit before DB call.

    let config = prompt::ContextSummaryConfig {
        inject: false,
        max_tokens: Some(500),
    };
    let trigger = make_callback_trigger();
    let result = load_gated_summary(&db, &config, Some(&trigger))
        .await
        .unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn gated_summary_no_summary_stored() {
    // inject = true, max_tokens = None, no summary in DB → returns None.
    let db = test_async_db();
    // No summary seeded.

    let config = prompt::ContextSummaryConfig {
        inject: true,
        max_tokens: None,
    };
    let result = load_gated_summary(&db, &config, None).await.unwrap();
    assert_eq!(result, None);
}

// -- Compact provider gate tests (mika#1491) --

#[test]
fn test_inject_skills_compact_provider_filters_tools() {
    // When is_compact_provider = true: no skill prompts appended,
    // tools filtered to COMPACT_PROVIDER_CORE_TOOLS only, per_skill_bytes empty.
    let tools = crate::tools::default_tools();
    let mut entry = make_skill_entry("self-dev", 30, &["run_claude_pilot"]);
    entry.prompt_snippet = "A very large skill prompt that should NOT appear.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base compact prompt.".to_string();

    let no_ctx = HashMap::new();
    let no_disabled: Vec<String> = vec![];
    let (defs, variant, per_skill_bytes) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "mikamodel",
        "mika",
        &no_ctx,
        "mika-test",
        &no_disabled,
        None,
        true, // compact provider
    );

    // No skill prompts should be appended
    assert!(
        !system.contains("<context type=\"skill\""),
        "compact provider should skip all skill prompts"
    );
    assert!(
        !system.contains("self-dev Skill"),
        "compact provider should not inject skill name"
    );

    // Only core tools should be present
    let tool_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    for core_tool in COMPACT_PROVIDER_CORE_TOOLS {
        assert!(
            tool_names.contains(core_tool),
            "core tool '{core_tool}' should be in compact tool set"
        );
    }
    assert!(
        defs.len() <= COMPACT_PROVIDER_CORE_TOOLS.len(),
        "compact provider should have at most {} tools, got {}",
        COMPACT_PROVIDER_CORE_TOOLS.len(),
        defs.len()
    );
    // Skill-defined tools should NOT be present
    assert!(
        !tool_names.contains(&"run_claude_pilot"),
        "skill-defined tools should be excluded in compact mode"
    );

    // No variant info (skills skipped)
    assert!(variant.is_none());
    // No per-skill bytes
    assert!(per_skill_bytes.is_empty());
}

#[test]
fn test_inject_skills_non_compact_provider_unchanged() {
    // When is_compact_provider = false: behavior unchanged — skills appended, all tools present.
    let tools = crate::tools::default_tools();
    let mut entry = make_skill_entry("test-skill", 30, &["test_tool"]);
    entry.prompt_snippet = "Use this skill.".to_string();
    let matched: Vec<&SkillEntry> = vec![&entry];
    let mut system = "Base prompt.".to_string();

    let no_ctx = HashMap::new();
    let no_disabled: Vec<String> = vec![];
    let (defs, variant, per_skill_bytes) = inject_skills_and_resolve_tools(
        &matched,
        &tools,
        &mut system,
        "anthropic",
        "claude-sonnet-4-6",
        &no_ctx,
        "mika-test",
        &no_disabled,
        None,
        false, // non-compact
    );

    // Skills should be appended
    assert!(
        system.contains("<context type=\"skill\""),
        "non-compact provider should include skill prompts"
    );
    assert!(system.contains("test-skill Skill"));

    // All default tools + skill tools should be present
    assert!(
        defs.len() > COMPACT_PROVIDER_CORE_TOOLS.len(),
        "non-compact should have more tools than core set"
    );
    let tool_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        tool_names.contains(&"test_tool"),
        "skill tools should be included"
    );

    // Variant and per_skill_bytes should be populated
    assert!(variant.is_some());
    assert!(!per_skill_bytes.is_empty());
}

#[test]
fn test_compact_provider_core_tools_are_valid() {
    // Every name in COMPACT_PROVIDER_CORE_TOOLS must exist in default_tools().
    // Prevents drift when tools are renamed or removed.
    let tools = crate::tools::default_tools();
    let tool_names: std::collections::HashSet<String> =
        tools.definitions().iter().map(|d| d.name.clone()).collect();

    for &core_tool in COMPACT_PROVIDER_CORE_TOOLS {
        assert!(
            tool_names.contains(core_tool),
            "COMPACT_PROVIDER_CORE_TOOLS entry '{core_tool}' not found in default_tools() — \
                 was it renamed or removed?"
        );
    }
}

// ===========================================================================
// mika#1889 — Per-turn usage/token logging (RT-005 brick 4/5)
//
// Structural coverage of the `build_turn_usage_fields` pure builder. The
// token/dimension mapping is tested directly here so we do not depend on a
// captured tracing subscriber (D5). The emission site itself is exercised
// no-panic by the existing eval harness in `tests/eval/`.
// ===========================================================================

fn usage_with_cache() -> LlmUsage {
    LlmUsage {
        input_tokens: 1234,
        output_tokens: 567,
        cache_creation_input_tokens: Some(89),
        cache_read_input_tokens: Some(4321),
    }
}

fn usage_without_cache() -> LlmUsage {
    LlmUsage {
        input_tokens: 10,
        output_tokens: 20,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    }
}

#[test]
fn build_turn_usage_success_with_cache_passes_through_tokens() {
    let u = usage_with_cache();
    let f = build_turn_usage_fields(
        3,
        Some(&u),
        "ToolUse",
        true,
        "success",
        250,
        None,
        None,
        None,
    );
    assert_eq!(f.step, 3);
    assert_eq!(f.input_tokens, 1234);
    assert_eq!(f.output_tokens, 567);
    // Anthropic-only cache fields: `Option<u64>` flattened to `0` when absent —
    // here `Some(4321)` and `Some(89)` are preserved verbatim.
    assert_eq!(f.cache_read_tokens, 4321);
    assert_eq!(f.cache_write_tokens, 89);
    assert_eq!(f.latency_ms, 250);
    assert_eq!(f.stop_reason, "ToolUse");
    assert!(f.tool_use_in_turn);
    assert_eq!(f.status, "success");
}

#[test]
fn build_turn_usage_success_no_cache_flattens_option_to_zero() {
    // Non-Anthropic providers report None for cache_creation; some
    // OpenAI-compat providers also report None for cache_read. The log MUST
    // still be jq-parseable unconditionally — `None` → `0`, never a missing
    // field. This is the load-bearing analyzer-shape invariant.
    let u = usage_without_cache();
    let f = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        None,
        None,
        None,
    );
    assert_eq!(f.input_tokens, 10);
    assert_eq!(f.output_tokens, 20);
    assert_eq!(f.cache_read_tokens, 0);
    assert_eq!(f.cache_write_tokens, 0);
    assert_eq!(f.latency_ms, 0);
    assert!(!f.tool_use_in_turn);
}

#[test]
fn build_turn_usage_error_arm_zeros_all_tokens() {
    // Error/timeout arms have no `LlmUsage`. R3 mandates the event still
    // fires so the covariable "turns" count is not silently undercounted —
    // the tokens roll to zero but the row exists.
    let f = build_turn_usage_fields(7, None, "error", false, "error", 42, None, None, None);
    assert_eq!(f.step, 7);
    assert_eq!(f.input_tokens, 0);
    assert_eq!(f.output_tokens, 0);
    assert_eq!(f.cache_read_tokens, 0);
    assert_eq!(f.cache_write_tokens, 0);
    // latency_ms is preserved even on the error arm — the transport spent
    // wall-clock even if it produced no tokens.
    assert_eq!(f.latency_ms, 42);
    assert_eq!(f.stop_reason, "error");
    assert!(!f.tool_use_in_turn);
    assert_eq!(f.status, "error");
}

#[test]
fn build_turn_usage_continuation_step_sentinel_preserved() {
    // The continuation turn uses `u32::MAX` as the step sentinel in both
    // the DB path (`save_continuation_llm_call`) and the log surface, so
    // the offline analyzer (brick 5/5) can distinguish continuation-turn
    // usage from in-loop step indices without a separate flag.
    let u = usage_without_cache();
    let f = build_turn_usage_fields(
        u32::MAX,
        Some(&u),
        "EndTurn",
        false,
        "success",
        100,
        None,
        None,
        None,
    );
    assert_eq!(f.step, u32::MAX);
}

#[test]
fn build_turn_usage_tool_use_boolean_is_raw_observation() {
    // Prime hard condition #1 (planning-tokens = SEUL primary outcome):
    // `tool_use_in_turn` is a RAW mechanical observation — did the model
    // request tool calls this turn — not a graded planning/execution
    // classification. Verified here by exercising both truth values with
    // otherwise-identical inputs.
    let u = usage_without_cache();
    let f_true =
        build_turn_usage_fields(1, Some(&u), "ToolUse", true, "success", 0, None, None, None);
    let f_false = build_turn_usage_fields(
        1,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        None,
        None,
        None,
    );
    assert!(f_true.tool_use_in_turn);
    assert!(!f_false.tool_use_in_turn);
    // No `phase`/`is_planning`/`role` field exists on the struct — D1/R5
    // enforced by absence (the field simply does not exist). This test
    // documents that invariant structurally.
}

#[test]
fn build_turn_usage_latency_ms_is_raw_pass_through() {
    // latency_ms is R1's turn-cost covariable. It's a raw transport
    // measurement (wall-clock of the HTTP call), not an estimand
    // component. Verified here as a pure pass-through.
    let u = usage_without_cache();
    let f = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        12345,
        None,
        None,
        None,
    );
    assert_eq!(f.latency_ms, 12345);
}

// ===========================================================================
// mika#2331 (AC1) — the turn says its size, including when it fails
// ===========================================================================

/// The load-bearing distinction of D6: `None` and `Some(0)` are different
/// answers. No request is empty, so a `0` would be a readable lie about a
/// measurement that did not happen; `null` says "not measured".
#[test]
fn mika2331_request_bytes_none_and_some_do_not_collapse() {
    let u = usage_without_cache();
    let measured = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        Some(59_812),
        Some(48_000),
        None,
    );
    assert_eq!(measured.request_bytes, Some(59_812));
    assert_eq!(measured.system_prompt_bytes, Some(48_000));

    let unmeasured = build_turn_usage_fields(
        0,
        Some(&u),
        "EndTurn",
        false,
        "success",
        0,
        None,
        None,
        None,
    );
    assert_eq!(unmeasured.request_bytes, None);
    assert_eq!(unmeasured.system_prompt_bytes, None);
    assert_ne!(unmeasured.request_bytes, Some(0));
}

/// The arm the whole of AC1 is about. A hung call returns no `usage`, so
/// every token count is zero — and until now that zero was the line's only
/// statement about how big the brief was. The two sizes are measured before
/// the call and must survive it.
#[test]
fn mika2331_error_arm_still_carries_the_brief_size() {
    let f = build_turn_usage_fields(
        4,
        None,
        "error",
        false,
        "error",
        420_000,
        Some(59_812),
        Some(48_000),
        None,
    );
    assert_eq!(f.input_tokens, 0, "no usage on the error arm — unchanged");
    assert_eq!(
        f.request_bytes,
        Some(59_812),
        "the size of a brief that timed out is exactly what a 420 s hang needs to be read"
    );
    assert_eq!(f.system_prompt_bytes, Some(48_000));
}

// ===========================================================================
// mika#1671 (AC2) — delegation-outcome classifier ruleset
//
// D2 is strict transport-only. The classifier must qualify: HTTP 503 (founding
// incident), transport/connection/dns error substrings, and deadline-exceeded
// (mapped from `TeamAgentOutcome::TimedOut`). It must NOT qualify: apologetic
// business responses that arrive as a successful `Ok(response)`, and clean
// successes — those route to `Completed`/`BusinessLogicFailure` so a mixed
// iteration never short-circuits (AC4).
// ===========================================================================

/// Build a `ToolCallSummary` stub for classifier tests. `success == false`
/// mirrors a `ToolOutput::error` result (the only shape the classifier inspects).
fn summary(name: &str, output: &str, success: bool) -> ToolCallSummary {
    ToolCallSummary {
        step: 0,
        name: name.to_string(),
        input_summary: String::new(),
        output_summary: output.to_string(),
        success,
        non_zero_exit: false,
    }
}

#[test]
fn classify_503_from_a2a_call_qualifies() {
    // Founding-incident shape: a2a_call 503 surfaced as a failed tool result.
    let summaries = vec![summary(
        "a2a_call",
        "HTTP status server error (503 Service Unavailable) for url (...)",
        false,
    )];
    assert!(matches!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::TransportError { .. }
    ));
}

#[test]
fn classify_transport_error_substring_qualifies() {
    let summaries = vec![summary("a2a_call", "transport error: broken pipe", false)];
    assert!(matches!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::TransportError { .. }
    ));
}

#[test]
fn classify_connection_refused_and_dns_error_qualify() {
    for sig in ["connection refused", "dns error while resolving host"] {
        let summaries = vec![summary("a2a_call", sig, false)];
        assert!(
            matches!(
                classify_delegation_from_summaries(&summaries),
                DelegationOutcome::TransportError { .. }
            ),
            "signature {sig:?} must qualify as transport failure"
        );
    }
}

#[test]
fn classify_deadline_exceeded_qualifies_via_timed_out() {
    // A hard agent-loop deadline surfaces as `TeamAgentOutcome::TimedOut`, which
    // maps to a retryable transport error per D2 (deadline-exceeded qualifies).
    let outcome = TeamAgentOutcome::TimedOut(
        "Agent timed out while processing team task (deadline exceeded in run_loop).".to_string(),
    );
    assert!(matches!(
        outcome.delegation_outcome(),
        DelegationOutcome::TransportError {
            retryable: true,
            ..
        }
    ));
}

#[test]
fn classify_apologetic_business_response_does_not_qualify() {
    // The model said "I can't" but every tool call succeeded — a business-logic
    // apology arrives as `Ok(response)` and must NOT be a transport failure (AC4).
    let summaries = vec![
        summary("read_file", "file contents ...", true),
        summary(
            "send_message",
            "I'm sorry, I can't complete this task with the available data.",
            true,
        ),
    ];
    assert_eq!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_clean_success_does_not_qualify() {
    let summaries = vec![summary("read_file", "ok", true)];
    assert_eq!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_empty_summaries_is_completed() {
    assert_eq!(
        classify_delegation_from_summaries(&[]),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_successful_call_with_503_in_output_does_not_qualify() {
    // Only FAILED calls are inspected: a successful result that merely mentions
    // "503" in prose (e.g. the agent describing a status code) is not a transport
    // failure — guards against false positives on business content.
    let summaries = vec![summary(
        "send_message",
        "The upstream returned 503 earlier but recovered; here is the summary.",
        true,
    )];
    assert_eq!(
        classify_delegation_from_summaries(&summaries),
        DelegationOutcome::Completed
    );
}

#[test]
fn classify_error_string_transport_shaped_qualifies() {
    // Hard agent-loop `Err` with a transport-shaped string still counts.
    assert!(matches!(
        classify_delegation_from_error("connection refused by peer"),
        DelegationOutcome::TransportError { .. }
    ));
}

#[test]
fn classify_error_string_non_transport_is_business_failure() {
    // A hard `Err` that is not transport-shaped must NOT contribute to the
    // all-transport short-circuit (AC4) — it maps to BusinessLogicFailure.
    assert_eq!(
        classify_delegation_from_error("agent 'foo' not found in team resources"),
        DelegationOutcome::BusinessLogicFailure
    );
}
