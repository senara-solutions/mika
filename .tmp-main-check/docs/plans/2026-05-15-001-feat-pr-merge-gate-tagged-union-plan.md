---
ticket: mika issue#794
type: feat
module: agent-core
tags: [tools, prompt-contract, webhook-qa, pr-merge-gate]
branch: feat/794/agent-pr-merge-with-gate-tagged-union
created: 2026-05-15
prior_art: mika#524 (structural-verdict-handler pattern)
---

# Plan: pr_merge_with_gate tagged-union return + webhook-qa gate_errored branch

## Context

`pr_merge_with_gate` returns unstructured string errors when preflight checks fail. The webhook-qa
prompt has no branch for these errors, so the LLM improvises — leading to Rule 6 violations (mika#792:
auto-merge armed on a CONFLICTING PR). Two units must ship together to close the prompt-vs-tool
contract gap.

**Prior art:** This plan extends the structural verdict handler pattern established in mika#524
(`docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md`) —
typed tool output → exhaustive prompt branching → structural enforcement of safety invariants.

## Phase 0 Pin

**Commit:** `8731102dff2e06ad5ff51f6efaa81728230052ac` (main at plan time)

### Pin 1 — `MergeGateResult` enum (lines 226-237)

```rust
/// Structured result returned by the tool as JSON.
#[derive(Debug, Serialize)]
#[serde(tag = "action")]
enum MergeGateResult {
    #[serde(rename = "merged")]
    Merged,
    #[serde(rename = "auto_merge_enabled")]
    AutoMergeEnabled { pending_checks: Vec<CheckInfo> },
    #[serde(rename = "blocked")]
    Blocked { failing_checks: Vec<CheckInfo> },
    #[serde(rename = "already_merged")]
    AlreadyMerged,
}
```

### Pin 2 — `ToolOutput::error()` call sites being converted (lines 130-133, 180, 202-214)

