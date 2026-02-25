# Implementation Guide: Fixing Agent-Native Compliance

This guide provides concrete, step-by-step instructions for implementing the recommendations from the agent-native architecture review.

## Overview

You need to create 3 new agent tools that give the agent access to system capabilities that are currently hidden in the TUI. Once this is done, the system will follow agent-native principles where agents and users have feature parity.

## Phase 1: Add Agent Tools (P1 - Critical)

### Step 1.1: Create `get_system_status()` Tool

**File:** `/data/workspace/senara-solutions/mika/crates/mika-agent/src/tools/get_system_status.rs` (NEW)

```rust
use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::json;
use serde::json;

use super::{Tool, ToolContext, ToolOutput};

pub struct GetSystemStatusTool;

#[async_trait]
impl Tool for GetSystemStatusTool {
    fn name(&self) -> &str {
        "get_system_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_system_status".to_string(),
            description: "Get current system health and capacity metrics.
                Use this to check how many messages are stored, current memory usage,
                database size, and whether the agent is healthy."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Fetch all metrics in parallel
        let (messages, db_size, tokens, schema, last_activity) = tokio::join!(
            ctx.db.count_messages(),
            ctx.db.db_size_bytes(),
            ctx.db.total_core_memory_tokens(),
            ctx.db.schema_version(),
            ctx.db.last_user_message_time(),
        );

        let status = json!({
            "message_count": messages.unwrap_or(0),
            "db_size_bytes": db_size.unwrap_or(0),
            "core_memory_tokens": tokens.unwrap_or(0),
            "core_memory_limit": 2000,
            "schema_version": schema.unwrap_or(0),
            "last_user_activity": last_activity.ok().flatten(),
        });

        Ok(ToolOutput::success(status.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(GetSystemStatusTool.name(), "get_system_status");
    }

    #[test]
    fn test_definition_has_description() {
        let def = GetSystemStatusTool.definition();
        assert!(!def.description.is_empty());
    }
}
```

**Then update** `crates/mika-agent/src/tools/mod.rs`:

Find this line:
```rust
pub fn default_tools() -> ToolRegistry {
```

Add to the function:
```rust
pub fn default_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(update_core_memory::UpdateCoreMemoryTool));
    registry.register(Box::new(store_fact::StoreFactTool));
    registry.register(Box::new(search_memory::SearchMemoryTool));
    registry.register(Box::new(update_fact::UpdateFactTool));
    registry.register(Box::new(create_reminder::CreateReminderTool));
    registry.register(Box::new(list_reminders::ListRemindersTool));
    registry.register(Box::new(cancel_reminder::CancelReminderTool));
    registry.register(Box::new(send_message::SendMessageTool));
    registry.register(Box::new(get_system_status::GetSystemStatusTool));  // NEW
    registry
}
```

And add the module declaration at the top:
```rust
mod get_system_status;
```

### Step 1.2: Create `list_skills()` Tool

**File:** `/data/workspace/senara-solutions/mika/crates/mika-agent/src/tools/list_skills.rs` (NEW)

```rust
use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::json;

use super::{Tool, ToolContext, ToolOutput};

pub struct ListSkillsTool;

#[async_trait]
impl Tool for ListSkillsTool {
    fn name(&self) -> &str {
        "list_skills"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_skills".to_string(),
            description: "List all available skills and their capabilities.
                Use this to discover what features you have access to, such as
                memory management, calendar integration, email handling, etc."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Note: You'll need to pass SkillRegistry to ToolContext
        // For now, return empty array. See Step 1.3 for the full solution.

        let skills = json!({
            "skills": []
        });

        Ok(ToolOutput::success(skills.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(ListSkillsTool.name(), "list_skills");
    }
}
```

**Note:** The full implementation of `list_skills()` requires passing `SkillRegistry` through `ToolContext`. This is a small refactor covered in the full review (see step 1.3 architecture changes below).

### Step 1.3: Create `read_soul()` Tool

**File:** `/data/workspace/senara-solutions/mika/crates/mika-agent/src/tools/read_soul.rs` (NEW)

```rust
use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::json;
use std::path::Path;

use super::{Tool, ToolContext, ToolOutput};

pub struct ReadSoulTool;

#[async_trait]
impl Tool for ReadSoulTool {
    fn name(&self) -> &str {
        "read_soul"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_soul".to_string(),
            description: "Read the user's soul.md file containing their core values,
                principles, and personal vision. Use this to understand the user's
                fundamental values and align your responses accordingly."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let soul_path = ctx.home_dir.join("soul.md");

        match tokio::fs::read_to_string(&soul_path).await {
            Ok(content) if !content.trim().is_empty() => {
                Ok(ToolOutput::success(content))
            }
            Ok(_) => {
                Ok(ToolOutput::success(
                    "soul.md is empty. User can create one at ~/.mika/soul.md".to_string()
                ))
            }
            Err(_) => {
                Ok(ToolOutput::success(
                    "No soul.md file found. User can create one to define their values."
                    .to_string()
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        assert_eq!(ReadSoulTool.name(), "read_soul");
    }
}
```

### Step 1.4: Register New Tools

