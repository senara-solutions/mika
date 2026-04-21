---
title: "feat: domain graph builder — deterministic import from skill manifests and tool registry"
type: feat
status: active
date: 2026-04-21
---

# Domain graph builder — deterministic import from skill manifests and tool registry

## Overview

Build the domain graph layer of Mika's Knowledge Graph (milestone mika#14). This ticket (mika#687) populates `kg_entities` and `kg_relationships` by enumerating four authoritative sources — `SkillRegistry`, `ToolRegistry`, `McpManager`, and agent configs — at server startup. No LLM calls, no prose processing, no per-agent scope. The output is a shared structural view of "what exists in this container and how pieces connect," which downstream KG tickets (#689 lexical ingestion, #690 subject extraction, #691 entity resolution, #688 query tool, #692 self-knowledge upgrade) consume.

## Problem Frame

Agents today cannot answer "what skill handles CI failures?" or "which tools does skill X provide?" without either iterating the live registry (which has no traversal semantics) or querying semantic memory over prose (which drifts). The KG domain graph is the structural substrate that makes those questions a one-edge or recursive-CTE traversal instead of a scan.

#686 landed the schema. This ticket populates it from the sources that already hold the truth: skill manifests (`skill.toml`), the tool registry, MCP connections as-of-boot, and agent configs. No new sources of truth — the domain graph is a projection, not an authority.

## Requirements Trace

- R1. Startup-time population of `kg_entities` (Skill, Tool, Agent, ProblemType) from authoritative sources.
- R2. Startup-time population of `kg_relationships` for structural edges (DEPENDS_ON, PROVIDES).
- R3. Idempotent rebuild — running the builder again (next startup, or forced via future CLI subcommand) does not duplicate or churn rows.
- R4. Upsert preserves entity rowids so `kg_chunks.entity_id` FK references survive rebuilds.
- R5. Documented staleness contract — the domain graph reflects registry state as of the last server boot.
- R6. Observability via structured logs (per conventions C3.1 — no audit_events for container-wide rebuild).

## Scope Boundaries

- Four entity types: Skill, Tool, Agent, ProblemType.
- Two relationship types: DEPENDS_ON (Skill→Skill), PROVIDES (Skill→Tool).
- ProblemType seed list (5 categories per ticket body: `ci_failure`, `merge_conflict`, `duplicate_pr`, `stale_uuid`, `fabrication`).
- Startup-only execution, hooked after `apply_overrides()` in `server/mod.rs`.
- Idempotent MERGE/upsert semantics.
- Structured logging at INFO with trace_id.

### Deferred to Separate Tasks

- Lexical chunk ingestion and linkage to domain entities: **mika#689**.
- Subject graph extraction and LLM-inferred relationships (`SOLVED_BY`, `CAUSES`, `INDICATES`, etc.): **mika#690**.
- Entity resolution (subject → domain linkage): **mika#691**.
- Query tool (`query_knowledge_graph`): **mika#688**.
- CLI subcommand for force-rebuild (`mika kg rebuild-domain`): deferred. Only build this if #688/#692 surface staleness as a real operational problem — not speculatively.
- `Agent → Skill` (HAS_SKILL) edges: explicitly excluded (see D2 — per-agent state belongs in `skill_overrides`, not the graph).
- `ProblemType → Skill` (SOLVED_BY) edges: excluded from domain graph (no authoritative source today). LLM-extracted solution-path edges land in the subject graph via #690/#691, with entity resolution linking them to domain ProblemType nodes.

## Context & Research

### Cross-cutting conventions

This plan cites `docs/architecture/kg-implementation-conventions.md` as the authoritative source for cross-cutting KG decisions. Sections that apply to #687:

- **C3.1 (observability — domain rebuild)**: Structured logs at INFO, `trace_id` included, no audit_events. See C3.1 for log-line format.

Sections C1 (embeddings) and C2 (non-interactive LLM calls) do not apply to #687 — this ticket is pure deterministic code with no embedding generation and no LLM calls.

### Relevant Code and Patterns

