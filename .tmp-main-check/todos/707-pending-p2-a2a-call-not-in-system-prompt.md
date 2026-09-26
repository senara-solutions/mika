---
status: pending
priority: p2
issue_id: 707
tags: [code-review, architecture]
dependencies: []
---

# a2a_call tool not documented in system prompt

## Problem Statement
The `a2a_call` tool is registered in `default_tools()` but the system prompt has zero references to "a2a" or "A2A". The agent has the tool available but no context explaining what A2A is, when to use it, or what remote agents might be available. The agent will only discover this tool from its JSON schema and will never proactively suggest using it.

## Findings
- `crates/mika-agent/src/prompt.rs`: Zero references to "a2a" or "A2A"
- The tool is registered and functional but the agent lacks context to use it effectively

## Proposed Solutions
Add A2A guidance to the system prompt explaining: (a) `a2a_call` sends messages to remote A2A-compatible agents, (b) any configured remote agent URLs, (c) the agent itself serves an A2A endpoint.

## Acceptance Criteria
- [ ] System prompt includes A2A tool guidance
