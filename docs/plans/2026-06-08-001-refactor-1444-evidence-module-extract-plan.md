# Plan — refactor(mika#1259): extract evidence/ module (mika#1444)

## Phase 0 — Pin

**A. Foundation §6 evidence/ definition:**
> `evidence/` — grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail.

**B. Sibling-accretion check across prior Wave 2 firings:** zero. No methods accreted from #1445 (dashboard_queries), #1446 (memory), #1447 (notifications), or #1449 (commitments). Evidence/ owns its full scope alone — body-read for #1444 anchors all method classifications independently.

**C. Surfaces body-read against current `crates/mika-agent/`:**

### C.1 — db.rs audit_events surface (12 methods + 1 struct + ~5 tests)

`crates/mika-agent/src/db.rs` is 17,645 lines. The audit_events chunk:

**Total: 10 methods + 1 struct + ~5 tests** in evidence/ scope (revised from 12 per F1 — see C.3 below for the 2 paginated methods removed from this scope).

| Symbol | Line | Description |
|---|---|---|
| `pub struct AuditEvent` | 371 | Audit-event row schema (id/agent_id/session_id/tool_name/target_key/before_value/after_value/reasoning/trace_id/rewound_by_trace_id/created_at) |
| `pub fn log_audit_event` | 7786 | INSERT a new audit_event row |
| `fn row_to_audit_event` | 7818 | Row → AuditEvent helper |
| `pub fn get_audit_events` | 7834 | SELECT by agent_id + session_id |
| `pub fn get_audit_events_since` | 7846 | SELECT by agent_id + since-timestamp |
| `pub fn count_audit_events_for_session` | 7858 | COUNT by agent_id + session_id |
| `pub fn compact_old_audit_events` | 7886 | DELETE older than N days |
| `pub fn get_audit_events_by_trace_ids` | 7929 | SELECT by trace_id IN (…) (rewind path) |
| `pub fn mark_audit_events_rewound` | 8023 | UPDATE rewound_by_trace_id (rewind path) |
| `pub fn count_audit_events` | 9518 | Count helper (used by both evidence-domain and dashboard-paginator callers — kept in evidence/ as the count IS audit-event-domain; dashboard_queries/ list-paginators call it via `Database::*`) |
| Tests | 10982-11348 | `test_log_and_get_audit_events`, `test_get_audit_events_since`, `test_compact_old_audit_events`, plus inline test fixtures |

### C.2 — agent.rs fabrication-guard predicate surface

`crates/mika-agent/src/agent.rs` is 11,401 lines. Evidence-domain helpers + regex statics + predicate functions:

| Symbol | Line | Description |
|---|---|---|
| `GITHUB_RESOURCE_URL_RE` static | 5555 | Regex for GitHub URLs in fabrication detection |
| `ACTION_CLAIM_RE` static | 5567 | Regex for action-claim verbs |
| `fn detect_fabricated_action_claim` | 5577 | Fabrication-guard predicate — claims of action via fabricated URL |
| `CALLBACK_STATE_CLAIM_RE` static | 5590 | Regex for callback-turn state claims |
| `fn detect_fabricated_callback_state_claim` | ~5615 | Fabrication-guard predicate — callback-turn state |
| `ASSERTED_UNAVAILABILITY_PATTERNS` static | 6250 | 5 regexes for tool-unavailability assertions |
| `fn detect_asserted_unavailability` | ~6282 | Fabrication-guard predicate — tool unavailability |
| `fn asserted_unavailability_satisfied` | 6308 | Predicate — has the tool actually been attempted? |
| `ASSERT_GROUNDED_LABEL` const | ~6324 | Guard label for assert-grounded |
| `GROUNDING_TOOLS` const | 6326 | `["run_gh", "check_task", "gh_read"]` — grounding tool allowlist |
| `struct AffirmativeStateClaim` | 6329 | Affirmative state-claim row (resource_type/resource_ref/claim_text) |
| `AFFIRMATIVE_STATE_CLAIM_PATTERNS` static | 6337 | 4 regexes for affirmative state claims |
| `fn assert_grounded_satisfied` | 6455 | Grounding-rule enforcement predicate — has tool call evidence |
| Tests | 11072-11252 | `test_assert_grounded_satisfied_*` (5+ tests) |

