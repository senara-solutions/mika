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
    /// Manage tokens (GitHub App installation tokens)
    Token(TokenArgs),
    /// List or switch LLM providers
    Provider(ProviderArgs),
    /// List or switch LLM models
    Model(ModelArgs),
    /// Manage gateway webhook dead-letter queue
    Webhook(WebhookArgs),
    /// Manage Knowledge Graph state
    Kg(KgArgs),
    /// Show resolved log file paths for an agent
    Logs(LogsArgs),
    /// Send an operator notification (writes to notifications session, optionally to Telegram)
    Notify(NotifyArgs),
    /// Git credential helper (used by git, not directly by users)
    #[command(name = "credential-helper")]
    CredentialHelper(CredentialHelperArgs),
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
            Commands::Provider(args) => args.agent_flag.agent.as_deref(),
            Commands::Model(args) => args.agent_flag.agent.as_deref(),
            Commands::Kg(args) => args.agent_override(),
            Commands::Logs(args) => args.agent_flag.agent.as_deref(),
            // No agent override — listed explicitly so adding a new Commands variant
            // produces a compile error, forcing a conscious scoping decision.
            Commands::Setup { .. }
            | Commands::Doctor(_)
            | Commands::Teams(_)
            | Commands::Dashboard(_)
            | Commands::Token(_)
            | Commands::Webhook(_)
            | Commands::Notify(_)
            | Commands::CredentialHelper(_) => None,
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
            | Commands::Dashboard(_)
            | Commands::Token(_)
            | Commands::Provider(_)
            | Commands::Model(_)
            | Commands::Webhook(_)
            | Commands::Kg(_)
            | Commands::Logs(_)
            | Commands::Notify(_)
            | Commands::CredentialHelper(_) => None,
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
    /// Link this message to a parent task (metadata threading).
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

    /// Force named skill(s) to be active for this invocation (repeatable).
    /// Mutually exclusive with --disable-skill per skill name.
    /// Example: --enable-skill self-dev --enable-skill qa-review
    /// Not supported in team mode (use DB overrides instead).
    #[arg(long, conflicts_with = "team")]
    pub enable_skill: Vec<String>,

    /// Transiently disable named skill(s) for this invocation (repeatable).
    /// Evicts the skill from the registry — prevents both always_on and keyword activation.
    /// Mutually exclusive with --enable-skill per skill name.
    /// Example: --disable-skill self-dev
    /// Not supported in team mode (use DB overrides instead).
    #[arg(long, conflicts_with = "team")]
    pub disable_skill: Vec<String>,

    /// Emit metadata trailer after response (e.g., session_id).
    /// Useful for cross-command integration where downstream consumers need session context.
    #[arg(long, conflicts_with = "team")]
    pub verbose: bool,
}

