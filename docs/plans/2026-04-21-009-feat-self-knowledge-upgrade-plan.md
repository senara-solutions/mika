---
title: "feat: self-knowledge upgrade — KG-backed agent self-awareness"
type: feat
status: active
date: 2026-04-21
---

# Self-knowledge upgrade — KG-backed agent self-awareness

## Overview

Upgrade the `self-knowledge` skill (milestone mika#14, ticket mika#692) to use `query_knowledge_graph` (#688) for richer, more accurate responses. The skill becomes the orchestration layer that routes questions to the KG, supplements with live-source fallbacks when the KG is sparse, and lets the LLM synthesize answers from both.

This is the user-facing payoff of the entire KG investment. It directly addresses the session audit finding where mika-dev thrashed with 22 tool calls because she didn't know which skill to use — with KG-backed self-knowledge, the agent queries once and gets the full solution chain.

## Problem Frame

Current `self-knowledge` is a static doc lookup: `get_documentation(topic: "architecture")` returns a reference doc. It can't answer "what skill handles CI failures?" or "what's the fix for stale UUIDs?" — those require structured reasoning over the KG, not document retrieval.

The KG now has the data to answer these questions. #688's `query_knowledge_graph` exposes the traversal. This ticket wires the self-knowledge skill to call it, handles staleness gracefully, and preserves the static doc lookup for reference-material questions.

## Requirements Trace

- R1. Self-knowledge skill orchestrates both `query_knowledge_graph` and `get_documentation`.
- R2. Registry/config fallback on KG miss (when `query_knowledge_graph` returns `starting_entity_missing`).
- R3. Fallback logic lives in the skill, not the tool.
- R4. Data-provenance annotations on results ("from KG" vs "from registry, not yet in KG").
- R5. Read-only — no mutations back into the KG or any other table.
- R6. Staleness fallback is part of a broader "combine KG with live state" pattern, not a unique edge case.

## Scope Boundaries

- Self-knowledge skill upgrade: tool orchestration, fallback logic, result annotation.
- No new KG tables or schema changes.
- No mutation paths into the KG (no "dismissed/validated/important" annotations on entities — that's a separate ticket if needed).
- No cross-agent subject resolution (deferred in #691 D8).
- `get_documentation` unchanged — same topic enum, same static docs.

## Context & Research

### Dependencies

- **#688:** `query_knowledge_graph` tool must exist with the return shape from #688 D4 (status field, agent_context metadata, entry_method).
- **All prior KG tickets:** Schema, domain graph, lexical chunks, subject entities, entity resolution must be populated for meaningful query results.

### Relevant Code and Patterns

- **Current self-knowledge skill:** `crates/mika-agent/templates/skills/self-knowledge/` — `skill.toml` (always_on), `tools.json` (get_documentation), `system_prompt.md`.
- **`get_documentation` handler:** `crates/mika-agent/src/skills/builtin_handlers.rs:125` — static doc lookup by topic enum. Returns `include_str!` content.
- **Existing tool chaining:** Agents already chain tools (search_memory → store_fact). Self-knowledge orchestrating query_knowledge_graph → get_documentation is the same pattern.
- **#687 D4 staleness contract:** Domain graph reflects state as of last boot. Create_agent between boots means no agent node in KG. #692 must handle this.

## Key Technical Decisions

### D1. Skill orchestrates, tools don't

Resolved during planning. The self-knowledge skill's system prompt guides the LLM to:

1. For structured questions ("what solves X?", "which skills handle Y?", "what does Z provide?"): call `query_knowledge_graph(question=..., agent_id=self)`.
2. For reference questions ("explain the memory model", "show me the API spec"): call `get_documentation(topic=...)`.
3. For the KG result: interpret `agent_context.enabled` based on question intent (filter, highlight, or ignore).

The skill owns the routing logic, not the tools. `query_knowledge_graph` returns data with metadata; the skill decides how to use it.

### D2. Registry/config fallback on `starting_entity_missing`

Resolved during planning. When `query_knowledge_graph` returns `status: "starting_entity_missing"` (the agent node or queried entity doesn't exist in the KG), the skill supplements with direct reads from live sources.

**Specific fallback categories:**

| Question category | Live source | Fallback query |
|-------------------|-------------|----------------|
| "What skills do I have?" | SkillRegistry | `entries_for(agent_id)` filtered by enabled |
| "What tools do I have?" | ToolRegistry | Tools registered for this agent |
| "What agents are on my team?" | agent_configs | Enumerate agents from config |
| "What is my role?" | agent_configs | Agent role/identity from config |
| "What MCP servers am I connected to?" | McpManager | Connected servers |

Each fallback is narrow and specific — not a generic "if KG is sparse, read registries." Broad fallback would hide legitimate "KG says no" answers.

**Tool fallback (not just `starting_entity_missing`):** The "What tools do I have?" row deserves special treatment. MCP tools added via dynamic connect since last boot are not in the KG but are in ToolRegistry. This isn't just a create_agent staleness case — it's the normal state for any agent with MCP connections. For tool queries, always supplement KG results with ToolRegistry diff (tools in ToolRegistry but not in `kg_entities WHERE type='tool'`). This runs on `status: "ok"` too, not just `starting_entity_missing`.

**When NOT to fall back:**
- `status: "traversal_empty"` — the entity exists in the KG, traversal found nothing. Trust the KG.
- `status: "ok"` with results — use results directly (except tool queries, which always supplement per above).
- Questions about subject-layer entities (problem_type, solution_path) — these don't have registry fallbacks; the KG is the only source.

### D3. Data-provenance annotations

Resolved during planning.

**Format:** Batch preamble for uniform-source results (all KG or all registry), per-item inline for mixed-source results.

- **All-KG:** No annotation. Default.
- **All-registry fallback:** Preamble: `"Note: the following results are from the live registry (not yet in KG — available after next restart):"` followed by the result list.
- **Mixed (KG + ToolRegistry supplement for MCP tools):** Each item carries inline source: `"skill:self-dev (KG)"`, `"mcp__stitch__generate_screen (ToolRegistry — added via MCP since last restart)"`.

Annotations appear in the result text that the LLM sees, not in a structured field — the LLM needs to reason about provenance in its natural-language response to the user.

### D4. Combine KG with live state — general pattern

Resolved during planning. The create_agent staleness case is one instance of a broader pattern: structural queries → KG; state queries → live sources; cross-layer answers combine both.

Three cases the skill handles:

1. **Agent not in KG yet** (KG empty for this agent, use registry entirely) — the create_agent case.
2. **Skill disabled since last boot** (KG has it, `agent_context.enabled = false`) — the skill_overrides case. Handled by #688 D5 metadata, interpreted by the LLM.
3. **MCP tool added since last boot** (ToolRegistry has it, KG doesn't) — handled by ToolRegistry diff supplementation in D2. For "what tools do I have?", always supplement KG results with tools present in ToolRegistry but absent from `kg_entities`. This is the normal state for MCP-dynamic tools, not an edge case.

All three cases are handled in MVP: case 1 by D2 fallback, case 2 by #688 D5 metadata, case 3 by D2 tool supplementation.

### D5. Read-only — no KG mutations from self-knowledge

Resolved during planning. The self-knowledge skill is purely a read interface. No mutations back into the KG:

- No "mark this entity as validated/dismissed/important" capability.
- No "add a fact I just learned" pathway through self-knowledge.
- No per-agent annotations on KG objects.

If per-agent annotations become a real need (e.g., agent marks certain solution paths as "tried and didn't work"), that's a separate ticket with its own design pass — new table, new write contract, new sole-writer designation.

Rationale: "KG is derived, not authored" is the architectural framing from the milestone. Agents consume the KG; the ingestion pipeline populates it. Self-knowledge is a consumer, not a populator. Adding mutation paths here would create a dual-write concern (extraction writes entities, self-knowledge annotates them) that the milestone explicitly avoided.

## Open Questions

### Resolved During Planning

- Tool shape — two tools, skill orchestrates (D1).
- Fallback placement — in skill, not tool (D2).
- Provenance annotations — on fallback results (D3).
- Staleness pattern — general "combine KG with live state" (D4).
- Read-only constraint — no KG mutations (D5).

### Deferred to Implementation

- Self-knowledge system prompt wording (routing guidance for KG vs doc queries).
- Whether the skill explicitly parses question categories for fallback routing or relies on the LLM to chain tools based on the prompt.
- Cross-agent self-knowledge ("what does the team know about X?") — requires cross-agent subject resolution (deferred in #691 D8).
- Integration with existing tool chaining patterns (does the skill's prompt guide multi-step tool use or is it a single-call wrapper?).

## Output Structure

```
crates/mika-agent/
├── templates/skills/self-knowledge/
│   ├── skill.toml                  # MODIFY: add query_knowledge_graph to dependencies
│   ├── tools.json                  # MODIFY: keep get_documentation, verify query_knowledge_graph is available
│   └── system_prompt.md            # MODIFY: add KG routing guidance
└── src/skills/builtin_handlers.rs  # get_documentation handler unchanged

docs/plans/
└── 2026-04-21-009-feat-self-knowledge-upgrade-plan.md   # this file
```

## Implementation Units

- [ ] **Unit 1: System prompt upgrade**

**Goal:** Update self-knowledge's `system_prompt.md` with routing guidance: when to use `query_knowledge_graph` vs `get_documentation`, how to interpret `agent_context` metadata, how to handle `starting_entity_missing` status.

**Requirements:** D1, D2, D3.

**Files:** `crates/mika-agent/templates/skills/self-knowledge/system_prompt.md`.

**Approach:** Add sections:
- "For questions about capabilities, problems, solutions, and skills: use `query_knowledge_graph`."
- "For reference documentation (architecture, API, config): use `get_documentation`."
- "When `query_knowledge_graph` returns `starting_entity_missing`: you or the queried entity may not be in the knowledge graph yet. Use registry information as fallback. Note the source in your response."
- "When results include `agent_context.enabled = false`: the skill exists but is disabled for this agent. Mention this in your response rather than hiding it."

---

- [ ] **Unit 2: Skill manifest update**

**Goal:** Ensure `query_knowledge_graph` is available to the self-knowledge skill.

**Requirements:** D1.

**Files:** `crates/mika-agent/templates/skills/self-knowledge/skill.toml`, optionally `tools.json`.

**Approach:** `query_knowledge_graph` is a builtin tool registered in `default_tools()` — it's available to all agents regardless of skill. The self-knowledge skill doesn't need to declare it in `tools.json`. The skill's `system_prompt.md` references it, so **#692 has a strict dependency on #688** — implement and ship #688 before #692. Do not attempt graceful degradation for the missing-tool case; LLMs hallucinate tool calls for nonexistent tools, producing worse behavior than not trying. The milestone ordering already enforces this (#692 is last).

---

- [ ] **Unit 3: Fallback logic and behavioral validation**

**Goal:** Verify (a) the tool chain runs correctly for each fallback case, and (b) the LLM produces responses that correctly reflect provenance and enablement metadata.

**Requirements:** D2, D3, D4.

**Files:** Integration tests, eval harness tests.

**Approach:** Two test layers:

**Layer 1 — Tool chain correctness (integration tests):**
- Agent in KG → `query_knowledge_graph` returns `status: "ok"` → no fallback.
- Agent not in KG → `status: "starting_entity_missing"` → skill calls SkillRegistry/ToolRegistry as fallback.
- Traversal empty → `status: "traversal_empty"` → no fallback, empty trusted.
- Tool query with MCP dynamic tools → KG results supplemented with ToolRegistry diff.

**Layer 2 — LLM behavioral tests (eval harness with MockLlmProvider):**

The self-knowledge system prompt includes contractual requirements the LLM must follow. Tests verify the LLM's response text meets these requirements:

| Scenario | Prompt contract | Response assertion |
|----------|----------------|-------------------|
| New agent, fallback fires | "When results come from registry, note this in your response" | Response contains "registry" or "not yet in KG" |
| Skill with `enabled: false` in results | "When a skill is disabled, explicitly say so rather than omitting it" | Response mentions the disabled skill with its disabled status |
| MCP tool from ToolRegistry supplement | "When tools were added since last restart, note they may not have KG context" | Response distinguishes KG-sourced from ToolRegistry-sourced tools |
| Subject-layer query, no fallback available | "When the KG returns no results for a topic, say so rather than inventing" | Response does NOT fabricate an answer when KG returns empty |

These are eval-style tests: seed MockLlmProvider with responses that follow/violate the prompt contracts, verify the system prompt's guidance produces correct behavior on representative inputs. Fragile by nature but the alternative is shipping untested LLM behavior.

**Test scenarios (combining both layers):**
- New agent asks "what skills do I have?" → fallback fires, response mentions registry source.
- Established agent asks "what skills do I have?" → KG results with `agent_context`, response distinguishes enabled/disabled.
- Agent asks "what tools do I have?" → KG + ToolRegistry diff, MCP tools annotated.
- Agent asks "what solves fabrication?" → KG traversal, no fallback. If KG empty, response says "no structured knowledge about this."

## Error Handling & Edge Cases

| Scenario | Expected behavior |
|----------|------------------|
| KG not populated (all prior tickets not yet implemented) | `query_knowledge_graph` doesn't exist → LLM falls back to `get_documentation` only. Graceful degradation to pre-KG behavior. |
| `query_knowledge_graph` returns error | Log warning, fall back to `get_documentation` if the question has a topic equivalent. Otherwise return "I couldn't query my knowledge graph; try asking again." |
| Agent asks about another agent's capabilities | `query_knowledge_graph(agent_id=other_agent)` returns that agent's context. If other agent not in KG → `starting_entity_missing` → skill can't fall back (no registry access for other agents). Return "Agent X not yet in the knowledge graph." |
| Agent asks "what skills should I enable?" | KG returns all skills with `agent_context.enabled`. LLM filters by `enabled: false` in its response synthesis. No tool-level filtering needed. |
| Agent asks a question that spans KG and static docs | LLM chains: `query_knowledge_graph` for the structured answer, then `get_documentation` for the reference. Skill prompt encourages this. |