There's also likely a `fn extract_affirmative_state_claim` helper used by the `assert_grounded` guard dispatch (not enumerated above; will surface during the move).

### C.3 — What stays OUT

**Per architect F1 (BLOCKING, addressed) — sibling-ticket conflict resolution with #1445 dashboard_queries/:**

- **`list_audit_events_paginated` (db.rs:9495)** — claimed by #1445 dashboard_queries/ GROOMED plan as dashboard-aggregation surface. §6 reading: "Read-side aggregation for dashboard surfaces" covers paginated reads of audit_events. STAYS in #1445 scope, NOT evidence/.
- **`list_audit_events_paginated_with_count` (db.rs:9813)** — same sibling-ownership as 9495. STAYS in #1445 scope.

Verified via `git show origin/refactor/1445/dashboard-queries-module-extraction:docs/plans/2026-06-08-001-refactor-1445-dashboard-queries-extract-plan.md` — #1445's Phase 0 §C explicitly lists both 9495 and 9813 in its 11-method dashboard-aggregation table. F1 finding was correct; this plan's original 12-method count was wrong. Revised to **10 methods**.

`count_audit_events` (9518) IS kept in evidence/ scope per this revision — #1445's plan does NOT claim it (the architect's F1 only flagged the two `list_*paginated*` methods, not the count helper). The count itself is an audit-event domain measurement, called by both evidence-domain code and (potentially) dashboard paginators via `Database::*`.

**Other excluded surfaces:**

