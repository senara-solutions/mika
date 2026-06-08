# Plan — refactor(mika#1259): extract planning/ module (mika#1451)

## Phase 0 — Pin

**A. Foundation §6 planning/ definition:**
> `planning/` — plan-doc invariants, dispatch-readiness predicates, agent-loop policy.

**B. Sibling-accretion check across prior Wave 2 firings:**

Per parent #1259 decomposition plan F2: *"if mika#1363 (auto-pull groomed-not-ready) ships first, `is_groomed(body)` predicate lands in `auto_pull.rs`. The planning/ extraction will absorb it."*

mika#1363 status at grooming-time: **OPEN** (not shipped). No is_groomed predicate exists yet in the codebase (grep verified: zero hits for `fn is_groomed`). The future-accretion path is documented but not active.

Per #1450 tool_execution/ GROOMED plan: `validate_dispatch_readiness` and `check_grooming_markers` already extract to `tool_execution/dispatch_gates.rs`, NOT planning/. The §6 "dispatch-readiness predicates" sub-concern that planning/ claims is partially covered by #1450's gate extraction.

**C. Surfaces body-read against current `crates/mika-agent/`:**

### C.1 — Agent-loop policy constants (the substantive content)

`crates/mika-agent/src/agent.rs:35-58` — five policy constants:

| Line | Symbol | Description |
|---|---|---|
| 35 | `const MAX_TOOL_STEPS: usize = 20` | Per-turn max tool steps for conversation mode |
| 36 | `const MAX_CALLBACK_TOOL_STEPS: usize = 20` | Per-turn max for callback/reminder triggers |
| 37 | `const MAX_TEAM_TOOL_STEPS: usize = 20` | Per-turn max for team agent mode |
| 58 | `const MAX_IMAGE_BYTES_PER_STEP: usize = 20 * 1024 * 1024` | Per-step image-byte budget (~20 MB) |
| 321 | `const MAX_REFLECTION_DIGEST_CHARS: usize = 50_000` | Per-reflection-digest char cap |

**Total: 5 constants ≈ 5 LoC of substance** (with comments/context, ~15 LoC of surface).

### C.2 — `max_steps()` impl methods (consumers of policy)

`agent.rs:270-275` — `fn max_steps(&self) -> usize` impl on RunMode enum (the conversation/silent/team-mode discriminator).

`agent.rs:3610-3618` — `fn max_steps(&self) -> usize` impl on SilentTrigger enum (the callback/reminder/heartbeat discriminator).

