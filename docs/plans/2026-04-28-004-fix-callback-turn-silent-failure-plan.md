---
title: "fix: callback-turn-must-emit-terminal-action EndTurn guard"
type: fix
status: active
date: 2026-04-28
ticket: senara-solutions/mika#870
branch: fix/870/mika-dev-callback-turn-dies-silently
origin: 2026-04-28 mika#868 dev-run audit
related: senara-solutions/mika#871 (parent task leak — sibling), senara-solutions/mika#862/#863/#864 (open EndTurn-guard family)
---

# fix: callback-turn-must-emit-terminal-action EndTurn guard

## Overview

mika#870 is the highest-blast-radius red flag from tonight's mika#868 dev-run audit: when a `long_running:run_claude_pilot` callback subtask is marked `delivered` but produced no PR, mika-dev's callback session runs diagnostic tool calls (5 LLM calls + 5 tool calls in the observed instance) and exits with **zero rows in `messages` for `role='assistant'`**. The operator gets no notification, parent task stays `in_progress`, and the autonomous-loop signal is lost.

This plan adds an 8th post-condition guard to the agent loop's intent-precondition registry that requires callback turns to invoke at least one of `send_message`, `update_task_status`, or `create_task` before EndTurn lands. The guard is the structural contract; a complementary prompt-level nudge is added to `build_callback_trigger_context` as defense-in-depth, not as the load-bearing layer.

## Problem Frame

### Observed failure

Callback session `callback-90672365-d67d-40b6-9a5a-4a37fbb16235` (mika-dev, 2026-04-28T19:59:34 → 20:02:43Z, kimi-k2.5):

- 5 LLM calls (159K input / 1K output, 187s latency, no errors, stop_reasons: ToolUse, EndTurn)
- 5 tool calls: `check_task` (parent → in_progress), `check_task` (callback subtask → tool error: "not a manual task"), `gh pr list --state open` (`[]`), `gh pr list --state all` (no PR for branch), `gh issue view 868` (OPEN)
- 0 rows in `messages` for `role='assistant'`
- No `send_message`, `update_task_status`, or `create_task` calls

The agent diagnosed the failure (no PR, issue still open, parent in_progress) and then ended without acting on the diagnosis. The 187s of latency was real LLM work — this isn't a crash; it's the loop terminating after EndTurn with no observable side effect.

### Root cause (located in agent.rs)

Three layers contribute, none of them sufficient on its own:

1. **Silent-mode loop exit without persistence** — `crates/mika-agent/src/agent.rs:1385-1393`. When the LLM returns empty text and `mode.follow_up_on_empty()` returns `false` (Silent mode does), the loop exits with `text: None`. The `messages` write (lines 1363-1374) is gated on `text.is_some()`, so a terminal turn with only tool calls and no text persists nothing. The DB telemetry `tool_calls` rows still land — that's how the audit found the diagnostic calls — but the conversation history surface shows nothing.

2. **No callback-specific post-condition guard.** Seven guards exist at `agent.rs:955-1333` (text-based / prose-style tool-call detection, required-tools, completion-claim, fabricated-action-claim, intent-precondition registry, persistence). None require an assistant text *or* a terminal-action tool call on callback turns. The intent-precondition registry at `agent.rs:3989-4041` has three entries (`webhook_ready_label_dispatch`, `webhook_zero_tools`, `resume_reconcile`) — adjacent in shape but none cover callback delivery.

3. **Callback prompt framing is a hint, not a contract.** `build_callback_trigger_context` (`agent.rs:74-91`) appends "If no skill-specific workflow applies, use send_message…" after `format_callback_framing`. That trailing instruction is soft — kimi-k2.5 in particular rationalizes around it (per `feedback_sonnet_over_kimi_for_grounding.md`). Per `feedback_prompt_enforcement_fragile.md`, prompt-level constraints are not load-bearing.

### Why this is a p1

