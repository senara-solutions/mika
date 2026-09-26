use anyhow::{Result, bail};
use std::io::{self, IsTerminal, Write};

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, format_ts};
use mika_common::home;
use mika_common::team;

use crate::cli::{TeamsArgs, TeamsCommand};
use crate::wizard;

pub async fn run(args: TeamsArgs) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;

    match args.command {
        TeamsCommand::List { format } => list(&global_home, &format),
        TeamsCommand::Create {
            name,
            no_interactive,
        } => create(&global_home, &name, no_interactive),
        TeamsCommand::Status { name, format } => status(&global_home, &name, &format),
        TeamsCommand::Log {
            name,
            format,
            limit,
        } => log(&global_home, &name, &format, limit),
        TeamsCommand::Delete { name, force } => delete(&global_home, &name, force),
        TeamsCommand::Validate { name, format } => {
            validate_teams(&global_home, name.as_deref(), &format)
        }
    }
}

/// Open the shared container database (sync, for read-only CLI commands).
pub(crate) fn open_container_db(global_home: &std::path::Path) -> Result<Database> {
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

fn list(global_home: &std::path::Path, format: &crate::cli::OutputFormat) -> Result<()> {
    let teams = team::list_teams(global_home);

    if teams.is_empty() {
        match format {
            crate::cli::OutputFormat::Json => println!("[]"),
            crate::cli::OutputFormat::Yaml => {
                print!("{}", serde_yaml::to_string(&serde_json::json!([]))?)
            }
            crate::cli::OutputFormat::Text => {
                println!("\n  No teams found. Run `mika teams create <name>` to create one.\n");
            }
        }
        return Ok(());
    }

    match format {
        crate::cli::OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = teams
                .iter()
                .map(|name| match team::load_team(global_home, name) {
                    Ok(def) => serde_json::json!({
                        "name": name,
                        "orchestrator": def.team.orchestrator,
                        "agents": def.agents.iter().map(|a| &a.name).collect::<Vec<_>>(),
                        "agent_count": def.agents.len(),
                        "max_iterations": def.flow.max_iterations,
                    }),
                    Err(e) => serde_json::json!({
                        "name": name,
                        "error": e.to_string(),
                    }),
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        crate::cli::OutputFormat::Yaml => {
            let entries: Vec<serde_json::Value> = teams
                .iter()
                .map(|name| match team::load_team(global_home, name) {
                    Ok(def) => serde_json::json!({
                        "name": name,
                        "orchestrator": def.team.orchestrator,
                        "agents": def.agents.iter().map(|a| &a.name).collect::<Vec<_>>(),
                        "agent_count": def.agents.len(),
                        "max_iterations": def.flow.max_iterations,
                    }),
                    Err(e) => serde_json::json!({
                        "name": name,
                        "error": e.to_string(),
                    }),
                })
                .collect();
            print!("{}", serde_yaml::to_string(&entries)?);
        }
        crate::cli::OutputFormat::Text => {
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
        }
    }
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

fn status(
    global_home: &std::path::Path,
    name: &str,
    format: &crate::cli::OutputFormat,
) -> Result<()> {
    let name = team::normalize_team_name(name);

    if !team::team_exists(global_home, &name) {
        bail!("Team '{name}' not found.");
    }

    let def = team::load_team(global_home, &name)?;

    let latest_run = open_container_db(global_home)
        .ok()
        .and_then(|db| db.load_latest_team_run(&name).ok().flatten());

    match format {
        crate::cli::OutputFormat::Json => {
            let mut obj = serde_json::json!({
                "team": def,
                "latest_run": serde_json::Value::Null,
            });
            if let Some(run) = latest_run {
                obj["latest_run"] = serde_json::json!({
                    "id": run.id,
                    "team_name": run.team_name,
                    "goal": run.goal,
                    "status": run.status,
                    "started_at": run.started_at,
                    "ended_at": run.ended_at,
                });
            }
            println!("{}", serde_json::to_string_pretty(&obj)?);
        }
        crate::cli::OutputFormat::Yaml => {
            let mut obj = serde_json::json!({
                "team": def,
                "latest_run": serde_json::Value::Null,
            });
            if let Some(run) = latest_run {
                obj["latest_run"] = serde_json::json!({
                    "id": run.id,
                    "team_name": run.team_name,
                    "goal": run.goal,
                    "status": run.status,
                    "started_at": run.started_at,
                    "ended_at": run.ended_at,
                });
            }
            print!("{}", serde_yaml::to_string(&obj)?);
        }
        crate::cli::OutputFormat::Text => {
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

            if let Some(latest) = latest_run {
                println!("\n  Latest run:");
                println!("    ID: {}", latest.id);
                println!("    Goal: {}", latest.goal);
                println!("    Status: {}", latest.status);
                println!("    Started: {}", format_ts(&latest.started_at));
                if let Some(ref ended) = latest.ended_at {
                    println!("    Ended: {}", format_ts(ended));
                }
            }
            println!();
        }
    }

    Ok(())
}

fn log(
    global_home: &std::path::Path,
    name: &str,
    format: &crate::cli::OutputFormat,
    limit: usize,
) -> Result<()> {
    let name = team::normalize_team_name(name);

    if !team::team_exists(global_home, &name) {
        bail!("Team '{name}' not found.");
    }

    let db = match open_container_db(global_home) {
        Ok(db) => db,
        Err(_) => {
            match format {
                crate::cli::OutputFormat::Json => println!("[]"),
                crate::cli::OutputFormat::Yaml => {
                    print!("{}", serde_yaml::to_string(&serde_json::json!([]))?)
                }
                crate::cli::OutputFormat::Text => {
                    println!("\n  No runs found for team '{name}'.\n");
                }
            }
            return Ok(());
        }
    };
    let runs = db.load_team_runs(&name, limit)?;

    if runs.is_empty() {
        match format {
            crate::cli::OutputFormat::Json => println!("[]"),
            crate::cli::OutputFormat::Yaml => {
                print!("{}", serde_yaml::to_string(&serde_json::json!([]))?)
            }
            crate::cli::OutputFormat::Text => {
                println!("\n  No runs found for team '{name}'.\n");
            }
        }
        return Ok(());
    }

    match format {
        crate::cli::OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = runs
                .iter()
                .map(|run| {
                    serde_json::json!({
                        "id": run.id,
                        "team_name": run.team_name,
                        "goal": run.goal,
                        "status": run.status,
                        "started_at": run.started_at,
                        "ended_at": run.ended_at,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        crate::cli::OutputFormat::Yaml => {
            let entries: Vec<serde_json::Value> = runs
                .iter()
                .map(|run| {
                    serde_json::json!({
                        "id": run.id,
                        "team_name": run.team_name,
                        "goal": run.goal,
                        "status": run.status,
                        "started_at": run.started_at,
                        "ended_at": run.ended_at,
                    })
                })
                .collect();
            print!("{}", serde_yaml::to_string(&entries)?);
        }
        crate::cli::OutputFormat::Text => {
            println!("\n  Run history for team '{name}':");
            for run in &runs {
                let started = format_ts(&run.started_at);
                let ended = run
                    .ended_at
                    .as_ref()
                    .map(|s| format_ts(s))
                    .unwrap_or_else(|| "in progress".to_string());
                println!("    [{}] {} | {} -> {}", run.id, started, run.status, ended);
                println!("      Goal: {}", run.goal);
            }
            println!();
        }
    }

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

fn validate_teams(
    global_home: &std::path::Path,
    name: Option<&str>,
    format: &crate::cli::OutputFormat,
) -> Result<()> {
    use mika_agent::skills::index::DiagnosticLevel;
    use mika_agent::validate::validate_team_config;

    let teams: Vec<String> = match name {
        Some(n) => {
            team::validate_team_name(n)?;
            if !team::team_exists(global_home, n) {
                match format {
                    crate::cli::OutputFormat::Json => println!("[]"),
                    crate::cli::OutputFormat::Yaml => {
                        print!("{}", serde_yaml::to_string(&serde_json::json!([]))?)
                    }
                    crate::cli::OutputFormat::Text => println!("\n  Team '{n}' not found.\n"),
                }
                return Ok(());
            }
            vec![n.to_string()]
        }
        None => {
            let found = team::list_teams(global_home);
            if found.is_empty() {
                match format {
                    crate::cli::OutputFormat::Json => println!("[]"),
                    crate::cli::OutputFormat::Yaml => {
                        print!("{}", serde_yaml::to_string(&serde_json::json!([]))?)
                    }
                    crate::cli::OutputFormat::Text => println!("\n  No teams found.\n"),
                }
                return Ok(());
            }
            found
        }
    };

    let mut all_diags: Vec<serde_json::Value> = Vec::new();
    let mut total_errors = 0;
    let mut total_warnings = 0;

    if matches!(format, crate::cli::OutputFormat::Text) {
        println!();
    }

    for team_name in &teams {
        let diags = validate_team_config(global_home, team_name);
        let has_errors = diags.iter().any(|d| d.level == DiagnosticLevel::Fail);
        let has_warnings = diags.iter().any(|d| d.level == DiagnosticLevel::Warn);

        if has_errors {
            total_errors += 1;
        }
        if has_warnings {
            total_warnings += 1;
        }

        match format {
            crate::cli::OutputFormat::Json | crate::cli::OutputFormat::Yaml => {
                for diag in &diags {
                    all_diags.push(serde_json::json!({
                        "team": team_name,
                        "level": diag.level,
                        "message": diag.message,
                    }));
                }
            }
            crate::cli::OutputFormat::Text => {
                println!("  {team_name}/");
                for diag in &diags {
                    println!("    {} {}", diag.tag(), diag.message);
                }
            }
        }
    }

    match format {
        crate::cli::OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&all_diags)?);
        }
        crate::cli::OutputFormat::Yaml => {
            print!("{}", serde_yaml::to_string(&all_diags)?);
        }
        crate::cli::OutputFormat::Text => {
            println!();
            let total = teams.len();
            let ok_count = total - total_errors;
            if total_errors == 0 && total_warnings == 0 {
                println!("  All {total} team(s) valid.");
            } else {
                println!(
                    "  {ok_count}/{total} valid, {total_errors} with errors, {total_warnings} with warnings."
                );
            }
            println!();
        }
    }

    if total_errors > 0 {
        bail!("team validation failed: {} error(s) found", total_errors);
    }

    Ok(())
}
