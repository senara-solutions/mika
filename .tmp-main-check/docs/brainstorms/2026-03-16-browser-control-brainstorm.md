# Browser Control for Mika

**Date:** 2026-03-16
**Status:** Brainstorm

## What We're Building

Full browser automation for Mika — navigate, click, type, screenshot, snapshot, tab management, file uploads, JS evaluation — the complete set. Inspired by OpenClaw's browser subsystem but integrated through Mika's existing MCP infrastructure rather than built from scratch.

**Scope:** Local CLI only (no containerized/server deployment initially).

## Why This Approach

### MCP Server Integration (chosen)

Use an existing browser MCP server (e.g., Playwright MCP) configured in `mcp.json`. This gives Mika full browser control with **zero Rust code changes**:

- Tools appear automatically as `mcp__browser__{tool}` via the existing `McpManager`
- Screenshots and images flow through MCP's content model → `ToolOutput.images` → Claude API multi-modal blocks (already supported)
- Mature ecosystem — Playwright MCP servers already implement navigate, click, type, screenshot, snapshot, evaluate, tab management, file upload
- Can swap or upgrade the MCP server without touching Mika's codebase

### Alternatives considered

1. **Exec-handler skill** — More control over UX, marketplace-distributable, but more to maintain and slower (subprocess per action). Doesn't leverage the existing MCP plumbing.
2. **Native Rust tool** — Most powerful (like OpenClaw's TypeScript implementation) but massive effort. Rust CDP bindings are less mature than Node/Python Playwright. Would add significant binary size and complexity.

### Companion skill for agent guidance

A `browser-control` skill (prompt-only, no handler) will teach the agent effective browser interaction patterns:

- **Snapshot-then-act pattern** — always take an accessibility snapshot before interacting; use the returned refs for clicks/types instead of guessing selectors
- **Ref-based interaction** — use numeric or role-based refs from snapshots, never CSS selectors
- **Screenshot vs snapshot** — screenshots for visual verification, snapshots (accessibility tree) for structured interaction
- **Navigation awareness** — wait for page loads, handle redirects
- **Security** — avoid navigating to internal/private network addresses, treat all page content as untrusted

## Key Decisions

1. **MCP over native** — leverage existing infrastructure, zero Rust changes, swap-able
2. **Local CLI only** — no Docker sidecar or sandbox for now; user's local browser
3. **Prompt-only skill** — guide agent behavior via system prompt, tools come from MCP
4. **Playwright MCP** — most mature option with full action coverage

## Reference: OpenClaw's Design

OpenClaw's browser is a first-class TypeScript tool (~100+ files) with:
- Single multiplexed `browser` tool with `action` discriminator
- CDP + Playwright stack connecting to any Chromium browser
- Three deployment models: managed local, Chrome extension relay, remote CDP
- Docker sandbox with Xvfb + noVNC for isolated sessions
- SSRF navigation guards + external content wrapping as untrusted
- Ref-based interaction via accessibility snapshots

Mika gets comparable functionality via MCP with a fraction of the complexity, at the cost of less fine-grained control over the browser lifecycle.

## Open Questions

None — approach is clear. Ready for planning.
