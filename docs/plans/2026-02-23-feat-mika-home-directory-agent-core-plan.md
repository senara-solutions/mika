---
title: "feat: Implement ~/.mika/ home directory and agent core systems"
type: feat
status: active
date: 2026-02-23
brainstorm: docs/brainstorms/2026-02-23-mika-home-directory-agent-core-brainstorm.md
---

# feat: Implement ~/.mika/ Home Directory & Agent Core Systems

## Overview

Transform Mika from a working-directory-relative CLI into a home-directory-based personal assistant. Create `~/.mika/` as the per-customer agent boundary containing human-editable persona files and an encrypted SQLite database. Implement six core systems: home directory bootstrap, identity/soul loading, core memory redesign (4 restricted blocks with per-block limits), memory audit log, system prompt overhaul, and Phase 1 heartbeat foundation.

## Problem Statement / Motivation

Mika currently stores its database relative to the working directory (`mika.db` in CWD) and has hardcoded personality in `prompt.rs`. There's no home directory, no way to customize the agent's personality via files, no audit trail for memory mutations, and core memory keys are unrestricted. The reference assistants (OpenClaw, LettaBot) both use home directories with persona files — Mika needs this foundation before Phase 2 (channels, K8s containers).

## Proposed Solution

Six implementation phases, each building on the previous. The phases are designed to be independently testable — each phase produces working code with passing tests.

## Technical Approach

### Architecture

```
~/.mika/                          (or $MIKA_HOME)
├── config.toml                   # User config overrides
├── identity.toml                 # Agent name, emoji
├── soul.md                       # Personality baseline (read-only to agent)
├── heartbeat.md                  # Heartbeat checklist
├── user.md                       # User context seed
├── data/
│   └── mika.db                   # Encrypted SQLite (all runtime data)
└── logs/
    └── mika.log                  # Tracing output (file appender)
```

**Config cascade:** `config/default.toml` (bundled) → `~/.mika/config.toml` (user) → `MIKA_*` env vars (highest priority)

**Secrets policy:** `anthropic_api_key` and `encryption_key` remain env-var-only. `config.toml` template includes comments explaining this, never secret values.

**Path override:** `MIKA_HOME` env var overrides `~/.mika/`. Essential for K8s containers and testing.

### Key Design Decisions (from brainstorm, with gap resolutions)

| Decision | Choice | Rationale |
|---|---|---|
| Bootstrap detection | Check for `~/.mika/data/mika.db` existence | Handles partial directory states (Gap Q1) |
| Onboarding completion | `onboarding_complete` flag in `core_memory` table | Set after first session ends, regardless of exchange count (Gap Q2) |
| Secrets in config.toml | No — env-var-only | Encryption key next to database negates encryption (Gap Q3) |
| Per-block token limit | Hard-enforce 500 tokens per block in tool code | Prevents single block from consuming budget (Gap Q4) |
| `reasoning` parameter | Required on `update_core_memory`, optional on `store_fact` | Agent must explain core memory edits for audit (Gap Q5) |
| Existing tools vs store_fact | Keep existing tools, add `store_fact`/`update_fact` as wrappers | `store_fact` routes to existing DB methods by category (Gap Q6) |
| Core memory keys | Restrict to 4 defined keys | Remove "custom keys" from tool description (Gap Q11) |
| Conversation ID | UUID generated at session start, passed via `ToolContext` | Needed for audit log and rate limiting (Gap Q15) |
| Rate limit scope | Per CLI session (in-memory counter), 3 edits max | Simple, no DB overhead (Gap Q12) |
| `soul.md` position | Replaces hardcoded preamble, first section of prompt | Maximum influence on agent behavior (Gap Q9) |
| File change detection | Read once at session start | Acceptable for Phase 1 CLI (Gap Q28) |
| File permissions | `~/.mika/` at 0700, files at 0600 | Prevent other users from reading (Gap Q30) |

### Implementation Phases

---

#### Phase 1: Home Directory Foundation