**Both impl methods stay in agent.rs (agent_loop/ #1452 domain).** Rust convention: enum impls live near the enum, NOT in a separate "policy" file. The methods consume the policy constants; they're not the policy themselves.

### C.3 — Plan-doc invariants surface (CURRENTLY EMPTY)

Grep verified: zero `fn validate_plan`, zero `check_plan_doc`, zero `plan_path_callout`, zero `plan_doc_exists` defined in `crates/mika-agent/src/`. The grooming-marker scan (`check_grooming_markers` at skills/executor.rs:803) is the closest existing surface, but that's `tool_execution/dispatch_gates.rs` per #1450.

**Sub-concern is empty at extraction-time.** Documented as a future-accretion target: when plan-doc validation logic is added (currently unreified), it lands in planning/.

### C.4 — Dispatch-readiness predicates surface (DELEGATED TO #1450)

`validate_dispatch_readiness` (skills/executor.rs:843) and `check_grooming_markers` (skills/executor.rs:803) are extracted to `tool_execution/dispatch_gates.rs` in #1450's GROOMED plan. Per #1450 pass-2 architect verdict, this resolves the cross-ticket boundary:

> "**§6 boundaries are: planning/ owns 'should this dispatch happen?' (policy), tool_execution/ owns 'is the tool ready to execute?' (gate). No overlap.**" — #1450 pass-2 architect.

The current predicate (`validate_dispatch_readiness`) sits on the gate-side per #1450. Planning/'s "dispatch-readiness predicates" sub-concern is empty at extraction-time — future predicates that answer "should this dispatch happen?" (policy-level, e.g., is_groomed from #1363) land in planning/.

### C.5 — Future-accretion targets (documented, not in #1451 scope)

When these sub-issues ship, their relevant code lands in planning/:
- **mika#1363 `is_groomed(body)` predicate** — pre-shipping target for planning/ (per decomposition F2)
- **Future plan-doc validation** — currently unreified; if reified later, lands here
- **Future agent-loop policy escalation** (e.g., adaptive max_steps based on context) — unreified

### C.6 — What stays OUT

- **`tool_execution/dispatch_gates.rs`** — already claimed by #1450 (validate_dispatch_readiness + check_grooming_markers)
- **`RunMode::max_steps()` + `SilentTrigger::max_steps()` impls** (agent.rs:270, 3610) — stay in agent.rs (agent_loop/ #1452 domain; impl methods live with their enums)
- **The two enums themselves (RunMode, SilentTrigger)** — agent_loop/ #1452
- **Skills loader matcher logic** (skills/matcher.rs) — skills/ infrastructure, not planning/

### C.7 — Cross-module dependency direction

| Consumer | Imports from planning/ | Direction |
|---|---|---|
| agent.rs (#1452 agent_loop/) | `crate::planning::policy::MAX_TOOL_STEPS`, `MAX_CALLBACK_TOOL_STEPS`, `MAX_TEAM_TOOL_STEPS`, `MAX_IMAGE_BYTES_PER_STEP`, `MAX_REFLECTION_DIGEST_CHARS` | agent_loop/ → planning/ ✓ |
| tool_execution/ (#1450) | No planning/ imports (per #1450 dispatch_gates extraction) | independent |
| All other §6 modules | No planning/ imports | independent |

One-way fan-in from agent_loop/ only. Pure leaf with respect to §6.

## Hypothesis (committed)

**SMALLEST Wave 2 firing by relocated LoC** (≈ 5 const lines + ~10 LoC of context = ~15 LoC of substance, plus future-accretion documentation).

**Extraction shape**: 2-file split:

```
crates/mika-agent/src/
├── planning/
│   ├── mod.rs          # §6 doc-comment + module-purpose + future-accretion targets
│   └── policy.rs       # 5 agent-loop policy constants relocated from agent.rs
```

This is a near-placeholder module. It satisfies parent #1259 AC4 ("each new module has its own mod.rs with a one-paragraph doc-comment naming the operational responsibility") and AC2 ("logic moved" — the 5 policy constants relocate from agent.rs). The other §6 sub-concerns are empty at extraction-time and documented as future-accretion targets.

**Rationale for proceeding despite empty sub-concerns**: parent #1259's goal is the 9-way partition shape; this sub-issue's job is to mark planning/ as a named module so future code lands there by convention. Holding back #1451 until #1363 ships would delay Wave 2 drain. Better: extract the placeholder + the existing policy constants now, let sub-concerns accrete as siblings ship.

## Approach (committed)

### A. Create module skeleton

```bash
mkdir -p crates/mika-agent/src/planning
```

### B. Move policy constants from agent.rs → planning/policy.rs

Cut lines 35-37 + 58 + 321 from agent.rs to `planning/policy.rs`. Estimated 5 lines of code + ~30 lines of context comments.

```rust
// planning/policy.rs

/// Per-turn max tool steps for conversation mode.
pub const MAX_TOOL_STEPS: usize = 20;

/// Per-turn max tool steps for callback/reminder triggers.
pub const MAX_CALLBACK_TOOL_STEPS: usize = 20;

/// Per-turn max tool steps for team agent mode.
pub const MAX_TEAM_TOOL_STEPS: usize = 20;

/// Per-step image-byte budget (~20 MB).
pub const MAX_IMAGE_BYTES_PER_STEP: usize = 20 * 1024 * 1024;

/// Per-reflection-digest char cap.
pub const MAX_REFLECTION_DIGEST_CHARS: usize = 50_000;
```

### C. mod.rs with doc-comment + re-exports

```rust
//! plan-doc invariants, dispatch-readiness predicates, agent-loop policy.
//!
//! Per Foundation §6, this module owns three sub-concerns. At extraction time
//! (2026-06-08), only **agent-loop policy** has materialized code; the other
//! two sub-concerns are documented future-accretion targets.
//!
//! - **agent-loop policy** — 5 constants in `policy.rs` bounding tool-step
//!   budgets per agent mode (conversation/callback/team) plus image-byte and
//!   reflection-digest caps. Consumed by `crate::agent::RunMode::max_steps`
//!   and `crate::agent::SilentTrigger::max_steps` (which stay in `agent.rs`
//!   per agent_loop/ #1452 — impls live near their enums).
//! - **dispatch-readiness predicates** — currently empty. The existing
//!   `validate_dispatch_readiness` and `check_grooming_markers` are dispatch
//!   *gates* (tool_execution/dispatch_gates.rs per #1450), not planning-level
//!   policy predicates. Future is_groomed/should_dispatch predicates that
//!   answer "should this dispatch happen?" (policy) — not "is the tool ready
//!   to execute?" (gate) — land here. Pre-shipping target: mika#1363's
//!   is_groomed(body) predicate.
//! - **plan-doc invariants** — currently empty. When plan-doc validation
//!   logic is reified (currently unreified), it lands here.

pub mod policy;

pub use policy::{
    MAX_CALLBACK_TOOL_STEPS, MAX_IMAGE_BYTES_PER_STEP, MAX_REFLECTION_DIGEST_CHARS, MAX_TEAM_TOOL_STEPS, MAX_TOOL_STEPS,
};
```

### D. Update lib.rs

```rust
pub mod planning;
```

### E. Update agent.rs call sites

`MAX_TOOL_STEPS`, `MAX_CALLBACK_TOOL_STEPS`, `MAX_TEAM_TOOL_STEPS`, `MAX_IMAGE_BYTES_PER_STEP`, `MAX_REFLECTION_DIGEST_CHARS` references in agent.rs become `crate::planning::policy::*` (or via the re-exports: `crate::planning::*`). Grep all usages and update.

### F. Verify

- `cargo build -p mika-agent` clean
- `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
- `cargo test -p mika-agent --lib` passes
- `grep -rn "^const MAX_TOOL_STEPS\|^const MAX_CALLBACK_TOOL_STEPS\|^const MAX_TEAM_TOOL_STEPS\|^const MAX_IMAGE_BYTES_PER_STEP\|^const MAX_REFLECTION_DIGEST_CHARS" crates/mika-agent/src/agent.rs` returns ZERO hits (verifies clean removal)
- `grep -rn "use crate::planning::\|crate::planning::policy::" crates/mika-agent/src/` returns at least 5+ hits (verifies imports updated)

## Acceptance Criteria

1. **AC1**: `crates/mika-agent/src/planning/mod.rs` created with Foundation §6 doc-comment naming all 3 sub-concerns (agent-loop policy, dispatch-readiness predicates, plan-doc invariants) + their disposition (1 materialized, 2 future-accretion). Per parent AC4.

2. **AC2**: `crates/mika-agent/src/planning/policy.rs` contains the 5 agent-loop policy constants relocated from agent.rs (MAX_TOOL_STEPS, MAX_CALLBACK_TOOL_STEPS, MAX_TEAM_TOOL_STEPS, MAX_IMAGE_BYTES_PER_STEP, MAX_REFLECTION_DIGEST_CHARS).

3. **AC3**: agent.rs has NO definitions of the 5 relocated constants. `grep -nE "^const (MAX_TOOL_STEPS|MAX_CALLBACK_TOOL_STEPS|MAX_TEAM_TOOL_STEPS|MAX_IMAGE_BYTES_PER_STEP|MAX_REFLECTION_DIGEST_CHARS)" crates/mika-agent/src/agent.rs` returns ZERO hits.

4. **AC4**: All call sites updated. References to the 5 constants in agent.rs use `crate::planning::policy::*` (or the re-export). `grep -rn "MAX_TOOL_STEPS\|MAX_CALLBACK_TOOL_STEPS\|MAX_TEAM_TOOL_STEPS\|MAX_IMAGE_BYTES_PER_STEP\|MAX_REFLECTION_DIGEST_CHARS" crates/ tests/` returns ONLY qualified-path references (`crate::planning::policy::*` or the re-export), zero bare references in agent.rs that don't resolve through the re-export.

5. **AC5**: `crates/mika-agent/src/lib.rs` declares `pub mod planning;` (parent AC4).

6. **AC6**: `cargo test -p mika-agent` passes (parent AC2).

7. **AC7**: `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean.

8. **AC8**: No behavior change (parent AC3) — pure constant relocation, same numeric values, same semantics.

9. **AC9**: Future-accretion documentation present in mod.rs doc-comment naming mika#1363 (is_groomed pre-shipping target).

## Out of scope

- `RunMode::max_steps()` impl at agent.rs:270 — stays with RunMode enum (agent_loop/ #1452)
- `SilentTrigger::max_steps()` impl at agent.rs:3610 — stays with SilentTrigger enum (agent_loop/ #1452)
- `validate_dispatch_readiness` + `check_grooming_markers` — already claimed by #1450 tool_execution/
- mika#1363 is_groomed predicate — doesn't exist yet (mika#1363 is OPEN at extraction-time); future-accretion target
- Plan-doc validation logic — not reified yet; future-accretion target

## Risk

**LOWEST Wave 2 firing.** Relocating 5 constants is a low-risk extraction.

- **Empty sub-concerns**: 2 of 3 §6 sub-concerns are empty at extraction-time. Risk: architect may rule this as "premature module" and ESCALATE. Mitigation: planning/ is named in §6 as a durable boundary; #1363's pre-shipping target + future plan-doc validation justify the placeholder. Honest closure: "decomposed, not yet fully populated."
- **Small call-site sweep**: 5 constants × multiple usages = ~10-20 reference updates in agent.rs. Bounded by `cargo build`.

## Test plan

1. `cargo build -p mika-agent` clean
2. `cargo test -p mika-agent --lib` passes
3. `cargo clippy -p mika-agent --tests --no-deps -- -D warnings` clean
4. `grep -nE "^const (MAX_TOOL_STEPS|MAX_CALLBACK_TOOL_STEPS|MAX_TEAM_TOOL_STEPS|MAX_IMAGE_BYTES_PER_STEP|MAX_REFLECTION_DIGEST_CHARS)" crates/mika-agent/src/agent.rs` returns ZERO hits (AC3)

## Implementation order

1. `mkdir -p crates/mika-agent/src/planning`
2. Create `planning/mod.rs` with doc-comment + `pub mod policy;` + re-exports
3. Create `planning/policy.rs` with the 5 const definitions copied verbatim from agent.rs
4. `cargo build` — should succeed (planning/ exists, agent.rs still has the consts)
5. Delete the 5 const definitions from agent.rs
6. Update agent.rs imports/references to use `crate::planning::policy::*` (or re-export)
7. `cargo build` — should succeed
8. lib.rs: add `pub mod planning;`
9. `cargo test -p mika-agent --lib` passes
10. AC3/AC4 grep verifications
