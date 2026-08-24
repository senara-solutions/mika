//! GitHub webhook handler for mika-gateway.
//!
//! Receives GitHub App webhook events at `POST /webhook/github`, validates the
//! HMAC-SHA256 signature, routes to the correct agent name, and forwards to the
//! agent container via `POST {container_url}/message`.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
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

use crate::audit_events::{
    self, DROP_DENYLISTED_SKILL, DROP_NO_ROUTE, DROP_REVIEWER_FILTER, DROP_SYNCHRONIZE_NO_DIFF,
    WebhookDropContext,
};
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
    /// Requested reviewer (present in pull_request.review_requested events).
    pub requested_reviewer: Option<GitHubUser>,
    /// Label data (present in issues.labeled events — the specific label just added).
    pub label: Option<GitHubLabel>,
    /// Repository data.
    pub repository: Option<GitHubRepository>,
    /// Commit SHA before the push (present on `pull_request.synchronize` events).
    pub before: Option<String>,
    /// Commit SHA after the push (present on `pull_request.synchronize` events).
    pub after: Option<String>,
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
pub struct GitHubLabel {
    pub name: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct GitHubRepository {
    pub full_name: Option<String>,
    pub html_url: Option<String>,
}

// -- Synchronize no-diff guard (#886) --

/// Response shape for GitHub Compare API (minimal — only the fields we need).
#[derive(Debug, serde::Deserialize)]
struct CompareResponse {
    /// File changes between the two commits. Empty array = no file changes.
    /// GitHub caps this at 300 files per response, but an empty array reliably
    /// means zero file changes (no pagination concern for the zero case).
    ///
    /// # `#[serde(default)]` risk acceptance
    ///
    /// If GitHub returns a malformed response without a `files` field,
    /// `serde(default)` yields an empty Vec, which we interpret as "no file
    /// changes" → suppression. This is vanishingly unlikely (GitHub's Compare
    /// API always includes `files`), and the consequence is low-severity —
    /// one suppressed review on a malformed response; the next genuine push
    /// triggers review normally.
    #[serde(default)]
    files: Vec<serde_json::Value>,
}

/// Check whether two commits have file-level differences.
///
/// Uses the GitHub Compare API: `GET /repos/{repo}/compare/{before}...{after}`.
/// Returns `Ok(true)` if files differ, `Ok(false)` if trees are identical.
/// Returns `Err` on any API/parse failure (caller should fail-open).
///
/// The zero-files heuristic: `files.is_empty()` is the determination.
/// For a trailer-only amend (the primary bug trigger), `before` and `after`
/// share the same tree SHA, so the compare returns `files: []` with
/// `status: "ahead"` (ahead by 1 commit, 0 file changes). We do NOT check
/// the `status` field — `status: "identical"` would miss the trailer-only
/// case (the commits are distinct objects, just with the same tree).
///
/// Accepts a pre-acquired token and API base URL to decouple token
/// acquisition (which may fail independently) from the compare call.
/// The caller handles token errors as a separate fail-open path.
const GITHUB_API_BASE_URL: &str = "https://api.github.com";

async fn commits_have_file_changes(
    token: &str,
    api_base_url: &str,
    repo_full_name: &str,
    before: &str,
    after: &str,
) -> Result<bool, anyhow::Error> {
    let url = format!("{api_base_url}/repos/{repo_full_name}/compare/{before}...{after}");

    let resp = reqwest::Client::new()
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", "mika-gateway")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .context("GitHub Compare API request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("GitHub Compare API returned HTTP {}", resp.status());
    }

    let body: CompareResponse = resp
        .json()
        .await
        .context("Failed to parse GitHub Compare API response")?;

    Ok(!body.files.is_empty())
}

// -- Event routing --

/// Skills that MUST NOT be instantiated via webhook-driven event flows.
///
/// Defense-in-depth for operator-only skills (#845). Layer 1 (well_known_agents.rs
/// disabled_skills) is the primary check; this gateway-side denylist catches future
/// routing-path additions that might bypass Layer 1.
///
/// The guard checks whether the formatted event text contains skill trigger
/// keywords that would activate a denylisted skill. If detected, the event
/// is dropped with a warning — the skill never reaches the agent.
const WEBHOOK_SKILL_DENYLIST: &[&str] = &["dev-groom"];

/// GitHub login of the autonomous QA reviewer bot. Only `review_requested`
/// events addressed to this reviewer trigger an autonomous qa-review session;
/// requests for human reviewers (e.g. the operator) must NOT spin up a full
/// qa-review (mika#1655). Out of scope: routing other reviewers to other
/// agents (different agents may want different handlers — see mika#1655 body).
const QA_REVIEWER_LOGIN: &str = "mika-platform-qa";

/// Internal org repos that should route to the well-known mika-dev agent container
/// without requiring a `github_repos` row. These are org-internal development repos,
/// not customer repos — they bypass the customer-registry lookup in multi-tenant mode.
///
/// When a webhook event arrives for one of these repos and no explicit `github_repos`
/// registration exists, the gateway resolves to the `agent_base_url` route (same as
/// single-tenant fallback) instead of dropping the event.
///
/// Adding a repo here is a code change + deploy — intentionally not env-tunable
/// (security-adjacent: changes on release cadence, not ops cadence).
///
/// See: mika#1382 (cross-repo ready-label dispatch).
const INTERNAL_REPOS: &[&str] = &[
    "senara-solutions/mika",
    "senara-solutions/mika-cloud",
    "senara-solutions/mika-skills",
    "senara-solutions/claude-pilot-py",
    "senara-solutions/mika-platform",
    "senara-solutions/wizzard",
];

/// Returns `true` if the given repository full name is in the internal-repo allowlist.
fn is_internal_repo(repo_full_name: &str) -> bool {
    INTERNAL_REPOS.contains(&repo_full_name)
}

const DEFAULT_GITHUB_BODY_TRUNCATION_CHARS: usize = 2_000;
const GITHUB_REVIEW_BODY_TRUNCATION_CHARS: usize = 16_000;

/// Check if a labeled event's label name matches a denylisted skill.
///
/// Only applies to `issues.labeled` events — returns `true` when the label name
/// matches a denylisted skill (case-insensitive). For all other event types,
/// returns `false` (Layer 1 disabled_skills is the primary defense).
fn is_webhook_denylisted_skill(
    event_type: &str,
    action: Option<&str>,
    label_name: Option<&str>,
) -> bool {
    // Only gate on issues.labeled — other event types rely on Layer 1
    if event_type != "issues" || action != Some("labeled") {
        return false;
    }
    let Some(name) = label_name else {
        return false;
    };
    let name_lower = name.to_lowercase();
    WEBHOOK_SKILL_DENYLIST
        .iter()
        .any(|skill| name_lower == *skill)
}

/// Returns `true` when a `pull_request.review_requested` event must be
/// suppressed because the requested reviewer is not the autonomous QA bot
/// ([`QA_REVIEWER_LOGIN`]).
///
/// `route_event` maps the `review_requested` action to `mika-qa`, but the
/// routing table cannot see the reviewer login. Without this filter, requesting
/// *any* reviewer on a PR — including the human operator — would spin up a full
/// autonomous qa-review session. Only requests addressed to the QA bot should
/// trigger one (mika#1655 AC1); every other reviewer (human, team, or a missing
/// `requested_reviewer`) is suppressed (mika#1655 AC2). Non-`review_requested`
/// actions are never suppressed by this guard (mika#1655 AC3).
fn is_suppressed_review_request(action: Option<&str>, requested_reviewer: Option<&str>) -> bool {
    action == Some("review_requested") && requested_reviewer != Some(QA_REVIEWER_LOGIN)
}

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
        ("issues", Some("labeled")) => Some("mika-dev"),
        ("issue_comment", Some("created")) => Some("mika-dev"),
        (
            "pull_request",
            Some("opened" | "synchronize" | "review_requested" | "ready_for_review"),
        ) => Some("mika-qa"),
        ("pull_request", Some("closed")) => Some("mika-dev"),
        ("pull_request_review", Some("submitted")) => Some("mika-dev"),
        ("check_suite", Some("completed")) => match check_conclusion {
            Some("failure" | "timed_out" | "success") => Some("mika-dev"),
            _ => None,
        },
        _ => None,
    }
}

