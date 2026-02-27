---
title: "Code Review: update_skill Tool Discoverability and Parity Gaps"
date: "2026-02-27"
category: "logic-errors"
tags:
  - code-review
  - agent-discovery
  - input-validation
  - security
  - symlink-guard
  - code-duplication
  - skill-management
severity: "P1 (Critical)"
component: "mika-agent crate — tools subsystem and prompt assembly"
symptoms:
  - "Agent unaware of update_skill tool (missing from system prompt guidance)"
  - "Skills could become untriggerable if all keywords removed from non-always_on skill"
  - "Module imports violated alphabetical ordering convention"
  - "Filesystem paths leaked in error messages"
  - "Symlink guard missing from toggle_skill (security inconsistency)"
  - "Test helper ctx_with_home() duplicated 4 times across test modules"
  - "Validation logic duplicated between create_skill and update_skill"
root_cause: "New update_skill tool added without comprehensive parity review against existing skill tools — missing prompt integration, post-mutation validation, consistent security guards, and shared code extraction."
resolution: "Eight findings resolved in commit f87d38b: prompt integration, post-mutation keyword validation, module reordering, path sanitization, shared symlink guard, centralized test helper, shared validators."
commits:
  - "3b424c1 — feat: add update_skill tool (original commit under review)"
  - "f87d38b — fix: resolve 8 code review findings from commit 3b424c1"
related_tools:
  - create_skill
  - update_skill
  - toggle_skill
  - list_skills
review_agents_used:
  - security-sentinel
  - architecture-strategist
  - code-simplicity-reviewer
  - pattern-recognition-specialist
  - agent-native-reviewer
  - learnings-researcher
---

# update_skill Tool: Discoverability and Parity Gaps

## Problem

After adding an `update_skill` tool (commit 3b424c1, 516 lines: 220 code + 296 tests), a comprehensive code review using 6 specialized agents found **8 issues across 3 priority levels**. The core problem: the tool was added without maintaining parity with existing skill tools across multiple dimensions — prompt awareness, validation, security guards, error messages, and test patterns.

### Symptoms

1. Agent could not discover `update_skill` because it wasn't mentioned in the system prompt
2. Users could create untriggerable skills by setting empty keywords on a non-always_on skill
3. Whitespace-only keywords (e.g., `["", "  "]`) were accepted without filtering
4. Error messages leaked full filesystem paths (e.g., `/home/user/.mika/skills/foo`)
5. `toggle_skill` lacked the symlink guard present in `create_skill` and `update_skill`
6. Identical test helper and validation code duplicated across 4 modules

## Root Cause

The `update_skill` tool replicated the structure of `create_skill` but missed several cross-cutting concerns:

- **Agent discoverability**: The system prompt's Tool Usage section is the agent's only way to learn about tools. New tools must be explicitly mentioned there.
- **Post-mutation invariants**: `create_skill` validates keywords-vs-always_on at creation time, but `update_skill` allowed mutations that violated this invariant after the fact.
- **Security guard coverage**: Symlink path verification was added to `create_skill` and copied to `update_skill`, but `toggle_skill` predated both and was never retrofitted.
- **Information leakage**: Error messages included `skill_dir.display()` for debugging convenience, but in multi-tenant containers this reveals internal layout.

## Solution

### Priority 1 — Critical

#### P1 #1: System Prompt Missing `update_skill`

**File:** `crates/mika-agent/src/prompt.rs`

Added tool hint to the Tool Usage section:

```rust
prompt.push_str("- You can update existing skill descriptions, keywords, prompts, or always_on settings with update_skill.\n");
```

Added test assertion to prevent regression:

```rust
assert!(prompt.contains("update_skill"));
```

#### P1 #2: Empty Keywords Validation Gap

**File:** `crates/mika-agent/src/tools/update_skill.rs`

Added keyword trim/filter to match `create_skill` pattern:

```rust
let keywords: Vec<String> = input["keywords"]
    .as_array()
    .map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    })
    .unwrap_or_default();
```

Added post-mutation invariant check before serializing to disk:

```rust
if manifest.triggers.keywords.is_empty() && !manifest.skill.always_on {
    return Ok(ToolOutput::error(
        "Skill would have no trigger mechanism. \
         Either provide keywords or set always_on to true.",
    ));
}
```

Added 4 tests: empty keywords rejected, whitespace-only filtered then rejected, empty keywords allowed with always_on=true, disabling always_on with no keywords rejected.

### Priority 2 — Important

#### P2 #3: Module Ordering

**File:** `crates/mika-agent/src/tools/mod.rs`

Swapped `mod update_skill;` and `mod update_fact;` to restore alphabetical ordering convention.

#### P2 #4: Symlink Guard Inconsistency

**Files:** `create_skill.rs`, `update_skill.rs`, `toggle_skill.rs`

Extracted shared helper in `create_skill.rs`:

