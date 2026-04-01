use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "mika", version, about = "Mika — AI Executive Assistant")]
pub struct Cli {
    /// Agent to use (overrides active agent)
    #[arg(long)]
    pub agent: Option<String>,

    /// Team to use (launches TUI in team mode)
    #[arg(long, conflicts_with = "agent")]
    pub team: Option<String>,

    /// Reuse an existing session instead of creating a new one.
    /// If the session does not exist yet, it will be created with this ID.
    #[arg(long, global = true)]
    pub session_id: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Shared `--agent` flag, flattened into subcommands that support agent override.
#[derive(clap::Args, Clone, Debug)]
pub struct AgentFlag {
    /// Agent to use (overrides active agent)
    #[arg(long)]
    pub agent: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Open interactive chat (default)
    Chat(ChatArgs),
    /// First-run bootstrap
    Setup {
        /// Setup mode: cli (default), server, or compose
        #[arg(long, value_enum, default_value = "cli")]
        mode: SetupMode,
        /// Anthropic API key (non-interactive, for CI/automation)
        #[arg(long)]
        api_key: Option<String>,
    },
    /// Inspect stored memory
    Memory(MemoryArgs),
    /// List or cancel reminders
    Reminders(ReminderArgs),
    /// Show health info
    Status(StatusArgs),
    /// View or edit configuration
    Config(ConfigArgs),
    /// Manage skills
    Skills(SkillsArgs),
    /// Send a message and print the response (non-interactive)
    Ask(AskArgs),
    /// Manage agents
    Agents(AgentsArgs),
    /// Manage teams
    Teams(TeamsArgs),
    /// Manage MCP (Model Context Protocol) servers
    Mcp(McpArgs),
    /// List or cancel pending tasks
    Tasks(TaskArgs),
    /// Check installation health
    Doctor(DoctorArgs),
    /// Manage the dashboard dev server
    Dashboard(DashboardArgs),
}

impl Commands {
    /// Extract `--agent` override from whichever subcommand carries it.
    pub fn agent_override(&self) -> Option<&str> {
        match self {
            Commands::Chat(args) => args.agent_flag.agent.as_deref(),
            Commands::Ask(args) => args.agent_flag.agent.as_deref(),
            Commands::Memory(args) => args.agent_flag.agent.as_deref(),
            Commands::Reminders(args) => args.agent_flag.agent.as_deref(),
            Commands::Status(args) => args.agent_flag.agent.as_deref(),
            Commands::Config(args) => args.agent_flag.agent.as_deref(),
            Commands::Skills(args) => args.agent_flag.agent.as_deref(),
            Commands::Mcp(args) => args.agent_flag.agent.as_deref(),
            Commands::Tasks(args) => args.agent_flag.agent.as_deref(),
            Commands::Agents(args) => args.agent_flag.agent.as_deref(),
            // No agent override — listed explicitly so adding a new Commands variant
            // produces a compile error, forcing a conscious scoping decision.
            Commands::Setup { .. }
            | Commands::Doctor(_)
            | Commands::Teams(_)
            | Commands::Dashboard(_) => None,
        }
    }

    /// Extract `--team` override (available on `chat` and `ask`).
    pub fn team_override(&self) -> Option<&str> {
        match self {
            Commands::Chat(args) => args.team.as_deref(),
            Commands::Ask(args) => args.team.as_deref(),
            // Listed explicitly for compile-time safety on new variants.
            Commands::Setup { .. }
            | Commands::Memory(_)
            | Commands::Reminders(_)
            | Commands::Status(_)
            | Commands::Config(_)
            | Commands::Skills(_)
            | Commands::Mcp(_)
            | Commands::Tasks(_)
            | Commands::Agents(_)
            | Commands::Doctor(_)
            | Commands::Teams(_)
            | Commands::Dashboard(_) => None,
        }
    }
}

#[derive(clap::Args)]
pub struct ChatArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// LLM model override for this session (e.g., sonnet, opus, haiku, openai/gpt-4o).
    /// One-shot override, not persisted to config.
    #[arg(long, conflicts_with = "team")]
    pub model: Option<String>,

    /// Team to use (launches TUI in team mode)
    #[arg(long, conflicts_with = "agent")]
    pub team: Option<String>,

    /// Continue from a previous team run (requires --team)
    #[arg(long, requires = "team", conflicts_with = "last_run")]
    pub run_id: Option<String>,

    /// Use the most recent finished team run as context (requires --team)
    #[arg(long, requires = "team", conflicts_with = "run_id")]
    pub last_run: bool,
}

