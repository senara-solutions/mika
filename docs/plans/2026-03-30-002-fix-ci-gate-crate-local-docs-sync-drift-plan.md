---
title: "fix: add CI gate for crate-local docs sync drift"
type: fix
status: completed
date: 2026-03-30
issue: "#320"
---

# fix: add CI gate for crate-local docs sync drift

## Overview

`scripts/sync-agent-docs.sh` copies `docs/` → `crates/mika-agent/docs/` for crates.io publishing, but has no CI enforcement. 80 PRs merged over 15 days without syncing. Add a CI job that detects drift and fails the PR.

## Problem Statement

The crate-local doc copies at `crates/mika-agent/docs/` exist as fallbacks for crates.io installs (where the workspace root `docs/` is not available). `build.rs` tries workspace root first, falls back to crate-local. The sync script must be run manually before `cargo publish`, and there's no automated check.

Additionally, the sync script has a pre-existing bug: it's missing `task-system.md` which is listed in both `build.rs` and already present in `crates/mika-agent/docs/`.

## Proposed Solution

1. Fix the sync script to include `task-system.md`
2. Add cross-reference comments between `build.rs` and `sync-agent-docs.sh`
3. Add a new `docs-sync` CI job to `ci.yml` that runs the sync script and checks for diffs
4. Run the sync to ensure current state is clean

## Acceptance Criteria

- [x] `scripts/sync-agent-docs.sh` includes `task-system.md` in its `DOCS` array
- [x] Cross-reference comments in `build.rs` and `sync-agent-docs.sh` point to each other
- [x] New `docs-sync` CI job in `.github/workflows/ci.yml`
- [x] Job runs on both push-to-main and pull_request (matching `check` job pattern)
- [x] Job skips `release-plz-*` branches (matching `pipeline-artifacts` pattern)
- [x] Job runs `bash scripts/sync-agent-docs.sh` then `git diff --exit-code crates/mika-agent/docs/`
- [x] Failure message shows the diff and instructs developer to run the sync script
- [x] Actions pinned to commit SHAs (reuse existing checkout SHA)
- [x] All crate-local docs are currently in sync after the PR (no pre-existing drift)

## Technical Approach

### Files to modify

1. **`scripts/sync-agent-docs.sh`** — Add `task-system.md` to `DOCS` array; add cross-ref comment
2. **`crates/mika-agent/build.rs`** — Add cross-ref comment to `DOCS` constant
3. **`.github/workflows/ci.yml`** — Add `docs-sync` job
4. **`crates/mika-agent/docs/`** — Re-sync any drifted files (by running the script)

### CI job structure

```yaml
docs-sync:
  name: Docs Sync
  runs-on: ubuntu-22.04
  if: >-
    github.event_name == 'push' ||
    (github.event_name == 'pull_request' &&
     !startsWith(github.head_ref, 'release-plz-'))
  steps:
    - uses: actions/checkout@<pinned-sha>  # v6
    - name: Check crate-local docs are in sync
      run: |
        bash scripts/sync-agent-docs.sh
        if ! git diff --exit-code crates/mika-agent/docs/; then
          echo ""
          echo "ERROR: crate-local docs are out of sync with docs/"
          echo "Run: bash scripts/sync-agent-docs.sh"
          echo "Then commit the updated files."
          exit 1
        fi
```

### Design decisions

- **Separate job** (not embedded in `pipeline-artifacts`): docs-sync is a build correctness concern, not a dev-workflow concern. Independent status check is clearer.
- **Lightweight**: No Rust toolchain needed — just bash + git. Fast runner allocation.
- **Runs on push-to-main too**: Catches drift from merge conflict resolution (advisory).
- **Uses sync script as check mechanism**: Single source of truth — the gate uses the same logic as manual sync.
- **`memory-classification.md` intentionally excluded**: It's in `docs/` but not in `build.rs` or the sync script. Not user-facing embedded doc — internal classification notes.

### Pre-existing discrepancies to fix

| File | `build.rs` | sync script | crate-local | Action |
|------|-----------|-------------|-------------|--------|
| `task-system.md` | ✅ | ❌ | ✅ | Add to sync script |
| `memory-classification.md` | ❌ | ❌ | ❌ | Intentionally excluded |

## Sources

- Related issue: #320
- Existing CI pattern: `.github/workflows/ci.yml` (`pipeline-artifacts` job)
- Sync script: `scripts/sync-agent-docs.sh`
- Build-time copy: `crates/mika-agent/build.rs`
- Documented learning: `docs/solutions/integration-issues/openapi-spec-drift-missing-utoipa-annotations.md`
- Documented learning: `docs/solutions/integration-issues/adding-get-documentation-topic.md`
- CI conventions: `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md`
