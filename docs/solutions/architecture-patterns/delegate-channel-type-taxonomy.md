---
title: "Promote delegate session channel_type from 'system' to 'delegate'"
category: architecture-patterns
date: 2026-03-24
tags: [delegate, channel_type, session, observability, dashboard]
severity: low
---

# Promote delegate session channel_type from "system" to "delegate"

## Problem

Delegate sessions (agent-to-agent, synchronous, task-scoped) used `channel_type = "system"`, conflating them with autonomous background sessions (heartbeat, reflection, callback, skill_run). This made delegate sessions indistinguishable from background tasks in dashboard queries and filters without cross-referencing JSON metadata.

## Root Cause

When delegate session persistence was first implemented (see `delegate-task-session-message-persistence.md`), `"system"` was chosen to match the silent dispatcher convention. The metadata already carried `"trigger": "delegate"`, but `channel_type` — the column-level observability signal — was not differentiated.

## Solution

Changed `channel_type` from `"system"` to `"delegate"` in `delegate_task.rs:246` (`create_session_with_parent` call). Updated the dashboard `CHANNEL_TYPES` filter array and `channelIcon()` to include `"delegate"` with an `ArrowRightLeft` icon.

**Key files:**
- `crates/mika-agent/src/tools/delegate_task.rs:246` — one string literal
- `dashboard/src/pages/Sessions.tsx` — filter + icon
- `crates/mika-agent/src/db.rs` — test `test_prune_targets_correct_prefixes`

**What did NOT change (and why):**
- **Pruning SQL** — uses session ID prefix matching (`id LIKE 'delegate-%'`), not channel_type
- **`unified_timeline` VIEW** — doesn't filter by channel
- **`VALID_CHANNELS` in prompt.rs** — user-facing channels only (cli, telegram, whatsapp, api)
- **`load_recent_messages`** — excludes only `'team'`; delegate messages were included before and remain included
- **No schema migration** — `channel_type` has no CHECK constraint; any string value is accepted
- **No data backfill** — old delegate sessions retain `"system"` and age out via 7-day pruning

## Prevention

The channel taxonomy is now: `cli`, `telegram`, `api`, `a2a`, `team`, `system`, `delegate`. When adding a new session type, check whether it belongs under `"system"` or warrants its own channel_type. The rule: if the session origin is semantically distinct and users would want to filter for it independently, give it a dedicated channel.

## Related

- `delegate-task-session-message-persistence.md` — the original session persistence solution (updated)
- `trace-id-structural-linkage-delegate-silent-callback.md` — trace_id propagation across delegate boundaries
- `observability-request-id-session-lifecycle.md` — session lifecycle rules