- **Source of truth for skills:** `crates/mika-agent/src/skills/mod.rs:158` (`pub struct SkillRegistry`). Loaded at startup in `server/mod.rs` via `SkillRegistry::from_dir()` + `apply_overrides()`. Each entry has `manifest.skill.name`, `manifest.skill.description`, `manifest.skill.always_on`, `manifest.skill.keywords`, `manifest.skill.dependencies`, and `manifest.handler.tools` (the list of tools the skill exposes).
- **Source of truth for tools:** `crates/mika-agent/src/tools/mod.rs:561` (`pub struct ToolRegistry`). Built up at startup from builtins + skill tools.json + MCP tools. Each tool has a name, source (`builtin` | `skill:<name>` | `mcp:<server>`), and description.
- **Source of truth for MCP tools as-of-boot:** `crates/mika-agent/src/mcp/` — `McpManager` loads at startup from `mcp.json` config. Tools exposed by MCP servers available at startup are registered in ToolRegistry and visible to this builder. Tools added via runtime MCP reconnects are NOT visible until next restart (see D4 — staleness contract).
- **Source of truth for agents:** `crates/mika-agent/src/db.rs` `agents` table + per-agent `config.toml` for `(name, role, model, home_dir)`. Note: agents can be created at runtime via the `create_agent` tool (no restart required). This creates a bounded staleness window — see D4.
- **Startup sequence:** `crates/mika-agent/src/server/mod.rs` around lines 339 (dev_mode check) and 534-538 (auto-provisioning well-known agents). The domain graph builder hooks in AFTER both of these — it enumerates whatever state exists after initialization, regardless of how those agents/skills got there.
- **Existing upsert precedents:** `crates/mika-agent/src/db.rs` `index_content()` pattern at `db.rs:6107` — idempotent write that uses explicit delete+insert for FTS5 sync. For `kg_entities`, UPSERT via `ON CONFLICT(entity_key) DO UPDATE` preserves rowid. For `kg_relationships`, DELETE-all-then-INSERT per rebuild is simpler and has no downstream dependencies (no FKs reference relationships).

### Institutional Learnings

- `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md` — Dual-write anti-pattern. Designate one writer per entity kind: the domain-graph builder owns all `skill:*`, `tool:*`, `agent:*`, `problem_type:*` nodes. No other code path writes these entity_keys. Document this explicitly in the builder's rustdoc.
- `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — No `SELECT *`; use column constants from `crates/mika-agent/src/db/kg_schema.rs` (landed in #686). Row mappers import constants, not inline column lists.
- `docs/solutions/database-issues/trace-id-as-observability-join-key.md` — Builder generates a single `trace_id` per rebuild invocation, logged with every row-count line. Allows "show me the last rebuild" via `grep trace_id=<id>` in logs. (No trace_id on the entity/relationship rows themselves — they're populated by deterministic startup code, not agent turns; per #686 D6, trace_id is for per-agent mutation tables.)

## Key Technical Decisions

### D1. Domain graph contains structure, not state

General principle: the domain graph holds **structural facts derivable from authoritative sources (manifests, configs, code)**. It does not hold **per-agent state that authoritatively lives elsewhere**.

Heuristic: *if the edge's truth value can differ across agents, or change between boots without a manifest change, it is state, not structure.*

Applying the test to candidate edge types:

| Edge | Test | Verdict |
|------|------|---------|
| `Skill -[DEPENDS_ON]-> Skill` | Derivable from `skill.toml`'s `dependencies` field; changes only when manifests change. | **Structural — include.** |
| `Skill -[PROVIDES]-> Tool` | Derivable from `skill.toml`'s `[handler] tools` field + MCP server tool lists; changes only when manifests/MCP configs change. | **Structural — include.** |
| `Agent -[HAS_SKILL]-> Skill` | Per-agent truth (varies across agents via `skill_overrides.enabled`); changes at runtime via override writes. | **State — exclude.** See D2. |
| `Agent -[HAS_TOOL]-> Tool` | Transitively computable from HAS_SKILL ∩ PROVIDES; no independent truth. | **Redundant — exclude.** |
| `ProblemType -[SOLVED_BY]-> Skill` | No authoritative source today (skills don't declare solved problem types in manifests). LLM-inferrable only. | **Subject-graph territory — defer to #690/#691.** |
| `Agent -[DELEGATES_TO]-> Agent` | Derivable from team configs (deterministic); changes only when configs change. | **Structural if we need it — out of scope for #687 initial set.** |

This principle applies to every future edge type considered in this milestone. When #690/#691 introduce new edges via subject extraction, those edges land in the subject graph (agent-scoped per #686 D1), with entity resolution linking them to domain nodes — they do not populate `kg_relationships` directly.

### D2. No `Agent → Skill` (HAS_SKILL) edges

Per-agent skill enable/disable state lives authoritatively in `skill_overrides.enabled` (schema v24, tri-state NULL/0/1). Replicating it as HAS_SKILL edges would create the exact stale-prose failure class the KG is intended to eliminate — just projected onto graph edges instead of text.

Queries that need "agent X's enabled skills" JOIN against `skill_overrides`:

```sql
SELECT s.entity_key, s.name
FROM kg_entities s
WHERE s.type = 'skill'
  AND (s.name NOT IN (
    SELECT skill_name FROM skill_overrides
    WHERE agent_id = ? AND enabled = 0
  ));
