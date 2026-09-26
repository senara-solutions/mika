---
status: complete
priority: p3
issue_id: 699
tags: [code-review, tui, rewind, tech-debt]
dependencies: []
---

# Rewind message reload diverges from startup loader role mapping

## Problem Statement

The post-rewind message reload loop (handlers.rs ~line 946) and the startup message loader (chat.rs ~line 445) handle roles differently:

| Role | Startup (chat.rs) | Rewind (handlers.rs) |
|------|-------------------|---------------------|
| `user` | ChatRole::User (with stale-framing filter) | ChatRole::User (no filter) |
| `assistant` | ChatRole::Assistant | ChatRole::Assistant |
| `tool_result` | ChatRole::System (with callback label) | **Skipped** |
| `system` | **Skipped** | ChatRole::System |
| `channel` field | Populated from msg.channel_type | Hardcoded `None` |

This drift was pre-existing in the cross-session rewind path. The recent fix unified same/cross-session paths but did not address the drift.

## Findings

- Pattern recognition agent identified 5 specific discrepancies
- Learnings researcher found `rewind-context-marker-confabulation-prevention.md` confirming `system` role should be included for rewind markers
- The `system` mapping in rewind is actually correct per the rewind marker doc; startup may be the one with the gap
- `tool_result` callback results won't display after rewind (minor — they show as system labels anyway)

## Proposed Solutions

### Option 1: Extract shared helper function (Recommended for long-term)
- Extract `load_display_messages_from_db()` used by both chat.rs startup and rewind handler
- **Pros:** Eliminates drift permanently, single source of truth
- **Cons:** Moderate effort, both paths have slightly different needs (stale framing filter, etc.)
- **Effort:** Medium
- **Risk:** Low

### Option 2: Leave as-is (Acceptable for now)
- The cross-session path has worked this way without complaints
- Differences are minor visual inconsistencies, not data loss
- **Pros:** No additional changes to a working fix
- **Cons:** Drift may worsen over time
- **Effort:** None
- **Risk:** Low

## Technical Details

- **Files:** `crates/mika-cli/src/tui/commands/handlers.rs`, `crates/mika-cli/src/commands/chat.rs`
- **Key function:** `load_recent_messages(20)` — both call sites use same limit

## Acceptance Criteria

- [ ] Both startup and rewind reload paths handle the same set of roles
- [ ] `tool_result` messages display consistently after rewind and after restart
- [ ] `channel` field populated in rewind reload path
