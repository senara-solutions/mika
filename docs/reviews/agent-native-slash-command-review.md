# Agent-Native Architecture Review: Slash-Command System

**Branch:** `feat/slash-commands`
**Commit:** `09d8595` — "feat(cli): add slash-command system with autocomplete popup to TUI"
**Review Date:** 2026-02-25
**Reviewer:** Claude Code (Agent-Native Architecture Specialist)

---

## Executive Summary

The slash-command system is a **well-intentioned TUI convenience feature that introduces significant agent-native compliance gaps**. The implementation is client-side only (good isolation), but it:

1. **Breaks action parity:** 7 commands (help, clear, exit, compact, memory, reminders, status) have partial/missing equivalents in the CLI subcommand structure
2. **Hides capabilities from agents:** Commands like `/skills`, `/skill`, `/soul`, `/config`, `/model` are invisible to the agent and cannot be invoked via tools
3. **Creates output silos:** Slash command output is TUI-only (markdown text) with no programmatic formats (JSON, structured data)
4. **Introduces command registration duplication:** Slash commands are defined separately from CLI subcommands with no shared registry
5. **Blocks agent introspection:** The agent cannot discover or execute system health checks, skill introspection, or configuration commands

---

## Capability Map

| UI Action | Location | CLI Equivalent | Agent Tool | System Prompt Doc | Status |
|-----------|----------|---|---|---|---|
| `/help` | handlers.rs:33 | N/A (only in TUI) | None | No | ❌ |
| `/clear` | handlers.rs:51 | N/A (TUI-only state) | None | No | ❌ |
| `/exit` \| `/quit` | handlers.rs:13 | N/A (TUI-only) | None | No | ❌ |
| `/compact` | handlers.rs:57 | N/A (no CLI subcommand) | None | No | ⚠️ |
| `/memory` \| `/mem` | handlers.rs:74 | `mika memory` (split commands) | None | No | ⚠️ |
| `/memory search` | handlers.rs:109 | `mika memory search` | None | No | ⚠️ |
| `/reminders` \| `/remind` | handlers.rs:161 | `mika reminders` (partial) | None | No | ⚠️ |
| `/status` \| `/stat` | handlers.rs:192 | `mika status` | None | No | ✅ Parity |
| `/soul` | handlers.rs:218 | `mika config soul` (partial) | None | No | ⚠️ |
| `/config` \| `/cfg` | handlers.rs:227 | `mika config` (partial) | None | No | ⚠️ |
| `/model` | handlers.rs:255 | N/A (info-only) | None | No | ❌ |
| `/export` | handlers.rs:259 | N/A (TUI feature) | None | No | ❌ |
| `/skills` | handlers.rs:307 | N/A (system feature) | None | No | ❌ |
| `/skill <name>` | handlers.rs:334 | N/A (system feature) | None | No | ❌ |

**Status Legend:**
- ✅ = Full parity (exists in both TUI and CLI with same functionality)
- ⚠️ = Partial parity (exists but command structure or output differs)
- ❌ = Missing parity (TUI-only or CLI-only, agent cannot access)

---

## Critical Issues

### 1. No Agent Access to System Status Tools

**Issue:** The `/status`, `/skills`, `/skill`, and `/model` commands run locally in the TUI without corresponding agent tools.

**Location:**
- `/status` → `handlers.rs:192-216`
- `/skills` → `handlers.rs:307-332`
- `/skill` → `handlers.rs:334-377`
- `/model` → `handlers.rs:255-257`

**Impact:**
- Agent cannot check if it's healthy or overloaded
- Agent cannot discover which skills are available at runtime
- Agent cannot understand what model is being used
- Agent cannot report system state in responses

**Example:**
```
User: "Are you busy? What model are you using?"
Agent: "I don't have tools to check my own status or model. I was configured with Claude Sonnet 4.6, but I can't verify what's currently running."
```

**Why This Matters (Agent-Native Principle: Action Parity):**
Users can type `/status` and see agent health, core memory usage, schema version, message count, DB size. The agent should have equivalent capability via a tool. This is foundational to agent self-awareness.

---

### 2. Skills System Completely Inaccessible to Agent

**Issue:** Skills are a core system feature, but the agent cannot:
- List available skills
- Read skill metadata (description, handler type, triggers, timeouts)
- Query if a specific skill is loaded
- Access skill capability information

