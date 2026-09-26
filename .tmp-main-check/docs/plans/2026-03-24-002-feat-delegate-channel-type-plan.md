---
title: "feat: Promote delegate session channel_type from 'system' to 'delegate'"
type: feat
status: completed
date: 2026-03-24
---

# Promote delegate session channel_type from "system" to "delegate"

Delegation sessions (agent-to-agent, synchronous, task-scoped) currently share `channel_type = "system"` with background sessions (heartbeat, reflection, callback, skill_run). These are orthogonal concerns — promoting "delegate" to a first-class channel type makes delegation queryable at the column level without JSON metadata extraction, completing the taxonomy that already split "team" out.

## Acceptance Criteria

- [x] Delegate sessions are created with `channel_type = "delegate"` (`delegate_task.rs:246`)
- [x] Prune test uses `"delegate"` channel for delegate session entry (`db.rs` `test_prune_targets_correct_prefixes`)
- [x] Dashboard Sessions page includes "delegate" in channel filter dropdown and icon mapping (`Sessions.tsx`)
- [x] Solution doc updated to reflect new channel value (`delegate-task-session-message-persistence.md`)
- [x] Plan doc updated to reflect new channel value (`2026-03-24-001-fix-delegate-task-persist-messages-plan.md`)
- [x] All existing tests pass (`cargo test`)

## Context

**Channel taxonomy after this change:**

| Channel | Purpose | Sessions |
|---------|---------|----------|
| `cli` | Terminal user interactions | Interactive CLI sessions |
| `telegram` | Telegram bot messages | Gateway-routed sessions |
| `api` | API-originated (future) | — |
| `a2a` | Agent-to-Agent protocol | A2A JSON-RPC sessions |
| `team` | Team orchestration runs | Per-agent team sessions |
| `system` | Autonomous background tasks | Heartbeat, reflection, callback, skill_run, compaction |
| **`delegate`** | **Agent-to-agent delegation** | **`delegate-{uuid}` sessions** |

**Design decisions:**

- **No data migration**: Forward-only. Old delegate sessions retain `channel_type="system"` and age out via 7-day pruning. Pre-1.0 versioning — no backward compat needed.
- **No schema migration**: No CHECK constraint on `channel_type` column. Any string value is accepted.
- **No `load_recent_messages` change**: Delegate messages were already included (as "system") and remain included (as "delegate"). No behavioral regression.
- **No `VALID_CHANNELS` change**: That list in `prompt.rs` is for user-facing communication channels only.
- **Pruning unaffected**: Uses session ID prefix matching (`id LIKE 'delegate-%'`), not channel_type.
- **Dashboard icon**: `ArrowRightLeft` from Lucide — semantic fit for agent-to-agent delegation.

## MVP

### `crates/mika-agent/src/tools/delegate_task.rs` (line 246)

```rust
// Before
"system",
// After
"delegate",
```

### `crates/mika-agent/src/db.rs` — `test_prune_targets_correct_prefixes` (~line 7640)

```rust
// Before: all sessions created with "system"
db.create_session("delegate-task-1", "mika", "system").unwrap();
// After: delegate session uses "delegate" channel
db.create_session("delegate-task-1", "mika", "delegate").unwrap();
```

### `dashboard/src/pages/Sessions.tsx` (line 8, line 10-23)

Add `'delegate'` to `CHANNEL_TYPES` array and add `case 'delegate':` with `ArrowRightLeft` icon to `channelIcon()`.

### `docs/solutions/architecture-patterns/delegate-task-session-message-persistence.md`

Update `channel_type = "system"` reference to `channel_type = "delegate"`.

### `docs/plans/2026-03-24-001-fix-delegate-task-persist-messages-plan.md`

Update `channel_type = "system"` reference and rationale.

## Sources

- Solution doc: `docs/solutions/architecture-patterns/delegate-task-session-message-persistence.md`
- Trace ID linkage doc: `docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md`
- Observability doc: `docs/solutions/architecture-patterns/observability-request-id-session-lifecycle.md`
