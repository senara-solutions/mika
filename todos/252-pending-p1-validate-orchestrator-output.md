---
status: pending
priority: p1
issue_id: 252
tags: [code-review, security, architecture]
dependencies: []
---

# Validate Orchestrator Output

## Problem Statement

The orchestrator's JSON task assignments are not validated. Agent names from LLM output are not checked against the team definition (unknown agents receive a default role of "specialist" then fail at runtime in `execute_tasks`). Output files are not validated for path traversal. Task descriptions have no length limits.

## Findings

- **File:** `crates/mika-agent/src/teams/engine.rs` lines 359-401
- **Severity:** P1 (Critical)
- **PR:** [#13](https://github.com/senara-solutions/mika/pull/13)

The `parse_task_assignments()` function deserializes the orchestrator's JSON output into task structures but performs no validation on the contents:

1. **Unknown agent names:** If the LLM hallucinates an agent name not in the team definition, it gets assigned a default "specialist" role. This leads to confusing runtime failures in `execute_tasks` or, worse, execution with incorrect capabilities/permissions.

2. **Output file path traversal:** The `output_file` field from task assignments is used without path validation. An LLM-generated or prompt-injected `output_file` like `../../etc/cron.d/backdoor` could write outside the intended output directory.

3. **Unbounded task descriptions:** Task descriptions come directly from LLM output with no length limit. An adversarial or confused orchestrator could produce extremely long task descriptions that consume excessive tokens or memory in downstream agent calls.

## Proposed Solutions

In `parse_task_assignments()`, add three validation steps:

1. **Reject unknown agent names:**
```rust
let known_agents: HashSet<&str> = team.agents.iter().map(|a| a.name.as_str()).collect();
tasks.retain(|task| {
    if !known_agents.contains(task.agent.as_str()) {
        tracing::warn!(agent = %task.agent, "Skipping task assigned to unknown agent");
        false
    } else {
        true
    }
});
```

2. **Validate output_file paths:**
```rust
if let Some(ref output_file) = task.output_file {
    if output_file.contains("..") || output_file.starts_with('/') {
        tracing::warn!(output_file = %output_file, "Rejecting task with invalid output_file path");
        continue;
    }
}
```

3. **Enforce length limit on task descriptions:**
```rust
const MAX_TASK_DESCRIPTION_LEN: usize = 5000;
if task.description.len() > MAX_TASK_DESCRIPTION_LEN {
    tracing::warn!(
        len = task.description.len(),
        "Rejecting task with oversized description"
    );
    continue;
}
```

## Technical Details

- The orchestrator is an LLM that produces structured JSON; its output should be treated as untrusted input
- Unknown agent names currently fall through to a default role assignment, masking the error
- Output file validation should use the same `Path::components()` approach recommended in issue #249 for consistency
- The 5000 character limit for task descriptions is a reasonable default; it should be configurable if needed
- All rejected tasks should be logged at `warn!` level for observability

## Acceptance Criteria

- [ ] Tasks assigned to agent names not in the team definition are skipped with a `warn!` log
- [ ] `output_file` values containing `..` or starting with `/` are rejected
- [ ] Task descriptions exceeding 5000 characters are rejected
- [ ] Unit test: task with unknown agent name is skipped
- [ ] Unit test: task with `output_file` containing path traversal is rejected
- [ ] Unit test: task with oversized description is rejected
- [ ] Valid tasks continue to be processed normally

## Work Log

- 2026-02-25: Finding identified during code review of PR #13

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- OWASP Input Validation: https://cheatsheetseries.owasp.org/cheatsheets/Input_Validation_Cheat_Sheet.html
