# Plan — refactor(mika#1259): extract memory/ module (mika#1446)

## Phase 0 — Pin

**A. Foundation §6** (`mika/docs/architecture/operational-partner-frame.md`):
> `memory/` — Core memory, structured facts, search, KG bridges. Reads `OperationalItem` to inform retrieval; writes Evidence on persistence ops.

**B. Sibling-accretion from #1445 (Wave 2.1) — 5 methods classified as memory/ during dashboard_queries/ grooming:**

| Line | Function | Classification grounds |
|---|---|---|
| 4383 | `list_agent_corpora` | KG bridges per §6 |
| 7517 | `list_people` | Person = memory-layer entity ("structured facts") |
| 7691 | `list_preferences` | Preferences = memory-layer per §6 |
| 7743 | `list_events` | Events = facts-of-state (memory) — confirmed via body-read at this canvass (no longer provisional) |
| 9032 | `list_facts_paginated_with_count` | "Structured facts" per §6 |

**Sibling-accretion validation outcome**: all 5 accreted classifications confirm at memory/ body-read. **Sibling-accretion-as-grounding-input mechanism validated at n=1** — accreted classifications were correct; #1446 grounding-gate firing absorbs them without conflict.

**C. Additional memory/ methods surfaced via #1446 body-read** (memory-relevant methods NOT yet in any sub-issue's accretion):

| Line | Function | Domain | In #1446 scope? |
|---|---|---|---|
| 7363 | `get_core_memory` | Core memory access | YES |
| 7382 | `get_all_core_memory` | Core memory bulk-read | YES |
| 7401 | `set_core_memory` | Core memory write | YES |
| 7457 | `upsert_person` | People write | YES |
| 7492 | `get_person` | People read | YES |
| 7680 | `get_preference` | Preferences read | YES |
| 8054 | `delete_person_by_name` | People delete | YES |
| 8082 | `delete_preference` | Preferences delete | YES |
| 8140 | `delete_event_by_description` | Events delete | YES |
| 8982 | `get_all_facts_for_indexing` | Facts bulk-read (indexing path) | YES |

**Total #1446 scope**: 5 accreted + 10 newly surfaced = 15 methods. Estimated LoC: ~1,500-2,500 lines (smaller per-method bodies than dashboard_queries paginated aggregations).

**D. What stays OUT of memory/:**

- **`kg/` module dir already exists** at `crates/mika-agent/src/kg/` with budget.rs, chunker.rs, config.rs, domain_builder.rs, entity_resolver.rs, ingestion_orchestrator.rs, lexical_ingestor.rs, query.rs, resolver_tick.rs. **kg/ is NOT absorbed into memory/.** Foundation §6 memory/ has "KG bridges" — i.e., the integration-layer methods on Database that bridge to kg/'s machinery. kg/'s own modules stay as-is.
- **`db/kg_schema.rs` and `db/operational.rs`** stay where they are. These are schema/migration code, not domain-method-owner code. db/ is its own infrastructure layer.
- **Audit-events methods split**: `list_audit_events_paginated*` (9495, 9813) went to dashboard_queries/ in #1445 (dashboard surface). `get_audit_events*` (7834, 7846, 7929) belong to **evidence/** per §6 ("tool-call audit trail") — NOT memory/. Belong to #1444 evidence/ grooming.

**E. Cross-module dependency check (grep for cross-§6-module calls):**

The 15 methods read `agents`, `core_memory`, `people`, `preferences`, `events`, `facts`, `agent_kg_corpora` tables. They do NOT call other §6 module methods (no `evidence::*`, no `task_state::*`, no `commitments::*`, no `dashboard_queries::*`). Pure leaf-with-respect-to-§6.

