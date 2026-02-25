# How Slash-Commands Should Look: Agent-Native Design

This document shows the ideal state after addressing the review findings.

## Principle: No Hidden Capabilities

Every user-facing command should have an agent equivalent.

```
User Layer (TUI)
    │
    ├─→ Slash commands      (e.g., /status)
    │   └─→ Read from DB/filesystem
    │       └─→ Display in TUI
    │
    └─→ Agent messages      (e.g., "How are you?")
        └─→ Agent loop
            ├─→ Retrieve context
            ├─→ Call Claude
            ├─→ Execute tools if needed  ← Agent can invoke same operations as /commands
            └─→ Return response

System Capabilities Layer (Shared)
    │
    ├─→ get_system_status()  ← Both user and agent can invoke
    ├─→ list_skills()        ← Both user and agent can invoke
    ├─→ read_soul()          ← Both user and agent can invoke
    ├─→ search_memory()      ← Both user and agent can invoke
    └─→ ...
```

## Command Parity Examples

### Example 1: System Status

**Before (Status Quo):**
```
User: /status
TUI: Reads DB directly, displays "Messages: 42..."
Agent: "I don't know my own health. I was told I use Claude Sonnet, but I can't verify."
```

**After (Agent-Native):**
```
User: /status                               Agent: (tool_use) get_system_status()
    ↓                                              ↓
TUI handler calls                           Agent tool calls
async fn handle_status(app) {               async fn execute(...) {
    let status = {                              let status = {
        messages: db.count().await?,            messages: db.count().await?,
        db_size_bytes: ...,                     db_size_bytes: ...,
        ...                                     ...
    };                                      };
    println!("{}", format_human(&status));      return ToolOutput::success(
}                                               json!(&status)
                                            )
                                            }

User sees:                                  Agent gets:
"Status:                                    {"messages": 42, "db_size_bytes": 131072,
 Messages: 42                               "core_memory_tokens": 1200, ...}
 DB size: 128 KB
 ..."                                       Agent can then respond:
                                            "You have 42 messages stored.
                                            Your core memory is using 1200 of 2000
                                            tokens. The database is 128 KB."
```

### Example 2: Skills Introspection

**Before:**
```
User: /skills
TUI: Displays loaded skills
Agent: (no concept of skills)
Agent: "I have no idea what capabilities I have beyond the fixed tools I was told about."
```

**After:**
```
User: /skills                               Agent: (tool_use) list_skills()

TUI calls:                                  Agent tool calls:
for skill in app.skills.skills() {          pub async fn execute(...) {
    println!("  {} ({})", name, handler);       let skills = ctx.db
}                                               .load_skills_registry()
                                                .await?;
                                                return ToolOutput::success(
                                                    json!(&skills)
                                                )
                                            }

User sees:                                  Agent gets:
"Loaded skills:                             {
  memory (builtin) — ...                    "skills": [
  calendar (builtin) — ...                  {
  email (http) — ...                        "name": "memory",
"                                           "description": "...",
                                            "handler_type": "builtin",
                                            "always_on": true
                                            }
                                            ]
                                            }

                                            Agent can now respond:
                                            "I have 3 skills loaded:
                                            memory (always available),
                                            calendar (HTTP-based),
                                            email (HTTP-based).
                                            I can help you with scheduling
                                            or email if you need it."
```

### Example 3: Configuration/Soul Reading

**Before:**
```
User: /soul
TUI: Reads and displays ~/mika/soul.md
Agent: Can't read these files
Agent: (can only go by what's in core memory, no direct access)
```

**After:**
```
User: /soul                                 Agent: (tool_use) read_soul()

TUI handler:                                Agent tool:
let content = tokio::fs::read_to_string     async fn execute(...) {
  (&app.home_dir.join("soul.md")).await?;    let path = ctx.home_dir.join("soul.md");
println!("{}", content);                     let content = tokio::fs::read_to_string
                                               (&path).await?;
User sees:                                    return ToolOutput::success(content)
(raw soul.md content)                       }

                                            Agent gets:
                                            (raw soul.md content as tool output)

                                            Agent can then reference user values:
                                            "I see from your soul.md that
                                            authenticity is core to you.
                                            That's why I'm being direct about..."
```

## Unified Command Registry Design

All commands live in one place, consumed by three layers:

```rust
// crates/mika-cli/src/commands/registry.rs (unified source of truth)

pub trait Command: Send + Sync {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str];
    fn description(&self) -> &str;
    fn handler(&self) -> Box<dyn CommandHandler>;
}

pub trait CommandHandler: Send + Sync {
    async fn execute(&self, ctx: &CommandContext) -> Result<CommandOutput>;
}

pub struct CommandOutput {
    pub text: String,           // Human-readable
    pub json: Option<Value>,    // Structured data
}

// Then register all commands
pub fn all_commands() -> Vec<Box<dyn Command>> {
    vec![
        Box::new(StatusCommand),
        Box::new(MemoryCommand),
        Box::new(SkillsCommand),
        // ...
    ]
}
```

### Layer 1: TUI Slash Commands

