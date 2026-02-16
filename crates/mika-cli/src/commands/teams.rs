use anyhow::{Result, bail};
use std::io::{self, Write};

use mika_common::config::Settings;
use mika_common::home;
use mika_common::team;

use crate::cli::{TeamsArgs, TeamsCommand};

pub async fn run(args: TeamsArgs) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;

    match args.command {
        TeamsCommand::List => list(&global_home),
        TeamsCommand::Create { name } => create(&global_home, &name),
        TeamsCommand::Run { name, goal } => run_team_cmd(&global_home, &name, &goal).await,
        TeamsCommand::Status { name } => status(&global_home, &name),
        TeamsCommand::Log { name } => log(&global_home, &name),
        TeamsCommand::Delete { name, force } => delete(&global_home, &name, force),
    }
}

fn list(global_home: &std::path::Path) -> Result<()> {
    let teams = team::list_teams(global_home);

    if teams.is_empty() {
        println!("\n  No teams found. Run `mika teams create <name>` to create one.\n");
        return Ok(());
    }

    println!("\n  Teams:");
    for name in &teams {
        let agent_count = match team::load_team(global_home, name) {
            Ok(def) => def.agents.len(),
            Err(_) => 0,
        };
        println!("    {name} ({agent_count} agents)");
    }
    println!();
    Ok(())
}

fn create(global_home: &std::path::Path, name: &str) -> Result<()> {
    let name = team::normalize_team_name(name);
    team::validate_team_name(&name)?;

    if team::team_exists(global_home, &name) {
        bail!("Team '{name}' already exists.");
    }

    let agents = mika_common::agent::list_agents(global_home);
    if agents.is_empty() {
        bail!("No agents found. Create agents first with `mika agents create <name>`.");
    }

    println!("\n  Creating team '{name}'");
    println!("  Available agents: {}", agents.join(", "));

    // Prompt for orchestrator
    println!();
    print!("  Orchestrator agent: ");
    io::stdout().flush()?;
    let mut orchestrator = String::new();
    io::stdin().read_line(&mut orchestrator)?;
    let orchestrator = orchestrator.trim().to_string();
    if orchestrator.is_empty() {
        bail!("Orchestrator name cannot be empty.");
    }
    if !mika_common::agent::agent_exists(global_home, &orchestrator) {
        bail!("Agent '{orchestrator}' not found.");
    }

    // Collect team members
    let mut team_agents = vec![mika_common::team::TeamAgent {
        name: orchestrator.clone(),
        role: "orchestrator".to_string(),
        mandate: "Decompose goals into tasks and coordinate the team".to_string(),
    }];

    println!("\n  Add team members (empty name to finish):");
    loop {
        print!("  Agent name: ");
        io::stdout().flush()?;
        let mut agent_name = String::new();
        io::stdin().read_line(&mut agent_name)?;
        let agent_name = agent_name.trim().to_string();
        if agent_name.is_empty() {
            break;
        }
        if !mika_common::agent::agent_exists(global_home, &agent_name) {
            println!("    Agent '{agent_name}' not found, skipping.");
            continue;
        }

        print!("  Role (e.g., specialist, qa, writer): ");
        io::stdout().flush()?;
        let mut role = String::new();
        io::stdin().read_line(&mut role)?;
        let role = role.trim().to_string();

        print!("  Mandate (what this agent does): ");
        io::stdout().flush()?;
        let mut mandate = String::new();
        io::stdin().read_line(&mut mandate)?;
        let mandate = mandate.trim().to_string();

        team_agents.push(mika_common::team::TeamAgent {
            name: agent_name,
            role: if role.is_empty() {
                "specialist".to_string()
            } else {
                role
            },
            mandate: if mandate.is_empty() {
                "Complete assigned tasks".to_string()
            } else {
                mandate
            },
        });
    }

    if team_agents.len() < 2 {
        bail!("A team needs at least 2 agents (orchestrator + one member).");
    }

    let def = mika_common::team::TeamDefinition {
        team: mika_common::team::TeamMeta {
            name: name.clone(),
            orchestrator,
        },
        agents: team_agents,
        flow: mika_common::team::TeamFlow { max_iterations: 3 },
    };

    // Write team.toml
    let dir = team::team_dir(global_home, &name);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(team::workspace_dir(global_home, &name))?;
    std::fs::create_dir_all(team::history_dir(global_home, &name))?;

    let toml_content = toml::to_string_pretty(&def)?;
    std::fs::write(dir.join("team.toml"), toml_content)?;

    println!(
        "\n  Created team '{name}' with {} agents.",
        def.agents.len()
    );
    println!("  Run it with: mika teams run {name} \"<goal>\"\n");
    Ok(())
}

async fn run_team_cmd(global_home: &std::path::Path, name: &str, goal: &str) -> Result<()> {
    let name = team::normalize_team_name(name);

    if !team::team_exists(global_home, &name) {
        bail!("Team '{name}' not found.");
    }

    let settings = Settings::load(global_home)?;

    println!("\n  Running team '{name}'...");
    println!("  Goal: {goal}\n");

    let progress = |msg: &str| {
        println!("  > {msg}");
    };

    let run = mika_agent::teams::run_team(
        &name,
        goal,
        global_home,
        &settings,
        Some(Box::new(progress)),
    )
    .await?;

    println!();
    match &run.status {
        mika_agent::teams::types::RunStatus::Completed => {
            println!("  Status: completed");
        }
        mika_agent::teams::types::RunStatus::Failed(msg) => {
            println!("  Status: failed - {msg}");
        }
        mika_agent::teams::types::RunStatus::Running => {
            println!("  Status: running (unexpected)");
        }
    }

    if let Some(deliverable) = &run.deliverable {
        println!("\n  --- Deliverable ---\n");
        println!("{deliverable}");
    }

    println!("\n  Run ID: {}", run.run_id);
    println!(
        "  History: {}\n",
        team::history_dir(global_home, &name).display()
    );

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

    // Show latest run if available
    let history_dir = team::history_dir(global_home, &name);
    if let Ok(Some(latest)) = mika_agent::teams::history::load_latest_run(&history_dir) {
        println!("\n  Latest run:");
        println!("    ID: {}", latest.run_id);
        println!("    Goal: {}", latest.goal);
        println!("    Status: {}", latest.status);
        println!("    Started: {}", latest.started_at);
        if let Some(ended) = &latest.ended_at {
            println!("    Ended: {ended}");
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

    let history_dir = team::history_dir(global_home, &name);
    let runs = mika_agent::teams::history::list_runs(&history_dir)?;

    if runs.is_empty() {
        println!("\n  No runs found for team '{name}'.\n");
        return Ok(());
    }

    println!("\n  Run history for team '{name}':");
    for run in &runs {
        let ended = run.ended_at.as_deref().unwrap_or("in progress");
        println!(
            "    [{}] {} | {} -> {}",
            run.run_id.get(..8).unwrap_or(&run.run_id),
            run.started_at.get(..10).unwrap_or(&run.started_at),
            run.status,
            ended.get(..10).unwrap_or(ended)
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
