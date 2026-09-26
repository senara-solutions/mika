//! Bounded A2A wait line under genuine concurrency (mika#2163).
//!
//! The unit tests inside `server::a2a_wait_queue` pin the shape of each outcome
//! one at a time. The wiring tests in `server::tests` pin that both `/a2a` gates
//! actually call the mechanism, over the real HTTP path. This file covers the
//! third thing neither of those can show: what the line does when several callers
//! are inside it **at the same time**.
//!
//! Why that needs its own file. A sequential test can only demonstrate that a
//! second call made after a first one returns; it cannot demonstrate that two
//! interleaved callers do not both win, or that a waiter parked behind a turn is
//! ever woken. The founding incident of mika#2163 was exactly a concurrency
//! shape — a grooming architect pass refused five times in a row by callers
//! overlapping on one agent — so a suite that never interleaves anything would
//! be green on the code that produced it. Same reasoning, and the same warning,
//! as `dispatcher_contention.rs`.
//!
//! Not gated behind an env var: unlike the three-dispatcher race, nothing here
//! touches a shared database or spawns OS threads. It is a mutex, a semaphore,
//! and a handful of tokio tasks.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mika_a2a::jsonrpc::AGENT_BUSY;
use mika_agent::server::a2a_wait_queue::{self, WaitSlot};
use mika_common::config::Settings;
use tokio::sync::{Mutex, Semaphore};

fn settings(depth: usize, wait_ms: u64) -> Settings {
    let mut s = Settings::test_defaults();
    s.a2a_queue_max_depth = Some(depth);
    s.a2a_queue_wait_timeout_ms = Some(wait_ms);
    s
}

/// Every caller admitted to the line eventually gets its turn.
///
/// Eight tasks race for one lock behind one holder. The invariant is the one the
/// bound depends on: `tokio::sync::Mutex` hands the lock out in request order, so
/// an admitted waiter cannot be lapped indefinitely by later arrivals. Without
/// that, "depth 8" would bound a scramble rather than a queue, and the number
/// would not mean what the operator reads it to mean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_admitted_waiter_eventually_gets_its_turn() {
    const WAITERS: usize = 8;
    let cfg = Arc::new(settings(WAITERS, 10_000));
    let lock = Arc::new(Mutex::new(()));
    let slots = Arc::new(Semaphore::new(WAITERS));
    let served = Arc::new(AtomicUsize::new(0));

    // Hold the lock so every waiter is genuinely parked, not merely sequential.
    let held = Arc::clone(&lock).lock_owned().await;

    let mut handles = Vec::new();
    for _ in 0..WAITERS {
        let (lock, slots, cfg, served) = (
            Arc::clone(&lock),
            Arc::clone(&slots),
            Arc::clone(&cfg),
            Arc::clone(&served),
        );
        handles.push(tokio::spawn(async move {
            let slot = a2a_wait_queue::try_take_slot(&slots, &cfg).expect("line has room");
            let acquired = a2a_wait_queue::wait_for_agent_lock(lock, slot, &cfg)
                .await
                .expect("an admitted waiter must be served");
            served.fetch_add(1, Ordering::SeqCst);
            // Hold the lock briefly, as a turn would.
            tokio::time::sleep(Duration::from_millis(5)).await;
            drop(acquired);
        }));
    }

    // All eight are in the line before any of them can proceed.
    for _ in 0..200 {
        if slots.available_permits() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        slots.available_permits(),
        0,
        "all eight should be waiting, not refused"
    );
    assert_eq!(
        served.load(Ordering::SeqCst),
        0,
        "nobody runs while the lock is held"
    );

    drop(held);
    for h in handles {
        tokio::time::timeout(Duration::from_secs(10), h)
            .await
            .expect("no waiter may be starved")
            .unwrap();
    }

    assert_eq!(served.load(Ordering::SeqCst), WAITERS);
    assert_eq!(
        slots.available_permits(),
        WAITERS,
        "every place must come back"
    );
}

/// The bound counts **waiters**, not turns — the subtle point the plan flagged
/// for explicit review (mika#2163 §3.2).
///
/// The permit is released the instant the lock is acquired. If it were held for
/// the duration of the turn instead, a depth of 1 would mean "one caller total"
/// rather than "one caller waiting", and the second caller below would be refused
/// `queue_full` instead of being allowed to wait. That difference is invisible in
/// any test that does not have a turn in flight and a waiter at the same time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bound_counts_waiters_not_turns() {
    let cfg = Arc::new(settings(1, 10_000));
    let lock = Arc::new(Mutex::new(()));
    let slots = Arc::new(Semaphore::new(1));

    // A — takes the single place, gets the lock, releases the place. Its turn is
    // now in flight and the line is empty again.
    let slot_a = a2a_wait_queue::try_take_slot(&slots, &cfg).unwrap();
    let turn_a = a2a_wait_queue::wait_for_agent_lock(Arc::clone(&lock), slot_a, &cfg)
        .await
        .expect("free lock");
    assert_eq!(
        slots.available_permits(),
        1,
        "a turn in flight must not occupy a place in the wait line"
    );

    // B — takes that place and waits behind A's turn.
    let waiter = {
        let (lock, slots, cfg) = (Arc::clone(&lock), Arc::clone(&slots), Arc::clone(&cfg));
        tokio::spawn(async move {
            let slot = a2a_wait_queue::try_take_slot(&slots, &cfg)
                .expect("B must be admitted: the place A used is free again");
            a2a_wait_queue::wait_for_agent_lock(lock, slot, &cfg)
                .await
                .map(|_| ())
        })
    };
    for _ in 0..200 {
        if slots.available_permits() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        slots.available_permits(),
        0,
        "B should now hold the only place"
    );

    // C — arrives on a genuinely full line and is refused, without waiting.
    let err = a2a_wait_queue::try_take_slot(&slots, &cfg).expect_err("the line is full");
    assert_eq!(err.code, AGENT_BUSY);
    assert_eq!(err.data.unwrap()["reason"], "queue_full");

    drop(turn_a);
    tokio::time::timeout(Duration::from_secs(10), waiter)
        .await
        .expect("B must be served once A's turn ends")
        .unwrap()
        .expect("B must not time out");
    assert_eq!(slots.available_permits(), 1);
}

