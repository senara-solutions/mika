//! Gateway-side `audit_events` writer for webhook silent-no-op paths (mika#1774).
//!
//! Turns four previously log-only silent drops in `github.rs` into durable
//! Postgres rows an operator or dashboard can key off — the missing surface
//! the 2026-07-01 14 h dead-qa window (mika#1711 AC3) needed.
//!
//! Scope invariants (from the ticket):
//! - Observability-only: writes are fire-and-forget; a DB failure logs a WARN
//!   and lets the drop decision stand. The gateway MUST NOT propagate an error
//!   that would change webhook processing behavior.
//! - Additive: the drop decision itself is unchanged. This module never gates
//!   routing.
//! - No retry logic: out of scope per ticket.
//!
//! Companion migration: `migrations/009_audit_events.sql`.

use serde_json::json;
use sqlx::PgPool;
use tracing::warn;

/// `tool_name` value written for every gateway-emitted `audit_events` row.
///
/// Load-bearing for the AC3 dashboard query pattern
/// (`WHERE tool_name = 'gateway_webhook'`). Keep in sync with any operator
/// dashboard SQL.
pub(crate) const TOOL_NAME: &str = "gateway_webhook";

/// Drop-reason `target_key` written when `route_event(...)` returns `None`
/// (unroutable event type / action / conclusion tuple).
pub(crate) const DROP_NO_ROUTE: &str = "webhook_no_route";

/// Drop-reason `target_key` written when the `pull_request.review_requested`
/// event targets a reviewer other than the QA bot (mika#1655 guard).
pub(crate) const DROP_REVIEWER_FILTER: &str = "webhook_reviewer_filter_dropped";

/// Drop-reason `target_key` written when an `issues.labeled` event names a
/// denylisted operator-only skill (#845 Layer-3 guard).
pub(crate) const DROP_DENYLISTED_SKILL: &str = "webhook_denylisted_skill_dropped";

/// Drop-reason `target_key` written when a `pull_request.synchronize` event
/// carries a diff with zero file changes (#886 no-op push guard).
pub(crate) const DROP_SYNCHRONIZE_NO_DIFF: &str = "webhook_synchronize_no_diff_change";

/// Structured context captured at a webhook silent-drop site.
///
/// Borrowed slices are stamped into the `metadata` JSONB column exactly once
/// per drop — the type is a plain param bag, not a persisted shape.
pub(crate) struct WebhookDropContext<'a> {
    pub(crate) event_type: &'a str,
    pub(crate) action: Option<&'a str>,
    pub(crate) check_conclusion: Option<&'a str>,
    pub(crate) delivery_id: &'a str,
    pub(crate) repo_full_name: Option<&'a str>,
}

/// Serialize the drop context into the `metadata` JSONB shape written to the
/// `audit_events` row. Split out from the DB write so the JSON shape can be
/// exercised by pure Rust unit tests (no Postgres required).
///
/// Shape matches the ticket payload spec: `event_type`, `action`,
/// `check_conclusion`, `delivery_id`, `repo_full_name`, `drop_reason`.
pub(crate) fn build_drop_metadata(
    ctx: &WebhookDropContext<'_>,
    drop_reason: &str,
) -> serde_json::Value {
    json!({
        "event_type": ctx.event_type,
        "action": ctx.action,
        "check_conclusion": ctx.check_conclusion,
        "delivery_id": ctx.delivery_id,
        "repo_full_name": ctx.repo_full_name,
        "drop_reason": drop_reason,
    })
}