- **`post_condition.rs` (51 lines)**: Generic `PostConditionGuard` struct + `GuardDecision` enum — a registry pattern used by ALL guards, not just evidence-domain ones (some guards are agent-loop-specific: max-steps, intent-precondition, required-suffix-line). Stays in `src/` root; future grooming may relocate it to `agent_loop/` (#1452) or keep it shared. Architect-confirmed: the registry's purpose is loop-level infrastructure; moving it to evidence/ would invert the dependency (agent_loop/ → evidence/ just for registry semantics).
- **Guard-dispatch site (where the predicate is called + reject decision happens) in agent.rs**: stays in agent.rs as part of `run_agent()` flow — owned by agent_loop/ (#1452). Evidence/ owns the *predicates*; agent_loop/ owns the *enforcement timing*.
- **`rewind.rs` AuditEvent consumers**: `rewind::audit_events_by_trace_ids` callers stay in rewind.rs. The rewind module is a future-grooming target (not part of §6 partition). After this extraction, rewind.rs imports `crate::evidence::AuditEvent` and `crate::evidence::audit::*`.

### C.3.1 — F2 sharpening (addressed): private-access enumeration for moved tests

Architect F2 flagged that moved tests may depend on private helpers requiring visibility changes. Body-read enumeration:

**evidence/guards.rs tests (assert_grounded_satisfied family, ~5 tests)** depend on:
- `ToolCallSummary` — verified at agent.rs:289 as `pub struct ToolCallSummary` (already crate-public). After move, evidence/guards.rs imports via `use crate::agent::ToolCallSummary;`. **No visibility change needed.**
- `AffirmativeStateClaim` — moves with the tests to evidence/guards.rs. **No visibility change needed.**
- `assert_grounded_satisfied`, helpers — move with the tests. **No visibility change needed.**

**evidence/audit.rs tests (audit_events family, ~5 tests)** depend on:
- `db.conn` direct INSERT access (test_get_audit_events_since uses `db.conn.execute("INSERT INTO audit_events ...")`) — verified at db.rs:869 as `pub(crate) conn: Connection` (already crate-public). After move, evidence/audit.rs tests still access `db.conn` from within the same crate. **No visibility change needed.**
- `db()` test-helper constructor — defined in db.rs's `#[cfg(test)] mod tests`. After move, evidence/audit.rs tests need to either re-import this helper or define a local equivalent. **Option chosen:** import via `use super::super::db::tests::*;` (or `pub(crate)` promotion if Rust visibility blocks the inner import-path). Verify at implementation time; document outcome in PR body.

**Risk assessment**: Both source-side helpers (`ToolCallSummary`, `db.conn`) are already crate-public via `pub` and `pub(crate)`. No visibility-surface changes required by this extraction. The only non-trivial test-relocation concern is the `db()` constructor's accessibility from a sibling test module — a small Rust-mechanics question that's resolved at implementation time, not a structural blocker. Documented as PR-body risk; no AC required.

### C.4 — Cross-module dependency direction

| Consumer | Imports | Direction |
|---|---|---|
| agent.rs (#1452 agent_loop/) | `crate::evidence::*` (guard predicates) | agent_loop/ → evidence/ ✓ |
| rewind.rs | `crate::evidence::AuditEvent`, audit_events_by_trace_ids | rewind/ → evidence/ ✓ |
| server/handlers/dashboard | `crate::evidence::*` (paginated lists) | server/ → evidence/ ✓ |
| db/operational.rs | (no evidence-domain deps) | independent ✓ |

One-way fan-in to evidence/. No reverse dependencies. Pure leaf with respect to §6.

## Hypothesis (committed)

**Extraction shape**: split into 3 files inside `crates/mika-agent/src/evidence/`:

- `evidence/mod.rs` — module-level doc-comment per Foundation §6 + re-exports of public surface
- `evidence/audit.rs` — `AuditEvent` struct + 10 db methods (revised from 12 per F1) + tests (relocated from `db.rs`)
- `evidence/guards.rs` — fabrication-guard predicates + grounding-rule enforcement helpers + regex statics + tests (relocated from `agent.rs`)

Rationale for 3-file split (deviation from #1447's pure-relocation shape): the source code is currently distributed across two giant files (agent.rs, db.rs). A 1:1 file relocation isn't possible — there's no `evidence.rs` to `git mv`. Splitting by sub-concern (audit vs guards) keeps each file under ~1000 LoC and matches Foundation §6's enumeration ("grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail").

## Approach (committed)

### A. Create module skeleton

```bash
mkdir -p crates/mika-agent/src/evidence
```

Three files:
- `evidence/mod.rs` (doc-comment + re-exports)
- `evidence/audit.rs` (audit_events surface)
- `evidence/guards.rs` (fabrication + grounding predicates)

### B. Extract audit_events from db.rs

1. Cut the `AuditEvent` struct (db.rs:371-385) → `evidence/audit.rs`.
2. Cut all 12 audit_events methods (db.rs:7786-9813 segments) → `evidence/audit.rs` as `impl Database { ... }` block — keeping them as Database methods preserves the AsyncDatabase wrapper contract.
3. Cut the 5+ audit_events tests (db.rs:10982-11348 segments) → `evidence/audit.rs` `#[cfg(test)] mod tests`.
4. db.rs is left lighter by ~600 LoC (struct + methods + tests).

### C. Extract fabrication-guard predicates from agent.rs

1. Cut the 4 regex statics (`GITHUB_RESOURCE_URL_RE`, `ACTION_CLAIM_RE`, `CALLBACK_STATE_CLAIM_RE`, `ASSERTED_UNAVAILABILITY_PATTERNS`, `AFFIRMATIVE_STATE_CLAIM_PATTERNS`) → `evidence/guards.rs`.
2. Cut the predicate functions (`detect_fabricated_action_claim`, `detect_fabricated_callback_state_claim`, `detect_asserted_unavailability`, `asserted_unavailability_satisfied`, `assert_grounded_satisfied`, `extract_affirmative_state_claim` if present) → `evidence/guards.rs`.
3. Cut the type/constant surface (`AffirmativeStateClaim` struct, `GROUNDING_TOOLS`, `ASSERT_GROUNDED_LABEL`) → `evidence/guards.rs`.
4. Cut the predicate tests (5+ `test_assert_grounded_satisfied_*` tests) → `evidence/guards.rs` `#[cfg(test)] mod tests`.
5. Update agent.rs call sites to `crate::evidence::guards::*` (with `pub use` from `evidence/mod.rs` so import path is shorter).
6. agent.rs is left lighter by ~400-500 LoC.

### D. mod.rs re-exports + doc

```rust
//! grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail.
//!
//! Per Foundation §6: this module owns the predicates that the agent loop's
//! guard-dispatch logic consults (assert_grounded, asserted_unavailability,
//! fabrication detection), plus the audit_events ledger that persists
//! tool-call provenance for the rewind path and dashboard surfaces.
//!
//! Guard *enforcement timing* (the reject-and-reprompt machinery at EndTurn)
//! lives in `crate::agent` (post-#1452 agent_loop/). This module exposes the
//! pure predicates that drive that machinery.

pub mod audit;
pub mod guards;

pub use audit::AuditEvent;
pub use guards::{AffirmativeStateClaim, GROUNDING_TOOLS, ASSERT_GROUNDED_LABEL};
```

### E. lib.rs

```rust
pub mod evidence;
```

### F. Update call sites

- `agent.rs`: ~10 references to predicates — re-import via `use crate::evidence::guards::{detect_fabricated_action_claim, asserted_unavailability_satisfied, assert_grounded_satisfied, ...};`
- `rewind.rs`: `use crate::db::AuditEvent;` → `use crate::evidence::AuditEvent;`
- `server/handlers/`: dashboard call sites for `list_audit_events_paginated` — verify import-path updates.
- Cross-crate: `crates/mika-gateway/` — grep confirms no audit-event or evidence-guard imports.

### G. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes — particular focus on the moved tests (assert_grounded_*, audit_events)
- `git log --follow crates/mika-agent/src/evidence/audit.rs` shows db.rs history fragments (via `--follow` flag's rename detection — works for cut-and-paste segments with sufficient similarity)
- `grep -rn "log_audit_event\|get_audit_events\|AuditEvent\b" crates/mika-agent/src/db.rs` returns zero hits (verifies clean removal)
- `grep -rn "assert_grounded_satisfied\|detect_fabricated_action_claim\|asserted_unavailability_satisfied" crates/mika-agent/src/agent.rs` returns only call-site references, not definitions

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/evidence/mod.rs` created with doc-comment per Foundation §6 (parent AC4). Re-exports `AuditEvent`, `AffirmativeStateClaim`, `GROUNDING_TOOLS`, `ASSERT_GROUNDED_LABEL` for shorter import paths.

2. **AC2**: `crates/mika-agent/src/evidence/audit.rs` contains the `AuditEvent` struct + 10 audit_events `Database` methods (revised from 12 per F1) + their tests, fully relocated from db.rs.

3. **AC3**: `crates/mika-agent/src/evidence/guards.rs` contains the 5 regex statics + 5-7 predicate functions + `AffirmativeStateClaim` + constants + tests, fully relocated from agent.rs.

4. **AC4**: db.rs and agent.rs each have NO definitions of moved symbols. `grep -rn "log_audit_event\|fn detect_fabricated_action_claim\|fn assert_grounded_satisfied" crates/mika-agent/src/{db,agent}.rs` returns ZERO definition hits (only call-site references in agent.rs).

5. **AC5**: All call sites updated. `grep -rn "use crate::db::AuditEvent\|use crate::db::log_audit_event\|use crate::agent::assert_grounded_satisfied" crates/ tests/` returns ZERO hits — old import paths fully purged across all file types (Rust source, doc-comments, non-Rust).

6. **AC6**: `crates/mika-agent/src/lib.rs` declares `pub mod evidence;` (parent AC4).

7. **AC7**: `cargo test -p mika-agent` passes (parent AC2). Particular checkpoint: the moved tests (`test_assert_grounded_satisfied_*`, `test_log_and_get_audit_events`, `test_get_audit_events_since`, `test_compact_old_audit_events`) pass in their new home.

8. **AC8**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

9. **AC9**: No behavior change (parent AC3) — pure code relocation, same predicate logic, same audit-event semantics.

10. **AC10**: Sub-file history preservation — `git log --follow crates/mika-agent/src/evidence/{audit,guards}.rs` shows the prior history fragments via git's rename-detection heuristic. Acceptable degradation: a chunk-cut from a 17k-line file may NOT be detected as a rename if the new file is small; document this in PR body as known limitation. (Unlike #1447 where `git mv` was clean, this extraction is chunk-cut + paste, so blame preservation is best-effort, not guaranteed.)

## Files to change

- **Create**: `crates/mika-agent/src/evidence/mod.rs`, `crates/mika-agent/src/evidence/audit.rs`, `crates/mika-agent/src/evidence/guards.rs`
- **Edit** (cut content + leave call-site imports): `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/agent.rs`
- **Edit** (lib.rs module declaration): `crates/mika-agent/src/lib.rs`
- **Edit** (import-path updates): `crates/mika-agent/src/rewind.rs`, possibly `crates/mika-agent/src/server/handlers/dashboard.rs` and other dashboard handlers — confirmed by grep at implementation time

## Out of scope

- `post_condition.rs` — generic guard registry, stays in src/ root. Future grooming may move to agent_loop/ (#1452) or keep shared.
- Guard *dispatch* (the reject-and-reprompt machinery at EndTurn in agent.rs's `run_agent()`) — stays in agent.rs, owned by agent_loop/ (#1452).
- Refactoring the predicate functions themselves (combining regexes, deduplicating patterns, etc.) — pure relocation only per parent AC3.
- `rewind.rs` ownership — rewind/ is not part of §6 partition; this extraction only updates rewind.rs's evidence imports.

## Risk

Medium. This is the LARGEST Wave 2 firing.

- **Chunk-cut from giant files**: db.rs (17.6k LoC) and agent.rs (11.4k LoC) are central — accidental over-cut or under-cut leaves the build broken. Mitigated by `cargo build` after each file's extraction (B → C → F sequence, not all-at-once).
- **Cross-call dependencies**: agent.rs's guard-dispatch logic references the predicates by name in inline `match` arms. Every reference must be updated to import-path. Mitigated by AC5's grep verification.
- **Test relocation**: tests use `super::*;` to access private functions; moved tests need `use super::*;` to work in the new location (or `use crate::evidence::guards::*;` if testing pub surface). Mitigated by running the specific moved tests post-move.
- **History-preservation degradation**: chunk-cuts may not be detected as renames by git. Acceptable per AC10.
- **Cross-crate impact**: `crates/mika-gateway/` does not import any of these symbols (verified via grep at plan-time). Low risk.

Larger risk surface than #1445/#1446/#1447/#1449, but bounded by the AC5 grep gate.

## Test plan

1. `cargo build -p mika-agent` clean — after each sub-step (B, C, F) to catch import errors early
2. `cargo test -p mika-agent --lib` passes
3. `cargo build -p mika-gateway` clean (cross-crate sanity — no expected impact, verify)
4. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
5. `grep -rn "use crate::db::AuditEvent\|use crate::db::log_audit_event\|use crate::agent::assert_grounded_satisfied\|use crate::agent::detect_fabricated_action_claim" crates/ tests/` returns **zero hits across all file types** (Rust source, doc-comments, non-Rust)
6. `grep -rn "log_audit_event\|fn detect_fabricated_action_claim\|fn assert_grounded_satisfied" crates/mika-agent/src/{db,agent}.rs` returns ZERO definition hits
7. Specifically run the moved tests: `cargo test -p mika-agent --lib evidence::audit::tests evidence::guards::tests`

## Implementation order

1. Create module skeleton (mkdir + 3 empty file stubs with doc-comments only)
2. lib.rs: `pub mod evidence;`
3. `cargo build` — should still pass (empty module)
4. Move audit_events from db.rs → evidence/audit.rs (B sub-step) + tests + cross-file import updates
5. `cargo build` — fix any remaining db.rs internal call-sites (post_condition or other paths)
6. `cargo test -p mika-agent --lib evidence::audit` — verify moved tests pass
7. Move predicate functions from agent.rs → evidence/guards.rs (C sub-step) + tests
8. `cargo build` — fix all agent.rs call-site imports
9. `cargo test -p mika-agent --lib evidence::guards` — verify moved tests pass
10. Update rewind.rs and dashboard handler imports (F sub-step)
11. `cargo build -p mika-agent && cargo test -p mika-agent --lib && cargo clippy -p mika-agent --tests --no-deps -- -D warnings`
12. AC4 + AC5 grep verifications
