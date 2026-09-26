---
module: mika-common
tags: [github-app, jwt, clock-skew, authentication, infrastructure]
problem_type: auth_failure
category: infrastructure
ticket: mika#1042
date: 2026-05-08
---

# JWT clock-skew headroom for GitHub App token exchange

## Problem

GitHub App installation token exchange returned HTTP 401 with `'Expiration time' claim ('exp') is too far in the future` on every invocation. The host clock was ~69s ahead of GitHub's authoritative clock, and the JWT generator computed `exp = now - 60 + 600 = now + 540` host-time, which landed at `github_now + 609` — 9 seconds over GitHub's hard 600s ceiling.

## Root cause

`JWT_LIFETIME` was set to the maximum 600s. Combined with the 60s `IAT_BACKDATE`, the effective `exp` relative to the host was `now + 540`. Any positive clock skew > 60s exceeded GitHub's `exp ≤ iat + 600s` validator.

## Fix

Shrink `JWT_LIFETIME` from `Duration::from_secs(600)` to `Duration::from_secs(540)` in `crates/mika-common/src/github_app.rs`. This gives ~120s of clock-skew headroom before the ceiling. The JWT is single-use (consumed in milliseconds) and never cached, so the shorter lifetime has zero operational cost.

Added a regression test pinning `JWT_LIFETIME.as_secs() == 540` and a structured log event (`gh_app_token_exchange_failed` on target `mika::github_auth`) for audit visibility when the App token path fails.

## Key insight

GitHub's `exp` validator checks against *its own* clock, not the issuer's. On hosts with imperfect NTP discipline (drift up to ~2 minutes is normal), the JWT generator must leave headroom below the 600s ceiling rather than using the full window. The `IAT_BACKDATE` (60s) helps with the `iat ≤ github_now` check but does nothing for the `exp` ceiling because the validator is `exp - iat ≤ 600`, not `exp - github_now ≤ 600`.

## Detection signal

```bash
# Grep for the structured event in server logs:
jq 'select(.fields.event == "gh_app_token_exchange_failed")' < /var/log/mika/server.log

# Check host clock skew against GitHub:
date -u; curl -sI https://github.com | grep -i '^date:'
```

## Files changed

- `crates/mika-common/src/github_app.rs` — constant change + test
- `crates/mika-common/src/config.rs` — structured log event
- `docs/configuration.md` — operator documentation