Memory/ does have a soft-dependency on **kg/** (KG bridges call into kg/ machinery), but kg/ is NOT a §6 module — it's infrastructure that memory/ depends on. That's not a §6-module-cross-dependency.

**F. Tests landscape**: existing tests reference these methods via `database.get_core_memory(...)`, `database.set_core_memory(...)`, etc. After extraction (methods stay `impl Database`), tests don't need changes.

## Hypothesis (committed)

**Extraction shape**: move the 15 memory/-domain methods from `db.rs` into `crates/mika-agent/src/memory/mod.rs`. Methods stay as `impl Database` to minimize call-site churn (per #1445's established pattern and parent AC3).

**Sibling-accretion bookkeeping**: the 5 accreted methods are explicit + verified at canvass-time, not silently absorbed. Phase 0 §B is the audit trail.

**Why NOT absorb kg/**: kg/ is an infrastructure module (ingestion orchestrator, chunker, entity resolver, etc.) that pre-dates Foundation §6 and serves multiple consumers. §6 memory/ provides KG-bridge methods on Database that callers use; kg/ provides the machinery those bridges drive. Different layers, different ownership.

## Approach (committed)

### A. Create the module

`crates/mika-agent/src/memory/mod.rs`:

```rust
//! Core memory, structured facts, search, and KG bridges.
//!
//! Owns Database query/write methods for:
//! - Core memory (key-value identity blocks injected per-turn)
//! - People (entity records used in conversation context)
//! - Preferences (operator-stamped routing hints)
//! - Events (factual record of state-of-world entries)
//! - Facts (semantic memory store)
//! - KG corpora bridges (integration with `crate::kg`'s ingestion + resolution)
//!
//! Per Foundation §6: reads `OperationalItem` to inform retrieval;
//! writes Evidence on persistence ops (evidence/ owns the actual Evidence
//! writes — memory/ owns the persistence side that triggers them).

use crate::db::Database;
// ... (move impl blocks from db.rs into this file)
```

### B. Move impl blocks

Migrate the 15 method definitions per Phase 0 §B + §C into `memory/mod.rs`. Each method's body unchanged; only the enclosing `impl Database { ... }` block relocates.

Group by data-kind in the new file:
- Core memory (get_core_memory, get_all_core_memory, set_core_memory)
- People (upsert_person, get_person, list_people, delete_person_by_name)
- Preferences (get_preference, list_preferences, delete_preference)
- Events (list_events, delete_event_by_description)
- Facts (get_all_facts_for_indexing, list_facts_paginated_with_count)
- KG bridges (list_agent_corpora)

### C. Update `lib.rs`

Add `pub mod memory;` to `crates/mika-agent/src/lib.rs`.

### D. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `wc -l crates/mika-agent/src/db.rs` shows ~1,500-2,500 line reduction

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/memory/mod.rs` created with one-paragraph doc-comment naming "core memory, structured facts, search, KG bridges" per Foundation §6 (per parent AC4).

2. **AC2**: Exactly the 15 classified memory/-domain methods (per Phase 0 §B + §C tables) relocate to `memory/mod.rs`. Methods stay `impl Database` to minimize call-site churn.

3. **AC3**: `crates/mika-agent/src/lib.rs` declares `pub mod memory;` (per parent AC4).

4. **AC4**: `cargo test -p mika-agent` passes unchanged (per parent AC2).

5. **AC5**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

6. **AC6**: No behavior change — pure module split (per parent AC3).

7. **AC7**: `wc -l crates/mika-agent/src/db.rs` shows reduction by ~1,500-2,500 lines.

8. **AC8**: kg/ module dir untouched (not absorbed). Verified by `git diff crates/mika-agent/src/kg/` returning empty.

## Files to change

- `crates/mika-agent/src/memory/mod.rs` — new file (extracted memory methods + doc-comment)
- `crates/mika-agent/src/db.rs` — remove relocated methods
- `crates/mika-agent/src/lib.rs` — add `pub mod memory;` declaration

## Out of scope

- HTTP handlers in `server/dashboard.rs` (transport layer; dashboard_queries already extracted, dashboard surfaces still call `database.list_people()` etc. via the relocated `impl Database` block — no signature change)
- kg/ machinery (separate infrastructure layer, pre-dates §6, multiple consumers)
- db/kg_schema.rs and db/operational.rs (db/ infrastructure, not domain-owner)
- audit_events methods (split between dashboard_queries/ already + evidence/ for #1444)
- Other §6 modules (#1444 evidence/, #1447 notifications/, etc. — separate sub-issues)

## Risk

Low (matches #1445's risk profile).
- **Impl-block relocation across files**: standard Rust; compiler stitches automatically; no call-site churn.
- **Sibling-accretion validity**: validated at body-read — all 5 accreted classifications hold.
- **kg/ scope clarity**: Phase 0 §D explicitly excludes kg/ absorption to prevent scope-creep. Future #1444 evidence/ grooming might surface additional kg/-related-but-not-memory/ methods; those route via separate sub-issue scoping.

## Test plan

1. `cargo build -p mika-agent` clean
2. `cargo test -p mika-agent --lib` passes (relies on `database.get_core_memory()`, `database.set_core_memory()`, `database.upsert_person()`, etc. — these still resolve to memory/mod.rs's impl Database block)
3. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
4. `git diff crates/mika-agent/src/kg/` returns empty (kg/ untouched)
5. `wc -l` on db.rs shows ~1,500-2,500 line reduction

## Implementation order

1. Create `memory/mod.rs` with doc-comment shell + `use crate::db::Database;`
2. For each of the 15 methods (Phase 0 §B + §C), copy from db.rs into memory/mod.rs preserving impl Database block + comments + tests-references
3. Remove the 15 methods from db.rs
4. Add `pub mod memory;` to lib.rs
5. Run `cargo build` — fix compile errors if any (unlikely — pure relocation)
6. Run `cargo clippy` + `cargo test`
7. Manual smoke per §test plan

## Sibling-accretion mechanism observation (for current_priorities ledger)

**n=1 of sibling-accretion-as-grounding-input** validated at this canvass. The mechanism shape:

- A sibling sub-issue's grounding-gate firing produces classification claims about methods that belong to OTHER sub-issues' scope
- Those classifications accrete into the not-yet-groomed siblings' scope-evidence
- The next sibling's grounding-gate firing verifies-or-refines the accretion
- Verified accretion (this canvass) → mechanism validated; refined/refuted accretion → mechanism + classifications get refined

Held lightly; not folded into bedrock. If a second sibling firing also validates inherited classifications, that's n=2 evidence the mechanism is stable across sub-issues.
