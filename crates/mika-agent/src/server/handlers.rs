use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::Timelike;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::Instrument;
use tracing::{error, info, warn};

use crate::agent::{
    self, AgentParams, SilentAgentParams, SilentTrigger, check_onboarding, run_silent_agent,
};
use crate::async_db::AsyncDatabase;
use crate::compaction;
use crate::messaging::{GatewayMessageSender, MessageSender};

use super::json_extractor::JsonBody;
use super::state::AppState;
use super::types::{AcceptedResponse, HealthResponse, HeartbeatRequest, MessageRequest};

/// GET /health — K8s liveness/readiness probe (no auth required).
pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    if !state.ready.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "starting".to_string(),
                uptime_secs: None,
            }),
        );
    }
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ok".to_string(),
            uptime_secs: Some(state.startup_time.elapsed().as_secs()),
        }),
    )
}

/// POST /message — gateway forwards a user message for async processing.
///
/// Returns 202 Accepted immediately, then spawns the agent loop in background.
/// Agent responses are delivered outbound via GatewayMessageSender.
pub async fn handle_message(
    State(state): State<AppState>,
    JsonBody(req): JsonBody<MessageRequest>,
) -> impl IntoResponse {
    // Validate input
    if req.text.is_empty() || req.text.len() > 50_000 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "text must be 1-50000 characters"
            })),
        )
            .into_response();
    }

    // Try to acquire the agent lock (non-blocking)
    let lock = match state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "request_id": req.request_id,
                    "error": "agent busy"
                })),
            )
                .into_response();
        }
    };

    // Store chat_id on every message (for outbound sends)
    let _ = state
        .db
        .set_customer_config("chat_id", &req.chat_id.to_string())
        .await;

    let request_id = req.request_id.clone();

    // Spawn flush of previously failed sends in parallel (best-effort, non-blocking)
    let flush_state = state.clone();
    tokio::spawn(async move {
        flush_failed_sends(&flush_state).await;
    });

    // Spawn async agent processing with request_id span for log correlation
    let s = state.clone();
    let span = tracing::info_span!("process_message", request_id = %request_id);
    tokio::spawn(
        async move {
            let _lock = lock; // Hold lock for duration of agent loop

            let session_id = uuid::Uuid::new_v4().to_string();
            let is_onboarding = check_onboarding(&s.db).await;

            let sender = GatewayMessageSender::new(
                s.gateway_url.clone(),
                s.internal_token.clone(),
                s.db.clone(),
                s.http_client.clone(),
                Some(req.request_id.clone()),
            );
            let sender_arc: Arc<dyn MessageSender> = Arc::new(sender);

            let params = AgentParams {
                db: &s.db,
                claude: &s.claude,
                tools: &s.tools,
                skills: &s.skills,
                user_message: &req.text,
                channel_type: &req.channel,
                session_id: &session_id,
                home_dir: &s.home_dir,
                is_onboarding,
                message_sender: Some(sender_arc.clone()),
                skip_compaction: true,
                embedding_client: s.embedding_client.as_ref(),
            };

            match agent::run_agent(&params).await {
                Ok(response) => {
                    info!("agent loop completed");
                    // Send final agent response to user via gateway
                    if !response.is_empty() {
                        if let Err(e) = sender_arc.send(&response).await {
                            error!(error = %e, "failed to send response");
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "agent loop failed");
                    let _ = sender_arc
                        .send("Sorry, I had a hiccup processing your message. Could you try again?")
                        .await;
                }
            }

            // Spawn compaction outside the lock
            drop(_lock);
            let db = s.db.clone();
            let claude = s.claude.clone();
            tokio::spawn(async move {
                if let Err(e) = compaction::maybe_compact(&db, &claude).await {
                    warn!(error = %e, "post-turn compaction failed");
                }
            });
        }
        .instrument(span),
    );

    (
        StatusCode::ACCEPTED,
        Json(AcceptedResponse {
            request_id,
            status: "accepted".to_string(),
        }),
    )
        .into_response()
}

