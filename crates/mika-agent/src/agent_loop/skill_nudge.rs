//! Nudge-driven skill creation (mika#1583).
//!
//! A soft, advisory prompt injection at turn-end that suggests the agent author
//! or refine a skill when a task pattern looks worth extracting. The nudge is
//! gated by a per-agent iteration counter and an identity flag (default off),
//! and it presupposes a usable authoring path (`allow_authoring = true` plus the
//! `skill_manage` tool actually presented to the LLM this turn).
//!
//! State lives on `AgentState` (the same home/lifetime as `skills_dirty`) and is
//! threaded into the agent loop by reference on `AgentParams` — the identical
//! pattern used by `skills_dirty` and `pr_reviews_posted`. Silent/team turns do
//! not nudge; only conversation mode (`run_agent`) participates in Phase 1.
//!
//! Provenance: inspired by Hermes Agent's `_iters_since_skill` counter (default
//! 10). Mika's adaptation is an advisory inline block in the *next* turn's system
//! prompt (not a background daemon), identity-gated default-off (multi-tenant
//! safety), with staged-then-promote authoring as the load-bearing safety
//! differentiator (sub-issue 1's `lifecycle_state`).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Name of the authoring tool the nudge presupposes. Kept in sync with the
/// registration in `tools/mod.rs`; the `skill_manage_tool_const_matches_registry`
/// unit test asserts membership in `crate::tools::BUILTIN_TOOL_NAMES` so a
/// rename in either file fails the build instead of silently no-op-ing every
/// future nudge (mika#1583 post-review finding #13).
const SKILL_MANAGE_TOOL: &str = "skill_manage";

/// Cross-turn nudge counter for a single agent (mika#1583). Lives on
/// `AgentState`, threaded into the agent loop by reference (same pattern as
/// `skills_dirty`). Both fields use `Relaxed` ordering — the only invariant is
/// per-agent monotonic counting, not cross-field synchronization.
#[derive(Debug, Default)]
pub struct SkillNudgeState {
    /// Tool-invoking turns since the last nudge fired. Reset to 0 when a nudge
    /// fires.
    pub iters_since_skill_nudge: AtomicU32,
    /// Set at turn-end when the threshold is crossed; consumed (cleared) at the
    /// next turn's prompt assembly.
    pub pending_skill_nudge: AtomicBool,
}

/// Per-turn nudge context threaded into `run_loop` from `run_agent` when nudges
/// may apply (conversation mode with a server-provided `SkillNudgeState`).
/// `None` in CLI / silent / team modes. The `enabled`/`interval`/`authoring_enabled`
/// fields are resolved snapshots of `identity.skills.*` for this turn.
pub(crate) struct SkillNudgeContext<'a> {
    pub state: &'a SkillNudgeState,
    /// `identity.skills.nudge_is_enabled()`.
    pub enabled: bool,
    /// `identity.skills.resolved_nudge_interval()` — validated `> 0` at identity load.
    pub interval: u32,
    /// `identity.skills.authoring_enabled()`.
    pub authoring_enabled: bool,
}

/// Pure decision helper — the single place the fire condition is expressed, so
/// the turn-end check and the unit tests both call it (keeps AC6/7/8 testing off
/// the full loop). `interval` is validated `> 0` at identity load, so no
/// zero-guard is needed here.
pub(crate) fn should_fire_nudge(
    enabled: bool,
    authoring_usable: bool,
    interval: u32,
    iters: u32,
) -> bool {
    enabled && authoring_usable && iters >= interval
}

/// Turn-end mutation on the shared `SkillNudgeState` (mika#1583 AC3).
///
/// Counts only useful (tool-invoking) turns — mirrors Hermes's iteration
/// semantics and the nudge block's "roughly {interval} tool-invoking turns"
/// language. When the fire condition holds, sets `pending_skill_nudge` and
/// resets the counter to 0. The nudge presupposes a usable authoring path:
/// `allow_authoring = true` AND `skill_manage` actually presented to the LLM this
/// turn (AC8 resolution — sub-issue 1 gates `skill_manage` at execution, not
/// visibility, so keying on `allow_authoring` plus tool presence is the
/// semantically correct presupposition).
pub(crate) fn apply_turn_end(
    ctx: &SkillNudgeContext<'_>,
    tool_use_occurred: bool,
    enabled_tool_names: &HashSet<String>,
) {
    // mika#1583 post-review defense-in-depth (F2 — reviewer #2/#6): a warm
    // agent with `nudge_enabled = false` must NOT accumulate iters across
    // turns. Without this early return, a Phase 1 operator flipping
    // `nudge_enabled` from false → true on a long-running agent (e.g. mika
    // Prime with N days uptime) would fire the nudge immediately AND the
    // rendered block would claim "Approximately N tool-invoking turns have
    // passed" for the wrong N — the counter is a live cross-turn quantity,
    // not a per-session one. Short-circuit here so the counter only runs
    // while the feature is enabled; when disabled the counter stays at
    // its previous value, so re-enabling starts a fresh interval.
    if !ctx.enabled {
        return;
    }
    if tool_use_occurred {
        ctx.state
            .iters_since_skill_nudge
            .fetch_add(1, Ordering::Relaxed);
    }
    let authoring_usable = ctx.authoring_enabled && enabled_tool_names.contains(SKILL_MANAGE_TOOL);
    let iters = ctx.state.iters_since_skill_nudge.load(Ordering::Relaxed);
    if should_fire_nudge(ctx.enabled, authoring_usable, ctx.interval, iters) {
        ctx.state.pending_skill_nudge.store(true, Ordering::Relaxed);
        ctx.state
            .iters_since_skill_nudge
            .store(0, Ordering::Relaxed);
    }
}

