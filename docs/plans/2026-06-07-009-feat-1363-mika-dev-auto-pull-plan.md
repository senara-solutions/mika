# Plan — feat(mika-dev): auto-pull next groomed-not-ready ticket (mika#1363)

## Phase 0 — Pin

**A. Recurring task substrate** (`crates/mika-agent/src/task_engine/mod.rs:35`):
```rust
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    label: &str,
    cron: &str,
    payload: &str,
) -> ...;
```
Already used for heartbeat (`server/mod.rs:1272`) and reflection (`server/mod.rs:1281`). Per-agent recurring tasks register at startup if missing.

**B. Heartbeat cron pattern** (`server/mod.rs:78-80`):
```rust
const HEARTBEAT_CRON: &str = "0 0 * * * *";  // every hour on the hour
```

**C. Task-engine tick loop** (`task_engine/engine.rs:42-49`):
- 1-second tick that fires tasks whose `next_fire_at <= now`
- Engine wrapped in `Arc<Mutex<TaskEngine>>`

**D. No existing auto-pull logic** — greenfield (per grep).

**E. Trigger payload pattern** — heartbeat fires `r#"{"trigger":"heartbeat"}"#`. A new auto-pull recurring task would fire `r#"{"trigger":"auto_pull_groomed"}"#`. The callback handler reads the trigger and routes.

## Hypothesis (committed)

mika-dev currently has zero auto-pull behavior. Adding the feature requires:
1. A new recurring-task cron at agent startup (e.g., every 10 minutes)
2. A new trigger-routing branch in the task callback (parses `auto_pull_groomed`)
3. A new task action that queries open groomed-not-ready tickets and applies `ready` to the highest-priority

The existing `ensure_recurring_task` substrate makes this clean.

## Approach (committed)

### A. Add AUTO_PULL_CRON const + register the recurring task

In `server/mod.rs` near `HEARTBEAT_CRON`:

```rust
/// Cron schedule for the auto-pull check: every 10 minutes.
/// Pre-filters (groomed-not-ready, priority ordering, queue-empty gate) applied at dispatch time.
const AUTO_PULL_CRON: &str = "0 */10 * * * *";
```

In the startup loop near `ensure_recurring_task(... "heartbeat" ...)`:

```rust
if let Ok(auto_pull_enabled) = std::env::var("MIKA_DEV_AUTO_PULL")
    && auto_pull_enabled == "0"
{
    info!(agent = %name, "auto_pull disabled via MIKA_DEV_AUTO_PULL=0");
} else if name == "mika-dev" {
    task_engine::ensure_recurring_task(
        &db,
        "auto_pull_groomed",
        AUTO_PULL_CRON,
        r#"{"trigger":"auto_pull_groomed"}"#,
    )
    .await;
}
```

(Gated to `name == "mika-dev"` so only that agent gets the task; gated on env var per AC4.)

### B. Trigger routing in callback handler

In the task callback path that handles `trigger: heartbeat | reflection`, add a `auto_pull_groomed` branch that fires the auto-pull logic.

### C. Auto-pull selection logic

```rust
async fn auto_pull_groomed_ticket(db: &AsyncDatabase, github_token: &str) -> Option<u64> {
    // 1. Queue-empty gate: count in_progress + pending long_running tasks
    let queue_count = db.count_active_self_dev_tasks().await?;
    if queue_count > 0 {
        return None; // not idle
    }

    // 2. Query open groomed-not-ready tickets
    let issues = gh::list_open_issues_without_label("ready").await?;

    // 3. Filter to groomed-only (body has Branch + Plan + GROOMED markers)
    let groomed: Vec<_> = issues.into_iter()
        .filter(|i| is_groomed(&i.body))
        .collect();

    // 4. Priority-rank: p0 > p1 > p2 > p3 > unlabelled
    let selected = groomed.into_iter()
        .max_by_key(|i| priority_rank(&i.labels))?;

    // 5. Apply ready label
    gh::apply_label(selected.number, "ready").await?;

    // 6. Audit-event the decision
    log_audit_event("auto_pull", &format!("selected #{} by priority {}", ...));

    Some(selected.number)
}

fn is_groomed(body: &str) -> bool {
    // Match canonical body callouts shape (from _write_canonical_callout)
    body.contains("> - **Branch:** `") && body.contains("> - **Plan:** `")
        && body.contains("GROOMED")
}
```