```

**Consumer-side concern for #688:** the KG query tool should transparently perform this JOIN when an LLM caller asks "what skills does agent X have?" — so callers don't need implicit knowledge of where enablement state lives. This is flagged for #688's plan.

### D3. Startup-only full rebuild, idempotent via upsert

Builder runs once on `mika-server` startup after `apply_overrides()`. No runtime mutation hooks, no agent-triggerable tool. Rationale:

- **The domain graph is a projection.** SkillRegistry, ToolRegistry, McpManager, and agent configs are the authoritative sources. A projection's rebuild cadence should match the volatility of its sources — and those sources are themselves loaded at startup and only mutated at explicit operator/admin boundaries, not in a hot path. The projection inherits that cadence.
- **Mutation hooks (rejected)** would assert runtime-mutable skill/MCP lifecycle semantics that Mika doesn't currently adopt. Every hook is a place where the projection can diverge from the source (hook fires but rebuild fails; new code path is added without the hook). Complexity pays for capabilities we don't have consumers for — classic YAGNI violation.
- **Agent-triggerable rebuild tool (rejected)** gives agents a knob without the information to know when to pull it. Agents have no visibility into system-level "has the registry changed" state; the tool would be called reflexively (wasting the rebuild cost on no-ops) or based on LLM guesses about staleness — precisely the drift the KG is trying to eliminate. If force-rebuild becomes useful for mika-dev investigation later, it belongs as a CLI subcommand (`mika kg rebuild-domain`), not a tool — that's an operator action, not an agent action.

Idempotency strategy:

- **Entities:** UPSERT via `INSERT ... ON CONFLICT(entity_key) DO UPDATE SET ...`. Preserves `id` (INTEGER rowid), which preserves `kg_chunks.entity_id` FK references from #689 onwards.
- **Relationships:** DELETE all domain-sourced relationships, then re-INSERT. Relationships have no downstream dependencies (nothing FK's into `kg_relationships.id`), so rebuild-and-replace is safe. Cost: a few hundred rewrites per boot, trivial.
- **Entity deletions:** entities no longer present in their source are DELETED. CASCADE on `kg_relationships` removes edges touching the deleted entity. `kg_chunks.entity_id` → NULL via ON DELETE SET NULL, preserving chunks but severing their domain-entity link. Downstream code handles this via `WHERE entity_id IS NOT NULL` filters where appropriate.

### D4. Staleness contract — bounded, documented

Domain graph reflects registry state **as of the last server boot**. Between boots, the following remain invisible to KG queries:

- Agents created via the runtime `create_agent` tool (no restart required, so staleness window is non-zero).
- MCP servers that connect/disconnect mid-session, along with any tools they expose or remove.
- Skills installed/uninstalled mid-session via `mika skills install/uninstall` (if such runtime installation is supported; today's flow requires a restart for most skill changes, but this may evolve).
- `skill_overrides.enabled` changes (expected — this is state, not structure; consumers JOIN against `skill_overrides` directly per D2).

The staleness window is bounded by the time until the next `mika-server` restart, and operators can force a restart to refresh.

Consumer implications:

- #688 query tool: document that "recent KG" means "last boot." If a user asks about a freshly created agent that hasn't been restarted into the graph yet, the query returns "not found." Acceptable — agents can be informed "this agent was just created; refresh after next restart to see it in the KG."
- #692 self-knowledge upgrade: same caveat. `self-knowledge` queries over the KG reflect last-boot state.

**This is an intentional design decision**, not an unfixed bug. The projection cadence matches the source cadence. If operational requirements evolve to demand live-updating domain graph semantics, that's a future ticket with its own design pass — not a silent requirement scope-creep on #687.

### D5. Observability via structured logs (per C3.1)

One `trace_id` per rebuild invocation, emitted at key points:

```
INFO trace_id=<uuid> event=domain_rebuild_start
INFO trace_id=<uuid> event=domain_rebuild_entities type=skill added=12 updated=3 removed=1
INFO trace_id=<uuid> event=domain_rebuild_entities type=tool added=47 updated=0 removed=0
INFO trace_id=<uuid> event=domain_rebuild_entities type=agent added=3 updated=0 removed=0
INFO trace_id=<uuid> event=domain_rebuild_entities type=problem_type added=5 updated=0 removed=0
INFO trace_id=<uuid> event=domain_rebuild_edges type=DEPENDS_ON count=18
INFO trace_id=<uuid> event=domain_rebuild_edges type=PROVIDES count=35
INFO trace_id=<uuid> event=domain_rebuild_complete duration_ms=234
```

No rows in `audit_events`. Query "what did the last rebuild do" via `grep trace_id=<id>` in structured logs. This matches the "domain rebuild is container-wide infrastructure, no agent attribution" rationale documented in the conventions doc C3.1.

## Open Questions

### Resolved During Planning

- Execution model — startup-only full rebuild (see D3).
- HAS_SKILL edges — excluded (see D2).
- Domain vs subject boundary for candidate edges — governed by the "structure not state" heuristic (see D1).
- Staleness contract — bounded to "as of last server boot," documented (see D4).
- Observability — structured logs only, per C3.1 (see D5).
- Idempotency approach — entity UPSERT by entity_key, relationship DELETE+REINSERT (see D3).

### Deferred to Implementation

- Exact log level for per-type counts (INFO vs DEBUG). INFO is the default above; may downgrade some lines to DEBUG if the log volume becomes noisy during development.
- MCP dynamic-tool handling: accept drift as known limitation per D4. If an MCP server's tools change mid-session, the domain graph doesn't reflect it until next restart. If usage surfaces this as a real problem, address via CLI force-rebuild subcommand (out of scope for this ticket).
- Whether ProblemType seed list lives as a Rust `const` or a TOML config file. Starts as a `const` in `kg_builder.rs`; promote to a config file only if operators need to extend it without recompiling.

## Output Structure

```
crates/mika-agent/src/
├── db/
│   └── kg_schema.rs                  # from #686 — add UPSERT helper + relationship rebuild helper
└── kg/                               # NEW module for KG builders and queries
    ├── mod.rs
    └── domain_builder.rs             # NEW: the builder itself