**Goal:** `~/.mika/` directory discovery, creation, and config integration.

**New dependency:** `dirs` crate (home directory discovery, cross-platform)

**Files to modify:**

- `crates/mika-common/Cargo.toml` — add `dirs = "6"`
- `crates/mika-common/src/config.rs` — new config source layer, `MIKA_HOME` support
- `crates/mika-common/src/lib.rs` — export new `home` module
- `crates/mika-agent/src/cli.rs` — bootstrap logic on startup

**New files:**

- `crates/mika-common/src/home.rs` — home directory discovery and initialization

**`home.rs` design:**

```rust
// crates/mika-common/src/home.rs

use std::path::{Path, PathBuf};

/// Resolve the Mika home directory.
/// Priority: $MIKA_HOME > ~/.mika/
pub fn resolve_home_dir() -> anyhow::Result<PathBuf> {
    if let Ok(custom) = std::env::var("MIKA_HOME") {
        return Ok(PathBuf::from(custom));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    Ok(home.join(".mika"))
}

/// Check if Mika has been initialized (database exists).
pub fn is_initialized(home_dir: &Path) -> bool {
    home_dir.join("data").join("mika.db").exists()
}

/// Create the ~/.mika/ directory structure with default files.
/// Sets permissions to 0700 for directory, 0600 for files.
pub fn bootstrap(home_dir: &Path) -> anyhow::Result<()> {
    // Create directories
    std::fs::create_dir_all(home_dir.join("data"))?;
    std::fs::create_dir_all(home_dir.join("logs"))?;

    // Write default files (only if they don't exist)
    write_default_if_missing(home_dir, "config.toml", DEFAULT_CONFIG)?;
    write_default_if_missing(home_dir, "identity.toml", DEFAULT_IDENTITY)?;
    write_default_if_missing(home_dir, "soul.md", DEFAULT_SOUL)?;
    write_default_if_missing(home_dir, "heartbeat.md", DEFAULT_HEARTBEAT)?;
    write_default_if_missing(home_dir, "user.md", DEFAULT_USER)?;

    // Set permissions (Unix only)
    #[cfg(unix)]
    set_permissions(home_dir)?;

    Ok(())
}
```

**Default file contents (constants in `home.rs`):**

- `DEFAULT_CONFIG` — minimal TOML with comments: `# Secrets (anthropic_api_key, encryption_key) must be set via MIKA_* env vars`, plus `claude_model`, `claude_max_tokens`, `log_level` with defaults
- `DEFAULT_IDENTITY` — `name = "Mika"\nemoji = "✦"`
- `DEFAULT_SOUL` — the full opinionated personality from the brainstorm (professional, proactive, concise exec assistant)
- `DEFAULT_HEARTBEAT` — the checklist from the brainstorm
- `DEFAULT_USER` — `# Tell Mika about yourself\n\nEdit this file with your name, role, preferences, and anything you'd like Mika to know about you.`

**Config changes (`config.rs`):**

```rust
pub fn load() -> anyhow::Result<Self> {
    let home_dir = crate::home::resolve_home_dir()?;
    let home_config = home_dir.join("config.toml");

    let settings = Config::builder()
        .add_source(File::with_name("config/default").required(false))
        .add_source(File::from(home_config).required(false))  // NEW
        .add_source(
            Environment::with_prefix("MIKA")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?
        .try_deserialize()?;
    Ok(settings)
}
```

**`db_path` default resolution:** The home dir is resolved BEFORE `Settings::load()`. Pass the resolved `home_dir` into `Settings::load(home_dir)` so it can set the default `db_path` to `{home_dir}/data/mika.db`. This avoids a chicken-and-egg: config needs to know the home dir for default paths, but the home dir must be resolved before config loads. Add `home_dir: PathBuf` to `Settings` (populated from the argument, not from config file).

**CLI changes (`cli.rs`):**

