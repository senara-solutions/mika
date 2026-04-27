---
title: "fix: ready-label dispatch — engine guard + prompt structural compulsion"
type: fix
status: active
date: 2026-04-27
---

# fix: ready-label dispatch — engine guard + prompt structural compulsion (mika#846)

## Overview

mika#842 introduced the Ready-Label Dispatch handler in `skills/bundled/self-dev/system_prompt.md` (lines 238–262). The handler removes the `ready` label correctly via `run_gh`, then prose-routes ("Then route to Generic Workflow Step 1") to dispatch via `run_claude_pilot`. Step 1 fires; Step 2 doesn't. Every `ready` label since 2026-04-27T17:55Z has produced the same dead pattern: `webhook_zero_tools` guard fires once, the LLM continues with text, and `EndTurn` returns silently with no dispatch and no operator notification.

This plan fixes forward — does NOT revert #842 — and addresses the bug at the **engine layer** (the load-bearing change) plus prompt and operator-feedback hardening.

## Problem Frame

`mika-dev`'s autonomous dispatch loop has been cold for 2+ hours. The operator (Vincent) labels a ticket `ready`; the bot removes the label; nothing happens. The failure is invisible past label-removal because:

1. **Prose-routed handler step**: line 259's `Then route to Generic Workflow Step 1` is positionally distant from the Generic Workflow's `IMMEDIATELY after Step 2, call run_claude_pilot. No other tool calls are permitted` imperative at line 62. The compulsion does not transfer across ~200 lines of prompt under cognitive load (architecture-pattern: `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`).
2. **`required_tools` is inert**: `[GitHub] Issue labeled ready on <repo>#<n>` does not match any of self-dev's keyword triggers (`implement`, `add feature`, `resume`, etc.). Self-dev activates as `MatchReason::AlwaysOn`, and the match-reason conditioning rule (`crates/mika-agent/src/skills/matcher.rs:205`) silently disables `required_tools = ["run_claude_pilot"]` enforcement for that turn.
3. **`webhook_zero_tools` guard is too coarse**: its `satisfied` predicate is `summaries.iter().any(|s| s.success)`. The successful `run_gh` (label removal) satisfies the guard immediately; the missing `run_claude_pilot` is never enforced.
4. **Silent EndTurn**: when guards are exhausted, `LoopResult` returns normally with no operator notification (`crates/mika-agent/src/agent.rs:1129–1149`). There is no "guard fired then exhausted" telemetry path.

The root cause is architectural: a multi-step autonomous handler was protected only by prompt prose, in a turn where `required_tools` enforcement is silently inert and the existing guard is too coarse to detect partial completion.

## Requirements Trace

- **R1** — On `[GitHub] Issue labeled ready on <repo>#<n>`, mika-dev MUST call `run_claude_pilot` with `prompt: "<repo>#<n>"`. Verifiable in `server.log` and via `~/.mika/data/mika.db` `tasks` table.
- **R2** — On other label adds (`bug`, `enhancement`, etc.), mika-dev MUST NOT dispatch. Webhook Fallthrough scope rule still applies.
- **R3** — On comment events containing literal text `implement mika issue#N`, mika-dev MUST NOT dispatch. Confirms #842's positive-consent gate didn't regress.
- **R4** — When the ready-label handler stalls mid-sequence, the operator MUST be notified (no silent failure past label removal).
- **R5** — All three R1–R3 manual tests pass before merge — the unchecked checkboxes from #842 must be checked here.

## Scope Boundaries

- Fix forward — do **NOT** revert mika#842. Its content-matching dispatch model replacement is correct; only the handler's structural compulsion is broken.
- Single PR. Engine + prompt + tests + solution doc + smoke verification.
- Defense in depth: engine guard is the primary fix; prompt strengthening is secondary; operator notification is tertiary.

### Deferred to Separate Tasks

