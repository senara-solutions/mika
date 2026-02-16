---
status: complete
priority: p2
issue_id: "080"
tags: [code-review, performance]
dependencies: []
---

# Eliminate unnecessary clones in agent loop

## Problem Statement
The agent loop clones `response.content` (potentially large JSON) on every tool-use step and uses `ref` + `.clone()` on the response string unnecessarily. Multiple per-turn allocations that can be avoided.

## Findings
- agent.rs:161,385 — `response.content.clone()` in tool-use loop (up to 10 iterations)
- agent.rs:47-53 — `Ok(ref response) => { Ok(response.clone()) }` unnecessary ref+clone
- agent.rs:100,341 — `tools.definitions()` rebuilds 8 tool defs per turn
- agent.rs:79,299 — `std::fs::read_to_string` for soul.md on every turn (should cache at startup)

## Proposed Solutions
### Option 1: Move instead of clone, cache static data
- Take ownership of `response.content` instead of cloning
- Remove `ref` binding, return owned String directly
- Cache soul.md/identity.toml at startup, pass via params
- Cache tool definitions in ToolRegistry
**Effort:** 45 minutes | **Risk:** Low

## Acceptance Criteria
- [ ] No `.clone()` on response.content in tool-use loop
- [ ] No unnecessary ref+clone on response string
- [ ] soul.md and identity loaded once at startup
- [ ] Tool definitions cached in ToolRegistry
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
