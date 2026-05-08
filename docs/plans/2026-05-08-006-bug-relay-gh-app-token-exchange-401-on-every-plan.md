---
type: bug
ticket: mika#1042
title: Fix JWT exp clock-skew over-window in GitHub App token exchange
date: 2026-05-08
sequence: 006
---

# Plan — fix GitHub App JWT exp clock-skew over-window

## TL;DR

The host clock is ~70s ahead of GitHub. The JWT generator computes `exp = (now − 60) + 600 = now + 540`, which GitHub sees as `github_now + 610` — past the 600s ceiling, and rejects with HTTP 401 `'Expiration time' claim ('exp') is too far in the future`. Fix by shrinking `JWT_LIFETIME` from 600s to 540s, giving 60s of skew headroom before GitHub's hard ceiling. Add a single regression test, a structured-log signal when fallback engages, and one operator note in `docs/configuration.md` about clock-skew tolerance and the operator action when an agent needs distinct GH identity. The ticket's "no PAT fallback for mika-relay" framing is inverted — `resolve_github_token` already prefers PAT first; the operator-side fix (mint a PAT for any agent that needs distinct GitHub identity) is unblocked the moment the JWT bug is gone.

## Evidence

- Verified host clock skew: `date -u` returns 17:29:26Z while GitHub's `Date:` response header at the same wall second returns `17:28:17 GMT` — the host is **69 seconds ahead** of GitHub's authoritative clock.
- Verified JWT math at `crates/mika-common/src/github_app.rs:277-293`:
  ```rust
  const IAT_BACKDATE: Duration = Duration::from_secs(60);   // line 22
  const JWT_LIFETIME: Duration = Duration::from_secs(600);  // line 25
  let iat = now.as_secs() - IAT_BACKDATE.as_secs();
  let exp = iat + JWT_LIFETIME.as_secs();
  ```
  By the host: `exp − now_host = 540s`. GitHub validates: `exp ≤ github_now + 600s`. Substituting `now_host = github_now + 69s`: GitHub sees `exp = github_now + 609s`, **9 seconds over the ceiling**. Empirical 401.
- Verified blast radius is global, not mika-relay-specific: `grep -c "GitHub App token exchange failed" /var/log/mika/server.log` → **312 occurrences** today across multiple agents (chase-hughes, mika-relay, …). Any agent without `MIKA_GITHUB_TOKEN` in its `~/.mika/agents/<name>/.env` falls through to the App path and hits the same JWT exp rejection.
- Verified ticket's PAT-fallback framing is inverted at `crates/mika-common/src/config.rs:969-989` — `resolve_github_token()` is **PAT-first, App-as-fallback** by design (per ADR-008 machine-user identity). mika-dev and mika-qa each have their own PAT in `~/.mika/agents/<name>/.env`; mika-relay has no `.env` file at all, so it falls through to the broken App path. The "fix" the ticket asks for ("configure PAT fallback for mika-relay") is operator config (mint a relay PAT or accept App-only), not a code change.

### Phase 0 pin — single JWT constant covers every callsite

Pinned at HEAD of `crates/mika-common/src/github_app.rs`:

- **Line 25** — sole definition: `const JWT_LIFETIME: Duration = Duration::from_secs(600);`. `grep -n "JWT_LIFETIME"` returns three hits in this file: definition (25), consumer (283), and the test assertion (502). No other crate defines or shadows the constant.
- **Line 277-293** — sole consumer: `fn generate_jwt(&self)` reads `JWT_LIFETIME.as_secs()` at line 283 to compute `exp = iat + JWT_LIFETIME.as_secs()`.
- **Line 126-150** — `pub async fn installation_token(&self)` is the public entry-point. Line 145 calls `self.generate_jwt()?` after the double-checked-locking cache miss path.
- **Line 162-183** — `pub async fn installation_token_with_file_cache(&self, cache_path: &Path)` is the file-cached entry-point (used by `mika token` / credential-helper / `mika skills`). Line 169 calls `self.installation_token().await?` — i.e., it delegates to the in-memory path on cache miss. There is **no separate JWT path** for the cached caller.
- **Cache lifetime is unrelated to the JWT lifetime.** `EXPIRY_BUFFER` (line 19, 5 min) is checked against `CachedToken.expires_at` (line 268), which is populated from GitHub's response in `exchange_jwt_for_token` at line 341 (the *installation token*'s 1h lifetime as returned by `/access_tokens`). Same for the file cache: `FileCachedToken.expires_at` (line 36-37) is the installation token's expiry, not the JWT's. Shrinking `JWT_LIFETIME` from 600 to 540 has zero effect on cache shape — caches key on the issued installation token, not on the single-use JWT.

Conclusion: a single-line change at line 25 covers every callsite — `installation_token`, `installation_token_with_file_cache`, and any future caller that goes through the same `generate_jwt → exchange_jwt_for_token` chain. Verified by code-read, not by inference.

