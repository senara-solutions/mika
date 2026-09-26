---
title: "Adding a builtin handler skill (git-ops pattern)"
category: architecture-patterns
date: 2026-03-27
tags: [builtin-handler, skill, bundled-skill, git, subprocess, registration]
related_issues: ["#300"]
---

# Adding a Builtin Handler Skill (git-ops pattern)

## Problem

Need to add a new bundled skill with a builtin handler that spawns subprocesses (git commands). The registration touches 4 files across 2 crate directories, and subprocess spawning requires security considerations (env scrubbing, argument injection prevention).

## Root Cause / Context

Builtin handler skills require coordinated changes across: template files (skill.toml, tools.json, system_prompt.md), handler registration (KNOWN_BUILTINS, execute dispatch), bundled skill registration (skill! macro, BUNDLED_SKILLS array), and documentation (docs/skills.md, docs/runtime-structure.md).

## Solution

### Registration checklist (4 files, 3 steps)

1. **Template files** in `crates/mika-agent/templates/skills/<name>/`:
   - `skill.toml` — manifest with `[skill]` and `[triggers]` sections
   - `tools.json` — tool definitions with `"handler": {"type": "builtin", "function": "<fn_name>"}`
   - `system_prompt.md` — LLM guidance

2. **Handler implementation** in `crates/mika-agent/src/skills/builtin_handlers.rs`:
   - Add function name to `KNOWN_BUILTINS` array (alphabetical)
   - Add match arm in `execute()` dispatch (alphabetical)
   - Implement handler function with signature: `async fn handler(input: &serde_json::Value, _ctx: &ToolContext<'_>) -> ToolOutput`

3. **Bundled skill registration** in `crates/mika-agent/src/bundled_skills.rs`:
   - Add `static` declaration using `skill!` macro
   - Add reference to `BUNDLED_SKILLS` array

### Subprocess security pattern for builtin handlers

```rust
// Build command with env scrubbing (reuse for all CLI builtin handlers)
let mut cmd = tokio::process::Command::new("git");
cmd.current_dir(repo_path);
cmd.env("GIT_TERMINAL_PROMPT", "0");  // Prevent credential prompts
super::executor::scrub_mika_env_vars(&mut cmd);  // Remove MIKA_* secrets

// Use spawn_and_collect for bounded output capture
let output = spawn_and_collect(cmd, "git", "Is git installed?").await;
```

### Key gotcha: spawn_and_collect success detection

`spawn_and_collect` always returns `is_error: false` — even on non-zero exit. It stuffs exit info into content as text prefixes ("Exit code: N", "Killed by signal: N"). To detect failure:

```rust
struct GitResult { content: String, success: bool }

// Parse spawn_and_collect output to detect actual failure
let success = !output.content.starts_with("Exit code:")
    && !output.content.starts_with("Killed by signal:")
    && !output.content.starts_with("Failed to spawn");
```

This coupling to `spawn_and_collect`'s format strings is fragile but necessary given the current API. Consider adding a comment noting the dependency.

### Input validation for subprocess args

Validate inputs that become subprocess arguments to prevent git argument injection:

```rust
// Reject refs starting with '-' (prevents --exec=, --strategy-option=, etc.)
if base.starts_with('-') {
    return Err(ToolOutput::error("Invalid base ref: must not start with '-'."));
}

// Enforce absolute paths for repo_path
if !std::path::Path::new(&repo_path).is_absolute() {
    return Err(ToolOutput::error("repo_path must be an absolute path."));
}
```

### Handler function signature consistency

All builtin handlers must accept `(input: &serde_json::Value, _ctx: &ToolContext<'_>)` even if `ctx` is unused. This keeps the dispatch table uniform and enables future refactoring without touching the dispatch.

## Prevention

- **Use the checklist above** when adding any new builtin handler skill
- **Always validate subprocess arguments** — reject values starting with `-` to prevent flag injection
- **Keep handler signatures uniform** — accept `ctx` even when unused
- **Choose specific keywords** in skill.toml — avoid bare common words like "git" (matches "forget", "digital")
- **Test both validation edge cases AND integration** (real git repo in tempdir)
- **Update docs in lockstep**: docs/skills.md (skill table), docs/runtime-structure.md (directory tree)