/// Consume a pending nudge at prompt-assembly time (mika#1583 AC4).
///
/// If `pending_skill_nudge` is set, append the advisory block to `system` and
/// clear the flag atomically (`swap(false, ..)` reads-and-clears). Returns
/// whether a nudge was injected. The caller gates this on
/// `nudge_is_enabled()` (belt-and-suspenders — pending is only ever set while
/// enabled, but an operator could flip the flag off between turns).
pub(crate) fn inject_pending_nudge(
    system: &mut String,
    state: &SkillNudgeState,
    interval: u32,
) -> bool {
    if state.pending_skill_nudge.swap(false, Ordering::Relaxed) {
        system.push('\n');
        system.push_str(&render_nudge_block(interval));
        system.push('\n');
        true
    } else {
        false
    }
}

/// Render the advisory `<skill-nudge>` block injected into the next turn's system
/// prompt. Semantics-preserving reword of the plan verbatim (mika#1583 post-review
/// finding #6): "You have completed roughly {N} turns" → "Approximately {N}
/// turns have passed". The plan/issue-body verbatim contained "completed",
/// which is one of the tokens in the #483 completion-claim guard's regex
/// (`\b(merged|deployed|completed?|shipped)\b` at `agent_loop/mod.rs:5557`).
/// Priming the LLM with "completed" in the system prompt biases the assistant
/// output to use the same word, tripping the guard's false-positive path even
/// though the nudge is not making a completion claim. The reword removes the
/// guard-vocabulary word without changing the advisory semantics — the block
/// still communicates "N turns have elapsed since your last skills review."
/// If `render_nudge_block` is ever consumed by a caller that treats it as a
/// claim (which it is not today), reintroducing "completed" needs a paired
/// guard update.
pub(crate) fn render_nudge_block(interval: u32) -> String {
    format!(
        "<skill-nudge priority=\"advisory\">\n\
Approximately {interval} tool-invoking turns have passed since the last\n\
skills review. If a recent task pattern is worth extracting into a reusable\n\
skill, consider calling `skill_manage(action=\"create\" | \"update\" | \"inspect\")`\n\
this turn. The skill will land `staged` and require operator promotion before\n\
it activates — your authoring is advisory, not load-bearing. If no pattern\n\
stands out, ignore this nudge and proceed normally.\n\
</skill-nudge>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_set_with_skill_manage() -> HashSet<String> {
        HashSet::from(["skill_manage".to_string(), "send_message".to_string()])
    }

    fn enabled_set_without_skill_manage() -> HashSet<String> {
        HashSet::from(["send_message".to_string()])
    }

    // AC6 — nudge does NOT fire when disabled, for every counter value.
    #[test]
    fn ac6_disabled_never_fires() {
        for iters in [0, 1, 9, 10, 11, 1_000, u32::MAX] {
            assert!(
                !should_fire_nudge(false, true, 10, iters),
                "disabled must never fire (iters={iters})"
            );
        }
    }

    // AC7 (decision) — fires when enabled + authoring usable + iters >= interval.
    #[test]
    fn ac7_fires_when_conditions_met() {
        assert!(should_fire_nudge(true, true, 10, 10));
        assert!(should_fire_nudge(true, true, 10, 11));
        assert!(!should_fire_nudge(true, true, 10, 9));
    }

    // AC7 (mutation) — turn-end sets pending and resets the counter to 0.
    #[test]
    fn ac7_turn_end_sets_pending_and_resets() {
        let state = SkillNudgeState::default();
        state.iters_since_skill_nudge.store(9, Ordering::Relaxed);
        let ctx = SkillNudgeContext {
            state: &state,
            enabled: true,
            interval: 10,
            authoring_enabled: true,
        };
        // 9 -> 10 (tool_use_occurred), which crosses the threshold.
        apply_turn_end(&ctx, true, &enabled_set_with_skill_manage());
        assert!(state.pending_skill_nudge.load(Ordering::Relaxed));
        assert_eq!(state.iters_since_skill_nudge.load(Ordering::Relaxed), 0);
    }

    // AC2 — counter increments only on tool-invoking turns; no-tool turn is a no-op.
    #[test]
    fn ac2_counts_only_tool_invoking_turns() {
        let state = SkillNudgeState::default();
        let ctx = SkillNudgeContext {
            state: &state,
            enabled: true,
            interval: 10,
            authoring_enabled: true,
        };
        apply_turn_end(&ctx, false, &enabled_set_with_skill_manage());
        assert_eq!(state.iters_since_skill_nudge.load(Ordering::Relaxed), 0);
        apply_turn_end(&ctx, true, &enabled_set_with_skill_manage());
        assert_eq!(state.iters_since_skill_nudge.load(Ordering::Relaxed), 1);
    }

    // AC8 — no fire when authoring path unusable, even enabled + iters >= interval.
    #[test]
    fn ac8_authoring_not_usable_never_fires() {
        // allow_authoring = false
        assert!(!should_fire_nudge(true, false, 10, 100));

        // skill_manage absent from the enabled set (allow_authoring = true).
        let state = SkillNudgeState::default();
        state.iters_since_skill_nudge.store(50, Ordering::Relaxed);
        let ctx = SkillNudgeContext {
            state: &state,
            enabled: true,
            interval: 10,
            authoring_enabled: true,
        };
        apply_turn_end(&ctx, true, &enabled_set_without_skill_manage());
        assert!(
            !state.pending_skill_nudge.load(Ordering::Relaxed),
            "skill_manage absent must not fire"
        );
    }

    // AC4 — render block contains the required advisory framing.
    #[test]
    fn ac4_render_block_contains_framing() {
        let block = render_nudge_block(10);
        assert!(block.contains("priority=\"advisory\""));
        assert!(block.contains("skill_manage"));
        assert!(block.contains("staged"));
        assert!(block.contains("operator promotion"));
        assert!(block.contains("Approximately 10 tool-invoking turns"));
    }

    // mika#1583 post-review finding #6 — anti-regression: the nudge block MUST
    // NOT contain any token in the #483 completion-claim guard's regex
    // vocabulary (`\b(merged|deployed|completed?|shipped)\b`). Priming the
    // system prompt with those tokens biases the assistant output and trips
    // the guard's false-positive path. If a future edit reintroduces any of
    // these words, this test fails and forces a paired guard update.
    #[test]
    fn nudge_block_avoids_completion_claim_vocabulary() {
        let block = render_nudge_block(10);
        for token in ["merged", "deployed", "completed", "complete", "shipped"] {
            assert!(
                !block.contains(token),
                "nudge block contains completion-claim guard token `{token}` — \
                 rerender_nudge_block reintroduced #483 guard-vocabulary. \
                 Reword to a synonym or pair with a guard change."
            );
        }
    }

    // mika#1583 post-review finding #13 — anti-regression: the local
    // `SKILL_MANAGE_TOOL` const MUST stay in sync with the canonical
    // `crate::tools::BUILTIN_TOOL_NAMES` roster. Renaming `skill_manage` in
    // `tools/mod.rs` without updating this const would silently no-op every
    // future nudge (the authoring_usable predicate would return false for a
    // now-nonexistent name). This test fails at build time on any rename.
    #[test]
    fn skill_manage_tool_const_matches_registry() {
        assert!(
            crate::tools::BUILTIN_TOOL_NAMES.contains(&SKILL_MANAGE_TOOL),
            "SKILL_MANAGE_TOOL ('{SKILL_MANAGE_TOOL}') is missing from \
             crate::tools::BUILTIN_TOOL_NAMES. Either the const drifted from \
             the registered name, or the tool was renamed in tools/mod.rs \
             without a paired update here (the nudge would silently no-op)."
        );
    }

    // AC4 — inject appends when pending and clears the flag after.
    #[test]
    fn ac4_inject_appends_and_clears() {
        let state = SkillNudgeState::default();
        state.pending_skill_nudge.store(true, Ordering::Relaxed);
        let mut system = String::from("SYSTEM PROMPT");
        let injected = inject_pending_nudge(&mut system, &state, 10);
        assert!(injected);
        assert!(system.contains("<skill-nudge"));
        assert!(!state.pending_skill_nudge.load(Ordering::Relaxed));

        // Second call: nothing pending, no append, returns false.
        let len_before = system.len();
        let injected2 = inject_pending_nudge(&mut system, &state, 10);
        assert!(!injected2);
        assert_eq!(system.len(), len_before);
    }
}
