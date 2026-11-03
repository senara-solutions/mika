//! Les tours webhook : déclencheurs, gardes et dispatch autorisé.
//!
//! Dispatch sur label `ready` et sa localisation (#846), la garde zéro-outil
//! et son rétrécissement de préfixe (#1469), ce que `send_message` ne
//! satisfait plus (#1089), le stand-down du domaine fallthrough et l'outil
//! que ce tour ne reçoit pas (mika#2517), le refus du dispatch non autorisé
//! (#910) et l'invariant d'ordre du registre d'intentions.
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// -- ready_label_dispatch trigger tests (#846) --

#[test]
fn ready_label_trigger_matches_canonical_marker() {
    assert!(ready_label_dispatch_trigger(
        "[GitHub] Issue labeled ready on mika#999"
    ));
    assert!(ready_label_dispatch_trigger(
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 — title with — em-dashes"
    ));
    // Body containing additional text after the marker still matches.
    assert!(ready_label_dispatch_trigger(
        "[GitHub] Issue labeled ready on mika#42\n\nIssue body follows."
    ));
}

#[test]
fn ready_label_trigger_rejects_other_labels() {
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue labeled bug on mika#999"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue labeled p1-important on mika#999"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue labeled enhancement on mika#999"
    ));
}

#[test]
fn ready_label_trigger_rejects_other_event_types() {
    // Other GitHub events that share the `[GitHub]` prefix must not trigger.
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] PR comment on mika#999 by samidarko"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Issue comment on mika#999 by samidarko"
    ));
    assert!(!ready_label_dispatch_trigger(
        "[GitHub] Check suite failure on branch fix/foo"
    ));
}

#[test]
fn ready_label_trigger_rejects_direct_prompts() {
    // Direct `mika ask` prompts with no source-prefix must not trigger.
    assert!(!ready_label_dispatch_trigger("implement mika issue#999"));
    assert!(!ready_label_dispatch_trigger(
        "ready label on mika#999 please"
    ));
    assert!(!ready_label_dispatch_trigger(""));
}

// -- ready_label_dispatch_satisfied tests --

