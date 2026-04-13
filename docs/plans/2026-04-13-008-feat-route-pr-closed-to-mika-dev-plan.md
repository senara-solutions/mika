---
title: "feat: Route pull_request.closed webhook to mika-dev"
type: feat
status: active
date: 2026-04-13
---

# Route pull_request.closed webhook to mika-dev

## Overview

When mika-qa passes a PR and `pr_merge_with_gate` enables auto-merge, the PR eventually closes via GitHub's auto-merge. Currently `pull_request.closed` events are silently dropped by the gateway — mika-dev never learns the PR merged, so the work item stays `in_progress` forever.

## Acceptance Criteria

- [x] `route_event("pull_request", Some("closed"), None)` returns `Some("mika-dev")`
- [x] `GitHubPullRequest` struct has `merged: Option<bool>` field
- [x] `format_event_text` for `pull_request` closed action includes merged status
- [x] Existing `test_route_event_pr_closed` updated to assert `Some("mika-dev")`
- [x] All existing gateway tests pass (`cargo test -p mika-gateway`)

## MVP

### `crates/mika-gateway/src/github.rs` — route_event()

Add `("pull_request", Some("closed"))` arm routing to `"mika-dev"`:

```rust
("pull_request", Some("opened" | "synchronize")) => Some("mika-qa"),
("pull_request", Some("closed")) => Some("mika-dev"),  // NEW
```

### `crates/mika-gateway/src/github.rs` — GitHubPullRequest struct

Add `merged` field:

```rust
pub struct GitHubPullRequest {
    pub number: Option<u64>,
    pub title: Option<String>,
    pub html_url: Option<String>,
    pub body: Option<String>,
    pub head: Option<GitHubRef>,
    pub merged: Option<bool>,  // NEW — true when PR was merged (vs closed without merge)
}
```

### `crates/mika-gateway/src/github.rs` — format_event_text()

In the `"pull_request"` match arm, append merged status when action is "closed":

```rust
"pull_request" => {
    // ... existing code ...
    let mut text = format!(
        "[GitHub] PR {action}: {repo_name}#{number} — {title} (branch: {branch})\n{url}"
    );
    // NEW: include merged status for closed PRs
    if action == "closed" {
        let merged = pr.and_then(|p| p.merged).unwrap_or(false);
        text.push_str(&format!("\nMerged: {merged}"));
    }
    // ... existing body append ...
}
```

### `crates/mika-gateway/src/github.rs` — test

Update existing test:

```rust
#[test]
fn test_route_event_pr_closed() {
    assert_eq!(
        route_event("pull_request", Some("closed"), None),
        Some("mika-dev")
    );
}
```

## Sources

- Gateway webhook routing: `crates/mika-gateway/src/github.rs:141-157`
- Format function: `crates/mika-gateway/src/github.rs:174-277`
- Existing test: `crates/mika-gateway/src/github.rs:742-744`
