//! GitHub webhook handler for mika-gateway.
//!
//! Receives GitHub App webhook events at `POST /webhook/github`, validates the
//! HMAC-SHA256 signature, routes to the correct agent name, and forwards to the
//! agent container via `POST {container_url}/message`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use rand::Rng;
use secrecy::ExposeSecret;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::{debug, error, info, warn};

use crate::routes::AppState;

// -- HMAC-SHA256 signature validation --

type HmacSha256 = Hmac<Sha256>;

/// Validate a GitHub webhook signature (`X-Hub-Signature-256` header) against the raw body.
///
/// The header value is expected to be `sha256=<hex-encoded HMAC digest>`.
/// Uses constant-time comparison to prevent timing attacks.
pub fn validate_signature(secret: &[u8], body: &[u8], signature_header: &str) -> bool {
    let hex_sig = match signature_header.strip_prefix("sha256=") {
        Some(s) => s,
        None => return false,
    };
    let Ok(expected) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    bool::from(computed.ct_eq(&expected))
}

// -- Event types --

/// Minimal representation of a GitHub webhook event payload.
/// Uses `Option` for all fields since GitHub frequently adds new fields
/// and not all event types have the same shape.
#[derive(Debug, serde::Deserialize)]
pub struct GitHubWebhookEvent {
    pub action: Option<String>,
    pub sender: Option<GitHubUser>,
    /// Present on events triggered by a GitHub App installation.
    pub installation: Option<GitHubInstallation>,
    /// check_suite-specific payload (only present for check_suite events).
    pub check_suite: Option<CheckSuite>,
    /// Issue data (present in issues and issue_comment events).
    pub issue: Option<GitHubIssue>,
    /// Pull request data (present in pull_request and pull_request_review events).
    pub pull_request: Option<GitHubPullRequest>,
    /// Comment data (present in issue_comment events).
    pub comment: Option<GitHubComment>,
    /// Review data (present in pull_request_review events).
    pub review: Option<GitHubReview>,
    /// Repository data.
    pub repository: Option<GitHubRepository>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubUser {
    pub login: String,
    #[serde(rename = "type")]
    pub user_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubInstallation {
    pub id: u64,
}

#[derive(Debug, serde::Deserialize)]
pub struct CheckSuite {
    pub conclusion: Option<String>,
    pub head_branch: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubIssue {
    pub number: Option<u64>,
    pub title: Option<String>,
    pub html_url: Option<String>,
    pub body: Option<String>,
    pub assignee: Option<GitHubUser>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubPullRequest {
    pub number: Option<u64>,
    pub title: Option<String>,
    pub html_url: Option<String>,
    pub body: Option<String>,
    pub head: Option<GitHubRef>,
    pub merged: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubRef {
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubComment {
    pub body: Option<String>,
    pub html_url: Option<String>,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubReview {
    pub state: Option<String>,
    pub body: Option<String>,
    pub html_url: Option<String>,
    pub user: Option<GitHubUser>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubRepository {
    pub full_name: Option<String>,
    pub html_url: Option<String>,
}

// -- Event routing --

/// Route a GitHub event to the target agent name based on event type and action.
///
/// Returns `None` for unroutable events (silently dropped).
/// The `check_conclusion` parameter is only relevant for `check_suite` events.
pub fn route_event(
    event_type: &str,
    action: Option<&str>,
    check_conclusion: Option<&str>,
) -> Option<&'static str> {
    match (event_type, action) {
        ("issues", Some("assigned")) => Some("mika-dev"),
        ("issue_comment", Some("created")) => Some("mika-dev"),
        ("pull_request", Some("opened" | "synchronize")) => Some("mika-qa"),
        ("pull_request", Some("closed")) => Some("mika-dev"),
        ("pull_request_review", Some("submitted")) => Some("mika-dev"),
        ("check_suite", Some("completed")) => match check_conclusion {
            Some("failure" | "timed_out" | "success") => Some("mika-dev"),
            _ => None,
        },
        _ => None,
    }
}

// -- Message text formatting --

/// Truncate text to `max_chars`, appending a truncation indicator if truncated.
fn truncate_body(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{truncated}\n\n[truncated]")
    }
}

/// Format a GitHub webhook event into a markdown summary suitable for the agent's context.
///
/// Includes event type, action, repository, relevant titles/bodies, and URLs.
pub fn format_event_text(event_type: &str, event: &GitHubWebhookEvent) -> String {
    let repo_name = event
        .repository
        .as_ref()
        .and_then(|r| r.full_name.as_deref())
        .unwrap_or("unknown");
    let action = event.action.as_deref().unwrap_or("unknown");

    match event_type {
        "issues" => {
            let issue = event.issue.as_ref();
            let number = issue.and_then(|i| i.number).unwrap_or(0);
            let title = issue
                .and_then(|i| i.title.as_deref())
                .unwrap_or("(no title)");
            let url = issue.and_then(|i| i.html_url.as_deref()).unwrap_or("");
            let body = truncate_body(issue.and_then(|i| i.body.as_deref()).unwrap_or(""), 2000);

            let mut text =
                format!("[GitHub] Issue {action}: {repo_name}#{number} — {title}\n{url}");
            if !body.is_empty() {
                text.push_str(&format!("\n\n{body}"));
            }
            if action == "assigned"
                && let Some(assignee) = issue.and_then(|i| i.assignee.as_ref())
            {
                text.push_str(&format!("\nAssigned to: @{}", assignee.login));
            }
            text
        }
        "issue_comment" => {
            let issue = event.issue.as_ref();
            let number = issue.and_then(|i| i.number).unwrap_or(0);
            let title = issue
                .and_then(|i| i.title.as_deref())
                .unwrap_or("(no title)");
            let comment = event.comment.as_ref();
            let commenter = comment
                .and_then(|c| c.user.as_ref())
                .map(|u| u.login.as_str())
                .unwrap_or("unknown");
            let comment_url = comment.and_then(|c| c.html_url.as_deref()).unwrap_or("");
            let body = truncate_body(comment.and_then(|c| c.body.as_deref()).unwrap_or(""), 2000);

            format!(
                "[GitHub] New comment on {repo_name}#{number} ({title}) by @{commenter}\n{comment_url}\n\n{body}"
            )
        }
        "pull_request" => {
            let pr = event.pull_request.as_ref();
            let number = pr.and_then(|p| p.number).unwrap_or(0);
            let title = pr.and_then(|p| p.title.as_deref()).unwrap_or("(no title)");
            let url = pr.and_then(|p| p.html_url.as_deref()).unwrap_or("");
            let branch = pr
                .and_then(|p| p.head.as_ref())
                .and_then(|h| h.ref_name.as_deref())
                .unwrap_or("unknown");
            let body = truncate_body(pr.and_then(|p| p.body.as_deref()).unwrap_or(""), 2000);

            let mut text = format!(
                "[GitHub] PR {action}: {repo_name}#{number} — {title} (branch: {branch})\n{url}"
            );
            if action == "closed" {
                let merged = pr.and_then(|p| p.merged).unwrap_or(false);
                text.push_str(&format!("\nMerged: {merged}"));
            }
            if !body.is_empty() {
                text.push_str(&format!("\n\n{body}"));
            }
            text
        }
        "pull_request_review" => {
            let pr = event.pull_request.as_ref();
            let number = pr.and_then(|p| p.number).unwrap_or(0);
            let title = pr.and_then(|p| p.title.as_deref()).unwrap_or("(no title)");
            let review = event.review.as_ref();
            let reviewer = review
                .and_then(|r| r.user.as_ref())
                .map(|u| u.login.as_str())
                .unwrap_or("unknown");
            let state = review.and_then(|r| r.state.as_deref()).unwrap_or("unknown");
            let review_url = review.and_then(|r| r.html_url.as_deref()).unwrap_or("");
            let body = truncate_body(review.and_then(|r| r.body.as_deref()).unwrap_or(""), 2000);

            let mut text = format!(
                "[GitHub] PR review ({state}) on {repo_name}#{number} ({title}) by @{reviewer}\n{review_url}"
            );
            if !body.is_empty() {
                text.push_str(&format!("\n\n{body}"));
            }
            text
        }
        "check_suite" => {
            let cs = event.check_suite.as_ref();
            let conclusion = cs
                .and_then(|c| c.conclusion.as_deref())
                .unwrap_or("unknown");
            let branch = cs
                .and_then(|c| c.head_branch.as_deref())
                .unwrap_or("unknown");

            format!("[GitHub] Check suite {conclusion} on {repo_name} (branch: {branch})")
        }
        _ => {
            format!("[GitHub] {event_type}.{action} on {repo_name}")
        }
    }
}

// -- Agent mapping --

/// Result of resolving a GitHub repo to a customer container.
pub(crate) struct ResolvedRoute {
    pub(crate) container_url: String,
    pub(crate) agent_mapping: serde_json::Value,
}

/// Validate that an agent name is well-formed: lowercase alphanumeric + hyphens,
/// 1-63 chars, no leading/trailing/consecutive hyphens.
fn is_valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// Apply per-repo agent name overrides from the `agent_mapping` JSONB column.
///
/// Keys are default agent names from `route_event()` (e.g. `"mika-dev"`),
/// values are the customer's replacement agent names (e.g. `"acme-dev"`).
/// Returns the original agent name if no override exists or the override
/// is not a valid agent name.
fn apply_agent_mapping(agent_mapping: &serde_json::Value, default_agent: &str) -> String {
    agent_mapping
        .get(default_agent)
        .and_then(|v| v.as_str())
        .filter(|s| is_valid_agent_name(s))
        .unwrap_or(default_agent)
        .to_string()
}

// -- LRU cache --

/// Default capacity for the delivery ID dedup cache.
pub const DELIVERY_CACHE_CAPACITY: usize = 10_000;

/// Create a new delivery dedup LRU cache.
pub fn new_delivery_cache() -> Arc<std::sync::Mutex<lru::LruCache<String, ()>>> {
    Arc::new(std::sync::Mutex::new(lru::LruCache::new(
        NonZeroUsize::new(DELIVERY_CACHE_CAPACITY).expect("non-zero"),
    )))
}

// -- Forwarding result type --

/// Outcome of a single forwarding attempt to an agent container.
/// Used by the retry wrapper to decide whether to retry, abandon, or succeed.
#[derive(Debug)]
pub(crate) enum ForwardResult {
    /// 200 or 202 — agent accepted the event.
    Success,
    /// 429 or 5xx, or a request timeout — transient, worth retrying.
    Retryable {
        /// Human-readable description for logging (e.g. "HTTP 429" or "request timeout").
        reason: String,
    },
    /// 4xx (other than 429), connection error (agent offline), or unresolvable route —
    /// retrying will not help.
    Permanent {
        /// Human-readable description for logging.
        reason: String,
    },
}

impl ForwardResult {
    /// Returns `true` if the result indicates a retryable failure.
    #[cfg(test)]
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }

    /// Returns `true` if the forwarding succeeded.
    #[cfg(test)]
    fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns the reason string for non-success results.
    #[cfg(test)]
    fn reason(&self) -> Option<&str> {
        match self {
            Self::Success => None,
            Self::Retryable { reason } | Self::Permanent { reason } => Some(reason),
        }
    }
}

// -- Retry schedule --

/// Delays between retry attempts for GitHub webhook delivery.
/// Total worst-case wall time: 2 + 5 + 15 + 60 + 300 = 382 seconds (+ request timeouts).
///
/// Each delay gets jitter applied (see [`apply_jitter`]) to prevent synchronized
/// retry bursts when many events hit the same 429/5xx response simultaneously.
pub(crate) const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(300),
];

/// Apply ±25% random jitter to a retry delay to prevent thundering-herd effects
/// when many in-flight retries wake up at the same instant.
fn apply_jitter(delay: Duration) -> Duration {
    let base_ms = delay.as_millis() as i64;
    // Jitter in ±25% of the base delay. Uses inclusive range so both bounds can hit.
    let jitter_range = base_ms / 4;
    if jitter_range == 0 {
        return delay;
    }
    let offset: i64 = rand::rng().random_range(-jitter_range..=jitter_range);
    let jittered = (base_ms + offset).max(0) as u64;
    Duration::from_millis(jittered)
}

// -- Webhook handler --

/// POST /webhook/github — receive GitHub App webhook events.
///
/// Validates the `X-Hub-Signature-256` header using HMAC-SHA256.
/// Returns 200 immediately after spawning an async forwarding task.
/// When `MIKA_GITHUB_WEBHOOK_SECRET` is not configured, returns 404.
///
/// The raw body is consumed as `Bytes` for HMAC validation before JSON parsing.
/// This endpoint is not documented in OpenAPI because the request body is raw
/// bytes (not JSON-schema-describable) and authentication is via HMAC signature.
pub(crate) async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. Check if GitHub webhook is configured
    let secret = match state.github_webhook_secret {
        Some(ref s) => s,
        None => return StatusCode::NOT_FOUND,
    };

    // 2. Validate X-Hub-Signature-256
    let sig = match headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            warn!("GitHub webhook missing X-Hub-Signature-256 header");
            return StatusCode::UNAUTHORIZED;
        }
    };
    if !validate_signature(secret.expose_secret().as_bytes(), &body, sig) {
        warn!("GitHub webhook signature validation failed");
        return StatusCode::UNAUTHORIZED;
    }

    // 3. Parse X-GitHub-Event header
    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    // 4. Parse X-GitHub-Delivery for idempotency and tracing
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let _span = tracing::info_span!(
        "github_webhook",
        delivery_id = %delivery_id,
        event_type = %event_type,
    )
    .entered();

    // 5. Pre-routing trace — fires for EVERY valid webhook before dedup/routing/filtering.
    // Diagnostic chain:
    //   - Never arrived:        no debug! at all for that delivery_id
    //   - Arrived but deduped:  this debug! + dedup debug!
    //   - Arrived, not routed:  this debug! + unroutable warn!
    //   - Arrived and delivered: this debug! + routing info!
    debug!(
        event_type,
        delivery_id = %delivery_id,
        "GitHub webhook received (pre-dedup, pre-routing)"
    );

    // 6. Handle ping (no routing needed)
    if event_type == "ping" {
        info!("GitHub webhook ping received");
        return StatusCode::OK;
    }

    // 7. Idempotency via X-GitHub-Delivery LRU cache
    if !delivery_id.is_empty() {
        match state.github_delivery_cache.lock() {
            Ok(mut cache) => {
                if cache.put(delivery_id.clone(), ()).is_some() {
                    debug!(delivery_id = %delivery_id, "GitHub webhook duplicate delivery, skipping");
                    return StatusCode::OK;
                }
            }
            Err(_) => {
                warn!("GitHub delivery cache lock poisoned, skipping dedup (fail-open)");
            }
        }
    }

    // 8. Parse body
    let event: GitHubWebhookEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "GitHub webhook body parse failed");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Self-event filter: with per-agent GitHub App identities (#422), each agent has
    // a distinct bot login. Loop prevention is guaranteed by the routing table (disjoint
    // event types per agent), not by identity filtering. If loop risks emerge (e.g.,
    // mika-qa's own check_suite events), filter by sender.login != agent's bot login.

    // 9. Route to agent
    let check_conclusion = event
        .check_suite
        .as_ref()
        .and_then(|cs| cs.conclusion.as_deref());
    let target_agent = match route_event(event_type, event.action.as_deref(), check_conclusion) {
        Some(agent) => agent,
        None => {
            warn!(
                event_type,
                action = ?event.action,
                delivery_id = %delivery_id,
                "GitHub webhook event not routable, dropping"
            );
            return StatusCode::OK;
        }
    };

    info!(
        event_type,
        action = ?event.action,
        target_agent,
        delivery_id = %delivery_id,
        "GitHub webhook routing event to agent"
    );

    // 10. Semaphore for backpressure
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            warn!("GitHub webhook at capacity, shedding load");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    };

