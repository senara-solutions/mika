---
title: "fix: not-behind-main assertion in autonomous merge path"
issue: 1577
type: fix
depth: Standard
origin: null
created: 2026-06-26
---

# fix: Not-Behind-Main Assertion in Autonomous Merge Path

## Summary

Add an up-to-date-with-main assertion to every autonomous merge site. The autonomous merge actor (`mika-platform-dev`) bypasses the GitHub ruleset, so the only gate that binds is in-code. Today, two PRs green against the same base can merge in sequence, producing a `main` that neither CI run validated. This fix compares the PR's `baseRefOid` against the current `main` HEAD SHA before every `run_gh_merge` call and blocks with a structured `BlockReason::BehindMain` when they diverge.

---

## Problem Frame

The `mika-platform-dev` actor has `bypass_mode: always` on the senara-solutions/mika ruleset. `strict_required_status_checks_policy` is `false`. The in-code merge path checks required-check passage but not whether the PR is up-to-date with `main`. Two PRs can both be green against their own (pre-A-merge) base and merge sequentially, producing a `main` that neither CI run validated. The race is latent (low PR concurrency today) but structural — will fire harder as the required-check set grows and CI Docker matrix is added.

---

## Requirements

- R1: New `BlockReason::BehindMain { pr_base_sha, current_main_sha }` variant on `BlockReason`.
- R2: All four merge sites check up-to-date-ness before calling `run_gh_merge` (immediate or auto-merge). If behind, no merge attempt; structured block returned.
- R3: Unit tests for the `BehindMain` block path in each affected site.
- R4: Block reason carries both SHAs so recovery digests can reference them.

---

## Key Technical Decisions

**KTD-1: SHA comparison via `gh pr view --json baseRefOid` + `gh api repos/{repo}/branches/main`.** The `mergeStateStatus` field in `gh pr view` would be the simplest check, but it respects the ruleset's `strict_required_status_checks_policy: false` setting — it returns `CLEAN` even when behind. An explicit SHA comparison is necessary to enforce up-to-date-ness below the bypass layer. The `baseRefOid` field from `gh pr view` gives the PR's base commit; a lightweight `gh api` call gives `main` HEAD. Both are deterministic, no LLM dependency.

**KTD-2: Check in `classify_preflight` (preflight path) rather than post-merge-attempt.** The behind-main check fits the existing preflight pattern — detect the block *before* attempting `run_gh_merge`, consistent with how CONFLICTING/CLOSED/DRAFT are handled. This avoids the race where `run_gh_merge` succeeds despite being behind (GitHub auto-merge can complete on a behind branch when strict checks are off). A single `is_behind_main` helper called from all four sites keeps the logic DRY.

**KTD-3: Shared `is_behind_main` async helper, not inline in `classify_preflight`.** `classify_preflight` is a pure function today (takes `&PrPreflight`, returns `Option<MergeGateResult>`). The behind-main check requires an async API call to fetch the current `main` HEAD SHA. Rather than making `classify_preflight` async (which would change its signature for all callers), add a standalone `is_behind_main(pr_number, repo, token) -> Result<Option<BehindMainInfo>, String>` helper that the four merge sites call before the check-classification branch. This keeps the pure function pure and localizes the new async work.

**KTD-4: Handler sites (`ci_success_handler`, `verdict_handler`) return `Passthrough` with enrichment on behind-main.** These handlers don't return `MergeGateResult` — they return `VerdictAction`. On behind-main, they return `Passthrough { enrichment }` with a digest instructing the LLM to rebase. This follows the same pattern as `HasFailures` in `verdict_handler` (line 358) — the handler declines to merge and passes through with guidance.

---

## Implementation Units

### U1. Add `BehindMain` variant to `BlockReason` and extend `PrPreflight`

**Goal:** Extend the type system to represent the behind-main block state.

**Requirements:** R1, R4

**Dependencies:** None

**Files:**
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs`

**Approach:** Add `BehindMain { pr_base_sha: String, current_main_sha: String }` variant to `BlockReason` with `#[serde(rename = "behind_main")]`. Add `base_ref_oid: String` field to `PrPreflight` (deserialized from `baseRefOid`). Update `run_gh_pr_view` to request `baseRefOid` in the JSON field list.

