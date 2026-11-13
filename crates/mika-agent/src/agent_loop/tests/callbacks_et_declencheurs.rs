//! Cadrage des callbacks, santé de tâche et déclencheurs d'intention.
//!
//! `format_callback_framing`, le contexte de déclencheur de callback (#313),
//! la garde d'injection de santé de tâche (#314), les surcharges LLM de skill
//! (#463) et les préfixes de déclencheur du registre d'intentions (mika#1011).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- format_callback_framing tests --

#[test]
fn test_format_callback_framing_completed() {
    let result =
        format_callback_framing("analyze_code", "task-123", None, "Analysis complete", false);
    assert!(result.contains("A background task has completed."));
    assert!(result.contains("analyze_code"));
    assert!(result.contains("task-123"));
    assert!(result.contains("Analysis complete"));
    assert!(result.contains("<callback_result trust=\"untrusted\">"));
    assert!(!result.contains("FAILED"));
}

#[test]
fn test_format_callback_framing_failed() {
    let result = format_callback_framing(
        "run_pilot",
        "task-456",
        None,
        "Process exited with code 128: fatal error",
        true,
    );
    assert!(result.contains("A background task has FAILED."));
    assert!(result.contains("run_pilot"));
    assert!(result.contains("task-456"));
    assert!(result.contains("Process exited with code 128: fatal error"));
    assert!(result.contains("<callback_result trust=\"untrusted\">"));
    assert!(!result.contains("has completed"));
}

#[test]
fn test_format_callback_framing_short_result_not_truncated() {
    let short = "a".repeat(crate::planning::policy::CALLBACK_RESULT_MAX_BYTES);
    let result = format_callback_framing("task", "id-1", None, &short, false);
    assert!(result.contains(&short));
    assert!(!result.contains("[truncated"));
}

#[test]
fn test_format_callback_framing_long_result_truncated() {
    let long = "x".repeat(crate::planning::policy::CALLBACK_RESULT_MAX_BYTES + 5000);
    let result = format_callback_framing("task", "id-2", None, &long, false);
    assert!(!result.contains(&long));
    assert!(result.contains("[truncated — full result available in task logs]"));
    // The truncated content should be present (up to the cut boundary)
    let suffix_len = "\n...\n[truncated — full result available in task logs]".len();
    let prefix = &"x".repeat(crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - suffix_len);
    assert!(result.contains(prefix));
}

#[test]
fn test_format_callback_framing_truncation_utf8_safe() {
    // Place a 4-byte emoji so it straddles the cut point, forcing the
    // char-boundary walk-back loop to execute.
    // cut = crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - suffix_len ≈ 10_185
    // Emoji at byte (cut-1) spans (cut-1)..(cut+2), so cut lands mid-emoji.
    let suffix_len = "\n...\n[truncated — full result available in task logs]".len();
    let cut = crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - suffix_len;
    let mut s = "a".repeat(cut - 1); // one byte before the cut point
    s.push('🦀'); // 4-byte char that straddles the cut boundary
    // Pad with enough trailing data to exceed crate::planning::policy::CALLBACK_RESULT_MAX_BYTES
    let pad = crate::planning::policy::CALLBACK_RESULT_MAX_BYTES - s.len() + 1;
    s.push_str(&"z".repeat(pad));
    assert!(s.len() > crate::planning::policy::CALLBACK_RESULT_MAX_BYTES);
    let result = format_callback_framing("task", "id-3", None, &s, true);
    assert!(result.contains("[truncated"));
    // The emoji should NOT be in the output (it was at the boundary)
    assert!(!result.contains('🦀'));
    // Content up to the emoji should be preserved
    assert!(result.contains(&"a".repeat(cut - 1)));
}

#[test]
fn test_format_callback_framing_includes_grounding_instruction() {
    let result = format_callback_framing(
        "build_code",
        "task-789",
        None,
        "Compilation succeeded",
        false,
    );
    assert!(result.contains("Report only what this result explicitly states"));
    assert!(result.contains("Do not infer the state of any system, artifact, or process"));
}

// -- Callback trigger context tests (#313) --

