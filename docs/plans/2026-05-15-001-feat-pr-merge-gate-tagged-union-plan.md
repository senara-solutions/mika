---
ticket: mika issue#794
type: feat
module: agent-core
tags: [tools, prompt-contract, webhook-qa, pr-merge-gate]
branch: feat/794/agent-pr-merge-with-gate-tagged-union
created: 2026-05-15
---

# Plan: pr_merge_with_gate tagged-union return + webhook-qa gate_errored branch

## Context

`pr_merge_with_gate` returns unstructured string errors when preflight checks fail. The webhook-qa
prompt has no branch for these errors, so the LLM improvises — leading to Rule 6 violations (mika#792:
auto-merge armed on a CONFLICTING PR). Two units must ship together to close the prompt-vs-tool
contract gap.

## Current State (from code read)

**Tool** (`crates/mika-agent/src/tools/pr_merge_with_gate.rs`, 812 lines):
- `MergeGateResult` enum: `Merged | AutoMergeEnabled | Blocked | AlreadyMerged`
- Errors return `ToolOutput::error(message)` — unstructured strings
- No preflight `mergeable` check; goes straight to `run_gh_checks()`
- `run_gh_merge()` failure handling: pattern-matches stderr for "already been merged", "draft",
  "merge conflict", "review required" — but these are string errors, not typed variants
- `classify_checks()` pure function: `HasFailures | HasPending | AllPassed`
- Serialized with `#[serde(tag = "action")]` — prompt branches on `action` field

**Prompt** (`skills/bundled/self-dev-webhook-qa/system_prompt.md`, 190 lines):
- Branches on `action`: `"merged"/"already_merged"`, `"auto_merge_enabled"`, `"blocked"`
- Lines 53-56: fallback for errors (no `action` field) — notify Vincent, stay in_progress
- But the fallback is just prose guidance; LLM can still improvise past it (as demonstrated)

**Test infra** (`crates/mika-agent/src/test_utils.rs`):
- `TestHarness` with `AsyncDatabase` + `ToolContext`
- Existing tests: `classify_checks()` (7 cases), validation, serialization (4 cases)
- No mock for `gh` subprocess; integration tests check tool definition + validation errors

## Implementation Plan

### Unit 1 — Structured return type for `pr_merge_with_gate`

**File: `crates/mika-agent/src/tools/pr_merge_with_gate.rs`**

#### Step 1: Add preflight GraphQL check + new types

Add a `run_gh_pr_view()` function that calls:
```
gh pr view <n> --repo <repo> --json mergeable,mergeStateStatus,statusCheckRollup,isDraft,state
```
in a single round-trip. Parse into a `PrPreflight` struct.

Add new enums:
```rust
pub enum BlockReason {
    MergeConflict,
    RequiredCheckFailed { failing: Vec<CheckInfo> },
    MissingApproval,
    PrClosed,
    Draft,
}

pub enum GateErrorKind {
    GhCliFailure { exit_code: i32 },
    NetworkError,
    AuthError,
    ParseError,
    Unknown,
}
```

Extend `MergeGateResult` with two new variants:
```rust
// Existing: Merged, AutoMergeEnabled, Blocked (rename to use BlockReason), AlreadyMerged
// New:
GateError { kind: GateErrorKind, detail: String },
```

Refactor `Blocked` to carry `BlockReason` instead of just `failing_checks`:
```rust
Blocked { reason: BlockReason, pr_url: String, detail: String },
```

**Serde tag:** Keep `#[serde(tag = "action")]` — new variants serialize as
`"action": "gate_errored"` and the existing `"blocked"` gains a `"reason"` field.

**Backward compatibility:** `"merged"`, `"auto_merge_enabled"`, `"already_merged"` unchanged.
`"blocked"` gains `reason` field but existing prompt branches on `action == "blocked"` still match —
the new `reason` field is additive.

#### Step 2: Rewrite execute() flow with preflight

New flow:
1. Validate inputs (unchanged)
2. **Preflight:** `run_gh_pr_view()` → `PrPreflight`
   - `state == "CLOSED"` → `Blocked { reason: PrClosed }`
   - `isDraft == true` → `Blocked { reason: Draft }`
   - `mergeable == "CONFLICTING"` or `mergeStateStatus == "DIRTY"` → `Blocked { reason: MergeConflict }`
