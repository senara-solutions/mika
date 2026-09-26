---
status: complete
priority: p2
issue_id: "030"
tags: [code-review, architecture, rust-v2]
dependencies: []
---

# Retry Detection via String Matching + Verbatim Error Forwarding

## Problem Statement

Two related issues in the Claude API client:
1. `is_retryable()` checks for "429", "500", "529" as substrings in the error message — fragile and can false-positive
2. Raw API error bodies are forwarded verbatim via `anyhow::bail!`, potentially leaking internal details or user data fragments to logs/CLI

**Why it matters:** Unreliable retry behavior and information disclosure.

## Findings

- **Source:** Security Sentinel (H2, H4), Architecture Strategist (D), Performance Oracle (OPT-3)
- **Location:** `crates/mika-common/src/claude.rs:204-224`

## Proposed Solutions

### Option A: Typed ClaudeApiError enum (Recommended)
- Create `ClaudeApiError` with `HttpError { status: u16, message: String }` and `Transport(reqwest::Error)`
- Match on status code directly in `is_retryable`
- Log full error server-side, return sanitized message to callers
- **Pros:** Correct retry logic, no information leakage
- **Cons:** Slightly more code
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] Retry logic matches on HTTP status codes, not strings
- [ ] Error messages to callers do not contain raw API response bodies
- [ ] 429/500/529 are retried; other errors are not
- [ ] reqwest timeouts are correctly identified as retryable