**Patterns to follow:** Existing `BlockReason::MergeConflict` variant shape. `PrPreflight` serde `rename_all = "camelCase"` handles the field name mapping.

**Test scenarios:**
- Serde round-trip: `BehindMain` variant serializes to `{"reason": "behind_main", "pr_base_sha": "...", "current_main_sha": "..."}`
- `PrPreflight` deserializes `baseRefOid` from gh JSON output

### U2. Add `is_behind_main` helper and `fetch_main_head_sha` subprocess helper

**Goal:** Provide a reusable async function that determines whether a PR is behind `main` by comparing `baseRefOid` against the current `main` HEAD SHA.

**Requirements:** R2

**Dependencies:** U1

**Files:**
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs`

**Approach:** Add `fetch_main_head_sha(repo, token) -> Result<String, String>` that runs `gh api repos/{repo}/git/ref/heads/main --jq .object.sha`. Add `is_behind_main(base_ref_oid: &str, repo: &str, token: &str) -> Result<Option<BehindMainInfo>, String>` where `BehindMainInfo { pr_base_sha, current_main_sha }`. Returns `Ok(Some(info))` when behind, `Ok(None)` when up-to-date, `Err` on API failure. The helper compares the two SHAs; if they differ, the PR is behind.

**Patterns to follow:** `run_gh_pr_view` and `run_gh_checks` subprocess helper pattern — uses `run_gh_subprocess` with token injection.

**Test scenarios:**
- `is_behind_main` returns `Some(BehindMainInfo)` when SHAs differ
- `is_behind_main` returns `None` when SHAs match
- `fetch_main_head_sha` parses SHA from `gh api` output

### U3. Wire behind-main check into `pr_merge_with_gate` tool

**Goal:** Block merge in the `HasPending` and `AllPassed` branches when the PR is behind `main`.

**Requirements:** R2

**Dependencies:** U1, U2

**Files:**
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs`

