# Plan: fix(self-dev) verify-post-state pattern — mika#789

type: bug
ticket: mika#789
date: 2026-05-18
consolidates: mika#727 (M4 premature advance), mika#732 (memory hygiene)
blocker-cleared: mika#788 (run_gh allowlist — CLOSED)
already-landed: mika#797 (M5 milestone close via gh api PATCH — commit 205c9516)

## Summary

Two prompt edits to `skills/bundled/self-dev/system_prompt.md` plus one grep cross-check:

1. **M4 merge verification** — stop the milestone loop from advancing when a child's PR has auto-merge enabled but hasn't actually merged yet.
2. **M1 + M5 memory hygiene** — keep `current_priorities` core memory in sync with the active milestone.
3. **Grep cross-check** — verify every `gh <subcommand>` in the prompt is in the post-#788 allowlist.

M5 milestone close is already implemented (#797, commit `205c9516`) and requires no changes.

## Scope

- **In scope:** `skills/bundled/self-dev/system_prompt.md` — prompt-only changes at three sites (M1, M4, M5).
- **Out of scope:** Engine code changes. Full prompt sweep for other intent-vs-completion sites (#789 body is explicit about this). Provider-specific prompt variants (none exist for self-dev; feedback_no_provider_prompts).

## Prior art

- M5 step 3 (close PATCH + readback + branching) — landed via #797, commit `205c9516`. This plan does NOT touch M5 step 3.
- `self-dev-webhook-qa` already correctly handles `auto_merge_enabled` by keeping the task `in_progress` — the bug is in the M4 loop advancing before the merge webhook confirms completion.

## Phase 1 — M4 merge verification

### Problem

When `pr_merge_with_gate` returns `auto_merge_enabled` during milestone M4 serial execution, the webhook handler correctly sets the child to `in_progress` with note "QA passed, auto-merge enabled, awaiting CI." But the M4 loop has no explicit instruction to HOLD at this point — the LLM pattern-matches the QA pass acknowledgment as "child done" and dispatches the next ticket.

### Fix

Insert a new verification gate between M4 step 2 ("Execute per-issue flow") and M4 step 3 ("Check child outcome"). The gate makes the hold state explicit:

**After M4 step 2, before step 3, add:**

> **2.5. Merge verification gate (verify-post-state):**
>
> After the QA webhook handler processes a `pass` verdict for this child's PR:
>
> - If `pr_merge_with_gate` returned `"merged"` or `"already_merged"`: **verify before advancing.** Call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")` and confirm `state == "MERGED"`. Only then proceed to step 3 with outcome `completed`. If state is not MERGED (race condition), treat as HOLD.
> - If `pr_merge_with_gate` returned `"auto_merge_enabled"`: the PR is NOT yet merged. This is a **HOLD state** — the child task stays `in_progress`. Do NOT advance to step 3. Do NOT dispatch the next child. Wait for the `pull_request.closed(merged: true)` webhook to arrive (handled by `self-dev-webhook-qa` → "Webhook Entry Point — PR Closed"). When the webhook arrives and the task transitions to `completed`, **verify before re-entering M4:** call `run_gh(["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")` and treat only `state == "MERGED"` as merge success. Only then re-enter M4 step 3 for this child.
> - If `pr_merge_with_gate` returned `"blocked"` or `"gate_errored"`: the webhook handler already routed to the appropriate block/error path. M4 step 3 will see the child as `blocked`.
>
> **Literal verification command** (per committed decision — do NOT re-derive):
> ```
> run_gh(command=["pr", "view", "<num>", "--json", "state,mergedAt"], repo="senara-solutions/<repo>")
> ```
> Treat only `state == "MERGED"` as merge success. Any other state → HOLD.
>
> **Rule:** `auto_merge_enabled` is an intent signal, not a completion signal. The child stays in the serial execution slot until the merge webhook confirms actual merge AND `run_gh pr view` verifies `state == "MERGED"`. This prevents dispatching the next ticket against code not yet on main.
>
> **Incident:** mika#727 — KG milestone #14, PR #726 had auto-merge enabled but CI failed; next ticket #689 was dispatched against missing code.

### Verification

The M4 step 3 outcome table already handles `completed`, `blocked`, `failed` correctly. The new step 2.5 adds: (a) explicit HOLD instruction for the `auto_merge_enabled` intermediate state, (b) mandatory `run_gh pr view` verification call before any advance — belt-and-suspenders with the webhook flow.

## Phase 2 — M1 memory hygiene

### Problem

M1 creates the milestone parent task but never writes to `current_priorities` in core memory. The system prompt carries stale milestone data indefinitely.

### Fix

After M1's `create_task` call and `store_fact`, add:

> **Memory (current_priorities):** After creating the milestone parent task:
> ```
> update_core_memory(section="current_priorities", action="rewrite",
>   content="Milestone <repo> milestone#<n> (<milestone_title>): in_progress. <one-line purpose from milestone description>. Issues (dependency order): #X, #Y, #Z.",
>   reasoning="Milestone initialized — update current_priorities to reflect active work")
> ```

This goes after the `store_fact` call and before the "Notify Vincent" line in M1.

## Phase 3 — M5 memory hygiene

### Problem

M5 close-out never clears `current_priorities`. After a milestone completes, the system prompt still shows the old milestone as active.

### Fix

After M5 step 5 (the existing `store_fact` call), before step 6 (Notify Vincent), add:

> **Memory (current_priorities):** After recording milestone completion:
> ```
> update_core_memory(section="current_priorities", action="rewrite",
>   content="No active milestone. Last completed: <repo> milestone#<n> (<milestone_title>).",
>   reasoning="Milestone completed — clear current_priorities to prevent stale prompt state")
> ```

## Phase 4 — Grep cross-check (AC4)

Run grep on the final edited `skills/bundled/self-dev/system_prompt.md` for every `gh <subcommand>` invocation. Compare against the post-#788 `GH_ALLOWED_SUBCOMMANDS` list in the executor.

```bash
grep -oP 'gh\s+\K\w+' skills/bundled/self-dev/system_prompt.md | sort -u
```

Cross-reference against the allowlist. Document findings in the PR description. Any hallucinated subcommand (like the historical `gh milestone`) must be removed or replaced with `gh api`.

## Phase 5 — Ticket housekeeping (AC5)

- Verify #727 and #732 are already closed (confirmed: both CLOSED).
- Add cross-references in the PR description: "Consolidates #727 (closed) and #732 (closed) per #789 scope."

## Deliverables

| # | Deliverable | File | AC |
|---|-------------|------|----|
| 1 | M4 merge verification gate (step 2.5) | `skills/bundled/self-dev/system_prompt.md` | AC1 |
| 2 | M1 memory hygiene (`update_core_memory`) | `skills/bundled/self-dev/system_prompt.md` | AC3 |
| 3 | M5 memory hygiene (`update_core_memory`) | `skills/bundled/self-dev/system_prompt.md` | AC3 |
| 4 | Grep cross-check findings | PR description | AC4 |
| 5 | #727 + #732 cross-references | PR description | AC5 |

AC2 (M5 milestone close with literal command spec) is already satisfied by #797 (commit `205c9516`). AC6 (integration assertion) is optional per the issue body.

## Risks

- **Prompt length:** Three additions to an already 672-line prompt. All are small (5-15 lines each). No structural reorganization needed.
- **M4 hold semantics:** The hold depends on the `pull_request.closed` webhook arriving. If GitHub delays the webhook, the milestone loop is paused (correct behavior — better than premature advance).
- **No engine changes:** This is entirely prompt-level. The engine already has the correct tool responses (`auto_merge_enabled` variant) and webhook routing. The fix is making the LLM respect the hold state.
