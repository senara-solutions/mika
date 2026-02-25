---
title: "Implement filesystem-based skill registry for extensible tool system"
date: "2026-02-25"
category: "architecture-decisions"
tags: ["agent-engine", "skill-system", "extensibility", "prompt-assembly", "architecture", "security"]
severity: "medium"
component: "mika-agent (skills module)"
root_cause: "All 8 tools were hardcoded in default_tools() with instructions baked into prompt.rs, requiring recompilation to add tools and bloating every prompt with irrelevant instructions"
resolution_time: "1 session (multi-stage implementation + code review)"
confidence: "high"
---

# Filesystem-Based Skill Registry Implementation

## Problem

Mika's 8 tools were hardcoded in Rust: registered in `default_tools()`, with instructions baked into `prompt.rs`. Adding tools required recompilation. Every prompt included all tool instructions regardless of relevance. There was no mechanism to load tool definitions from external sources at runtime, conditionally include only relevant tool instructions per conversation, or add new tools without rebuilding.

## Root Cause

Tool definitions and prompt instructions were compiled statically into the binary. The `build_system_prompt()` function in `prompt.rs` contained hardcoded instructions for every tool (memory, reminders, messaging), and `default_tools()` in `tools/mod.rs` registered all tools unconditionally. No runtime extensibility existed.

## Solution

A filesystem-based skill registry at `~/.mika/skills/` with dynamic loading and keyword-based matching. 23 files changed, 1179 insertions, 67 deletions.

### Architecture

```
~/.mika/skills/
  memory/
    skill.toml          # manifest (metadata, triggers, handler config)
    system_prompt.md    # skill-specific prompt instructions
  reminders/
    skill.toml
    system_prompt.md
  messaging/
    skill.toml
    system_prompt.md
```

### Implementation (12-step plan)

1. **Manifest types** (`skills/manifest.rs`): `SkillManifest`, `Triggers`, `Handler` enum (Builtin/Exec/Http), `SkillOptions`. Serde with `tag = "type"` for polymorphic handler deserialization.
2. **Skill index** (`skills/index.rs`): `SkillEntry` struct with manifest + directory path + pre-lowercased keywords. `scan_skills_dir()` iterates subdirectories, parses TOML, skips invalid manifests with warn-level logging.
3. **Keyword matcher** (`skills/matcher.rs`): Returns `always_on` skills plus keyword-matched skills. Case-insensitive substring matching on user message.
4. **Lazy loader** (`skills/loader.rs`): `load_prompt_snippet()` and `load_tool_definitions()` via `tokio::fs`. No caching (files re-read per turn).
5. **Handler dispatch** (`skills/handler.rs`): Exec handler via `tokio::process::Command`, HTTP handler via `reqwest::Client::post()`. Configurable per-skill timeout.
6. **SkillRegistry** (`skills/mod.rs`): `from_dir()`, `empty()`, `match_message()`, `has_skills()`. `resolve_matched_skills()` merges builtin and external tool definitions with deduplication.
7. **ToolRegistry addition**: New `definition_by_name()` method for name-based tool definition lookup.
8. **Agent loop integration**: `skills` field added to `AgentParams` and `SilentAgentParams`. Three-way branch: matched skills use skill tools; no skills dir falls back to all builtin tools; no match provides no tools.
9. **Prompt extraction**: Removed tool-specific instructions from `prompt.rs`, moved to per-skill `system_prompt.md` files. Snippets appended as `## {name} Skill\n{content}`.
10. **Skill templates**: Created `templates/skills/{memory,reminders,messaging}/` with manifests and prompt snippets. Embedded via `include_str!` in mika-common for bootstrap seeding.
11. **Bootstrap seeding**: `home.rs` creates `~/.mika/skills/` and seeds builtin skills on first run. Existing user-modified skills are preserved.
12. **Server/CLI wiring**: `SkillRegistry` added to AppState, handlers, scheduler, and CLI chat/ask commands.

