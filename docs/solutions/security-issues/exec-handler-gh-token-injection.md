---
title: "Exec handler GitHub identity gap — GH_TOKEN injection asymmetry"
category: security-issues
date: 2026-04-11
severity: high
tags: [identity-separation, GH_TOKEN, exec-handler, scrub, defense-in-depth, agent-parity]
issue: "#515"
umbrella: "#517"
modules: [executor, agent, skills, qa-review]
related:
  - docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md
  - docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md
  - docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md
  - docs/adr/008-github-identity-separation.md
---

# Exec handler GitHub identity gap — GH_TOKEN injection asymmetry

## Problem

The Mika engine had two parallel paths for invoking the `gh` CLI from agent context, with **inconsistent** identity handling:

| Path | Token re-injected? | Resulting identity |
|------|--------------------|--------------------|
| Builtin `run_gh` (Rust) | Yes — `cmd.env("GH_TOKEN", token)` after scrub | Agent's `MIKA_GITHUB_TOKEN` (e.g., `mika-platform-dev`) |
| Builtin `pr_merge_with_gate` (Rust) | Yes — same pattern | Agent's `MIKA_GITHUB_TOKEN` |
| **Exec handler skills** (`run_gh.sh`, etc.) | **No** | Host `~/.config/gh/hosts.yml` (typically the developer's personal account) |

The gap manifested during mika-qa's PR review on mika-skills#124 (2026-04-10):

```
$ gh pr review 124 --approve --body '...'
failed to create review: GraphQL: Review Can not approve your own pull request
```

mika-qa's `MIKA_GITHUB_TOKEN` mapped to the `mika-platform-qa` machine user — but its `qa-review` skill uses an **exec handler** (`handlers/run_gh.sh`), not the builtin. The handler subprocess inherited zero credentials after `scrub_mika_env_vars()` ran, so `gh` fell back to the host's active account (`samidarko`). Since `samidarko` was also the PR author, GitHub refused the approval as a self-review.

## Root cause

`scrub_mika_env_vars()` (`crates/mika-agent/src/skills/executor.rs:38-47`) is intentionally aggressive — it strips all `MIKA_*` env vars plus a small allowlist (`EXTRA_SCRUB_VARS = ["GH_TOKEN"]`) as defense-in-depth against credential leakage from `~/.mika/.env` into child processes (see [gh-token-identity-collision-dotenv-leak.md](./gh-token-identity-collision-dotenv-leak.md)).

The builtin `run_gh` handler at `crates/mika-agent/src/skills/builtin_handlers.rs:831-838` re-injects the agent's token immediately after scrubbing:

```rust
scrub_mika_env_vars(&mut cmd);
if let Some(token) = ctx.github_token {
    cmd.env("GH_TOKEN", token);
}
```

But `execute_exec()` and `execute_long_running()` (the generic exec handler dispatchers in `executor.rs`) **never received `github_token`** — `execute_skill_tool()` had no parameter for it, and the call site in `agent.rs` didn't pass `dispatch.ctx.github_token` even though `ToolDispatchCtx.ctx` had it available.

Result: defense-in-depth scrubbing was working correctly, but only the builtin path knew how to recover from it. Skill-level exec handlers were silently downgraded to host identity.

## Solution

Thread `github_token: Option<&str>` from `ToolDispatchCtx.ctx.github_token` through the exec handler call chain and re-inject after scrubbing — same pattern as the builtins.

**Files changed (commit `859c2dd`):**

- `crates/mika-agent/src/skills/executor.rs`:
  - Add `github_token: Option<&str>` parameter to `execute_skill_tool()`, `execute_inner()`, `execute_exec()`, `execute_long_running()`
  - Add `github_token: Option<String>` parameter to `spawn_long_running_exec()` (owned because the spawned task needs `'static`)
  - In both `execute_exec()` and `spawn_long_running_exec()`: inject `GH_TOKEN` after `scrub_mika_env_vars()`

- `crates/mika-agent/src/agent.rs`: pass `dispatch.ctx.github_token` to `execute_skill_tool()` at the call site

- `crates/mika-cli/src/commands/skills.rs`: CLI `mika skills test` passes `None` (no agent identity context in standalone CLI invocation)

**Injection code (mirrors builtin `run_gh`):**

```rust
scrub_mika_env_vars(&mut cmd);
// Re-inject agent's GitHub token for platform identity separation.
// Same pattern as builtin run_gh handler (builtin_handlers.rs).
if let Some(token) = github_token {
    cmd.env("GH_TOKEN", token);
}
```

**`None` semantics preserved:** when `github_token` is `None` (CLI test mode, agent without `MIKA_GITHUB_TOKEN` configured), no injection happens. The handler's `gh` calls fall back to host auth — existing behavior, no silent token substitution. This honors the [dedicated-github-token-agent-operations](../architecture-patterns/dedicated-github-token-agent-operations.md) principle: never silently fall back between token scopes.

## Verification

New unit test `test_exec_handler_receives_gh_token` in `executor.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_handler_receives_gh_token() {
    let tmp = tempfile::tempdir().unwrap();
    write_script(
        &tmp.path().join("check_token.sh"),
        "#!/bin/sh\necho \"GH_TOKEN=$GH_TOKEN\"",
    );

    let tool = make_exec_tool(tmp.path(), "check_token.sh");

    // With github_token provided — should appear in child env
    let output = execute_skill_tool(
        &tool, serde_json::json!({}), 30, None, Some("ghp_test_token_123"),
    ).await;
    assert!(output.content.contains("GH_TOKEN=ghp_test_token_123"));

    // Without github_token — GH_TOKEN should be absent (scrubbed)
    let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
    assert!(!output.content.contains("GH_TOKEN=ghp_"));
}
```

`cargo test --workspace`: 2159 tests pass. `cargo clippy --workspace --all-targets`: clean. Pre-commit lefthook hooks pass (rust-fmt, rust-clippy).

## Prevention

1. **Pattern audit:** When adding any new subprocess spawn site that calls `scrub_mika_env_vars()`, verify it also re-injects required platform credentials (`GH_TOKEN` today, possibly more tomorrow). The `pr_merge_with_gate` builtin tool followed this pattern correctly; the generic exec handler did not. Asymmetry like this is the canonical sign of a parity gap.

2. **Test the scrub-then-inject contract:** Tests that exercise child env vars (like the new `test_exec_handler_receives_gh_token`) catch regressions where someone adds a new spawn site without the inject step. Worth replicating for any future credential.

3. **Architectural rule (per [ADR-008](../../adr/008-github-identity-separation.md)):** Agents never hold GitHub App credentials. They only ever receive a `GH_TOKEN` injected by the engine. Any new agent-spawned subprocess that talks to GitHub must go through this single inject point.

4. **Known remaining gap (P3):** Exec handler subprocesses can also invoke `git push` / `git clone` over HTTPS, which authenticates via git credential helpers — not `GH_TOKEN`. This commit does not address that path. Self-dev currently uses SSH for git, so impact is low. See `todos/751-complete-p3-extend-gh-token-injection-to-git-https-exec-handlers.md` for the documented limitation. The fix would set `GIT_ASKPASS` to a helper that emits the token; deferred until a real use case arises.

## Cross-references

- Companion doc audit: extended `docs/configuration.md` and `docs/skills.md` to document exec handler `GH_TOKEN` injection (commit `95be4a5`)
- ADR-008 (`docs/adr/008-github-identity-separation.md`) — the architectural decision behind why machine user PATs (not GitHub App bots) handle action authorship in the self-dev loop
- Related security solutions:
  - [gh-token-identity-collision-dotenv-leak](./gh-token-identity-collision-dotenv-leak.md) — why `GH_TOKEN` is in `EXTRA_SCRUB_VARS` in the first place
  - [env-var-leakage-exec-handler-child-processes](./env-var-leakage-exec-handler-child-processes.md) — the three-tier env isolation model that exec handlers participate in
- Related architecture pattern:
  - [dedicated-github-token-agent-operations](../architecture-patterns/dedicated-github-token-agent-operations.md) — never silently fall back between token scopes
- Audit traces:
  - mika-qa session `8c0d7827` (2026-04-10T22:21) — the failing review attempt where this was detected
  - PR mika#518 — the PR mika-qa was trying to approve when the failure occurred (eventually merged manually)

## Lessons

- **Defense-in-depth that works asymmetrically is worse than no defense.** Scrubbing `GH_TOKEN` from child processes was correct; the bug was that not all subprocess spawn sites knew how to re-inject the agent's token after the scrub. The resulting silent fallback to host identity is exactly the failure mode the scrubbing was trying to prevent.
- **A new identity model surfaces hidden parity gaps.** Once mika-dev and mika-qa got separate PAT identities (per ADR-008), the `samidarko`-as-fallback bug instantly became visible — both PR author and reviewer resolved to the same human user, and GitHub's self-approval check caught it. Before identity separation, this gap was invisible because everything ran as the developer anyway.
- **Single-line config gaps cause multi-week debugging cycles.** The fix is 4 lines of Rust. Finding it required: (1) noticing the failed review, (2) tracing through the executor scrub path, (3) comparing against the builtin handler, (4) understanding why threads of `github_token` stopped at `execute_skill_tool()`. The doc audit (`docs/configuration.md`) now calls this out for the next person.