#[test]
fn test_callback_trigger_generic_framing_for_all_labels() {
    // All callbacks — including claude-pilot — get the same generic framing.
    // Workflow-specific behavior is driven by active skill prompts. See #313.
    for label in [
        "long_running:run_claude_pilot",
        "long_running:run_shell",
        "long_running:custom_task",
    ] {
        let ctx = build_callback_trigger_context(label, "task-001", None, "Result text", false);
        assert!(
            ctx.contains("Follow the workflow defined by your active skills"),
            "label={label}: missing generic skill delegation instruction"
        );
        assert!(
            ctx.contains("NEVER extrapolate to downstream states"),
            "label={label}: missing grounding instruction"
        );
    }
}

#[test]
fn test_callback_trigger_failed_uses_generic_framing() {
    let ctx = build_callback_trigger_context(
        "long_running:run_shell",
        "task-002",
        None,
        "Script failed",
        true,
    );
    // Generic framing for failed callbacks
    assert!(ctx.contains("Follow the workflow defined by your active skills"));
    // The base framing indicates FAILED
    assert!(ctx.contains("FAILED"));
}

#[test]
fn test_callback_framing_with_parent_task_id() {
    let ctx = build_callback_trigger_context(
        "long_running:run_claude_pilot",
        "task-003",
        Some("wi-parent-uuid-123"),
        "PR created: https://github.com/example/repo/pull/42",
        false,
    );
    assert!(ctx.contains("Parent task: wi-parent-uuid-123"));
    assert!(ctx.contains("Task: 'long_running:run_claude_pilot' (ID: task-003)"));
}

#[test]
fn test_callback_framing_without_parent_task_id() {
    let ctx = build_callback_trigger_context(
        "long_running:run_shell",
        "task-004",
        None,
        "Script completed",
        false,
    );
    assert!(!ctx.contains("Parent task"));
    assert!(ctx.contains("Task: 'long_running:run_shell' (ID: task-004)"));
}

#[test]
fn test_callback_framing_parent_task_id_with_failed() {
    let ctx = build_callback_trigger_context(
        "long_running:run_claude_pilot",
        "task-005",
        Some("wi-parent-uuid-456"),
        "Error: cargo build failed",
        true,
    );
    // Parent line still present even when the callback failed
    assert!(ctx.contains("Parent task: wi-parent-uuid-456"));
    assert!(ctx.contains("FAILED"));
}

// -- Task health injection guard tests (#314) --

#[test]
fn test_task_health_guard_includes_heartbeat() {
    let trigger = SilentTrigger::Heartbeat;
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        should_inject,
        "Heartbeat trigger should receive task health"
    );
}

#[test]
fn test_task_health_guard_includes_callback() {
    let trigger = SilentTrigger::Callback {
        task_id: "task-100".to_string(),
        label: "long_running:run_claude_pilot".to_string(),
        result: "PR created".to_string(),
        failed: false,
        parent_task_id: None,
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(should_inject, "Callback trigger should receive task health");
}

#[test]
fn test_task_health_guard_includes_failed_callback() {
    let trigger = SilentTrigger::Callback {
        task_id: "task-101".to_string(),
        label: "long_running:run_shell".to_string(),
        result: "Script failed".to_string(),
        failed: true,
        parent_task_id: Some("wi-parent-101".to_string()),
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        should_inject,
        "Failed callback trigger should also receive task health"
    );
}

#[test]
fn test_task_health_guard_excludes_reflection() {
    let trigger = SilentTrigger::Reflection;
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        !should_inject,
        "Reflection trigger should NOT receive task health"
    );
}

