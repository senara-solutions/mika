use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "mika", version, about = "Mika — AI Executive Assistant")]
pub struct Cli {
    /// Agent to use (overrides active agent)
    #[arg(long, global = true)]
    pub agent: Option<String>,

    /// Team to use (launches TUI in team mode)
    #[arg(long, global = true, conflicts_with = "agent")]
    pub team: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
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
    Status,
    /// View or edit configuration
    Config(ConfigArgs),
    /// Manage skills
    Skills(SkillsArgs),
    /// Send a message and print the response (non-interactive)
    Ask {
        /// The message to send (use "-" to read from stdin)
        message: String,
        /// Mark a callback task complete with this message as the result before running the agent.
        /// Used by background processes: mika ask --task-id <uuid> "findings..."
        #[arg(long)]
        task_id: Option<String>,
    },
    /// Manage agents
    Agents(AgentsArgs),
    /// Manage teams
    Teams(TeamsArgs),
    /// Manage MCP (Model Context Protocol) servers
    Mcp(McpArgs),
    /// List or cancel pending tasks
    Tasks(TaskArgs),
}

#[derive(clap::Args)]
pub struct AgentsArgs {
    #[command(subcommand)]
    pub command: AgentsCommand,
}

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// List all agents
    List,
    /// Create a new agent
    Create {
        /// Name for the new agent (lowercase, alphanumeric, hyphens)
        name: String,
    },
    /// Delete an agent (cannot delete "mika")
    Delete {
        /// Name of the agent to delete
        name: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
    /// Switch the active agent
    Switch {
        /// Name of the agent to switch to
        name: String,
    },
    /// Clone an agent's personality (soul, identity, config) into a new agent
    Clone {
        /// Source agent to clone from
        source: String,
        /// Name for the new agent
        name: String,
    },
}

#[derive(clap::Args)]
pub struct TeamsArgs {
    #[command(subcommand)]
    pub command: TeamsCommand,
}

#[derive(Subcommand)]
pub enum TeamsCommand {
    /// List all teams
    List,
    /// Create a new team (interactive)
    Create {
        /// Name for the new team (lowercase, alphanumeric, hyphens)
        name: String,
    },
    /// Run a team workflow
    Run {
        /// Name of the team to run
        name: String,
        /// The goal or task for the team
        goal: String,
    },
    /// Show team definition and latest run status
    Status {
        /// Name of the team
        name: String,
    },
    /// Show execution history
    Log {
        /// Name of the team
        name: String,
    },
    /// Delete a team and all its data
    Delete {
        /// Name of the team to delete
        name: String,
        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

#[derive(clap::Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: Option<SkillsCommand>,
}

#[derive(Subcommand)]
pub enum SkillsCommand {
    /// List all skills
    List,
    /// Show details for a specific skill
    Info {
        /// Skill name
        name: String,
    },
    /// Create a new skill from template
    Create {
        /// Name for the new skill
        name: String,
    },
    /// Test a skill tool with sample input
    Test {
        /// Skill name
        skill: String,
        /// Tool name within the skill
        tool: String,
        /// JSON input (default: {})
        #[arg(long, default_value = "{}")]
        input: String,
    },
    /// Enable a disabled skill
    Enable {
        /// Skill name
        name: String,
    },
    /// Disable a skill
    Disable {
        /// Skill name
        name: String,
    },
    /// Install a skill from a Git repository
    Install {
        /// Git URL or GitHub shorthand (user/repo)
        source: String,
        /// Install under a different name (alias)
        #[arg(long)]
        name: Option<String>,
    },
    /// Uninstall a marketplace-installed skill
    Uninstall {
        /// Skill name to uninstall
        name: String,
    },
    /// Update marketplace-installed skills
    Update {
        /// Skill name to update (omit to update all)
        name: Option<String>,
    },
}

#[derive(clap::Args)]
pub struct MemoryArgs {
    #[command(subcommand)]
    pub command: Option<MemoryCommand>,
}

#[derive(Subcommand)]
pub enum MemoryCommand {
    /// Search across all memory types
    Search { query: String },
    /// List tracked people
    People,
    /// List commitments
    Commitments {
        /// Filter by status (pending, completed, cancelled)
        #[arg(long, default_value = "pending")]
        status: String,
    },
    /// List preferences
    Preferences,
    /// List events
    Events,
    /// Reset a core memory block to its default value
    Reset { block: String },
}

#[derive(clap::Args)]
pub struct ReminderArgs {
    #[command(subcommand)]
    pub command: Option<ReminderCommand>,
}

#[derive(Subcommand)]
pub enum ReminderCommand {
    /// Cancel a reminder by ID (from `mika reminders`)
    Cancel { id: String },
}

#[derive(clap::Args)]
pub struct TaskArgs {
    #[command(subcommand)]
    pub command: Option<TaskCommand>,
}

#[derive(Subcommand)]
pub enum TaskCommand {
    /// Cancel a task by ID (from `mika tasks`)
    Cancel { id: String },
}

#[derive(clap::Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Open identity.toml in $EDITOR
    Edit,
    /// Print soul.md to stdout
    Soul,
}

#[derive(clap::Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: Option<McpCommand>,
}

#[derive(Subcommand)]
pub enum McpCommand {
    /// List configured MCP servers
    List,
    /// Add a new MCP server
    Add {
        /// Server name (used as identifier)
        name: String,
        /// Transport type: "stdio" or "http"
        #[arg(long)]
        transport: String,
        /// Command to run (stdio transport)
        #[arg(long)]
        command: Option<String>,
        /// Arguments for the command (stdio transport)
        #[arg(long, num_args = 1..)]
        args: Option<Vec<String>>,
        /// URL to connect to (http transport)
        #[arg(long)]
        url: Option<String>,
        /// HTTP headers as KEY=VALUE pairs (http transport only)
        #[arg(long = "header", num_args = 1..)]
        headers: Option<Vec<String>>,
    },
    /// Remove a configured MCP server
    Remove {
        /// Name of the server to remove
        name: String,
    },
    /// Enable a configured MCP server
    Enable {
        /// Name of the server to enable
        name: String,
    },
    /// Disable a configured MCP server
    Disable {
        /// Name of the server to disable
        name: String,
    },
}
