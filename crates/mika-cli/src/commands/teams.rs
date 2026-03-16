use anyhow::{Result, bail};
use std::io::{self, IsTerminal, Write};

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, format_unix_ts};
use mika_common::home;
use mika_common::team;

use crate::cli::{TeamsArgs, TeamsCommand};
use crate::wizard;

pub async fn run(args: TeamsArgs) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;

    match args.command {
        TeamsCommand::List => list(&global_home),
        TeamsCommand::Create {
            name,
            no_interactive,
        } => create(&global_home, &name, no_interactive),
        TeamsCommand::Status { name } => status(&global_home, &name),
        TeamsCommand::Log { name } => log(&global_home, &name),
        TeamsCommand::Delete { name, force } => delete(&global_home, &name, force),
    }
}

/// Open the shared container database (sync, for read-only CLI commands).
fn open_container_db(global_home: &std::path::Path) -> Result<Database> {
    let db_path = home::container_db_path(global_home);
    if !db_path.exists() {
        bail!("No database found. Run `mika` first to initialize.");
    }
    Database::open(&db_path).map_err(|e| anyhow::anyhow!("Failed to open database: {e}"))
}

/// Open the shared container database (async, for team run command).
pub(crate) fn open_container_db_async(global_home: &std::path::Path) -> Result<AsyncDatabase> {
    let db_path = home::container_db_path(global_home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let db = Database::open(&db_path)?;
    Ok(AsyncDatabase::new(db))
}

fn list(global_home: &std::path::Path) -> Result<()> {
    let teams = team::list_teams(global_home);

    if teams.is_empty() {
        println!("\n  No teams found. Run `mika teams create <name>` to create one.\n");
        return Ok(());
    }

    println!("\n  Teams:");
    for name in &teams {
        match team::load_team(global_home, name) {
            Ok(def) => {
                println!("    {name} ({} agents)", def.agents.len());
            }
            Err(e) => {
                println!("    {name} (error loading: {e})");
            }
        }
    }
    println!();
    Ok(())
}

fn create(global_home: &std::path::Path, name: &str, no_interactive: bool) -> Result<()> {
    let name = team::normalize_team_name(name);
    team::validate_team_name(&name)?;

    if team::team_exists(global_home, &name) {
        bail!("Team '{name}' already exists.");
    }

    let agents = mika_common::agent::list_agents(global_home);
    if agents.len() < 2 {
        bail!(
            "At least 2 agents are needed to create a team. \
             Create agents first with `mika agents create <name>`."
        );
    }

    let interactive = !no_interactive && std::io::stdin().is_terminal();

    if !interactive {
        bail!(
            "Team creation requires an interactive terminal.\n  \
             Use `mika teams create {name}` in a terminal, \
             or create teams/{name}/team.toml manually."
        );
    }

    let result = wizard::run_team_wizard(&name, &agents)?;

    // Build orchestrator agent entry
    let mut team_agents = vec![mika_common::team::TeamAgent {
        name: result.orchestrator.clone(),
        role: "orchestrator".to_string(),
        mandate: "Decompose goals into tasks and coordinate the team".to_string(),
    }];
    team_agents.extend(result.agents);

    let def = mika_common::team::TeamDefinition {
        team: mika_common::team::TeamMeta {
            name: name.clone(),
            orchestrator: result.orchestrator,
        },
        agents: team_agents,
        flow: mika_common::team::TeamFlow {
            max_iterations: result.max_iterations,
        },
    };

    // Write team.toml
    let dir = team::team_dir(global_home, &name);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(team::workspace_base_dir(global_home, &name))?;

    let toml_content = toml::to_string_pretty(&def)?;
    std::fs::write(dir.join("team.toml"), toml_content)?;

    println!(
        "\n  Created team '{name}' with {} agents.",
        def.agents.len()
    );
    println!("  Run it with: mika ask --team {name} \"<goal>\"\n");
    Ok(())
}

fn status(global_home: &std::path::Path, name: &str) -> Result<()> {
    let name = team::normalize_team_name(name);

    if !team::team_exists(global_home, &name) {
        bail!("Team '{name}' not found.");
    }

    let def = team::load_team(global_home, &name)?;

    println!("\n  Team: {}", def.team.name);
    println!("  Orchestrator: {}", def.team.orchestrator);
    println!("  Agents:");
    for agent in &def.agents {
        println!(
            "    {} (role: {}): {}",
            agent.name, agent.role, agent.mandate
        );
    }
    println!("  Max iterations: {}", def.flow.max_iterations);

    // Show latest run if available from shared container DB
    if let Ok(db) = open_container_db(global_home)
        && let Ok(Some(latest)) = db.load_latest_team_run(&name)
    {
        println!("\n  Latest run:");
        println!("    ID: {}", latest.id);
        println!("    Goal: {}", latest.goal);
        println!("    Status: {}", latest.status);
        println!("    Started: {}", format_unix_ts(latest.started_at));
        if let Some(ended) = latest.ended_at {
            println!("    Ended: {}", format_unix_ts(ended));
        }
    }
    println!();

    Ok(())
}

fn log(global_home: &std::path::Path, name: &str) -> Result<()> {
    let name = team::normalize_team_name(name);

    if !team::team_exists(global_home, &name) {
        bail!("Team '{name}' not found.");
    }

    let db = match open_container_db(global_home) {
        Ok(db) => db,
        Err(_) => {
            println!("\n  No runs found for team '{name}'.\n");
            return Ok(());
        }
    };
    let runs = db.load_team_runs(&name, 50)?;

    if runs.is_empty() {
        println!("\n  No runs found for team '{name}'.\n");
        return Ok(());
    }

    println!("\n  Run history for team '{name}':");
    for run in &runs {
        let started = format_unix_ts(run.started_at);
        let ended = run
            .ended_at
            .map(format_unix_ts)
            .unwrap_or_else(|| "in progress".to_string());
        println!(
            "    [{}] {} | {} -> {}",
            run.id.get(..8).unwrap_or(&run.id),
            started.get(..10).unwrap_or(&started),
            run.status,
            ended.get(..10).unwrap_or(&ended)
        );
        println!("      Goal: {}", run.goal);
    }
    println!();

    Ok(())
}

fn delete(global_home: &std::path::Path, name: &str, force: bool) -> Result<()> {
    let name = team::normalize_team_name(name);

    if !team::team_exists(global_home, &name) {
        bail!("Team '{name}' not found.");
    }

    if !force {
        print!("  Delete team '{name}' and all its data? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let dir = team::team_dir(global_home, &name);
    std::fs::remove_dir_all(&dir)?;
    println!("  Deleted team '{name}'.");
    Ok(())
}
