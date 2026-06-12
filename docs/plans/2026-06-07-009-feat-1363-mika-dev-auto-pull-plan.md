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
    // 1. Queue-empty gate (mika#1363 F2): exact predicate.
    // Count tasks for mika-dev that are currently active (i.e., the loop is
    // not idle). Includes:
    //   - status='in_progress' (currently running)
    //   - status='pending' (awaiting dispatch slot)
    // Restricted to source='self_dev' to exclude system-tasks (heartbeat,
    // reflection, recurring auto_pull itself). Excludes 'completed', 'failed',
    // 'blocked' — those are terminal.
    //
    // SQL (in db.rs::count_active_self_dev_tasks):
    //   SELECT COUNT(*) FROM tasks
    //   WHERE agent_id = 'mika-dev'
    //     AND source = 'self_dev'
    //     AND status IN ('in_progress', 'pending')
    let queue_count = db.count_active_self_dev_tasks().await?;
    if queue_count > 0 {
        return None; // not idle — webhook-driven dispatch already covering
    }

    // 2. Query open groomed-not-ready tickets (mika#1363 F4).
    // gh CLI has no negative label filter. Pattern: fetch all open + filter
    // client-side for absence of 'ready'.
    //
    // CLI: gh issue list --repo senara-solutions/mika --state open \
    //        --json number,body,labels,updatedAt --limit 100
    // Filter: keep only issues where labels[].name does NOT contain "ready"
    // (`!issue.labels.iter().any(|l| l.name == "ready")`).
    let all_open = gh::list_open_issues_with_labels().await?;
    let issues: Vec<_> = all_open.into_iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == "ready"))
        .collect();

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
    // mika#1363 F1: parse canonical callout block structurally. Matches the
    // exact grooming-history line shape emitted by _write_canonical_callout +
    // by the /mika-ask-arch by-hand workflow. The full regex anchors the
    // verdict keyword to the grooming-history callout — not any prose
    // "GROOMED" elsewhere in the body.
    //
    // Canonical shape (must all three present):
    //   > - **Branch:** `<branch>`
    //   > - **Plan:** `docs/plans/<file>.md` (committed on branch @ <sha>)
    //   > - **Grooming history:** <...> → second-pass (GROOMED) — session-id: <uuid>
    //
    // Regex: anchored multiline; matches the second-pass-GROOMED-callout shape.
    static GROOMING_HISTORY_RE: OnceLock<Regex> = OnceLock::new();
    let re = GROOMING_HISTORY_RE.get_or_init(|| {
        Regex::new(r"(?m)^> - \*\*Grooming history:\*\*.+second-pass \(GROOMED\)")
            .unwrap()
    });
    re.is_match(body)
        && body.contains("> - **Branch:** `")
        && body.contains("> - **Plan:** `docs/plans/")
}
```

### D. Circuit-breaker (AC3) — committed storage

New table `auto_pull_stats` (additive — no migration of existing rows):

```sql
CREATE TABLE IF NOT EXISTS auto_pull_stats (
    repo_full_name TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    failure_count INTEGER NOT NULL DEFAULT 0,
    last_auto_pull_at TEXT,
    last_failure_at TEXT,
    PRIMARY KEY (repo_full_name, issue_number)
);
```

Counter lifecycle:
- Increment when: parent task created by auto-pull (within 60min window) hits status='failed'
- Reset when: auto-pull label-apply succeeds (successful dispatch is a proxy for recovery; 'not auto-pull' in original AC distinguished manual vs automatic reset, not prohibiting reset-on-success)
- Skip threshold: `failure_count >= 3` → auto-pull skips this ticket
- Audit-event on skip: `target_key='auto_pull_skip'`, value cites `failure_count`

The new table keeps stats independent of the tasks table (which has per-task semantics, not per-issue). Single primary key on (repo, issue#) makes lookup O(log n).

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
   - Counter resets when auto-pull label-apply succeeds (successful dispatch is the recovery signal)

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