### Key Code Patterns

```rust
// Startup — scan skills directory once
let skill_registry = SkillRegistry::from_dir(&home_dir.join("skills"));

// Per turn — match and resolve
let matched = skill_registry.match_message(user_message);
let (tool_defs, skill_tool_map) = resolve_matched_skills(tool_registry, &matched).await;

// Prompt assembly — append matched skill snippets
for entry in &matched {
    let snippet = loader::load_prompt_snippet(&entry.dir).await;
    if !snippet.is_empty() {
        write!(system, "\n## {} Skill\n{}\n", entry.manifest.name, snippet).unwrap();
    }
}

// Tool dispatch — builtin first, then external handlers
if let Some(tool) = tool_registry.get(name) {
    tool.call(input, ctx).await
} else if let Some((handler, timeout)) = skill_tool_map.get(name) {
    execute_skill_tool(handler, name, input, *timeout).await
}
```

## Issues Encountered

### 1. Struct Field Addition Breaks Compilation (Expected)

Adding `skills: &'a SkillRegistry` to `AgentParams` broke 4 construction sites: `scheduler.rs`, `handlers.rs` (2 places), and `ask.rs`. Each needed the new field passed through.

**Fix**: Updated all construction sites to pass the skills registry. **Lesson**: Use builder patterns for structs with many fields to make additions non-breaking.

### 2. Exec Handler Appends Tool Name as Argument

The exec handler calls `Command::new(command).args(args).arg(tool_name)`, which appends the tool name as a bare argument. The timeout test used `sleep 10` as the command, but `sleep 10 slow_tool` fails immediately because sleep doesn't accept extra arguments.

**Fix**: Changed test to use `sh -c "sleep 10"` wrapper so the extra arg is consumed by `sh` rather than `sleep`.

### 3. Templates in Gitignored Directory

`.gitignore` contained `data/`, which blocked committing `data/skills/` template files. The templates needed to be in the repo for `include_str!` at compile time.

**Fix**: Moved templates from `data/skills/` to `templates/skills/` and updated all `include_str!` paths in `home.rs`.

### 4. Code Review Found 14 Issues (3 P1 Security)

Post-implementation review by 7 parallel agents found that the exec/http handlers introduce serious security risks (env var leakage, unsandboxed execution, SSRF) despite having zero current users. The entire exec/http subsystem is speculative — all 3 builtin skills use `always_on = true` with builtin handlers, making the keyword matcher, exec dispatch, HTTP dispatch, and tools.json loader all dead code.

**Key finding**: The P3 YAGNI cleanup (removing ~500 lines of speculative code) would simultaneously resolve all 3 P1 security issues by eliminating the attack surface entirely.

## Verification

- `cargo build` — clean
- `cargo test` — 207 tests pass (24 new skill tests)
- `cargo fmt` — clean
- Commit: d88a644

## Prevention Strategies

### YAGNI for Plugin Systems

Build only the handler types you need today. The exec/http handlers were fully implemented with tests, timeout handling, error paths, and agent loop integration — but zero skills use them. This created 500+ lines of attack surface for a non-existent use case.

**Rule**: If no concrete caller exists, don't build the implementation. Add a `Handler::Builtin` variant now; add `Exec`/`Http` when the first real skill needs them, with proper security review.

### Security Boundaries for Extensible Systems

When building plugin/skill systems where user-writable config files control behavior:

1. **Environment isolation**: Always `.env_clear()` before spawning child processes. Allowlist only safe vars (PATH, HOME).
2. **Command validation**: If exec handlers are needed, validate command paths (absolute only, from allowlisted directories) and sanitize arguments.
3. **URL validation**: Block private IP ranges (RFC1918, link-local, loopback, cloud metadata endpoints).
4. **Prompt sandboxing**: Wrap injected prompt snippets in delimited sections. Add size limits. Consider trust levels (builtin vs. user-provided).
5. **File size limits**: Check file size before deserializing TOML/JSON from user-writable directories (e.g., 64KB for manifests, 256KB for tool definitions).