crates/mika-agent/src/server/
└── mod.rs                            # MODIFY: hook builder after apply_overrides()

crates/mika-agent/tests/
└── kg/                               # NEW test suite root
    └── domain_builder.rs             # NEW: integration tests for the builder

docs/plans/
└── 2026-04-21-004-feat-domain-graph-builder-plan.md   # this file
```

## High-Level Technical Design

> *This illustrates the builder's shape and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```rust
// crates/mika-agent/src/kg/domain_builder.rs

/// Projection owner for Skill/Tool/Agent/ProblemType nodes and their
/// structural edges. The sole writer of entity_keys in the
/// `skill:*`, `tool:*`, `agent:*`, `problem_type:*` namespaces.
///
/// Invariants:
/// - Runs once per server boot, after SkillRegistry::apply_overrides().
/// - Idempotent: re-running produces the same graph state.
/// - Never called from agent turns or tool handlers.
/// - All writes carry a single trace_id for the rebuild invocation.
pub struct DomainGraphBuilder<'a> {
    db: &'a AsyncDatabase,
    skill_registry: &'a SkillRegistry,
    tool_registry: &'a ToolRegistry,
    mcp_manager: &'a McpManager,
    agent_configs: &'a [AgentConfig],
    trace_id: String,
}

impl<'a> DomainGraphBuilder<'a> {
    pub async fn rebuild(&self) -> Result<RebuildStats> {
        // 1. Gather current sources into a DesiredState struct.
        let desired = self.enumerate_sources()?;

        // 2. UPSERT entities (preserves rowid).
        let entity_stats = self.upsert_entities(&desired.entities).await?;

        // 3. DELETE then INSERT domain-sourced relationships.
        let edge_stats = self.rebuild_relationships(&desired.edges).await?;

        // 4. DELETE entities no longer in sources (CASCADE removes edges,
        //    SET NULL on kg_chunks.entity_id).
        let removed = self.prune_stale_entities(&desired.entity_keys).await?;

        Ok(RebuildStats { /* counts */ })
    }

    fn enumerate_sources(&self) -> Result<DesiredState> {
        // Skill nodes from skill_registry.entries()
        // Tool nodes from tool_registry (builtin + skill-owned + MCP)
        // Agent nodes from agent_configs
        // ProblemType nodes from const PROBLEM_TYPE_SEEDS
        // DEPENDS_ON edges from skill.manifest.skill.dependencies
        // PROVIDES edges from skill.manifest.handler.tools + mcp server tool lists
    }
}
```

### Startup integration

```rust
// crates/mika-agent/src/server/mod.rs (around existing apply_overrides call site)

let skill_registry = SkillRegistry::from_dir(...);
skill_registry.apply_overrides(&db).await?;
skill_registry.validate_loaded();
skill_registry.log_summary();

