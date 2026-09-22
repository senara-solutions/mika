use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
// mika#2066 AC1 — `--version` names the commit: `mika 0.12.2 (968dbe94)`. The
// stamp is sourced from the shared `mika_common::build_info` capture, not a
// per-crate `build.rs`. clap prints "{bin} {version}" and short-circuits before
// any config load, so an unconfigured install can still state its provenance.
#[command(
    name = "mika",
    version = mika_common::build_info::version_static(),
    about = "Mika — AI Executive Assistant"
)]
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

    /// Continue the most recent session or team run
    #[arg(short = 'c', long = "continue")]
    pub continue_session: bool,

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
    /// Milestone-scope operational coordinator (mika-manager Phase 1, LECTURE seule)
    Milestone(MilestoneArgs),
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
            Commands::Logs(args) => args.agent_override(),
            // No agent override — listed explicitly so adding a new Commands variant
            // produces a compile error, forcing a conscious scoping decision.
            Commands::Setup { .. }
            | Commands::Doctor(_)
            | Commands::Teams(_)
            | Commands::Dashboard(_)
            | Commands::Token(_)
            | Commands::Webhook(_)
            | Commands::Notify(_)
            | Commands::Milestone(_)
            | Commands::CredentialHelper(_) => None,
        }
    }

    /// Extract `-c`/`--continue` override (available on `chat` and `ask`).
    pub fn continue_override(&self) -> bool {
        match self {
            Commands::Chat(args) => args.continue_session,
            Commands::Ask(args) => args.continue_session,
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
            | Commands::Milestone(_)
            | Commands::CredentialHelper(_) => false,
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
            | Commands::Milestone(_)
            | Commands::CredentialHelper(_) => None,
        }
    }
}

#[derive(clap::Args)]
pub struct ChatArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Model-id override for this session (e.g., sonnet, opus, claude-sonnet-4-6).
    /// One-shot, not persisted. Routes through the agent's configured llm_provider —
    /// does NOT re-dispatch based on the model-name prefix.
    #[arg(long, conflicts_with = "team")]
    pub model: Option<String>,

    /// Team to use (launches TUI in team mode)
    #[arg(long, conflicts_with = "agent")]
    pub team: Option<String>,

    /// Continue from a previous team run (requires --team)
    #[arg(long, requires = "team", conflicts_with_all = ["last_run", "continue_session"])]
    pub run_id: Option<String>,

    /// Use the most recent finished team run as context (requires --team; deprecated, use -c instead)
    #[arg(long, requires = "team", conflicts_with_all = ["run_id", "continue_session"])]
    pub last_run: bool,

    /// Continue the most recent session or team run
    #[arg(short = 'c', long = "continue", conflicts_with_all = ["run_id", "last_run"])]
    pub continue_session: bool,

    /// Launch directly in audit mode (show internal messages, equivalent to typing `/inbox` after launch).
    /// Default off — chat opens in inbox mode (internal messages filtered).
    #[arg(long)]
    pub inbox: bool,
}