- **Prompt size reduction** — `self-dev/system_prompt.md` is 55,632 / 57,344 bytes (96.9% of cap). Hardening salience by trimming the prompt is a separate refactor (file as follow-up). This plan adds ~25 lines for the structural compulsion fix, which means the new file size will be ~56,200 bytes. Acceptable headroom for now; aggressive trim deferred.
- **CI gate threshold tightening** (#828 follow-up) — the `bundled_skills_load.rs` test only catches load-failure, not 75%-of-cap warnings as a hard fail. File as follow-up.
- **Match-reason rule re-evaluation** — whether `required_tools` should apply to AlwaysOn skills on webhook turns is a broader question. The new INTENT_GUARDS entry sidesteps it for this specific handler.
- **Architect process improvement** — neither mika-arch first-pass nor second-pass on #841/#842 flagged the prose-route risk despite three on-file compound docs warning of this exact class. File as follow-up to add prose-route detection to the architect critique checklist.

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/self-dev/system_prompt.md` lines 60–94 (Generic Workflow Step 3 with the working `IMMEDIATELY` imperative — the load-bearing structural pattern to mirror)
- `skills/bundled/self-dev/system_prompt.md` lines 238–262 (the regressed Ready-Label Dispatch handler — to strengthen)
- `skills/bundled/self-dev/system_prompt.md` lines 461–597 (Milestone Workflow `MANDATORY` + `GATE` pattern — also worth mirroring)
- `crates/mika-agent/src/agent.rs:3525–3551` — `INTENT_GUARDS` const with `webhook_zero_tools` (mika#696) and `resume_reconcile` (mika#702). New entry goes here.
- `crates/mika-agent/src/agent.rs:1033–1074` — guard dispatch loop (re-prompt path)
- `crates/mika-agent/src/agent.rs:1129–1149` — EndTurn return path. Operator notification gap is here.
- `crates/mika-gateway/src/github.rs:159` — `("issues", Some("labeled"))` routing (the source of the marker)
- `crates/mika-gateway/src/github.rs:208–218` — `format_event_text` labeled branch (emits the `[GitHub] Issue labeled <name> on <repo>#<n>` marker)
- `skills/bundled/claude-pilot/tools.json` — `run_claude_pilot` schema; required inputs are `prompt: "<repo>#<n>"` and `task_id: <UUID>`

### Institutional Learnings (cite in solution doc)

- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` — INTENT_GUARDS registry pattern; this is the architecturally consistent fix shape ("adding new guards a data-declaration task")
- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — with-gradient vs against-gradient classifier; `EndTurn after multi-step handler` is against-gradient → engine layer
- `docs/solutions/architecture-patterns/webhook-zero-tools-guard-fabrication-prevention-2026-04-20.md` — original webhook_zero_tools design rationale, including why `any tool succeeded` is the right satisfied predicate for THAT class of failure (it isn't right for THIS one)
- `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md` — "Prompt admonitions are advisory, not enforceable"
- `docs/solutions/logic-errors/self-dev-task-not-found-silent-end-turn-2026-04-20.md` — silent EndTurn after tool error; the GATE pattern with explicit "do NOT end the turn until X or escalation fires"
- `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md` — same shape: prose-route between sections fails; routing must be explicit at every entry point
- `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — operator-notification continuation-turn pattern
- `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md` — direct principle: removing the `ready` label is an intent signal; calling `run_claude_pilot` is the completion signal

### Auto-memory references (cite in PR body, solution doc)

- `feedback_prompt_enforcement_fragile.md` — "Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints."
- `feedback_smoke_before_claiming_done.md` — "Build binary + run command + paste real output before claiming behavior; no 'should work' prose"
- `feedback_full_pipeline_always.md` — "Full /mika pipeline always, even for trivial fixes — CE review catches real bugs"
- `feedback_compound_infra_fixes.md` — "Infra fixes evaporate faster than product fixes; compound every non-trivial one, look back for prior related fixes before shipping a new one"

## Key Technical Decisions

- **D1 — Engine guard is the primary fix.** Add a third `INTENT_GUARDS` entry (`webhook_ready_label_dispatch`). Trigger: message starts with `[GitHub] Issue labeled ready on `. Satisfied predicate: `run_claude_pilot` was called in this turn. This makes the dispatch contract structural. *Rationale:* the codebase's own architectural compass (registry pattern + `engine-guards-vs-prompt-rules`) prescribes engine-layer enforcement for against-gradient invariants like "after webhook X, tool Y must be called."

- **D2 — Prompt strengthening is defense-in-depth, not the load-bearing fix.** Restructure the Ready-Label Dispatch handler to inline the dispatch sequence (no more prose-route to Generic Workflow Step 1) and add the `MANDATORY — do NOT skip` header + `IMMEDIATELY after Step 1, call run_claude_pilot. No other tool calls permitted between Step 1 and run_claude_pilot.` imperative — mirroring the line 62 pattern that already works in Generic Workflow. *Rationale:* belt and suspenders; the engine guard catches the failure, the prompt prevents the failure from happening in the first place.

- **D3 — Operator notification on `webhook_ready_label_dispatch` exhaustion.** When the new guard fires, gets re-prompted, and the LLM still doesn't call `run_claude_pilot` on the second attempt, emit a `send_message` to the operator: `Ready-label dispatch stalled on <repo>#<n>: label was removed but run_claude_pilot was not called. Investigate /var/log/mika/server.log for trace_id <id>. Re-add the ready label to retry.` *Rationale:* the worst part of the regression was that it was silent for 2+ hours. Eliminate that.

- **D4 — Do NOT change match-reason conditioning behavior in this PR.** The fact that `required_tools` is inert for AlwaysOn skills on webhook turns is a known engine behavior. Changing it has wider blast radius than this fix needs. The new INTENT_GUARDS entry handles the specific case; broader question goes to a follow-up ticket.

- **D5 — Use `repo:` and `issue_number:` parsing in the guard, not regex on display string.** The marker text is `[GitHub] Issue labeled ready on <repo>#<n> — <title>` (where `<title>` may contain arbitrary text including dashes). Parse robustly: split on `[GitHub] Issue labeled ready on `, take the rest up to the first ` — ` or end-of-line, split on `#`. Edge case: title containing `[GitHub]` or `Issue labeled` literals — guarded by anchoring on `starts_with`.

- **D6 — Tests use `EvalHarness` with `MockLlmProvider`.** The eval harness already drives the agent loop deterministically (`crates/mika-agent/CLAUDE.md` § Testing). New tests sit alongside existing intent-precondition tests. Real-provider eval matrix tests deferred — the structural guard's correctness is verifiable without real LLM calls.

## Open Questions

### Resolved During Planning

- **Q: Should `required_tools` enforcement be expanded to AlwaysOn skills on webhook turns?** A: No — out of scope. The new INTENT_GUARDS entry handles this case. Broader change deferred (D4).
- **Q: Should we trim self-dev/system_prompt.md in this PR?** A: No — current change adds ~25 lines, leaves headroom. Aggressive trim is a separate refactor (Deferred).
- **Q: Should the new guard's correction message reference the `task_id` UUID?** A: No — the guard fires before `create_task` happens (or fires after `create_task` but before `run_claude_pilot`). Correction message should tell the LLM what to do next ("you removed the label; you MUST now call `run_claude_pilot` with `prompt: <repo>#<n>` and `task_id: <UUID from create_task>`").
- **Q: How does the new guard interact with `webhook_zero_tools`?** A: They co-exist. `webhook_zero_tools` fires when ZERO tools succeeded; `webhook_ready_label_dispatch` fires when SOME tool succeeded but NOT `run_claude_pilot`. The triggers are different (`starts_with("[GitHub]")` vs `starts_with("[GitHub] Issue labeled ready on ")`); both can apply to the same turn but `webhook_ready_label_dispatch` is the more specific match. Order in the registry matters for re-prompt budget; place `webhook_ready_label_dispatch` BEFORE `webhook_zero_tools`.

### Deferred to Implementation

- Exact wording of the correction message — finalize during /ce:work after seeing the existing message strings nearby in `agent.rs`.
- Whether `send_message` for D3 emits to chat_id from the webhook payload or to a config-defined operator chat_id — defer to implementation; check existing operator notifications in `silent-callback-max-steps-exhaustion.md` precedent path.
- Whether `webhook_ready_label_dispatch` should be allowed two retries instead of the standard one — defer; default to one retry consistent with existing guards, escalate to operator on second failure (D3).

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```rust
// Pattern, not implementation. New entry in INTENT_GUARDS const.
IntentPrecondition {
    label: "webhook_ready_label_dispatch",
    trigger: |msg| msg.starts_with("[GitHub] Issue labeled ready on "),
    // satisfied iff run_claude_pilot was actually called this turn (success OR error — the contract
    // is "tool was attempted with the right intent", not "the dispatch eventually succeeded")
    satisfied: |summaries| summaries.iter().any(|s| s.tool_name == "run_claude_pilot"),
    correction_message: "[Your response was rejected. The ready label was removed but \
         you did NOT call run_claude_pilot. The Ready-Label Dispatch handler requires you \
         to call run_claude_pilot with prompt: \"<repo>#<n>\" and task_id: <UUID from \
         create_task>. Call create_task then run_claude_pilot now. Do not end this turn \
         until run_claude_pilot has been called.]",
}
```

Operator escalation hook (D3) attaches to guard exhaustion: when `intent_guard_retries` already contains `webhook_ready_label_dispatch` and the LLM EndTurns again without calling `run_claude_pilot`, emit a `send_message` before returning `LoopResult`.

Prompt-side (defense in depth) — Ready-Label Dispatch handler header becomes:

```markdown
### Ready-Label Dispatch (MANDATORY — do not skip, do not defer)

When the message starts with `[GitHub] Issue labeled ready on <repo>#<n>`, the operator has set the `ready` label. Execute these steps in order. **The engine enforces this sequence via the `webhook_ready_label_dispatch` intent-precondition guard — failing to call `run_claude_pilot` will reject your response.**

1. **First**, call `run_gh("issue edit <n> --remove-label ready")`. [...failure path unchanged...]

2. **Second**, call `gh_read` (or use the issue body already in scope) to read the issue title and body — needed for `create_task`.

3. **Third**, call `create_task` with the issue reference and parsed body. Capture the returned `task_id`.

4. **IMMEDIATELY after Step 3**, call `run_claude_pilot` with `prompt: "<repo>#<n>"` and `task_id: <UUID from Step 3>`. **No other tool calls are permitted between Step 3 and this call.** Do not read files, do not analyze code, do not produce a plan, do not summarize findings, do not list "next steps." Call `run_claude_pilot` NOW.

**GATE: If you completed Step 1 (label removed) but did not call `run_claude_pilot`, you MUST call `create_task` then `run_claude_pilot` immediately.**
```

## Implementation Units

- [ ] **Unit 1: Add `webhook_ready_label_dispatch` to `INTENT_GUARDS` registry**

**Goal:** Add a third entry to `INTENT_GUARDS` that triggers on the ready-label marker and is satisfied only when `run_claude_pilot` is called. Make the dispatch contract structural.

**Requirements:** R1, R2 (the guard's trigger condition is specific to ready-label markers, so non-ready label events are unaffected)

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (`INTENT_GUARDS` const ~line 3525–3551, plus correction message)
- Test: `crates/mika-agent/src/agent.rs` (`#[cfg(test)] mod tests` — co-located with existing intent-precondition tests; OR new file under `tests/eval/` if eval-style is more natural for this case)

**Approach:**
- New entry uses `tool_name == "run_claude_pilot"` predicate (any attempt, success or failure)
- Order: place `webhook_ready_label_dispatch` BEFORE `webhook_zero_tools` so the more specific trigger fires first when both apply
- Single retry budget (standard, matches `webhook_zero_tools`)
- Correction message names the missing step explicitly and lists the required tool inputs (`prompt`, `task_id`)
- Re-prompt path is unchanged — `agent.rs:1033–1074` already handles the dispatch loop

**Patterns to follow:**
- `webhook_zero_tools` entry at `agent.rs:3525–3537` (closest analogue)
- `resume_reconcile` entry at `agent.rs:3538–3549` (similar specificity — fires on a more specific message pattern)

**Test scenarios:**
- *Happy path*: webhook turn with `[GitHub] Issue labeled ready on mika#999` marker, LLM mock that calls `run_gh` → `gh_read` → `create_task` → `run_claude_pilot` → guard satisfied, turn completes normally
- *Edge case (ready-label, partial completion)*: same marker, LLM mock that calls `run_gh` then EndTurns with text only → first turn rejected with correction message, second LLM mock turn calls `run_claude_pilot` → guard satisfied
- *Edge case (ready-label, two-pass failure)*: same marker, LLM mock that EndTurns without `run_claude_pilot` on both turns → guard exhausted, operator notification fires (Unit 3 cross-test), `LoopResult` returns
- *Negative case (other label)*: `[GitHub] Issue labeled bug on mika#999` marker → guard does NOT trigger (verifies trigger is specific to `ready`)
- *Negative case (mika ask direct prompt)*: `implement mika issue#999` (no `[GitHub]` prefix) → guard does NOT trigger
- *Edge case (label name parsing)*: `[GitHub] Issue labeled ready on senara-solutions/mika#999 — title with — dashes — in it` → trigger fires (anchored on `starts_with`)
- *Integration*: alongside `webhook_zero_tools`, both can apply to the same turn; verify `webhook_ready_label_dispatch` runs first and the more-specific trigger correctly registers in `intent_guard_retries`

**Verification:**
- `cargo test -p mika-agent` passes including the new tests
- `cargo clippy` clean
- Existing `webhook_zero_tools` and `resume_reconcile` test cases still pass (no regression to existing guards)

---

- [ ] **Unit 2: Strengthen Ready-Label Dispatch handler with structural compulsion**

**Goal:** Defense in depth — restructure the prompt handler to inline the full dispatch sequence with `MANDATORY` framing, `IMMEDIATELY` imperative, and `GATE` pattern. Engine guard catches violations; prompt prevents them.

**Requirements:** R1, R2, R3

**Dependencies:** None (prompt change is independent of Unit 1, but lands in same PR)

**Files:**
- Modify: `skills/bundled/self-dev/system_prompt.md` (lines 238–262)

**Approach:**
- Header changes from `### Ready-Label Dispatch` to `### Ready-Label Dispatch (MANDATORY — do not skip, do not defer)` to mirror line 60
- Section opens with reference to the engine guard (`engine enforces this sequence via webhook_ready_label_dispatch intent-precondition guard`) — establishes structural reality, not just prose discipline
- Step 2 ("Then route to Generic Workflow Step 1") is replaced with explicit inline steps 2/3/4: `gh_read` → `create_task` → `run_claude_pilot`. No more prose-route to a different section
- Step 4 carries the `IMMEDIATELY... no other tool calls permitted... NOW` imperative from line 62
- Closing `GATE` line catches LLM mid-handler ("If you completed Step 1 but did not call `run_claude_pilot`, you MUST...")
- Net change: ~25 lines added → ~80 lines removed/modified (net +5 to +10 lines after consolidation)

**Patterns to follow:**
- Line 60 (`Step 3 — Launch claude-pilot (MANDATORY — do not skip, do not defer)`) — header style
- Line 62 (`IMMEDIATELY after Step 2, call run_claude_pilot. No other tool calls are permitted...`) — imperative style
- Lines 461, 489, 507 (Milestone Workflow GATE pattern) — closing gate

**Test scenarios:**
- Test expectation: prompt-only changes; verified via the `webhook_ready_label_dispatch` guard tests in Unit 1 (the guard tests use real prompt content via the eval harness). No standalone test needed.
- Confirm prompt size after edits is still under cap: `wc -c skills/bundled/self-dev/system_prompt.md` < 57344 bytes

**Verification:**
- `cargo test -p mika-agent --test bundled_skills_load` passes (skill still loads)
- Smoke test (Unit 4) confirms the prompt change drives the desired behavior on a real LLM

---

- [ ] **Unit 3: Operator notification on `webhook_ready_label_dispatch` exhaustion**

**Goal:** Eliminate the silent failure mode. When the new guard fires, re-prompts, and the LLM still doesn't call `run_claude_pilot`, emit `send_message` to the operator before returning `LoopResult`.

**Requirements:** R4

**Dependencies:** Unit 1 (uses the new guard label)

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (EndTurn return path ~line 1129–1149, OR a more central guard-exhaustion hook depending on what the implementer finds is cleanest)
- Test: same file (extend Unit 1's two-pass-failure test scenario)

**Approach:**
- After all retry budgets exhausted, before returning `LoopResult { text: ..., max_steps_exceeded: false }`:
  - Check whether `webhook_ready_label_dispatch` is in `intent_guard_retries` AND `run_claude_pilot` is NOT in `summaries`
  - If so, emit `send_message` (or equivalent) to the operator with: `Ready-label dispatch stalled on <repo>#<n>: label was removed but run_claude_pilot was not called. Investigate trace_id <id>. Re-add the ready label to retry.`
  - Log `error!` with structured fields for telemetry
- Look at `silent-callback-max-steps-exhaustion.md` precedent for the canonical operator-notification continuation pattern
- The implementer should decide whether the notification path lives in:
  - (a) the EndTurn return site (simple, narrow), or
  - (b) a generalized guard-exhaustion hook in the dispatch loop (broader applicability, more refactor)
  - Default to (a) for minimum blast radius; flag (b) as follow-up if appropriate

**Execution note:** This is the kind of edge-case behavior that's easy to "look right" but hard to verify without hitting it. **Test-first this unit.** Write the failing test (LLM mock that EndTurns twice without `run_claude_pilot` → expect `send_message` to fire) before implementing the notification path.

**Patterns to follow:**
- `crates/mika-agent/src/agent.rs:1232–1245` — `max_steps_exceeded` warn path (similar shape — log + return)
- `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md` — operator-notification continuation pattern

**Test scenarios:**
- *Happy path (no notification needed)*: ready-label turn completes successfully on first attempt → no operator notification fires
- *Single-retry recovery*: ready-label turn fails first attempt, recovers on re-prompt → no operator notification fires
- *Exhausted (notification fires)*: ready-label turn fails both attempts → `send_message` fires with correctly parsed `<repo>#<n>` and trace_id. Verify message text matches expected format.
- *Other guards exhausted*: `webhook_zero_tools` exhausts on a non-ready-label webhook → no ready-label-specific notification (notification is bound to `webhook_ready_label_dispatch` guard, not all guard exhaustions)
- *Edge case (parsing)*: marker with edge-case title (containing `[GitHub]` literal in title, or empty title) → notification still extracts `<repo>#<n>` correctly (or skips the parse-derived field gracefully)

**Verification:**
- New tests pass
- Manually invoking the notification path in a test harness produces a well-formed message
- Existing tests for `LoopResult` return paths unchanged

---

- [ ] **Unit 4: Smoke test on a real webhook (the unchecked #842 checkboxes)**

**Goal:** Execute all three manual test cases from #842's PR test plan that shipped unchecked. This is the acceptance test for R5.

**Requirements:** R1, R2, R3, R5

**Dependencies:** Units 1, 2, 3 must be implemented; build + deploy completed locally

**Files:**
- Test artifact (not committed): `docs/plans/2026-04-27-010-fix-846-ready-label-dispatch-regression-plan.md` (this plan, updated with Verification results)

**Approach:**
- Build: `cargo build --release --features telemetry`
- Deploy: `make deploy` (rebuilds + restarts mika-server)
- Wait for mika-server restart and skill registry reload (look for `INFO Skills registered` in server.log)
- Open a throwaway test issue on `senara-solutions/mika`, e.g. `test: ready-label dispatch verification`. Apply `ready` label.
- Watch `/var/log/mika/gateway.log` for `GitHub event forwarded to agent container target_agent=mika-dev`
- Watch `/var/log/mika/server.log` for `INFO run_claude_pilot invocation` with non-empty `task_id`
- Verify `/var/log/claude-pilot/<task_id>.log` exists and is being written
- Verify `~/.mika/data/mika.db` `tasks` table has a new row (`SELECT id, status, prompt FROM tasks ORDER BY created_at DESC LIMIT 1`)
- Close the test issue. Remove the test branch (the dispatched run will create a no-op PR, also delete that)
- Repeat with a non-`ready` label (e.g. `bug`) on a different test issue → confirm webhook routed but `run_claude_pilot` NOT called, no new task row
- Post a comment on a third test issue containing literal text `implement mika issue#999` → confirm no dispatch, no new task row
- All three checkboxes from #842's plan can now be checked

**Execution note:** Test the failure mode too. Manually patch the prompt to remove the `IMMEDIATELY` imperative (Unit 2's compulsion), rebuild, apply ready label, observe that **the engine guard alone catches the violation** and either re-prompts to recovery OR fires the operator notification (Unit 3). Then revert the prompt patch. This proves Unit 1 is load-bearing as designed.

**Test scenarios:**
- See Approach (the smoke test cases ARE the test scenarios)

**Verification:**
- All three #842 PR test plan checkboxes checked
- No `webhook_zero_tools` warnings in server.log for ready-label turns
- No `webhook_ready_label_dispatch` warnings on the happy path (only on the deliberate failure-mode test)

---

- [ ] **Unit 5: Solution doc — pattern citation**

**Goal:** Document the pattern: "prose-routed handler steps in self-dev fail under cognitive load; the architectural fix is an INTENT_GUARDS entry, not more prose." Future architect reviews check for this.

**Requirements:** R5 (acceptance for institutional learning)

**Dependencies:** Units 1, 2 (the fix shape is documented)

**Files:**
- Create: `docs/solutions/workflow-issues/ready-label-dispatch-handler-regression-2026-04-27.md`

**Approach:**
- Frontmatter: `module: self-dev`, `tags: [intent_guards, dispatch, prompt_engineering]`, `problem_type: silent_dispatch_failure`, `category: workflow-issues`
- Sections: Problem (the regression chain), Resolution (engine guard + prompt strengthening + operator notification), Diagnostic signals, Anti-pattern, Lesson, Citations
- Citations to existing solution docs (see Context & Research above) and to the auto-memory feedback files
- Cross-reference: link from this solution doc to mika#846, mika#842, mika#841, and the related compound docs
- Add to `docs/solutions/INDEX.md` if such an index exists (check during /ce:work)

**Patterns to follow:**
- `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` (the #841 doc — same area, similar shape)
- `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md` (the registry pattern doc)

**Test scenarios:**
- Test expectation: none — pure documentation. Verify links resolve (every relative path in the doc points to an existing file).

**Verification:**
- Doc is well-formed YAML frontmatter + Markdown
- All cross-reference paths resolve (`grep -E 'docs/solutions/.*\.md' new_doc.md` followed by `test -f` on each)
- Adds to the institutional knowledge that prevents the next iteration of this bug

## System-Wide Impact

- **Interaction graph:** New guard interacts with existing `webhook_zero_tools` (more specific trigger, runs first). Operator notification path (Unit 3) shares the `send_message` infrastructure with `silent-callback-max-steps-exhaustion` and other operator-notification consumers.
- **Error propagation:** Failed `run_gh` in Step 1 of the prompt handler (existing behavior — unchanged) → operator gets aborted message, no dispatch, label persists. Failed `run_claude_pilot` after `create_task` → standard claude-pilot error path. Guard-exhaustion → new operator notification (Unit 3).
- **State lifecycle risks:** None new. The `ready` label is removed atomically by Step 1 before `create_task`/`run_claude_pilot` (the existing #841 ordering invariant).
- **API surface parity:** New INTENT_GUARDS entry is internal to mika-agent; no external API change. `send_message` operator notification reuses existing API.
- **Integration coverage:** Unit 4 (smoke test) is the cross-layer integration test — gateway → agent → guard → run_claude_pilot → claude-pilot worker → PR creation. Unit-level mock-LLM tests in Unit 1/3 cover the engine internals.
- **Unchanged invariants:** mika#841's positive-consent gate stays — only `ready` label triggers dispatch. Webhook Fallthrough scope rule for non-`ready` labels stays. `mika ask` direct-prompt routing (no `[GitHub]` prefix) stays.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| New guard's trigger is too specific and misses edge cases (e.g., uppercase `Ready`, label name with whitespace) | Trigger uses lowercase literal `[GitHub] Issue labeled ready on `; gateway emits this exact format (verified in `crates/mika-gateway/src/github.rs:208–218`). Test with the empty-label-name edge case from #842's review feedback. |
| Operator notification spam if guard fires legitimately during transient LLM hiccups | Single-retry budget (matches existing guards) — notification only fires after BOTH attempts fail. If transient, the retry recovers and no notification fires. |
| Prompt strengthening adds bytes to an already-95.9% prompt | Net addition is ~5–10 lines (~300–500 bytes); leaves ~7% headroom. Aggressive trim is a separate ticket (Deferred). |
| Match-reason rule still leaves `required_tools` inert for AlwaysOn webhook turns more broadly | Out of scope (D4). The new guard solves the specific case. Filing follow-up to broaden if other handlers exhibit similar regressions. |
| The architect process that missed this in #841/#842 review may miss similar bugs in other handlers | Out of scope (Deferred). Filing follow-up to add prose-route detection to mika-arch's critique checklist. |

## Documentation / Operational Notes

- **PR body** must include `Closes #846` AND check off all three of #842's manual test plan items (the unchecked checkboxes that motivated this fix).
- **Compound doc** (Unit 5) is part of the PR, not a follow-up — `feedback_compound_infra_fixes.md`: "compound every non-trivial one."
- **Deploy verification**: after merge + deploy, observe one full `ready`-label cycle on a real ticket end-to-end (close to mika#844, the originally-blocked ticket — Vincent reapplies `ready`, mika-dev dispatches, PR opens). This is the live acceptance, not just the smoke test in Unit 4.

## Sources & References

- **Origin issue:** mika#846
- **Regression source:** mika#842 (`fix(self-dev+gateway): gate dispatch on ready label or direct prompt only`)
- **Originally-blocked ticket:** mika#844 (`refactor(skills): rename claude-pilot skill to dev-pilot`)
- **Prior art (compound docs):**
  - `docs/solutions/architecture-patterns/intent-precondition-registry-guard-generalization-2026-04-21.md`
  - `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
  - `docs/solutions/architecture-patterns/webhook-zero-tools-guard-fabrication-prevention-2026-04-20.md`
  - `docs/solutions/best-practices/prompt-vs-tool-contract-mismatch-2026-04-24.md`
  - `docs/solutions/logic-errors/self-dev-task-not-found-silent-end-turn-2026-04-20.md`
  - `docs/solutions/logic-errors/milestone-callback-misrouted-to-generic-workflow.md`
  - `docs/solutions/runtime-errors/silent-callback-max-steps-exhaustion.md`
  - `docs/solutions/best-practices/intent-signal-not-completion-signal-2026-04-24.md`
  - `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md`
- **Auto-memory citations:** `feedback_prompt_enforcement_fragile.md`, `feedback_smoke_before_claiming_done.md`, `feedback_full_pipeline_always.md`, `feedback_compound_infra_fixes.md`
- **Related code:** `crates/mika-agent/src/agent.rs` (INTENT_GUARDS, run_loop, EndTurn), `skills/bundled/self-dev/system_prompt.md` (handler), `crates/mika-gateway/src/github.rs` (marker source)
- **Related plans:** `docs/plans/2026-04-27-009-fix-gate-dispatch-on-ready-label-plan.md` (#841 plan that #842 implemented; the architect reviews here did not flag the prose-route risk)
