# Memory-Aware Agents Course Analysis

**Date:** 2026-03-26
**Source:** DeepLearning.AI — "Memory-Aware Agents" (4 notebooks)
**Goal:** Extract patterns adaptable to Mika's Rust/SQLite agent architecture

---

## Notebook 1: Constructing the Memory Manager

### 1. Core concept

The notebook introduces a **7-type memory taxonomy** for AI agents: conversational, knowledge base, workflow, toolbox, entity, summary, and tool log. Each type maps to a distinct storage backend (SQL table for exact-match retrieval, vector store for semantic retrieval) and is orchestrated through a single `MemoryManager` class. The key insight is the **deterministic vs. agent-triggered classification**: some memory ops must run every turn (reading conversation history, writing messages) while others require LLM judgment (summarization, entity extraction).

### 2. What they use vs. what we'd use

| Their approach | Mika equivalent |
|---|---|
| Oracle DB + OracleVS vector store | SQLite + `search_content` (FTS5) + `search_embeddings` (sqlite-vec) |
| `MemoryManager` class (Python) | `db.rs` functions + `ToolContext` (already unified) |
| `StoreManager` with 5 vector stores | Single `search_content` FTS5 table with `source_type` column |
| `CONVERSATIONAL_MEMORY` SQL table | `messages` table (session_id + agent_id scoped) |
| `TOOL_LOG_MEMORY` SQL table | `tool_calls` table (v15+, with `llm_call_id` correlation) |
| `ENTITY_MEMORY` vector store | `people` table (structured) + `search_content` (FTS5) |
| `WORKFLOW_MEMORY` vector store | No direct equivalent — workflows are implicit in `tool_calls` history |
| `SUMMARY_MEMORY` vector store | No direct equivalent — context compaction not yet implemented |
| `TOOLBOX_MEMORY` vector store | No equivalent — tools are statically registered via `ToolRegistry` |
| HuggingFace embeddings (768-dim) | OpenAI embeddings (optional, via `embedding_client`) |
| 7 separate tables/stores | Unified schema with typed columns (more normalized) |

### 3. Adaptable patterns (ranked by value/effort)

**P1: Deterministic vs. agent-triggered memory classification**
- **What it is** — Explicitly classifying each memory operation as "always run" or "LLM decides."
- **Why it matters for Mika** — Mika's core memory reads are already deterministic (injected into system prompt every turn), but workflow/entity writes and summarization are ad hoc. Formalizing this classification prevents the "chicken-and-egg" problem where the agent can't choose to look up what it doesn't know exists.
- **How to adapt** — Document which memory operations are deterministic in `prompt.rs` context assembly vs. which are tool-callable. No code change needed — this is an architectural awareness pattern that validates Mika's existing design.
- **YAGNI check** — Active problem: YES. Mika already does this implicitly; making it explicit improves reasoning about what context the agent sees each turn.

**P2: Workflow memory (tool execution pattern recall)**
- **What it is** — Storing successful tool execution sequences as searchable patterns so the agent can recall "last time I did X, the steps were Y."
- **Why it matters for Mika** — Mika already has `tool_calls` and `tool_history` context blocks, but these are per-session. Cross-session workflow recall (e.g., "how did I handle a PR review last week?") doesn't exist.
- **How to adapt** — A new `workflow_patterns` table or entries in `search_content` with `source_type = 'workflow'`. The agent loop's step 6 (persist learning) would write a condensed workflow summary after multi-tool sequences. Retrieval via FTS5 + existing `search_memory` tool.
- **YAGNI check** — Speculative for now. Mika Prime's sprint management could benefit, but there's no active bug or feature request. **Defer.**

**P3: Tool log as a first-class memory store (not just observability)**
- **What it is** — Writing full tool outputs to a persistent store and giving the agent a tool to retrieve past outputs (not just the truncated version in chat context).
- **Why it matters for Mika** — Mika's `tool_calls` table (v15+) already stores inputs/outputs for observability. But the agent has no tool to query past tool results across sessions. JIT retrieval of prior tool outputs would prevent redundant API calls.
- **How to adapt** — Add a `search_tool_history` builtin tool that queries `tool_calls` table by tool_name/keyword/time range. Minimal: a SQL query wrapper exposed as a tool.
- **YAGNI check** — Moderate value. Useful when the agent re-encounters the same task across sessions. Low effort since the data already exists.

