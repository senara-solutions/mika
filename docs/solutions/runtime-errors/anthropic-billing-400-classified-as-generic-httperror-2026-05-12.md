---
title: Anthropic HTTP 400 billing errors classified as generic HttpError causing retry storms
date: 2026-05-12
category: runtime-errors
module: mika-common
problem_type: runtime_error
component: tooling
symptoms:
  - "1086 WARN entries over 4 hours from billing-rejected 400 responses being retried"
  - "Operator-visible error chain shows generic 'Claude API returned an unexpected error' — no billing context"
  - "Indistinguishable from transient outage without manual log analysis"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags:
  - anthropic
  - billing
  - error-classification
  - retry
  - non-retriable
  - claude-api
  - http-400
---

# Anthropic HTTP 400 billing errors classified as generic HttpError causing retry storms

## Problem

When Anthropic returns HTTP 400 with `"Your credit balance is too low to access the Anthropic API"`, the response was classified as a generic `HttpError` (catch-all at `crates/mika-common/src/claude.rs`). This caused:
1. Full retry schedule (3 retries with exponential backoff) on every request
2. Generic `AGENT_ERROR_REPLY` surfaced to Telegram — no actionable billing context
3. 1086 WARN entries over ~4 hours before manual diagnosis

## Symptoms

- `/var/log/mika/server.log` accumulated billing-rejection WARN entries at high volume
- Error chain showed `"LLM provider error: Claude API returned an unexpected error. Please try again."` — dropped the billing message entirely
- Multiple agents (mika-dev, mika-qa) sharing the same Anthropic account flooded Telegram with generic errors simultaneously

## What Didn't Work

- The existing `is_retryable()` function correctly excluded HTTP 400 from retryable codes (only 429/500/529 are retried), BUT the `HttpError { status: 400 }` variant still entered the retry loop's `send_once` call and was classified with the generic catch-all context message. The non-retriable exit path still wrapped it with `"Claude API returned an unexpected error. Please try again."` — losing the actionable billing message.

## Solution

Added a typed `BillingError { message: String }` variant to `ClaudeApiError` with a three-part detection conjunction in `send_once`:

```rust
// In send_once response parsing:
if status_code == 400
    && let Ok(ref parsed_err) = parsed
    && parsed_err.error.error_type.as_deref() == Some("invalid_request_error")
    && message.starts_with(ANTHROPIC_BILLING_MESSAGE_PREFIX)
{
    error!(error_message = %message, "Anthropic billing error — non-retriable");
    return Err(ClaudeApiError::BillingError { message });
}
```

Key implementation details:
- `ANTHROPIC_BILLING_MESSAGE_PREFIX` pinned as a module-level const (`"Your credit balance is too low"`)
- `ApiErrorDetail` extended with `#[serde(rename = "type")] error_type: Option<String>` to capture the inner `error.type`
- `BillingError` is non-retriable — falls into `is_retryable()`'s `_ => false` arm automatically
- Billing path logs at `error!` (not `warn!`) for operator visibility
- Error chain includes verbatim message + `https://console.anthropic.com/settings/billing` URL
- `AGENT_ERROR_REPLY` unchanged — end users can't action billing

The `if let` billing check was placed before the `match &e` block in the retry loop to avoid a Rust borrow/move conflict (need to read `message` from the variant AND consume `e` into `anyhow::Error::from()`).

## Why This Works

The conjunction of HTTP 400 + `error.type == "invalid_request_error"` + message prefix is specific enough to catch billing errors without false-positiving on other 400 responses (malformed requests, bad model names, oversize payloads all share `invalid_request_error` as the error type but have different message text). The prefix is intentionally brittle — if Anthropic changes the wording, classification silently falls back to `HttpError { status: 400 }` (safe default, still non-retriable, just no billing URL in the error chain).

## Prevention

- **Scope to Anthropic only.** Do not generalize across providers until a second billing incident reveals the abstraction shape. Two data points beat one.
- **Pin detection strings as constants.** Makes them grep-discoverable when upstream changes their API responses.
- **Use `error!` for permanent errors, `warn!` for transient.** Log level is the first signal an operator sees — permanent errors at `warn!` are invisible in noisy logs.
- **Preserve verbatim upstream messages in error chains.** Don't summarize or rephrase — the exact wording is what operators search for in logs.

## Related Issues

- [mika#1088](https://github.com/senara-solutions/mika/issues/1088) — this fix
- `docs/solutions/integration-issues/gateway-inbound-webhook-retry-on-429-5xx.md` — symmetric pattern on the gateway inbound path (429/5xx transient vs 4xx permanent)