**Location:**
- Slash commands: `handlers.rs:307-377` (TUI-only)
- Skills system: `crates/mika-agent/src/skills/mod.rs`
- Skill registry: Passed to TUI app (`chat.rs:29`) but not to agent system prompt

**Impact:**
```
User: "Can you help me with my calendar?"
Agent: (no idea if calendar skill is loaded)
Agent: "I could try to help, but I don't know what capabilities I have."
```

The agent needs runtime access to:
1. `list_skills()` → Vec<SkillEntry> with name, description, handler type, always_on, triggers
2. `get_skill(name)` → SkillEntry with full manifest details

---

### 3. Command Output Not Programmatically Consumable

**Issue:** Slash command output is hardcoded for human readability (formatted text) with no structured format option.

**Location:**
- Status: `handlers.rs:193-216` returns formatted string
- Memory: `handlers.rs:84-159` returns formatted string with no JSON option
- Skills: `handlers.rs:307-332` returns formatted string
- Reminders: `handlers.rs:161-190` returns formatted string

**Impact:**
```rust
// Currently: TUI-only formatted text
"Status:
  Messages: 42
  DB size: 128 KB
  Core memory: 1200/2000 tokens
  Schema: v6"

// Should also support structured output for pipes/automation
// { "messages": 42, "db_size_bytes": 131072, "core_memory_tokens": 1200, "schema_version": 6 }
```

This breaks the Unix philosophy of composable tools. Users cannot:
- `mika status | jq '.core_memory_tokens'`
- Check DB size in a monitoring script
- Export memory data for analysis

---

### 4. Command Registry Isolation from CLI Structure

**Issue:** Slash commands are defined separately from the CLI subcommand structure, creating maintenance burden and inconsistency.

**Location:**
- TUI slash commands registry: `tui/commands/mod.rs:13-92` (SlashCommand struct)
- CLI subcommands registry: `cli.rs:1-82` (clap Commands enum)
- Command dispatch in TUI: `tui/commands/handlers.rs:8-31` (match statement)
- Command dispatch in CLI: `main.rs:20-36` (match statement)

**Problem:**
```
# TUI side (tui/commands/mod.rs)
SlashCommand {
    name: "memory",
    aliases: &["mem"],
    description: "Show core memory blocks",
    args_hint: Some("[search <query>]"),
}

# CLI side (cli.rs)
pub enum Commands {
    Memory(MemoryArgs),
    // ...
}

# Two separate registries, two separate match statements, two separate docs
# When you add a command to one, you must remember to add to the other
```

Adding a new command requires:
1. Add SlashCommand to `COMMANDS` array
2. Add match arm to `dispatch()` function
3. Add clap subcommand enum variant
4. Add match arm to `main.rs`
5. Create new file in `commands/` module

**Red Flag:** The design doesn't prevent divergence. The TUI could have commands the CLI doesn't, or vice versa.

---

### 5. Soul and Config Display Without Structural Access

**Issue:** `/soul` and `/config` are read-only display commands with no tool for the agent to read these files.

**Location:**
- `/soul`: `handlers.rs:218-225` reads `soul.md`
- `/config`: `handlers.rs:227-253` reads config files

**Impact:**
```
User: "What's my soul.md say?"
Agent: "I can't read your soul.md file directly. I would need a tool to do that."
```

This blocks the agent from:
- Reminding the user of their core values
- Referencing configured preferences in responses
- Validating configuration before making recommendations

---

## Warnings

### 6. Clear Command Is TUI State Only

**Issue:** `/clear` clears the TUI message display but doesn't have a CLI equivalent.

**Location:** `handlers.rs:51-55`

**Design Consideration:**
```rust
async fn handle_clear(app: &mut App<'_>, _args: &str) -> String {
    app.messages.clear();
    app.scroll_offset = 0;
    "Chat display cleared.".to_string()
}
```

This is TUI-specific and shouldn't be a slash command (it's UI state, not system capability). However, the deeper issue is there's no concept of "clearing conversation history" available to users programmatically. Options:

