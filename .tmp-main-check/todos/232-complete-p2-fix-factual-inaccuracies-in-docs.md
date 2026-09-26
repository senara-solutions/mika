---
status: complete
priority: p2
issue_id: 232
tags: [code-review, documentation, accuracy]
dependencies: []
---

# Fix Factual Inaccuracies in Documentation

## Problem Statement

Several documentation files contain statements that don't match the actual codebase behavior. These inaccuracies could mislead users and operators.

## Findings

1. **deployment.md: Heartbeat status code** — States heartbeat returns 202, but the actual handler returns 200 (`Ok(Json(...))` in handlers.rs).

2. **skills.md: Restart contradiction** — One section says "Changes take effect on next message" (correct, since loader.rs reads from disk each time), but another section says "restart required." These contradict each other.

3. **configuration.md/slash-commands.md: /config handler path** — The `/config` handler checks `config/local.toml` (relative, development config), but documentation describes it as showing `~/.mika/config.toml` (user home config). The handler's path resolution may not match what's documented.

4. **slash-commands.md: /clear --all** — The `--all` flag is documented in the COMMANDS array `args_hint` but the handler function ignores the `_args` parameter entirely. Document should note this is not yet implemented, or the handler should be fixed.

## Proposed Solutions

### Solution A: Fix docs to match code (Recommended)
- Fix heartbeat status to 200 in deployment.md
- Remove "restart required" contradiction in skills.md
- Clarify /config path behavior
- Add "(not yet implemented)" note for /clear --all
- **Effort:** Small
- **Risk:** Low

### Solution B: Fix code to match docs
- Change heartbeat to return 202
- Implement /clear --all in handler
- **Effort:** Medium
- **Risk:** Medium — changes behavior

## Acceptance Criteria

- [ ] No factual contradictions between docs and code
- [ ] All status codes match actual responses
- [ ] /clear --all either works or is documented as planned

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from review | Architecture strategist found 4 inaccuracies |
