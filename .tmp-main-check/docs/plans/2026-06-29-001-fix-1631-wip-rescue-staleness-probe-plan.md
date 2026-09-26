# Plan: fix(dispatch-lib): wip-rescue commits go stale against sibling-PR struct changes (mika#1631)

## Problem Statement

Wip-rescue commits created by `dispatch-lib.sh::_dirty_worktree_rescue` pass clippy/fmt/tests at commit time, but become silently invalid when sibling PRs merge and change struct definitions. The wip branch has no freshness check against main — `git rebase` doesn't catch type-incompatible changes when the text doesn't overlap (struct definition in `db.rs` vs test literals in `skills/mod.rs`). The defect surfaces only at operator-attempted rebase, often days later.

**Hard evidence:** wip-commit `a76b09ba` (mika#1581) had 5 `SkillOverride` test literals. PR #1624 (mika#1584) merged adding `use_count` and `last_used_at` fields. Rebase succeeded silently (no text conflict), but post-rebase clippy failed with 6× `E0063 missing fields`.

## Requirements

1. Detect wip-rescue PR staleness within one CI run after a sibling PR merges to main (AC1).
2. Surface the detection as a structured signal — PR comment, label, or status check failure (AC2).
3. False positive rate ≤ 1/week under normal operation (AC3).

## Approach: CI Staleness Probe (Mechanism A) + Label-Based Targeting

The ticket proposes two mechanisms. Mechanism A (CI probe on push to main) is the right primary fix because it catches drift that emerges *after* the wip-commit — the dominant failure mode. Mechanism B (post-commit rebase+clippy in wip-rescue itself) only catches born-stale commits and adds latency to the rescue path; it's deferred.

### Design

A new GitHub Actions workflow (`wip-staleness-check.yml`) triggers on every push to main. It:

1. Lists open draft PRs whose title starts with `wip(` (the wip-rescue naming convention) or that carry a `wip-rescue` label.
2. For each matching PR, checks out the PR branch, attempts a rebase onto the new main, and runs `cargo clippy --all-targets --all-features -- -D warnings`.
3. On clippy failure: adds a `stale-against-main` label to the PR and posts a comment with the clippy errors.
4. On clippy success (or if already labelled `stale-against-main` from a prior run but now clean): removes the `stale-against-main` label if present.

**Why label + comment, not a required status check:** wip-rescue PRs are draft PRs awaiting operator review. A required status check would block merge, but these PRs are already draft-blocked. The label + comment approach gives the operator a visible signal without adding merge-gate noise to non-wip PRs (AC3).

**Scope limiting for AC3:** The workflow only targets PRs matching the wip-rescue pattern (title prefix `wip(` OR label `wip-rescue`). Normal feature PRs are never touched. Since wip-rescue PRs are rare (0–3 active at any time), the false positive surface is minimal.

## Implementation Steps

### Step 1: Add `wip-rescue` and `stale-against-main` labels

**File:** `.github/labels.yml`

Add two new labels under the `# ── Automation` section:

```yaml
- name: wip-rescue
  color: "d97706"
  description: "PR created by dispatch-lib wip-rescue recovery (mika#1282)"

- name: stale-against-main
  color: "b60205"
  description: "PR branch fails clippy after rebase onto current main (mika#1631)"
```

### Step 2: Apply `wip-rescue` label at PR creation time in dispatch-lib

**File:** `skills/bundled/_shared/dispatch-lib.sh`

After the `gh pr create` call in the recovery block (~line 2463), add a `gh pr edit --add-label wip-rescue` call. This makes label-based filtering reliable even if the PR title format changes in the future.

Specifically, after the `if [ -n "$RESCUED_PR_URL" ]; then` block (~line 2486), add:

```bash
# mika#1631: tag rescued PRs for staleness-probe targeting
gh pr edit "$RESCUED_PR_URL" --add-label "wip-rescue" 2>&9 || true
```

This covers both recovery classes (dirty-worktree and commit-pushed-no-pr).

### Step 3: Create the staleness-check CI workflow

**File:** `.github/workflows/wip-staleness-check.yml`

```yaml
name: Wip Staleness Check

on:
  push:
    branches: [main]

permissions:
  contents: read
  pull-requests: write
  issues: write

concurrency:
  group: wip-staleness-check
  cancel-in-progress: true

jobs:
  check-wip-prs:
    name: Check wip-rescue PRs against main
    runs-on: [self-hosted, Linux, X64, gentux]
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
        with:
          fetch-depth: 0

      - name: Install Rust toolchain
        run: |
          if ! command -v rustup &>/dev/null; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal
            echo "$HOME/.cargo/bin" >> $GITHUB_PATH
          fi
          rustup show

      - uses: Swatinem/rust-cache@779680da715d629ac1d338a641029a2f4372abb5  # v2
        with:
          prefix-key: v1-rust

      - name: Create dashboard dist placeholder
        run: mkdir -p dashboard/dist

      - name: Find and check wip-rescue PRs
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          set -euo pipefail

          # Find open draft PRs with wip-rescue label OR wip( title prefix
          WIP_PRS=$(gh pr list --state open --draft --json number,headRefName,title,labels \
            --jq '[.[] | select(
              (.title | startswith("wip(")) or
              (.labels | map(.name) | index("wip-rescue"))
            )]' 2>/dev/null || echo "[]")

          COUNT=$(echo "$WIP_PRS" | jq 'length')
          echo "Found $COUNT wip-rescue PR(s) to check"

          if [ "$COUNT" -eq 0 ]; then
            echo "No wip-rescue PRs open — nothing to do"
            exit 0
          fi

          ANY_STALE=0

          echo "$WIP_PRS" | jq -c '.[]' | while IFS= read -r pr; do
            PR_NUM=$(echo "$pr" | jq -r '.number')
            BRANCH=$(echo "$pr" | jq -r '.headRefName')
            TITLE=$(echo "$pr" | jq -r '.title')
            HAS_STALE_LABEL=$(echo "$pr" | jq '[.labels[].name] | index("stale-against-main") != null')

            echo ""
            echo "=== Checking PR #${PR_NUM}: ${TITLE} (branch: ${BRANCH}) ==="

            # Fetch the PR branch
            if ! git fetch origin "$BRANCH" 2>/dev/null; then
              echo "WARN: could not fetch branch $BRANCH for PR #$PR_NUM — skipping"
              continue
            fi

            # Create a temporary branch for the rebase test
            TEMP_BRANCH="staleness-check/${PR_NUM}"
            git branch -D "$TEMP_BRANCH" 2>/dev/null || true
            git checkout -b "$TEMP_BRANCH" "origin/$BRANCH" 2>/dev/null

            # Attempt rebase onto main
            if ! git rebase origin/main 2>/dev/null; then
              echo "Rebase conflict detected for PR #$PR_NUM — marking stale"
              git rebase --abort 2>/dev/null || true
              git checkout main 2>/dev/null
              git branch -D "$TEMP_BRANCH" 2>/dev/null || true

              if [ "$HAS_STALE_LABEL" = "false" ]; then
                gh pr edit "$PR_NUM" --add-label "stale-against-main" || true
                gh pr comment "$PR_NUM" --body "## Stale against main (mika#1631 probe)

          This wip-rescue PR has **rebase conflicts** against current \`main\` (detected after merge of ${{ github.sha }}).

          Action needed: rebase and resolve conflicts before promoting from draft." || true
              fi
              continue
            fi

            # Rebase succeeded — run clippy
            CLIPPY_OUTPUT=""
            if CLIPPY_OUTPUT=$(cargo clippy --all-targets --all-features -- -D warnings 2>&1); then
              echo "PR #$PR_NUM: clippy passes after rebase — OK"

              # Remove stale label if it was set from a prior run
              if [ "$HAS_STALE_LABEL" = "true" ]; then
                gh pr edit "$PR_NUM" --remove-label "stale-against-main" || true
                gh pr comment "$PR_NUM" --body "## No longer stale (mika#1631 probe)

          This wip-rescue PR now passes clippy after rebase onto current \`main\`. The \`stale-against-main\` label has been removed." || true
              fi
            else
              echo "PR #$PR_NUM: clippy FAILS after rebase — marking stale"
              ANY_STALE=1

              # Truncate clippy output for the comment (keep first 80 lines)
              CLIPPY_EXCERPT=$(echo "$CLIPPY_OUTPUT" | head -80)

              if [ "$HAS_STALE_LABEL" = "false" ]; then
                gh pr edit "$PR_NUM" --add-label "stale-against-main" || true
              fi

              gh pr comment "$PR_NUM" --body "## Stale against main (mika#1631 probe)

          This wip-rescue PR **fails clippy** after rebase onto current \`main\` (triggered by merge of \`${{ github.sha }}\`).

          <details>
          <summary>Clippy errors (first 80 lines)</summary>

          \`\`\`
          ${CLIPPY_EXCERPT}
          \`\`\`

          </details>

          Action needed: rebase onto main, fix the compilation errors, then promote from draft." || true
            fi

            # Clean up
            git checkout main 2>/dev/null
            git branch -D "$TEMP_BRANCH" 2>/dev/null || true
          done

          echo ""
          echo "Staleness check complete."
```

### Step 4: Add test coverage for label application in dispatch-lib

**File:** `skills/bundled/_shared/test-dispatch-lib.sh`

Add an assertion to the existing rescue-PR test block that verifies the `wip-rescue` label is applied. This is a structural test (checking the `gh pr edit --add-label` command is present in the rescue flow), not a live GitHub API test.

### Step 5: Update Signal documentation

**File:** `CLAUDE.md` (root, Environment Variables § Post-restart safety check)

Add **Signal N — wip-rescue staleness probe (mika#1631):**

```
- **Signal N — wip-rescue staleness probe (#1631).** `grep stale-against-main` in GitHub PR labels — any open draft PR with this label has a type-incompatible rebase against current main. The `wip-staleness-check` workflow runs on every push to main and probes all `wip(` titled or `wip-rescue` labelled draft PRs. Operator action: rebase the branch, fix clippy errors, then promote from draft.
```

## Verification Contract

| ID | Check | Method |
|----|-------|--------|
| V1 | Wip-rescue draft PRs get `wip-rescue` label at creation | Dispatch a test wip-rescue; verify label via `gh pr view --json labels` |
| V2 | Staleness probe fires on push to main | Merge a sibling PR; verify `wip-staleness-check` workflow runs |
| V3 | Stale wip-PR gets `stale-against-main` label + comment | Create a wip PR with type-incomplete literals; merge a struct-extending PR; verify label + comment appear |
| V4 | Clean wip-PR is not falsely labelled | Create a wip PR with no type issues; merge an unrelated PR; verify no label |
| V5 | Previously-stale PR recovers | After V3, fix the wip PR locally and push; verify `stale-against-main` label is removed on next main push |

## Definition of Done

- [ ] `wip-rescue` and `stale-against-main` labels defined in `.github/labels.yml`
- [ ] dispatch-lib applies `wip-rescue` label to rescued draft PRs
- [ ] `wip-staleness-check.yml` workflow committed and functional
- [ ] Test coverage for label application in `test-dispatch-lib.sh`
- [ ] Signal documentation updated in root `CLAUDE.md`
- [ ] All existing CI checks pass (`cargo clippy`, `cargo test`, `make verify-bundled-skills`)

## Acceptance criteria

These are transcribed from the issue body (mika#1631):

- **AC1** — A test wip-commit with type-incomplete test literals + a sibling-merge that adds the missing fields is detected by the new probe within one engine tick or CI run after the sibling merges.
- **AC2** — The probe's failure produces a structured signal (PR comment, label, or status check fail) — not just a log line.
- **AC3** — False positive rate ≤ 1 per week under normal operation (don't make every PR push fire a noisy false-stale).

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Self-hosted runner build time adds latency to the push-to-main path | `cancel-in-progress: true` ensures only the latest main push is checked; wip-rescue PRs are rare (0–3), so the probe runs infrequently in practice |
| Clippy cache invalidation after rebase produces false failures | Uses the same `Swatinem/rust-cache` setup as the main CI job; rebase onto main means the cache from the latest main CI run is warm |
| Label creation race (label doesn't exist yet on first run) | Labels are managed by `labels.yml` sync; add them in Step 1 before the workflow exists |
| Workflow token permissions insufficient for PR edits | Explicitly requests `pull-requests: write` and `issues: write` permissions |

## Out of Scope

- **Mechanism B** (post-commit rebase+clippy in wip-rescue): deferred. Adds build latency to the rescue path and only catches born-stale commits, not future drift.
- **Required status check on wip PRs:** wip-rescue PRs are already draft-blocked. A required check adds merge-gate noise to all PRs for a signal that only applies to the rare wip-rescue case.
- **Auto-fix (automatic rebase + clippy fix):** too risky for unreviewed wip content. The probe surfaces the problem; the operator fixes it.