    // 11. Format message text
    let text = format_event_text(event_type, &event);
    let request_id = if delivery_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        delivery_id.clone()
    };

    // 12. Async dispatch with retry — return 200 to GitHub immediately.
    // Retry budget: initial attempt + up to 5 retries with backoff [2s, 5s, 15s, 60s, 300s].
    // Events that exhaust retries are persisted in the DLQ (#590).
    let forwarding_state = state.clone();
    let target = target_agent.to_string();
    let repo_name = event.repository.as_ref().and_then(|r| r.full_name.clone());
    let semaphore = state.webhook_semaphore.clone();
    let event_type_owned = event_type.to_string();
    tokio::spawn(async move {
        deliver_with_retry(
            &forwarding_state,
            &target,
            &text,
            &request_id,
            repo_name.as_deref(),
            &event_type_owned,
            permit,
            &semaphore,
        )
        .await;
    });

    StatusCode::OK
}

/// Resolve the container URL and agent mapping for a GitHub webhook event by
/// looking up the repository's full_name in the `github_repos` table. Falls back
/// to `agent_base_url` for backward compatibility with single-tenant deployments.
pub(crate) async fn resolve_github_container_url(
    state: &AppState,
    repo_full_name: Option<&str>,
) -> Option<ResolvedRoute> {
    // Multi-tenant: look up repo → customer mapping
    if let Some(repo_name) = repo_full_name {
        match sqlx::query_as::<_, (uuid::Uuid, serde_json::Value)>(
            "SELECT customer_id, agent_mapping FROM github_repos WHERE repo_full_name = $1",
        )
        .bind(repo_name)
        .fetch_optional(&state.pool)
        .await
        {
            Ok(Some((customer_id, agent_mapping))) => {
                let container_url = crate::routes::container_url_str(
                    &customer_id.to_string(),
                    state.agent_base_url.as_deref(),
                    &state.agents_namespace,
                );
                let has_mapping = agent_mapping.as_object().is_some_and(|m| !m.is_empty());
                info!(
                    repo = repo_name,
                    customer_id = %customer_id,
                    agent_mapping_active = has_mapping,
                    "resolved GitHub repo to customer"
                );
                return Some(ResolvedRoute {
                    container_url,
                    agent_mapping,
                });
            }
            Ok(None) => {
                // debug when fallback will handle it (single-tenant), warn when event will be dropped
                if state.agent_base_url.is_some() {
                    debug!(
                        repo = repo_name,
                        "GitHub repo not in github_repos table, falling back to agent_base_url"
                    );
                } else {
                    warn!(
                        repo = repo_name,
                        "GitHub repo not registered and no fallback configured"
                    );
                }
            }
            Err(e) => {
                warn!(repo = repo_name, error = %e, "failed to query github_repos table");
            }
        }
    }

    // Fallback: single-tenant mode via agent_base_url
    if let Some(ref base) = state.agent_base_url {
        Some(ResolvedRoute {
            container_url: base.clone(),
            agent_mapping: serde_json::Value::Object(serde_json::Map::new()),
        })
    } else {
        warn!(
            "GitHub webhook: no repo mapping found and no MIKA_AGENT_BASE_URL configured, dropping event"
        );
        None
    }
}