/// POST /heartbeat — K8s CronJob triggers proactive check-in.
///
/// Pre-filters (active hours, rate limits) without acquiring Mutex.
/// Returns 204 if skipped, 200 if accepted for processing.
pub async fn handle_heartbeat(
    State(state): State<AppState>,
    JsonBody(req): JsonBody<HeartbeatRequest>,
) -> impl IntoResponse {
    info!(request_id = %req.request_id, "heartbeat received");

    // Pre-filter: check if heartbeat should run (no Mutex, no Claude call)
    if !heartbeat_should_run(&state.db).await {
        info!(request_id = %req.request_id, "heartbeat skipped by pre-filter");
        return StatusCode::NO_CONTENT;
    }

    // try_lock — heartbeat is skippable if agent is busy
    let lock = match state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => return StatusCode::NO_CONTENT,
    };

    // Spawn silent agent loop
    let s = state.clone();
    tokio::spawn(async move {
        let _lock = lock;
        let session_id = uuid::Uuid::new_v4().to_string();

        let sender = GatewayMessageSender::new(
            s.gateway_url.clone(),
            s.internal_token.clone(),
            s.db.clone(),
            s.http_client.clone(),
            Some(req.request_id.clone()),
        );
        let sender_arc: Arc<dyn MessageSender> = Arc::new(sender);

        let params = SilentAgentParams {
            db: &s.db,
            claude: &s.claude,
            tools: &s.tools,
            skills: &s.skills,
            trigger: SilentTrigger::Heartbeat,
            home_dir: &s.home_dir,
            session_id: &session_id,
            message_sender: Some(sender_arc),
            embedding_client: s.embedding_client.as_ref(),
        };

        if let Err(e) = run_silent_agent(&params).await {
            warn!(error = %e, "heartbeat agent loop failed");
        }

        // Record the heartbeat send for rate limiting
        if let Err(e) = s.db.record_heartbeat_send().await {
            warn!(error = %e, "failed to record heartbeat send");
        }
    });

    StatusCode::OK
}

/// Flush previously failed outbound sends (best-effort, up to 5).
async fn flush_failed_sends(state: &AppState) {
    let sends = match state.db.get_pending_failed_sends(5).await {
        Ok(s) if !s.is_empty() => s,
        _ => return,
    };

    let sender = GatewayMessageSender::new(
        state.gateway_url.clone(),
        state.internal_token.clone(),
        state.db.clone(),
        state.http_client.clone(),
        None,
    );

    for send in sends {
        match sender.send(&send.text).await {
            Ok(()) => {
                let _ = state.db.delete_failed_send(send.id).await;
                info!(id = send.id, "flushed failed send");
            }
            Err(_) => {
                let _ = state.db.increment_failed_send_retry(send.id).await;
            }
        }
    }
}

/// Pre-filter for heartbeat: checks active hours, rate limits, and recent activity.
/// Returns false if the heartbeat should be skipped (cheap — no Mutex or Claude call).
async fn heartbeat_should_run(db: &AsyncDatabase) -> bool {
    let tz_str = db
        .get_customer_config("timezone")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "UTC".to_string());

    // 1. Active hours check (8:00-21:00 in customer's timezone)
    let now_utc = chrono::Utc::now();
    let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
    let now_local = now_utc.with_timezone(&tz);
    let hour = now_local.hour();
    if !(8..21).contains(&hour) {
        return false;
    }

    // 2. Rate limit: max 1 heartbeat per hour
    if db.count_heartbeat_sends_last_hour().await.unwrap_or(0) >= 1 {
        return false;
    }

    // 3. Rate limit: max 3 heartbeats per day
    if db.count_heartbeat_sends_today(&tz_str).await.unwrap_or(0) >= 3 {
        return false;
    }

    // 4. Skip if user messaged recently (within last 2 hours)
    if let Ok(Some(last_msg)) = db.last_user_message_time().await {
        if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(&last_msg, "%Y-%m-%d %H:%M:%S") {
            let elapsed = now_utc.naive_utc() - parsed;
            if elapsed < chrono::TimeDelta::hours(2) {
                return false;
            }
        }
    }

    true
}
