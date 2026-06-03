# Plan: Harden PR review dedup key + regression tests (mika#736)

**Issue:** senara-solutions/mika#736
**Type:** Bug fix (residual gaps after mika#821)
**Branch:** `fix/736/qa-review-double-review-in-single-turn`

## Background

mika-qa posted two `pr review --approve` calls within a single webhook-triggered turn on PR #735 (2026-04-22). The root cause was a collision between the per-turn `AtomicBool` reset and the required-tools gate forcing a retry turn. mika#821 landed two fixes:

- **Fix A:** Session-scoped `pr_reviews_posted` DashMap on `AppState` (primary defense)
- **Fix B:** `has_successful_pr_review()` extends early-accept to skip guard #3 (required-tools gate)

Both fixes are in production and working. This ticket addresses three residual gaps identified during the post-fix audit.

## Gap Analysis

### Gap 1: `make_pr_dedup_key` is format-fragile

`make_pr_dedup_key(args, repo)` uses `args.get(2)` raw as the positional component of the dedup key. If the LLM uses `"735"` on the first call and `"https://github.com/senara-solutions/mika/pull/735"` on the second call (or vice versa), the keys differ:

```
__default__|735
__default__|https://github.com/senara-solutions/mika/pull/735
```

The session-scoped map lookup fails silently, and the duplicate review is posted. This is the highest-risk gap — the LLM's argument formatting is not deterministic across turns, especially when a required-tools retry forces re-planning.

### Gap 2: Integration test doesn't exercise session-scoped guard

`tests/eval/test_pr_review_idempotency.rs` uses a `StubRunGhTool` that only implements the per-turn `AtomicBool` check. It does NOT implement or test the session-scoped DashMap defense (Fix A). The test passes even if Fix A is removed. The unit tests in `builtin_handlers.rs` test the DashMap in isolation but don't prove the end-to-end agent loop flow works.

### Gap 3: No grounding regression test for the exact #736 reproduction trace

The `tests/eval/grounding_regressions/` suite has 35 scenarios but none that reproduce the specific qa-review + required-tools-gate-retry + duplicate-review chain from #736/#821.

## Implementation

### Step 1: Normalize PR identifiers in `make_pr_dedup_key`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

Add a `normalize_pr_identifier(s: &str) -> &str` helper that extracts the PR number from both formats:

```rust
/// Extract the PR number from a positional argument.
///
/// Handles:
/// - Bare numbers: "735" → "735"
/// - GitHub URLs: "https://github.com/org/repo/pull/735" → "735"
/// - Full URLs with query/fragment: ".../pull/735?diff=unified" → "735"
///
/// Falls back to the original string if no number can be extracted
/// (preserves current behavior for unknown formats).
fn normalize_pr_identifier(s: &str) -> &str {
    // Try to extract number from GitHub PR URL pattern
    if let Some(idx) = s.rfind("/pull/") {
        let after = &s[idx + 6..];
        // Take only digits (strip query params, fragments, trailing slashes)
        let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        if end > 0 {
            return &after[..end];
        }
    }
    s
}
```

Update `make_pr_dedup_key` to normalize the positional:

```rust
fn make_pr_dedup_key(args: &[String], repo: Option<&str>) -> String {
    let positional = args
        .get(2)
        .map(|s| normalize_pr_identifier(s))
        .unwrap_or("__current_branch__");
    format!("{}|{}", repo.unwrap_or("__default__"), positional)
}
```

**Unit tests** (same file, `#[cfg(test)]` block):

1. `test_normalize_pr_identifier_bare_number` — `"735"` → `"735"`
2. `test_normalize_pr_identifier_github_url` — `"https://github.com/org/repo/pull/735"` → `"735"`
3. `test_normalize_pr_identifier_url_with_query` — `".../pull/735?diff=unified"` → `"735"`
4. `test_normalize_pr_identifier_non_pr_url` — `"https://example.com/other"` → passthrough
5. `test_normalize_pr_identifier_branch_ref` — `"--approve"` → passthrough (not a PR identifier)
6. `test_make_pr_dedup_key_url_vs_number_same_key` — URL and bare number produce the same dedup key

### Step 2: Upgrade eval integration test

**File:** `crates/mika-agent/tests/eval/test_pr_review_idempotency.rs`

Add a new test `pr_review_session_scope_blocks_required_tools_retry` that exercises the full required-tools-gate-retry → session-map-blocks-second-review path:

1. Configure a skill with `required_tools = ["qa_pr_view", "run_gh"]` via the eval harness
2. Mock a sequence where:
   - LLM call 1: `run_gh pr review 455 --approve` (succeeds, no `qa_pr_view` call)
   - LLM call 2 (forced by required-tools gate): `qa_pr_view` + `run_gh pr review 455 --approve` (second review should be rejected)
   - LLM call 3: text response acknowledging the duplicate error
3. Assert: only one successful `run_gh pr review` call in the tool summaries
4. Assert: the duplicate error message appears in the trace

This requires upgrading `StubRunGhTool` to accept an optional `Arc<DashMap<String, HashSet<String>>>` for session-scoped dedup, or using the real `handle_run_gh` via the builtin handler path.

**Decision:** Use a new `SessionAwareStubRunGhTool` that implements both per-turn and session-scoped logic (the real handler has too many dependencies for eval tests). Thread the DashMap via `EvalHarness` builder.

### Step 3: Add grounding regression scenario

**File:** `crates/mika-agent/tests/eval/grounding_regressions/qa_review_required_tools_retry_duplicate.rs`

Add scenario 34: qa-review required-tools-gate duplicate review prevention.

- **Class:** `duplicate-side-effect-suppressed`
- **Shape:** `required-tools-retry-session-guard`
- **Fixture:** Pre-fix response showing a second `pr review` call after required-tools retry
- **Hard assertions:**
  - `assert_response_forbids` the second review's output text (if applicable)
  - Custom assertion: at most one successful `run_gh pr review` call in tool summaries

Register the scenario in `grounding_regressions/mod.rs`.

**Tag vocabulary:** `grounding:duplicate-side-effect-suppressed` (new tag, follows the `grounding:*` namespace convention from #741).

## Test Plan

- [ ] `cargo test -p mika-agent -- make_pr_dedup_key` — unit tests for key normalization
- [ ] `cargo test -p mika-agent -- normalize_pr_identifier` — unit tests for PR identifier normalization
- [ ] `cargo test -p mika-agent -- test_run_gh_session_scope` — existing session-scope unit tests (verify no regressions)
- [ ] `cargo test -p mika-agent --test eval pr_review` — eval integration tests
- [ ] `cargo test -p mika-agent --test eval grounding_regressions` — grounding regression suite
- [ ] `cargo clippy -p mika-agent` — no new warnings
- [ ] `cargo test -p mika-agent` — full crate test suite passes

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/builtin_handlers.rs` | Add `normalize_pr_identifier`, update `make_pr_dedup_key`, add unit tests |
| `crates/mika-agent/tests/eval/test_pr_review_idempotency.rs` | Add session-scope eval test with required-tools-gate replay |
| `crates/mika-agent/tests/eval/grounding_regressions/qa_review_required_tools_retry_duplicate.rs` | New grounding regression scenario |
| `crates/mika-agent/tests/eval/grounding_regressions/mod.rs` | Register new scenario |
| `crates/mika-agent/tests/eval/grounding_regressions/fixtures/qa_review_required_tools_retry_duplicate_pre_fix.json` | Pre-fix fixture |
| `crates/mika-agent/tests/eval/grounding_regressions/README.md` | Update capability matrix and tag vocabulary |

## Risk Assessment

**Low risk.** The normalization change is additive — it only makes the dedup key more stable (fewer false negatives). No existing successful dedup is affected (URL→number normalization can't make two different PRs collide because the PR number is unique per repo).

The `rfind("/pull/")` approach is safe: GitHub PR URLs always have `/pull/<number>` as the path segment. Non-GitHub URLs or malformed inputs fall through to the existing passthrough behavior.

## Not in Scope

- **Session map eviction for conversation-mode sessions** — tracked as DEBT-E in `mika-platform/docs/coherence-debt.md`. Bounded leak; separate concern.
- **Auditing other `AtomicBool` per-turn flags** — `tool_arg_suffix_rejected` and `skills_dirty` are validation/state flags, not side-effect dedup guards. Their per-turn reset is correct behavior. No action needed.
- **`make_pr_dedup_key` repo normalization** — the `repo` component already comes from the `--repo` flag which is either present or absent. The LLM doesn't typically switch between `--repo org/repo` and omitting it. If this surfaces, it's a follow-up.
