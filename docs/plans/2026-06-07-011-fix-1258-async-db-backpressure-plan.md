# Plan — fix(async_db): non-pinning backpressure under DB saturation (mika#1258)

## Phase 0 — Pin

**A. Bounded channel setup** (`crates/mika-agent/src/async_db.rs:49`):
```rust
let (tx, rx) = mpsc::sync_channel::<DbClosure>(512);
```
512-slot std::sync::mpsc bounded channel between async callers and the DB worker thread.

**B. The blocking-send site** (`crates/mika-agent/src/async_db.rs:109-130`):
```rust
pub async fn with_db<T: Send + 'static>(
    &self,
    f: impl FnOnce(&mut Database) -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = oneshot::channel();
    let sender = {
        let sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
        sender_guard.as_ref().ok_or_else(|| anyhow!("database has been shut down"))?.clone()
    };
    sender
        .send(Box::new(move |db| {  // ← Codex audit's concern: blocks under saturation
            let _ = tx.send(f(db));
        }))?;
    rx.await?
}
```
The lock is properly released before send (Codex notes this is good). But `sender.send()` is `std::sync::mpsc::SyncSender::send` — blocks the calling thread when the channel is full. Under multi-tenant Mika Cloud load, this pins Tokio worker threads.

**C. Shutdown-path send** (`crates/mika-agent/src/async_db.rs:2314`):
```rust
.send(Box::new(move |_db| {
    let _ = tx.send(());
}))
```
Same `SyncSender::send`. Lower priority (shutdown is rare) but same fix should apply for consistency.

**D. Test infrastructure**: file has 2300+ lines of code; tests use `with_db` extensively. Any change to the signature or async-semantic shape touches many tests.

**E. The 3 options framed by Codex** (verbatim from issue body):
1. Move blocking send to `spawn_blocking` — quickest, but moves pin from worker pool to blocking pool (just larger ceiling, doesn't solve saturation root cause)
2. Switch to `tokio::sync::mpsc` — async-native backpressure, but every call site needs cancellation/timeout review
3. DB-as-actor — strongest separation, async backpressure with explicit queue policies, largest delta

**F. Sequencing constraint** (per issue body): the "Operational partner foundation" project's Layer-1 work (Task Ledger) may dovetail with Option 3. The ticket asks: if DB-as-actor is the path, this ticket lands the actor; if not, this ticket lands the smaller fix and operational-partner inherits.

## Hypothesis (committed)

**Option 2 (`tokio::sync::mpsc`) is the right structural fix for this ticket.** Reasoning:

- **Option 1 (`spawn_blocking`) is structurally insufficient.** Codex explicitly names this as a defer, not a fix: "doesn't solve the queue-saturation root cause (just moves the pin from worker pool to blocking pool, which has a larger ceiling)." Under sustained Mika Cloud multi-tenant load, the blocking pool will also saturate; the symptoms shift but persist.

- **Option 2 is the minimum-viable structural fix.** True async backpressure — `tokio::sync::mpsc::Sender::send().await` yields to the executor instead of blocking. Tokio's scheduler can run other tasks on the same worker thread.

- **Option 3 (DB-as-actor) is over-scoped for this ticket.** The actor pattern adds queue-policy explicitness (priority, fairness, dropping rules) but that's policy, not the bottleneck the audit named. Layer-1 Task Ledger work in the operational-partner project may want actor-shape for its OWN reasons (transactional semantics, observability), but THIS ticket's failure mode (Tokio worker pinning under saturation) is solved by async backpressure alone.

- **Sequencing answer:** this ticket lands the async-mpsc fix. The operational-partner project's Layer-1 work can later promote the async-mpsc internals to an actor API without re-opening this ticket's concern. Layered.

## Approach (committed)

### A. Replace std::sync::mpsc with tokio::sync::mpsc

Change at line 49:
```rust
// Before:
let (tx, rx) = mpsc::sync_channel::<DbClosure>(512);
// After:
let (tx, rx) = tokio::sync::mpsc::channel::<DbClosure>(512);
```

### B. Replace SyncSender field type

At line 36:
```rust
// Before:
sender: Mutex<Option<SyncSender<DbClosure>>>,
// After:
sender: Mutex<Option<tokio::sync::mpsc::Sender<DbClosure>>>,
```

### C. Convert send call to async send().await

At line 125:
```rust
// Before:
sender.send(Box::new(move |db| { let _ = tx.send(f(db)); }))?;
// After:
sender
    .send(Box::new(move |db| { let _ = tx.send(f(db)); }))
    .await
    .map_err(|_| anyhow!("DB worker channel closed"))?;
```

### D. Update DB worker loop

The DB worker thread (consumer side) currently uses `rx.recv()` (blocking std::sync::mpsc::Receiver). Tokio's `mpsc::Receiver::recv` is async. The DB worker is a dedicated thread (per Phase 0 D — the file's structure), so two options:

- **D1:** keep the worker as a thread, use `tokio::runtime::Handle::block_on(rx.recv())` to bridge. Cleanest if the worker MUST be a separate thread for SQLite reasons.
- **D2:** spawn the worker as a tokio task (`tokio::spawn`). Simpler async-native pattern.

**Pre-flight verification needed**: SQLite + tokio thread-shape constraints. SQLite's `rusqlite::Connection` is `!Send`-by-default; pinning to a dedicated thread is the standard pattern. If `block_on` bridge is the right shape, D1; otherwise D2 with `Connection::open` per-task or a connection pool.

(The plan flags this verification rather than committing — the wrong choice here would regress SQLite thread-safety, which is harder to detect than the original pinning bug.)

### E. Shutdown path send (line 2314)

Same conversion: `.await` on the async send. Same `map_err` shape.

### F. Cancellation semantics review

`tokio::sync::mpsc::Sender::send().await` is cancellation-safe — if the future is dropped before completion, the value is also dropped (not sent). This differs from `std::sync::mpsc::SyncSender::send` which blocks unconditionally.

Every `with_db` call site needs review for cancellation-aware behavior:
- Most call sites are inside `tokio::spawn` tasks; cancellation is exceptional but possible (engine shutdown, timeout)
- The closure passed to `with_db` runs ONLY if send succeeds; cancellation drops it cleanly — no state corruption
- The oneshot receiver waits for the closure's result; cancellation of the oneshot drops the wait but doesn't cancel the in-flight DB operation

This is generally safe with the existing call patterns. Plan calls for a grep audit of all call sites (~100+ per the file size) and a note in the PR description that cancellation semantics were verified.

## Acceptance Criteria

1. **AC1:** `crates/mika-agent/src/async_db.rs` uses `tokio::sync::mpsc::channel` instead of `std::sync::mpsc::sync_channel` for the DbClosure channel. Verified by grep.

2. **AC2:** `with_db` calls `send().await` on the async sender. Verified by code review.

3. **AC3:** Existing `async_db` test suite passes unchanged (regression coverage on non-saturated path).

4. **AC4:** New regression test that saturates the channel and asserts the agent loop continues making progress:
   - Test name: `test_async_db_saturated_channel_does_not_pin_workers`
   - Mechanism: spawn N closures faster than the worker can drain (artificial slow closure); concurrently spawn a "control" task that increments a counter every 10ms
   - Assert: the control task's counter increments throughout (worker pool not pinned)
   - Marked `#[ignore]` if timing-sensitive; PR description names the manual command to run pre-merge

5. **AC5:** All `with_db` call sites grep-audited for cancellation-aware behavior; PR description names the audit result (counts of sites + any flagged for follow-up).

6. **AC6:** DB worker loop pattern (D1 vs D2) chosen with explicit justification in the PR description.

7. **AC7:** `cargo build -p mika-agent` + `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` + `cargo test -p mika-agent --lib` all pass.

## Files to change

- `crates/mika-agent/src/async_db.rs` — channel type, sender type, send-site, worker loop
- `crates/mika-agent/tests/` — new saturation regression test (file name TBD by directory convention)

**Single-file primary change.** No new dependencies (`tokio` is already in tree).

## Out of scope

- DB-as-actor refactor (Option 3) — deferred to operational-partner project Layer-1 if needed; this ticket's async-mpsc shape is forward-compatible (an actor API can wrap the async channel later)
- SQL query performance / index optimization (orthogonal)
- Per-tenant queue policies / priority queueing (operational-partner Layer-1 concern)

## Risk

Medium.
- **SQLite + tokio thread-shape decision (D1 vs D2)** is the highest-risk choice. Plan explicitly flags this for pre-implementation verification. If verification reveals SQLite constraints force D1 (block_on bridge), the runtime gain is smaller but still positive (the SEND side is async; only the RECV side has block_on overhead).
- **Cancellation semantics**: `send().await` cancellation drops values; most call sites are inside tokio::spawn and tolerate this, but a manual audit is required (AC5).
- **Existing tests passing alone is insufficient evidence** — the bug only manifests under saturation. AC4's regression test (even if `#[ignore]`-gated) is the actual binding.

## Implementation order

1. Pre-flight: verify SQLite + tokio thread-shape constraints (D1 vs D2 decision). Read `rusqlite::Connection` Send semantics + check whether existing worker is `Send` or `!Send`.
2. Replace channel type at line 49 + sender type at line 36.
3. Convert with_db's send to `send().await`.
4. Update DB worker loop per chosen D1/D2.
5. Update shutdown-path send at line 2314.
6. Grep all `with_db` call sites; verify cancellation semantics; document audit result.
7. Add saturation regression test (AC4).
8. Run cargo build + clippy + lib tests.
9. Run saturation test (manual / `#[ignore]`-gated).

## Test plan

1. Unit: existing async_db tests pass.
2. Integration: saturation test (AC4) — worker pool not pinned under DB saturation.
3. Manual smoke: dev agent loop runs for ~5 min under simulated DB pressure; verify no apparent hangs or stuck dispatches.