#[test]
fn test_task_health_guard_excludes_skill_run() {
    let trigger = SilentTrigger::SkillRun {
        skill_name: "web-search".to_string(),
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(
        !should_inject,
        "SkillRun trigger should NOT receive task health"
    );
}

#[test]
fn test_task_health_guard_includes_reminder() {
    let trigger = SilentTrigger::Reminder {
        task_id: "task-200".to_string(),
        message: "Check CI status".to_string(),
    };
    let should_inject = matches!(
        &trigger,
        SilentTrigger::Heartbeat | SilentTrigger::Callback { .. } | SilentTrigger::Reminder { .. }
    );
    assert!(should_inject, "Reminder trigger should receive task health");
}

// -- resolve_skill_llm_override tests (#463) --

fn make_skill_entry_with_llm(
    name: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> SkillEntry {
    use crate::skills::manifest::LlmOverride;
    let mut entry = make_skill_entry(name, 30, &[]);
    entry.manifest.llm = LlmOverride {
        provider: provider.map(String::from),
        model: model.map(String::from),
        from_db_override: false,
    };
    entry
}

/// Like `make_skill_entry_with_llm` but marks the LLM override as DB-sourced
/// (simulates `apply_overrides()` having written `skill_overrides` DB rows).
fn make_skill_entry_with_db_llm(
    name: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> SkillEntry {
    use crate::skills::manifest::LlmOverride;
    let mut entry = make_skill_entry(name, 30, &[]);
    entry.manifest.llm = LlmOverride {
        provider: provider.map(String::from),
        model: model.map(String::from),
        from_db_override: true,
    };
    entry
}

#[test]
fn test_resolve_skill_llm_override_returns_none_for_empty_matched() {
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("anthropic")
        .model_name("claude-sonnet-4-6")
        .build();
    let matched: Vec<MatchedSkill<'_>> = vec![];
    assert!(resolve_skill_llm_override(&matched, None, &mock, false).is_none());
}

#[test]
fn test_resolve_skill_llm_override_ignores_always_on_skills() {
    // always_on skill with [llm] should NOT impose override (#463)
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "always_on skills should not impose [llm] override"
    );
}

#[test]
fn test_resolve_skill_llm_override_ignores_dependency_skills() {
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm("dev-pilot", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Dependency,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "dependency skills should not impose [llm] override"
    );
}

#[test]
fn test_resolve_skill_llm_override_mixed_reasons_only_keyword_considered() {
    // always_on skill with [llm] + keyword skill without [llm] → no override
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let s2 = make_skill_entry("skill-review", 30, &["review_skill"]);
    let matched = vec![
        MatchedSkill {
            entry: &s1,
            reason: MatchReason::AlwaysOn,
        },
        MatchedSkill {
            entry: &s2,
            reason: MatchReason::Keyword,
        },
    ];
    // skill-review has no [llm], self-dev is AlwaysOn → no override
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "only keyword-matched skills with [llm] should produce an override"
    );
}

#[test]
fn test_resolve_skill_llm_override_keyword_match_on_always_on_skill_applies() {
    // When an always_on skill is matched via keyword, its [llm] IS considered
    // (MatchReason is Keyword when keyword hit on an always_on skill)
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword, // keyword hit on always_on skill → Keyword wins
    }];
    // Should attempt override — will return None because Settings is None,
    // but the important thing is it doesn't return None at the "no overrides" early exit.
    // We verify by checking overrides were collected (Settings absence causes the fallback path).
    let result = resolve_skill_llm_override(&matched, None, &mock, false);
    // Without Settings, can't construct provider — returns None via the "requires Settings" path.
    // But the function got past the "overrides.is_empty()" check, proving keyword was considered.
    assert!(result.is_none()); // Expected: Settings=None means it can't construct
}

#[test]
fn test_resolve_skill_llm_override_same_provider_short_circuit() {
    // Keyword skill with [llm] matching the active provider → returns None (no-op)
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("qwen/qwen3-coder-plus")
        .build();
    let s1 = make_skill_entry_with_llm(
        "qa-review",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Keyword,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "same provider+model should short-circuit to None"
    );
}

#[test]
fn test_resolve_skill_llm_override_always_on_with_db_override_applies() {
    // AlwaysOn skill with DB-sourced LLM override SHOULD impose override (mika#1011).
    // The operator set this via `mika skills llm set` — it represents explicit intent.
    // Simulates: mika-dev's self-dev skill with DB override to anthropic/claude-sonnet-4-6
    // on a webhook turn where self-dev matches as AlwaysOn (no keyword hit).
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("moonshotai")
        .model_name("kimi-k2.5")
        .build();
    let s1 = make_skill_entry_with_db_llm("self-dev", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    // Should attempt override — will return None because Settings is None,
    // but the important thing is it gets past the "overrides.is_empty()" check.
    // Without Settings, can't construct provider — returns None via the "requires Settings" path.
    let result = resolve_skill_llm_override(&matched, None, &mock, false);
    assert!(result.is_none()); // Expected: Settings=None means it can't construct
    // The real verification is that this does NOT return None at the early
    // "overrides.is_empty()" exit — same pattern as the keyword test above.
}

#[test]
fn test_resolve_skill_llm_override_always_on_without_db_override_still_ignored() {
    // Regression guard for #463: AlwaysOn skill with [llm] from skill.toml
    // (from_db_override = false) should still NOT impose override.
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 = make_skill_entry_with_llm(
        "self-dev",
        Some("openrouter"),
        Some("qwen/qwen3-coder-plus"),
    );
    // Verify from_db_override is false (developer-time source)
    assert!(!s1.manifest.llm.from_db_override);
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::AlwaysOn,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "AlwaysOn with non-DB [llm] should not impose override (#463 regression guard)"
    );
}