#[derive(clap::Args)]
pub struct StatusArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
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
    List {
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
    /// Validate agent configuration
    Validate {
        /// Agent name to validate (omit to validate all)
        name: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Wipe all state for an agent without deleting the agent itself
    Reset {
        /// Agent name to reset
        name: String,
        /// Skip active-task safety check
        #[arg(long)]
        force: bool,
        /// Preview what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,
        /// Skip confirmation prompt
        #[arg(long, short)]
        yes: bool,
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
    List {
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
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
    /// Validate team configuration
    Validate {
        /// Team name to validate (omit to validate all)
        name: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
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
    List {
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
    /// Install a skill from a Git repository, local path, or the bundled-skill library.
    Install {
        /// Git URL, GitHub shorthand (user/repo), local path, or — with `--copy` —
        /// a bundled skill name from the canonical library at `~/.mika/skills/`.
        source: String,
        /// Install under a different name (alias). Ignored when `--copy` is set.
        #[arg(long)]
        name: Option<String>,
        /// Create symlink instead of copy (local sources only). Mutually exclusive
        /// with `--copy`.
        #[arg(long, conflicts_with = "copy")]
        link: bool,
        /// Materialize a real per-agent directory copy of a bundled skill from
        /// the library (mika#1213). The default install for bundled skills is
        /// a symlink into `~/.mika/skills/<skill>`; `--copy` opts out so the
        /// operator can hot-patch the deployed skill without bumping the
        /// binary. Library sync will not touch `--copy`-managed directories.
        #[arg(long)]
        copy: bool,
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
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Manage per-skill LLM provider/model overrides
    Llm {
        /// Skill name
        name: String,
        #[command(subcommand)]
        action: SkillLlmAction,
    },
    /// Manage per-provider/per-model prompt variants
    Variants {
        #[command(subcommand)]
        action: SkillVariantsAction,
    },
}

#[derive(Subcommand)]
pub enum SkillLlmAction {
    /// Set the LLM override for this skill (provider/model, e.g. anthropic/claude-sonnet-4-6)
    Set {
        /// Provider/model spec, e.g. `anthropic/claude-sonnet-4-6` or `deepseek/deepseek-chat`
        model: String,
    },
    /// Clear the LLM override and revert to manifest default
    Reset,
    /// Show the effective LLM provider/model and its source
    Show,
}

#[derive(Subcommand)]
pub enum SkillVariantsAction {
    /// List all variants for a skill
    List {
        /// Skill name
        name: String,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Show cross-skill variant summary
    Status {
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Show diff between variant and base prompt
    Diff {
        /// Skill name
        name: String,
        /// Variant specifier: provider/model (e.g. anthropic/claude-sonnet-4-6)
        variant: String,
        /// Variant source filter: hand-authored, generated, or experimental
        #[arg(long)]
        source: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Reflect on variant impact after base prompt edit
    Reflect {
        /// Skill name
        name: String,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Validate variant prompts against the 4-rule gate
    Validate {
        /// Skill name (omit to validate all)
        name: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Promote an experimental variant to generated (after validation)
    Promote {
        /// Skill name
        name: String,
        /// Variant specifier: provider/model (e.g. anthropic/claude-sonnet-4-6)
        variant: String,
    },
    /// Print the command to regenerate a variant (prepare-only in v1)
    Regen {
        /// Skill name
        name: String,
        /// Variant specifier: provider/model (e.g. anthropic/claude-sonnet-4-6)
        variant: String,
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
    Search {
        query: String,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
    /// List pending reminders
    List {
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Show details for a specific reminder
    Get {
        /// Reminder ID
        id: String,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
    /// List active tasks
    List {
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Show details for a specific task
    Get {
        /// Task ID
        id: String,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
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

#[derive(clap::Args)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub command: TokenCommand,
}

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Print a GitHub App installation token to stdout
    Github,
}

#[derive(clap::Args)]
pub struct ProviderArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,

    /// Provider name to switch to (omit to list all)
    pub name: Option<String>,

    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
}

#[derive(Subcommand)]
pub enum ProviderSubcommand {
    /// Set a provider config field
    Set {
        /// Field to set: model, api-key, or base-url
        #[arg(value_enum)]
        field: ProviderField,
        /// Value (required for model/base-url, omit for api-key to use interactive prompt)
        value: Option<String>,
    },
}

#[derive(Clone, ValueEnum)]
pub enum ProviderField {
    /// Set the model for the current provider
    Model,
    /// Set the API key (interactive prompt, never passed as argument)
    ApiKey,
    /// Set the base URL for the current provider
    BaseUrl,
}

#[derive(clap::Args)]
pub struct ModelArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,

    /// Model name or alias to switch to (omit to list)
    pub name: Option<String>,
}

#[derive(clap::Args)]
pub struct LogsArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(clap::Args)]
pub struct NotifyArgs {
    /// Notification message text
    #[arg(long)]
    pub text: String,

    /// Delivery channel: cli (default, writes to DB only) or telegram (also sends via gateway)
    #[arg(long, value_enum, default_value = "cli")]
    pub channel: NotifyChannel,

    /// Severity level for the notification
    #[arg(long, value_enum, default_value = "info")]
    pub severity: NotifySeverity,
}

#[derive(Clone, Default, ValueEnum)]
pub enum NotifyChannel {
    /// Write to notifications session only
    #[default]
    Cli,
    /// Write to notifications session and send via Telegram gateway
    Telegram,
}

#[derive(Clone, Default, ValueEnum)]
pub enum NotifySeverity {
    /// Informational notification
    #[default]
    Info,
    /// Warning — operator attention recommended
    Warn,
    /// Escalation — operator action required
    Escalate,
}

#[derive(clap::Args)]
pub struct CredentialHelperArgs {
    /// Operation: get, store, or erase
    pub operation: String,
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

#[derive(clap::Args)]
pub struct KgArgs {
    #[command(subcommand)]
    pub command: KgCommand,
}

impl KgArgs {
    /// Extract `--agent` override from whichever KG subcommand carries it.
    pub fn agent_override(&self) -> Option<&str> {
        match &self.command {
            KgCommand::Status(args) => args.agent_flag.agent.as_deref(),
            KgCommand::ListAgents(args) => args.agent_flag.agent.as_deref(),
            KgCommand::Purge(args) => Some(args.agent.as_str()),
            // validate is workspace-wide, no agent filter
            KgCommand::Validate(_) => None,
        }
    }
}

#[derive(Subcommand)]
pub enum KgCommand {
    /// Show KG state summary (all agents or one)
    Status(KgStatusArgs),
    /// List agents with KG state
    #[command(name = "list-agents")]
    ListAgents(KgListAgentsArgs),
    /// Purge an agent's KG state
    Purge(KgPurgeArgs),
    /// Check for orphan rows and FK violations
    Validate(KgValidateArgs),
}

#[derive(clap::Args)]
pub struct KgStatusArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(clap::Args)]
pub struct KgListAgentsArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(clap::Args)]
pub struct KgPurgeArgs {
    /// Agent whose KG state to purge (required)
    #[arg(long)]
    pub agent: String,

    /// Skip interactive confirmation
    #[arg(long)]
    pub yes: bool,

    /// Also delete shared-corpus rows if no other agent references them
    #[arg(long)]
    pub include_orphaned_corpus: bool,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(clap::Args)]
pub struct KgValidateArgs {
    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

/// Known model shorthands: (shorthand, full_model_id, display_name).
/// Single source of truth — used by both CLI `--model` flag and TUI `/model` command.
pub const MODEL_ALIASES: &[(&str, &str, &str)] = &[
    ("sonnet", "anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6"),
    ("opus", "anthropic/claude-opus-4-6", "Claude Opus 4.6"),
    ("haiku", "anthropic/claude-haiku-4-5", "Claude Haiku 4.5"),
    ("gpt4o", "openai/gpt-4o", "GPT-4o"),
    ("deepseek", "deepseek/deepseek-chat", "DeepSeek Chat"),
    ("gemini", "google/gemini-2.5-flash", "Gemini 2.5 Flash"),
];

/// Resolve a model alias (e.g., "sonnet") to its full model ID (e.g., "anthropic/claude-sonnet-4-6").
/// All aliases include their provider prefix for cross-provider correctness.
/// Returns the input unchanged if it's not a known alias.
pub fn resolve_model_alias(input: &str) -> String {
    let lower = input.to_lowercase();
    for &(alias, full_id, _display) in MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return full_id.to_string();
        }
    }
    input.to_string()
}

#[derive(clap::Args)]
pub struct WebhookArgs {
    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,

    #[command(subcommand)]
    pub command: WebhookCommand,
}

#[derive(Subcommand)]
pub enum WebhookCommand {
    /// List dead-letter queue entries (pending + dead)
    #[command(name = "list-dead")]
    ListDead {
        /// Filter by status: "pending" or "dead" (omit for both)
        #[arg(long)]
        status: Option<String>,
        /// Maximum entries to return (default 100)
        #[arg(long)]
        limit: Option<i64>,
    },
    /// Replay a single DLQ entry by delivery ID
    Replay {
        /// The delivery ID to replay
        delivery_id: String,
    },
    /// Replay all dead DLQ entries
    #[command(name = "replay-all")]
    ReplayAll,
}
