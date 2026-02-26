---
title: "TUI cross-channel polling for mika ask, bundled skill update propagation, config tools"
date: 2026-02-26
module:
  - mika-cli (TUI cross-channel polling, /config set refactor)
  - mika-agent (bundled_skills seeder, config_keys, get_config/set_config tools, DB query bounds)
severity: medium
tags:
  - tui
  - cross-channel-polling
  - mika-ask
  - bundled-skills
  - skill-seeder
  - symlink-guard
  - security-hardening
  - config-tools
  - agent-native-parity
  - unbounded-query
  - code-review
symptoms:
  - "Messages sent via `mika ask` from a separate terminal do not appear in a concurrently running TUI session"
  - "Bundled skill template changes (e.g. removing search_docs) are not propagated to existing installs"
  - "The agent has no tools to read or modify customer config (timezone, chat_id)"
  - "load_messages_after DB query has no LIMIT clause (unbounded result set)"
  - "write_skill follows symlinks when overwriting bundled skill files"
root_cause:
  - "POLLED_CHANNELS only contained ['telegram'], excluding 'cli' messages from other processes"
  - "seed_bundled_skills skipped existing directories, so template updates never propagated"
  - "No agent tools for config read/write; validation duplicated in CLI handler"
---

# TUI cross-channel polling for `mika ask`, bundled skill updates, config tools

## Problem

Running `mika ask "hello"` from a separate terminal works (agent responds), but the message and response never appear in a concurrently running TUI session. The user expects cross-session visibility, similar to how Telegram messages already appear via polling.

Secondary issues discovered during implementation and code review:

1. **Stale bundled skills:** The seeder (`seed_bundled_skills`) skipped existing directories, so removing `search_docs` from templates didn't update existing installs.
2. **Symlink vulnerability:** The always-overwrite fix opened a symlink-following attack vector in `write_skill`.
3. **Unbounded poll query:** `load_messages_after` had no `LIMIT` clause.
4. **Agent-native parity gap:** `/config set` and `/config` were CLI-only with no agent tool equivalents.

## Root Cause Analysis

### 1. Cross-channel polling excluded CLI messages

`POLLED_CHANNELS` was `&["telegram"]`. The TUI's `poll_cross_channel_messages` filtered on these channels only, so messages with `channel_type = "cli"` (written by `mika ask`) were never returned.

The original exclusion was intentional -- avoid re-displaying the TUI's own messages. But the **watermark mechanism** (`last_seen_msg_id`) already prevents self-re-display: after each agent turn, the watermark is bumped to `max_message_id()`, so the TUI's own messages are always below the watermark at the next poll.

Additionally, `chat.rs` history loader manually prepended `"cli"` via `std::iter::once("cli").chain(POLLED_CHANNELS)`, creating an inconsistency between history loading and live polling.

### 2. Seed-skip-if-exists prevented template updates

```rust
// Old behavior
if skill_dir.exists() {
    continue;  // Never updates existing installs
}
```

This was correct when bundled skills were immutable after first seed, but became a bug once templates needed to evolve. The `search_docs` tool removal from the self-knowledge skill was the immediate trigger.

### 3. No symlink protection on write path

`write_skill()` called `std::fs::write()` and `std::fs::create_dir_all()` without checking for symlinks. With always-overwrite semantics, a symlinked skill directory would redirect writes (including executable handler scripts with mode 0o700) to arbitrary locations.

### 4. Unbounded poll query

```sql
-- Old: no LIMIT
SELECT id, role, content, channel_type, created_at
FROM conversations WHERE id > ?1 AND channel_type IN (...)
ORDER BY id ASC
```

Under normal usage returns 0-2 rows, but adversarial input (scripted rapid `mika ask`) could produce unbounded result sets.

### 5. Duplicated config validation, no agent tools

The `/config set` CLI handler had inline `SETTABLE_CONFIG_KEYS`, `chat_id` i64 parsing, and `timezone` chrono_tz validation. No agent tool existed to call this logic programmatically.

## Solution

### Fix 1: Include "cli" in cross-channel polling

**File:** `crates/mika-cli/src/tui/app.rs`

```rust
// Before
pub const POLLED_CHANNELS: &[&str] = &["telegram"];

// After
pub const POLLED_CHANNELS: &[&str] = &["telegram", "cli"];
```

