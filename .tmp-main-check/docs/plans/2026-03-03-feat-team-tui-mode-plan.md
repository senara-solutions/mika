---
title: "feat: Add --team CLI option for TUI team mode"
type: feat
status: completed
date: 2026-03-03
---

# feat: Add --team CLI option for TUI team mode

## Overview

Add a mutually exclusive `--team <name>` global CLI option alongside the existing `--agent <name>` option. When specified, the TUI launches in "team mode" where each user message is treated as a goal submitted to `run_team()`, with progress updates streaming as system messages and the deliverable displayed as the assistant response.

## Problem Statement / Motivation

Teams currently only work via the non-interactive `mika teams run <name> <goal>` CLI command. The TUI `/team` slash command explicitly redirects users to the CLI. This creates a disconnected experience — users must leave the TUI to work with teams, losing the benefits of the interactive chat interface (progressive reveal, scrolling, history, slash commands).

## Proposed Solution

### Architecture: `AppMode` enum on `App`

The `App` struct gains a `mode: AppMode` field that distinguishes agent mode from team mode. Agent-specific resources (`db`, `claude`, `skills`, `home_dir`, agent worker channels) move into the `Agent` variant. Team-specific state (`team_name`, `settings`, `global_home`) lives in the `Team` variant. Shared TUI state (`messages`, `textarea`, `status`, `scroll_offset`, etc.) remains directly on `App`.

```rust
// crates/mika-cli/src/tui/app.rs
enum AppMode {
    Agent {
        db: AsyncDatabase,
        claude: ClaudeClient,
        skills: Arc<SkillRegistry>,
        home_dir: PathBuf,
        agent_name: String,
        // ... other agent-specific fields
    },
    Team {
        team_name: String,
        settings: Settings,
        global_home: PathBuf,
    },
}
```

### Worker: New `TeamRequest` / `TeamResponse` channel protocol

Team mode uses a separate worker task that receives goals and streams responses:

```rust
enum TeamRequest {
    Goal(String),
    Quit,
}

enum TeamResponse {
    Progress(String),         // "Decomposing goal...", "Running researcher..."
    Deliverable(String),      // Final team output
    Error(String),            // Team execution failed
}
```

The `ProgressCallback` closure captures an `mpsc::UnboundedSender<TeamResponse>` and sends `Progress` messages through it. The TUI `tick()` handler polls this channel alongside the existing agent channel.

### CLI: `--team` with `conflicts_with`

```rust
// crates/mika-cli/src/cli.rs
pub struct Cli {
    #[arg(long, global = true)]
    pub agent: Option<String>,

    #[arg(long, global = true, conflicts_with = "agent")]
    pub team: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}
```

### Flow

1. `main()` checks `cli.team` before `cli.agent`
2. If `--team` is set: validate team exists, validate subcommand compatibility (only `None`/`Chat` allowed), then call `commands::chat::run_team_mode(&team_name)`
3. `run_team_mode()` loads `Settings::load(global_home)`, spawns a team worker, builds `App` in team mode
4. User types a goal -> sent as `TeamRequest::Goal` -> worker calls `run_team()` with a progress callback -> progress/deliverable streamed back to TUI
5. Slash commands are gated by `app.is_team_mode()` — agent-specific commands return error messages

## Technical Considerations

### Architecture

- **`App` struct refactoring**: Agent-specific fields (`db`, `claude`, `skills`, `home_dir`, `agent_name`, `thinking_level`, `context_tokens`, `last_seen_msg_id`) move into `AppMode::Agent`. Accessors like `app.db()` become methods that return `Option<&AsyncDatabase>` or panic with a descriptive message. Slash command handlers check mode before accessing agent resources.
- **Worker abstraction**: Rather than a unified worker trait, use separate worker types (`AgentWorker` stays as-is, new `TeamWorker` is simpler). The `App` holds either via enum or separate optional fields.
- **Channel unification**: Both agent and team modes can use a unified `WorkerResponse` enum with variants for agent responses and team responses, polled in the same `tick()` call.

### Subcommand gating

In `main()`, when `--team` is set, only `None` (bare `mika`) and `Some(Commands::Chat)` proceed. All other subcommands bail with: `"--team cannot be used with the '{cmd}' subcommand. Use 'mika teams run {name} \"goal\"' for non-interactive team runs."`