3. **Check classification** (from preflight's `statusCheckRollup`, not separate `gh pr checks` call):
   - `HasFailures` → `Blocked { reason: RequiredCheckFailed { failing } }`
   - `HasPending` → attempt auto-merge → `AutoMergeEnabled`
   - `AllPassed` → attempt merge → `Merged`
4. **Merge attempt errors** → pattern-match into `GateError` or `Blocked` as appropriate
5. **Any `gh` CLI failure** → `GateError { kind: GhCliFailure { exit_code }, detail }`

Key change: the separate `run_gh_checks()` call is replaced by parsing `statusCheckRollup` from
the preflight GraphQL response. This eliminates the "no checks reported" error that triggered #792.

#### Step 3: Refactor error paths

Convert all existing `ToolOutput::error()` returns to the appropriate `MergeGateResult` variant.
Input validation errors (bad pr_number, invalid repo) stay as `ToolOutput::error()` — these are
caller bugs, not PR state. The distinction:
- Caller bugs (bad input) → `ToolOutput::error()` (tool didn't run)
- PR state issues → `MergeGateResult::Blocked` (tool ran, PR can't merge)
- Infrastructure failures → `MergeGateResult::GateError` (tool tried, infra failed)

#### Step 4: Tests for Unit 1

All tests in `pr_merge_with_gate.rs` (module `tests`):

1. **Serialization tests for new variants:** `Blocked { MergeConflict }`, `Blocked { RequiredCheckFailed }`,
   `Blocked { MissingApproval }`, `Blocked { Draft }`, `Blocked { PrClosed }`, `GateError` variants.
   Assert `action` field, `reason` field, and typed sub-fields serialize correctly.

2. **#792 regression test (unit):** Mock `gh pr view` to return
   `{ mergeable: "CONFLICTING", mergeStateStatus: "DIRTY", statusCheckRollup: [] }`.
   Assert: returns `Blocked { reason: MergeConflict }` — NOT an error string, NOT a `gh pr merge` attempt.

3. **Preflight classification tests:** Various `PrPreflight` combinations → expected `MergeGateResult`.
   - CONFLICTING + empty checks → MergeConflict (not "no checks")
   - MERGEABLE + failing checks → RequiredCheckFailed
   - MERGEABLE + pending checks → AllPassed not yet (flows to auto-merge path)
   - CLOSED → PrClosed
   - Draft → Draft

4. **GateError serialization:** Verify `action: "gate_errored"` with `kind` and `detail` fields.

### Unit 2 — Webhook-QA prompt update

**File: `skills/bundled/self-dev-webhook-qa/system_prompt.md`**

#### Step 5: Add structured branches for new variants

Update the `pr_merge_with_gate` response handling section (currently lines 32-58) to add:

**`blocked` with `reason` field branching:**
```
action = "blocked", reason = "merge_conflict"
  → notify Vincent: "PR #{number} has merge conflicts. Rebase needed: `gh pr checkout {n}; git rebase origin/main`"
  → do NOT call run_gh pr merge
  → do NOT call run_claude_pilot (conflict resolution is conversation-mode territory)
  → task status: in_progress

action = "blocked", reason = "required_check_failed"
  → (existing CI-fix flow) check ci_fix_count, dispatch run_claude_pilot or escalate

action = "blocked", reason = "missing_approval"
  → notify Vincent: "PR #{number} needs approval review"
  → task status: in_progress

action = "blocked", reason = "draft" | "pr_closed"
  → notify Vincent; escalate; unexpected in webhook-qa context
```

**`gate_errored` branch:**
```
action = "gate_errored"
  → notify Vincent with kind + detail from response
  → do NOT fall back to run_gh pr merge (explicit prohibition)
  → task status: in_progress
```

#### Step 6: Annotate Rule 6

Update Rule 6 (line ~114) to note that structural enforcement is now via the typed response
contract — Rule 6 in prose remains as documentation, runtime enforcement via policy table
is a follow-up.

#### Step 7: Tests for Unit 2

**#792 regression test (prompt-layer, integration test):**
Per ticket AC — stub `pr_merge_with_gate` to return `Blocked { reason: MergeConflict }`.
Assert: (a) one `send_message` with conflict context, (b) zero `run_gh pr merge` calls,
(c) task stays `in_progress`.

Mock at the **tool/prompt boundary** (stubbing `MergeGateResult` variants directly), not at
the `gh` CLI byte boundary. Document deviation if existing test infra uses CLI-byte stubs.

**`gate_errored` integration test:**
Stub tool to return `GateError { kind: GhCliFailure { exit_code: 1 }, detail: "..." }`.
Assert: `send_message` with kind+detail, zero `run_gh pr merge`, task stays `in_progress`.

**Fixture:** Use fixture seeded from trace `a45e31bc-401d-11f1-8227-de6e000e1099` with
provenance comment. Redaction check: only `@mika-platform-qa` (authorized identity).

## Commit Strategy

Single branch, two commits (ordered):
1. `feat(agent): pr_merge_with_gate tagged-union return with preflight check (#794)` — Unit 1
2. `feat(agent): webhook-qa gate_errored and blocked[merge_conflict] branches (#794)` — Unit 2

## Risk Assessment

**Low risk:** The `action` tag approach is backward-compatible. Existing `"merged"`,
`"auto_merge_enabled"`, `"already_merged"` are unchanged. `"blocked"` gains additive `reason` field.

**Medium risk:** Replacing `run_gh_checks()` with `statusCheckRollup` from GraphQL. Need to verify
that `statusCheckRollup` includes the same check data as `gh pr checks --required`. The `--required`
filter may not be available in the GraphQL response — may need to filter in code or use a different
GraphQL field.

**Mitigation:** Keep `run_gh_checks()` as fallback if `statusCheckRollup` doesn't provide required-check
filtering. Preflight still catches CONFLICTING state before reaching the checks path.

## Open Questions for Architect

1. Should `statusCheckRollup` fully replace `run_gh_checks()`, or should the preflight only guard
   the `mergeable` state while checks classification stays on the existing `gh pr checks --required` path?
   The ticket says "single GraphQL call" but `gh pr checks --required` has filtering that GraphQL's
   `statusCheckRollup` may not replicate exactly.

2. The existing `AlreadyMerged` variant (detected by pattern-matching "already been merged" in stderr)
   — should this move to preflight via `state == "MERGED"` check, or stay as a catch in the merge
   error path for race conditions?
