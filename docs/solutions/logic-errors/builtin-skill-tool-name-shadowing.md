---
title: "Builtin Tool Name Collision Silently Shadows Skill Tool in Dispatch Chain"
date: "2026-03-06"
problem_type: "logic-errors"
component: "agent-tool-dispatch"
symptoms:
  - "Exec-handler skill tool is registered but never invoked"
  - "Agent always uses builtin implementation regardless of skill configuration"
  - "No error or warning emitted — skill tool silently unreachable"
  - "Skill-specific capabilities (e.g., image return via __mika_v1 protocol) never activate"
root_cause: "The dispatch chain (builtins → skills → MCP) resolves tools by name in priority order. A builtin named `read_file` wins unconditionally over a skill that registers a tool with the same name, because the lookup short-circuits at the first match. No collision detection is performed at registration or dispatch time."
solution_type: "rename"
tags:
  - "tool-dispatch"
  - "naming-collision"
  - "skill-integration"
  - "silent-failure"
  - "agent-tools"
  - "rust"
related_issues: []
related_solutions:
  - "../integration-issues/write-file-tool-overwrite-confirmation-flow.md"
  - "../logic-errors/tool-path-reporting-misbehavior.md"
  - "../integration-issues/adding-prompt-only-bundled-skill.md"
---

# Builtin Tool Name Collision Silently Shadows Skill Tool in Dispatch Chain

## Problem Statement

When a builtin tool and an exec-handler skill both register a tool under the same name,
the dispatch chain (builtins → skills → MCP) silently selects the builtin on every
invocation — the skill's tool is permanently unreachable with no error, no log warning,
and no indication to the agent or developer that shadowing is occurring.

In Mika, builtins `read_file` and `list_files` shadowed a `file-reader` skill that also
registered `read_file`, suppressing the skill's image-return capability (`__mika_v1`
protocol) for the entire lifetime of the affected code. The skill was installed and
configured but never reachable.

## Symptoms

- Exec-handler skill is present in the skill registry (`/skills` shows it loaded), but
  the skill's behavior (e.g., image handling) never occurs in agent responses.
- The builtin's simpler behavior is always used even when the skill is enabled and
  keyword-matched.
- No error or panic — the shadowing is entirely silent.
- The symptom only appears when the skill and builtin have overlapping tool names;
  it is invisible during unit tests that test each in isolation.

## Root Cause

The tool dispatch chain in `crates/mika-agent/src/tools/mod.rs` iterates builtins first,
then skills, then MCP servers. Because builtin tools are checked before skill-provided
tools, any builtin whose `name()` return value matches a skill's tool name will always
shadow the skill:

```
dispatch_tool("read_file")
  → check builtins → ReadFileTool.name() == "read_file" → MATCH, execute, return
  → skills never reached
  → MCP never reached
```

There was no duplicate-detection at registration time, no warning at dispatch time, and no
test coverage that exercised the skill path specifically when a same-named builtin existed.

The original motivation for adding `read_file` as a builtin was to ensure file access
worked in silent mode (where exec-handler skills are filtered out for security). But the
builtin was given the same generic name as the skill, creating a silent collision.

## Working Solution

### Core Fix: Rename Colliding Builtins

Give builtins namespace-scoped names that are semantically distinct from skill-provided
tool names. The `_home_` infix signals that these tools operate on the agent's home
directory specifically, not the general filesystem.

**`crates/mika-agent/src/tools/read_home_file.rs`** (was `read_file.rs`):
```rust
// Before:
pub struct ReadFileTool;
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
}

// After:
pub struct ReadHomeFileTool;
impl Tool for ReadHomeFileTool {
    fn name(&self) -> &str { "read_home_file" }
}
```

**`crates/mika-agent/src/tools/list_home_files.rs`** (was `list_files.rs`):
```rust
// Before:
pub struct ListFilesTool;
impl Tool for ListFilesTool {
    fn name(&self) -> &str { "list_files" }
}

// After:
pub struct ListHomeFilesTool;
impl Tool for ListHomeFilesTool {
    fn name(&self) -> &str { "list_home_files" }
}
```

**`crates/mika-agent/src/tools/mod.rs`** — updated module declarations and registration:
```rust
// Before:
mod read_file;
mod list_files;
pub fn default_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(read_file::ReadFileTool),
        Box::new(list_files::ListFilesTool),
        // ...
    ]
}

// After:
mod read_home_file;
mod list_home_files;
pub fn default_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(read_home_file::ReadHomeFileTool),
        Box::new(list_home_files::ListHomeFilesTool),
        // ...
    ]
}
```

### Finding All References

After renaming, search for all hardcoded tool name strings and update them:

```bash
# System prompt references
grep -rn '"read_file"\|"list_files"' crates/mika-agent/src/prompt.rs

# Any silent prompt or agent context structs
grep -rn '"read_file"\|"list_files"' crates/mika-agent/src/agent.rs

# Documentation and OpenAPI specs
grep -rn 'read_file\|list_files' docs/ CLAUDE.md
```

### Also Fixed in the Same Review

**`validate_and_resolve_path` gained `create_parents: bool`** — the helper was silently
creating parent directories even for read-only tools. Write tools pass `true`; read-only
tools pass `false`:

```rust
// Write tools (create dirs if needed):
let resolved = validate_and_resolve_path(path, home_dir, true)?;

// Read-only tools (no dir creation):
let resolved = validate_and_resolve_path(path, home_dir, false)?;
```

