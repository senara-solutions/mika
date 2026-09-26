---
title: "fix: Skill-review tool registration and bundled skill tier narrowing"
type: fix
status: active
date: 2026-04-09
issue: senara-solutions/mika#499
---

# fix: Skill-review tool registration and bundled skill tier narrowing

## Overview

The skill-review bundled skill is completely non-functional. The Rust handler code was correctly merged (`write_skill_variant` into `review_skill` per #477), but the skill templates were never updated. The skill's `tools.json` still declares `write_skill_variant` pointing to a non-existent builtin, causing `load_tools_json()` to filter it out — the skill registers **zero tools**. Additionally, the `required_tools` constraint references the non-existent tool, making the required-tools gate impossible to satisfy.

This umbrella fix addresses all four sub-issues:
- **#477** — Update stale templates to reference `review_skill`
- **#468** — Mid-session tool loss was a misdiagnosis (tools are immutable per-turn); the real issue is the stale template
- **#469** — Harden `review_skill` error messages and system prompt
- **#486** — Narrow the built-in skill block from all-bundled to trust-critical only

## Problem Statement

**Observed:** mika-qa agent, 2026-04-09T07:55:28Z, trace `bb0b7da1`. User says "review skill qa-review" — both `review_skill` and `write_skill_variant` return "Unknown tool."

**Root cause chain:**

1. `templates/skills/skill-review/tools.json` declares tool `write_skill_variant` with `"handler": {"type": "builtin", "function": "write_skill_variant"}`
2. `load_tools_json()` at `index.rs:1510-1521` checks `KNOWN_BUILTINS` — `write_skill_variant` is NOT there (correctly removed during the merge)
3. The tool is filtered out with a `warn!` log. The skill registers **zero tools**.
4. `skill.toml` has `required_tools = ["review_skill", "write_skill_variant"]` — `write_skill_variant` can never be called
5. The required-tools gate rejects the agent's response, retries once, fails again
6. `system_prompt.md` instructs the agent to call `write_skill_variant` — a tool that doesn't exist

**Why #468 (mid-session tool loss) is not a separate bug:** Research proved that `match_skills()` runs once before `run_loop()` at `agent.rs:1199` and the `skill_tool_map` is immutable for the entire loop (passed as `&HashMap`). Tools cannot disappear mid-session. The "loss" was the tool never being registered in the first place.

## Proposed Solution

### Part 1: Fix stale templates (#477, #468)

Update three files in `crates/mika-agent/templates/skills/skill-review/`:

**`tools.json`** — Replace `write_skill_variant` with `review_skill`:
```json
[
  {
    "name": "review_skill",
    "description": "Inspect a skill's prompt, tools, and runtime model context, and optionally persist a model-tuned variant. Call without 'content' to inspect; call with 'content' to write the adapted prompt.",
    "input_schema": {
      "type": "object",
      "properties": {
        "skill_name": {
          "type": "string",
          "description": "Skill name to review, or '*' for batch mode"
        },
        "content": {
          "type": "string",
          "description": "Full adapted prompt to persist as the model-tuned variant. Omit to inspect only."
        },
        "dry_run": {
          "type": "boolean",
          "default": false,
          "description": "Preview the destination path without writing"
        },
        "force": {
          "type": "boolean",
          "default": false,
          "description": "Overwrite an existing variant for the current model"
        }
      },
      "required": ["skill_name"]
    },
    "handler": {
      "type": "builtin",
      "function": "review_skill"
    }
  }
]
```

**`skill.toml`** — Fix `required_tools`:
```toml
[constraints]
required_tools = ["review_skill"]
```

**`system_prompt.md`** — Rewrite for one-tool, two-call workflow referencing `review_skill` throughout. Remove all references to `write_skill_variant` and `write_agent_file`.

### Part 2: Narrow bundled skill block (#486)

**Tier mechanism:** Add a `TRUST_CRITICAL_SKILLS: &[&str]` constant in `bundled_skills.rs` alongside `BUNDLED_SKILLS`. Add `pub fn is_trust_critical_skill(name: &str) -> bool`.

**Trust-critical skills** (3) — prompts that govern self-awareness, security posture, or the ability to modify other skills:
- `skill-review` — can modify any skill's prompt (self-referential risk)
- `self-knowledge` — governs agent self-awareness and core identity
- `agents-teams` — controls multi-agent orchestration and delegation

**Reviewable bundled skills** (9) — functional skills whose prompts can be safely adapted per-model:
- `tmux`, `shell-exec`, `web-search`, `file-reader`, `git-ops`, `google-workspace`, `github`, `mcp`, `browser-control`

**Rationale:** Trust-critical skills have prompts where model-specific rewording could weaken security guards, alter self-identity, or introduce delegation loopholes. Functional skills have prompts focused on tool usage mechanics — safe to adapt.

**Code changes:**
- `builtin_handlers.rs` line ~996: Change `is_bundled_skill()` → `is_trust_critical_skill()` in the `review_skill` guard
- `builtin_handlers.rs` line ~1283: Change `is_bundled_skill()` in `review_skill_batch()` to `is_trust_critical_skill()`
- **Other call sites unchanged:** `install.rs`, `delete_skill.rs`, `update_skill.rs` keep using `is_bundled_skill()` — users should not be able to delete or overwrite ANY bundled skill

### Part 3: Harden system prompt (#469)

Update the system prompt with:
- Explicit mention that the target model is derived from `runtime_model` in the inspect response — do not guess
- Clear instruction to preserve at least 50% of the source prompt size (explain the truncation guard)
- Warning that `write_agent_file` must NEVER be used for variant writes — `review_skill` is the only correct tool
- Updated restrictions section listing only trust-critical skills (not all 12)

## Technical Considerations

- **Dispatch chain safety:** `review_skill` is already in `KNOWN_BUILTINS` and `execute()` dispatch — no dispatch changes needed
- **`required_tools` validation:** The tool name `review_skill` in `tools.json` matches `required_tools = ["review_skill"]` — no advisory warning will fire from `validate_skill()`
- **`seed_bundled_skills()` propagation:** On next restart, templates are overwritten on disk. Running agents need a restart to pick up changes.
- **No schema migration:** This is a template-only + handler-guard change. No DB changes.

## System-Wide Impact

- **Interaction graph:** `review_skill` handler is called via `execute()` dispatch when the skill-review skill is keyword-matched. The tool definition now comes from the updated `tools.json`. No other callers change.
- **Error propagation:** The required-tools gate will now succeed (since `review_skill` is actually registered). Errors from the handler itself (missing skill, truncation, overwrite) propagate as `ToolOutput::success` with descriptive messages.
- **State lifecycle risks:** None. `persist_variant` performs atomic `create_dir_all` + `write` + `skills_dirty.store(true)`. Same as before.
- **API surface parity:** The `review_skill` tool gains visibility (was hidden due to stale templates). The tool surface shrinks by one (`write_skill_variant` references removed). Pre-1.0 — no backward compat needed.

## Acceptance Criteria

- [x] `tools.json` declares `review_skill` with handler function `review_skill` and the four parameters (`skill_name`, `content`, `dry_run`, `force`)
- [x] `skill.toml` has `required_tools = ["review_skill"]` (no `write_skill_variant`)
- [x] `system_prompt.md` references only `review_skill`, never `write_skill_variant` or `write_agent_file`
- [x] `is_trust_critical_skill()` function exists in `bundled_skills.rs` with 3 skills
- [x] `review_skill` handler allows reviewing non-trust-critical bundled skills (e.g., `web-search`)
- [x] `review_skill` handler blocks trust-critical skills with clear error message
- [x] Batch mode (`skill_name: "*"`) skips only trust-critical skills, includes reviewable bundled skills
- [x] `cargo build`, `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings` all pass
- [x] No references to `write_skill_variant` remain in templates or system prompts
- [x] `grep -rn 'write_skill_variant' crates/mika-agent/templates/` returns zero matches

## Critical Files

| File | Change |
|---|---|
| `crates/mika-agent/templates/skills/skill-review/tools.json` | Replace `write_skill_variant` tool with `review_skill` tool definition |
| `crates/mika-agent/templates/skills/skill-review/skill.toml` | Update `required_tools` to `["review_skill"]` |
| `crates/mika-agent/templates/skills/skill-review/system_prompt.md` | Full rewrite for `review_skill` one-tool workflow |
| `crates/mika-agent/src/bundled_skills.rs` | Add `TRUST_CRITICAL_SKILLS` constant + `is_trust_critical_skill()` function |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (~line 996) | Change `is_bundled_skill()` → `is_trust_critical_skill()` in review guard |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (~line 1283) | Change `is_bundled_skill()` → `is_trust_critical_skill()` in batch mode |
| `crates/mika-agent/src/skills/builtin_handlers.rs` (tests) | Add tests for trust-critical vs reviewable bundled skills |

## Verification

```bash
# Build and test
cargo build -p mika-agent
cargo test -p mika-agent
cargo clippy -p mika-agent --all-targets -- -D warnings

# No stale references in templates
! grep -rn 'write_skill_variant' crates/mika-agent/templates/

# Trust-critical function exists
grep -n 'is_trust_critical_skill' crates/mika-agent/src/bundled_skills.rs
```

## Sources

- Issue: senara-solutions/mika#499 (umbrella)
- Sub-issues: #477, #468, #469, #486
- Prior plan (merge): `docs/plans/2026-04-07-004-fix-merge-skill-variant-into-review-plan.md`
- Prior plan (block built-in): `docs/plans/2026-04-07-006-fix-skill-review-block-built-in-skills-plan.md`
- Learnings: `docs/solutions/architecture-patterns/adding-builtin-handler-skill-git-ops.md` (registration checklist)
- Learnings: `docs/solutions/security-issues/review-skill-builtin-trust-boundary.md` (trust boundary guard)
- Learnings: `docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md` (variant loading)