```rust
// crates/mika-cli/src/tui/commands/mod.rs
pub fn get_slash_commands() -> Vec<SlashCommand> {
    REGISTRY
        .all_commands()
        .into_iter()
        .map(|cmd| SlashCommand {
            name: cmd.name(),
            aliases: cmd.aliases(),
            description: cmd.description(),
            args_hint: None, // derive from handler
        })
        .collect()
}

pub async fn dispatch(app: &mut App<'_>, input: &str) -> Option<String> {
    let (cmd_name, args) = parse_command(input);
    let cmd = REGISTRY.get_command(cmd_name)?;
    let output = cmd.handler().execute(&ctx).await.ok()?;
    Some(output.text) // Display human-readable version
}
```

### Layer 2: CLI Subcommands

```rust
// crates/mika-cli/src/cli.rs (auto-generated from registry)
pub enum Commands {
    #[command(about = "Show system status")]
    Status {
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },
    #[command(about = "Show memory")]
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    // ... all commands from REGISTRY
}

// main.rs dispatches to registry handler
match cli.command {
    Some(Commands::Status { format }) => {
        let ctx = init::init_db_only()?;
        let output = REGISTRY.get_command("status")?
            .handler()
            .execute(&ctx)
            .await?;
        match format {
            OutputFormat::Text => println!("{}", output.text),
            OutputFormat::Json => println!("{}", output.json?),
        }
    },
    // ...
}
```

### Layer 3: Agent Tools

```rust
// crates/mika-agent/src/tools/system_status.rs
#[async_trait]
impl Tool for GetSystemStatusTool {
    fn name(&self) -> &str { "get_system_status" }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Uses same handler as CLI/TUI
        let registry = REGISTRY; // Global registry
        let output = registry
            .get_command("status")?
            .handler()
            .execute(&ctx)
            .await?;

        Ok(ToolOutput::success(output.json?))
    }
}
```

## Result: No More Duplication

- **One command registry:** Add a command once, available everywhere
- **No code duplication:** TUI, CLI, and agent all call same handler
- **Consistent output:** Same data structure, different presentation (text vs JSON)
- **Single documentation:** System prompt documents all commands once
- **Easier testing:** Test handlers once, validate all three paths work

## System Prompt Example

```markdown
## Available System Capabilities

The agent has access to these system tools in addition to memory tools:

### get_system_status()
Returns current system health metrics. Use this when users ask about health,
performance, or capacity.

Response format:
{
  "message_count": 42,
  "db_size_bytes": 131072,
  "core_memory_tokens": 1200,
  "core_memory_limit": 2000,
  "schema_version": 6,
  "last_user_activity": "2026-02-25T15:30:00Z"
}

Example: User says "Am I running out of context?" → call get_system_status(),
report core_memory_tokens / core_memory_limit.

### list_skills()
Returns available skills and their capabilities. Use this when deciding what
to offer help with, or when users ask what you can do.

Response format:
{
  "skills": [
    {
      "name": "memory",
      "description": "Store and retrieve facts",
      "handler_type": "builtin",
      "always_on": true,
      "tools": ["store_fact", "search_memory", "update_fact"]
    }
  ]
}

Example: User says "Can you help with calendar?" → call list_skills(),
check if calendar skill is loaded, offer help if available.

### read_soul()
Read the user's soul.md file to understand their core values and principles.
Use this to align responses with user values.

Response format:
(raw text content of ~/.mika/soul.md)

Example: User asks for advice → call read_soul(), consider their values
when formulating response.
```

## Silent Mode Enablement

Background agent (reminders, heartbeats) can now check health before acting:

```rust
// crates/mika-agent/src/scheduler.rs
async fn fire_reminder(&self, reminder: &Reminder) -> Result<()> {
    // Before acting, check system health
    let status = run_agent_with_tools(&ToolContext {
        tools: vec![get_system_status],  // Only system tool available
        // ...
    }).await?;

    // Only proceed if system is healthy
    let status_json = status.json.ok_or("missing status")?;
    if status_json["is_busy"].as_bool().unwrap_or(false) {
        // Don't fire reminder, agent is busy
        return Ok(());
    }

    // Proceed with reminder
    send_reminder(&reminder).await
}
```

## Migration Path

1. **Phase 1:** Create agent tools (get_system_status, list_skills, read_soul)
   - Non-breaking
   - Agent gains capabilities
   - TUI/CLI unchanged

2. **Phase 2:** Unify command registry
   - Refactor TUI commands to use new registry
   - Refactor CLI commands to use new registry
   - Handlers stay same, just reorganized

3. **Phase 3:** Add JSON output support
   - Add --format flag to CLI commands
   - Update handlers to return structured data
   - Update system prompt to document JSON outputs

4. **Phase 4:** Close the loop
   - Agent tools call unified handlers
   - Documentation updated
   - Tests validate all three paths work

---

## Key Takeaway

The slash-command feature is not "wrong" — it's just incomplete. The missing piece is **agent accessibility**. Every capability the user can access via `/status` should be available to the agent via `get_system_status()`, and the implementation should be shared to eliminate duplication.

This is what agent-native means: **agents and users share the same tools, just different UI layers**.