Added channel badge suppression in `poll_cross_channel_messages` to match the history loader pattern -- CLI messages get `channel: None` (no badge), foreign channels get `Some("telegram")`:

```rust
let channel = if msg.channel_type == "cli" {
    None
} else {
    Some(msg.channel_type.clone())
};
```

**File:** `crates/mika-cli/src/commands/chat.rs`

Removed redundant `std::iter::once("cli")` from history loader -- `POLLED_CHANNELS` now contains `"cli"`:

```rust
// Before: std::iter::once("cli").chain(POLLED_CHANNELS.iter().copied())...
// After:
POLLED_CHANNELS.iter().map(|s| s.to_string()).collect()
```

**Why the watermark prevents self-re-display:**

1. User sends message in TUI -> saved to DB -> agent responds -> reveal completes
2. At reveal-complete: `self.last_seen_msg_id = self.db.max_message_id().await`
3. Next poll tick: `WHERE id > last_seen_msg_id` -> TUI's own messages are past the watermark
4. `mika ask` message written with `id > last_seen_msg_id` -> picked up by next poll

### Fix 2: Always-overwrite bundled skills with symlink guards

**File:** `crates/mika-agent/src/bundled_skills.rs`

Changed `seed_bundled_skills` to always write bundled files:

```rust
pub fn seed_bundled_skills(skills_dir: &Path) {
    for skill in BUNDLED_SKILLS {
        let skill_dir = skills_dir.join(skill.name);
        let is_update = skill_dir.exists();

        if let Err(e) = write_skill(&skill_dir, skill) {
            warn!(skill = skill.name, error = %e, "failed to seed bundled skill");
            if !is_update {
                let _ = std::fs::remove_dir_all(&skill_dir);
            }
        } else if is_update {
            debug!(skill = skill.name, "updated bundled skill");
        } else {
            info!(skill = skill.name, "seeded bundled skill");
        }
    }
}
```

Key behaviors:
- **New install failure:** Partial directory is cleaned up
- **Update failure:** Existing directory is preserved (partial update better than deletion)
- **Extra user files:** Preserved (only bundled files are written)
- **Non-bundled skill directories:** Never touched

Added symlink guards at both directory and file level using `symlink_metadata()` (does not follow symlinks):

```rust
fn write_skill(skill_dir: &Path, skill: &BundledSkill) -> std::io::Result<()> {
    if skill_dir.exists() && skill_dir.symlink_metadata()?.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "skill directory is a symlink, refusing to write",
        ));
    }
    for file in skill.files {
        let file_path = skill_dir.join(file.path);
        // ... create parent dirs ...
        if file_path.exists() && file_path.symlink_metadata()?.file_type().is_symlink() {
            return Err(/* symlink error */);
        }
        std::fs::write(&file_path, file.content)?;
        // ... set permissions ...
    }
    Ok(())
}
```

### Fix 3: Bounded poll query

**File:** `crates/mika-agent/src/db.rs`

Added `LIMIT 100` to both branches of `load_messages_after`. Surplus messages are picked up in subsequent polls via the watermark:

```sql
SELECT id, role, content, channel_type, created_at
FROM conversations
WHERE id > ?1 AND role != 'summary' AND channel_type IN (...)
ORDER BY id ASC
LIMIT 100
```

### Fix 4: Agent config tools with shared validation

**New file:** `crates/mika-agent/src/config_keys.rs` -- shared validation module:
- `SETTABLE_CONFIG_KEYS: &[&str]` allowlist (`"chat_id"`, `"timezone"`)
- `validate_config_value(key, value) -> Result<(), String>` per-key validation
- `is_settable_key(key) -> bool` and `settable_keys_display() -> String` helpers

**New file:** `crates/mika-agent/src/tools/get_config.rs` -- parameterless tool returning all config entries via `ctx.db.list_customer_config()`.

**New file:** `crates/mika-agent/src/tools/set_config.rs` -- tool with `key` (enum) and `value` params. Validates against shared allowlist and per-key validators.

**Modified:** `crates/mika-cli/src/tui/commands/handlers.rs` -- removed inline validation, imports from shared `config_keys` module.

**Modified:** `crates/mika-agent/src/prompt.rs` -- added config tool mention to Tool Usage section.

## Tests Added

