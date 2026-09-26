---
title: "Verdict handler used global AppState token instead of per-agent Settings"
category: security-issues
date: 2026-04-13
tags: [github-token, verdict-handler, per-agent-identity, ADR-008]
module: mika-agent
severity: high
---

# Verdict handler used global AppState token instead of per-agent Settings

## Problem

The structural verdict handler (`handlers.rs:233`) passed `s.github_token.as_deref()` — the global `AppState` token from `~/.mika/.env` — to `try_handle_pr_review_verdict()`. Per ADR-008, `MIKA_GITHUB_TOKEN` is commented out in the global `.env` because tokens are per-agent. The handler always saw `None` and could never initiate merges.

Log message: `[verdict_handler] VERDICT: pass received but no GitHub token configured. Manual merge required.`

## Root Cause

The verdict handler was added (commit `c9a6320`) on the same day as the per-agent identity separation work. It was wired to `AppState.github_token` (global) instead of `AgentState.settings` (per-agent). The `run_agent()` function at `agent.rs:1243` correctly resolves per-agent tokens via `settings.resolve_github_token()`, but the verdict handler runs *before* `run_agent()` and never benefited from this resolution.

Same class of bug as the exec handler GH_TOKEN injection asymmetry.

## Solution

Two changes in `crates/mika-agent/src/server/handlers.rs`:

1. **Verdict handler token (line 232-235):** Resolve per-agent token before calling `try_handle_pr_review_verdict()`:
   ```rust
   let verdict_github_token = a
       .settings
       .resolve_github_token(a.github_app.as_deref())
       .await;
   ```
   Then pass `verdict_github_token.as_deref()` instead of `s.github_token.as_deref()`.

2. **AgentParams token (line 274):** Changed `s.github_token.as_deref()` to `a.settings.agent_github_token()` for consistency (dead code in practice since `run_agent()` overrides it when `settings` is `Some`).

## Prevention

When adding new code paths that need a GitHub token in server mode, always resolve from the per-agent `Settings` (`a.settings.resolve_github_token()` or `a.settings.agent_github_token()`), never from `AppState.github_token`. The global token exists for backward compatibility but is empty in multi-agent deployments per ADR-008.

**Pattern to follow:** `run_agent()` at `agent.rs:1243-1247`.

## Cross-References

- ADR-008: `docs/adr/008-github-identity-separation.md`
- Same class of bug: `docs/solutions/security-issues/exec-handler-gh-token-injection.md`
- Verdict handler architecture: `docs/solutions/architecture-patterns/structural-verdict-handler-pr-review-auto-merge.md`
- Issue: #561
