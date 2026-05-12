# Plan: Classify Anthropic HTTP 400 Billing Errors as Typed Non-Retriable Variant

- **Issue:** mika issue#1088
- **Type:** fix
- **Branch:** `fix/1088/llm-classify-anthropic-http-400-billing`
- **Date:** 2026-05-12

## Problem

Anthropic billing rejections (HTTP 400 + "Your credit balance is too low") are classified as generic `HttpError`, retried through the full backoff schedule, and surfaced with a generic user-facing message. The operator-visible error chain drops the actionable billing message entirely.

**Incident 2026-05-12:** 1086 billing-rejection WARN entries over ~4 hours; indistinguishable from a transient outage without a log dive.

## Scope

**Anthropic provider only.** No cross-provider generalization in this PR. When a second non-Anthropic billing incident occurs, the second variant will reveal the abstraction shape.

## Implementation Steps

### Step 1: Extend `ApiErrorDetail` with `error_type` field

**File:** `crates/mika-common/src/claude.rs` (lines 273-276)

Add an `error_type` field to `ApiErrorDetail` to capture the Anthropic `error.type` value:

```rust
#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    #[serde(rename = "type")]
    error_type: Option<String>,
    message: String,
}
```

`Option<String>` because the field may be absent in non-standard error responses. Using `#[serde(rename = "type")]` since `type` is a Rust keyword.

### Step 2: Add `BillingError` variant to `ClaudeApiError`

**File:** `crates/mika-common/src/claude.rs` (lines 280-288)

Add a new variant after `HttpError`:

```rust
#[derive(Error, Debug)]
pub enum ClaudeApiError {
    #[error("Claude API HTTP error ({status}): {message}")]
    HttpError { status: u16, message: String },
    #[error("Anthropic billing error: {message}")]
    BillingError { message: String },
    #[error("Claude API request failed")]
    Transport(#[from] reqwest::Error),
    #[error("Claude API response parse error")]
    ParseError(#[source] reqwest::Error),
}
```

### Step 3: Add billing message prefix constant

**File:** `crates/mika-common/src/claude.rs` (top of module, near other constants)

```rust
/// Prefix of the Anthropic error message when the account has insufficient credits.
/// Pinned as a const because `invalid_request_error` is Anthropic's generic 4xx type
/// (also covers malformed requests, bad model names, oversize payloads) — the substring
/// is the actual discriminator.
const ANTHROPIC_BILLING_MESSAGE_PREFIX: &str = "Your credit balance is too low";
```

### Step 4: Classify billing errors in `send_once` response parsing

**File:** `crates/mika-common/src/claude.rs` (lines 706-721)

After parsing the error body, check the conjunction of HTTP 400 + `error.type == "invalid_request_error"` + message prefix match:

```rust
let status_code = status.as_u16();
let body = response.text().await.unwrap_or_default();
let parsed = serde_json::from_str::<ApiErrorResponse>(&body);
let message = parsed.as_ref()
    .map(|e| e.error.message.clone())
    .unwrap_or_else(|_| {
        let truncated: String = body.chars().take(200).collect();
        format!("unexpected error response (HTTP {status_code}): {truncated}")
    });

// Billing classification: HTTP 400 + invalid_request_error + billing prefix
if status_code == 400 {
    if let Ok(ref parsed_err) = parsed {
        if parsed_err.error.error_type.as_deref() == Some("invalid_request_error")
            && message.starts_with(ANTHROPIC_BILLING_MESSAGE_PREFIX)
        {
            error!(error_message = %message, "Anthropic billing error — non-retriable");
            return Err(ClaudeApiError::BillingError { message });
        }
    }
}

warn!(status = status_code, error_message = %message, "Claude API error response");
return Err(ClaudeApiError::HttpError { status: status_code, message });
```

Key decisions:
- Parse `ApiErrorResponse` once, reuse for both billing check and message extraction.
- Billing path logs at `error!` (not `warn!`) — distinct from the generic 4xx path.
- Non-billing 400s fall through to the existing `HttpError` path unchanged.

### Step 5: Make `BillingError` non-retriable

**File:** `crates/mika-common/src/claude.rs` (lines 736-742)