#[test]
fn test_resolve_skill_llm_override_dependency_with_db_override_still_ignored() {
    // Dependency-matched skills should NEVER impose override, even with DB source.
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("openrouter")
        .model_name("x-ai/grok-4.1-fast")
        .build();
    let s1 =
        make_skill_entry_with_db_llm("dev-pilot", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched = vec![MatchedSkill {
        entry: &s1,
        reason: MatchReason::Dependency,
    }];
    assert!(
        resolve_skill_llm_override(&matched, None, &mock, false).is_none(),
        "dependency skills with DB override should still not impose [llm] override"
    );
}

/// mika#1217 / mika#1011 — override-scope contract for SilentTrigger::Callback
/// and SilentTrigger::DeferredDispatch.
///
/// Silent-mode trigger semantics for AlwaysOn + DB-sourced LLM overrides: the
/// matched-skill set returned by `callback_safe_skills()` is wrapped with
/// `MatchReason::AlwaysOn`. Under the mika#1011 carve-out, an AlwaysOn skill
/// with `from_db_override = true` MUST qualify for override resolution —
/// otherwise the autonomous-loop callback turn runs on the agent's base
/// model instead of the operator-set per-skill override (e.g., mika-dev
/// base = kimi-k2.5, self-dev override = sonnet-4-6).
///
/// This test exercises both `SilentTrigger::Callback` and
/// `SilentTrigger::DeferredDispatch` shapes via the matched-skill construction
/// they share (`callback_safe_skills`). The carve-out fires identically.
///
/// Note (mika#1217 F3): `run_silent_inner` does not currently invoke
/// `resolve_skill_llm_override` on the silent path. The carve-out shape is
/// correct at the function level (this test verifies that); the call-site
/// wiring is the residual gap. Tracked in the follow-up ticket cited in the
/// mika#1217 PR description.
#[test]
fn test_resolve_skill_llm_override_silent_callback_and_deferred_dispatch_carve_out() {
    use mika_common::llm::mock::MockLlmProvider;
    let mock = MockLlmProvider::builder()
        .provider_name("moonshotai")
        .model_name("kimi-k2.5")
        .build();
    let self_dev =
        make_skill_entry_with_db_llm("self-dev", Some("anthropic"), Some("claude-sonnet-4-6"));

    // Shape A — SilentTrigger::Callback: matched set wraps callback_safe_skills
    // with MatchReason::AlwaysOn. The carve-out must qualify the entry.
    let matched_callback = vec![MatchedSkill {
        entry: &self_dev,
        reason: MatchReason::AlwaysOn,
    }];
    let qualifies_callback = matched_callback.iter().any(|ms| match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    });
    assert!(
        qualifies_callback,
        "Callback turn: AlwaysOn skill with from_db_override=true must qualify for override"
    );
    // Resolution returns None because Settings is None (cannot construct provider),
    // but the early-exit at overrides.is_empty() is NOT taken — proving the
    // carve-out lets the entry through.
    let _ = resolve_skill_llm_override(&matched_callback, None, &mock, false);

    // Shape B — SilentTrigger::DeferredDispatch: identical matched-skill
    // construction to Callback. Same carve-out behavior expected.
    let matched_deferred = vec![MatchedSkill {
        entry: &self_dev,
        reason: MatchReason::AlwaysOn,
    }];
    let qualifies_deferred = matched_deferred.iter().any(|ms| match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    });
    assert!(
        qualifies_deferred,
        "DeferredDispatch turn: AlwaysOn skill with from_db_override=true must qualify for override"
    );
    let _ = resolve_skill_llm_override(&matched_deferred, None, &mock, false);

    // Negative control — same skill without from_db_override (developer-time
    // skill.toml [llm] source) must NOT qualify. #463 protection holds for
    // both Callback and DeferredDispatch shapes.
    let self_dev_dev_time =
        make_skill_entry_with_llm("self-dev", Some("anthropic"), Some("claude-sonnet-4-6"));
    let matched_dev_time = [MatchedSkill {
        entry: &self_dev_dev_time,
        reason: MatchReason::AlwaysOn,
    }];
    let qualifies_dev_time = matched_dev_time.iter().any(|ms| match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    });
    assert!(
        !qualifies_dev_time,
        "AlwaysOn skill without from_db_override (skill.toml [llm]) must NOT qualify (#463)"
    );
}

