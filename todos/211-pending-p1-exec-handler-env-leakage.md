---
status: pending
priority: p1
issue_id: "211"
tags: [code-review, security, skills-system]
dependencies: []
---

# Exec Handler Inherits Full Process Environment

## Problem Statement
The exec handler in `handler.rs` spawns child processes via `tokio::process::Command::new(command)` without calling `.env_clear()`. This means child processes inherit the full environment of the Mika process, including `MIKA_ANTHROPIC_API_KEY` and any other secrets. A malicious or compromised exec skill could exfiltrate API keys.

## Findings
- Location: `crates/mika-agent/src/skills/handler.rs:46`
- `Command::new(command).args(args).arg(tool_name).env("MIKA_TOOL_INPUT", &input_json)` — adds env var but doesn't clear existing ones
- The Anthropic API key and internal tokens are in the process environment
- Any exec skill gets full access to all secrets

## Proposed Solutions

### Option 1: Add .env_clear() before .env()
- **Pros**: Simple, effective, minimal change
- **Cons**: May break legitimate skills that need PATH or other system vars
- **Effort**: Small
- **Risk**: Low (can allowlist PATH, HOME if needed)

### Option 2: Allowlist specific safe env vars
- **Pros**: Secure but still functional for skills needing PATH
- **Cons**: Slightly more code
- **Effort**: Small
- **Risk**: Low

## Recommended Action
Option 2 — `.env_clear()` then re-add only safe vars (PATH, HOME, MIKA_TOOL_INPUT).

## Technical Details
- **Affected Files**: `crates/mika-agent/src/skills/handler.rs`
- **Related Components**: Exec skill handler, security
- **Database Changes**: No

## Acceptance Criteria
- [ ] Exec handler clears env before spawning
- [ ] Only safe vars (PATH, HOME, MIKA_TOOL_INPUT) passed to child
- [ ] Tests verify secrets are not inherited

## Work Log

### 2026-02-25 - Created from code review
**By:** Claude Code Review
**Actions:** Finding identified by security-sentinel agent

## Resources
- OWASP: Environment Variable Exposure
