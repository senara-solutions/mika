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

/// mika#2360 — `tool_name` written for every served admin read of a tenant's
/// recurring registry. Deliberately NOT [`TOOL_NAME`]: that constant carries
/// the mika#1774 webhook-drop population, and mixing admin reads into it
/// would split that query without saying so. Operator query:
/// `SELECT target_key, created_at FROM audit_events
///  WHERE tool_name = 'gateway_admin_read' ORDER BY created_at DESC;`
pub(crate) const TOOL_NAME_ADMIN_READ: &str = "gateway_admin_read";

/// mika#2360 — `target_key` for an admin read of one tenant's registry. The id
/// is a validated `Uuid`, so the indexed column never carries caller text.
pub(crate) fn admin_read_target_key(customer_id: &uuid::Uuid) -> String {
    format!("tenant:{customer_id}")
}

/// mika#2360 — `metadata.route` of a read of the tenant's recurring registry.
pub(crate) const ADMIN_READ_ROUTE_RECURRING_TASKS: &str =
    "GET /admin/tenants/{customer_id}/recurring-tasks";

/// mika#2387 — `metadata.route` of a read of the tenant's outbound-send
/// history. Distinct from [`ADMIN_READ_ROUTE_RECURRING_TASKS`], and that
/// distinction is the whole reason [`log_admin_read`] takes a `route`: both
/// routes share one `tool_name` (one auth scope, one population, one operator
/// query "who read this tenant's data"), so `metadata->>'route'` is the only
/// thing that can tell them apart. An audit row naming the wrong route is
/// worse than no audit row at all.
pub(crate) const ADMIN_READ_ROUTE_OUTBOUND_MESSAGES: &str =
    "GET /admin/tenants/{customer_id}/outbound-messages";

/// Serialize the `metadata` JSONB shape of one admin read. Split out from the
/// DB write for the same reason as [`build_drop_metadata`]: the shape — and in
/// particular which route the row names — is then exercisable by a pure unit
/// test, with no Postgres.
pub(crate) fn build_admin_read_metadata(
    customer_id: &uuid::Uuid,
    route: &str,
) -> serde_json::Value {
    json!({
        "route": route,
        "customer_id": customer_id,
    })
}

/// mika#2360 — persist one admin read of a tenant's data under the shared
/// `gateway_admin_read` scope. Fire-and-forget on the model of
/// [`log_webhook_drop`]: a DB failure logs a WARN and never changes the
/// response (a read must not depend on a write).
///
/// `route` must be one of the `ADMIN_READ_ROUTE_*` constants — mika#2387 made
/// it a parameter rather than a literal, because a second caller writing rows
/// that name the first caller's route would make the audit answer false.
pub(crate) async fn log_admin_read(pool: &PgPool, customer_id: &uuid::Uuid, route: &str) {
    let target_key = admin_read_target_key(customer_id);
    let metadata = build_admin_read_metadata(customer_id, route);
    let result = sqlx::query(
        r#"
        INSERT INTO audit_events (tool_name, target_key, metadata)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(TOOL_NAME_ADMIN_READ)
    .bind(&target_key)
    .bind(&metadata)
    .execute(pool)
    .await;

    if let Err(e) = result {
        warn!(
            target_key,
            route,
            error = %e,
            "failed to persist gateway_admin_read audit_event (response unchanged)"
        );
    }
}

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

    /// mika#2360 R9 — the admin-read population is its own `tool_name`, never
    /// merged into the webhook-drop one, and the key is `tenant:{uuid}`.
    #[test]
    fn mika2360_admin_read_audit_constants() {
        assert_eq!(TOOL_NAME_ADMIN_READ, "gateway_admin_read");
        assert_ne!(
            TOOL_NAME_ADMIN_READ, TOOL_NAME,
            "admin reads must not be written under the webhook-drop tool_name"
        );
        let id = uuid::Uuid::parse_str("a0394c24-9558-4cb6-9078-52043912ecbc").unwrap();
        assert_eq!(
            admin_read_target_key(&id),
            "tenant:a0394c24-9558-4cb6-9078-52043912ecbc"
        );
    }

    /// mika#2387 T3 — the audit row of an outbound-messages read names **its
    /// own** route. The `assert_ne!` half is what carries the test: a writer
    /// left hard-coded on mika#2360's literal would still produce a row with a
    /// plausible `route` field, and only a comparison against that literal
    /// catches it.
    ///
    /// The `assert_eq!` on the recurring-tasks metadata is the non-regression
    /// half of U1: parameterizing `route` must leave mika#2360's own audit
    /// value unchanged, byte for byte.
    #[test]
    fn mika2387_audit_route_names_this_endpoint() {
        let id = uuid::Uuid::parse_str("a0394c24-9558-4cb6-9078-52043912ecbc").unwrap();

        let outbound = build_admin_read_metadata(&id, ADMIN_READ_ROUTE_OUTBOUND_MESSAGES);
        assert_eq!(
            outbound["route"],
            "GET /admin/tenants/{customer_id}/outbound-messages"
        );
        assert_ne!(
            outbound["route"], ADMIN_READ_ROUTE_RECURRING_TASKS,
            "an outbound-messages read must not be recorded as a registry read"
        );
        assert_eq!(
            outbound["customer_id"],
            "a0394c24-9558-4cb6-9078-52043912ecbc"
        );

        // Non-regression: mika#2360's row is unchanged by the parameterization.
        let recurring = build_admin_read_metadata(&id, ADMIN_READ_ROUTE_RECURRING_TASKS);
        assert_eq!(
            recurring["route"],
            "GET /admin/tenants/{customer_id}/recurring-tasks"
        );

        // One scope, one population: the discriminant is the route, not the
        // tool_name (mika#2387 D1).
        assert_ne!(
            ADMIN_READ_ROUTE_OUTBOUND_MESSAGES,
            ADMIN_READ_ROUTE_RECURRING_TASKS
        );
    }

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
