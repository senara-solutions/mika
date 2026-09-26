# Plan review pass 2 — verify-post-state pattern for self-dev milestone M4 + memory hygiene (mika#789)

Pass 1 verdict: ITERATE. Three blockers and three polish items. This pass addresses each.

## Scope discipline (pre-emptive)

mika#789's stated scope is: add verify-post-state semantics to M4 (auto-merge gate) and M1/M5 (memory hygiene). It does NOT redesign M4's loop control flow. Specifically:

- **M4 advance behavior on `auto_merge_enabled` is unchanged from current.** The verify-post-state addition runs *after* advance, not as a re-entry guard.
- **The HOLD state introduced in Phase 1** is a *task-status note* the LLM observes when re-entering M4. The question "how does M4 re-enter against a child already in HOLD" is a pre-existing control-flow gap that surfaces because HOLD makes it observable, but the bug ("M4 advances without verifying merged state") exists *today* with no HOLD state at all.
- **The HOLD re-entry design call is tracked as mika#1208** with explicit option space (engine-halt vs iterator-over-non-HOLD) for architect review post Phase C / post mika#1207.

Peer-Claude reviewers: please accept Blocker 1 as a scope-punt-with-tracking. The verify-post-state semantics being added are correct *with or without* a HOLD re-entry fix. If you disagree on principle, escalate to Vincent — do not block pass-2 on inline resolution of mika#1208's design space.

## Acceptance criteria

AC1. `skills/bundled/self-dev/system_prompt.md` M4 includes a new step 2.5 (merge verification gate) between existing steps 2 and 3. The new step explicitly distinguishes the four `pr_merge_with_gate` return values (`merged`, `already_merged`, `auto_merge_enabled`, `blocked`/`gate_errored`) with deterministic per-value handling.

AC2. M1 step adds an `update_core_memory` call writing `current_priorities` with the milestone in-progress shape. M5 step 5 adds the matching `update_core_memory` call writing `current_priorities` with the milestone-completed shape.

AC3. Every `gh <subcommand>` referenced anywhere in `skills/bundled/self-dev/system_prompt.md` is in the post-#788 `run_gh` allowlist. Grep cross-check confirms zero hallucinated subcommands.

AC4. Validation: both `cargo test -p mika-agent --test eval test_callback_milestone_advance` and the `skill_self_dev_plan_coherence` golden test pass with the modified prompt. A manual M4 dry-run against a fixture child in `auto_merge_enabled` state produces the expected verify-step trace.

## Blocker resolutions

### Blocker 1 (HOLD re-entry semantics) — scope-punt to mika#1208

**Resolution: defer, do not address in #789.**

The pass-1 brief introduces HOLD as a *task-status note* the LLM reads at re-entry. The actual transition rule (`reprocess?` `resume to next?` `skip?`) is a new control-flow concept with no existing vocabulary in `self-dev/system_prompt.md` — confirmed by grep of "skip" usage in the file (operator-instructed cancellation at line 484, "do NOT skip" imperatives for setup; no skip-and-re-check pattern).

Choosing between option (a) (M4 engine-halts on any HOLD child, dispatcher resumes on webhook) and option (b) (iterator-over-non-HOLD with HOLD-timeout escape) is a state-machine design call that belongs at architect level, not in a prompt edit. Both are filed in mika#1208 as the option space.

**Why this is correct scope for #789:**
- The bug #789 targets ("M4 advances on `auto_merge_enabled` without verifying merged state") exists today with no HOLD state at all.
- #789's verify-post-state addition runs *after* the LLM's advance decision, not as a re-entry guard.
- Current M4 behavior on `auto_merge_enabled` is preserved: the LLM still observes the QA-pass webhook turn, can still pattern-match into "next child" mode if it wants, and the existing webhook handler's "PR Closed (auto-merge completion)" entry point still completes the task on merge. #789 adds an explicit `run_gh pr view` verification *call*, not a behavioral guard.

**What pass-2 will do in the prompt:** Where pass-1 said "explicit HOLD — child stays in_progress, no next-child dispatch", pass-2 says "**verify-only addition:** call `run_gh(['pr', 'view', '<num>', '--json', 'state,mergedAt'])` and store result via store_fact. The LLM's existing advance behavior on `auto_merge_enabled` is unchanged from current; the verify call is observability for the post-state, not a HOLD gate. HOLD-as-gate semantics are out-of-scope, tracked at mika#1208."