#[derive(clap::Args)]
pub struct AskArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// LLM model override for this invocation (e.g., sonnet, opus, haiku, openai/gpt-4o).
    /// One-shot override, not persisted to config.
    #[arg(long, conflicts_with = "team")]
    pub model: Option<String>,

    /// The message to send (use "-" to read from stdin)
    pub message: String,
    /// Correlate this message with a task for observability. Without --task-complete,
    /// only records the task-id in session/trace metadata. With --task-complete, marks
    /// the callback task as completed.
    #[arg(long, conflicts_with = "team")]
    pub task_id: Option<String>,
    /// Signal that the task should be marked as completed (requires --task-id).
    /// Without this flag, --task-id is correlation-only.
    #[arg(long, requires = "task_id", conflicts_with = "team")]
    pub task_complete: bool,
    /// Link this message to a parent work item (metadata threading).
    /// Used by claude-asked relay: mika ask --parent-task-id <uuid> "question"
    #[arg(long)]
    pub parent_task_id: Option<String>,
    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
    /// Team to run (runs full team cycle, prints deliverable)
    #[arg(long, conflicts_with = "agent")]
    pub team: Option<String>,
    /// Continue from a previous team run (requires --team)
    #[arg(long, requires = "team", conflicts_with = "last_run")]
    pub run_id: Option<String>,

    /// Use the most recent finished team run as context (requires --team)
    #[arg(long, requires = "team", conflicts_with = "run_id")]
    pub last_run: bool,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,
}

#[derive(Clone, Default, ValueEnum)]
pub enum OutputFormat {
    /// Plain text (default)
    #[default]
    Text,
    /// JSON: {"role": "assistant", "content": "..."}
    Json,
}

#[derive(Clone, ValueEnum)]
pub enum SetupMode {
    /// CLI mode (default): configure API keys, telemetry, internal token
    Cli,
    /// Server mode: CLI config + routing URL, dashboard token, server port
    Server,
    /// Compose mode: generate a .env file for docker-compose in the current directory
    Compose,
    /// OAuth mode: authorize Mika with your Claude Pro/Max subscription via PKCE
    Oauth,
}

#[derive(clap::Args)]
pub struct AgentsArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

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
        /// Skip interactive wizard (use defaults)
        #[arg(long)]
        no_interactive: bool,
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
        /// Skip interactive wizard (requires team.toml to be created manually)
        #[arg(long)]
        no_interactive: bool,
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
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
        /// Maximum number of runs to show
        #[arg(long, short = 'n', default_value = "10")]
        limit: usize,
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
    #[command(flatten)]
    pub agent_flag: AgentFlag,

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
    /// Install a skill from a Git repository or local path
    Install {
        /// Git URL, GitHub shorthand (user/repo), or local path
        source: String,
        /// Install under a different name (alias)
        #[arg(long)]
        name: Option<String>,
        /// Create symlink instead of copy (local sources only)
        #[arg(long)]
        link: bool,
    },
    /// Uninstall a marketplace-installed skill
    Uninstall {
        /// Skill name to uninstall
        name: String,
        /// Also remove orphaned dependencies without prompting
        #[arg(long)]
        remove_deps: bool,
    },
    /// Update marketplace-installed skills
    Update {
        /// Skill name to update (omit to update all)
        name: Option<String>,
    },
    /// Validate skill manifests and handler scripts
    Validate {
        /// Skill name to validate (omit to validate all)
        name: Option<String>,
    },
}

#[derive(clap::Args)]
pub struct MemoryArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

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
    #[command(flatten)]
    pub agent_flag: AgentFlag,

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
    #[command(flatten)]
    pub agent_flag: AgentFlag,

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
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Open identity.toml in $EDITOR
    Edit,
    /// Print soul.md to stdout
    Soul,
    /// Get the value of a configuration key
    Get {
        /// Configuration key name
        key: String,
        /// Show value source (file, env, db, default)
        #[arg(long)]
        verbose: bool,
    },
    /// Set a configuration value
    Set {
        /// Configuration key name
        key: String,
        /// Value to set (omit for secret keys to use interactive prompt)
        value: Option<String>,
    },
    /// List all configuration keys and their values
    List {
        /// Show value source per key
        #[arg(long)]
        verbose: bool,
    },
}

#[derive(clap::Args)]
pub struct McpArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    #[command(subcommand)]
    pub command: Option<McpCommand>,
}

#[derive(clap::Args)]
pub struct DoctorArgs {
    /// Make a live API call to verify credentials
    #[arg(long)]
    pub verify_api: bool,
    /// Output machine-readable JSON instead of colored text
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct DashboardArgs {
    #[command(subcommand)]
    pub command: DashboardCommand,
}

#[derive(Subcommand)]
pub enum DashboardCommand {
    /// Enable the embedded dashboard on the running server
    Start,
    /// Disable the embedded dashboard on the running server
    Stop,
    /// Show dashboard status
    Status,
    /// Open dashboard in browser
    Open,
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

/// Known model shorthands: (shorthand, full_model_id, display_name).
/// Single source of truth — used by both CLI `--model` flag and TUI `/model` command.
pub const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "claude-haiku-4-5", "Claude Haiku 4.5"),
    ("gpt4o", "openai/gpt-4o", "GPT-4o"),
    ("deepseek", "deepseek/deepseek-chat", "DeepSeek Chat"),
    ("gemini", "google/gemini-2.5-flash", "Gemini 2.5 Flash"),
];

/// Resolve a model alias (e.g., "sonnet") to its full model ID (e.g., "claude-sonnet-4-6").
/// Returns the input unchanged if it's not a known alias (allows provider-prefixed names like "openai/gpt-4o").
pub fn resolve_model_alias(input: &str) -> String {
    let lower = input.to_lowercase();
    for &(alias, full_id, _display) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return full_id.to_string();
        }
    }
    input.to_string()
}
