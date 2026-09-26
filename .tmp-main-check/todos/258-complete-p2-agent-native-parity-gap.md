---
status: complete
priority: p2
issue_id: 258
tags: [code-review, architecture, agent-native]
dependencies: []
---

# Agent-Native Parity Gap for Teams Feature

## Problem Statement

The teams feature is entirely CLI/TUI-only. The conversational agent has no tools to list, run, or inspect teams. No `run_team`, `list_teams`, or `team_status` tools exist. The system prompt has no team awareness. Users on Telegram/WhatsApp cannot trigger team workflows. Agent-native score: 0/8 capabilities accessible through the agent.

## Findings

- **Files:**
  - `crates/mika-agent/src/tools/mod.rs` (default_tools function)
  - `crates/mika-agent/src/prompt.rs` (system prompt assembly)
- The `default_tools()` function registers 8 tools (core memory, facts, search, reminders, etc.) but none related to teams
- The system prompt built by `build_system_prompt()` has no mention of teams or collaborative workflows
- Team execution is only accessible via CLI subcommands (`mika team run`, `mika team list`, etc.)
- Users interacting via Telegram/WhatsApp through the gateway have no way to trigger team workflows
- This violates the "conversation-first" design principle stated in the project overview

## Proposed Solutions

This is follow-up work that can be implemented incrementally:

1. **`run_team` tool:** Wraps `teams::run_team()`, accepts team name and optional goal override, returns the deliverable text as the tool result. The agent can then present results conversationally.

2. **`list_teams` tool:** Returns available team definitions from the teams directory. Alternatively, inject team names directly into the system prompt to avoid a tool call.

3. **System prompt awareness:** Add a section to `build_system_prompt()` that lists available teams with brief descriptions, so the agent knows when to suggest using them.

4. **`team_status` tool (optional):** Query status of recent team runs, useful for long-running teams.

```rust
// Example run_team tool
pub struct RunTeamTool;

#[async_trait]
impl Tool for RunTeamTool {
    fn name(&self) -> &str { "run_team" }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<String> {
        let team_name = input["team_name"].as_str().unwrap();
        let goal = input["goal"].as_str();
        let result = teams::run_team(team_name, goal, ctx).await?;
        Ok(result.deliverable)
    }
}
```

## Technical Details

- The `run_team` tool would need access to the teams directory path and agent configuration
- Team execution can be long-running (minutes), so the tool should work within the agent's 5-minute timeout or have special handling
- The tool context (`ToolContext`) may need extension to include team-related configuration
- Consider whether team runs should count against the agent's 10-tool-step limit or have their own budget
- Server mode (mika-spirit) would also benefit from team access via the `/message` endpoint

## Acceptance Criteria

- [ ] Agent can list available teams via tool or system prompt injection
- [ ] Agent can trigger a team run via the `run_team` tool
- [ ] Team results are returned as tool output for conversational presentation
- [ ] System prompt includes team awareness when teams are configured
- [ ] Telegram/WhatsApp users can trigger team workflows through conversation

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |
| 2026-02-25 | Approved during triage session. Status: pending -> ready |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- Mika design principle: "conversation-first AI executive assistant"
