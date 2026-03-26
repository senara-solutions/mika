---
title: "Conditional required_tools enforcement via match reason tracking"
category: architecture-patterns
date: 2026-03-26
severity: high
tags: [skills, required_tools, always_on, match_reason, mika_ask, callback]
issue: "#265"
modules: [skills/matcher.rs, agent.rs, db.rs, commands/ask.rs, skills/index.rs]
---

# Conditional required_tools enforcement via match reason tracking

## Problem

When using `mika ask` for code implementation tasks, the agent announced it would implement but never called `run_claude_pilot` — only performed reconnaissance (reading files, creating work items). The session ended without commits or actual implementation.

Three root causes combined:

1. **No `required_tools` on self-dev skill** — The self-dev skill had no `[constraints]` section, so the required_tools enforcement gate (#270) couldn't fire.
2. **`always_on` poisoned `required_tools`** — Self-dev is `always_on = true`. Naively adding `required_tools = ["run_claude_pilot"]` would enforce it on EVERY message (even "what's the weather?") because the engine had no concept of WHY a skill matched.
3. **`mika ask` is one-shot** — No `TaskEngine` runs, so even if `run_claude_pilot` IS called and spawns a background process, the callback sits orphaned until TUI or server starts. No user indication that work is in progress.

## Root Cause

The `match_skills()` function returned a flat `Vec<&SkillEntry>` with no metadata about match provenance. `collect_required_tools()` collected constraints from ALL matched skills indiscriminately. This meant `always_on` skills couldn't declare `required_tools` without enforcing them on every single message — a semantic mismatch, since the skill's constraints should only apply when the user's message actually triggers the skill's purpose.

## Solution

### 1. Match reason tracking in `skills/matcher.rs`

Added `MatchReason` enum (`AlwaysOn`, `Keyword`, `Dependency`) and `MatchedSkill<'a>` wrapper struct. Changed `match_skills()` to return `Vec<MatchedSkill<'a>>` instead of `Vec<&'a SkillEntry>`. Key precedence rule: if a skill is both `always_on` and has a keyword hit, the reason is `Keyword` (more specific match wins).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    AlwaysOn,   // Matched because always_on=true, no keyword hit
    Keyword,    // Matched via keyword (even if also always_on)
    Dependency, // Pulled in via BFS dependency resolution
}

pub struct MatchedSkill<'a> {
    pub entry: &'a SkillEntry,
    pub reason: MatchReason,
}
```

### 2. Conditional enforcement in `agent.rs`

`collect_required_tools()` now filters to `MatchReason::Keyword` only:

```rust
fn collect_required_tools(matched: &[MatchedSkill<'_>]) -> HashSet<String> {
    matched
        .iter()
        .filter(|m| m.reason == MatchReason::Keyword)
        .flat_map(|m| m.entry.manifest.constraints.required_tools.iter())
        .cloned()
        .collect()
}
```

At call sites, existing functions that don't need `MatchReason` receive extracted `Vec<&SkillEntry>` via `matched.iter().map(|m| m.entry).collect()`. This avoids changing signatures of functions that don't care about match reason.

### 3. Pending callback awareness in `mika ask`

Added `get_pending_callbacks_for_session()` DB query. After the agent loop, `ask.rs` queries for pending callback tasks and prints a stderr notice. JSON format includes optional `pending_tasks` array (omitted when empty via `skip_serializing_if`).

### 4. Validation warning in `validate_skill()`

Warns if an `always_on` skill with no keywords declares `required_tools` — those constraints will never be enforced.

## Key Decisions

- **Keyword wins over AlwaysOn**: If a skill is both `always_on=true` and its keyword matches the message, the reason is `Keyword`. This ensures constraints are enforced when the user's message clearly triggers the skill's purpose.
- **Dependencies don't enforce constraints**: Skills pulled in via BFS dependency resolution are tagged `Dependency` and excluded from constraint enforcement. Only directly-triggered skills contribute.
- **Adapter pattern at call sites**: Rather than refactoring every downstream function to accept `MatchedSkill`, we extract `&[&SkillEntry]` at the call site. Only `collect_required_tools` sees the wrapper. Minimizes churn.
- **Stderr for notice, not exit code**: The pending callback notice goes to stderr (stdout stays clean for piping). Exit code remains 0.

## Prevention

1. **When adding a new dimension to matched skills**, track it in the match output rather than re-scanning. The `MatchReason` pattern is extensible.
2. **When adding `required_tools` to an `always_on` skill**, ensure the skill also has keywords — otherwise constraints will never fire. The validation diagnostic catches this.
3. **When adding one-shot CLI commands that invoke the agent loop**, check whether long-running/callback tasks can be spawned and whether there's a consumer. Print a notice if not.
4. **DB column names**: The tasks table uses `created_by_session` (not `session_id`). Always verify column names against the schema before writing queries.

## Related

- Issue: #265 (mika announces implementation but doesn't execute)
- Issue: #270 (required-tools enforcement gate)
- `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md` — The engine-level enforcement mechanism this builds on
- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — Three-layer defense pattern (code guard > core memory > prompt)
- `docs/solutions/logic-errors/callback-processing-race-steals-tui-notifications.md` — Related callback dispatch architecture
