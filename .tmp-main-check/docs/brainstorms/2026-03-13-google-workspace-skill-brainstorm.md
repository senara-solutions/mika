# Brainstorm: Google Workspace Builtin Skill

**Date:** 2026-03-13
**Issue:** [#75](https://github.com/senara-solutions/mika/issues/75)
**Status:** Ready for planning

## What We're Building

A builtin skill that enables the Mika agent to interact with Google Workspace services (Gmail, Calendar, Drive) using the [Google Workspace CLI (`gws`)](https://github.com/googleworkspace/cli). The skill follows the exact same pattern as the existing GitHub (`run_gh`) skill — a single `run_gws` builtin handler with command-as-array input and a subcommand allowlist.

## Why This Approach

- **Consistency:** Single-tool pattern mirrors `run_gh` exactly — same validation, same security model, same dispatch path. No new patterns to learn or maintain.
- **gws CLI is Rust-native:** No npm/Node dependency. Can be installed as a pre-built binary in Docker (like `gh`), keeping the image lean.
- **JSON-first:** `gws` outputs JSON by default, which the LLM can parse directly. Supports `--format json/table/yaml/csv`.
- **Wide coverage with minimal code:** The `gws` CLI auto-generates commands from Google's Discovery Service, so one tool gives access to all supported operations across Gmail, Calendar, and Drive.

## Key Decisions

### 1. Single `run_gws` tool (not per-service tools)
One builtin handler function dispatches all `gws` commands. The LLM decides which service to call based on the system prompt guidance. This keeps the tool count low and the codebase consistent.

### 2. Subcommand allowlist: `gmail`, `calendar`, `drive`
Only these three service names are permitted as the first element of the command array. Blocked: `auth`, `config`, and any other subcommands — same security posture as `run_gh` blocking `auth`/`api`/`config`.

### 3. Authentication via env var (`MIKA_GOOGLE_TOKEN`)
The skill reads `MIKA_GOOGLE_TOKEN` from Mika's environment and passes it as `GOOGLE_WORKSPACE_CLI_TOKEN` to the `gws` child process. This is the simplest approach for both Docker containers and local CLI usage. No `gws auth login` flow needed — token management is a setup concern, not an agent concern.

### 4. Pre-built binary for Docker
Download `gws` binary from GitHub Releases with checksum verification in `Dockerfile.agent` (same pattern as the `gh` CLI install step). No `cargo install` in Docker — keeps build times fast.

### 5. Local install via `cargo install`
For local (non-Docker) usage, users install `gws` via:
```
cargo install --git https://github.com/googleworkspace/cli --locked
```
The `mika doctor` command should check for `gws` in PATH (optional dependency, like `gh`).

## Implementation Scope

### Files to create
- `crates/mika-agent/templates/skills/google-workspace/skill.toml` — Skill manifest
- `crates/mika-agent/templates/skills/google-workspace/tools.json` — Single `run_gws` tool definition
- `crates/mika-agent/templates/skills/google-workspace/system_prompt.md` — LLM guidance with examples

### Files to modify
- `crates/mika-agent/src/skills/builtin_handlers.rs` — Add `run_gws` handler + `validate_gws_input` function
- `Dockerfile.agent` — Add `gws` binary install step (pre-built from GitHub Releases)
- `.env.example` — Add `MIKA_GOOGLE_TOKEN` documentation

### Handler design (mirrors `run_gh`)

```
validate_gws_input(input):
  1. Reject string command (must be array)
  2. Parse array elements
  3. Total length <= 10,000 chars
  4. First element must be in allowlist: ["gmail", "calendar", "drive"]
  5. Reject --token / --credentials-file flags in array (prevent smuggling)

run_gws(input, ctx):
  1. Validate input
  2. Build tokio::process::Command("gws")
  3. Set args from validated array
  4. Set env GOOGLE_WORKSPACE_CLI_TOKEN from MIKA_GOOGLE_TOKEN
  5. Scrub MIKA_* env vars
  6. Set GWS-specific env (if any)
  7. Kill on drop, null stdin, piped stdout/stderr
  8. Capture output, truncate to 10K chars
  9. Return ToolOutput
```

### System prompt content (key sections)
- Available services and their common operations
- Command format: `["gmail", "messages", "list", "--params", "{\"maxResults\": 5}"]`
- Helper commands: `["gmail", "+send", "--to", "user@example.com", "--subject", "Hello"]`
- `--format json` for structured output, `--page-limit` for pagination
- Destructive operation confirmation (send email, delete, etc.)
- Auth error handling guidance

### Trigger keywords
`google`, `gmail`, `email`, `calendar`, `schedule`, `meeting`, `event`, `google drive`, `gdrive`, `google docs`

### Skill settings
- `always_on = false` — Requires trigger keywords (same as GitHub)
- `timeout_secs = 30` — Default timeout

## Security Considerations

1. **Subcommand allowlist** — Only `gmail`, `calendar`, `drive`
2. **Flag smuggling prevention** — Reject `--token`, `--credentials-file`, `--config` in command array
3. **Env var scrubbing** — Remove all `MIKA_*` vars from child process
4. **Token isolation** — Only `GOOGLE_WORKSPACE_CLI_TOKEN` is explicitly set
5. **Output truncation** — 10,000 char max (matches `run_gh`)
6. **Input length limit** — 10,000 char total command array
7. **Destructive op guidance** — System prompt instructs confirmation before send/delete

## Resolved Questions

- **Single tool vs per-service?** Single `run_gws` — consistent with `run_gh` pattern.
- **Which services?** Gmail, Calendar, Drive initially.
- **Auth mechanism?** Env var token (`MIKA_GOOGLE_TOKEN` → `GOOGLE_WORKSPACE_CLI_TOKEN`).
- **Allow `auth` subcommand?** No — blocked entirely, same as GitHub skill.
- **Docker install method?** Pre-built binary from GitHub Releases with checksum verification.

## Open Questions

None — all design questions resolved.
