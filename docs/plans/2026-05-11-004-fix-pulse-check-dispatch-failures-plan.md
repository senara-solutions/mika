# Plan: mika#980 — Pulse-check dispatch failure awareness

## Problem

`get_task_health_summary()` in `db.rs` detects 6 task-level anomaly types (stuck callbacks, failed recurring, long-running, stale blocked, stale pending, GitHub-linked) but none cross-check the `tool_calls` table for consecutive dispatch failures. When mika-dev's iteration loop is wedged (repeated `run_claude_pilot` calls with `success=0`), the task row stays `in_progress` and the pulse-check path sees no anomaly — producing confident "nothing stuck" answers.

The evidence: 6 consecutive `success=0` `run_claude_pilot` calls over 1h46m, mika-dev pulse "nothing stuck" 2 minutes before audit confirmed the wedge.

## Solution

Add a 7th anomaly type `dispatch_failures` to `get_task_health_summary()` that queries `tool_calls` for recent consecutive failures on long-running dispatch tools. When the failure count exceeds a threshold, the anomaly surfaces in the `<task-health>` block automatically — the existing prompt injection in `prompt.rs` (lines 950-964) renders all anomalies generically.

## Design Decisions

### D1: Query shape — tool_calls, not tasks

The existing 6 anomaly queries all use the `query_anomalies` helper which expects 6-column task-shaped rows. The new query operates on `tool_calls` (different table, different shape). Rather than force-fitting into `query_anomalies`, the new query is standalone with its own construction of `TaskHealthAnomaly`.

**Correlation to tasks:** The query JOINs `tool_calls` with `tasks` via `session_id → sessions.task_id` to find the parent task, so the anomaly attaches to the actual in_progress task (not a synthetic entry). If no task correlation exists, the anomaly still surfaces with a descriptive standalone entry.

### D2: Threshold — 3 failures in 2 hours

Per the ticket's detection rule: `recent_failures >= 3` within a 2-hour window. Both values are constants in `health_thresholds`:
- `DISPATCH_FAILURE_THRESHOLD: u32 = 3`
- `DISPATCH_FAILURE_WINDOW_SECS: i64 = 7200` (2 hours)

### D3: Tool scope — `run_claude_pilot` only

The ticket is specifically about `run_claude_pilot` failures causing false "nothing stuck" signals. The query filters on `tool_name = 'run_claude_pilot'`. If other long-running dispatch tools need coverage in the future, the filter can be expanded to a list.

### D4: Anomaly rendering

The anomaly uses existing `TaskHealthAnomaly` fields:
- `anomaly_type`: `"dispatch_failures"`
- `age_description`: `"N failures in last 2h"` (where N is the actual count)
- `task_id`/`label`/etc: from the correlated task, or synthetic if no task found

This renders in the prompt as:
```
- [dispatch_failures] <task_id>: <label> (N failures in last 2h)
```

The LLM will see both `long_running` (if the task exceeds 1h) AND `dispatch_failures` — complementary signals, not redundant.

### D5: No prompt.rs changes needed

The prompt injection code at `prompt.rs:950-964` iterates `health.anomalies` generically — any new `anomaly_type` value appears automatically. The `<task-health-instructions>` block already instructs the agent to "review each anomaly" and "notify the user with anomaly details." No prompt-level changes needed.

### D6: No system prompt changes needed

The ticket suggests a "system-prompt addition" as one option. The engine-level approach (anomaly injection via `<task-health>`) is strictly better: it's data-driven, version-controlled in Rust, and already tested. Adding free-text prompt rules for pulse-check behavior would be fragile (same failure mode family as the mika-arch persistence-meta hallucination).

## Implementation

### Step 1: Add threshold constants

**File:** `crates/mika-agent/src/task_engine/types.rs` (lines 34-49)

Add to `health_thresholds` module:
```rust
/// Minimum number of recent dispatch failures that constitutes a "wedged" anomaly.
pub const DISPATCH_FAILURE_THRESHOLD: u32 = 3;

/// Time window (seconds) in which dispatch failures are counted.
pub const DISPATCH_FAILURE_WINDOW_SECS: i64 = 7_200; // 2 hours
```

### Step 2: Add dispatch_failures anomaly query

**File:** `crates/mika-agent/src/db.rs` (after line 4871, before the `anomalies.truncate` at line 4875)

New query block as anomaly #7:

```rust
// 7. Dispatch failures: consecutive run_claude_pilot failures in the recent window
{
    let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len());
    if remaining > 0 {
        let window_start = timestamp::format(
            &(now - Duration::seconds(health_thresholds::DISPATCH_FAILURE_WINDOW_SECS)),
        );
        
        // Count recent failures and find the most recent associated task
        let mut stmt = self.conn.prepare(
            "SELECT 
                COUNT(*) as failure_count,
                MAX(tc.created_at) as latest_failure
             FROM tool_calls tc
             WHERE tc.agent_id = ?1
               AND tc.tool_name = 'run_claude_pilot'
               AND tc.success = 0
               AND tc.created_at >= ?2"
        )?;
        
        let result: Result<(u32, Option<String>)> = stmt.query_row(
            rusqlite::params![agent_id, &window_start],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(Into::into);
        
        if let Ok((failure_count, latest_failure)) = result {
            if failure_count >= health_thresholds::DISPATCH_FAILURE_THRESHOLD {
                // Try to correlate to an in_progress task
                let task_info: Option<(String, String)> = self.conn.prepare(
                    "SELECT t.id, t.label
                     FROM tasks t
                     WHERE t.agent_id = ?1
                       AND t.status = 'in_progress'
                       AND t.trigger_type = 'manual'
                     ORDER BY t.updated_at DESC
                     LIMIT 1"
                )?.query_row(
                    rusqlite::params![agent_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ).ok();
                
                let (task_id, label) = task_info.unwrap_or_else(|| {
                    (agent_id.to_string(), "run_claude_pilot dispatch".to_string())
                });
                
                anomalies.push(TaskHealthAnomaly {
                    task_id,
                    label,
                    trigger_type: "manual".to_string(),
                    status: "in_progress".to_string(),
                    anomaly_type: "dispatch_failures".to_string(),
                    age_description: format!("{} failures in last 2h", failure_count),
                    reference_url: None,
                });
            }
        }
    }
}
```

### Step 3: Update TaskHealthAnomaly doc comment

**File:** `crates/mika-agent/src/db.rs` (line 291)

Update the `anomaly_type` doc comment to include the new type:
```rust
/// One of: "stuck_callback", "stale_blocked", "failed_recurring", "long_running", "github_linked", "dispatch_failures"
```

### Step 4: Add unit tests

**File:** `crates/mika-agent/src/db.rs` (in the existing `#[cfg(test)] mod tests` block)

Test cases:
1. **Below threshold**: 2 recent `success=0` calls → no `dispatch_failures` anomaly
2. **At threshold**: 3 recent `success=0` calls → `dispatch_failures` anomaly appears with correct count
3. **Above threshold**: 6 recent `success=0` calls → anomaly shows "6 failures in last 2h"
4. **Outside window**: 3 `success=0` calls older than 2h → no anomaly
5. **Mixed**: successful calls interspersed with failures → counts only failures
6. **Task correlation**: with an in_progress task, anomaly attaches to that task's id/label

### Step 5: Update CLAUDE.md anomaly documentation

**File:** `crates/mika-agent/CLAUDE.md`

In the "Silent Mode Agent Loop" section, update the task health awareness paragraph to mention the 7th anomaly type:
```
Anomaly types: `stuck_callback` (...), `dispatch_failures` (3+ recent `run_claude_pilot` failures in 2h window — wedged iteration loop detection, #980).
```

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/task_engine/types.rs` | Add 2 threshold constants |
| `crates/mika-agent/src/db.rs` | Add anomaly #7 query block + update doc comment + add tests |
| `crates/mika-agent/CLAUDE.md` | Update anomaly type list |

## Acceptance Criteria Mapping

| AC | How Addressed |
|----|---------------|
| AC1: precondition check runs detection-rule SQL before composing queue-state answer | The anomaly query runs inside `get_task_health_summary()`, which is called on every heartbeat/callback/reminder turn. The `<task-health>` block is injected into the system prompt before the LLM composes any answer. |
| AC2: failure count and recommended action in pulse answer | The anomaly's `age_description` carries the failure count. The `<task-health-instructions>` already instruct the agent to "notify the user with anomaly details and suggest an action." |
| AC3: five consecutive pulse-checks during wedge all reflect the wedge | The query is stateless and re-runs on every turn. As long as the `tool_calls` rows exist and are within the 2h window, the anomaly will appear on every pulse-check. |

## Risk Assessment

**Low risk.** This is an additive query in an existing anomaly-detection function. No schema changes, no new tables, no prompt rewrites. The worst failure mode is the query returning an error, which is handled by the `Result` — the other 6 anomalies still surface. The `remaining > 0` check respects `MAX_ANOMALIES` cap.

## Out of Scope

- The underlying builtin-handler timeout that causes the iteration loop to wedge (separate engine concern)
- Pulse-check answers from other agents (mika-qa, mika-arch) — ticket is mika-dev-specific, but the fix is agent-generic by design (any agent with `run_claude_pilot` failures benefits)
- Auto-remediation (e.g., automatic server restart on wedge detection) — the anomaly surfaces the signal; the agent or operator decides the action
