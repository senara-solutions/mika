---
title: "feat: Implement memory-aware agent patterns"
type: feat
status: completed
date: 2026-03-28
origin: docs/brainstorms/2026-03-26-memory-aware-agents-course-analysis.md
issue: 304
---

# Implement Memory-Aware Agent Patterns

## Overview

Implement five prioritized improvements from the DeepLearning.AI "Memory-Aware Agents" course analysis. These patterns address gaps in Mika's context management: explicit conflict resolution semantics in the system prompt, a compaction strategy ADR for future implementation, a `search_tool_history` builtin tool for cross-session tool output recall, and documentation of the deterministic vs. agent-triggered memory classification.

The brainstorm (see origin) validates that Mika's architecture already embodies the right abstractions. The implementation focuses on closing identified gaps rather than restructuring.

## Proposed Solution

Five items ordered by implementation priority:

### Phase 1: Context Priority Semantics in System Prompt

**What:** Add a conflict-resolution paragraph to the `## Instructions` section of `build_system_prompt()` in `prompt.rs`.

**Priority rule:**
> When information from different sources conflicts, prefer in this order: current user message > core memory > active skill context > conversation summary > conversation history > search results.

**Placement:** Inside the `## Instructions` section (after the existing behavioral rules), keeping it within the prompt-cached region. Conversation-mode only — silent/heartbeat prompts have a different structure and the priority ordering is primarily relevant to user-facing conversation.

**Why this ranking:**
- Current user message is always ground truth (the user just said it)
- Core memory is agent-curated and actively maintained
- Active skill context is deterministically injected for the current turn
- Conversation summary is a lossy derivative — ranked below live context but above raw history
- Conversation history may contain outdated information
- Search results are the broadest, least-curated source

**Files:**
- `crates/mika-agent/src/prompt.rs` — add paragraph to `write_instructions_section()`

**Acceptance criteria:**
- [x] Priority paragraph appears in conversation-mode system prompt
- [x] Compaction summaries are explicitly addressed in the ranking (not ambiguous between "conversation history" and "search results")
- [x] Placement is after existing instruction bullets, inside the cached prompt region
- [x] Unit test verifies the paragraph appears in built system prompt output

### Phase 2: `search_tool_history` Builtin Tool

**What:** A read-only tool that queries the `tool_calls` table, enabling agents to recall prior tool results without re-running external calls.

**Parameters:**
| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `tool_name` | `String` | No | — | Filter by exact tool name |
| `keyword` | `String` | No | — | Search in input/output fields (SQLite `LIKE '%keyword%'`) |
| `from` | `String` | No | — | ISO 8601 start time |
| `to` | `String` | No | — | ISO 8601 end time |
| `success` | `bool` | No | — | Filter by success/failure |
| `limit` | `u32` | No | 20 | Max results (capped at 50) |

**Design decisions:**

1. **Agent scoping:** Non-orchestrator agents automatically scoped to own `agent_id` (same pattern as `query_timeline` and `list_audit_events`). Orchestrators can see all agents' tool calls.

2. **Keyword search:** Use `LIKE '%keyword%'` on `input` and `output` columns. This matches existing codebase patterns and avoids introducing a new FTS5 virtual table. Acceptable for the `tool_calls` table size (30-day retention window).

3. **Output truncation:** Each result's `input` and `output` fields truncated to 500 chars. Total output capped at 10KB. Include a `total_matches` count so the agent knows if results were truncated. This follows the established truncation-at-injection pattern (see brainstorm: callback-result-too-large solution).

4. **Retention awareness:** Tool description includes "Tool call history is retained for 30 days." so the LLM knows the data boundary.

5. **`store_tool_calls` disabled:** Return empty results with no special handling. The config check would require threading `Settings` into `ToolContext`, which is a larger architectural change not warranted here.

6. **Mode availability:** Registered in `default_tools()`, available in all modes including silent/heartbeat. Low risk — the tool is read-only and the heartbeat agent may legitimately need to check past tool outcomes.

**Implementation approach:**

1. Add `keyword: Option<String>` to `ToolCallFilters` in `db.rs`. Update `query_tool_calls()` SQL to include `AND (input LIKE ? OR output LIKE ?)` when keyword is present.

