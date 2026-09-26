---
status: complete
priority: p2
issue_id: "334"
tags: [code-review, security, observability]
dependencies: []
---

# Log Level Too Quiet for Disabled Skill Seeding

## Problem Statement

When `disable_bundled_skills` is true, the log message is emitted at `debug` level. In production (default `log_level = "info"`), this message is suppressed. An operator reviewing logs after an incident would have no indication that skill seeding was disabled — a security-relevant configuration that prevents handler script updates from propagating.

## Findings

- Flagged by: security-sentinel, pattern-recognition-specialist
- Location: `crates/mika-agent/src/startup.rs:33`
- Current code: `tracing::debug!("bundled skill seeding disabled by config");`
- The setting prevents security fixes in handler scripts (shell-exec, file-reader) from being deployed

## Proposed Solutions

### Option A: Elevate to `warn!` level
- **Pros:** Visible in all production deployments, clearly signals non-default behavior
- **Cons:** May be noisy for developers who intentionally use this setting
- **Effort:** Small (one-line change)
- **Risk:** None

### Option B: Use `info!` level
- **Pros:** Visible in production, less alarming than warn
- **Cons:** Easier to overlook in noisy logs
- **Effort:** Small
- **Risk:** None

## Recommended Action

Option A — `warn!` level is appropriate for a security-relevant config override.

## Acceptance Criteria

- [ ] Log level for "bundled skill seeding disabled" is `warn` or `info`, not `debug`
- [ ] Message is visible in default production log configuration
