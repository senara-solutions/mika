---
ticket: mika#1517
branch: fix/1517/auto-pull-skip-open-pr
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/1517
execution: code
---

# Plan: auto_pull skips issues with open PRs (mika#1517)

## Problem frame

`crates/mika-agent/src/auto_pull.rs::select_best_candidate` filters out `ready`-labeled issues but does not check whether an issue already has an open PR closing it. When `dispatch-lib`'s recovery paths (mika#1282 dirty-worktree, mika#1396 commit-pushed-no-pr) create a DRAFT PR and the `ready` label is subsequently removed, the next auto-pull tick (cron `0 */10 * * * *`) re-promotes the same ticket, triggering a parallel pilot session. Today's session confirmed n=3 same-day on mika#606 alone (15:18Z PR creation → 16:50Z pilot 2 → 19:00Z pilot 3 in flight), burning ~$3–5 per redundant pilot.

## Scope boundaries

**In scope:**
- Add a candidate filter that excludes issues with an open PR closing them.
- Populate the filter set by querying `gh pr list --state open --json number,closingIssuesReferences` once per auto-pull tick.
- Update existing tests with the new `select_best_candidate` signature.
- Add a test for the PR-existence filter.

**Out of scope:**
- Callback-time re-validation (direction C in mika#1517's comment) — separate concern, separate plan.
- Removing the `ready` label inside dispatch-lib's recovery paths (direction B').
- Worktree-prep idempotency (direction D).
- Backfill of existing draft PRs on `senara-solutions/mika`.

## Implementation Units

### U1 — Add `gh_list_open_pr_closing_issues` helper

**Goal:** Fetch the set of issue numbers that have an open PR closing them.

**Files:**
- Modify: `crates/mika-agent/src/auto_pull.rs` — add new async helper.

**Approach:**

```rust
async fn gh_list_open_pr_closing_issues(github_token: &str) -> Result<HashSet<u64>> {
    let mut cmd = tokio::process::Command::new("gh");
    cmd.args([
        "pr", "list",
        "--repo", DEFAULT_REPO,
        "--state", "open",
        "--json", "number,closingIssuesReferences",
        "--limit", "100",
    ]);
    cmd.env("GH_TOKEN", github_token);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh pr list failed: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw: Vec<serde_json::Value> = serde_json::from_str(&stdout)?;

    let mut closed_issue_numbers = HashSet::new();
    for pr in raw {
        if let Some(refs) = pr["closingIssuesReferences"].as_array() {
            for ref_obj in refs {
                if let Some(n) = ref_obj["number"].as_u64() {
                    closed_issue_numbers.insert(n);
                }
            }
        }
    }
    Ok(closed_issue_numbers)
}
```

**Verification:** Manual smoke (the test layer mocks the candidate set directly; this helper is the thin gh-shell, same shape as `gh_list_open_issues`).

### U2 — Update `select_best_candidate` signature + filter

**Goal:** Candidate selection skips issues with open PRs.

**Files:**
- Modify: `crates/mika-agent/src/auto_pull.rs::select_best_candidate`.

**Approach:** Add a `&HashSet<u64>` parameter and a `.filter()` step before the `is_groomed` filter:

```rust
pub fn select_best_candidate(
    issues: Vec<Issue>,
    open_pr_issue_numbers: &HashSet<u64>,
) -> Option<Issue> {
    let candidates: Vec<_> = issues
        .into_iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == "ready"))
        .filter(|i| !open_pr_issue_numbers.contains(&i.number))
        .filter(|i| is_groomed(&i.body))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    candidates.into_iter().max_by(|a, b| {
        let pa = priority_rank(&a.labels);
        let pb = priority_rank(&b.labels);
        pa.cmp(&pb).then_with(|| b.updated_at.cmp(&a.updated_at))
    })
}
```

**Verification:** `cargo test -p mika-agent --lib auto_pull::tests::test_select_*` — all existing tests updated with `&HashSet::new()` as the new param; new test asserts that an issue in the open-PR set is skipped.

### U3 — Wire caller to populate `open_pr_issue_numbers`