#[test]
fn ready_label_satisfied_when_run_claude_pilot_succeeded() {
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "create_task".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "run_claude_pilot".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_when_only_run_gh_succeeded() {
    // Reproduces the #846 regression: label removal succeeded but
    // run_claude_pilot was never called.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_gh".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_satisfied_when_run_claude_pilot_attempted_terminally() {
    // Attempt with terminal failure (e.g., global_dispatch_active when a
    // concurrent dispatch is active) still satisfies the guard. Forcing
    // a retry on terminal failures would produce misleading "never called"
    // operator notifications.  The LLM handles the failure via send_message
    // per the prompt's Step 4 (#846 adversarial review).
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_when_empty() {
    assert!(!ready_label_dispatch_satisfied(&[]));
}

// -- #1089: send_message no longer satisfies the guard --
// Post-#996, all legitimate completion paths call run_claude_pilot
// (dispatch via dev-pilot, auto-groom via dev-groom). The send_message-only
// path was removed after fabricated check_task pre-flights exploited the
// over-broad OR-shape to short-circuit dispatch via NoChannel escalation.

#[test]
fn ready_label_not_satisfied_when_only_send_message_called() {
    // #1089 — send_message alone no longer satisfies the guard.
    // Previously this was the grooming-rejection path (#907); post-#996
    // auto-groom replaced it with run_claude_pilot(dev-groom).
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "send_message".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_satisfied_when_both_dispatch_and_notification() {
    // Both run_claude_pilot and send_message present (e.g., dispatch
    // succeeded then operator was notified). Satisfied because
    // run_claude_pilot is present.
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_claude_pilot".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "send_message".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_when_only_send_message_failed() {
    // #1089 — send_message (even failed) does not satisfy the guard.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "send_message".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: false,
    }];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

#[test]
fn ready_label_not_satisfied_fabricated_check_task_then_send_message() {
    // #1089 — reproduces the exact fabrication pattern: agent calls
    // check_task (stale, fails), then send_message (fabricated escalation).
    // Guard must reject because run_claude_pilot was never called.
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "check_task".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: false,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "send_message".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(!ready_label_dispatch_satisfied(&summaries));
}

// -- parse_ready_label_location tests (#846 operator notification) --

#[test]
fn parse_location_from_simple_marker() {
    let out = parse_ready_label_location("[GitHub] Issue labeled ready on mika#999");
    assert_eq!(out, Some("mika#999".to_string()));
}

#[test]
fn parse_location_from_marker_with_em_dash_title() {
    let out = parse_ready_label_location(
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 \u{2014} title with \u{2014} more dashes",
    );
    assert_eq!(out, Some("senara-solutions/mika#1234".to_string()));
}

#[test]
fn parse_location_from_marker_with_newline_body() {
    let out = parse_ready_label_location(
        "[GitHub] Issue labeled ready on mika#42\n\nIssue body follows.",
    );
    assert_eq!(out, Some("mika#42".to_string()));
}

#[test]
fn parse_location_returns_none_for_other_markers() {
    assert!(parse_ready_label_location("[GitHub] Issue labeled bug on mika#999").is_none());
    assert!(parse_ready_label_location("implement mika issue#999").is_none());
    assert!(parse_ready_label_location("").is_none());
}

#[test]
fn parse_location_returns_none_for_empty_location() {
    // Marker present but no location text after it.
    assert!(parse_ready_label_location("[GitHub] Issue labeled ready on ").is_none());
    assert!(parse_ready_label_location("[GitHub] Issue labeled ready on \n").is_none());
}

#[test]
fn ready_label_guard_runs_before_webhook_zero_tools() {
    // Registry-order invariant: the more specific ready-label guard must
    // be evaluated before the generic webhook_zero_tools guard. Otherwise
    // a successful run_gh on the ready-label turn satisfies
    // webhook_zero_tools and the missing run_claude_pilot is never caught.
    let labels: Vec<&str> = INTENT_GUARDS.iter().map(|g| g.label).collect();
    let ready_idx = labels
        .iter()
        .position(|l| *l == "webhook_ready_label_dispatch")
        .expect("webhook_ready_label_dispatch must be registered");
    let zero_idx = labels
        .iter()
        .position(|l| *l == "webhook_zero_tools")
        .expect("webhook_zero_tools must be registered");
    assert!(
        ready_idx < zero_idx,
        "webhook_ready_label_dispatch (idx={ready_idx}) must precede \
             webhook_zero_tools (idx={zero_idx}) so the more specific trigger \
             fires first"
    );
}

// -- #1469 webhook_zero_tools_trigger prefix-narrowing tests --

#[test]
fn webhook_zero_tools_trigger_skips_check_suite_success() {
    assert!(!webhook_zero_tools_trigger(
        "[GitHub] Check suite success on senara-solutions/mika (branch: main)"
    ));
}

#[test]
fn webhook_zero_tools_trigger_skips_pr_closed() {
    assert!(!webhook_zero_tools_trigger(
        "[GitHub] PR closed: senara-solutions/mika#1000 — title (branch: foo)"
    ));
}

#[test]
fn webhook_zero_tools_trigger_skips_discussion() {
    assert!(!webhook_zero_tools_trigger(
        "[GitHub] discussion.created on senara-solutions/mika"
    ));
}

#[test]
fn webhook_zero_tools_trigger_fires_on_check_suite_failure() {
    assert!(webhook_zero_tools_trigger(
        "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)"
    ));
}

#[test]
fn webhook_zero_tools_trigger_fires_on_ready_label() {
    assert!(webhook_zero_tools_trigger(
        "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title"
    ));
}

#[test]
fn webhook_zero_tools_trigger_fires_on_pr_review() {
    assert!(webhook_zero_tools_trigger(
        "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer"
    ));
}

#[test]
fn webhook_zero_tools_trigger_skips_non_github() {
    assert!(!webhook_zero_tools_trigger("[Slack] message"));
    assert!(!webhook_zero_tools_trigger(""));
}

// -- mika#2517 U3 — the fallthrough domain no longer arms this guard --

/// **V4** — the five shapes of the Webhook Fallthrough domain stand the
/// guard down; the three shapes outside it still arm it.
///
/// **This inverts two assertions mika#1469 shipped.** `fires_on_new_comment`
/// and `fires_on_non_ready_label` asserted that a comment and a non-`ready`
/// label re-prompt a turn with zero tool calls. On exactly that population
/// the guard's premise — *"webhook events require action"* — contradicts the
/// `self-dev` HARD GATE, which says the correct action is to acknowledge and
/// stop. The old assertions were pinning the conflict, so they are replaced
/// here rather than carried: keeping them would mean the suite asserts both
/// halves of a contradiction.
///
/// The positive half is what makes this non-vacuous. A trigger that returned
/// `false` on everything would satisfy the negative half alone — and would
/// silently retire the mika#696 guard for the CI, PR-review and ready-label
/// paths, where it is still load-bearing.
#[test]
fn mika2517_the_fallthrough_domain_stands_the_zero_tools_guard_down() {
    for msg in [
        "[GitHub] Issue labeled bug on senara-solutions/mika#999",
        "[GitHub] New comment on senara-solutions/mika#933 (title) by @samidarko",
        "[GitHub] Issue assigned: senara-solutions/mika#100 — title",
        "[GitHub] Issue closed: senara-solutions/mika#100 — title",
        "[GitHub] discussion.created on senara-solutions/mika",
    ] {
        assert!(
            !webhook_zero_tools_trigger(msg),
            "{msg:?} is Webhook Fallthrough: its prompt contract is to \
                 acknowledge and stop, so re-prompting it for calling no tool is \
                 the engine contradicting itself (mika#2517 AC2)"
        );
    }
}

/// **V4, positive half** — outside the domain the guard is untouched.
#[test]
fn mika2517_the_zero_tools_guard_still_fires_outside_the_domain() {
    for msg in [
        // Ready-label: the dispatch path, where a turn with zero tools is a
        // dispatch that did not happen.
        "[GitHub] Issue labeled ready on senara-solutions/mika#933 — title",
        // qa skill territory.
        "[GitHub] PR review (approved) on senara-solutions/mika#1000 (title) by @reviewer",
        // ci skill territory — and note its *success* sibling is still
        // excluded by the mika#1469 literal, which is why that literal stays.
        "[GitHub] Check suite failure on senara-solutions/mika (branch: fix/foo)",
    ] {
        assert!(
            webhook_zero_tools_trigger(msg),
            "{msg:?} is outside the fallthrough domain — mika#2517 must not \
                 retire the mika#696 guard there"
        );
    }
}

// -- mika#2517 U2 — the tool the fallthrough turn is not handed --

/// **V3** — the domain widens the denylist; off the domain the slice is
/// handed through **identically**.
///
/// The `Cow::Borrowed` assertion is the load-bearing half: it is what makes
/// "no regression outside the domain" a property of the type rather than of
/// a comparison that could pass on a fresh allocation with the same content.
#[test]
fn mika2517_effective_disabled_tools_widens_only_on_the_domain() {
    let identity = vec!["run_team".to_string()];

    let off_domain = effective_disabled_tools(&identity, "implement mika#2517");
    assert!(
        matches!(off_domain, std::borrow::Cow::Borrowed(_)),
        "a turn outside the domain must hand the identity slice through \
             untouched — an owned clone here means the nominal path allocates"
    );
    assert_eq!(&*off_domain, identity.as_slice());

    let on_domain = effective_disabled_tools(
        &identity,
        "[GitHub] Issue labeled bug on senara-solutions/mika#999",
    );
    assert!(
        on_domain.iter().any(|t| t == "run_team"),
        "the identity denylist must survive the widening"
    );
    assert!(
        on_domain.iter().any(|t| t == "create_task"),
        "a fallthrough turn must not be handed create_task (mika#2517 AC1)"
    );
}

/// mika#2667 AC2 — the `hold[review]` turn handed back with no active task
/// withholds `create_task`; a `block[ac]` handed to the LLM does not.
#[test]
fn mika2667_a_hold_review_without_task_withholds_create_task() {
    let identity: Vec<String> = Vec::new();
    let held = format!(
        "{} on https://github.com/senara-solutions/mika/pull/12. …\n\n\
             [GitHub] PR review (commented) on senara-solutions/mika#12 (t) by @mika-platform-qa",
        crate::server::verdict_handler::HOLD_REVIEW_NO_TASK_MARKER
    );
    assert!(
        effective_disabled_tools(&identity, &held)
            .iter()
            .any(|t| t == "create_task")
    );

    // Negative control: the `block[ac]` passthrough and the raw PR review
    // event keep the nominal, untouched slice.
    for msg in [
        "[verdict_handler] VERDICT: block[ac] on https://github.com/senara-solutions/mika/pull/12 \
             but no active in_progress task found. Passing through to LLM.",
        "[GitHub] PR review (commented) on senara-solutions/mika#12 (t) by @mika-platform-qa",
    ] {
        assert!(
            matches!(
                effective_disabled_tools(&identity, msg),
                std::borrow::Cow::Borrowed(_)
            ),
            "{msg}"
        );
    }
}

/// The widening is idempotent: an identity that already denies the tool
/// does not end up denying it twice.
#[test]
fn mika2517_the_widening_does_not_duplicate_an_existing_entry() {
    let identity = vec!["Create_Task".to_string()];
    let widened = effective_disabled_tools(
        &identity,
        "[GitHub] New comment on senara-solutions/mika#933 by @samidarko",
    );
    assert_eq!(
        widened.len(),
        1,
        "the match is case-insensitive downstream, so re-adding the name \
             would be a duplicate with no effect and a misleading log line: {widened:?}"
    );
}

/// The withheld list and the filter agree on the name (mika#2517).
///
/// `apply_agent_tool_visibility` matches case-insensitively, so this pins
/// the end-to-end path rather than the constant alone: the tool the
/// constant names must be the tool the filter removes.
#[test]
fn mika2517_the_withheld_tool_is_actually_evicted_by_the_filter() {
    let mut defs = vec![
        mika_common::claude::ToolDefinition {
            name: "create_task".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        },
        mika_common::claude::ToolDefinition {
            name: "run_claude_pilot".to_string(),
            description: String::new(),
            input_schema: serde_json::json!({}),
        },
    ];
    let disabled = effective_disabled_tools(
        &[],
        "[GitHub] Issue labeled bug on senara-solutions/mika#999",
    );
    apply_agent_tool_visibility(&mut defs, &disabled, None, "mika-test");

    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        !names.contains(&"create_task"),
        "left in the array: {names:?}"
    );
    assert!(
        names.contains(&"run_claude_pilot"),
        "run_claude_pilot stays SERVED on purpose — its gate 0 refusal is the \
             measurement that the model tried, and hiding the tool would delete \
             that signal (mika#2517 § 3.3 (b)): {names:?}"
    );
}

// -- #910 webhook_no_unauthorized_dispatch trigger tests --

#[test]
fn no_unauthorized_dispatch_trigger_matches_comment_events() {
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] New comment on senara-solutions/mika#906 by @samidarko\nhttps://github.com/senara-solutions/mika/issues/906#issuecomment-123\n\nGroomed end-to-end."
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_matches_non_ready_labels() {
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled bug on mika#999"
    ));
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled p1-important on mika#999"
    ));
    assert!(webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled enhancement on mika#999"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_skips_pr_review() {
    // PR review events are qa skill territory — must NOT trigger (#1102).
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] PR review (approved) on senara-solutions/mika#694 by reviewer"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_skips_check_suite() {
    // Check suite events are ci skill territory — must NOT trigger (#1102).
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Check suite failure on branch fix/foo"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_ready_label() {
    // Ready-label events must NOT trigger this guard — the positive-case
    // guard (webhook_ready_label_dispatch) handles them.
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled ready on mika#999"
    ));
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[GitHub] Issue labeled ready on senara-solutions/mika#1234 \u{2014} title"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_direct_prompts() {
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "implement mika issue#999"
    ));
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "dispatch mika#906"
    ));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_empty() {
    assert!(!webhook_no_unauthorized_dispatch_trigger(""));
}

