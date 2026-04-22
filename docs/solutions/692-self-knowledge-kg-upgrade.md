---
module: crates/mika-agent/templates/skills/self-knowledge
tags: [knowledge-graph, self-knowledge, skill-prompt, tool-routing, fallback]
problem_type: feature-implementation
date: 2026-04-22
issue: 692
---

# Self-Knowledge Upgrade: KG-Backed Agent Self-Awareness

## Problem

The self-knowledge skill was a static documentation lookup — `get_documentation(topic)` returned reference docs but couldn't answer structural questions like "what solves CI failures?" or "which skills handle PR reviews?" These require graph traversal over the knowledge graph, not document retrieval.

With `query_knowledge_graph` (#688) available as a builtin tool, the self-knowledge skill needed to orchestrate between two tools: KG queries for structural questions and documentation lookups for reference material.

## Solution

**Prompt-only orchestration.** The skill's system prompt was rewritten to route questions to the appropriate tool:

1. **Structural questions** → `query_knowledge_graph` (capabilities, problems, solutions, dependencies)
2. **Reference questions** → `get_documentation` (architecture, API spec, configuration)
3. **Spanning questions** → chain both tools

**Fallback on `starting_entity_missing`.** When the KG returns `starting_entity_missing` (agent or entity not yet ingested), the prompt guides the LLM to specific live registry fallbacks per question category (e.g., `list_skills` for skill questions, `list_agents` for team questions).

**No fallback on `traversal_empty`.** When the entity exists but has no connections, the prompt instructs the LLM to trust the KG — this is legitimate "no results" rather than a staleness problem.

**MCP tool awareness.** The prompt notes that MCP-dynamic tools added since last restart won't appear in KG results, so the agent should mention this when answering tool queries.

**Agent context interpretation.** Skill entities include `agent_context.enabled` metadata. The prompt instructs the LLM to explicitly mention disabled skills rather than hiding them.

## Key Decisions

- **No Rust code changes.** `query_knowledge_graph` is a builtin in `default_tools()` — available to all agents automatically. The skill's `tools.json` doesn't need to declare it. All routing logic lives in the system prompt.
- **No new tools or handlers.** The fallback uses existing tools (`list_skills`, `list_agents`, `read_agent_file`, `list_agent_files`).
- **Read-only.** Self-knowledge is purely a read interface — no KG mutations, no annotations, no validation workflows.

## Testing

Seven eval harness tests (`test_self_knowledge_kg.rs`) verify loop integration for the key scenarios: KG query for capability questions, doc query for reference questions, chained tool use, `starting_entity_missing` fallback, `traversal_empty` no-fallback, agent context usage, and disabled skill mention. These verify the agent loop processes scripted sequences correctly — prompt routing correctness requires LLM-based evaluation.

## Pattern

This is a "skill as orchestrator" pattern: the skill doesn't implement new tool logic — it provides routing guidance in the system prompt that helps the LLM choose between existing tools. The fallback table pattern (question category → specific tool to use) is reusable for any skill that needs to handle tool unavailability gracefully.
