---
title: "Skill install git clone fails for private repos — missing GitHub auth"
category: integration-issues
date: 2026-04-06
tags: [skills, git, github-app, authentication, marketplace]
related_issues: []
---

# Skill install git clone fails for private repos

## Problem

`mika --agent mika-qa skills install senara-solutions/qa-review` fails with `fatal: could not read Username for 'https://github.com': terminal prompts disabled` when the target repo is private. Agents have GitHub App credentials in per-agent `.env` files, but these never reach the git subprocess.

## Root Cause

`git_command()` in `crates/mika-agent/src/skills/git.rs` sets `GIT_TERMINAL_PROMPT=0` and calls `scrub_mika_env_vars_std()`, removing all `MIKA_*` env vars from the child process. No credentials are injected after the scrub. The `mika credential-helper get` command exists but is never registered with git during clone.

Contrast with `run_gh` in `builtin_handlers.rs` which follows a "scrub first, inject after" pattern — it scrubs MIKA_* vars, then injects `ctx.github_token` as `GH_TOKEN`. The git clone path had no equivalent injection.

## Solution

Inject the agent's GitHub token directly into the HTTPS URL before cloning: `https://github.com/user/repo.git` becomes `https://x-access-token:{token}@github.com/user/repo.git`.

**Key changes:**

1. **`git.rs`**: `clone_to_temp()` accepts `github_token: Option<&str>`. A private `inject_github_token()` helper rewrites only `https://github.com/` URLs — SSH, non-GitHub HTTPS, and other protocols pass through unchanged. Error messages always use the original URL (never the token-embedded URL).

2. **`install.rs`**: `update_skill()` threads the token parameter to `clone_to_temp()`.

3. **`skills.rs`**: A `resolve_github_token_for_git()` async helper (mirroring `credential_helper.rs:get_installation_token()`) resolves the token: GitHub App installation token (short-lived, 1h) preferred, PAT fallback. Token is resolved lazily — only in the `Install` and `Update` match arms, not for `list`/`info`/`validate`/etc.

## Prevention

- When adding new git subprocess calls, follow the scrub-then-inject pattern established by `run_gh`. Scrub MIKA_* vars for defense-in-depth, then inject the specific credential needed.
- The `resolve_github_token_for_git()` pattern (App > PAT > None) should be reused for any future git operations needing auth.
- Test private repo access as part of agent validation (`mika agents validate` could check git auth).
