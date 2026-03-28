---
title: Memory Classification
description: Deterministic vs agent-triggered memory operation classification
---

# Memory Operation Classification

**Date:** 2026-03-28
**Origin:** [Memory-Aware Agents Course Analysis](brainstorms/2026-03-26-memory-aware-agents-course-analysis.md)

This document classifies every memory operation in Mika's agent architecture as either **deterministic** (engine-controlled, runs every turn without LLM involvement) or **agent-triggered** (the LLM decides when to invoke via tool call). This classification is the key architectural insight from the DeepLearning.AI "Memory-Aware Agents" course analysis.

## Why This Matters

The distinction determines what context the agent sees each turn (deterministic) versus what it must actively choose to retrieve or persist (agent-triggered). Understanding this classification helps when:

- Designing new memory features (should this be always-on or a tool?)
- Debugging context issues (the agent should have seen X -- is it deterministic or does it need to call a tool?)
- Evaluating compaction strategies (deterministic operations are affected by compaction; agent-triggered operations are not)

## Classification

### Deterministic Operations (Engine-Controlled)

These run automatically on every agent turn. The engine handles them in `prompt.rs`, `agent.rs`, and related infrastructure code. The LLM never decides whether they happen.

| Operation | Location | Trigger | Description |
|-----------|----------|---------|-------------|
| Core memory injection | `prompt.rs` `write_core_memory_section()` | Every turn | All 5 core memory blocks (user_summary, self_model, current_priorities, key_people, workflows) are injected into the system prompt as `<core-memory>` XML |
| Soul content injection | `prompt.rs` `write_soul_section()` | Every turn | Agent personality from `soul.md` loaded and prepended to system prompt |
| Skill prompt injection | `agent.rs` `inject_skills_and_resolve_tools()` | Every turn | Matched skills' prompts injected based on keyword triggers; `always_on` skills injected unconditionally |
| Context resolution | `skills/context.rs` `resolve_contexts()` | Every turn (when skills have `[context.*]`) | Deterministic data pre-fetch (e.g., `gh_pr_diff`) before LLM sees the query |
| Conversation history loading | `agent.rs` `load_recent_messages()` | Every turn | Full message history for the session loaded into the conversation |
| Conversation summary injection | `agent.rs` (post `build_system_prompt`) | Every turn (when summary exists) | Compaction summary injected as `<context type="summary" trust="data">` |
| Compaction summarization | `compaction.rs` `maybe_compact()` | Post-turn, when message count > 50 | Older messages summarized via LLM, summary stored, originals deleted |
| Tool call recording | `agent.rs` via `save_tool_call()` | After each tool execution | Full input/output (50KB cap) persisted to `tool_calls` table |
| LLM call recording | `agent.rs` via `save_llm_call()` | After each LLM API call | Model, tokens, latency, stop reason persisted to `llm_calls` table |
| Message persistence | `agent.rs` via `save_message()` | After each user/assistant message | Every message persisted to `messages` table with session FK |
| Audit event logging | Various write paths | On every memory mutation | Mutations logged to `audit_events` for rewind support |
| Tool history context | History builder in `agent.rs` | Every turn | `<context type="tool_history">` blocks appended to assistant messages showing prior tool call summaries |
| Step-awareness nudge | `agent.rs` | At tool step 8 of 10 | Injection encouraging the agent to wrap up (conversation mode only) |

### Agent-Triggered Operations (LLM-Decides)

These only happen when the LLM decides to call the corresponding tool. The engine provides the tools; the LLM decides when and how to use them.

| Operation | Tool | Category | Description |
|-----------|------|----------|-------------|
| Store structured fact | `store_fact` | Write | Persist a fact (person, commitment, preference, event) to the facts table |
| Update structured fact | `update_fact` | Write | Modify an existing fact |
| Edit core memory | `update_core_memory` | Write | Modify a specific core memory block (up to 500 tokens per block) |
| Search memory | `search_memory` | Read | Hybrid search (FTS5 + vector) across facts and search_content |
| Search tool history | `search_tool_history` | Read | Query past tool calls by name, keyword, time range |
| Query timeline | `query_timeline` | Read | Query unified timeline VIEW across all subsystems |
| Get session messages | `get_session_messages` | Read | Retrieve messages from a past session |
| List audit events | `list_audit_events` | Read | List memory mutation audit events |
| Send message | `send_message` | Write (silent mode) | Explicitly send a message to the user in silent/heartbeat mode |
| Create reminder | `create_reminder` | Write | Schedule a future task |
| Store skill | `create_skill` | Write | Create a new custom skill |

## Design Principles

1. **Deterministic operations handle the "what the agent always needs."** Core memory, soul, conversation history, and matched skills are injected every turn because the agent needs them to maintain continuity and personality.

2. **Agent-triggered operations handle the "what the agent might need."** Searching memory, recalling past tool results, and persisting new facts are situational -- the LLM judges when they're relevant.

3. **New write tools need query counterparts.** This is a codified convention (see CLAUDE.md). After compaction, conversation history may be summarized, so the agent cannot rely on memory of past tool calls -- it must verify current state through tools.

4. **Deterministic operations are subject to compaction.** When conversation history is compacted, the agent loses verbatim access to older messages. Deterministic injections (core memory, summary) bridge this gap by keeping the most important context always visible.

5. **Agent-triggered reads bypass compaction.** Tools like `search_memory`, `search_tool_history`, and `query_timeline` read directly from the database, not from conversation history. They provide a path to recall information that compaction may have summarized away.

## Context Priority

When information from these different sources conflicts, the system prompt establishes a priority ordering (see `prompt.rs` Instructions section):

> current user message > core memory > active skill context > conversation summary > conversation history > search results

This ordering reflects the trust hierarchy: the user's latest message is always ground truth, agent-curated core memory is actively maintained, and broader search results are the least curated.

## Related Documents

- [ADR-007: Session Conversation Compaction Strategy](adr/007-session-conversation-compaction-strategy.md)
- [Architecture: Three-Layer Memory Model](architecture.md)
- [ADR-003: Layer 3 Hybrid Vector Search](adr/003-layer3-hybrid-vector-search.md)
- [Solution: Agent Creates Duplicates After Compaction](solutions/logic-errors/agent-creates-duplicates-after-compaction.md)
- [Brainstorm: Memory-Aware Agents Course Analysis](brainstorms/2026-03-26-memory-aware-agents-course-analysis.md)
