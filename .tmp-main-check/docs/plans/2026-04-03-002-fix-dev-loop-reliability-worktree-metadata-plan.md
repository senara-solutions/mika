---
title: "fix: dev loop reliability — worktree hooks and metadata cleanup"
type: fix
status: completed
date: 2026-04-03
issues: [406, 398, 385]
---

# Dev Loop Reliability — Worktree Hooks and Metadata Cleanup

## Overview

Two reliability improvements for the autonomous dev loop: (1) ensure lefthook pre-commit hooks run in worktrees created by the `/mika` pipeline, and (2) normalize `cost_usd` callback metadata to numeric JSON type so the dashboard can display it.

## Task 1: lefthook install in worktrees (#398)

### Problem

The `/mika` pipeline creates git worktrees but never runs `lefthook install` in them. Lefthook hooks (fmt, clippy, secrets scan — 7 checks total) are silently bypassed. Evidence: PR #396 was created with `cargo fmt` violations and `cargo clippy` errors, requiring two manual fix commits.

### Fix

Add `lefthook install` after worktree creation in two files:

1. **`mika/.claude/commands/mika.md`** — per-repo pipeline (after step 5 "cd into the worktree")
2. **`mika-platform/.claude/commands/mika.md`** — meta-repo dispatcher (after worktree creation in self-targeting pipeline)

**Failure policy:** Non-blocking. Use `command -v lefthook >/dev/null 2>&1 && lefthook install` to silently skip when lefthook is not installed (Docker containers, CI environments may lack it).

**Idempotency:** `lefthook install` is idempotent — safe to run on fresh or reused worktrees.

**Existing worktree detection:** When step 3 detects an already-existing worktree (`CREATED_WORKTREE=false`), still run `lefthook install` before the pipeline starts. An existing worktree may lack hooks.

### Files to change

| File | Change |
|------|--------|
| `.claude/commands/mika.md` | Add `lefthook install` after worktree cd (line ~26), and after existing worktree detection (line ~20) |

### Out of scope (follow-up)

- `mika-skills/claude-pilot/handlers/run.sh` — autonomous handler creates worktrees too, but lives in a different repo. File a follow-up issue.
- `mika-platform/.claude/commands/mika.md` — meta-repo dispatcher. Separate repo, separate PR.

## Task 2: normalize cost_usd to numeric type (#385)

### Problem

In `extract_callback_fields()` (`dispatcher.rs` line 901), `cost_usd` is stored as a JSON string (`"7.07"`) while `turns` and `duration_ms` are stored as numbers. Downstream, `dashboard_dev_runs.rs` line 62 calls `as_f64()` which returns `None` for strings — **cost_usd is always null in the dashboard**.

### Fix — Write path

```rust
// dispatcher.rs, extract_callback_fields(), line 899-901
// BEFORE:
if let Some(cap) = RE_COST.captures(result) {
    map.insert("cost_usd".into(), serde_json::Value::String(cap[1].into()));
}

// AFTER:
if let Some(cap) = RE_COST.captures(result) {
    if let Some(n) = cap[1].parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
        map.insert("cost_usd".into(), serde_json::Value::Number(n));
    }
}
```

Pattern matches `turns` and `duration_ms` handling. `from_f64()` returns `None` for NaN/Infinity (impossible from the regex `r"Cost:\s*\$([0-9]+(?:\.[0-9]+)?)"` but handled defensively).

### Fix — Read path (backward compatibility)

Update `dashboard_dev_runs.rs` line 62 to parse both string and number types for historical data:

```rust
// BEFORE:
cp.get("cost_usd").and_then(|v| v.as_f64()),

// AFTER:
cp.get("cost_usd").and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))),
```

This fixes both old (string) and new (number) data without a migration.

### Tests to update

5 assertions in `dispatcher.rs` that compare `cost_usd` as string:

| Test | Line | Current | Expected |
|------|------|---------|----------|
| `test_extract_success_format` | ~1151 | `assert_eq!(cp["cost_usd"], "7.07")` | `assert_eq!(cp["cost_usd"], 7.07)` |
| `test_extract_pipeline_failure_format` | ~1166 | `assert_eq!(cp["cost_usd"], "3.50")` | `assert_eq!(cp["cost_usd"], 3.5)` |
| `test_extract_partial_fields` | ~1178 | `assert_eq!(cp["cost_usd"], "1.23")` | `assert_eq!(cp["cost_usd"], 1.23)` |
| `test_extract_unknown_values_skipped` | ~1196 | `assert!(cp.get("cost_usd").is_none())` | No change |
| `test_try_extract_callback_metadata_writes_to_parent` | ~1291 | `assert_eq!(cp["cost_usd"], "7.07")` | `assert_eq!(cp["cost_usd"], 7.07)` |

### Files to change

| File | Change |
|------|--------|
| `crates/mika-agent/src/task_engine/dispatcher.rs` | Fix `extract_callback_fields()` cost_usd to numeric; update 4 test assertions |
| `crates/mika-agent/src/server/dashboard_dev_runs.rs` | Add dual-parse for backward compatibility |

### Out of scope (follow-up)

- `mika-skills/self-dev/system_prompt.md` — prompt example shows `"cost_usd": "..."` (string). Should be updated to show numeric value and note engine pre-writes these fields. Separate repo.

## Acceptance Criteria

- [x] `lefthook install` runs after worktree creation in `mika/.claude/commands/mika.md`
- [x] `lefthook install` runs for detected existing worktrees too
- [x] lefthook failure is non-blocking (silent skip if binary not found)
- [x] `cost_usd` stored as JSON number in callback metadata
- [x] Dashboard reads both string and number `cost_usd` (backward compat)
- [x] All tests pass (`cargo test -p mika-agent`)

## Sources

- Issue #406 (umbrella), #398 (lefthook), #385 (cost_usd)
- Learnings: `docs/solutions/build-errors/lefthook-not-installed-worktree-ci-failure.md`
- Learnings: `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`
- `crates/mika-agent/src/task_engine/dispatcher.rs:878` — `extract_callback_fields()`
- `crates/mika-agent/src/server/dashboard_dev_runs.rs:62` — `DevRunResponse` read path
- `.claude/commands/mika.md:14-26` — worktree setup section
