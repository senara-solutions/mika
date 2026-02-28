---
title: "feat: Add builtin GitHub skill using gh CLI"
type: feat
status: active
date: 2026-02-28
---

# Add Builtin GitHub Skill

## Overview

Add a builtin GitHub skill that enables Mika to interact with GitHub using the `gh` CLI. The skill is keyword-triggered (not always-on), uses an exec handler to run `gh` commands, and provides rich system prompt context for common GitHub operations.

## Problem Statement / Motivation

Mika currently has no way to interact with GitHub. Users frequently need to check PR status, create issues, review CI results, and manage branches. The `gh` CLI is already installed on the user's desktop and provides a comprehensive GitHub interface. Wrapping it as a Mika skill gives the agent GitHub awareness triggered by natural conversation.

## Proposed Solution

Create a new builtin skill following the established pattern (like `web-search`, `tmux`, `shell-exec`):

- **Handler type:** `exec` — a shell script that executes `gh` commands
- **Trigger:** Keyword-matched (`always_on = false`) on GitHub-related terms
- **Tool:** Single `run_gh` tool with a `command` parameter for the `gh` subcommand
- **System prompt:** Rich guidance with common `gh` operation examples
- **Docker:** Install `gh` CLI binary in `Dockerfile.agent` runtime stage

### Why exec handler (not builtin Rust)?

The `gh` CLI is an external binary — there's no Rust API to call. Exec handlers are the established pattern for wrapping external CLIs (same as `tmux`, `shell-exec`, `file-reader`). This also means the skill is correctly filtered from heartbeat/silent mode via `safe_always_on_skills()`.

### Why a single `run_gh` tool (not per-operation tools)?

The `gh` CLI surface is large (PRs, issues, releases, workflows, repos, gists, etc.). Creating individual tools for each would be excessive and brittle. A single `run_gh` tool with good system prompt guidance lets the agent construct any `gh` command. This mirrors the `shell-exec` pattern but scoped to `gh`.

## Technical Approach

### Files to Create

#### 1. `templates/skills/github/skill.toml`

```toml
[skill]
name = "github"
description = "Interact with GitHub using the gh CLI"
version = "0.1.0"
always_on = false
timeout_secs = 30

[triggers]
keywords = ["github", "pull request", "PR", "issue", "repo", "branch", "release", "merge", "CI", "workflow", "commit"]
```

#### 2. `templates/skills/github/system_prompt.md`

Guidance for the agent on how to use `gh` for common operations:
- Listing/creating/reviewing PRs
- Listing/creating issues
- Checking repo status
- Viewing CI/CD workflow runs
- Merging PRs
- Viewing releases
- Checking notifications

Include concrete command examples. Emphasize:
- Always use `--json` flag for structured output when parsing results
- Use `--limit` to avoid overwhelming output
- Confirm destructive operations (merge, close, delete) with the user first
- Handle errors gracefully (not authenticated, wrong repo, no results)

#### 3. `templates/skills/github/tools.json`

```json
[
  {
    "name": "run_gh",
    "description": "Execute a GitHub CLI (gh) command. Use for interacting with GitHub repositories, pull requests, issues, workflows, and more.",
    "input_schema": {
      "type": "object",
      "properties": {
        "command": {
          "type": "string",
          "description": "The gh subcommand and arguments to execute (e.g., 'pr list --state open', 'issue create --title \"Bug\" --body \"Details\"')"
        },
        "repo": {
          "type": "string",
          "description": "Repository in OWNER/REPO format (optional, defaults to current repo context)"
        }
      },
      "required": ["command"]
    },
    "handler": {
      "type": "exec",
      "command": "handlers/run.sh"
    }
  }
]
```

#### 4. `templates/skills/github/handlers/run.sh`

Shell script that:
1. Reads JSON input from stdin (command, optional repo)
2. Validates `gh` is installed
3. Validates `gh` is authenticated
4. Prepends `--repo OWNER/REPO` if repo parameter provided
5. Executes `gh $command` and returns output
6. Returns stderr on failure

### Files to Modify

#### 5. `crates/mika-agent/src/bundled_skills.rs`

- Add `static GITHUB_SKILL: BundledSkill = skill!(...)` declaration with all template files
- Add `&GITHUB_SKILL` to the `BUNDLED_SKILLS` array

#### 6. `Dockerfile.agent`

Add `gh` CLI binary installation to the runtime stage. Use direct tarball download (avoids adding apt repos):

```dockerfile
# Install GitHub CLI
RUN ARCH=$(dpkg --print-architecture) && \
    wget -qO /tmp/gh.tar.gz "https://github.com/cli/cli/releases/download/v2.65.0/gh_2.65.0_linux_${ARCH}.tar.gz" && \
    tar -xzf /tmp/gh.tar.gz -C /tmp && \
    mv /tmp/gh_*/bin/gh /usr/local/bin/gh && \
    rm -rf /tmp/gh*
```

## Acceptance Criteria

- [x] `templates/skills/github/skill.toml` — manifest with keywords
- [x] `templates/skills/github/system_prompt.md` — agent guidance with examples
- [x] `templates/skills/github/tools.json` — `run_gh` tool definition
- [x] `templates/skills/github/handlers/run.sh` — exec handler script
- [x] `bundled_skills.rs` — GitHub skill registered and included in `BUNDLED_SKILLS`
- [x] `Dockerfile.agent` — `gh` CLI installed in runtime stage
- [x] `cargo build` passes
- [x] `cargo test` passes (existing tests, especially `test_seed_creates_all_skills`)
- [x] `cargo clippy` clean
- [x] Handler script validates `gh` availability and authentication
- [x] Handler script supports optional `--repo` override

## Security Considerations

- **Exec handler filtering:** Skill uses exec handlers, so it's automatically excluded from heartbeat/silent mode by `safe_always_on_skills()`. The agent cannot use GitHub in autonomous background runs.
- **Not always-on:** Only activated when user message contains GitHub keywords. Reduces attack surface.
- **No credential storage:** `gh` manages its own authentication. Mika never sees tokens.
- **System prompt guidance:** Instructs agent to confirm destructive operations (merge, close, delete) with the user.
- **Input validation:** Handler script validates input, checks for empty commands.

## Dependencies & Risks

- **`gh` authentication in containers:** The `gh` CLI requires authentication. In containerized mode, users will need to configure `gh auth` (e.g., via `GH_TOKEN` env var). The handler should detect and report auth failures gracefully.
- **No risk to existing skills:** New skill is additive, doesn't modify any existing code paths.

## References

- Existing exec handler pattern: `templates/skills/tmux/`, `templates/skills/shell-exec/`
- Bundled skill registration: `crates/mika-agent/src/bundled_skills.rs`
- Skill matching: `crates/mika-agent/src/skills/matcher.rs`
- Skill executor: `crates/mika-agent/src/skills/executor.rs`
- Dockerfile: `Dockerfile.agent`
