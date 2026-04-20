---
title: "feat(server): structural check_suite.completed(failure) handler"
type: feat
status: active
date: 2026-04-20
issue: 594
deepened: 2026-04-20
---

# feat(server): structural check_suite.completed(failure) handler

## Overview

Add a structural handler that intercepts `check_suite.completed` failure/timed_out webhook events before the LLM turn, matches them to open PRs and existing work items, fetches failing-job context, and constructs a pre-digest that instructs the LLM to dispatch `run_claude_pilot` for an autonomous CI fix. This is the failure-side companion to `ci_success_handler.rs` (#571).

## Problem Frame

`check_suite.completed/failure` events route to mika-dev via the gateway but fall through to a regular LLM turn. The LLM burns the 5-minute engine wall-clock budget on diagnostic tool calls (`gh pr view`, `gh pr checks`, `gh run view --job`) without ever dispatching `run_claude_pilot` to actually fix the failure. Observed twice on PR #592 (mika#589) — both attempts hit the same timeout pattern.

The success-side handler (#571) proved that structural webhook interception is the correct pattern: deterministic state-machine transitions should happen in the engine layer, not via LLM improvisation (see `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`).

## Requirements Trace

- R1. New `ci_failure_handler.rs` intercepts `check_suite.completed/failure` and `timed_out` events before LLM turn
- R2. For PRs with existing work items, constructs pre-digest with structured failure context for LLM to dispatch `run_claude_pilot`
- R3. For PRs without existing work items (manual PRs), passes through (no-op)
- R4. For branches with no open PR, passes through (no-op)
- R5. Circuit breaker: `ci_fix_count >= 2` triggers escalation instead of dispatch
- R6. Loop prevention: `main`/`master` early skip + no-open-PR gate
- R7. Fix `CHECK_SUITE_RE` regex in `webhook_queue.rs` to match actual gateway format
- R8. Tests cover: parser (failure + timed_out + non-matching), pre-digest formatting, loop prevention, circuit breaker, no-PR case, no-work-item case
- R9. Handler increments `ci_fix_count` deterministically (not reliant on LLM)

## Scope Boundaries

- No direct dispatch of `run_claude_pilot` — the handler constructs a pre-digest; the LLM invokes the tool through the normal skill executor path
- No changes to the gateway event routing — `check_suite.completed(failure/timed_out)` already routes to `mika-dev`
- No auto-merge cancellation in the structural handler — left to the LLM via the self-dev-webhook-ci skill prompt (step 4)
- No changes to `validate_dispatch_readiness()` — existing guards are authoritative

### Deferred to Separate Tasks

- Resume mode for `run_claude_pilot` (reuse existing worktree/branch): skill-level concern in `mika-skills/self-dev/`, tracked separately
- Adding `cargo test` to the `/mika` pipeline pre-push: separate improvement, doesn't replace this handler

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/server/ci_success_handler.rs` — direct companion, mirror structure
- `crates/mika-agent/src/server/verdict_handler.rs` — same `VerdictAction` return type
- `crates/mika-agent/src/server/verdict.rs` — shared verdict types
- `crates/mika-agent/src/server/handlers.rs` lines 760-810 — where structural handlers are called in `run_agent_for_message()`
- `crates/mika-agent/src/server/webhook_queue.rs` lines 49-54 — broken `CHECK_SUITE_RE` regex
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — `run_gh_subprocess`, `run_gh_checks`, `classify_checks`, `GhCheck`
- `crates/mika-agent/src/db.rs` — `find_active_task_by_branch`, `find_active_task_by_pr_url`
- `crates/mika-agent/src/task_metadata.rs` — `merge_metadata()`
- `skills/bundled/self-dev-webhook-ci/system_prompt.md` — LLM-side CI failure rules

### Institutional Learnings

- `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — structural handlers > prompt rules for deterministic state transitions
- `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md` — companion handler pattern, pre-digest XML tag, completion-claim avoidance
- `docs/solutions/architecture-patterns/webhook-deferral-queue-callback-sequencing.md` — check_suite events use tier-2 (branch) correlation; regex must match actual gateway format
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — defense-in-depth matrix for dispatch loops
- `docs/solutions/logic-errors/webhook-fallthrough-dispatches-unrelated-backlog-work.md` — three-layer defense against LLM fallthrough dispatch

## Key Technical Decisions

- **Handler constructs pre-digest, not direct dispatch:** Matches `ci_success_handler` pattern. The handler gathers context and replaces the webhook text; the LLM reads it and invokes `run_claude_pilot` through the normal tool path. This keeps the handler simple (no exec skill machinery) while the dispatch-readiness guard in `executor.rs` remains the authoritative gate. (R2)

- **Handler increments `ci_fix_count` deterministically:** The self-dev-webhook-ci prompt instructs the LLM to increment `ci_fix_count` in step 5, but this is unreliable (compaction can drop it). The structural handler reads and increments `ci_fix_count` in task metadata before returning the pre-digest. The pre-digest tells the LLM NOT to re-increment. Makes the circuit breaker deterministic. (R5, R9)

- **Fix `CHECK_SUITE_RE` regex:** The current regex expects `Check suite (failure) on` but the gateway produces `Check suite failure on` (no parentheses). This means the deferral queue never correlates check_suite events. Fix it for both failure and success paths. (R7)

- **Cap failing job log fetches:** Max 3 failing jobs, 100 lines (tail) per job, 60s total timeout for all log fetches. Prevents agent-lock starvation and context-window blowup. (R2)

- **Global dispatch check in pre-digest:** If another task has an active callback (global single-session guard), the pre-digest includes this information so the LLM knows dispatch will be rejected. Still passes the context — the LLM can notify the user or defer. (R2)

## Open Questions

### Resolved During Planning

- **Who increments `ci_fix_count`?** Handler increments deterministically. Pre-digest tells LLM not to re-increment. If LLM double-increments due to compaction, worst case is premature escalation (safe failure mode).
- **Should `timed_out` follow the same path as `failure`?** Yes — the `run_claude_pilot` session reads the logs and determines actionability. Infrastructure timeouts are rare; code-level timeouts (test hangs) are fixable.
- **Which task lookup: branch or PR URL?** After finding the PR, construct PR URL and try `find_active_task_by_pr_url` first, then fall back to `find_active_task_by_branch`. Matches `ci_success_handler` pattern.

### Deferred to Implementation

- Exact error message formatting for `gh run view --log-failed` parsing — depends on actual output structure
- Whether to fetch logs in parallel with `tokio::join!` or sequentially — depends on observed latency

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
check_suite.completed(failure|timed_out) event arrives at handle_message
  │
  ▼
parse_check_suite_failure(text) ──── None ──→ Passthrough (not our event)
  │ Some(repo, branch, conclusion)
  ▼
branch == main|master? ──── yes ──→ Passthrough (loop prevention)
  │ no
  ▼
find_open_pr(repo, branch) ──── None ──→ Passthrough (no PR)
  │ Some(pr)
  ▼
find_active_task(pr_url, branch) ──── None ──→ Passthrough (manual PR)
  │ Some(task)
  ▼
task.status != in_progress? ──── yes ──→ Passthrough (wrong status)
  │ no
  ▼
has_active_callback_child? ──── yes ──→ Passthrough + enrichment (fix in-flight)
  │ no
  ▼
ci_fix_count >= 2? ──── yes ──→ Handled (escalation pre-digest)
  │ no
  ▼
increment ci_fix_count in metadata
  │
  ▼
fetch failing checks + job logs (max 3 jobs, 100 lines each)
  │
  ▼
check global dispatch guard
  │
  ▼
construct dispatch pre-digest with failure context
  │
  ▼
Handled { pre_digest } → LLM reads context, invokes run_claude_pilot
```

## Implementation Units

- [x] **Unit 1: Fix `CHECK_SUITE_RE` regex in webhook_queue.rs**

**Goal:** Fix the check_suite regex to match the actual gateway format so deferral queue correlation works for all check_suite events.

**Requirements:** R7

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/server/webhook_queue.rs`

**Approach:**
- Change regex from `\[GitHub\] Check suite \(([^)]+)\) on (\S+) \(branch: ([^)]+)\)` to `\[GitHub\] Check suite (\S+) on (\S+) \(branch: ([^)]+)\)` — removes the parentheses around the conclusion capture group
- Update the doc comment to match the actual gateway format

**Patterns to follow:**
- `parse_check_suite_success()` in `ci_success_handler.rs` — uses string prefix matching, not regex

**Test scenarios:**
- Happy path: `[GitHub] Check suite failure on senara-solutions/mika (branch: feat/test)` → captures conclusion=`failure`, repo=`senara-solutions/mika`, branch=`feat/test`
- Happy path: `[GitHub] Check suite success on org/repo (branch: main)` → captures conclusion=`success`
- Happy path: `[GitHub] Check suite timed_out on org/repo (branch: feat/x)` → captures conclusion=`timed_out`
- Edge case: text with trailing newlines → still captures from first line
- Error path: `[GitHub] PR review ...` → no match (returns None from check_suite path)

**Verification:**
- Existing `webhook_queue` tests pass with updated regex
- New test cases for actual gateway format pass

---

- [x] **Unit 2: Core `ci_failure_handler.rs` — parser, task lookup, metadata, pre-digest**

**Goal:** Implement the structural handler that parses CI failure events, matches them to PRs and work items, applies circuit-breaker logic, fetches failure context, and constructs the pre-digest message.

**Requirements:** R1, R2, R3, R4, R5, R6, R9

**Dependencies:** Unit 1

**Files:**
- Create: `crates/mika-agent/src/server/ci_failure_handler.rs`
- Modify: `crates/mika-agent/src/server/mod.rs` (add `pub mod ci_failure_handler;`)

**Approach:**
- Mirror `ci_success_handler.rs` structure: `CheckSuiteFailureEvent` struct, `parse_check_suite_failure()` function, `try_handle_ci_failure()` async entry point
- Parser handles both `failure` and `timed_out` conclusions from the gateway format `[GitHub] Check suite {conclusion} on {repo} (branch: {branch})`
- Early returns: non-matching event → Passthrough, main/master branch → Passthrough, no open PR → Passthrough, no matching task → Passthrough, task not in_progress → Passthrough, active callback child → Passthrough with enrichment
- Circuit breaker: read `ci_fix_count` from task metadata JSON, if >= 2 return escalation pre-digest
- Increment `ci_fix_count` via `merge_metadata()` + `db.update_task_metadata()` before constructing dispatch pre-digest
- Fetch failing checks via `run_gh_checks()` + `classify_checks()`, then for up to 3 failing jobs fetch log excerpts via `run_gh_subprocess` with `gh run view --job <id> --log-failed`
- Truncate each job log to last 100 lines
- Check global dispatch guard via `db.has_active_callback_tasks_excluding()` — include result in pre-digest context
- Construct pre-digest in `<ci_failure_handler>` XML tags, avoiding completion-claim trigger words
- Log audit event `ci_failure_handled`
- Send notification via `message_sender`
- Reuse `find_open_pr()` from `ci_success_handler` — make it `pub(crate)` or extract to shared module
- Reuse `VerdictAction` from `super::verdict_handler`
- All subprocess calls wrapped in 60s timeouts

**Patterns to follow:**
- `ci_success_handler.rs` — overall structure, error handling, timeout wrapping, pre-digest formatting
- `verdict_handler.rs` — VerdictAction return type
- `pr_merge_with_gate.rs` — `run_gh_subprocess`, `run_gh_checks`, `classify_checks`

**Test scenarios:**
- Happy path: `[GitHub] Check suite failure on senara-solutions/mika (branch: feat/test)` → parsed correctly with repo + branch + conclusion
- Happy path: `[GitHub] Check suite timed_out on org/repo (branch: fix/bug)` → parsed correctly
- Edge case: trailing context after the first line → only first line parsed
- Error path: `[GitHub] Check suite success on ...` → returns None (not a failure)
- Error path: `[GitHub] PR review ...` → returns None (different event type)
- Error path: empty string → returns None
- Error path: malformed (missing branch part) → returns None
- Happy path: dispatch pre-digest avoids completion-claim trigger words (regex check against `COMPLETION_CLAIM_RE`)
- Happy path: escalation pre-digest avoids completion-claim trigger words
- Happy path: in-flight pre-digest avoids completion-claim trigger words
- Happy path: pre-digest contains `<ci_failure_handler>` and `</ci_failure_handler>` tags
- Happy path: dispatch pre-digest contains task ID and "Do NOT re-increment ci_fix_count" instruction
- Happy path: escalation pre-digest contains escalation instruction

**Verification:**
- `cargo test -p mika-agent` passes including all new unit tests
- `cargo clippy -p mika-agent` clean

---

- [x] **Unit 3: Register handler in `handlers.rs`**

**Goal:** Wire the CI failure handler into the server's message processing pipeline alongside existing structural handlers.

**Requirements:** R1

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/server/handlers.rs`

**Approach:**
- Add call to `ci_failure_handler::try_handle_ci_failure()` in the `if req.channel == "github"` block in `run_agent_for_message()`, after the existing `ci_success_handler` call
- Follow the same `match` pattern for `VerdictAction::Handled` / `Passthrough` result handling
- Use the same `verdict_github_token` already resolved for the verdict and CI success handlers
- Order-independent — the failure handler self-selects on `failure`/`timed_out` conclusions, the success handler self-selects on `success`

**Patterns to follow:**
- Lines 788-810 of `handlers.rs` — existing CI success handler registration

**Test scenarios:**
- Integration: handler registration is compile-time verified (module exists, function signature matches)

**Test expectation: none** — registration is a wiring change with no behavioral logic; correctness is verified by Unit 2's parser tests ensuring only failure/timed_out events match.

**Verification:**
- `cargo build` succeeds
- The handler is called in the correct position in the pipeline

---

- [x] **Unit 4: Make `find_open_pr` reusable across handlers**

**Goal:** Extract `find_open_pr()` from `ci_success_handler.rs` into a shared location so both success and failure handlers can use it without duplication.

**Requirements:** R2

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/src/server/ci_success_handler.rs` (change `find_open_pr` and `PrInfo` visibility to `pub(crate)`)

**Approach:**
- Make `find_open_pr()` and `PrInfo` struct `pub(crate)` in `ci_success_handler.rs`
- The failure handler imports them via `super::ci_success_handler::{find_open_pr, PrInfo}`
- Minimal change — avoids creating a new shared module for just one helper

**Patterns to follow:**
- `pr_merge_with_gate.rs` uses `pub(crate)` for helpers shared across handlers

**Test expectation: none** — visibility change only; existing `ci_success_handler` tests validate the function.

**Verification:**
- `cargo build` succeeds with the import in `ci_failure_handler.rs`

---

- [x] **Unit 5: Update `crates/mika-agent/CLAUDE.md` with handler documentation**

**Goal:** Document the new CI failure handler in the crate-level CLAUDE.md for future development context.

**Requirements:** Documentation

**Dependencies:** Units 1-3

**Files:**
- Modify: `crates/mika-agent/CLAUDE.md`

**Approach:**
- Add a `### Structural CI Failure Handler` section after the existing `### Structural CI Success Handler` section
- Document: purpose, companion relationship to success handler, pre-digest pattern, circuit breaker logic, `CHECK_SUITE_RE` fix, issue reference

**Patterns to follow:**
- Existing `### Structural CI Success Handler` section format

**Test expectation: none** — documentation only.

**Verification:**
- Documentation accurately reflects the implementation

## System-Wide Impact

- **Interaction graph:** The handler runs in `run_agent_for_message()` before the LLM turn. Its `VerdictAction::Handled` replaces `req.text` with the pre-digest. The LLM then reads the pre-digest and invokes `run_claude_pilot` through the normal skill executor path, which applies `validate_dispatch_readiness()`. The webhook deferral queue (`webhook_queue.rs`) correlates check_suite events by branch — the regex fix in Unit 1 enables this for the first time.
- **Error propagation:** All `gh` subprocess failures return `Passthrough` (fall through to LLM). The LLM turn is the fallback — same as today, but now only triggered when the structural handler cannot act.
- **State lifecycle risks:** `ci_fix_count` increment happens before the pre-digest is returned. If the handler increments but the LLM turn fails to dispatch (global guard, timeout), the count is incremented without a fix attempt. Acceptable: premature escalation is the safe failure mode (alerts user sooner).
- **API surface parity:** No new HTTP endpoints. No dashboard changes needed — the handler's audit events (`ci_failure_handled`) appear in the existing unified timeline.
- **Integration coverage:** The full path (webhook → handler → pre-digest → LLM → dispatch) is not unit-testable due to subprocess dependencies. The self-dev-webhook-ci skill prompt's step 5 covers the LLM side. The structural handler's unit tests cover parsing + formatting + metadata logic.
- **Unchanged invariants:** `ci_success_handler` continues to handle success events independently. `verdict_handler` continues to handle PR review events. Handler call order remains irrelevant — each self-selects on event type.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| `ci_fix_count` increment without dispatch (LLM fails to call `run_claude_pilot`) | Safe failure mode: premature escalation alerts user. Max impact: 1 wasted count. |
| Agent lock held during log fetches (up to 3 jobs × 60s) | Cap at 3 jobs + 60s total timeout. Worst case: 60s lock hold, matching existing handler patterns. |
| `CHECK_SUITE_RE` fix may cause previously-uncorrelated webhooks to be deferred | Correct behavior — deferral prevents race conditions with in-flight callbacks. |
| Pre-digest + skill prompt double-increment of `ci_fix_count` | Pre-digest explicitly instructs "Do NOT re-increment." If ignored, worst case is premature escalation. |

## Sources & References

- Related issues: #594 (this), #571 (success handler, closed), #583 (webhook fallthrough)
- Related code: `crates/mika-agent/src/server/ci_success_handler.rs`, `crates/mika-agent/src/server/handlers.rs`
- Solution docs: `docs/solutions/architecture-patterns/engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md`
