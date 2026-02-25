use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "mika", about = "Mika — AI Executive Assistant")]
pub struct Cli {
    /// Agent to use (overrides active agent)
    #[arg(long, global = true)]
    pub agent: Option<String>,

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
    /// List loaded skills or inspect a specific skill
    Skills {
        /// Show details for a specific skill by name
        name: Option<String>,
    },
    /// Send a message and print the response (non-interactive)
    Ask {
        /// The message to send (use "-" to read from stdin)
        message: String,
    },
    /// Manage agents
    Agents(AgentsArgs),
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
    /// Delete an agent (cannot delete "main")
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
    /// Cancel a reminder by ID
    Cancel { id: i64 },
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
