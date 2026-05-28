# Plan: fix(async_db): Replace blocking sync_channel send with tokio::sync::mpsc

**Ticket:** mika#1258
**Type:** fix
**Priority:** p2-normal
**Component:** agent-core (`crates/mika-agent/src/async_db.rs`)

## Problem

`AsyncDatabase::with_db()` uses `std::sync::mpsc::sync_channel(512)` with a bounded capacity. The `SyncSender::send()` call at line 124 blocks when the channel is full. Although the mutex is released before `send()` (preventing a deadlock-under-contention scenario), the `send()` itself is a **blocking** call on a Tokio worker thread. Under DB saturation (queue depth > 512), this pins Tokio worker threads, causing non-linear latency growth for all async tasks — webhook handling, dispatch, and the agent loop itself stall.

The failure mode does not manifest in single-customer self-host but will under multi-tenant Mika Cloud load.

## Decision: Option 2 — Switch to `tokio::sync::mpsc`

### Why not Option 1 (spawn_blocking)
`spawn_blocking` moves the pin from the worker pool to Tokio's blocking pool (default 512 threads). It solves the immediate problem (worker threads unpin) but:
- Adds a thread-pool hop per DB operation (~100s of calls per agent turn)
- Does not provide async backpressure — the blocking pool silently queues
- Masks the root cause: the channel itself should be async-native

### Why not Option 3 (dedicated actor API)
A full actor abstraction (typed message enum, request/response routing, queue policies) is the strongest separation. However:
- The `with_db` closure-dispatch pattern already IS an actor — it has a single-owner thread, a message channel, and closure-based dispatch. The only defect is the channel type.
- The operational-partner Layer-1 work (Task Ledger) will likely introduce a typed actor for the operational-items write path, but that work has its own ticket scope and should not be coupled to this correctness fix.
- Option 2 is the minimal delta that fixes the defect. If Layer-1 later needs a full actor, this fix is not wasted — `tokio::sync::mpsc` is the channel primitive a typed actor would use anyway.

### Sequencing vs operational-partner project
This ticket lands Option 2 now. The operational-partner Layer-1 refactor inherits the result: it will find `AsyncDatabase` already using an async channel, and can extend or wrap it without first having to fix the blocking-send defect. No coupling, no blocking dependency.

## Implementation

### Step 1: Replace the channel type in `AsyncDatabaseInner`

**File:** `crates/mika-agent/src/async_db.rs`

Replace `std::sync::mpsc::{SyncSender, sync_channel}` with `tokio::sync::mpsc::{Sender, channel}`.

The inner struct changes:

```rust
// Before
struct AsyncDatabaseInner {
    sender: Mutex<Option<SyncSender<DbClosure>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
}

// After
struct AsyncDatabaseInner {
    sender: Mutex<Option<tokio::sync::mpsc::Sender<DbClosure>>>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
}
```

**Channel capacity stays at 512.** The bounded capacity is intentional backpressure — the change is from blocking-backpressure to async-backpressure.

### Step 2: Adapt the DB worker thread to receive from a tokio channel