**Approach:** After `classify_preflight` passes (returns `None`) and before the `classify_checks` match, call `is_behind_main(preflight.base_ref_oid, repo, token)`. If behind, return `MergeGateResult::Blocked { reason: BlockReason::BehindMain { ... }, ... }` immediately — do not proceed to check classification or merge attempt. On API error, log a warning and proceed (fail-open — the GitHub merge itself will reject if there's a real problem). This ensures both `HasPending` (auto-merge) and `AllPassed` (immediate merge) are guarded with a single check point.

**Patterns to follow:** The preflight early-return pattern at line 144 (`if let Some(result) = classify_preflight(...)`).

**Test scenarios:**
- PR behind main returns `Blocked { reason: BehindMain }` with both SHAs populated
- PR up-to-date with main proceeds to check classification normally
- API error fetching main SHA logs warning and proceeds (fail-open)

### U4. Wire behind-main check into `ci_success_handler`

**Goal:** Block merge in the CI success handler when the PR is behind `main`.

**Requirements:** R2

**Dependencies:** U1, U2

**Files:**
- `crates/mika-agent/src/server/ci_success_handler.rs`

**Approach:** After the CI aggregation gate (step 5, around line 242) and before the merge initiation (step 6, line 255), fetch `baseRefOid` via `run_gh_pr_view` (the handler already has `pr.number` and `event.repo`) and call `is_behind_main`. If behind, return `VerdictAction::Passthrough { enrichment }` with a digest explaining the PR is behind main and needs rebase. The existing `run_gh_pr_view` call returns `PrPreflight` which now includes `base_ref_oid` (from U1), so the handler can access it without an additional API call — but the handler doesn't currently call `run_gh_pr_view`. Add the call.

**Patterns to follow:** The `Passthrough` enrichment pattern used for `HasFailures` in `verdict_handler` (line 358).

**Test scenarios:**
- PR behind main returns `Passthrough` with behind-main enrichment message
- PR up-to-date proceeds to merge initiation
- Enrichment message includes both SHAs for diagnostic reference

### U5. Wire behind-main check into `verdict_handler`

**Goal:** Block merge in the verdict handler when the PR is behind `main`.

**Requirements:** R2

**Dependencies:** U1, U2

**Files:**
- `crates/mika-agent/src/server/verdict_handler.rs`

**Approach:** In `handle_pass_verdict`, after fetching CI checks (step around line 318-341) and before the `classify_checks` match (line 343), fetch `baseRefOid` via `run_gh_pr_view` and call `is_behind_main`. If behind, return `VerdictAction::Passthrough { enrichment }` with a diagnostic digest. Same pattern as U4.

**Patterns to follow:** The `Passthrough` enrichment pattern at line 358 for `HasFailures`.

**Test scenarios:**
- PR behind main returns `Passthrough` with behind-main enrichment message
- PR up-to-date proceeds to check classification and merge
- Both auto-merge (HasPending) and immediate merge (AllPassed) paths are blocked when behind

### U6. Unit tests for `BehindMain` block paths

**Goal:** Comprehensive unit tests for the behind-main assertion across all four merge sites.

**Requirements:** R3

**Dependencies:** U1-U5

**Files:**
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` (inline `#[cfg(test)] mod tests`)
- `crates/mika-agent/src/server/ci_success_handler.rs` (inline tests)
- `crates/mika-agent/src/server/verdict_handler.rs` (inline tests)

**Approach:** Add tests for `classify_preflight`-level assertions (pure function tests), `is_behind_main` helper tests, and integration-style tests for each handler. For `pr_merge_with_gate`, test that the `Blocked { BehindMain }` result is returned with correct SHAs. For handlers, test that `Passthrough` with enrichment is returned when behind. Use the existing test pattern — `PrPreflight` struct construction for pure tests, and the existing mock patterns for handler tests where available.

**Patterns to follow:** Existing `regression_792_conflicting_pr_returns_blocked_merge_conflict` test pattern for preflight tests. Handler tests follow the existing `#[cfg(test)]` module patterns in each file.

**Test scenarios:**
- `pr_merge_with_gate`: behind-main returns `Blocked { BehindMain { pr_base_sha, current_main_sha } }`
- `pr_merge_with_gate`: up-to-date returns `None` from behind-main check (proceeds normally)
- `is_behind_main`: matching SHAs return `None`
- `is_behind_main`: differing SHAs return `Some(BehindMainInfo)`
- `fetch_main_head_sha`: parses valid SHA from API response
- Handler enrichment messages contain both SHAs

---

## Scope Boundaries

### In scope
- `BehindMain` variant on `BlockReason`
- Behind-main check at all four merge sites (two in `pr_merge_with_gate`, one each in `ci_success_handler` and `verdict_handler`)
- Unit tests for all new code paths

### Deferred to Follow-Up Work
- Auto-rebase actor / structural rebase trigger when `BehindMain` fires
- `strict_required_status_checks_policy=true` toggle on the ruleset
- GitHub merge-queue integration
- CI Docker matrix gate addition (depends on this ticket closing the underlying race first)

---

## Risks & Dependencies

- **GitHub API rate limits:** Each behind-main check adds one `gh api` call per merge attempt. At current PR concurrency (low), this is negligible. If concurrency rises, the call could be cached briefly, but that's premature optimization for now.
- **Fail-open policy:** If `fetch_main_head_sha` fails (network, auth), the check is skipped with a warning. This is intentional — the underlying race is latent and rare; a flaky API shouldn't block all merges. The GitHub merge itself provides a last-resort guard.

---

## Sources & Research

- Issue #1577 analysis of the merge race shape and bypass-layer reasoning
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — existing `BlockReason` enum, `classify_preflight`, `run_gh_pr_view`, `run_gh_merge` patterns
- `crates/mika-agent/src/server/ci_success_handler.rs` — merge site at line 255, `VerdictAction::Passthrough` pattern
- `crates/mika-agent/src/server/verdict_handler.rs` — merge site at line 369, `HasFailures` passthrough pattern
- `docs/solutions/logic-errors/stale-base-conflicting-prs-no-self-heal-2026-04-23.md` — prior stale-base analysis
- GitHub REST API: `GET /repos/{owner}/{repo}/git/ref/heads/{branch}` returns `{ object: { sha } }`