### Slash commands in team mode

| Command | Team mode behavior |
|---|---|
| `/help` | Show team-mode-specific help |
| `/clear` | Works (clears display) |
| `/exit`, `/quit`, `/q` | Works (exits TUI) |
| `/export` | Works (exports `app.messages`) |
| `/teams` | Works (lists teams from filesystem) |
| `/agents` | Works (informational) |
| `/model` | Disabled — "Model switching is not available in team mode" |
| `/think` | Disabled — "Thinking levels are not available in team mode" |
| `/agent`, `/switch` | Disabled — "Agent switching is not available in team mode" |
| `/memory` | Disabled — "Memory is agent-specific, not available in team mode" |
| `/reminders` | Disabled — "Reminders are agent-specific, not available in team mode" |
| `/status` | Shows team info (name, agent count, latest run status) |
| `/compact` | Disabled |
| `/soul` | Disabled |
| `/config` | Disabled |
| `/skills`, `/skill` | Disabled |
| `/team` | Shows current team info (no args), error if trying to switch |
| `/attach` | Disabled — "Image attachments are not supported in team mode" |

### Header/Footer in team mode

- **Header**: "Team: {team_name}" instead of agent identity name
- **Footer**: `team: {name} | status | / commands | Ctrl+C quit` (no model, no think level, no context tokens)

### Progress message rendering

Use `ChatRole::System` for progress messages (the existing gray/dim style, not the error red). If system messages currently render as red, add a `ChatRole::Progress` variant with a distinct style (e.g., dim yellow or blue).

### Conversation persistence

- **Input history**: Stored at `{team_dir}/.input_history` (e.g., `~/.mika/teams/research/.input_history`)
- **Display history on startup**: Load the latest completed run from `{team_dir}/history/` and display the goal + deliverable as the initial two messages
- **Export path**: `{team_dir}/exports/`

### Cancellation

MVP: `JoinHandle::abort()` on the team worker when `/exit` or Ctrl+C is pressed during a running team. This is acceptable since the team engine already saves run history at each phase boundary. Future: add `CancellationToken` to `TeamEngine` for cooperative shutdown.

### No MCP, no reminders, no scheduler

Team mode skips MCP connection, reminder recovery, and scheduler spawning. These are agent-specific features that the team engine handles internally for each specialist agent.

## Acceptance Criteria

- [x] `mika --team research` launches TUI in team mode with team name in header
- [x] `mika --agent work --team research` errors with clap mutual exclusion message
- [x] `mika --team nonexistent` errors before entering TUI: "Team 'nonexistent' not found"
- [x] `mika --team research memory` errors: "--team cannot be used with the 'memory' subcommand"
- [x] User message in team mode triggers `run_team()` with the message as goal
- [x] Progress updates from team engine appear as system messages in TUI
- [x] Team deliverable appears as assistant response with progressive reveal
- [x] Team execution failure shows error message in TUI
- [x] Agent-specific slash commands (`/model`, `/think`, `/memory`, `/agent`, etc.) show "not available in team mode" messages
- [x] Universal slash commands (`/help`, `/clear`, `/exit`, `/export`, `/teams`) work in team mode
- [x] `/status` in team mode shows team info (name, agents, latest run)
- [x] Footer shows team name and status (no model/think/context tokens)
- [x] Input history persists per-team at `{team_dir}/.input_history`
- [x] Latest completed team run loads as initial messages on TUI startup
- [x] Ctrl+C during team execution aborts the run and exits cleanly
- [x] All existing agent-mode tests continue to pass (`cargo test`)
- [x] New tests for CLI mutual exclusion, team mode init, slash command gating

## MVP

### `crates/mika-cli/src/cli.rs` — Add `--team` flag

```rust
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
```

### `crates/mika-cli/src/main.rs` — Branch on `--team` early

