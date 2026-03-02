---
status: complete
priority: p3
issue_id: 390
tags: [code-review, quality, simplification]
dependencies: [389]
---

# Remove grep fallback from shell-exec run.sh

## Problem Statement

The grep fallback in `shell-exec/handlers/run.sh` (lines 14-17) preserves the exact bug that PR #47 is fixing. When triggered, it silently reintroduces the double-quote truncation problem. The `github/handlers/run.sh` and `file-reader/handlers/read.sh` already use `jq` unconditionally with no fallback, and `Dockerfile.agent` installs `jq` as a runtime dependency.

## Findings

- **Source**: code-simplicity-reviewer, PR #47
- **Key argument**: The fallback is a YAGNI violation — it adds 7 lines of code to handle a scenario (jq not installed) that already causes hard failures in other handlers. The fallback preserves the exact bug being fixed, silently.
- **Estimated LOC reduction**: 7 lines (~25% of run.sh)

## Proposed Solutions

### Option A: Remove fallback, use jq directly (Recommended)
Match `github/run.sh` and `file-reader/read.sh` patterns.

- **Pros**: 7 lines removed, eliminates known-broken code path, consistent with other handlers
- **Cons**: Hard failure if jq absent (but consistent with github/file-reader handlers)
- **Effort**: Small
- **Risk**: None

## Technical Details

- **Affected file**: `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`

## Acceptance Criteria

- [ ] run.sh uses jq directly with no grep fallback
- [ ] Handler fails cleanly with an error if jq is not installed
- [ ] All tests pass
