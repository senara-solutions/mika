# Groom Ticket: HTTP Client Retry Logic

## Ticket: mika#1230 — Add retry logic to HTTP client with exponential backoff

### Description

The `reqwest` HTTP client in `mika-common` currently fails immediately on transient
errors (5xx, timeouts, connection resets). Add retry logic with exponential backoff.

### Proposed Solution

- Create `RetryPolicy` struct: `max_retries: u32`, `base_delay: Duration`, `max_delay: Duration`
- Wrap retry logic in a `retry_request()` async helper
- Retry on: HTTP 429, 500, 502, 503, 504, connection errors, timeouts
- Do NOT retry on: 4xx (except 429), request body errors
- Backoff formula: `min(base_delay * 2^attempt, max_delay)` + jitter (±25%)
- Default: 3 retries, 500ms base, 10s max

### Acceptance Criteria

- [ ] Transient failures are retried up to `max_retries` times
- [ ] Backoff is exponential with jitter
- [ ] Non-retryable errors fail immediately
- [ ] Total retry time is bounded (no infinite loops)
- [ ] Caller can override policy per-request
- [ ] Existing tests pass without modification

### Location

`crates/mika-common/src/http/retry.rs` (new module)