#[test]
fn no_unauthorized_dispatch_trigger_rejects_callback() {
    // Callback triggers have a different prefix.
    assert!(!webhook_no_unauthorized_dispatch_trigger(
        "[callback: run_claude_pilot]"
    ));
}

// -- #910 webhook_no_unauthorized_dispatch satisfied tests --

#[test]
fn no_unauthorized_dispatch_satisfied_when_empty() {
    assert!(webhook_no_unauthorized_dispatch_satisfied(&[]));
}

#[test]
fn no_unauthorized_dispatch_satisfied_when_only_run_gh() {
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_gh".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: true,
        non_zero_exit: false,
    }];
    assert!(webhook_no_unauthorized_dispatch_satisfied(&summaries));
}

#[test]
fn no_unauthorized_dispatch_satisfied_when_pilot_failed() {
    // Failed run_claude_pilot (e.g., task_not_dispatchable) is already
    // blocked by the dispatch-readiness guard — no double-rejection needed.
    let summaries = vec![ToolCallSummary {
        step: 0,
        name: "run_claude_pilot".to_string(),
        input_summary: String::new(),
        output_summary: String::new(),
        success: false,
        non_zero_exit: true,
    }];
    assert!(webhook_no_unauthorized_dispatch_satisfied(&summaries));
}