### Avoid Duplication in Agent Loops

Extract shared skill resolution logic into a single pure function rather than duplicating in `run_agent_inner` and `run_silent_inner`. Use a return struct (`SkillResolution`) that both loops consume.

### Struct Evolution

Use builder patterns or `Default` implementations for parameter structs that grow over time. This prevents compilation breakage at every construction site when adding fields.

## Related Documents

- [Async Database Wrapper Pattern](../architecture/async-database-wrapper-pattern.md) — similar architectural pattern (wrapping sync API with async interface)
- [Phase 2 Axum HTTP Server Architecture](./phase2-axum-http-server-architecture.md) — server architecture that skills integrate with
- [Mika CLI 21-Findings Parallel Resolution](../code-review-workflow/mika-cli-21-findings-parallel-resolution.md) — previous code review resolution workflow
- [Stored Data Prompt Injection (#089)](../../todos/089-complete-p2-stored-data-prompt-injection.md) — related finding on prompt injection via stored data
- [Learnings for Rust Rewrite](../../docs/learnings-for-rust-rewrite.md) — tool system design patterns (Section 4.3)
- [Platform Systems Brainstorm](../../docs/brainstorms/2026-02-24-platform-systems-brainstorm.md) — gateway and agent architecture context

## Code Review Findings (Todos 211-224)

| ID | Priority | Finding |
|----|----------|---------|
| 211 | P1 | Exec handler leaks env vars (API keys) to child processes |
| 212 | P1 | Unsandboxed arbitrary command execution via exec skills |
| 213 | P1 | SSRF via HTTP handler with no URL validation |
| 214 | P2 | reqwest::Client created per HTTP call (no connection pooling) |
| 215 | P2 | Prompt injection via unsanitized system_prompt.md |
| 216 | P2 | Fallback behavior divergence (3-way branch) |
| 217 | P2 | Hardcoded magic string for silent mode matching |
| 218 | P2 | Cross-crate template ownership (mika-common embeds agent knowledge) |
| 219 | P2 | Duplicated skill logic in two agent loops |
| 220 | P2 | Unbounded TOML/JSON deserialization |
| 221 | P2 | No CLI command to inspect skills |
| 222 | P3 | ~500 LOC speculative dead code (YAGNI) |
| 223 | P3 | Disk I/O every turn for prompt snippets |
| 224 | P3 | Empty messaging skill prompt (lost instructions) |

## Recommended Tests

```rust
// Security: verify exec handler doesn't leak secrets
#[tokio::test]
async fn test_exec_handler_does_not_leak_api_key() {
    std::env::set_var("MIKA_ANTHROPIC_API_KEY", "secret123");
    let handler = Handler::Exec { command: "sh".into(), args: vec!["-c".into(), "env".into()], tools: vec![] };
    let output = execute_skill_tool(&handler, "test", json!({}), Duration::from_secs(5)).await;
    assert!(!output.content.contains("secret123"));
}

// Resilience: invalid manifests don't break startup
#[test]
fn test_scan_skills_skips_invalid_manifests() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("bad")).unwrap();
    std::fs::write(tmp.path().join("bad/skill.toml"), "invalid").unwrap();
    let skills = scan_skills_dir(tmp.path());
    assert!(skills.is_empty()); // graceful skip
}

// Deduplication: overlapping tool names across skills
#[tokio::test]
async fn test_matched_skills_deduplicate_tools() {
    let skill1 = make_builtin_entry("a", &["store_fact", "search_memory"], true);
    let skill2 = make_builtin_entry("b", &["store_fact", "update_fact"], true);
    let (defs, _) = resolve_matched_skills(&tools::default_tools(), &[&skill1, &skill2]).await;
    assert_eq!(defs.len(), 3); // 3 unique, not 4
}
```
