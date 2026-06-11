//! Deterministic scoring engine for operational items (mika#1263, sub-issue 2a).
//!
//! The scoring formula is the sole ranking authority for the What's Next engine.
//! No LLM involvement — the LLM's role (sub-issue 2c) is downstream narration only.
//!
//! ## Design
//!
//! `priority()` is a **pure function** — it takes an `OperationalItem`, a timestamp,
//! and a pre-loaded `HashMap` of blocked counts. No DB access in the scoring loop.
//! `rank()` is the batch entry point that loads dependency counts in a single query
//! before calling `priority()` on each item.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::operational::calibration::*;
use crate::operational::types::{OperationalItem, OperationalKind, Owner};
use crate::timestamp;

/// Per-term breakdown of an item's composite priority score.
#[derive(Debug, Clone)]
pub struct ScoreBreakdown {
    pub total: f32,
    pub urgency: f32,
    pub commitment_weight: f32,
    pub user_importance: f32,
    pub stale_time: f32,
    pub dependency_risk: f32,
    pub confidence_penalty: f32,
}

/// An operational item paired with its score breakdown.
#[derive(Debug, Clone)]
pub struct ScoredItem {
    pub item: OperationalItem,
    pub breakdown: ScoreBreakdown,
}

/// Compute composite priority score for a single item.
///
/// Returns a [`ScoreBreakdown`] with each term's contribution.
/// `blocked_counts` is a pre-loaded map of `item_id -> count of items blocked by it`.
///
/// This is a pure function — no IO, no DB access. Testable with synthetic fixtures.
pub fn priority(
    item: &OperationalItem,
    now: DateTime<Utc>,
    blocked_counts: &HashMap<String, u32>,
) -> ScoreBreakdown {
    let urgency = compute_urgency(item, now);
    let commitment_weight = compute_commitment_weight(item);
    let user_importance = item.user_importance.min(USER_IMPORTANCE_MAX);
    let stale_time = compute_stale_time(item, now);
    let dependency_risk = compute_dependency_risk(item, blocked_counts);
    let confidence_penalty = (1.0 - item.confidence) * CONFIDENCE_PENALTY_MULTIPLIER;

    let total = urgency + commitment_weight + user_importance + stale_time + dependency_risk
        - confidence_penalty;

    ScoreBreakdown {
        total,
        urgency,
        commitment_weight,
        user_importance,
        stale_time,
        dependency_risk,
        confidence_penalty,
    }
}