```rust
let home_dir = home::resolve_home_dir()?;
let is_first_run = !home::is_initialized(&home_dir);

if is_first_run {
    home::bootstrap(&home_dir)?;
    println!("\n  ✦ Mika initialized at {}\n", home_dir.display());
}

let settings = Settings::load()?;
// ... rest of initialization
```

**Tests:**
- `home::resolve_home_dir()` with and without `MIKA_HOME`
- `home::bootstrap()` creates all expected files and directories
- `home::is_initialized()` returns false before bootstrap, true after
- `Settings::load()` picks up `~/.mika/config.toml` values (use temp dir + `MIKA_HOME`)
- File permissions are 0700/0600 on Unix

---

#### Phase 2: Database Migration v2 & Session ID

**Goal:** New `memory_events` table, rename `current_goals` → `current_priorities`, add `key_people` block, introduce session/conversation ID.

**Files to modify:**

- `crates/mika-agent/src/db.rs` — migration v2, new methods, session ID support

**Migration v2 SQL:**

```sql
-- Add memory_events audit log
CREATE TABLE IF NOT EXISTS memory_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    target_key TEXT NOT NULL,
    before_value_encrypted BLOB,
    after_value_encrypted BLOB NOT NULL,
    reasoning_encrypted BLOB,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_memory_events_session ON memory_events(session_id);
CREATE INDEX idx_memory_events_target ON memory_events(target_key);

-- Rename current_goals to current_priorities (data migration)
UPDATE core_memory SET key = 'current_priorities' WHERE key = 'current_goals';
```

**New DB methods:**

```rust
/// Log a memory mutation event for auditability.
pub fn log_memory_event(
    &self,
    session_id: &str,
    tool_name: &str,
    target_key: &str,
    before: Option<&str>,
    after: &str,
    reasoning: Option<&str>,
) -> Result<()>

/// Seed core memory with 4 blocks (updated from 3).
/// If user.md content is provided, use it for user_summary instead of default.
pub fn seed_core_memory(&self, user_md_content: Option<&str>) -> Result<()>
```

**Updated seed_core_memory:**

```rust
pub fn seed_core_memory(&self, user_md_content: Option<&str>) -> Result<()> {
    let user_summary = user_md_content.unwrap_or("New user. No information yet.");
    self.set_core_memory("user_summary", user_summary)?;
    self.set_core_memory("persona", "Mika — personal AI executive assistant.")?;
    self.set_core_memory("current_priorities", "Get to know the user and understand their needs.")?;
    self.set_core_memory("key_people", "No one tracked yet.")?;
    Ok(())
}
```

**Session ID:** Generated as UUID v4 in `cli.rs` at process start. Passed through to `ToolContext`.

**Tests:**
- Migration v2 runs cleanly on fresh DB
- Migration v2 runs on existing v1 DB, preserving data
- `current_goals` renamed to `current_priorities` with data preserved
- `log_memory_event` writes encrypted before/after values
- `seed_core_memory` creates 4 blocks
- `seed_core_memory` uses `user.md` content when provided

---

#### Phase 3: Core Memory Redesign & Audit Log

**Goal:** Restrict core memory to 4 keys, add per-block 500-token limit, add `action` parameter (replace|append|remove_line), add `reasoning` parameter, wire audit logging, add rate limiting.

**Files to modify:**

- `crates/mika-agent/src/tools/update_core_memory.rs` — complete rewrite
- `crates/mika-agent/src/tools/mod.rs` — expand `ToolContext`

**ToolContext expansion:**

```rust
pub struct ToolContext<'a> {
    pub db: &'a Database,
    pub session_id: &'a str,
    pub home_dir: &'a Path,
    pub core_memory_edit_count: &'a std::sync::atomic::AtomicU32,
    pub is_onboarding: bool,
}
```

**Updated `update_core_memory` tool:**

