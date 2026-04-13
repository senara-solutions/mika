---
title: "fix: verdict handler uses global AppState token instead of per-agent Settings token"
type: fix
status: completed
date: 2026-04-13
---

# fix: verdict handler uses global AppState token instead of per-agent Settings token

The verdict handler at `handlers.rs:233` passes `s.github_token.as_deref()` (global `AppState`) to `try_handle_pr_review_verdict()`. The global `~/.mika/.env` has `MIKA_GITHUB_TOKEN` commented out per ADR-008, so the handler always sees `None` and logs "no GitHub token configured. Manual merge required."

Per-agent tokens in `~/.mika/agents/mika-dev/.env` are correctly configured but never consulted. This is the same class of bug as the exec handler GH_TOKEN injection fix (`docs/solutions/security-issues/exec-handler-gh-token-injection.md`).

## Acceptance Criteria

- [x] Verdict handler resolves the GitHub token from per-agent `Settings` via `resolve_github_token()`, matching the `run_agent()` pattern at `agent.rs:1243-1247`
- [x] `VERDICT: pass` merges succeed when the per-agent `.env` has `MIKA_GITHUB_TOKEN` set but the global `.env` does not
- [x] No silent fallback between token scopes — if the per-agent token is `None` and no GitHub App is configured, the handler degrades gracefully (existing behavior)
- [x] Existing verdict handler tests pass unchanged (function signature stays the same — only the caller changes)

## MVP

### crates/mika-agent/src/server/handlers.rs

Before (line 233):
```rust
s.github_token.as_deref(),
```

After — resolve per-agent token before the verdict handler call:
```rust
// Resolve per-agent GitHub token (PAT > App > None), matching run_agent() pattern
let verdict_github_token = a.settings.resolve_github_token(
    a.github_app.as_deref()
).await;
```

Then pass `verdict_github_token.as_deref()` as the third argument to `try_handle_pr_review_verdict()`.

### Secondary consistency fix (line 268)

The `AgentParams` construction also passes `s.github_token.as_deref()` at line 268. While `run_agent()` overrides this via `settings.resolve_github_token()` at line 1243 (since `settings` is always `Some` in this path), fixing it for consistency:

```rust
github_token: a.settings.agent_github_token(),
```

## Sources

- ADR-008: `docs/adr/008-github-identity-separation.md`
- Same class of bug: `docs/solutions/security-issues/exec-handler-gh-token-injection.md`
- Token resolution pattern: `crates/mika-agent/src/agent.rs:1243-1247`
- Issue: #561
