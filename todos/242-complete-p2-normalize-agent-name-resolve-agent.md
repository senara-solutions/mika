---
status: complete
priority: p2
issue_id: "242"
tags: [code-review, correctness, normalization]
dependencies: []
---

# Agent name not normalized in `resolve_agent` (server)

## Problem Statement

The server's `resolve_agent` does a plain `HashMap::get` with no normalization. The CLI normalizes names via `normalize_agent_name` (trim + lowercase). If the gateway sends `"agent": "Main"`, the server returns 404 even though "main" exists.

## Findings

- **Source:** Agent-Native Reviewer
- **File:** `crates/mika-agent/src/server/state.rs:48-55`

## Proposed Solutions

Apply `agent::normalize_agent_name` in `resolve_agent` before the HashMap lookup:

```rust
let effective = if name.is_empty() { &self.default_agent } else { name };
let normalized = agent::normalize_agent_name(effective);
self.agents.get(&normalized)
```

## Acceptance Criteria

- [ ] `resolve_agent` normalizes the name before lookup
- [ ] `"Main"` resolves to `"main"` agent

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Case-sensitivity mismatch between CLI and server |