**Goal:** `auto_pull_groomed_ticket` calls the new helper and threads the result into `select_best_candidate`.

**Files:**
- Modify: `crates/mika-agent/src/auto_pull.rs::auto_pull_groomed_ticket`.

**Approach:** After step 2 (fetch open issues) and before step 3 (select candidate), fetch the open-PR set. On error, log warn and use an empty set (fail-open — duplicate dispatch is worse than the substrate but better than no dispatch at all):

```rust
let open_pr_issue_numbers = match gh_list_open_pr_closing_issues(github_token).await {
    Ok(set) => set,
    Err(e) => {
        warn!(error = %e, "auto_pull: failed to list open PR closing-issue refs; proceeding without filter");
        HashSet::new()
    }
};

let candidate = match select_best_candidate(issues, &open_pr_issue_numbers) {
    // ...
};
```

**Verification:** Build clean. Behavioral test deferred to U2 (the filter logic lives in `select_best_candidate`).

### U4 — Update existing tests

**Goal:** All existing `select_best_candidate` tests compile and pass with the new signature.

**Files:**
- Modify: `crates/mika-agent/src/auto_pull.rs::tests` — every call to `select_best_candidate` gets a second arg `&HashSet::new()`.

**Approach:** Mechanical — pass `&HashSet::new()` to every call site. Tests are not exercising the new filter, so an empty set preserves their current semantics.

### U5 — Add filter regression test

**Goal:** Prove the new filter excludes issues with open PRs.

**Files:**
- Modify: `crates/mika-agent/src/auto_pull.rs::tests` — add `test_select_skips_issues_with_open_pr`.

**Approach:**

```rust
#[test]
fn test_select_skips_issues_with_open_pr() {
    let body = groomed_body();
    let issues = vec![
        make_issue(606, &body, &["p2-normal"], "2026-06-13T15:18Z"),
        make_issue(851, &body, &["p2-normal"], "2026-06-13T16:00Z"),
    ];
    let mut open_pr_set = HashSet::new();
    open_pr_set.insert(606); // mika#606 has an open PR — should be skipped

    let result = select_best_candidate(issues, &open_pr_set);
    assert!(result.is_some());
    assert_eq!(result.unwrap().number, 851);
}
```

## Acceptance Criteria

- AC1: `select_best_candidate` skips any issue whose number is in `open_pr_issue_numbers`.
- AC2: `auto_pull_groomed_ticket` fetches the open-PR closing-issue set once per tick via `gh pr list --json closingIssuesReferences` and passes it to `select_best_candidate`.
- AC3: On `gh_list_open_pr_closing_issues` failure, the function logs warn and proceeds with an empty set (fail-open — preserves current behavior on infra glitches).
- AC4: All existing `auto_pull::tests` pass with the new signature.
- AC5: New regression test `test_select_skips_issues_with_open_pr` asserts the filter behavior.
- AC6: `cargo clippy --all-targets --all-features -- -D warnings` clean.

## Risk shape

- **Failure mode of the filter**: false negatives (PR exists but no `closingIssuesReferences` link). Mitigation: operator can link PR via `gh issue edit --add-link` or PR body `Closes #N`. This is an existing convention.
- **GH API rate**: one extra `gh pr list` call per 10min auto-pull tick — negligible.
- **Backwards compatibility**: only public function `select_best_candidate` signature changes. Only caller is `auto_pull_groomed_ticket` in the same module.

## References

- Substrate ticket: mika#1517 (filed 2026-06-13 by mika-platform-claude with code-level root cause + sharpened fix proposal)
- Founding incidents: mika#802 (PRs #1512+#1516), mika#606 (PR #1514, n=3 dispatches same day)
- Sibling substrate: mika#1513 (Check preemption — makes recovery PRs sit longer)
- Sibling substrate: mika#1515 (recovery PR title hygiene)
- Sibling direction (not in scope): C (callback re-validation at pilot-spawn)
- Operator memory: `feedback_n_equals_2_is_the_signal` — n=2 triggers tactical fix
- Operator memory: `feedback_samidarko_claude_new_center_keep_pushing` — auto-ship warrant for the resulting PR
