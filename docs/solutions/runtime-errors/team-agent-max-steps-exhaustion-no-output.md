---
title: "Team Agent Max-Steps Exhaustion Produces No Useful Output"
date: 2026-03-04
category: runtime-errors
tags:
  - team-mode
  - agent-loop
  - tool-steps
  - continuation-turns
  - tracing
  - logging
  - error-handling
severity: high
components:
  - mika-agent/src/agent.rs
  - mika-agent/src/teams/engine.rs
  - mika-agent/src/teams/mod.rs
  - mika-cli/src/main.rs
symptoms:
  - "Team agent response is static string 'Agent exceeded maximum tool steps.' with no summary"
  - "Team logs mixed in global ~/.mika/logs/ instead of per-team directory"
  - "Agent names missing from log entries — multi-agent runs indistinguishable"
  - "Sentinel error string __conversational__: used for control flow"
root_causes:
  - "Hard-coded fallback string instead of continuation turn for team agents"
  - "Log directory hardcoded to global path without team name"
  - "TeamAgentParams missing agent_name field — no tracing span context"
  - "DecomposeResult encoded as error string instead of typed enum"
  - "Agent lacked high-level code analysis skill, burned steps on grep/find/sed"
related:
  - docs/solutions/runtime-errors/agent-max-steps-no-followup.md
  - docs/solutions/integration-issues/agent-team-management-tools-integration.md
  - docs/solutions/database-issues/team-graph-persistence-replacing-toml-history.md
  - docs/solutions/integration-issues/team-tui-mode-cli-integration.md
  - docs/adr/004-multi-agent-teams-orchestration.md
---

# Team Agent Max-Steps Exhaustion Produces No Useful Output

## Problem

During team run `a579e63f` with the `odds-engine` team, the `odds-engine-cto` agent
exhausted all 10 tool steps performing `run_shell` code investigation and never reached
`write_workspace`. The response stored in the team database was the hard-coded string
`"Agent exceeded maximum tool steps."` — no summary of what the agent discovered.

### Tool Call Trace

| Step | Command |
|------|---------|
| 1 | `find` — locate Python files |
| 2 | `grep` — aggregate_signals in orchestrator |
| 3 | `grep` — convergence thresholds in config.py |
| 4 | `sed` — read signal_orchestrator.py lines 90-210 |
| 5 | `grep` — trade_executor.py rejection paths |
| 6 | `grep` — SIGNAL_WEIGHTS in orchestrator |
| 7 | `git log --oneline -15` |
| 8 | `grep` — consensus_pct threshold logic |
| 9 | `find` — looking for log files |
| 10 | `find` — locate momentum_signal_generator.py |

The agent was still in research mode at step 10. It never transitioned to synthesis.

### Additional Issues Discovered

Investigation of the team run logs revealed three more problems:

1. **Wrong log directory** — Team mode logged to `~/.mika/logs/` (global) instead of
   `~/.mika/teams/{team}/logs/`.
2. **No agent attribution** — All team agents shared a single tracing subscriber with
   no agent name, making multi-agent log analysis impossible.
3. **Sentinel error string** — Conversational orchestrator replies were encoded as
   `bail!("__conversational__:{reply}")`, requiring callers to parse error messages
   for control flow.

## Root Cause Analysis

Two compounding factors caused the CTO to burn all steps:

1. **Tool-intensive investigation pattern.** The agent attempted to understand a Python
   signal orchestrator through sequential file exploration (`find` → `grep` → `sed` →
   `grep` ...). Each step required reading file listings, searching for patterns, and
   examining specific line ranges. This naturally consumes 10+ steps.

2. **Missing code analysis skill.** The agent lacked a high-level reasoning tool. Instead
   of delegating to an LLM-based tool (one call replaces 10+ shell commands), the agent
   fell back to low-level shell commands.

The hard-coded fallback string then discarded whatever partial understanding the agent
had accumulated over those 10 steps.

## Solution

Five fixes applied across the Mika codebase and the CTO agent's skill set.

### Fix 1: Team Log Directory

**File:** `crates/mika-cli/src/main.rs`

```rust
// Before
let log_dir = global_home.join("logs");

// After
let log_dir = team::team_dir(&global_home, &team_name).join("logs");
```

Each team now has isolated logs at `~/.mika/teams/{team_name}/logs/`. The `team_dir()`
helper already existed in `mika_common::team`.

### Fix 2: Per-Agent Tracing Spans

**File:** `crates/mika-agent/src/agent.rs`

Added `agent_name: &'a str` to `TeamAgentParams` and wrapped execution with a tracing
span using the `Instrument` trait:

```rust
async fn run_team_agent_inner(params: &TeamAgentParams<'_>) -> Result<Option<String>> {
    run_team_agent_inner_impl(params)
        .instrument(tracing::info_span!("team_agent", agent = %params.agent_name))
        .await
}
```

