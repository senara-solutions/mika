# Plan — refactor(mika#1259): extract dashboard_queries/ module (mika#1445)

## Phase 0 — Pin

**A. Foundation §6** (`mika/docs/architecture/operational-partner-frame.md`):
> `dashboard_queries/` — Read-side aggregation for dashboard surfaces. Reads only; `OperationalItem` is the canonical join target.

**B. Existing dashboard surface** (`crates/mika-agent/src/server/dashboard.rs`, 1508 lines):
24 HTTP handlers (`handle_timeline`, `handle_agents_list`, `handle_agent_detail`, `handle_sessions_list`, `handle_tasks_list`, `handle_cost_trend`, etc.). **These are HTTP-handler glue, NOT pure-read-aggregation logic.** They call into `db.rs` query methods. Handler glue stays where it is — operationally it belongs to the `server/` infrastructure layer, not §6's `dashboard_queries/` (which is a domain-of-OperationalItem-reads, not a transport layer).

**C. Dashboard-aggregation query methods in `db.rs`** (read-only, dashboard-surface-relevant):

| Function | Line | Purpose |
|---|---|---|
| `list_agents_with_stats` | 9194 | agent list w/ aggregated stats |
| `get_agent_with_stats` | 9217 | single agent detail w/ stats |
| `list_sessions_paginated` | 9242 | sessions list w/ pagination |
| `list_audit_events_paginated` | 9495 | audit events w/ pagination |
| `list_tasks_paginated` | 9566 | tasks w/ pagination |
| `list_team_runs_paginated` | 9694 | team runs w/ pagination |
| `list_sessions_paginated_with_count` | 9775 | sessions + total count |
| `list_audit_events_paginated_with_count` | 9813 | audit events + total count |
| `list_tasks_paginated_with_count` | 9828 | tasks + total count |
| `list_team_runs_paginated_with_count` | 9840 | team runs + total count |
| `list_dev_runs_paginated_with_count` | 9877 | dev runs + total count |

