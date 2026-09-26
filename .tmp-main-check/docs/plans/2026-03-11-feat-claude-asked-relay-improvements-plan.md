---
title: "feat: Improve claude-asked relay for mika-dev agent"
type: feat
status: completed
date: 2026-03-11
origin: docs/brainstorms/2026-03-11-claude-asked-relay-improvements-brainstorm.md
---

# feat: Improve Claude-Asked Relay for Mika-Dev Agent

## Overview

Four focused improvements to the `claude-asked-relay` script and mika-dev's skills so she can effectively handle Claude Code questions during self-dev runs. Adds session threading (`--session` CLI flag), structured message context, context7 MCP access, and a decision framework for when to research vs. auto-approve vs. escalate.

## Problem Statement / Motivation

When Claude Code asks questions during self-dev runs, mika-dev receives them as isolated one-shot messages with no continuity, no research tools (no MCP configured), and a skill prompt that only covers tmux mechanics. She can't make informed decisions because:

1. Each question creates a fresh session — no visibility into prior Q&A from the same run
2. No context7 MCP — can't look up library documentation to answer technical questions
3. The skill prompt has no decision framework — just tmux key-press mechanics
4. The relay script doesn't pass the Claude Code session ID or structured metadata

(see brainstorm: docs/brainstorms/2026-03-11-claude-asked-relay-improvements-brainstorm.md, "What We're Building")

## Proposed Solution

Four changes that work independently but compound together:

1. **`mika ask --session <id>`** — reuse sessions across questions from the same Claude Code run
2. **Relay script enrichment** — extract session_id, pass `--agent mika-dev --session`, structured prefix
3. **MCP for mika-dev** — copy context7 config for library doc lookups
4. **Skill prompt upgrades** — decision framework + research guidance in `claude-tmux-relay`

(see brainstorm: "Why This Approach")

