---
status: pending
priority: p2
issue_id: "741"
tags: [code-review, architecture, performance]
dependencies: []
---

# Consolidate duplicate GitHubApp::from_settings() calls

## Problem Statement

`GitHubApp::from_settings()` is called multiple times in `server/mod.rs`, `delegate_task.rs`, and `TeamEngine`, creating independent `Arc<GitHubApp>` instances each with its own token cache. This means:
- Per-agent duplicate token exchanges on cold starts
- Per-delegation PEM re-parsing (base64 decode + RSA parse)
- Cache fragmentation (N agents = N independent caches for the same installation_id)

## Findings

- `server/mod.rs` lines 271 and 298: Two `from_settings` calls in `init_agent()` — one for TaskDispatcher, one for AgentState
- `server/mod.rs` line 536: Global AppState also calls `from_settings`
- `delegate_task.rs` line 263: Fresh `from_settings` per delegation
- `teams/engine.rs`: `TeamEngine::new()` and `resume()` each call `from_settings`

## Proposed Solutions

### Option A: Share single Arc from server init (Recommended)
- Create one `Arc<GitHubApp>` in server startup, clone into all AgentState and TaskDispatcher instances
- **Pros:** Single cache, minimal code change
- **Cons:** None
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Only one `GitHubApp::from_settings()` call per agent in server init
- [ ] TaskDispatcher receives cloned Arc, not freshly constructed instance
- [ ] TeamEngine and delegate_task receive Arc from caller, not from settings
