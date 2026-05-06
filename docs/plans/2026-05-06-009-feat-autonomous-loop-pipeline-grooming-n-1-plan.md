---
ticket: mika#1001
type: feat
title: Pipeline grooming N+1 concurrently with dispatch N
date: 2026-05-06
seq: 009
---

# Plan: pipeline grooming N+1 concurrently with dispatch N (mika#1001)

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

### What the guard's existing call sites actually check

**`mika/crates/mika-agent/src/db.rs`** — `has_active_callback_tasks_excluding(task_id)` returns the first active callback task whose parent is NOT `task_id`. The query is implicitly scoped to the calling DB connection's agent_id (single-tenant DB-per-agent).

The `task_id` exclusion is so the same parent can re-dispatch on retry without false-positive blocking itself. The guard does NOT differentiate between dispatch classes (grooming vs implementation).

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

Add a `class: &str` parameter. Query becomes:

```sql
SELECT parent.id AS parent_id, child.id AS child_id
  FROM tasks parent
  INNER JOIN tasks child ON child.parent_task_id = parent.id
  WHERE parent.agent_id = ?
    AND parent.id != ?
    AND parent.dispatch_class = ?
    AND child.trigger_type = 'callback'
    AND child.status IN ('pending', 'in_progress')
  LIMIT 1
```

Pre-v32 rows with `dispatch_class IS NULL` still match `'implement'` queries via a `COALESCE(dispatch_class, 'implement')` shim during transition.

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

### 2.E — Compose with mika#996 task-reuse pattern

mika#996's M4 grooming pre-flight reuses the milestone child's `task_id` across grooming and dispatch phases. With Option A:
- When dev-groom fires for the child, the task's `dispatch_class` is `'groom'` for that lifecycle.
- After dev-groom completes (Verdict: GROOMED), the task's `dispatch_class` flips to `'implement'` for the dev-pilot dispatch.
- `update_task_status` accepts an optional `dispatch_class` field for this transition.

Alternative: keep the task `task_id` constant but track grooming as a sibling task with `parent_task_id = milestone_child_id`. Cleaner separation, slightly more rows in the tasks table. **Architect ratifies which shape.**

## Phase 3 — Tests

**Test 1 — concurrency baseline (Option A):** simulate two long-running dispatches on the same `agent_id` with different classes (`implement` + `groom`). Assert both complete without `global_dispatch_active` rejection. Assert session message ordering preserves causality (each dispatch's callbacks arrive in the right order for that dispatch's task_id chain).

**Test 2 — same-class rejection still fires:** simulate two `implement`-class dispatches on the same agent. Assert second one rejects with `global_dispatch_active`.

**Test 3 — milestone cascade integration with mika#996+#991:** enqueue 3 milestone children. Simulate child 1 dispatching (`dev-pilot`) AND child 2 grooming (`dev-groom`) concurrently. Assert both complete; assert child 2's Plan callout is committed before child 1 callback returns; assert child 3 grooming starts as soon as child 2 grooming completes (pipelining works steady-state).

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

- [x] Steady-state autonomous-loop cadence: grooming for the next ungroomed ticket completes by the time the current ticket's dispatch completes (no per-ticket sequential blocking on the architect roundtrip). **Phase 2.B-2.E** (per-class slot) — assuming Option A. If architect picks B or C, the equivalent capability ships under that shape.
- [x] Agent state coherence: no observable race between concurrent grooming and dispatch. **Phase 0 pin** confirms async DB serialization and per-task-id state isolation; **Phase 3 Test 1** verifies empirically.
- [x] Test coverage: milestone-cascade test enqueuing 5 ungroomed tickets with steady-state pipelining verified. **Phase 3 Test 3.**
- [x] If chosen option introduces new failure modes, each has a named recovery path. **Phase 1 per-option state-coherence concerns enumerated.**

## Risks and known unknowns

- **Risk: Option A's binary slot ceiling.** Two-class limit. If a third concurrent dispatch class becomes a need, the model breaks. Mitigation: Phase 4 follow-up #1 escalates to Option C if needed.
- **Risk: cross-callback session message ordering.** Two callbacks writing to mika-dev's session messages table — no race per AsyncDatabase serialization, but the perceived ordering (which message appears first in the session log) may interleave across dispatches. Mitigation: existing `created_at` timestamps are ISO 8601 strings with second precision; if higher precision needed for forensic analysis, separate ticket. Acceptable for steady-state operation.
- **Unknown: how Option A composes with mika#996's task-reuse pattern.** Phase 2.E names two alternatives (flip class on existing task vs. spawn sibling task); the architect's grooming pass picks. Either works structurally.
- **Unknown: existing pre-v32 task rows with `dispatch_class IS NULL` and active callbacks at migration time.** Phase 2.A backward-compat shim treats them as `'implement'`; if a pre-v32 in-flight `dev-groom` task is mid-dispatch at migration time, it gets classed as `implement` retroactively. Mild edge case; surfaces only during the deploy window.

## Compound learning to write at PR-close

A short compound at `mika/docs/solutions/best-practices/per-class-dispatch-slot-2026-05-XX.md`. Title: **"Per-class slot split: lightest-touch concurrency for asymmetric dispatch classes."** Principle:

> When a single-session-at-a-time guard becomes a cadence bottleneck for a specific dispatch class (e.g., grooming), and the dispatch classes have orthogonal state-coherence requirements (i.e., grooming touches plan files; implementation touches code files), splitting the guard by class is lighter-touch than introducing a worker-pool architecture. The state-coherence invariants per class are preserved by the existing async DB serialization plus per-task-id row isolation; the guard split only relaxes the cross-class constraint.

Contrapositive: when dispatch classes share state-coherence requirements (e.g., two implementation dispatches both writing to the same code surface), per-class splits are unsafe — worker-pool is the right shape because each worker has its own isolated state.