`is_retryable` already returns `false` for non-matching variants. Since `BillingError` is a new variant, it will fall into the `_ => false` arm. No code change needed — but the retry loop at lines 562-567 must also handle the early-return case.

The retry loop structure (line 563: `if attempt < MAX_RETRIES && is_retryable(&e)`) already handles this correctly — `is_retryable` returns `false` for `BillingError`, so the loop falls through to the final error mapping at line 583.

### Step 6: Add billing error context in final error mapping

**File:** `crates/mika-common/src/claude.rs` (lines 583-607)

Add a match arm for `BillingError` before the catch-all `HttpError` arm:

```rust
ClaudeApiError::BillingError { .. } => {
    anyhow::Error::from(e).context(format!(
        "Anthropic API rejected the request: {}. \
         Top up the account at https://console.anthropic.com/settings/billing.",
        match &e {
            ClaudeApiError::BillingError { message } => message.as_str(),
            _ => unreachable!(),
        }
    ))
}
```

The verbatim message from Anthropic survives in the error chain, so the exact wording is preserved even if Anthropic changes it in the future.

### Step 7: Handle `BillingError` in post-retry-exhaustion path

**File:** `crates/mika-common/src/claude.rs` (lines 618-643)

The `last_error` mapping after the retry loop should also handle `BillingError`. However, since `BillingError` is non-retriable, it will never reach the post-loop path — it's returned immediately at line 583. No change needed.

### Step 8: Confirm `AGENT_ERROR_REPLY` is unchanged

**File:** `crates/mika-agent/src/server/handlers.rs` (lines 719-720, 913-915)

No changes. The billing classification is purely operator-visible (logs + error chain). End users (Telegram) still see the generic fallback — they can't action billing.

## Tests

All in `crates/mika-common/src/claude.rs`:

1. **`classifies_400_billing_response_as_typed_variant`** — Mock a 400 response with `{"error": {"type": "invalid_request_error", "message": "Your credit balance is too low to access..."}}`. Assert `BillingError { message }` is constructed.

2. **`does_not_classify_400_non_billing_as_billing`** — Mock a 400 response with `{"error": {"type": "invalid_request_error", "message": "Invalid model name 'foo'"}}`. Assert `HttpError` is constructed (not `BillingError`).

3. **`retry_loop_returns_immediately_on_billing_error`** — Use a mock HTTP client returning billing 400 on first attempt. Assert exactly 1 attempt (no retries).

4. **`final_error_chain_contains_actionable_billing_message`** — Assert the operator-visible error chain contains the verbatim billing message AND the `console.anthropic.com/settings/billing` URL.

## Verification Checklist

- [ ] `BillingError { message: String }` variant exists on `ClaudeApiError`
- [ ] `const ANTHROPIC_BILLING_MESSAGE_PREFIX` pinned at module top
- [ ] `ApiErrorDetail` captures `error.type` via `#[serde(rename = "type")]`
- [ ] Billing classification requires conjunction: HTTP 400 + `invalid_request_error` + prefix match
- [ ] Billing path logs at `error!` level (distinct from generic `warn!`)
- [ ] Retry loop returns immediately on `BillingError` (no backoff entry)
- [ ] Operator-visible error chain contains verbatim message + billing URL
- [ ] `AGENT_ERROR_REPLY` at `handlers.rs:719-720` is unchanged
- [ ] Four unit tests pass
- [ ] No changes to non-Anthropic providers

## Risks

- **Brittle detection:** The substring match is intentionally brittle — pinned as a const, tested explicitly. If Anthropic changes the message wording, the classification silently falls back to `HttpError` (safe default). The const makes the substring grep-discoverable for future updates.
- **No second 4xx class:** The plan intentionally avoids a `PermanentClientError { kind, message }` grouping. When a second permanent-4xx class arrives (bad model, content policy, etc.), refactor then — two data points beats one.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/claude.rs` | Add `BillingError` variant, `ANTHROPIC_BILLING_MESSAGE_PREFIX` const, extend `ApiErrorDetail`, classify billing in `send_once`, add billing context in error mapping, 4 unit tests |
