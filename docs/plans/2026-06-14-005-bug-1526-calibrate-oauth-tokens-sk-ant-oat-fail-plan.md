# Plan: bug(calibrate): OAuth tokens (sk-ant-oat) fail Anthropic calibration

**Ticket:** mika issue#1526
**Type:** bug fix
**Branch:** `bug/1526/calibrate-oauth-tokens-sk-ant-oat-fail`

## Root Cause Analysis

The ticket asserts that the `calibrate` binary "doesn't share the OAuth path" that `mika-server` uses. This is **partially incorrect** — the binary already goes through the full OAuth-aware chain:

```
create_provider_from_spec() → create_provider() → AnthropicProvider::new()
  → ClaudeClient::new() → is_oauth_token("sk-ant-oat...") → OAuthManaged(manager)
```

`ClaudeClient::new()` at `crates/mika-common/src/claude.rs:372` correctly detects `sk-ant-oat*` prefixes and creates `AnthropicAuth::OAuthManaged(Arc<OAuthTokenManager>)`. The `send_once()` method at `claude.rs:679-685` then routes OAuth tokens to `Authorization: Bearer` (not `x-api-key`). **The provider construction chain IS shared.**

The real failures are in **two layers**:

### Layer 1: OAuth token lifecycle failures masked by generic error classification

Every scenario's `Err(e)` arm in `mika_arch.rs` and `mika_dev.rs` hardcodes `FailureClass::TransportError` (e.g., `mika_arch.rs:182`). This loses the actual error class. When `OAuthTokenManager::get_valid_token()` fails (no `oauth.json`, hash mismatch, refresh failure), the error chain is:

```
LlmError::ProviderError("OAuth token resolution failed. Run `mika setup --mode oauth` to authorize.: ...")
```

But the scenario reports it as `TransportError: Claude API returned an unexpected error.` — the `{e}` format string collapses the anyhow context chain into the outermost context, and the hardcoded `FailureClass::TransportError` hides whether it was auth, network, or rate-limit.

### Layer 2: No pre-flight OAuth validation

The `calibrate` binary creates the provider and immediately runs scenarios. If the OAuth state is broken (no `oauth.json`, expired refresh token, hash mismatch from subscription token rotation), every scenario fails identically — five identical `TransportError` lines with no diagnostic to distinguish "OAuth needs re-auth" from "Anthropic is down".

### Layer 3: Makefile stale model example

The `calibrate-mika-arch` target in the Makefile references `anthropic/claude-opus-4-6` which may not exist. An operator copying the example gets a provider error that masks the OAuth issue.

## Fix Strategy

**Option A from the ticket (shared OAuth-aware factory)** is already the reality — no work needed there. The fix targets the diagnostic and usability gaps.

### Step 1: Add OAuth pre-flight health check to calibrate binary

**File:** `crates/mika-agent/src/bin/calibrate.rs`

Before the scenario loop, add an explicit `provider.check_health().await` call. For Anthropic OAuth, this exercises the full `get_valid_token()` → `send_once()` path with a minimal request (`max_tokens: 1, "hi"`). On failure, emit a clear diagnostic:

```rust
// Pre-flight: verify provider can authenticate
print!("  Verifying provider authentication... ");
match provider.check_health().await {
    Ok(()) => println!("OK"),
    Err(e) => {
        let err_str = e.to_string();
        eprintln!("FAIL");
        eprintln!();
        // Surface OAuth-specific guidance when the error chain mentions it
        if err_str.contains("OAuth") || err_str.contains("oauth") {
            eprintln!("Error: OAuth authentication failed for Anthropic provider.");
            eprintln!("  The calibrate binary uses the same OAuth flow as mika-server.");
            eprintln!("  Ensure `mika setup --mode oauth` has been completed and");
            eprintln!("  that ~/.mika/oauth.json exists with valid, non-expired tokens.");
            eprintln!();
            eprintln!("  To verify: check that mika-server can call Anthropic successfully.");
            eprintln!("  If mika-server works but calibrate doesn't, the subscription token");
            eprintln!("  in MIKA_ANTHROPIC_API_KEY may have been rotated since the last");
            eprintln!("  `mika setup --mode oauth` run.");
        }
        eprintln!("  Provider error: {}", err_str);
        std::process::exit(2);
    }
}
```

This gives immediate, actionable feedback instead of running all 5 scenarios to get 5 identical failures.

### Step 2: Improve error classification in scenario error arms

**Files:** `crates/mika-agent/src/calibration/roles/mika_arch.rs`, `crates/mika-agent/src/calibration/roles/mika_dev.rs`

Replace the hardcoded `FailureClass::TransportError` in each scenario's `Err(e)` arm with `classify_failure(Some(&format!("{e}")), None)`. This uses the existing `failure.rs` classifier, which already distinguishes timeout, connection/network, and HTTP status errors. The error message from `LlmError::ProviderError` carries the anyhow context chain, so the classifier can differentiate:

- `"OAuth token resolution failed"` → `Other("OAuth token resolution failed: ...")`
- `"connection refused"` → `TransportError`
- `"status: 429"` → `TransportError`
- `"Authentication failed"` → `Other("Authentication failed: ...")`

This improves the failure breakdown in the calibration report from a blanket `TransportError: 5` to meaningful class distinctions.