```rust
const ALLOWED_SECTIONS: &[&str] = &["persona", "user_summary", "current_priorities", "key_people"];
const MAX_TOKENS_PER_BLOCK: i32 = 500;
const MAX_CORE_MEMORY_EDITS_PER_SESSION: u32 = 3;

// Tool schema:
{
    "section": { "type": "string", "enum": ["persona", "user_summary", "current_priorities", "key_people"] },
    "action": { "type": "string", "enum": ["replace", "append", "remove_line"] },
    "content": { "type": "string", "description": "New content (for replace/append) or line to remove (for remove_line)" },
    "reasoning": { "type": "string", "description": "Why you are making this change (for audit trail)" }
}
// required: ["section", "action", "content", "reasoning"]
```

**Action implementations:**

- `replace` — full block replacement (current behavior)
- `append` — load existing value, append `\n{content}`, check per-block limit
- `remove_line` — load existing value, find line containing `content` (case-insensitive substring), remove first matching line. Return error if no match found.

**Rate limit:**
- Check `core_memory_edit_count.load()` before executing
- If >= `MAX_CORE_MEMORY_EDITS_PER_SESSION` AND `is_onboarding` is false, return error: "Core memory edit limit (3) reached for this session. Focus on using your existing knowledge."
- Onboarding sessions are exempt from the rate limit (agent needs 4+ edits to seed all blocks)
- Increment counter after successful edit
- Counter stored as `AtomicU32` in `ToolContext`, zeroed at session start

**Audit logging:**
- After every successful mutation, call `db.log_memory_event(session_id, "update_core_memory", section, before_value, after_value, reasoning)`
- `before_value` is the existing block content (None for new blocks)

**Tests:**
- Section validation rejects unknown keys
- `replace` action replaces entire block
- `append` action appends to existing content
- `append` rejected when result exceeds 500 tokens
- `remove_line` removes matching line
- `remove_line` returns error when no match
- Rate limit triggers after 3 edits in a session
- Audit event logged for each mutation
- Per-block 500 token limit enforced

---

#### Phase 4: System Prompt Overhaul

**Goal:** Load `soul.md` and `identity.toml` from `~/.mika/`, replace hardcoded personality, inject onboarding prompt on first run.

**New dependency:** `toml` (for `identity.toml` parsing) — likely already transitive via config-rs, but add explicit dependency if needed.

**Files to modify:**

- `crates/mika-agent/src/prompt.rs` — complete rewrite
- `crates/mika-agent/src/agent.rs` — pass `PromptContext` instead of bare `db`

**New types:**

```rust
// crates/mika-agent/src/prompt.rs

#[derive(Debug, Deserialize)]
pub struct Identity {
    pub name: String,
    #[serde(default = "default_emoji")]
    pub emoji: String,
}

pub struct PromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub is_onboarding: bool,
    pub heartbeat_md: Option<&'a str>,  // Only for heartbeat mode
}
```

**New prompt structure:**

```
[soul.md content]

## Identity
You are {identity.name}.

## Core Memory
These are your persistent memory blocks. Update them using the update_core_memory tool.

### persona
{persona block content}

### user_summary
{user_summary block content}

### current_priorities
{current_priorities block content}

### key_people
{key_people block content}

## Instructions
- Update your core memory when you learn important things about the user.
- Track people, commitments, preferences, and events using the appropriate tools.
- Never fabricate information. If you don't know something, say so.
- You have 4 memory blocks (persona, user_summary, current_priorities, key_people),
  each limited to ~500 tokens. Be concise and prioritize what matters most.

[ONBOARDING_PROMPT if is_onboarding]
```

**Onboarding prompt (injected at end):**

```
## First Session
This is your first conversation with the user. Introduce yourself briefly and warmly.
Ask who they are and what they're working on. Use update_core_memory to seed all
four blocks (persona, user_summary, current_priorities, key_people) from their
responses. Keep it to 2-3 natural exchanges, then transition to being helpful
with whatever they need.
```

