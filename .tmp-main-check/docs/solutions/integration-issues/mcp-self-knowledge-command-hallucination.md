---
title: "MCP Self-Knowledge Gaps: Agent Suggests Nonexistent CLI Commands"
date: 2026-03-01
category: integration-issues
tags:
  - mcp
  - self-knowledge
  - skills
  - cli
  - hallucination
  - system-prompt
severity: medium
component: mika-agent
related_files:
  - templates/skills/self-knowledge/system_prompt.md
  - templates/skills/mcp/system_prompt.md
  - crates/mika-cli/src/commands/mcp.rs
---

# MCP Self-Knowledge Gaps: Agent Suggests Nonexistent CLI Commands

## Problem Statement

Mika suggested `mika mcp show context7` — a command that doesn't exist. The user
asked about MCP configuration and Mika confidently recommended a nonexistent
subcommand instead of using its self-knowledge tools to verify.

Additionally, `mika mcp list` didn't show whether headers were configured, making
it impossible for users to verify their MCP HTTP config without reading `mcp.json`
directly.

### Symptoms

- Mika suggests CLI commands that don't exist (e.g., `mika mcp show`)
- Users can't verify MCP header configuration via CLI
- Mika doesn't call `get_cli_reference` before suggesting commands

## Root Cause

Two independent issues:

### 1. Weak self-knowledge instructions

The self-knowledge skill prompt said "Never invent commands... always verify using
these tools first" but this was too gentle. The LLM treated it as a soft
suggestion rather than a hard requirement, leading to hallucinated commands.

### 2. MCP skill missing cross-references

When the MCP skill triggered (on keyword "mcp"), it provided CLI command
documentation but didn't instruct Mika to verify commands via `get_cli_reference`.
The static documentation could drift from reality.

## Solution

### 1. Strengthened self-knowledge system prompt

Made the instruction forceful and unambiguous:
- Changed from "Never invent" to "CRITICAL: NEVER suggest... without first calling"
- Added explicit instruction to call `get_cli_reference` BEFORE responding
- Added bad/good examples showing the exact anti-pattern to avoid

### 2. Updated MCP skill with self-knowledge reference

- Added "(these are the ONLY mcp subcommands — do not suggest any others)"
- Added instruction to use `get_cli_reference` for verification
- Added troubleshooting section for common MCP issues

### 3. Enhanced `mika mcp list` with header display

Added header key names (values never shown) to the list output:
```
MCP Servers (1):
  context7: http (https://mcp.context7.com/mcp) [enabled]
    headers: Authorization
```

## Key Insight

**LLM instruction strength matters.** Polite instructions like "never invent" are
treated as soft suggestions. Strong, direct instructions with concrete examples of
what NOT to do are much more effective at preventing hallucination. The combination
of:

1. **Explicit prohibition** ("NEVER suggest... without first calling")
2. **Concrete bad example** ("Suggesting `mika mcp show context7` without verifying")
3. **Concrete good example** ("Call `get_cli_reference` first, see that...")
4. **Cross-skill reinforcement** (MCP skill also says "do not suggest any others")

Creates multiple reinforcement points that make hallucination much less likely.

## Prevention

- Always use strong, unambiguous language in system prompts for safety-critical
  behaviors (tool verification, command accuracy)
- Cross-reference between related skills to reinforce important behaviors
- Provide concrete bad/good examples — LLMs respond better to examples than rules
- When users interact with config, provide CLI tools to verify state (like showing
  header keys in `mika mcp list`)

## Related

- [MCP Client Integration with rmcp](mcp-client-integration-rmcp.md) — initial MCP integration
- [MCP HTTP Headers and CLI Integration](mcp-http-headers-cli-integration.md) — HTTP headers support
- [MCP HTTP TLS Missing](mcp-http-tls-missing-rmcp.md) — TLS fix for HTTPS servers
