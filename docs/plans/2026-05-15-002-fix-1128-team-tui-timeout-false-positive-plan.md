# Fix: Team TUI surfaces "Agent timed out" after deliverables successfully landed

**Issue:** senara-solutions/mika#1128
**Type:** bug
**Severity:** UX / observability — work is correct, only the reporting is wrong

## Problem

After a `mika --team <name>` run completes successfully (specialist agents write outputs to workspace, orchestrator synthesizes a final deliverable), the TUI displays "Agent timed out while processing team task." The deliverables exist on disk ~5 minutes before the timeout message appears.

### Root cause

The deliver phase calls `run_agent()` to have the writer/orchestrator synthesize a final deliverable. `run_agent()` delegates to `run_team_agent()`, which imposes a per-agent deadline of `TEAM_AGENT_TIMEOUT_SECS = 300s`. When the deadline fires, `run_team_agent` returns the hardcoded fallback string from three sites:

| Pin | File | Line | Timeout path |
|-----|------|------|-------------|
| P1 | `crates/mika-agent/src/agent.rs` | 3857 | Prelude deadline exceeded before entering `run_loop` |
| P2 | `crates/mika-agent/src/agent.rs` | 3922 | Max-steps exceeded, deadline too close for continuation |
| P3 | `crates/mika-agent/src/agent.rs` | 3970 | `LoopResult::DeadlineExceeded` from `run_loop` |
| P4 | `crates/mika-agent/src/agent.rs` | 49 | `TEAM_AGENT_TIMEOUT_SECS` constant (300s) |
| P5 | `crates/mika-agent/src/teams/engine.rs` | 1359-1400 | `deliver()` method — receives timeout string as deliverable |
| P6 | `crates/mika-agent/src/teams/engine.rs` | 501-514 | `deliver_phase()` — stores and emits deliverable |
| P7 | `crates/mika-agent/src/teams/engine.rs` | 1445-1447 | `run_agent()` — `run_team_agent().unwrap_or_default()` |
| P8 | `crates/mika-agent/src/teams/engine.rs` | 29 | `TEAM_RUN_TIMEOUT_SECS` constant (900s) |
| P9 | `crates/mika-agent/src/teams/engine.rs` | 439-452 | Outer `tokio::time::timeout` wrapper |
| P10 | `crates/mika-cli/src/tui/app.rs` | 1184-1203 | TUI `TeamEvent::Deliverable` handler |
| P11 | `crates/mika-cli/src/commands/chat.rs` | 822-827 | TUI team task spawner — sends `TeamEvent::Deliverable(run.deliverable)` |
| P12 | `crates/mika-agent/src/agent.rs` | 3891-3908 | `LoopResult::Done` — successful path returning `Ok(text)` |

`deliver()` (P5) treats whatever string `run_agent()` returns as the deliverable text — including the timeout fallback. It stores that fallback string as `self.run.deliverable`, persists it to the `deliverable.md` metadata file, and emits it as `TeamEvent::Deliverable`. The TUI (P10) then displays this timeout message as the run's result.

Meanwhile, specialist outputs and the orchestrator's synthesis (written to workspace via `write_workspace` tool calls during the agent loop) are on disk and correct. The timeout occurs in the final LLM turn *after* `write_workspace` completed — the agent timed out producing the text-response-to-caller, not the actual deliverable.

### Why the deliver phase times out

The deliver agent receives a `build_deliverable_context` prompt containing all workspace outputs. For complex runs with multiple specialist outputs (the reproduction case had 3 files totaling ~52KB), the synthesis LLM call can take 3-5 minutes. Combined with the prelude context assembly, this exhausts the 300s per-agent budget.

### Design decision: timeout budget vs. workspace fallback

The deliver phase shares the same `TEAM_AGENT_TIMEOUT_SECS = 300s` budget as specialist agents (P4). Two options:

**Option A: Separate deliver timeout** — give the deliver phase its own constant (e.g., `TEAM_DELIVER_TIMEOUT_SECS = 600s`). Pros: addresses root cause. Cons: doesn't help when synthesis genuinely takes longer than any fixed budget (model latency varies); doesn't help with the outer 900s timeout; the operator still sees a misleading message on genuine timeout.