### 4. Reject list

| Idea | Why reject |
|---|---|
| **Separate vector stores per memory type** | Mika's `search_content` + `source_type` achieves the same with less complexity. Multiple stores = multiple indexes = more maintenance. |
| **`StoreManager` class with getter methods** | Over-abstraction. Mika's `db.rs` is a flat module with direct functions — simpler, no indirection. |
| **Oracle HNSW vector indexes** | Mika uses sqlite-vec with flat cosine search. HNSW would require a new dependency and is only useful at scale (10K+ vectors). Mika's per-agent DB is small. |
| **`DistanceStrategy` configuration** | Premature abstraction. Cosine distance is fine for all Mika use cases. No need for configurable distance metrics. |
| **Python class hierarchy (StoreManager → MemoryManager → Toolbox)** | Three layers of wrapping. Mika's flat Rust modules with `ToolContext` are simpler and more explicit. |

### 5. Proposed ADR candidates

None from this notebook alone — the patterns either validate existing Mika design or are deferred.

---

## Notebook 2: Scaling Agent Tool Use with Semantic Tool Memory

### 1. Core concept

When an agent has hundreds of tools, passing all tool definitions to the LLM creates context bloat, selection confusion, and higher costs. The solution is **semantic tool retrieval**: store tool metadata (augmented docstrings + synthetic queries) as embeddings, then at inference time vector-search for the top-K tools relevant to the current query. The `Toolbox` class uses an LLM to enrich tool docstrings for better retrieval separability.

### 2. What they use vs. what we'd use

| Their approach | Mika equivalent |
|---|---|
| `Toolbox` class with vector store | `ToolRegistry` (static, all tools always available) |
| LLM-augmented docstrings for embedding | Tool `description` field in `ToolDefinition` |
| Semantic similarity search for tool selection | No equivalent — all registered tools are sent to Claude every turn |
| `@toolbox.register_tool(augment=True)` decorator | `impl Tool for X` trait + `default_tools()` registration |
| `read_toolbox()` as a meta-tool | No equivalent |
| Synthetic query generation for each tool | No equivalent |
| Top-K tool retrieval per query | Skill keyword matching (`triggers.keywords`) is the closest analog |

### 3. Adaptable patterns (ranked by value/effort)

**P1: Keyword-matched skill → focused tool set (already exists)**
- **What it is** — Mika's skill system already does a version of this: skills are matched by keyword triggers, and only matched skills' tools are injected into the turn.
- **Why it matters for Mika** — Validates the existing design. Skills with `triggers.keywords` act as a curated, deterministic version of semantic tool retrieval.
- **How to adapt** — No change needed. This confirms the skill system is the right abstraction.
- **YAGNI check** — Already solved.

**P2: Dynamic tool subsetting for large tool registries**
- **What it is** — When 30+ tools are registered (builtins + skill tools + MCP tools), only send the most relevant subset to the LLM per turn.
- **Why it matters for Mika** — Mika currently sends ~30 builtin tools + matched skill tools + MCP tools. As MCP servers and skills grow, this could become context-expensive. Claude handles large tool sets well, but token cost scales linearly.
- **How to adapt** — Two approaches: (a) categorize builtin tools into "always" vs. "on-demand" groups, only inject on-demand when the query matches; (b) semantic search over tool descriptions using existing FTS5. The skill system's keyword matching already does (a) for skill tools.
- **YAGNI check** — Not yet a problem. Mika has ~30 builtins + a few skill tools. Monitor tool count; act when it exceeds ~50. **Defer.**

**P3: LLM-augmented tool descriptions**
- **What it is** — Using an LLM to rewrite terse tool descriptions into richer ones that improve retrieval quality.
- **Why it matters for Mika** — If/when semantic tool selection is implemented, richer descriptions would help. But today tool descriptions are consumed directly by Claude, which handles them fine.
- **How to adapt** — N/A until dynamic tool subsetting is implemented.
- **YAGNI check** — Speculative. **Reject for now.**