Every claude-pilot timeout, push failure, or mid-pipeline crash will fail this same way. Tonight's instance was caught by `/mika-audit dev-run` because Vincent ran the audit — without that ritual, the failure is invisible. The platform-level next step in `mika-platform/docs/logs/2026-04-28 - Mika Validation Run (Groom + Dispatch).md` (groom + dispatch the monitoring dashboard milestone) is gated on this fix because milestone dispatches multiply the ghost-failure cost: any one ticket's silent failure costs N audit cycles to surface.

## Requirements Trace

- **R1.** New `IntentPrecondition` entry `callback_terminal_action` registered in the `INTENT_GUARDS` array at `crates/mika-agent/src/agent.rs:3989`. **Trigger predicate:** active `SilentTrigger` is `Callback` (introspected from the synthetic user-message header set by `build_callback_trigger_context`). **Satisfied predicate:** tool-call summaries contain BOTH `update_task_status` AND `send_message` (`create_task` optional, NOT required — see F1 audit in § "Architect first-pass concerns" below). **On violation:** reject EndTurn, inject a corrective system message, re-enter the loop. Re-fire prevented by the existing `intent_guard_retries: HashSet<&'static str>` at `agent.rs:803` — guard fires once per loop, then becomes dormant. Pattern matches the existing three guards: `webhook_zero_tools` at `agent.rs:4018-4027` (correction message text-shape verified as the canonical template), `webhook_ready_label_dispatch` at `agent.rs:4005-4015`, `resume_reconcile` at `agent.rs:4030+`.
- **R2.** Prompt-level nudge in `build_callback_trigger_context` (`agent.rs:74-91`) appending the same AND-shape constraint as a single appended paragraph. Defense-in-depth only — not load-bearing per `feedback_prompt_enforcement_fragile.md`.
- **R3.** Integration test file `crates/mika-agent/tests/eval/callback_terminal_action.rs`, modelled on the existing `crates/mika-agent/tests/eval/test_callback_turn.rs` scaffold (which already constructs `SilentTrigger::Callback` framing — verified via `grep "SilentTrigger::Callback" tests/eval/`, line 31). Three cases: **(1) happy path** — turn 1 emits both `update_task_status` + `send_message`, loop exits cleanly; **(2) recovery** — turn 1 emits only `check_task` + `gh_pr_list` (no terminal calls), guard fires once with corrective message, turn 2 emits the missing terminal calls, loop exits; **(3) persistent failure** — turn 1 emits zero terminal calls, guard fires once, turn 2 still emits zero, guard now dormant via `intent_guard_retries`, loop continues until max-tool-steps cap halts (regression sentinel for cap interaction). Use `EvalHarness::builder()` + `MockLlmProvider` per `mika/CLAUDE.md` testing conventions.
- **R4.** No new DB columns or schema migrations. The required information (active `SilentTrigger`, tool-call summaries) is already available to `IntentPrecondition::satisfied`'s existing closure signature.

## Proposed Fix

### Primary: engine guard

**Where:** `crates/mika-agent/src/agent.rs:3989` — append a new `IntentPrecondition` entry to the existing `INTENT_GUARDS: &[IntentPrecondition]` array. Implementation shape (modelled directly on `webhook_zero_tools` at agent.rs:4017-4027):

```rust
// #870 — callback turns must update parent task AND notify operator before EndTurn.
// Issue body Expected Behavior prescribes BOTH update_task_status AND send_message
// (create_task is "Optionally relaunches claude-pilot…"). F1 callback-site audit
// (see plan § "Architect first-pass concerns") confirmed the long_running:run_claude_pilot
// flow is the only callback site; no skill-side deferrers exist.
IntentPrecondition {
    label: "callback_terminal_action",
    trigger: callback_trigger_active,
    satisfied: callback_terminal_action_satisfied,
    correction_message: "[Your response was rejected because this callback turn ended \
         without the required terminal actions. Callback turns MUST: \
         (1) call `update_task_status` to mark the parent self_dev task terminal \
             (`failed`/`pending`/`completed` based on the callback result), AND \
         (2) call `send_message` to notify the operator of the result. \
         Optionally call `create_task` to relaunch claude-pilot if the failure mode \
         is retry-safe. EndTurn without (1) AND (2) will be rejected. \
         Re-read the callback framing and produce both terminal actions before EndTurn.]",
}
```

