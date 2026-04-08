---
title: "fix: Client-side CI gate before merge (pr_merge_with_gate tool)"
type: fix
status: completed
date: 2026-04-08
issue: "#490"
---

# fix: Client-side CI gate before merge

## Overview

Add a new `pr_merge_with_gate` builtin tool that enforces a client-side CI gate before every PR merge. The tool reads the GitHub `statusCheckRollup` via `gh pr checks --required`, classifies the state of required checks, and either merges immediately, enables auto-merge, or blocks — returning a structured JSON result the agent can act on.

**Root cause:** On 2026-04-08, mika#485 merged with a required CI check in FAILURE state because the merge code path called `gh pr merge` without first inspecting the check rollup. This tool makes the gate impossible to skip — even if the LLM ignores prompt instructions, the tool itself refuses to merge with failing required checks.

## Problem Statement

The current merge flow (observed in trace `5d613822-3348-11f1-81f8-c22d175085f1`):

1. Webhook/callback signals "PR ready for merge"
2. Agent calls `run_gh pr merge <num> --squash --delete-branch`
3. Agent calls `update_work_item_status` and notifies user

There is **no** `gh pr view --json statusCheckRollup` before step 2. The merge proceeds whether CI is green, red, or mid-flight.

## Proposed Solution

A new `Tool` trait implementor in `crates/mika-agent/src/tools/pr_merge_with_gate.rs` that:

1. Runs `gh pr checks <number> --repo <repo> --required --json name,state,bucket` to get only required check statuses
2. Classifies results using the decision matrix (see below)
3. Either merges, enables auto-merge, or blocks — all in one atomic tool call
4. Returns structured JSON so the agent can act on the result programmatically

### Decision Matrix

| Required check states | `bucket` values | Tool action | Return |
|---|---|---|---|
| Any FAILURE / CANCELLED / TIMED_OUT / ACTION_REQUIRED | `fail` or `cancel` present | **Block.** Do NOT merge. | `{action: "blocked", failing_checks: [{name, state, link}]}` |
| Any PENDING / IN_PROGRESS / QUEUED (no failures) | `pending` present, no `fail`/`cancel` | Enable auto-merge: `gh pr merge --auto --squash --delete-branch` | `{action: "auto_merge_enabled", pending_checks: [{name, state}]}` |
| All SUCCESS / NEUTRAL / SKIPPED | All `pass` or `skipping` | Merge immediately: `gh pr merge --squash --delete-branch` | `{action: "merged"}` |
| Empty (no required checks) | `[]` | Merge immediately (same as all-pass) | `{action: "merged"}` |

## Technical Considerations

### Architecture: Tool Trait Implementor (not Builtin Handler)

The tool is a standalone `Tool` trait implementor in `tools/pr_merge_with_gate.rs`, **not** a builtin handler in `skills/builtin_handlers.rs`. Rationale:

- **Structural safety:** A `Tool` impl enforces the gate in code. A builtin handler behind a skill manifest could be disabled per-agent, which defeats the purpose of a safety gate.
- **Consistent with `check_work_item`:** That tool also calls GitHub APIs and is registered in `default_tools()`.
- **No skill manifest needed:** The tool is a direct, always-available capability.

### Subprocess Execution Pattern

The tool spawns `gh` CLI subprocesses (not REST API). Pattern mirrors `run_gh` in `builtin_handlers.rs`:

1. Create `tokio::process::Command::new("gh")`
2. Pass args as array (not shell string)
3. Set `GH_PROMPT_DISABLED=1`
4. Call `scrub_mika_env_vars(&mut cmd)` to remove MIKA_* vars
5. Re-inject `ctx.github_token` as `GH_TOKEN` after scrub
6. Collect stdout/stderr with bounded read

**Shared helper extraction:** `scrub_mika_env_vars` is in `skills/executor.rs`. It's already `pub(crate)` — accessible from `tools/`. For subprocess spawn+collect, write a small helper in the tool module (not worth extracting to a shared module for one consumer).

### Input Validation

- **`repo`:** Must match `^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$` (owner/repo format). Reject URLs, paths, and shell metacharacters.
- **`pr_number`:** Positive u64 (enforced by serde).
- **`merge_method`:** Whitelist `["squash", "merge", "rebase"]`. Default: `"squash"`. Reject anything else before subprocess invocation.
- **`delete_branch`:** Optional bool, default `true`.

### Token Handling