## Technical Approach

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Claude Code Hook Event (JSON on stdin)                 │
│  {session_id, cwd, tool_name, tool_input, ...}          │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  claude-asked-relay (bash script)                       │
│  • Extract session_id, project, event_id                │
│  • Filter: skip sub-agents, auto-approved, worktrees    │
│  • Format: [claude-asked|session:X|project:Y|event:Z]   │
│  • Serialize: flock to prevent concurrent agent calls   │
│  • Send: mika --agent mika-dev ask --session "$sid" msg │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  mika ask --session <id> (Rust CLI)                     │
│  • Use provided session or generate UUID                │
│  • Create session if not exists (warn-and-continue)     │
│  • Run full agent loop with history from all sessions   │
│  • MCP connected (context7 for doc lookups)             │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│  mika-dev agent loop                                    │
│  • claude-tmux-relay skill (always_on) activates        │
│  • Decision: auto-approve / research+answer / escalate  │
│  • Research via context7 MCP or web_search if needed    │
│  • Respond via tmux send-keys -t mika                   │
└─────────────────────────────────────────────────────────┘
```

### Implementation Phases

#### Phase 1: CLI — Add `--session` Flag

**Scope:** Minimal Rust change to support session reuse.

**Tasks:**

- [x] Add `--session` optional arg to `Ask` variant in `crates/mika-cli/src/cli.rs:42-49`
  ```rust
  Ask {
      message: String,
      #[arg(long)]
      task_id: Option<String>,
      /// Reuse an existing session (creates if not found)
      #[arg(long)]
      session: Option<String>,
  },
  ```
- [x] Update pattern match in `crates/mika-cli/src/main.rs:147` to pass the new field
  ```rust
  Some(Commands::Ask { message, task_id, session }) => {
      match commands::ask::run(&message, &agent_name, task_id.as_deref(), session.as_deref()).await {
  ```
- [x] Update `commands::ask::run` signature in `crates/mika-cli/src/commands/ask.rs:13` to accept `session: Option<&str>`
- [x] In `ask.rs`: use provided session ID if given, else generate UUID (current behavior)
  ```rust
  let session_id = session
      .map(|s| s.to_string())
      .unwrap_or_else(|| Uuid::new_v4().to_string());
  ```
- [x] Validate `--session` is non-empty when provided (bail if empty string)
- [x] Run `cargo test` — no test changes needed, existing tests use no flags

**Key insight from research:** `create_session` uses plain `INSERT` (not idempotent), but `ask.rs` wraps it in `if let Err(e)` with `tracing::warn` — so passing an existing session ID just logs a warning and continues. No change to `create_session` needed.

**Session scoping note:** The `--session` flag ensures messages are saved under a stable session ID for grouping and later introspection (via `get_session_messages` tool). The agent loop still loads history by agent_id (cross-session), which is correct — mika-dev benefits from seeing all recent context, not just relay messages. The session ID provides audit trail and the ability to manually look up prior Q&A from the same Claude Code run.

#### Phase 2: Relay Script — Enrichment and Hardening

**Scope:** Update `~/.local/bin/claude-asked-relay` to pass structured context.

**Tasks:**

- [x] Extract `session_id` from envelope after existing field extractions (line 22-25):
  ```bash
  session_id=$(field '.payload.session_id // ""')
  ```
- [x] Guard against empty session_id — generate fallback UUID:
  ```bash
  if [[ -z "$session_id" ]]; then
    session_id=$(uuidgen)
  fi
  ```
- [x] Update message formatting to use structured prefix. Replace the `$msg` construction in each case branch so the body is separate from the prefix. Extract the body into `$body`, then construct `$msg` as:
  ```bash
  msg="[claude-asked|session:${session_id:0:8}|project:$project|event:$event_id] $body"
  ```
  (Truncate session_id to first 8 chars in the display prefix for readability — full ID passed via `--session`)
- [x] Update the SEND section (line 151) to use explicit agent and session:
  ```bash
  flock -n /tmp/claude-relay.lock \
    mika --agent mika-dev ask --session "$session_id" "$msg" \
    || echo "[relay:$event_id] SKIP busy (flock)" >> /tmp/claude-hooks-debug.log
  ```
- [x] Add `flock` concurrency guard to prevent two relay invocations running simultaneously (the `mika ask` agent loop is not safe for concurrent runs against the same agent DB)
- [x] Verify `uuidgen` is available on the system (standard on Linux, part of `util-linux`)

**SpecFlow edge case resolved:** Empty `session_id` falls back to a generated UUID instead of creating a session with empty string ID. The `flock -n` (non-blocking) skips if another relay is already running, logging the skip. This prevents concurrent agent loops and DB contention.

#### Phase 3: MCP Configuration — Context7 for Mika-Dev

**Scope:** File copy, zero code changes.

**Tasks:**

- [x] Copy `/home/samidarko/.mika/agents/mika/mcp.json` to `/home/samidarko/.mika/agents/mika-dev/mcp.json`
- [x] Verify file permissions are 0600 (contains auth token)
- [ ] Test: `mika --agent mika-dev mcp list` should show context7 as enabled
- [ ] Test: `mika --agent mika-dev ask "use context7 to look up the rmcp crate documentation"` should successfully connect and query

**Alternative:** Use CLI: `mika --agent mika-dev mcp add context7 --transport http --url https://mcp.context7.com/mcp --header "Authorization=Bearer <token>"`. Either approach works; file copy is simpler.

#### Phase 4: Skill Prompt Upgrades

**Scope:** Text-only changes, zero code changes.

##### 4a. Upgrade `claude-tmux-relay/system_prompt.md`

- [x] Replace the current 27-line prompt with a comprehensive decision framework
- [x] New prompt structure:
  1. **Message format** — explain the structured `[claude-asked|session:...|project:...|event:...]` prefix
  2. **Decision framework** — three tiers with concrete examples:
     - **Auto-approve (Enter):** `ls`, `cat`, `grep`, `find`, `git status`, `git log`, `git diff`, `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt`, `npm run build`, read-only file operations
     - **Research + answer directly:** Technical questions about APIs, libraries, patterns, syntax, architecture decisions. Use `mcp__context7__resolve-library-id` → `mcp__context7__query-docs` for library-specific questions. Use `web_search` for broader technical questions. Research BEFORE answering.
     - **Escalate to Vincent via Telegram:** `rm`, `rm -rf`, `git push --force`, `git reset --hard`, `DROP TABLE`, `cargo publish`, any command that deletes data or is irreversible. Forward the full question with context.
  3. **Research workflow** — step-by-step: identify the library/API → resolve library ID → query docs → synthesize answer
  4. **Response mechanics** — tmux send-keys for menu navigation (keep existing arrow key guidance)
  5. **Session context** — note that prior Q&A from the same Claude Code run may be visible in conversation history; use this context to give better answers
  6. **Ambiguous cases** — when unsure whether a command is safe, treat it as research+answer (err on the side of caution without escalating everything)

##### 4b. Keep `web-search/system_prompt.md` generic

**SpecFlow resolution:** The web-search skill is a bundled template affecting all agents. All claude-asked-specific research guidance goes in the `claude-tmux-relay` skill prompt instead. The web-search prompt stays unchanged.

## Acceptance Criteria

- [ ] `mika --agent mika-dev ask --session test-123 "hello"` creates a session with ID `test-123` and returns a response
- [ ] Calling `mika --agent mika-dev ask --session test-123 "follow up"` reuses the same session (warn log, no crash)
- [ ] `mika --agent mika-dev ask --session "" "hello"` fails with a clear error (empty session guard)
- [ ] The relay script extracts `session_id` from the Claude Code hook envelope
- [ ] The relay script uses `flock` to prevent concurrent agent invocations
- [ ] The relay script falls back to `uuidgen` when `session_id` is missing from the envelope
- [ ] `mika --agent mika-dev mcp list` shows context7 as enabled
- [ ] The `claude-tmux-relay` skill prompt includes a three-tier decision framework
- [ ] The skill prompt includes research guidance for context7 and web_search
- [x] `cargo test` passes with no regressions
- [x] `cargo clippy` clean

## Dependencies & Risks

**Dependencies:**
- `uuidgen` (part of `util-linux`, standard on Gentoo)
- `flock` (part of `util-linux`, standard on Gentoo)

**Risks:**
- **Low:** `flock -n` drops concurrent hooks silently. If Claude Code fires two hooks in rapid succession, one is skipped. Acceptable — the skipped event is logged for debugging. The next iteration (work-item tracking) will handle queueing properly.
- **Low:** MCP connection latency. Each `mika ask` establishes a new HTTP connection to context7. If context7 is slow, the hook response is delayed. Claude Code is waiting for the answer. Acceptable for now — the agent can fall back to answering without research if MCP is unreachable (graceful degradation is built into the MCP system).
- **Low:** Session history is agent-scoped, not session-scoped. The `--session` flag provides grouping for audit/introspection but the agent sees all recent messages. This is actually better — mika-dev benefits from full context. The `get_session_messages` introspection tool can be used to look up prior Q&A from a specific session when needed.

## Sources & References

### Origin

- **Brainstorm document:** [docs/brainstorms/2026-03-11-claude-asked-relay-improvements-brainstorm.md](docs/brainstorms/2026-03-11-claude-asked-relay-improvements-brainstorm.md) — Key decisions carried forward: session threading via `--session` flag, explicit `--agent mika-dev`, research+answer directly (no escalation for technical questions), hardcoded tmux session name.

### Internal References

- CLI arg pattern: `crates/mika-cli/src/cli.rs:42-49` (`--task-id` as model for `--session`)
- Ask command: `crates/mika-cli/src/commands/ask.rs:13-156`
- Session creation: `crates/mika-agent/src/db.rs:1677-1683` (plain INSERT, not idempotent)
- Keyword matching: `crates/mika-agent/src/skills/matcher.rs:8-22` (substring-based, `claude-asked` matches in prefix)
- MCP connect in ask mode: `crates/mika-cli/src/commands/ask.rs:119` (`init::connect_mcp`)

### Institutional Learnings

- MCP HTTP headers integration: `docs/solutions/integration-issues/mcp-http-headers-cli-integration.md`
- Shell handler jq safety: `docs/solutions/integration-issues/shell-exec-jq-json-parsing.md`
- Env var scrubbing: `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md`