```rust
pub(super) fn verify_skill_path(skills_dir: &Path, skill_dir: &Path) -> Result<(), String> {
    match (skills_dir.canonicalize(), skill_dir.canonicalize()) {
        (Ok(parent), Ok(child)) if child.starts_with(&parent) => Ok(()),
        (Ok(_), Ok(_)) => Err("Skill directory escaped skills root (possible symlink attack).".into()),
        _ => Err("Failed to verify skill directory location.".into()),
    }
}
```

Replaced inline checks in `create_skill` and `update_skill`. Added the guard to `toggle_skill` (was missing entirely).

#### P2 #5: Path Disclosure

**Files:** `update_skill.rs`, `toggle_skill.rs`, `create_skill.rs`

Changed all skill tool error messages to omit filesystem paths:

```rust
// Before:
format!("Skill '{name}' not found at {}.", skill_dir.display())
// After:
format!("Skill '{name}' not found.")
```

### Priority 3 — Cleanup

#### P3 #6: Duplicated `ctx_with_home` Test Helper

**Files:** `test_utils.rs` + 4 test modules

Added `ctx_with_home()` method to `TestHarness`:

```rust
pub fn ctx_with_home<'a>(&'a self, home: &'a std::path::Path) -> ToolContext<'a> {
    ToolContext {
        db: &self.db,
        session_id: "test-session",
        home_dir: home,
        core_memory_edit_count: &self.counter,
        is_onboarding: false,
        message_sender: None,
        embedding_client: None,
    }
}
```

Removed identical function from `create_skill`, `update_skill`, `toggle_skill`, and `list_skills` test modules.

#### P3 #7: Shared Validators

**Files:** `create_skill.rs`, `update_skill.rs`

Extracted three `pub(super)` validation functions in `create_skill.rs`:

- `validate_description(desc: &str) -> Result<(), String>` — non-empty, max 10,000 chars
- `validate_system_prompt(prompt: &str) -> Result<(), String>` — non-empty, max 10,000 chars
- `validate_keywords(keywords: &[String]) -> Result<(), String>` — max 50 keywords, max 100 chars each

Both `create_skill` and `update_skill` now call these shared functions instead of inline validation.

## Verification

- 377 tests pass (`cargo test -p mika-agent`)
- 0 new clippy warnings (`cargo clippy -p mika-agent`)
- 7 files changed, +254/-183 lines

## Prevention

### Checklist: Adding a New Tool

1. **Prompt integration**: Add tool hint to `prompt.rs` Tool Usage section + test assertion
2. **Validation parity**: Reuse shared validators (`validate_description`, `validate_system_prompt`, `validate_keywords`, `validate_skill_name`). Validate post-mutation state, not just individual inputs.
3. **Security guards**: If the tool constructs paths from user input, call `verify_skill_path()` before any filesystem operations. Never include filesystem paths in error messages.
4. **Module registration**: Add `mod` declaration in alphabetical order in `mod.rs`. Register in `default_tools()`.
5. **Test helpers**: Use `TestHarness::ctx_with_home()` — do not duplicate context assembly in test modules.
6. **Cross-tool review**: Before implementing, review all related tools for patterns that must be replicated.

### Invariants for Skill-Mutating Tools

All tools that modify skill state **MUST** enforce:

| Invariant | Guard |
|-----------|-------|
| Trigger mechanism exists | `keywords.is_empty() && !always_on` → error |
| Path stays inside skills root | `verify_skill_path()` before filesystem ops |
| Keywords are clean | Trim whitespace, filter empty strings |
| Error messages are opaque | No `skill_dir.display()` in user-facing errors |
| Validation is shared | Import from `create_skill`, don't duplicate |

### Testing Strategy

- **Agent discoverability**: `prompt.rs` tests assert each discoverable tool name appears in `build_system_prompt()` output
- **Validation parity**: Same invalid inputs must fail identically in `create_skill` and `update_skill`
- **Security guard coverage**: `grep -n "verify_skill_path" crates/mika-agent/src/tools/*.rs` should show all mutation tools

## Related Documentation

- [agent-skill-hallucination-tui-scroll-telegram-awareness.md](agent-skill-hallucination-tui-scroll-telegram-awareness.md) — Original create_skill/list_skills/toggle_skill implementation with security hardening
- [filesystem-skill-registry-implementation.md](../architecture-decisions/filesystem-skill-registry-implementation.md) — Skill system architecture and 14 code review findings on exec/http handlers
- [agent-cli-self-knowledge-and-skill-triggers.md](agent-cli-self-knowledge-and-skill-triggers.md) — Agent discoverability patterns and drift-detection tests
- [agent-loop-variant-extraction-and-deduplication.md](../refactoring/agent-loop-variant-extraction-and-deduplication.md) — Prior DRY refactoring of duplicated agent loop variants
- [code-review-7aba1ec-shell-injection-memory-safety.md](../security-issues/code-review-7aba1ec-shell-injection-memory-safety.md) — Security review patterns including path canonicalization
- [parallel-agent-code-review-methodology.md](../code-review-workflow/parallel-agent-code-review-methodology.md) — Multi-agent code review workflow used for this review