#[derive(clap::Args)]
pub struct AskArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Model-id override for this invocation (e.g., sonnet, opus, claude-sonnet-4-6).
    /// One-shot, not persisted. Routes through the EXECUTING agent's configured
    /// llm_provider — does NOT re-dispatch based on the model-name prefix.
    /// Reaches the execution surface on both paths, local and --remote (mika#2304).
    /// Fail-closed: an override the executing agent cannot serve fails the turn
    /// with a message naming the model and the provider, never a silent fallback.
    /// Under --verbose the reported model is the one the SERVER attests, or none.
    #[arg(long, conflicts_with = "team")]
    pub model: Option<String>,

    /// The message to send. Three doors, and they resolve in this order
    /// (mika#1982): the argument wins when present; the "-" sentinel always
    /// reads the standard input, terminal or not; and an absent argument reads
    /// the standard input when it is not a terminal. Absent on a terminal is a
    /// usage error, never a silent wait.
    pub message: Option<String>,
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
    #[arg(long, requires = "team", conflicts_with_all = ["last_run", "continue_session"])]
    pub run_id: Option<String>,

    /// Use the most recent finished team run as context (requires --team; deprecated, use -c instead)
    #[arg(long, requires = "team", conflicts_with_all = ["run_id", "continue_session"])]
    pub last_run: bool,

    /// Continue the most recent session or team run
    #[arg(short = 'c', long = "continue", conflicts_with_all = ["run_id", "last_run"])]
    pub continue_session: bool,

    /// INERT since mika#1727 — accepted, validated, and without effect on the turn.
    /// It configures this process's local skill registry, which is no longer the
    /// execution surface: the turn runs at mika-spirit. Passing it warns on stderr.
    /// To restrict a turn's skills on the server, use --only-skill.
    /// Mutually exclusive with --disable-skill per skill name.
    #[arg(long, conflicts_with = "team")]
    pub enable_skill: Vec<String>,

    /// INERT since mika#1727 — accepted, validated, and without effect on the turn.
    /// Same reason as --enable-skill above, and the same warning on stderr.
    /// To restrict a turn's skills on the server, use --only-skill.
    /// Mutually exclusive with --enable-skill per skill name.
    #[arg(long, conflicts_with = "team")]
    pub disable_skill: Vec<String>,

    /// Restrict this invocation to the named skill(s) — everything else is
    /// transiently evicted (repeatable, not persisted). Unlike --enable-skill,
    /// this one reaches the execution surface: it travels to mika-spirit in
    /// `message/send` request metadata (mika#2363).
    ///
    /// Strictly subtractive — it can never activate a skill the turn would
    /// otherwise have left inactive. Naming a skill this agent does not carry
    /// keeps nothing, so a typo yields a turn with no skills rather than a
    /// silently unrestricted one.
    ///
    /// Mutually exclusive with --enable-skill / --disable-skill: three selection
    /// semantics on one turn is a composition nobody wants to debug.
    /// Example: --only-skill mika-arch-groom-ticket
    #[arg(long, conflicts_with_all = ["team", "enable_skill", "disable_skill"])]
    pub only_skill: Vec<String>,

    /// Isolate this invocation's conversation window to its own session
    /// (mika#1951). The turn reads history under `HistoryScope::Session` and
    /// injects no compaction summary, whatever the agent's `identity.toml` says.
    ///
    /// Strictly restrictive, by construction: the flag is a bool on the wire
    /// (`mika.session_isolated`), so it can narrow a window and never widen one.
    /// A caller cannot use it to make an agent configured `session` read other
    /// sessions' history.
    ///
    /// Reaches the execution surface on both paths, local and --remote — the key
    /// is posted at the single `build_send_params` site they share.
    ///
    /// Under --verbose the reported isolation is the one the SERVER attests, never
    /// this flag: a spirit predating mika#1951 accepts the flag, ignores it, and
    /// would otherwise be reported as isolated with full authority — the false
    /// green this ticket exists to close.
    /// Example: --isolated --session-id <fresh-uuid>
    #[arg(long, conflicts_with = "team")]
    pub isolated: bool,

    /// Emit metadata trailer after response (e.g., session_id).
    /// Useful for cross-command integration where downstream consumers need session context.
    #[arg(long, conflicts_with = "team")]
    pub verbose: bool,

    /// Remote Mika endpoint to proxy this ask to (e.g.,
    /// `https://gw.example.com/a2a/{customer_id}/{agent}`). When set, bypasses the
    /// local in-process agent loop and dispatches via the A2A protocol. Also settable
    /// via the `MIKA_REMOTE_AGENT_URL` env var. The flag overrides the env var when
    /// both are set (env precedence handled in `main.rs` since the workspace clap
    /// does not enable the `env` feature). Authentication uses `MIKA_INTERNAL_TOKEN`
    /// as a bearer header. Conflicts with `--team` (team mode runs locally).
    #[arg(long, conflicts_with = "team")]
    pub remote: Option<String>,
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
    /// YAML
    Yaml,
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
    /// Show the LLM budget and model an agent is running under, with provenance
    ///
    /// Reads the record mika-spirit resolved when it initialized the agent. It
    /// is deliberately NOT computed here: this process's environment is not the
    /// server's, so a local resolution could report a setting that is not in
    /// force, with the authority of a measurement (mika#2457).
    Budget {
        /// Agent to report on (defaults to --agent, then the active agent)
        #[arg(long)]
        agent: Option<String>,
        /// Output format: text (default) or json
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Re-apply the authoritative identity.toml and soul.md for an existing agent
    Reprovision {
        /// Agent name to re-provision
        name: String,
        /// Tier template to apply (customer agents only): default | family | champion.
        /// Defaults to MIKA_AGENT_TIER. Refused for well-known agents.
        //
        // Deliberately `Option<String>` and not a clap `ValueEnum`: the tier
        // vocabulary and its fail-closed rule (mika#2023 AC2) live in
        // `AgentTier::parse`, and a `ValueEnum` would restate them here and
        // diverge the day a fourth tier arrives (mika#2230 D2).
        #[arg(long)]
        tier: Option<String>,
        /// Re-apply identity.toml only, leaving soul.md untouched
        #[arg(long)]
        identity_only: bool,
        /// Show what would be written without writing anything
        #[arg(long)]
        dry_run: bool,
        /// Skip the typed-name confirmation
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

        /// Filter by source: bundle or marketplace
        #[arg(long)]
        source: Option<String>,

        /// Filter by always-on state
        #[arg(long = "always-on")]
        always_on: Option<bool>,
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
    /// Promote a staged skill to active (mika#1582)
    Promote {
        /// Skill name
        name: String,
    },
    /// Archive an active skill (mika#1582)
    Archive {
        /// Skill name
        name: String,
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
    /// Restore an archived skill from its most recent snapshot
    Restore {
        /// Skill name to restore
        name: String,
    },
    /// Curator operations
    Curator {
        #[command(subcommand)]
        action: CuratorAction,
    },
}

#[derive(Subcommand)]
pub enum CuratorAction {
    /// Show the most recent curator review results
    Status,
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

    /// Output format: text (default), json, or yaml
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,

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

/// `mika milestone {read,assess,report}` — Phase 1 LECTURE seule.
///
/// Reads milestone state via `gh` CLI (wrapper-only INV-2), assesses it
/// (rules-driven), and reports Markdown. No dispatch, no ticket mutation,
/// no PR merge. See `mika_agent::milestone_manager` module docstring for
/// the full Phase 1 discipline.
#[derive(clap::Args)]
pub struct MilestoneArgs {
    #[command(subcommand)]
    pub command: MilestoneCommand,
}

#[derive(Subcommand)]
pub enum MilestoneCommand {
    /// Compose and print milestone state as JSON.
    Read {
        /// `<owner/repo>#<milestone_number>` — e.g. `senara-solutions/mika#1799`
        target: String,
        #[arg(long, value_enum, default_value = "json")]
        format: OutputFormat,
    },
    /// Read + apply rules; print Assessment (recommendation, alerts, severity).
    Assess {
        /// `<owner/repo>#<milestone_number>`
        target: String,
        #[arg(long, value_enum, default_value = "json")]
        format: OutputFormat,
        /// Silence threshold in JOURS (days). Default: 3.
        #[arg(long, default_value_t = 3)]
        silence_threshold_days: u32,
    },
    /// Read + assess + render the § 2d Markdown report to stdout.
    Report {
        /// `<owner/repo>#<milestone_number>`
        target: String,
        /// Silence threshold in JOURS (days). Default: 3.
        #[arg(long, default_value_t = 3)]
        silence_threshold_days: u32,
    },
    /// List Phase 1 reports written to the offline sink, most recent first
    /// (mika#2267).
    ///
    /// The cadence writes a report to the offline sink whenever no delivery
    /// URL is configured, and as a fallback when an HTTP delivery fails. Until
    /// mika#2267 nothing read that directory — this is the reader. The output
    /// always names the directory it consulted and how that path was decided,
    /// because the CLI (run by the operator) and the daemon (run by the
    /// service, possibly under another HOME) can resolve different ones.
    Reports {
        /// Restrict to one milestone: `<owner/repo>#<number>`. Absent → all.
        #[arg(long)]
        target: Option<String>,
        /// Print the most recent report's Markdown to stdout instead of
        /// listing. `--format` is ignored: the content is the output.
        #[arg(long)]
        latest: bool,
        /// How many entries to list. Ignored with `--latest`.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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
    Cancel {
        id: String,
        /// Skip the confirmation prompt when a pilot is still running under
        /// this task (mika#2335). Required in non-interactive contexts.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Re-arm a dead recurring task without waiting out the 24 h zombie-veto
    /// window and without editing the database (mika#2446).
    ///
    /// Refuses when the label has no dead row (a rearm never creates a
    /// recurrence), when it is already armed, and when its trigger is not
    /// routable by this binary. The act is recorded in `audit_events`
    /// (`tool_name = 'recurring_operator_rearm'`).
    Rearm {
        /// Recurring task label (e.g. `worktree_reap`)
        label: String,
    },
    /// Force-promote the next pending deferred dispatch wrapper for a class.
    /// Fails if the per-class dispatch slot is occupied, unless --override is set.
    PromoteDeferred {
        /// Dispatch class: 'implement' or 'groom'
        class: String,
        /// Cancel the slot-occupying callback first, then promote
        #[arg(long = "override")]
        r#override: bool,
    },
    /// List `ready` issues whose task is stuck: `pending` past the reaper's
    /// grace window with nothing in the dispatch queue representing it.
    ///
    /// This is the silence-breaker for mika#2045 — such an issue carries
    /// `ready`, the queue counts it, and nothing ever picks it up. Exit code is
    /// always 0: this is a probe, not a gate.
    Stuck {
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Subscribe to mika-spirit's task-lifecycle SSE stream and print each
    /// TaskEventFrame as it arrives (one JSON object per line).
    ///
    /// mika#1758 stub consumer for `GET /api/v1/dashboard/tasks/stream`.
    /// Requires a running mika-spirit and `MIKA_INTERNAL_TOKEN` /
    /// `MIKA_DASHBOARD_TOKEN` in the environment. Diagnostic — not part
    /// of the customer-facing surface.
    Stream {
        /// Override mika-spirit base URL (defaults to $MIKA_SPIRIT_URL /
        /// http://localhost:$MIKA_SPIRIT_PORT / http://localhost:8080).
        #[arg(long)]
        url: Option<String>,
    },
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
    #[command(subcommand)]
    pub command: Option<LogsCommand>,

    /// Agent to use (overrides active agent)
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

impl LogsArgs {
    /// Extract `--agent` override from whichever Logs subcommand carries it.
    pub fn agent_override(&self) -> Option<&str> {
        match &self.command {
            Some(LogsCommand::Paths(args)) => args.agent_flag.agent.as_deref(),
            Some(LogsCommand::Activity(args)) => args.agent_flag.agent.as_deref(),
            None => self.agent_flag.agent.as_deref(),
        }
    }
}

#[derive(Subcommand)]
pub enum LogsCommand {
    /// Show resolved log file paths
    Paths(LogsPathsArgs),
    /// Query cross-surface activity (messages, LLM calls, tool calls, tasks)
    Activity(LogsActivityArgs),
}

#[derive(clap::Args)]
pub struct LogsPathsArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Output format: text (default) or json
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
}

#[derive(clap::Args)]
pub struct LogsActivityArgs {
    #[command(flatten)]
    pub agent_flag: AgentFlag,

    /// Filter by session ID (prefix match supported)
    #[arg(long)]
    pub session: Option<String>,

    /// Filter by task ID
    #[arg(long)]
    pub task: Option<String>,

    /// Filter by trace ID
    #[arg(long)]
    pub trace: Option<String>,

    /// Time window start: "30m", "2h", "1d", "today", or ISO 8601 timestamp.
    /// Default: "1h"
    #[arg(long, default_value = "1h")]
    pub since: String,

    /// Time window end (same format as --since). Default: now.
    #[arg(long)]
    pub until: Option<String>,

    /// Surfaces to include (comma-separated). Valid: messages, llm_calls, tool_calls, tasks, server_log.
    /// Default: "messages,llm_calls,tool_calls,tasks" (all DB surfaces).
    #[arg(long, default_value = "messages,llm_calls,tool_calls,tasks")]
    pub include: String,

    /// Filter content by substring pattern
    #[arg(long)]
    pub grep: Option<String>,

    /// Maximum number of events to display
    #[arg(long, short = 'n', default_value = "200")]
    pub limit: usize,

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
    List {
        /// Output format: text (default), json, or yaml
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
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

// Model aliases and alias resolution moved to `mika_common::llm::model_override`
// with mika#2304: since mika#1727 the turn runs in mika-spirit, so the id must be
// resolved against the *executing* agent's provider, and `mika-agent` needs to
// reach the same implementation this crate uses. This re-export keeps the three
// listing call sites (`commands::model`, the TUI `/model` handler and its
// completer) reading `crate::cli::MODEL_ALIASES` unchanged — a re-export, not a
// second definition, which is what `mika2304_the_cli_keeps_no_second_resolver`
// checks. `resolve_model_alias` is reached through its canonical path now that
// `init::override_model` calls the common entry point instead.
pub use mika_common::llm::model_override::MODEL_ALIASES;

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse_cli(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    #[test]
    fn chat_inbox_flag_default_off() {
        let cli = parse_cli(&["mika", "chat"]);
        match cli.command {
            Some(Commands::Chat(args)) => assert!(!args.inbox),
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn chat_inbox_flag_on() {
        let cli = parse_cli(&["mika", "chat", "--inbox"]);
        match cli.command {
            Some(Commands::Chat(args)) => assert!(args.inbox),
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn chat_inbox_with_agent() {
        let cli = parse_cli(&["mika", "chat", "--agent", "mika-relay", "--inbox"]);
        match cli.command {
            Some(Commands::Chat(args)) => {
                assert!(args.inbox);
                assert_eq!(args.agent_flag.agent, Some("mika-relay".to_string()));
            }
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn chat_inbox_with_team() {
        let cli = parse_cli(&["mika", "chat", "--team", "my-team", "--inbox"]);
        match cli.command {
            Some(Commands::Chat(args)) => {
                assert!(args.inbox);
                assert_eq!(args.team, Some("my-team".to_string()));
            }
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn chat_continue_short_flag_parses() {
        let cli = parse_cli(&["mika", "chat", "-c"]);
        match cli.command {
            Some(Commands::Chat(args)) => assert!(args.continue_session),
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn chat_continue_long_flag_parses() {
        let cli = parse_cli(&["mika", "chat", "--continue"]);
        match cli.command {
            Some(Commands::Chat(args)) => assert!(args.continue_session),
            _ => panic!("expected Chat command"),
        }
    }

    #[test]
    fn chat_continue_conflicts_with_run_id() {
        let result =
            Cli::try_parse_from(["mika", "chat", "-c", "--run-id", "abc", "--team", "dev"]);
        assert!(result.is_err(), "-c and --run-id should conflict");
    }

    #[test]
    fn ask_continue_parses() {
        let cli = parse_cli(&["mika", "ask", "-c", "hello"]);
        match cli.command {
            Some(Commands::Ask(args)) => assert!(args.continue_session),
            _ => panic!("expected Ask command"),
        }
    }

    #[test]
    fn ask_continue_conflicts_with_last_run() {
        let result =
            Cli::try_parse_from(["mika", "ask", "-c", "--last-run", "--team", "dev", "hello"]);
        assert!(result.is_err(), "-c and --last-run should conflict");
    }

    #[test]
    fn top_level_continue_parses() {
        let cli = parse_cli(&["mika", "-c"]);
        assert!(cli.continue_session);
        assert!(cli.command.is_none());
    }
}