This is a smaller change than pass-1 proposed and avoids the state-machine question entirely. If the architect disagrees, escalate; do not force inline design.

### Blocker 2 (Phase 1 step 2.5 has no failure branch) — explicit 4-mode enumeration

The new step 2.5 specifies handling for every documented failure mode of the verification call. Pass-2 prompt text:

> **2.5. Merge verification (verify-post-state, additive observability):**
>
> After `pr_merge_with_gate` returns for this child's PR, call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")` and dispose per the following table:
>
> | `pr_merge_with_gate` return | `run_gh` result | Disposition |
> |-----------------------------|-----------------|-------------|
> | `merged` / `already_merged` | `state == "MERGED"`, `mergedAt` non-null | `store_fact(category="event", description="...verified merged on GitHub at <mergedAt>")`, proceed to step 3 with outcome `completed` |
> | `merged` / `already_merged` | `state != "MERGED"` OR `mergedAt` null | **Halt:** `update_task_status(task_id=<child_wi>, status="failed", note="merge state divergent: pr_merge_with_gate reported merged, gh pr view shows <state>/<mergedAt>")`. Surface to operator via `send_message`. Stop loop. |
> | `merged` / `already_merged` | `gh pr view` non-zero exit (transient API error) | Single retry after 5s. If retry also fails, halt with note "gh pr view verification failed after retry: <stderr>". |
> | `auto_merge_enabled` | (verify call still runs — `state` will be `OPEN`) | `store_fact(category="event", description="...auto-merge enabled, GitHub state OPEN at verify time")`. **Proceed to step 3 with outcome `in_progress`** (current behavior preserved per mika#1208 carve-out). Webhook handler completes the task on `pull_request.closed(merged: true)`. **The verify call on `auto_merge_enabled` is forensic, not behavioral — it pins the GitHub state at advance time to the work-item record, so post-hoc divergence investigations have ground truth without log archaeology.** |
> | `blocked` / `gate_errored` | (verify call still runs — `state` informational) | Existing webhook handler routes per its block/error path. Step 3 sees outcome `blocked` and follows the milestone-pause flow at line 484. |
> | Any | `run_gh` returns parse error (JSON malformed) | **Halt:** `update_task_status(task_id=<child_wi>, status="failed", note="gh pr view JSON parse failed: <output snippet>")`. Surface to operator. Stop loop. |

The "halt" disposition follows the same pattern as M2's GATE handling (line 393: "If `resolve_issue_order` fails ... fall back ... log warning. Do NOT skip M3"). Operator-recoverable; not silently swallowed.

### Blocker 3 (no verification plan for the prompt change) — Phase 5 added

Add new Phase 5 to the plan:

**Phase 5 — Validation (gating, blocks merge)**

1. **Eval suite — automated:**
   - `cargo test -p mika-agent --test eval test_callback_milestone_advance` — exercises M4 callback advance path with the modified prompt. The existing test scenarios for `pr_merge_with_gate` return values must continue to pass; the new verify-step trace appears in the expected mock sequence.
   - `cargo test -p mika-agent --test eval --test skill_self_dev_plan_coherence` — golden test for the self-dev prompt's structural coherence (skill-level integration). Must pass with the modified prompt.

2. **Manual fixture dry-run — additive:**
   Run the agent loop against a synthetic fixture child task in `auto_merge_enabled` state with a mocked `run_gh pr view` returning `state=OPEN`. Verify the LLM produces:
   - The `run_gh pr view` call with correct args
   - The `store_fact` capturing the post-state observation
   - The `update_task_status` with outcome `in_progress` (NOT a premature `completed`)

   Fixture lives at `crates/mika-agent/tests/eval/fixtures/<convention-aligned name>` *(implementer: align with existing fixture conventions in `tests/eval/fixtures/` — JSON file, fixture builder, or other. Name to match sibling fixtures' shape.)*

3. **Rollback plan:**
   This is a prompt-only change. Rollback = revert the `system_prompt.md` commit on `feat/789/self-dev-verify-post-state-milestone-workflow`, `make deploy`, restart. One-commit rollback, ~3min recovery. No data migration, no schema changes, no downstream consumers to coordinate.

   Trigger condition for rollback: production milestone run shows an LLM-level regression (e.g., the verify call fires but the LLM mis-disposes the result, marks a non-merged child `completed`). Detection via the existing post-condition guards (`completion-claim guard` at EndTurn catches false "merged" claims).

## Polish resolutions

### Polish 4 — AC1–AC4 mirrored inline

Done above in the "Acceptance criteria" section. Pass-1 referenced "AC4" without showing the list; this is fixed.

### Polish 5 — Phase 4 (grep cross-check) reclassified

Pass-1 listed Phase 4 as an implementation phase. It's verification, not implementation. Moved into Phase 5 as a sub-step:

> **Phase 5 step 4 — Allowlist grep cross-check:**
> *Implementer: derive the allowlist from current match arms in `crates/mika-agent/src/tools/run_gh.rs` at implementation time. Grep regex constructed from that source of truth, not from this brief. Zero hallucinated subcommands is the gate; the regex is implementation detail.*

The plan now has three implementation phases (1: M4 merge verify; 2: M1 memory hygiene; 3: M5 memory hygiene) + one validation phase (5) covering eval, fixture, rollback, grep.

### Polish 6 — M5 placement declared settled

Pass-1 left uncertainty #3 open ("After `store_fact` (step 5) and before notify (step 6). Should it go before or after `update_task_status(completed)` (step 4)?").

**Settled:** After `store_fact` (step 5), before notify (step 6). Reasoning: `store_fact` writes the verified-completion event first, so the memory write reflects the persisted record. Placing the memory update before `update_task_status(completed)` would write "milestone completed" memory while the task is still `in_progress` — drift between memory and task state. Placing after `store_fact` ensures the memory write follows the durable persistence.

This is no longer an open question; pass-2 ships it as the chosen ordering.

## What's unchanged from pass 1

The three implementation edits (Phase 1 M4 verify, Phase 2 M1 memory, Phase 3 M5 memory) retain their structural shape — site of insertion, syntactic form, surrounding context. Pass-2 changes only the *content of Phase 1's verify-step* (no HOLD-gate prose, explicit failure-mode table per Blocker 2) and adds Phase 5 (validation per Blocker 3).

## Where I'm uncertain (pass 2)

1. **The `auto_merge_enabled` row in the Blocker 2 table proceeds with outcome `in_progress`.** This preserves current behavior per the mika#1208 carve-out. But this means the verify call's result for `auto_merge_enabled` is observability-only (stored as fact, not gating). A peer reviewer might reasonably ask "what's the point of the verify call in this row?" Answer: it captures the post-state at advance time so a future operator forensics on a divergent milestone can correlate. Not a behavioral guard. Is this defensible scope, or should `auto_merge_enabled` be dropped entirely from the verify table (verify only runs on `merged`/`already_merged`)?

2. **Phase 5 step 2's fixture format.** I'm proposing `fixtures/m4_auto_merge_enabled_post_state.json` but haven't checked the existing fixture conventions in `crates/mika-agent/tests/eval/fixtures/` for whether they use raw JSON, JSON schema, or a Rust-side fixture builder. If conventions differ, the fixture file name and format adjust accordingly. Mention to implementer in the plan.

3. **Phase 5 step 4's grep regex.** The `gh` allowlist isn't exhaustively documented in one place; the implementer should derive it from `crates/mika-agent/src/tools/run_gh.rs`'s match arms. If the allowlist has changed recently (post #788), regenerate the regex. This is implementer-side, not architect-side, but flagging.

## Verdict requested

Verdict: GROOMED or Verdict: ITERATE.

If GROOMED, the operator will manually patch `second-pass (GROOMED)` into mika#789's body, apply the `ready` label, and the autonomous-loop impl path will pick up via the canonical dispatch (engine grooming-marker check at `dispatch-lib.sh:185-188` passes on the marker).

If ITERATE, please be specific about which of the three uncertainties above (or which Blocker resolution) needs revision.