## Root cause and design choice

**Root cause.** GitHub's installation-token endpoint enforces a strict `exp ≤ iat + 600s` ceiling against *its own* clock — not the issuer's. With a host clock skewed +Δ seconds ahead of GitHub, an `exp` computed at host-time `now − 60 + 600 = now + 540` lands at GitHub-time `github_now + 540 + Δ`. Any positive Δ > 60s exceeds the ceiling. GitHub does not parameterize this; it is a fixed validator on their side.

**Design choice.** Two viable in-code mitigations:

1. **Shrink `JWT_LIFETIME` from 600s to 540s** (`exp = now + 480` host-time, `github_now + 549` for Δ=69s). Tolerates clock skew up to **120s** before hitting the ceiling. JWT lifetime is operationally irrelevant — the JWT is single-use and consumed within milliseconds; only the resulting installation token (1h lifetime) is cached. Cost: zero. (Chosen.)
2. **Auto-tune from GitHub's `Date:` response header**: parse `Date:` from the 401 response, compute observed skew, retry once with adjusted `iat`. Cost: complexity (state machine, retry logic, no upstream guarantee `Date:` is wall-accurate to ms). Rejected — over-engineered for a problem solved by a one-line constant change.

The ticket's "fix host clock skew via NTP" is correct as a hardening *operational* recommendation but not as the code fix: deployments will run on hosts with imperfect NTP discipline, and the JWT generator should be tolerant rather than brittle. NTP drift up to ~2 minutes is normal on managed-but-not-paranoid hosts.

