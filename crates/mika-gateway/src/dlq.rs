//! Dead-letter queue (DLQ) for GitHub webhook deliveries that exhaust retries.
//!
//! Provides persistence, background retry, and HTTP endpoints for operator
//! inspection and manual replay. See issue #590.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use tracing::{error, info, warn};

use crate::github::{self, ForwardResult};
use crate::routes::AppState;

// -- Types --

/// Status of a webhook delivery in the DLQ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[allow(dead_code)]
pub(crate) enum DeliveryStatus {
    Pending,
    Delivered,
    Dead,
}

impl std::fmt::Display for DeliveryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Delivered => write!(f, "delivered"),
            Self::Dead => write!(f, "dead"),
        }
    }
}

/// A webhook delivery row from the `webhook_deliveries` table.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub(crate) struct WebhookDelivery {
    pub delivery_id: String,
    pub event_type: String,
    pub target_agent: String,
    pub repo_full_name: Option<String>,
    pub payload: String,
    pub request_id: String,
    pub created_at: DateTime<Utc>,
    pub attempts: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub status: String,
    pub last_error: Option<String>,
}

/// Parameters for inserting a new DLQ entry.
pub(crate) struct NewDelivery<'a> {
    pub delivery_id: &'a str,
    pub event_type: &'a str,
    pub target_agent: &'a str,
    pub repo_full_name: Option<&'a str>,
    pub payload: &'a str,
    pub request_id: &'a str,
    pub attempts: i32,
    pub last_error: &'a str,
}

// -- Write path --

/// Insert a failed delivery into the DLQ. Fire-and-forget: logs errors but
/// never propagates them to the caller.
pub(crate) async fn insert_delivery(pool: &PgPool, delivery: NewDelivery<'_>) {
    let result = sqlx::query(
        r#"
        INSERT INTO webhook_deliveries
            (delivery_id, event_type, target_agent, repo_full_name, payload, request_id, attempts, last_error, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending')
        ON CONFLICT (delivery_id) DO NOTHING
        "#,
    )
    .bind(delivery.delivery_id)
    .bind(delivery.event_type)
    .bind(delivery.target_agent)
    .bind(delivery.repo_full_name)
    .bind(delivery.payload)
    .bind(delivery.request_id)
    .bind(delivery.attempts)
    .bind(delivery.last_error)
    .execute(pool)
    .await;

    match result {
        Ok(_) => {
            info!(
                delivery_id = delivery.delivery_id,
                target_agent = delivery.target_agent,
                attempts = delivery.attempts,
                "webhook delivery added to DLQ"
            );
        }
        Err(e) => {
            error!(
                delivery_id = delivery.delivery_id,
                error = %e,
                "failed to insert webhook delivery into DLQ"
            );
        }
    }
}

// -- Background worker --

/// Maximum number of worker retry attempts before marking a delivery as dead.
const MAX_WORKER_ATTEMPTS: i32 = 10;

/// How often the worker wakes to check for pending deliveries.
const WORKER_INTERVAL: Duration = Duration::from_secs(30);

/// Maximum deliveries to process per worker tick.
const WORKER_BATCH_SIZE: i64 = 50;

/// Run the DLQ background worker. Wakes every 30s, retries pending deliveries
/// that are past their backoff window, and transitions them to delivered or dead.
///
/// This function runs forever — spawn it as a tokio task.
pub(crate) async fn run_dlq_worker(state: AppState) {
    info!("DLQ background worker started");
    loop {
        tokio::time::sleep(WORKER_INTERVAL).await;
        if let Err(e) = process_pending_deliveries(&state).await {
            error!(error = %e, "DLQ worker tick failed");
        }
    }
}

/// Process one batch of pending deliveries. Exposed for testing.
async fn process_pending_deliveries(state: &AppState) -> anyhow::Result<()> {
    // Select pending rows whose backoff window has elapsed.
    // Backoff formula: 30s * 2^attempts (capped at 1 hour).
    let rows = sqlx::query_as::<_, WebhookDelivery>(
        r#"
        SELECT *
        FROM webhook_deliveries
        WHERE status = 'pending'
          AND (
            last_attempt_at IS NULL
            OR last_attempt_at < now() - (LEAST(30 * power(2, attempts), 3600) || ' seconds')::interval
          )
        ORDER BY created_at ASC
        LIMIT $1
        "#,
    )
    .bind(WORKER_BATCH_SIZE)
    .fetch_all(&state.pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    info!(
        count = rows.len(),
        "DLQ worker processing pending deliveries"
    );

    for row in rows {
        process_single_delivery(state, &row).await;
    }

    Ok(())
}

/// Process a single DLQ entry: resolve route, attempt delivery, update status.
async fn process_single_delivery(state: &AppState, delivery: &WebhookDelivery) {
    // Respect semaphore backpressure
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!(
                delivery_id = %delivery.delivery_id,
                "DLQ worker skipping delivery — semaphore at capacity"
            );
            return;
        }
    };

    // Re-resolve route (container may have moved since original failure)
    let route =
        match github::resolve_github_container_url(state, delivery.repo_full_name.as_deref()).await
        {
            Some(r) => r,
            None => {
                let error_msg = "route resolution failed (no repo mapping or fallback)";
                update_delivery_failure(
                    &state.pool,
                    &delivery.delivery_id,
                    delivery.attempts,
                    error_msg,
                )
                .await;
                return;
            }
        };

    // Attempt delivery
    let result = github::forward_to_resolved_route(
        state,
        &route,
        &delivery.target_agent,
        &delivery.payload,
        &delivery.request_id,
        delivery.repo_full_name.as_deref(),
    )
    .await;

    // Release permit after attempt
    drop(permit);

    match result {
        ForwardResult::Success => {
            mark_delivered(&state.pool, &delivery.delivery_id).await;
            info!(
                delivery_id = %delivery.delivery_id,
                target_agent = %delivery.target_agent,
                "DLQ delivery succeeded"
            );
        }
        ForwardResult::Retryable { reason } | ForwardResult::Permanent { reason } => {
            update_delivery_failure(
                &state.pool,
                &delivery.delivery_id,
                delivery.attempts,
                &reason,
            )
            .await;
        }
    }
}

