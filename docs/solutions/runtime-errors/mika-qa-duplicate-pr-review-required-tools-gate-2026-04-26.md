---
title: "mika-qa submits duplicate PR review when required-tools gate forces a retry turn"
date: 2026-04-26
category: runtime-errors
module: mika-agent
problem_type: bug
component: agent_loop_post_conditions
severity: medium
applies_when:
  - mika-qa reviews a PR and the qa-review skill's `[constraints] required_tools` includes a tool the LLM didn't call on first turn
  - Any post-condition gate forces a retry turn after a successful side-effect-emitting tool call has already run
  - Defenses-in-depth that are per-turn-scoped (AtomicBool reset between turns) collide with retry-induced new turns
tags:
  - mika-qa
  - pr-review
  - duplicate-submission
  - required-tools-gate
  - per-turn-vs-per-session
  - post-condition-chain
  - run_gh
---

# mika-qa submits duplicate PR review when required-tools gate forces a retry turn

## Symptom

PR #819 (mika#817's implementation) received **two `APPROVED` reviews** from `mika-platform-qa`, ~44 seconds apart, both reviewing the same commit (`a35c40a4`), both within a single mika-qa session (`d3fbe0e2-d54c-4867-b304-7c1252ab4b10`).

```
gh api repos/senara-solutions/mika/pulls/819/reviews --jq '.[] | {state, submitted_at, commit_id: .commit_id[:8]}'
{"state":"APPROVED","submitted_at":"2026-04-26T11:14:14Z","commit_id":"a35c40a4"}
{"state":"APPROVED","submitted_at":"2026-04-26T11:14:58Z","commit_id":"a35c40a4"}
```

GitHub recorded both reviews; both are visible in the PR's review tab. No operational harm in this specific case (both APPROVED, identical content modulo paraphrase, the verdict handler auto-merge is idempotent on duplicate APPROVEDs), but the duplicate emission is a real bug class that could cause harm in other contexts (duplicate notifications, double-counted metrics, audit-log noise, downstream consumers expecting one-review-per-session).

## What didn't work

Two existing defenses-in-depth, both designed to prevent exactly this class of failure, both **silently bypassed** by the actual chain.

### Defense 1: `ToolContext.pr_review_posted` AtomicBool

Per `crates/mika-agent/CLAUDE.md` § Post-Conditions: *"`run_gh` also tracks `ToolContext.pr_review_posted` (AtomicBool per turn) and rejects duplicate `pr review` calls with a structured `duplicate_pr_review` error."*

The bool is **per-turn**, not per-session. New turn → fresh `ToolContext` → bool resets. The second `run_gh pr review` ran in a new turn, the bool was false, no `duplicate_pr_review` error fired.

### Defense 2: PR review early-accept (#695)

Per the same CLAUDE.md section: *"`has_successful_pr_review()` checks if `all_tool_summaries` contains a successful `run_gh` call with `"pr"` and `"review"` in the input. When true, guards #4–#7 are skipped."*

This guard is labeled "3b" in the post-condition chain — it skips guards #4–#7 (completion-claim, fabricated-action-claim, intent-precondition, persistence) but **does not skip guard #3 (required-tools gate).** The actual trigger of the retry was guard #3, which is positioned *before* the early-accept logic.

## Solution / chain analysis

The qa-review skill at `mika/skills/bundled/qa-review/skill.toml` declares:

```toml
[constraints]
required_tools = ["qa_pr_view", "run_gh"]
```

Reproduction trace (session `d3fbe0e2-…`, all on the same `trace_id` `fa078a20-…`, single user message → 8 LLM calls → 2 `EndTurn` stop_reasons):

| Step | Time | Tool | Outcome |
|---|---|---|---|
| 0 | 11:14:38 | `run_gh pr diff 819 --name-only` | OK |
| 1 | 11:15:07 | **`run_gh pr review 819 --approve`** | OK — **review #1 submitted** |
| 1 | 11:15:12 | `store_fact` | OK |
| 2 (LLM step 3, EndTurn) | 11:15:19 | — | EndTurn emitted |
| **`Required-tools gate (#3) rejects EndTurn`** | — | — | `qa_pr_view` was never called in this turn — gate fires, forces retry |
| 4 | 11:15:22 | `qa_pr_view` + `run_gh pr diff 819 --name-only` | OK (parallel) — gate now satisfied |
| 5 | 11:15:51 | **`run_gh pr review 819 --approve`** | OK — **review #2 submitted** (new turn, fresh AtomicBool) |
| 6 | 11:15:55 | `store_fact` | OK |
| 7 (LLM step 7, EndTurn) | 11:16:03 | — | EndTurn — turn ends, second review final |

The chain:

1. **First turn.** mika-qa reads the qa-review skill prompt (which says "Fetch PR metadata using the `qa_pr_view` tool" at line 61) but uses `run_gh pr diff` instead, then jumps straight to `pr review --approve`. Skill-prompt drift; the model didn't follow the prompt's explicit ordering.
2. **EndTurn at LLM step 3.** Agent loop runs the post-condition chain on the assistant text response.
3. **Required-tools gate (#3) checks the union of required_tools across all keyword-matched skills (only qa-review here) against tools called in the turn.** `qa_pr_view` was missing. Gate rejects, forces a retry by re-prompting.
4. **Retry creates a new turn.** Fresh `ToolContext`, fresh `pr_review_posted` AtomicBool reset to false.
5. **Second turn.** Model now calls `qa_pr_view` (satisfying the gate) but ALSO re-runs the entire review flow — `pr diff`, `pr review --approve`, `store_fact`. The second `pr review` call passes the AtomicBool check (it's false, not pre-set from turn 1) and is submitted to GitHub.

The early-accept (#695) was designed for exactly this risk but is positioned in the chain as guard "3b" — it skips guards #4-#7 only. Guard #3 (required-tools) runs *before* the early-accept logic and isn't gated by it.

## Why this works (i.e., what the existing design got right)

The post-condition chain is correct in principle: required-tools enforcement matters; it catches skills that declare a constraint and then the model skips it. The `pr_review_posted` AtomicBool is correct in principle: per-turn dedup catches the within-turn case (model calls `pr review` twice in one turn). The early-accept is correct in principle: once the workflow's primary action completes, post-condition retries should not force re-emission of side-effects.

Each piece is sound. The hole is in the *positioning* — the early-accept skips guards #4-#7 but not #3, so the path through #3 leaks the case where the primary action already succeeded and the gate's retry will re-emit it.

## Why this works (the fix)

Two compatible fixes; either is sufficient on its own.

### Fix A — Session-scoped duplicate guard (preferred)

Move `pr_review_posted` from `ToolContext` (per-turn) to a session-scoped state. Once a successful `pr review` lands in this session for a given PR, `run_gh` rejects any subsequent `pr review` for the same PR with `duplicate_pr_review`.

**Why preferred:** prevents duplicates regardless of why the second turn fires (required-tools gate, max-steps continuation, manual user re-prompt, webhook re-delivery, anything). Defense at the tool-call layer is the right place — closest to where the side-effect actually emits.

**Implementation surface:** `crates/mika-agent/src/tools/run_gh.rs` (or wherever `pr_review_posted` lives today; likely a builtin handler). Promote the AtomicBool from `ToolContext` to a session-scoped table or in-memory map. Lifecycle: keyed by `(session_id, pr_url)`, lives until session ends.

**Concrete shape:**
```rust
// At session start: HashMap<String, HashSet<String>> // session_id -> set of PR URLs reviewed
// Before run_gh pr review fires: check if (session_id, pr_url) is in the map
// On success: insert (session_id, pr_url)
// On session end: drop the entry
```

### Fix B — Extend early-accept to skip guard #3 too

Update `has_successful_pr_review()` consumers to also short-circuit the required-tools gate, not just guards #4-#7. Same intent: once the workflow's primary action completed, no post-condition should force retry that would re-emit it.

**Smaller change:** ~5 lines in `agent.rs` post-condition chain. Less robust than Fix A — only handles the required-tools-gate trigger; doesn't protect against future post-condition additions or other retry sources.

### Both?

Fix A is the durable one. Fix B is the "while we're in here" tightening. Recommend landing both in the same PR — A as the primary defense, B as defense-in-depth at the post-condition layer.

## Prevention

The class of bug — *per-turn defenses + retry-induced new turns = silent bypass* — applies beyond pr review. Any AtomicBool or once-per-turn flag in `ToolContext` is potentially vulnerable. Audit at next reasonable opportunity:

```bash
grep -rn 'AtomicBool\|once_per_turn\|per_turn' crates/mika-agent/src/ | grep -v test
```

For each instance, ask: "if a post-condition retry forced a new turn, would this defense bypass?" If yes, consider session-scope.

The retry-creates-new-turn semantic is itself worth surfacing in the agent-loop CLAUDE.md docs — the `step` counter resets, the `ToolContext` is fresh, the trace_id stays the same. Per-trace might be a useful middle-ground scope for some defenses (broader than per-turn, narrower than per-session).

## Implementation

Both fixes landed in PR for mika#821 (`fix/mika-qa-duplicate-review-session-scope` branch):

- **Fix A:** `pr_reviews_posted: Arc<DashMap<String, HashSet<String>>>` on `AppState`, threaded through `ToolContext`. `run_gh` checks and populates the map. Entries evicted at 4 `end_session()` dispatcher callsites. `make_pr_dedup_key()` derives keys from `gh pr review` arguments.
- **Fix B:** `has_successful_pr_review()` check inserted inside the required-tools gate block (before the re-prompt), allowing EndTurn when a PR review already succeeded.
- **6 new tests:** session-scope dedup (cross-turn duplicate, different-PR-same-session, same-PR-different-session, required-tools-gate-retry-blocks), `make_pr_dedup_key` unit tests, early-accept-skips-guard-#3.

## Related

- senara-solutions/mika#811 / PR #813 — added mika-arch with the existing post-condition chain.
- senara-solutions/mika#695 — added the PR review early-accept guard (the one that doesn't help here because of its position).
- senara-solutions/mika#582 — added per-turn dedup of duplicate `tool_use` ids (similar shape but at a different layer; not affected by this bug).
- mika-qa session `d3fbe0e2-d54c-4867-b304-7c1252ab4b10` — the canonical reproduction.
- PR #819 reviews `4176850101` (review #1) and `4176850683` (review #2) — both APPROVED, same commit, ~44s apart.
- `mika/skills/bundled/qa-review/skill.toml` — the skill that declares `required_tools = ["qa_pr_view", "run_gh"]`.
- `mika/crates/mika-agent/CLAUDE.md` § Post-Conditions — documents the chain order; guard #3 is "Required-tools gate", #3b is the early-accept.