/// Forward a GitHub event to a pre-resolved agent container route.
///
/// Used by [`deliver_with_retry`] so the route (Postgres lookup + agent mapping)
/// is resolved exactly once, regardless of how many retries occur.
pub(crate) async fn forward_to_resolved_route(
    state: &AppState,
    route: &ResolvedRoute,
    default_agent: &str,
    text: &str,
    request_id: &str,
    repo_full_name: Option<&str>,
) -> ForwardResult {
    let target_agent = apply_agent_mapping(&route.agent_mapping, default_agent);
    if target_agent != default_agent {
        info!(
            default_agent,
            mapped_agent = %target_agent,
            repo = ?repo_full_name,
            "applied agent_mapping override"
        );
    }

    let url = &route.container_url;
    let payload = serde_json::json!({
        "text": text,
        "channel": "github",
        "request_id": request_id,
        "agent": target_agent,
    });

    let result = state
        .http_client
        .post(format!("{url}/message"))
        .bearer_auth(state.internal_token.expose_secret())
        .json(&payload)
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    match result {
        // `is_success()` returns true for 2xx (including 202 Accepted).
        Ok(resp) if resp.status().is_success() => ForwardResult::Success,
        Ok(resp) => {
            let status = resp.status().as_u16();
            if status == 429 || (500..=599).contains(&status) {
                ForwardResult::Retryable {
                    reason: format!("HTTP {status}"),
                }
            } else {
                ForwardResult::Permanent {
                    reason: format!("HTTP {status}"),
                }
            }
        }
        Err(e) => {
            if e.is_connect() {
                // Connection refused / DNS failure — agent is offline, retrying won't help.
                ForwardResult::Permanent {
                    reason: format!("connection error: {e}"),
                }
            } else {
                // Timeout or other transient network error — worth retrying.
                ForwardResult::Retryable {
                    reason: format!("network error: {e}"),
                }
            }
        }
    }
}

