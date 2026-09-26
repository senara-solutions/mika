# Brainstorm: Claude-Asked Relay Improvements for Mika-Dev

**Date:** 2026-03-11
**Status:** Complete
**Author:** Sami / Claude

## What We're Building

Improve the `~/.local/bin/claude-asked-relay` script and mika-dev's skills so she can effectively handle Claude Code questions during self-dev runs. This is the first real feature mika-dev will monitor autonomously, and she needs: structured context in messages, session continuity across questions, research capabilities (MCP/web-search), and better decision-making guidance.

**The gap today:** The relay sends bare text with no session threading. Each question from Claude Code creates an isolated conversation. Mika-dev has no MCP servers (can't use context7 for docs), and her `claude-tmux-relay` skill prompt only covers tmux mechanics — no guidance on when to research vs. auto-approve vs. escalate.

## Why This Approach

Four focused improvements that work together, all achievable before work-item tracking lands:

1. **MCP for mika-dev** — copy context7 config so she can look up library docs
2. **Enrich relay envelope** — pass Claude Code session_id and project path as structured context
3. **Session threading via `--session` flag** — reuse sessions across questions from the same Claude Code run
4. **Upgraded skill prompts** — teach mika-dev when and how to research before answering

This approach was chosen because:
- Each piece is independently useful but they compound together
- Session threading is the minimal Rust change (new `--session` flag on `mika ask`)
- MCP config is a file copy — zero code changes
- Skill prompt upgrades are text-only — zero code changes
- Work-item tracking will build on this foundation (parent_task_id linking, etc.)

## Key Decisions

### 1. Session Threading — New `--session` CLI Flag

Add `--session <id>` to `mika ask`. The relay script passes the Claude Code `session_id` from the hook envelope. If the session already exists in mika-dev's DB, the agent sees prior messages from that session — giving continuity across multiple questions from the same Claude Code run.

In `ask.rs`: if `--session` is provided, use it instead of generating a new UUID. Call `create_session` which is idempotent (or check if it exists first).

### 2. Agent Targeting — Explicit `--agent mika-dev`

The relay script will use `mika --agent mika-dev ask ...` instead of bare `mika ask`. This makes the routing explicit and safe even if the active agent changes.

### 3. Tmux Session Name — Hardcoded 'mika'

The tmux session for Claude Code self-dev is always named `mika`. The skill can keep hardcoding `tmux send-keys -t mika`. Only revisit when supporting multiple concurrent dev sessions.

### 4. Decision Authority — Research + Answer Directly

Mika-dev uses web-search and context7 MCP to research technical questions, then answers them herself directly in the session. Only destructive operations (rm, force push, DROP TABLE) are escalated to Vincent via Telegram. She does NOT escalate routine technical decisions.

### 5. MCP Configuration — Copy from mika agent

Create `/home/samidarko/.mika/agents/mika-dev/mcp.json` with the same context7 config as the main mika agent. This gives mika-dev access to `mcp__context7__resolve-library-id` and `mcp__context7__query-docs` tools for looking up library documentation.

### 6. Relay Envelope — Structured Prefix

The relay script will include the Claude Code `session_id` in the message sent to mika-dev, formatted as a structured prefix that the skill can parse:

```
[claude-asked|session:<session_id>|project:<project>|event:<event_id>] <message body>
```

The `session_id` from the envelope is also passed as `--session` to thread the mika conversation.

## Changes Required

### Script: `~/.local/bin/claude-asked-relay`

1. Extract `session_id` from envelope: `session_id=$(field '.payload.session_id // ""')`
2. Include session_id in the structured prefix: `[claude-asked|session:$session_id|project:$project|event:$event_id]`
3. Pass `--session "$session_id"` and `--agent mika-dev` to `mika ask`
4. Final send line: `mika --agent mika-dev ask --session "$session_id" "[claude-asked|session:$session_id|project:$project|event:$event_id] $body"`

### CLI: `mika ask --session <id>` (Rust change)

1. Add `--session` optional arg to `Ask` variant in `cli.rs`
2. In `ask.rs`: use provided session ID if given, else generate UUID (current behavior)
3. Check if session exists before creating (idempotent)

### MCP: `~/.mika/agents/mika-dev/mcp.json`

Copy from `/home/samidarko/.mika/agents/mika/mcp.json` — context7 HTTP transport config.

### Skill: `claude-tmux-relay/system_prompt.md`

Major upgrade to the prompt:

- **Message parsing:** Explain the structured `[claude-asked|session:...|project:...|event:...]` prefix format
- **Decision framework:** Three tiers — auto-approve (safe read-only), research + answer (technical questions), escalate (destructive ops)
- **Research guidance:** When receiving a technical question, use `mcp__context7__resolve-library-id` + `mcp__context7__query-docs` for library questions, `web_search` for general technical questions. Research BEFORE answering.
- **Response mechanics:** Keep existing tmux send-keys guidance for menu navigation
- **Context awareness:** Remind that session threading means prior Q&A from the same Claude Code run is visible in conversation history

### Skill: `web-search/system_prompt.md`

Add guidance for technical context: when triggered by a claude-asked message, search for the specific library/API/pattern being asked about. Summarize findings concisely — the answer will be relayed back to Claude Code.

## Constraints

- No new Rust crates or tables — just a new CLI flag and session reuse logic
- MCP config is a file copy — no code changes
- Skill prompt changes are text-only — no code changes
- `--task-id` is NOT used yet (work-item tracking not landed)
- The `--session` flag is a stepping stone — work-item tracking will add `--parent-task` for richer threading

## Open Questions

None — all design questions resolved during brainstorming.
