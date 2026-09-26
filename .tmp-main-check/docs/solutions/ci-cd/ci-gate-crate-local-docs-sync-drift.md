---
title: "CI gate for crate-local docs sync drift"
category: ci-cd
date: 2026-03-30
tags: [ci, docs-sync, build-rs, crates-io, drift-detection]
issue: "#320"
module: scripts, .github/workflows
---

# CI Gate for Crate-Local Docs Sync Drift

## Problem

`scripts/sync-agent-docs.sh` copies `docs/` → `crates/mika-agent/docs/` for crates.io publishing, but had no CI enforcement. Over a 15-day window (~80 PRs), the crate-local copies drifted from the canonical `docs/` source. If `cargo publish` had run during that window, stale documentation would have been shipped.

Additionally, the sync script was missing `task-system.md` — a file listed in both `build.rs` and already present in `crates/mika-agent/docs/`.

## Root Cause

The sync step was entirely manual with no automated verification. The `build.rs` and sync script maintained independent file lists (`DOCS` constant vs `DOCS` array) with no cross-reference or CI check.

## Solution

### 1. Fixed the sync script

Added `task-system.md` to the `DOCS` array in `scripts/sync-agent-docs.sh` to match `build.rs`.

### 2. Added cross-reference comments

Both `crates/mika-agent/build.rs` and `scripts/sync-agent-docs.sh` now reference each other:

```rust
// Keep in sync with scripts/sync-agent-docs.sh DOCS array.
// CI enforces this via the docs-sync job in .github/workflows/ci.yml.
const DOCS: &[&str] = &[ ... ];
```

```bash
# Keep DOCS list in sync with crates/mika-agent/build.rs DOCS constant.
# CI enforces this via the docs-sync job in .github/workflows/ci.yml.
```

### 3. Added CI job

New `docs-sync` job in `.github/workflows/ci.yml`:

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
          echo "::error::Crate-local docs are out of sync with docs/"
          echo "Run: bash scripts/sync-agent-docs.sh"
          exit 1
        fi
```

Key design decisions:
- **Separate job** (not embedded in `pipeline-artifacts`): build correctness concern vs dev-workflow concern
- **Lightweight**: No Rust toolchain — just bash + git
- **Excludes release-plz branches**: Matches `pipeline-artifacts` pattern
- **Runs on push-to-main too**: Catches drift from merge conflict resolution
- **Shows the diff**: Developer sees exactly which files diverged

## Prevention

1. **CI gate catches all drift automatically** — no manual step to remember
2. **Cross-reference comments** in both `build.rs` and `sync-agent-docs.sh` remind developers the lists must match
3. When adding a new doc topic, the CI gate will fail if the sync script isn't updated (see also: `docs/solutions/integration-issues/adding-get-documentation-topic.md`)

## Related

- `docs/solutions/integration-issues/openapi-spec-drift-missing-utoipa-annotations.md` — Same two-copy sync pattern for OpenAPI specs
- `docs/solutions/integration-issues/adding-get-documentation-topic.md` — Checklist includes crate-local copy step
- `docs/solutions/ci-cd/rust-workspace-release-plz-github-actions.md` — CI conventions (action pinning, release-plz exclusion)