/// Mark a delivery as successfully delivered.
async fn mark_delivered(pool: &PgPool, delivery_id: &str) {
    if let Err(e) = sqlx::query(
        "UPDATE webhook_deliveries SET status = 'delivered', last_attempt_at = now() WHERE delivery_id = $1",
    )
    .bind(delivery_id)
    .execute(pool)
    .await
    {
        error!(delivery_id, error = %e, "failed to mark DLQ delivery as delivered");
    }
}

/// Increment attempts and record the failure. Transitions to 'dead' if max attempts reached.
async fn update_delivery_failure(
    pool: &PgPool,
    delivery_id: &str,
    current_attempts: i32,
    error_msg: &str,
) {
    let new_attempts = current_attempts + 1;
    let new_status = if new_attempts >= MAX_WORKER_ATTEMPTS {
        "dead"
    } else {
        "pending"
    };

    if new_status == "dead" {
        warn!(
            delivery_id,
            attempts = new_attempts,
            "DLQ delivery exhausted worker retry budget, marking dead"
        );
    }

    if let Err(e) = sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET attempts = $1, last_attempt_at = now(), last_error = $2, status = $3
        WHERE delivery_id = $4
        "#,
    )
    .bind(new_attempts)
    .bind(error_msg)
    .bind(new_status)
    .bind(delivery_id)
    .execute(pool)
    .await
    {
        error!(delivery_id, error = %e, "failed to update DLQ delivery failure");
    }
}

// -- Query/Replay (HTTP endpoint helpers) --

