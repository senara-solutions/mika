---
title: "User-friendly Telegram message when agent container is offline"
category: ux-improvements
date: 2026-03-29
tags: [gateway, telegram, error-handling, reqwest, user-experience]
issue: "#309"
---

# User-friendly Telegram message when agent container is offline

## Problem

When a Mika agent container was unreachable (scaled to zero, deprovisioned, or DNS not resolving), the gateway sent the same generic message used for all errors:

> "I'm having trouble right now. Please try again in a moment."

Users had no way to distinguish an offline agent from a transient glitch, leading to repeated futile retries and no signal to contact their administrator.

## Root Cause

`handle_forward_result()` in `crates/mika-gateway/src/routes.rs` treated all `reqwest::Error` variants identically in the `Err(e)` branch, calling `reply_transient_error()` regardless of whether the failure was a connection error (agent down) or a transient network issue (timeout, broken pipe).

## Solution

Added a pure classifier function `forward_error_message(is_connect: bool) -> &'static str` that returns:

- **Connect errors** (connection refused, DNS failure): `"Your Mika assistant is currently offline. Please contact your administrator or check your subscription status at console.getmika.ai."`
- **Other errors** (timeout, broken pipe): `"I'm having trouble right now. Please try again in a moment."`

Modified the `Err(e)` branch to use `reqwest::Error::is_connect()` for classification, which returns `true` for both connection refused AND DNS resolution failure.

Key design decisions:
- Function takes `bool` (not `&reqwest::Error`) to enable pure sync unit testing, consistent with existing gateway test patterns (no `#[tokio::test]` in this crate)
- `reply_transient_error()` left unchanged -- still correct for DB errors, pairing errors, and the `Ok(resp)` error branch (container running but returning 4xx/5xx)
- `is_connect` added as a structured field in the `warn!` log for observability filtering

## Prevention

- When adding user-facing error messages, always classify errors into actionable categories rather than using a single generic message
- Follow the Security Hardening Playbook Pattern #2: return opaque messages to users, log full error details server-side (never expose internal hostnames, status codes, or error strings)
- Test error classification with pure functions that take primitive inputs for easy sync testing
