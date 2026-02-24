---
status: ready
priority: p3
issue_id: "153"
tags: [plan-review, performance, security]
dependencies: []
---

# Miscellaneous gateway optimizations and hardening

## Problem Statement
Several small findings from the review don't warrant individual todos but should be tracked together: workspace dependency management, security headers, credential redaction, timeout tuning, client reuse, and rate limiting.

**Why it matters:** Each item is small but collectively they improve security, performance, and maintainability.

## Findings
- Source: Various agents (Architecture Strategist, Security Sentinel, Performance Oracle)
- Items listed below with source agent

## Individual Items

### 1. Security headers (Security Sentinel M-4)
Add `tower-http` security headers: X-Content-Type-Options, X-Frame-Options, Strict-Transport-Security.

### 2. DATABASE_URL redaction (Security Sentinel M-3)
Ensure DATABASE_URL (contains credentials) is redacted in Debug impl, matching existing Settings pattern.

### 3. Container forwarding timeout (Performance Oracle)
Reduce container forwarding timeout from 5s to 2s — containers should respond quickly to /message.

### 4. reqwest::Client reuse (Performance Oracle)
Share a single reqwest::Client in AppState for both container forwarding and Telegram API calls (connection pooling).

### 5. Telegram API rate limiter (Performance Oracle)
Consider adding a global rate limiter (governor crate, 25 req/sec) for Telegram API calls to avoid 429s.

### 6. Workspace dependency management (Architecture Strategist)
Add sqlx, uuid to workspace Cargo.toml dependencies for version consistency.

### 7. allowed_updates minimality (Security Sentinel L-5)
Set `allowed_updates: ["message"]` in setWebhook to only receive message updates (not channel_post, edited_message, etc.).

## Acceptance Criteria
- [ ] Security headers added to all responses
- [ ] DATABASE_URL redacted in logs/debug output
- [ ] Container forwarding timeout ≤ 2s
- [ ] Single shared reqwest::Client in AppState
- [ ] Telegram rate limiting considered
- [ ] Workspace deps consistent
- [ ] allowed_updates set to minimal required set

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Consolidated small findings from multiple agents
