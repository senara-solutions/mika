---
title: "fix: remove MIKA_INVESTIGATE_GITHUB_TOKEN fallback from agent_github_token()"
type: fix
status: active
date: 2026-03-31
issue: "#359"
---

# fix: remove MIKA_INVESTIGATE_GITHUB_TOKEN fallback from agent_github_token()

## Overview

`Settings::agent_github_token()` silently falls back from `MIKA_GITHUB_TOKEN` to `MIKA_INVESTIGATE_GITHUB_TOKEN`. This caused mika-qa to use the mika-platform fine-grained PAT (investigation-only token) for agent operations when `MIKA_GITHUB_TOKEN` was intentionally unset, resulting in a self-approval attempt on a PR.

Silent fallbacks mask configuration intent. The fix removes the fallback so the two tokens serve strictly separate purposes.

## Problem Statement

```rust
// crates/mika-common/src/config.rs:690-697
pub fn agent_github_token(&self) -> Option<&str> {
    self.github_token
        .as_deref()
        .or(self.investigate_github_token.as_deref())  // ← remove this
}
```

When `MIKA_GITHUB_TOKEN` is deliberately unset (e.g., during a token swap so agents fall back to host `gh auth`), the investigate token silently takes over for all agent operations — context injection, work item enrichment, `run_gh`, PR merge — none of which it was intended for.

## Proposed Solution

Remove the `.or()` fallback. `agent_github_token()` returns `MIKA_GITHUB_TOKEN` only. `MIKA_INVESTIGATE_GITHUB_TOKEN` remains used solely by the investigation panel (`investigate.rs`), which reads it directly.

## Acceptance Criteria

- [ ] `agent_github_token()` returns `self.github_token.as_deref()` only — no fallback
- [ ] `.env.example` updated: remove "Falls back to" comment on `MIKA_GITHUB_TOKEN`
- [ ] `mika doctor` diagnostic updated: no longer mentions fallback to investigate token
- [ ] Error message in `dashboard_dev_runs.rs` updated: remove `(or MIKA_INVESTIGATE_GITHUB_TOKEN)`
- [ ] `docs/configuration.md` updated: remove fallback documentation
- [ ] `CLAUDE.md` updated: remove "Falls back to MIKA_INVESTIGATE_GITHUB_TOKEN" from env var docs
- [ ] Solution doc `dedicated-github-token-agent-operations.md` updated to reflect new behavior
- [ ] All tests pass (`cargo test`)
- [ ] `cargo clippy` clean

## Files to Change

### Core fix (1 file)

| File | Line | Change |
|------|------|--------|
| `crates/mika-common/src/config.rs` | 690-697 | Remove `.or(self.investigate_github_token.as_deref())` from `agent_github_token()` |

### Error messages and diagnostics (2 files)

| File | Line | Change |
|------|------|--------|
| `crates/mika-cli/src/commands/doctor.rs` | ~68 | Update fallback message → "not set (agent GitHub operations disabled)" |
| `crates/mika-agent/src/server/dashboard_dev_runs.rs` | ~162 | Remove `(or MIKA_INVESTIGATE_GITHUB_TOKEN)` from error message |

### Documentation (4 files)

| File | Change |
|------|--------|
| `.env.example` | Remove "Falls back to MIKA_INVESTIGATE_GITHUB_TOKEN if not set." comment |
| `docs/configuration.md` | Remove fallback mentions |
| `CLAUDE.md` | Update env var section for `MIKA_GITHUB_TOKEN` |
| `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md` | Update to reflect no-fallback behavior |

### NOT changed (verification)

- **`crates/mika-agent/src/server/investigate.rs`** — reads `settings.investigate_github_token` directly, unaffected
- **All 10 `agent_github_token()` call sites** — no changes needed, they go through the centralized method
- **Test fixtures** — already set both tokens to `None`, unaffected
- **`crates/mika-cli/src/commands/setup.rs`** — still collects both tokens separately, unaffected

## Sources

- Issue: [#359](https://github.com/senara-solutions/mika/issues/359)
- Learnings: `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md` — documents the centralized helper and all 8+ construction sites
- Learnings: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md` — 9-layer checklist for env var changes