/// Secondary fan-out targets for events that need to reach more than one agent (mika#1711).
///
/// Returns an empty slice for events with only a primary target. Currently used only for
/// `check_suite.completed(success)` — the primary target `mika-dev` handles merge-readiness
/// while `mika-qa` (secondary) fires an autonomous review via the
/// `qa-review-webhook-success` skill.
///
/// **Invariant:** the returned slice does NOT include the primary target from
/// `route_event`. Callers dispatch to primary + all secondaries.
pub fn secondary_targets(
    event_type: &str,
    action: Option<&str>,
    check_conclusion: Option<&str>,
) -> &'static [&'static str] {
    match (event_type, action, check_conclusion) {
        // check_suite success fan-out: mika-dev is primary (existing merge-readiness path),
        // mika-qa is secondary (new autonomous review path — mika#1711).
        ("check_suite", Some("completed"), Some("success")) => &["mika-qa"],
        _ => &[],
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
            let body = truncate_body(
                issue.and_then(|i| i.body.as_deref()).unwrap_or(""),
                DEFAULT_GITHUB_BODY_TRUNCATION_CHARS,
            );

            // For label-add events, emit a structured marker with the specific
            // label name so mika-dev's prompt can pattern-match unambiguously.
            // Checked first to avoid allocating body/text that would be discarded.
            if action == "labeled"
                && let Some(label_name) = event
                    .label
                    .as_ref()
                    .and_then(|l| l.name.as_deref())
                    .filter(|n| !n.is_empty())
            {
                return format!(
                    "[GitHub] Issue labeled {label_name} on {repo_name}#{number} — {title}\n{url}"
                );
            }
            // Fallback for labeled without label name: uses generic format below.

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
            let body = truncate_body(
                comment.and_then(|c| c.body.as_deref()).unwrap_or(""),
                DEFAULT_GITHUB_BODY_TRUNCATION_CHARS,
            );

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
            let body = truncate_body(
                pr.and_then(|p| p.body.as_deref()).unwrap_or(""),
                DEFAULT_GITHUB_BODY_TRUNCATION_CHARS,
            );

            let mut text = format!(
                "[GitHub] PR {action}: {repo_name}#{number} — {title} (branch: {branch})\n{url}"
            );
            if action == "closed" {
                let merged = pr.and_then(|p| p.merged).unwrap_or(false);
                text.push_str(&format!("\nMerged: {merged}"));
            }
            if action == "review_requested"
                && let Some(reviewer) = &event.requested_reviewer
            {
                text.push_str(&format!("\nRequested reviewer: @{}", reviewer.login));
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
            let body = truncate_body(
                review.and_then(|r| r.body.as_deref()).unwrap_or(""),
                GITHUB_REVIEW_BODY_TRUNCATION_CHARS,
            );

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
    /// 429 or 5xx, request timeout, or localhost connection error (agent may be
    /// restarting during deploy, #1293) — transient, worth retrying.
    Retryable {
        /// Human-readable description for logging (e.g. "HTTP 429" or "connection error").
        reason: String,
    },
    /// 4xx (other than 429), non-localhost connection error, or unresolvable route —
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

/// Whether a [`ForwardResult::Retryable`] reason denotes an HTTP 429 ("agent
/// busy") rejection — the only failure mode that drives the per-target circuit
/// breaker (mika#1710). The reason string is produced by
/// [`forward_to_resolved_route`] as `format!("HTTP {status}")`, so a 429 is
/// exactly `"HTTP 429"`. 5xx and connection-error retries use a different reason
/// and must not trip the 429 breaker (see `test_deliver_retry_budget_six_attempts_breaker_below_threshold`).
fn is_rate_limit_reason(reason: &str) -> bool {
    reason.starts_with("HTTP 429")
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

    // 2b. Fire-and-forget forward to cm-api (cm#88 Option B).
    // Signature verified — safe to fan out. cm-api re-verifies HMAC against
    // its own per-entity `webhook_secret` (samidarko populated all 6
    // github_repo entities with the same value as MIKA_GITHUB_WEBHOOK_SECRET),
    // so the same raw bytes + signature header land there authoritatively.
    // Fire-and-forget on cm-unreachable is the deliberate discipline — cm
    // MUST NEVER be on the gateway's critical path (same shape as cm#99
    // async-emit for cpp permission events). Failures log and drop; the
    // gateway's own routing to mika-spirit continues regardless.
    forward_to_cm_api(&state, sig, &headers, &body);

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

    // NOTE: Not using `.entered()` because the no-diff guard (#886) introduced
    // an `.await` point in this handler. `Entered` is `!Send`, making the future
    // `!Send` if held across await. Instead, enter/drop around sync blocks.
    let span = tracing::info_span!(
        "github_webhook",
        delivery_id = %delivery_id,
        event_type = %event_type,
    );
    let _entered = span.enter();

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
    let repo_full_name = event
        .repository
        .as_ref()
        .and_then(|r| r.full_name.as_deref());
    let target_agent = match route_event(event_type, event.action.as_deref(), check_conclusion) {
        Some(agent) => agent,
        None => {
            warn!(
                event_type,
                action = ?event.action,
                delivery_id = %delivery_id,
                "GitHub webhook event not routable, dropping"
            );
            // Observability-only audit_events row (mika#1774 AC1). The drop
            // decision is unchanged; a DB failure is logged WARN inside
            // `log_webhook_drop` and never propagates.
            //
            // Drop the span guard before the `.await` — `Entered` is `!Send`
            // (see note at the top of this handler) and would make the future
            // `!Send` if held across the DB write.
            let drop_ctx = WebhookDropContext {
                event_type,
                action: event.action.as_deref(),
                check_conclusion,
                delivery_id: &delivery_id,
                repo_full_name,
            };
            drop(_entered);
            audit_events::log_webhook_drop(&state.pool, &drop_ctx, DROP_NO_ROUTE).await;
            return StatusCode::OK;
        }
    };

    // 9a. Review-requested reviewer filter (mika#1655). `route_event` maps the
    // `review_requested` action to mika-qa, but only requests addressed to the
    // QA bot should trigger an autonomous qa-review. Suppress requests for human
    // reviewers (or teams / missing reviewer) here — before the routing info!
    // log and any dispatch — so a human-reviewer request never spins up a full
    // qa-review session.
    if is_suppressed_review_request(
        event.action.as_deref(),
        event.requested_reviewer.as_ref().map(|u| u.login.as_str()),
    ) {
        warn!(
            event_type,
            delivery_id = %delivery_id,
            requested_reviewer = ?event.requested_reviewer.as_ref().map(|u| u.login.as_str()),
            "GitHub webhook review_requested dropped — reviewer is not the QA bot"
        );
        // Observability-only audit_events row (mika#1774 AC1). Drop the span
        // guard first — `Entered` is `!Send` and would poison the future.
        let drop_ctx = WebhookDropContext {
            event_type,
            action: event.action.as_deref(),
            check_conclusion,
            delivery_id: &delivery_id,
            repo_full_name,
        };
        drop(_entered);
        audit_events::log_webhook_drop(&state.pool, &drop_ctx, DROP_REVIEWER_FILTER).await;
        return StatusCode::OK;
    }

    info!(
        event_type,
        action = ?event.action,
        target_agent,
        delivery_id = %delivery_id,
        "GitHub webhook routing event to agent"
    );

    // 9b. Webhook skill denylist guard (#845, Layer 3 defense-in-depth).
    // Only applies to issues.labeled events — checks the label name against the
    // denylist. Layer 1 (well_known_agents.rs disabled_skills) is the primary
    // defense; this gateway-side guard catches routing-path additions that bypass
    // Layer 1.
    let denylist_drop: Option<WebhookDropContext<'_>> = {
        let label_name = event.label.as_ref().and_then(|l| l.name.as_deref());
        if is_webhook_denylisted_skill(event_type, event.action.as_deref(), label_name) {
            warn!(
                event_type,
                delivery_id = %delivery_id,
                target_agent,
                label_name = ?label_name,
                "GitHub webhook dropped by skill denylist guard (operator-only skill trigger detected)"
            );
            Some(WebhookDropContext {
                event_type,
                action: event.action.as_deref(),
                check_conclusion,
                delivery_id: &delivery_id,
                repo_full_name,
            })
        } else {
            None
        }
    };

    // Drop the span guard before the await to keep the handler future Send.
    // Re-entered after the no-diff check for the remaining sync operations.
    drop(_entered);

    if let Some(drop_ctx) = denylist_drop {
        // Observability-only audit_events row (mika#1774 AC1). Fires AFTER
        // the span-guard drop so the DB write's `.await` stays `Send`.
        audit_events::log_webhook_drop(&state.pool, &drop_ctx, DROP_DENYLISTED_SKILL).await;
        return StatusCode::OK;
    }

    // 9c. Synchronize no-diff guard (#886): suppress mika-qa dispatch
    // for no-op pushes (trailer-only amend, commit-message-only change).
    // Uses GitHub Compare API to check file-level differences between
    // the before and after commit SHAs. Fail-open on any error.
    //
    // Token acquisition and Compare API call are split so that each
    // failure mode (token refresh error vs. API error vs. timeout) has
    // its own explicit fail-open path — required for test coverage (#886 AC).
    if event_type == "pull_request"
        && event.action.as_deref() == Some("synchronize")
        && let Some(before) = event.before.as_deref()
        && let Some(after) = event.after.as_deref()
        && let Some(github_app) = state.github_app.as_ref()
    {
        let repo = event
            .repository
            .as_ref()
            .and_then(|r| r.full_name.as_deref())
            .unwrap_or("");
        let api_base = state
            .github_api_base_url
            .as_deref()
            .unwrap_or(GITHUB_API_BASE_URL);
        match github_app.installation_token().await {
            Err(e) => {
                // Fail-open: token refresh failure should not block legitimate reviews.
                warn!(
                    error = %e,
                    delivery_id = %delivery_id,
                    before,
                    after,
                    "synchronize_no_diff_check token refresh failed, proceeding with dispatch (fail-open)"
                );
            }
            Ok(token) => {
                match commits_have_file_changes(&token, api_base, repo, before, after).await {
                    Ok(false) => {
                        info!(
                            event_type,
                            delivery_id = %delivery_id,
                            before,
                            after,
                            repo,
                            "webhook_synchronize_no_diff_change: suppressing qa-review dispatch for no-op push"
                        );
                        // Observability-only audit_events row (mika#1774 AC1).
                        // Span guard was already dropped above; safe to await.
                        let repo_opt = if repo.is_empty() { None } else { Some(repo) };
                        audit_events::log_webhook_drop(
                            &state.pool,
                            &WebhookDropContext {
                                event_type,
                                action: event.action.as_deref(),
                                check_conclusion,
                                delivery_id: &delivery_id,
                                repo_full_name: repo_opt,
                            },
                            DROP_SYNCHRONIZE_NO_DIFF,
                        )
                        .await;
                        return StatusCode::OK;
                    }
                    Ok(true) => {
                        // Files changed — proceed with normal dispatch.
                    }
                    Err(e) => {
                        // Fail-open: API error should not block legitimate reviews.
                        warn!(
                            error = %e,
                            delivery_id = %delivery_id,
                            before,
                            after,
                            "synchronize_no_diff_check failed, proceeding with dispatch (fail-open)"
                        );
                    }
                }
            }
        }
    }
    // No before/after SHAs or no github_app — proceed with dispatch (fail-open).

    // Re-enter span for remaining sync operations (semaphore, format, spawn).
    let _entered = span.enter();

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

    let target = target_agent.to_string();
    let repo_name = event.repository.as_ref().and_then(|r| r.full_name.clone());

    // 11b. In-flight delivery bound (mika#1710 R4/AC4). The `webhook_semaphore`
    // only bounds *concurrent HTTP* — tasks sleeping between retries hold no
    // permit and could accumulate without limit under a flood. `delivery_slots`
    // caps the number of concurrently-spawned delivery tasks; the permit is held
    // for the entire delivery lifetime (including retry sleeps). At capacity we
    // shed durably to the DLQ (bounded, drop-nothing overflow store) instead of
    // spawning an unbounded task.
    let delivery_slot = match state.delivery_slots.clone().try_acquire_owned() {
        Ok(slot) => slot,
        Err(_) => {
            warn!(
                target_agent = %target,
                request_id = %request_id,
                event = "delivery_buffer_full",
                "in-flight delivery cap reached — shedding webhook to DLQ instead of spawning"
            );
            crate::dlq::insert_delivery(
                &state.pool,
                crate::dlq::NewDelivery {
                    delivery_id: &request_id,
                    event_type,
                    target_agent: &target,
                    repo_full_name: repo_name.as_deref(),
                    payload: &text,
                    request_id: &request_id,
                    attempts: 0,
                    last_error: "delivery_buffer_full",
                },
            )
            .await;
            return StatusCode::OK;
        }
    };

    // 12. Async dispatch with retry — return 200 to GitHub immediately.
    // Retry budget: initial attempt + up to 5 retries with backoff [2s, 5s, 15s, 60s, 300s].
    // Events that exhaust retries (or trip the target circuit breaker) are
    // persisted in the DLQ (#590, mika#1710).
    let forwarding_state = state.clone();
    let semaphore = state.webhook_semaphore.clone();
    let event_type_owned = event_type.to_string();
    let repo_name_for_primary = repo_name.clone();
    let text_for_primary = text.clone();
    let request_id_for_primary = request_id.clone();
    tokio::spawn(async move {
        // Hold the delivery slot for the whole delivery lifetime (retry sleeps
        // included); it is released when this task ends.
        let _delivery_slot = delivery_slot;
        deliver_with_retry(
            &forwarding_state,
            &target,
            &text_for_primary,
            &request_id_for_primary,
            repo_name_for_primary.as_deref(),
            &event_type_owned,
            permit,
            &semaphore,
        )
        .await;
    });

    // 12b. Fan-out to secondary agents (mika#1711). Currently only
    // check_suite.completed(success) fans out — mika-dev (primary above) drives
    // merge readiness, mika-qa (secondary here) fires autonomous review via the
    // qa-review-webhook-success skill.
    //
    // Each secondary target acquires its own semaphore permit + delivery slot.
    // If either slot cannot be acquired for a secondary, we log-and-skip (the
    // primary is already dispatched — a secondary failure must not fail the
    // primary). Fan-out failures also do NOT enqueue the DLQ: the primary is
    // authoritative for retry/replay, and a lost secondary review will be
    // re-driven by the next relevant event.
    let secondaries = secondary_targets(event_type, event.action.as_deref(), check_conclusion);
    for &secondary in secondaries {
        let secondary_permit = match state.webhook_semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                warn!(
                    secondary_target = secondary,
                    event_type,
                    delivery_id = %delivery_id,
                    "webhook fan-out skipped for secondary — semaphore full"
                );
                continue;
            }
        };
        let secondary_slot = match state.delivery_slots.clone().try_acquire_owned() {
            Ok(s) => s,
            Err(_) => {
                warn!(
                    secondary_target = secondary,
                    event_type,
                    delivery_id = %delivery_id,
                    "webhook fan-out skipped for secondary — delivery slots full"
                );
                continue;
            }
        };

        let forwarding_state = state.clone();
        let semaphore = state.webhook_semaphore.clone();
        let event_type_owned = event_type.to_string();
        let secondary_target = secondary.to_string();
        let text_for_secondary = text.clone();
        let request_id_for_secondary = request_id.clone();
        let repo_name_for_secondary = repo_name.clone();
        info!(
            event_type,
            action = ?event.action,
            secondary_target = %secondary_target,
            delivery_id = %delivery_id,
            "GitHub webhook fan-out to secondary agent"
        );
        tokio::spawn(async move {
            let _delivery_slot = secondary_slot;
            deliver_with_retry(
                &forwarding_state,
                &secondary_target,
                &text_for_secondary,
                &request_id_for_secondary,
                repo_name_for_secondary.as_deref(),
                &event_type_owned,
                secondary_permit,
                &semaphore,
            )
            .await;
        });
    }

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
                // Internal-repo allowlist: org-internal repos resolve to the well-known
                // agent route without requiring a github_repos registration (mika#1382).
                // Explicit registrations (above) always take precedence.
                if is_internal_repo(repo_name) {
                    if let Some(ref base) = state.agent_base_url {
                        info!(
                            repo = repo_name,
                            "internal repo resolved via allowlist to agent_base_url"
                        );
                        return Some(ResolvedRoute {
                            container_url: base.clone(),
                            agent_mapping: serde_json::Value::Object(serde_json::Map::new()),
                        });
                    }
                    // Multi-tenant without agent_base_url: internal repos cannot be routed.
                    // This is expected in K8s prod where internal repos should have explicit
                    // github_repos rows pointing to the mika-dev customer container.
                    warn!(
                        repo = repo_name,
                        "internal repo matched allowlist but no MIKA_AGENT_BASE_URL configured, \
                         dropping event (add a github_repos row for multi-tenant routing)"
                    );
                    return None;
                }

                // Non-internal repo with no registration
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
                // Connection refused to a local agent — may be restarting during deploy.
                // Retry with backoff; if all retries fail the event falls through to DLQ.
                // Scoped to localhost per mika#1293 — extend to other routes if the
                // same pattern is observed (see issue "Out of scope").
                let is_localhost = route.container_url.starts_with("http://localhost")
                    || route.container_url.starts_with("http://127.0.0.1");
                if is_localhost {
                    ForwardResult::Retryable {
                        reason: format!("connection error (localhost, retryable): {e}"),
                    }
                } else {
                    ForwardResult::Permanent {
                        reason: format!("connection error: {e}"),
                    }
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

/// Fire-and-forget forward a validated GitHub webhook to cm-api (cm#88 Option B).
///
/// Called from [`handle_github_webhook`] immediately after HMAC verification
/// succeeds. Preserves the raw body and the `X-Hub-Signature-256`,
/// `X-GitHub-Event`, and `X-GitHub-Delivery` headers so cm-api can re-verify
/// against its own per-entity `webhook_secret` (which samidarko populated
/// with the same value as `MIKA_GITHUB_WEBHOOK_SECRET`, so any signed
/// payload accepted here is accepted there).
///
/// **Discipline**: cm MUST NEVER be on the gateway's critical path. This
/// mirrors the fire-and-forget contract from cm#99 (cpp permission events →
/// cm event_log): time-bounded transport, drop-on-error, log-and-continue,
/// never blocks the caller. The gateway's response to GitHub is unaffected
/// by cm's reachability or verdict.
///
/// **Disabled path**: when `state.cm_api_url` is `None`, this is a no-op
/// (zero cost, zero HTTP calls). Enable via `MIKA_CM_API_URL` env var.
fn forward_to_cm_api(state: &AppState, signature: &str, headers: &HeaderMap, body: &Bytes) {
    let Some(cm_url) = state.cm_api_url.as_ref() else {
        return;
    };
    let event_type = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let sig = signature.to_string();
    let url = format!("{}/api/v1/webhooks/github", cm_url.trim_end_matches('/'));
    let http = state.http_client.clone();
    let body = body.clone();

    tokio::spawn(async move {
        // Bounded transport so a slow cm-api never wedges the gateway task pool.
        // 5s is well above cm-api's < 50ms healthy latency; treats any longer
        // response as cm-unreachable and drops.
        let req = http
            .post(&url)
            .header("x-hub-signature-256", &sig)
            .header("x-github-event", &event_type)
            .header("x-github-delivery", &delivery_id)
            .header("content-type", "application/json")
            .body(body.clone())
            .timeout(Duration::from_secs(5));

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    debug!(
                        cm_url = %url,
                        event_type = %event_type,
                        delivery_id = %delivery_id,
                        status = status.as_u16(),
                        "cm-api forwarded webhook accepted"
                    );
                } else {
                    warn!(
                        cm_url = %url,
                        event_type = %event_type,
                        delivery_id = %delivery_id,
                        status = status.as_u16(),
                        "cm-api rejected forwarded webhook (dropping)"
                    );
                }
            }
            Err(e) => {
                warn!(
                    cm_url = %url,
                    event_type = %event_type,
                    delivery_id = %delivery_id,
                    error = %e,
                    "cm-api forward failed (dropping — cm is not on the critical path)"
                );
            }
        }
    });
}