2. Add `AsyncDatabase::search_tool_history()` wrapper that:
   - Accepts `ToolCallFilters` + limit
   - Automatically injects `agent_id` for non-orchestrator scoping
   - Returns `Vec<ToolCallRow>` + total count

3. Create `tools/search_tool_history.rs`:
   - Standard input validation (empty check, 10K char max on keyword)
   - Construct `ToolCallFilters` from parameters
   - Call async DB method
   - Format results with truncated input/output (500 chars each)
   - Cap total output at 10KB with truncation notice

4. Register in `default_tools()` alongside other introspection tools.

**Files:**
- `crates/mika-agent/src/tools/search_tool_history.rs` — new tool implementation (~100 lines)
- `crates/mika-agent/src/tools/mod.rs` — add `mod search_tool_history;` + register in `default_tools()`
- `crates/mika-agent/src/db.rs` — add `keyword` to `ToolCallFilters`, update `query_tool_calls()` SQL
- `crates/mika-agent/src/async_db.rs` — add async wrapper method
- `crates/mika-agent/src/prompt.rs` — add tool usage guidance in `## Tool Usage` section

**Acceptance criteria:**
- [x] Tool returns results filtered by tool_name, keyword, time range, success status
- [x] Non-orchestrator agents cannot see other agents' tool calls
- [x] Each result's input/output truncated to 500 chars
- [x] Total output capped at 10KB
- [x] Default limit 20, max limit 50
- [x] Tool description mentions 30-day retention
- [x] Unit tests: empty table, keyword search, agent_id scoping, combined filters, output truncation

### Phase 3: ADR-007 — Session Conversation Compaction Strategy

**What:** Architecture Decision Record evaluating three compaction strategies with a recommendation.

**Current state:** Mika has message-count-based compaction (50-message threshold, keeps 20 recent, LLM summarization, summary injected as `<context type="summary">` block). This works but is not token-aware.

**Strategies to evaluate:**

| Strategy | Description | Pros | Cons |
|----------|-------------|------|------|
| **Rolling window truncation** | Keep last N messages, discard older | Simple, predictable, no LLM cost | Information loss, no recall of old context |
| **LLM-driven summarization** | Summarize older messages into core memory blocks or dedicated summary table | High fidelity, structured output aligned with core memory blocks | Extra LLM call cost, latency, lossy |
| **Hybrid** (recommended) | Recent N messages verbatim + LLM summary of older messages (current approach enhanced) | Balances fidelity and cost | More complex, dual storage |

**Evaluation criteria:**
1. **Token cost** — extra LLM calls, prompt size reduction
2. **Information preservation** — fidelity of retained context
3. **Rewind compatibility** — current rewind operates on individual messages; summarization-into-core-memory would change this contract
4. **Prompt caching impact** — volatile summary content at end of system prompt maximizes cache hits on stable prefix
5. **Multi-provider support** — smaller context windows (e.g., 32K for some providers) make compaction critical
6. **Backward compatibility** — schema v16 changes, migration path

**Files:**
- `docs/adr/007-session-conversation-compaction-strategy.md` — new ADR

**Acceptance criteria:**
- [x] Three strategies evaluated with pros/cons
- [x] Rewind compatibility analyzed for each strategy
- [x] Prompt caching impact considered
- [x] Recommendation with rationale
- [x] Follows existing ADR format (see `docs/adr/001-*.md`)

### Phase 4: Memory Classification Documentation

**What:** Architecture doc classifying each memory operation as deterministic (always runs, engine-controlled) or agent-triggered (LLM decides via tool call).

**Classification:**

| Operation | Type | Mechanism | Rationale |
|-----------|------|-----------|-----------|
| Core memory injection | Deterministic | `prompt.rs` system prompt assembly | Always-on context, agent sees it every turn |
| Skill prompt injection | Deterministic | `inject_skills_and_resolve_tools()` keyword matching | Engine matches and injects without LLM involvement |
| Conversation history loading | Deterministic | `load_recent_messages()` in agent loop | Full history replayed each turn |
| Compaction summarization | Deterministic | `maybe_compact()` post-turn, count-based | Engine triggers at threshold, not agent choice |
| Tool call recording | Deterministic | `store_tool_call()` fire-and-forget | Every tool execution recorded automatically |
| LLM call recording | Deterministic | `store_llm_call()` fire-and-forget | Every LLM call recorded automatically |
| Message persistence | Deterministic | `save_message()` in agent loop | Every message persisted automatically |
| Audit event logging | Deterministic | Various memory write paths | Mutations logged automatically |
| `store_fact` | Agent-triggered | Tool call | Agent decides when to persist facts |
| `update_fact` | Agent-triggered | Tool call | Agent decides when to update facts |
| `update_core_memory` | Agent-triggered | Tool call | Agent decides when to edit core memory |
| `search_memory` | Agent-triggered | Tool call | Agent decides when to search |
| `send_message` | Agent-triggered | Tool call (silent mode) | Agent decides when to message user |

