# Plan: Split agent.rs (10k) + db.rs (17k) by Operational Domain

**Ticket:** mika#1259
**Type:** refactor
**Date:** 2026-05-31

## Problem Statement

`crates/mika-agent/src/agent.rs` (10,974 lines) and `crates/mika-agent/src/db.rs` (17,569 lines) are architectural risk: migrations, schema, query logic, agent loop policy, guard logic, and prompt/tool orchestration are all co-located, making local reasoning expensive. The mika#1251 investigation demonstrated this concretely — tracing a single tool name through `agent.rs` required reading past 10k lines of unrelated state-machine logic.

## Sequencing Note

This ticket is Layer 3 of the "Operational partner foundation" project. Layer 1 (Task Ledger, mika#1262) has landed — the `operational_items` table and `crates/mika-agent/src/operational/` module exist at schema v39. Layer 2 ("What's Next" engine) consumes that ledger. This refactor restructures the code AROUND the model derived from Layers 1-2, not before it.

The domain boundaries below are derived from the operational model established by Layer 1's `OperationalItem` shape (7 kind variants: goal/task/commitment/decision/blocker/evidence/next_action) and the existing `operational/` module structure (types.rs, write.rs, query.rs, calibration.rs).

## Current State Analysis

### agent.rs (10,974 lines)

| Domain | Lines | Key Items |
|--------|-------|-----------|
| Callback framing | 99–202 | `build_callback_trigger_context()`, `format_callback_framing()`, `AgentOutput` |
| Tool metadata & summaries | 272–453 | `ToolCallSummary`, `tool_calls_metadata_json()`, `format_tool_summary_block()` |
| Loop types & helpers | 455–738 | `ContinuationResult`, `attempt_continuation_turn()`, `strip_prior_images()`, `LoopResult` |
| `LoopMode` + `run_loop()` | 230–269, 804–2299 | The core tool-step loop (~1,500 lines) |
| Conversation agent | 2301–3395 | `AgentContext`, `AgentParams`, `run_agent()`, `run_agent_inner()`, onboarding |
| Tool dispatch | 3028–3346 | `process_tool_calls()`, `execute_tool()` |
| Silent agent | 3396–4143 | `SilentTrigger`, `SilentAgentParams`, `run_silent_agent()` |
| Team agent | 4145–4523 | `TeamAgentParams`, `run_team_agent()` |
| Skill/tool resolution | 4525–5101 | `build_skill_tool_map()`, `inject_skills_and_resolve_tools()`, `apply_agent_tool_visibility()` |
| Post-condition guards | 5103–end | 11 EndTurn guards, `IntentPrecondition`, fabrication detection, all `LazyLock<Regex>` statics |

### db.rs (17,569 lines)

| Domain | Lines | Key Items |
|--------|-------|-----------|
| Sub-module declarations | 1–2 | `pub mod kg_schema; pub mod operational;` |
| Types (structs/enums) | 114–725 | ~40 type definitions across all domains |
| v27 coalesce SQL | 726–867 | Free function for KG migration |
| Schema/migrations | 873–4366 | `open`, `migrate`, `migrate_v1..v39_to_v40` (~3,500 lines) |
| Agent/corpus registry | 4367–4578 | Agent, team, skill override CRUD |
| Task engine DB | 4607–6244 | ~1,640 lines of task CRUD, status transitions, callbacks, dispatch |
| Telemetry (LLM/tool calls) | 6339–6988 | `save_llm_call`, `save_tool_call`, queries |
| Session management | 6246–7009 | Session CRUD, message load/save |
| Messages & conversation | 7010–7295 | Message CRUD, compaction boundary |
| Core memory | 7296–7389 | Core memory CRUD |
| Structured facts | 7390–7718 | People, commitments, preferences, events |
| Audit log | 7719–7986 | Audit event recording and queries |
| Search/embeddings | 8688–8964 | FTS5, vector search, hybrid search |
| Team runs | 8137–8687 | Heartbeat, messaging, team run CRUD |
| Dashboard queries | 9020–9895 | All read-side aggregation for dashboard |
| Agent reset | 10027–10264 | `reset_agent_state()` |
| KG CLI helpers | 10277–10565 | `kg_count_rows`, `purge_kg_for_agent`, etc. |
| Tests | 10567–17569 | ~7,000 lines of tests |

### Existing Infrastructure

- `db/` subdirectory exists with `kg_schema.rs` (503 lines) and `operational.rs` (961 lines)
- `tools/` provides the established one-file-per-concern split pattern
- `operational/` top-level module exists with `types.rs`, `write.rs`, `query.rs`, `calibration.rs`
- `task_engine/` exists with `engine.rs`, `dispatcher.rs`, `queue.rs`, `types.rs`, `cron.rs`, `process_liveness.rs`, `process_kill.rs`

## Proposed Domain Split

### agent.rs → `agent/` directory module

The file becomes `agent/mod.rs` (re-exports + ~200 lines of shared types) with domain sub-modules:

| Module | Source Lines | Contents | Rationale |
|--------|-------------|----------|-----------|
| `agent/mod.rs` | ~200 | `AgentContext`, `AgentOutput`, `LoopMode`, constants, re-exports | Shared types used across all sub-modules |
| `agent/callback.rs` | ~120 | `build_callback_trigger_context()`, `format_callback_framing()` | Self-contained callback context preparation |
| `agent/tool_metadata.rs` | ~200 | `ToolCallSummary`, `tool_calls_metadata_json()`, `format_tool_summary_block()`, `truncate_summary()` | Tool call history serialization |
| `agent/loop_core.rs` | ~1,700 | `run_loop()`, `LoopResult`, `ContinuationResult`, `attempt_continuation_turn()`, `strip_prior_images()`, `save_continuation_llm_call()` | The core tool-step iteration loop — the largest single unit, kept together because `run_loop` is a single function with deep local coupling |
| `agent/conversation.rs` | ~1,100 | `AgentParams`, `run_agent()`, `run_agent_inner()`, `run_agent_with_deadline()`, `check_onboarding()`, `seed_user_person()`, `persist_deadline_fallback()`, `load_gated_summary()` | Conversation-mode entry points |
| `agent/silent.rs` | ~750 | `SilentTrigger`, `SilentAgentParams`, `run_silent_agent()`, `run_silent_inner()` | Silent/background mode entry points |
| `agent/team.rs` | ~380 | `TeamAgentOutcome`, `TeamAgentParams`, `run_team_agent()`, `run_team_agent_inner_impl()` | Team mode entry points |
| `agent/skill_resolution.rs` | ~580 | `build_skill_tool_map()`, `resolve_skill_llm_override()`, `inject_skills_and_resolve_tools()`, `apply_agent_tool_visibility()`, `collect_required_*()`, `emit_system_prompt_assembled()` | Skill and tool injection/resolution |
| `agent/guards.rs` | ~1,800+ | `IntentPrecondition`, `INTENT_GUARDS`, all detection functions, all `LazyLock<Regex>` statics, milestone/fabrication/grounding guards | Post-condition EndTurn guard chain |
| `agent/tool_dispatch.rs` | ~320 | `process_tool_calls()`, `execute_tool()` | Tool execution dispatch (if cleanly separable from `loop_core`; otherwise stays in `loop_core`) |

**Estimated `agent/mod.rs` residual:** ~200 lines (down from 10,974).

**Why this differs from the ticket's proposed shape:** The ticket proposed domain boundaries like `task_state/`, `commitments/`, `planning/`, `evidence/`. Those map to the *operational model* (Layer 1-2), not to `agent.rs`'s actual contents. `agent.rs` contains the *agent loop machinery* — how the LLM is invoked, how tools are dispatched, how post-conditions are enforced. The operational model domains are already correctly placed in `operational/`, `task_engine/`, `db/`. Splitting `agent.rs` by its actual operational domains (loop, guards, modes, skill resolution) is the correct decomposition.

### db.rs → `db/` directory module

The file becomes `db/mod.rs` (re-exports + `Database` struct + `open`/`migrate` + shared helpers) with domain sub-modules:

| Module | Source Lines | Contents | Rationale |
|--------|-------------|----------|-----------|
| `db/mod.rs` | ~300 | `Database` struct, `open()`, `open_in_memory()`, constants, `format_ts()`, `today_midnight_utc()`, `format_age()`, re-exports | Core struct + construction |
| `db/types.rs` | ~610 | All struct/enum definitions (40+ types) | Type definitions consumed across all domain modules |
| `db/migrations.rs` | ~3,500 | `current_version()`, `migrate()`, `migrate_v1..v39_to_v40()`, `column_exists()`, `v27_coalesce_sql()` | Schema evolution — the single largest chunk, naturally isolated |
| `db/registry.rs` | ~210 | Agent, team, skill override CRUD | Agent/team registration and skill override management |
| `db/tasks.rs` | ~1,640 | All task engine DB methods | Task CRUD, status transitions, callbacks, dispatch coordination |
| `db/sessions.rs` | ~760 | Session CRUD | Session lifecycle management |
| `db/messages.rs` | ~480 | Message CRUD, compaction boundary | Message persistence and retrieval |
| `db/memory.rs` | ~420 | Core memory + structured facts (people, commitments, preferences, events) | All memory-layer DB operations |
| `db/audit.rs` | ~270 | Audit event recording and queries | Audit trail |
| `db/telemetry.rs` | ~650 | LLM call + tool call storage and queries | Observability data persistence |
| `db/search.rs` | ~280 | FTS5, vector search, hybrid search, content indexing | Search infrastructure |
| `db/team_runs.rs` | ~550 | Heartbeat, reflection, messaging, team run CRUD, team workspace | Team execution tracking |
| `db/dashboard.rs` | ~880 | All read-side aggregation for dashboard API | Dashboard query surface |
| `db/agent_reset.rs` | ~240 | `ResetAgentCounts`, `count_agent_state()`, `reset_agent_state()` | Agent state reset (CLI) |
| `db/kg_cli.rs` | ~290 | KG row counts, purge, orphan check, resolution stats | KG CLI helpers |
| `db/kg_schema.rs` | 503 (existing) | KG table/column name constants | Already split |
| `db/operational.rs` | 961 (existing) | Operational items DB methods | Already split |

**Estimated `db/mod.rs` residual:** ~300 lines (down from 17,569 excluding tests).

### Test Distribution

Tests (~7,000 lines in `db.rs`) follow their respective modules:

| Test Module | Follows |
|-------------|---------|
| `db/tests/migrations.rs` | `db/migrations.rs` |
| `db/tests/tasks.rs` | `db/tasks.rs` |
| `db/tests/sessions.rs` | `db/sessions.rs` |
| `db/tests/messages.rs` | `db/messages.rs` |
| `db/tests/memory.rs` | `db/memory.rs` |
| `db/tests/audit.rs` | `db/audit.rs` |
| `db/tests/telemetry.rs` | `db/telemetry.rs` |
| `db/tests/search.rs` | `db/search.rs` |
| `db/tests/team_runs.rs` | `db/team_runs.rs` |
| `db/tests/dashboard.rs` | `db/dashboard.rs` |
| `db/tests/registry.rs` | `db/registry.rs` |

Each test module uses `#[cfg(test)]` gating. Shared test helpers (`test_db()`, `test_async_db()`) stay in `db/mod.rs` or a dedicated `db/test_helpers.rs`.

**Alternative:** Keep tests inline as `#[cfg(test)] mod tests` within each domain module (matching the current Rust convention used elsewhere in the crate). This avoids a separate `tests/` directory and keeps tests co-located with their code. Decision: **inline tests** — matches existing crate convention and reduces the number of files.

## Implementation Strategy

### Phase 1: db.rs split (17,569 → ~300 residual)

**Why db.rs first:** It has the most lines, the simplest internal coupling (methods on a single `Database` struct with no cross-method calls between domains), and the `db/` subdirectory already exists with two modules. The split is mechanical: move `impl Database` blocks and their types to sub-modules.

**Technique:** Each new module gets `use super::Database;` and implements methods via `impl Database { ... }` blocks. Rust allows multiple `impl` blocks for the same struct across modules within a crate. Types move to `db/types.rs` and are re-exported from `db/mod.rs`.

**Steps:**

1. **Create `db/types.rs`** — Move all struct/enum definitions (lines 114–725) from `db.rs`. Add `pub use types::*;` to `db/mod.rs`.

2. **Create `db/migrations.rs`** — Move `current_version()`, `migrate()`, all `migrate_v*()` functions, `column_exists()`, `check_v27_coalesce_guard()`, `v27_coalesce_sql()`. This is the largest single chunk (~3,500 lines) and the most self-contained.

3. **Create `db/tasks.rs`** — Move all task-related methods. These are well-bounded by the `Task`/`NewTask` types.

4. **Create `db/sessions.rs`** — Move session CRUD methods.

5. **Create `db/messages.rs`** — Move message CRUD and compaction boundary methods.

6. **Create `db/memory.rs`** — Move core memory + structured facts methods (people, commitments, preferences, events).

7. **Create `db/telemetry.rs`** — Move LLM call and tool call storage/query methods.

8. **Create `db/audit.rs`** — Move audit event methods.

9. **Create `db/search.rs`** — Move FTS5/vector/hybrid search methods.

10. **Create `db/team_runs.rs`** — Move heartbeat, reflection, messaging, team run CRUD methods.

11. **Create `db/dashboard.rs`** — Move all dashboard read-side aggregation methods.

12. **Create `db/registry.rs`** — Move agent/team/skill override CRUD.

13. **Create `db/agent_reset.rs`** — Move `ResetAgentCounts` and reset methods.

14. **Create `db/kg_cli.rs`** — Move KG CLI helper methods.

15. **Verify:** `cargo test -p mika-agent` passes. `cargo clippy -p mika-agent` clean.

### Phase 2: agent.rs split (10,974 → ~200 residual)

**Why agent.rs second:** It has more complex internal coupling — `run_loop()` calls `process_tool_calls()` which calls `execute_tool()`, guards reference types from the loop, etc. The split requires careful attention to visibility.

**Technique:** Convert `agent.rs` to `agent/mod.rs` + sub-modules. Functions become `pub(crate)` where needed by sibling modules. Types used across sub-modules are defined in `mod.rs` or a shared `types.rs` sub-module.

**Steps:**

1. **Create `agent/guards.rs`** — Move all post-condition guard logic (the largest chunk, ~1,800+ lines). This is the most self-contained: guards take `&str` text, `&[ToolCallSummary]`, and `&HashSet<String>` — no deep coupling to the loop. Move all `LazyLock<Regex>` statics, `IntentPrecondition`, detection functions.

2. **Create `agent/silent.rs`** — Move `SilentTrigger`, `SilentAgentParams`, `run_silent_agent()`, `run_silent_inner()`. Self-contained entry point that calls `run_loop()`.

3. **Create `agent/team.rs`** — Move `TeamAgentOutcome`, `TeamAgentParams`, `run_team_agent()`. Self-contained entry point that calls `run_loop()`.

4. **Create `agent/conversation.rs`** — Move `AgentParams`, `run_agent()`, `run_agent_inner()`, `run_agent_with_deadline()`, onboarding, summary loading. Self-contained entry point that calls `run_loop()`.

5. **Create `agent/skill_resolution.rs`** — Move skill/tool injection and resolution helpers.

6. **Create `agent/callback.rs`** — Move callback framing functions.

7. **Create `agent/tool_metadata.rs`** — Move `ToolCallSummary` and formatting functions.

8. **Create `agent/loop_core.rs`** — Move `run_loop()`, `LoopResult`, `ContinuationResult`, continuation helpers, `strip_prior_images()`. This is the gravitational center — it calls guards, tool dispatch, and skill resolution. Keep `process_tool_calls()` and `execute_tool()` here (they are called inline from `run_loop()` and share too much local state to split cleanly).

9. **Clean up `agent/mod.rs`** — Keep `AgentContext`, `AgentOutput`, `LoopMode`, constants, re-exports.

10. **Verify:** `cargo test -p mika-agent` passes. `cargo clippy -p mika-agent` clean.

### Phase 3: Verification and documentation

1. **Verify all tests pass:** `cargo test -p mika-agent` (all ~3,463 tests).
2. **Verify eval tests pass:** `cargo test -p mika-agent --test eval`
3. **Verify build:** `cargo build`
4. **Verify lint:** `cargo clippy -p mika-agent`
5. **Add module doc-comments:** Each new module gets a one-paragraph `//!` doc-comment naming its operational responsibility.
6. **Verify line counts:** Both `agent/mod.rs` and `db/mod.rs` should be under ~2k lines (target from AC).

## Dependency Graph (Split Ordering)

```
db.rs split (Phase 1) — no agent.rs dependency
  ├── types.rs (first — other modules import these)
  ├── migrations.rs (second — standalone)
  ├── all other domain modules (parallel, independent)
  └── tests move inline with their domain module

agent.rs split (Phase 2) — depends on db.rs being stable
  ├── guards.rs (first — most self-contained)
  ├── silent.rs, team.rs, conversation.rs (parallel — each is an entry point)
  ├── skill_resolution.rs, callback.rs, tool_metadata.rs (parallel — helper modules)
  └── loop_core.rs (last — imports from guards, tool_metadata, skill_resolution)
```

## Risk Mitigation

1. **No behavior change.** This is a pure module split. Every function, struct, and impl block moves verbatim. No logic changes.

2. **Test preservation.** All existing tests move with their code. `cargo test -p mika-agent` must produce the same pass count before and after.

3. **Incremental commits.** Each phase step gets its own commit. If any step breaks tests, it's immediately visible and revertable.

4. **Visibility management.** Functions currently `pub` stay `pub`. Functions currently private become `pub(crate)` only when needed by sibling sub-modules. No new public API surface.

5. **Import path stability.** Re-exports from `db/mod.rs` and `agent/mod.rs` mean that external callers (other crates, tests) continue to use `crate::db::Task`, `crate::agent::run_agent()`, etc. No import path changes for consumers.

## Acceptance Criteria Mapping

| AC | How Met |
|----|---------|
| Domain boundaries from Layer 1's foundation doc | `operational/` module structure (types/write/query/calibration) is preserved. The `db/` split boundaries (tasks, memory, audit) align with OperationalItem's kind variants. The ticket's sketch proposed `task_state/`, `commitments/`, etc. — those map to `db/tasks.rs` and `db/memory.rs` respectively. The `agent.rs` split follows the actual code structure because `agent.rs` contains loop machinery, not operational model logic. |
| `cargo test -p mika-agent` passes unchanged | Phase 3 step 1 — tests move inline with code, no test changes |
| No behavior change | Pure module split — function bodies unchanged |
| Each module has doc-comment | Phase 3 step 5 |
| `agent.rs` and `db.rs` each drop below ~2k lines | `agent/mod.rs` target ~200 lines, `db/mod.rs` target ~300 lines |
| mika#1253 (loader-engine parity) lands at/after | This refactor creates `agent/skill_resolution.rs` where the assertion naturally lives |
| mika#1254 (fabrication guard audit) lands at/after | This refactor creates `agent/guards.rs` where predicates naturally live |

## Estimated Effort

- Phase 1 (db.rs split): ~2-3 hours of mechanical refactoring
- Phase 2 (agent.rs split): ~2-3 hours (more coupling to untangle)
- Phase 3 (verification): ~30 minutes
- **Total: ~5-6 hours**

The refactoring is mechanical (move + re-export) but the volume is high. The main risk is getting visibility modifiers right on the first pass.