**Onboarding detection:** Check if `user_summary` still equals its seed value ("New user. No information yet."). If so, inject the onboarding prompt. This avoids needing a 5th core memory key or a separate table — the seed value IS the completion marker. Once the agent updates `user_summary` during onboarding, the next session starts in normal mode.

**Agent.rs changes:**

```rust
// At session start (once, not per loop step):
let soul_content = std::fs::read_to_string(home_dir.join("soul.md")).unwrap_or_default();
let identity = prompt::load_identity(home_dir)?;
let core_memory = db.get_all_core_memory()?;
let is_onboarding = core_memory.iter()
    .find(|e| e.key == "user_summary")
    .map(|e| e.value == "New user. No information yet.")
    .unwrap_or(true);

let ctx = prompt::PromptContext {
    soul_content: &soul_content,
    identity: &identity,
    core_memory: &core_memory,
    is_onboarding,
    heartbeat_md: None,
};
let system = prompt::build_system_prompt(&ctx);
```

**Tests:**
- Prompt includes soul.md content at the beginning
- Prompt includes all 4 core memory blocks
- Identity name appears in prompt
- Onboarding prompt injected when user_summary equals seed value
- Onboarding prompt NOT injected for returning user
- `load_identity` parses valid TOML
- `load_identity` returns defaults if file missing

---

#### Phase 5: Tool Consolidation (store_fact, update_fact, search_memory)

**Goal:** Add unified fact management tools with audit logging. Keep existing tools as internal implementations.

**Files to modify:**

- `crates/mika-agent/src/tools/mod.rs` — register new tools, update `default_tools()`
- `crates/mika-agent/src/db.rs` — add `update_fact`, `search_facts` methods

**New files:**

- `crates/mika-agent/src/tools/store_fact.rs`
- `crates/mika-agent/src/tools/update_fact.rs`
- `crates/mika-agent/src/tools/search_memory.rs`

**`store_fact` tool:**

```rust
// Schema:
{
    "category": { "type": "string", "enum": ["person", "commitment", "preference", "event"] },
    "name": { "type": "string", "description": "Person's name (person category only)" },
    "relationship": { "type": "string", "description": "Relationship to user (person)" },
    "notes": { "type": "string", "description": "Additional notes (person)" },
    "description": { "type": "string", "description": "What the commitment/event is (commitment, event)" },
    "due_date": { "type": "string", "description": "ISO date (commitment, event)" },
    "key": { "type": "string", "description": "Preference category (preference)" },
    "value": { "type": "string", "description": "Preference value (preference)" },
    "reasoning": { "type": "string", "description": "Why you are storing this fact" }
}
// required: ["category"]
// Required per category:
//   person: name
//   commitment: description
//   preference: key, value
//   event: description
```

Routes to existing DB methods by category:
- `"person"` → `db.upsert_person(name, relationship, notes)`
- `"commitment"` → `db.add_commitment(description, due_date, person_id)`
- `"preference"` → `db.set_preference(key, value)`
- `"event"` → `db.add_event(description, due_date, notes)`

Validates required fields per category before routing. Returns `ToolOutput::error` if missing.

Logs audit event after successful storage.

**`update_fact` tool:**

```rust
// Schema:
{
    "category": { "type": "string", "enum": ["person", "commitment", "preference", "event"] },
    "id": { "type": "integer", "description": "Row ID of the fact to update" },
    "updates": { "type": "object", "description": "Fields to update" },
    "reasoning": { "type": "string", "description": "Why you are updating this fact" }
}
```

Category-specific update routing:
- `"commitment"` → `db.update_commitment_status(id, status)` (most common: mark completed/cancelled)
- `"person"` → new `db.update_person(id, updates)` method
- `"preference"` → `db.set_preference(category, value)` (upsert by category)
- `"event"` → new `db.update_event(id, updates)` method

Captures before state, logs audit event.

**`search_memory` tool:**

```rust
// Schema:
{
    "query": { "type": "string", "description": "Search term" },
    "category": { "type": "string", "enum": ["all", "person", "commitment", "preference", "event", "core_memory"], "default": "all" }
}
```

