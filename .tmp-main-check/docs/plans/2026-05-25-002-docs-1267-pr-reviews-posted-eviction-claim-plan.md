---
issue: mika#1267
type: docs
title: Fix pr_reviews_posted eviction claim in CLAUDE.md
date: 2026-05-25
---

# Plan: Fix pr_reviews_posted eviction claim in CLAUDE.md (mika#1267)

## Problem

`crates/mika-agent/CLAUDE.md` line 28 (Post-Conditions § 3b) states:

> Entries evicted at `end_session()` callsites.

This is inaccurate. `end_session()` (`db.rs:6244`) only updates the `sessions` table timestamp — it does not touch the in-memory `DashMap`. The actual eviction happens at 5 dispatch/delegate session-teardown sites.

## Evidence (verified)

- **Write site:** `builtin_handlers.rs:1993` — `map.entry(ctx.session_id).or_default().insert(key)`, conversation mode only.
- **Eviction sites (5):**
  - `delegate_task.rs:351` — `map.remove(&session_id)`
  - `dispatcher.rs:282` — skill task dispatch
  - `dispatcher.rs:516` — defer chain
  - `dispatcher.rs:763` — heartbeat
  - `dispatcher.rs:922` — reflection
- **`end_session()`:** `db.rs:6244` / `async_db.rs` — DB-only, no `DashMap` access.
- **Gap:** Conversation-mode sessions that post a review are not covered by any of the 5 eviction sites → bounded slow leak (one entry per such session, session-scoped keys prevent false-dedup).

## Deliverable

Replace the inaccurate sentence in `crates/mika-agent/CLAUDE.md` line 28 within the 3b description.

### Before

> Entries evicted at `end_session()` callsites.

### After

> Entries evicted at the 5 dispatch/delegate session-teardown sites (`delegate_task.rs` + 4 `dispatcher.rs` callsites). Conversation-mode sessions that post a review are not covered — a known slow, bounded leak (tracked as coherence-debt DEBT-E in `mika-platform/docs/coherence-debt.md`).

## Scope

- **In scope:** One sentence replacement in `crates/mika-agent/CLAUDE.md`.
- **Out of scope:** Fixing the underlying leak (tracked separately as DEBT-E). No code changes.

## AC tie-backs

- AC1: The inaccurate `end_session()` claim is replaced with the accurate eviction-site description.
- AC2: The DEBT-E cross-reference is preserved per the issue's requested correction text.

## Risk

None — doc-only change, no behavioral impact.
