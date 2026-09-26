---
status: complete
priority: p3
issue_id: 431
tags: [code-review, quality, duplication]
dependencies: []
---

# Log level resolution duplicated between team and agent branches in main.rs

## Problem Statement

The team mode branch in `crates/mika-cli/src/main.rs` (lines 33-46) duplicates the log level resolution logic from the agent mode branch (~15 lines). Both parse TOML config and check environment variables to determine the log level.

## Findings

- Source: pattern-recognition-specialist, code-simplicity-reviewer
- ~12 lines of duplicated TOML config parsing and env var checking
- Agent mode falls back from agent config to global config; team mode only needs global config

## Proposed Solutions

### Option A: Extract `resolve_log_level()` helper (Recommended)
- `fn resolve_log_level(config_paths: &[&Path]) -> String`
- Both branches call with their respective config file paths
- **Pros:** DRY, ~12 LOC saved
- **Effort:** Small

## Acceptance Criteria

- [ ] Log level resolution exists in one place
- [ ] Both team and agent branches use the shared helper