// NEW: domain graph rebuild, after all registries are initialized
let builder = DomainGraphBuilder::new(&db, &skill_registry, &tool_registry, &mcp_manager, &agent_configs);
match builder.rebuild().await {
    Ok(stats) => info!(event = "domain_rebuild_complete", ?stats, "domain graph ready"),
    Err(e) => warn!(error = %e, "domain graph rebuild failed; KG queries may return stale results until next restart"),
}
```

### Failure policy

Rebuild failures are **logged, not panicked**. If the builder fails (DB error, unexpected registry state), mika-server continues to boot — KG queries will return stale or empty results, but interactive agent turns still work. This matches the "indexing is best-effort" policy from the conventions doc (C1), extended to the domain layer.

## Implementation Units

- [ ] **Unit 1: `kg` module scaffolding + DomainGraphBuilder skeleton**

**Goal:** Create the module structure and the builder type signature with stubbed `rebuild()`. No logic yet — just the shape and the rustdoc that documents the invariants (sole writer, startup-only, idempotent).

**Requirements:** R1 (shape only).

**Dependencies:** #686 schema landed (`kg_schema.rs` exists).

**Files:**
- Create: `crates/mika-agent/src/kg/mod.rs`
- Create: `crates/mika-agent/src/kg/domain_builder.rs`
- Modify: `crates/mika-agent/src/lib.rs` — add `pub mod kg;`

**Approach:**
- Module-level rustdoc in `kg/mod.rs` explaining the KG layer split (domain is deterministic, subject/lexical are agent-scoped — point at `kg_schema.rs` and `docs/architecture/kg-implementation-conventions.md` for the cross-cutting details).
- `DomainGraphBuilder` struct with fields: `db: &AsyncDatabase`, `skill_registry: &SkillRegistry`, `tool_registry: &ToolRegistry`, `mcp_manager: &McpManager`, `agent_configs: &[AgentConfig]`, `trace_id: String`.
- `pub async fn rebuild(&self) -> Result<RebuildStats>` — stubbed, returns `Err(unimplemented)`.
- `RebuildStats` struct with public fields for per-type counts (added/updated/removed per entity type, edges per relationship type).
- Unit-level tests are deferred to Unit 6 (integration tests once the builder works end-to-end).

**Patterns to follow:**
- Module structure: `crates/mika-agent/src/skills/mod.rs` — pub struct + impl block + module rustdoc style.
- Async error handling: `anyhow::Result` with `thiserror` for any new error enum.

**Test scenarios:**
Test expectation: none — this unit is scaffolding. Compilation is the success signal.

**Verification:**
- `cargo build -p mika-agent` succeeds.
- Rustdoc renders the module and struct descriptions cleanly.

---

- [ ] **Unit 2: Entity enumeration from authoritative sources**

**Goal:** Implement `enumerate_sources()` — walks the four registries/configs and produces a `DesiredState` struct listing every entity that should exist in the graph.

**Requirements:** R1.

**Dependencies:** Unit 1.

**Files:**
- Modify: `crates/mika-agent/src/kg/domain_builder.rs`
- Test: inline `#[cfg(test)]` module with mock registries

**Approach:**
- `DesiredState { entities: Vec<DesiredEntity>, edges: Vec<DesiredEdge>, entity_keys: HashSet<String> }`.
- Skill enumeration: iterate `skill_registry.entries()`. For each, build `DesiredEntity { entity_key: format_entity_key("skill", &s.manifest.skill.name), type: "skill", name, properties_json: json!({ "description": ..., "always_on": ..., "keywords": ..., "version": ... }).to_string() }`.
- Tool enumeration: iterate `tool_registry` for builtins + skill-owned tools + MCP tools (as-of-boot). Each becomes `tool:<name>`. Properties include `source` (`builtin` / `skill:<name>` / `mcp:<server>`) and `description`.
- Agent enumeration: iterate `agent_configs`. Each becomes `agent:<name>`. Properties include `role`, `model`. **Do NOT include `enabled_skills`** — that's state, not structure (D1/D2).
- ProblemType enumeration: `const PROBLEM_TYPE_SEEDS: &[&str] = &["ci_failure", "merge_conflict", "duplicate_pr", "stale_uuid", "fabrication"]`. Each becomes `problem_type:<slug>` with an empty properties object (expanded later by subject-graph linkage).
- Duplicate detection: if two skills expose the same tool name, that's ONE Tool node with properties describing both sources (the `properties_json` should capture `sources: [{"skill": "X"}, {"skill": "Y"}]`). Don't create two separate Tool nodes.

**Patterns to follow:**
- `SkillRegistry::entries()` iteration shape in `crates/mika-agent/src/skills/mod.rs`.
- `ToolRegistry` struct access in `crates/mika-agent/src/tools/mod.rs`.

**Test scenarios:**
- Happy path: mock registries with 2 skills, 3 tools, 1 agent → `DesiredState` has 2 skill entities, 3 tool entities, 1 agent entity, 5 problem_type entities.
- Edge case: two skills expose the same tool → ONE Tool entity in `DesiredState` with both sources noted in properties.
- Edge case: skill with no dependencies field → no DEPENDS_ON edges, no crash.
- Edge case: empty agent_configs → zero agent entities, still produces the 5 problem_type seeds.

**Verification:**
- All test scenarios pass.
- Output is deterministic (same input → same `DesiredState`).

---

- [ ] **Unit 3: Relationship enumeration**

