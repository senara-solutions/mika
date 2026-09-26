---
title: "feat: Add GitHub App auth to skill install git clone"
type: feat
status: completed
date: 2026-04-06
---

# feat: Add GitHub App auth to skill install git clone

## Overview

`mika skills install senara-solutions/qa-review` fails for private repos because `clone_to_temp()` scrubs all MIKA_* env vars and sets `GIT_TERMINAL_PROMPT=0` — git has no credentials. Agents have GitHub App credentials in per-agent `.env` but they never reach the git subprocess.

## Proposed Solution

Resolve the agent's GitHub token (App preferred, PAT fallback) once in the CLI `run()` function, then thread it through the call chain to `clone_to_temp()`, which injects it into HTTPS GitHub URLs as `x-access-token:{token}@github.com`. This follows the same scrub-then-inject pattern used by `run_gh` in `builtin_handlers.rs`.

Token resolution reuses the exact pattern from `credential_helper.rs:get_installation_token()`.

## Acceptance Criteria

- [x] `mika --agent mika-qa skills install senara-solutions/qa-review` succeeds for private repos when agent has GitHub App credentials
- [x] `mika skills install some-public/repo` still works without token (regression)
- [x] `mika skills update` also uses auth for git-sourced skills
- [x] Token never appears in error messages or user-visible output
- [x] Non-GitHub HTTPS URLs and SSH URLs are not rewritten
- [x] Unit tests cover URL injection helper

## MVP

### 1. `crates/mika-agent/src/skills/git.rs` — token injection

Add `github_token: Option<&str>` to `clone_to_temp()`. Add `inject_github_token()` helper.

```rust
// crates/mika-agent/src/skills/git.rs

/// Inject a GitHub token into an HTTPS GitHub URL for authentication.
/// Only rewrites `https://github.com/...` URLs. All others unchanged.
pub(crate) fn inject_github_token(url: &str, token: &str) -> String {
    if url.starts_with("https://github.com/") {
        url.replacen(
            "https://github.com/",
            &format!("https://x-access-token:{token}@github.com/"),
            1,
        )
    } else {
        url.to_string()
    }
}

pub fn clone_to_temp(url: &str, github_token: Option<&str>) -> Result<TempDir> {
    let tmp = TempDir::new().context("failed to create temp directory for clone")?;

    let effective_url = match github_token {
        Some(token) => inject_github_token(url, token),
        None => url.to_string(),
    };

    let output = git_command()
        .args(["clone", "--depth", "1", &effective_url])
        .arg(tmp.path())
        .output()
        .context("failed to run git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Use original URL — never expose token
        bail!("git clone failed: {stderr}");
    }

    Ok(tmp)
}
```

Unit tests for `inject_github_token`:

```rust
#[test]
fn test_inject_github_token() {
    let url = inject_github_token("https://github.com/user/repo.git", "ghs_abc123");
    assert_eq!(url, "https://x-access-token:ghs_abc123@github.com/user/repo.git");
}

#[test]
fn test_inject_github_token_non_github() {
    let url = inject_github_token("https://gitlab.com/user/repo.git", "token");
    assert_eq!(url, "https://gitlab.com/user/repo.git");
}

#[test]
fn test_inject_github_token_ssh() {
    let url = inject_github_token("git@github.com:user/repo.git", "token");
    assert_eq!(url, "git@github.com:user/repo.git");
}
```

### 2. `crates/mika-agent/src/skills/install.rs` — thread token

```rust
// Line 482: add github_token parameter
pub fn update_skill(
    agent_home: &Path,
    skills_dir: &Path,
    name: &str,
    github_token: Option<&str>,
) -> Result<UpdateResult> {
    // ...existing code...
    // Line 533: pass token
    let tmp = git::clone_to_temp(&entry.url, github_token)
        .with_context(|| format!("failed to clone {} for update", entry.url))?;
    // ...rest unchanged...
}
```

### 3. `crates/mika-cli/src/commands/skills.rs` — resolve and thread

Add token resolution helper (mirrors `credential_helper.rs:get_installation_token()`):

```rust
/// Resolve a GitHub token for git clone authentication.
/// Prefers GitHub App installation token, falls back to PAT.
async fn resolve_github_token_for_git(
    global_home: &Path,
    agent_home: &Path,
) -> Option<String> {
    let settings =
        mika_common::config::Settings::load_for_agent(global_home, agent_home).ok()?;

    // Try GitHub App first (short-lived installation token)
    if let Some(app) = mika_common::github_app::GitHubApp::from_settings(&settings) {
        let cache_path = agent_home.join("github_app_token.json");
        if let Ok(token) = app.installation_token_with_file_cache(&cache_path).await {
            return Some(token);
        }
    }

    // Fall back to PAT
    settings.agent_github_token().map(|s| s.to_string())
}
```

Resolve once in `run()`, thread through call sites:

- `run()`: call `resolve_github_token_for_git()` before `match args.command`
- `install_skill()`: add `github_token: Option<&str>`, pass to `install_from_git()`
- `install_from_git()`: add `github_token: Option<&str>`, pass to `git::clone_to_temp(url, github_token)`
- `update_skills()`: add `github_token: Option<&str>`, pass to `install::update_skill()`

## Security

- Token in process args: acceptable — short-lived (1h), same machine, same pattern as `run_gh`
- Token in temp `.git/config`: fine — TempDir drops on scope exit, token expires
- Error messages: must use original URL, never `effective_url`
- Scope: only `https://github.com/` URLs rewritten (matches credential helper filter)

## Sources

- `crates/mika-agent/src/skills/git.rs:79` — `clone_to_temp()` current implementation
- `crates/mika-agent/src/skills/git.rs:165` — `git_command()` env scrubbing
- `crates/mika-agent/src/skills/install.rs:482` — `update_skill()` call site
- `crates/mika-cli/src/commands/skills.rs:478,512,913` — CLI install/update functions
- `crates/mika-cli/src/commands/credential_helper.rs:86` — `get_installation_token()` pattern to reuse
- `crates/mika-agent/src/skills/builtin_handlers.rs:816` — `run_gh` scrub-then-inject pattern