**Option B: Typed return + workspace fallback** — make `run_team_agent` return a typed enum that distinguishes timeout from success, then have `deliver()` fall back to workspace content on timeout. Pros: fixes the misleading message regardless of timeout budget; graceful degradation; the operator always sees the best available content. Cons: slightly larger change surface.

**Decision: Option B.** The workspace fallback addresses the symptom (misleading message) AND provides value beyond what a longer timeout could — even if we doubled the deliver budget, a timeout would still produce a misleading message. The fallback ensures that when work is complete on disk, the operator sees it. A separate deliver timeout constant is deferred — it can be added later if the fallback fires too often in practice.

## Fix approach

**Two-layer fix:** (1) typed return from `run_team_agent` to distinguish timeout from success, (2) workspace-content fallback in `deliver()` when the timeout variant is detected.

### Changes

#### 1. Add `TeamAgentOutcome` enum (`crates/mika-agent/src/agent.rs`)

Replace the current `Result<Option<String>>` return type with a typed enum that distinguishes timeout from success. This addresses mika-arch F2 (string-equality detection is fragile):

```rust
/// Outcome of a team agent run. Typed to distinguish timeout from success
/// so callers can make informed fallback decisions (#1128).
#[derive(Debug)]
pub(crate) enum TeamAgentOutcome {
    /// Agent completed and produced text (or None for tool-use-only turns).
    Done(Option<String>),
    /// Agent hit the per-agent deadline. The string describes which timeout path fired.
    TimedOut(String),
}
```

At P1 (line 3857), P2 (line 3922), P3 (line 3970): return `Ok(TeamAgentOutcome::TimedOut(...))` instead of `Ok(Some(fallback.to_string()))`.

At P12 (line 3891-3908): return `Ok(TeamAgentOutcome::Done(text))` instead of `Ok(text)`.

#### 2. Update `run_agent()` to propagate the typed outcome (`crates/mika-agent/src/teams/engine.rs`)

Change `run_agent()` (P7) to return `Result<TeamAgentOutcome>` instead of `Result<String>`. The `.unwrap_or_default()` at line 1447 becomes a match:

```rust
async fn run_agent(&self, agent_name: &str, task_message: &str, team_context: &str) -> Result<TeamAgentOutcome> {
    // ... existing setup ...
    Ok(crate::agent::run_team_agent(&params).await?)
}
```

Callers that currently use the string directly (`decompose`, `execute_task`, `review`, `deliver`) gain a match on `TeamAgentOutcome`. For all callers except `deliver`, the `TimedOut` variant extracts the description string (preserving current behavior). Only `deliver` takes the workspace fallback path.

#### 3. Add workspace fallback in `deliver()` (`crates/mika-agent/src/teams/engine.rs`)

After `run_agent()` returns, match on the outcome:

```rust
async fn deliver(&self) -> Result<String> {
    // ... existing writer/agent_name resolution and context building ...

    let outcome = self.run_agent(&agent_name, "Produce the final deliverable.", &context).await?;

    let response = match outcome {
        TeamAgentOutcome::Done(text) => text.unwrap_or_default(),
        TeamAgentOutcome::TimedOut(reason) => {
            warn!(
                target: "mika::otel",
                agent = %agent_name,
                workspace = %self.workspace_dir.display(),
                reason = %reason,
                "deliver agent timed out — falling back to workspace content"
            );
            // Workspace-content fallback (#1128): specialist outputs are already
            // on disk — use them rather than surfacing a misleading timeout message.
            self.read_workspace_fallback()
                .unwrap_or_else(|| reason)
        }
    };

    // ... existing persistence code using `response` ...
    Ok(response)
}
```

#### 4. Add `read_workspace_fallback()` method (`crates/mika-agent/src/teams/engine.rs`)