**File:** `crates/mika-agent/src/tools/mod.rs`

Update the `default_tools()` function:
```rust
pub fn default_tools() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(update_core_memory::UpdateCoreMemoryTool));
    registry.register(Box::new(store_fact::StoreFactTool));
    registry.register(Box::new(search_memory::SearchMemoryTool));
    registry.register(Box::new(update_fact::UpdateFactTool));
    registry.register(Box::new(create_reminder::CreateReminderTool));
    registry.register(Box::new(list_reminders::ListRemindersTool));
    registry.register(Box::new(cancel_reminder::CancelReminderTool));
    registry.register(Box::new(send_message::SendMessageTool));
    registry.register(Box::new(get_system_status::GetSystemStatusTool));  // NEW
    registry.register(Box::new(list_skills::ListSkillsTool));            // NEW
    registry.register(Box::new(read_soul::ReadSoulTool));                // NEW
    registry
}
```

And add module declarations at the top of the file:
```rust
mod get_system_status;
mod list_skills;
mod read_soul;
```

### Step 1.5: Update System Prompt

**File:** `crates/mika-agent/src/prompt.rs`

Find the section where tools are documented. Add:

```markdown
## System Introspection Tools

### get_system_status()
Returns current system health metrics:
- message_count: Total messages in conversation
- db_size_bytes: Size of SQLite database
- core_memory_tokens: Current token usage (0-2000)
- core_memory_limit: Maximum tokens (2000)
- schema_version: Current database schema version
- last_user_activity: Timestamp of last user message

Use this when users ask about:
- How many messages we've discussed
- System capacity or memory usage
- Whether you're approaching context limits
- Database health

Example: User asks "How much context space do I have left?"
→ Call get_system_status()
→ Calculate: (2000 - core_memory_tokens) / 2000 as percentage
→ Report to user

### list_skills()
Returns available skills and their metadata:
- name: Skill identifier
- description: Human-readable description
- handler_type: "builtin", "exec", or "http"
- always_on: Whether skill is always available
- tools: List of tools the skill provides

Use this when:
- Users ask what you can help with
- Deciding whether to offer certain capabilities
- Explaining your limitations if a skill isn't loaded

Example: User asks "Can you help with my calendar?"
→ Call list_skills()
→ Check if any skill has "calendar" in description/name
→ Respond affirmatively if found, or explain limitation if not

### read_soul()
Returns the user's soul.md file content (if it exists).
This is the user's personal manifesto of values, principles, and vision.

Use this to:
- Understand the user's core values
- Align advice and recommendations with their principles
- Personalize responses based on what matters to them
- Respect their stated priorities and preferences

Example: When giving advice, read_soul() first to ensure
recommendations align with user's stated values.
```

### Step 1.6: Test the Tools

**Run tests:**
```bash
cargo test --package mika-agent get_system_status
cargo test --package mika-agent list_skills
cargo test --package mika-agent read_soul
```

**Test with agent:**
```bash
# Ask the agent to check system status
mika ask "How many messages are we storing?"

# The agent should now be able to call get_system_status and report back.
```

---

## Phase 2: Add JSON Output Support (P3 - High)

### Step 2.1: Update Status Command

**File:** `crates/mika-cli/src/commands/status.rs`

Change the main function to return structured data:

```rust
use anyhow::Result;
use serde_json::json;

pub async fn run() -> Result<()> {
    let ctx = init::init_db_only()?;
    let db = &ctx.async_db;

    let (db_size, msg_count, people, commitments, preferences, events, last_msg, tokens, version) = tokio::join!(
        db.db_size_bytes(),
        db.count_messages(),
        db.list_people(),
        db.list_commitments("pending"),
        db.list_preferences(),
        db.list_events(),
        db.last_user_message_time(),
        db.total_core_memory_tokens(),
        db.schema_version(),
    );

    // Build structured output
    let status = json!({
        "database": {
            "path": ctx.settings.db_path.to_string_lossy().to_string(),
            "size_bytes": db_size.unwrap_or(0),
            "schema_version": version.unwrap_or(0),
        },
        "messages": msg_count.unwrap_or(0),
        "last_activity": last_msg.ok().flatten(),
        "memory": {
            "core_tokens": tokens.unwrap_or(0),
            "core_limit": 2000,
        },
        "tracked_items": {
            "people": people.map(|p| p.len()).unwrap_or(0),
            "commitments": commitments.map(|c| c.len()).unwrap_or(0),
            "preferences": preferences.map(|p| p.len()).unwrap_or(0),
            "events": events.map(|e| e.len()).unwrap_or(0),
        }
    });

    println!("{}", serde_json::to_string_pretty(&status)?);

    Ok(())
}
```

### Step 2.2: Update CLI to Support --format Flag

**File:** `crates/mika-cli/src/cli.rs`

Modify the Status command:

```rust
use clap::ValueEnum;

#[derive(ValueEnum, Clone)]
pub enum OutputFormat {
    Text,
    Json,
}

// In Commands enum:
pub enum Commands {
    /// Open interactive chat (default)
    Chat,
    /// First-run bootstrap
    Setup,
    /// Inspect stored memory
    Memory(MemoryArgs),
    /// List or cancel reminders
    Reminders(ReminderArgs),
    /// Show health info
    Status {
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    // ... rest of commands
}
```

