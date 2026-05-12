# Plan: Fix cross-session duplicate QA review on synchronize webhook (#886)

type: fix
issue: 886
date: 2026-05-12

## Problem

`pull_request.synchronize` webhooks fire for every push to a PR branch, including no-op pushes (trailer-only amends, commit-message-only changes). Each push spawns a new mika-qa session. The session-scope `pr_reviews_posted` DashMap (#821/#822) only dedupes within a single session, so cross-session duplicates pass through unchecked.

Evidence: PR #885 received two identical APPROVED reviews from two separate mika-qa sessions, ~9 minutes apart, triggered by a trailer-only force-push.

## Root Cause

The gateway's `route_event()` unconditionally routes all `pull_request.synchronize` events to mika-qa. There is no check for whether the push actually changed the PR's diff. The session-scope DashMap was designed for within-session dedup (required-tools-gate retry); cross-session dedup was explicitly named as a future risk in the compound doc (lines 130-132).

## Decision: Gateway-level suppression

The ticket identifies three fix surfaces. This plan implements **Surface 1 (gateway)** as the primary fix.

**Why gateway, not agent:** The bug is "we triggered work that shouldn't have been triggered." A full qa-review session costs an LLM call + GitHub API calls + tool executions. Suppressing at the gateway saves all of that compute. Agent-level dedup (Surfaces 2/3) would still create a session, load skills, and potentially make API calls before detecting the duplicate.

**How:** For `synchronize` events, the gateway compares the `before` and `after` commit SHAs (provided by GitHub in the webhook payload) via the GitHub Compare API. If the comparison shows zero file changes, the event is suppressed — no dispatch to mika-qa.

### Architectural boundary crossing: gateway as GitHub API consumer

The gateway is currently a stateless webhook router. This change introduces outbound GitHub API calls — the first time the gateway consumes the GitHub API rather than just receiving webhooks from it. This is a deliberate, narrow expansion of the gateway's responsibility:

**Justification:** The suppression decision is a routing concern, not an agent concern. The gateway already discriminates by event type and action (`route_event()`); this extends that discrimination to event content (whether the push changed files). The alternative — dispatching to the agent and having it decide — wastes a full session's compute for a decision the gateway can make with one API call.

**Scope containment:** The gateway does NOT become a general-purpose GitHub API consumer. The API capability is limited to a single endpoint (`compare/{before}...{after}`) used only for `synchronize` events. The `GitHubApp` instance is shared from `mika-common` (already a dependency) and its token cache handles refresh automatically via double-checked locking.

**Precedent:** This is a new pattern for the gateway. If future guards need GitHub API access, they should use the same `Arc<GitHubApp>` instance. No separate ADR warranted — the scope is a single API call gated to a single event action.

## Pinned Source

### GitHubWebhookEvent struct (`crates/mika-gateway/src/github.rs:54-76`)

```rust
pub struct GitHubWebhookEvent {
    pub action: Option<String>,
    pub sender: Option<GitHubUser>,
    pub installation: Option<GitHubInstallation>,
    pub check_suite: Option<CheckSuite>,
    pub issue: Option<GitHubIssue>,
    pub pull_request: Option<GitHubPullRequest>,
    pub comment: Option<GitHubComment>,
    pub review: Option<GitHubReview>,
    pub requested_reviewer: Option<GitHubUser>,
    pub label: Option<GitHubLabel>,
    pub repository: Option<GitHubRepository>,
}
```

**Change:** Add `before: Option<String>` and `after: Option<String>` after `label`. These are top-level fields in GitHub's `pull_request.synchronize` webhook payload. Serde `#[serde(default)]` is not needed — `Option<String>` already defaults to `None` for missing fields.

### AppState struct (`crates/mika-gateway/src/routes.rs:85-104`)

```rust
pub struct AppState {
    pub pool: PgPool,
    pub telegram: TelegramClient,
    pub http_client: reqwest::Client,
    pub internal_token: SecretString,
    pub webhook_secret: SecretString,
    pub ready: Arc<AtomicBool>,
    pub webhook_semaphore: Arc<tokio::sync::Semaphore>,
    pub agent_base_url: Option<String>,
    pub agents_namespace: String,
    pub webhook_counter: Arc<AtomicU64>,
    pub github_webhook_secret: Option<SecretString>,
    pub github_delivery_cache: Arc<std::sync::Mutex<lru::LruCache<String, ()>>>,
}
```

**Change:** Add `pub github_app: Option<Arc<GitHubApp>>` — constructed at startup via `GitHubApp::from_settings(&settings)` (returns `None` when credentials are incomplete, which is the fail-open case). No separate `GitHubApiClient` wrapper needed — `GitHubApp` already owns an `http_client` and handles token caching.

### Webhook handler insertion point (`crates/mika-gateway/src/github.rs:618-647`)

```rust
    // 9b. Webhook skill denylist guard (#845, Layer 3 defense-in-depth).
    {
        let label_name = event.label.as_ref().and_then(|l| l.name.as_deref());
        if is_webhook_denylisted_skill(event_type, event.action.as_deref(), label_name) {
            warn!(...);
            return StatusCode::OK;
        }
    }

    // 10. Semaphore for backpressure
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        ...
    };

    // 11. Format message text
    let text = format_event_text(event_type, &event);
```

**Insert between steps 9b and 10:** The no-diff guard goes here — after routing and denylist checks, before semaphore acquisition and event formatting. Suppressed events don't consume a semaphore permit or incur formatting cost.

### Routing logic (`crates/mika-gateway/src/github.rs:189-207`)

```rust
pub fn route_event(
    event_type: &str,
    action: Option<&str>,
    check_conclusion: Option<&str>,
) -> Option<&'static str> {
    match (event_type, action) {
        ("issues", Some("assigned")) => Some("mika-dev"),
        ("issues", Some("labeled")) => Some("mika-dev"),
        ("issue_comment", Some("created")) => Some("mika-dev"),
        ("pull_request", Some("opened" | "synchronize" | "review_requested")) => Some("mika-qa"),
        ("pull_request", Some("closed")) => Some("mika-dev"),
        ("pull_request_review", Some("submitted")) => Some("mika-dev"),
        ("check_suite", Some("completed")) => match check_conclusion { ... },
        _ => None,
    }
}
```

The no-diff guard matches `event_type == "pull_request" && action == "synchronize"` — consistent with `route_event()`'s pattern match. The guard fires after `route_event()` has already confirmed the target is mika-qa.

### GitHubApp token management API (`crates/mika-common/src/github_app.rs:56-154`)

```rust
pub struct GitHubApp {
    app_id: u64,
    signing_key: EncodingKey,
    installation_id: u64,
    cache: RwLock<Option<CachedToken>>,
    http_client: reqwest::Client,
}

impl GitHubApp {
    pub fn from_settings(settings: &Settings) -> Option<Arc<Self>> { ... }
    pub async fn installation_token(&self) -> Result<String> { ... }
}
```

**Token lifecycle:** `installation_token()` uses `RwLock`-based double-checked locking. Fast path: read lock returns cached token if valid (checked against `EXPIRY_BUFFER` = 5 minutes before actual expiry). Slow path: write lock, generate JWT, exchange for installation token, cache. Installation tokens expire after 1 hour; proactive refresh at 55 minutes. **No additional refresh logic needed in the gateway** — each `installation_token()` call transparently handles refresh.

**Fail-open paths for token errors:**
- `GitHubApp::from_settings()` returns `None` when credentials are incomplete → `AppState.github_app` is `None` → guard skipped entirely, all synchronize events pass through
- `installation_token()` fails (JWT generation error, HTTP error to GitHub API) → `commits_have_file_changes()` returns `Err` → fail-open, event dispatched

## Implementation

### Step 1: Add `before`/`after` fields to `GitHubWebhookEvent`

**File:** `crates/mika-gateway/src/github.rs`

```rust
pub struct GitHubWebhookEvent {
    pub action: Option<String>,
    pub sender: Option<GitHubUser>,
    pub installation: Option<GitHubInstallation>,
    pub check_suite: Option<CheckSuite>,
    pub issue: Option<GitHubIssue>,
    pub pull_request: Option<GitHubPullRequest>,
    pub comment: Option<GitHubComment>,
    pub review: Option<GitHubReview>,
    pub requested_reviewer: Option<GitHubUser>,
    pub label: Option<GitHubLabel>,
    pub repository: Option<GitHubRepository>,
    /// Commit SHA before the push (present on pull_request.synchronize events).
    pub before: Option<String>,
    /// Commit SHA after the push (present on pull_request.synchronize events).
    pub after: Option<String>,
}
```

### Step 2: Wire `GitHubApp` into `AppState`

**File:** `crates/mika-gateway/src/routes.rs`

Add to `AppState`:
```rust
pub github_app: Option<Arc<GitHubApp>>,
```

At gateway startup (wherever `AppState` is constructed), add:
```rust
let github_app = GitHubApp::from_settings(&settings);
```

### Step 3: Add `commits_have_file_changes()` helper

**File:** `crates/mika-gateway/src/github.rs` (or new module `crates/mika-gateway/src/github_api.rs`)

```rust
/// Response shape for GitHub Compare API (minimal — only the fields we need).
#[derive(Debug, serde::Deserialize)]
struct CompareResponse {
    /// File changes between the two commits. Empty array = no file changes.
    /// GitHub caps this at 300 files per response, but an empty array reliably
    /// means zero file changes (no pagination concern for the zero case).
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
async fn commits_have_file_changes(
    github_app: &GitHubApp,
    repo_full_name: &str,
    before: &str,
    after: &str,
) -> Result<bool, anyhow::Error> {
    let token = github_app.installation_token().await?;

    let url = format!(
        "https://api.github.com/repos/{repo_full_name}/compare/{before}...{after}"
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("Accept", "application/vnd.github.v3+json")
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", "mika-gateway")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .context("GitHub Compare API request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "GitHub Compare API returned HTTP {}",
            resp.status()
        );
    }

    let body: CompareResponse = resp
        .json()
        .await
        .context("Failed to parse GitHub Compare API response")?;

    Ok(!body.files.is_empty())
}
```

**Why `Vec<serde_json::Value>` for `files`:** We only need to know if the array is empty. Deserializing full file objects is waste. `serde_json::Value` is the cheapest typed deserialization that preserves the "is it empty?" check.

**Why a fresh `reqwest::Client::new()`:** The gateway's `AppState.http_client` is configured for agent forwarding (potentially with different timeouts/headers). A dedicated client for the GitHub API avoids coupling. The cost is negligible — `reqwest::Client` is cheap to construct and connection pooling is per-host regardless.

**5-second timeout rationale:** GitHub Compare API typical latency is 100-500ms. 5s provides 10x headroom for slow responses. On timeout, the `Err` path triggers fail-open — the event dispatches normally. Hardcoded (not configurable) — this is a single-purpose guard, not a user-facing feature.

### Step 4: Synchronize no-diff guard in webhook handler

**File:** `crates/mika-gateway/src/github.rs`

Insert between steps 9b (skill denylist guard) and 10 (semaphore):

```rust
// 9c. Synchronize no-diff guard (#886): suppress mika-qa dispatch
// for no-op pushes (trailer-only amend, commit-message-only change).
// Uses GitHub Compare API to check file-level differences between
// the before and after commit SHAs. Fail-open on any error.
if event_type == "pull_request"
    && event.action.as_deref() == Some("synchronize")
{
    if let (Some(before), Some(after), Some(github_app)) = (
        event.before.as_deref(),
        event.after.as_deref(),
        state.github_app.as_ref(),
    ) {
        let repo = event.repository.as_ref()
            .and_then(|r| r.full_name.as_deref())
            .unwrap_or("");
        match commits_have_file_changes(github_app, repo, before, after).await {
            Ok(false) => {
                info!(
                    event_type,
                    delivery_id = %delivery_id,
                    before,
                    after,
                    repo,
                    "webhook_synchronize_no_diff_change: suppressing qa-review dispatch for no-op push"
                );
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
    // No before/after SHAs or no github_app — proceed with dispatch (fail-open).
}
```

### Step 5: Tests

**Unit tests (`crates/mika-gateway/src/github.rs`):**

1. `test_synchronize_before_after_deserialization` — Parse a `pull_request.synchronize` webhook payload with `before`/`after` fields; verify both are `Some`.
2. `test_synchronize_before_after_absent_for_opened` — Parse a `pull_request.opened` webhook payload; verify `before`/`after` are `None`.
3. `test_compare_response_empty_files` — Deserialize `{"files": []}` into `CompareResponse`; verify `files.is_empty()`.
4. `test_compare_response_with_files` — Deserialize `{"files": [{"filename": "foo.rs"}]}` into `CompareResponse`; verify `!files.is_empty()`.
5. `test_compare_response_missing_files_field` — Deserialize `{}` into `CompareResponse`; verify `files.is_empty()` (via `#[serde(default)]`).

**Integration/E2E tests:**
6. `test_synchronize_no_diff_suppressed` — Mock GitHub Compare API to return `{"files": []}`. Send a synchronize webhook. Verify: 200 OK returned, no forwarding to agent, `webhook_synchronize_no_diff_change` log emitted.
7. `test_synchronize_with_diff_dispatched` — Mock Compare API to return `{"files": [{"filename": "src/main.rs"}]}`. Send a synchronize webhook. Verify: event forwarded to agent.
8. `test_synchronize_api_error_fail_open` — Mock Compare API to return HTTP 500. Send a synchronize webhook. Verify: event forwarded (fail-open), `synchronize_no_diff_check failed` warning logged.
9. `test_synchronize_api_timeout_fail_open` — Mock Compare API to hang beyond 5s. Verify: event forwarded (fail-open).
10. `test_synchronize_token_refresh_failure_fail_open` — `github_app.installation_token()` returns `Err`. Verify: event forwarded (fail-open).
11. `test_synchronize_no_github_app_passes_through` — `AppState.github_app` is `None`. Send a synchronize webhook. Verify: event forwarded (graceful degradation).

### Step 6: Structured logging

The `webhook_synchronize_no_diff_change` event name matches the acceptance criteria. Fields:
- `event_type`: `"pull_request"`
- `delivery_id`: GitHub delivery UUID
- `before`: old head SHA
- `after`: new head SHA
- `repo`: repository full name

## Failure modes

| Scenario | Behavior | Rationale |
|----------|----------|-----------|
| GitHub API timeout (>5s) | Fail-open (dispatch proceeds) | Don't block legitimate reviews for slow API |
| GitHub API HTTP error (4xx/5xx) | Fail-open | Transient errors shouldn't suppress reviews |
| `before`/`after` absent in payload | Pass through | Defensive; these fields are documented but not guaranteed |
| GitHub App not configured | Pass through | `AppState.github_app` is `None`; all synchronize events dispatch normally |
| Token refresh failure | Fail-open | `installation_token()` error propagates to `commits_have_file_changes()` → `Err` → fail-open |
| Compare API response unexpected format | Fail-open | `serde_json` parse failure treated as error |
| Rate limited by GitHub API (429) | Fail-open | Same as API error path |

## Out of scope

- **PR-scope dedup table (Surface 3):** More robust for cross-session duplicates with genuine diff changes (rapid pushes). Follow-up ticket to be filed if this class is observed after this fix ships.
- **Agent-level dedup (Surface 2):** Redundant with gateway suppression for the no-op-push case.
- **Re-review on genuine diff changes:** Already correct — the compare API returns non-empty `files`, so dispatch proceeds.
- **Rebases against updated base branch:** Compare API between `before` and `after` includes upstream changes incorporated during rebase, so `files` is non-empty. QA re-review fires correctly.

## Verification

After deploy:
1. Open a PR that triggers mika-qa review (one webhook, one review posted).
2. Force-push a trailer-only amend (e.g., add `Pipeline-Exempt: true` trailer).
3. Observe: `webhook_synchronize_no_diff_change` log event, no second qa-review session, no duplicate review.
4. Force-push a genuine code change.
5. Observe: no suppression log, new qa-review session, updated review posted.

```bash
# Verify suppression is working:
grep webhook_synchronize_no_diff_change server.log | jq '{delivery_id, before, after, repo}'
```

## Related

- mika#695 — Original within-session duplicate review (CLOSED)
- mika#821 / mika#822 — Session-scope DashMap fix (MERGED)
- `docs/solutions/runtime-errors/mika-qa-duplicate-pr-review-required-tools-gate-2026-04-26.md` — Compound doc naming this gap
- PR #885 — Canonical reproduction