**Why `Instrument`, not `span.enter()`?** In async functions, `span.enter()` returns a
guard that exits the span on the first `.await` — not at function end. The `Instrument`
trait correctly re-enters the span after each await point. See the
[tracing docs on async](https://docs.rs/tracing/latest/tracing/span/struct.Span.html#in-asynchronous-code).

Log entries now carry the agent field:
```
team_agent{agent="odds-engine-cto"}: executing skill tool run_shell
team_agent{agent="research-lead"}: executing skill tool write_workspace
```

### Fix 3: Continuation Turn for Max-Steps-Exceeded

**File:** `crates/mika-agent/src/agent.rs`

Replaced the hard-coded string with the same continuation turn pattern used in the CLI
agent loop (see [agent-max-steps-no-followup.md](agent-max-steps-no-followup.md)):

```rust
if result.max_steps_exceeded {
    // Strip tools and thinking — force text-only response
    request.tools = None;
    request.thinking = None;
    request.messages.push(Message {
        role: "user".to_string(),
        content: MessageContent::Text(
            "[You ran out of tool steps. Summarize what you accomplished \
             and what remains undone. Be concise.]".to_string(),
        ),
    });

    let continuation = tokio::time::timeout(
        Duration::from_secs(CONTINUATION_TIMEOUT_SECS),  // 60s
        claude.send_message(&request),
    ).await;

    // Extract text or fall back to structured tool summary
    let text = match continuation {
        Ok(Ok(resp)) => {
            let t = resp.text();
            if t.is_empty() {
                format_step_exceeded_fallback(&result.tool_call_summaries)
            } else { t }
        }
        Ok(Err(e)) => {
            warn!(error = %e, "team agent continuation turn API error");
            format_step_exceeded_fallback(&result.tool_call_summaries)
        }
        Err(_) => {
            warn!("team agent continuation turn timed out");
            format_step_exceeded_fallback(&result.tool_call_summaries)
        }
    };
    return Ok(Some(text));
}
```

The `format_step_exceeded_fallback()` function shows the last 5 tool names with status,
giving the orchestrator enough context to re-assign or adjust.

### Fix 4: Typed DecomposeResult Enum

**File:** `crates/mika-agent/src/teams/engine.rs`

Replaced the sentinel error string with a proper enum:

```rust
pub enum DecomposeResult {
    Tasks(Vec<TaskAssignment>),
    Conversational(String),
}
```

Before:
```rust
// Callers parsed error messages
match self.decompose(goal).await {
    Err(e) if e.to_string().starts_with("__conversational__:") => {
        let reply = e.to_string().strip_prefix("__conversational__:").unwrap();
        // ...
    }
}
```

After:
```rust
match self.decompose(goal).await? {
    DecomposeResult::Tasks(tasks) => { /* execute tasks */ }
    DecomposeResult::Conversational(reply) => { /* deliver reply */ }
}
```

Type-safe, self-documenting, compiler-enforced exhaustive matching.

### Fix 5: ask-claude Skill for CTO Agent

**Location:** `~/.mika/agents/odds-engine-cto/skills/ask-claude/`

Created a skill that delegates code analysis to the Claude Code CLI. One `ask_claude`
call replaces 10+ grep/find/sed commands:

```sh
# Instead of 10 shell steps:
cd ~/workspace/senara-solutions/odds-engine && \
  claude -p "Analyze signal_orchestrator.py: what are the signal weights, \
  and why would aggregate_signals never produce COMBINED signals when only \
  one generator is active?" --output-format text
```

The skill is `always_on = true` with 120s timeout. The system prompt guides the agent:
- Use `ask_claude` for code analysis, debugging, investigation
- Keep `run_shell` for operational tasks: `make check`, `git status`, service health

## Verification

1. **Clippy + tests:** `cargo clippy --all-targets` clean, all 834 tests pass.
2. **Log isolation:** `mika --team odds-engine` → logs appear in
   `~/.mika/teams/odds-engine/logs/`, not `~/.mika/logs/`.
3. **Agent attribution:** `grep "team_agent{agent" ~/.mika/teams/odds-engine/logs/mika.log`
   shows CTO and Quant as distinguishable entries.
4. **Continuation turn:** Agents hitting max steps now produce a text summary instead of
   the static fallback string.

## Prevention

### Team Agent Prompt Design

- **Discourage shell exploration.** System prompts should say: "Use `ask_claude` for
  code analysis. Keep `run_shell` for simple checks (`make check`, `git status`)."
- **Budget tool steps.** Reserve 2-3 steps for `write_workspace` output. Don't spend
  all steps on research.
- **Prefer delegation.** For complex multi-file investigations, use `delegate_task` or
  specialized skills instead of manual grep/find/sed sequences.

### Monitoring

- Watch for agents that consistently hit max steps on the same goal types.
- Per-agent step counts are now visible in team logs via the `agent` tracing field.
- The continuation turn summary quality indicates whether the agent made meaningful
  progress or was stuck in a loop.

### Testing

- Test max-step scenarios by setting a low `MAX_TOOL_STEPS` and verifying the
  continuation turn produces a coherent summary.
- Verify that `format_step_exceeded_fallback()` correctly shows the last 5 tool names.
- Integration test: team run with an agent that needs > 10 steps should still produce
  a deliverable via the continuation turn.

## Related Documentation

- [Agent Max-Steps No Follow-Up](agent-max-steps-no-followup.md) — conversation mode
  fix (same continuation turn pattern, applied first)
- [Agent/Team Management Tools Integration](../integration-issues/agent-team-management-tools-integration.md) — per-tool timeout overrides (`run_team` 300s, `delegate_task` 120s)
- [Team Graph Persistence](../database-issues/team-graph-persistence-replacing-toml-history.md) — `team_runs`/`team_messages` SQLite schema where team agent output is stored
- [Team TUI Mode](../integration-issues/team-tui-mode-cli-integration.md) — how team
  progress streams to the TUI
- [ADR-004: Multi-Agent Teams Orchestration](../../adr/004-multi-agent-teams-orchestration.md) — foundational team architecture