#[test]
fn no_unauthorized_dispatch_not_satisfied_when_pilot_succeeded() {
    // This is the unauthorized dispatch case — run_claude_pilot succeeded
    // on a non-ready webhook turn.
    let summaries = vec![
        ToolCallSummary {
            step: 0,
            name: "run_gh".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 1,
            name: "create_task".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
        ToolCallSummary {
            step: 2,
            name: "run_claude_pilot".to_string(),
            input_summary: String::new(),
            output_summary: String::new(),
            success: true,
            non_zero_exit: false,
        },
    ];
    assert!(!webhook_no_unauthorized_dispatch_satisfied(&summaries));
}

// -- #910 ordering invariant --

#[test]
fn no_unauthorized_dispatch_guard_ordering() {
    // webhook_no_unauthorized_dispatch must appear AFTER
    // webhook_ready_label_dispatch (which handles the positive case)
    // and BEFORE webhook_zero_tools (logical grouping of webhook guards).
    let labels: Vec<&str> = INTENT_GUARDS.iter().map(|g| g.label).collect();
    let ready_idx = labels
        .iter()
        .position(|l| *l == "webhook_ready_label_dispatch")
        .expect("webhook_ready_label_dispatch must be registered");
    let no_dispatch_idx = labels
        .iter()
        .position(|l| *l == "webhook_no_unauthorized_dispatch")
        .expect("webhook_no_unauthorized_dispatch must be registered");
    let zero_idx = labels
        .iter()
        .position(|l| *l == "webhook_zero_tools")
        .expect("webhook_zero_tools must be registered");
    assert!(
        ready_idx < no_dispatch_idx,
        "webhook_ready_label_dispatch (idx={ready_idx}) must precede \
             webhook_no_unauthorized_dispatch (idx={no_dispatch_idx})"
    );
    assert!(
        no_dispatch_idx < zero_idx,
        "webhook_no_unauthorized_dispatch (idx={no_dispatch_idx}) must precede \
             webhook_zero_tools (idx={zero_idx})"
    );
}

#[test]
fn intent_guards_registry_count() {
    // Six guards after adding deferred_dispatch_action (mika#1011).
    assert_eq!(
        INTENT_GUARDS.len(),
        6,
        "INTENT_GUARDS should have exactly 6 entries"
    );
}
