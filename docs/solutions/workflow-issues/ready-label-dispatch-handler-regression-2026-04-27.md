---
module: self-dev
tags: [intent_guards, dispatch, prompt_engineering, silent_failure, structural_enforcement]
problem_type: silent_dispatch_failure
category: workflow-issues
date: 2026-04-27
---

# Ready-label dispatch silently failed past label removal — engine guard, not prose, was the fix

## Problem

mika#842 (`fix(self-dev+gateway): gate dispatch on ready label or direct prompt only`, merged 2026-04-27T17:48Z) introduced a positive-consent dispatch model — applying the `ready` label to a ticket should cause mika-dev to dispatch claude-pilot. The PR added a "Ready-Label Dispatch" handler section to `skills/bundled/self-dev/system_prompt.md` (mika#846 fix in the same file). The handler instructed the LLM to:

1. Call `run_gh issue edit --remove-label ready`
2. "Then route to Generic Workflow Step 1 (fetch issue body, create task, dispatch claude-pilot)"

In production every webhook from 2026-04-27T17:55Z onward executed Step 1 successfully and stopped. **No `run_claude_pilot` call. No PR. No operator notification.** The `ready` label disappeared from the ticket, and the loop went silent.

Server logs show the same pattern across 7+ trace IDs that day: agent runs 7–8 LLM steps, each step the `webhook_zero_tools` intent-precondition guard fires with `missing_names: ["gh_read", "run_claude_pilot"]`, and the agent EndTurns at step 7 or 8 with `stop_reason: EndTurn`. No `error!` log, no Telegram message — the failure was invisible to the operator past label-removal. Vincent retried four times before reporting the regression.

## Resolution

Three layers, ordered by load-bearing weight:

### Engine layer (load-bearing)

Add `webhook_ready_label_dispatch` as a new entry in `INTENT_GUARDS` in `crates/mika-agent/src/agent.rs`. Registry entry:

- **Trigger**: `msg.starts_with("[GitHub] Issue labeled ready on ")` — strictly more specific than the existing `webhook_zero_tools` trigger (`starts_with("[GitHub]")`). Place it **before** `webhook_zero_tools` in the registry array so it evaluates first.
- **Satisfied predicate**: `summaries.iter().any(|s| s.success && s.name == "run_claude_pilot")`. Mirrors the existing pattern of requiring tool success — a failed `run_claude_pilot` is a real problem the LLM must handle (e.g. via `send_message`), not silently EndTurn on.
- **Correction message**: explicit instruction to call `create_task` then `run_claude_pilot` with `prompt: "<repo>#<n>"` and `task_id: <UUID>`.

This is the architecturally consistent fix. The codebase had documented this exact pattern on 2026-04-21 — see `intent-precondition-registry-guard-generalization-2026-04-21.md`: *"adding new guards [is] a data-declaration task (one entry in `INTENT_GUARDS`) rather than duplicating the guard boilerplate each time."* The fix #842 needed was already prescribed; it just wasn't applied.

### Prompt layer (defense-in-depth)

Restructure the Ready-Label Dispatch handler in `skills/bundled/self-dev/system_prompt.md`:

- Header gains `(MANDATORY — do not skip, do not defer)` to mirror Generic Workflow Step 3 (which has the working imperative pattern).
- Open the section with a reference to the engine guard (`The engine enforces this sequence via the webhook_ready_label_dispatch intent-precondition guard`) — establishes structural reality, not just prose discipline.
- **Inline** the dispatch sequence: Step 2 (`gh_read` for issue body), Step 3 (`create_task`), Step 4 (`run_claude_pilot`). No more prose-route to "Generic Workflow Step 1" sitting ~200 lines away in a different section.
- Add the line-62-style imperative to Step 4: *"IMMEDIATELY after Step 3, call `run_claude_pilot`. No other tool calls are permitted between Step 3 and this call. Do not read additional files, do not analyze code, do not produce a plan…"*
- Closing GATE: *"If Step 1 succeeded but you have NOT called `run_claude_pilot` in this turn, you MUST call `gh_read` (Step 2), `create_task` (Step 3), and `run_claude_pilot` (Step 4) immediately — do not end the turn."*

Defense in depth: the engine guard catches the failure if it slips through; the prompt prevents the failure from happening in the first place.

### Operator-feedback layer

When the new guard fires, gets re-prompted, and the LLM still doesn't call `run_claude_pilot`, the EndTurn return path in `run_loop` now:

- Logs `error!` with structured fields: `trace_id`, `location` (parsed `<repo>#<n>` from the marker), `label`.
- Emits an operator-facing `send_message` via `tool_ctx.message_sender` (when present): *"Ready-label dispatch stalled on `<repo>#<n>`: the `ready` label was removed but run_claude_pilot was never called. Investigate trace_id `<id>` in /var/log/mika/server.log. To retry, re-add the `ready` label."*

This eliminates the silent-failure mode. Investigation went from "Vincent waits two hours, retries, gets frustrated" to "Vincent gets a Telegram ping with the trace_id within seconds."

## Diagnostic signals