**P4: `read_toolbox` as a meta-tool (tool discovery)**
- **What it is** — Giving the agent a tool to search for other tools it doesn't currently have loaded.
- **Why it matters for Mika** — In a multi-agent team, one agent might not know about tools available in another agent's skill set. A meta-tool could search across the skill marketplace.
- **How to adapt** — A `discover_skills` tool that searches `mika-skills/` manifests by description. Low priority.
- **YAGNI check** — Speculative. Mika teams already share tools via delegation. **Defer.**

### 4. Reject list

| Idea | Why reject |
|---|---|
| **Vector-based tool retrieval replacing static registration** | Mika's tool count is small (~30-40). Semantic retrieval adds latency and complexity for minimal benefit at this scale. Skill keyword matching is a better fit. |
| **Embedding every tool description** | Overhead of maintaining embeddings for tools that change rarely. Static `ToolDefinition` descriptions work fine with Claude. |
| **Synthetic query generation per tool** | Over-engineered for Mika's scale. Useful if you have 500+ tools; Mika won't reach that. |
| **`augment=True` decorator pattern** | Rust doesn't have Python decorators. Mika's `impl Tool` trait is more explicit and type-safe. |
| **Search-and-store pattern (auto-persist tool results to knowledge base)** | Mika's `tool_calls` table already persists all results. Auto-promoting to searchable knowledge is dangerous — it conflates ephemeral results with curated knowledge. |

### 5. Proposed ADR candidates

None — the key insight (semantic tool selection) is already handled by Mika's skill keyword matching system, and full semantic retrieval is premature at current tool counts.

---

## Notebook 3: Memory Operations — Extraction, Consolidation, and Self-Updating Memory

### 1. Core concept

Long-running conversations consume the context window. This notebook builds a **context compaction pipeline**: monitor token usage → when above threshold, summarize conversation into structured sections (technical info, emotional context, entities, action items) → store summary with a backreference ID → mark original messages as "summarized" → provide an `expand_summary()` tool for JIT retrieval of originals. The key mechanism is **self-updating memory**: summarized messages are tagged with `summary_id` to prevent re-processing.

### 2. What they use vs. what we'd use

| Their approach | Mika equivalent |
|---|---|
| `calculate_context_usage()` token monitor | No equivalent — Mika doesn't monitor context window fill |
| `summarise_context_window()` LLM summarizer | No equivalent — Mika has no conversation summarization |
| `offload_to_summary()` compaction policy | No equivalent |
| `expand_summary()` JIT retrieval tool | No equivalent |
| `summary_id` column on messages | No equivalent — `messages` table has no summarization tracking |
| 80% threshold trigger | No equivalent |
| Structured summary format (4 sections) | No equivalent |
| `SUMMARY_MEMORY` vector store | No equivalent table |

### 3. Adaptable patterns (ranked by value/effort)