```rust
/// Read workspace files and format them as a fallback deliverable.
/// Returns `None` if the workspace contains no non-metadata output files.
fn read_workspace_fallback(&self) -> Option<String> {
    let mut parts: Vec<(std::time::SystemTime, String)> = Vec::new();
    let entries = std::fs::read_dir(&self.workspace_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        // Skip .meta directory and non-files
        if path.is_dir() {
            continue;
        }
        let filename = path.file_name()?.to_string_lossy().to_string();
        let mtime = entry.metadata().ok()?.modified().ok()?;
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                parts.push((mtime, format!("## {}\n\n{}", filename, content)));
            }
        }
    }

    if parts.is_empty() {
        return None;
    }

    // Sort by modification time so specialist outputs appear in execution order.
    parts.sort_by_key(|(mtime, _)| *mtime);

    let header = "# Team Deliverable (workspace fallback)\n\n\
        > The synthesis agent timed out, but specialist outputs were completed successfully.\n\
        > Below are the workspace outputs in execution order.\n\n";
    let body: Vec<&str> = parts.iter().map(|(_, text)| text.as_str()).collect();
    Some(format!("{}{}", header, body.join("\n\n---\n\n")))
}
```

#### 5. Update other `run_agent()` callers

Other callers of `run_agent()` — `decompose()`, `execute_task()`, `review()` — currently use the string directly. Update them to extract the text from `TeamAgentOutcome`, preserving current behavior:

```rust
// In decompose(), execute_task(), review():
let outcome = self.run_agent(&agent_name, task_message, &context).await?;
let response = match outcome {
    TeamAgentOutcome::Done(text) => text.unwrap_or_default(),
    TeamAgentOutcome::TimedOut(reason) => reason,
};
```

These callers keep the timeout string as their response — the workspace fallback is specific to `deliver()` where the distinction matters most.

#### 6. Add tests (`crates/mika-agent/src/teams/engine.rs`)

Tests for `read_workspace_fallback()`:

1. **Populated workspace** — create temp dir with `cto-review.md` and `quant-review.md`, verify fallback includes both with headers sorted by mtime.
2. **Empty workspace** — create empty temp dir, verify returns `None`.
3. **Only .meta directory** — create temp dir with `.meta/` subdir containing files, verify returns `None` (directories skipped).
4. **Mixed content** — workspace with files and `.meta/` dir, verify only files appear in output.

### What this does NOT change

- **Timeout values** — `TEAM_AGENT_TIMEOUT_SECS` (300s, P4) and `TEAM_RUN_TIMEOUT_SECS` (900s, P8) stay the same. A separate deliver timeout constant is deferred — add it if the fallback fires too often.
- **Specialist execution** — no changes to decompose, execute, or review phases (they keep the timeout string as-is).
- **Outer timeout** — the `tokio::time::timeout` wrapper (P9, line 439) already has a reasonable message ("Check workspace for partial results"). That path returns `Err(anyhow)` which sets `RunStatus::Failed`, not `RunStatus::Completed` — a different code path from the per-agent timeout.
- **Team notification** — `build_run_completion_message` in server mode follows the same `run.deliverable` path and benefits from the fix automatically.
- **`child_task_id` updates** — the three timeout sites (P1-P3) still update the child task's result via `update_task_completed`. This is correct — the task engine records the timeout, while the deliverable now falls back to workspace content.

## Testing

1. **Unit test:** `read_workspace_fallback` with populated workspace, empty workspace, workspace with `.meta/` only, mixed content.
2. **Type-check:** `cargo build` verifies all `run_agent()` callers handle `TeamAgentOutcome` exhaustively (compiler-enforced — no `#[non_exhaustive]`).
3. **Integration:** Manually run `mika --team <team>` with a goal that produces multiple specialist outputs. Verify the deliverable is displayed correctly.
4. **Regression:** `cargo test -p mika-agent` passes (no team engine test regressions).

## Risk assessment

**Low risk.** The typed return change is mechanically straightforward — `run_team_agent` already has three distinct timeout paths and one success path; the enum makes this explicit. The workspace fallback only activates when the deliver agent times out. The happy path (agent completes within deadline) is unchanged. The fallback produces strictly more useful output than the current behavior (actual workspace content vs. a misleading error string).
