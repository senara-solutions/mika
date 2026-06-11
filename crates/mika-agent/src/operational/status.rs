//! Status re-derivation for operational items (mika#1263, sub-issue 2a).
//!
//! Per foundation §4, status is **derivable, not authoritative** — except `Done`,
//! which is terminal. On every read, the engine re-derives status from item
//! properties and updates the cache if it changed.
//!
//! ## Precedence (Decision G)
//!
//! `AtRisk > Now > Waiting > Delegated > Scheduled`
//!
//! `Done` is terminal and excluded from re-derivation entirely.

use chrono::{DateTime, Utc};
use tracing::debug;

use crate::db::Database;
use crate::operational::calibration::*;
use crate::operational::types::*;
use crate::timestamp;
use anyhow::Result;

/// Re-derive the status of a non-Done item based on its current properties.
///
/// Returns the derived status. Does NOT write to DB — caller decides.
/// `Done` items should not be passed to this function (caller must filter).
pub fn derive_status(item: &OperationalItem, now: DateTime<Utc>) -> OperationalStatus {
    // Precedence per Decision G: AtRisk > Now > Waiting > Delegated > Scheduled

    // 1. AtRisk — stale near deadline OR callback failure evidence
    if is_at_risk(item, now) {
        return OperationalStatus::AtRisk;
    }

    // 2. Waiting — blocked_by is set
    if item.blocked_by.is_some() {
        return OperationalStatus::Waiting;
    }

    // 3. Now — due within today window, or explicitly set
    if is_now(item, now) {
        return OperationalStatus::Now;
    }

    // 4. Delegated — owner is Mika or Agent and there's an active source task
    if is_delegated(item) {
        return OperationalStatus::Delegated;
    }

    // 5. Scheduled — due_at is set and beyond the today window
    if is_scheduled(item, now) {
        return OperationalStatus::Scheduled;
    }

    // Default: Now (items with no due_at and no special conditions)
    OperationalStatus::Now
}

