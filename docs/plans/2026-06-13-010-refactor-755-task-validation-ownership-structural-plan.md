---
ticket: mika#755
branch: refactor/755/task-validation-ownership-structural
status: active
date: 2026-06-13
origin: https://github.com/senara-solutions/mika/issues/755
execution: code
---

# Plan: type-level guard for task-validation ownership (mika#755)

## Problem frame

mika#752 fixed a bug where `validate_task_exists` was named for existence semantics but silently enforced agent-ownership semantics. The fix landed as bidirectional doc comments cross-linking `validate_task_exists` and `get_task_unscoped`. Doc comments are prompt-level enforcement — a future developer can ignore them and pick the wrong function for the wrong context.

This ticket implements Option A from the body: a type-level newtype that makes the wrong call a compile error.

## Resolution of first-pass findings

**F1 (BLOCKING) — Tripwire condition resolved: ship now with Option A.**

The architect's analysis (and the body's own conclusion) supports ship-now over wait-for-recurrence:
- `feedback_prompt_enforcement_fragile.md`: "Engine-level structural guards beat prompt-level 'do not.'"
- The body's "Why NOT ship now" reasons are stale (#752 closed, milestone #16 unblocked) or refuted by the institutional principle.
- The fix is bounded (~30-50 lines, single crate, behavior unchanged).
- Waiting for n=2 incurs cost (tracking obligation, future regression on a subtle bug class) for a fix that's already known.

**F2 (sharpening) — Tightened ACs.**

Original body had conditional ACs ("Pick A or B based on call-site churn"). The plan commits to Option A and lists structural-guarantee-specific ACs:

| AC | Guarantee |
|----|-----------|
| AC1 | `AgentScopedTaskId(String)` newtype defined; constructor is private (or `pub(crate)` scoped to the agent-tool dispatch context). |
| AC2 | `validate_task_exists` signature takes `&AgentScopedTaskId` (not `&str`). |
| AC3 | CLI / non-tool correlation paths call `db.get_task_unscoped` directly with `&str` — no newtype required (correct: they don't have agent context). |
| AC4 | #752's doc comments updated to point at the type-level guard (`see AgentScopedTaskId for the structural enforcement`). |
| AC5 | Static analysis check: any new caller of `validate_task_exists` must obtain an `AgentScopedTaskId` through the canonical constructor. Verified via grep + manual review at PR time; future regressions caught by compile failure. |

## Scope boundaries

- Add `AgentScopedTaskId` newtype in `crates/mika-agent/src/tools/mod.rs` (alongside the existing UUID validation helpers).
- Constructor: `pub(crate) fn from_tool_context(ctx: &ToolContext, raw: &str) -> Result<Self, ToolOutput>` — only callable from inside the tools crate, only constructible when a `ToolContext` (which encodes agent scope) is in scope.
- Change `validate_task_exists` signature: `&AgentScopedTaskId` instead of `&str`.
- Update all `validate_task_exists` callers — they're all inside `crates/mika-agent/src/tools/` per `grep -rn validate_task_exists crates/`.
- Update #752's doc comments on both sides.
- **Out of scope:** changing `db.get_task_unscoped` (it correctly takes `&str` — its callers are non-tool correlation paths); refactoring the broader task-validation chain (layer 1, 3); the `validate_task` and `validate_uuid` helpers; per-table query helpers in `db.rs`.

## Implementation Units

### U1 — `AgentScopedTaskId` newtype

**Goal:** A newtype that can only be constructed when a `ToolContext` exists.

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs` (alongside existing helpers around line 348)

**Approach:**

```rust
/// A task UUID that has been validated as format-correct AND obtained inside an
/// agent-tool context. The `validate_task_exists` function takes only this type,
/// so non-agent-scoped paths (CLI correlation, raw introspection) physically
/// cannot pass it raw `&str`s — they must use `db.get_task_unscoped` instead.
///
/// Constructor is `pub(crate)` and requires a `ToolContext`, encoding the
/// invariant: this type only exists in agent-tool execution paths.
///
/// See mika#755 for the structural rationale; mika#752 for the original bug.
#[derive(Debug, Clone)]
pub struct AgentScopedTaskId(String);

impl AgentScopedTaskId {
    /// Constructor — only callable inside the tools crate, only when a
    /// `ToolContext` is present (i.e., from a tool's `execute` body).
    pub(crate) fn from_tool_context(
        _ctx: &ToolContext<'_>,
        raw: &str,
    ) -> Result<Self, ToolOutput> {
        validate_uuid("task_id", raw)?;  // format check
        Ok(Self(raw.to_string()))
    }

    /// Read-only access to the underlying UUID string for query construction.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
```

The `_ctx` parameter is unused at runtime but is the structural anchor — you cannot call this function without a `ToolContext` in scope, which means you cannot get an `AgentScopedTaskId` from a non-tool context.

**Test scenarios:**
- **Compile-time check:** code that tries to construct `AgentScopedTaskId` without a `ToolContext` fails to compile (verify via expected-compile-error doctest).
- **Constructor format validation:** invalid UUID → `Err(ToolOutput)`.
- **Constructor passes a `ToolContext`:** valid UUID → `Ok(AgentScopedTaskId)`.

**Verification:** `cargo test -p mika-agent tools::tests` passes.

### U2 — `validate_task_exists` signature change

**Goal:** Function takes `&AgentScopedTaskId` instead of `&str`.

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs` (`validate_task_exists` around line 348)

**Approach:**

Current signature:
```rust
pub(crate) async fn validate_task_exists(
    db: &AsyncDatabase,
    field_name: &str,
    value: &str,  // <-- raw string, no structural guard
) -> Result<Task, ToolOutput>
```

New signature:
```rust
pub(crate) async fn validate_task_exists(
    db: &AsyncDatabase,
    field_name: &str,
    task_id: &AgentScopedTaskId,  // <-- structural guard
) -> Result<Task, ToolOutput>
```

The function body uses `task_id.as_str()` for SQL parameter binding. Format validation has already happened in the constructor; the function only does existence + agent-scope checks.

**Test scenarios:**
- All existing tests pass after caller updates (U3).
- New test: trying to pass a raw `&str` fails to compile (negative test via `compile_fail` doctest if proportionate).

**Verification:** `cargo build -p mika-agent` clean after U3 lands.

### U3 — Update callers

**Goal:** All callers of `validate_task_exists` construct an `AgentScopedTaskId` first.

**Files:**
- Modify: each caller site found via `grep -rn validate_task_exists crates/mika-agent/src/tools/`

**Approach:** At each call site, replace:

```rust
let task = validate_task_exists(db, "task_id", &input_task_id).await?;
```

with:

```rust
let scoped = AgentScopedTaskId::from_tool_context(ctx, &input_task_id)?;
let task = validate_task_exists(db, "task_id", &scoped).await?;
```

The `ctx` parameter is the existing `ToolContext` in scope at every call site (tools' `execute` bodies all have `ctx: &ToolContext`).

`validate_task` (which wraps `validate_task_exists`) gets the same update — it now takes `&AgentScopedTaskId` too.

**Test scenarios:** existing tests pass after the type update.

**Verification:** `cargo build -p mika-agent` clean; `cargo test -p mika-agent` passes.

### U4 — Update #752 doc comments

**Goal:** Cross-link references now point at the type-level guard.

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs` (doc comment on `validate_task_exists`)
- Modify: `crates/mika-agent/src/db.rs` (doc comment on `get_task_unscoped`)

**Approach:** Replace the doc-only enforcement language with structural framing:

```rust
/// Validates that a task exists AND belongs to the calling agent's scope.
/// Takes `&AgentScopedTaskId` to make the ownership invariant compile-checked
/// (mika#755). Compare with `db::get_task_unscoped` for raw-id correlation
/// paths that intentionally cross agent boundaries (e.g., CLI introspection,
/// internal queries).
```

The mirror doc on `get_task_unscoped` notes: "raw `&str` — for correlation; use `AgentScopedTaskId` + `validate_task_exists` for agent-scoped tool paths."

**Verification:** `cargo doc` builds; manual read.

### U5 — Compound doc

**Goal:** Capture the structural-guard pattern.

**Files:**
- Create: `docs/solutions/best-practices/structural-guards-vs-doc-comments-2026-06-13.md`

**Approach:** Short doc citing mika#752 → mika#755 evolution: prompt-level → structural. Pattern: "When doc comments are the only thing preventing a wrong choice, the next subtle bug is one read-skim away. Promote to a newtype." Links to `feedback_prompt_enforcement_fragile.md`.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 → U3 (U3 updates callers that depend on U2's signature change)
- U4 (docs) and U5 (compound doc) ship in same PR; last

## Patterns to follow (cross-cutting)

- `crates/mika-agent/src/tools/mod.rs::validate_uuid` and existing UUID-validation chain — the layer this newtype slots into
- `feedback_prompt_enforcement_fragile.md` — the institutional principle being applied

## Verification (top-level)

- `cargo test -p mika-agent` passes (existing + new tests)
- `cargo clippy --workspace` clean
- `cargo fmt --all -- --check` clean
- Grep verification: `grep -rn validate_task_exists crates/` returns only callers that construct `AgentScopedTaskId` first
- Code review at PR time: any new caller must obtain `AgentScopedTaskId` through the canonical constructor (verified by reviewer; future regressions caught by compile failure)

## Risk / known unknowns

- **Constructor visibility too narrow.** If a non-tools-crate caller legitimately needs `AgentScopedTaskId` (none currently identified, but possible), constructor visibility may need to widen to `pub` with documentation. Defer until a real case arises.
- **`ToolContext` parameter as unused arg.** Compiler may warn about `_ctx` being unused. Use `_` prefix to suppress, document the structural-anchor rationale inline.
- **Refactor PR review burden.** ~30-50 lines across N call sites. Single-purpose PR; bounded scope; behavior unchanged. Minimal review burden if commits are well-structured (U1 + U2 + U3 + U4 as four small commits in the same PR).

## Out-of-scope (explicit)

- Refactoring `db.get_task_unscoped` (correctly takes `&str` — its callers are non-tool paths).
- Refactoring the broader UUID-validation chain (layer 1 `validate_uuid`, layer 3 `validate_task`).
- Renaming `validate_task_exists` (Option B from the body — superseded by Option A).
- Adding a runtime guard inside `db.get_task_unscoped` to detect agent-tool misuse — runtime guards are weaker than compile-time guards; structural is the goal.