**Additional `list_*` methods classified per §6 module ownership** (per architect F1 — body-read each method's domain):

| Line | Function | §6 module owner | In #1445 scope? |
|---|---|---|---|
| 4383 | `list_agent_corpora` | memory/ (KG corpora = "KG bridges" per §6) | NO — belongs to #1446 memory/ |
| 4423 | `list_agents_db` | dashboard_queries/ (core agents list, read-only aggregation) | YES |
| 4588 | `list_teams_db` | dashboard_queries/ (teams list, read-only) | YES |
| 5044 | `list_manual_tasks` | task_state/ (Task-kind owner per §6: "task lifecycle") | NO — belongs to #1448 task_state/ |
| 5194 | `list_active_tasks` | task_state/ (Task-kind owner) | NO — belongs to #1448 task_state/ |
| 7517 | `list_people` | memory/ (Person = memory-layer entity per §6: "structured facts") | NO — belongs to #1446 memory/ |
| 7599 | `list_commitments` | commitments/ (Commitment-kind owner per §6: "promise tracking") | NO — belongs to #1449 commitments/ |
| 7691 | `list_preferences` | memory/ (preferences = memory-layer per §6: "structured facts") | NO — belongs to #1446 memory/ |
| 7743 | `list_events` | memory/ (events = facts-of-state per §6: "structured facts") — ambiguous; classify as memory/ pending body-read at #1446 grooming | NO (provisional) — flag for #1446 grooming review |
| 8375 | `list_customer_config` | dashboard_queries/ (config-read for dashboard surfaces; not a §6-domain-owner) | YES |
| 9032 | `list_facts_paginated_with_count` | memory/ ("structured facts" per §6) | NO — belongs to #1446 memory/ |

**Reduced #1445 scope**: only `list_agents_db` (4423), `list_teams_db` (4588), `list_customer_config` (8375) from the secondary group qualify as dashboard_queries/. The other 8 line-pinned methods belong to sibling §6 modules (#1446 memory/, #1448 task_state/, #1449 commitments/) and stay in db.rs until those sub-issues groom.

**Revised LoC estimate**: the 11 methods at 9194-9877 are the bulk (~1,200-1,500 LoC). Plus list_agents_db + list_teams_db + list_customer_config (~150-250 LoC combined). **Total #1445 scope: ~1,400-1,750 lines moved.**

**D. Cross-module dependencies** (verified via grep on each candidate function):

- The aggregation queries read from `agents`, `sessions`, `messages`, `audit_events`, `tasks`, `team_runs`, `llm_calls`, `tool_calls`, `dev_runs` tables.
- They do NOT call other §6 module methods (no `evidence::*`, no `task_state::*`, no `memory::*`, etc. — confirmed since none of those modules exist yet, but the underlying primitives are db.rs-internal anyway).
- Pure read-only — no writes.

**Leaf-confirmation**: dashboard_queries is a true leaf in Foundation §6's dependency-graph. Reads OperationalItem (when those tables are queried) but doesn't depend on other §6 modules. Extracting first stocks the dependency-graph leaf-position before other §6 modules need to import from it (which they likely won't — dashboard_queries is read-only, intended for HTTP-handler-callers in `server/`).

**E. Tests landscape**:
Existing tests for these query methods live in `tests/` or `crates/mika-agent/tests/`. After extraction, tests that import the moved methods need updated `use` paths (`mika_agent::db::list_agents_with_stats` → `mika_agent::dashboard_queries::list_agents_with_stats` or similar, depending on whether the methods stay `impl Database` or become free-functions).

## Hypothesis (committed)

**Extraction shape**: move the 11+ dashboard-aggregation query methods from `db.rs` into `crates/mika-agent/src/dashboard_queries/mod.rs`. Methods stay as `impl Database` (no signature change) to minimize call-site churn — only the `use` paths change in `server/dashboard.rs`.

**Why `impl Database` not free-functions**:
- All existing methods are `impl Database` with `&self` receivers. Free-function refactor would force every call-site to pass `&db` explicitly. Cross-cuts the 24 HTTP handlers + any other internal callers.
- The §6 module-boundary is a *logical responsibility ownership* boundary, not a Rust-namespace-purity boundary. `impl Database` blocks can live across multiple files (Rust splits `impl` blocks naturally).
- Tests using these methods don't need changes — `database.list_agents_with_stats()` still works after extraction because the `impl Database` block is just relocated.

This matches the parent #1259 AC3 ("pure module split; logic identical").

## Approach (committed)

### A. Create the module

`crates/mika-agent/src/dashboard_queries/mod.rs`:

```rust
//! Read-side aggregation for dashboard surfaces.
//!
//! Owns the dashboard-relevant query methods on [`Database`]: paginated
//! lists of agents, sessions, tasks, team runs, audit events, dev runs;
//! single-row detail queries with aggregated stats; cost-trend rollups.
//!
//! Per Foundation §6 (`docs/architecture/operational-partner-frame.md`):
//! reads only; `OperationalItem` is the canonical join target as the
//! Layer-1 ledger matures.
//!
//! All methods stay as `impl Database` so call-sites (HTTP handlers in
//! `server/dashboard.rs`, tests) don't need signature updates — only
//! the path under `mika_agent::dashboard_queries::Database::*`.

use crate::db::Database;
// ... (move impl blocks from db.rs into this file)
```

### B. Move impl blocks

Migrate the 11+ method definitions from `db.rs:9194-9877` (and the `list_*` methods at lines 4383, 4423, etc.) into `dashboard_queries/mod.rs`. Each method's body unchanged; only the enclosing `impl Database { ... }` block relocates.

Group by query-shape in the new file:
- Agent aggregations (`list_agents_with_stats`, `get_agent_with_stats`)
- Session pagination (`list_sessions_paginated`, `list_sessions_paginated_with_count`)
- Task pagination (similar)
- Team-run pagination
- Audit-event pagination
- Dev-run pagination
- Cost-trend / cost-aggregation (whatever lives at `handle_cost_trend`'s db-side)

### C. Update `lib.rs`

Add `pub mod dashboard_queries;` to `crates/mika-agent/src/lib.rs`.

### D. Update imports if needed

If `server/dashboard.rs` or any other file imports specific method names via `use mika_agent::db::list_agents_with_stats` (rare; impl-method calls don't usually need `use`), update those.

`grep -rn "use .*db::list_agents_with_stats\|use .*db::get_agent_with_stats" crates/` to find any such imports. Likely zero hits — impl-method calls are typically `db.list_agents_with_stats()` which doesn't need `use`.

### E. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `cargo test -p mika-agent --test integration` (or whatever the integration suite is) passes
- `wc -l crates/mika-agent/src/db.rs` shows ~16,000 lines (down from 17,645 — confirms ~1,500 LoC moved)

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/dashboard_queries/mod.rs` created with one-paragraph doc-comment naming "read-side aggregation for dashboard surfaces" per Foundation §6 (per parent AC4).

2. **AC2**: Exactly the 14 classified dashboard-aggregation methods (per Phase 0 §C revised table) relocate to `dashboard_queries/mod.rs`. Methods stay `impl Database` to minimize call-site churn. The other 8 line-pinned `list_*` methods (memory/-grade, task_state/-grade, commitments/-grade) stay in db.rs for sibling §6 module extractions.

3. **AC3**: `crates/mika-agent/src/lib.rs` declares `pub mod dashboard_queries;` (per parent AC4).

4. **AC4**: `cargo test -p mika-agent` passes unchanged (per parent AC2). No test signature updates required.

5. **AC5**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

6. **AC6**: No behavior change — pure module split, query semantics identical (per parent AC3). Verified by test suite.

7. **AC7**: `wc -l crates/mika-agent/src/db.rs` shows reduction by ~1,400-1,750 lines (from 17,645 to ~15,900-16,250). Confirms the extraction-volume claim per Phase 0 §C revised classification.

## Files to change

- `crates/mika-agent/src/dashboard_queries/mod.rs` — new file (extracted query methods + doc-comment)
- `crates/mika-agent/src/db.rs` — remove relocated query methods (preserve the surrounding `impl Database` block structure for sibling methods that stay)
- `crates/mika-agent/src/lib.rs` — add `pub mod dashboard_queries;` declaration

No test file changes anticipated (impl-method calls don't require `use` updates).

## Out of scope

- HTTP handlers in `server/dashboard.rs` (transport layer, not §6's dashboard_queries domain)
- Other Foundation §6 modules (#1444 evidence/, #1446 memory/, etc. — separate sub-issues)
- Adding new OperationalItem-joined queries (post-decomposition work; future tickets)
- Changing query semantics or pagination shape (pure relocation, per AC6)

## Risk

Low.
- **Method-receiver convention**: keeping `impl Database` blocks across files is standard Rust (the compiler stitches them). No call-site churn.
- **Compiler ordering**: Rust's mod-system handles cross-file impl blocks regardless of declaration order. No compiler-order risk.
- **Cross-cutting query dependencies**: aggregation methods query multiple tables; the extraction doesn't change query bodies, only their file location. SQL stays identical.
- **Grounding-gate first home-position firing risk**: this is the first per-sub-issue grooming on the §1259 decomposition. If body-read surfaces scope-mismatches (e.g., query methods that actually belong to a *different* §6 module), surface immediately — that's exactly what the grounding-gate is for. Plan-time scope (11+ aggregation methods) is the best-evidence shape; canvass-time may refine.

## Test plan

1. `cargo build -p mika-agent` clean
2. `cargo test -p mika-agent --lib` passes
3. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
4. Manual: spawn dev mika-spirit, hit a few dashboard endpoints (`/dashboard/agents`, `/dashboard/sessions`), verify queries return same shape as pre-refactor
5. `wc -l` on `db.rs` post-change confirms ~1,500-line reduction

## Implementation order

1. Per Phase 0 §C classification: move ONLY `list_agents_with_stats` (9194), `get_agent_with_stats` (9217), `list_sessions_paginated` (9242), `list_audit_events_paginated` (9495), `list_tasks_paginated` (9566), `list_team_runs_paginated` (9694), `list_sessions_paginated_with_count` (9775), `list_audit_events_paginated_with_count` (9813), `list_tasks_paginated_with_count` (9828), `list_team_runs_paginated_with_count` (9840), `list_dev_runs_paginated_with_count` (9877), `list_agents_db` (4423), `list_teams_db` (4588), `list_customer_config` (8375) — the 14 dashboard-aggregation-grade methods. Leave the other 8 line-pinned methods (`list_agent_corpora`, `list_manual_tasks`, `list_active_tasks`, `list_people`, `list_commitments`, `list_preferences`, `list_events`, `list_facts_paginated_with_count`) in db.rs for their respective §6 module extractions (#1446 memory/, #1448 task_state/, #1449 commitments/).
2. Build the new `dashboard_queries/mod.rs` with doc-comment + extracted impl block
3. Remove relocated methods from `db.rs`
4. Add `pub mod dashboard_queries;` to `lib.rs`
5. `cargo build` + `cargo clippy` + `cargo test`
6. Manual smoke per §test plan