```rust
// After CLI parse, before agent resolution:
if let Some(team_name) = cli.team {
    let team_name = mika_common::team::normalize_team_name(&team_name);
    let global_home = home::resolve_home_dir()?;
    if !mika_common::team::team_exists(&global_home, &team_name) {
        anyhow::bail!("Team '{team_name}' not found.");
    }

    // Only allow bare `mika --team` or `mika --team chat`
    match cli.command {
        None | Some(Commands::Chat) => {
            // Init logging with global home (no agent-specific dir)
            let _log_guard = mika_common::logging::init_pretty(
                "warn", Some(&global_home.join("logs")), LogOutput::FileOnly,
            );
            return commands::chat::run_team(&team_name, &global_home).await;
        }
        Some(cmd) => {
            anyhow::bail!(
                "--team cannot be used with subcommands other than 'chat'. \
                 Use 'mika teams run {team_name} \"goal\"' for non-interactive team runs."
            );
        }
    }
}
// ... existing agent resolution follows
```

### `crates/mika-cli/src/commands/chat.rs` — New `run_team()` entry point

```rust
pub async fn run_team(team_name: &str, global_home: &Path) -> Result<()> {
    let settings = Settings::load(global_home)?;
    let team_dir = mika_common::team::team_dir(global_home, team_name);

    let (team_tx, team_rx) = mpsc::unbounded_channel::<TeamRequest>();
    let (response_tx, response_rx) = mpsc::unbounded_channel::<TeamResponse>();

    // Spawn team worker
    let worker_settings = settings.clone();
    let worker_home = global_home.to_path_buf();
    let worker_name = team_name.to_string();
    let handle = tokio::spawn(async move {
        team_worker_loop(worker_name, worker_home, worker_settings, team_rx, response_tx).await;
    });

    // Build App in team mode
    let mut app = App::new_team(team_tx, response_rx, team_name, team_dir);

    // Load latest run as initial history
    app.load_team_history(global_home, team_name);

    // ... TUI event loop (reuse existing structure) ...
}
```

### `crates/mika-cli/src/tui/app.rs` — `AppMode` enum + team constructor

```rust
pub enum AppMode {
    Agent {
        db: AsyncDatabase,
        claude: ClaudeClient,
        skills: Arc<SkillRegistry>,
        home_dir: PathBuf,
        agent_name: String,
        session_id: String,
        // ... agent-specific fields
    },
    Team {
        team_name: String,
        team_dir: PathBuf,
        team_tx: mpsc::UnboundedSender<TeamRequest>,
        team_rx: mpsc::UnboundedReceiver<TeamResponse>,
    },
}

impl App<'_> {
    pub fn is_team_mode(&self) -> bool {
        matches!(self.mode, AppMode::Team { .. })
    }
}
```

### `crates/mika-cli/src/tui/commands/handlers.rs` — Gate agent-specific commands

```rust
fn handle_slash_command(app: &mut App, cmd: &str, args: &str) -> Option<String> {
    // Team mode gate for agent-specific commands
    if app.is_team_mode() {
        match cmd {
            "model" | "think" | "agent" | "switch" | "memory" | "reminders"
            | "compact" | "soul" | "config" | "skills" | "skill" | "attach" => {
                return Some(format!("/{cmd} is not available in team mode."));
            }
            _ => {} // Universal commands fall through
        }
    }
    // ... existing dispatch ...
}
```

## Sources

### Internal References

- CLI args: `crates/mika-cli/src/cli.rs`
- Main dispatch: `crates/mika-cli/src/main.rs:87-113`
- Chat command: `crates/mika-cli/src/commands/chat.rs`
- Init context: `crates/mika-cli/src/init.rs:14-79`
- TUI App state: `crates/mika-cli/src/tui/app.rs:1-66`
- Slash handlers: `crates/mika-cli/src/tui/commands/handlers.rs:607-624`
- Team engine: `crates/mika-agent/src/teams/mod.rs:19-31`
- Team types: `crates/mika-agent/src/teams/types.rs`
- Team engine impl: `crates/mika-agent/src/teams/engine.rs:34-57`
- Teams CLI command: `crates/mika-cli/src/commands/teams.rs:153-203`
- Team common: `crates/mika-common/src/team.rs`

### Institutional Learnings

- Agent-team management tools integration: `docs/solutions/integration-issues/agent-team-management-tools-integration.md` — `global_home_dir` vs `home_dir` distinction is critical
- MCP CLI integration: `docs/solutions/integration-issues/mcp-http-headers-cli-integration.md` — pattern for adding global CLI flags
