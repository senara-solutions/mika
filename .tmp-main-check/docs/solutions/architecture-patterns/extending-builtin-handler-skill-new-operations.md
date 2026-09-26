---
title: "Extending a builtin handler skill with new operations"
date: 2026-04-21
category: architecture-patterns
module: skills
problem_type: best_practice
component: tooling
severity: low
applies_when:
  - Adding new operations to an existing builtin handler skill (e.g., git-ops, run_gh)
  - Extending a tool's operation enum without breaking existing operations
tags: [builtin-handler, skill, git-ops, operation-enum, validation, backward-compatible]
---

# Extending a Builtin Handler Skill with New Operations

## Context

Builtin handler skills like git-ops use an `operation` enum to dispatch to per-operation handler functions. When adding new operations (e.g., `pull`, `checkout`, `worktree_add`), the change must be backward-compatible — existing operations must keep working identically. The pattern touches 4 files: the handler implementation, tools.json, system_prompt.md, and skill.toml.

## Guidance

### 1. Extract the operation allowlist into a constant

Move the hardcoded operation list from the validation function into a `const` array. This prevents the allowlist and error messages from drifting:

```rust
const GIT_OPS_VALID_OPERATIONS: &[&str] = &[
    "fetch", "rebase", "merge",
    "pull", "checkout",
    "worktree_add", "worktree_remove", "worktree_list", "worktree_prune",
];
```

Use the constant in both the validation check and error messages via `.join(", ")`.

### 2. Add new parameters as optional fields with operation-gated validation

New operations may need parameters that existing operations don't use (e.g., `branch` for `checkout`, `path` for `worktree_add`). Add them as `Option<String>` to the input struct and validate conditionally:

- Extract the parameter (optional, may be None)
- Reject argument injection (dash-prefix check, same as `base`)
- Validate required-when rules per operation (e.g., `branch` required for `checkout` and `worktree_add`)
- Validate absolute path requirement for filesystem paths

### 3. Extend preflight checks by operation name

If the new operation needs clean-tree checks (like `pull` does), add its name to the preflight condition alongside existing operations. Use string comparison — no enum type needed for this pattern.

### 4. Add per-operation handler functions

Each new operation gets its own `async fn` following the existing pattern:
- Accept only the parameters relevant to that operation
- Use `run_git()` for subprocess execution (inherits env scrubbing)
- Return `ToolOutput::success()` or `ToolOutput::error()` with structured messages
- For operations with fallback behavior (e.g., `worktree_add` tries `-b` first, then existing branch), chain the attempts with clear error reporting

### 5. Wire into the dispatch match

Add new arms to the `match params.operation.as_str()` block. Use `.as_deref().unwrap()` for parameters guaranteed present by validation.

### 6. Update all 4 template files atomically

- `tools.json`: extend the `enum` array and add new property definitions
- `system_prompt.md`: add usage examples and operation documentation
- `skill.toml`: update description and add trigger keywords
- `docs/skills.md`: update the keyword list and description paragraph

## Why This Matters

A single constant for the operation allowlist prevents the most common drift bug: adding a new operation to the dispatch match but forgetting to update the validation allowlist (or vice versa). The conditional parameter validation pattern keeps the single-tool interface clean while supporting operations with different parameter requirements.

## When to Apply

- Adding operations to git-ops, run_gh, run_gws, or any builtin handler with an operation enum
- Any time a builtin handler's tool schema needs new parameters that are only relevant for specific operations

## Examples

The git-ops skill extension (#610) added 6 operations and 2 new parameters following this pattern. The key files changed:

- `crates/mika-agent/src/skills/builtin_handlers.rs` — `GIT_OPS_VALID_OPERATIONS` constant, `GitOpsInput` struct with `branch`/`path` fields, validation logic, 6 new handler functions, 28 new tests
- `crates/mika-agent/templates/skills/git-ops/tools.json` — enum extended, `branch` and `path` properties added
- `crates/mika-agent/templates/skills/git-ops/system_prompt.md` — full rewrite with all 9 operations documented
- `crates/mika-agent/templates/skills/git-ops/skill.toml` — keywords expanded, version bumped to 0.2.0

## Related

- [Adding a builtin handler skill (git-ops pattern)](adding-builtin-handler-skill-git-ops.md) — the initial creation pattern
- [Adding a builtin handler skill (skill-review pattern)](adding-skill-review-builtin-handler.md) — another builtin handler example
- GitHub issue: #610
