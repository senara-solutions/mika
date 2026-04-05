---
status: complete
priority: p1
issue_id: "650"
tags: [code-review, security]
dependencies: []
---

# `run_gh`: `--repo` flag smuggling via command array

## Problem Statement

The `run_gh` builtin handler appends `--repo <value>` from the separate `repo` parameter, but does not check whether `--repo` or `-R` already appears in the `command` array. An agent (or prompt-injected input) could pass `["pr", "list", "--repo", "victim-org/private-repo"]` in the command array, bypassing the intended repo parameter.

The system prompt says "Do not include `--repo` in the command array", but this is a soft instruction the agent may ignore, and prompt injection could bypass it entirely.

The `gh` CLI uses the *last* `--repo` flag when duplicates are present, making behavior fragile and version-dependent.

## Findings

- **Agent-native reviewer**: Flagged as critical. The command array can smuggle `--repo` to target arbitrary repositories.
- **Security sentinel**: Noted that subsequent array elements after `command[0]` are passed verbatim with no validation beyond the subcommand allowlist.

## Proposed Solutions

### Solution 1: Reject `--repo` / `-R` in command array (Recommended)
Add validation that rejects command arrays containing `--repo` or `-R`:
```rust
if command.iter().any(|a| a == "--repo" || a == "-R") {
    return ToolOutput::error(
        "Do not include --repo in the command array. Use the separate 'repo' parameter instead."
            .to_string(),
    );
}
```
- **Pros**: Simple, explicit, prevents the attack vector entirely
- **Cons**: None significant
- **Effort**: Small
- **Risk**: Low

## Recommended Action

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/builtin_handlers.rs` (lines 286-294)
- **Components**: `run_gh` builtin handler

## Acceptance Criteria

- [ ] Command arrays containing `--repo` or `-R` are rejected with a clear error
- [ ] Test covers `--repo` smuggling attempt
- [ ] Test covers `-R` shorthand smuggling attempt

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-12 | Created from code review | Identified by agent-native and security reviewers |

## Resources

- Issue #119: Migrate `run_gh` to Rust builtin handler