**Files:**
- `docs/memory-classification.md` — new architecture doc

**Acceptance criteria:**
- [x] Every memory operation classified with rationale
- [x] Clear distinction between engine-controlled and agent-controlled operations
- [x] References to relevant source files

### Phase 5: Tool Count Growth Awareness

**What:** No code change. Document the monitoring threshold in ADR-007 as a future consideration: when total tools per turn (builtins + skill + MCP) regularly exceed ~50, revisit dynamic tool subsetting. The `llm_calls` table already records the data needed for monitoring.

**Acceptance criteria:**
- [x] Mentioned as "Future Considerations" section in ADR-007

## System-Wide Impact

### Interaction Graph
- **Phase 1:** `build_system_prompt()` → system prompt text → LLM API call. No side effects beyond prompt content.
- **Phase 2:** `search_tool_history` tool → `AsyncDatabase::search_tool_history()` → `Database::query_tool_calls()` → SQLite `tool_calls` table. Read-only, no state mutation.

### Error Propagation
- **Phase 2:** DB query errors → `anyhow::Result` → tool error message to agent. No retry needed for read-only queries.

### State Lifecycle Risks
- None. Phase 2 is read-only. Phase 1 is prompt-only. Phases 3-5 are documentation.

### API Surface Parity
- The `search_tool_history` tool is agent-facing only. Dashboard API already has `GET /api/v1/tool-calls` with equivalent filtering — no parity change needed.

### Prompt Caching Impact
- Phase 1 adds ~50 tokens to the Instructions section. This is within the cached region and does not shift the cache breakpoint. Cache hit rate should be unaffected since the paragraph is static content.
- Phase 2 adds one tool definition (~150 tokens). Tool definitions are cached via the last-tool breakpoint. Adding a tool shifts the breakpoint but doesn't reduce overall cache efficiency.

## Dependencies & Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `LIKE '%keyword%'` slow on large `tool_calls` table | Low (30-day retention caps size) | Medium (slow tool response) | Monitor; add FTS5 virtual table if needed later |
| Context priority paragraph ignored by LLM | Low | Low (no worse than today) | Test with conflicting sources; iterate on wording |
| ADR-007 compaction strategy chosen prematurely | Medium | Low (ADR, not implementation) | Mark as "Proposed" status, not "Accepted" |

## Sources & References

### Origin
- **Brainstorm document:** [docs/brainstorms/2026-03-26-memory-aware-agents-course-analysis.md](docs/brainstorms/2026-03-26-memory-aware-agents-course-analysis.md) — Key decisions: (1) context priority semantics is the highest-value/lowest-effort improvement, (2) `search_tool_history` closes the cross-session tool recall gap, (3) conversation compaction is the most actionable architectural gap

### Internal References
- `crates/mika-agent/src/prompt.rs` — system prompt assembly
- `crates/mika-agent/src/tools/query_timeline.rs` — agent scoping pattern for introspection tools
- `crates/mika-agent/src/db.rs:ToolCallFilters` — existing filter struct for tool_calls queries
- `crates/mika-agent/src/compaction.rs` — current compaction implementation
- `docs/adr/001-axum-http-server-architecture.md` — ADR format reference
- `docs/solutions/runtime-errors/callback-result-too-large-causes-agent-timeout.md` — truncation-at-injection pattern
- `docs/solutions/logic-errors/tool-calls-metadata-tail-drop-loses-entries.md` — budget math pattern
- `docs/solutions/architecture/rewind-context-marker-confabulation-prevention.md` — gap markers when removing context

### Related Work
- Issue: #304
- Branch: `docs/memory-aware-agents-brainstorm` (brainstorm origin)
