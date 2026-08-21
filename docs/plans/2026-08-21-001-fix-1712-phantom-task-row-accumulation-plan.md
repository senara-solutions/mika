---
ticket: mika#1712
type: fix
scope: engine
shape: (b) sweep-only (ratified sami 2026-08-21)
schema_version_change: yes
---

# fix(engine): phantom task-row accumulation (shape (b) sweep-only)

## Rationale — why sweep-only

Sami arbitrated the shape (mécanisme, pas Prime) on 2026-08-21 (bearing
`/var/spool/claude-mail/mpc/archive/2026-08-21-143740-*.md`, summarized in issue comment
#5373564110). Verbatim:

> **Route : (b) + re-groom séparé.**
> 1. **Ship AC3+AC5 sweep-only maintenant** — ça couvre entièrement le symptôme observé
>    (24 rows/18h) sans casser la sémantique de `create_task`. AC1/AC2/AC4 tombent (ils
>    rejetteraient du légitime — tes divergences 4-6 sont convaincantes).
> 2. **Corrige le body du ticket avant d'implémenter** (fichier réel
>    `task_engine/engine.rs:482-649`, schema v44, mechanism = tracking-row-orphaned) — on
>    ne ship pas sur un body faux, même en sweep-only.
> 3. **Le sweep DOIT émettre sa télémétrie** : rows balayées par passe, loggées — pas de
>    cap silencieux. C'est la donnée d'entrée du point 4.
> 4. **Cause racine (« pourquoi les tracking rows orphelinent ») = ticket séparé, groom
>    pass 2**, p2, nourri par la télémétrie du sweep. Si le rythme de fuite accélère, il
>    remonte en p1 — le signal de travail nous le dira (engine-liveness).
> 5. **(c) rejeté** : changer la sémantique de tous les tracking tasks pour faire tenir
>    un AC écrit sur un mécanisme qui n'existe pas, c'est l'inverse de la bonne
>    direction.

**Schema-version note.** Sami's bearing (§2) references schema v44 because that was the
head at the time of the bearing (2026-08-21 14:37Z). Between the bearing and this plan's
architect second-pass, main advanced to v45 via mika#1865 (`f58cd49a` —
`pilot_transcripts` observability table). This plan therefore targets **v45 → v46**
(marker migration for the phantom sweep), not v44 → v45. Sami's schema-invariant point
still holds — new defenses land at the current schema head + 1, whatever that is at
implementation time.

**Rebase requirement.** Branch base is `b983a8ff` (pre-v45-bump). Implementer MUST rebase
onto `origin/main` before Phase 1; without the rebase, `migrate_v45_to_v46` will not
apply cleanly and the compile-time schema-pin at
`crates/mika-agent/tests/eval/kg_fixtures/mod.rs:26` (`PINNED_SCHEMA_VERSION`) will need
its own bump 45 → 46. See mika#1865's fix commit for the pattern (same file, same
mechanism, previous PR bumped 44 → 45).

**What this plan implements (shape (b)):** AC3 watchdog sweep + AC5 startup sweep + AC6
regression test + AC7 sweep telemetry. Ships the defense that stops the observed bleed
(24 rows / 18h → 0 rows accumulated) without changing `create_task` semantics.