The IAT_BACKDATE constant (60s) is doing its job for the `iat` validator (GitHub's `iat ≤ github_now`); it does not help with the `exp` ceiling because the validator is `exp − iat ≤ 600`, not `exp − github_now ≤ 600`.

## Scope

In:
- Code change: shrink `JWT_LIFETIME` from `Duration::from_secs(600)` to `Duration::from_secs(540)` in `crates/mika-common/src/github_app.rs:25`. Update doc comment on the constant.
- Test: extend `test_generate_jwt_claims` in the same file to assert `exp − iat == 540` and add a comment explaining the headroom rationale (60s skew tolerance below GitHub's 600s ceiling).
- Structured-log signal on fallback: replace the existing `tracing::warn!` at `crates/mika-common/src/config.rs:982-985` with a structured `tracing::warn!(target: "mika::github_auth", event = "gh_app_token_exchange_failed", error = %e, has_pat_fallback = false, "...")`. This addresses the ticket's "structured-log signal (not just WARN) when fallback engages, so silent fallback drift is detectable in audits." Two log fields named per existing observability conventions (`event` for grep-targets, structured kvs for filterable analysis).
- Doc update: extend `crates/mika-agent/docs/configuration.md` § "GitHub token for agent operations" with two short paragraphs:
  1. Note the JWT clock-skew tolerance (now 60s headroom; operator should still keep host NTP healthy if drift exceeds ~1min sustained).
  2. State the operator path for adding a distinct PAT to any agent (e.g., mika-relay): create `~/.mika/agents/<name>/.env`, set `MIKA_GITHUB_TOKEN=<pat>`, restart the server. Reference `resolve_github_token` semantics (PAT-first, App-as-fallback).

Out (explicitly):
- No retry-on-401 logic. Single-line constant change is the minimum viable fix.
- No auto-tune from `Date:` header. Logged as a future option only if 540s proves insufficient under field conditions.
- No new env var to make `JWT_LIFETIME` operator-tunable. Until we observe a deployment where 540s is insufficient, the constant is the simpler shape (per `feedback_keep_simple.md`).
- No PAT minting for mika-relay. That is operator action gated on whether mika-relay needs distinct GitHub identity; the ticket does not establish that need (mika-relay's job is permission-policy classification, not gh API mutation). The doc note tells the operator how to do it if wanted.
- No companion change to mika#1041 (verdict-guard regression). Intentionally separate; that PR is its own scope.

## Implementation steps

1. **Code change** (`crates/mika-common/src/github_app.rs`):
   - Change `const JWT_LIFETIME: Duration = Duration::from_secs(600);` → `Duration::from_secs(540);`.
   - Update the doc comment from `/// JWT lifetime (GitHub maximum: 10 minutes).` to:
     ```
     /// JWT lifetime — 9 minutes (60s under GitHub's 10-minute hard ceiling).
     /// The headroom tolerates positive host-clock skew up to ~120s before
     /// GitHub's `exp ≤ iat + 600s` validator rejects the token. A single-use
     /// JWT has no caching value, so shortening the lifetime has no
     /// operational cost. See mika#1042.
     ```
2. **Regression test** (`crates/mika-common/src/github_app.rs::tests::test_generate_jwt_claims`):
   - Extend the existing `assert_eq!(exp - iat, JWT_LIFETIME.as_secs());` block with an explicit `assert_eq!(JWT_LIFETIME.as_secs(), 540);` so the regression class is "someone bumped the constant back to 600 without thinking through skew."
   - Add a one-line comment above the assertion citing mika#1042.
3. **Structured-log signal** (`crates/mika-common/src/config.rs:982-985`):
   - Replace the current single-line warn with a structured event:
     ```rust
     tracing::warn!(
         target: "mika::github_auth",
         event = "gh_app_token_exchange_failed",
         error = %e,
         has_pat_fallback = false,
         "GitHub App token exchange failed; no PAT configured for fallback"
     );
     ```
   - Symmetric event for the success path is **not** added — silence on success is the convention; only the failure path is the audit-relevant signal.
4. **Documentation** (`crates/mika-agent/docs/configuration.md`):
   - Find the existing § "GitHub token for agent operations (PAT fallback)" (line 193). Append the two paragraphs described above. Keep the language imperative and short — operator action and clock-skew note, no narrative. Per `feedback_keep_simple.md`.
5. **No schema change. No CLAUDE.md change** (the change is too small to be load-bearing on conventions).

## Verification

Build:
```bash
cargo build -p mika-common
```

Unit test (covers steps 1+2):
```bash
cargo test -p mika-common --lib github_app::tests::test_generate_jwt_claims
```

Empirical end-to-end check (post-deploy, against the real host with skewed clock):
```bash
# Confirm skew is non-zero (this is the *condition* that previously caused failure):
date -u; curl -sI https://github.com | grep -i '^date:'

# Confirm `mika token` (which reuses the same JWT path through the file-cached helper) returns a non-empty token:
mika token --agent mika-relay 2>&1 | head -3
# Expected: a token line, not an HTTP 401.

# Confirm the warn line stops firing for a fresh dispatch:
tail -F /var/log/mika/server.log | jq 'select(.message | test("GitHub App token exchange failed"))'
# Trigger any agent turn that lacks a PAT (e.g., chase-hughes heartbeat). Expect zero new lines.
```

If a new 401 appears with a different message (anything other than `'Expiration time' claim ('exp')`), that indicates a different failure class and is out of scope for this PR.

## Acceptance criteria mapping

The ticket lists three:
1. **"GH App token exchange returns 200 in normal operation"** — satisfied by the constant change. With the host's current ~69s skew, host-side `exp = now + 480` lands at GitHub-side `github_now + 549` — well under the 600s ceiling. Verified by the empirical check above.
2. **"If JWT path fails, PAT fallback engages and gh-tool calls still work"** — *contract is preserved as designed* but with corrected framing: `resolve_github_token` is PAT-first; the App is the fallback. For an agent without a PAT (e.g., mika-relay), no in-code "PAT fallback" exists by design — the operator path is to mint a PAT and add it to the agent's `.env`. This is documented in step 4.
3. **"Structured-log signal when fallback engages, so silent fallback drift is detectable in audits"** — satisfied by step 3. The structured `event = "gh_app_token_exchange_failed"` field is greppable / jq-filterable in the JSON server log; operators and audits can detect agents that were trying-and-failing the App path silently.

## Risks and rollback

- **Risk: 540s is still insufficient under extreme skew.** If host drift exceeds ~120s the symptom returns. Mitigation: the structured-log signal added in step 3 makes the recurrence visible immediately. Follow-up is the auto-tune-from-Date-header path; we have not committed to it but the diagnostic is in place.
- **Risk: bumping the constant breaks an unrelated cache-lifetime assumption.** Audited the file: `EXPIRY_BUFFER` (5 min) and the file-cache `expires_at` are both keyed off the *installation token*'s expiry returned by GitHub (1 hour by default), not the JWT lifetime. The JWT is single-use within `exchange_jwt_for_token` and never persisted. No cache-shape regression.
- **Rollback:** revert the single-line constant change. Test stays correct (it asserts the chosen lifetime literal, so reverting the constant requires reverting the assertion too — kept atomically in the same PR).

## Out-of-scope follow-ups (separate tickets, not this PR)

- mika#1041 verdict-guard scope leak (companion regression noted in #1042 evidence — its own PR, not bundled).
- Operator decision on whether mika-relay should have its own machine user PAT. Not a code question; route through Vincent if needed.
- Auto-tune-from-Date-header retry logic. Park as future option, gated on whether 540s is insufficient in practice.

## Related references

- ADR-008 — distinct machine-user identity per agent (mika-dev, mika-qa each have their own PAT).
- `feedback_keep_simple.md` — minimum viable fix; no operator-tunable env var until needed.
- `feedback_compound_infra_fixes.md` — flagged: this is an infra fix; compound a brief solution doc post-merge to make the JWT-skew-headroom invariant searchable next time.
- `feedback_smoke_before_claiming_done.md` — the "Verification" section uses real `date -u` + `mika token` output, not "should work" prose.