**Goal:** Extend `enumerate_sources()` (or add a companion method) to compute `DEPENDS_ON` and `PROVIDES` edges.

**Requirements:** R2.

**Dependencies:** Unit 2.

**Files:**
- Modify: `crates/mika-agent/src/kg/domain_builder.rs`

**Approach:**
- `DesiredEdge { from_entity_key: String, to_entity_key: String, type: String, properties_json: Option<String> }`.
- DEPENDS_ON: for each skill with non-empty `dependencies`, one edge `skill:<from> -[DEPENDS_ON]-> skill:<to>` per dependency.
- PROVIDES: for each skill, one edge `skill:<skill_name> -[PROVIDES]-> tool:<tool_name>` per tool in `manifest.handler.tools`. Also include MCP-server-owned tools: for each MCP server's tool list, no skill owns it, so PROVIDES edges come only from skill manifests. (If an MCP tool is not provided by any skill, it has no incoming PROVIDES edge — the Tool node still exists with `source: "mcp:<server>"`.)
- **Apply the D1 heuristic during enumeration**: if a candidate edge's truth value would vary per agent (e.g., HAS_SKILL) or require LLM inference, it is NOT enumerated here. This is a checklist principle, not a code guard — the enumeration code simply doesn't produce such edges.

**Patterns to follow:**
- Skill manifest access pattern: `skill.manifest.skill.dependencies.iter()` and `skill.manifest.handler.tools.iter()` (verify exact field names against `skills/manifest.rs`).

**Test scenarios:**
- Happy path: skill X depends on skill Y → one DEPENDS_ON edge. Skill X exposes tool T → one PROVIDES edge.
- Edge case: skill references a dependency that doesn't exist in the registry → log a warning, skip the edge (do NOT create a dangling edge).
- Edge case: MCP tool with no declaring skill → Tool entity exists, no PROVIDES edge into it (acceptable; self-knowledge queries will show the tool with `source: "mcp:<server>"` instead of a providing skill).
- Invariant: no HAS_SKILL or AGENT_HAS_TOOL edges produced under any input shape. Add an explicit assertion in a test ensuring the enumerated edge types are only in `{"DEPENDS_ON", "PROVIDES"}`.

**Verification:**
- All test scenarios pass.
- The "no state-shaped edges" invariant has an explicit test.

---

- [ ] **Unit 4: Idempotent write — UPSERT entities + rebuild relationships**

**Goal:** Implement the write side. `upsert_entities` uses `INSERT ... ON CONFLICT(entity_key) DO UPDATE`. `rebuild_relationships` deletes all domain-sourced relationships and re-inserts. `prune_stale_entities` DELETEs entities no longer in sources.

**Requirements:** R3, R4.

**Dependencies:** Units 2 and 3.

**Files:**
- Modify: `crates/mika-agent/src/kg/domain_builder.rs`
- Modify: `crates/mika-agent/src/db/kg_schema.rs` — add `UPSERT_ENTITY_SQL` const, `DELETE_DOMAIN_RELATIONSHIPS_SQL` const

**Approach:**
- `upsert_entities(&self, desired: &[DesiredEntity]) -> Result<EntityStats>`:
  - For each desired entity, execute the UPSERT. Capture whether the row was inserted (new) or updated (existing rowid preserved).
  - Batch within a single AsyncDatabase closure for transaction atomicity.
- `rebuild_relationships(&self, desired: &[DesiredEdge]) -> Result<EdgeStats>`:
  - DELETE from `kg_relationships` where `type IN ('DEPENDS_ON', 'PROVIDES')` — scoped to domain-sourced types, leaves subject/resolution layer edges untouched.
  - For each desired edge, INSERT (looking up from/to entity_ids by entity_key via JOIN).
  - Returns count per type.
- `prune_stale_entities(&self, desired_keys: &HashSet<String>) -> Result<u32>`:
  - DELETE from `kg_entities` where `entity_key NOT IN (desired_keys)`. CASCADE removes edges; ON DELETE SET NULL on `kg_chunks.entity_id` preserves chunk rows.
  - Returns count of deleted entities.
- All three operations happen within a single transaction — if any step fails, the whole rebuild rolls back and the graph remains in its previous state (stale but consistent).

**Patterns to follow:**
- Transaction boundaries in AsyncDatabase closures: `crates/mika-agent/src/async_db.rs` existing patterns like `index_content`.
- UPSERT syntax: rusqlite supports `INSERT ... ON CONFLICT(col) DO UPDATE SET col = excluded.col, ...`.

