# Skill Dependency Resolution and Unsolicited Action Guard

## Problem

Two behavioral bugs stemmed from limitations in the skill matching system (GitHub issue #134):

**Bug 1 — Tool unavailability from missing dependency resolution:** When an `always_on` skill (e.g., `self-dev`) instructed the agent to use tools from another skill (e.g., `tmux`), those tools were unavailable if the current user message didn't keyword-match the dependency skill. The `match_skills()` function performed single-pass keyword matching with no concept of skill dependencies. A confirmation message like "yes please" wouldn't load `tmux`, so the agent fell back to `run_shell`.

**Bug 2 — Unsolicited multi-step action on informational questions:** When the user asked "can you list my tmux sessions?" (an informational question), the agent listed sessions but then immediately sent a `/mika` command — starting a full dev pipeline without user confirmation.

## Root Cause

1. `match_skills()` in `matcher.rs` performed single-pass keyword matching with no dependency awareness. Skills could not declare that they required other skills.
2. No system prompt guardrail distinguished informational questions from action requests, so the agent interpreted questions as implicit instructions.

## Solution

### 1. Dependency Declaration (manifest.rs)

Added optional `dependencies: Vec<String>` field to `SkillInfo` with `#[serde(default)]` for backward compatibility:

```rust
/// Other skills that should be loaded when this skill is active.
/// One level only — no transitive resolution.
#[serde(default)]
pub dependencies: Vec<String>,
```

**Design decision — one level only:** Transitive dependency resolution (A→B→C) was explicitly rejected. It would require cycle detection, conflict resolution, and versioning — complexity that doesn't match the skill system's flat, composable design. One-level dependencies are self-documenting and bounded.

### 2. Two-Pass Matching Algorithm (matcher.rs)

Rewrote `match_skills()` with a two-pass approach using `HashSet<usize>` for dedup:

- **Pass 1:** Collect direct matches (always_on OR keyword hit) into a `HashSet<usize>` of skill indices
- **Pass 2:** For each initially matched skill, resolve its dependencies — look up by name (case-insensitive via `eq_ignore_ascii_case`), check enabled flag, add to the set

Key properties:
- **No infinite loops on circular deps:** Pass 2 iterates a snapshot of initial indices, not the accumulating set
- **Deduplication:** HashSet prevents loading the same skill twice when multiple skills depend on it
- **Order-preserving:** Final collection iterates the original skill array, filtering by set membership
- **Disabled deps skipped:** Dependency pull checks the `enabled` flag, respecting user's choice

### 3. Consolidated Validation (mod.rs)

Dependency validation was consolidated into `apply_overrides()` — the single function that runs at every registry initialization site (startup, hot reload, team init, delegation). This eliminated 7 standalone validation calls and ensured validation cannot be accidentally skipped.

The validator emits `tracing::warn` for unknown dependencies (doesn't crash — operator flexibility for optional/transitional deps).

### 4. Safe Mode Exclusion (mod.rs)

`safe_always_on_skills()` — used by silent/heartbeat/background agents — intentionally does NOT resolve dependencies. This prevents exec/http handler skills from being pulled into autonomous background contexts where they could execute arbitrary commands unsupervised. Regression test `test_safe_always_on_skills_excludes_exec_dependency` guards this boundary.

### 5. Agent-Native Parity (create_skill.rs, update_skill.rs)

Both `create_skill` and `update_skill` tools accept an optional `dependencies` parameter with validation:
- Max 20 dependencies per skill
- Max 128 characters per dependency name
- No empty names
- Shared `validate_dependencies()` function

### 6. Confirmation Before Action Guardrail (prompt.rs)

Added an explicit instruction to the system prompt:

> When the user asks an informational question (e.g., "can you list...", "what are...", "show me..."), answer the question directly and stop. Do not interpret questions as implicit requests to start multi-step workflows. If follow-up action may be useful, suggest it and wait for confirmation.

## Key Design Patterns

| Pattern | Why |
|---------|-----|
| One-level dependencies only | Bounded complexity, self-documenting, no cycle detection needed |
| HashSet dedup with snapshot iteration | Prevents infinite loops on circular deps while ensuring dedup |
| Consolidated validation at `apply_overrides()` | Single point of truth, impossible to skip |
| Warnings not crashes on missing deps | Operator flexibility, graceful degradation |
| Explicit safe-mode exclusion | Security boundary — autonomous agents must not load exec handlers via deps |
| Case-insensitive name matching | Consistent with system-wide COLLATE NOCASE convention |

## Prevention

- **Adding new skills with tool dependencies:** Declare `dependencies = ["skill-name"]` in `skill.toml` instead of relying on keyword matching.
- **New initialization sites:** Always call `apply_overrides()` on the `SkillRegistry` — it handles both DB overrides and dependency validation.
- **Background/autonomous contexts:** Use `safe_always_on_skills()`, never `match_skills()`. The regression test prevents accidental unification.
- **Agent behavior boundaries:** Prompt guardrails belong in the Instructions section of `build_system_prompt()` to ensure they apply across all modes (CLI, server, team).

## Related

- `docs/solutions/architecture-patterns/delegation-work-item-guard-enforcement.md` — code guards over prompt instructions pattern
- `docs/solutions/database-issues/skill-override-persistence-via-db-layer.md` — `apply_overrides()` post-scan overlay pattern
- `docs/solutions/integration-issues/skills-doc-code-drift-and-validation-infrastructure.md` — graceful degradation for broken skills
- `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md` — tool name dedup patterns
- `docs/adr/002-filesystem-skill-registry.md` — core skill system design

## Files Changed

- `crates/mika-agent/src/skills/manifest.rs` — `dependencies` field on `SkillInfo`
- `crates/mika-agent/src/skills/matcher.rs` — two-pass `match_skills()` algorithm
- `crates/mika-agent/src/skills/mod.rs` — validation in `apply_overrides()`, `safe_always_on_skills()` doc
- `crates/mika-agent/src/prompt.rs` — confirmation before action guardrail
- `crates/mika-agent/src/tools/create_skill.rs` — `dependencies` parameter + validation
- `crates/mika-agent/src/tools/update_skill.rs` — `dependencies` parameter