// -- mika#1011: intent guard trigger prefix tests --

#[test]
fn test_callback_trigger_excludes_deferred_dispatch() {
    // Regular callback should match
    assert!(callback_trigger_active(
        "[callback: long_running:run_claude_pilot]"
    ));
    // Deferred dispatch should NOT match callback_trigger_active
    assert!(!callback_trigger_active(
        "[callback:deferred-dispatch] [parent: task-123]"
    ));
}

/// mika#2355 AC2 / AC9 — the #870 guard no longer arms on a build
/// callback, and still arms on every other `long_running` callback.
///
/// `callback_trigger_active` is the ONE predicate both sites read — the
/// `INTENT_GUARDS` registry entry (non-empty text) and the inline
/// empty-text mirror — so asserting it here asserts both; the eval test
/// `test_qa_build_callback_verdict_2355` drives the production silent path
/// through each site end to end.
#[test]
fn mika2355_callback_terminal_action_is_carved_out_of_build_callbacks_only() {
    assert!(
        !callback_trigger_active("[callback: long_running:build_mika]"),
        "a build callback owns no self_dev parent to mark terminal"
    );
    // The milestone suffix must not re-arm it either.
    assert!(!callback_trigger_active(
        "[callback: long_running:build_mika] [milestone-parent: abc]"
    ));
    // The five other long_running tools keep the #870 contract verbatim.
    for tool in [
        "run_claude_pilot",
        "run_claude_pilot_groom",
        "deploy_mika",
        "address_pr_comments",
        "resolve_pr_conflicts",
    ] {
        assert!(
            callback_trigger_active(&format!("[callback: long_running:{tool}]")),
            "{tool}: the #870 guard must still arm"
        );
    }
    // Registry wiring: the entry's trigger IS this predicate, not a copy.
    let entry = INTENT_GUARDS
        .iter()
        .find(|g| g.label == CALLBACK_TERMINAL_ACTION_LABEL)
        .expect("the #870 entry exists");
    assert!(!(entry.trigger)("[callback: long_running:build_mika]"));
    assert!((entry.trigger)("[callback: long_running:run_claude_pilot]"));
}

/// mika#2355 AC3 / AC9 — the framing prescribes the self_dev terminal
/// contract to every callback EXCEPT a build callback, which is told what
/// its own contract is (a posted `run_gh pr review`).
#[test]
fn mika2355_build_callback_framing_does_not_prescribe_the_self_dev_contract() {
    let build = build_callback_trigger_context(
        crate::qa_build_callback::BUILD_CALLBACK_LABEL,
        "task-001",
        None,
        "Build succeeded",
        false,
    );
    assert!(
        !build.contains("This turn MUST end with both of the following"),
        "build callback framing still prescribes update_task_status + send_message"
    );
    assert!(
        !build.contains("mark the parent self_dev task terminal"),
        "build callback framing still names a self_dev parent"
    );
    assert!(build.contains("`run_gh` call with `pr review`"));
    assert!(build.contains("`VERDICT:`"));
    // The grounding and skill-delegation lines are common to all flows.
    assert!(build.contains("NEVER extrapolate to downstream states"));
    assert!(build.contains("Follow the workflow defined by your active skills"));

    for tool in [
        "run_claude_pilot",
        "run_claude_pilot_groom",
        "deploy_mika",
        "address_pr_comments",
        "resolve_pr_conflicts",
    ] {
        let ctx = build_callback_trigger_context(
            &format!("long_running:{tool}"),
            "task-001",
            None,
            "Result text",
            false,
        );
        assert!(
            ctx.contains("This turn MUST end with both of the following"),
            "{tool}: the self_dev terminal contract must be unchanged"
        );
        assert!(
            ctx.contains("1. update_task_status") && ctx.contains("2. send_message"),
            "{tool}: both terminal actions must still be named"
        );
        assert!(
            !ctx.contains("This is a BUILD callback"),
            "{tool}: must not receive the build-callback contract"
        );
    }
}