**`list_home_files` blocking I/O** — `std::fs::read_dir` was called from `async fn`
without `spawn_blocking`, blocking the Tokio executor:

```rust
// Before (blocks tokio worker thread):
let entries = collect_entries(&resolved)?;

// After (correct):
let entries = tokio::task::spawn_blocking(move || collect_entries(&resolved))
    .await??;
```

**AsyncDB mutex held across blocking send** — `with_db` was holding the async mutex while
calling `SyncSender::send()`, creating a deadlock risk under concurrent access:

```rust
// Before (holds mutex across blocking send):
let guard = self.inner.lock().await;
guard.sender.send(closure)?;

// After (releases mutex before send):
let sender = {
    let guard = self.inner.lock().await;
    guard.sender.clone()
}; // mutex released here
sender.send(closure)?;
```

**Backup failure aborts migration** — `Database::open()` was proceeding with a
destructive drop-all migration even after a backup failure:

```rust
// Before (warn and proceed — data loss risk):
Err(e) => tracing::warn!(error = %e, "could not backup — proceeding anyway"),

// After (hard abort):
Err(e) => return Err(anyhow::anyhow!("backup failed: {e} — aborting migration")),
```

**Full UUID in reminder output** — `list_reminders` showed 8-char prefixes but
`cancel_reminder` required full UUIDs, making cancellation impossible:

```rust
// Before:
format!("  #{}: {} (due: {})", &reminder.id[..8], reminder.message, ...)

// After:
format!("  {}: {} (due: {})", reminder.id, reminder.message, ...)
```

## Step-by-Step Fix for Future Occurrences

1. **Detect collisions.** After building `default_tools()`, assert no duplicate names:
   ```rust
   #[test]
   fn no_duplicate_tool_names_in_default_tools() {
       let tools = default_tools();
       let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
       let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
       assert_eq!(names.len(), unique.len(),
           "Duplicate tool names: {:?}", names);
   }
   ```

2. **Apply domain prefix to new builtins.** Tools scoped to agent-private storage use
   a qualifying infix: `read_home_file`, `list_home_files`, `write_home_file`. Generic
   verb-noun names (`read_file`, `list_files`) are reserved for skill and MCP tools.

3. **Rename the colliding builtin** — update the `.rs` filename, struct name (PascalCase),
   `fn name()` return value, `mod` declaration, `Box::new(...)` in `mod.rs`, all
   `prompt.rs` references, and documentation.

4. **Run `cargo test`** — the dedup test and any prompt-parsing tests will surface
   remaining references.

## Prevention

### Naming Convention

| Scope | Convention | Example |
|-------|-----------|---------|
| Agent home directory | `_home_` infix | `read_home_file`, `list_home_files` |
| Core memory | `memory_` prefix already used | `update_core_memory`, `search_memory` |
| Generic filesystem (skill/MCP) | plain name | `read_file` (skill only) |
| MCP tools | auto-namespaced | `mcp__{server}__{tool}` |

### Registration-Time Dedup Check

Add to any function that assembles the final tool list:
```rust
let mut seen = std::collections::HashSet::new();
for tool in &all_tools {
    assert!(seen.insert(tool.name()),
        "Duplicate tool name in dispatch chain: {}", tool.name());
}
```

### Checklist for Adding a New Builtin Tool

- [ ] Search for the proposed `fn name()` string across skill `skill.toml` files and MCP
      tool patterns (`mcp__{server}__{tool}`). No match must exist.
- [ ] Use a domain-scoped name if operating on agent-private storage.
- [ ] Add the name to a `BUILTIN_TOOL_NAMES` constant for documentation and installer
      validation.
- [ ] Update the system prompt text (`prompt.rs`) to use the new name.
- [ ] Grep for any hardcoded references in `agent.rs`, `CLAUDE.md`, and docs.
- [ ] Verify `cargo test` passes — the dedup test will catch missed registrations.

### Detecting Blocking I/O in Async Tool Implementations

```bash
# Find synchronous filesystem calls in tool implementations:
grep -rn 'std::fs::' crates/mika-agent/src/tools/ \
  | grep -v 'spawn_blocking\|#\[cfg(test)\]'
```

Any match is a candidate for `tokio::task::spawn_blocking` wrapping or migration to
`tokio::fs`.

### UUID Output Consistency

When a tool outputs an entity identifier (reminder ID, task ID, fact ID), always read it
from the DB row — never generate a new UUID at output time. Write a round-trip test:

```rust
#[tokio::test]
async fn create_and_cancel_reminder_use_same_uuid() {
    let db = test_db().await;
    let create_out = create_reminder_tool.invoke(&db, ...).await.unwrap();
    let uuid = extract_uuid(&create_out);
    // cancel using the same UUID — must succeed
    cancel_reminder_tool.invoke(&db, &uuid).await.unwrap();
}
```

## Related Documentation

- [write-file-tool-overwrite-confirmation-flow.md](../integration-issues/write-file-tool-overwrite-confirmation-flow.md) — same file tool family; `validate_and_resolve_path()` helper used by all five file tools
- [tool-path-reporting-misbehavior.md](../logic-errors/tool-path-reporting-misbehavior.md) — follow-up: absolute path reporting rule applies to all file tools
- [adding-prompt-only-bundled-skill.md](../integration-issues/adding-prompt-only-bundled-skill.md) — establishes skill registration checklist including keyword collision checks
- [agent-team-management-tools-integration.md](../integration-issues/agent-team-management-tools-integration.md) — establishes `Tool::timeout_secs()` pattern and conditional tool registration
