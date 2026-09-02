//! Three-dispatcher exec-slot race (mika#1948 Porte 2, AC8).
//!
//! The unit tests prove the claim refuses a second caller. This one proves the
//! property that actually matters operationally: under a genuine concurrent
//! race between the three dispatchers — the autonomous loop, the milestone
//! manager, and the operator — every ticket is dispatched EXACTLY once, and the
//! run does not deadlock.
//!
//! Why this cannot be a unit test: the defect it guards against is a
//! time-of-check/time-of-use window. A sequential test can only show that a
//! second call after a first is refused; it cannot show that two callers
//! *interleaved* do not both succeed. Only real concurrency exercises the
//! atomicity of the claim.
//!
//! Gated behind `#[ignore]` + `MIKA_MANAGER_CONTENTION_TEST=1` per AC8, because
//! it spawns real threads against a shared DB and is slower and more
//! contention-sensitive than the rest of the suite.
//!
//! Run with:
//! ```text
//! MIKA_MANAGER_CONTENTION_TEST=1 cargo test -p mika-agent \
//!     --test dispatcher_contention -- --ignored
//! ```

use mika_agent::db::{Database, SlotClaim};
use std::sync::{Arc, Mutex};
use std::thread;

/// The three dispatchers that share an exec slot.
const DISPATCHERS: &[&str] = &["mika_dev", "mika_manager", "operator"];

fn gated() -> bool {
    std::env::var("MIKA_MANAGER_CONTENTION_TEST").is_ok()
}

/// Five tickets, three dispatchers, all racing for the same class.
///
/// Each (ticket, dispatcher) pair attempts a claim. The invariant: for each
/// ticket, AT MOST one dispatcher may hold the slot at a time, and across the
/// run every ticket is claimed exactly once — never zero times (deadlock) and
/// never twice (two writers on one branch, the 2026-08-30 incident).
#[test]
#[ignore = "requires MIKA_MANAGER_CONTENTION_TEST=1"]
fn three_dispatcher_race_dispatches_each_ticket_exactly_once() {
    if !gated() {
        return;
    }

    for ticket in 0..5u32 {
        // A fresh DB per ticket: the slot is per (agent, class), so a shared DB
        // would serialise the tickets rather than race them, and the test would
        // pass without exercising anything.
        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let winners = Arc::new(Mutex::new(Vec::<String>::new()));

        let handles: Vec<_> = DISPATCHERS
            .iter()
            .map(|source| {
                let db = Arc::clone(&db);
                let winners = Arc::clone(&winners);
                let source = source.to_string();
                thread::spawn(move || {
                    let holder = format!("task-{source}-{ticket}");
                    let claim = {
                        let mut guard = db.lock().unwrap();
                        guard
                            .try_acquire_dispatch_slot(
                                "mika",
                                "implement",
                                &holder,
                                Some(&source),
                                120,
                            )
                            .expect("claim must not error")
                    };
                    if claim.acquired() {
                        winners.lock().unwrap().push(source);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("no dispatcher thread may panic");
        }

        let winners = winners.lock().unwrap();
        assert_eq!(
            winners.len(),
            1,
            "ticket {ticket}: exactly one dispatcher may win the slot — \
             0 means deadlock, >1 means two writers on one branch. Winners: {winners:?}"
        );

        // And the DB must agree with the winner: the lease names them.
        let holder = db
            .lock()
            .unwrap()
            .dispatch_slot_lease_holder("mika", "implement")
            .unwrap()
            .expect("a won slot must have a recorded holder");
        assert_eq!(
            holder.1.as_deref(),
            Some(winners[0].as_str()),
            "ticket {ticket}: the recorded lease must name the dispatcher that won"
        );
    }
}

/// No deadlock across a longer run: after each winner releases, the slot must
/// become claimable again. A claim that could not be handed on would stall the
/// class permanently — fail-closed turning into loop-breaking.
#[test]
#[ignore = "requires MIKA_MANAGER_CONTENTION_TEST=1"]
fn slot_is_handed_on_across_successive_dispatches() {
    if !gated() {
        return;
    }

    let mut db = Database::open_in_memory().unwrap();
    for round in 0..10u32 {
        let holder = format!("task-round-{round}");
        let claim = db
            .try_acquire_dispatch_slot("mika", "implement", &holder, Some("mika_dev"), 120)
            .unwrap();
        assert_eq!(
            claim,
            SlotClaim::Acquired,
            "round {round}: the slot must be claimable after the previous holder released"
        );
        assert!(
            db.release_dispatch_slot("mika", "implement", &holder)
                .unwrap(),
            "round {round}: the holder must be able to release its own lease"
        );
    }
}

/// Contention must be reported, not silently swallowed: the loser learns who
/// beat it. Without this the operator sees a throughput bottleneck with no name
/// on it — the observability half of Porte 2.
#[test]
#[ignore = "requires MIKA_MANAGER_CONTENTION_TEST=1"]
fn loser_learns_which_dispatcher_holds_the_slot() {
    if !gated() {
        return;
    }

    let mut db = Database::open_in_memory().unwrap();
    db.try_acquire_dispatch_slot("mika", "implement", "op-task", Some("operator"), 120)
        .unwrap();
    let loser = db
        .try_acquire_dispatch_slot("mika", "implement", "mgr-task", Some("mika_manager"), 120)
        .unwrap();

    match loser {
        SlotClaim::Held {
            holder_task_id,
            dispatcher_source,
            ..
        } => {
            assert_eq!(holder_task_id, "op-task");
            assert_eq!(
                dispatcher_source.as_deref(),
                Some("operator"),
                "the refusal must name the blocking dispatcher so contention is reportable"
            );
        }
        SlotClaim::Acquired => panic!("the second claimant must not acquire a held slot"),
    }
}
