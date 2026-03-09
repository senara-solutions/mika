# Dashboard Investigation Panel

**Date:** 2026-03-09
**Status:** Brainstorm complete

## What We're Building

A side panel in the observability dashboard that lets you ask questions about Mika's behavior — "Why did this agent use curl instead of web_search?" — and get answers from a dedicated investigation agent with read-only DB access.

**Two trigger levels:**
- **Tool call row**: A magnifying glass icon on each tool call row. Opens the panel pre-filled with context about that specific tool call (name, input, output, success/failure).
- **Message bubble**: An "Investigate" button on assistant messages. Opens the panel with the full message + all its tool calls as broader context.

Both open the same side panel — the only difference is how much context gets pre-loaded.

**The side panel** is a mini chat thread (like Datadog, Chrome DevTools, Langfuse). The main session content stays visible on the left while you ask questions on the right. Supports follow-up turns: "Why did run_shell return 404?" → "What URL was it trying?" → "Is there a skill for Wikipedia lookups?"

## Why This Approach

### Separate investigation agent (not one-shot Claude, not main agent loop)

- **Isolation**: Investigation doesn't pollute Mika's conversation history or lock the agent
- **Capability**: The investigator has read-only tools to query the DB — it can look up related sessions, check for recurring patterns, cross-reference with memory. A one-shot Claude call with no tools is just "explain this JSON to me"
- **Self-improvement foundation**: Today you trigger investigations manually from the dashboard. Tomorrow the self-check skill can use the same investigator to analyze performance automatically. Next month, reflection reads investigation findings to update behavior

### Implementation reuses existing infrastructure

The investigator is just another agent in `~/.mika/agents/investigator/`:

```
Agent: "investigator"
Soul: "You analyze Mika agent behavior. You have read-only access
      to the unified timeline, audit log, sessions, and memory."
Tools: query_timeline, query_audit_log, query_sessions, search_memory (read-only)
Memory: none (stateless per investigation)
```

Same agent loop, same DB, same server. No new architecture.

## Key Decisions

1. **Side panel UX** — not inline (clutters table) or modal (blocks data view). Side panel keeps the data you're investigating visible while you ask questions about it.

2. **Separate investigation agent** — not one-shot Claude (no tools = limited analysis) or main agent loop (pollutes history, locks agent). Dedicated agent with read-only DB tools.

3. **Both trigger levels** — per-tool-call icon for specific questions ("why did this fail?") and per-message button for broad questions ("was this the right approach?"). Same panel, different initial context.

4. **Stateless per investigation** — no persistent memory for the investigator. Each panel session is ephemeral. The investigator reads from the shared DB but writes nothing.

## Scope

### Backend (mika-server)
- New endpoint: `POST /api/v1/investigate` — accepts message_id or (message_id + tool_call_index), user question, optional conversation history for follow-ups
- Runs the investigator agent with pre-assembled context (the target message, its tool calls, surrounding messages from the session)
- Returns the investigator's response (synchronous — investigation should be fast with read-only tools)

### Investigation agent
- Registered as a built-in agent (not user-created) with a fixed soul
- Read-only tools: `query_timeline`, `query_sessions`, `query_messages`, `search_memory`
- No write tools, no skills, no MCP, no management tools
- Stateless — no session persistence, no compaction

### Dashboard (React)
- Side panel component (slide-out from right, ~40% width)
- Chat thread UI inside the panel (user question bubbles + investigator response bubbles)
- Two trigger points: icon on tool call rows, button on assistant message bubbles
- Context pre-loading based on trigger level
- Panel state managed locally (not persisted)

## Resolved Questions

1. **SSE streaming** — Stream tokens via server-sent events. Even though investigations should be fast, streaming gives real-time feedback and makes the panel feel responsive. Worth the complexity.

2. **Lazy creation** — Investigator agent created on first use, not at startup. No overhead if never used. No persistent agent entry cluttering the agents list.

3. **No rate limiting for now** — Single user, YAGNI. Add later if needed.
