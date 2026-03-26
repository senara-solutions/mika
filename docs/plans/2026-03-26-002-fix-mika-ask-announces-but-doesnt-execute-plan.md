---
title: "fix: Conditional required_tools enforcement and mika ask callback awareness"
type: fix
status: completed
date: 2026-03-26
issue: 265
---

# fix: Conditional required_tools enforcement and mika ask callback awareness

## Overview

When using `mika ask` for code implementation tasks, the agent announces it will implement but never calls `run_claude_pilot` — only performs reconnaissance (reads files, creates work items). The session ends without commits or actual implementation. This is a combination of missing engine-level enforcement and a structural gap in `mika ask`'s one-shot design.

## Problem Statement

Three root causes combine to produce the failure:

1. **No `required_tools` on self-dev skill** — The `self-dev` skill (`mika-skills/self-dev/skill.toml`) has no `[constraints]` section. The `required_tools` enforcement gate (#270) exists but cannot fire because the skill doesn't declare which tools are mandatory.

2. **`always_on` poisons `required_tools`** — Self-dev is `always_on = true`. If we naively add `required_tools = ["run_claude_pilot"]`, `collect_required_tools()` would enforce it on EVERY message (including "what's the weather?") because `always_on` skills always match. The engine currently has no concept of WHY a skill matched.

3. **`mika ask` is one-shot** — No `TaskEngine` runs in `mika ask`. Even if `run_claude_pilot` IS called and spawns a background process, the callback task sits orphaned in SQLite until the user opens TUI or server. The user gets no indication that work is in progress.

## Proposed Solution

### Phase 1: Conditional required_tools enforcement (core fix)

Add match-reason tracking to skill matching so `required_tools` constraints are only enforced when a skill matched via keyword, not just because it's `always_on`.

### Phase 2: Self-dev skill constraint (cross-repo, mika-skills)

Add `[constraints] required_tools = ["run_claude_pilot"]` to `mika-skills/self-dev/skill.toml`. This is a **separate change in the mika-skills repo** — not included in this PR but enabled by it.

### Phase 3: `mika ask` pending callback awareness (UX improvement)

After the agent loop in `ask.rs`, query for pending callback tasks and print a notice to stderr. Add optional `pending_tasks` field to `--format json` output.

## Technical Approach

### Phase 1: Match reason tracking

**File: `crates/mika-agent/src/skills/matcher.rs`**

1. Add a `MatchReason` enum:

```rust
/// Why a skill was included in the matched set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    /// Matched because `always_on = true` (no keyword hit on this message)
    AlwaysOn,
    /// Matched because at least one keyword matched the user message
    Keyword,
    /// Pulled in as a transitive dependency of another matched skill
    Dependency,
}
```

2. Add a `MatchedSkill` wrapper:

```rust
/// A skill entry annotated with the reason it was included.
#[derive(Debug)]
pub struct MatchedSkill<'a> {
    pub entry: &'a SkillEntry,
    pub reason: MatchReason,
}
```

3. Change `match_skills()` return type from `Vec<&'a SkillEntry>` to `Vec<MatchedSkill<'a>>`. The first pass records whether each skill matched via keyword or always_on (if both, keyword wins). The BFS pass marks deps as `Dependency`.

**File: `crates/mika-agent/src/agent.rs`**

4. Update `collect_required_tools()` to accept `&[MatchedSkill]` and filter to `MatchReason::Keyword` only:

```rust
fn collect_required_tools(matched: &[MatchedSkill]) -> HashSet<String> {
    matched
        .iter()
        .filter(|m| m.reason == MatchReason::Keyword)
        .flat_map(|m| m.entry.manifest.constraints.required_tools.iter())
        .cloned()
        .collect()
}
```

5. Update all call sites that consume the matched skills list:
   - `run_agent_inner()` (~line 1011) — `SkillRegistry::match_message()` call
   - `run_team_agent()` (~line 2037) — same
   - `inject_skills_and_resolve_tools()` — needs `&[MatchedSkill]` or extracted `&[&SkillEntry]`
   - `build_skill_tool_map()` — same
   - `max_skill_timeout()` — same

   For functions that only need the `SkillEntry`, extract via `matched.iter().map(|m| m.entry).collect()`.

**File: `crates/mika-agent/src/skills/mod.rs`**

6. Update `SkillRegistry::match_message()` to return `Vec<MatchedSkill<'_>>`.

**File: `crates/mika-agent/src/skills/index.rs`**

7. Add validation warning in `validate_skill()`: if a skill is `always_on = true`, has no keywords, and declares `required_tools`, emit advisory: "required_tools will only be enforced when keywords match."

### Phase 3: `mika ask` callback awareness

**File: `crates/mika-cli/src/commands/ask.rs`**

8. After `agent::run_agent()` returns, query for pending callback tasks in this session:

```rust
let pending = ctx.async_db.get_pending_callbacks(&session_id).await?;
if !pending.is_empty() {
    eprintln!(
        "\n[mika] {} background task(s) started. \
         Open TUI (`mika`) or start server to receive results.",
        pending.len()
    );
}
```

9. For `--format json`, add optional `pending_tasks` field:

```rust
struct AskJsonResponse<'a> {
    role: &'a str,
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pending_tasks: Vec<String>, // task IDs
}
```

**File: `crates/mika-agent/src/db.rs`**

10. Add `get_pending_callbacks(session_id)` query:

```sql
SELECT id FROM tasks
WHERE trigger_type = 'callback'
  AND status = 'pending'
  AND session_id = ?
```

## System-Wide Impact

- **Interaction graph:** `match_skills()` → `SkillRegistry::match_message()` → `run_agent_inner()` / `run_team_agent()` → `collect_required_tools()` → enforcement gate in the agent loop. The change is contained to the skill matching → required_tools pipeline.
- **Error propagation:** No new error paths. The enforcement gate already handles the retry case.
- **State lifecycle risks:** None. The `MatchReason` is ephemeral (per-turn, not persisted).
- **API surface parity:** Team agent path (`run_team_agent`) uses the same `match_skills()` and must be updated in lockstep.
- **Backward compatibility:** The `match_skills()` return type change is internal (not public API). No schema changes. The `pending_tasks` JSON field is additive (skip_serializing_if empty).

## Acceptance Criteria

- [x] `collect_required_tools()` only collects from keyword-matched skills
- [x] Self-dev skill matched via keyword (e.g., "implement X") → `required_tools` enforced
- [x] Self-dev skill matched via always_on only (e.g., "what time is it") → `required_tools` NOT enforced
- [x] Mixed match (keyword + always_on) → keyword wins, `required_tools` enforced
- [x] Dependency-resolved skills do NOT contribute to `required_tools`
- [x] `mika ask` prints stderr notice when callback tasks are created
- [x] `mika ask --format json` includes `pending_tasks` array when callbacks exist
- [x] `mika skills validate` warns about always_on + no keywords + required_tools combo
- [x] All existing tests pass
- [x] New tests for match reason tracking and conditional enforcement

## Deferred

- `mika ask --wait` flag (mini task engine) — tracked separately
- Cross-repo `self-dev` skill.toml update (mika-skills repo)

## Sources

- Issue: #265
- Required-tools enforcement: #270, `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- Grounding rule pattern: `docs/solutions/prompt-engineering/grounding-rule-downstream-state-hallucination.md`
- Code-level enforcement principle: `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md`