- Requires `ctx.github_token` (resolved from GitHub App installation token or `MIKA_GITHUB_TOKEN` PAT via `Settings::resolve_github_token()`).
- Returns `ToolOutput::error()` if no token is available — graceful degradation, same pattern as `check_work_item`.

### Error Handling

| `gh` failure | Tool response |
|---|---|
| PR already merged | `{action: "already_merged"}` (parse "already been merged" from stderr) |
| PR is draft | `ToolOutput::error("PR is a draft — convert to ready before merging")` |
| Merge conflicts | `ToolOutput::error("Merge conflicts — resolve before merging")` |
| Missing review approvals | `ToolOutput::error("Required reviews not met")` |
| PR not found / 404 | `ToolOutput::error("PR not found")` |
| `gh` not installed | `ToolOutput::error("gh CLI not found — install from https://cli.github.com")` |
| Network/auth failure | `ToolOutput::error(stderr)` |
| TOCTOU race (checks pass → merge fails) | `ToolOutput::error(stderr)` — agent can re-call to get updated state |

### Timeout

Override `timeout_secs()` to return `Some(60)` — two sequential subprocess calls (checks + merge) plus GitHub API latency.

### Registration

Register in `default_tools()` in `tools/mod.rs`. All agents get the tool — it's a safe gate that *prevents* bad merges, so wide availability is a feature, not a risk. The tool itself never merges with failing checks regardless of who calls it.

### `merge_sha` — Dropped

`gh pr merge` does not return the merge SHA in its output. Fetching it would require an additional API call. Per the "merge two-step LLM tool contracts into single atomic tools" learning, keep the tool simple. The agent can use `gh pr view --json mergeCommit` separately if needed.

### Auto-merge Semantics

When `auto_merge_enabled` is returned:
- The PR is **not yet merged**. The agent must not claim "merged".
- The tool description explicitly warns: "auto_merge_enabled means GitHub will merge when all checks pass — the PR is NOT yet merged."
- The `pull_request.closed(merged)` webhook will fire when the actual merge happens, which flows through the existing callback system.

## System-Wide Impact