/// Score and rank a list of items. Returns items sorted by priority DESC.
///
/// `blocked_counts` must be pre-loaded via a single `GROUP BY blocked_by` query
/// (see `Database::batch_blocked_counts`). This eliminates N+1 queries.
pub fn rank(
    items: Vec<OperationalItem>,
    now: DateTime<Utc>,
    blocked_counts: &HashMap<String, u32>,
) -> Vec<ScoredItem> {
    let mut scored: Vec<ScoredItem> = items
        .into_iter()
        .map(|item| {
            let breakdown = priority(&item, now, blocked_counts);
            ScoredItem { item, breakdown }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.breakdown
            .total
            .partial_cmp(&a.breakdown.total)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scored
}

// ---------------------------------------------------------------------------
// Term computations
// ---------------------------------------------------------------------------

/// Urgency: time-based proximity to due date.
/// - `None` → 0.0
/// - Past-due → `URGENCY_MAX`
/// - Otherwise → `URGENCY_MAX / (hours_until_due + 1.0)`
fn compute_urgency(item: &OperationalItem, now: DateTime<Utc>) -> f32 {
    let due_at = match &item.due_at {
        Some(s) => match timestamp::parse(s) {
            Ok(dt) => dt,
            Err(_) => return 0.0,
        },
        None => return 0.0,
    };

    let hours_until_due = (due_at - now).num_seconds() as f64 / 3600.0;
    if hours_until_due <= 0.0 {
        URGENCY_MAX
    } else {
        URGENCY_MAX / (hours_until_due as f32 + 1.0)
    }
}

/// Commitment weight: only applies to `Commitment` kind items.
/// Weight varies by owner type.
fn compute_commitment_weight(item: &OperationalItem) -> f32 {
    if item.kind != OperationalKind::Commitment {
        return 0.0;
    }
    match &item.owner {
        Owner::User => COMMITMENT_WEIGHT_USER,
        Owner::Mika => COMMITMENT_WEIGHT_MIKA,
        Owner::Person(_) | Owner::Agent(_) => COMMITMENT_WEIGHT_THIRD_PARTY,
    }
}

/// Stale time: `log(hours_since_update + 1.0) * STALE_TIME_MULTIPLIER`, capped.
fn compute_stale_time(item: &OperationalItem, now: DateTime<Utc>) -> f32 {
    let updated_at = match timestamp::parse(&item.updated_at) {
        Ok(dt) => dt,
        Err(_) => return 0.0,
    };

    let hours_since_update = (now - updated_at).num_seconds() as f64 / 3600.0;
    let hours = hours_since_update.max(0.0);
    let score = (hours as f32 + 1.0).ln() * STALE_TIME_MULTIPLIER;
    score.min(STALE_TIME_CAP)
}

/// Dependency risk: count of items blocked by this item × per-blocked weight, capped.
fn compute_dependency_risk(item: &OperationalItem, blocked_counts: &HashMap<String, u32>) -> f32 {
    let count = blocked_counts.get(&item.id).copied().unwrap_or(0);
    let score = count as f32 * DEPENDENCY_RISK_PER_BLOCKED;
    score.min(DEPENDENCY_RISK_CAP)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operational::types::*;

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
            updated_at: "2026-06-01T00:00:00Z".to_string(),
        };
        overrides(&mut item);
        item
    }

    fn now() -> DateTime<Utc> {
        timestamp::parse("2026-06-10T12:00:00Z").unwrap()
    }

    fn empty_counts() -> HashMap<String, u32> {
        HashMap::new()
    }

    // -- Urgency term --

    #[test]
    fn urgency_no_due_date() {
        let item = make_item(|_| {});
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.urgency, 0.0);
    }

    #[test]
    fn urgency_past_due() {
        let item = make_item(|i| {
            i.due_at = Some("2026-06-09T00:00:00Z".to_string());
        });
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.urgency, URGENCY_MAX);
    }

    #[test]
    fn urgency_future_due() {
        // Due in 24 hours
        let item = make_item(|i| {
            i.due_at = Some("2026-06-11T12:00:00Z".to_string());
        });
        let score = priority(&item, now(), &empty_counts());
        // URGENCY_MAX / (24.0 + 1.0) = 100.0 / 25.0 = 4.0
        assert!((score.urgency - 4.0).abs() < 0.01);
    }

    // -- Commitment weight term --

    #[test]
    fn commitment_weight_non_commitment() {
        let item = make_item(|i| i.kind = OperationalKind::Task);
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.commitment_weight, 0.0);
    }

    #[test]
    fn commitment_weight_user_owned() {
        let item = make_item(|i| {
            i.kind = OperationalKind::Commitment;
            i.owner = Owner::User;
        });
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.commitment_weight, COMMITMENT_WEIGHT_USER);
    }

    #[test]
    fn commitment_weight_mika_owned() {
        let item = make_item(|i| {
            i.kind = OperationalKind::Commitment;
            i.owner = Owner::Mika;
        });
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.commitment_weight, COMMITMENT_WEIGHT_MIKA);
    }

    #[test]
    fn commitment_weight_third_party() {
        let item = make_item(|i| {
            i.kind = OperationalKind::Commitment;
            i.owner = Owner::Person("Alice".to_string());
        });
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.commitment_weight, COMMITMENT_WEIGHT_THIRD_PARTY);
    }

    // -- User importance term --

    #[test]
    fn user_importance_clamped() {
        let item = make_item(|i| i.user_importance = 999.0);
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.user_importance, USER_IMPORTANCE_MAX);
    }

    #[test]
    fn user_importance_passthrough() {
        let item = make_item(|i| i.user_importance = 25.0);
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.user_importance, 25.0);
    }

    // -- Stale time term --

    #[test]
    fn stale_time_recent_item() {
        // Updated just now
        let item = make_item(|i| {
            i.updated_at = "2026-06-10T12:00:00Z".to_string();
        });
        let score = priority(&item, now(), &empty_counts());
        // ln(0 + 1) * 5 = 0
        assert!(score.stale_time < 0.01);
    }

    #[test]
    fn stale_time_old_item() {
        // Updated 9.5 days ago (228 hours)
        let item = make_item(|i| {
            i.updated_at = "2026-06-01T00:00:00Z".to_string();
        });
        let score = priority(&item, now(), &empty_counts());
        // ln(228 + 1) * 5 ≈ ln(229) * 5 ≈ 5.434 * 5 ≈ 27.17
        assert!(score.stale_time > 20.0);
        assert!(score.stale_time <= STALE_TIME_CAP);
    }

    // -- Dependency risk term --

    #[test]
    fn dependency_risk_none() {
        let item = make_item(|_| {});
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.dependency_risk, 0.0);
    }

    #[test]
    fn dependency_risk_some() {
        let item = make_item(|_| {});
        let mut counts = HashMap::new();
        counts.insert("item-1".to_string(), 3);
        let score = priority(&item, now(), &counts);
        assert_eq!(score.dependency_risk, 30.0);
    }

    #[test]
    fn dependency_risk_capped() {
        let item = make_item(|_| {});
        let mut counts = HashMap::new();
        counts.insert("item-1".to_string(), 100);
        let score = priority(&item, now(), &counts);
        assert_eq!(score.dependency_risk, DEPENDENCY_RISK_CAP);
    }

    // -- Confidence penalty term --

    #[test]
    fn confidence_full() {
        let item = make_item(|i| i.confidence = 1.0);
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.confidence_penalty, 0.0);
    }

    #[test]
    fn confidence_zero() {
        let item = make_item(|i| i.confidence = 0.0);
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.confidence_penalty, CONFIDENCE_PENALTY_MULTIPLIER);
    }

    #[test]
    fn confidence_partial() {
        let item = make_item(|i| i.confidence = 0.5);
        let score = priority(&item, now(), &empty_counts());
        assert!((score.confidence_penalty - 25.0).abs() < 0.01);
    }

    // -- Composite formula --

    #[test]
    fn composite_formula_sums_correctly() {
        let item = make_item(|i| {
            i.due_at = Some("2026-06-09T00:00:00Z".to_string()); // past due
            i.kind = OperationalKind::Commitment;
            i.owner = Owner::User;
            i.user_importance = 30.0;
            i.confidence = 0.8;
            i.updated_at = "2026-06-10T12:00:00Z".to_string(); // just now
        });
        let mut counts = HashMap::new();
        counts.insert("item-1".to_string(), 2);

        let score = priority(&item, now(), &counts);

        let expected = score.urgency
            + score.commitment_weight
            + score.user_importance
            + score.stale_time
            + score.dependency_risk
            - score.confidence_penalty;
        assert!((score.total - expected).abs() < 0.001);
    }

    // -- Deterministic ranking --

    #[test]
    fn rank_sorts_by_priority_desc() {
        let items = vec![
            make_item(|i| {
                i.id = "low".to_string();
                i.user_importance = 5.0;
            }),
            make_item(|i| {
                i.id = "high".to_string();
                i.due_at = Some("2026-06-09T00:00:00Z".to_string()); // past due
                i.user_importance = 40.0;
            }),
            make_item(|i| {
                i.id = "mid".to_string();
                i.user_importance = 20.0;
            }),
            make_item(|i| {
                i.id = "penalty".to_string();
                i.confidence = 0.0; // heavy penalty
            }),
            make_item(|i| {
                i.id = "commitment".to_string();
                i.kind = OperationalKind::Commitment;
                i.owner = Owner::User;
                i.user_importance = 10.0;
            }),
        ];

        let ranked = rank(items, now(), &empty_counts());

        assert_eq!(ranked.len(), 5);
        // "high" should be first (past-due urgency + high importance)
        assert_eq!(ranked[0].item.id, "high");
        // "penalty" should be last (confidence=0 → -50 penalty)
        assert_eq!(ranked[4].item.id, "penalty");
        // Verify descending order
        for window in ranked.windows(2) {
            assert!(
                window[0].breakdown.total >= window[1].breakdown.total,
                "ranking not descending: {} >= {} failed",
                window[0].breakdown.total,
                window[1].breakdown.total,
            );
        }
    }

    // -- Edge cases --

    #[test]
    fn invalid_due_at_treated_as_none() {
        let item = make_item(|i| {
            i.due_at = Some("not-a-date".to_string());
        });
        let score = priority(&item, now(), &empty_counts());
        assert_eq!(score.urgency, 0.0);
    }

    #[test]
    fn empty_items_rank() {
        let ranked = rank(Vec::new(), now(), &empty_counts());
        assert!(ranked.is_empty());
    }
}