Phase 1 implementation: decrypt-and-scan. For each table matching the category filter, load all rows, decrypt text fields, check if any contain the query (case-insensitive substring). Return matching rows with their IDs and content.

This is O(n) and will need FTS5/vector replacement in Layer 3, but is correct for Phase 1 with small datasets.

**Tool registry update:**

```rust
pub fn default_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(update_core_memory::UpdateCoreMemoryTool));
    registry.register(Box::new(store_fact::StoreFactTool));
    registry.register(Box::new(update_fact::UpdateFactTool));
    registry.register(Box::new(search_memory::SearchMemoryTool));
    registry
}
```

Remove the old individual tools (`upsert_person`, `add_commitment`, `set_preference`) from the registry. Keep their DB methods — `store_fact` delegates to them.

**Tests:**
- `store_fact` routes each category correctly
- `store_fact` logs audit event
- `update_fact` updates commitment status
- `update_fact` logs audit event with before/after
- `search_memory` finds matching person by name
- `search_memory` finds matching commitment by description
- `search_memory` returns empty for no matches
- `search_memory` filters by category

---

#### Phase 6: Heartbeat Foundation & CLI Commands

**Goal:** Heartbeat data model, heartbeat prompt assembly, `/heartbeat` CLI command, `/reset` CLI command, user.md re-seed mechanism.

**Files to modify:**

- `crates/mika-agent/src/db.rs` — heartbeat metadata table (migration v3)
- `crates/mika-agent/src/prompt.rs` — heartbeat prompt assembly
- `crates/mika-agent/src/cli.rs` — slash command handling
- `crates/mika-agent/src/agent.rs` — heartbeat mode support

**Migration v3 SQL:**

```sql
CREATE TABLE IF NOT EXISTS heartbeat_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    result TEXT NOT NULL,            -- 'no_action' or 'message_sent'
    summary_encrypted BLOB,         -- brief summary of what was evaluated
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

**Heartbeat prompt assembly (`prompt.rs`):**

```rust
pub fn build_heartbeat_prompt(ctx: &PromptContext, heartbeat_md: &str) -> String {
    // soul.md + identity + core memory + heartbeat-specific instructions
    // Injects: current local time, heartbeat checklist, pending commitments
    // NO_ACTION contract: respond with exactly "NO_ACTION" if nothing warrants contact
}
```

**CLI slash commands (in `cli.rs` REPL loop):**

```rust
if input.starts_with('/') {
    match input {
        "/heartbeat" => {
            // Run agent in heartbeat mode
            // Load heartbeat.md, build heartbeat prompt
            // Check response for "NO_ACTION"
            // If not NO_ACTION, display the message
        }
        s if s.starts_with("/reset ") => {
            let block = s.strip_prefix("/reset ").unwrap().trim();
            // Validate block is one of the 4 allowed keys
            // Delete block from core_memory table
            // If block is "user_summary", re-seed from user.md
            // Otherwise re-seed with default value
        }
        "/memory" => {
            // Display all core memory blocks
        }
        "/help" => {
            // Show available commands
        }
        _ => println!("Unknown command: {input}. Type /help for available commands.")
    }
    continue;
}
```

**Heartbeat mode in agent:**

The heartbeat runs through the same `run_agent` function but with a different system prompt (heartbeat prompt instead of normal prompt) and a synthetic user message ("Heartbeat check"). The response is checked for the literal string "NO_ACTION" — if present, the heartbeat is logged as `no_action` and nothing is displayed.

**Tests:**
- Heartbeat prompt includes soul.md, core memory, and checklist
- Heartbeat prompt includes timezone/time context
- `NO_ACTION` response detected and suppressed
- Non-NO_ACTION response displayed
- `/reset user_summary` deletes block and re-seeds from user.md
- `/reset persona` deletes block and re-seeds with default
- `/reset invalid_key` returns error
- `/memory` displays all 4 blocks
- Heartbeat log written to DB

## Acceptance Criteria

### Functional Requirements

- [ ] `~/.mika/` directory created on first run with all default files
- [ ] `✦ Mika initialized at ~/.mika/` printed on first run
- [ ] Config loads from `~/.mika/config.toml` with correct precedence
- [ ] `MIKA_HOME` env var overrides `~/.mika/` path
- [ ] Database created at `~/.mika/data/mika.db`
- [ ] v1 → v2 migration renames `current_goals` to `current_priorities`
- [ ] v2 migration adds `memory_events` table
- [ ] Core memory restricted to 4 keys: persona, user_summary, current_priorities, key_people
- [ ] Per-block 500-token limit enforced
- [ ] `update_core_memory` supports replace, append, remove_line actions
- [ ] `reasoning` required on core memory edits
- [ ] Rate limit: max 3 core memory edits per session
- [ ] MemoryEvent audit log written for every memory mutation
- [ ] `soul.md` content loaded into system prompt (replaces hardcoded personality)
- [ ] `identity.toml` loaded for agent name
- [ ] Onboarding prompt injected on first conversation
- [ ] Onboarding detected by seed value in `user_summary` block
- [ ] `store_fact` routes to correct table by category
- [ ] `update_fact` updates existing facts with audit
- [ ] `search_memory` finds matching facts across tables
- [ ] `/heartbeat` CLI command triggers heartbeat evaluation
- [ ] `/reset <block>` deletes and re-seeds a core memory block
- [ ] `/memory` displays current core memory state
- [ ] File permissions set to 0700 (dir) / 0600 (files) on Unix

### Non-Functional Requirements

- [ ] All existing 32 tests pass (no regressions)
- [ ] New tests for each phase (target: ~50+ total tests)
- [ ] `cargo clippy` clean
- [ ] No secrets in `~/.mika/config.toml` template
- [ ] Graceful error if home directory cannot be created

### Quality Gates

- [ ] `cargo test` passes after each phase
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo fmt --check` passes

