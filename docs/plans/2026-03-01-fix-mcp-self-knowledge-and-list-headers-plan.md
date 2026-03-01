---
title: "fix: MCP self-knowledge gaps and list missing header display"
type: fix
status: completed
date: 2026-03-01
---

# fix: MCP self-knowledge gaps and list missing header display

## Overview

Mika suggests nonexistent CLI commands (e.g., `mika mcp show context7`) and
`mika mcp list` doesn't display whether headers are configured, making it
impossible for users to verify their MCP HTTP config without reading JSON directly.

## Problem Statement

1. **Hallucinated commands:** Mika suggested `mika mcp show context7` — a command
   that doesn't exist. The self-knowledge skill says "Never invent commands...
   always verify using these tools first" but the LLM still hallucinated.

2. **Missing header display:** `mika mcp list` output shows transport and
   enabled status but NOT whether headers are configured:
   ```
   MCP Servers (1):
     context7: http (https://mcp.context7.com/mcp) [enabled]
   ```
   Users told "The list doesn't show headers" and can't verify config.

3. **MCP skill doesn't reinforce self-knowledge:** When the MCP skill triggers
   (keyword "mcp"), it provides static CLI command docs but doesn't instruct
   Mika to verify commands via `get_cli_reference` first.

## Proposed Solution

### 1. Enhance `mika mcp list` to show header keys

**File:** `crates/mika-cli/src/commands/mcp.rs` — `list_servers()` function

Show header key names (values redacted) for HTTP transport servers:
```
MCP Servers (1):
  context7: http (https://mcp.context7.com/mcp) [enabled]
    headers: Authorization, X-Api-Key
```

### 2. Strengthen self-knowledge system prompt

**File:** `templates/skills/self-knowledge/system_prompt.md`

Make the instruction more forceful:
- Add explicit "NEVER suggest commands you haven't verified"
- Add instruction to call `get_cli_reference` BEFORE answering any CLI question
- Include examples of what NOT to do

### 3. Update MCP skill to reference self-knowledge

**File:** `templates/skills/mcp/system_prompt.md`

Add instruction: "For CLI commands, always verify with get_cli_reference first.
Do not suggest commands that are not in the reference."

## Acceptance Criteria

- [x] `mika mcp list` shows header key names (values redacted) for HTTP servers
- [x] Self-knowledge system prompt explicitly prohibits suggesting unverified commands
- [x] MCP skill system prompt references self-knowledge verification
- [x] `cargo test` passes
- [x] `cargo clippy` passes

## References

- `crates/mika-cli/src/commands/mcp.rs:38-66` — `list_servers()` function
- `templates/skills/self-knowledge/system_prompt.md` — Self-knowledge prompt
- `templates/skills/mcp/system_prompt.md` — MCP skill prompt
- `~/.mika/agents/main/data/mika.db` — Conversation showing hallucinated command
