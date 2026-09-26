---
title: "ADR-007: Session Conversation Compaction Strategy"
---

# ADR-007: Session Conversation Compaction Strategy

**Date:** 2026-03-28
**Status:** Proposed
**Components:** agent, compaction, prompt, memory

## Context

Mika sessions (interactive, team runs, multi-hour conversations) generate long conversation histories. The current compaction mechanism (see `compaction.rs`) uses a message-count threshold:

- **Trigger:** 50 messages in a session
- **Window:** Keep 20 most recent messages verbatim
- **Action:** Summarize older messages via Claude API, store as a summary message
- **Injection:** Summary appears as `<context type="summary" trust="data">` block in the system prompt

This works for Claude's 200K context window but has limitations:

1. **Not token-aware.** 50 messages of tool-heavy output may consume more tokens than 50 short exchanges. The threshold doesn't account for actual token usage.
2. **Multi-provider support.** Cheaper models (Groq, Ollama, Mistral) have 32K-128K context windows. The current threshold is too generous for smaller windows.
3. **Cost scaling.** Long sessions with rich core memory + skill prompts + tool history accumulate cost linearly with conversation length.
4. **Rewind compatibility.** The current compaction deletes old messages and stores a summary. Rewind (`rewind.rs`) operates on individual messages -- it can only rewind to messages that still exist. Compacted messages are irreversibly lost from the message stream.

The DeepLearning.AI "Memory-Aware Agents" course analysis (see `docs/brainstorms/2026-03-26-memory-aware-agents-course-analysis.md`) identified conversation compaction as Mika's most actionable architectural gap and validated the current hybrid approach as directionally correct.

## Strategies Evaluated

### Strategy A: Rolling Window Truncation

**Approach:** Keep the last N messages, discard older ones entirely. No summarization, no LLM call.

| Criterion | Assessment |
|-----------|------------|
| **Token cost** | Zero -- no extra LLM calls |
| **Information preservation** | Poor -- all context beyond the window is lost with no trace |
| **Rewind compatibility** | Fully compatible -- rewind only needs messages that exist, and all existing messages are unmodified |
| **Prompt caching impact** | Excellent -- stable message sequence, no volatile summary block |
| **Multi-provider support** | N can be tuned per provider's context window |
| **Implementation complexity** | Trivial -- SQL DELETE + window size config |

**Verdict:** Too lossy. The agent would have no record of earlier conversation context, leading to repeated questions and lost continuity in multi-hour sessions.

### Strategy B: LLM-Driven Summarization into Core Memory

**Approach:** Periodically summarize older messages and write the summary into core memory blocks (e.g., `user_summary`, `current_priorities`). Delete the original messages.

| Criterion | Assessment |
|-----------|------------|
| **Token cost** | Medium -- requires an LLM call for summarization, but reduces prompt size significantly |
| **Information preservation** | High for key facts -- the structured core memory format aligns with what the agent needs most. But lossy for conversational nuance, tool call context, and temporal ordering |
| **Rewind compatibility** | **Poor** -- summarization overwrites core memory blocks. Rewind tracks individual message changes via audit log, but core memory edits from summarization would be interleaved with agent-triggered edits, making rollback ambiguous |
| **Prompt caching impact** | Poor -- core memory changes every compaction cycle, invalidating the cached system prompt prefix |
| **Multi-provider support** | Good -- core memory is compact (~2500 tokens), works across all context windows |
| **Implementation complexity** | High -- requires structured LLM output matching core memory block format, conflict resolution with agent-edited blocks, new audit trail for compaction-driven edits |

**Verdict:** Elegant in theory but creates rewind and caching problems. The conflation of compaction-driven writes and agent-driven writes to core memory blocks makes the audit trail ambiguous.

### Strategy C: Hybrid -- Verbatim Window + Summary (Current Approach, Enhanced)

**Approach:** Keep the last N messages verbatim in the conversation history. Summarize older messages into a dedicated summary block injected into the system prompt. Store the summary as a message in the DB (current behavior). Original messages are deleted after summarization.

This is what Mika already does, with proposed enhancements:

