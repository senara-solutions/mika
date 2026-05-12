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

## Implementation

### Step 1: Parse `before`/`after` from webhook payload

**File:** `crates/mika-gateway/src/github.rs`

Add two fields to `GitHubWebhookEvent`:

```rust
/// Commit SHA before the push (present on pull_request.synchronize events).
pub before: Option<String>,
/// Commit SHA after the push (present on pull_request.synchronize events).
pub after: Option<String>,
```

These are top-level fields in GitHub's `pull_request.synchronize` webhook payload. Serde will deserialize them when present and leave `None` for other event types.

### Step 2: Add GitHub API client capability to gateway

**File:** `crates/mika-gateway/src/github.rs` (or new `crates/mika-gateway/src/github_api.rs`)

The gateway already has:
- `reqwest` dependency (used for forwarding to agent containers)
- Access to `mika-common::github_app` (GitHub App JWT generation and installation token exchange)
- `MIKA_GITHUB_APP_ID`, `MIKA_GITHUB_APP_PRIVATE_KEY`, `MIKA_GITHUB_APP_INSTALLATION_ID` env vars

Add a minimal GitHub API helper function:

```rust
/// Compare two commits and return whether they have file changes.
/// Returns `Ok(true)` if files differ, `Ok(false)` if identical trees.
/// Returns `Err` on API failure (caller should fail-open).
async fn commits_have_file_changes(
    client: &reqwest::Client,
    github_token: &str,
    repo_full_name: &str,
    before: &str,
    after: &str,
) -> Result<bool, anyhow::Error>
```

Implementation:
- `GET https://api.github.com/repos/{repo}/compare/{before}...{after}`
- Parse response; check if `files` array is empty
- Use `Accept: application/vnd.github.v3+json` header
- Auth via `Authorization: token {installation_token}`
- 5-second timeout (this is in the webhook hot path)

**Token management:** Wire up `mika-common::github_app::exchange_installation_token()` at gateway startup. Cache the installation token in `GatewayState` with TTL refresh (installation tokens last 1 hour). The gateway already holds an `Arc<GatewayState>` passed to all handlers. Add an `Option<Arc<GitHubApiClient>>` field — `None` when GitHub App credentials are not configured (fail-open: all synchronize events pass through).

### Step 3: Synchronize no-diff guard in webhook handler

**File:** `crates/mika-gateway/src/github.rs`

Insert the guard between step 9 (routing) and step 10 (semaphore) in `github_webhook_handler`:

```rust
// 9c. Synchronize no-diff guard (#886): suppress mika-qa dispatch
// for no-op pushes (trailer-only amend, commit-message-only change).
if event_type == "pull_request"
    && event.action.as_deref() == Some("synchronize")
{
    if let (Some(before), Some(after), Some(api_client)) = (
        event.before.as_deref(),
        event.after.as_deref(),
        state.github_api_client.as_ref(),
    ) {
        let repo = event.repository.as_ref()
            .and_then(|r| r.full_name.as_deref())
            .unwrap_or("");
        match api_client.commits_have_file_changes(repo, before, after).await {
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
    // No before/after SHAs or no API client — proceed with dispatch.
}
```

**Position rationale:** Before the semaphore (step 10) so suppressed events don't consume a semaphore permit. After the skill denylist guard (step 9b) to maintain the existing defense-in-depth ordering.

### Step 4: Tests

**Gateway unit tests (`crates/mika-gateway/src/github.rs`):**

1. `test_synchronize_before_after_deserialization` — Verify `before`/`after` fields parse correctly from a synchronize webhook payload.
2. `test_synchronize_before_after_absent_for_other_actions` — Verify `before`/`after` are `None` for `opened`/`closed` events.
3. `test_format_event_text_synchronize_includes_shas` — (Only if we include SHAs in formatted text; may not be needed since the gateway handles suppression.)

**Gateway integration tests:**
4. `test_synchronize_no_diff_suppressed` — Mock the compare API to return zero files; verify the handler returns 200 without forwarding.
5. `test_synchronize_with_diff_dispatched` — Mock the compare API to return files; verify the handler proceeds with forwarding.
6. `test_synchronize_api_failure_fail_open` — Mock the compare API to return an error; verify the handler proceeds with forwarding (fail-open).
7. `test_synchronize_no_api_client_passes_through` — When `github_api_client` is `None`, all synchronize events pass through (graceful degradation).

### Step 5: Structured logging

The `webhook_synchronize_no_diff_change` event name matches the acceptance criteria. Fields:
- `event_type`: `"pull_request"`
- `delivery_id`: GitHub delivery UUID
- `before`: old head SHA
- `after`: new head SHA
- `repo`: repository full name
- `pr_number`: PR number (if extractable from event)

## Failure modes

| Scenario | Behavior | Rationale |
|----------|----------|-----------|
| GitHub API timeout/error | Fail-open (dispatch proceeds) | Don't block legitimate reviews for transient failures |
| `before`/`after` absent in payload | Pass through | Defensive; GitHub documents these fields but we don't trust |
| GitHub App not configured | Pass through | Gateway without App credentials still routes webhooks normally |
| Rate limited by GitHub API | Fail-open | Same as API error; the compare call is lightweight (one per synchronize event) |
| Compare API returns unexpected format | Fail-open | Parse failure treated as error |

## Out of scope

- **PR-scope dedup table (Surface 3):** More robust but higher complexity. Could be a follow-up if gateway suppression proves insufficient for edge cases.
- **Agent-level dedup (Surface 2):** Redundant with gateway suppression.
- **Re-review on genuine diff changes:** Already correct behavior — the compare API returns non-empty `files` for real changes, so dispatch proceeds.
- **Rebases against updated base branch:** Compare API between `before` and `after` shows incorporated upstream changes, so `files` will be non-empty. QA re-review fires correctly.

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