| Test | File | What it verifies |
|------|------|-----------------|
| `test_seed_updates_existing_bundled_skills` | `bundled_skills.rs` | Overwrite on re-seed |
| `test_seed_preserves_extra_files_in_bundled_dir` | `bundled_skills.rs` | User files survive re-seed |
| `test_symlinked_skill_dir_is_skipped` | `bundled_skills.rs` | Symlinked dir rejected, target empty |
| `test_symlinked_file_inside_skill_dir_is_rejected` | `bundled_skills.rs` | Symlinked file target not overwritten |
| `config_keys` module tests (8) | `config_keys.rs` | Valid/invalid keys and values |
| `get_config` tool tests (2) | `get_config.rs` | Empty config, populated config |
| `set_config` tool tests (7) | `set_config.rs` | Success paths, all error cases |

Total: 18 new tests. Full suite: 470 tests passing.

## Prevention Strategies

### 1. Channel/filter exclusion bugs

When implementing a filter or exclusion list, document **why** each item is excluded and **what mechanism would break** if it were included. If a separate mechanism (like the watermark) already prevents the undesired behavior, do not add a redundant filter.

**Review checklist:** Does this change introduce a hardcoded filter? Is every excluded value justified? Could a new variant be silently dropped?

### 2. Stale bundled resource / seed-skip-if-exists

Bundled resources that ship with the binary should always be treated as **authoritative**. Any `seed_*` or `init_*` function should overwrite bundled resources on every startup. User-customizable resources should be stored separately or marked to prevent overwriting.

**Review checklist:** Does this seed function skip writing if target exists? How will bundled content updates reach existing installations?

### 3. Symlink/path traversal on file writes

Any code that writes files to a semi-trusted directory must check for symlinks before writing. Use `symlink_metadata()` (not `metadata()`, which follows symlinks) and reject if `is_symlink()`.

**Review checklist:** Does this code write to a path that could be a symlink? Does it use `fs::metadata()` where `fs::symlink_metadata()` is needed?

### 4. Unbounded database queries

Every `SELECT` that returns rows must have a `LIMIT` clause. The limit should be a named constant. Pagination via cursor/watermark handles overflow.

**Review checklist:** Does every new `SELECT` have a `LIMIT`? Is there backpressure for polling queries?

### 5. Agent-native parity

Every user-facing capability should have an agent tool equivalent or an explicit exclusion reason. Validation logic must be shared between CLI handlers and agent tools, never duplicated.

**Review checklist:** Does this PR add a CLI command? Does the agent have a corresponding tool? Is validation shared?

## Cross-References

- **Prior art:** `docs/solutions/ui-bugs/multi-channel-tui-visibility-cross-channel-polling.md` -- original cross-channel polling implementation
- **Skill system architecture:** `docs/solutions/architecture-decisions/filesystem-skill-registry-implementation.md`
- **Skill seeding implementation:** `docs/solutions/integration-issues/cli-telegram-messaging-and-skill-seeding.md`
- **Symlink guard pattern:** `docs/solutions/logic-errors/agent-skill-hallucination-tui-scroll-telegram-awareness.md` (create_skill guard)
- **Channel awareness:** `docs/solutions/logic-errors/agent-cli-self-knowledge-and-skill-triggers.md`
- **Resolved todos:** #289 (config tools), #302 (symlink guard), #303 (LIMIT query), #288 (centralize channel constants), #281 (create_skill symlink guard)

## Files Changed

```
crates/mika-agent/src/bundled_skills.rs    — Always-overwrite + symlink guards + tests
crates/mika-agent/src/config_keys.rs       — NEW: shared config validation
crates/mika-agent/src/db.rs                — LIMIT 100 on load_messages_after
crates/mika-agent/src/lib.rs               — Export config_keys module
crates/mika-agent/src/prompt.rs            — Mention config tools in system prompt
crates/mika-agent/src/tools/get_config.rs  — NEW: agent tool
crates/mika-agent/src/tools/set_config.rs  — NEW: agent tool
crates/mika-agent/src/tools/mod.rs         — Register new tools
crates/mika-cli/src/commands/chat.rs       — Remove redundant "cli" prepend
crates/mika-cli/src/tui/app.rs             — POLLED_CHANNELS + channel badge filter
crates/mika-cli/src/tui/commands/handlers.rs — Use shared config_keys module
```