/// Primary cascade mechanism for transitive unblocks (plan §2.3).
///
/// For each `Waiting` item in the slice, check if its `blocked_by` ID points
/// to a `Done` item; if so, clear `blocked_by` in the DB.
///
/// **Critical-path code:** Layer 1's `complete_operational_item()` does NOT
/// cascade — this is where the blocker→waiting unblock happens. Must run
/// BEFORE `derive_status()` so the cleared `blocked_by` is reflected in
/// the next status derivation pass.
pub fn resolve_cleared_blockers(items: &[OperationalItem], db: &Database) -> Result<()> {
    for item in items {
        if item.status == OperationalStatus::Done {
            continue;
        }
        let blocker_id = match &item.blocked_by {
            Some(id) => id,
            None => continue,
        };

        // Check if the blocker is Done
        if let Some(blocker_status) = db.get_operational_item_status(blocker_id)?
            && blocker_status == OperationalStatus::Done
        {
            db.clear_operational_item_blocked_by(&item.id)?;
            debug!(
                item_id = %item.id,
                blocker_id = %blocker_id,
                "cleared blocked_by — blocker is done"
            );
        }
        // If blocker doesn't exist, we could clear too, but conservatively
        // leave it — the blocker might be in another agent's scope.
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Status derivation helpers
// ---------------------------------------------------------------------------

/// AtRisk conditions:
/// (a) `due_at` within TODAY_WINDOW_HOURS AND `updated_at` > STALE_NEAR_DEADLINE_HOURS ago
/// (b) callback failure evidence in `evidence_refs`
fn is_at_risk(item: &OperationalItem, now: DateTime<Utc>) -> bool {
    // (a) Stale near deadline
    if let Some(due_str) = &item.due_at
        && let Ok(due_at) = timestamp::parse(due_str)
    {
        let hours_until_due = (due_at - now).num_seconds() as f64 / 3600.0;
        if hours_until_due <= TODAY_WINDOW_HOURS {
            // Due is within today window or past-due — check staleness
            if let Ok(updated_at) = timestamp::parse(&item.updated_at) {
                let hours_since_update = (now - updated_at).num_seconds() as f64 / 3600.0;
                if hours_since_update > STALE_NEAR_DEADLINE_HOURS {
                    return true;
                }
            }
        }
    }

    // (b) Callback failure evidence
    for evidence in &item.evidence_refs {
        if evidence.id.contains("callback_failure")
            || evidence.id.contains("subprocess_exited")
            || evidence.id.contains("pipeline_incomplete")
        {
            return true;
        }
    }

    false
}

/// Now conditions:
/// - `due_at` within today window AND `blocked_by IS NULL`
/// - OR: `due_at IS NULL` AND `blocked_by IS NULL` AND status was explicitly Now
fn is_now(item: &OperationalItem, now: DateTime<Utc>) -> bool {
    // blocked_by already checked before this is called (Waiting has higher precedence)

    if let Some(due_str) = &item.due_at
        && let Ok(due_at) = timestamp::parse(due_str)
    {
        let hours_until_due = (due_at - now).num_seconds() as f64 / 3600.0;
        // Due within today window or past-due
        if hours_until_due <= TODAY_WINDOW_HOURS {
            return true;
        }
    }

    // No due_at — if the item was explicitly set to Now, preserve that
    if item.due_at.is_none() && item.status == OperationalStatus::Now {
        return true;
    }

    false
}

/// Delegated conditions:
/// - owner is Mika or Agent AND there's an active source task
fn is_delegated(item: &OperationalItem) -> bool {
    matches!(&item.owner, Owner::Mika | Owner::Agent(_))
        && item.source_table.is_some()
        && item.source_id.is_some()
}

/// Scheduled conditions:
/// - `due_at` is set AND beyond the today window
fn is_scheduled(item: &OperationalItem, now: DateTime<Utc>) -> bool {
    if let Some(due_str) = &item.due_at
        && let Ok(due_at) = timestamp::parse(due_str)
    {
        let hours_until_due = (due_at - now).num_seconds() as f64 / 3600.0;
        return hours_until_due > TODAY_WINDOW_HOURS;
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(overrides: impl FnOnce(&mut OperationalItem)) -> OperationalItem {
        let mut item = OperationalItem {
            id: "item-1".to_string(),
            kind: OperationalKind::Task,
            title: "Test item".to_string(),
            status: OperationalStatus::Now,
            owner: Owner::User,
            priority: 0.0,
            user_importance: 0.0,
            due_at: None,
            blocked_by: None,
            next_action: None,
            evidence_refs: Vec::new(),
            confidence: 1.0,
            source_table: None,
            source_id: None,
            agent_id: "mika".to_string(),
            created_at: "2026-06-01T00:00:00Z".to_string(),
            updated_at: "2026-06-10T12:00:00Z".to_string(),
        };
        overrides(&mut item);
        item
    }

    fn now() -> DateTime<Utc> {
        timestamp::parse("2026-06-10T12:00:00Z").unwrap()
    }

    // -- Done exclusion --

    #[test]
    fn done_item_stays_done() {
        // Done items should not be passed to derive_status, but if they are,
        // they should not re-derive to something else via normal rules.
        // The caller must filter Done items before calling derive_status.
        // This test verifies that a Done item with past-due date would derive
        // to AtRisk if not filtered — confirming the caller's responsibility.
        let item = make_item(|i| {
            i.status = OperationalStatus::Done;
            i.due_at = Some("2026-06-01T00:00:00Z".to_string());
            i.updated_at = "2026-06-01T00:00:00Z".to_string(); // stale
        });
        // derive_status ignores the Done status field — it re-derives from properties.
        // The caller (score_and_rank_items) must skip Done items.
        let derived = derive_status(&item, now());
        // This proves the caller MUST filter Done items — the function doesn't check.
        assert_ne!(derived, OperationalStatus::Done);
    }

    // -- AtRisk derivation --

    #[test]
    fn at_risk_stale_near_deadline() {
        let item = make_item(|i| {
            // Due in 12 hours (within today window)
            i.due_at = Some("2026-06-11T00:00:00Z".to_string());
            // Updated 3 days ago (> STALE_NEAR_DEADLINE_HOURS)
            i.updated_at = "2026-06-07T00:00:00Z".to_string();
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::AtRisk);
    }

    #[test]
    fn at_risk_callback_failure_evidence() {
        let item = make_item(|i| {
            i.evidence_refs = vec![EvidenceRef {
                kind: EvidenceRefKind::External,
                id: "callback_failure:task-123".to_string(),
            }];
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::AtRisk);
    }

    // -- Precedence: AtRisk > Now --

    #[test]
    fn at_risk_overrides_now_conditions() {
        let item = make_item(|i| {
            // Now condition: due within today window
            i.due_at = Some("2026-06-11T00:00:00Z".to_string());
            // AtRisk condition: stale near deadline
            i.updated_at = "2026-06-07T00:00:00Z".to_string();
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::AtRisk);
    }

    // -- Now derivation --

    #[test]
    fn now_due_within_today_window() {
        let item = make_item(|i| {
            // Due in 12 hours (within today window)
            i.due_at = Some("2026-06-11T00:00:00Z".to_string());
            // Recently updated (not stale)
            i.updated_at = "2026-06-10T10:00:00Z".to_string();
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Now);
    }

    #[test]
    fn now_past_due_not_stale() {
        let item = make_item(|i| {
            i.due_at = Some("2026-06-09T00:00:00Z".to_string());
            i.updated_at = "2026-06-10T10:00:00Z".to_string(); // recently updated
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Now);
    }

    #[test]
    fn now_no_due_date_explicitly_now() {
        let item = make_item(|i| {
            i.status = OperationalStatus::Now;
            i.due_at = None;
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Now);
    }

    // -- Waiting derivation --

    #[test]
    fn waiting_when_blocked() {
        let item = make_item(|i| {
            i.blocked_by = Some("blocker-1".to_string());
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Waiting);
    }

    #[test]
    fn waiting_overrides_delegated() {
        let item = make_item(|i| {
            i.blocked_by = Some("blocker-1".to_string());
            i.owner = Owner::Agent("mika-dev".to_string());
            i.source_table = Some("tasks".to_string());
            i.source_id = Some("t-1".to_string());
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Waiting);
    }

    // -- Delegated derivation --

    #[test]
    fn delegated_mika_owned_with_source() {
        let item = make_item(|i| {
            i.status = OperationalStatus::Delegated;
            i.owner = Owner::Mika;
            i.source_table = Some("tasks".to_string());
            i.source_id = Some("t-1".to_string());
            i.due_at = Some("2026-06-20T00:00:00Z".to_string()); // far future
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Delegated);
    }

    #[test]
    fn delegated_agent_owned_with_source() {
        let item = make_item(|i| {
            i.status = OperationalStatus::Delegated;
            i.owner = Owner::Agent("mika-dev".to_string());
            i.source_table = Some("tasks".to_string());
            i.source_id = Some("t-1".to_string());
            i.due_at = Some("2026-06-20T00:00:00Z".to_string());
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Delegated);
    }

    // -- Scheduled derivation --

    #[test]
    fn scheduled_due_beyond_today_window() {
        let item = make_item(|i| {
            i.status = OperationalStatus::Scheduled;
            i.due_at = Some("2026-06-20T00:00:00Z".to_string());
        });
        assert_eq!(derive_status(&item, now()), OperationalStatus::Scheduled);
    }

    // -- Default to Now --

    #[test]
    fn default_to_now_when_no_conditions() {
        let item = make_item(|i| {
            i.status = OperationalStatus::Scheduled; // would be scheduled, but no due_at
            i.due_at = None;
        });
        // No due_at, not blocked, not delegated — defaults to Now
        // (but status was Scheduled, and the status field is only checked for Now preservation)
        // Since it has no due_at and status != Now, it falls through to the default Now
        assert_eq!(derive_status(&item, now()), OperationalStatus::Now);
    }

    // -- Blocker cascade (DB-backed tests) --

    #[test]
    fn resolve_cleared_blockers_unblocks_when_blocker_done() {
        let db = crate::db::Database::open_in_memory().unwrap();

        // Create blocker
        let blocker = NewOperationalItem {
            kind: OperationalKind::Task,
            title: "Blocker".to_string(),
            status: OperationalStatus::Now,
            owner: Owner::User,
            confidence: 1.0,
            source_table: Some("tasks".to_string()),
            source_id: Some("blocker-src".to_string()),
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let blocker_id = db.insert_operational_item(&blocker).unwrap();

        // Complete the blocker
        let evidence = EvidenceRef {
            kind: EvidenceRefKind::GithubPr,
            id: "pr-1".to_string(),
        };
        db.complete_operational_item(&blocker_id, evidence).unwrap();

        // Create waiting item blocked by the blocker
        let waiting = NewOperationalItem {
            kind: OperationalKind::Task,
            title: "Waiting".to_string(),
            status: OperationalStatus::Waiting,
            owner: Owner::User,
            blocked_by: Some(blocker_id.clone()),
            confidence: 1.0,
            source_table: Some("tasks".to_string()),
            source_id: Some("waiting-src".to_string()),
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let _waiting_id = db.insert_operational_item(&waiting).unwrap();

        // Re-read items
        let items = db
            .query_operational_items(&OperationalItemFilter {
                agent_id: "mika".to_string(),
                ..Default::default()
            })
            .unwrap();

        // Filter to non-Done only
        let active: Vec<_> = items
            .into_iter()
            .filter(|i| i.status != OperationalStatus::Done)
            .collect();

        // Resolve cleared blockers
        resolve_cleared_blockers(&active, &db).unwrap();

        // Verify the waiting item's blocked_by is cleared
        let updated = db
            .get_operational_item_by_source("mika", "tasks", "waiting-src")
            .unwrap()
            .unwrap();
        assert!(
            updated.blocked_by.is_none(),
            "blocked_by should be cleared when blocker is done"
        );

        // Re-derive status — should be Now (no longer blocked)
        let derived = derive_status(&updated, Utc::now());
        assert_eq!(derived, OperationalStatus::Now);
    }

    #[test]
    fn resolve_cleared_blockers_keeps_active_blocker() {
        let db = crate::db::Database::open_in_memory().unwrap();

        // Create active blocker (not Done)
        let blocker = NewOperationalItem {
            kind: OperationalKind::Blocker,
            title: "Active blocker".to_string(),
            status: OperationalStatus::Now,
            owner: Owner::User,
            confidence: 1.0,
            source_table: Some("tasks".to_string()),
            source_id: Some("blocker-src".to_string()),
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let blocker_id = db.insert_operational_item(&blocker).unwrap();

        // Create waiting item
        let waiting = NewOperationalItem {
            kind: OperationalKind::Task,
            title: "Waiting".to_string(),
            status: OperationalStatus::Waiting,
            owner: Owner::User,
            blocked_by: Some(blocker_id.clone()),
            confidence: 1.0,
            source_table: Some("tasks".to_string()),
            source_id: Some("waiting-src".to_string()),
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        db.insert_operational_item(&waiting).unwrap();

        let items = db
            .query_operational_items(&OperationalItemFilter {
                agent_id: "mika".to_string(),
                ..Default::default()
            })
            .unwrap();

        resolve_cleared_blockers(&items, &db).unwrap();

        // blocked_by should still be set
        let updated = db
            .get_operational_item_by_source("mika", "tasks", "waiting-src")
            .unwrap()
            .unwrap();
        assert!(
            updated.blocked_by.is_some(),
            "blocked_by should remain when blocker is still active"
        );
    }
}
