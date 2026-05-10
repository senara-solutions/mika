# Plan: Unify docs-only exemption — extend verify-pipeline.sh to honor pipeline-exempt label

**Issue:** mika issue#1067
**Type:** ops
**Date:** 2026-05-11
**Option:** A (make CI honor the label)

## Problem

Two inconsistent escape mechanisms exist for docs-only PRs:
1. **mika-qa skill** (post-#1065): `pipeline-exempt` label on the PR → QA approves
2. **CI verify-pipeline-artifacts**: `Pipeline-Exempt: docs-only — <reason>` commit trailer → CI passes

Operator must apply BOTH for autonomous docs-only PRs to merge. Empirically confirmed on mika#1062 (2026-05-10).

## Solution

Extend `scripts/verify-pipeline.sh` to read PR labels from `GITHUB_EVENT_PATH` (the event payload JSON file, always present in GitHub Actions `pull_request` events) via `jq`. If the `pipeline-exempt` label is present, bypass the **docs-only** rejection path only. The code-only rejection path remains unconditional — consistent with mika#1064's source-change guard in qa-review.

No `gh` CLI or `GITHUB_TOKEN` dependency. No `ci.yml` change needed. Single file change.

The commit trailer remains the fallback for:
- Local runs (no PR context / no `GITHUB_EVENT_PATH`)
- Backward compatibility with existing workflow

## Pinned Source

### Trailer scan (lines 78–87)

```bash
# Exempt trailers: scan commit messages in base..HEAD
COMMIT_BODIES=$(git log --format=%B "${MERGE_BASE}..HEAD" 2>/dev/null || true)
EXEMPT_DOCS_ONLY=0
EXEMPT_CODE_ONLY=0
if echo "$COMMIT_BODIES" | grep -qE '^Pipeline-Exempt: docs-only(\s.*)?$'; then
  EXEMPT_DOCS_ONLY=1
fi
if echo "$COMMIT_BODIES" | grep -qE '^Pipeline-Exempt: code-only(\s.*)?$'; then
  EXEMPT_CODE_ONLY=1
fi
```

### Docs-only rejection path (lines 89–98)

```bash
if [[ -n "$DOCS_BUCKET" && -z "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_DOCS_ONLY" == "1" ]]; then
    echo "warn: docs-only PR allowed by Pipeline-Exempt: docs-only trailer" >&2
  else
    echo "REJECT: docs-only PR: plan/solution present but no source changes" >&2
    echo "        Add 'Pipeline-Exempt: docs-only — <reason>' trailer to a commit" >&2
    echo "        if this docs-only ship is intentional (e.g. standalone /ce:compound)." >&2
    ERRORS=$((ERRORS + 1))
  fi
fi
```

### Code-only rejection path (lines 100–109)

```bash
if [[ -z "$DOCS_BUCKET" && -n "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_CODE_ONLY" == "1" ]]; then
    echo "warn: code-only PR allowed by Pipeline-Exempt: code-only trailer" >&2
  else
    echo "REJECT: code-only PR: source changes present but no plan/solution doc" >&2
    echo "        Add 'Pipeline-Exempt: code-only — <reason>' trailer to a commit" >&2
    echo "        if this code-only ship is intentional." >&2
    ERRORS=$((ERRORS + 1))
  fi
fi
```

### CI workflow step (`.github/workflows/ci.yml`)

```yaml
  pipeline-artifacts:
    name: Pipeline Artifacts
    runs-on: ubuntu-22.04
    if: >-
      github.event_name == 'pull_request' &&
      !startsWith(github.head_ref, 'release/') &&
      !startsWith(github.head_ref, 'release-please--')
    steps:
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd  # v6
        with:
          fetch-depth: 0
      - name: Verify pipeline artifacts
        run: bash scripts/verify-pipeline.sh origin/main
```

## Changes

### 1. `scripts/verify-pipeline.sh` — add label check from event payload

**After** the existing commit-trailer scan (line 87), add a label-based exemption block that reads directly from the GitHub Actions event payload file:

```bash
# Label-based exemption: read PR labels from GitHub Actions event payload
# GITHUB_EVENT_PATH is always set in GitHub Actions; contains the full event JSON.
# For pull_request events, labels are at .pull_request.labels[].name.
# No gh CLI or GITHUB_TOKEN needed — reads a local file.
EXEMPT_LABEL_DOCS=0
if [[ -n "${GITHUB_EVENT_PATH:-}" ]]; then
  if jq -e '.pull_request.labels[]? | select(.name == "pipeline-exempt")' "$GITHUB_EVENT_PATH" >/dev/null 2>&1; then
    EXEMPT_LABEL_DOCS=1
  fi
fi
```

**Modify the docs-only rejection block** to also check `EXEMPT_LABEL_DOCS`:

```bash
if [[ -n "$DOCS_BUCKET" && -z "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_DOCS_ONLY" == "1" || "$EXEMPT_LABEL_DOCS" == "1" ]]; then
    if [[ "$EXEMPT_LABEL_DOCS" == "1" ]]; then
      echo "warn: docs-only PR allowed by pipeline-exempt label" >&2
    else
      echo "warn: docs-only PR allowed by Pipeline-Exempt: docs-only trailer" >&2
    fi
  else
    echo "REJECT: docs-only PR: plan/solution present but no source changes" >&2
    echo "        Apply the 'pipeline-exempt' label to the PR (preferred), or add" >&2
    echo "        'Pipeline-Exempt: docs-only — <reason>' trailer to a commit." >&2
    ERRORS=$((ERRORS + 1))
  fi
fi
```

**The code-only rejection path is NOT modified.** The `pipeline-exempt` label only bypasses the docs-only check, consistent with mika#1064's source-change guard in qa-review — a PR with source changes but no pipeline artifacts should never be exempted by label alone. If a code-only exemption is ever needed, it has its own trailer (`Pipeline-Exempt: code-only`). A separate label can be added later if warranted.

### 2. Error message update (docs-only path only)

Update the REJECT error message to mention the label as the preferred mechanism, with the trailer as fallback. (Shown in change #1 above.)

## Design decisions

### Why `GITHUB_EVENT_PATH` + `jq` instead of `gh pr view`

The `pull_request` event payload at `GITHUB_EVENT_PATH` already contains the labels array. Reading a local JSON file via `jq` is strictly superior to an API call:
- No `gh` CLI dependency at runtime
- No `GITHUB_TOKEN` requirement (eliminates the `.github/workflows/ci.yml` change entirely)
- No network round-trip
- No graceful degradation needed — if `GITHUB_EVENT_PATH` is unset, CI isn't running and the script falls through to trailer-only behavior (correct for local use)

### Why docs-only path only, not blanket exemption

The `pipeline-exempt` label bypasses ONLY the docs-only rejection path, not the code-only path. This is consistent with:
- mika#1064's source-change guard in qa-review (label is ignored when source files changed)
- The label's description in `.github/labels.yml` ("Docs-only or non-code PR exempt from pipeline gates")
- The principle that code-changing PRs should always have pipeline artifacts

Single `pipeline-exempt` label retained. Label granularity (docs-only vs. code-only sub-labels) deferred — not needed when the label only bypasses the docs-only path.

## Files modified

| File | Change |
|------|--------|
| `scripts/verify-pipeline.sh` | Add label query block from event payload + update docs-only exemption conditional + update error message |

## What does NOT change

- `.github/workflows/ci.yml` — no change needed (no `gh` or `GITHUB_TOKEN` dependency)
- The code-only rejection path — remains unconditional (trailer-only exemption)
- The `mika-platform/scripts/verify-pipeline.sh` — separate repo, out of scope. It has the same trailer-only pattern; follow-up ticket recommended after this PR validates the approach.
- The mika-qa skill (`qa-review`) — already handles `pipeline-exempt` label per #1065.
- The commit trailer mechanism — preserved for backward compatibility and local use.

## Acceptance criteria mapping

| AC | How satisfied |
|----|---------------|
| 1. `pipeline-exempt` label → CI passes without trailer | Label check via `GITHUB_EVENT_PATH` in verify-pipeline.sh |
| 2. No label AND no trailer → CI fails | Default path unchanged; `EXEMPT_LABEL_DOCS=0` when label absent |
| 3. Trailer without label → CI passes | Existing `EXEMPT_DOCS_ONLY` / `EXEMPT_CODE_ONLY` logic preserved |
| 4. E2E smoke (label only, no trailer) → both QA + CI pass | Integration of #1065 (QA) + this ticket (CI) |
| 5. Error message mentions label as preferred | Updated REJECT message text in docs-only path |

## Risks

- **`GITHUB_EVENT_PATH` absent locally:** Guarded by `[[ -n "${GITHUB_EVENT_PATH:-}" ]]`. Local runs fall through to trailer-only. Zero risk.
- **`jq` dependency:** Already required by the project (CLAUDE.md: "Host dependency: `jq` is required by all skill handler scripts"). Pre-installed on GitHub Actions ubuntu runners. Zero risk.
- **Event payload staleness:** If labels are applied AFTER the workflow triggers, the payload won't reflect them. But a `pull_request.labeled` event fires when labels change, re-triggering CI — the second run has the correct payload. Same staleness boundary as the `gh pr view` approach.

## Estimated scope

~15 lines changed in 1 file. Small, focused change.
