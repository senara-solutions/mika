# Plan: mika#980 — Pulse-check dispatch failure awareness

## Problem

`get_task_health_summary()` in `db.rs` detects 6 task-level anomaly types (stuck callbacks, failed recurring, long-running, stale blocked, stale pending, GitHub-linked) but none cross-check the `tool_calls` table for consecutive dispatch failures. When mika-dev's iteration loop is wedged (repeated `run_claude_pilot` calls with `success=0`), the task row stays `in_progress` and the pulse-check path sees no anomaly — producing confident "nothing stuck" answers.

The evidence: 6 consecutive `success=0` `run_claude_pilot` calls over 1h46m, mika-dev pulse "nothing stuck" 2 minutes before audit confirmed the wedge.

## Solution

Add a 7th anomaly type `dispatch_failures` to `get_task_health_summary()` that queries `tool_calls` for recent consecutive failures on long-running dispatch tools. When the failure count exceeds a threshold, the anomaly surfaces in the `<task-health>` block automatically — the existing prompt injection in `prompt.rs` (lines 950-964) renders all anomalies generically.

## Design Decisions

### D1: Query shape — tool_calls, not tasks

The existing 6 anomaly queries all use the `query_anomalies` helper which expects 6-column task-shaped rows. The new query operates on `tool_calls` (different table, different shape). Rather than force-fitting into `query_anomalies`, the new query is standalone with its own construction of `TaskHealthAnomaly`.

**Correlation to tasks (revised per mika-arch first-pass — `anomaly_task_correlation_principle`):** The query JOINs `tool_calls` → `sessions` (via `session_id`) → `tasks` (via `sessions.task_id`) to find the parent task. This is a direct session→task JOIN, not a heuristic "most recently updated in_progress task" lookup. The JOIN ensures the anomaly attaches to the *specific* task whose session produced the failures, not an arbitrary in_progress task that happens to exist. If the JOIN produces no result (e.g., tool_call session has no task linkage), the anomaly still surfaces with a descriptive standalone entry using the agent_id as identifier.

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

### D6: Time window aging defense (revised per mika-arch first-pass — `time_window_aging_antipattern`)

The 2h sliding window creates a silent-drop risk: if a wedge persists longer than 2h without new failure attempts (e.g., the agent stops retrying after repeated failures), all failure rows age out of the window and the anomaly disappears — the exact "nothing stuck" fabrication the ticket describes.

**Defense: dual-signal query.** The query checks TWO conditions (OR):
1. **Sliding window**: `tool_calls.success=0 AND created_at >= now - 2h` — catches active failure streams
2. **Stale-dispatch**: `tasks.status = 'in_progress'` AND the most recent `run_claude_pilot` tool call for this agent (regardless of success/failure) is older than 1h — catches wedges where the agent stopped retrying entirely

The stale-dispatch signal reuses the existing `LONG_RUNNING_DEFAULT_SECS` (1h) threshold. If the agent hasn't even *attempted* a dispatch in over 1h while a task sits `in_progress`, that's a wedge signal regardless of the tool_calls failure count.

This eliminates the aging antipattern: either the failures are recent (signal 1) or the silence itself is the signal (signal 2).

### D7: No system prompt changes needed

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

New query block as anomaly #7 (dual-signal, session→task JOIN):