/// Saturation is decided once, atomically, under a real race.
///
/// Sixteen callers arrive together on a line of eight. The property is a
/// conservation law: admitted + refused must equal every caller, and admitted
/// must never exceed the configured depth. A time-of-check/time-of-use hole in
/// the admission step would show up here as an over-admission, and nowhere in a
/// sequential test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_arrivals_never_over_admit() {
    const DEPTH: usize = 8;
    const CALLERS: usize = 16;
    let cfg = Arc::new(settings(DEPTH, 10_000));
    let lock = Arc::new(Mutex::new(()));
    let slots = Arc::new(Semaphore::new(DEPTH));
    let admitted = Arc::new(AtomicUsize::new(0));
    let refused = Arc::new(AtomicUsize::new(0));

    let held = Arc::clone(&lock).lock_owned().await;

    let mut handles = Vec::new();
    for _ in 0..CALLERS {
        let (lock, slots, cfg, admitted, refused) = (
            Arc::clone(&lock),
            Arc::clone(&slots),
            Arc::clone(&cfg),
            Arc::clone(&admitted),
            Arc::clone(&refused),
        );
        handles.push(tokio::spawn(async move {
            match a2a_wait_queue::try_take_slot(&slots, &cfg) {
                Ok(slot) => {
                    admitted.fetch_add(1, Ordering::SeqCst);
                    let _ = a2a_wait_queue::wait_for_agent_lock(lock, slot, &cfg).await;
                }
                Err(err) => {
                    assert_eq!(err.code, AGENT_BUSY);
                    assert_eq!(err.data.unwrap()["reason"], "queue_full");
                    refused.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
    }

    // Let the refusals settle before releasing the lock, so the counts describe
    // the saturated moment rather than the drain.
    for _ in 0..200 {
        if admitted.load(Ordering::SeqCst) + refused.load(Ordering::SeqCst) == CALLERS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(held);
    for h in handles {
        tokio::time::timeout(Duration::from_secs(10), h)
            .await
            .expect("no caller may hang")
            .unwrap();
    }

    let (a, r) = (
        admitted.load(Ordering::SeqCst),
        refused.load(Ordering::SeqCst),
    );
    assert_eq!(
        a + r,
        CALLERS,
        "every caller must be either admitted or refused"
    );
    assert!(
        a <= DEPTH,
        "admitted {a} exceeds the configured depth {DEPTH}"
    );
    assert_eq!(r, CALLERS - a);
    assert_eq!(
        slots.available_permits(),
        DEPTH,
        "every place must come back"
    );
}

/// The kill-switch is not a narrower line — it is no line at all.
///
/// With the queue disabled every caller takes the legacy path, so a concurrent
/// arrival is refused with `-32603` and never consumes a place. A rollback that
/// still moved permits around would leave a second mechanism running underneath
/// an operator who believes they turned it off.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_kill_switch_bypasses_the_line_entirely() {
    let mut s = settings(8, 10_000);
    s.a2a_queue_enabled = Some(false);
    let cfg = Arc::new(s);
    let lock = Arc::new(Mutex::new(()));
    let slots = Arc::new(Semaphore::new(8));

    let held = Arc::clone(&lock).lock_owned().await;

    let mut handles = Vec::new();
    for _ in 0..4 {
        let (lock, slots, cfg) = (Arc::clone(&lock), Arc::clone(&slots), Arc::clone(&cfg));
        handles.push(tokio::spawn(async move {
            let slot = a2a_wait_queue::try_take_slot(&slots, &cfg).expect("no line to be full");
            assert!(matches!(slot, WaitSlot::Disabled));
            let err = a2a_wait_queue::wait_for_agent_lock(lock, slot, &cfg)
                .await
                .expect_err("busy agent, legacy path");
            assert_eq!(err.code, -32603);
            assert_eq!(err.message, "Agent is busy");
            assert!(err.data.is_none());
        }));
    }
    for h in handles {
        tokio::time::timeout(Duration::from_secs(10), h)
            .await
            .expect("the legacy path never waits")
            .unwrap();
    }

    assert_eq!(
        slots.available_permits(),
        8,
        "the disabled path must not touch the line"
    );
    drop(held);
}
