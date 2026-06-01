# Plan: mika#1363 — mika-dev auto-pull next groomed-not-ready ticket when queue empty

## Problem

mika-dev sits idle when no ticket has the `ready` label. The existing dispatch flow is purely webhook-driven: operator applies `ready` → webhook fires → mika-dev dispatches. When no `ready`-labelled tickets exist, mika-dev does nothing — even when groomed-but-not-ready tickets are available in the backlog.

Observed 2026-06-01: 91 open mika tickets, ~15 groomed-not-ready, 0 ready-labelled, mika-dev queue empty for 30+ minutes.

## Design

### Core idea

Add a periodic idle-queue probe to mika-dev's task engine that, when the queue is empty and no active dispatches exist, queries GitHub for the highest-priority groomed-not-ready ticket and applies the `ready` label to it. This reuses the entire existing webhook → dispatch pipeline naturally — no new dispatch machinery needed.

### Why label-application instead of direct dispatch

Applying `ready` is the canonical positive-consent signal (mika#841). The existing `self-dev-webhook-ready-label` handler already handles grooming checks, task creation, dispatch slot validation, and error recovery. Duplicating that logic in the engine would be fragile. Instead, the auto-pull mechanism is a **feeder** — it identifies the next ticket and applies the label; the webhook handler does the rest.

### Where it lives

The auto-pull logic is engine-level, running in the periodic DB scan cadence of `task_engine/engine.rs`. It's gated by:
1. Agent identity (only mika-dev, or agents with auto-pull enabled)
2. Env var `MIKA_DEV_AUTO_PULL` (default enabled; `0` or `false` to disable)
3. Queue emptiness (no active callbacks for any dispatch class)

## Implementation

### Step 1: Add `MIKA_DEV_AUTO_PULL` env var and config

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs` (new module)

Add a new module `auto_pull` under `task_engine/`. This keeps the feature self-contained and avoids bloating `engine.rs`.

```rust
/// Default interval in ticks (1 tick = 1 second). 600 ticks = 10 minutes.
const AUTO_PULL_INTERVAL_TICKS: u64 = 600;

/// Maximum consecutive failures before circuit-breaker trips for a ticket.
const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;

pub fn is_auto_pull_enabled() -> bool {
    let val = std::env::var("MIKA_DEV_AUTO_PULL")
        .unwrap_or_else(|_| "1".to_string())
        .to_lowercase();
    // Default enabled; only disabled on explicit "0" or "false"
    val != "0" && val != "false"
}
```

**Convention note:** Follows the same `std::env::var().unwrap_or_default().to_lowercase()` pattern as `MIKA_DISPATCH_BYPASS_GROOMING_CHECK` in `executor.rs`. Default is **enabled** (unlike the bypass which defaults disabled) because this feature is the intended steady-state behavior.

### Step 2: Add idle-queue detection

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs`

```rust
pub async fn is_queue_idle(db: &AsyncDatabase, agent_id: &str) -> bool {
    // Check both dispatch classes — implement and groom
    for class in &["implement", "groom"] {
        if db.has_any_active_callback_for_class(agent_id, class).await.unwrap_or(true) {
            return false;
        }
    }
    // Also check for any pending/in_progress manual tasks that haven't been dispatched yet
    // (tasks created but awaiting dispatch)
    !db.has_undispatched_active_tasks(agent_id).await.unwrap_or(true)
}
```

**New DB method:** `has_undispatched_active_tasks(agent_id)` — checks for manual tasks in `pending`/`in_progress` status without a callback child. This catches the gap between task creation and dispatch (e.g., a task created by the webhook handler but not yet dispatched).

**File:** `crates/mika-agent/src/db.rs`

```sql
SELECT EXISTS(
    SELECT 1 FROM tasks
    WHERE agent_id = ?
      AND trigger_type = 'manual'
      AND status IN ('pending', 'in_progress')
      AND source = 'self_dev'
      AND NOT EXISTS (
          SELECT 1 FROM tasks t2
          WHERE t2.parent_task_id = tasks.id
            AND t2.trigger_type = 'callback'
            AND t2.status IN ('pending', 'in_progress', 'completed')
      )
    LIMIT 1
)
```

### Step 3: GitHub candidate selection

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs`

The candidate selection runs `gh` CLI via a new helper (not the `run_gh` tool — this is engine-level, not agent-tool-level). Uses `tokio::process::Command` with the same env scrubbing as `run_gh`.

```rust
pub async fn find_best_candidate(
    github_token: &str,
    repo: &str,
    circuit_breaker: &CircuitBreakerState,
) -> Result<Option<AutoPullCandidate>> {
    // 1. List open issues (exclude ready-labelled)
    // gh issue list --repo <repo> --state open --json number,title,body,labels --limit 50
    
    // 2. Filter to groomed-not-ready:
    //    - body contains "Plan: docs/plans/" (grooming marker)
    //    - body contains "second-pass" or "(GROOMED)" (architect verdict)
    //    - labels do NOT contain "ready"
    //    - labels do NOT contain "blocked" or "needs-triage"
    
    // 3. Priority sort: p0 > p1 > p2 > p3 > unlabelled
    //    Within same priority: oldest first (lowest issue number)
    
    // 4. Skip circuit-broken tickets
    
    // 5. Return the top candidate
}
```

**Grooming marker detection reuses the same three-signal check as `validate_dispatch_readiness()` (executor.rs check 5):**
- `> - **Branch:**` — branch callout present
- `docs/plans/` — plan file reference
- `second-pass` marker (canonical `(GROOMED)` or spec-tolerated variants)

This ensures the auto-pull only selects tickets that would pass the dispatch-readiness gate.

**Priority label parsing:**
```rust
fn priority_rank(labels: &[String]) -> u8 {
    for label in labels {
        match label.as_str() {
            "p0-critical" => return 0,
            "p1-important" => return 1,
            "p2-normal" => return 2,
            "p3-low" => return 3,
            _ => {}
        }
    }
    4 // unlabelled = lowest priority
}
```

### Step 4: Circuit breaker state

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs`

In-memory state tracked on the `TaskEngine` struct (not persisted to DB — resets on restart, which is the desired behavior since restarts often fix transient issues).

```rust
pub struct CircuitBreakerState {
    /// issue_number -> consecutive failure count
    failures: HashMap<u64, u32>,
    /// issue_number -> timestamp when tripped
    tripped: HashMap<u64, String>,
}

impl CircuitBreakerState {
    pub fn record_failure(&mut self, issue_number: u64) -> bool {
        let count = self.failures.entry(issue_number).or_insert(0);
        *count += 1;
        if *count >= CIRCUIT_BREAKER_THRESHOLD {
            self.tripped.insert(issue_number, crate::timestamp::now());
            true // tripped
        } else {
            false
        }
    }
    
    pub fn record_success(&mut self, issue_number: u64) {
        self.failures.remove(&issue_number);
        self.tripped.remove(&issue_number);
    }
    
    pub fn is_tripped(&self, issue_number: u64) -> bool {
        self.tripped.contains_key(&issue_number)
    }
}
```

**How failures are detected:** The auto-pull mechanism applies `ready` and then monitors the resulting task. If the task ends in `failed` status (detected on the next auto-pull tick via a DB query for recently-failed tasks with `auto_pull` source metadata), the circuit breaker increments.

### Step 5: Apply `ready` label

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs`

```rust
pub async fn apply_ready_label(
    github_token: &str,
    repo: &str,
    issue_number: u64,
) -> Result<()> {
    // gh issue edit <n> --add-label ready --repo <repo>
    let output = tokio::process::Command::new("gh")
        .args(["issue", "edit", &issue_number.to_string(), "--add-label", "ready", "--repo", repo])
        .env("GH_TOKEN", github_token)
        .output()
        .await?;
    
    if !output.status.success() {
        anyhow::bail!("gh issue edit failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}
```

### Step 6: Wire into engine tick loop

**File:** `crates/mika-agent/src/task_engine/engine.rs`

Add `auto_pull_tick_count: u64` and `circuit_breaker: CircuitBreakerState` fields to `TaskEngine`.

In the periodic DB scan block (inside `tick()`, after `complete_parent_tasks_on_callback_success`):

```rust
// Auto-pull: check if queue is idle and pull next groomed-not-ready ticket
if self.tick_count.is_multiple_of(auto_pull::AUTO_PULL_INTERVAL_TICKS) {
    if !self.dispatcher.cli_mode {
        self.check_auto_pull().await;
    }
}
```

New method on `TaskEngine`:

```rust
async fn check_auto_pull(&mut self) {
    // 1. Check feature enabled
    if !auto_pull::is_auto_pull_enabled() {
        return;
    }
    
    // 2. Check agent identity — only well-known dev agents
    // Uses agent_id from dispatcher context
    let agent_id = &self.dispatcher.agent_id;
    if !auto_pull::is_auto_pull_agent(agent_id) {
        return;
    }
    
    // 3. Check queue idle
    if !auto_pull::is_queue_idle(&self.db, agent_id).await {
        trace!(agent_id, "auto_pull: queue not idle, skipping");
        return;
    }
    
    // 4. Get GitHub token
    let github_token = match auto_pull::get_github_token() {
        Some(t) => t,
        None => {
            warn!("auto_pull: no GitHub token configured, skipping");
            return;
        }
    };
    
    // 5. Find candidate
    let repo = "senara-solutions/mika"; // TODO: multi-repo support
    match auto_pull::find_best_candidate(&github_token, repo, &self.circuit_breaker).await {
        Ok(Some(candidate)) => {
            info!(
                issue_number = candidate.number,
                priority = ?candidate.priority,
                title = %candidate.title,
                "auto_pull: selected candidate"
            );
            
            // 6. Apply ready label
            match auto_pull::apply_ready_label(&github_token, repo, candidate.number).await {
                Ok(()) => {
                    info!(issue_number = candidate.number, "auto_pull: applied ready label");
                    
                    // 7. Audit event
                    if let Err(e) = self.db.log_audit_event(
                        agent_id,
                        &format!("system-{}", agent_id),
                        "task_engine_auto_pull",
                        &format!("https://github.com/{}/issues/{}", repo, candidate.number),
                        None, // before
                        Some("ready"), // after (label applied)
                        Some(&format!(
                            "auto_pull: selected #{} (priority: {}, reason: highest-priority groomed-not-ready ticket, queue idle)",
                            candidate.number, candidate.priority_label
                        )),
                        None, // trace_id
                    ).await {
                        warn!(error = %e, "auto_pull: failed to log audit event");
                    }
                }
                Err(e) => {
                    warn!(error = %e, issue_number = candidate.number, "auto_pull: failed to apply ready label");
                    self.circuit_breaker.record_failure(candidate.number);
                }
            }
        }
        Ok(None) => {
            debug!("auto_pull: no groomed-not-ready candidates found");
        }
        Err(e) => {
            warn!(error = %e, "auto_pull: candidate search failed");
        }
    }
}
```

### Step 7: Agent identity gate

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs`

```rust
/// Well-known agents eligible for auto-pull.
/// Currently only mika-dev. Extend as needed.
const AUTO_PULL_AGENTS: &[&str] = &["mika-dev"];

pub fn is_auto_pull_agent(agent_id: &str) -> bool {
    AUTO_PULL_AGENTS.contains(&agent_id)
}
```

### Step 8: Circuit breaker trip notification

When the circuit breaker trips (3 consecutive failures for a ticket), emit a structured log event and send a notification to the operator via `send_message`:

```rust
if self.circuit_breaker.record_failure(candidate.number) {
    warn!(
        issue_number = candidate.number,
        "auto_pull_circuit_breaker_tripped: ticket stuck after {} consecutive failures, needs operator review",
        CIRCUIT_BREAKER_THRESHOLD
    );
    // Audit the trip
    let _ = self.db.log_audit_event(
        agent_id,
        &format!("system-{}", agent_id),
        "task_engine_auto_pull",
        &format!("https://github.com/{}/issues/{}", repo, candidate.number),
        None,
        Some("circuit_breaker_tripped"),
        Some(&format!(
            "auto_pull: circuit breaker tripped for #{} after {} consecutive failures",
            candidate.number, CIRCUIT_BREAKER_THRESHOLD
        )),
        None,
    ).await;
}
```

### Step 9: Monitor dispatch outcomes for circuit breaker

The circuit breaker needs to detect when an auto-pulled ticket fails. Add metadata to the task created by the webhook handler to track auto-pull origin:

**File:** `crates/mika-agent/src/task_engine/auto_pull.rs`

After applying the `ready` label, store the issue number in an in-memory `pending_auto_pulls: HashSet<u64>`. On each auto-pull tick, also scan for recently failed tasks whose `reference_url` matches a pending auto-pull issue:

```rust
async fn check_auto_pull_outcomes(&mut self) {
    let agent_id = &self.dispatcher.agent_id;
    for issue_number in self.pending_auto_pulls.clone() {
        let ref_url = format!("https://github.com/senara-solutions/mika/issues/{}", issue_number);
        match self.db.get_task_by_reference_url(agent_id, &ref_url).await {
            Ok(Some(task)) if task.status == "completed" => {
                self.circuit_breaker.record_success(issue_number);
                self.pending_auto_pulls.remove(&issue_number);
            }
            Ok(Some(task)) if task.status == "failed" || task.status == "cancelled" => {
                let tripped = self.circuit_breaker.record_failure(issue_number);
                self.pending_auto_pulls.remove(&issue_number);
                if tripped {
                    // emit circuit breaker trip (see Step 8)
                }
            }
            _ => {} // still in progress or not found yet
        }
    }
}
```

### Step 10: Documentation

**File:** Root `CLAUDE.md` — add to Environment Variables section:

```
- `MIKA_DEV_AUTO_PULL` — Enable auto-pull of groomed-not-ready tickets when mika-dev's queue is idle (default: true). When `0` or `false`, mika-dev only dispatches on explicit `ready` label. When enabled, mika-dev checks every ~10 minutes for idle queue and applies `ready` to the highest-priority groomed-not-ready ticket.
```

**File:** `crates/mika-agent/CLAUDE.md` — add to Task Engine section after the orphaned parent reaper description:

```
**Auto-pull feeder (mika#1363):** `auto_pull.rs` — periodic idle-queue probe (every 600 ticks / ~10 min) that applies `ready` to the highest-priority groomed-not-ready ticket when mika-dev's dispatch queue is empty. Agent-gated (only `AUTO_PULL_AGENTS`), env-gated (`MIKA_DEV_AUTO_PULL`, default enabled). Candidate selection: `gh issue list` → filter for grooming markers (same 3-signal check as `validate_dispatch_readiness`) → priority sort (p0>p1>p2>p3>unlabelled) → oldest-first tiebreaker → circuit-breaker exclusion. Per-ticket circuit breaker (3-strike, in-memory) prevents stuck tickets from blocking the queue. Reuses the existing webhook → dispatch pipeline — no new dispatch machinery.
```

## File change summary

| File | Change |
|------|--------|
| `crates/mika-agent/src/task_engine/auto_pull.rs` | **NEW** — auto-pull module (Steps 1-5, 7-9) |
| `crates/mika-agent/src/task_engine/mod.rs` | Add `pub mod auto_pull;` |
| `crates/mika-agent/src/task_engine/engine.rs` | Add fields to `TaskEngine`, wire `check_auto_pull()` into tick loop (Step 6) |
| `crates/mika-agent/src/db.rs` | Add `has_undispatched_active_tasks()` and `get_task_by_reference_url()` queries (Step 2, 9) |
| `CLAUDE.md` (root) | Document `MIKA_DEV_AUTO_PULL` env var (Step 10) |
| `crates/mika-agent/CLAUDE.md` | Document auto-pull in Task Engine section (Step 10) |

## Testing

### Unit tests (in `auto_pull.rs`)

1. `test_is_auto_pull_enabled_default` — default returns true
2. `test_is_auto_pull_enabled_disabled` — `MIKA_DEV_AUTO_PULL=0` returns false
3. `test_is_auto_pull_enabled_false_string` — `MIKA_DEV_AUTO_PULL=false` returns false
4. `test_priority_rank_ordering` — p0 < p1 < p2 < p3 < unlabelled
5. `test_circuit_breaker_trips_at_threshold` — trips after 3 failures
6. `test_circuit_breaker_resets_on_success` — success clears failure count
7. `test_circuit_breaker_skips_tripped_tickets` — tripped tickets excluded from candidates
8. `test_grooming_marker_detection` — three-signal check matches groomed issues

### Integration tests (in `tests/eval/`)

9. `test_auto_pull_idle_queue_detection` — mock DB with no active callbacks returns idle
10. `test_auto_pull_busy_queue_detection` — mock DB with active callback returns not-idle

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| Auto-pull races with operator manually applying `ready` | The `ready` label webhook handler is idempotent — `create_task` deduplicates on `reference_url`. If operator applies `ready` first, auto-pull finds no candidates (no groomed-not-ready tickets without `ready`). |
| Auto-pull selects a ticket the operator doesn't want dispatched | Circuit breaker catches repeated failures. Operator can add `blocked` or `needs-triage` label to exclude from candidate pool. `MIKA_DEV_AUTO_PULL=0` disables entirely. |
| GitHub API rate limiting on `gh issue list` | 10-minute interval means ~6 API calls/hour. Well within rate limits. Errors are logged and skipped (fail-open, try again next tick). |
| Circuit breaker state lost on restart | Intentional — restart often fixes transient issues. Persistent circuit breaker would require DB state and adds complexity without clear benefit. |
| Multi-repo support | Initial implementation targets `senara-solutions/mika` only. The `repo` parameter is a constant that can be extended to a configurable list in a follow-up. |

## Out of scope

- **Multi-repo auto-pull** — targeting only `senara-solutions/mika` initially. Other repos can be added by extending `AUTO_PULL_AGENTS` and the repo selection logic.
- **Auto-pull for non-mika-dev agents** — gated by `AUTO_PULL_AGENTS` constant. Future agents opt in by adding to the list.
- **Configurable interval** — hardcoded 600 ticks (10 min). Add `MIKA_DEV_AUTO_PULL_INTERVAL_SECS` env var if tuning is needed.
- **Dashboard surface** — auto-pull events are visible via `audit_events` and structured logs. A dedicated dashboard widget is a follow-up.
