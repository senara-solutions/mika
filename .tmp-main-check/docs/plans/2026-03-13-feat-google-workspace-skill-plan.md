---
title: Add Google Workspace builtin skill via gws CLI
type: feat
status: completed
date: 2026-03-13
origin: docs/brainstorms/2026-03-13-google-workspace-skill-brainstorm.md
---

# feat: Add Google Workspace builtin skill via gws CLI

## Overview

Add a `google-workspace` builtin skill that enables the Mika agent to interact with Gmail, Calendar, and Drive using the [Google Workspace CLI (`gws`)](https://github.com/googleworkspace/cli). The skill mirrors the existing `run_gh` builtin handler pattern exactly — single tool, command-as-array input, subcommand allowlist, env var scrubbing (see brainstorm: `docs/brainstorms/2026-03-13-google-workspace-skill-brainstorm.md`).

## Problem Statement / Motivation

An executive assistant that cannot access email, calendar, and files is severely limited. Google Workspace is the dominant productivity suite. Issue [#75](https://github.com/senara-solutions/mika/issues/75) requests this capability. The `gws` CLI is Rust-native, JSON-first, and covers all Google Workspace APIs through auto-generated commands — making it an ideal fit.

## Proposed Solution

### Phase 1: Skill templates (3 files)

**`crates/mika-agent/templates/skills/google-workspace/skill.toml`**
```toml
[skill]
name = "google-workspace"
description = "Interact with Google Workspace (Gmail, Calendar, Drive) using the gws CLI"
version = "0.1.0"
always_on = false
timeout_secs = 45

[triggers]
keywords = ["google", "gmail", "google calendar", "google drive", "gdrive"]
```

- `timeout_secs = 45` (not 30) — gws fetches Google's Discovery Service on first call, which can be slow.
- Keywords are specific to avoid false positives. Removed generic "email", "calendar", "meeting", "schedule", "event" to prevent overlap with the existing `calendar` skill. Kept "google" prefix variants.

**`crates/mika-agent/templates/skills/google-workspace/tools.json`**
```json
[
  {
    "name": "run_gws",
    "description": "Execute a Google Workspace CLI (gws) command for Gmail, Calendar, or Drive operations.",
    "input_schema": {
      "type": "object",
      "properties": {
        "command": {
          "type": "array",
          "items": { "type": "string" },
          "minItems": 1,
          "description": "The gws subcommand and arguments as an array of strings (e.g., [\"gmail\", \"messages\", \"list\", \"--params\", \"{\\\"maxResults\\\": 5}\"])"
        }
      },
      "required": ["command"]
    },
    "handler": {
      "type": "builtin",
      "function": "run_gws"
    }
  }
]
```

No `repo` equivalent parameter needed (unlike `run_gh`).

**`crates/mika-agent/templates/skills/google-workspace/system_prompt.md`**

Key sections:
- Allowed services: gmail, calendar, drive
- Command format with examples (standard and helper `+` commands)
- Use `--format json` for structured output
- Use `--page-limit N` instead of `--page-all` to avoid output truncation
- Use `--dry-run` for previewing destructive operations (send, delete)
- Exit code semantics: 0=success, 1=API error, 2=auth error, 3=validation, 4=discovery, 5=internal
- Auth error guidance: "Token may be expired — ask user to refresh MIKA_GOOGLE_TOKEN"
- Confirmation required before: sending emails, deleting files/events, modifying permissions

### Phase 2: Builtin handler (`builtin_handlers.rs`)

**Add `"run_gws"` to `KNOWN_BUILTINS`** (line 33).

**Add `validate_gws_input` function** — mirrors `validate_gh_input`:
1. Reject string command (must be array)
2. Parse array elements as strings
3. Empty array check
4. Total length ≤ 10,000 chars
5. First element must be in allowlist: `["gmail", "calendar", "drive"]`
6. Reject flag smuggling: `--token`, `--credentials-file`, `--config`, `--config-dir` in command array
7. Return `GwsArgs { args: Vec<String> }`

**Add `run_gws` async function:**
1. Call `validate_gws_input`
2. Read `MIKA_GOOGLE_TOKEN` from `std::env::var` (direct read, same approach as `gh` relying on ambient `GH_TOKEN`)
3. If missing, return error: "Google Workspace CLI token not configured. Set MIKA_GOOGLE_TOKEN in ~/.mika/.env"
4. Build `tokio::process::Command::new("gws")`
5. Set args from validated array
6. Scrub ALL `GOOGLE_WORKSPACE_CLI_*` env vars (prevent ambient credential leakage)
7. Set `GOOGLE_WORKSPACE_CLI_TOKEN` from the read value
8. Call `scrub_mika_env_vars(&mut cmd)`
9. Set: kill_on_drop, null stdin, piped stdout/stderr
10. Capture output with bounded reads (MAX_OUTPUT_LEN)
11. Return ToolOutput (success: stdout, non-zero: "Exit code: N\n{stderr}{stdout}")

**Token access decision:** Read `std::env::var("MIKA_GOOGLE_TOKEN")` directly in the handler — NOT through `ToolContext`. Rationale: `run_gh` reads nothing from context either; adding to `ToolContext` would require ~15 plumbing changes across agent.rs, loop params, test harness for no benefit. The env var is always available since `Settings` loads it via config-rs with `MIKA_` prefix.

**Extra security (beyond `run_gh` pattern):** Scrub `GOOGLE_WORKSPACE_CLI_*` env vars from child process before setting only the configured token. This prevents ambient credentials from the user's shell from leaking into the agent's gws calls.

### Phase 3: Bundled skill registration (`bundled_skills.rs`)

Add `GOOGLE_WORKSPACE_SKILL` static using the `skill!` macro:
```rust
static GOOGLE_WORKSPACE_SKILL: BundledSkill = skill!("google-workspace", [
    ("skill.toml" => "../templates/skills/google-workspace/skill.toml"),
    ("system_prompt.md" => "../templates/skills/google-workspace/system_prompt.md"),
    ("tools.json" => "../templates/skills/google-workspace/tools.json"),
]);
```

Add to `BUNDLED_SKILLS` array.

### Phase 4: Settings + config registry

Add `google_token: Option<String>` to `Settings` struct in `crates/mika-common/src/config.rs`. Add corresponding `ConfigKeyInfo` entry for `mika config list/get` support. Add redaction in the manual `Debug` impl (same as `anthropic_api_key`).

### Phase 5: Docker (`Dockerfile.agent`)

Add gws binary install step after the gh CLI step:
```dockerfile
# Install Google Workspace CLI (for google-workspace builtin skill) with checksum verification
RUN ARCH_MAP="amd64:x86_64-unknown-linux-gnu arm64:aarch64-unknown-linux-gnu" && \
    DEB_ARCH=$(dpkg --print-architecture) && \
    GWS_ARCH=$(echo "$ARCH_MAP" | tr ' ' '\n' | grep "^${DEB_ARCH}:" | cut -d: -f2) && \
    GWS_VERSION="0.13.3" && \
    wget -qO /tmp/gws.tar.gz "https://github.com/googleworkspace/cli/releases/download/v${GWS_VERSION}/gws-${GWS_ARCH}.tar.gz" && \
    wget -qO /tmp/gws.tar.gz.sha256 "https://github.com/googleworkspace/cli/releases/download/v${GWS_VERSION}/gws-${GWS_ARCH}.tar.gz.sha256" && \
    cd /tmp && echo "$(cat gws.tar.gz.sha256)  gws.tar.gz" | sha256sum -c - && \
    tar -xzf /tmp/gws.tar.gz -C /tmp && \
    mv /tmp/gws /usr/local/bin/gws && \
    rm -rf /tmp/gws*
```

Notes:
- gws uses per-file `.sha256` checksums (not a combined checksums file like gh)
- Asset naming uses Rust target triples: `gws-x86_64-unknown-linux-gnu.tar.gz`
- Both amd64 and arm64 Linux binaries are available (verified: v0.13.3)

### Phase 6: Environment + docs

**`.env.example`** — add after Brave Search section:
```
# Google Workspace CLI token for google-workspace skill (optional)
# Used by gws CLI for Gmail, Calendar, Drive operations.
# Get token via: gws auth login (then export), or use a service account.
# MIKA_GOOGLE_TOKEN=ya29...
```

**`CLAUDE.md`** — add `MIKA_GOOGLE_TOKEN` to Environment Variables section.

### Phase 7: Tests

Unit tests in `builtin_handlers.rs` `#[cfg(test)] mod tests`:
- `test_validate_gws_input_string_rejected` — must be array
- `test_validate_gws_input_empty_array` — empty check
- `test_validate_gws_input_allowed_subcommands` — gmail, calendar, drive pass
- `test_validate_gws_input_disallowed_subcommands` — auth, config, admin, etc. fail
- `test_validate_gws_input_token_smuggling` — `--token` in array rejected
- `test_validate_gws_input_credentials_file_smuggling` — `--credentials-file` rejected
- `test_validate_gws_input_config_smuggling` — `--config`, `--config-dir` rejected
- `test_validate_gws_input_length_limit` — over 10K chars rejected
- `test_validate_gws_input_non_string_elements` — array with non-strings rejected
- `test_run_gws_missing_token` — error message when MIKA_GOOGLE_TOKEN unset
- `test_run_gws_binary_not_found` — error when gws not in PATH

Note: Tests for validation only (no spawning gws). Same approach as `run_gh` tests.

## Technical Considerations

### Calendar skill overlap
The existing `calendar` skill uses HTTP handlers pointing to `localhost:8080/api/events` (placeholder, not implemented). Its keywords overlap ("calendar", "meeting", "schedule", "event"). Resolution: the google-workspace skill uses `google`-prefixed keywords ("google calendar") to avoid triggering alongside the old calendar skill. The old skill can be deprecated in a follow-up PR — out of scope here.

### Token refresh
OAuth2 access tokens expire in ~60 minutes. The `gws` CLI also supports `GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE` for service accounts (auto-refresh, no expiry). This plan supports only the token env var for simplicity. Credentials file support can be added later if users need long-lived auth. The system prompt will guide users on token refresh.

### Discovery service cold start
The gws CLI fetches API schemas from Google on first invocation. `timeout_secs = 45` accommodates this. System prompt notes first-call latency.

### Env var scrubbing (security-critical)
Beyond standard `MIKA_*` scrubbing, the handler explicitly clears ALL `GOOGLE_WORKSPACE_CLI_*` env vars from the child process, then sets only `GOOGLE_WORKSPACE_CLI_TOKEN`. This prevents:
- Ambient `GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE` overriding the configured token
- `GOOGLE_WORKSPACE_CLI_CONFIG_DIR` redirecting to unexpected config
- `GOOGLE_WORKSPACE_CLI_CLIENT_ID`/`CLIENT_SECRET` leaking to child

## Acceptance Criteria

- [x] `run_gws` builtin handler implemented in `builtin_handlers.rs`
- [x] Subcommand allowlist enforced: only `gmail`, `calendar`, `drive`
- [x] Flag smuggling blocked: `--token`, `--credentials-file`, `--config`, `--config-dir`
- [x] `GOOGLE_WORKSPACE_CLI_*` env vars scrubbed from child process
- [x] `MIKA_GOOGLE_TOKEN` passed as `GOOGLE_WORKSPACE_CLI_TOKEN` to child
- [x] Missing token returns helpful error message
- [x] Skill templates created: `skill.toml`, `tools.json`, `system_prompt.md`
- [x] Skill registered in `KNOWN_BUILTINS` and `BUNDLED_SKILLS`
- [x] `google_token` added to `Settings` with redacted `Debug` + `ConfigKeyInfo`
- [x] `Dockerfile.agent` installs `gws` v0.13.3 with checksum verification
- [x] `.env.example` documents `MIKA_GOOGLE_TOKEN`
- [x] `CLAUDE.md` updated with new env var
- [x] All validation tests pass (`cargo test`)
- [x] No keyword overlap triggers with existing `calendar` skill

## Dependencies & Risks

- **gws CLI availability:** Pre-built binaries verified for v0.13.3 (linux amd64/arm64). Risk: upstream release format changes.
- **Google Discovery Service:** Required at runtime. Risk: network dependency for first call. Mitigation: 45s timeout.
- **Token management:** Users must obtain and refresh tokens externally. Risk: friction. Mitigation: clear docs + system prompt guidance.

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-13-google-workspace-skill-brainstorm.md](docs/brainstorms/2026-03-13-google-workspace-skill-brainstorm.md) — Key decisions: single run_gws tool, gmail/calendar/drive allowlist, env var token auth, pre-built Docker binary.
- **Reference implementation:** `crates/mika-agent/src/skills/builtin_handlers.rs` (run_gh handler, lines 224-399)
- **Skill templates:** `crates/mika-agent/templates/skills/github/` (skill.toml, tools.json, system_prompt.md)
- **Bundled registration:** `crates/mika-agent/src/bundled_skills.rs` (GITHUB_SKILL, BUNDLED_SKILLS)
- **gws CLI:** https://github.com/googleworkspace/cli (v0.13.3, Apache-2.0)
- **Learnings applied:** Tool name shadowing (`docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md`), keyword specificity (`docs/solutions/integration-issues/adding-prompt-only-bundled-skill.md`)
- GitHub issue: [#75](https://github.com/senara-solutions/mika/issues/75)