/// Insert one `audit_events` row for a webhook silent-drop. Fire-and-forget:
/// a DB failure is logged at WARN and the drop decision is unchanged.
///
/// `drop_reason` is written both as the `target_key` column and as the
/// `drop_reason` field of `metadata` — the column drives the AC3 dashboard
/// query; the JSON copy keeps the row self-describing when the whole row is
/// exported (log shippers, JSON dumps).
pub(crate) async fn log_webhook_drop(
    pool: &PgPool,
    ctx: &WebhookDropContext<'_>,
    drop_reason: &str,
) {
    let metadata = build_drop_metadata(ctx, drop_reason);
    let result = sqlx::query(
        r#"
        INSERT INTO audit_events (tool_name, target_key, metadata)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(TOOL_NAME)
    .bind(drop_reason)
    .bind(&metadata)
    .execute(pool)
    .await;

    if let Err(e) = result {
        warn!(
            drop_reason,
            event_type = ctx.event_type,
            delivery_id = ctx.delivery_id,
            error = %e,
            "failed to persist gateway_webhook audit_event (drop decision unchanged)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ctx() -> WebhookDropContext<'static> {
        WebhookDropContext {
            event_type: "issues",
            action: Some("labeled"),
            check_conclusion: None,
            delivery_id: "12345678-1234-1234-1234-123456789abc",
            repo_full_name: Some("senara-solutions/mika"),
        }
    }

    #[test]
    fn drop_no_route_metadata_shape() {
        let ctx = WebhookDropContext {
            event_type: "check_suite",
            action: Some("completed"),
            check_conclusion: Some("cancelled"),
            delivery_id: "delivery-noroute",
            repo_full_name: Some("senara-solutions/mika"),
        };
        let meta = build_drop_metadata(&ctx, DROP_NO_ROUTE);
        assert_eq!(meta["event_type"], "check_suite");
        assert_eq!(meta["action"], "completed");
        assert_eq!(meta["check_conclusion"], "cancelled");
        assert_eq!(meta["delivery_id"], "delivery-noroute");
        assert_eq!(meta["repo_full_name"], "senara-solutions/mika");
        assert_eq!(meta["drop_reason"], DROP_NO_ROUTE);
    }

    #[test]
    fn drop_reviewer_filter_metadata_shape() {
        let ctx = WebhookDropContext {
            event_type: "pull_request",
            action: Some("review_requested"),
            check_conclusion: None,
            delivery_id: "delivery-reviewer",
            repo_full_name: Some("senara-solutions/mika"),
        };
        let meta = build_drop_metadata(&ctx, DROP_REVIEWER_FILTER);
        assert_eq!(meta["event_type"], "pull_request");
        assert_eq!(meta["action"], "review_requested");
        assert!(meta["check_conclusion"].is_null());
        assert_eq!(meta["delivery_id"], "delivery-reviewer");
        assert_eq!(meta["repo_full_name"], "senara-solutions/mika");
        assert_eq!(meta["drop_reason"], DROP_REVIEWER_FILTER);
    }

    #[test]
    fn drop_denylisted_skill_metadata_shape() {
        let ctx = base_ctx();
        let meta = build_drop_metadata(&ctx, DROP_DENYLISTED_SKILL);
        assert_eq!(meta["event_type"], "issues");
        assert_eq!(meta["action"], "labeled");
        assert_eq!(meta["drop_reason"], DROP_DENYLISTED_SKILL);
    }

    #[test]
    fn drop_synchronize_no_diff_metadata_shape() {
        let ctx = WebhookDropContext {
            event_type: "pull_request",
            action: Some("synchronize"),
            check_conclusion: None,
            delivery_id: "delivery-nodiff",
            repo_full_name: Some("senara-solutions/mika"),
        };
        let meta = build_drop_metadata(&ctx, DROP_SYNCHRONIZE_NO_DIFF);
        assert_eq!(meta["event_type"], "pull_request");
        assert_eq!(meta["action"], "synchronize");
        assert!(meta["check_conclusion"].is_null());
        assert_eq!(meta["delivery_id"], "delivery-nodiff");
        assert_eq!(meta["drop_reason"], DROP_SYNCHRONIZE_NO_DIFF);
    }

    #[test]
    fn null_optionals_serialize_as_json_null() {
        let ctx = WebhookDropContext {
            event_type: "issues",
            action: None,
            check_conclusion: None,
            delivery_id: "",
            repo_full_name: None,
        };
        let meta = build_drop_metadata(&ctx, DROP_NO_ROUTE);
        assert!(meta["action"].is_null());
        assert!(meta["check_conclusion"].is_null());
        assert!(meta["repo_full_name"].is_null());
        assert_eq!(meta["delivery_id"], "");
    }

    #[test]
    fn drop_reason_constants_match_ticket_spec() {
        // Load-bearing for the AC3 dashboard query — any rename here breaks
        // downstream operator SQL and the mika#1711 root-class detection.
        assert_eq!(TOOL_NAME, "gateway_webhook");
        assert_eq!(DROP_NO_ROUTE, "webhook_no_route");
        assert_eq!(DROP_REVIEWER_FILTER, "webhook_reviewer_filter_dropped");
        assert_eq!(DROP_DENYLISTED_SKILL, "webhook_denylisted_skill_dropped");
        assert_eq!(
            DROP_SYNCHRONIZE_NO_DIFF,
            "webhook_synchronize_no_diff_change"
        );
    }
}
