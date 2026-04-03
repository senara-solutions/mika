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
use secrecy::ExposeSecret;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::{debug, info, warn};

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
        ("pull_request_review", Some("submitted")) => Some("mika-dev"),
        ("check_suite", Some("completed")) => match check_conclusion {
            Some("failure" | "timed_out") => Some("mika-dev"),
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

// -- LRU cache --

/// Default capacity for the delivery ID dedup cache.
pub const DELIVERY_CACHE_CAPACITY: usize = 10_000;

/// Create a new delivery dedup LRU cache.
pub fn new_delivery_cache() -> Arc<std::sync::Mutex<lru::LruCache<String, ()>>> {
    Arc::new(std::sync::Mutex::new(lru::LruCache::new(
        NonZeroUsize::new(DELIVERY_CACHE_CAPACITY).expect("non-zero"),
    )))
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

    // 5. Handle ping (no routing needed)
    if event_type == "ping" {
        info!("GitHub webhook ping received");
        return StatusCode::OK;
    }

    // 6. Idempotency via X-GitHub-Delivery LRU cache
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

    // 7. Parse body
    let event: GitHubWebhookEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "GitHub webhook body parse failed");
            return StatusCode::BAD_REQUEST;
        }
    };

    // Self-event filter intentionally removed: mika-dev and mika-qa share one GitHub
    // App identity but subscribe to disjoint event types. No routing path exists that
    // would deliver an agent's own action back to itself. Loop prevention is guaranteed
    // by the routing table, not by identity filtering.
    // Future: give mika-qa a dedicated App token (Option 3) if per-agent audit trails
    // or permission scopes become necessary.

    // 8. Route to agent
    let check_conclusion = event
        .check_suite
        .as_ref()
        .and_then(|cs| cs.conclusion.as_deref());
    let target_agent = match route_event(event_type, event.action.as_deref(), check_conclusion) {
        Some(agent) => agent,
        None => {
            debug!(
                event_type,
                action = ?event.action,
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

    // 12. Async dispatch — return 200 to GitHub immediately
    let forwarding_state = state.clone();
    let target = target_agent.to_string();
    tokio::spawn(async move {
        let _permit = permit; // held until task completes
        forward_github_event(&forwarding_state, &target, &text, &request_id).await;
    });

    StatusCode::OK
}

/// Forward a GitHub event to the agent container via `POST {container_url}/message`.
///
/// Uses `channel: "github"` and `chat_id: 0` (no reply channel).
/// Single-tenant routing: always uses `agent_base_url` or first customer's container.
async fn forward_github_event(state: &AppState, target_agent: &str, text: &str, request_id: &str) {
    // Single-tenant: route to agent_base_url (required for GitHub webhooks in Phase 2a)
    let url = match &state.agent_base_url {
        Some(base) => base.clone(),
        None => {
            warn!(
                "GitHub webhook forwarding requires MIKA_AGENT_BASE_URL (multi-tenant not yet supported)"
            );
            return;
        }
    };

    let payload = serde_json::json!({
        "text": text,
        "chat_id": 0,
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
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
            info!(
                target_agent,
                request_id, "GitHub event forwarded to agent container"
            );
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            warn!(
                status,
                target_agent, request_id, "agent container returned error for GitHub event"
            );
        }
        Err(e) => {
            let is_connect = e.is_connect();
            warn!(
                error = %e,
                target_agent,
                request_id,
                is_connect,
                "agent container unreachable for GitHub event"
            );
        }
    }
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
    fn test_route_event_issues_opened() {
        assert_eq!(
            route_event("issues", Some("opened"), None),
            Some("mika-dev")
        );
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
        assert_eq!(route_event("pull_request", Some("closed"), None), None);
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
            None
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
}