/// Retry wrapper for GitHub webhook delivery.
///
/// Calls [`forward_to_resolved_route`] once using the caller's semaphore permit.
/// On a retryable failure (429, 5xx, request timeout, or localhost connection
/// error — see #1293), releases the permit,
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

        // Circuit breaker (mika#1710): if the target agent's circuit is open,
        // short-circuit this delivery straight to the DLQ instead of issuing an
        // HTTP attempt against a saturated agent. This is the cross-event
        // coordination layer that stops N independent retry chains from
        // co-amplifying a 429 flood — the DLQ re-attempts on its own spaced
        // schedule. The half-open probe (see `check_delivery`) periodically lets
        // one delivery through to re-test recovery.
        if state.target_health.check_delivery(target_agent)
            == crate::circuit_breaker::DeliveryDecision::ShortCircuit
        {
            warn!(
                target_agent,
                request_id,
                attempts_made,
                "target circuit open — short-circuiting delivery to DLQ (no HTTP attempt)"
            );
            current_permit.take();
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
                    last_error: if last_reason.is_empty() {
                        "circuit_open"
                    } else {
                        &last_reason
                    },
                },
            )
            .await;
            return;
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
                // Delivery landed — close the target's circuit and reset 429 state.
                state.target_health.record_success(target_agent);
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
                // Feed the per-target circuit breaker (mika#1710). Only genuine
                // HTTP 429 "agent busy" rejections drive it — 5xx and connection
                // retries are a different failure mode. On the soft threshold the
                // circuit opens and the next iteration short-circuits to the DLQ;
                // on the rolling-window hard threshold we emit the AC5 self-heal
                // pause signal.
                if is_rate_limit_reason(&reason)
                    && state.target_health.record_429(target_agent)
                        == crate::circuit_breaker::Record429Outcome::HardPaused
                {
                    warn!(
                        target_agent,
                        request_id,
                        event = "gateway_target_paused",
                        "target 429 flood crossed the hard threshold — pausing retries to this target (self-heal)"
                    );
                }
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
    fn test_route_event_issues_labeled() {
        assert_eq!(
            route_event("issues", Some("labeled"), None),
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
    fn test_route_event_pr_ready_for_review() {
        // mika#1822: draft→ready transitions were silently dropped as
        // "not routable" because the routing table only covered opened /
        // synchronize / review_requested. Every PR opened as draft (dispatch-lib's
        // wip-rescue path, systematic) then promoted to ready never triggered
        // mika-qa. This test pins the routing table entry that closes the gap.
        assert_eq!(
            route_event("pull_request", Some("ready_for_review"), None),
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
    fn test_route_event_pr_review_requested() {
        // route_event maps the action to mika-qa regardless of reviewer; the
        // reviewer-login discrimination is enforced separately by
        // is_suppressed_review_request (mika#1655). This test pins the routing
        // table's behavior (AC3 — no regression on the action→agent mapping).
        assert_eq!(
            route_event("pull_request", Some("review_requested"), None),
            Some("mika-qa")
        );
    }

    // --- mika#1655: review-requested reviewer filter ---

    #[test]
    fn test_review_request_for_qa_bot_is_dispatched() {
        // AC1: a review_requested addressed to the QA bot is NOT suppressed,
        // so it reaches mika-qa and triggers an autonomous qa-review.
        assert!(!is_suppressed_review_request(
            Some("review_requested"),
            Some(QA_REVIEWER_LOGIN)
        ));
    }

    #[test]
    fn test_review_request_for_human_reviewer_is_suppressed() {
        // AC2 (negative case): a review_requested addressed to a human reviewer
        // MUST be suppressed — it must not spin up a qa-review session.
        assert!(is_suppressed_review_request(
            Some("review_requested"),
            Some("vincent")
        ));
    }

    #[test]
    fn test_review_request_with_no_reviewer_is_suppressed() {
        // AC2 (defensive): a review_requested whose reviewer cannot be resolved
        // (e.g. a team request carrying `requested_team` instead of
        // `requested_reviewer`) is suppressed — fail-closed, not fail-open.
        assert!(is_suppressed_review_request(Some("review_requested"), None));
    }

    #[test]
    fn test_non_review_request_actions_are_never_suppressed() {
        // AC3: the filter only governs the review_requested action. opened,
        // synchronize, and closed pass through untouched regardless of the
        // (absent) reviewer field.
        assert!(!is_suppressed_review_request(Some("opened"), None));
        assert!(!is_suppressed_review_request(Some("synchronize"), None));
        assert!(!is_suppressed_review_request(Some("closed"), None));
        // Even if a reviewer login is somehow present on a non-review action.
        assert!(!is_suppressed_review_request(
            Some("opened"),
            Some(QA_REVIEWER_LOGIN)
        ));
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

    // --- secondary_targets tests (mika#1711) ---

    #[test]
    fn test_secondary_targets_check_suite_success_fans_out_to_qa() {
        // The core fix: mika-qa must receive check_suite.completed(success) via
        // fan-out so autonomous review fires without needing an explicit
        // review_requested webhook. Primary target stays mika-dev (route_event).
        assert_eq!(
            secondary_targets("check_suite", Some("completed"), Some("success")),
            &["mika-qa"]
        );
    }

    #[test]
    fn test_secondary_targets_check_suite_failure_no_fanout() {
        // Failures stay mika-dev-only — self-dev-webhook-ci handles them.
        assert!(secondary_targets("check_suite", Some("completed"), Some("failure")).is_empty());
    }

    #[test]
    fn test_secondary_targets_check_suite_timed_out_no_fanout() {
        assert!(secondary_targets("check_suite", Some("completed"), Some("timed_out")).is_empty());
    }

    #[test]
    fn test_secondary_targets_pull_request_events_no_fanout() {
        for action in [
            "opened",
            "synchronize",
            "review_requested",
            "ready_for_review",
            "closed",
        ] {
            assert!(
                secondary_targets("pull_request", Some(action), None).is_empty(),
                "pull_request.{action} must not fan out"
            );
        }
    }

    #[test]
    fn test_secondary_targets_issues_no_fanout() {
        for action in ["assigned", "labeled"] {
            assert!(
                secondary_targets("issues", Some(action), None).is_empty(),
                "issues.{action} must not fan out"
            );
        }
    }

    #[test]
    fn test_secondary_targets_pull_request_review_no_fanout() {
        assert!(secondary_targets("pull_request_review", Some("submitted"), None).is_empty());
    }

    #[test]
    fn test_secondary_targets_check_suite_without_conclusion_no_fanout() {
        // Defensive: an incomplete check_suite payload must not fan out.
        assert!(secondary_targets("check_suite", Some("completed"), None).is_empty());
    }

    #[test]
    fn test_secondary_targets_no_intersection_with_primary() {
        // Invariant: for every routable event, the secondary list must NOT
        // include the primary target — that would cause double-delivery to
        // the same agent.
        let cases: &[(&str, Option<&str>, Option<&str>)] = &[
            ("issues", Some("assigned"), None),
            ("issues", Some("labeled"), None),
            ("issue_comment", Some("created"), None),
            ("pull_request", Some("opened"), None),
            ("pull_request", Some("synchronize"), None),
            ("pull_request", Some("review_requested"), None),
            ("pull_request", Some("closed"), None),
            ("pull_request_review", Some("submitted"), None),
            ("check_suite", Some("completed"), Some("success")),
            ("check_suite", Some("completed"), Some("failure")),
            ("check_suite", Some("completed"), Some("timed_out")),
        ];
        for (event_type, action, conclusion) in cases {
            let primary = route_event(event_type, *action, *conclusion);
            let secondaries = secondary_targets(event_type, *action, *conclusion);
            if let Some(primary_name) = primary {
                assert!(
                    !secondaries.contains(&primary_name),
                    "{event_type}.{action:?}({conclusion:?}) — secondary_targets must not include primary '{primary_name}'"
                );
            }
        }
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

    // -- Internal repo allowlist tests (mika#1382) --

    #[test]
    fn test_is_internal_repo_known_repos() {
        assert!(is_internal_repo("senara-solutions/mika"));
        assert!(is_internal_repo("senara-solutions/mika-cloud"));
        assert!(is_internal_repo("senara-solutions/mika-skills"));
        assert!(is_internal_repo("senara-solutions/claude-pilot-py"));
        assert!(is_internal_repo("senara-solutions/mika-platform"));
        assert!(is_internal_repo("senara-solutions/wizzard"));
    }

    #[test]
    fn test_is_internal_repo_unknown_repos() {
        assert!(!is_internal_repo("senara-solutions/other-repo"));
        assert!(!is_internal_repo("other-org/mika"));
        assert!(!is_internal_repo(""));
        assert!(!is_internal_repo("mika"));
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
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
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
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("pull_request", &event);
        assert!(text.contains("[GitHub] PR opened"));
        assert!(text.contains("org/repo#10"));
        assert!(text.contains("fix/bug"));
    }

    #[test]
    fn test_format_event_text_pr_review_preserves_verdict_past_legacy_cap() {
        let long_review = format!(
            "{}\nVERDICT: pass\nReady to merge.",
            "review detail. ".repeat(220)
        );
        assert!(long_review.chars().count() > DEFAULT_GITHUB_BODY_TRUNCATION_CHARS);
        assert!(long_review.chars().count() < GITHUB_REVIEW_BODY_TRUNCATION_CHARS);

        let event = GitHubWebhookEvent {
            action: Some("submitted".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: None,
            pull_request: Some(GitHubPullRequest {
                number: Some(909),
                title: Some("Fix review routing".to_string()),
                html_url: Some("https://github.com/senara-solutions/mika/pull/909".to_string()),
                body: None,
                head: Some(GitHubRef {
                    ref_name: Some("fix/review-routing".to_string()),
                }),
                merged: None,
            }),
            comment: None,
            review: Some(GitHubReview {
                state: Some("approved".to_string()),
                body: Some(long_review),
                html_url: Some(
                    "https://github.com/senara-solutions/mika/pull/909#pullrequestreview-1"
                        .to_string(),
                ),
                user: Some(GitHubUser {
                    login: "mika-qa".to_string(),
                    user_type: Some("Bot".to_string()),
                }),
            }),
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("senara-solutions/mika".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };

        let text = format_event_text("pull_request_review", &event);
        assert!(text.contains("[GitHub] PR review (approved)"));
        assert!(text.contains("by @mika-qa"));
        assert!(text.contains("VERDICT: pass"));
        assert!(!text.contains("[truncated]"));
    }

    #[test]
    fn test_format_event_text_pr_review_still_truncates_above_review_cap() {
        let long_review = format!(
            "{}\nVERDICT: pass\n{}",
            "lead detail. ".repeat(20),
            "tail detail. ".repeat(2_000)
        );
        assert!(long_review.chars().count() > GITHUB_REVIEW_BODY_TRUNCATION_CHARS);

        let event = GitHubWebhookEvent {
            action: Some("submitted".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: None,
            pull_request: Some(GitHubPullRequest {
                number: Some(909),
                title: Some("Fix review routing".to_string()),
                html_url: Some("https://github.com/senara-solutions/mika/pull/909".to_string()),
                body: None,
                head: Some(GitHubRef {
                    ref_name: Some("fix/review-routing".to_string()),
                }),
                merged: None,
            }),
            comment: None,
            review: Some(GitHubReview {
                state: Some("approved".to_string()),
                body: Some(long_review),
                html_url: Some(
                    "https://github.com/senara-solutions/mika/pull/909#pullrequestreview-1"
                        .to_string(),
                ),
                user: Some(GitHubUser {
                    login: "mika-qa".to_string(),
                    user_type: Some("Bot".to_string()),
                }),
            }),
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("senara-solutions/mika".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };

        let text = format_event_text("pull_request_review", &event);
        assert!(text.contains("VERDICT: pass"));
        assert!(text.contains("[truncated]"));
    }

    /// Regression fixture for the mika#909/#898 failure shape: VERDICT placed at
    /// the very end of a body that exceeds the 16 KB review cap. The verdict line
    /// must be clipped — this documents the structural cap boundary. If a future
    /// cap raise is needed, this test documents the failure mode that drove the
    /// prior raise.
    #[test]
    fn test_format_event_text_pr_review_verdict_at_bottom_clips_when_body_exceeds_cap() {
        // Build a body where VERDICT appears well past the 16 KB boundary
        let preamble = "review preamble. ".repeat(1_200); // ~20,400 chars
        let long_review = format!("{preamble}\nVERDICT: pass\nReady to merge.");
        assert!(long_review.chars().count() > GITHUB_REVIEW_BODY_TRUNCATION_CHARS);

        // Verify VERDICT is positioned past the cap
        let verdict_offset = long_review.find("VERDICT: pass").unwrap();
        assert!(verdict_offset > GITHUB_REVIEW_BODY_TRUNCATION_CHARS);

        let event = GitHubWebhookEvent {
            action: Some("submitted".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: None,
            pull_request: Some(GitHubPullRequest {
                number: Some(909),
                title: Some("Fix review routing".to_string()),
                html_url: Some("https://github.com/senara-solutions/mika/pull/909".to_string()),
                body: None,
                head: Some(GitHubRef {
                    ref_name: Some("fix/review-routing".to_string()),
                }),
                merged: None,
            }),
            comment: None,
            review: Some(GitHubReview {
                state: Some("approved".to_string()),
                body: Some(long_review),
                html_url: Some(
                    "https://github.com/senara-solutions/mika/pull/909#pullrequestreview-1"
                        .to_string(),
                ),
                user: Some(GitHubUser {
                    login: "mika-qa".to_string(),
                    user_type: Some("Bot".to_string()),
                }),
            }),
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("senara-solutions/mika".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };

        let text = format_event_text("pull_request_review", &event);
        // VERDICT is past the cap — must be clipped
        assert!(
            !text.contains("VERDICT: pass"),
            "VERDICT should be clipped when positioned past the 16KB cap"
        );
        assert!(text.contains("[truncated]"));
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
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("check_suite", &event);
        assert!(text.contains("[GitHub] Check suite failure"));
        assert!(text.contains("org/repo"));
        assert!(text.contains("main"));
    }

    #[test]
    fn test_format_event_text_pr_review_requested_with_reviewer() {
        let event = GitHubWebhookEvent {
            action: Some("review_requested".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: None,
            pull_request: Some(GitHubPullRequest {
                number: Some(15),
                title: Some("Add feature".to_string()),
                html_url: Some("https://github.com/org/repo/pull/15".to_string()),
                body: None,
                head: Some(GitHubRef {
                    ref_name: Some("feat/new".to_string()),
                }),
                merged: None,
            }),
            comment: None,
            review: None,
            requested_reviewer: Some(GitHubUser {
                login: "mika-platform-qa".to_string(),
                user_type: Some("User".to_string()),
            }),
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("pull_request", &event);
        assert!(text.contains("[GitHub] PR review_requested"));
        assert!(text.contains("org/repo#15"));
        assert!(text.contains("Requested reviewer: @mika-platform-qa"));
    }

    #[test]
    fn test_format_event_text_issue_labeled_extracts_label_name() {
        let event = GitHubWebhookEvent {
            action: Some("labeled".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: Some(GitHubIssue {
                number: Some(841),
                title: Some("Gate dispatch on ready label".to_string()),
                html_url: Some("https://github.com/senara-solutions/mika/issues/841".to_string()),
                body: None,
                assignee: None,
            }),
            pull_request: None,
            comment: None,
            review: None,
            requested_reviewer: None,
            label: Some(GitHubLabel {
                name: Some("ready".to_string()),
            }),
            repository: Some(GitHubRepository {
                full_name: Some("senara-solutions/mika".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("issues", &event);

        // Contract: the producer's output for label_name="ready" must start with
        // the canonical READY_LABEL_DISPATCH_MARKER prefix shared via mika-common.
        // Renaming the constant or the producer template breaks this assertion
        // and surfaces the cross-crate drift at CI time (mika#852).
        assert!(
            text.starts_with(mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER),
            "format_event_text drifted from READY_LABEL_DISPATCH_MARKER: \
             expected prefix {:?}, got {:?}",
            mika_common::github_event_format::READY_LABEL_DISPATCH_MARKER,
            text,
        );

        // Existing exact-shape assertion (regression: full output stays stable).
        assert_eq!(
            text,
            "[GitHub] Issue labeled ready on senara-solutions/mika#841 — Gate dispatch on ready label\nhttps://github.com/senara-solutions/mika/issues/841"
        );
    }

    #[test]
    fn test_format_event_text_issue_labeled_empty_label_name() {
        // Empty label name should fall back to generic format, not produce
        // a malformed structured marker with a blank name.
        let event = GitHubWebhookEvent {
            action: Some("labeled".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: Some(GitHubIssue {
                number: Some(100),
                title: Some("Test issue".to_string()),
                html_url: Some("https://github.com/org/repo/issues/100".to_string()),
                body: None,
                assignee: None,
            }),
            pull_request: None,
            comment: None,
            review: None,
            requested_reviewer: None,
            label: Some(GitHubLabel {
                name: Some(String::new()),
            }),
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("issues", &event);
        // Must fall back to generic format, not the structured "labeled <name> on" marker
        assert!(text.starts_with("[GitHub] Issue labeled:"));
        assert!(!text.contains("Issue labeled  on")); // no blank-name marker
    }

    #[test]
    fn test_format_event_text_issue_labeled_missing_label_name() {
        let event = GitHubWebhookEvent {
            action: Some("labeled".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: Some(GitHubIssue {
                number: Some(100),
                title: Some("Test issue".to_string()),
                html_url: Some("https://github.com/org/repo/issues/100".to_string()),
                body: None,
                assignee: None,
            }),
            pull_request: None,
            comment: None,
            review: None,
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("issues", &event);
        // Falls back to generic format when label name is unavailable
        assert!(text.starts_with("[GitHub] Issue labeled:"));
        assert!(text.contains("org/repo#100"));
    }

    #[test]
    fn test_format_event_text_pr_review_requested_without_reviewer() {
        let event = GitHubWebhookEvent {
            action: Some("review_requested".to_string()),
            sender: None,
            installation: None,
            check_suite: None,
            issue: None,
            pull_request: Some(GitHubPullRequest {
                number: Some(15),
                title: Some("Add feature".to_string()),
                html_url: Some("https://github.com/org/repo/pull/15".to_string()),
                body: None,
                head: Some(GitHubRef {
                    ref_name: Some("feat/new".to_string()),
                }),
                merged: None,
            }),
            comment: None,
            review: None,
            requested_reviewer: None,
            label: None,
            repository: Some(GitHubRepository {
                full_name: Some("org/repo".to_string()),
                html_url: None,
            }),
            before: None,
            after: None,
        };
        let text = format_event_text("pull_request", &event);
        assert!(text.contains("[GitHub] PR review_requested"));
        assert!(!text.contains("Requested reviewer"));
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
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");

        let state = AppState {
            pool,
            telegram: Some(telegram),
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: Some(SecretString::from("b".repeat(64))),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some("http://localhost:9999".to_string()),
            agents_namespace: "mika-agents".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: webhook_secret.map(|s| SecretString::from(s.to_string())),
            github_delivery_cache: new_delivery_cache(),
            github_app: None,
            github_api_base_url: None,
            orchestrator_inbox_enabled: false,
            inbox_subscriber_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
            gateway_external_url: None,
            cm_api_url: None,
            target_health: Arc::new(crate::circuit_breaker::TargetCircuitBreaker::new()),
            delivery_slots: Arc::new(tokio::sync::Semaphore::new(
                crate::circuit_breaker::MAX_INFLIGHT_DELIVERIES,
            )),
            search_egress_client: None,
            fetch_egress_client: None,
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
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");
        let delivery_cache = new_delivery_cache();

        let state = AppState {
            pool,
            telegram: Some(telegram),
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: Some(SecretString::from("b".repeat(64))),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some("http://localhost:9999".to_string()),
            agents_namespace: "mika-agents".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: Some(SecretString::from("test-secret")),
            github_delivery_cache: delivery_cache.clone(),
            github_app: None,
            github_api_base_url: None,
            orchestrator_inbox_enabled: false,
            inbox_subscriber_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
            gateway_external_url: None,
            cm_api_url: None,
            target_health: Arc::new(crate::circuit_breaker::TargetCircuitBreaker::new()),
            delivery_slots: Arc::new(tokio::sync::Semaphore::new(
                crate::circuit_breaker::MAX_INFLIGHT_DELIVERIES,
            )),
            search_egress_client: None,
            fetch_egress_client: None,
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
    fn test_forward_result_localhost_connection_error_is_retryable() {
        // mika#1293: localhost connection errors are retryable (agent may be restarting).
        let r = ForwardResult::Retryable {
            reason: "connection error (localhost, retryable): connection refused".to_string(),
        };
        assert!(r.is_retryable());
        assert_eq!(
            r.reason(),
            Some("connection error (localhost, retryable): connection refused")
        );
    }

    #[test]
    fn test_forward_result_non_localhost_connection_error_remains_permanent() {
        // Non-localhost connection errors remain permanent per mika#1293 scope.
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

    // -- Connection error classification tests (mika#1293) --
    //
    // These test the actual `forward_to_resolved_route` function to verify that
    // localhost connection errors are Retryable and non-localhost are Permanent.

    #[tokio::test]
    async fn test_localhost_connection_error_is_retryable_integration() {
        // Connect to a port that refuses connections (no server listening).
        // Uses 127.0.0.1 to avoid DNS resolution — a refused TCP connection
        // triggers `e.is_connect()` in reqwest.
        let state = test_state_with_base_url("http://127.0.0.1:1");
        let route = ResolvedRoute {
            container_url: "http://127.0.0.1:1".to_string(),
            agent_mapping: serde_json::json!({}),
        };

        let result = forward_to_resolved_route(
            &state,
            &route,
            "mika-dev",
            "test event",
            "delivery-conn-localhost",
            Some("org/repo"),
        )
        .await;

        assert!(
            matches!(result, ForwardResult::Retryable { .. }),
            "localhost connection error should be Retryable, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_non_localhost_connection_error_remains_permanent_integration() {
        // Use 127.0.0.2:1 — still on the loopback interface (triggers TCP RST /
        // connection refused, so `is_connect()` returns true), but does NOT match
        // our localhost check (`starts_with("http://localhost")` or
        // `starts_with("http://127.0.0.1")`). This exercises the non-localhost
        // branch of the classification logic.
        let state = test_state_with_base_url("http://127.0.0.2:1");
        let route = ResolvedRoute {
            container_url: "http://127.0.0.2:1".to_string(),
            agent_mapping: serde_json::json!({}),
        };

        let result = forward_to_resolved_route(
            &state,
            &route,
            "mika-dev",
            "test event",
            "delivery-conn-remote",
            Some("org/repo"),
        )
        .await;

        assert!(
            matches!(result, ForwardResult::Permanent { .. }),
            "non-localhost connection error should be Permanent, got: {result:?}"
        );
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
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");

        AppState {
            pool,
            telegram: Some(telegram),
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: Some(SecretString::from("b".repeat(64))),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some(base_url.to_string()),
            agents_namespace: "test".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: Some(SecretString::from("test-secret")),
            github_delivery_cache: new_delivery_cache(),
            github_app: None,
            github_api_base_url: None,
            orchestrator_inbox_enabled: false,
            inbox_subscriber_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
            gateway_external_url: None,
            cm_api_url: None,
            target_health: Arc::new(crate::circuit_breaker::TargetCircuitBreaker::new()),
            delivery_slots: Arc::new(tokio::sync::Semaphore::new(
                crate::circuit_breaker::MAX_INFLIGHT_DELIVERIES,
            )),
            search_egress_client: None,
            fetch_egress_client: None,
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

    /// Test-only short retry schedule: 5 sub-millisecond delays so the delivery
    /// loop runs fast while the 30s soft-open circuit window still dominates.
    const TEST_DELAYS_FAST: [Duration; 5] = [Duration::from_millis(1); 5];

    #[tokio::test]
    async fn test_deliver_retry_budget_exhausted_after_six_attempts() {
        // mika#1710 D1 REFRAME: with the shared per-target circuit breaker active,
        // a lone event against a persistently-busy target trips the soft breaker at
        // CB_SOFT_THRESHOLD (3) consecutive 429s and short-circuits to the DLQ
        // instead of hammering all 6 attempts. The reduced in-chain retry budget is
        // the ratified amplification-control semantics (plan § Decision D1);
        // durability is preserved because the event lands in the DLQ, which
        // re-attempts on its own spaced schedule. Previously this asserted 6 HTTP
        // calls; the new correct outcome is CB_SOFT_THRESHOLD calls.
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::TOO_MANY_REQUESTS; 6]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry_inner(
            &state,
            "mika-dev",
            "test event",
            "delivery-exhausted",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
            &TEST_DELAYS_FAST,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            crate::circuit_breaker::CB_SOFT_THRESHOLD as usize,
            "breaker must short-circuit to the DLQ at the soft trip (~3 attempts), not all 6"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_deliver_retry_budget_six_attempts_breaker_below_threshold() {
        // mika#1710 D1 COMPANION: non-429 retryable failures (503) do NOT drive the
        // 429 circuit breaker, so the pure 6-attempt exhaustion path is preserved
        // for the breaker-not-tripped case. Proves the breaker keys strictly on
        // HTTP 429 ("agent busy"), not on generic retryable errors.
        let (base_url, call_count) =
            mock_agent_server(vec![StatusCode::SERVICE_UNAVAILABLE; 6]).await;
        let state = test_state_with_base_url(&base_url);
        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry_inner(
            &state,
            "mika-dev",
            "test event",
            "delivery-503-exhausted",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
            &TEST_DELAYS_FAST,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            6,
            "503s don't trip the 429 breaker — full 6-attempt budget must be spent"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_deliver_short_circuits_to_dlq_when_open() {
        // mika#1710 R1: with the target's circuit pre-opened, deliver_with_retry_inner
        // must persist to the DLQ WITHOUT issuing a single HTTP attempt.
        let (base_url, call_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url);

        // Pre-open the circuit for the target (3 consecutive 429s → 30s open window).
        let now = std::time::Instant::now();
        for _ in 0..crate::circuit_breaker::CB_SOFT_THRESHOLD {
            state.target_health.record_429_at("mika-dev", now);
        }

        let semaphore = state.webhook_semaphore.clone();
        let permit = semaphore.clone().try_acquire_owned().unwrap();

        deliver_with_retry_inner(
            &state,
            "mika-dev",
            "test event",
            "delivery-open",
            Some("org/repo"),
            "issues",
            permit,
            &semaphore,
            &TEST_DELAYS_FAST,
        )
        .await;

        assert_eq!(
            call_count.load(Ordering::SeqCst),
            0,
            "open circuit must short-circuit to the DLQ with zero HTTP attempts"
        );
        assert_eq!(semaphore.available_permits(), 30);
    }

    #[tokio::test]
    async fn test_inflight_bound_sheds_to_dlq() {
        // mika#1710 R4/AC4: when the in-flight delivery bound is exhausted, a new
        // webhook is shed to the DLQ instead of spawning an (unbounded) delivery
        // task — the target agent must receive zero delivery attempts. The gateway
        // still returns 200 to GitHub (durable at-least-once via the DLQ).
        let (base_url, call_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let mut state = test_state_with_base_url(&base_url);
        // Exhaust the in-flight delivery bound (zero available slots).
        state.delivery_slots = Arc::new(tokio::sync::Semaphore::new(0));
        let app = axum::Router::new()
            .route("/webhook/github", post(handle_github_webhook))
            .with_state(state);

        // issue_comment.created is routable (→ mika-dev), so the only reason the
        // mock agent receives zero calls is the in-flight shed to the DLQ.
        let body = br#"{"action":"created","sender":{"login":"octocat","type":"User"},"issue":{"number":7,"title":"t"},"comment":{"body":"hello"},"repository":{"full_name":"org/repo"}}"#;
        let req = make_request("test-secret", body, "issue_comment", "inflight-shed-uuid");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "shed path must still return 200 to GitHub"
        );

        // Give any (erroneously) spawned delivery task time to reach the mock agent.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            0,
            "exhausted in-flight bound must shed to the DLQ, not deliver"
        );
    }

    #[tokio::test]
    async fn test_inflight_bound_allows_when_capacity() {
        // mika#1710 R4/AC4: with capacity available, the webhook spawns a normal
        // delivery that reaches the agent — proving the bound does not over-shed.
        let (base_url, call_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let state = test_state_with_base_url(&base_url); // default MAX_INFLIGHT_DELIVERIES slots
        let app = axum::Router::new()
            .route("/webhook/github", post(handle_github_webhook))
            .with_state(state);

        let body = br#"{"action":"created","sender":{"login":"octocat","type":"User"},"issue":{"number":8,"title":"t"},"comment":{"body":"hello"},"repository":{"full_name":"org/repo"}}"#;
        let req = make_request("test-secret", body, "issue_comment", "inflight-allow-uuid");
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Poll up to a generous budget for the async delivery to land (route
        // resolution against the lazy fake pool errors after ~100ms then falls
        // back to the mock agent base URL).
        let start = std::time::Instant::now();
        while call_count.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(3) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            call_count.load(Ordering::SeqCst),
            1,
            "with capacity available the webhook must be delivered exactly once"
        );
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

    // -- Webhook skill denylist guard tests (#845) --

    #[test]
    fn test_webhook_denylist_blocks_dev_groom_label() {
        // issues.labeled with label name "dev-groom" must be blocked.
        assert!(is_webhook_denylisted_skill(
            "issues",
            Some("labeled"),
            Some("dev-groom"),
        ));
    }

    #[test]
    fn test_webhook_denylist_case_insensitive() {
        assert!(is_webhook_denylisted_skill(
            "issues",
            Some("labeled"),
            Some("Dev-Groom"),
        ));
        assert!(is_webhook_denylisted_skill(
            "issues",
            Some("labeled"),
            Some("DEV-GROOM"),
        ));
    }

    #[test]
    fn test_webhook_denylist_allows_normal_events() {
        // issues.labeled with a non-denylisted label passes through.
        assert!(!is_webhook_denylisted_skill(
            "issues",
            Some("labeled"),
            Some("ready"),
        ));
        // Non-labeled issue events pass through even if free-text mentions
        // "dev-groom" (no false-positive on body content).
        assert!(!is_webhook_denylisted_skill(
            "issues",
            Some("assigned"),
            None,
        ));
        // Other event types always pass through (Layer 1 is the primary defense).
        assert!(!is_webhook_denylisted_skill(
            "pull_request",
            Some("opened"),
            None,
        ));
        assert!(!is_webhook_denylisted_skill(
            "issue_comment",
            Some("created"),
            None,
        ));
    }

    #[test]
    fn test_webhook_denylist_contains_dev_groom() {
        // Verify the denylist contains dev-groom
        assert!(
            WEBHOOK_SKILL_DENYLIST.contains(&"dev-groom"),
            "WEBHOOK_SKILL_DENYLIST must contain dev-groom for operator-only enforcement"
        );
    }

    // -- Synchronize no-diff guard tests (#886) --

    #[test]
    fn test_synchronize_before_after_deserialization() {
        // pull_request.synchronize payloads include before/after commit SHAs
        let json = r#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 1},
            "repository": {"full_name": "org/repo"}
        }"#;
        let event: GitHubWebhookEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.before.as_deref(), Some("abc123"));
        assert_eq!(event.after.as_deref(), Some("def456"));
    }

    #[test]
    fn test_synchronize_before_after_absent_for_opened() {
        // pull_request.opened payloads do NOT include before/after
        let json = r#"{
            "action": "opened",
            "pull_request": {"number": 1},
            "repository": {"full_name": "org/repo"}
        }"#;
        let event: GitHubWebhookEvent = serde_json::from_str(json).unwrap();
        assert!(event.before.is_none());
        assert!(event.after.is_none());
    }

    #[test]
    fn test_compare_response_empty_files() {
        // Empty files array → no file changes (no-op push)
        let json = r#"{"files": []}"#;
        let resp: CompareResponse = serde_json::from_str(json).unwrap();
        assert!(resp.files.is_empty());
    }

    #[test]
    fn test_compare_response_with_files() {
        // Non-empty files → files changed
        let json = r#"{"files": [{"filename": "foo.rs", "status": "modified"}]}"#;
        let resp: CompareResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.files.is_empty());
    }

    #[test]
    fn test_compare_response_missing_files_field() {
        // Missing files field → defaults to empty via #[serde(default)]
        // Risk acceptance: vanishingly unlikely, low-severity consequence (one suppressed review)
        let json = r#"{"status": "ahead"}"#;
        let resp: CompareResponse = serde_json::from_str(json).unwrap();
        assert!(resp.files.is_empty());
    }

    #[tokio::test]
    async fn test_synchronize_no_github_app_passes_through() {
        // When github_app is None, synchronize events pass through (graceful degradation).
        // This test verifies the handler returns 200 and the event is dispatched
        // (not suppressed) when GitHubApp is not configured.
        let app = test_router(Some("test-secret"));

        let body = br#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 1, "title": "Test PR"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "pull_request", "sync-uuid-1");
        let resp = app.oneshot(req).await.unwrap();
        // 200 OK — the event is routed to mika-qa and dispatched (spawn returns 200)
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -- Synchronize no-diff guard integration tests (#886, tests 6-10) --
    //
    // These tests exercise the full webhook handler path with a mock GitHub
    // Compare API server and a mock agent server, verifying both suppression
    // (no-diff) and fail-open (error/timeout/token failure) behaviors.

    /// RSA 2048-bit test key for GitHubApp construction in integration tests.
    /// Same key as `mika-common::github_app::tests::TEST_RSA_PEM`.
    /// Name must contain `TEST_RSA_PEM` to pass the no-secrets lefthook guard.
    const TEST_RSA_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEpAIBAAKCAQEAqmNXtQx4L3Eko0G+ky5u03BpRRwLfQ1+zuRzUxtDIAb2LFcf\n\
2PCCusvna5qAuXfCttcsTTFt0+x3vqI3wkO7pZ7MQatBcuQSFL3eSDhqNNLZ8zh6\n\
evsuCwgdhn+etApM8PtEpwcps/pjlLsIb9iyB7jcYBQsr0lGRrXVdsGPQXF0yGkr\n\
Vd3zH11tqLOWdDGXlpZTZkwwow7ojVID5POWHkp1WkY6xYCq6qYA1Gt9VfQNCzE8\n\
WKjsd0phBgN1W4le0Q30UFiaFDErbW1uqrsQShz0Wv9bHbUpVOyTotGdXOBWg0CP\n\
rJ5IBmt8KF73HLaK0zOwIe9qwlCLMszH4d+TaQIDAQABAoIBAA6rd/EwDibzhFaE\n\
Ag709/i/XGjlVb3iDBFvDNjSZ5CZ2NcPdz/70R2ZEacribquK3cHhppsz4pn+RVS\n\
LR/OKhlD100uG/fy1/WuNTWdmdNLdhVhPvZYqumrPLOISFcy7dXvpEUHMll7DNjQ\n\
05ShoQ5WJa8l/YTn96N940+Ssa1OHesGZJa4ATP+fxiXqow5Mq/DbLTWBQ0Kj2Qc\n\
WZFa6wc1ws61zK81U69gtW7+nnX2hzcboQhq8RVEmtJKINmfieuHSl0QOZsEuh09\n\
fFjLLwUhwIrmZKNv3hpqJpyKL6dvgr1f+5xyfgYUoQIFB2G8V7+Xto1urGYHNjRO\n\
DVWCbXMCgYEA04A9zJnYxwNPqnC86rxWy9fN0AsB8S4sWoO9M/ZWexfiWsMK6Mze\n\
uOfj1cVNjBm6aLJL6F2ts/ig4wA6alR72P5ZRqneAMFgIes5SP7j70U3gFodcCe/\n\
RoVhWNyjX4Oz9Dwu57QK5DB+3NRM/4On0wsO4GjQgl1RQnDZfYccRUcCgYEAzjyz\n\
CzQKzT21jyzb0/0xBovlUwxnctXV5lHScHETXh8TJdgD4gU+tBJNcJoa/swSNRgL\n\
6KfXj1LH4tbl0vBZps3RpuWVobqEZrkBjkJO9aGRsTkqtlEQJ0Lc3yQPbTWENlG+\n\
VbfrOkAyTn69LNmndOMBKq7syBrKJTtwgVcTec8CgYEAghGr79ftaPawV7FdfT62\n\
YkYlXHxohVpQDJpYEUy9gpX9rrOkUecsUarKgv0D49UuvpRn+k8iNDwDNZc+VYX/\n\
ZEOHw91TmkNSS4nNgQbARrXanCTPVdob19LPO0b1chgc42bfsb8Xs53fZw9pCvp8\n\
i12RmJDdKk8ZWjLsjjY5PKECgYB9EqC+nZwjZlYyc1EJ2hYeUz8LQ42FPhuPp3WJ\n\
DXpibVQOclfAfc/OIv9l13+hoJ82JdQrD4cR+3EPp6YPbAXivBV2Muuw/k2HgpFn\n\
9dyu6IJTyUiW8shqFwmeJd9ZKsh4rNBSacy1MfOQWRpfFcyRfY3aleUxYdXQCKEt\n\
P2KnTwKBgQCMt/E5AyZ1x7xsD68M/+dQc4kZG+3wyjfgkQ5tivveW5JxRNJ7Doy/\n\
Zk4PUTq3pSCC2sQY5Ay2b2iPez8d660jFuWT02+0sQdFmGwnFC9IxdEUPZXxeRr6\n\
omInFBLWVyWK89xoc49UvUcyRcbL3iWqa+zAv7eOC5TZyy1SVJtPVw==\n\
-----END RSA PRIVATE KEY-----";

    /// Build a mock GitHub Compare API server returning a controlled response.
    /// The `compare_response` closure is called for each request to produce
    /// the status code and body. Returns `(base_url, call_count)`.
    async fn mock_github_compare_server(
        status: StatusCode,
        body: &'static str,
    ) -> (String, Arc<AtomicUsize>) {
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();

        let app = axum::Router::new().route(
            "/repos/{owner}/{repo}/compare/{spec}",
            axum::routing::get(move || {
                let cc = cc.clone();
                async move {
                    cc.fetch_add(1, Ordering::SeqCst);
                    (status, body.to_string())
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

    /// Build a mock GitHub Compare API server that hangs forever (for timeout tests).
    async fn mock_github_compare_server_hanging() -> (String, Arc<AtomicUsize>) {
        let call_count = Arc::new(AtomicUsize::new(0));
        let cc = call_count.clone();

        let app = axum::Router::new().route(
            "/repos/{owner}/{repo}/compare/{spec}",
            axum::routing::get(move || {
                let cc = cc.clone();
                async move {
                    cc.fetch_add(1, Ordering::SeqCst);
                    // Hang beyond the 5s timeout
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    (StatusCode::OK, r#"{"files": []}"#.to_string())
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

    /// Build a test router with a GitHubApp (pre-seeded token) and custom API base URL.
    /// Also spins up a mock agent server so forwarded events are captured.
    async fn test_router_with_github_app(
        github_api_base_url: &str,
        agent_base_url: &str,
        github_app: Option<Arc<mika_common::github_app::GitHubApp>>,
    ) -> axum::Router {
        use crate::routes::AppState;
        use crate::telegram::TelegramClient;
        use secrecy::SecretString;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        let http_client = reqwest::Client::new();
        let telegram =
            TelegramClient::new(http_client.clone(), SecretString::from("fake-bot-token"));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy("postgres://fake:fake@localhost/fake")
            .expect("lazy pool");

        let state = AppState {
            pool,
            telegram: Some(telegram),
            http_client,
            internal_token: SecretString::from("a".repeat(64)),
            webhook_secret: Some(SecretString::from("b".repeat(64))),
            ready: Arc::new(AtomicBool::new(true)),
            webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(30)),
            agent_base_url: Some(agent_base_url.to_string()),
            agents_namespace: "test".to_string(),
            webhook_counter: Arc::new(AtomicU64::new(0)),
            github_webhook_secret: Some(SecretString::from("test-secret".to_string())),
            github_delivery_cache: new_delivery_cache(),
            github_app,
            github_api_base_url: Some(github_api_base_url.to_string()),
            orchestrator_inbox_enabled: false,
            inbox_subscriber_semaphore: Arc::new(tokio::sync::Semaphore::new(10)),
            gateway_external_url: None,
            cm_api_url: None,
            target_health: Arc::new(crate::circuit_breaker::TargetCircuitBreaker::new()),
            delivery_slots: Arc::new(tokio::sync::Semaphore::new(
                crate::circuit_breaker::MAX_INFLIGHT_DELIVERIES,
            )),
            search_egress_client: None,
            fetch_egress_client: None,
        };

        axum::Router::new()
            .route("/webhook/github", post(handle_github_webhook))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_synchronize_no_diff_suppressed() {
        // Mock Compare API returns empty files → no-op push detected.
        // Handler should suppress dispatch: return 200 but NOT forward to agent.
        let (github_api_url, compare_count) =
            mock_github_compare_server(StatusCode::OK, r#"{"files": []}"#).await;
        let (agent_url, agent_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let github_app =
            mika_common::github_app::GitHubApp::new_with_test_token("fake-token").await;

        let app = test_router_with_github_app(&github_api_url, &agent_url, Some(github_app)).await;

        let body = br#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 42, "title": "Test PR"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "pull_request", "sync-no-diff-1");
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        // Compare API was called
        assert_eq!(
            compare_count.load(Ordering::SeqCst),
            1,
            "Compare API should be called exactly once"
        );
        // Give a brief window for any async spawn to land (there shouldn't be one)
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Agent server should NOT have been called (dispatch suppressed)
        assert_eq!(
            agent_count.load(Ordering::SeqCst),
            0,
            "Agent should not receive the event when diff is empty (no-op push)"
        );
    }

    #[tokio::test]
    async fn test_synchronize_with_diff_dispatched() {
        // Mock Compare API returns non-empty files → genuine diff detected.
        // Handler should forward the event to the agent.
        let (github_api_url, compare_count) = mock_github_compare_server(
            StatusCode::OK,
            r#"{"files": [{"filename": "src/main.rs", "status": "modified"}]}"#,
        )
        .await;
        let (agent_url, agent_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let github_app =
            mika_common::github_app::GitHubApp::new_with_test_token("fake-token").await;

        let app = test_router_with_github_app(&github_api_url, &agent_url, Some(github_app)).await;

        let body = br#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 42, "title": "Test PR"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "pull_request", "sync-with-diff-1");
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            compare_count.load(Ordering::SeqCst),
            1,
            "Compare API should be called exactly once"
        );
        // Wait for the async spawn to deliver the event
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            agent_count.load(Ordering::SeqCst),
            1,
            "Agent should receive the event when diff contains file changes"
        );
    }

    #[tokio::test]
    async fn test_synchronize_api_error_fail_open() {
        // Mock Compare API returns HTTP 500 → fail-open, event dispatched.
        let (github_api_url, compare_count) = mock_github_compare_server(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"message": "internal error"}"#,
        )
        .await;
        let (agent_url, agent_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let github_app =
            mika_common::github_app::GitHubApp::new_with_test_token("fake-token").await;

        let app = test_router_with_github_app(&github_api_url, &agent_url, Some(github_app)).await;

        let body = br#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 42, "title": "Test PR"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "pull_request", "sync-api-err-1");
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            compare_count.load(Ordering::SeqCst),
            1,
            "Compare API should be called exactly once"
        );
        // Fail-open: event should be forwarded to agent despite API error
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            agent_count.load(Ordering::SeqCst),
            1,
            "Agent should receive the event on API error (fail-open)"
        );
    }

    #[tokio::test]
    async fn test_synchronize_api_timeout_fail_open() {
        // Mock Compare API hangs beyond the 5s timeout → fail-open, event dispatched.
        let (github_api_url, compare_count) = mock_github_compare_server_hanging().await;
        let (agent_url, agent_count) = mock_agent_server(vec![StatusCode::OK]).await;
        let github_app =
            mika_common::github_app::GitHubApp::new_with_test_token("fake-token").await;

        let app = test_router_with_github_app(&github_api_url, &agent_url, Some(github_app)).await;

        let body = br#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 42, "title": "Test PR"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "pull_request", "sync-timeout-1");
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            compare_count.load(Ordering::SeqCst),
            1,
            "Compare API should receive the request before timing out"
        );
        // Fail-open: event should be forwarded to agent despite timeout
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            agent_count.load(Ordering::SeqCst),
            1,
            "Agent should receive the event on API timeout (fail-open)"
        );
    }

    #[tokio::test]
    async fn test_synchronize_token_refresh_failure_fail_open() {
        // GitHubApp with empty cache and unreachable exchange endpoint →
        // installation_token() returns Err → fail-open, event dispatched.
        //
        // We use GitHubApp::new() with a valid signing key but no seeded token.
        // installation_token() will attempt JWT exchange against the real GitHub
        // API URL (which will fail in test), producing the Err path.
        let signing_key = jsonwebtoken::EncodingKey::from_rsa_pem(TEST_RSA_PEM.as_bytes()).unwrap();
        // No seeded token — installation_token() will try (and fail) JWT exchange
        let github_app = mika_common::github_app::GitHubApp::new(99999, signing_key, 99999);

        // Mock agent server captures forwarded events
        let (agent_url, agent_count) = mock_agent_server(vec![StatusCode::OK]).await;
        // Compare API won't be reached (token failure happens first), but set up
        // a base URL anyway to avoid hitting real GitHub.
        let (github_api_url, compare_count) =
            mock_github_compare_server(StatusCode::OK, r#"{"files": []}"#).await;

        let app = test_router_with_github_app(&github_api_url, &agent_url, Some(github_app)).await;

        let body = br#"{
            "action": "synchronize",
            "before": "abc123",
            "after": "def456",
            "pull_request": {"number": 42, "title": "Test PR"},
            "repository": {"full_name": "org/repo"}
        }"#;
        let req = make_request("test-secret", body, "pull_request", "sync-token-fail-1");
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        // Compare API should NOT be called (token failure short-circuits)
        assert_eq!(
            compare_count.load(Ordering::SeqCst),
            0,
            "Compare API should not be called when token refresh fails"
        );
        // Fail-open: event should be forwarded to agent despite token failure
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            agent_count.load(Ordering::SeqCst),
            1,
            "Agent should receive the event on token refresh failure (fail-open)"
        );
    }

    // -- cm#88 Option B: fork GitHub webhook to cm-api tests --
    //
    // These verify the fire-and-forget discipline: cm MUST NEVER be on the
    // gateway's critical path. See `forward_to_cm_api` for the contract.

    /// When `cm_api_url` is `None`, forwarding is a compile-time no-op —
    /// zero HTTP calls, no task spawn, no cost. This is the shipped default.
    #[tokio::test]
    async fn test_forward_to_cm_api_noop_when_url_absent() {
        let state = test_state_with_base_url("http://unused");
        assert!(
            state.cm_api_url.is_none(),
            "test_state_with_base_url must default cm_api_url to None"
        );
        let headers = HeaderMap::new();
        let body = Bytes::from_static(b"{}");
        // Function should return immediately without panicking. No easy
        // observable side-effect to assert beyond "did not panic + did not
        // block" — both follow from the code inspection + the `None` guard
        // returning before any I/O.
        forward_to_cm_api(&state, "sha256=deadbeef", &headers, &body);
    }

    /// When `cm_api_url` points at an unreachable address, the CALLER of
    /// `forward_to_cm_api` must not be blocked — the spawned task takes
    /// the timeout hit in the background. This is the load-bearing property
    /// samidarko named: "cm MUST NEVER be on the gateway's critical path"
    /// (mirrors cm#99 async-emit discipline).
    #[tokio::test]
    async fn test_forward_to_cm_api_is_fire_and_forget_on_unreachable() {
        let mut state = test_state_with_base_url("http://unused");
        // 240.0.0.1 is TEST-NET-3 reserved space — guaranteed unroutable,
        // so the reqwest connect attempt hangs until timeout.
        state.cm_api_url = Some("http://240.0.0.1:65535".to_string());
        let headers = HeaderMap::new();
        let body = Bytes::from_static(b"{}");

        // The 5s timeout on the spawned task means the request itself takes
        // seconds to fail. But the CALLER of forward_to_cm_api should
        // return in microseconds — the spawned task takes the wait.
        let start = std::time::Instant::now();
        forward_to_cm_api(&state, "sha256=deadbeef", &headers, &body);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(100),
            "forward_to_cm_api must return quickly (spawn only). Elapsed: {elapsed:?} — indicates the caller waited on the HTTP call, violating fire-and-forget."
        );
    }
}