/// Retry wrapper for GitHub webhook delivery.
///
/// Calls [`forward_to_resolved_route`] once using the caller's semaphore permit.
/// On a retryable failure (429, 5xx, or request timeout), releases the permit,
/// sleeps for the next delay in [`RETRY_DELAYS`] (with jitter), re-acquires a
/// permit, and retries.
///
/// **Route caching:** The container URL and agent mapping are resolved once at
/// the start (single Postgres query), then reused for every retry. This avoids
/// hitting Postgres up to 6 times per event during sustained agent failure.
///
/// **Semaphore lifecycle:** The permit is released during sleep to prevent
/// cross-channel starvation (the 30-permit semaphore is shared with Telegram).
/// If the semaphore is full when re-acquiring, the retry is abandoned with a
/// dedicated ERROR log — the event is dropped because the system is overloaded,
/// not because retries were exhausted. These two failure modes are logged
/// separately so operators can distinguish them.
///
/// **Jitter:** Each retry delay gets ±25% random jitter (see [`apply_jitter`])
/// so that many in-flight retries don't synchronize on the same wakeup instant
/// and recreate the original burst.
///
/// **Dedup LRU interaction:** The `X-GitHub-Delivery` ID is inserted into the
/// in-memory LRU cache (10k entries, size-based eviction, no TTL) *before* this
/// function is called. Under extreme webhook volume (>10k deliveries during a
/// single 300s retry sleep), the LRU may evict the entry. If GitHub redelivers
/// the same event, the gateway would treat it as new. Agent-side idempotency
/// (task unique index) prevents duplicate processing. A TTL on the LRU
/// cache would close this gap but is deferred (see #590 for DLQ).
#[allow(clippy::too_many_arguments)]
async fn deliver_with_retry(
    state: &AppState,
    target_agent: &str,
    text: &str,
    request_id: &str,
    repo_full_name: Option<&str>,
    event_type: &str,
    initial_permit: tokio::sync::OwnedSemaphorePermit,
    semaphore: &Arc<tokio::sync::Semaphore>,
) {
    deliver_with_retry_inner(
        state,
        target_agent,
        text,
        request_id,
        repo_full_name,
        event_type,
        initial_permit,
        semaphore,
        &RETRY_DELAYS,
    )
    .await
}

