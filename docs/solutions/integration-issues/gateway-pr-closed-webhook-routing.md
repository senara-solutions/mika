---
title: Route pull_request.closed webhook to mika-dev
category: integration-issues
date: 2026-04-13
tags: [gateway, webhook, github, auto-merge, work-item-lifecycle]
---

# Gateway drops pull_request.closed — tasks stuck in_progress

## Problem

When mika-qa passes a PR and `pr_merge_with_gate` enables auto-merge, GitHub eventually closes the PR (merged: true) after CI passes. The gateway's `route_event()` had no arm for `pull_request.closed`, so the event was silently dropped. mika-dev never learned the PR merged and the task stayed `in_progress` indefinitely.

## Root Cause

`route_event()` in `crates/mika-gateway/src/github.rs` only matched `opened | synchronize` for `pull_request` events — the `closed` action fell through to the `_ => None` catch-all.

## Solution

1. Added `("pull_request", Some("closed")) => Some("mika-dev")` to `route_event()`
2. Added `merged: Option<bool>` to `GitHubPullRequest` struct (matches GitHub's payload)
3. In `format_event_text()`, appended `Merged: true/false` line when action is `"closed"` so the downstream skill can distinguish merged vs. closed-without-merge

## Prevention

When adding new webhook event handling in the autonomous loop, trace the full lifecycle: event source (GitHub) -> gateway routing -> skill activation -> task state transition. Any gap in this chain means state gets stuck.
