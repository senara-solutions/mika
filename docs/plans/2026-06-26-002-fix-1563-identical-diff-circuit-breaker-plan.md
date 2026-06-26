# Plan: fix(mika-dev): Circuit-Breaker for Identical-Diff QA Rejection Loop (mika issue#1563)

## Summary

Add a structural circuit breaker to the verdict handler that detects when the same PR diff is rejected by mika-qa N≥3 times and halts re-dispatch with operator escalation. The existing retry counters (`verdict_block_ac`, `verdict_block_ci`) count attempts generically but do not detect convergence failure — the case where claude-pilot produces the same code change on each retry.

---

## Problem Frame

PR mika#1556 was rejected by mika-qa 7 consecutive times over ~3 hours with identical diffs. Each cycle cost tokens for zero forward progress. No structural alarm fired; the loop was halted manually.

The autonomous loop has retry counters per verdict class but no **diff-equivalence check**. If claude-pilot cannot resolve the QA feedback and produces the same code, the counter increments but the dispatch cycle repeats. The founding doctrine (`feedback_prompt_enforcement_fragile`) requires this gate to be structural, not prompt-based.

---

## Requirements

- **R1.** On each `block[ac]` or `block[ci]` verdict that triggers a re-dispatch, capture the PR's diff fingerprint before dispatching.
- **R2.** Store the diff fingerprint history in task metadata alongside existing retry counters.
- **R3.** When the same diff fingerprint has been rejected ≥3 times (across any verdict class), halt re-dispatch and escalate to the operator instead.
- **R4.** Emit a structured log event and audit event when the circuit breaker fires.
- **R5.** The `hold[review]` verdict class, which passes through to the LLM for dispatch, should include the diff fingerprint in the pre-digest so the skill-level handler can also detect identical-diff loops.
- **R6.** The circuit breaker must not block when the diff has changed between rejections (forward progress is being made).
- **R7.** The circuit breaker check runs before the generic retry-count check — an identical-diff halt is more informative than a generic "retry limit reached."

---

## Key Technical Decisions

**KTD-1: Diff fingerprint = `headRefOid` (PR head commit SHA).**

Use `gh pr view <number> --repo <repo> --json headRefOid --jq '.headRefOid'` to get the PR's current head commit SHA. This is the simplest correct proxy for "has the code changed":
- If claude-pilot makes no new commits, SHA is identical → circuit breaker fires.
- If claude-pilot commits but the diff is semantically identical (e.g., whitespace-only), SHA differs → circuit breaker does NOT fire (false negative, acceptable for v1).
- If claude-pilot rebases and makes real changes, SHA differs → correct.

The alternative (hashing `gh pr diff` output) is more semantically correct but adds latency (~2-5s for large diffs vs ~0.5s for a JSON field query) and complexity (pipe to sha256sum). The head commit SHA is fast, cheap, and catches the exact failure mode from the founding incident (7 cycles with zero new commits). A future ticket can upgrade to diff-content hashing if the false-negative rate is meaningful.