```rust
// 7. Dispatch failures: dual-signal wedge detection
//    Signal A: >= THRESHOLD recent failures in the sliding window
//    Signal B: stale dispatch — no run_claude_pilot attempt in > 1h while task is in_progress
{
    let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len());
    if remaining > 0 {
        let window_start = timestamp::format(
            &(now - Duration::seconds(health_thresholds::DISPATCH_FAILURE_WINDOW_SECS)),
        );
        let stale_threshold = timestamp::format(
            &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
        );

        // Signal A: Count recent failures with session→task JOIN for correlation
        let signal_a: Option<(u32, Option<String>, Option<String>)> = self.conn.prepare(
            "SELECT 
                COUNT(*) as failure_count,
                t.id as task_id,
                t.label as task_label
             FROM tool_calls tc
             LEFT JOIN sessions s ON tc.session_id = s.id
             LEFT JOIN tasks t ON s.task_id = t.id AND t.status = 'in_progress'
             WHERE tc.agent_id = ?1
               AND tc.tool_name = 'run_claude_pilot'
               AND tc.success = 0
               AND tc.created_at >= ?2
             GROUP BY t.id
             ORDER BY failure_count DESC
             LIMIT 1"
        )?.query_row(
            rusqlite::params![agent_id, &window_start],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).ok();

        // Signal B: Stale dispatch — most recent run_claude_pilot attempt is older than 1h
        // while an in_progress task exists
        let signal_b: Option<(String, String)> = self.conn.prepare(
            "SELECT t.id, t.label
             FROM tasks t
             WHERE t.agent_id = ?1
               AND t.status = 'in_progress'
               AND t.trigger_type = 'manual'
               AND NOT EXISTS (
                   SELECT 1 FROM tool_calls tc2
                   WHERE tc2.agent_id = ?1
                     AND tc2.tool_name = 'run_claude_pilot'
                     AND tc2.created_at >= ?2
               )
             ORDER BY t.updated_at DESC
             LIMIT 1"
        )?.query_row(
            rusqlite::params![agent_id, &stale_threshold],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).ok();

        // Emit anomaly from Signal A (threshold met)
        if let Some((count, task_id, task_label)) = signal_a {
            if count >= health_thresholds::DISPATCH_FAILURE_THRESHOLD {
                let (tid, tlabel) = match (task_id, task_label) {
                    (Some(id), Some(label)) => (id, label),
                    _ => (agent_id.to_string(), "run_claude_pilot dispatch".to_string()),
                };
                anomalies.push(TaskHealthAnomaly {
                    task_id: tid,
                    label: tlabel,
                    trigger_type: "manual".to_string(),
                    status: "in_progress".to_string(),
                    anomaly_type: "dispatch_failures".to_string(),
                    age_description: format!("{} failures in last 2h", count),
                    reference_url: None,
                });
            }
        }

        // Emit anomaly from Signal B (stale dispatch) — only if Signal A didn't fire
        if anomalies.last().map_or(true, |a| a.anomaly_type != "dispatch_failures") {
            if let Some((task_id, task_label)) = signal_b {
                anomalies.push(TaskHealthAnomaly {
                    task_id,
                    label: task_label,
                    trigger_type: "manual".to_string(),
                    status: "in_progress".to_string(),
                    anomaly_type: "dispatch_stale".to_string(),
                    age_description: "no dispatch attempt in >1h".to_string(),
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
/// One of: "stuck_callback", "stale_blocked", "failed_recurring", "long_running", "github_linked", "dispatch_failures", "dispatch_stale"
```

### Step 4: Add unit tests

**File:** `crates/mika-agent/src/db.rs` (in the existing `#[cfg(test)] mod tests` block)

Test cases:
1. **Signal A — below threshold**: 2 recent `success=0` calls → no `dispatch_failures` anomaly
2. **Signal A — at threshold**: 3 recent `success=0` calls → `dispatch_failures` anomaly with correct count
3. **Signal A — above threshold**: 6 recent `success=0` calls → anomaly shows "6 failures in last 2h"
4. **Signal A — outside window**: 3 `success=0` calls older than 2h → no `dispatch_failures` anomaly
5. **Signal A — mixed**: successful calls interspersed with failures → counts only failures
6. **Signal A — task correlation via session→task JOIN**: failures in a session linked to an in_progress task → anomaly uses that task's id/label
7. **Signal B — stale dispatch**: in_progress task exists, no `run_claude_pilot` call in >1h → `dispatch_stale` anomaly
8. **Signal B — not stale**: in_progress task exists, recent `run_claude_pilot` call within 1h → no `dispatch_stale` anomaly
9. **Signal A+B mutual exclusion**: when Signal A fires, Signal B does NOT also fire (prevents double-anomaly)
10. **Signal B only**: failures aged out of 2h window BUT no recent dispatch attempt AND in_progress task → `dispatch_stale` fires (the aging defense case from mika-arch review)

### Step 5: Update CLAUDE.md anomaly documentation

**File:** `crates/mika-agent/CLAUDE.md`

In the "Silent Mode Agent Loop" section, update the task health awareness paragraph to mention the 7th anomaly type:
```
Anomaly types: `stuck_callback` (...), `dispatch_failures` (3+ recent `run_claude_pilot` failures in 2h window — wedged iteration loop detection, #980), `dispatch_stale` (in_progress task with no dispatch attempt in >1h — aging defense for wedges that stop retrying, #980).
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
| AC3: five consecutive pulse-checks during wedge all reflect the wedge | Dual-signal defense: Signal A (sliding window) catches active failure streams. Signal B (stale dispatch) catches wedges where failures aged out of the window — if no dispatch attempt in >1h while task is in_progress, `dispatch_stale` fires. Together, there is no time window where a wedged state produces a clean pulse. |

## Risk Assessment

**Low risk.** This is an additive query in an existing anomaly-detection function. No schema changes, no new tables, no prompt rewrites. The worst failure mode is the query returning an error, which is handled by the `Result` — the other 6 anomalies still surface. The `remaining > 0` check respects `MAX_ANOMALIES` cap.

## Out of Scope

- The underlying builtin-handler timeout that causes the iteration loop to wedge (separate engine concern)
- Pulse-check answers from other agents (mika-qa, mika-arch) — ticket is mika-dev-specific, but the fix is agent-generic by design (any agent with `run_claude_pilot` failures benefits)
- Auto-remediation (e.g., automatic server restart on wedge detection) — the anomaly surfaces the signal; the agent or operator decides the action
