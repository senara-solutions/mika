---
status: pending
priority: p3
issue_id: 647
tags: [code-review, security]
dependencies: []
---

# Handler script `unset` lists missing `MIKA_DASHBOARD_TOKEN` and `MIKA_OTLP_AUTH_HEADER`

## Problem Statement

The shell handler scripts (`github/handlers/run.sh` and
`shell-exec/handlers/run.sh`) explicitly `unset` five MIKA_* env vars as
defense-in-depth. However, two secret env vars are missing from the list:
`MIKA_DASHBOARD_TOKEN` and `MIKA_OTLP_AUTH_HEADER`.

The primary defense (executor's `scrub_mika_env_vars()` wildcard `MIKA_*`
removal) covers all vars. The handler-level `unset` is a secondary defense
layer that would matter if a handler script were invoked outside the executor.

## Findings

- `crates/mika-agent/templates/skills/github/handlers/run.sh:15` — unsets 5 vars
- `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh:13` — unsets 5 vars
- `crates/mika-agent/src/skills/executor.rs:24-30` — wildcard `MIKA_*` scrub (primary defense)
- Missing: `MIKA_DASHBOARD_TOKEN`, `MIKA_OTLP_AUTH_HEADER`

Detected by: security-sentinel agent

## Proposed Solutions

### Option A: Add missing vars to unset lists
- Add `MIKA_DASHBOARD_TOKEN MIKA_OTLP_AUTH_HEADER` to both handler scripts
- **Pros:** Complete defense-in-depth
- **Cons:** Manual maintenance as new secrets are added
- **Effort:** Small
- **Risk:** Low

### Option B: Switch to wildcard unset
- Replace explicit list with: `for var in $(env | grep ^MIKA_ | cut -d= -f1); do unset "$var"; done`
- **Pros:** Automatically covers all current and future MIKA_* vars
- **Cons:** Slightly more complex shell, may unset non-secret vars too
- **Effort:** Small
- **Risk:** Low

## Recommended Action

Option A for consistency with existing pattern. Option B if maintenance burden grows.

## Technical Details

- **Affected files:** `crates/mika-agent/templates/skills/github/handlers/run.sh`, `crates/mika-agent/templates/skills/shell-exec/handlers/run.sh`

## Acceptance Criteria

- [ ] Both handler scripts unset all secret MIKA_* env vars
- [ ] `cargo test` passes (handler scripts are templates, no Rust compilation impact)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Pre-existing gap, not introduced by this change |

## Resources

- Security sentinel agent review
- Executor scrub: `crates/mika-agent/src/skills/executor.rs:24-30`