**Line 130-133 (checks fetch failure — the #792 trigger):**
```rust
Err(e) => {
    return Ok(ToolOutput::error(format!(
        "Failed to fetch check statuses: {e}"
    )));
}
```

**Lines 202-214 (merge-attempt stderr pattern matching):**
```rust
} else if output_lower.contains("draft") {
    Ok(ToolOutput::error(
        "PR is a draft — convert to ready before merging",
    ))
} else if output_lower.contains("merge conflict")
    || output_lower.contains("not mergeable")
{
    Ok(ToolOutput::error(
        "Merge conflicts — resolve before merging",
    ))
} else if output_lower.contains("review") && output_lower.contains("required") {
    Ok(ToolOutput::error("Required reviews not met"))
} else {
    Ok(ToolOutput::error(format!("Merge failed: {output}")))
}
```

**Line 180 (auto-merge failure):**
```rust
Err(e) => Ok(ToolOutput::error(format!("Auto-merge failed: {e}"))),
```

### Pin 3 — `run_gh_checks()` function signature (lines 324-334)

```rust
pub(crate) async fn run_gh_checks(
    pr_number: u64,
    repo: &str,
    token: &str,
) -> Result<Vec<GhCheck>, String> {
    let pr_str = pr_number.to_string();
    let args = vec![
        "pr",
        "checks",
        &pr_str,
        "--repo",
```

### Pin 4 — `execute()` flow (lines 125-134, the pre-classification path)

```rust
// -- Step 1: Fetch required check statuses --
let checks_result = run_gh_checks(pr_number, repo, token).await;
let checks = match checks_result {
    Ok(c) => c,
    Err(e) => {
        return Ok(ToolOutput::error(format!(
            "Failed to fetch check statuses: {e}"
        )));
    }
};
```

### Pin 5 — Webhook-QA prompt merge-result branching (lines 32-56)

**File:** `skills/bundled/self-dev-webhook-qa/system_prompt.md` (confirmed exists — mika#1106
is CLOSED but the decomposition shipped through other means; the file is live on main)

```markdown
   1. Call `pr_merge_with_gate({"pr_number": <number>, "repo": "<owner/repo>"})` ...
   2. Branch on the `action` field in the response:

      **`"merged"` or `"already_merged"`** — PR is merged:
      ...
      **`"auto_merge_enabled"`** — CI checks pending, auto-merge activated:
      ...
      **`"blocked"`** — Required CI checks failing:
      ...
      **Error (no `action` field)** — Tool returned a plain string instead of a JSON object with `action`:
      - Correlate to task (Step 4).
      - Notify Vincent via `send_message`: "Merge failed for {repo}#{number}: {error message}. {PR URL}"
      - Proceed to Step 5 with `in_progress` (do not block — Vincent may resolve manually).
```

## Implementation Plan

### Unit 1 — Structured return type for `pr_merge_with_gate`

**File: `crates/mika-agent/src/tools/pr_merge_with_gate.rs`**

#### Step 1: Add preflight `gh pr view` call + new types

Add `run_gh_pr_view()` function:
```
gh pr view <n> --repo <repo> --json mergeable,mergeStateStatus,isDraft,state
```
Parse into a `PrPreflight` struct. **Critically: this call does NOT include `statusCheckRollup`.**
Per mika-arch NF1, the preflight handles state checks only (CONFLICTING, CLOSED, DRAFT). The
check classification path stays on the existing `run_gh_checks()` with `--required` filtering.

**Design decision (resolves open question 1):** Keep `run_gh_checks()` for the checks-classification
path. `gh pr checks --required` has server-side filtering for required vs optional checks that
`statusCheckRollup` does not replicate. The preflight detects "cannot merge regardless of checks";
`run_gh_checks()` detects "checks haven't passed yet." Different questions, different tools.

Add new enums:
```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "reason")]
pub(crate) enum BlockReason {
    #[serde(rename = "merge_conflict")]
    MergeConflict,
    #[serde(rename = "required_check_failed")]
    RequiredCheckFailed { failing_checks: Vec<CheckInfo> },
    #[serde(rename = "missing_approval")]
    MissingApproval,
    #[serde(rename = "pr_closed")]
    PrClosed,
    #[serde(rename = "draft")]
    Draft,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub(crate) enum GateErrorKind {
    #[serde(rename = "gh_cli_failure")]
    GhCliFailure { exit_code: i32 },
    #[serde(rename = "network_error")]
    NetworkError,
    #[serde(rename = "parse_error")]
    ParseError,
    #[serde(rename = "unknown")]
    Unknown,
}
```

#### Step 2: Extend `MergeGateResult` — additive changes only (addresses F3)

**F3 resolution — backward-compatible `Blocked` variant change:**

The `Blocked` variant **retains** `failing_checks` and **gains** an additional `reason` field.
This is truly additive — existing prompt branches on `action == "blocked"` with `failing_checks`
still work unchanged.

```rust
#[derive(Debug, Serialize)]
#[serde(tag = "action")]
enum MergeGateResult {
    #[serde(rename = "merged")]
    Merged,
    #[serde(rename = "auto_merge_enabled")]
    AutoMergeEnabled { pending_checks: Vec<CheckInfo> },
    #[serde(rename = "blocked")]
    Blocked {
        reason: BlockReason,
        failing_checks: Vec<CheckInfo>,  // RETAINED — backward compat
        detail: String,
    },
    #[serde(rename = "already_merged")]
    AlreadyMerged,
    #[serde(rename = "gate_errored")]
    GateError {
        kind: GateErrorKind,
        detail: String,
    },
}
```

**Exact serde output shapes for each variant:**

`Merged`:
```json
{ "action": "merged" }
```

`AutoMergeEnabled` (unchanged):
```json
{ "action": "auto_merge_enabled", "pending_checks": [...] }
```

`AlreadyMerged` (unchanged):
```json
{ "action": "already_merged" }
```

`Blocked` with `RequiredCheckFailed` (backward compatible — retains `failing_checks`):
```json
{
  "action": "blocked",
  "reason": { "reason": "required_check_failed", "failing_checks": [...] },
  "failing_checks": [...],
  "detail": "1 required check failed"
}
```
Note: `failing_checks` appears at both top-level (backward compat) and inside `reason`
(for structured branching). The top-level copy is the backward-compat surface; the `reason`
copy is the forward surface.

`Blocked` with `MergeConflict` (new):
```json
{
  "action": "blocked",
  "reason": { "reason": "merge_conflict" },
  "failing_checks": [],
  "detail": "PR has merge conflicts — rebase needed"
}
```
Note: `failing_checks` is empty (no checks involved) but present for schema consistency.

`Blocked` with `MissingApproval`:
```json
{
  "action": "blocked",
  "reason": { "reason": "missing_approval" },
  "failing_checks": [],
  "detail": "Required reviews not met"
}
```

`GateError` (new):
```json
{
  "action": "gate_errored",
  "kind": { "kind": "gh_cli_failure", "exit_code": 1 },
  "detail": "gh exit_code 1: ..."
}
```

#### Step 3: Rewrite `execute()` flow with preflight

New flow:
1. Validate inputs (unchanged — lines 82-112)
2. Require GitHub token (unchanged — lines 114-123)
3. **NEW: Preflight** `run_gh_pr_view()` → `PrPreflight`
   - `state == "CLOSED"` → `Blocked { reason: PrClosed, failing_checks: vec![], detail }`
   - `state == "MERGED"` → `AlreadyMerged` (preflight primary — per NF2)
   - `isDraft == true` → `Blocked { reason: Draft, failing_checks: vec![], detail }`
   - `mergeable == "CONFLICTING"` or `mergeStateStatus == "DIRTY"` →
     `Blocked { reason: MergeConflict, failing_checks: vec![], detail }`
   - Preflight `gh` failure → `GateError { kind: GhCliFailure, detail }`
4. **Check classification** (existing `run_gh_checks()` with `--required` — kept per NF1):
   - `HasFailures` → `Blocked { reason: RequiredCheckFailed { failing }, failing_checks: failing_copy, detail }`
   - `HasPending` → attempt auto-merge → `AutoMergeEnabled`
   - `AllPassed` → attempt merge → `Merged`
   - Checks fetch failure → `GateError { kind: GhCliFailure, detail }` (was `ToolOutput::error`)
5. **Merge attempt errors** → pattern-match into `GateError` (was `ToolOutput::error`):
   - "already been merged" → `AlreadyMerged` (stderr fallback for races — per NF2)
   - "draft" → `Blocked { reason: Draft }`
   - "merge conflict" / "not mergeable" → `Blocked { reason: MergeConflict }`
   - "review required" → `Blocked { reason: MissingApproval }`
   - Other → `GateError { kind: Unknown, detail }`
   - Auto-merge failure → `GateError { kind: GhCliFailure, detail }`

**AlreadyMerged detection (resolves open question 2):** Both preflight (`state == "MERGED"`)
and stderr fallback ("already been merged") coexist. Preflight is primary (catches before
merge attempt); stderr is race-condition fallback.

**Error path refactor — kept distinctions:**
- Caller bugs (bad input) → `ToolOutput::error()` (tool didn't run). Lines 87, 95, 98, 106, 118.
- PR state issues → `MergeGateResult::Blocked` (tool ran, PR can't merge)
- Infrastructure failures → `MergeGateResult::GateError` (tool tried, infra failed)

#### Step 4: Tests for Unit 1

All tests in `pr_merge_with_gate.rs` (module `tests`):

1. **Serialization tests for new variants** (extend existing 4 tests at lines 660-705):
   - `serialize_blocked_merge_conflict`: `Blocked { reason: MergeConflict }` → verify
     `action == "blocked"`, `reason.reason == "merge_conflict"`, `failing_checks == []`
   - `serialize_blocked_missing_approval`: verify `reason.reason == "missing_approval"`
   - `serialize_blocked_draft`: verify `reason.reason == "draft"`
   - `serialize_gate_error`: verify `action == "gate_errored"`, `kind.kind == "gh_cli_failure"`
   - Update existing `serialize_blocked_result` to include `reason: RequiredCheckFailed`

2. **#792 regression test (unit):** Create a `PrPreflight` with
   `{ mergeable: "CONFLICTING", mergeStateStatus: "DIRTY", isDraft: false, state: "OPEN" }`.
   Call the preflight classification function.
   Assert: returns `Blocked { reason: MergeConflict }` — NOT an error string.
   Assert: no `gh pr merge` subprocess spawned (test at the pure-function layer).

3. **Preflight classification tests** (new pure function `classify_preflight`):
   - CONFLICTING + OPEN → MergeConflict
   - DIRTY + OPEN → MergeConflict
   - MERGEABLE + OPEN → pass-through to checks
   - CLOSED → PrClosed
   - MERGED → AlreadyMerged
   - Draft → Draft

4. **Backward compatibility test:** Verify `serialize_blocked_result` (existing test, updated)
   still has `action == "blocked"` and `failing_checks` array at top level.

### Unit 2 — Webhook-QA prompt update

**File: `skills/bundled/self-dev-webhook-qa/system_prompt.md`** (confirmed live on main)

#### Step 5: Add structured branches for new variants

Update the `pr_merge_with_gate` response handling section (lines 32-56). Replace the existing
`"blocked"` branch and `"Error"` branch with structured variants:

```markdown
      **`"blocked"`** — PR cannot merge. Branch on the `reason` field:

        **`reason.reason = "required_check_failed"`** — Required CI checks failing:
        - (existing CI-fix flow, unchanged) Check `ci_fix_count`, dispatch or escalate.
        - `failing_checks` array contains check names for claude-pilot prompt.

        **`reason.reason = "merge_conflict"`** — PR has merge conflicts:
        - Notify Vincent: "{repo}#{number} has merge conflicts. Rebase needed."
        - Do NOT call `run_gh pr merge`.
        - Do NOT call `run_claude_pilot` (conflict resolution is conversation-mode territory).
        - Task status: `in_progress`.

        **`reason.reason = "missing_approval"`** — PR needs review approval:
        - Notify Vincent: "{repo}#{number} needs approval review. {PR URL}"
        - Task status: `in_progress`.

        **`reason.reason = "draft"` or `reason.reason = "pr_closed"`** — Unexpected in webhook-qa:
        - Notify Vincent with context. Escalate.

        **Unrecognized `reason` value** — Future variant not yet handled:
        - Notify Vincent: "Unrecognized block reason: {reason}. {PR URL}"
        - Do NOT call `run_gh pr merge`.
        - Task status: `in_progress`.

      **`"gate_errored"`** — Tool infrastructure failure:
        - Notify Vincent with `kind` and `detail` from response.
        - Do NOT fall back to `run_gh pr merge` (explicit prohibition).
        - Do NOT call `run_claude_pilot`.
        - Task status: `in_progress`.
```

**NF4 addressed:** The "Unrecognized `reason` value" catch-all branch prevents future
improvisation when new `BlockReason` variants are added. Same principle as `gate_errored` —
unknown states escalate rather than improvise.

#### Step 6: Annotate Rule 6

Update Rule 6 (~line 153) to note structural enforcement:
```
Rule 6: ... (Structural enforcement: `pr_merge_with_gate` now returns typed variants.
The `gate_errored` and `blocked` branches above are the exhaustive handling surface.
Runtime enforcement via policy table — see follow-up ticket.)
```

#### Step 7: Tests for Unit 2

**#792 regression test (prompt-layer, integration test):**
Per ticket AC — mock at the **tool/prompt boundary** (stubbing `MergeGateResult` values
directly), NOT at the `gh` CLI byte boundary. The contract being tested is "what
`pr_merge_with_gate` returns to the prompt" — tool classification correctness is Unit 1's job.

Stub `pr_merge_with_gate` to return:
```json
{
  "action": "blocked",
  "reason": { "reason": "merge_conflict" },
  "failing_checks": [],
  "detail": "PR has merge conflicts"
}
```
Assert: (a) one `send_message` with conflict context, (b) zero `run_gh pr merge` calls,
(c) task stays `in_progress`.

**`gate_errored` integration test:**
Stub tool to return:
```json
{
  "action": "gate_errored",
  "kind": { "kind": "gh_cli_failure", "exit_code": 1 },
  "detail": "gh exit_code 1: no checks reported"
}
```
Assert: `send_message` with kind+detail, zero `run_gh pr merge`, task stays `in_progress`.

**Fixture:** Use fixture seeded from trace `a45e31bc-401d-11f1-8227-de6e000e1099` with
provenance comment. Redaction check: only `@mika-platform-qa` (authorized identity).

**Test infrastructure note:** If existing `EvalHarness` + `MockLlmProvider` infrastructure
does not support tool-output stubbing, this is the right place to establish the boundary-level
pattern. Document the deviation in the PR description per ticket AC.

## Commit Strategy

Single branch, two commits (ordered):
1. `feat(agent): pr_merge_with_gate tagged-union return with preflight check (#794)` — Unit 1
2. `feat(agent): webhook-qa gate_errored and blocked[merge_conflict] branches (#794)` — Unit 2

## Risk Assessment

**Low risk:** The `action` tag approach is backward-compatible. Existing `"merged"`,
`"auto_merge_enabled"`, `"already_merged"` are unchanged. `"blocked"` retains `failing_checks`
at top level (F3 resolution) — the new `reason` field is additive.

**Low risk (reduced from medium):** `run_gh_checks()` with `--required` filtering is retained
for the checks path (NF1 resolution). The preflight `gh pr view` only handles state checks
(mergeable, isDraft, state). No replacement of the checks-classification path.

## Architect Findings Applied

| Finding | Severity | Resolution |
|---------|----------|------------|
| F1 — Phase 0 Pin absent | BLOCKING | Added Phase 0 Pin section with 5 pins at commit `8731102d` |
| F2 — Prompt file target ambiguity | BLOCKING | Verified: mika#1106 CLOSED but decomposition shipped separately; `skills/bundled/self-dev-webhook-qa/system_prompt.md` confirmed live on main |
| F3 — `Blocked` payload breaking change | BLOCKING | `Blocked` retains `failing_checks` at top level; `reason` is additive field. Exact serde shapes documented per variant |
| NF1 — Keep `run_gh_checks()` | NON-BLOCKING | Applied: preflight for state, `run_gh_checks()` for checks classification |
| NF2 — `AlreadyMerged` preflight + stderr | NON-BLOCKING | Applied: preflight primary (`state == "MERGED"`), stderr fallback for races |
| NF4 — Unknown `BlockReason` catch-all | NON-BLOCKING | Applied: "Unrecognized reason" branch in prompt |
| NF5 — Cite mika#524 compound doc | NON-BLOCKING | Applied: frontmatter `prior_art` + Context section reference |