**What this plan does NOT implement:** AC1 (refuse-to-write invariant), AC2 (hydrate-or-fail
lifecycle), AC4 (`phantom_write` audit) — all three superseded by sami bearing §1.
Divergences D1/D2/D3 documented in §7 explain WHY. The write-time enforcement path was
inverse-good-direction: it would reject legitimate tracking tasks. The cause-racine
investigation ("why the tracking rows orphan mid-orchestration") is deferred to mika#1934
(blocked-by #1712), fed by the AC7 telemetry.

## Problem

The `tasks` table accumulates rows with `action_type='none'`, `process_id IS NULL`,
empty `input_context`, empty `result`, and `status IN ('in_progress','blocked','pending')`
that occupy per-class dispatch slots but never dispatch and never terminate. Operator
observed 24 such rows accumulate over ~18h on 2026-07-01 (mika-dev, agent scope) and
manually swept them via ad-hoc SQL.

The existing callback watchdog
(`crates/mika-agent/src/task_engine/engine.rs:482-649` —
`check_callback_process_liveness`, cadence 60 ticks) cannot sweep them because its very
first predicate (`engine.rs:501-504`) is `process_id IS NOT NULL` — a NULL-PID row is
invisible to `/proc/<pid>/stat` liveness comparison. The orphaned-parent reaper
(`task_engine::engine::reap_orphaned_parent_tasks`, `engine.rs:661-817`) also misses
these because it JOINs on a `resume_agent` callback child, which the phantom rows do not
have.

## Root cause (verified in code — mechanism = orphaned tracking rows)

### Where the "phantom" rows actually come from

The ticket's original file path `crates/mika-agent/src/task/callback_watchdog.rs` **does
not exist**. The real watchdog surface is:

- `crates/mika-agent/src/task_engine/process_liveness.rs` — pure `/proc/<pid>/stat`
  primitives (`is_same_process_alive`, `read_process_start_time`).
- `crates/mika-agent/src/task_engine/engine.rs:482-649` — `check_callback_process_liveness`,
  called every `DB_SCAN_INTERVAL_TICKS = 60` ticks from `TaskEngine::tick`
  (`engine.rs:232`). Loop skips any task whose `process_id` is NULL or ≤ 0
  (`engine.rs:501-504`).
- `crates/mika-agent/src/task_engine/engine.rs:661-817` — `reap_orphaned_parent_tasks`
  (mika#871), which relies on a `resume_agent` callback child + missing `pr_url` and
  never fires for the tracking-task shape.

### Sole production writers of `action_type='none'`

Grep across `crates/mika-agent/src/` for `action_type::NONE` + `trigger_type::MANUAL`
finds exactly one production writer, in two branches of the same tool:

- `crates/mika-agent/src/tools/create_task.rs:235-258` (primary INSERT).
- `crates/mika-agent/src/tools/create_task.rs:318-341` (retry after unique-constraint
  race with dedup).

Both branches emit `NewTask { trigger_type: MANUAL, action_type: NONE, next_fire_at:
None, action_config: "{}", input_context: None, .. }`. This is **not** a placeholder
awaiting hydration — it is the **final steady state of a tracking task**. The
`create_task` tool description at `create_task.rs:44-51` names the surface: "Create a
trackable task to represent a piece of work." Status transitions on these rows
(`update_task_status` → `in_progress`/`blocked`/`completed`, `db.rs:5307`) never touch
`action_type` or `process_id`.

Every other `action_type: "none"` hit in the src tree is either (a) a test-setup helper
constructing a synthetic `Task`/`NewTask` for a unit test (`tools/list_tasks.rs:207`,
`tools/check_task.rs:346`, `tools/post_action_hooks.rs:305`, etc.), (b) a display
fallback ("show 'none' when field is empty") in the TUI/API surfaces, or (c) a docstring
token. There is no code path that inserts a placeholder-then-UPDATEs to a real
`action_type`.

### The actual leak class — orphaned tracking rows

The `tasks` table CHECK constraint (`db.rs:1174-1179`) allows `action_type='none'` in
any combination with any `status`. The two leaked samples cited in the ticket —
`52835a02 mika-dev mika#1583 nudge-driven skill creation | in_progress` and
`613a996d mika-dev mika#1584 curator background task | pending` — match the create_task
label shape exactly. These are tracking rows that were transitioned to `in_progress`
(operator or agent said "start work") and never transitioned to a terminal status
because the surrounding orchestration (auto-groom → auto-pilot → PR → close) wedged or
was abandoned. **No handler crashed mid-hydration.**

Leak class = **orphaned tracking rows** (create → in_progress → surrounding
orchestration wedged, no terminal transition). Not create-placeholder-then-hydrate
(that mechanism does not exist in current code).

WHY the surrounding orchestration wedges is out of scope for this plan — mika#1934 is
the follow-up investigation, blocked-by #1712, fed by the AC7 telemetry (rows/pass rate
signal).

## Acceptance criteria tie-back

Shape (b) shipped ACs:

- **AC3** → §5 AC3 + §6 Phase 3. NULL-PID phantom sweep added to the existing 60-tick
  watchdog.
- **AC5** → §5 AC5 + §6 Phase 4. Startup sweep added to `TaskEngine::startup_recovery`.
- **AC6** → §5 AC6 + §6 Phase 6. New integration test in
  `crates/mika-agent/tests/eval/test_phantom_task_row_sweep.rs`.
- **AC7** → §5 AC7 + §6 Phase 3/4. Sweep telemetry: per-row audit event
  (`tool_name='phantom_aged_out'`) + per-pass aggregate log line + count metric surface.
  No silent cap.

Superseded (sami bearing §1):

- **AC1** → SUPERSEDED. Divergence D1 (§7): AC1 rejects every legitimate `create_task`
  call because tracking tasks by design have `next_fire_at = None`. Enforcement is
  inverse-good-direction.
- **AC2** → SUPERSEDED. Divergence D2 (§7): AC2 rejects every
  `update_task_status(id, "in_progress")` on manual tracking rows — that IS the
  documented lifecycle, not a bug.
- **AC4** → SUPERSEDED. Divergence D3 (§7): `phantom_write` fires on every `create_task`
  (~50-500x/day/agent), producing noise proportional to normal usage. AC7 sweep
  telemetry replaces the audit signal AC4 was reaching for, keyed to actual anomalies
  (sweeps) rather than normal writes.

## Design decisions

### AC3 — watchdog sweep for NULL-PID phantoms

**Cadence:** reuse the existing 60-tick tick loop in `engine.rs:220-280` (matches
`check_callback_process_liveness` cadence). No new interval needed.

**Placement:** add `sweep_null_pid_phantoms()` as a new method on `TaskEngine`, called
from `tick()` immediately after `check_callback_process_liveness()` (`engine.rs:232`).
Runs inside the same tick window; the mutex is already held.

**SQL** (new method `Database::find_phantom_tracking_tasks(agent_id, age_seconds)` in
`db.rs`, alongside `find_orphaned_parent_tasks` at `db.rs:6203`):

```sql
SELECT id, agent_id, label, status, created_at, updated_at
FROM tasks
WHERE agent_id = ?1
  AND action_type = 'none'
  AND process_id IS NULL
  AND status IN ('in_progress', 'blocked')
  AND updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
ORDER BY id;
```

**Grace period:** 60 minutes default via
`Settings::effective_phantom_sweep_age_seconds()` (new accessor, env override
`MIKA_PHANTOM_SWEEP_AGE_SECONDS`, default `3600`). Placed in `Settings` alongside
`callback_watchdog_grace_period_secs` at `config.rs:913` (the existing operational
watchdog knob), following the same `Option<u64>` + `effective_*()` accessor pattern.
Read at `self.dispatcher.settings.effective_phantom_sweep_age_seconds()` in
`sweep_null_pid_phantoms()` (mirrors `engine.rs:494-498`). Passed to SQL as the
negative-offset string `format!("-{age_seconds} seconds")` so `strftime` computes
`now - age_seconds`.

**Rationale for making it tunable (mika-arch F3):** Some tracking tasks legitimately
represent multi-hour or multi-day work (long-running groom tickets, multi-day
milestones). If sami/operator observes a false-positive on a legitimate long-lived
tracking row, a config nudge (raise the env var to 7200/14400) unblocks without a
code roll. `review-guide.md § KISS` — hardcoded magic numbers encoding operational
policy are technical debt.

**Kill-switch (unchanged):** Setting `MIKA_PHANTOM_SWEEP_AGE_SECONDS=0` is NOT the
disable path (0 sweeps everything immediately). Instead, add a second env
`MIKA_PHANTOM_SWEEP_DISABLED=1` that short-circuits both AC3 and AC5 sweep functions
at their head. Not implemented in v1 unless the architect confirms the operator wants
a switch — the tunable grace via F3 covers the "false-positive" failure mode without
needing a full disable.

**Transition logic:** for each matched row, call `update_task_failed(id, agent_id,
"phantom_aged_out")` (guarded UPDATE with terminal-state race protection,
`db.rs:5355`). On `Ok(true)`:

1. Increment `swept_count` (u32 in-scope counter for AC7 aggregate).
2. Write `audit_events` row with `tool_name='phantom_aged_out'`,
   `target_key = format!("task:{id}")`, `before_value = Some(&status)`,
   `after_value = Some("failed")`,
   `reasoning = Some("phantom_aged_out: manual/none row with NULL process_id aged past 60min")`,
   `trace_id = Some(&trace_id)`, `session_id = format!("system-{agent_id}")`.
   Matches the reaper's audit-event shape (`engine.rs:749-767`).

On `Ok(false)` (row already terminal — lost race with another sweep): log at debug and
continue without incrementing the counter.

**SOLE WRITER contract:** `phantom_aged_out` audit tool_name is written *only* by this
method — pair it with a `// SOLE WRITER: phantom_aged_out` comment matching the reaper
convention (`engine.rs:660`, `engine.rs:836`).

**Per-pass telemetry (AC7 tick source):** at the end of the sweep loop, emit
`info!(event = "phantom_sweep_complete", source = "watchdog_tick", count = swept_count,
agent_id = %agent_id)`. No silent cap on `swept_count` — the loop iterates the full
result set from `find_phantom_tracking_tasks`. If the operator observes a runaway pass
count, that IS the AC7 signal (feeds mika#1934), not something to suppress.

### AC5 — startup sweep

**Location:** extend `TaskEngine::startup_recovery` at `engine.rs:87-166`, adding a
new step 2b between the existing step 2 (`in_progress` orphan sweep) and step 3 (heap
load).

**SQL:** reuse `find_phantom_tracking_tasks` from AC3 with `age_seconds = 0` (all
matching rows regardless of freshness). Rationale: any phantom row present at startup
outlived a prior server process — that is the "startup sweep" AC5 wants.

**Transition logic:** for each match, call `update_task_failed(id, agent_id,
"startup_sweep")` and emit an `audit_events` row with
`tool_name='phantom_aged_out'`, `after_value=Some("failed")`,
`reasoning = Some("startup_sweep: pre-existing phantom found at boot")`. Note:
`tool_name` stays `phantom_aged_out` (single sole-writer discriminator); the source is
carried by the `reasoning` field and the aggregate log line (below). This keeps the
SQL query surface for the audit-event ledger a single `WHERE tool_name='phantom_aged_out'`
predicate rather than a two-branch union.

**Per-pass telemetry (AC7 startup source):** aggregate the count and emit exactly one
info line if `n > 0`:

```
info!(event = "phantom_sweep_complete", source = "startup_sweep", count = n,
      agent_id = %agent_id, reason = "phantom_signature_null_pid_manual_none");
```

Zero-hit case stays quiet (mirrors the existing `prune_old_sessions` log shape at
`engine.rs:141-145`).

**Large-backlog warn threshold (mika-arch F5):** when `n > 100`, additionally emit
```
warn!(event = "phantom_sweep_large_backlog", count = n, agent_id = %agent_id,
      source = "startup_sweep");
```
This does NOT cap the sweep — all `n` audit_events and the aggregate info line still
fire (per sami §3 "no silent cap"). The warn adds operator visibility on anomalous
state without hiding data. Threshold documented in Signal O.

### AC7 — sweep telemetry (mandatory per sami bearing §3)

**Purpose:** provide the data-input to the cause-racine investigation (mika#1934).
Without a rate signal on rows/pass over time, #1934 cannot escalate from p2 to p1 on
worsening. The telemetry IS the engine-liveness signal.

**Three-surface contract:**

1. **Per-row audit event.** Every swept row writes one `audit_events` row with
   `tool_name='phantom_aged_out'`. Queryable via SQL for post-hoc analysis:
   ```sql
   SELECT DATE(created_at) AS day, COUNT(*) AS rows_swept
   FROM audit_events
   WHERE tool_name = 'phantom_aged_out'
   GROUP BY day ORDER BY day;
   ```
2. **Per-pass aggregate log line.** Both AC3 and AC5 emit
   `event="phantom_sweep_complete"` with `source ∈ {"watchdog_tick","startup_sweep"}`
   and `count=N`. Greppable in `server.log`:
   ```
   grep phantom_sweep_complete server.log | jq '{ts:.timestamp, source:.source, count:.count, agent:.agent_id}'
   ```
3. **No silent cap.** Neither AC3 nor AC5 caps the sweep count. If a single pass sweeps
   thousands of rows, all thousand emit an audit_event and the aggregate line reports
   the true count. Operator sees the outlier immediately.

**Field mapping for `audit_events`** (given `audit_events` schema at `db.rs:1329-1345`
has no `kind` column — the discriminator is `tool_name`, per the codebase convention
set by `task_engine_reaper` at `engine.rs:753` and `task_engine_parent_completer` at
`engine.rs:879`):

| audit_events col | Value                                                          |
|------------------|----------------------------------------------------------------|
| `tool_name`      | `"phantom_aged_out"` (both AC3 and AC5 — single sole-writer)   |
| `target_key`     | `format!("task:{id}")`                                          |
| `before_value`   | `Some(&status)` (pre-sweep status: `"in_progress"` or `"blocked"`) |
| `after_value`    | `Some("failed")`                                                 |
| `reasoning`      | `Some(&format!("phantom_aged_out: manual/none row with NULL process_id aged past 60min"))` (AC3) or `Some("startup_sweep: pre-existing phantom found at boot")` (AC5) |
| `session_id`     | `format!("system-{agent_id}")`                                   |
| `trace_id`       | `Some(generate_trace_id())` (fresh per sweep pass)              |

### AC6 — regression test

**File:** new `crates/mika-agent/tests/eval/test_phantom_task_row_sweep.rs`, registered
via `mod test_phantom_task_row_sweep;` in `tests/eval.rs` next to
`test_phantom_retry_guard` (`tests/eval.rs:62`).

**Harness:** the existing `test_phantom_retry_guard.rs` uses `EvalHarness` +
`MockLlmProvider`. The AC6 scenario does not need an LLM — it needs a DB, a
`TaskEngine`, and a way to trigger a tick. Use `TaskEngine::new` + direct DB inserts +
`engine.tick()` in a loop, matching the pattern in `engine.rs:1261-1381`
(`test_startup_recovery_empty_db`, `test_periodic_scan_picks_up_new_tasks`).

**Injection strategy** (critical — this is what makes the test verify the FIX, not just
green under absence of the bug): each test constructs the phantom row explicitly via
`db.insert_task(NewTask { trigger_type: MANUAL, action_type: NONE, process_id: None,
input_context: None, .. })` with the aged `updated_at` timestamp manually backdated via
a direct SQL `UPDATE tasks SET updated_at = ? WHERE id = ?` (mirrors
`test_expire_timed_out_tasks` at `engine.rs:1385`). This injects the exact leak class
the plan targets. Without the injection, the test would trivially pass by asserting on
a codebase that never produces the shape naturally in-test.

**Verify-fix-is-load-bearing** (per `feedback_verify_pipeline_passes_without_the_fix`,
revised per mika-arch F4 — SQL injection + audit_events count assertion, no engine
plumbing):

Instead of stubbing the sweep method (which needs invasive `pub(crate)` test-only
fields on `TaskEngine`), the load-bearing assertion is:

1. Inject the aged phantom row via direct DB INSERT + backdated `updated_at`.
2. **Baseline query** — `assert_eq!(db.count_audit_events_by_tool_name("phantom_aged_out"), 0)`.
3. Run one full `engine.tick()` past the 60-tick sweep interval.
4. **Post-sweep query** — `assert_eq!(db.count_audit_events_by_tool_name("phantom_aged_out"), 1)`
   AND `assert_eq!(db.get_task_status(id), "failed")`.
5. **Row-shape assertion** — fetch the audit_events row, assert its `target_key`
   contains the injected task id and `after_value == "failed"`.

Load-bearing: if the sweep code path is removed (e.g., the call from `tick()` is
commented out, or `sweep_null_pid_phantoms` is early-returned), assertion #4 fails
(audit count stays 0). If the injection is not exercising the shape (e.g., wrong
predicate), assertion #4 fails too. This IS the "does the test see the bug when the
fix is removed?" check mika#1173 established, without needing engine test-only fields.

A tiny helper `db.count_audit_events_by_tool_name(&str) -> Result<i64>` is added to
`db.rs` (private test-only or `pub(crate)`, no external callers) to keep the assertion
readable.

**Assertion form (three primary tests + audit-count load-bearing thread):**

1. `phantom_ages_out_after_grace` — inject a manual/none/in_progress/NULL-PID row with
   `updated_at = now - 3700s`, baseline-assert `audit_events(phantom_aged_out) == 0`,
   tick until sweep runs (one tick past the 60-tick interval), assert
   `status == 'failed'` AND `audit_events(phantom_aged_out) == 1` AND the audit row's
   `target_key` includes the injected task id.
2. `fresh_manual_none_row_not_swept` — inject same shape but with `updated_at = now`,
   baseline-assert `audit_events(phantom_aged_out) == 0`, tick 61 times, assert
   `status == 'in_progress'` AND `audit_events(phantom_aged_out) == 0` (age guard
   prevents both the status transition and the audit write).
3. `startup_sweep_clears_preexisting_phantoms` — inject two phantom rows directly into
   a fresh DB, call `TaskEngine::new` + `startup_recovery`, assert both are `failed`
   with `audit_events(phantom_aged_out) == 2` (reasoning field distinguishes
   `startup_sweep` from `watchdog_tick`).

The load-bearing property is achieved by the baseline-then-delta audit_events
assertion in tests 1 and 3: if the sweep is bypassed, count stays 0 and the test
fails. No engine plumbing needed (per mika-arch F4).

Timeout: each test bounded at 5s. Pattern: `test_tick_fires_due_task` at
`engine.rs:1289`.

## Implementation steps

### Phase 1 — schema migration (v45 → v46)

- File: `crates/mika-agent/src/db.rs`.
  - Bump `CURRENT_SCHEMA_VERSION` from 45 to 46 (`db.rs:30`).
  - Add `fn migrate_v45_to_v46(&mut self) -> Result<()>` beneath `migrate_v44_to_v45`
    (`db.rs:4361`). Body: no DDL — marker-only migration to give a version anchor for
    the new watchdog sweep semantics.
    ```rust
    // v46: behavioral change only (phantom NULL-PID sweep, mika#1712).
    // No DDL. Reserved for future DDL if write-time enforcement (AC1/AC2) lands
    // via cause-racine investigation (mika#1934).
    let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("INSERT INTO schema_version (version) VALUES (46)", [])?;
    tx.commit()?;
    info!("v45→v46: no DDL; behavioral marker for mika#1712 phantom sweep");
    Ok(())
    ```
  - Register in the migration chain block around `db.rs:1046`.
- Update `crates/mika-agent/CLAUDE.md` migration ledger (the `Schema Version` section)
  with a v45→v46 entry noting: "no DDL; behavioral marker for mika#1712 phantom
  NULL-PID sweep in `TaskEngine`. Reserved for future DDL if write-time enforcement
  lands (mika#1934)." (per mika-arch F2)
- Bump `PINNED_SCHEMA_VERSION` in
  `crates/mika-agent/tests/eval/kg_fixtures/mod.rs:26` from 45 to 46 (compile-time
  assertion at line 28 will otherwise fail — same pattern as mika#1865's fix
  commit).

### Phase 2 — new DB method for phantom detection

- File: `crates/mika-agent/src/db.rs`, next to `find_orphaned_parent_tasks`
  (`db.rs:6203`).
  - Add `pub struct PhantomTrackingTask { pub id: String, pub agent_id: String,
    pub label: String, pub status: String, pub created_at: String, pub updated_at:
    String }` matching the SELECT columns.
  - Add `pub fn find_phantom_tracking_tasks(&self, agent_id: &str, age_seconds: i64)
    -> Result<Vec<PhantomTrackingTask>>` with the SQL from §5 AC3. `age_seconds = 0`
    supports the AC5 startup-sweep call.
- File: `crates/mika-agent/src/async_db.rs`.
  - Add the async wrapper mirroring `find_orphaned_parent_tasks` pattern (grep
    `find_orphaned_parent_tasks` there for shape).
- Unit test: add `test_find_phantom_tracking_tasks_*` next to
  `test_find_orphaned_parent_tasks_*` at `db.rs:12894`. Cover: (a) matching
  status/action_type/pid/age; (b) mismatched action_type; (c) non-NULL pid; (d)
  terminal-status row; (e) age-guarded fresh row.

### Phase 3 — watchdog extension (AC3 + AC7 tick source)

- File: `crates/mika-common/src/config.rs`. Add
  `pub phantom_sweep_age_seconds: Option<u64>` to the `Settings` struct right after
  the existing `callback_watchdog_grace_period_secs: Option<u64>` at `config.rs:913`.
  Wire the env override `MIKA_PHANTOM_SWEEP_AGE_SECONDS` via the same `config-rs`
  prefix pattern the sibling field uses. Add
  `pub fn effective_phantom_sweep_age_seconds(&self) -> u64 { self.phantom_sweep_age_seconds.unwrap_or(3600) }`
  right after `effective_callback_watchdog_grace_period_secs()` at `config.rs:1318`.
  Extend the test at `config.rs:2396-2403` to cover both the None-default and the
  env-override path (mirrors the callback watchdog test).
- File: `crates/mika-agent/src/task_engine/engine.rs`.
  - Read the value from `self.dispatcher.settings.effective_phantom_sweep_age_seconds()`
    inside `sweep_null_pid_phantoms()` (matches the callback watchdog pattern at
    `engine.rs:494-498`). No new field on `TaskEngine`; the access is per-call and
    lifetime-scoped to the sweep pass. No hardcoded const in `engine.rs`.
  - Add method `async fn sweep_null_pid_phantoms(&self)` patterned on
    `reap_orphaned_parent_tasks` (`engine.rs:661`). Implements the per-row + per-pass
    telemetry described in §5 AC3 + AC7. SOLE-WRITER contract for `phantom_aged_out`
    documented at the fn head.
  - Call it from `tick()` immediately after `check_callback_process_liveness()`
    (`engine.rs:232`), inside the `if self.tick_count.is_multiple_of(...)` block that
    already gates the 60-tick cadence.
- File: `crates/mika-agent/CLAUDE.md` — add
  `MIKA_PHANTOM_SWEEP_AGE_SECONDS` to the "Optional (callback watchdog)" env-var
  section (next to `MIKA_CALLBACK_WATCHDOG_GRACE_PERIOD_SECS`).

### Phase 4 — startup sweep (AC5 + AC7 startup source)

- File: `crates/mika-agent/src/task_engine/engine.rs`.
  - Add a step 2b in `startup_recovery` (`engine.rs:87-166`) between the current step
    2 (orphan `in_progress` sweep, `engine.rs:100-121`) and step 3 (schedulable load,
    `engine.rs:124`). Reuse `find_phantom_tracking_tasks(agent_id, 0)`. Iterate,
    `update_task_failed(id, agent_id, "startup_sweep")`, emit `phantom_aged_out` audit
    event with `startup_sweep` reasoning, aggregate count, emit
    `phantom_sweep_complete` info line with `source="startup_sweep"` on `n > 0`.

### Phase 5 — regression test (AC6)

- File: new `crates/mika-agent/tests/eval/test_phantom_task_row_sweep.rs` implementing
  the four tests in §5 AC6.
- File: `crates/mika-agent/tests/eval.rs` — add `mod test_phantom_task_row_sweep;`
  near `mod test_phantom_retry_guard;` (line 62).

### Phase 6 — documentation

- Update `crates/mika-agent/CLAUDE.md` under "Post-restart safety check (#757)" or
  a new "Post-deploy telemetry" subsection: add "Signal O — phantom sweep telemetry"
  documenting how to read the `phantom_aged_out` audit-events surface + the
  `phantom_sweep_complete` log line + the mika#1934 escalation threshold.
- Cross-link in the PR body: mika#1712 (this PR) + mika#1934 (blocked-by) + sami
  bearing comment #5373564110.

## Contradictions with the original AC set (historical rationale for shape (b))

Preserved verbatim so the sami arbitration is traceable to the code-grounded findings
that motivated it.

**Divergence D1 (blocks AC1 as written — SUPERSEDED per sami §1).** AC1 reads: "Task
INSERT with `action_type='none'` allowed ONLY if `status='pending'` AND `next_fire_at`
is set". The only production callsite that writes `action_type='none'` is `create_task`
(`tools/create_task.rs:235-258`), which by design writes `next_fire_at = None` because
tracking tasks do not fire. Enforcing AC1 as a DB CHECK breaks every tracking-task
creation. Sami arbitration: **AC1 dropped** — inverse-good-direction to reject
legitimate writes.

**Divergence D2 (blocks AC2 as written — SUPERSEDED per sami §1).** AC2 reads: "Rows
that transition to `status='in_progress'` MUST have `action_type != 'none'` and
`process_id` set". Every tracking task transitioned to `in_progress` by the
`update_task_status` tool (`db.rs:5307`) has `action_type='none'` and no `process_id`
— this is the *documented* lifecycle of a manual tracking row, not a bug. There is no
"async callback path that legitimately leaves process_id NULL" in the current code —
the only NULL-PID `in_progress` shape is the tracking-task shape. Sami arbitration:
**AC2 dropped** — same reason as D1.

**Divergence D3 (blocks AC4 usefulness — SUPERSEDED per sami §1).** AC4 asks for a
`phantom_write` audit event on write. Given D1+D2, no writes are currently
phantom-by-design — they are tracking-by-design. `phantom_write` would fire on every
`create_task` call (~50-500x per day per agent), producing audit-log noise proportional
to normal usage. Sami arbitration: **AC4 dropped** — audit signal keyed to sweeps
(AC7) instead of normal writes, matching the actual anomaly boundary.

## Scope boundaries

- **In scope:** watchdog extension (AC3), startup sweep (AC5), integration tests
  (AC6), sweep telemetry (AC7), schema-version bump (marker only), new DB query +
  async wrapper, documentation of the telemetry surface.
- **Out of scope (per ticket + sami bearing):** WHY the tracking rows orphan
  mid-orchestration → mika#1934 (cause-racine investigation, blocked-by #1712, fed by
  AC7 telemetry).
- **Out of scope (shape (a)/(c) rejected):** AC1 DB CHECK, AC2 DB CHECK, AC4
  `phantom_write` audit kind, `create_task` semantic rewrite.
- **Not touched:** `dispatcher.rs`, `process_liveness.rs`, `reap_orphaned_parent_tasks`,
  `complete_parent_tasks_on_callback_success`, `create_task` tool logic.

## Test strategy

- **Unit (cargo test):**
  - `db.rs`: `find_phantom_tracking_tasks` — construct rows of every shape (matching
    and non-matching), assert exact set returned. Pattern:
    `test_find_orphaned_parent_tasks_*` at `db.rs:12894`.
  - `engine.rs`: mock the sweep logic without full tick, mirroring
    `test_expire_timed_out_tasks` at `engine.rs:1385`.
- **Integration (`cargo test -p mika-agent --test eval`):**
  - New `test_phantom_task_row_sweep.rs` with four scenarios per §5 AC6 (three
    primary + one verify-fix-load-bearing).
- **Migration test:** extend the migration-chain test at `db.rs:16389` to include the
  v45→v46 step (`assert_eq!(final_version, CURRENT_SCHEMA_VERSION)`).
- **Post-migration idempotency:** re-run `migrate_v45_to_v46` on a DB already at v46;
  assert no-op (the `if version >= 46 { return Ok(()); }` guard).

## Rollback / migration safety

- Schema bump v45→v46 is a marker with no DDL. Rollback = accept a DB at v46 while
  running v45 code: safe because v45 code never queries the marker.
- **Explicit rollback story (per mika-arch F7):** revert the PR. Already-swept rows
  remain `failed` — this is the terminal state and is idempotent (no data loss, no
  downstream side effects, the phantom rows were already invisible to dispatch).
  New phantoms resume accumulating from the moment of revert until the fix
  re-deploys. No data migration needed. Operator SQL, if desired to un-fail a
  false-positive row post-revert: `UPDATE tasks SET status='in_progress',
  result=NULL WHERE id=? AND status='failed'` — but the sweep already emitted the
  audit_event, which stays as historical record.
- Existing-phantom handling on upgrade: **AC5 startup sweep is the answer.** First
  boot of the new binary walks the phantom set (unlimited age at startup), transitions
  all to `failed` with `phantom_aged_out` audit + `startup_sweep` reasoning, emits
  aggregate info line with count. No operator SQL required.
- If the new sweep produces surprise transitions in production:
  - **Grace window is tunable** (per mika-arch F3) — set
    `MIKA_PHANTOM_SWEEP_AGE_SECONDS=7200` (or larger) to raise the threshold without
    a code roll.
  - **Full kill-switch** (deferred) — a `MIKA_PHANTOM_SWEEP_DISABLED=1` env that
    early-returns both AC3 and AC5. Not implemented in v1; added only if the tunable
    grace proves insufficient in production.

## Post-deploy verification

**Signal O — phantom sweep telemetry (this PR).** Documentation-only signal (no
runtime metric export in v1). Operator computes the rate signal by hand from the
grep + SQL surfaces below, feeding it into mika#1934's escalation policy.

1. `grep phantom_sweep_complete server.log | jq '{source, count, agent}'` — expect
   `source="startup_sweep"` on first restart after deploy with the backlog count;
   subsequent restarts show `count=0`. Steady state (post-backlog-drain):
   `source="watchdog_tick"` lines with `count=0` most ticks, occasional non-zero as
   the phantoms accumulate then get swept 60 min after their `updated_at`.
2. **Large-backlog watch** — `grep phantom_sweep_large_backlog server.log` should
   return zero lines after the first post-deploy startup. Any subsequent hit means
   a single sweep pass swept > 100 rows — anomalous state, feeds mika#1934
   immediately regardless of the moving-average threshold.
3. `SELECT COUNT(*) FROM audit_events WHERE tool_name='phantom_aged_out'` — total
   rows swept since deploy. Compare against
   `SELECT COUNT(*) FROM tasks WHERE action_type='none' AND process_id IS NULL AND status IN ('in_progress','blocked')`
   — should trend to 0.
4. **Rate signal for mika#1934** — the moving-average calculation and the p1
   escalation threshold live in **mika#1934's body** (per mika-arch F6). #1712's
   telemetry is the input; #1934's policy owns the transform + trigger.

**Config surface documented in Signal O:**

- `MIKA_PHANTOM_SWEEP_AGE_SECONDS` (default `3600`, per mika-arch F3) — grace
  window for AC3 watchdog sweep. Startup sweep (AC5) is unaffected (always
  age=0 at startup — the phantom outlived a prior process by definition).