1. Remove `/clear` (it's UI-only, not a real feature)
2. Add a `mika chat --clear` flag or `mika memory clear` subcommand
3. Create an agent tool `clear_conversation()` (but this should be user-initiated, not agent-initiated)

---

### 7. Export Command Lacks Composition Support

**Issue:** `/export` writes to `~/.mika/exports/` with a hardcoded filename format.

**Location:** `handlers.rs:259-305`

**Problem:**
```rust
let filename = format!("session-{short_session}-{timestamp}.md");
let filepath = exports_dir.join(&filename);
// Always writes to exports_dir with fixed naming
```

Users cannot:
- Export to stdout: `mika ask "..." | tee response.txt`
- Specify output path
- Choose export format (JSON, HTML, CSV)

**Better Design:**
```rust
// Option 1: Support format flag
mika export --format json --output ~/my-export.json

// Option 2: Export to stdout
mika memory people --format json | jq '.people[] | select(.relationship == "family")'

// Option 3: Pipe-friendly
mika ask "..." | mika export --format md > /tmp/response.md
```

---

### 8. Compact Command Lacks Agent Awareness

**Issue:** `/compact` runs conversation compaction without agent knowledge.

**Location:** `handlers.rs:57-72`

```rust
async fn handle_compact(app: &mut App<'_>) -> String {
    if app.status != AgentStatus::Idle {
        return "Cannot compact while agent is busy.".to_string();
    }
    match mika_agent::compaction::maybe_compact(&app.db, &app.claude).await {
        Ok(()) => format!("Compacted conversation ({count} messages)."),
        Err(e) => format!("Compaction failed: {e}"),
    }
}
```

**Design Gap:**
- Agent doesn't know when compaction happens
- Agent can't trigger compaction (only user can)
- No agent tool `trigger_compaction()` exists

The agent should be able to request compaction when it notices message count is high:
```
Agent: [tool_use name="trigger_compaction"]
```

---

## Observations & Considerations

### 9. Autocomplete Popup Good UX, But Needs Help Documentation

The autocomplete system (`tui/commands/autocomplete.rs`) is well-implemented with:
- Prefix matching (case-insensitive)
- Alias support
- Visual feedback

However, the help doesn't explain all commands clearly. `/help` output (line 33) should include examples for complex commands:
```
❌ Current: "  /memory [search <query>] — Show core memory blocks"
✅ Better:  "  /memory [search <query>] — Show core memory blocks
               Examples: /memory (all), /memory search alex (find mentions of alex)"
```

---

### 10. Silent Mode Agent Cannot Use Slash Commands

**Issue:** Background/silent agent has no way to execute "system commands" like checking health or reporting on skills.

**Location:** `crates/mika-agent/src/agent.rs` has `run_silent_agent()`, but it doesn't have access to slash command infrastructure.

**Context:** Per CLAUDE.md, silent mode is used for reminders and heartbeats where text output is NOT delivered to user. Agent must use `send_message` tool to communicate.

**Design Gap:**
- Silent agent can't check if it's healthy before sending a reminder
- Silent agent can't list available skills to decide what to recommend
- Silent agent can't read config/soul to inform decisions

---

## Recommendations

### Priority 1: Critical — Add Agent Tools for System Introspection

**Action:** Create three new agent tools to mirror slash command capabilities:

1. **`get_system_status()`** → JSON with:
   ```json
   {
     "message_count": 42,
     "db_size_bytes": 131072,
     "core_memory_tokens": 1200,
     "core_memory_limit": 2000,
     "schema_version": 6,
     "is_busy": false,
     "last_user_activity": "2026-02-25T15:30:00Z"
   }
   ```

2. **`list_skills()`** → JSON with:
   ```json
   {
     "skills": [
       {
         "name": "memory",
         "description": "...",
         "handler_type": "builtin",
         "always_on": true,
         "tools": ["store_fact", "search_memory", ...]
       }
     ]
   }
   ```

3. **`read_soul()`** → raw text content of `soul.md`

**Why:** These enable the agent to:
- Self-monitor and report health
- Understand its own capabilities at runtime
- Personalize responses based on user configuration
- Work in silent mode without blind spots

**Effort:** 3 small tools (~50 lines each), update system prompt with tool documentation (~30 lines).

---

### Priority 2: High — Unify Command Registry

**Action:** Create a shared command registry that both TUI and CLI consume from.

**Design:** Replace `SlashCommand` struct and manual registrations with a unified definition:

```rust
// crates/mika-cli/src/commands/registry.rs (NEW)

pub enum CommandRegistry {
    // Chat/agent commands
    Agent { subcommand: AgentCommand },

    // System/info commands
    System { subcommand: SystemCommand },
}

pub enum SystemCommand {
    Status,
    Memory(MemoryArgs),
    Reminders(ReminderArgs),
    Config(ConfigArgs),
}

pub trait CommandHandler {
    async fn execute(&self) -> Result<CommandOutput>;
}

pub struct CommandOutput {
    pub text: String,           // Human-readable
    pub json: Option<Value>,    // Structured data
}
```

Both CLI and TUI then use:
```rust
// In cli.rs: clap subcommands generated from registry
// In tui/commands/mod.rs: slash commands generated from registry
// In tui/commands/handlers.rs: dispatch uses registry handler
// In tui/input.rs: autocomplete uses registry
```

**Why:** Single source of truth. When you add a command, it's available in all three places: CLI subcommand, TUI slash command, autocomplete.

**Effort:** ~200 lines to refactor, but eliminates future drift.

---

### Priority 3: High — Add JSON Output Format Support

**Action:** Add `--format json` flag to all info-reporting commands.

```rust
pub struct CommandOutput {
    pub text: String,
    pub json: Option<Value>,
}

// In each handler:
let output = CommandOutput {
    text: "Status:\n  Messages: 42\n...".to_string(),
    json: Some(json!({
        "messages": 42,
        "db_size_bytes": 131072,
        "core_memory_tokens": 1200,
        "schema_version": 6,
    })),
};

// In CLI, respect --format flag
if format == "json" {
    println!("{}", serde_json::to_string_pretty(&output.json)?);
} else {
    println!("{}", output.text);
}
```

**Affected Commands:**
- `mika status --format json`
- `mika memory search --format json`
- `mika reminders --format json`
- `/status` in TUI (format is always pretty-printed)

**Why:** Enables automation, tooling, and data export.

**Effort:** ~100 lines across all handlers.

---

### Priority 4: Medium — Add Agent Tool for Conversation Compaction

**Action:** Create a tool `request_compaction()` that allows the agent to suggest compaction.

```rust
// crates/mika-agent/src/tools/request_compaction.rs

#[async_trait]
impl Tool for RequestCompactionTool {
    fn name(&self) -> &str { "request_compaction" }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "request_compaction".to_string(),
            description: "Request conversation history compaction if message count is high.
                Compaction summarizes older messages to free up context.".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let count = ctx.db.count_messages().await?;
        if count <= 50 {
            return Ok(ToolOutput::success(format!(
                "No compaction needed ({count}/50 messages)."
            )));
        }

        compaction::maybe_compact(ctx.db, ...).await?;
        Ok(ToolOutput::success(format!("Compacted {count} messages.")))
    }
}
```

Document in system prompt:
```
Agent can use request_compaction to automatically compact conversation history
when it becomes very long. This is a background operation that summarizes older
messages to preserve context while freeing tokens.
```

**Why:** Allows agent autonomy in managing its own context window.

---

### Priority 5: Medium — Remove TUI-Only Commands from Slash Registry

**Action:** `/clear` and `/exit` are UI state, not system features. Consider:

Option A: Remove them from slash commands (keep as hotkeys)
- `Ctrl+L` → clear display
- `Ctrl+D` or `/exit` → quit

Option B: Move to CLI with proper subcommand semantics
- `mika chat --clear-history` (agent's perspective: new session)
- `mika chat --exit` (nonsensical; just don't run chat)

Option C: Keep as slash commands but document they're TUI-only
```rust
SlashCommand {
    name: "clear",
    aliases: &[],
    description: "Clear chat display (TUI-only) — use Ctrl+L",
    args_hint: None,
}
```

**Recommendation:** Option C + hotkey. Document clearly.

---

### Priority 6: Low — Enhance Export Command

**Action:** Make export more flexible:

```rust
// Support options
mika ask "..." --export-format json --export-to - | jq .
mika memory people --export-to ~/people.json --export-format json
```

Or use standard pipe convention:
```rust
// Follow Unix philosophy: write to stdout by default
mika memory people --format json | tee ~/people.json
```

---

## Architecture Strength: Good Client-Side Isolation

**Positive Finding:** The slash-command system correctly keeps all command processing on the TUI side without modifying the agent loop. This is good:

```
User Input
    ↓
TUI Input Handler
    ├─→ "/" detected: Queue slash command
    │   ↓
    │   Slash Command Dispatcher (tui/commands/handlers.rs)
    │   ├─→ Read from DB directly
    │   ├─→ Read from filesystem
    │   └─→ Format and display
    │
    └─→ Regular message: Send to Agent
        ↓
        Agent Loop (mika-agent)
        ├─→ Retrieve context
        ├─→ Call Claude API
        ├─→ Execute tools if needed
        └─→ Return response
```

The agent loop remains clean. However, **the agent should have equivalent tools available**, so it's not blocked from learning the same information.

---

## Code Quality Assessment

### What's Well Done

1. **Autocomplete Implementation** (`tui/commands/autocomplete.rs:1-152`)
   - Proper prefix matching with case-insensitivity
   - Alias support
   - Clear, testable logic
   - Good error handling

2. **Handler Organization** (`tui/commands/handlers.rs`)
   - Separate handler functions for each command (not one giant match statement)
   - Consistent error handling patterns
   - Good use of async where needed

3. **Command Registry** (`tui/commands/mod.rs`)
   - Metadata-rich (name, aliases, description, args_hint)
   - Tests for filter and parse logic
   - Clear documentation

### What Needs Improvement

1. **Missing Structured Output** — All handlers return String, no JSON option
2. **No Shared Registry** — Command definitions split between TUI and CLI
3. **Test Coverage** — No tests for actual handler outputs (only metadata logic)
4. **Documentation** — System prompt doesn't mention available commands
5. **Agent Opacity** — Agent has no visibility into system state or capabilities

---

## Summary: Agent-Native Compliance Scorecard

| Principle | Status | Details |
|-----------|--------|---------|
| **Action Parity** | ⚠️ Partial | 7 of 13 commands lack agent equivalents |
| **Context Parity** | ❌ No | Agent doesn't know which skills are loaded, model in use, or system health |
| **Shared Workspace** | ✅ Yes | All commands read/write same DB and filesystem |
| **Primitives over Workflows** | ⚠️ Partial | Most commands are primitives (read state), but compact is a workflow |
| **Dynamic Context Injection** | ❌ No | System prompt doesn't include skill list, command registry, or capabilities doc |

**Overall Verdict:** **NEEDS WORK**

The feature is well-built from a TUI perspective, but it violates agent-native principles by:
1. Creating hidden capabilities only available to users
2. Blocking agent self-introspection (health, skills, configuration)
3. Siloing command output in text-only format
4. Preventing automation and composition

---

## Files Affected by This Review

**Core Changes Needed:**
- `crates/mika-agent/src/tools/mod.rs` — Add 3 new tools
- `crates/mika-agent/src/tools/get_system_status.rs` — NEW
- `crates/mika-agent/src/tools/list_skills.rs` — NEW
- `crates/mika-agent/src/tools/read_soul.rs` — NEW
- `crates/mika-agent/src/prompt.rs` — Update system prompt
- `crates/mika-cli/src/tui/commands/handlers.rs` — Add JSON output support
- `crates/mika-cli/src/cli.rs` — Add --format flag to relevant commands
- `crates/mika-cli/src/commands/status.rs` — Add JSON output
- `crates/mika-cli/src/commands/memory.rs` — Add JSON output
- `crates/mika-cli/src/commands/reminders.rs` — Add JSON output

**Refactoring Opportunity:**
- `crates/mika-cli/src/commands/registry.rs` — NEW unified registry
- `crates/mika-cli/src/tui/commands/mod.rs` — Update to use registry
- `crates/mika-cli/src/main.rs` — Update to use registry

---

## Next Steps

1. **Review this feedback** with the team
2. **Prioritize fixes:**
   - P1: Agent tools for introspection (blocks agent self-awareness)
   - P2: Unified command registry (reduces maintenance burden)
   - P3: JSON output support (enables automation)
3. **Create focused PRs:**
   - "feat(agent): add system introspection tools"
   - "refactor(cli): unify command registry"
   - "feat(cli): add JSON output format support"
4. **Update system prompt** with new tool documentation
5. **Add agent tests** that exercise new tools

---

## References

- **CLAUDE.md:** Project context, agent loop structure, memory model
- **Branch:** `feat/slash-commands` (commit 09d8595)
- **Agent Loop Design:** `crates/mika-agent/src/agent.rs`
- **Tool System:** `crates/mika-agent/src/tools/mod.rs`
- **Skills System:** `crates/mika-agent/src/skills/mod.rs`
