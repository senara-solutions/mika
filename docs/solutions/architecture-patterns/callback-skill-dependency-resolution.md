---
title: Callback turns must resolve skill dependencies via BFS
category: architecture-patterns
date: 2026-04-16
issue: 578
module: agent-core / skills
problem_type: logic_error
component: tooling
symptoms:
  - "Unknown tool: run_claude_pilot on callback turn retry"
  - "self-dev skill prompt references run_claude_pilot but the tool definition is missing from the LLM tool registry"
root_cause: logic_error
resolution_type: code_fix
severity: high
tags: [callback, skills, dependency-resolution, silent-mode, tool-registry, bfs]
---

# Callback turns must resolve skill dependencies via BFS

## Problem

After #567 loosened the silent-mode tool filter so callback turns could use exec/http handler tools, callback turns still produced `Unknown tool: run_claude_pilot` errors. The `self-dev` skill's prompt instructed the agent to call `run_claude_pilot`, but the tool definition was absent from the LLM's tool registry.

## Symptoms

- `Unknown tool: run_claude_pilot` on callback turn retry (task `84f9ec29`, mika#334)
- `self-dev` skill prompt visible in callback context but its dependency `claude-pilot` (which defines `run_claude_pilot`) excluded from tool definitions

## What Didn't Work

- The #567 fix (`callback_safe_skills()`) correctly preserved exec/http handler tools in callback turns, but it filtered only on `enabled && always_on` without resolving transitive dependencies. The `claude-pilot` skill has `always_on = false` — it is only reachable as a declared dependency of `self-dev`.

## Solution

Add BFS transitive dependency resolution to `callback_safe_skills()`, mirroring the algorithm in `match_skills()` (matcher.rs lines 60-76):

```rust
// crates/mika-agent/src/skills/mod.rs — callback_safe_skills()
pub fn callback_safe_skills(&self) -> Vec<&SkillEntry> {
    use std::collections::{HashSet, VecDeque};

    // Seed: enabled + always_on skills
    let mut included: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<usize> = VecDeque::new();

    for (i, entry) in self.skills.iter().enumerate() {
        if entry.enabled && entry.manifest.skill.always_on {
            included.insert(i);
            queue.push_back(i);
        }
    }

    // BFS transitive dependency resolution
    while let Some(idx) = queue.pop_front() {
        for dep_name in &self.skills[idx].manifest.skill.dependencies {
            if let Some(dep_idx) = self.skills.iter()
                .position(|e| e.manifest.skill.name.eq_ignore_ascii_case(dep_name))
            {
                if self.skills[dep_idx].enabled && !included.contains(&dep_idx) {
                    included.insert(dep_idx);
                    queue.push_back(dep_idx);
                }
            }
        }
    }

    // Collect in original order
    self.skills.iter().enumerate()
        .filter(|(i, _)| included.contains(i))
        .map(|(_, entry)| entry)
        .collect()
}
```

Key design decisions:
- `safe_always_on_skills()` intentionally does NOT get dependency resolution — autonomous triggers (heartbeat, reflection, reminder) must not pull in exec/http handler skills via dependencies
- Disabled dependencies break their sub-tree (consistent with `match_skills()`)
- The `HashSet<usize>` visited set prevents infinite loops on circular dependencies

## Why This Works

In conversation mode, `match_skills()` does BFS dependency resolution — `self-dev` (always_on) declares `dependencies = ["claude-pilot"]`, so `claude-pilot` gets pulled in with `MatchReason::Dependency`. Callback turns are continuations of conversation-mode tool calls, so they need the same tool access.

The original `callback_safe_skills()` from #567 only returned `enabled && always_on` skills (a flat filter). Skills like `claude-pilot` that have `always_on = false` but are dependencies of always-on skills were silently excluded. Adding BFS ensures the tool registry matches what the agent had when it originally initiated the long-running call.

## Prevention

- 7 regression tests cover: direct dependency resolution, transitive chains, disabled-dep subtree break, no-duplicate when dep is also always_on, circular dependency safety, unknown dep graceful skip, and a side-by-side comparison of `safe_always_on_skills()` vs `callback_safe_skills()`
- The exhaustive `match` on `SilentTrigger` in `run_silent_inner()` forces compile errors when new variants are added
- Four independent safety guards remain for callback turns: `validate_dispatch_readiness()` (#525), `is_task_context: true`, untrusted callback framing, and `is_callback_turn` flag

## Related Issues

- mika#567 — Callback exec handler tool availability (predecessor fix)
- mika#525 — Dispatch readiness guard (structural backstop)
- `docs/solutions/architecture-patterns/callback-exec-handler-tool-availability.md` — Predecessor doc
- `docs/solutions/architecture-patterns/callback-task-loop-prevention.md` — Loop prevention safety model
- `docs/solutions/architecture-patterns/skill-dependency-resolution-and-action-guard.md` — Origin of BFS algorithm