**P1: Context window monitoring**
- **What it is** — Tracking how much of the context window is consumed before each LLM call and taking action (summarization, pruning) when above a threshold.
- **Why it matters for Mika** — Mika's agent loop has `MAX_TOOL_STEPS = 10` as a hard limit but doesn't monitor token usage. Long conversations with rich core memory + skill context could approach Claude's limits, especially with smaller models or when multi-provider support lands.
- **How to adapt** — Add a `context_tokens_estimate()` function in `prompt.rs` that counts approximate tokens before calling the LLM. Log it to `llm_calls.input_tokens` (already tracked post-call). Use it to warn or trigger compaction.
- **YAGNI check** — Moderate. Not an active bug today (Claude's 200K context is generous), but becomes critical with cheaper/smaller models. **Flag for multi-provider milestone.**

**P2: Conversation summarization with backreference**
- **What it is** — Compressing old conversation turns into structured summaries while maintaining a link back to originals for JIT expansion.
- **Why it matters for Mika** — Mika sessions can get long (especially team runs). The `messages` table grows, and the system prompt rebuilds full conversation history each turn. Summarization would reduce prompt size and cost.
- **How to adapt** — Two options: (a) Add `summary_id` column to `messages`, create a `summaries` table, implement `summarize_session` as a builtin tool. (b) Simpler: use the existing `core_memory.user_summary` block as a rolling summary and truncate old messages from the prompt (keep them in DB for audit). Option (b) is closer to Mika's existing design.
- **YAGNI check** — Active adjacent problem: Mika Prime runs multi-hour sessions. Conversation compaction would reduce costs. **Worth an ADR.**

**P3: Self-updating memory (mark-as-processed pattern)**
- **What it is** — After summarizing a set of messages, mark them with the summary ID so they're excluded from future reads and never re-summarized.
- **Why it matters for Mika** — Prevents the compaction pipeline from re-processing already-compressed content. Essential if P2 is implemented.
- **How to adapt** — Add a nullable `summary_id` column to `messages`. `read_conversational_memory()` (if such a function existed) would filter `WHERE summary_id IS NULL`. Simple, clean.
- **YAGNI check** — Depends on P2. If conversation summarization is built, this is a mandatory companion.

**P4: Structured summary format (4 sections)**
- **What it is** — Summaries capture technical information, emotional context, entities, and action items as separate sections.
- **Why it matters for Mika** — Mika's `core_memory` already separates concerns (user_summary, self_model, current_priorities, key_people, workflows). A structured summary format would align with these blocks — technical info maps to user_summary, entities map to key_people, action items map to current_priorities.
- **How to adapt** — When summarizing, instruct the LLM to output sections that map to core memory blocks. The `update_core_memory` tool already supports targeted block updates.
- **YAGNI check** — Neat alignment with existing architecture. Low effort if summarization is built. **Bundle with P2.**

### 4. Reject list

| Idea | Why reject |
|---|---|
| **Separate `SUMMARY_MEMORY` vector store** | Summaries should be in the same `messages` or `search_content` table with a type discriminator. A separate store is over-separation. |
| **`offload_to_summary()` string manipulation** | The notebook's approach of regex-replacing sections in a big context string is fragile. Mika builds context programmatically in `prompt.rs` — structured code, not string surgery. |
| **"Emotional context" in summaries** | Mika is an executive assistant, not a therapist. Emotional context tracking adds complexity for minimal value. Technical info + entities + action items are sufficient. |
| **LLM-generated summary labels** | The course uses an extra LLM call to generate an 8-12 word label for each summary. Wasteful — a timestamp + thread_id is sufficient for identification. |

### 5. Proposed ADR candidates

**ADR: Session Conversation Compaction Strategy**

Mika sessions (especially team runs and multi-hour interactive sessions) generate long conversation histories that are fully replayed in the system prompt each turn. As multi-provider LLM support lands and cheaper/smaller models are used, context window pressure will become a real constraint. This ADR should evaluate three strategies: (a) rolling window truncation (drop messages older than N turns), (b) LLM-driven summarization into core memory blocks with backreference, (c) hybrid (recent N turns verbatim + summary of older turns). The decision should consider cost (extra LLM calls for summarization), information loss, and alignment with existing core memory architecture.

---

## Notebook 4: Memory-Aware Agent (Full Integration)

### 1. Core concept

This notebook assembles the full agent loop: before each LLM call, **build a partitioned context window** by reading from all 7 memory stores. Monitor token usage and auto-summarize if above 80%. Retrieve only relevant tools via semantic search. After the LLM responds, persist conversation, workflow patterns, entities, and tool logs. The "Just-In-Time retrieval" pattern lets the agent call `expand_summary()` to recover detail from compressed summaries only when needed.

### 2. What they use vs. what we'd use

| Their approach | Mika equivalent |
|---|---|
| `call_agent()` orchestration function | `run_loop()` in `agent.rs` |
| Partitioned context: `## Conversation Memory`, `## Knowledge Base Memory`, etc. | `PromptContext` with `soul_content`, `core_memory`, skill injection |
| Pre-loop context assembly from 5 memory stores | `inject_skills_and_resolve_tools()` + core memory injection in system prompt |
| 80% threshold → `offload_to_summary()` | No equivalent — no context monitoring |
| `read_toolbox(query, k=5)` semantic tool selection | Skill keyword matching + static `default_tools()` |
| `write_workflow(query, steps, final_answer)` post-loop | No equivalent — workflows not persisted |
| `write_entity()` with LLM extraction | `store_fact` tool (agent-triggered, not automatic) |
| Tool result truncation (3000 chars) + TOOL_LOG full persist | Tool output in `tool_calls` table (full) + truncated in prompt (via `output_summary` 300 chars) |
| `max_iterations = 10` | `MAX_TOOL_STEPS = 10` |
| System prompt with memory store semantics | `soul.md` + `<core-memory>` XML blocks |
| JIT `expand_summary()` tool | No equivalent |

### 3. Adaptable patterns (ranked by value/effort)

**P1: Partitioned context with explicit section semantics**
- **What it is** — The system prompt tells the LLM what each memory section means and how to prioritize conflicts (current question > recent conversation > knowledge base > old summaries).
- **Why it matters for Mika** — Mika already uses `<core-memory>` XML tags and `<context type="skill">` blocks. But there's no explicit conflict resolution instruction ("if core memory says X and a skill says Y, prefer..."). Adding priority semantics would reduce hallucination in multi-source contexts.
- **How to adapt** — Add a paragraph to `soul.md` or `prompt.rs` system prompt template that establishes priority: current message > core memory > skill context > conversation history. Minimal code change.
- **YAGNI check** — Active improvement. Low effort, high clarity. **Do this.**

**P2: Pre-loop deterministic context assembly**
- **What it is** — Before the LLM sees the query, deterministically read all relevant memory stores and inject them as structured sections.
- **Why it matters for Mika** — Mika already does this: core memory is always injected, skills are matched and injected, conversation history is loaded. This validates Mika's architecture.
- **How to adapt** — No change needed. Mika's `prompt.rs` context assembly is already deterministic.
- **YAGNI check** — Already solved.

**P3: Tool output truncation with full-output persistence**
- **What it is** — Truncate large tool results before injecting into chat context (3000 chars), but persist the full output to a log table for later retrieval.
- **Why it matters for Mika** — Mika already does a version of this: `tool_calls` table stores full output, and `ToolCallSummary.output_summary` is capped at 300 chars for the prompt. The course uses a more generous 3000-char limit and provides a JIT retrieval tool.
- **How to adapt** — The 300-char summary might be too aggressive. Consider bumping to 1000-2000 chars, or making it configurable per tool. Add a `read_tool_log` builtin tool for JIT retrieval of full outputs from prior turns.
- **YAGNI check** — Low-priority refinement. Current 300-char summaries work because the full output is in the same-turn `tool_result` block. Cross-session retrieval is the gap.

**P4: Post-loop workflow and entity persistence**
- **What it is** — After the agent loop completes, automatically save the tool execution sequence as a "workflow" and extract entities from the response.
- **Why it matters for Mika** — Mika saves `ToolCallSummary` metadata per message but doesn't distill cross-step patterns. Entity extraction is agent-triggered via `store_fact`. Automatic entity extraction could populate `people` table without agent action.
- **How to adapt** — Entity extraction: add a lightweight post-loop step in `agent.rs` that scans the final response for @-mentions or proper nouns and upserts into `people`. Workflow persistence: write a condensed step summary to `search_content` with `source_type = 'workflow'`.
- **YAGNI check** — Entity extraction: moderate value (reduces reliance on agent using `store_fact`). Workflow persistence: speculative. **Defer both; revisit when Mika Prime runs multi-day sprints.**

### 4. Reject list

| Idea | Why reject |
|---|---|
| **`AGENT_SYSTEM_PROMPT` with 7 memory section semantics** | Mika's system prompt is already structured via `soul.md` + XML blocks. Rewriting it to match this format would be a lateral move, not an improvement. |
| **`call_openai_chat()` with `tool_choice = "auto"`** | Mika uses Claude's native tool_use. OpenAI-specific patterns don't apply. |
| **`write_entity("", "", "", llm_client=client, text=query)` automatic extraction** | Passing the LLM client into a write function for inline extraction is a poor separation of concerns. Mika's `store_fact` tool keeps this explicit. |
| **JSON-based tool argument parsing in Python** | Mika uses serde_json with typed structs. Already superior. |
| **Thread-based conversation isolation** | Mika already has `session_id` + `agent_id` scoping. Equivalent concept, better implementation. |
| **`read_summary_context(query, thread_id=thread_id)` blending semantic + thread-scoped retrieval** | Over-complex. If summaries are needed, a simple "last N summaries for this session" query is cleaner than semantic search over summaries. |

### 5. Proposed ADR candidates

None beyond the one from Notebook 3 (Session Conversation Compaction Strategy).

---

## Synthesis: Cross-Notebook Themes

### Theme 1: Mika's architecture already embodies the right abstractions

The course's `MemoryManager` → `StoreManager` → `Toolbox` hierarchy is a Python-idiomatic version of what Mika already has in Rust:
- `MemoryManager` ≈ `db.rs` functions + `ToolContext`
- `StoreManager` ≈ `search_content` table with `source_type` discriminator
- `Toolbox` ≈ `ToolRegistry` + skill keyword matching
- Deterministic context assembly ≈ `prompt.rs` + `inject_skills_and_resolve_tools()`

The course validates Mika's design. The main gap is that Mika lacks **conversation compaction** and **cross-session tool output retrieval**.

### Theme 2: Deterministic vs. agent-triggered is the right framing

The course's classification of memory operations into deterministic (always run) and agent-triggered (LLM decides) is exactly how Mika works:
- **Deterministic:** core memory injection, skill matching, conversation history loading
- **Agent-triggered:** `store_fact`, `update_core_memory`, `search_memory`

This validates Mika's approach of keeping context assembly in `prompt.rs` (deterministic) and memory writes as tools (agent-triggered).

### Theme 3: Context compaction is the most actionable gap

Across all four notebooks, the most relevant pattern for Mika is **conversation compaction**:
- Mika replays full conversation history in the system prompt each turn
- Long sessions (team runs, multi-hour interactions) will hit context limits
- The course's approach (summarize → backreference → JIT expand) is sound
- Mika's existing `core_memory` blocks (especially `user_summary` and `current_priorities`) are natural targets for rolled-up summaries

### Theme 4: Semantic tool selection is solved differently (and better) by skills

The course uses vector similarity to select tools at runtime. Mika uses **declarative skill matching** with keyword triggers and `required_tools` constraints. The skill approach is:
- More predictable (deterministic keyword match vs. probabilistic embedding similarity)
- More explicit (skill authors declare when their tools apply)
- More controllable (`always_on`, dependencies, per-skill LLM overrides)

Semantic tool retrieval only becomes valuable at 100+ tools, which Mika won't reach.

### Theme 5: Tool output as persistent memory (not just observability)

Both the course and Mika persist tool outputs. The difference:
- Course: agent can query past tool outputs via `read_toolbox`/`read_tool_logs`
- Mika: `tool_calls` table exists for observability but isn't queryable by the agent

Bridging this gap (a `search_tool_history` tool) would be low-effort and prevent redundant external API calls.

---

## Prioritized Next Actions (max 5)

### 1. Add context priority semantics to system prompt
**Effort:** Trivial (one paragraph in `soul.md` or `prompt.rs`)
**Value:** Reduces hallucination when core memory, skill context, and conversation history conflict
**Action:** Add a priority rule to the system prompt template: "When information conflicts, prefer: current user message > core memory > active skill context > conversation history > search results."

### 2. ADR: Session Conversation Compaction Strategy
**Effort:** Design work (ADR document)
**Value:** Prepares for multi-provider support (smaller context windows) and reduces cost for long sessions
**Action:** Write an ADR evaluating three strategies: rolling window truncation, LLM-driven summarization into core memory blocks, and hybrid. Include cost/quality tradeoffs and alignment with existing core memory architecture. This becomes critical when cheaper models with smaller context windows are used.

### 3. `search_tool_history` builtin tool
**Effort:** Small (SQL query wrapper exposed as a tool, ~50 lines Rust)
**Value:** Agents can recall prior tool results without re-running external calls. Useful for Mika Prime's sprint management.
**Action:** Implement a tool that queries `tool_calls` table by `tool_name`, keyword in `input`/`output`, and time range. Return structured results.

### 4. Document deterministic vs. agent-triggered memory classification
**Effort:** Trivial (documentation only)
**Value:** Makes the architectural intent explicit for future contributors and for reasoning about new features
**Action:** Add a section to `mika/CLAUDE.md` or an architecture doc that classifies each memory operation (core memory reads, skill injection, fact storage, etc.) as deterministic or agent-triggered, with rationale.

### 5. Monitor tool count growth
**Effort:** None (awareness item)
**Value:** Prevents premature optimization while flagging when semantic tool selection becomes necessary
**Action:** No code change. When total tools per turn (builtins + skill + MCP) regularly exceed 50, revisit dynamic tool subsetting. Track via `llm_calls` table's tool count metadata.
