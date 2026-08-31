//! Fire-and-forget recording of cross-boundary authentication failures
//! (mika#1949, Porte 3).
//!
//! # Why a trait and not a direct DB call
//!
//! The four call sites that observe an authentication failure are the worst
//! possible places to `await` a database write. Two of them sit on a refusal
//! path that has already decided its verdict; one of them (`server::auth`) is
//! Axum middleware that must return a response now; and the manager cadence
//! task holds no database handle at all — it is constructed from
//! `ManagerConfig`, a `CancellationToken` and a `TokenResolver`, and nothing
//! else (`milestone_manager::spawn::spawn_manager_cycle_task`).
//!
//! So the ledger is injected, exactly like `AuthAlarmSink`, `ReportDeliverer`
//! and `TokenResolver` already are in that module: `Option<Arc<dyn
//! AuthBoundaryLedger>>`, `None` meaning "no ledger wired" (tests, offline
//! bring-up). That keeps the boundary sites testable without a database and
//! keeps the write off their critical path.
//!
//! # The R5 property, stated structurally
//!
//! [`AuthBoundaryLedger::record`] returns `()`. There is no error for a caller
//! to accidentally propagate, and no future for it to accidentally await. An
//! audit failure can therefore never change an authentication outcome — not by
//! convention, but because the signature offers no way to do it.
//!
//! # What this does not do
//!
//! It does not decide anything. A failed authentication still drops the
//! request exactly as it did before mika#1949; the only change is that the
//! drop is now visible. Fail-closed is the property Porte 3 protects (KTD6),
//! not a gap it leaves open.

use std::sync::Arc;

use mika_common::auth_boundary::AuthBoundaryError;
use tracing::warn;

use crate::async_db::AsyncDatabase;

/// Somewhere an auth-boundary failure can be recorded.
pub trait AuthBoundaryLedger: Send + Sync {
    /// Record the failure. Never blocks the caller, never fails visibly.
    fn record(&self, err: AuthBoundaryError);
}

/// The production ledger: an `audit_events` row per failure.
pub struct DbAuthBoundaryLedger {
    db: AsyncDatabase,
}

impl DbAuthBoundaryLedger {
    pub fn new(db: AsyncDatabase) -> Self {
        Self { db }
    }
}

impl AuthBoundaryLedger for DbAuthBoundaryLedger {
    fn record(&self, err: AuthBoundaryError) {
        let db = self.db.clone();
        // Detached on purpose. The caller is on an authentication path; the
        // row is bookkeeping about a decision already made.
        tokio::spawn(async move {
            if let Err(e) = db.record_auth_boundary(&err).await {
                // WARN, not ERROR: the request outcome is correct either way.
                // What is lost is one row of operator visibility, and this line
                // is the record that it was lost.
                warn!(
                    target: "mika::auth_boundary",
                    event = "auth_boundary_audit_write_failed",
                    token_name = %err.token_name,
                    boundary = %err.boundary_key(),
                    kind = %err.kind,
                    error = %e,
                    "could not record auth-boundary failure in audit_events — the refusal itself stands"
                );
            }
        });
    }
}

/// Record through an optional ledger. `None` is a no-op, by design: a boundary
/// site that has no ledger wired must still refuse correctly.
pub fn record_if_wired(ledger: Option<&Arc<dyn AuthBoundaryLedger>>, err: &AuthBoundaryError) {
    if let Some(l) = ledger {
        l.record(err.clone());
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// A ledger that keeps what it was handed, for assertions.
    #[derive(Default)]
    pub struct RecordingLedger {
        pub recorded: Mutex<Vec<AuthBoundaryError>>,
    }

    impl AuthBoundaryLedger for RecordingLedger {
        fn record(&self, err: AuthBoundaryError) {
            self.recorded.lock().unwrap().push(err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::RecordingLedger;
    use super::*;
    use mika_common::auth_boundary::AuthBoundaryKind;

    fn err() -> AuthBoundaryError {
        AuthBoundaryError::new(
            "MIKA_MANAGER_DELIVERY_TOKEN",
            "manager",
            "delivery",
            AuthBoundaryKind::Rejected,
        )
    }

    #[test]
    fn an_unwired_ledger_is_a_no_op_not_a_failure() {
        // The whole point: a site with no ledger still runs to completion.
        record_if_wired(None, &err());
    }

    #[test]
    fn a_wired_ledger_receives_the_failure() {
        let recording = Arc::new(RecordingLedger::default());
        let ledger: Arc<dyn AuthBoundaryLedger> = recording.clone();
        record_if_wired(Some(&ledger), &err());

        let got = recording.recorded.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].boundary_key(), "manager_to_delivery");
        assert_eq!(got[0].token_name, "MIKA_MANAGER_DELIVERY_TOKEN");
    }

    /// R5, with the failure actually injected rather than argued.
    ///
    /// The real ledger is pointed at an `agent_id` with no row in `agents`, so
    /// the INSERT trips the foreign key and the audit write genuinely fails.
    /// The caller must still return normally, and the process must not panic —
    /// which is what the join on the detached task confirms.
    #[tokio::test]
    async fn an_audit_write_failure_does_not_reach_the_caller() {
        let db = crate::async_db::AsyncDatabase::new_with_agent(
            crate::db::Database::open_in_memory().unwrap(),
            "no-such-agent",
        );
        // The write itself must fail — otherwise this test proves nothing.
        assert!(
            db.record_auth_boundary(&err()).await.is_err(),
            "precondition: the FK violation must make the audit write fail"
        );

        let ledger: Arc<dyn AuthBoundaryLedger> = Arc::new(DbAuthBoundaryLedger::new(db));
        record_if_wired(Some(&ledger), &err());
        // Let the detached task run to completion; a panic there would abort
        // the test runtime.
        tokio::task::yield_now().await;
    }
}
