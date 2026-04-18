---
title: "Extract shared github_get helper from check_task"
category: code-review-patterns
date: 2026-03-28
tags: [refactoring, deduplication, github-api, reqwest]
---

# Extract shared github_get helper from check_task

## Problem

`fetch_github_pr_status` and `fetch_github_issue_status` in `check_task.rs` shared ~15 lines of identical boilerplate: reqwest client construction with 10s timeout, Authorization/User-Agent/Accept headers, HTTP status code error mapping (401/403/404/429), and JSON response parsing.

## Root Cause

The two GitHub API fetch functions were written independently with copy-pasted HTTP plumbing. Each only differed in the URL path and response field extraction.

## Solution

Extracted a shared `github_get(token: &str, url: &str) -> Result<Value, String>` helper that handles client construction, headers, error mapping, and JSON parsing. Each caller now delegates to `github_get` and handles only its response-specific field extraction (~15 LOC reduction).

The 404 error message was generalized from context-specific ("PR not found" / "issue not found") to generic ("not found or not accessible") since callers already provide context in their error wrapping.

## Prevention

When adding new GitHub API endpoints to `check_task.rs`, reuse the `github_get` helper rather than duplicating HTTP boilerplate.