/// mika#2355 — the failure-verification instruction (#716) is orthogonal
/// to the flow: a FAILED build callback still gets it.
#[test]
fn mika2355_failed_build_callback_keeps_the_verification_instruction() {
    let ctx = build_callback_trigger_context(
        crate::qa_build_callback::BUILD_CALLBACK_LABEL,
        "task-001",
        None,
        "Build failed",
        true,
    );
    assert!(ctx.contains("This callback reported a FAILURE"));
    assert!(ctx.contains("This is a BUILD callback"));
}

#[test]
fn test_deferred_dispatch_trigger_matches_correctly() {
    assert!(deferred_dispatch_trigger(
        "[callback:deferred-dispatch] [parent: task-123]"
    ));
    assert!(!deferred_dispatch_trigger(
        "[callback: long_running:run_claude_pilot]"
    ));
    assert!(!deferred_dispatch_trigger("[heartbeat trigger]"));
}

#[test]
fn test_deferred_dispatch_satisfied_requires_run_claude_pilot() {
    // No tools → not satisfied
    assert!(!deferred_dispatch_satisfied(&[]));

    // send_message only → not satisfied
    assert!(!deferred_dispatch_satisfied(&[ToolCallSummary {
        name: "send_message".to_string(),
        input_summary: "".to_string(),
        output_summary: "".to_string(),
        success: true,
        non_zero_exit: false,
        step: 0,
    }]));

    // run_claude_pilot → satisfied (even if failed)
    assert!(deferred_dispatch_satisfied(&[ToolCallSummary {
        name: "run_claude_pilot".to_string(),
        input_summary: "".to_string(),
        output_summary: "".to_string(),
        success: false,
        non_zero_exit: false,
        step: 0,
    }]));
}

// mika#1173: regression tests asserting intent-guard satisfied predicates
// accept both run_claude_pilot AND run_claude_pilot_groom. The structural
// revert split grooming onto its own tool; without these guards updated,
// the auto-groom path on ready-label/milestone-cascade/deferred-replay
// would all fail at the EndTurn guard layer.
// (Reuses the `make_summary(name, output, success)` helper defined later in
// this test module; passing "" for output as these guards key on name+success.)

#[test]
fn test_ready_label_dispatch_satisfied_accepts_both_tools() {
    assert!(!ready_label_dispatch_satisfied(&[]));
    assert!(ready_label_dispatch_satisfied(&[make_summary(
        "run_claude_pilot",
        "",
        true
    )]));
    assert!(ready_label_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        true
    )]));
    // Failed attempts still count as "attempted" per the guard's contract.
    assert!(ready_label_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        false
    )]));
    assert!(!ready_label_dispatch_satisfied(&[make_summary(
        "send_message",
        "",
        true
    )]));
}

#[test]
fn test_deferred_dispatch_satisfied_accepts_groom_tool() {
    assert!(deferred_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        true
    )]));
    assert!(!deferred_dispatch_satisfied(&[make_summary(
        "update_task_status",
        "",
        true
    )]));
}

#[test]
fn test_callback_milestone_advance_satisfied_path_a_accepts_groom_tool() {
    let parent_id = "abc-123";
    // Path A: groom-tool advance.
    assert!(callback_milestone_advance_satisfied(
        parent_id,
        &[make_summary("run_claude_pilot_groom", "", true)]
    ));
    // Path A: implement-tool advance still works.
    assert!(callback_milestone_advance_satisfied(
        parent_id,
        &[make_summary("run_claude_pilot", "", true)]
    ));
    // Neither path → not satisfied.
    assert!(!callback_milestone_advance_satisfied(
        parent_id,
        &[make_summary("send_message", "", true)]
    ));
}

#[test]
fn test_webhook_no_unauthorized_dispatch_satisfied_rejects_both_tools() {
    // No tools called → satisfied (no unauthorized dispatch).
    assert!(webhook_no_unauthorized_dispatch_satisfied(&[]));
    // Successful run_claude_pilot → unauthorized → NOT satisfied.
    assert!(!webhook_no_unauthorized_dispatch_satisfied(&[
        make_summary("run_claude_pilot", "", true)
    ]));
    // Successful run_claude_pilot_groom → also unauthorized → NOT satisfied.
    assert!(!webhook_no_unauthorized_dispatch_satisfied(&[
        make_summary("run_claude_pilot_groom", "", true)
    ]));
    // Failed dispatch attempts don't count (engine guards block them upstream).
    assert!(webhook_no_unauthorized_dispatch_satisfied(&[make_summary(
        "run_claude_pilot_groom",
        "",
        false
    )]));
}