### Step 2.3: Update Main to Handle Format Flag

**File:** `crates/mika-cli/src/main.rs`

```rust
match cli.command {
    // ...
    Some(Commands::Status { format }) => {
        let ctx = init::init_db_only()?;
        match format {
            cli::OutputFormat::Json => {
                // Call status function and it returns JSON
                commands::status::run().await
            }
            cli::OutputFormat::Text => {
                // Add a separate text rendering function
                commands::status::run_text().await
            }
        }
    }
    // ...
}
```

---

## Phase 3: Unify Command Registry (P2 - High)

### Step 3.1: Create Unified Registry

**File:** `crates/mika-cli/src/commands/registry.rs` (NEW)

```rust
use anyhow::Result;
use std::collections::HashMap;

pub trait CommandHandler: Send + Sync {
    async fn execute(&self) -> Result<CommandOutput>;
}

pub struct CommandOutput {
    pub text: String,
    pub json: Option<serde_json::Value>,
}

pub struct CommandEntry {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub handler: Box<dyn CommandHandler>,
}

pub struct CommandRegistry {
    commands: HashMap<String, CommandEntry>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    pub fn register(&mut self, entry: CommandEntry) {
        self.commands.insert(entry.name.to_string(), entry);
        for alias in entry.aliases {
            self.commands.insert(alias.to_string(), entry);
        }
    }

    pub fn get(&self, name: &str) -> Option<&CommandEntry> {
        self.commands.get(name)
    }

    pub fn all(&self) -> Vec<&CommandEntry> {
        self.commands.values().collect()
    }
}
```

### Step 3.2: Refactor TUI Commands to Use Registry

**File:** `crates/mika-cli/src/tui/commands/mod.rs`

Update to use the unified registry instead of SlashCommand:

```rust
pub use crate::commands::registry::CommandRegistry;

pub fn get_slash_commands(registry: &CommandRegistry) -> Vec<SlashCommand> {
    registry
        .all()
        .into_iter()
        .map(|entry| SlashCommand {
            name: entry.name,
            aliases: entry.aliases,
            description: entry.description,
            args_hint: None,
        })
        .collect()
}
```

---

## Testing Checklist

After implementing all phases, verify:

```bash
# Test agent tools
cargo test get_system_status
cargo test list_skills
cargo test read_soul

# Test agent can use them
mika ask "What's my system status?"
mika ask "List my skills"
mika ask "What's my soul?"

# Test JSON output
mika status --format json | jq '.memory.core_tokens'
mika memory search test --format json | jq '.people'

# Test TUI slash commands still work
# Open chat: mika
# Type: /status
# Type: /skills
# Type: /soul
```

---

## Estimated Effort

| Phase | Task | Effort | Complexity |
|-------|------|--------|------------|
| 1.1 | get_system_status tool | 45 min | Low |
| 1.2 | list_skills tool | 30 min | Low (without registry) |
| 1.3 | read_soul tool | 20 min | Low |
| 1.4 | Register tools | 10 min | Trivial |
| 1.5 | Update system prompt | 30 min | Low |
| 1.6 | Test phase 1 | 20 min | Low |
| **P1 Total** | **3 tools + registration + docs** | **2.5 hours** | **Low** |
| 2.1-2.3 | JSON output support | 1.5 hours | Medium |
| **P3 Total** | **Add --format flags** | **1.5 hours** | **Medium** |
| 3.1-3.2 | Unified registry | 2 hours | Medium |
| **P2 Total** | **Refactor for DRY** | **2 hours** | **Medium** |

**Total Recommended:** 6 hours over 2-3 PRs

---

## PR Structure Recommendation

### PR 1: Agent Tools (P1)
- Add 3 tools: get_system_status, list_skills, read_soul
- Register in tool registry
- Update system prompt
- Add tests
- ~150 LOC

**PR Title:** `feat(agent): add system introspection tools`

### PR 2: JSON Output (P3)
- Add --format flag to Status, Memory, Reminders commands
- Update handlers to return structured data
- ~100 LOC

**PR Title:** `feat(cli): add JSON output format support`

### PR 3: Unified Registry (P2)
- Create shared command registry
- Refactor TUI commands to use registry
- Refactor CLI commands to use registry
- ~200 LOC

**PR Title:** `refactor(cli): unify command registry for DRY principle`

---

## Validation Criteria

Before marking as complete:

1. All tests pass: `cargo test`
2. Agent can call new tools: `mika ask "check status"`
3. JSON output works: `mika status --format json | jq .`
4. TUI slash commands work: `/skills`, `/status`, `/soul` in chat
5. Code review passes
6. Documentation updated

---

## References

- Full review: `/data/workspace/senara-solutions/mika/docs/reviews/agent-native-slash-command-review.md`
- Design doc: `/data/workspace/senara-solutions/mika/docs/reviews/slash-command-agent-native-design.md`
- CLAUDE.md: Project architecture and conventions