**Rationale:** The founding incident (mika#1556) had identical `headRefOid` across all 7 rejections. The SHA check would have caught it at rejection 3.

**KTD-2: Threshold = 3 identical-diff rejections.**

Matches `BLOCK_AC_MAX_RETRIES`, `BLOCK_CI_MAX_RETRIES`, and `CIRCUIT_BREAKER_THRESHOLD` in `auto_pull.rs`. Three attempts give claude-pilot a fair chance while bounding waste.

**KTD-3: Cross-verdict tracking.**

The diff fingerprint history is shared across verdict classes. If `block[ac]` produces the same SHA twice and then `block[ci]` produces it a third time, the circuit breaker fires on the third. The convergence failure is about the code not changing, regardless of what QA is complaining about.

**KTD-4: Storage in task metadata JSON.**

Add a `verdict_diff_fingerprints` array to the task metadata:
```json
{
  "verdict_diff_fingerprints": [
    {"sha": "abc123", "verdict": "block[ac]", "at": "2026-06-26T01:00:00Z"},
    {"sha": "abc123", "verdict": "block[ci]", "at": "2026-06-26T01:30:00Z"}
  ]
}
```

This preserves the existing metadata-based retry counter pattern. The array stores all fingerprints (not just duplicates) so the operator can inspect the full rejection history.

**KTD-5: Gate placement — before generic retry-count check.**

The identical-diff check runs before `read_verdict_retry_count()`. When both would fire (identical diff AND retry limit reached), the identical-diff message is more actionable: "same diff rejected 3x" tells the operator the fix attempts aren't changing anything, while "retry limit reached" is ambiguous about whether progress was made.

**KTD-6: `hold[review]` gets enrichment, not structural gating.**

The `hold[review]` path currently passes through to the LLM (no structural dispatch in the verdict handler). Adding structural dispatch to `hold[review]` is out of scope. Instead, include the current diff fingerprint and the fingerprint history in the `hold[review]` pre-digest so the skill-level handler (`self-dev-webhook-qa`) has the data to make a prompt-level decision. This is acceptable because `hold[review]` already has `qa_retry_count >= 2` gating at the skill level.

---

## Scope Boundaries

### In scope

- Structural diff-fingerprint circuit breaker in `verdict_handler.rs` for `block[ac]` and `block[ci]`
- Diff fingerprint enrichment in `hold[review]` pre-digest
- Structured logging and audit events for circuit breaker firing
- Unit tests for the circuit breaker logic

### Deferred to Follow-Up Work

- Diff-content hashing (SHA of `gh pr diff` output) for semantic equivalence — upgrade path if `headRefOid` false-negative rate is meaningful
- Auto-recovery from convergence failure (re-grooming, architect escalation) — v1 halts + notifies only
- Cross-PR pattern detection ("5 PRs hit the same plan-AC mismatch") — separate alarm concern
- Convergence with mika#1384 Option A (structural dispatch) — both would benefit from a unified dispatcher-side gate; defer to that ticket. **Forward-compatibility posture:** This circuit breaker is additive and composable with a future unified dispatcher gate. It runs inside `handle_block_ac`/`handle_block_ci` (per-verdict-handler), while #1384's unified gate would run at the dispatcher level (before verdict routing). If #1384 lands, the per-handler circuit breaker becomes defense-in-depth behind the dispatcher gate — no refactoring required, only a decision on whether to keep both layers or collapse into the dispatcher gate (per review-guide.md § Orthogonality — changes should propagate minimally)

---

## Implementation Units

### U1. Add diff fingerprint fetch helper

**Goal:** Add a function to fetch the PR's head commit SHA from GitHub.

**Requirements:** R1

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/server/verdict_handler.rs`

**Approach:** Add an async function `fetch_pr_head_sha(pr_number: u64, repo: &str, token: &str) -> Result<String, String>` that calls `gh pr view <number> --repo <repo> --json headRefOid --jq '.headRefOid'` via the existing `spawn_and_collect` helper used by `run_gh_checks` and `run_gh_merge`. Return the SHA string on success, error string on failure. Wrap in a 15-second timeout (shorter than the 60s merge timeout — this is a single-field JSON query).

**Parameter sourcing:** `repo` is sourced from `event.repo` (the `PrReviewEvent` field, available in all verdict handler functions). `token` is sourced from the `github_token: Option<&str>` parameter threaded through `dispatch_verdict()` → individual handlers — the same parameter used by `run_gh_checks(event.pr_number, &event.repo, token)` at verdict_handler.rs:216 and `run_gh_merge()`. `fetch_pr_head_sha` requires a valid token (fail-open on `None` — skip the circuit breaker, log a warning, fall through to existing retry counters).

**Patterns to follow:** `run_gh_checks()` and `run_gh_merge()` in the same file for subprocess spawning pattern and timeout wrapping. `run_gh_checks(event.pr_number, &event.repo, token)` at line 216 is the exact calling-convention model.

**Test scenarios:**
- Test that `fetch_pr_head_sha` constructs the correct `gh` command arguments
- (Integration: tested via the circuit breaker unit tests in U3)

---

### U2. Add diff fingerprint metadata helpers

**Goal:** Add helpers to read/write the diff fingerprint history from/to task metadata JSON.

**Requirements:** R2

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/server/verdict_handler.rs`

**Approach:** Add two functions following the existing `read_verdict_retry_count` / `update_verdict_block_metadata` pattern:

- `read_diff_fingerprints(metadata: &Option<String>) -> Vec<DiffFingerprint>` — deserialize `verdict_diff_fingerprints` array from metadata JSON. Return empty vec if absent or malformed.
- `append_diff_fingerprint(db, task_id, metadata, sha, verdict_class, timestamp) -> Result<()>` — append a new entry to the `verdict_diff_fingerprints` array using the existing `task_metadata::merge_metadata` two-level shallow merge.

Define a small `DiffFingerprint` struct: `{ sha: String, verdict: String, at: String }`. The `at` field uses ISO 8601 UTC format (`"2026-06-26T01:00:00Z"`) — the same format used by all timestamp columns in the schema (see `crate::timestamp::now()`) and consistent with KTD-4's example JSON.

**Patterns to follow:** `read_verdict_retry_count()` for metadata deserialization pattern. `update_verdict_block_metadata()` for the metadata-write pattern with `merge_metadata`.

**Test scenarios:**
- `read_diff_fingerprints` returns empty vec when metadata is `None`
- `read_diff_fingerprints` returns empty vec when `verdict_diff_fingerprints` key is absent
- `read_diff_fingerprints` correctly deserializes a populated array
- `read_diff_fingerprints` returns empty vec on malformed JSON (graceful degradation)

---

### U3. Implement identical-diff circuit breaker in `handle_block_ac` and `handle_block_ci`

**Goal:** Add the identical-diff check to both block verdict handlers, gating re-dispatch when the same SHA has been rejected ≥3 times.

**Requirements:** R1, R3, R6, R7

**Dependencies:** U1, U2

**Files:**
- `crates/mika-agent/src/server/verdict_handler.rs`

**Approach:**

In both `handle_block_ac()` and `handle_block_ci()`, after the task lookup and active-callback-child check but **before** the existing `read_verdict_retry_count()` call:

1. Call `fetch_pr_head_sha()` to get the current PR head SHA. On failure, log a warning and skip the circuit breaker (fail-open — the existing retry counter is still active as fallback).
2. Call `read_diff_fingerprints()` to get the history.
3. Count occurrences of the current SHA in the history.
4. If count ≥ `IDENTICAL_DIFF_THRESHOLD` (const = 3):
   - Mark the task as `blocked` via `db.update_task_status()`
   - Write metadata with the circuit breaker reason
   - Log a structured `identical_diff_circuit_breaker` event
   - Write an `audit_events` row
   - Notify the operator via `send_notification()`
   - Return `VerdictAction::Handled` with a pre-digest instructing the LLM NOT to dispatch
5. If count < threshold: append the current fingerprint to the history via `append_diff_fingerprint()`, then fall through to the existing retry-count logic.

Add a constant: `const IDENTICAL_DIFF_THRESHOLD: u32 = 3;`

**Patterns to follow:** The existing retry-limit escalation path in `handle_block_ac()` (lines ~444-500) for the block + metadata + audit + notify + pre-digest pattern.

**Test scenarios:**
- Circuit breaker does NOT fire on first rejection (count=0, fresh SHA)
- Circuit breaker does NOT fire on second rejection with same SHA (count=1)
- Circuit breaker fires on third rejection with same SHA (count=2 in history + current = 3)
- Circuit breaker does NOT fire when SHA changes between rejections (forward progress)
- Circuit breaker fires across verdict classes (2x `block[ac]` + 1x `block[ci]` with same SHA)
- Circuit breaker is fail-open when `fetch_pr_head_sha` returns an error
- Circuit breaker fires before the generic retry-count check (ordering)
- Pre-digest contains "identical diff" and "circuit breaker" for operator clarity
- Pre-digest avoids completion-claim trigger words (parity with existing pre-digest tests)

---

### U4. Add diff fingerprint enrichment to `hold[review]` pre-digest

**Goal:** Include the current diff fingerprint and history in the `hold[review]` pre-digest so the skill-level handler has convergence data.

**Requirements:** R5

**Dependencies:** U1, U2

**Files:**
- `crates/mika-agent/src/server/verdict_handler.rs`

**Approach:**

In `handle_hold_review()`, after the task lookup:

1. Fetch the PR head SHA (fail-open on error — omit fingerprint data from pre-digest).
2. Read the existing fingerprint history.
3. Append the current fingerprint to metadata.
4. Include in the `format_hold_review_pre_digest()` output:
   - `Diff fingerprint: <sha>`
   - `Identical-diff rejection count: N/3`
   - If N ≥ threshold: an explicit warning line "IDENTICAL DIFF CIRCUIT BREAKER: same diff rejected {N}x — DO NOT dispatch run_claude_pilot."

This gives the LLM-side handler (`self-dev-webhook-qa`) the data to make an informed decision without requiring structural dispatch in the `hold[review]` path.

**Patterns to follow:** The existing `format_hold_review_pre_digest()` for pre-digest formatting. The `update_hold_metadata()` call site for metadata writes.

**Test scenarios:**
- `hold[review]` pre-digest includes diff fingerprint when SHA fetch succeeds
- `hold[review]` pre-digest omits fingerprint data gracefully when SHA fetch fails
- `hold[review]` pre-digest includes circuit breaker warning when identical-diff count ≥ threshold
- Pre-digest avoids completion-claim trigger words

---

### U5. Add structured logging and audit events

**Goal:** Emit observable events when the circuit breaker fires for post-deploy monitoring.

**Requirements:** R4

**Dependencies:** U3

**Files:**
- `crates/mika-agent/src/server/verdict_handler.rs`

**Approach:**

When the circuit breaker fires (U3 step 4):

1. **Structured log event** (`info!` level):
   - Event name: `identical_diff_circuit_breaker`
   - Fields: `pr_number`, `repo`, `task_id`, `head_sha`, `identical_count`, `verdict_class`, `trace_id`

2. **Audit event** (via `db.add_audit_event()`):
   - `tool_name`: `"verdict_handler"`
   - `target_key`: `"identical_diff_circuit_breaker"`
   - Fields: `pr_url`, `head_sha`, `identical_count`, `verdict_history` (JSON array of the matching fingerprints)

Add a **Signal** entry to CLAUDE.md § "Post-restart safety check (#757)". The existing signal inventory runs A through K (verified: Signal A extraction, Signal B budget, Signal C resolver, Signal D cost, Signal E tick drain, Signal F per-corpus fairness, Signal G extraction fairness, Signal H extraction tick drain, Signal I search index backfill, Signal J no-op wrapper detection, Signal K guard fabrication telemetry). The next available identifier is **L**:
- **Signal L — identical-diff circuit breaker.** `grep identical_diff_circuit_breaker server.log` — any hits indicate the circuit breaker fired. The `head_sha` and `identical_count` fields show the convergence failure details. Investigate the PR and plan for the root cause of the stuck fix loop.

**Patterns to follow:** Existing audit event writes in `handle_block_ac()` for the audit event shape. `domain_rebuild_invalidated_resolutions` for the structured log event naming convention.

**Test scenarios:**
- Verify the log event is emitted with correct fields when circuit breaker fires
- Verify the audit event is written with correct fields
- Verify no log/audit events when circuit breaker does NOT fire

---

## Open Questions

None — the approach is straightforward and follows established patterns. The `headRefOid` vs diff-content-hash tradeoff is documented in KTD-1 with a clear upgrade path.

---

## System-Wide Impact

- **Autonomous loop behavior:** PRs that trigger the circuit breaker will stop being re-dispatched and will be marked `blocked`. The operator must manually triage (amend the plan, rewrite the QA criteria, or close the PR).
- **Token cost:** Eliminates the unbounded token burn from convergence failures. Worst case is 3 fix attempts before halt (~$2-5), vs the previous unbounded loop (~$5-15 per 3 hours of operator inattention).
- **Backward compatibility:** The circuit breaker is additive. Tasks without `verdict_diff_fingerprints` in metadata are treated as having zero history (no circuit breaker firing). Existing retry counters continue to work independently.

---

## Sources & Research

- mika#1563 — founding ticket with incident analysis (7-cycle convergence failure on PR mika#1556)
- mika#1384 — sibling substrate class (structural dispatch vs prompt rule)
- `crates/mika-agent/src/server/verdict_handler.rs` — existing structural verdict handler with `BLOCK_AC_MAX_RETRIES = 3` and `BLOCK_CI_MAX_RETRIES = 3`
- `crates/mika-agent/src/auto_pull.rs` — circuit breaker precedent with `CIRCUIT_BREAKER_THRESHOLD = 3`
- `feedback_prompt_enforcement_fragile` — doctrine requiring structural gates over prompt rules at the loop substrate

---

## Revision history

- rev 2 (2026-06-26): addressed F1 by specifying `repo` and `token` parameter sourcing in U1 — `repo` from `event.repo`, `token` from the `github_token: Option<&str>` parameter threaded through `dispatch_verdict()`, matching the `run_gh_checks()` calling convention at verdict_handler.rs:216 (Unresolved-Decision Gate, mika#1244); addressed F2 by removing R5 from U3's requirements list — R5 (`hold[review]` enrichment) is implemented by U4, not U3 (review-guide.md § Single Responsibility); addressed F3 by verifying the existing signal inventory (A–K) in CLAUDE.md and confirming "Signal L" is the next available identifier (review-guide.md § DRY); addressed F4 by specifying `DiffFingerprint.at` as ISO 8601 UTC format consistent with `crate::timestamp::now()` and KTD-4's example JSON (review-guide.md § KISS); addressed F5 by adding a forward-compatibility note stating the per-handler circuit breaker is additive to and composable with #1384's unified dispatcher gate — no refactoring required on convergence (review-guide.md § Orthogonality).
