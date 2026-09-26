# Plan: Enforce Tool Execution Before Accepting Assistant Responses

**Issue:** #270
**Type:** Bug fix
**Component:** agent-core

## Problem

The agent engine accepts any assistant response regardless of whether required tool calls were made. A skill can define a multi-step review process with mandatory tool calls (e.g., qa-review requiring `run_gh`), but the engine has no mechanism to enforce that those calls actually happen. The model can shortcut the entire process by fabricating results.

## Evidence

- mika-qa fabricated a complete PR review without making any tool calls
- Session `e3d6cd8b` shows 2 messages (user → assistant) with 0 tool calls
- The assistant fabricated a `<context type="tool_history">` block with fake `run_gh` results

## Approach: Option A — Skill-Declared Required Tools

Skills declare required tool calls in `skill.toml` via a new `[constraints]` section:

```toml
[constraints]
required_tools = ["run_gh"]
```

The agent engine collects all `required_tools` from matched skills, tracks which tools were actually called during the turn, and rejects the response if required tools weren't used.

### Why Option A over B/C

- **Option B** (turn-level minimum) is too blunt — not all turns need tool calls
- **Option C** (response validation hook) is effectively what A does, but A is declarative and configurable per-skill
- Option A follows the existing pattern of skill manifests defining behavior (like `always_on`, `timeout_secs`, `dependencies`)

## Implementation

### 1. Add `Constraints` to `SkillManifest` (manifest.rs)

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Constraints {
    #[serde(default)]
    pub required_tools: Vec<String>,
}
```

Add to `SkillManifest`:
```rust
pub struct SkillManifest {
    // ... existing fields ...
    #[serde(default)]
    pub constraints: Constraints,
}
```

### 2. Collect required tools from matched skills (agent.rs)

New helper function:
```rust
fn collect_required_tools(matched: &[&SkillEntry]) -> HashSet<String> {
    matched.iter()
        .flat_map(|e| e.manifest.constraints.required_tools.iter())
        .cloned()
        .collect()
}
```

### 3. Add enforcement to `run_loop()` (agent.rs)

Add `required_tools: &HashSet<String>` parameter to `run_loop()`.

In the `EndTurn` handler, before accepting a text response:
1. Compute `missing = required_tools - tools_called`
2. If missing is non-empty AND we haven't already retried:
   - Inject a correction message listing the missing tools
   - Continue the loop (counts against max_steps)
3. Allow at most 1 such retry to prevent infinite loops

Track `tools_called: HashSet<String>` by recording tool names in the `ToolUse` branch.

### 4. Update all `run_loop()` call sites

- `run_agent_inner()` — pass collected required tools from matched skills
- `run_silent_inner()` — pass empty set (silent mode uses safe_always_on_skills, no required_tools)
- `run_team_agent()` — pass collected required tools from matched skills

### 5. Add validation in skill scanning (index.rs)

In `validate_skill()`, warn if `required_tools` references tool names not found in the skill's `tools.json` (advisory warning, not a hard error — tools might be builtins).

### 6. Tests

- **manifest.rs**: Parse `[constraints]` section, defaults to empty
- **agent.rs**: `collect_required_tools` helper tests
- **agent.rs**: Verify `required_tools` enforcement logic in `run_loop` (unit tests for the check function)

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/manifest.rs` | Add `Constraints` struct, add to `SkillManifest` |
| `crates/mika-agent/src/agent.rs` | Add `collect_required_tools()`, modify `run_loop()` signature and enforcement, update all call sites |
| `crates/mika-agent/src/skills/index.rs` | Add `required_tools` validation in `validate_skill()` |
| `crates/mika-agent/src/skills/matcher.rs` | Update test helpers for new `SkillManifest` field |

## Risks

- **Retry budget**: The enforcement retry counts against `max_steps`, so a skill requiring tools at step 9 of 10 might not have room. This is acceptable — the enforcement is a best-effort guardrail, not a guarantee.
- **False positives**: If a model legitimately doesn't need to call a required tool (e.g., answering from cache), the enforcement will force an unnecessary call. This is the desired behavior for the use case described in #270.
- **Backward compatibility**: `#[serde(default)]` ensures existing `skill.toml` files parse without changes.