`tokio::sync::mpsc::Receiver` is `!Send` across thread boundaries by design, but it CAN be moved into a thread (it's `Send`, just its `recv()` is async). The DB worker thread is a plain OS thread, not a Tokio task. Two approaches:

**Approach A — Use `blocking_recv()` on the OS thread:**
`tokio::sync::mpsc::Receiver` provides `blocking_recv()` which is safe to call from a non-Tokio thread. This is the minimal change.

```rust
// In new_with_agent:
let (tx, mut rx) = tokio::sync::mpsc::channel::<DbClosure>(512);
let handle = std::thread::Builder::new()
    .name("mika-db".to_string())
    .spawn(move || {
        while let Some(f) = rx.blocking_recv() {
            if let Err(_panic) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    f(&mut db);
                }))
            {
                tracing::error!("database closure panicked — thread continues");
            }
        }
    })
    .expect("failed to spawn database thread");
```

This is the correct approach. The DB worker thread blocks on `blocking_recv()` (expected — it's a dedicated thread), while the sending side (`with_db`) uses async `send().await`.

### Step 3: Make `with_db` use async send

```rust
pub async fn with_db<T: Send + 'static>(
    &self,
    f: impl FnOnce(&mut Database) -> Result<T> + Send + 'static,
) -> Result<T> {
    let (tx, rx) = oneshot::channel();
    let sender = {
        let sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
        sender_guard
            .as_ref()
            .ok_or_else(|| anyhow!("database has been shut down"))?
            .clone()
    };
    sender
        .send(Box::new(move |db| {
            let _ = tx.send(f(db));
        }))
        .await  // <-- This is now async, not blocking
        .map_err(|_| anyhow!("database thread has stopped"))?;
    rx.await
        .map_err(|_| anyhow!("database thread dropped reply"))?
}
```

**Key change:** `.send().await` instead of `.send()`. When the channel is full, the calling task yields back to the Tokio scheduler instead of blocking the worker thread. Other async tasks (webhook handling, dispatch, etc.) continue running.

### Step 4: Update shutdown

The `shutdown()` method drops the sender to signal the DB thread. `tokio::sync::mpsc::Sender` dropping works identically — `blocking_recv()` returns `None` when all senders are dropped.

```rust
pub fn shutdown(&self) {
    {
        let mut sender_guard = self.inner.sender.lock().expect("sender lock poisoned");
        *sender_guard = None;  // Drop the Sender — same semantics
    }
    // ... join handle unchanged
}
```

No behavior change needed.

### Step 5: Update imports

Remove `std::sync::mpsc::{self, SyncSender}`. The only remaining `std::sync` import is `Arc` and `Mutex` (still needed for the `Option<Sender>` wrapper and the thread handle).

### Step 6: Add regression test for saturation resilience

Add an `#[ignore]` test that verifies Tokio worker threads remain responsive under DB queue saturation:

```rust
/// Regression test for mika#1258: verify that a saturated DB queue
/// does not pin Tokio worker threads.
#[tokio::test]
#[ignore] // Slow — run before merging async_db changes
async fn test_db_saturation_does_not_block_tokio_workers() {
    let db = test_async_db();
    
    // Fill the channel with slow operations
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let b2 = barrier.clone();
    
    // Send a closure that blocks the DB thread
    let sender = db.inner.sender.lock().unwrap().as_ref().unwrap().clone();
    sender.send(Box::new(move |_db| {
        // Block the DB thread so the channel fills up
        std::thread::sleep(std::time::Duration::from_secs(2));
    })).await.unwrap();
    
    // Fill the channel to capacity
    for i in 0..512 {
        let _ = sender.try_send(Box::new(move |_db| {
            // No-op closures to fill the queue
        }));
    }
    
    // Now verify that a Tokio task can still make progress
    // even though the DB channel is full
    let progress = tokio::spawn(async move {
        // This should complete immediately — it's pure async work
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        42
    });
    
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        progress,
    ).await;
    
    assert!(result.is_ok(), "Tokio worker thread was pinned by DB saturation");
    assert_eq!(result.unwrap().unwrap(), 42);
}
```

**Note:** The exact test shape may need refinement during implementation. The key assertion is: with a full DB channel, `tokio::spawn`ed tasks still make progress (they're not blocked behind a sync `send()`).

### Step 7: Verify existing tests pass unchanged

Run `cargo test -p mika-agent -- async_db` — all 11 existing `async_db` tests must pass without modification. The async `send().await` is transparent to callers because `with_db` is already `async fn`.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/async_db.rs` | Replace `std::sync::mpsc` with `tokio::sync::mpsc`; change `send()` to `send().await`; change `rx.recv()` to `rx.blocking_recv()`; add saturation regression test |

**Single-file change.** All ~233 call sites of `with_db` are already `async` and `.await` the result — no caller changes needed.

## Risk Assessment

**Low risk.** The change is narrowly scoped:
- `tokio::sync::mpsc` is a mature, well-tested channel primitive
- `blocking_recv()` on the DB thread is the documented pattern for bridging async/sync boundaries
- Channel capacity (512) is unchanged — backpressure semantics preserved
- The `with_db` signature is unchanged — all callers are unaffected
- Shutdown semantics are identical (drop sender → receiver returns None)

**What could go wrong:**
- `tokio::sync::mpsc::Sender::clone()` has slightly different performance characteristics than `SyncSender::clone()` — both are cheap (Arc increment), so no concern
- If `blocking_recv()` is called from inside a Tokio runtime context on the DB thread, it could panic — but the DB thread is a plain `std::thread::spawn`, not a Tokio task, so this is safe

## Verification

1. `cargo test -p mika-agent -- async_db` — all existing tests pass
2. `cargo test -p mika-agent -- async_db --ignored` — new saturation test passes
3. `cargo clippy -p mika-agent` — no new warnings
4. `make deploy` + smoke test: run a few agent turns, verify normal operation
