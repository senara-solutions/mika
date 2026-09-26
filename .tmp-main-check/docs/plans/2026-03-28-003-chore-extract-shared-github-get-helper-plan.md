---
title: "chore: extract shared github_get helper in check_work_item"
type: refactor
status: completed
date: 2026-03-28
---

# chore: extract shared github_get helper in check_work_item

## Overview

Extract a shared `github_get(token, url) -> Result<Value, String>` helper from `fetch_github_pr_status` and `fetch_github_issue_status` in `check_work_item.rs`. Both functions share ~15 lines of identical boilerplate for HTTP client construction, request headers, error status mapping, and JSON response parsing.

## Problem Statement

From PR #258 review: `fetch_github_pr_status` (lines 89-118) and `fetch_github_issue_status` (lines 149-178) contain identical code for:

1. `reqwest::Client::builder().timeout(10s).build()` with error mapping
2. `.get(url)` with `Authorization`, `User-Agent`, `Accept` headers
3. `.send().await` with error mapping
4. HTTP status code → human-readable error mapping (401, 403, 404, 429)
5. `.json::<Value>().await` with error mapping

Each caller only differs in the response field extraction after the JSON is parsed.

## Proposed Solution

Extract a single `github_get` helper that handles steps 1-5, returning `Result<Value, String>`. Each caller (`fetch_github_pr_status`, `fetch_github_issue_status`) calls `github_get` and handles only its response-specific field extraction.

## Acceptance Criteria

- [x] New `github_get(token: &str, url: &str) -> Result<Value, String>` function in `check_work_item.rs`
- [x] `fetch_github_pr_status` uses `github_get` for HTTP request, only handles PR-specific fields
- [x] `fetch_github_issue_status` uses `github_get` for HTTP request, only handles issue-specific fields
- [x] All existing tests pass unchanged (`cargo test -p mika-agent -- check_work_item`)
- [x] `cargo clippy` clean
- [x] ~15 LOC net reduction

## MVP

### crates/mika-agent/src/tools/check_work_item.rs

The shared helper:

```rust
/// Perform an authenticated GET request to the GitHub REST API.
///
/// Constructs a client with a 10-second timeout, sets standard headers,
/// maps common HTTP error codes to human-readable messages, and returns
/// the parsed JSON body.
async fn github_get(token: &str, url: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "mika-agent")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "token invalid or expired".to_string(),
            403 => "token lacks required permissions".to_string(),
            404 => "not found or not accessible".to_string(),
            429 => "rate limit exceeded".to_string(),
            _ => format!("HTTP {status}"),
        };
        return Err(msg);
    }

    response
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}
```

Simplified callers:

```rust
async fn fetch_github_pr_status(
    token: &str, owner: &str, repo: &str, number: u64,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let body = github_get(token, &url).await?;

    // PR-specific field extraction
    let state = body["state"].as_str().unwrap_or("unknown");
    let merged = body["merged"].as_bool().unwrap_or(false);
    let draft = body["draft"].as_bool().unwrap_or(false);
    let head_ref = body["head"]["ref"].as_str().unwrap_or("unknown");

    let display_state = if draft {
        "draft".to_string()
    } else if merged {
        "closed (merged)".to_string()
    } else if state == "closed" {
        "closed (not merged)".to_string()
    } else {
        state.to_string()
    };

    Ok(format!("GitHub PR Status:\n  State: {display_state}\n  Branch: {head_ref}"))
}

async fn fetch_github_issue_status(
    token: &str, owner: &str, repo: &str, number: u64,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}");
    let body = github_get(token, &url).await?;

    // Issue-specific field extraction
    let state = body["state"].as_str().unwrap_or("unknown");
    let state_reason = body["state_reason"].as_str();

    let display_state = match state_reason {
        Some("completed") => "closed (completed)".to_string(),
        Some("not_planned") => "closed (not planned)".to_string(),
        _ => state.to_string(),
    };

    Ok(format!("GitHub Issue Status:\n  State: {display_state}"))
}
```

## Sources

- PR #258 review comment identifying the duplication
- Related issue: #260