/// List deliveries by status. If status is None, returns both pending and dead.
pub(crate) async fn list_deliveries(
    pool: &PgPool,
    status_filter: Option<&str>,
    limit: i64,
) -> anyhow::Result<Vec<WebhookDelivery>> {
    let rows = match status_filter {
        Some(status) => {
            sqlx::query_as::<_, WebhookDelivery>(
                "SELECT * FROM webhook_deliveries WHERE status = $1 ORDER BY created_at DESC LIMIT $2",
            )
            .bind(status)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, WebhookDelivery>(
                "SELECT * FROM webhook_deliveries WHERE status IN ('pending', 'dead') ORDER BY created_at DESC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

/// Replay a single delivery by ID. Returns the updated delivery or None if not found.
pub(crate) async fn replay_delivery(
    state: &AppState,
    delivery_id: &str,
) -> anyhow::Result<Option<WebhookDelivery>> {
    // Fetch the delivery
    let delivery = sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM webhook_deliveries WHERE delivery_id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.pool)
    .await?;

    let delivery = match delivery {
        Some(d) => d,
        None => return Ok(None),
    };

    // Attempt replay
    replay_single(state, &delivery).await;

    // Return updated row
    let updated = sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM webhook_deliveries WHERE delivery_id = $1",
    )
    .bind(delivery_id)
    .fetch_optional(&state.pool)
    .await?;

    Ok(updated)
}

/// Replay all dead deliveries. Returns (succeeded, failed) counts.
pub(crate) async fn replay_all_dead(state: &AppState) -> anyhow::Result<(u32, u32)> {
    let rows = sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM webhook_deliveries WHERE status = 'dead' ORDER BY created_at ASC",
    )
    .fetch_all(&state.pool)
    .await?;

    let mut succeeded = 0u32;
    let mut failed = 0u32;

    for row in &rows {
        replay_single(state, row).await;

        // Check result
        let updated = sqlx::query_scalar::<_, String>(
            "SELECT status FROM webhook_deliveries WHERE delivery_id = $1",
        )
        .bind(&row.delivery_id)
        .fetch_one(&state.pool)
        .await?;

        if updated == "delivered" {
            succeeded += 1;
        } else {
            failed += 1;
        }
    }

    Ok((succeeded, failed))
}

/// Replay a single delivery entry (shared by replay_delivery and replay_all_dead).
async fn replay_single(state: &AppState, delivery: &WebhookDelivery) {
    // Acquire semaphore permit
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            let error_msg = "semaphore at capacity during replay";
            warn!(
                delivery_id = %delivery.delivery_id,
                "DLQ replay skipped — {error_msg}"
            );
            update_delivery_failure(
                &state.pool,
                &delivery.delivery_id,
                delivery.attempts,
                error_msg,
            )
            .await;
            return;
        }
    };

    // Re-resolve route
    let route =
        match github::resolve_github_container_url(state, delivery.repo_full_name.as_deref()).await
        {
            Some(r) => r,
            None => {
                let error_msg = "route resolution failed during replay";
                update_delivery_failure(
                    &state.pool,
                    &delivery.delivery_id,
                    delivery.attempts,
                    error_msg,
                )
                .await;
                return;
            }
        };

    // Attempt delivery
    let result = github::forward_to_resolved_route(
        state,
        &route,
        &delivery.target_agent,
        &delivery.payload,
        &delivery.request_id,
        delivery.repo_full_name.as_deref(),
    )
    .await;

    drop(permit);

    match result {
        ForwardResult::Success => {
            mark_delivered(&state.pool, &delivery.delivery_id).await;
            info!(
                delivery_id = %delivery.delivery_id,
                "DLQ replay succeeded"
            );
        }
        ForwardResult::Retryable { reason } | ForwardResult::Permanent { reason } => {
            update_delivery_failure(
                &state.pool,
                &delivery.delivery_id,
                delivery.attempts,
                &reason,
            )
            .await;
            warn!(
                delivery_id = %delivery.delivery_id,
                reason = %reason,
                "DLQ replay failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_status_display() {
        assert_eq!(DeliveryStatus::Pending.to_string(), "pending");
        assert_eq!(DeliveryStatus::Delivered.to_string(), "delivered");
        assert_eq!(DeliveryStatus::Dead.to_string(), "dead");
    }

    #[test]
    fn max_attempts_threshold() {
        // Verify the constant matches the issue spec
        assert_eq!(MAX_WORKER_ATTEMPTS, 10);
    }
}