Indicators that this regression is recurring (re-emergence detection):

- `grep -E '"intent_guard":"webhook_ready_label_dispatch"' /var/log/mika/server.log` produces matches → guard is firing → either prompt strengthening regressed or LLM behavior drifted.
- `grep -E 'ready_label_dispatch_stalled' /var/log/mika/server.log` produces matches → guard fired AND retry didn't recover → operator already notified, but trend warrants attention.
- `gh api /repos/<repo>/issues/events` shows `labeled:ready` followed by `unlabeled:ready` from `mika-platform-dev` with no subsequent `mention` from `claude-pilot[bot]` within 60 seconds → silent dispatch failure.

## Anti-pattern (avoid in future handlers)

**Prose-routing across sections.** A handler that does its own work then says "now route to <other section> step <N>" is structurally weak. Three reasons it fails:

1. The "IMMEDIATELY/MUST/NO OTHER TOOLS" imperatives in the destination section are positionally bound to that section's preceding steps. They do not transfer when the LLM "arrives" via prose-routing from a different section.
2. Under cognitive load (long prompts, large contexts), the LLM reads instructions sequentially within the active section and is more likely to EndTurn than to re-anchor on another section's discipline.
3. Required-tools enforcement (`[constraints] required_tools` in `skill.toml`) is silently inert for `MatchReason::AlwaysOn` skill activations (see `crates/mika-agent/src/skills/matcher.rs` match-reason conditioning rule, mika#265). Webhook turns never trigger keyword matches, so `required_tools` does not backstop anything for them.

**Correct pattern**: when a handler needs to drive a multi-step tool sequence, inline the steps, add the in-section imperative, and add an `INTENT_GUARDS` registry entry that structurally enforces the contract. Treat `required_tools` as documentation, not enforcement, for AlwaysOn skills.

## Lesson

mika#842's PR description listed two manual test plan items, both literally unchecked at merge time:

```markdown
- [ ] Manual: add `ready` label to a test issue → verify mika-dev removes label and dispatches
- [ ] Manual: post a comment containing `implement mika issue#N` → verify mika-dev does NOT dispatch
```

**An untested handler is an unverified handler.** The bug was not subtle — a single `ready` label apply on any open issue would have surfaced it instantly. Future PRs that touch dispatch hot paths must execute their own test plan before merge.

This compounds with `feedback_smoke_before_claiming_done.md` (auto-memory): "Build binary + run command + paste real output before claiming behavior; no 'should work' prose." The mika-arch first-pass and second-pass critiques on mika#841/#842 reviewed gateway routing, atomicity ordering, label taxonomy, and documentation — but did not flag the prose-route risk, did not require executing the manual test plan, and did not cite the on-file compound docs that warned about this exact pattern (`engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`, `prompt-enforcement-structural-guards.md`, `prompt-vs-tool-contract-mismatch-2026-04-24.md`).

Follow-up: extend mika-arch's critique checklist to detect prose-routed handlers in skill prompts and require the corresponding INTENT_GUARDS entry.

## Citations

### Compound docs (prior art on this exact class)

- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` — registry pattern; "adding new guards [is] a data-declaration task"
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — with-gradient vs against-gradient classifier; webhook → tool dispatch is against-gradient → engine layer
- `docs/solutions/architecture-patterns/webhook-zero-tools-guard-fabrication-prevention-2026-04-20.md` — original `webhook_zero_tools` design rationale (predecessor guard)
- `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md` — "Prompt admonitions are advisory, not enforceable"
- `docs/solutions/logic-errors/self-dev-task-not-found-silent-end-turn-2026-04-20.md` — same "silent EndTurn after partial completion" shape; established the GATE pattern
- `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md` — same "prose-route between sections fails" shape; routing must be explicit at every entry point
- `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — operator-notification pattern (the model used for the Unit 3 escalation)
- `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md` — direct principle: removing the `ready` label is an intent signal; calling `run_claude_pilot` is the completion signal
- `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` — design context for #841/#842 (the work this regression came out of)

### Auto-memory feedback citations

- `feedback_prompt_enforcement_fragile.md` — "Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints."
- `feedback_smoke_before_claiming_done.md` — "Build binary + run command + paste real output before claiming behavior; no 'should work' prose."
- `feedback_full_pipeline_always.md` — "Full /mika pipeline always, even for trivial fixes — CE review catches real bugs."
- `feedback_compound_infra_fixes.md` — "Infra fixes evaporate faster than product fixes; compound every non-trivial one, look back for prior related fixes before shipping a new one."

### Related issues / PRs

- mika#846 — this fix
- mika#842 — regression source (positive-consent dispatch gate)
- mika#841 — design context (the architectural decision #842 implemented)
- mika#844 — first ticket blocked by the regression (`ready` label applied 19:44 UTC, never dispatched)
- mika#702 — `INTENT_GUARDS` registry generalization (the pattern this fix instantiates)
- mika#696 — original `webhook_zero_tools` guard (the predecessor)
- mika#265 — match-reason conditioning rule (why `required_tools` was silently inert here)