The trigger and satisfied predicates follow the existing function signature (`fn(&str) -> bool` for trigger, `fn(&[ToolCallSummary]) -> bool` for satisfied). The trigger detects the synthetic user-message header set by `build_callback_trigger_context` (`agent.rs:74-91`) — exact header substring TBD during implementation; the existing `webhook_zero_tools` trigger uses `msg.starts_with("[GitHub]")` as its template.

**On violation:** the existing loop machinery at `agent.rs:803` (`intent_guard_retries: HashSet<&'static str>`) ensures the guard fires once, injects the corrective message via the existing rejection path, and re-enters. If the agent still skips the terminal actions, the guard is dormant and the loop continues until the max-tool-steps cap halts. That is strictly better than today's silent exit because the cap halt is logged and observable in audit telemetry.

**Naming alignment:** open siblings (#862 asserted-unavailability EndTurn guard, #863 quoted-resource pre-fetch guard, #864 required-suffix-line EndTurn guard) follow `<concern>-<verb>-<EndTurn|guard>`. Per `feedback_no_premature_extraction` and review-guide.md §3 YAGNI, this plan does NOT introduce a shared helper module across the four guards — extraction waits until the second EndTurn-family guard lands and the actual shared shape is concrete (see § "Out of Scope" follow-up sentinel).

### Secondary: prompt nudge

**Where:** `crates/mika-agent/src/agent.rs:74-91` — `build_callback_trigger_context`. After the existing trailing line ("If no skill-specific workflow applies, use send_message…"), append a single paragraph that mirrors the AND-shape contract:

```
This turn MUST end with both of the following before EndTurn:
1. update_task_status — mark the parent self_dev task terminal (failed/pending/completed) based on the callback result
2. send_message — notify the operator of the result

Optionally also call create_task to relaunch claude-pilot if the failure mode is retry-safe.

EndTurn without both (1) and (2) will be rejected by the engine and you will be re-prompted.
```

The phrasing matches the guard's `correction_message` so prompt-internalization and correction-internalization converge on the same outcome. Per `feedback_prompt_enforcement_fragile.md`, this layer is NOT load-bearing — the engine guard is the contract.

### Tests

**File:** `crates/mika-agent/tests/eval/callback_terminal_action.rs` (new, follows the pattern in existing eval tests).

Three test cases:

1. **Happy path.** `MockLlmProvider` returns `[send_message, EndTurn]` in turn 1. Assert: loop exits cleanly after turn 1; one row in `messages` for `role='assistant'`; one `send_message` row in `tool_calls`.
2. **Silent-failure recovery.** `MockLlmProvider` returns `[check_task, gh_pr_list, EndTurn(empty text)]` in turn 1, then `[send_message, EndTurn]` in turn 2. Assert: turn 1's EndTurn is rejected; corrective system message is injected; turn 2 produces `send_message`; loop exits after turn 2.
3. **Persistent failure.** `MockLlmProvider` returns `[check_task, EndTurn(empty)]` for 20 consecutive turns. Assert: loop halts at the max-tool-steps cap with a recorded `callback_terminal_action` guard violation; status is recorded as a failure rather than a clean exit.

Use `EvalHarness::builder()` with `.silent_mode(true)` and a synthesized `SilentTrigger::Callback` payload. No real LLM provider, no network.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Add `callback_trigger_active` and `callback_terminal_action_satisfied` predicate fns; append a new `IntentPrecondition` entry to `INTENT_GUARDS` at line ~3989; append terminal-action constraint paragraph to `build_callback_trigger_context` at line ~88-89. |
| `crates/mika-agent/tests/eval/callback_terminal_action.rs` | New file — three test cases (happy / recovery / persistent failure), modelled on `tests/eval/test_callback_turn.rs`. |
| `crates/mika-agent/tests/eval/mod.rs` | Register the new test module (follow existing pattern from `test_callback_turn`). |
| `CHANGELOG.md` | Add entry under "Fixed" — "Callback turns now require both `update_task_status` AND `send_message` before EndTurn; missing terminal actions are rejected and re-prompted via the intent-precondition registry (#870)." |

No schema changes. No new dependencies. No new env vars.

## Verification

### Unit / integration

```bash
cd /data/workspace/mika-platform/.claude/worktrees/fix-870-mika-dev-callback-turn-dies-silently/mika
cargo test -p mika-agent --test eval callback_terminal_action
cargo test -p mika-agent  # full suite must pass
cargo clippy -- -D warnings
cargo fmt --check
```

### Synthetic dev-run reproduction

After merge:

1. Dispatch a contrived task that will cause claude-pilot to exit without a PR (e.g., `mika ask mika-dev "implement issue#XXXX"` where XXXX is a deliberately unreachable issue, or kill claude-pilot mid-`/ce:review`).
2. Wait for the callback session to fire.
3. Query `~/.mika/data/mika.db`:
   ```sql
   SELECT COUNT(*) FROM messages
     WHERE session_id = '<callback_session_id>' AND role = 'assistant';  -- expect ≥ 1
   SELECT tool_name, COUNT(*) FROM tool_calls
     WHERE session_id = '<callback_session_id>'
       AND tool_name IN ('send_message','update_task_status','create_task')
     GROUP BY tool_name;  -- expect ≥ 1 row
   ```
4. Run `/mika-audit dev-run <task_id>`. The audit's "Red flags" section MUST NOT include "callback session emitted no assistant message" or "no terminal action called."

The bug is fixed iff both (3) and (4) succeed for at least one synthetic failure-mode case.

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Guard fires on legitimate happy-path callbacks (e.g., a callback turn that just acknowledges and goes back to sleep). | F1 callback-site audit (see § "Architect first-pass concerns") confirmed only one callback flow exists today (long_running:run_claude_pilot via `task_engine/dispatcher.rs:320`). No skill-side deferrers. If a future skill introduces a legitimate non-terminal callback, surface as a follow-up; the guard fires once then becomes dormant via `intent_guard_retries`, so the worst case is one corrective re-prompt per loop, not a permanent block. |
| max-tool-steps cap is reached before the agent produces a terminal action (persistent-failure test case). | Better failure mode than today's silent exit — the cap halt is logged and observable via `/mika-audit dev-run`. Per F2 verification, the registry's re-enter-once pattern guarantees the loop progresses rather than spinning. |
| Prompt nudge wording drifts from the guard's `correction_message` wording over time. | Both strings live in `agent.rs` within ~50 lines of each other; reviewer flags drift in plan-on-branch review. Not extracted to a `const` unnecessarily — tying them with shared identifiers is the cheaper signal until a third site needs the same string. |
| Conflict with #862/#863/#864 once those engine guards land. | This guard is an additive entry to the existing `INTENT_GUARDS` array; the four guards don't share state. F3 follow-up sentinel covers the eventual shared-helper extraction (see § "Out of Scope"). |

## Out of Scope

- **mika#871 (parent task leak engine reaper).** Even after this guard lands, the parent task lifecycle needs an independent safety net for the case where the callback turn itself crashes before the guard can fire. That's #871's option B — separate ticket, separate plan.
- **Audit-finding #4 (claude-pilot-py SDK init bug).** Vincent's call on whether to file. Independent code path.
- **Shared helper extraction across #862/#863/#864/#870.** Premature per review-guide.md §3 YAGNI. **Follow-up sentinel** to file when the second EndTurn-family guard from this set lands and the actual shared shape is concrete: "Revisit `IntentPrecondition` helper extraction (correction_message templating, registry registration boilerplate, retry-tracking integration) once two of {#862, #863, #864, #870} have shipped."

## Architect first-pass concerns (resolved in this revision)

This revision applies the four findings from mika-arch's first-pass review (session `280e7d2c-4b5a-4651-83a2-e99555d80727`).

### F1 — Required-action set: AND-shape, not any-one-of (BLOCKING, resolved)

The first-pass brief proposed `{send_message, update_task_status, create_task}` any-one-of. The issue body's Expected Behavior section specifies AND-shape: "Updates the parent self_dev task to `failed` (or `pending` for safe retry). Sends a user-facing notification via `send_message`. Optionally relaunches claude-pilot if the failure mode is retry-safe." mika-arch flagged this as a contract divergence between plan and issue body that cannot ship without resolution.

**Callback-site audit** (per F1's prescribed grep):

```bash
grep -rn "SilentTrigger::Callback\|build_callback_trigger_context" crates/mika-agent/src/ skills/bundled/
```

Per-site disposition:

- `crates/mika-agent/src/task_engine/dispatcher.rs:294, 320` — `dispatch_resume_agent` constructs `SilentTrigger::Callback` for the long_running task callback flow. Single production producer of callback turns.
- `crates/mika-agent/src/skills/mod.rs:637` — comment-only reference to `callback_safe_skills()` whitelist. No callback creation here.
- `crates/mika-agent/src/agent.rs:74-91` — `build_callback_trigger_context` (consumer of `SilentTrigger::Callback`).
- `crates/mika-agent/src/agent.rs:2536, 2634, 2640, 2694, 2741, 2767, 2807, 2812, 2861` — agent-loop dispatch handling for callback mode (consumer-side).
- `crates/mika-agent/src/agent.rs:5552, 5566, 5581, 5594, 5607, 5627, 5638, 5648, 5656, 5666, 5681, 5698, 5716` — unit tests in the agent module (test-only; do not establish production semantics).
- `skills/bundled/` — **zero hits**. No skill-side handler defers terminal action to a sibling task.

**Conclusion:** option (a) — conform to issue body. Guard requires BOTH `update_task_status` AND `send_message`. `create_task` (relaunch) optional. Plan's R1, R2, Proposed Fix § Primary, § Secondary now reflect AND-shape. Issue body did not need update; plan now matches body's pre-existing prescription.

### F2 — Recovery pattern: re-enter-once, verified (BLOCKING, resolved)

Read `crates/mika-agent/src/agent.rs:4017-4027` (`webhook_zero_tools`) as the canonical template. Pattern is:

- Guard's `correction_message` injected as a synthetic user message via the existing rejection path.
- Loop re-enters at iteration top.
- Re-fire prevented by `intent_guard_retries: HashSet<&'static str>` at `agent.rs:803` — guard label inserted on first fire, presence prevents re-fire on subsequent iterations.

Pattern is **re-enter-once**. After the first re-prompt, the guard is dormant; if the agent still skips the terminal actions, the loop continues and eventually halts at `max_steps` (`agent.rs:828`, `mode.max_steps()`). Plan's R3 test 3 (persistent failure) is a regression sentinel for this cap interaction — kept.

### F3 — Ship alone (sharpening, applied)

Plan's lean (ship #870 standalone, refactor toward shared helper when the second guard from {#862, #863, #864} lands) is correct per review-guide.md §3 YAGNI. Documented in § "Out of Scope" as a follow-up sentinel.

### F5 — Eval scaffold fidelity (sharpening, applied)

`grep "SilentTrigger::Callback\|AgentMode::Silent" crates/mika-agent/tests/eval/` returns one hit:

- `crates/mika-agent/tests/eval/test_callback_turn.rs:31` — "D3 assertion 1: SilentTrigger::Callback framing fires" — this file is the canonical scaffold template for callback-mode integration tests via `EvalHarness` + `MockLlmProvider`. R3 now cites it explicitly. No new Phase 0 work needed.

### F6 — Per-site audit format (sharpening, applied)

The F1 audit above uses the per-site disposition format (path:line — what + verification status). Same shape as mika#788 / mika#845 / mika-platform#56 / mika-platform#58 PR descriptions. No follow-up needed.

---

## Architect verdict

- **First-pass (mika-arch session `280e7d2c-4b5a-4651-83a2-e99555d80727`):** ITERATE. Two blockers (F1, F2), three sharpenings (F3, F5, F6). All resolved in this revision. F4 (Test 3 conditional on F2) collapses to "keep" because F2 resolved to re-enter pattern.
- **Second-pass:** pending.