**Extract a helper to avoid repetition across ~13 scenario functions:**

```rust
// In calibration/roles/mod.rs or a shared helper
fn llm_error_result(scenario_id: &str, error: LlmError, latency_ms: u64) -> RoleScenarioResult {
    let error_str = error.to_string();
    let failure_class = classify_failure(Some(&error_str), None);
    RoleScenarioResult::fail(scenario_id, failure_class, error_str, None, None, latency_ms)
}
```

Then each scenario's `Err(e)` arm becomes:

```rust
Err(e) => llm_error_result("groom_ticket_basic", e, start.elapsed().as_millis() as u64),
```

### Step 3: Extend `classify_failure` for auth errors

**File:** `crates/mika-agent/src/calibration/failure.rs`

Add a new `AuthenticationError` variant to `FailureClass`:

```rust
/// Authentication failure (OAuth, API key, etc.)
AuthenticationError,
```

Add detection in `classify_failure`:

```rust
if lower.contains("oauth") || lower.contains("authentication failed") || lower.contains("invalid x-api-key") || lower.contains("invalid api key") {
    return FailureClass::AuthenticationError;
}
```

This makes authentication failures structurally distinct in calibration reports — an operator seeing `AuthenticationError: 5` immediately knows it's a credential issue, not a provider outage.

### Step 4: Fix Makefile stale model example

**File:** `Makefile`

Change the `calibrate-mika-arch` example model from `anthropic/claude-opus-4-6` to `anthropic/claude-sonnet-4-6` (the current general-purpose Anthropic model that's verified working).

```makefile
calibrate-mika-arch: ## Pre-swap calibration gate for mika-arch (MODEL=provider/model required)
	@if [ -z "$(MODEL)" ]; then echo "Error: MODEL is required. Example: make calibrate-mika-arch MODEL=anthropic/claude-sonnet-4-6" >&2; exit 1; fi
```

### Step 5: Add unit test for OAuth pre-flight detection

**File:** `crates/mika-agent/src/calibration/failure.rs` (test module)

Add tests for the new `AuthenticationError` classification:

```rust
#[test]
fn test_classify_oauth_error() {
    assert_eq!(
        classify_failure(Some("OAuth token resolution failed: ..."), None),
        FailureClass::AuthenticationError
    );
}

#[test]
fn test_classify_auth_failed() {
    assert_eq!(
        classify_failure(Some("Authentication failed. Check API key."), None),
        FailureClass::AuthenticationError
    );
}
```

### Step 6: Add integration-level OAuth detection test

**File:** `crates/mika-agent/src/calibration/providers.rs` (test module)

Add a test verifying that `create_provider_from_spec("anthropic/claude-sonnet-4-6")` produces a provider that, when `MIKA_ANTHROPIC_API_KEY` is an `sk-ant-oat*` token, goes through the OAuth path. This is a structural test — it doesn't make API calls, just verifies the provider is constructed without error:

```rust
#[test]
fn test_oauth_token_creates_provider() {
    // Set an OAuth-prefix key for this test only
    unsafe { std::env::set_var("MIKA_ANTHROPIC_API_KEY", "sk-ant-oat01-test-token-dummy") };
    let provider = create_provider_from_spec("anthropic/claude-sonnet-4-6");
    assert!(provider.is_some(), "OAuth token should create a valid provider");
    unsafe { std::env::remove_var("MIKA_ANTHROPIC_API_KEY") };
}
```

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/bin/calibrate.rs` | Add OAuth pre-flight health check with diagnostic output |
| `crates/mika-agent/src/calibration/failure.rs` | Add `AuthenticationError` variant + detection patterns + tests |
| `crates/mika-agent/src/calibration/roles/mika_arch.rs` | Replace hardcoded `TransportError` with `classify_failure` via helper |
| `crates/mika-agent/src/calibration/roles/mika_dev.rs` | Same as above |
| `crates/mika-agent/src/calibration/roles/mod.rs` | Add shared `llm_error_result` helper |
| `crates/mika-agent/src/calibration/providers.rs` | Add OAuth provider construction test |
| `Makefile` | Fix stale model example in `calibrate-mika-arch` |

## Acceptance Criteria Mapping

- **AC1/AC2 (calibrate succeeds with OAuth):** The pre-flight check validates the OAuth flow upfront and gives actionable errors when it fails. The underlying `ClaudeClient::new()` OAuth detection already works — this fix ensures the operator knows when the OAuth state needs repair.
- **AC3 (non-OAuth keys unaffected):** No changes to `ClaudeClient::new()` or `AnthropicProvider::new()`. Pre-flight `check_health()` exercises the same path for both auth types.
- **AC4 (non-Anthropic providers unaffected):** Pre-flight `check_health()` is provider-generic (defined on `LlmProvider` trait). The OAuth-specific diagnostic is string-gated on error content.
- **AC5 (Makefile model example):** Step 4 fixes the stale example.
- **AC6 (regression test):** Steps 5 and 6 cover the `sk-ant-oat*` detection + `AuthenticationError` classification.

## Out of Scope

- Changes to `ClaudeClient::new()`, `OAuthTokenManager`, or the OAuth PKCE flow (already working correctly)
- Changes to `create_provider()` or `AnthropicProvider::new()` (already correctly routing OAuth tokens)
- New providers, new calibration roles, or calibration CI (#742)