**Test scenarios:**
- Happy path: empty DB → rebuild → all desired entities and edges present.
- Idempotency: run rebuild twice with identical sources → second run produces zero inserts, zero updates, zero deletes (or updates with no-op SET).
- Rowid preservation: insert a kg_chunk referencing a skill entity → rebuild with same sources → chunk's entity_id still resolves to the same skill row.
- Entity removal: skill X present on first rebuild, removed on second → second rebuild deletes the skill entity. CASCADE removes DEPENDS_ON/PROVIDES edges touching it. Any kg_chunks pointing at it get entity_id = NULL but the chunk row survives.
- Transaction rollback: inject a DB error mid-rebuild → entire rebuild rolls back, graph is in pre-rebuild state.
- Edge correctness: DEPENDS_ON from skill X to skill Y resolves via entity_key → INSERT succeeds with correct from_entity_id/to_entity_id. If a dependency references an unknown skill (shouldn't happen post-Unit 3 filter), the INSERT fails with a FK violation and rolls back.

**Verification:**
- All test scenarios pass.
- `cargo test -p mika-agent kg::domain_builder` clean.

---

- [ ] **Unit 5: Startup integration + failure policy**

**Goal:** Hook `DomainGraphBuilder::rebuild()` into the server startup sequence after `apply_overrides()`. Failures log a warning and allow startup to continue — KG queries return stale or empty results rather than blocking the server.

**Requirements:** R1, R5, R6.

**Dependencies:** Unit 4.

**Files:**
- Modify: `crates/mika-agent/src/server/mod.rs`

**Approach:**
- Construct the builder after `skill_registry.apply_overrides().await?` and `skill_registry.log_summary()`.
- Call `rebuild().await`. On `Ok(stats)`, log `info!(event="domain_rebuild_complete", ?stats, ...)`. On `Err(e)`, log `warn!(event="domain_rebuild_failed", error=%e, ...)` and continue startup.
- Emit the per-step INFO log lines from within the builder's `rebuild()` method using the builder's `trace_id` (the server-level log line at the end is for post-hoc "did we finish successfully" inspection).

**Patterns to follow:**
- Error handling in `server/mod.rs` startup path — existing `?` / `match` patterns for subsystem initialization.

**Test scenarios:**
- Integration: start a mika-server in test mode with a mock registry → startup succeeds and kg_entities contains the expected rows.
- Failure path: inject a DB error into the builder → server startup still succeeds, log contains the `domain_rebuild_failed` warning, kg_entities is empty but accessible.
- Idempotency in server context: restart the test server → second startup logs `domain_rebuild_complete` with added=0, updated=N, removed=0.

**Verification:**
- Integration test passes.
- Server starts cleanly with domain graph populated in the expected schema.

---

- [ ] **Unit 6: Integration test suite**

**Goal:** End-to-end tests that exercise the builder against realistic mock registries, covering idempotency, staleness, and cross-ticket invariants.

**Requirements:** R3, R4, R5.

**Dependencies:** Units 1–5.

**Files:**
- Create: `crates/mika-agent/tests/kg/domain_builder.rs`
- Create: `crates/mika-agent/tests/kg/mod.rs` (wiring)

**Approach:**
- Helper: `fn mock_registries(skills: &[...], tools: &[...], agents: &[...]) -> (SkillRegistry, ToolRegistry, McpManager, Vec<AgentConfig>)` — minimal scaffolding to produce realistic but deterministic input.
- Test 1 (`builder_populates_fresh_db`): open_in_memory → rebuild → assert every expected entity + edge exists.
- Test 2 (`builder_is_idempotent`): rebuild twice → second run has net-zero changes (may be updates, but no inserts/deletes).
- Test 3 (`builder_preserves_chunk_links`): rebuild → insert a mock kg_chunk with a real entity_id → rebuild again → chunk's entity_id still resolves.
- Test 4 (`builder_deletes_removed_entities`): first rebuild with skill X → second rebuild without skill X → skill X entity is gone, edges touching it are gone, any kg_chunks had entity_id = NULL.
- Test 5 (`builder_produces_no_state_edges`): explicit assertion that `kg_relationships.type` after rebuild is only in `{'DEPENDS_ON', 'PROVIDES'}` (no HAS_SKILL, AGENT_HAS_TOOL, SOLVED_BY, etc.).
- Test 6 (`builder_handles_mcp_tool_without_providing_skill`): mock an MCP server with a tool that no skill declares → Tool entity exists with `source: "mcp:<server>"` and no incoming PROVIDES edge.
- Test 7 (`builder_failure_rollback`): inject a simulated DB error in the middle of rebuild → kg_entities/kg_relationships remain in pre-rebuild state.

**Patterns to follow:**
- `crates/mika-agent/src/test_utils.rs` for settings / DB fixtures.
- Existing integration tests under `crates/mika-agent/tests/` for structure.

**Test scenarios:**
See the seven tests above; each is an explicit test scenario.

**Verification:**
- `cargo test -p mika-agent --test kg` runs the suite green.

## System-Wide Impact

- **Interaction graph:** One new startup hook in `server/mod.rs`. No changes to tool handlers, agent loop, or webhook handlers. The builder is a read-only consumer of SkillRegistry/ToolRegistry/McpManager and a writer to kg_entities/kg_relationships.
- **Error propagation:** Rebuild failure is a `warn!` log, not a panic. Server startup continues. KG queries that depend on populated data get stale/empty results until the next successful rebuild.
- **State lifecycle risks:**
  - **Rowid drift across rebuilds** is the subtle risk — if we accidentally deleted+reinserted entities, every `kg_chunks.entity_id` would become orphaned. D3's UPSERT-by-entity_key strategy prevents this. Integration test 3 (`builder_preserves_chunk_links`) guards against regression.
  - **Transaction atomicity** — the entire rebuild (entity UPSERTs + relationship DELETE+INSERT + entity DELETE) runs in one transaction. Partial rebuilds are not exposed to queries.
- **API surface parity:** None changed. No new tools, no new endpoints, no new CLI subcommands in this ticket. (A future `mika kg rebuild-domain` subcommand is deferred per D3.)
- **Integration coverage:** Unit 6's integration suite covers the full pipeline.
- **Unchanged invariants:** `SkillRegistry`, `ToolRegistry`, `McpManager`, and `skill_overrides` are unchanged. `apply_overrides()` runs at the same point in startup. Existing `search_memory`, `hybrid_search`, and `audit_events` code paths are untouched.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Rowid drift orphans future `kg_chunks.entity_id` references | UPSERT by entity_key preserves rowid. Integration test 3 catches regressions. |
| Rebuild introduces a startup latency bump on servers with many skills | Entity count is bounded (low hundreds at most). Batch writes in one transaction. Measure in Unit 5 — if rebuild exceeds 500ms, revisit batching. |
| Stale MCP tool state between boots surfaces as confused queries | Documented in D4. If it becomes a real operational problem, add a CLI force-rebuild subcommand (deferred). |
| A skill's `dependencies` field references a skill that isn't loaded (missing dep) | Unit 3 skips edges with missing endpoints and logs a warning. The `kg_entities` table stays consistent; only the missing-dep edge is absent. |
| Two skills expose the same tool with different descriptions | Unit 2 dedupes into one Tool entity, records both sources in `properties_json`. First-seen description wins — acceptable because descriptions for the same tool should match by convention. |
| `agents` table contains agents without matching config files (DB-only stubs) | Unit 2 enumerates from `agent_configs`, not the DB. DB-only stubs are invisible to the graph — acceptable, since the graph represents "agents with real config" as nodes. Document if this becomes confusing. |
| HAS_SKILL is "accidentally" added by a future contributor | D1's "structure not state" heuristic + Unit 6 test 5 (explicit invariant assertion) block regression. |

## Documentation / Operational Notes

- The "as of last server boot" staleness contract (D4) needs one sentence in operator-facing documentation (docs/deployment.md or equivalent) when the KG becomes user-visible via #688 and #692. Not required in this ticket — flag for the docs pass in #692.
- If `mika kg rebuild-domain` CLI subcommand is added later, document it alongside existing `mika skills` subcommands.
- Rebuild duration is worth monitoring. The INFO log line `event=domain_rebuild_complete duration_ms=<N>` gives operators a baseline. If duration grows substantially (say, >1s) across releases, investigate whether enumeration has become quadratic in skill count.

## Sources & References

- **Origin ticket:** [mika#687](https://github.com/senara-solutions/mika/issues/687)
- **Milestone:** [mika milestone#14 "Knowledge Graph"](https://github.com/senara-solutions/mika/milestone/14)
- **Depends on:** [mika#686 (schema)](https://github.com/senara-solutions/mika/issues/686)
- **Cross-cutting conventions:** [`docs/architecture/kg-implementation-conventions.md`](../architecture/kg-implementation-conventions.md) (C1, C2 not applicable; C3.1 applies)
- **ID convention:** [`docs/architecture/kg-id-convention.md`](../architecture/kg-id-convention.md) (landed in #686)
- **Related code:**
  - `crates/mika-agent/src/skills/mod.rs` — SkillRegistry
  - `crates/mika-agent/src/tools/mod.rs` — ToolRegistry
  - `crates/mika-agent/src/mcp/` — McpManager
  - `crates/mika-agent/src/server/mod.rs` — startup sequence (hook site)
  - `crates/mika-agent/src/db/kg_schema.rs` — schema constants (from #686)
- **Institutional learnings:**
  - `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md` — sole-writer designation for entity_keys
  - `docs/solutions/database-issues/sql-column-mismatch-trace-detail-view.md` — column constants, no `SELECT *`
  - `docs/solutions/database-issues/trace-id-as-observability-join-key.md` — trace_id-per-rebuild pattern