/// Inner retry loop with the delay schedule injected. Production callers go
/// through [`deliver_with_retry`] which uses [`RETRY_DELAYS`]. Tests pass a
/// custom schedule so timing assertions don't race the production 2s sleep.
#[allow(clippy::too_many_arguments)]
async fn deliver_with_retry_inner(
    state: &AppState,
    target_agent: &str,
    text: &str,
    request_id: &str,
    repo_full_name: Option<&str>,
    event_type: &str,
    initial_permit: tokio::sync::OwnedSemaphorePermit,
    semaphore: &Arc<tokio::sync::Semaphore>,
    retry_delays: &[Duration],
) {
    // Resolve the route once. Cached across all retry attempts.
    let route = match resolve_github_container_url(state, repo_full_name).await {
        Some(r) => r,
        None => {
            error!(
                target_agent,
                request_id,
                "GitHub event delivery failed — no route resolved (repo not registered and no fallback), event dropped"
            );
            drop(initial_permit);
            return;
        }
    };

    // Run the full attempt sequence: initial + up to RETRY_DELAYS.len() retries.
    // `attempts_made` tracks how many HTTP calls were actually issued, which is
    // used for accurate logging on any terminal outcome.
    let mut attempts_made: u32 = 0;
    let mut last_reason: String = String::new();
    let mut current_permit = Some(initial_permit);

    for (retry_idx, delay) in std::iter::once(None)
        .chain(retry_delays.iter().map(Some))
        .enumerate()
    {
        // The first iteration (retry_idx=0) has `delay = None` and uses the caller's permit.
        // Subsequent iterations sleep then re-acquire a permit.
        if let Some(delay) = delay {
            tokio::time::sleep(apply_jitter(*delay)).await;

            match semaphore.clone().try_acquire_owned() {
                Ok(p) => {
                    current_permit = Some(p);
                }
                Err(_) => {
                    error!(
                        target_agent,
                        request_id,
                        attempts_made,
                        last_reason = %last_reason,
                        "GitHub event delivery abandoned — gateway semaphore at capacity during retry, persisting to DLQ"
                    );
                    crate::dlq::insert_delivery(
                        &state.pool,
                        crate::dlq::NewDelivery {
                            delivery_id: request_id,
                            event_type,
                            target_agent,
                            repo_full_name,
                            payload: text,
                            request_id,
                            attempts: attempts_made as i32,
                            last_error: &last_reason,
                        },
                    )
                    .await;
                    return;
                }
            }
        }

        let attempt = retry_idx as u32 + 1;
        let result = forward_to_resolved_route(
            state,
            &route,
            target_agent,
            text,
            request_id,
            repo_full_name,
        )
        .await;
        attempts_made = attempt;

        // Release the permit immediately after the attempt, before the next sleep.
        current_permit.take();

        match result {
            ForwardResult::Success => {
                if attempt == 1 {
                    info!(
                        target_agent,
                        request_id, "GitHub event forwarded to agent container"
                    );
                } else {
                    info!(
                        target_agent,
                        request_id,
                        attempt,
                        "GitHub event forwarded to agent container (retry succeeded)"
                    );
                }
                return;
            }
            ForwardResult::Permanent { reason } => {
                error!(
                    target_agent,
                    request_id,
                    attempt,
                    reason = %reason,
                    "GitHub event delivery failed (permanent, no further retries)"
                );
                return;
            }
            ForwardResult::Retryable { reason } => {
                let remaining = retry_delays.len().saturating_sub(retry_idx);
                warn!(
                    target_agent,
                    request_id,
                    attempt,
                    remaining_retries = remaining,
                    reason = %reason,
                    "GitHub event delivery failed, will retry"
                );
                last_reason = reason;
            }
        }
    }

    // All retries exhausted — every attempt returned Retryable. Persist to DLQ.
    error!(
        target_agent,
        request_id,
        total_attempts = attempts_made,
        last_reason = %last_reason,
        "GitHub event delivery failed — retry budget exhausted, persisting to DLQ"
    );
    crate::dlq::insert_delivery(
        &state.pool,
        crate::dlq::NewDelivery {
            delivery_id: request_id,
            event_type,
            target_agent,
            repo_full_name,
            payload: text,
            request_id,
            attempts: attempts_made as i32,
            last_error: &last_reason,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- HMAC-SHA256 validation tests --

    fn compute_signature(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        format!("sha256={}", hex::encode(result))
    }

    #[test]
    fn test_validate_signature_valid() {
        let secret = b"test-secret";
        let body = b"hello world";
        let sig = compute_signature(secret, body);
        assert!(validate_signature(secret, body, &sig));
    }

    #[test]
    fn test_validate_signature_invalid_body() {
        let secret = b"test-secret";
        let body = b"hello world";
        let sig = compute_signature(secret, body);
        assert!(!validate_signature(secret, b"wrong body", &sig));
    }

    #[test]
    fn test_validate_signature_wrong_secret() {
        let body = b"hello world";
        let sig = compute_signature(b"secret-a", body);
        assert!(!validate_signature(b"secret-b", body, &sig));
    }

    #[test]
    fn test_validate_signature_missing_prefix() {
        let secret = b"test-secret";
        let body = b"hello world";
        // No "sha256=" prefix
        assert!(!validate_signature(secret, body, "abc123"));
    }

    #[test]
    fn test_validate_signature_empty() {
        let secret = b"test-secret";
        let body = b"hello world";
        assert!(!validate_signature(secret, body, ""));
    }

    #[test]
    fn test_validate_signature_invalid_hex() {
        let secret = b"test-secret";
        let body = b"hello world";
        assert!(!validate_signature(secret, body, "sha256=notvalidhex!!!"));
    }

    // -- Event routing tests --

    #[test]
    fn test_route_event_issues_opened_ignored() {
        assert_eq!(route_event("issues", Some("opened"), None), None);
    }

    #[test]
    fn test_route_event_issues_assigned() {
        assert_eq!(
            route_event("issues", Some("assigned"), None),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_issues_closed() {
        assert_eq!(route_event("issues", Some("closed"), None), None);
    }

    #[test]
    fn test_route_event_issue_comment_created() {
        assert_eq!(
            route_event("issue_comment", Some("created"), None),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_issue_comment_deleted() {
        assert_eq!(route_event("issue_comment", Some("deleted"), None), None);
    }

    #[test]
    fn test_route_event_pr_opened() {
        assert_eq!(
            route_event("pull_request", Some("opened"), None),
            Some("mika-qa")
        );
    }

    #[test]
    fn test_route_event_pr_synchronize() {
        assert_eq!(
            route_event("pull_request", Some("synchronize"), None),
            Some("mika-qa")
        );
    }

    #[test]
    fn test_route_event_pr_closed() {
        assert_eq!(
            route_event("pull_request", Some("closed"), None),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_pr_review_submitted() {
        assert_eq!(
            route_event("pull_request_review", Some("submitted"), None),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_check_suite_failure() {
        assert_eq!(
            route_event("check_suite", Some("completed"), Some("failure")),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_check_suite_timed_out() {
        assert_eq!(
            route_event("check_suite", Some("completed"), Some("timed_out")),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_check_suite_success() {
        assert_eq!(
            route_event("check_suite", Some("completed"), Some("success")),
            Some("mika-dev")
        );
    }

    #[test]
    fn test_route_event_unknown() {
        assert_eq!(route_event("unknown_event", Some("action"), None), None);
    }

    #[test]
    fn test_route_event_ping() {
        // Ping is handled before routing in the handler, but route_event returns None
        assert_eq!(route_event("ping", None, None), None);
    }

    #[test]
    fn test_route_event_no_action() {
        assert_eq!(route_event("issues", None, None), None);
    }

    // -- Message text formatting tests --

    #[test]
    fn test_format_event_text_issue_opened() {
        let event = GitHubWebhookEvent {
            action: Some("opened".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: Some(GitHubIssue {
                number: Some(42),
                title: Some("Bug report".to_string()),
                html_url: Some("https://github.com/org/repo/issues/42".to_string()),
                body: Some("Something is broken".to_string()),
                assignee: None,
            }),
            pull_request: None,
            comment: None,
            review: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
        };
        let text = format_event_text("issues", &event);
        assert!(text.contains("[GitHub] Issue opened"));
        assert!(text.contains("org/repo#42"));
        assert!(text.contains("Bug report"));
        assert!(text.contains("Something is broken"));
    }

    #[test]
    fn test_format_event_text_pr_opened() {
        let event = GitHubWebhookEvent {
            action: Some("opened".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: None,
            pull_request: Some(GitHubPullRequest {
                number: Some(10),
                title: Some("Fix bug".to_string()),
                html_url: Some("https://github.com/org/repo/pull/10".to_string()),
                body: None,
                head: Some(GitHubRef {
                    ref_name: Some("fix/bug".to_string()),
                }),
                merged: None,
            }),
            comment: None,
            review: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
        };
        let text = format_event_text("pull_request", &event);
        assert!(text.contains("[GitHub] PR opened"));
        assert!(text.contains("org/repo#10"));
        assert!(text.contains("fix/bug"));
    }

    #[test]
    fn test_format_event_text_check_suite() {
        let event = GitHubWebhookEvent {
            action: Some("completed".to_string()),
            sender: None,
            installation: None,
            check_suite: Some(CheckSuite {
                conclusion: Some("failure".to_string()),
                head_branch: Some("main".to_string()),
            }),
            issue: None,
            pull_request: None,
            comment: None,
            review: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
        };
        let text = format_event_text("check_suite", &event);
        assert!(text.contains("[GitHub] Check suite failure"));
        assert!(text.contains("org/repo"));
        assert!(text.contains("main"));
    }

    // -- Delivery cache tests --

    #[test]
    fn test_delivery_cache_dedup() {
        let cache = new_delivery_cache();
        let mut c = cache.lock().unwrap();
        // First insert returns None (not a duplicate)
        assert!(c.put("delivery-1".to_string(), ()).is_none());
        // Second insert returns Some (duplicate)
        assert!(c.put("delivery-1".to_string(), ()).is_some());
    }

    #[test]
    fn test_delivery_cache_capacity() {
        let small_cache = Arc::new(std::sync::Mutex::new(lru::LruCache::new(
            NonZeroUsize::new(2).unwrap(),
        )));
        let mut c = small_cache.lock().unwrap();
        c.put("a".to_string(), ());
        c.put("b".to_string(), ());
        c.put("c".to_string(), ()); // evicts "a" (LRU)
        // "a" was evicted — inserting again returns None (not a duplicate)
        assert!(c.put("a".to_string(), ()).is_none());
        // "c" was recently used — still present
        assert!(c.put("c".to_string(), ()).is_some());
    }

    // -- Integration tests (full handler via tower::ServiceExt::oneshot) --

    use axum::body::Body;
    use axum::routing::post;
    use tower::ServiceExt;

    /// Build a minimal test router with only the GitHub webhook route.
    /// Does NOT require Postgres or Telegram client.
    fn test_router(webhook_secret: Option<&str>) -> axum::Router {
        use crate::routes::AppState;
        use crate::telegram::TelegramClient;
        use secrecy::SecretString;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        let http_client = reqwest::Client::new();
        let telegram =
            TelegramClient::new(http_client.clone(), SecretString::from("fake-bot-token"));

        // PgPool::connect_lazy creates a pool that only connects when first used.
        // Our GitHub webhook handler never touches Postgres, so this is safe.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");

        let state = AppState {
            pool,
            telegram,
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: SecretString::from("b".repeat(64)),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some("http://localhost:9999".to_string()),
            agents_namespace: "mika-agents".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: webhook_secret.map(|s| SecretString::from(s.to_string())),
            github_delivery_cache: new_delivery_cache(),
        };

        axum::Router::new()
            .route("/webhook/github", post(handle_github_webhook))
            .with_state(state)
    }

    fn make_request(
        secret: &str,
        body: &[u8],
        event_type: &str,
        delivery_id: &str,
    ) -> axum::http::Request<Body> {
        let sig = compute_signature(secret.as_bytes(), body);
        axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/github")
            .header("x-hub-signature-256", sig)
            .header("x-github-event", event_type)
            .header("x-github-delivery", delivery_id)
            .header("content-type", "application/json")
            .body(Body::from(body.to_vec()))
            .unwrap()
    }

    #[tokio::test]
    async fn test_webhook_ping_returns_200() {
        let app = test_router(Some("test-secret"));
        let body = br#"{"zen": "Keep it logically awesome."}"#;
        let req = make_request("test-secret", body, "ping", "ping-uuid-1");

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_webhook_invalid_signature_returns_401() {
        let app = test_router(Some("test-secret"));
        let body = br#"{"action": "opened"}"#;
        // Sign with wrong secret
        let sig = compute_signature(b"wrong-secret", body);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/github")
            .header("x-hub-signature-256", sig)
            .header("x-github-event", "issues")
            .header("x-github-delivery", "uuid-1")
            .body(Body::from(body.to_vec()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_webhook_missing_signature_returns_401() {
        let app = test_router(Some("test-secret"));
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/webhook/github")
            .header("x-github-event", "issues")
            .header("x-github-delivery", "uuid-1")
            .body(Body::from(br#"{"action": "opened"}"#.to_vec()))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_webhook_unconfigured_returns_404() {
        let app = test_router(None); // No webhook secret
        let body = br#"{"action": "opened"}"#;
        let req = make_request("irrelevant", body, "issues", "uuid-1");

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_webhook_duplicate_delivery_returns_200() {
        // We need a shared router to test dedup across two requests.
        // Use the same AppState via Router cloning.
        use crate::routes::AppState;
        use crate::telegram::TelegramClient;
        use secrecy::SecretString;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        let http_client = reqwest::Client::new();
        let telegram =
            TelegramClient::new(http_client.clone(), SecretString::from("fake-bot-token"));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");
        let delivery_cache = new_delivery_cache();

        let state = AppState {
            pool,
            telegram,
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: SecretString::from("b".repeat(64)),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some("http://localhost:9999".to_string()),
            agents_namespace: "mika-agents".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: Some(SecretString::from("test-secret")),
            github_delivery_cache: delivery_cache.clone(),
        };

        let app = axum::Router::new()
            .route("/webhook/github", post(handle_github_webhook))
            .with_state(state);

        // Issue event body (routable)
        let body = br#"{"action": "opened", "issue": {"number": 1, "title": "Test"}, "repository": {"full_name": "org/repo"}}"#;

        // First request — should succeed
        let req1 = make_request("test-secret", body, "issues", "same-delivery-uuid");
        let resp1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        // Second request with same delivery ID — should be deduped (still 200)
        let req2 = make_request("test-secret", body, "issues", "same-delivery-uuid");
        let resp2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        // Verify the cache has the delivery ID
        let cache = delivery_cache.lock().unwrap();
        assert!(cache.contains(&"same-delivery-uuid".to_string()));
    }

    #[tokio::test]
    async fn test_webhook_unroutable_event_returns_200() {
        let app = test_router(Some("test-secret"));
        let body = br#"{"action": "closed"}"#;
        let req = make_request("test-secret", body, "issues", "unroutable-uuid");

        let resp = app.oneshot(req).await.unwrap();
        // Unroutable events return 200 (silently dropped)
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_webhook_malformed_json_returns_400() {
        let app = test_router(Some("test-secret"));
        let body = b"not json at all {{{";
        let req = make_request("test-secret", body, "issues", "bad-json-uuid");

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_webhook_routable_event_returns_200() {
        let app = test_router(Some("test-secret"));
        let body = br#"{
            "action": "opened",
            "sender": {"login": "octocat", "type": "User"},
            "issue": {"number": 42, "title": "Feature request"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "issues", "routable-uuid");

        let resp = app.oneshot(req).await.unwrap();
        // Routable event accepted — forwarding happens async
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Regression test for #403: GitHub sends minimal `installation` objects
    /// without `app_id` on some event types. After removing the dead `app_id`
    /// field from `GitHubInstallation`, these payloads must parse successfully.
    #[tokio::test]
    async fn test_webhook_minimal_installation_parses() {
        let app = test_router(Some("test-secret"));
        let body = br#"{
            "action": "opened",
            "sender": {"login": "octocat", "type": "User"},
            "issue": {"number": 42, "title": "Feature request",
                      "html_url": "https://github.com/org/repo/issues/42",
                      "body": "test body"},
            "repository": {"full_name": "org/repo"},
            "installation": {"id": 12345}
        }"#;
        let req = make_request("test-secret", body, "issues", "minimal-install-uuid");

        let resp = app.oneshot(req).await.unwrap();
        // Should parse and route successfully with minimal installation object
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Verify that payloads still including `app_id` in the installation object
    /// parse correctly after the field was removed from the struct (serde ignores
    /// unknown fields by default).
    #[tokio::test]
    async fn test_webhook_installation_with_extra_fields_parses() {
        let app = test_router(Some("test-secret"));
        let body = br#"{
            "action": "opened",
            "sender": {"login": "octocat", "type": "User"},
            "issue": {"number": 42, "title": "Feature request",
                      "html_url": "https://github.com/org/repo/issues/42",
                      "body": "test body"},
            "repository": {"full_name": "org/repo"},
            "installation": {"id": 12345, "app_id": 67890, "node_id": "MDIzOk"}
        }"#;
        let req = make_request("test-secret", body, "issues", "extra-fields-uuid");

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -- Body truncation tests --

    #[test]
    fn test_truncate_body_short() {
        assert_eq!(truncate_body("hello", 2000), "hello");
    }

    #[test]
    fn test_truncate_body_long() {
        let long = "a".repeat(2500);
        let result = truncate_body(&long, 2000);
        assert!(result.starts_with(&"a".repeat(2000)));
        assert!(result.ends_with("[truncated]"));
    }

    // -- Agent mapping tests --

    #[test]
    fn test_apply_agent_mapping_empty_object() {
        let mapping = serde_json::json!({});
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "mika-dev");
        assert_eq!(apply_agent_mapping(&mapping, "mika-qa"), "mika-qa");
    }

    #[test]
    fn test_apply_agent_mapping_valid_override() {
        let mapping = serde_json::json!({
            "mika-dev": "acme-dev",
            "mika-qa": "acme-qa"
        });
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "acme-dev");
        assert_eq!(apply_agent_mapping(&mapping, "mika-qa"), "acme-qa");
    }

    #[test]
    fn test_apply_agent_mapping_partial_override() {
        let mapping = serde_json::json!({
            "mika-dev": "custom-dev"
        });
        // Override exists for mika-dev
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "custom-dev");
        // No override for mika-qa — use default
        assert_eq!(apply_agent_mapping(&mapping, "mika-qa"), "mika-qa");
    }

    #[test]
    fn test_apply_agent_mapping_empty_string_value() {
        let mapping = serde_json::json!({
            "mika-dev": ""
        });
        // Empty string override is treated as absent — use default
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "mika-dev");
    }

    #[test]
    fn test_apply_agent_mapping_non_string_value() {
        let mapping = serde_json::json!({
            "mika-dev": 42,
            "mika-qa": null
        });
        // Non-string values are ignored — use defaults
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "mika-dev");
        assert_eq!(apply_agent_mapping(&mapping, "mika-qa"), "mika-qa");
    }

    #[test]
    fn test_apply_agent_mapping_null_value() {
        let mapping = serde_json::Value::Null;
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "mika-dev");
    }

    #[test]
    fn test_apply_agent_mapping_rejects_invalid_name() {
        let mapping = serde_json::json!({
            "mika-dev": "has spaces",
            "mika-qa": "-leading-hyphen"
        });
        // Invalid agent names fall back to defaults
        assert_eq!(apply_agent_mapping(&mapping, "mika-dev"), "mika-dev");
        assert_eq!(apply_agent_mapping(&mapping, "mika-qa"), "mika-qa");
    }

    // -- Agent name validation tests --

    #[test]
    fn test_is_valid_agent_name() {
        assert!(is_valid_agent_name("mika-dev"));
        assert!(is_valid_agent_name("acme-qa"));
        assert!(is_valid_agent_name("a"));
        assert!(is_valid_agent_name("agent123"));
    }

    #[test]
    fn test_is_valid_agent_name_rejects_invalid() {
        assert!(!is_valid_agent_name(""));
        assert!(!is_valid_agent_name("-leading"));
        assert!(!is_valid_agent_name("trailing-"));
        assert!(!is_valid_agent_name("double--hyphen"));
        assert!(!is_valid_agent_name("HAS UPPER"));
        assert!(!is_valid_agent_name("has spaces"));
        assert!(!is_valid_agent_name("special@chars"));
        assert!(!is_valid_agent_name(&"a".repeat(64))); // too long
    }

    // -- ForwardResult classification tests --

    #[test]
    fn test_forward_result_success_classification() {
        let r = ForwardResult::Success;
        assert!(r.is_success());
        assert!(!r.is_retryable());
        assert!(r.reason().is_none());
    }

    #[test]
    fn test_forward_result_retryable_429() {
        let r = ForwardResult::Retryable {
            reason: "HTTP 429".to_string(),
        };
        assert!(r.is_retryable());
        assert!(!r.is_success());
        assert_eq!(r.reason(), Some("HTTP 429"));
    }

    #[test]
    fn test_forward_result_retryable_5xx() {
        for status in [500, 502, 503, 504] {
            let r = ForwardResult::Retryable {
                reason: format!("HTTP {status}"),
            };
            assert!(r.is_retryable(), "HTTP {status} should be retryable");
        }
    }

    #[test]
    fn test_forward_result_permanent_4xx() {
        for status in [400, 404, 405, 422] {
            let r = ForwardResult::Permanent {
                reason: format!("HTTP {status}"),
            };
            assert!(!r.is_retryable(), "HTTP {status} should NOT be retryable");
            assert!(!r.is_success());
        }
    }

    #[test]
    fn test_forward_result_permanent_connection_error() {
        let r = ForwardResult::Permanent {
            reason: "connection error: connection refused".to_string(),
        };
        assert!(!r.is_retryable());
        assert_eq!(r.reason(), Some("connection error: connection refused"));
    }

    #[test]
    fn test_forward_result_retryable_timeout() {
        let r = ForwardResult::Retryable {
            reason: "network error: request timeout".to_string(),
        };
        assert!(r.is_retryable());
    }

    // -- Retry schedule tests --

    #[test]
    fn test_retry_delays_schedule() {
        assert_eq!(RETRY_DELAYS.len(), 5);
        assert_eq!(RETRY_DELAYS[0], Duration::from_secs(2));
        assert_eq!(RETRY_DELAYS[1], Duration::from_secs(5));
        assert_eq!(RETRY_DELAYS[2], Duration::from_secs(15));
        assert_eq!(RETRY_DELAYS[3], Duration::from_secs(60));
        assert_eq!(RETRY_DELAYS[4], Duration::from_secs(300));
    }

    // -- deliver_with_retry tests --
    //
    // These tests use a mock HTTP server (wiremock or similar) to simulate
    // agent container responses. Since we don't have wiremock as a dependency,
    // we test the retry logic using a real Axum server bound to an ephemeral port.

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Spin up a minimal Axum server that returns a sequence of status codes.
    /// Returns `(base_url, call_count)` so tests can assert how many HTTP calls
    /// were actually made — essential for verifying the no-retry contract.
    ///
    /// When `responses` is exhausted, falls back to `StatusCode::OK`. Tests that
    /// rely on "no retry" must therefore assert `call_count == expected` rather
    /// than relying on default behavior.
    async fn mock_agent_server(responses: Vec<StatusCode>) -> (String, Arc<AtomicUsize>) {
        let call_count = Arc::new(AtomicUsize::new(0));
        let responses = Arc::new(responses);

        let cc = call_count.clone();
        let resps = responses.clone();
        let app = axum::Router::new().route(
            "/message",
            post(move || {
                let cc = cc.clone();
                let resps = resps.clone();
                async move {
                    let idx = cc.fetch_add(1, Ordering::SeqCst);
                    resps.get(idx).copied().unwrap_or(StatusCode::OK)
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (format!("http://127.0.0.1:{}", addr.port()), call_count)
    }

    /// Build an AppState pointing to a mock agent server.
    fn test_state_with_base_url(base_url: &str) -> AppState {
        use crate::telegram::TelegramClient;
        use secrecy::SecretString;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        let http_client = reqwest::Client::new();
        let telegram =
            TelegramClient::new(http_client.clone(), SecretString::from("fake-bot-token"));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");

        AppState {
            pool,
            telegram,
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: SecretString::from("b".repeat(64)),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some(base_url.to_string()),
            agents_namespace: "test".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: Some(SecretString::from("test-secret")),
            github_delivery_cache: new_delivery_cache(),
        }
    }

    #[tokio::test]
    async fn test_deliver_success_on_first_attempt() {
        let (base_url, call_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry(
            &state,
            "mika-dev",
            "test event",
            "delivery-1",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "success on first attempt should make exactly 1 HTTP call"
        );
        // Permit was consumed (dropped inside deliver_with_retry on success)
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_deliver_retry_on_429_then_success() {
        // First call returns 429, second returns 200
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::TOO_MANY_REQUESTS, StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        // Accepts the real 2s wait for the first retry — RETRY_DELAYS is a
        // compile-time const so we can't easily inject shorter delays.
        deliver_with_retry(
            &state,
            "mika-dev",
            "test event",
            "delivery-429",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            2,
            "429-then-success should make exactly 2 HTTP calls"
        );
        // Should succeed after one retry — all permits returned
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_deliver_retry_on_503_then_success() {
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry(
            &state,
            "mika-dev",
            "test event",
            "delivery-503",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
        )
        .await;

        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_deliver_no_retry_on_400() {
        // 400 is permanent — should not retry. Queue OK as fallback so a
        // regression that retries would mask itself with success, making the
        // call_count assertion the sole guarantee of no-retry.
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::BAD_REQUEST, StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry(
            &state,
            "mika-dev",
            "test event",
            "delivery-400",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "400 is permanent — must make exactly 1 HTTP call, no retry"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_deliver_no_retry_on_404() {
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::NOT_FOUND, StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry(
            &state,
            "mika-dev",
            "test event",
            "delivery-404",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "404 is permanent — must make exactly 1 HTTP call, no retry"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    #[ignore = "takes ~6.5 minutes of wall time (RETRY_DELAYS total 382s); run with --ignored"]
    async fn test_deliver_retry_budget_exhausted_after_six_attempts() {
        // All 6 attempts (initial + 5 retries) return 429 — retry budget exhausted,
        // ERROR logged, event dropped. This is the plan's Unit 2 required scenario.
        //
        // Wall-clock cost: 2 + 5 + 15 + 60 + 300 = 382s (±25% jitter per step).
        // Marked #[ignore] so normal `cargo test` skips it; run with:
        //   cargo test -p mika-gateway -- --ignored test_deliver_retry_budget_exhausted
        //
        // Queue 6 explicit 429s so every attempt is Retryable. Uses tokio::time::timeout
        // to cap the test wall-clock in case of test infra issues.
        let (base_url, call_count) = mock_agent_server(vec![
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
        ])
        .await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        // Total wall time: 2 + 5 + 15 + 60 + 300 = 382s base. Cap at 500s via timeout.
        // NOTE: This test takes ~6.5 minutes of real time because RETRY_DELAYS is
        // a compile-time const. Marked #[ignore] by default — run with `cargo test
        // -- --ignored test_deliver_retry_budget_exhausted` for coverage.
        let result = tokio::time::timeout(
            Duration::from_secs(500),
            deliver_with_retry(
                &state,
                "mika-dev",
                "test event",
                "delivery-exhausted",
                Some("org/repo"),
                "issues",
                permit,
                &semaphore,
            ),
        )
        .await;

        assert!(
            result.is_ok(),
            "deliver_with_retry did not return within 500s"
        );
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            6,
            "all retries should be exhausted — expected 6 HTTP calls"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    /// Polls `semaphore.available_permits()` until it reaches `target`, up to `budget`.
    /// Returns the elapsed wait. Used in conjunction with the test-only long
    /// retry-delay schedule below — once the first HTTP attempt completes
    /// (whenever that is on slow CI), the retry sleep is long enough that the
    /// poll has a wide window to observe the released permit.
    async fn wait_for_permits(
        semaphore: &Arc<tokio::sync::Semaphore>,
        target: usize,
        budget: Duration,
    ) -> Duration {
        let start = std::time::Instant::now();
        loop {
            if semaphore.available_permits() >= target {
                return start.elapsed();
            }
            if start.elapsed() >= budget {
                return start.elapsed();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Test-only retry-delay schedules. Production uses 2s for the first delay;
    /// these tests need the released-permit window to be wide enough for the
    /// polling assertion to land regardless of how slow the CI runner's HTTP
    /// roundtrip is. Two schedules:
    ///
    /// - `OBSERVE_ONLY`: very long first delay (60s). The test observes the
    ///   released permit, then aborts the spawned task. Used by the
    ///   "permit released during retry sleep" test where we don't need the
    ///   retry to fire — only to observe the released state.
    /// - `OBSERVE_AND_ABANDON`: 5s first delay. The test observes release,
    ///   steals the permit, then waits for the retry to attempt re-acquire
    ///   and abandon. 5s leaves 4s polling headroom on slow CI plus enough
    ///   time for the abandon path to fire within the test timeout.
    const TEST_DELAYS_OBSERVE_ONLY: [Duration; 1] = [Duration::from_secs(60)];
    const TEST_DELAYS_OBSERVE_AND_ABANDON: [Duration; 1] = [Duration::from_secs(5)];

    #[tokio::test]
    async fn test_deliver_semaphore_released_during_retry_sleep() {
        // 429 on first attempt — during the retry sleep, the permit must be released.
        // Uses a 60s test-only retry delay (vs production's 2s) so the polling
        // assertion isn't racing the retry sleep window.
        let (base_url, _call_count) =
            mock_agent_server(vec![StatusCode::TOO_MANY_REQUESTS, StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1)); // Only 1 permit!
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        let sem_clone = semaphore.clone();
        let handle = tokio::spawn(async move {
            // Pass repo_full_name=None to skip the resolve_github_container_url
            // SQL lookup. test_state_with_base_url uses a lazy fake Postgres
            // pool whose first connection attempt blocks for sqlx's default
            // connect_timeout (~30s) on CI hosts where localhost:5432 doesn't
            // RST-fast — that's what was eating our timing budget.
            deliver_with_retry_inner(
                &state,
                "mika-dev",
                "test event",
                "delivery-sem",
                None,
                "issues",
                permit,
                &sem_clone,
                &TEST_DELAYS_OBSERVE_ONLY,
            )
            .await;
        });

        // Poll for permit release with a 10s budget — generous on any CI runner.
        // The retry sleep is 60s so we have 60s once first attempt completes.
        let waited = wait_for_permits(&semaphore, 1, Duration::from_secs(10)).await;
        assert_eq!(
            semaphore.available_permits(),
            1,
            "permit should be released during retry sleep (waited {waited:?})"
        );

        // Don't wait for the spawned task to finish (it's now in a 60s sleep).
        // Aborting is safe: deliver_with_retry has no shared state to clean up.
        handle.abort();
    }

    #[tokio::test]
    async fn test_deliver_abandoned_when_semaphore_full() {
        // 429 on first attempt, then semaphore is full on retry → abandon path.
        // Uses 60s test-only retry delay so the test has plenty of time to
        // steal the permit between first-attempt-end and retry-reacquire.
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::TOO_MANY_REQUESTS, StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);

        // Semaphore with 1 permit — we'll hold the spare one to block retry
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        let sem_clone = semaphore.clone();
        let state_clone = state.clone();
        let handle = tokio::spawn(async move {
            // Pass repo_full_name=None to skip the resolve_github_container_url
            // SQL lookup (see sibling test for rationale).
            deliver_with_retry_inner(
                &state_clone,
                "mika-dev",
                "test event",
                "delivery-blocked",
                None,
                "issues",
                permit,
                &sem_clone,
                &TEST_DELAYS_OBSERVE_AND_ABANDON,
            )
            .await;
        });

        // Wait for the first attempt to release the permit. Budget 4s — the
        // 5s retry delay leaves us 1s margin to steal the permit before the
        // retry tries to re-acquire it. Generous compared to the prior 1.4s
        // budget that flaked in CI, but bounded by the retry delay.
        wait_for_permits(&semaphore, 1, Duration::from_secs(4)).await;
        assert_eq!(
            semaphore.available_permits(),
            1,
            "first attempt should release the permit before the retry sleep"
        );

        // Grab the permit before the retry can — this blocks the retry.
        // Retry sleep is 60s, so we have a huge window to do this.
        let _blocker = semaphore
            .clone()
            .try_acquire_owned()
            .expect("test must hold the only permit before retry tries to re-acquire");

        // The retry should be abandoned because the semaphore is full.
        // Wrap in timeout to catch the race where _blocker wasn't grabbed in
        // time: if the retry succeeded, deliver_with_retry returns quickly
        // (still within timeout); if _blocker was grabbed, deliver_with_retry
        // abandons and returns immediately.
        tokio::time::timeout(Duration::from_secs(10), handle)
            .await
            .expect("deliver_with_retry should not hang")
            .unwrap();

        // Assert exactly one HTTP call was made. If the retry happened
        // (the abandon path didn't fire), call_count would be 2.
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "semaphore-full abandonment should result in exactly 1 HTTP call"
        );
    }

    #[tokio::test]
    async fn test_deliver_accepts_202() {
        let (base_url, call_count) = mock_agent_server(vec![StatusCode::ACCEPTED]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry(
            &state,
            "mika-dev",
            "test event",
            "delivery-202",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "202 is success — no retry"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    // -- Jitter tests --

    #[test]
    fn test_apply_jitter_within_bounds() {
        let base = Duration::from_secs(10);
        // Run many times to exercise the full jitter range. ±25% of 10s = [7.5s, 12.5s].
        for _ in 0..200 {
            let jittered = apply_jitter(base);
            assert!(
                jittered >= Duration::from_millis(7500) && jittered <= Duration::from_millis(12500),
                "jittered delay {jittered:?} out of [7.5s, 12.5s]"
            );
        }
    }

    #[test]
    fn test_apply_jitter_small_delay() {
        // A 3ms delay has jitter_range = 0 (3/4 truncates to 0), so passes through unchanged.
        let d = Duration::from_millis(3);
        assert_eq!(apply_jitter(d), d);
    }

    #[test]
    fn test_apply_jitter_zero_delay() {
        // A 0ms delay should pass through unchanged (jitter_range = 0).
        let d = Duration::from_millis(0);
        assert_eq!(apply_jitter(d), d);
    }
}
