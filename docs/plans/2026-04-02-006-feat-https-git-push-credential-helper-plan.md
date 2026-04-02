---
title: "feat: HTTPS git push with installation token — credential helper and mika token CLI"
type: feat
status: active
date: 2026-04-02
issue: "#383"
parent: "senara-solutions/mika-platform#3 (Phase 3)"
---

# feat: HTTPS git push with installation token — credential helper and mika token CLI

## Overview

Switch git push in autonomous claude-pilot sessions from SSH to HTTPS with GitHub App installation tokens. This gives mika-dev-bot[bot] attribution on pushes, replacing the hardcoded PAT in `mika-skills/claude-pilot/handlers/run.sh`. Adds two pieces: a `mika token github` CLI subcommand that prints a fresh installation token to stdout, and a built-in git credential helper subcommand (`mika credential-helper`) that wraps it in git credential protocol format.

## Problem Statement / Motivation

Currently, claude-pilot sessions push via SSH (inheriting the host's `git@github.com:` remote) and authenticate `gh` CLI with a hardcoded PAT (`run.sh` line 86). This has three problems:

1. **Wrong attribution:** Pushes appear as the PAT owner, not `mika-dev-bot[bot]`
2. **Hardcoded secret:** A PAT is embedded in a shell script checked into `mika-skills`
3. **Expiry risk:** PATs can be revoked without the system noticing; GitHub App installation tokens are auto-renewed

Phase 1 (#381) built `GitHubApp` in `mika-common` with JWT signing, installation token exchange, and in-memory caching. This phase wires it into the git push flow.

## Proposed Solution

### Architecture Decision: `mika credential-helper` subcommand (not a separate script)

The SpecFlow analysis identified that a separate shell script (`~/.mika/git-credential-mika-dev-bot`) introduces deployment concerns (creation lifecycle, executable bit, PATH requirements, shell interpreter dependency). Instead, make `mika` itself the credential helper via a built-in subcommand:

```
git config credential.helper '!mika credential-helper'
```

This eliminates script installation, PATH issues, and keeps everything in Rust. Git invokes `mika credential-helper get` and the subcommand handles the credential protocol directly.

### Components

1. **`mika token github`** — Prints a bare installation token to stdout. Lightweight: loads dotenv + Settings, creates `GitHubApp`, calls `installation_token()`. No DB, no tracing init, no agent setup.

2. **`mika credential-helper`** — Git credential helper protocol implementation. Reads stdin for `protocol=`/`host=`, filters to `github.com` only, calls `GitHubApp::installation_token()`, outputs credential protocol response. Handles `get`/`store`/`erase` operations (only `get` produces output; `store` and `erase` are silent no-ops).

3. **Claude-pilot worktree setup** (companion change in `mika-skills`) — After worktree creation, set HTTPS remote and configure credential helper. Remove hardcoded PAT. Set `GH_TOKEN=$(mika token github)` for `gh` CLI.

## Technical Approach

### Phase 1: CLI subcommands (`crates/mika-cli/`)

#### `mika token github` subcommand

**Files:**
- `crates/mika-cli/src/cli.rs` — Add `Token(TokenArgs)` variant to `Commands` enum
- `crates/mika-cli/src/commands/token.rs` — Implementation
- `crates/mika-cli/src/commands/mod.rs` — Add `pub mod token;`
- `crates/mika-cli/src/main.rs` — Add dispatch arm

```rust
// cli.rs — TokenArgs and TokenCommand
#[derive(clap::Args)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub command: TokenCommand,
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Print a GitHub App installation token to stdout
    Github,
}
```

**Implementation (`commands/token.rs`):**
1. Load `~/.mika/.env` via `mika_common::dotenv::load_dotenv()`
2. Create `Settings` (lightweight — no DB needed)
3. Create `GitHubApp::from_settings(&settings)` — returns `Option<Arc<GitHubApp>>`
4. If `None`: print error to stderr, exit 1 (config incomplete)
5. Call `installation_token().await` — if error: print to stderr, exit 1
6. Print bare token to stdout (no newline suffix beyond what `println!` adds)
7. Exit 0

**Key constraints:**
- Token ONLY to stdout, all diagnostics to stderr
- No tracing/logging initialization (fast startup for credential helper usage)
- Exit code 1 on any failure (missing config, network error, invalid key)

**`Commands::agent_override()`** — Add `Commands::Token(_) => None` (no agent scoping)
**`Commands::team_override()`** — Add `Commands::Token(_) => None`

#### `mika credential-helper` subcommand

**Files:** Same as above (added to `TokenArgs` or as a separate top-level command)

**Decision:** Make it a separate top-level command for cleaner git config syntax (`!mika credential-helper` vs `!mika token credential-helper`).

```rust
// cli.rs
#[derive(Subcommand)]
pub enum Commands {
    // ... existing ...
    /// Git credential helper (used by git, not directly by users)
    #[command(name = "credential-helper")]
    CredentialHelper(CredentialHelperArgs),
}

#[derive(clap::Args)]
pub struct CredentialHelperArgs {
    /// Operation: get, store, or erase
    pub operation: String,
}
```

**Implementation (`commands/credential_helper.rs`):**
1. If `operation` is not `"get"`: exit 0 silently (no-op for `store`/`erase`)
2. Read stdin line-by-line until empty line (git credential protocol)
3. Parse `protocol=` and `host=` fields
4. **Host filter (security critical):** If `host` is not `github.com`, exit 0 with no output (let git try other credential sources). This prevents leaking tokens to non-GitHub hosts.
5. If `protocol` is not `https`, exit 0 with no output
6. Load dotenv, create Settings, create GitHubApp (same as `mika token github`)
7. If GitHubApp unavailable: exit 0 with no output (let git fall through to other helpers)
8. Call `installation_token().await`
9. If error: exit 0 with no output (graceful degradation — git tries other sources)
10. Output:
    ```
    protocol=https
    host=github.com
    username=x-access-token
    password=<token>
    ```

**Why exit 0 on failure (not exit 1):** Git credential helpers that exit non-zero cause git to abort the operation entirely. By exiting 0 with no output, git falls through to the next credential source (SSH, system keychain, etc.), preserving backward compatibility.

### Phase 2: Token caching (file-based)

The SpecFlow analysis identified that `GitHubApp`'s in-memory cache is useless for short-lived CLI processes. Each `mika credential-helper get` invocation makes a full JWT + HTTP round-trip (~1-2 seconds).

**Add file-based cache at `~/.mika/github_app_token.json`:**

```json
{
  "token": "ghs_...",
  "expires_at": "2026-04-02T19:30:00Z"
}
```

**Implementation in `github_app.rs`:**
- Add `pub async fn installation_token_with_file_cache(&self, cache_path: &Path) -> Result<String>`
- Check file cache first (same 5-minute `EXPIRY_BUFFER` validation)
- On miss: call existing `installation_token()`, write result to file
- File permissions: `0o600` (owner-only read/write)
- Graceful: if file cache read/write fails, fall through to in-memory cache
- CLI subcommands use this method; agent runtime continues using in-memory cache

This reduces latency from ~1.5s to ~1ms for cached tokens during multi-push sessions.

### Phase 3: Claude-pilot worktree setup (companion change — `mika-skills` repo)

**File:** `mika-skills/claude-pilot/handlers/run.sh`

**Changes:**
1. **Remove hardcoded PAT** (line 86): Delete `export GH_TOKEN="github_pat_..."`
2. **After worktree creation** (after line 157, also in reuse path):
   ```bash
   # Configure HTTPS remote with credential helper for mika-dev-bot attribution
   git -C "$WORKTREE_DIR" remote set-url origin "https://github.com/senara-solutions/${REPO}.git"
   git -C "$WORKTREE_DIR" config credential.helper '!mika credential-helper'
   ```
3. **Set `GH_TOKEN` dynamically** (replacing line 86):
   ```bash
   # Fresh installation token for gh CLI (expires in ~1 hour)
   if command -v mika &>/dev/null; then
     GH_TOKEN_VALUE=$(mika token github 2>/dev/null)
     if [ -n "$GH_TOKEN_VALUE" ]; then
       export GH_TOKEN="$GH_TOKEN_VALUE"
     fi
   fi
   ```

**Note:** This is a companion change in `mika-skills`. The `mika` repo PR must be merged and deployed first so `mika token github` and `mika credential-helper` are available when `run.sh` uses them.

### Phase 4: `gh` CLI token expiry in long sessions

**Current state:** `GH_TOKEN` set once at session start, expires after ~1 hour.

**Approach:** Accept the limitation for now. Rationale:
- Claude-pilot sessions typically complete within 30-60 minutes
- If `gh` fails with 401, the claude-pilot error handling retries or surfaces the failure
- The `canUseTool` callback in `run.sh` could refresh the token on each tool use (future enhancement)
- `run_gh` in the agent loop already resolves a fresh token per turn via `resolve_github_token()`

**Future enhancement (not in this PR):** Add `GH_TOKEN` refresh in `canUseTool` callback — every tool permission check calls `mika token github` and re-exports.

## System-Wide Impact

### Interaction Graph

`git push` → git reads `credential.helper` config → spawns `mika credential-helper get` → loads `~/.mika/.env` → `GitHubApp::from_settings()` → `installation_token()` → JWT + HTTP to GitHub API → token returned via credential protocol → git authenticates HTTPS push.

For `gh` CLI: `run.sh` calls `mika token github` → sets `GH_TOKEN` → claude-pilot launches → Claude Code uses `gh` via Bash tool.

### Error Propagation

- Missing GitHub App config → `credential-helper` returns empty (git falls through to other sources)
- GitHub API failure → `credential-helper` returns empty (same graceful degradation)
- `mika` binary not on PATH → git reports credential helper error → push fails with auth error
- All errors surface as git authentication failures — the `credential-helper` never crashes git

### State Lifecycle Risks

- **File cache (`github_app_token.json`):** Stale cache is harmless (expired tokens trigger refresh). Race condition between concurrent credential-helper invocations is benign (worst case: two JWT exchanges, both succeed).
- **Remote URL change:** One-way migration from SSH to HTTPS in worktrees. Existing SSH-based workflows unaffected (only worktrees created by `run.sh` are modified).

### API Surface Parity

- `mika token github` — New CLI subcommand (no agent loop involvement)
- `mika credential-helper` — New CLI subcommand (git plumbing, not user-facing)
- No changes to agent tools, HTTP server, or dashboard API

## Acceptance Criteria

### Functional Requirements

- [x] `mika token github` prints a valid GitHub App installation token to stdout when all 3 config vars are set
- [x] `mika token github` prints an error to stderr and exits 1 when config is incomplete
- [x] `mika credential-helper get` returns valid credentials for `github.com` HTTPS requests
- [x] `mika credential-helper get` returns empty for non-GitHub hosts (security filter)
- [x] `mika credential-helper store` and `mika credential-helper erase` exit 0 silently
- [x] File-based token cache at `~/.mika/github_app_token.json` with `0o600` permissions
- [x] Cached tokens reused within 5-minute expiry buffer
- [ ] `git push` via HTTPS with credential helper succeeds and attributes to the GitHub App bot

### Testing Requirements

- [x] Unit tests for credential protocol parsing (stdin → fields)
- [x] Unit tests for host filtering (github.com accepted, others rejected)
- [x] Unit tests for operation dispatch (get/store/erase)
- [x] Unit tests for file cache read/write/expiry
- [x] `test_clap_markdown_contains_all_commands` updated for new subcommands
- [ ] Manual integration test: `mika token github` produces a valid token
- [ ] Manual integration test: `git push` with credential helper attributes to bot

### Non-Functional Requirements

- [x] `mika credential-helper get` completes in <500ms with cached token
- [x] No tracing/logging initialization in token subcommands (fast startup)
- [x] File cache has restrictive permissions (0o600)
- [x] No MIKA_* env vars leaked to child processes

## Dependencies & Risks

### Dependencies

- **Phase 1 (#381):** `GitHubApp` in `mika-common` — already merged ✅
- **`mika` binary on PATH:** Credential helper requires `mika` to be installed. Container images (`Dockerfile.agent`) already install it.
- **Companion PR in `mika-skills`:** `run.sh` changes must ship after `mika` binary update

### Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `mika` not on PATH in worktree context | Low | Push fails | `run.sh` verifies binary exists before configuring credential helper |
| Token exchange latency on cold cache | Medium | 1-2s push delay | File-based cache eliminates on subsequent pushes |
| `gh` token expiry in long sessions | Medium | `gh` commands fail | Accept for now; future `canUseTool` refresh |
| Breaking existing SSH-based workflows | Low | Would break all pushes | Change is scoped to `run.sh` worktrees only; parent repos keep SSH |

## Implementation File List

### `mika` repo (this PR)

| File | Action | Purpose |
|------|--------|---------|
| `crates/mika-cli/src/cli.rs` | Modify | Add `Token` and `CredentialHelper` command variants |
| `crates/mika-cli/src/commands/mod.rs` | Modify | Add `pub mod token;` and `pub mod credential_helper;` |
| `crates/mika-cli/src/commands/token.rs` | Create | `mika token github` implementation |
| `crates/mika-cli/src/commands/credential_helper.rs` | Create | `mika credential-helper` implementation |
| `crates/mika-cli/src/main.rs` | Modify | Add dispatch arms for new commands |
| `crates/mika-common/src/github_app.rs` | Modify | Add `installation_token_with_file_cache()` |

### `mika-skills` repo (companion PR)

| File | Action | Purpose |
|------|--------|---------|
| `claude-pilot/handlers/run.sh` | Modify | HTTPS remote, credential helper config, remove hardcoded PAT |

## Sources & References

- Related issue: #383
- Parent issue: senara-solutions/mika-platform#3 (Phase 3)
- Phase 1 PR: #381
- Learnings: `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`
- Learnings: `docs/solutions/architecture-patterns/github-app-jwt-authentication-module.md`
- Learnings: `docs/solutions/integration-issues/run-gh-github-token-injection.md`
- Git credential helper protocol: https://git-scm.com/docs/gitcredentials
- GitHub App installation tokens: https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app