### D. Circuit-breaker (AC3)

Maintain a `auto_pull_failures` table or per-ticket metadata. Increment on cascade-failure detection:
- After auto-pull fires `ready`, the dispatch-pipeline runs. If parent task hits `failed` within N minutes, increment counter for that ticket.
- If counter reaches 3, skip the ticket on future auto-pulls AND emit a "ticket stuck" audit-event.

Simpler v1: store per-ticket counters in `task_metadata` or a new `auto_pull_stats` table.

### E. Operator override (AC4)

Environment variable `MIKA_DEV_AUTO_PULL=0` gates the recurring task registration at startup. If set, the task is not added; mika-dev returns to webhook-only behavior.

## Acceptance Criteria

1. **AC1:** mika-dev's task-engine has a 10-minute recurring task `auto_pull_groomed` registered at startup (verified by querying `recurring_tasks` table for agent=mika-dev label=auto_pull_groomed after server boot).

2. **AC2:** Auto-pull selection:
   - Filters to open issues without `ready` label
   - Selects highest-priority (p0 > p1 > p2 > p3 > unlabelled)
   - Among same-priority, prefers oldest (`updatedAt ASC`)
   - Applied `ready` label visible via `gh issue view <N> --json labels`

3. **AC3:** Per-ticket 3-strike circuit-breaker:
   - Cascade failure within 60min of auto-pull increments counter
   - Counter at 3 → ticket skipped on subsequent auto-pull AND audit-event emitted
   - Counter resets on operator-driven `ready` (not auto-pull)

4. **AC4:** `MIKA_DEV_AUTO_PULL=0` env var disables the feature (verified by `recurring_tasks` table NOT containing auto_pull_groomed row).

5. **AC5:** Audit-event entry per auto-pull decision:
   - `target_key: "auto_pull"`
   - `after_value` includes ticket number + selection reasoning
   - Both selection AND skip decisions logged

6. **AC6:** Tests:
   - `cargo test -p mika-agent` passes
   - New unit tests for `is_groomed(body)` and `priority_rank(labels)`
   - Integration test for the auto-pull selection logic against mock issue set

## Files

- `crates/mika-agent/src/server/mod.rs` — add `AUTO_PULL_CRON` + recurring-task registration (~line 1272 region)
- `crates/mika-agent/src/task_engine/handler.rs` (or wherever callback triggers route) — add `auto_pull_groomed` branch
- `crates/mika-agent/src/auto_pull.rs` — new module with selection + circuit-breaker logic
- `crates/mika-agent/src/db.rs` — `count_active_self_dev_tasks` query if not present
- `crates/mika-agent/tests/` — auto_pull integration test

## Out of scope

- Cross-repo auto-pull (mika-only for v1; mika#1382 covers cross-repo dispatch routing)
- Auto-pull for other agents (mika-arch, mika-qa) — separate concern
- "Auto-groom" path (auto-applying `ready` to ungroomed tickets) — that's `feedback_grooming_mode_pivot` territory and not safe during freeze
- Removing or modifying the webhook-driven dispatch path (this is additive)

## Risk

Medium.
- Selection logic could cascade-fail on a broken ticket (mitigated by AC3 circuit-breaker)
- Network/auth failures on `gh issue list` could log spam (mitigated by error-suppression + audit-event-only-on-success)
- Race condition: operator manually applies `ready` while auto-pull fires — both attempt dispatch (mitigated by `create_task` idempotency on `reference_url`)

## Test plan

1. Unit: `is_groomed(body)` returns true on canonical callout shape, false on prose
2. Unit: `priority_rank(labels)` orders correctly
3. Integration: with mock GH issues, verify auto-pull selects correctly
4. Integration: queue-empty gate works (active tasks block auto-pull)
5. Manual: `MIKA_DEV_AUTO_PULL=0` disables registration

## Implementation order

1. Add `AUTO_PULL_CRON` const and gated registration to `server/mod.rs`.
2. Add `is_groomed(body)` + `priority_rank(labels)` helpers in new `auto_pull.rs` module.
3. Add `count_active_self_dev_tasks` query if needed.
4. Implement `auto_pull_groomed_ticket()` orchestration.
5. Wire callback trigger routing.
6. Implement circuit-breaker (table + counter logic).
7. Unit tests.
8. Integration tests.
9. Manual smoke-test with `MIKA_DEV_AUTO_PULL=0`/`=1`.
