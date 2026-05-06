---
ticket: mika#996
type: feat
title: Auto-groom every ready-labelled ticket through mika-arch before claude-pilot dispatches
date: 2026-05-06
seq: 007
---

# Plan: auto-groom on dispatch (mika#996)

## Verified state (post-architect-pass-1, operator-resolved)

mika-arch pass-1 returned ESCALATE on three concerns. Vincent resolved each:

- **E1 (AC#4 deferral) — resolved by body edit.** mika#996's AC#4 ("Grooming for ticket N+1 runs concurrently with dispatch of ticket N") was stripped from the issue body via 2026-05-06 edit. Reasoning: AC#4 was a **different problem class** than the auto-groom-on-dispatch capability mika#996 is actually about. Genuine N+1-concurrency requires either an agent-lock split (mika#22 / Bounded-B territory) or an ephemeral-worker pattern — not "two concurrent claude-pilot dispatches per agent" (which misframes the agent-state-coherence concern). A+B serial closes mika#996's actual gap. Concurrency filed as **mika#1001** with the design space explicitly NOT pre-decided. Per `feedback_dont_decorate_forced_decisions` and the issue-as-versioned-contract pattern: the contract was wrong; edit the contract.
- **E2 (lift dev-groom operator-only restriction) — resolved by reframing the consent gate.** Architect was ratifying against stale ground. The consent gate did not move from operator-control to autonomous-control; **it relocated**. Original mika#841 gate was "operator slash-command path" because dev-groom was operator-only at the time. After the May 2 worker-agent thread moved dev-groom into the self-dev family as a peer of dev-pilot, the design intent shifted: autonomous mika-dev dispatches grooming the same way it dispatches implementation. **The consent gate is now the `ready` label transition + the existing positive-consent dispatcher (mika#807/#810).** Auto-grooming a `ready`-labelled ticket is not unattended self-grooming — it's responding to a label-event consent signal explicitly emitted by the operator (or operator-directed mika-prime). Phase 1's restriction lift stands. Phase 1 is extended to (a) name this rationale explicitly in the dev-groom skill prompt, and (b) add an `[output] required_suffix_lines` contract for dev-groom (per NF1 below — verified absent in current `skill.toml`).
- **E3 (Phase 0 / U3 contradiction on engine-guard skill-discrimination) — resolved in-session by source read.** `crates/mika-agent/src/agent.rs:4307-4321` and `crates/mika-agent/CLAUDE.md` confirm `webhook_ready_label_dispatch` is skill-agnostic — accepts ANY `run_claude_pilot` attempt regardless of `skill` parameter. Phase 0's claim was structurally correct; brief's U3 hedge was the contradiction. Folded as a one-sentence Phase 0 cleanup below.
- **NF1 (dev-groom callback envelope) — verified at plan time, promoted to Phase 0 Pin.** `dev-groom/skill.toml` has no `[output] required_suffix_lines`. Phase 5 step 19 of dev-groom's prompt posts a summary comment to the ticket but the callback envelope back to mika-dev does NOT pin a `Verdict:` line. Phase 1 is extended to add `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]` to `dev-groom/skill.toml` so mika-dev's auto-groom callback handler can mechanically parse the verdict from the callback text.
- **NF2 (HANDLER CRASH retry-once) — promoted to terminal-semantics rule.** Reframed: the auto-groom path's failure handler enforces (a) **task-reuse on retry** (same `groom_task_id`, no new `create_task`), and (b) **second-crash terminal** — on the second consecutive HANDLER CRASH for the same `groom_task_id`, treat as ESCALATE (surface to operator, do NOT retry again). Naming this as terminal-semantics, not just retry-semantics, eliminates the latent infinite-retry class.

## Why

The orchestrator's manual `/mika-groom-ticket` workflow cannot keep up with cascade-mode dispatch speed. On 2026-05-06, milestone#13 cascade dispatched mika#671 to claude-pilot before the orchestrator could pre-groom it. claude-pilot ran `/ce:plan` from scratch in its worktree, missing the architect's two-pass roundtrip. Retroactive grooming would have collided with the in-flight implementation on the same branch — actively harmful per `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md`.

This is the third structural autonomous-loop fix surfaced this session (alongside mika#988 closed-issue auto-skip and mika#991 post-callback conversational turn). All three address loop **correctness/legibility**, not speed — per `feedback_loop_stability_beats_loop_speed.md`, parallelism justifies on stability.

The fix bar: every ticket that reaches `dev-pilot` has a committed Plan callout from `mika-arch-groom-ticket`'s two-pass cycle, regardless of whether dispatch was triggered by webhook (`ready` label) or milestone cascade.

## Phase 0 — Pin (verified state, source-anchored)

All paths verified against the worktree at HEAD `6225e3af` (main).

### Existing infrastructure — most of this is already built (mika#907 + dev-groom + mika-arch)

**`mika/skills/bundled/self-dev/system_prompt.md`:**

- **Lines 242–278: Ready-Label Dispatch atomic handler** — already enforces a grooming pre-flight, but currently REJECTS ungroomed tickets rather than auto-grooming. Specifically, lines 256–264 (Step 3, GROOMING PRE-FLIGHT — mika#907):

  ```
  3. **Third (GROOMING PRE-FLIGHT — mika#907)**, scan the fetched issue body for the grooming marker: `> - **Plan:**`. ...

     **If the marker is NOT found in the issue body:** Do NOT call `create_task` or `run_claude_pilot`. Call `send_message` to notify the operator:
     > "Ready-label dispatch blocked on `<repo>#<n>`: ... Run `/mika-groom-ticket <repo>#<n>` to produce the plan, then re-add the `ready` label."
  ```

  This is the rejection path. mika#996 converts this branch into an auto-groom dispatch.

- **Engine guard `webhook_ready_label_dispatch`** — verified skill-agnostic at `crates/mika-agent/src/agent.rs:4307-4321` and per `crates/mika-agent/CLAUDE.md` § "Intent-precondition registry (#702)". The guard's `satisfied` predicate (`ready_label_dispatch_satisfied`) checks only that `run_claude_pilot` was attempted (success or terminal failure) — it does NOT inspect the `skill` parameter. Both `dev-pilot` AND `dev-groom` dispatches satisfy the guard identically. **No engine-level Rust change needed for the webhook path.** This claim is source-anchored, not asserted.

- **Lines 620–669: Milestone Workflow Step M4 (Serial execution loop)** — currently has zero grooming pre-flight. M4 Step 2 reads "Execute per-issue flow (Steps 1-6 from main workflow): Read GitHub issue / Launch claude-pilot with `task_id=<child_task_id>` / Wait for completion callback / ..." Launch is direct `dev-pilot` with no Plan-callout check. **This is the milestone-cascade gap that produced the mika#671 incident.**

**`mika/skills/bundled/dev-groom/skill.toml` and `system_prompt.md`:**

- Line 1 of system_prompt.md: `## dev-groom — Operator-Triggered Grooming Skill`
- Line 3: `This skill is operator-only — never auto-invoke from webhooks or autonomous flows.`

  This is a deliberate restriction that mika#996 lifts (or carves an exception for autonomous flows).

- The skill's actual grooming sequence (Phase 1–5 with two-pass architect review) is already implemented and matches `/mika-groom-ticket` exactly. **No change to grooming sequence itself.**

**`mika/skills/bundled/mika-arch-groom-ticket/`:**

- Already exists. Used by both `/mika-groom-ticket` (orchestrator path) and `dev-groom` (autonomous path) for the architect roundtrip. **No change.**

### Webhook event format

Webhook event message that triggers Ready-Label Dispatch starts with: `[GitHub] Issue labeled ready on <repo>#<n>`. The handler at lines 248–276 parses `<repo>` and `<n>` from this prefix.

### Engine task model — no Rust changes needed (provisional, verify in implementation)

- `run_claude_pilot` is the engine-level tool call that dispatches a long-running claude-pilot subprocess.
- It accepts `skill="dev-pilot"` OR `skill="dev-groom"` (per dev-groom's existing tools.json contract — verify at implementation).
- Callbacks return to the originating session; the engine has no special-case for groom-vs-implement — both are just terminal callbacks.
- The grooming task is created via `create_task` with `source: "self_dev"` and links to the same `reference_url` as the eventual dispatch task. **Verify at implementation:** does `create_task` enforce uniqueness on `reference_url`? If yes, the grooming task and dispatch task need different `reference_url` shapes (e.g., add a `?phase=groom` suffix, or use the ticket URL plus a discriminator). If no, both can share the URL.

### Symptom evidence

- mika#671 dispatched ungroomed via milestone#13 cascade on 2026-05-06 (~11:30Z). Verified via `mika tasks --agent mika-dev` showing #671 in_progress with no grooming sub-task as predecessor. Orchestrator session was mid-grooming #668 at the time.
- mika#907 closed (the precursor ticket that added the rejection path) — verifiable via `gh issue view 907 --repo senara-solutions/mika --json state`.

## Scope

**In scope:**

- **Phase 1:** Lift the operator-only restriction in `mika/skills/bundled/dev-groom/system_prompt.md` line 3, OR carve an explicit exception for autonomous-loop invocations.
- **Phase 2 (webhook path):** Replace the rejection branch in `self-dev/system_prompt.md` Ready-Label Dispatch Step 3 (lines 256–264) with an auto-groom-dispatch branch. After `dev-groom` callback returns successful (Plan callout now present in issue body), the handler re-enters the dispatch flow and fires `dev-pilot`.
- **Phase 3 (milestone path):** Add a Plan-callout pre-flight to `self-dev/system_prompt.md` Milestone Workflow Step M4 Step 2. Before launching `dev-pilot` for a child, check the child's issue body for `> - **Plan:**`. If absent, launch `dev-groom` first (using the same `child_task_id`); on its callback, then launch `dev-pilot`.
- **Phase 4:** Integration tests for both paths.
- **Phase 5:** Documentation updates.
- **Phase 6:** Follow-ups filed at PR-merge time.

**Out of scope (explicitly):**

- **Replacing `/mika-groom-ticket` slash command.** The orchestrator-side command stays — used for explicit human-driven grooming, free-text tickets, and any path outside the autonomous loop.
- **Engine-level Rust changes to dispatch logic.** Phase 0's pin shows the engine guard already accepts `run_claude_pilot` regardless of skill. Plan stays at the prompt+skill layer. **If implementation discovers an engine-level constraint that requires Rust changes, halt and surface to operator** — that's a meaningful scope expansion that warrants its own ticket.
- **Concurrency/pipelining (groom-N+1 while dispatch-N runs).** The ticket body's Option C proposed this. **Deferred to a follow-up** because: (a) the steady-state cadence of cascade dispatch is already minutes-to-hours per ticket, so serial groom→dispatch adds an acceptable ~15-25 min per ticket; (b) implementing concurrency requires the engine to allow two concurrent claude-pilot dispatches, which is a load-bearing engine constraint not currently in place. The simpler serial fix lands here; concurrency lands as its own ticket if cadence proves unacceptable.
- **Changing the architect's two-pass discipline.** The READY/ITERATE/ESCALATE → GROOMED/ESCALATE cycle is unchanged — only the trigger model changes.
- **Auto-grooming on labels other than `ready`.** Labels like `enhancement`, `p1-important`, etc. continue to fall through per the existing "Other label-add events" rule (line 278). Auto-groom only fires on `ready`.

**Position on the four ticket-body options (defended):**

The ticket body proposed A (pre-dispatch grooming hook), B (milestone-parent-aware), C (Hybrid A+B with concurrency), and D (orchestrator depth-widening band-aid). This plan picks **A+B without the concurrency layer** — neither pure-A nor pure-C.

- D rejected: band-aid, doesn't fix the structural race.
- A alone: covers webhook path (`ready` label) but leaves milestone-cascade ungroomed.
- B alone: covers milestone-cascade but leaves direct `ready`-label dispatch ungroomed.
- C (Hybrid + concurrency): right architecture but the concurrency layer requires engine changes (allow two concurrent claude-pilot dispatches) that meaningfully widen scope.

**A+B serial:** auto-groom on both paths, run grooming serially before dispatch. ~15–25 min per ticket added cadence cost. Concurrency follows as a follow-up if/when the cost proves unacceptable.

## Phase 1 — Reframe dev-groom invocation contexts and add output contract

**Files touched:**
- `mika/skills/bundled/dev-groom/system_prompt.md` — replace operator-only line with multi-context framing + add Consent gate relocation rationale paragraph.
- `mika/skills/bundled/dev-groom/skill.toml` — add `[output] required_suffix_lines` (NF1 promotion).

### 1.A — Update `system_prompt.md` line 3 + add Consent gate relocation rationale

**Current line 3:**
> This skill is operator-only — never auto-invoke from webhooks or autonomous flows.

**New text (replaces line 3 plus inserts a rationale paragraph immediately after the skill's opening sentence):**
> This skill is invoked in three contexts: (1) operator-direct via the `/mika-groom-ticket` slash command, (2) autonomous webhook-triggered when a `ready`-labelled ticket lacks a Plan callout (via mika#996's auto-groom flow), and (3) autonomous milestone-cascade pre-flight when a milestone child lacks a Plan callout. The grooming sequence (Phases 1–5, two-pass architect review) is identical across all three contexts.
>
> **Consent gate relocation (mika#996):** Earlier versions of this skill restricted invocation to operator-only paths because the consent gate was the slash-command path itself. After dev-groom moved into the self-dev family as a peer of dev-pilot (May 2 worker-agent thread), the design intent shifted: autonomous mika-dev dispatches grooming the same way it dispatches implementation. The consent gate **relocated** to the `ready` label transition + the existing positive-consent dispatcher (mika#807/#810). Auto-grooming a `ready`-labelled ticket is not unattended self-grooming — it's responding to a label-event consent signal explicitly emitted by an operator (or an operator-directed mika-prime). The denylist (mika#811) and the spec-deviation pause (Vincent-only judgment-call protocol) remain the operator-control surfaces over what mika-dev is allowed to do; whether mika-dev grooms one of its own `ready`-labelled tickets is downstream of those gates, not parallel to them. This rationale is named explicitly here so the next reader does not have to reconstruct it from prior threads.

The skill prompt's actual grooming logic does not change — only the framing note + rationale.

### 1.B — Add `[output] required_suffix_lines` to `skill.toml` (NF1 promotion)

**Current `dev-groom/skill.toml` (verified at plan time):**

```toml
[skill]
name = "dev-groom"
description = "Operator-triggered two-pass grooming flow — takes a ticket from open to GROOMED plan-on-branch via /ce:plan and mika-arch architect review"
version = "0.1.0"
always_on = false
timeout_secs = 600

[triggers]
keywords = ["groom", "groom ticket", "/mika-groom-ticket", "groom issue"]
```

No `[output]` section exists. The skill's callback envelope to mika-dev currently contains operator-prose summary text without a pinned final verdict line. mika-dev's auto-groom callback handler (Phase 2 step d, Phase 3 step c) needs to mechanically parse `Verdict: GROOMED` or `Verdict: ESCALATE` from the callback text. Without a pinned suffix, the parse depends on the LLM happening to emit the line — fragile.

**New section to add:**

```toml
[output]
required_suffix_lines = [
    "Verdict: GROOMED",
    "Verdict: ESCALATE",
]
```

This opts dev-groom into the engine-level `required-suffix-line guard (#864)` — the LLM's response is rejected if its last 3 non-empty lines do not include an exact match for one of the entries. Same pattern already used by `mika-arch-second-review` and `mika-arch-groom-ticket`.

**Update `dev-groom/system_prompt.md` Phase 5 step 19** to also emit the verdict as the final line of the callback summary message:

```markdown
19. Post a summary comment on the ticket. End the callback summary with a final line matching exactly one of:
    - `Verdict: GROOMED` (after a successful second-pass GROOMED disposition)
    - `Verdict: ESCALATE` (after either pass returned ESCALATE)
    The engine's required-suffix-line guard enforces this — your turn will be rejected if the line is absent.
```

Description update: the `[skill] description` field also gets a one-line edit to drop "Operator-triggered" → "Two-pass grooming flow (operator or autonomous)" — small alignment with the lifted restriction.

### 1.C — Verify no other code path enforces operator-only as a contract

**Pre-implementation verification (mandatory before Phase 1.A's edit):**

```bash
grep -rn "operator-only" mika/
grep -rn "auto-invoke\|never auto-invoke" mika/
grep -rn "dev-groom" mika/crates/ mika/skills/
```

If any code outside `dev-groom/system_prompt.md` references the operator-only restriction as a contract (e.g., a hardcoded check that rejects `dev-groom` invocations from non-operator sources), Phase 1 must address that site too — surface to operator before editing if the surface is wider than expected.

Expected: no other enforcement sites. The restriction was prompt-only, per the May 2 thread documenting that the move into the self-dev family did not add Rust-level enforcement.

## Phase 2 — Webhook path: replace rejection with auto-groom dispatch

**File touched:** `mika/skills/bundled/self-dev/system_prompt.md`, lines 256–264.

**Current rejection text (verbatim):**
```
**If the marker is NOT found in the issue body:** Do NOT call `create_task` or `run_claude_pilot`. Call `send_message` to notify the operator:

> "Ready-label dispatch blocked on `<repo>#<n>`: issue body lacks the grooming marker (`> - **Plan:**`). The ticket must be groomed before dispatch. Run `/mika-groom-ticket <repo>#<n>` to produce the plan, then re-add the `ready` label."

Stop the turn after `send_message`. The engine guard accepts `send_message` as valid completion for this path.
```

**New auto-groom-dispatch text:**
```
**If the marker is NOT found in the issue body:** The ticket is ungroomed. Auto-groom via `dev-groom` skill before dispatching.

  a. Call `create_task` with `reference_url: "https://github.com/<repo>/issues/<n>"`, `label: "groom <repo>#<n>"`, `description: <issue body>`, `source: "self_dev"`. Capture the returned `task_id` as `groom_task_id`.

     **Note on idempotency:** if `create_task` is unique-on-`reference_url`, use a discriminator: `reference_url: "https://github.com/<repo>/issues/<n>?phase=groom"` (verify exact form at implementation; the dispatch task created in Step 4 of the post-groom path uses the canonical URL without the discriminator).

  b. **IMMEDIATELY** call `run_claude_pilot` with:
     ```json
     {"skill": "dev-groom", "prompt": "<repo> issue#<n>", "task_id": "<groom_task_id>"}
     ```

  c. Stop the turn. The grooming task runs in the background; its callback re-enters this session's task loop with the grooming result. **Do not call `send_message` to notify the operator** — auto-grooming is the new default behavior, not an exception.

  **On the dev-groom callback (received as a regular post-callback turn):**

  d. Verify the callback indicates `Verdict: GROOMED` (the dev-groom skill posts this in its summary message to the issue and includes it in the callback result text).

  e. **If GROOMED:** Re-enter the Ready-Label Dispatch flow at Step 4 (create_task + run_claude_pilot for `dev-pilot`). The issue body now has the Plan callout because dev-groom added it. The dispatch task uses the canonical `reference_url` (no `?phase=groom` suffix).

  f. **If ESCALATE:** dev-groom surfaces an architect ESCALATION. Treat as a blocking event: send_message to operator with the ESCALATE reason from the callback, mark the parent task `blocked` if applicable, stop the turn. Do NOT auto-dispatch.

  g. **If callback indicates failure (HANDLER CRASH, timeout, etc.) — terminal-semantics rule (NF2):**
     - **Retry policy:** retry once, **reusing the same `groom_task_id`** (do NOT call `create_task` again — that would resurrect a failed task as a fresh one and lose the failure-count metadata that drives the second-crash detection). The retry is `run_claude_pilot({"skill": "dev-groom", "prompt": "<repo> issue#<n>", "task_id": "<existing groom_task_id>"})`.
     - **Second-crash terminal:** on the second consecutive HANDLER CRASH for the same `groom_task_id`, treat as ESCALATE. Send_message to operator with both failure reasons concatenated; stop the turn. Do NOT retry a third time.
     - **Why this is a terminal-semantics rule, not just a retry-semantics rule:** without the second-crash terminal clause, a flapping dev-groom subprocess (e.g., transient mika-arch API rate-limit, OOM in the architect roundtrip) would re-fire on every webhook redelivery or every milestone-cascade tick, with no upper bound. The terminal clause closes the latent infinite-retry class.
     - **Implementation note:** the failure-count is tracked in the `groom_task_id` task's metadata (`metadata.groom_crash_count`, incremented by mika-dev's callback handler on each HANDLER CRASH). The check `groom_crash_count >= 2` triggers the terminal path. mika-dev must increment deterministically on the callback handler, not rely on the LLM to count.
```

**Engine-guard compatibility:** the new path satisfies `webhook_ready_label_dispatch` because Step b's `run_claude_pilot` (with `skill: dev-groom`) is a `run_claude_pilot` attempt. The guard accepts it. No engine change.

**The "Other label-add events" rule (line 278) is unchanged.** Only `ready` triggers auto-groom; other labels still fall through.

## Phase 3 — Milestone path: add Plan-callout pre-flight to M4 Step 2

**File touched:** `mika/skills/bundled/self-dev/system_prompt.md`, Step M4 lines 620–669.

**Insertion point:** between current M4 Step 1 (update child to `in_progress`) and M4 Step 2 (Execute per-issue flow / Launch claude-pilot).

**New M4 Step 1.5 (insert):**
```
1.5. **Grooming pre-flight (mika#996):** Before launching `dev-pilot` for the child, verify the child's issue body has the `> - **Plan:**` callout. Run:

```json
run_gh({
  "command": ["issue", "view", "<issue_number>", "--json", "body", "--jq", ".body"],
  "repo": "senara-solutions/<repo>"
})
```

If the response contains the literal string `> - **Plan:**`, proceed to Step 2 (existing per-issue flow with `dev-pilot`).

If the response does NOT contain `> - **Plan:**`, the child is ungroomed. Auto-groom before dispatching:

a. **Update child status to track grooming phase:** `update_task_status(task_id=<child_task_id>, status="in_progress", note="Grooming via dev-groom before dev-pilot dispatch (mika#996)")`. The child task remains the same `task_id` — grooming and dispatch are two phases of the same child task.

b. **Launch dev-groom:**
   ```json
   run_claude_pilot({"skill": "dev-groom", "prompt": "<repo> issue#<issue_number>", "task_id": "<child_task_id>"})
   ```

c. **Wait for the dev-groom callback.** This is a normal post-callback turn — handle per the existing callback flow but recognize the `dev-groom` skill output:
   - If callback indicates `Verdict: GROOMED`, the issue body now has the Plan callout. **Re-enter M4 Step 2** for the same child (now the dev-pilot dispatch).
   - If callback indicates `Verdict: ESCALATE`, treat as `blocked` per M4 Step 3 (PAUSE milestone, notify Vincent).
   - **If callback indicates failure (HANDLER CRASH, timeout, etc.) — terminal-semantics rule (NF2):** same shape as Phase 2 step g. Retry once with the **same `child_task_id`** (no new `create_task`); on second consecutive HANDLER CRASH for the same `child_task_id`, treat as `blocked` per M4 Step 3 (PAUSE milestone, notify operator, stop). Do NOT retry a third time. The `groom_crash_count` metadata is tracked on the child task itself (the milestone child, NOT a separate groom task — milestone-cascade reuses the child task across grooming + dispatch phases per Phase 3 step a).

d. **Engine-guard implications:** the milestone-cascade path does not flow through `webhook_ready_label_dispatch` (it's the operator-direct + milestone-parent-spawned path). No new guard is needed; M4's existing dispatch-readiness checks already accept `dev-groom` as a valid `run_claude_pilot` skill.
```

**Cadence:** each ungroomed child adds ~15–25 min for the architect roundtrip (two passes). For milestone#13's remaining children (~6 tickets, none currently groomed), this would add 90–150 min of cumulative grooming time. **Acceptable** because grooming runs serially with dispatch; the milestone's overall wall-clock grows but correctness is guaranteed.

**Step M4 Step 2 unchanged.** It continues to launch `dev-pilot` after the optional grooming gate completes. The existing post-callback handling (steps 3, 3b deploy hook, etc.) is unchanged.

## Phase 4 — Tests

**Test 1 (webhook-path auto-groom):** simulate a `[GitHub] Issue labeled ready on mika#NNN` event for an issue whose body lacks the Plan callout. Assert:
- mika-dev calls `run_claude_pilot` with `skill="dev-groom"` (not `send_message` rejection).
- After the simulated dev-groom callback returns GROOMED, mika-dev calls `run_claude_pilot` with `skill="dev-pilot"`.
- The total turn count is 2 (groom turn + dispatch turn), not 1 (rejection only) and not >2 (no extra confirmation prompts).

**Test 2 (milestone-cascade auto-groom):** simulate Step M4 entry for a child whose body lacks Plan callout. Assert:
- M4 Step 1.5 fires `run_claude_pilot` with `skill="dev-groom"`.
- After the simulated dev-groom callback, M4 Step 2 fires `run_claude_pilot` with `skill="dev-pilot"`.
- The child task's status is `in_progress` throughout (not transitioned to `completed` between phases).

**Test 3 (already-groomed bypass):** for both paths, simulate an issue whose body DOES contain the Plan callout. Assert dev-groom is NOT invoked; dispatch goes straight to dev-pilot.

**Test 4 (ESCALATE path):** simulate dev-groom returning `Verdict: ESCALATE`. Assert mika-dev does NOT auto-dispatch dev-pilot; for webhook path, mika-dev calls `send_message` to operator with the ESCALATE reason; for milestone path, mika-dev pauses the milestone and notifies operator.

**Test harness:** existing eval harness in `crates/mika-agent/tests/eval/` already supports MockLlmProvider sequence-based tests. Tests live in that directory. Each test stubs the dev-groom and dev-pilot dispatch results to assert the orchestration sequence, not the actual grooming/implementation work.

**Halt threshold (sibling of mika#988 Phase 3 and mika#667 Phase 2.B):** if any test requires more than **80 lines** of harness setup beyond the existing pattern (e.g., a new mock skill, a new fake task type), halt and surface to operator before writing further tests. The existing mock-LLM-driven eval harness should suffice; significantly larger setup signals an architectural mismatch.

## Phase 5 — Documentation

**Files updated:**

- **`mika/skills/bundled/self-dev/system_prompt.md`** — atomic handler text rewritten per Phase 2; M4 Step 1.5 inserted per Phase 3. The "Atomic handler" section title may want a brief note that the new flow auto-grooms instead of rejecting (one-line addition near line 248).

- **`mika/skills/bundled/dev-groom/system_prompt.md`** — operator-only restriction lifted per Phase 1.

- **`mika/CLAUDE.md`** — autonomous-loop section: one-paragraph note that the loop now auto-grooms ungroomed `ready`-labelled or milestone-child tickets before dispatching, with grooming and dispatch running serially as two phases of the same child task.

- **`mika-platform/.claude/commands/mika.md`** (in the meta-repo, separate PR) — note that orchestrator-manual `/mika-groom-ticket` becomes optional for autonomous-loop tickets, still required for free-text dispatch and human-driven grooming. **This is in a different repo and should be filed as a companion PR, not bundled into this PR.** Mark it as a Phase 6 follow-up cross-repo work.

- **`mika/docs/solutions/best-practices/auto-groom-on-dispatch-2026-05-06.md`** (new compound at PR-close time) — the principle: grooming is not orchestrator overhead; it's a phase of the dispatch pipeline.

## Phase 6 — Out-of-scope follow-ups (filed at PR-merge time)

1. **Concurrency: groom-N+1 while dispatch-N runs — filed as mika#1001.** Design space explicitly NOT pre-decided in the follow-up: the architect's grooming pass for mika#1001 picks between agent-lock split (mika#22 / Bounded-B territory), ephemeral-worker pattern, or mika-worker pool. **Do NOT pre-commit to "two concurrent claude-pilot dispatches per agent" framing** — that's a misframing of the agent-state-coherence concern. The decision is which agent-state-coherence model the autonomous loop adopts.

2. **Companion PR on mika-platform: update `/mika.md` to note auto-groom is the default.** Single-line change. File as `mika-platform#NNN`.

3. **Auto-groom on label-add for non-ready labels (e.g., `p0-critical`).** Out of scope here; could be useful for high-priority tickets that should always be groomed regardless of who applies which label. Defer until operator workflow demands it.

4. **Engine-level metric: count auto-grooms per day.** A small Prometheus counter (`mika_autogroom_total{trigger="webhook|milestone"}`) for visibility into how often auto-groom fires vs how often tickets arrive already-groomed. Useful for understanding orchestrator-vs-autonomous load split. Defer.

## Acceptance criteria (from the ticket — post-2026-05-06 body edit)

mika#996's body was edited 2026-05-06 to strip the original AC#4 (concurrent groom-N+1 / dispatch-N). The current AC list reads:

- [x] Every ticket reaching claude-pilot has been through the two-pass `mika-arch-groom-ticket` cycle, with `Plan:` callout committed on its dispatch branch. **Phases 2 + 3** (both webhook and milestone paths gated).
- [x] The architect roundtrip is **not** orchestrator-blocking — runs autonomously in mika-dev's queue. **Phase 1 + 2 + 3** — orchestrator no longer needed for ready-labelled or milestone-cascade tickets.
- [x] If grooming returns ESCALATE, the ticket halts at grooming and surfaces to operator — does NOT auto-fall-through to dispatch. **Phase 2 step f, Phase 3 step c.**
- [x] Tests at the engine level: cascade test that enqueues 5 tickets, asserts each has Plan callout before dispatch fires. **Phase 4 Test 1+2.**
- [x] Documentation updates per the ticket. **Phase 5.**

**Closure shape:** mika#996 closes with full AC satisfaction post-body-edit. The concurrency layer (formerly AC#4) is **mika#1001**, filed at body-edit time per the issue-as-versioned-contract pattern. Body edit + linked follow-up is the canonical lighter shape compared to closing-unsatisfied-and-superseding (per the April 27 lifecycle thread on mika#807).

## Risks and known unknowns

- **Risk: `create_task` uniqueness-on-reference_url behavior.** Phase 2's auto-groom path creates a grooming task that may share `reference_url` with the eventual dispatch task. If `create_task` is unique-on-URL, the second `create_task` (for dispatch) returns the existing groom task's ID instead of creating a new one. Mitigation: verify at implementation; use a `?phase=groom` discriminator if needed. **Halt and surface to operator if the discriminator approach conflicts with downstream URL-matching code (e.g., webhook correlation by URL).**

- **Resolved at plan time (was NF1):** dev-groom callback envelope — verified absent in current `dev-groom/skill.toml` (no `[output]` section). Promoted to Phase 1.B: add `[output] required_suffix_lines = ["Verdict: GROOMED", "Verdict: ESCALATE"]` and update Phase 5 step 19 of the dev-groom prompt to emit the verdict as the final line. The engine's `required-suffix-line guard (#864)` then enforces the contract. No implementation-time uncertainty.

- **Risk: cadence cost during transition period.** Milestone#13 has ~6 ungroomed children remaining; auto-grooming each adds 15–25 min. Total milestone#13 wall-clock grows by ~90–150 min after this PR ships. Mitigation: this is acceptable; concurrency (Phase 6 follow-up #1) closes the gap if it proves unacceptable.

- **Risk: dev-groom skill prompt's "operator-only" warning is load-bearing in some downstream check.** Phase 1 lifts it. Mitigation: grep for the literal "operator-only" string in `mika/` before committing; verify no other code or doc references it as a contract.

- **Resolved at plan time (was U3):** engine guard skill-discrimination — verified skill-agnostic at `agent.rs:4307-4321`. No implementation-time uncertainty. Removed from risk list.

- **Unknown: whether deploy-hook label semantics (`needs-build`, `needs-deploy`) need to fire after grooming or only after dispatch.** Currently M4 Step 3b's deploy hook fires post-implementation. With grooming added as a phase, the deploy hook should still fire post-implementation only (grooming doesn't produce deployable artifacts). Verify at implementation that no logic accidentally triggers deploy-hook after grooming.

## Compound learning to write at PR-close

A compound at `mika/docs/solutions/best-practices/auto-groom-on-dispatch-2026-05-06.md`. Title: **"Grooming as a phase of dispatch, not as orchestrator overhead."** Principle:

> When a ticket-handling pipeline has a "validate-before-execute" step that depends on an LLM-driven prerequisite (architect review, design check, security audit, etc.), the prerequisite should run as a phase of the pipeline itself, not as a manual gate the operator runs separately. Manual gates do not scale with pipeline cadence — when the pipeline accelerates, the gate becomes the bottleneck. Make the gate's invocation part of the pipeline's contract.

Cite this PR's two integration points (webhook `ready` handler, milestone M4) as instances of the principle. Cite as the contrapositive: mika#988 (closed-issue auto-skip) where the gate IS the right shape because the validation is non-LLM-driven and instant.
