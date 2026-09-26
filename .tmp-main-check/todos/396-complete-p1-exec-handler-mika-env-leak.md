---
status: complete
priority: p1
issue_id: "396"
tags: [code-review, security, marketplace, pr-56]
dependencies: []
---

# Exec handler subprocess does NOT scrub MIKA_* environment variables

## Problem Statement

The `execute_exec` function in `executor.rs` only strips `TMUX` and `TMUX_PANE` from child processes. It does **not** scrub `MIKA_*` environment variables. This means when a marketplace-installed skill with an exec handler runs, the handler script inherits `MIKA_ANTHROPIC_API_KEY`, `MIKA_OPENAI_API_KEY`, `MIKA_INTERNAL_TOKEN`, `MIKA_BRAVE_API_KEY`, and any other secrets.

While this is a pre-existing issue, PR #56 materially amplifies the risk because it enables installation of **untrusted third-party exec handlers** from arbitrary Git repositories. Before this PR, exec handlers were bundled (trusted) or manually created. Now a malicious repo author can publish a skill that exfiltrates secrets.

The `git_command()` in `git.rs` correctly scrubs MIKA_* vars, proving the codebase is aware of this pattern.

## Findings

- **Source**: security-sentinel agent, learnings-researcher agent
- **File**: `crates/mika-agent/src/skills/executor.rs` (lines 247-266)
- **Evidence**: `git_command()` in `git.rs` scrubs MIKA_* but `execute_exec` does not
- **CLAUDE.md states**: "Shell-exec and github handlers `unset` MIKA_* env vars before executing commands (defense-in-depth)" — this is done in shell scripts but marketplace handlers have no obligation to do so

## Proposed Solutions

### Option A: Add MIKA_* env scrubbing to execute_exec (Recommended)

Add the same loop from `git_command()` to the `Command` builder in `execute_exec`:

```rust
// Scrub MIKA_* env vars (defense-in-depth)
for (key, _) in std::env::vars() {
    if key.starts_with("MIKA_") {
        cmd.env_remove(&key);
    }
}
```

- Pros: Simple, follows existing pattern, defense-in-depth
- Cons: None — this is the standard pattern already used elsewhere
- Effort: Small (5 lines)
- Risk: Low

## Recommended Action

Option A. Must fix before merge.

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/executor.rs`
- **Components**: Skill executor, subprocess spawning

## Acceptance Criteria

- [ ] `execute_exec` scrubs all `MIKA_*` env vars before spawning child processes
- [ ] Existing handler tests still pass
- [ ] Pattern matches `git_command()` in `git.rs`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-03 | Created from PR #56 code review | Pre-existing issue amplified by marketplace |

## Resources

- PR #56: feat: add git-based skills marketplace
- `crates/mika-agent/src/skills/git.rs:111-122` — reference pattern
- `crates/mika-agent/src/skills/executor.rs:247-266` — affected code
