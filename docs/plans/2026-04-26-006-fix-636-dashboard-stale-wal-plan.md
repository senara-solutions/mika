---
title: "fix(server): dashboard reads stale SQLite WAL snapshot — sessions invisible until restart"
type: fix
status: active
date: 2026-04-26
origin: senara-solutions/mika#636
---

# Plan — fix dashboard stale WAL snapshot (mika#636)

**Issue:** [mika#636](https://github.com/senara-solutions/mika/issues/636) — `fix(server): dashboard reads stale SQLite WAL snapshot — sessions invisible until restart`
**Branch:** `fix/636/dashboard-reads-stale-sqlite-wal-snapshot`
**Type:** fix (P1 bug)
**Labels:** bug, p1-important, dashboard

## Problem

The dashboard API (`/api/v1/sessions`) returns stale data — new sessions created by other processes (e.g. `mika ask --agent mika-dev`) are invisible until mika-server is restarted. The DB has 5731 sessions; the API only sees 5705, with the newest visible session over an hour behind. Restarting mika-server makes the new sessions appear immediately.

Root cause: the server's dashboard connection is pinned to a stale WAL snapshot. **Critical architectural fact** (verified during planning): `dashboard_db` is constructed via `default_agent.db.clone()` at `crates/mika-agent/src/server/mod.rs:727-731`, which means **dashboard_db and the default agent's `db` share the same underlying Connection (same OS thread, same `AsyncDatabaseInner` Arc)**. The comment at line 725-726 confirms: "Create an unscoped dashboard DB handle that shares the same DB thread as the default agent."

Consequence: any stuck transaction on the default agent's Connection pins the dashboard's WAL snapshot too. The runtime path that produces stuck transactions: a raw `BEGIN` / `BEGIN IMMEDIATE` followed by manual `execute_batch("COMMIT")`, where an error between BEGIN and COMMIT skips the COMMIT and leaves the transaction open. The connection then sees a stale snapshot until the connection is closed (server restart).

## Approach

Two complementary fixes shipped together:

### Fix A — RAII Transactions (root-cause hygiene)

Replace raw `BEGIN` / `BEGIN IMMEDIATE` + `execute_batch("COMMIT")` patterns in `crates/mika-agent/src/db.rs` with `rusqlite::Transaction`. The `Transaction` type's `Drop` implementation runs `ROLLBACK` automatically if `commit()` is not called.

**Verified callsite inventory (per planning-time grep + line-by-line inspection):**

#### A1 — Runtime callsites (load-bearing for #636)

These run after server startup and can leak transactions during normal operation. **Per Finding 3 of architect review (session `06aa3ec5-...`), all three execute on the default-agent Connection that dashboard_db shares.** Fixing these closes #636.

| Line | Function | Existing pattern | Replacement |
|---|---|---|---|
| 5619 | `replace_with_summary` (conversation compaction at 50-message threshold) | `BEGIN` (**DEFERRED**) + 3 writes + `COMMIT` | `Connection::transaction()` (DEFERRED) + writes + `tx.commit()?` |
| 3737 | `set_skill_enabled` (skill_overrides toggle) | `BEGIN IMMEDIATE` + result-collection IIFE + conditional `COMMIT` | `Connection::transaction_with_behavior(Immediate)` + writes + `tx.commit()?` |
| 3777 | `delete_skill_llm_override` (atomic UPDATE+DELETE on skill_overrides) | `BEGIN IMMEDIATE` + IIFE + conditional `COMMIT` | `Connection::transaction_with_behavior(Immediate)` + writes + `tx.commit()?` |

**Critical: line 5619 is `BEGIN` (DEFERRED), not IMMEDIATE.** Replacing it with `transaction_with_behavior(Immediate)` would change the locking semantics and could introduce contention. Use `Connection::transaction()` (default DEFERRED) for that callsite specifically. Lines 3737 and 3777 use `BEGIN IMMEDIATE` — those map to `transaction_with_behavior(Immediate)`.

#### A2 — Migration callsites (hygiene only, NOT load-bearing for #636)

All `BEGIN IMMEDIATE` calls in `db.rs:1800-3509` are inside the `migrate()` function — schema upgrades that run once at startup, gated by `schema_version` checks. They cannot cause the runtime bug because they only execute if the schema is out of date. After successful boot, the schema is current and these paths don't run again.

| Lines | Context |
|---|---|
| 1800, 1931, 2001, 2044, 2094, 2564 | Per-version migration blocks (v18→v23) |
| 2796, 2843, 2864, 2889, 2907, 2922, 2937, 2957, 2981, 3153, 3259, 3509 | Per-version migration blocks (v23→v28) |

Plus lines 1042/1596: a `BEGIN;` (DEFERRED) inside the v1 fresh-install schema creation block. Runs once on first install, never again. Excluded from runtime fix scope; folded into A2.

A2 is mechanical hygiene — same RAII refactor applied to migration paths. Bundled because it's the same SQL pattern in the same file, single review pass. **PR description names A1 as load-bearing and A2 as hygiene** so reviewers scrutinize A1 and skim A2.

**Refactor pattern (worked example for `replace_with_summary`):**

```rust
// Before (line 5619-5638):
self.conn.execute_batch("BEGIN")?;
self.conn.execute("DELETE FROM messages ...", params![...])?;
self.conn.execute("DELETE FROM messages WHERE ... AND role = 'summary'", params![...])?;
self.conn.execute("INSERT INTO messages ...", params![...])?;
let row_id = self.conn.last_insert_rowid();
self.conn.execute_batch("COMMIT")?;
Ok(row_id)

// After:
let tx = self.conn.transaction()?;  // DEFERRED — preserves existing semantics
tx.execute("DELETE FROM messages ...", params![...])?;
tx.execute("DELETE FROM messages WHERE ... AND role = 'summary'", params![...])?;
tx.execute("INSERT INTO messages ...", params![...])?;
let row_id = tx.last_insert_rowid();
tx.commit()?;
Ok(row_id)
// On any `?` early-return: `tx` drops, automatic ROLLBACK fires.
```

### Fix B — Periodic WAL checkpoint (defense-in-depth)

Add a tokio task to mika-server that runs `PRAGMA wal_checkpoint(PASSIVE)` on `dashboard_db` (which is default-agent-db's Connection) every 60 seconds. This forces the connection to advance past any held snapshot regardless of transaction-leak status.

**Hard-coded 60s interval** (per Finding 5 — drop env-var configurability for v1; YAGNI). Comment in code references `MIKA_DASHBOARD_CHECKPOINT_INTERVAL_SECS` as a future tunable if operator demand surfaces.

**PASSIVE mode known limitation** (per Finding 4): if any connection holds a read transaction during the checkpoint window, those pages are skipped and the WAL doesn't shrink. Under continuous read load, PASSIVE may never fully checkpoint. The pragma returns `(busy_or_log_size, checkpointed_frames)` — log both at INFO. **If WAL file continues growing despite 60s PASSIVE checkpoints, escalate to RESTART mode** (more aggressive, briefly blocks writers). Not an upfront change; documented as the operating-envelope trigger.

### Why both fixes

Fix A1 closes the structural cause of #636 by removing the leak path. Fix B is the safety net: even if A1 misses a callsite (or a future regression introduces a new stuck-transaction path), the periodic checkpoint forces snapshot refresh.

### Why not Fix C (reopen connection)

Issue body's "nuclear option." Skipped because connection re-open thrashes prepared-statement caches and breaks in-flight queries. WAL checkpoint achieves snapshot refresh without connection churn. If A+B prove insufficient, defer Fix C.

## Files

| Action | File | Lines | Change |
|---|---|---|---|
| Modify (A1) | `crates/mika-agent/src/db.rs` | 3 callsites (~10 lines each) | `replace_with_summary` (5619) → `Connection::transaction()`; `set_skill_enabled` (3737) and `delete_skill_llm_override` (3777) → `Connection::transaction_with_behavior(Immediate)` |
| Modify (A2) | `crates/mika-agent/src/db.rs` | ~24 migration callsites + 1 fresh-install block | Same RAII pattern; preserves existing IMMEDIATE semantics on all migration paths |
| Add | `crates/mika-agent/src/server/checkpoint.rs` (new) | +50 | `spawn_dashboard_checkpoint_task(db: AsyncDatabase)` — tokio task running `PRAGMA wal_checkpoint(PASSIVE)` every 60s with INFO-level structured-log entries (`checkpoint.start`, `checkpoint.complete` with `(busy_pages, checkpointed_pages)`, `checkpoint.error`) |
| Modify | `crates/mika-agent/src/server/mod.rs` | +3 | Spawn `spawn_dashboard_checkpoint_task(state.dashboard_db.clone())` in `run_server()` after `dashboard_db` construction (~line 731) |
| Modify | `crates/mika-agent/src/server/mod.rs` | +1 | Register `pub mod checkpoint;` |

Net diff estimate: ~150-180 lines (~30 mechanical refactor + ~50 new module + ~5 wiring).

## Tests

Inline in `crates/mika-agent/src/db.rs` mod tests:

1. **`test_transaction_commit_persists`** — start a tx via `Connection::transaction()`, write a row, commit, verify a separate Connection sees the row.
2. **`test_transaction_drop_without_commit_rolls_back`** — start a tx, write a row, drop without commit, verify a separate Connection does NOT see the row.
3. **`test_replace_with_summary_rollback_on_error`** — induce a constraint violation mid-`replace_with_summary` (e.g., FK violation on session_id), verify the transaction rolls back and prior summary is preserved.

Inline in new `crates/mika-agent/src/server/checkpoint.rs` mod tests:

4. **`test_checkpoint_pragma_returns_pages`** — open a test DB with WAL writes pending, call `PRAGMA wal_checkpoint(PASSIVE)`, verify the return value parses as `(busy_or_log_size, checkpointed_frames)` and at least one page checkpointed.
5. **`test_checkpoint_concurrent_with_reader`** — spawn a reader holding a transaction, run PASSIVE checkpoint, verify the checkpoint completes (possibly with non-zero `busy` count, which is expected) and reader query still succeeds.

End-to-end staleness regression test (per Finding 6):

6. **`test_dashboard_sees_writes_from_separate_connection`** — open two `Connection`s to the same temp DB **with `cache=private`** (per Finding 6 — disable shared cache to simulate cross-process visibility; in-process shared cache would mask the bug). Connection A writes a session via `replace_with_summary` flow. Connection B reads via `list_sessions`. Without the fix, B should see stale data if A's transaction is held; with the fix + periodic checkpoint, B sees fresh data after a checkpoint cycle. **Note:** this test is the load-bearing regression check — its design must accurately simulate cross-process WAL behavior or it will silently false-negative.

## Acceptance criteria

- [ ] `replace_with_summary` (db.rs:5619) uses `Connection::transaction()` (DEFERRED) + `tx.commit()` instead of raw `BEGIN`/`COMMIT`.
- [ ] `set_skill_enabled` (db.rs:3737) uses `Connection::transaction_with_behavior(Immediate)` + `tx.commit()` instead of raw `BEGIN IMMEDIATE`/`COMMIT`.
- [ ] `delete_skill_llm_override` (db.rs:3777) uses `Connection::transaction_with_behavior(Immediate)` + `tx.commit()` instead of raw `BEGIN IMMEDIATE`/`COMMIT`.
- [ ] All `BEGIN IMMEDIATE` + `execute_batch("COMMIT")` patterns in `db.rs` migration paths (lines 1800-3509) refactored to `Transaction` (A2 hygiene).
- [ ] `dashboard_db` connection runs `PRAGMA wal_checkpoint(PASSIVE)` every 60 seconds while mika-server is running, with INFO-level structured-log entries per checkpoint (`checkpoint.start`, `checkpoint.complete`, `checkpoint.error`).
- [ ] PASSIVE-limitation note in `checkpoint.rs` doc-comment: if WAL grows despite checkpoints, escalate to RESTART mode (operating-envelope trigger).
- [ ] Tests 1, 2, 3, 4, 5, 6 above pass.
- [ ] Test 6 (cross-connection staleness regression) uses `cache=private` mode to disable in-process shared cache — verified via test setup code review.
- [ ] End-to-end manual verification: with mika-server running, create a session via `mika ask` from a separate process, wait 65 seconds, assert dashboard `/api/v1/sessions` includes the new session.
- [ ] All existing tests pass (`cargo test -p mika-agent`).
- [ ] `cargo clippy --all-targets` clean.
- [ ] `cargo fmt --check` clean.

## Out of scope

- Fix C (reopen connection periodically) — deferred unless A+B prove insufficient under real load.
- Replacing `BEGIN`/`COMMIT` patterns outside `db.rs` (e.g., skill executors) — those run in subprocess contexts, not the long-lived dashboard connection.
- `MIKA_DASHBOARD_CHECKPOINT_INTERVAL_SECS` env-var configurability — deferred per Finding 5 (YAGNI). Comment in code names the future tunable.
- Tuning `PRAGMA wal_autocheckpoint` (the global SQLite-level checkpoint threshold) — Fix B's explicit periodic checkpoint achieves the same effect with predictable timing and observable logs.
- Refactoring AsyncDatabase architecture (e.g., adding a connection pool) — broader change with much larger blast radius; not needed for this bug.
- Restoring `dashboard_db` as an independent connection (decoupling from default-agent-db) — would prevent the shared-Connection pin entirely, but is a structural change. Documented as a follow-up trigger if A+B prove insufficient.

## Risks

| Risk | Mitigation |
|---|---|
| `replace_with_summary`'s DEFERRED→DEFERRED refactor changes nothing semantically but writes still fail under concurrent writer pressure | Tests 1-3 verify the rollback path; production has only one writer per Connection (single-thread AsyncDatabase model), so concurrent-writer scenarios don't occur on the same Connection. |
| `set_skill_enabled` / `delete_skill_llm_override` IMMEDIATE→IMMEDIATE refactor unchanged in semantics | Mechanical translation, same locking. Existing tests cover the happy path; new test (#3) covers rollback-on-error. |
| A2 migration refactor introduces a regression in startup migrations | Migrations are idempotent (gated by `schema_version`). Existing migration tests run on every build. The Transaction-Drop semantics are strictly safer than raw BEGIN/COMMIT (auto-rollback on error path), never less safe. |
| Periodic checkpoint causes contention with active dashboard queries | `PASSIVE` mode is non-blocking — only checkpoints pages not currently being read. Test 5 verifies concurrent reader compatibility. |
| 60-second interval too long, dashboard still feels stale | Issue reports "1+ hour behind" — 60s is dramatically tighter. If operator demand surfaces, env-var configurability is a 5-line addition. |
| `wal_checkpoint(PASSIVE)` returns non-zero busy count if WAL is locked | Log at INFO with `(busy_pages, checkpointed_pages)` from pragma return. Single missed checkpoint isn't catastrophic — next interval resolves. Operating-envelope trigger documented for RESTART-mode escalation. |
| Test 6 (cross-connection staleness) silently false-negatives due to shared cache | Test setup explicitly uses `cache=private` (per Finding 6). Test code review verifies this before commit. |

## Sequencing

1. **A1 first** (runtime callsites — load-bearing). Three callsites, ~30 lines mechanical refactor.
2. **A2 second** (migration hygiene). ~24 callsites, same pattern.
3. **B third** (checkpoint task). New module + wiring in run_server.
4. **Tests inline** with each change.
5. **Run** `cargo test -p mika-agent`, `cargo clippy --all-targets`, `cargo fmt --check`.
6. **Manual end-to-end verification** (acceptance criterion 9).
7. **Open PR** cross-referencing #636. PR description names A1 as load-bearing, A2 as hygiene, B as defense-in-depth.
8. **Post-merge:** monitor `checkpoint.error` log lines for a week. If consistently quiet, fix is holding. If WAL file grows despite checkpoints, escalate to RESTART mode per the operating-envelope trigger.

## Verification

End-to-end test before merging:

```bash
# Capture baseline
sqlite3 ~/.mika/data/mika.db "SELECT COUNT(*) FROM sessions" # → N
curl -s -H "Authorization: Bearer $MIKA_DASHBOARD_TOKEN" \
  http://localhost:8080/api/v1/sessions?per_page=1 | jq .total # → N (matches)

# Create session via separate process
mika ask --agent mika-test "ping"

# Wait one checkpoint cycle
sleep 65

# Verify dashboard sees it
curl -s -H "Authorization: Bearer $MIKA_DASHBOARD_TOKEN" \
  http://localhost:8080/api/v1/sessions?per_page=1 | jq .total # → N+1
sqlite3 ~/.mika/data/mika.db "SELECT COUNT(*) FROM sessions" # → N+1 (matches)
```

Both counts equal after the wait. Pre-fix, dashboard count would lag indefinitely (until restart).

## Discovery items (resolved during planning)

1. **`dashboard_db` shares Connection with default agent** (Finding 3 verified) — `crates/mika-agent/src/server/mod.rs:727-731`: `dashboard_db = agents.get(&default_agent).expect("...").db.clone()`. AsyncDatabase clone shares underlying `Arc<AsyncDatabaseInner>` (single OS thread, single Connection). Direct implication: stuck transactions on default-agent-db pin dashboard view.

2. **Runtime BEGIN callsites are 3, not 30+** (Finding 2 verified) — line-by-line inspection of `db.rs` shows: `replace_with_summary:5619` (DEFERRED), `set_skill_enabled:3737` (IMMEDIATE), `delete_skill_llm_override:3777` (IMMEDIATE). All other ~24 BEGIN IMMEDIATE callsites are inside `migrate()` and run only at startup. Lines 1042/1596 are inside the v1 fresh-install schema block (one-shot per install).

3. **Mixed transaction semantics across runtime callsites** (Finding 2 verified) — `replace_with_summary` uses DEFERRED; the two skill_overrides functions use IMMEDIATE. Refactor must preserve each callsite's existing locking semantics: DEFERRED → `Connection::transaction()`, IMMEDIATE → `Connection::transaction_with_behavior(Immediate)`. Single-mode refactor (e.g., always-IMMEDIATE) would change semantics.

4. **`rusqlite::Transaction` is available** (Finding 2 verified) — already in the existing rusqlite dependency. No new crate needed.

5. **`PRAGMA wal_checkpoint(PASSIVE)` is non-blocking** (Finding 4 verified) — SQLite docs confirm only checkpoints pages not currently being read; never blocks readers. Returns `(busy_pages, checkpointed_pages)` for observability.

6. **A2 migration refactor is hygiene, not load-bearing** (Finding 1 verified) — migrations gated by schema_version; cannot run mid-runtime. The PR description must distinguish A1 vs A2 reviewer-scrutiny posture.

7. **Test 6 false-negative risk** (Finding 6 verified) — in-process shared cache can mask cross-process WAL visibility. Test setup uses `cache=private` URI to simulate cross-process behavior accurately.