1. **Token-aware trigger:** Add a `context_tokens_estimate()` function that approximates token usage before the LLM call. Trigger compaction when estimated tokens exceed a provider-specific threshold (e.g., 80% of context window), not just at 50 messages.
2. **Per-provider window sizing:** Different providers have different context windows. The `N` (verbatim window size) should be configurable per provider kind (e.g., 20 for Claude, 10 for smaller models).
3. **Summary stability for caching:** Place the volatile summary block at the end of the system prompt (after the stable prefix of soul + identity + time + core memory + instructions). This maximizes prompt cache hits on the stable prefix.
4. **Gap marker on compaction:** After compaction, inject a `role='system'` marker explaining what was summarized and how many messages were compressed. This prevents agent confabulation about the missing context (same pattern as the rewind context marker -- see `docs/solutions/architecture/rewind-context-marker-confabulation-prevention.md`).

| Criterion | Assessment |
|-----------|------------|
| **Token cost** | Low-medium -- one LLM call per compaction (amortized over ~30 messages per cycle) |
| **Information preservation** | Good -- recent messages are verbatim, older messages are summarized with tool name extraction. The summary already includes `[used: tool_a, tool_b]` suffixes via `extract_tool_names()` |
| **Rewind compatibility** | Compatible -- rewind operates on messages still in the verbatim window. Messages beyond the window are summarized and deleted, so they cannot be individually rewound, but the summary is a message itself that rewind can operate on |
| **Prompt caching impact** | Good with enhancement #3 -- summary at end of system prompt means the stable prefix (soul, identity, time, core memory, instructions) stays cached |
| **Multi-provider support** | Good with enhancement #2 -- window size and threshold adapt to provider context windows |
| **Implementation complexity** | Low -- enhancements are incremental on the existing `compaction.rs` |

**Verdict:** Best fit. Builds on the existing implementation with targeted enhancements.

## Decision

**Adopt Strategy C (Hybrid, Enhanced)** with the following implementation phases:

### Phase 1: Token-Aware Trigger (High Priority)
Add `context_tokens_estimate()` to `prompt.rs` that counts approximate tokens (using the 4 chars per token heuristic) for: system prompt + core memory + skill prompts + conversation history + tool definitions. Compare against a per-provider threshold. If above threshold, trigger compaction regardless of message count. Keep the 50-message count trigger as a fallback.

### Phase 2: Per-Provider Window Sizing (Medium Priority)
Add `compaction_window_size` and `compaction_threshold_ratio` to the per-provider config. Default: 20 messages / 80% for Claude, smaller for providers with smaller context windows. This becomes critical when multi-provider support is actively used.

### Phase 3: Summary Placement Optimization (Low Priority)
The summary is already injected after `build_system_prompt()` returns (in `agent.rs`). Verify it appears after the stable prefix. If not, reorder to maximize cache hit rate. This is a no-op if the current placement already achieves this (likely does -- the summary is appended last).

### Phase 4: Gap Marker on Compaction (Low Priority)
After `replace_with_summary()`, inject a `role='system'` message: "The conversation was compacted. [N] older messages were summarized. The summary above captures key decisions and actions. Do not attempt to recall details from the compacted messages." Follow the same pattern as `rewind_context_marker`.

## Consequences

- **Positive:** Multi-provider support unblocked, cost reduction for long sessions, cache-friendly prompt structure, established pattern for future compaction enhancements.
- **Negative:** Token estimation adds a small overhead per turn (character counting, not LLM call). Per-provider config adds complexity to `Settings`.
- **Risk:** Token estimation by character counting is imprecise. Mitigate by using a conservative 80% threshold (buffer for estimation error).

## Future Considerations

### Dynamic Tool Subsetting
When total tools per turn (builtins + skill tools + MCP tools) regularly exceed ~50, the tool definitions themselves become a significant portion of the context window. At that point, revisit dynamic tool subsetting: categorize builtin tools into "always" vs. "on-demand" groups, only inject on-demand tools when the query matches keywords. The skill system's keyword matching already does this for skill tools. Monitor via the `llm_calls` table, which records token counts per turn.

### JIT Summary Expansion
The course proposes an `expand_summary()` tool that lets the agent recover detail from compressed summaries on demand. This is a natural extension if the summary proves too lossy for certain use cases (e.g., multi-day team sprints). Implementation: store the original messages in a `compacted_messages` table with a `summary_id` foreign key, add a `read_compacted_messages` builtin tool.

### Entity Extraction Post-Loop
Automatic extraction of people, commitments, and preferences from agent responses (currently agent-triggered via `store_fact`). Deferred until Mika Prime runs multi-day autonomous sprints where manual fact storage is insufficient.