## Dependencies & Prerequisites

- `dirs` crate v6 (home directory discovery)
- `uuid` crate (session ID generation) — check if already a dependency
- Existing `ring`, `rusqlite`, `config`, `tokio`, `tracing` dependencies unchanged

## Risk Analysis & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| v1→v2 migration data loss | Low | High | Test migration on copy of real DB; `current_goals` rename is safe SQL UPDATE |
| search_memory O(n) scan too slow | Low (Phase 1) | Medium | Acceptable for small datasets; FTS5 in Layer 3 resolves this |
| Rate limit too restrictive | Medium | Low | 3 per session is a starting point; easy to adjust or make configurable |
| `remove_line` matching ambiguity | Medium | Low | Case-insensitive substring match, first match wins; agent can retry with more specific string |
| Tests writing to real ~/.mika/ | Low | High | All tests MUST use `MIKA_HOME` to point to temp directories |

## References & Research

### Internal References

- Brainstorm: `docs/brainstorms/2026-02-23-mika-home-directory-agent-core-brainstorm.md`
- Current config: `crates/mika-common/src/config.rs`
- Current DB: `crates/mika-agent/src/db.rs`
- Current prompt: `crates/mika-agent/src/prompt.rs`
- Current CLI: `crates/mika-agent/src/cli.rs`
- Current agent loop: `crates/mika-agent/src/agent.rs`
- Current tools: `crates/mika-agent/src/tools/`
- Current encryption: `crates/mika-common/src/crypto.rs`
- Pending todo: `todos/027-pending-p1-sync-sqlite-blocking-tokio.md`

### External References

- OpenClaw architecture: `/home/samidarko/workspace/senara-solutions/openclaw/` — workspace files pattern, heartbeat design
- LettaBot architecture: `/home/samidarko/workspace/senara-solutions/lettabot/` — memory blocks, silent heartbeat, onboarding
- `dirs` crate: https://docs.rs/dirs/latest/dirs/
