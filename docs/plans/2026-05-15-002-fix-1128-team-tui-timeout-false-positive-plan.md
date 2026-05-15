# Fix: Team TUI surfaces "Agent timed out" after deliverables successfully landed

**Issue:** senara-solutions/mika#1128
**Type:** bug
**Severity:** UX / observability — work is correct, only the reporting is wrong

## Problem

After a `mika --team <name>` run completes successfully (specialist agents write outputs to workspace, orchestrator synthesizes a final deliverable), the TUI displays "Agent timed out while processing team task." The deliverables exist on disk ~5 minutes before the timeout message appears.

### Root cause

The deliver phase (`engine.rs:501-514`) calls `run_agent()` to have the writer/orchestrator synthesize a final deliverable. `run_agent()` delegates to `run_team_agent()` (`agent.rs:3666+`), which imposes a per-agent deadline of `TEAM_AGENT_TIMEOUT_SECS = 300s`. If this deadline fires, `run_team_agent` returns the hardcoded fallback string `"Agent timed out while processing team task."` (three sites: line 3857, 3922, 3970).

The problem: `deliver()` (`engine.rs:1359-1400`) treats whatever string `run_agent()` returns as the deliverable text — including the timeout fallback. It stores that fallback string as `self.run.deliverable`, persists it to the `deliverable.md` metadata file, and emits it as `TeamEvent::Deliverable`. The TUI then displays this timeout message as the run's result.

Meanwhile, specialist outputs and even the orchestrator's synthesis (written to workspace via `write_workspace` tool calls during the agent loop) are on disk and correct. The timeout occurs in the final LLM turn *after* `write_workspace` completed — the agent timed out producing the text-response-to-caller, not the actual deliverable.

### Why the deliver phase times out

The deliver agent receives a `build_deliverable_context` prompt containing all workspace outputs. For complex runs with multiple specialist outputs (the reproduction case had 3 files totaling ~52KB), the synthesis LLM call can take 3-5 minutes. Combined with the prelude context assembly, this exhausts the 300s per-agent budget.

## Fix approach

**Workspace-content fallback in `deliver()`** — when `run_agent()` returns the timeout fallback string, read the workspace files and construct a deliverable from them. This is graceful degradation: perfect LLM-synthesized deliverable when time permits, serviceable file-concatenation fallback when it doesn't.

### Changes

#### 1. Add timeout detection constant (`crates/mika-agent/src/agent.rs`)

Extract the hardcoded timeout fallback string into a `pub(crate) const`:

```rust
pub(crate) const TEAM_AGENT_TIMEOUT_FALLBACK: &str = "Agent timed out while processing team task.";
```

Replace all three inline uses (lines 3857, 3922, 3970) with this constant.

#### 2. Add workspace fallback in `deliver()` (`crates/mika-agent/src/teams/engine.rs`)

After `run_agent()` returns, check if the response equals `TEAM_AGENT_TIMEOUT_FALLBACK`. If so:

1. Log a structured warning: `deliver_timeout_workspace_fallback` with `agent_name`, `workspace_dir`, `trace_id`.
2. Read non-metadata files from the workspace directory (skip `.meta/`).
3. Concatenate them with filename headers into a fallback deliverable string.
4. If workspace has files, return the fallback. If workspace is empty, return the original timeout message (genuine failure).

```rust
async fn deliver(&self) -> Result<String> {
    // ... existing code through run_agent call ...
    let response = self.run_agent(&agent_name, "Produce the final deliverable.", &context).await?;

    // Workspace-content fallback when the deliver agent times out (#1128).
    // The specialist outputs are already on disk — use them rather than
    // surfacing a misleading "timed out" message.
    if response == crate::agent::TEAM_AGENT_TIMEOUT_FALLBACK {
        warn!(
            target: "mika::otel",
            agent = %agent_name,
            workspace = %self.workspace_dir.display(),
            "deliver agent timed out — falling back to workspace content"
        );
        if let Some(fallback) = self.read_workspace_fallback() {
            // Still persist the fallback to messages table
            // ... existing persistence code with fallback instead of response ...
            return Ok(fallback);
        }
        // Workspace empty — genuine failure, keep the timeout message
    }

    // ... existing persistence code ...
    Ok(response)
}
```

#### 3. Add `read_workspace_fallback()` method (`crates/mika-agent/src/teams/engine.rs`)

```rust
/// Read workspace files and format them as a fallback deliverable.
/// Returns `None` if the workspace contains no output files.
fn read_workspace_fallback(&self) -> Option<String> {
    let mut parts = Vec::new();
    let entries = std::fs::read_dir(&self.workspace_dir).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();
        // Skip .meta directory and non-files
        if path.is_dir() {
            continue;
        }
        let filename = path.file_name()?.to_string_lossy().to_string();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                parts.push(format!("## {}\n\n{}", filename, content));
            }
        }
    }

    if parts.is_empty() {
        return None;
    }

    let header = "# Team Deliverable (workspace fallback)\n\n\
        > The synthesis agent timed out, but specialist outputs were completed successfully.\n\
        > Below are the workspace outputs.\n\n";
    Some(format!("{}{}", header, parts.join("\n\n---\n\n")))
}
```

#### 4. Update TUI to recognize and style the fallback (`crates/mika-cli/src/tui/app.rs`)

No change needed — the TUI already displays whatever text `TeamEvent::Deliverable` carries. The fallback is proper markdown content, not a misleading error string.

#### 5. Add test (`crates/mika-agent/src/teams/engine.rs`)

Unit test for `read_workspace_fallback()`:
- Create a temp directory with sample output files
- Verify fallback includes all file contents with headers
- Verify `.meta/` subdirectory is skipped
- Verify empty workspace returns `None`

### What this does NOT change

- **Timeout values** — `TEAM_AGENT_TIMEOUT_SECS` (300s) and `TEAM_RUN_TIMEOUT_SECS` (900s) stay the same. The fix addresses the reporting, not the budget.
- **Specialist execution** — no changes to decompose, execute, or review phases.
- **Outer timeout** — the `tokio::time::timeout` wrapper in `execute()` (line 439) already has a reasonable message ("Check workspace for partial results").
- **Team notification** — `build_run_completion_message` in server mode follows the same `run.deliverable` path and benefits from the fix automatically.

## Testing

1. **Unit test:** `read_workspace_fallback` with populated workspace, empty workspace, workspace with `.meta/` only.
2. **Integration:** Manually run `mika --team <team>` with a goal that produces multiple specialist outputs. Verify the deliverable is displayed correctly.
3. **Regression:** Existing `cargo test -p mika-agent` passes (no team engine test regressions).

## Risk assessment

**Low risk.** The fix is a fallback path that only activates when the deliver agent times out. The happy path (agent completes within deadline) is unchanged. The fallback produces strictly more useful output than the current behavior (actual workspace content vs. a misleading error string).
