---
ticket: mika#1001
type: feat
title: Pipeline grooming N+1 concurrently with dispatch N
date: 2026-05-06
seq: 009
---

# Plan: pipeline grooming N+1 concurrently with dispatch N (mika#1001)

## Verified state (post-architect-pass-1)

- **Option A confirmed by architect.** Per-class slot split. mika-arch session `c7eeca33-094b-47a6-b80c-181e6216fc2b`. Options B (dedicated mika-groomer) and C (worker pool) explicitly rejected on YAGNI grounds.
- **F1 (Phase 0 Pin source bodies + caller list) addressed.** Pinned at `db.rs:5165-5185` (SQL body of `has_active_callback_tasks_excluding`) and `executor.rs:704` (function signature) + `executor.rs:913, 3217, 3233, 3249` (caller list). Detail in Phase 0 below.
- **F2 (mika#996 task-reuse interaction) addressed.** Plan commits to **option (a)**: flip `dispatch_class` on the existing task as it transitions from grooming to dispatch. Option (b) (sibling task) eliminated at plan time per cross-ticket sequencing rule — option (b) would require revising mika#996's already-GROOMED plan-on-branch. Phase 2.E now states this commitment.
- **NF1 (migration COALESCE in SQL not application-layer) addressed.** Phase 2.A's SQL now uses `COALESCE(dispatch_class, 'implement')` for backward compat with pre-v32 NULL rows. No application-layer NULL coercion.
- **NF2 (forensic audit `task_id` partitioning) addressed.** One-sentence note in Phase 2 documenting that interleaved session messages from concurrent dispatches are partitioned via `task_id` for forensic analysis.
- **NF3 (Phase 3 test coexistence with mika#991 AC#3) addressed.** Phase 3 Test 3 explicitly notes coexistence with mika#991's 3-dispatch chained-advance test — the fixtures overlap but assertions differ (mika#991 = sequential advance; mika#1001 = pipelined concurrency). Both must pass independently.

## Why

mika#996 (auto-groom on dispatch, groomed and ready) ships A+B serial: when an ungroomed `ready`-labelled or milestone-cascade ticket reaches mika-dev, the engine fires `dev-groom` via `run_claude_pilot` first, waits for `Verdict: GROOMED`, then dispatches `dev-pilot`. The grooming + dispatch run sequentially in mika-dev's single session. Per the mika#996 grooming pass, this adds ~15-25 minutes per ungroomed ticket to the cadence.

mika#996's stripped AC#4 (groom N+1 concurrently with dispatch N) lives here. The cadence cost compounds across long milestones — milestone#13's ~7 children at 90-150 min added wall-clock per cycle is the threshold operator feedback flagged on 2026-05-06.

This is a **different problem class** than mika#996. mika#996 was a prompt+skill change: replace rejection with auto-dispatch in `self-dev/system_prompt.md`, lift the operator-only restriction in `dev-groom`, insert a Plan-callout pre-flight in Milestone Workflow Step M4. No engine state model changed. mika#1001 cannot ship as a prompt-only change — the structural concurrency is gated by the **single-session-at-a-time guard** (`global_dispatch_active`), which is load-bearing for agent state coherence.

The fix bar: grooming for ticket N+1 runs concurrently with dispatch of ticket N, with no observable race between the two — session memory writes, message channel, DB writes, callback delivery all preserve their pre-mika#996 invariants.

## Phase 0 — Pin (verified state, source-anchored)

All paths verified against worktree at HEAD `514c24d2` (origin/main post 2026-05-06 cascade merges).

### The single-session guard — the load-bearing constraint

**`mika/crates/mika-agent/src/skills/executor.rs:785-814`** — `validate_dispatch_readiness()` global dispatch guard:

```rust
// Global dispatch guard (#583): reject if ANY other task has an active
// callback child. Enforces single-session-at-a-time across all tasks.
match db.has_active_callback_tasks_excluding(task_id).await {
    Ok(Some((blocking_parent_id, blocking_callback_id))) => {
        return Err(serde_json::json!({
            "error": "global_dispatch_active",
            "task_id": task_id,
            "blocking_task_id": blocking_parent_id,
            "blocking_callback_id": blocking_callback_id,
            "reason": format!(
                "Another task ('{}') already has an active dispatch \
                 (callback task '{}'). Only one long-running dispatch may be \
                 active at a time. Wait for it to complete or cancel it before \
                 dispatching again.",
                blocking_parent_id, blocking_callback_id
            )
        }).to_string());
    }
    Ok(None) => { /* No conflicting dispatch — proceed */ }
    Err(e) => {
        return Err(serde_json::json!({
            "error": "dispatch_check_failed",
            ...
        }).to_string());
    }
}
```

Per `crates/mika-agent/CLAUDE.md` § "Long-running":
> **Dispatch-readiness guard (#525):** ... (3) no other task may have an active callback child — global single-session-at-a-time guard (rejects with `global_dispatch_active`, **scoped to `agent_id`**) (#583)

**Critical scope:** the guard is per-agent (`scoped to agent_id`). Two different agents can have concurrent callbacks, but one agent cannot have two callbacks in flight. mika-dev dispatching `dev-groom` via `run_claude_pilot` creates a callback child under mika-dev's agent_id, occupying mika-dev's single slot until the grooming subprocess completes.

**Why the guard exists (mika#583 rationale):** prevents two `run_claude_pilot` callbacks from racing on the same agent's session memory, message channel, and DB writes. Removing or weakening the guard requires preserving these state-coherence invariants by some other mechanism.

### What the guard's existing call sites actually check (architect F1 — pinned)

**`crates/mika-agent/src/db.rs:5165-5185`** — `has_active_callback_tasks_excluding(excluded_parent_id, agent_id)` SQL body verbatim:

```rust
pub fn has_active_callback_tasks_excluding(
    &self,
    excluded_parent_id: &str,
    agent_id: &str,
) -> Result<Option<(String, String)>> {
    let mut stmt = self.conn.prepare(
        "SELECT parent_task_id, id FROM tasks
         WHERE trigger_type = 'callback'
           AND status IN ('pending', 'in_progress')
           AND parent_task_id IS NOT NULL
           AND parent_task_id != ?1
           AND agent_id = ?2
         LIMIT 1",
    )?;
    ...
}
```

The signature **already takes `agent_id` as a parameter** (corrects an earlier ambiguity in this plan). The query has 5 WHERE clauses; adding a per-class predicate means appending one more (`AND COALESCE(dispatch_class, 'implement') = ?3`) and bumping the parameter list.

**`crates/mika-agent/src/skills/executor.rs:704`** — `validate_dispatch_readiness` signature:

```rust
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
) -> Result<String, String>
```

**Caller list (verified by `grep -rn "validate_dispatch_readiness" crates/mika-agent/src/`):**

- **Production (1 call site):** `executor.rs:913` — `let wi_status = match validate_dispatch_readiness(&ctx.db, task_id, github_token).await { ... }`. This is the dispatch wrapper inside `run_claude_pilot`.
- **Tests (3 call sites):** `executor.rs:3217, 3233, 3249` — each is a test scenario asserting guard behavior.

Adding a `dispatch_class: &str` parameter to `validate_dispatch_readiness` requires updating one production caller (which knows the skill name and can derive the class via the mapping in Phase 2.D) and three test callers (each gets a synthetic class arg matching the test's expected behavior). **Not a 1-line change** — it's a 4-call-site update plus the function body internal call to `has_active_callback_tasks_excluding`.

**Revised line estimate from F1's pin:** ~80-120 lines of Rust + SQL (originally estimated 50-100 — the test-caller updates + COALESCE + caller-side class derivation push the upper bound). Still well below the threshold for "this is a different ticket."

The `task_id` exclusion is so the same parent can re-dispatch on retry without false-positive blocking itself. The guard does NOT differentiate between dispatch classes (grooming vs implementation) today; that's the change Phase 2 makes.

### Existing dispatch classes (currently undifferentiated)

`run_claude_pilot` accepts a `skill` parameter with current values per mika#893: `["dev-pilot", "dev-groom"]`. The `_shared/dispatch-lib.sh` skill→entry mapping derives the entry command. Both skills produce callback tasks with identical lifecycle and identical guard treatment — the engine has no concept that one is "implementation work" and the other is "grooming work."

### Sibling concurrency facts

- **mika-arch is a separate agent** (`agent_id='mika-arch'`). Its `mika ask --agent mika-arch` invocations do NOT consume mika-dev's slot. mika-arch is currently called via `/mika-ask-arch` from inside dev-groom's claude-pilot subprocess.
- **Team-engine concurrency** (per `crates/mika-agent/CLAUDE.md` § "Team task tree") allows parent + child agent runs but uses delegation primitives different from `run_claude_pilot`. Not directly applicable here.
- **mika-arch identity** (per `crates/mika-agent/CLAUDE.md` § "Identity-driven tool denylist (#811)") deliberately denies platform-mutational tools (file writes, task mutations, run_claude_pilot, PR merge). mika-arch as currently scoped CANNOT do dev-groom's full Phase 1-5 work (branch creation, plan write, git push, issue-body edit) — it's a read-only architect.

### Symptom evidence

- mika#996 grooming pass (this session, 2026-05-06): cadence cost calculation of "~15-25 minutes per ungroomed ticket" from architect roundtrip + serial `dev-pilot` start time.
- milestone#13 wall-clock observation: 7 children + 90-150 min added cumulative if all ungroomed.

## Scope

**In scope:**

- **Phase 1 — Design pick.** The architect's grooming pass picks one of three primary shapes (Option A: per-class slot split, Option B: dedicated groom-worker agent, Option C: mika-worker pool). The plan explores all three with concrete shapes; the architect ratifies or pushes back.
- **Phase 2 — Implementation of the chosen shape.** Engine-level Rust changes in `crates/mika-agent/src/skills/executor.rs`, `crates/mika-agent/src/db.rs`, possibly `crates/mika-agent/src/agent.rs` if a new agent type is introduced.
- **Phase 3 — Tests.** Concurrency tests at the eval-harness level: dispatch N + groom N+1 simultaneously, assert both complete without race, assert no observable divergence in session/DB state.
- **Phase 4 — Documentation + out-of-scope follow-ups.**

**Out of scope (explicitly):**

- **Generalizing concurrency to other long-running task types** (e.g., dev-pilot N+1 concurrent with dev-pilot N). Implementation tasks have stronger state-coherence requirements than grooming. If the chosen shape generalizes naturally, that's a benefit; if not, this PR scopes to grooming only.
- **Changing dev-groom's grooming sequence.** The two-pass architect roundtrip is unchanged.
- **Changing mika#988's auto-skip behavior** or **mika#996's auto-groom-on-dispatch behavior**. Both shipped/groomed; this PR composes with them, doesn't modify them.
- **Pre-deciding the design.** The architect's grooming pass owns that decision. Plan presents options; architect picks.

## Phase 1 — Design space exploration (architect picks one)

### Option A — Per-class slot split (lightest touch)

**Shape:** add a `dispatch_class TEXT` column to the `tasks` table. Allowed values: `'implement'` (dev-pilot, deploy_mika), `'groom'` (dev-groom). The single-session guard becomes per-class: `has_active_callback_tasks_excluding(task_id, class)` filters by class. mika-dev can have **one implement + one groom** in flight simultaneously.

**Files touched:**
- `crates/mika-agent/src/db.rs` — schema migration (add `dispatch_class` column, index), update `has_active_callback_tasks_excluding` to take a class arg.
- `crates/mika-agent/src/skills/executor.rs:785-814` — `validate_dispatch_readiness` queries the right slot based on the dispatch's class.
- `crates/mika-agent/src/skills/executor.rs` — set `dispatch_class` on long-running task creation based on the `skill` parameter.
- Schema migration `v31->v32`.

**State-coherence concerns:**
- Two callback subprocesses writing to mika-dev's session simultaneously. Both append messages to the same session_id. Already handled per existing per-message INSERT semantics; no race because each callback subprocess is a separate process with its own `mika ask --task-complete` call.
- Both processes might write `tool_calls` or `llm_calls` rows interleaved. Existing AsyncDatabase mpsc dispatch (per `crates/mika-agent/CLAUDE.md` § "Async DB") serializes writes through a single OS thread. No race.
- Concurrent `update_task_status` on different `task_id`s — independent, no race.
- Concurrent `create_task` on different `reference_url`s — independent.

**Verdict (proposed):** lowest-touch shape. Single column addition + one query update + dispatch readiness call-site update. ~50-100 lines of Rust + SQL. Tests at concurrency level for race exclusion.

### Option B — Dedicated `mika-groomer` worker agent

**Shape:** introduce a new well-known agent `mika-groomer` (or `mika-dev-groomer`) that runs grooming dispatches. mika-dev's `run_claude_pilot(skill="dev-groom")` is intercepted; the engine creates the grooming task on `mika-groomer`'s task tree instead of mika-dev's. Callback delivers back to mika-dev's session, but the in-flight subprocess consumes mika-groomer's slot, not mika-dev's.

**Files touched:**
- `crates/mika-agent/src/dev_mode/` — new well-known agent provisioning (mirror of mika-arch's pattern at `mika-agent/CLAUDE.md` § "Fail-closed identity for well-known agents (#811)").
- `crates/mika-agent/src/skills/executor.rs` — `run_claude_pilot` redirects grooming dispatches to mika-groomer's agent_id.
- `crates/mika-agent/src/skills/_shared/dispatch-lib.sh` — callback delivery routing (verify whether the callback's agent_id needs to differ from the calling agent's session).

**State-coherence concerns:**
- Cross-agent callback delivery: dev-groom subprocess on mika-groomer's slot, callback returns to mika-dev's session. Verify no engine-level assumption that callback sender_agent == receiver_agent. Likely needs `mika ask --task-complete --agent mika-dev` from the groomer's subprocess, which is supported via `--agent` flag (per existing patterns).
- Per-agent-session model: mika-groomer would have its own DB? No — same DB per host (single SQLite file `~/.mika/data/mika.db`). Tasks scoped to agent_id at row level.
- mika-groomer's identity needs to be configured for grooming-only work — denylist for non-grooming tools, allowlist limited to dev-groom skill.

**Verdict (proposed):** more elegant separation but larger surface. New agent provisioning + cross-agent callback verification. ~200-400 lines of Rust + SQL + identity config. Stronger isolation between dispatch classes (grooming subprocess can't accidentally affect mika-dev's session beyond its own callback delivery).

### Option C — `mika-worker` pool (most general)

**Shape:** introduce a pool of ephemeral worker agents (`mika-worker-1`, `mika-worker-2`, ...). The engine maintains a per-class worker assignment registry. Grooming dispatches grab an available groom-class worker; if none available, queue. Generalizes naturally to other dispatch classes (CI fix workers, deploy workers, etc.).

**Files touched:**
- `crates/mika-agent/src/task_engine/worker_pool.rs` (new module).
- `crates/mika-agent/src/dev_mode/` — pool provisioning.
- `crates/mika-agent/src/skills/executor.rs` — worker assignment + release.
- New schema for worker registry, worker→task assignments, pool capacity.

**State-coherence concerns:**
- Larger surface area. Pool size is a tunable. Worker death/restart/recovery needed.
- Open question: how do callbacks deliver back to the originator (mika-dev)? Same cross-agent pattern as Option B, generalized.

**Verdict (proposed):** the right shape if mika is heading toward worker-pool architecture more broadly, but premature for the immediate cadence concern. ~600-1000 lines of Rust + SQL + ops surface. Recommend deferring to a separate ticket if the architect picks B and it works.

### Recommended pick (architect ratifies or pushes back)

**Option A** is recommended:

1. **Lowest-touch.** Single column + single query update + single guard site update. ~50-100 lines.
2. **Composes with mika#996.** mika#996's auto-groom flow already creates separate task IDs for grooming and dispatch (per its plan's `?phase=groom` discriminator discussion). Adding a `dispatch_class` column on those existing task records is additive.
3. **Composes with mika#991.** mika#991's `callback_milestone_advance` guard fires on milestone-context callbacks; the guard's satisfaction predicate (`run_claude_pilot` for next child OR `update_task_status(blocked|completed)`) is unchanged by the slot split.
4. **Defers the larger architecture question.** If milestone wall-clock proves still unacceptable after Option A ships, Options B/C remain available as follow-ups.

**Tradeoff Option A doesn't solve:** if mika-dev needs to do a third concurrent dispatch class (e.g., deploy_mika while grooming + implementing), Option A's binary slot split runs out. Option C's pool handles that natively. The architect's call: accept the binary limit (Option A's ceiling) or invest in the pool now.

## Phase 2 — Implementation (assuming Option A)

**This phase becomes concrete only after the architect ratifies the design pick.** Below is the Option A implementation sketch; Options B/C would have correspondingly different shapes.

### 2.A — Schema migration v31→v32

Add `dispatch_class TEXT` column to `tasks` (nullable for backward compat; non-null on long-running task creation going forward).

```sql
ALTER TABLE tasks ADD COLUMN dispatch_class TEXT
  CHECK (dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom'));
CREATE INDEX idx_tasks_dispatch_class ON tasks(agent_id, dispatch_class, status)
  WHERE dispatch_class IS NOT NULL;
```

Migration is additive-nullable; pre-v32 rows have `dispatch_class IS NULL` and are treated as `'implement'` (default class) by the guard for backward compat.

### 2.B — Update `has_active_callback_tasks_excluding`

Add a `class: &str` parameter. Per the Phase 0 pin, the existing function already takes `excluded_parent_id` + `agent_id`; the new param is the third positional arg. The query revision (against the actual SQL pinned in Phase 0):

```sql
SELECT parent_task_id, id FROM tasks
  WHERE trigger_type = 'callback'
    AND status IN ('pending', 'in_progress')
    AND parent_task_id IS NOT NULL
    AND parent_task_id != ?1
    AND agent_id = ?2
    AND COALESCE(dispatch_class, 'implement') = ?3
  LIMIT 1
```

**SQL-layer COALESCE (architect NF1):** the `COALESCE(dispatch_class, 'implement')` clause handles pre-v32 rows whose `dispatch_class IS NULL`. Treats them as `'implement'` directly in the SQL, NOT in application-layer Rust code. This ensures direct DB queries, debugging sessions, and any future tooling all see consistent semantics — application-layer NULL coercion would be a maintenance hazard for a column that can have NULL rows in production indefinitely (no backfill).

The `task_id` exclusion + agent_id scope from the existing query are unchanged. Adding the class predicate makes the query class-specific without affecting the pre-existing scope semantics.

### 2.C — Update `validate_dispatch_readiness`

`executor.rs:785-814` becomes:

```rust
let class = match skill {
    "dev-pilot" | "deploy_mika" => "implement",
    "dev-groom" => "groom",
    _ => "implement",  // default for unknown skills
};

match db.has_active_callback_tasks_excluding(task_id, class).await {
    // ... existing match arms unchanged ...
}
```

### 2.D — Set `dispatch_class` on task creation

Long-running task creation site (in `executor.rs`) sets `dispatch_class` from the skill mapping above. Pre-existing tasks remain NULL; new tasks always have a class.

### 2.E — Compose with mika#996 task-reuse pattern (architect F2 — committed)

**Decision: option (a) — flip `dispatch_class` on the existing task.** This commits the plan to the only viable shape; option (b) (sibling task) is eliminated at plan time.

mika#996's M4 grooming pre-flight reuses the milestone child's `task_id` across grooming and dispatch phases. mika#996 is already GROOMED with this task-reuse contract baked into its plan. Per cross-ticket sequencing rule (`feedback_dont_decorate_forced_decisions` + the issue-as-versioned-contract pattern): a downstream ticket cannot adopt a shape that requires revising an upstream GROOMED plan. Option (b) (sibling task with `parent_task_id = milestone_child_id`) would do exactly that — it would invalidate mika#996's task-reuse assumption.

**Implementation shape (committed):**

- When dev-groom fires for the milestone child, the task's `dispatch_class` is set to `'groom'` for that lifecycle. The same `task_id` is what mika-dev's auto-groom path passes to `run_claude_pilot`.
- When the dev-groom callback returns `Verdict: GROOMED` (per mika#996's pinned output contract), mika-dev's auto-groom callback handler flips the task's `dispatch_class` to `'implement'` BEFORE issuing the dev-pilot dispatch:

  ```rust
  // After dev-groom GROOMED callback, before dev-pilot dispatch:
  db.update_task_dispatch_class(child_task_id, "implement").await?;
  // Then dispatch dev-pilot per mika#996's existing flow:
  run_claude_pilot({"skill": "dev-pilot", "task_id": child_task_id, ...})
  ```

- `update_task_dispatch_class` is a new (or extended) DB method. Atomically updates the column. Idempotent.

**State sequence verified compatible with mika#996:**

| Phase | Task state | dispatch_class |
|---|---|---|
| M4 child created | status=pending, no callback children | NULL (initial) |
| Auto-groom dispatched (mika#996 Phase 3) | status=in_progress, callback child for dev-groom | `'groom'` (set on dispatch) |
| dev-groom callback returns GROOMED | status=in_progress, callback child terminal | `'groom'` → flip to `'implement'` |
| dev-pilot dispatched (mika#996 Phase 3 step e) | status=in_progress, callback child for dev-pilot | `'implement'` |
| dev-pilot callback returns | status=in_progress | `'implement'` (unchanged) |
| Milestone child closes (mika#991 advance) | status=completed | `'implement'` (frozen) |

**The flip is a single atomic update** — no race because dev-groom's callback delivery is serialized via the engine's callback delivery path (per `crates/mika-agent/CLAUDE.md` § "Callback/resume lifecycle"), which guarantees the callback turn completes before the next `run_claude_pilot` is permitted.

**Forensic audit (architect NF2):** when concurrent dispatches on the same agent (one `'implement'` + one `'groom'`) write interleaved messages to mika-dev's session, the `task_id` field on each message partitions the timeline. A query like `SELECT * FROM messages WHERE session_id = ? ORDER BY created_at` produces the chronological sequence; partitioning by `task_id` yields per-dispatch causal ordering. The async DB serialization (per `crates/mika-agent/CLAUDE.md` § "Async DB") guarantees no within-dispatch reordering. Cross-dispatch interleaving is acceptable for steady-state operation; per-dispatch causality is the load-bearing invariant.

## Phase 3 — Tests

**Test 1 — concurrency baseline (Option A):** simulate two long-running dispatches on the same `agent_id` with different classes (`implement` + `groom`). Assert both complete without `global_dispatch_active` rejection. Assert session message ordering preserves causality (each dispatch's callbacks arrive in the right order for that dispatch's task_id chain).

**Test 2 — same-class rejection still fires:** simulate two `implement`-class dispatches on the same agent. Assert second one rejects with `global_dispatch_active`.

**Test 3 — milestone cascade integration with mika#996+#991 (architect NF3 — coexists with mika#991 AC#3 test):** enqueue 3 milestone children. Simulate child 1 dispatching (`dev-pilot`) AND child 2 grooming (`dev-groom`) concurrently. Assert both complete; assert child 2's Plan callout is committed before child 1 callback returns; assert child 3 grooming starts as soon as child 2 grooming completes (pipelining works steady-state).

**Coexistence with mika#991 AC#3 test (Test 7 in mika#991's plan):** mika#991's 3-dispatch chained-advance test exercises sequential advance without LLM turn between callbacks (asserts `callback_milestone_advance` guard + `PostCallbackAdvance` trigger fire correctly). mika#1001's Test 3 exercises pipelined concurrency (asserts the per-class slot split allows dispatch N + groom N+1 simultaneously). The fixtures overlap (3 milestone children, terminal-status simulations) but the assertions differ — mika#991 asserts sequential causality of advance; mika#1001 asserts pipelined wall-clock overlap. **Both tests must pass independently in the eval harness.** If a fixture conflict surfaces (e.g., shared mock LLM sequence), the implementer surfaces to operator before splitting the fixtures.

**Test 4 — pre-v32 task compatibility:** seed task rows with `dispatch_class IS NULL` (simulating pre-migration tasks). Assert the guard treats them as `'implement'` correctly.

**Halt threshold (sibling of mika#988 Phase 3, mika#996 Phase 4, mika#991 Phase 6):** if any test requires more than **80 lines** of harness setup beyond existing patterns, halt and surface to operator. Concurrency tests are inherently more complex; if 80 lines isn't enough, the architecture may need rework.

## Phase 4 — Documentation + out-of-scope follow-ups

**Documentation:**
- `crates/mika-agent/CLAUDE.md` § "Long-running": update the dispatch-readiness guard description to note the per-class split. Add a callout that the slot is binary (one implement + one groom max per agent).
- `mika/CLAUDE.md` autonomous-loop section: one-paragraph note on pipelined grooming.
- `mika/docs/solutions/best-practices/per-class-dispatch-slot-2026-05-XX.md` (new compound at PR-close): the principle: when a single-session-at-a-time guard becomes a cadence bottleneck for a specific dispatch class, splitting the slot by class is the lightest-touch shape — preserves state-coherence invariants per task class without reaching for a worker-pool architecture.

**Follow-ups filed at PR-merge time:**
1. **Worker-pool architecture (Option C-shape) if Option A's binary ceiling proves insufficient.** Trigger: third concurrent dispatch class becomes a need (e.g., deploy_mika simultaneous with grooming + implementation).
2. **mika-groomer dedicated agent (Option B-shape).** Defer unless cross-agent isolation becomes a security or auditing concern.

## Acceptance criteria (from the ticket)

- [x] Steady-state autonomous-loop cadence: grooming for the next ungroomed ticket completes by the time the current ticket's dispatch completes (no per-ticket sequential blocking on the architect roundtrip). **Phase 2.B-2.E** (Option A per-class slot, architect-confirmed).
- [x] Agent state coherence: no observable race between concurrent grooming and dispatch. **Phase 0 pin** confirms async DB serialization and per-task-id state isolation; **Phase 3 Test 1** verifies empirically.
- [x] Test coverage: milestone-cascade test enqueuing 5 ungroomed tickets with steady-state pipelining verified. **Phase 3 Test 3.**
- [x] If chosen option introduces new failure modes, each has a named recovery path. **Phase 1 per-option state-coherence concerns enumerated.**

## Risks and known unknowns

- **Risk: Option A's binary slot ceiling.** Two-class limit. If a third concurrent dispatch class becomes a need, the model breaks. Mitigation: Phase 4 follow-up #1 escalates to Option C if needed.
- **Risk: cross-callback session message ordering.** Two callbacks writing to mika-dev's session messages table — no race per AsyncDatabase serialization, but the perceived ordering (which message appears first in the session log) may interleave across dispatches. Mitigation: existing `created_at` timestamps are ISO 8601 strings with second precision; if higher precision needed for forensic analysis, separate ticket. Acceptable for steady-state operation.
- **Resolved at plan time (was the mika#996 task-reuse interaction unknown):** committed to option (a) — flip `dispatch_class` on the existing task — per architect F2 + cross-ticket sequencing rule. Phase 2.E states the implementation shape with a state-sequence table verifying compatibility with mika#996's task-reuse contract.
- **Unknown: existing pre-v32 task rows with `dispatch_class IS NULL` and active callbacks at migration time.** Phase 2.A backward-compat shim treats them as `'implement'`; if a pre-v32 in-flight `dev-groom` task is mid-dispatch at migration time, it gets classed as `implement` retroactively. Mild edge case; surfaces only during the deploy window.

## Compound learning to write at PR-close

A short compound at `mika/docs/solutions/best-practices/per-class-dispatch-slot-2026-05-XX.md`. Title: **"Per-class slot split: lightest-touch concurrency for asymmetric dispatch classes."** Principle:

> When a single-session-at-a-time guard becomes a cadence bottleneck for a specific dispatch class (e.g., grooming), and the dispatch classes have orthogonal state-coherence requirements (i.e., grooming touches plan files; implementation touches code files), splitting the guard by class is lighter-touch than introducing a worker-pool architecture. The state-coherence invariants per class are preserved by the existing async DB serialization plus per-task-id row isolation; the guard split only relaxes the cross-class constraint.

Contrapositive: when dispatch classes share state-coherence requirements (e.g., two implementation dispatches both writing to the same code surface), per-class splits are unsafe — worker-pool is the right shape because each worker has its own isolated state.