- **Interaction graph:** Agent calls `pr_merge_with_gate` → tool spawns `gh pr checks` subprocess → parses JSON → (if green) spawns `gh pr merge` subprocess → returns structured result → agent calls `update_work_item_status` → agent calls `send_message`. No callbacks, no middleware, no observers.
- **Error propagation:** `gh` subprocess errors surface as `ToolOutput::error()`. The agent sees the error in its tool result and can retry or notify the user. No retry logic in the tool itself.
- **State lifecycle risks:** None — the tool is stateless. It reads check status and optionally triggers a merge. No DB writes, no partial state.
- **API surface parity:** `run_gh pr merge` remains available as an escape hatch. The self-dev prompt (separate mika-skills PR) will direct the agent to prefer `pr_merge_with_gate`.
- **Completion-claim guard interaction:** After a successful `merged` result, the agent must still call `update_work_item_status` before saying "merged" in text — the existing guard (#483) enforces this. The tool description will instruct: "After a successful merge, update the work item status before reporting to the user."

## Acceptance Criteria

### Core Tool (`crates/mika-agent/src/tools/pr_merge_with_gate.rs`)

- [x] New `PrMergeWithGateTool` struct implementing `Tool` trait
- [x] Input schema: `pr_number` (u64, required), `repo` (string, required, owner/repo format), `merge_method` (string, optional, default "squash"), `delete_branch` (bool, optional, default true)
- [x] `repo` validation: regex `^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$`
- [x] `merge_method` validation: whitelist `["squash", "merge", "rebase"]`
- [x] Token requirement: error if `ctx.github_token` is `None`
- [x] Decision matrix implemented exactly as documented
- [x] Empty required-checks case → treat as all-pass → immediate merge
- [x] `already_merged` detection from `gh pr merge` stderr
- [x] `timeout_secs()` returns `Some(60)`
- [x] Registered in `default_tools()` in `tools/mod.rs`

### Subprocess Management

- [x] `gh` spawned with `env_clear()` + allowlist or `scrub_mika_env_vars()`
- [x] `GH_TOKEN` injected from `ctx.github_token` after scrub
- [x] `GH_PROMPT_DISABLED=1` set
- [x] Bounded stdout/stderr read (prevent OOM from large output)
- [x] `gh` not-found error handled gracefully

### Tests (`#[cfg(test)] mod tests` in the tool file)

- [x] Unit test: all required checks pass → `action: "merged"`
- [x] Unit test: some checks pending, none failing → `action: "auto_merge_enabled"`
- [x] Unit test: any check failing → `action: "blocked"` with failing check names
- [x] Unit test: empty checks array → `action: "merged"` (no required checks)
- [x] Unit test: mixed failing + pending → `action: "blocked"` (failure takes priority)
- [x] Unit test: invalid `merge_method` → error
- [x] Unit test: invalid `repo` format → error
- [x] Unit test: missing `github_token` → error
- [x] Unit test: `repo` format validation (valid and invalid cases)
- [x] Unit test: `classify_checks()` function tested independently with fixture JSON
- [x] Integration test via `TestHarness`: tool registered, execute with mock — verify `is_error` on missing token

### Documentation

- [x] Tool description in the `definition()` method includes decision matrix summary
- [x] Tool description warns about `auto_merge_enabled` semantics
- [x] Tool description instructs agent to update work item status after merge

## Implementation Plan

### Phase 1: Core Tool (single file)

**File: `crates/mika-agent/src/tools/pr_merge_with_gate.rs`**

```rust
// Structures:
pub struct PrMergeWithGateTool;

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

#[derive(Debug, Serialize, Deserialize)]
struct CheckInfo {
    name: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
}

// Key functions:
fn validate_repo(repo: &str) -> Result<(), String>
fn validate_merge_method(method: &str) -> Result<(), String>
fn classify_checks(checks: &[CheckInfo]) -> CheckClassification
async fn run_gh_command(args: &[&str], token: &str) -> Result<GhOutput, String>
```

**Classification logic (pure function, highly testable):**

```rust
enum CheckClassification {
    AllPassed,       // all pass/skipping, or empty
    HasPending,      // pending present, no fail/cancel
    HasFailures,     // fail or cancel present
}

fn classify_checks(checks: &[GhCheck]) -> CheckClassification {
    let has_failures = checks.iter().any(|c| matches!(c.bucket.as_str(), "fail" | "cancel"));
    let has_pending = checks.iter().any(|c| c.bucket == "pending");

    if has_failures {
        CheckClassification::HasFailures
    } else if has_pending {
        CheckClassification::HasPending
    } else {
        CheckClassification::AllPassed
    }
}
```

### Phase 2: Registration

**File: `crates/mika-agent/src/tools/mod.rs`**

- Add `mod pr_merge_with_gate;`
- Add `registry.register(Box::new(pr_merge_with_gate::PrMergeWithGateTool));` in `default_tools()`

### Phase 3: Tests

All tests in `#[cfg(test)] mod tests` at the bottom of `pr_merge_with_gate.rs`.

- `classify_checks()` tested with fixture JSON covering all 5 matrix branches
- Input validation tested (repo format, merge_method whitelist)
- Missing token tested via `TestHarness`
- No subprocess mocking needed for classification tests — the pure function is the critical path

## Dependencies

- **mika#489** (metadata shallow-merge): Not a hard blocker for this PR. This PR does not write metadata. The `ci_fix_count` metadata field lives in the self-dev prompt (mika-skills, separate PR).
- **`gh` CLI >= 2.24.0:** Required for `--required` flag on `gh pr checks`. Dockerfile.agent already includes `gh`. Version check is nice-to-have, not blocking.

## Out of Scope

- Self-dev prompt update (separate mika-skills PR after this lands)
- `ci_fix_count` metadata field (handled by self-dev prompt)
- Dashboard for retry metrics
- Admin/bypass merge mode
- Teaching mika-qa to gate on CI (QA is advisory per `feedback_qa_advisory_ci_gate_on_dev`)
- `merge_sha` in the response (not worth the extra API call)

## Sources & References

- Issue: [mika#490](https://github.com/senara-solutions/mika/issues/490)
- Umbrella: senara-solutions/mika-platform#17 (section B)
- Incident: mika#485 merged with Pipeline Artifacts: FAILURE
- Pattern reference: `crates/mika-agent/src/tools/check_work_item.rs` — GitHub API tool pattern
- Pattern reference: `crates/mika-agent/src/skills/builtin_handlers.rs:817-841` — `run_gh` subprocess pattern
- Learning: `docs/solutions/architecture-patterns/merge-two-step-llm-tool-contracts.md` — make operations atomic
- Learning: `docs/solutions/integration-issues/run-gh-github-token-injection.md` — GH_TOKEN injection after scrub
- Learning: `docs/solutions/code-review-patterns/extract-shared-github-get-helper.md` — reuse patterns
- Learning: `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md` — tool results as ground truth
