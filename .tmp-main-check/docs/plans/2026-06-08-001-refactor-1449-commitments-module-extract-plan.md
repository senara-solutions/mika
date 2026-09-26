# Plan — refactor(mika#1259): extract commitments/ module (mika#1449)

## Phase 0 — Pin

**A. Foundation §6**:
> `commitments/` — Promise tracking, follow-ups, due-date reminders. Owns `kind = Commitment`.

**B. Sibling-accretion from #1445 (Wave 2.1)**:
- 7599: `list_commitments(agent_id, status) -> Vec<Commitment>` — confirmed at this canvass body-read.

**C. Additional commitment-related methods surfaced via #1449 body-read**:

| Line | Function | In scope? |
|---|---|---|
| 7599 | `list_commitments` | YES (accreted from #1445) |
| 7613 | `update_commitment_status` | YES |
| 7627 | `get_commitment_status` | YES |
| 7638 | `get_commitment_details` | YES |
| 8115 | `delete_commitment_by_description` | YES |

**Plus**: `pub struct Commitment` at db.rs:321 — type stays in db.rs (Database-domain type; methods relocate to commitments/, struct stays as Rust-namespace-internal-to-db.rs since it's referenced by other db.rs code). Avoids forcing the struct to move.

**Total #1449 scope**: 5 methods. Estimated LoC: ~200-400 lines. **Smallest sub-issue scope so far** in Wave 2.

**D. Sibling-accretion validation (n=2 evidence point)**: #1445 classified list_commitments as commitments/ (NO from dashboard_queries/ scope). This canvass confirms — list_commitments operates on `commitments` table, returns `Commitment` struct, no cross-§6-module calls. **Sibling-accretion mechanism validates again at n=2.**

**E. Cross-module dependency check**: grep on each method's body confirms no calls to other §6 module methods (no `evidence::*`, `task_state::*`, `memory::*`, etc.). Pure leaf-with-respect-to-§6.

**F. No commitment-related code in agent.rs**: confirmed via grep. The extraction is db.rs-only.

## Hypothesis (committed)

**Extraction shape**: move the 5 commitment-domain methods from `db.rs` into `crates/mika-agent/src/commitments/mod.rs`. Methods stay as `impl Database` to minimize call-site churn. `pub struct Commitment` at db.rs:321 stays in db.rs (Database-internal Rust namespace).

Same pattern as #1445 + #1446. Smallest scope of the three so far — should be cleanest extraction.

## Approach (committed)

### A. Create the module

`crates/mika-agent/src/commitments/mod.rs`:

```rust
//! Promise tracking, follow-ups, due-date reminders.
//!
//! Owns the Database query/write methods for `kind = Commitment` operational
//! items. Provides CRUD on the `commitments` table:
//! - list_commitments (by agent + status)
//! - update_commitment_status / get_commitment_status / get_commitment_details
//! - delete_commitment_by_description
//!
//! Per Foundation §6: owns `kind = Commitment` (promise-tracking domain).
//! The `Commitment` struct stays defined in `db.rs` as a Database-internal
//! type since other db.rs code references it; methods relocate here as
//! impl Database blocks (no call-site churn).

use crate::db::{Database, Commitment};
// ... (move 5 impl Database methods from db.rs into this file)
```

### B. Move impl blocks

Migrate the 5 method definitions from db.rs lines 7599, 7613, 7627, 7638, 8115 into `commitments/mod.rs`. Each method's body unchanged.

### C. Update `lib.rs`

Add `pub mod commitments;` to `crates/mika-agent/src/lib.rs`.

### D. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `wc -l crates/mika-agent/src/db.rs` shows ~200-400 line reduction

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/commitments/mod.rs` created with one-paragraph doc-comment per Foundation §6 (parent AC4).

2. **AC2**: Exactly the 5 commitment methods (Phase 0 §C) relocate to `commitments/mod.rs`. Methods stay `impl Database`. `pub struct Commitment` stays in db.rs.

3. **AC3**: `crates/mika-agent/src/lib.rs` declares `pub mod commitments;` (parent AC4).

4. **AC4**: `cargo test -p mika-agent` passes unchanged (parent AC2).

5. **AC5**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

6. **AC6**: No behavior change (parent AC3).

7. **AC7**: `wc -l crates/mika-agent/src/db.rs` shows reduction by ~200-400 lines.

## Files to change

- `crates/mika-agent/src/commitments/mod.rs` — new file (5 methods + doc-comment)
- `crates/mika-agent/src/db.rs` — remove 5 methods (struct stays)
- `crates/mika-agent/src/lib.rs` — add `pub mod commitments;`

## Out of scope

- `pub struct Commitment` definition relocation (stays in db.rs as Database-internal type)
- Other §6 modules
- New commitment-domain features (pure relocation)

## Risk

Low. Smallest extraction of Wave 2 so far. Same `impl Database` pattern as #1445 + #1446. No new substrate.

## Test plan

1. `cargo build -p mika-agent` clean
2. `cargo test -p mika-agent --lib` passes
3. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
4. `wc -l` on db.rs shows ~200-400 line reduction

## Implementation order

1. Create `commitments/mod.rs` with doc-comment + `use crate::db::{Database, Commitment};`
2. Move the 5 methods (lines 7599, 7613, 7627, 7638, 8115) into the new file
3. Remove from db.rs
4. Add `pub mod commitments;` to lib.rs
5. cargo build + clippy + test

## Sibling-accretion observation (n=2)

Wave 2.1 (#1445) classified `list_commitments` as commitments/. Wave 2.3 (#1449) body-read confirmed. **n=2 of sibling-accretion-as-grounding-input mechanism validated.** The mechanism holds across two consecutive sibling firings; future Wave-2 sub-issue grooming can rely on accreted classifications without re-deriving from scratch.

If a third firing also validates accreted classifications (e.g., #1448 task_state/ verifying list_manual_tasks + list_active_tasks), that's n=3 — mechanism is mature enough to fold into bedrock-grade doctrine if Mika Prime concurs.
