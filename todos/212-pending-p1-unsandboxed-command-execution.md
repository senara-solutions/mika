---
status: pending
priority: p1
issue_id: "212"
tags: [code-review, security, skills-system]
dependencies: []
---

# Unsandboxed Arbitrary Command Execution via Exec Skills

## Problem Statement
The exec handler executes any command specified in a skill's `skill.toml` without validation, sandboxing, or path restrictions. A user-writable `~/.mika/skills/` directory means any process with write access to the user's home directory can plant a malicious skill that executes arbitrary commands with the user's full privileges.

## Findings
- Location: `crates/mika-agent/src/skills/handler.rs:46`
- No path validation on command (could be `/bin/rm -rf /`)
- No allowlist/denylist for commands
- Skills directory is user-writable by design
- `tool_name` is appended as a shell argument without sanitization (line 48)
- Combined with env leakage (#211), this is a full privilege escalation vector

## Proposed Solutions

### Option 1: Remove exec handler entirely (YAGNI)
- **Pros**: Eliminates attack surface completely; no exec skills exist today
- **Cons**: Must re-implement when exec skills are actually needed
- **Effort**: Small (delete code)
- **Risk**: None (no current users)

### Option 2: Add command validation + sandboxing
- **Pros**: Keeps extensibility
- **Cons**: Complex to implement correctly (path validation, seccomp, etc.)
- **Effort**: Large
- **Risk**: Medium (hard to get sandboxing right)

### Option 3: Require explicit user approval per skill on first load
- **Pros**: User-in-the-loop security
- **Cons**: UX friction
- **Effort**: Medium
- **Risk**: Low

## Recommended Action
Option 1 — Remove exec/http handlers now. They're speculative code with zero users. Re-add with proper security when needed.

## Technical Details
- **Affected Files**: `crates/mika-agent/src/skills/handler.rs`, `crates/mika-agent/src/skills/manifest.rs`
- **Related Components**: Skills system, agent loop
- **Database Changes**: No

## Acceptance Criteria
- [ ] Exec handler removed or properly sandboxed
- [ ] Tool name validated before use as argument
- [ ] No arbitrary command execution possible from skill manifests

## Work Log

### 2026-02-25 - Created from code review
**By:** Claude Code Review
**Actions:** Finding identified by security-sentinel agent

## Resources
- Related: #211 (env leakage compounds this issue)
- OWASP: Command Injection
