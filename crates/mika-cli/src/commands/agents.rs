use anyhow::{Result, bail};
use std::io::{self, IsTerminal, Write};

use mika_common::agent::{self, DEFAULT_AGENT};
use mika_common::home;

use crate::cli::{AgentsArgs, AgentsCommand, OutputFormat};
use crate::wizard;

pub async fn run(args: AgentsArgs) -> Result<()> {
    let global_home = home::resolve_home_dir()?;

    // Auto-migrate on any agents subcommand
    home::migrate_to_multi_agent(&global_home)?;

    match args.command {
        AgentsCommand::List { format } => list(&global_home, &format),
        AgentsCommand::Create {
            name,
            no_interactive,
        } => create(&global_home, &name, no_interactive).await,
        AgentsCommand::Delete { name, force } => delete(&global_home, &name, force),
        AgentsCommand::Switch { name } => switch(&global_home, &name),
        AgentsCommand::Clone { source, name } => clone(&global_home, &source, &name),
        AgentsCommand::Validate { name, format } => {
            validate_agents(&global_home, name.as_deref(), &format)
        }
    }
}

fn list(global_home: &std::path::Path, format: &OutputFormat) -> Result<()> {
    let agents = agent::list_agents(global_home);
    let active = home::read_active_agent(global_home);

    match format {
        OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = agents
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "name": name,
                        "active": *name == active,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        OutputFormat::Text => {
            if agents.is_empty() {
                println!("\n  No agents found. Run `mika agents create <name>` to create one.\n");
                return Ok(());
            }

            println!("\n  Agents:");
            for name in &agents {
                let marker = if *name == active { " (active)" } else { "" };
                println!("    {name}{marker}");
            }
            println!();
        }
    }
    Ok(())
}

async fn create(global_home: &std::path::Path, name: &str, no_interactive: bool) -> Result<()> {
    let name = agent::normalize_agent_name(name);
    agent::validate_agent_name(&name)?;

    if agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' already exists.");
    }

    // Ensure agents/ dir exists (for fresh installs that aren't legacy)
    std::fs::create_dir_all(global_home.join("agents"))?;

    home::bootstrap_agent(global_home, &name)?;

    let interactive = !no_interactive && std::io::stdin().is_terminal();

    if interactive {
        let result = wizard::run_agent_wizard(&name)?;
        let agent_home = mika_common::agent::agent_dir(global_home, &name);

        // Overwrite identity.toml with wizard answers
        let identity = format!(
            "name = \"{}\"\nemoji = \"{}\"\n",
            result.display_name, result.emoji
        );
        std::fs::write(agent_home.join("identity.toml"), identity)?;

        // Generate or template soul.md if specialization was provided
        if !result.specialization.is_empty() {
            let soul = match try_generate_soul(global_home, &name, &result).await {
                Some(generated) => generated,
                None => wizard::template_soul_md(
                    &result.display_name,
                    &result.specialization,
                    &result.communication_style,
                ),
            };
            std::fs::write(agent_home.join("soul.md"), soul)?;
        }
    }

    // Always seed on explicit creation, regardless of disable_bundled_skills config
    let agent_home = mika_common::agent::agent_dir(global_home, &name);
    mika_agent::startup::seed_bundled_skills_if_needed(&agent_home, false);

    println!("\n  Created agent '{name}'.");
    println!("  Use `mika --agent {name}` or `mika agents switch {name}` to use it.\n");
    Ok(())
}

/// Try to generate soul.md via LLM. Returns None on any failure.
async fn try_generate_soul(
    global_home: &std::path::Path,
    name: &str,
    result: &wizard::AgentWizardResult,
) -> Option<String> {
    let settings = mika_common::config::Settings::load(global_home).ok()?;
    let provider = settings.make_llm_provider().ok()?;

    println!("  Generating personality...");
    match wizard::generate_soul_md(
        provider.as_ref(),
        name,
        &result.specialization,
        &result.communication_style,
    )
    .await
    {
        Some(soul) => Some(soul),
        None => {
            println!("  Could not generate personality, using template.");
            None
        }
    }
}

fn delete(global_home: &std::path::Path, name: &str, force: bool) -> Result<()> {
    let name = agent::normalize_agent_name(name);

    if name == DEFAULT_AGENT {
        bail!("Cannot delete the default agent '{DEFAULT_AGENT}'.");
    }

    if !agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' not found.");
    }

    if !force {
        print!("  Delete agent '{name}' and all its data? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  Cancelled.");
            return Ok(());
        }
    }

    let dir = agent::agent_dir(global_home, &name);
    std::fs::remove_dir_all(&dir)?;

    // If deleted agent was active, switch back to main
    let active = home::read_active_agent(global_home);
    if active == name {
        home::write_active_agent(global_home, DEFAULT_AGENT)?;
        println!("  Switched active agent to '{DEFAULT_AGENT}'.");
    }

    println!("  Deleted agent '{name}'.");
    Ok(())
}

fn switch(global_home: &std::path::Path, name: &str) -> Result<()> {
    let name = agent::normalize_agent_name(name);
    agent::validate_agent_name(&name)?;

    if !agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' not found. Create it with `mika agents create {name}`.");
    }

    home::write_active_agent(global_home, &name)?;
    println!("\n  Switched to agent '{name}'.\n");
    Ok(())
}

fn clone(global_home: &std::path::Path, source: &str, target: &str) -> Result<()> {
    let source = agent::normalize_agent_name(source);
    let target = agent::normalize_agent_name(target);
    agent::validate_agent_name(&target)?;

    if !agent::agent_exists(global_home, &source) {
        bail!("Source agent '{source}' not found.");
    }
    if agent::agent_exists(global_home, &target) {
        bail!("Agent '{target}' already exists.");
    }

    // Bootstrap the new agent with defaults first
    home::bootstrap_agent(global_home, &target)?;

    let src_dir = agent::agent_dir(global_home, &source);
    let dst_dir = agent::agent_dir(global_home, &target);

    // Copy personality files (overwrite the defaults)
    for filename in &[
        "soul.md",
        "identity.toml",
        "config.toml",
        "heartbeat.md",
        "user.md",
    ] {
        let src = src_dir.join(filename);
        if src.is_file() {
            std::fs::copy(&src, dst_dir.join(filename))?;
        }
    }

    // Copy skills directory
    let src_skills = src_dir.join("skills");
    if src_skills.is_dir() {
        copy_dir_recursive(&src_skills, &dst_dir.join("skills"), 0)?;
    }

    println!("\n  Cloned '{source}' personality into new agent '{target}'.");
    println!("  The new agent starts with a fresh database (no conversation history).\n");
    Ok(())
}

fn validate_agents(
    global_home: &std::path::Path,
    name: Option<&str>,
    format: &OutputFormat,
) -> Result<()> {
    use mika_agent::skills::index::DiagnosticLevel;
    use mika_agent::validate::validate_agent;

    let agents: Vec<String> = match name {
        Some(n) => {
            agent::validate_agent_name(n)?;
            if !agent::agent_exists(global_home, n) {
                match format {
                    OutputFormat::Json => println!("[]"),
                    OutputFormat::Text => println!("\n  Agent '{n}' not found.\n"),
                }
                return Ok(());
            }
            vec![n.to_string()]
        }
        None => {
            let found = agent::list_agents(global_home);
            if found.is_empty() {
                match format {
                    OutputFormat::Json => println!("[]"),
                    OutputFormat::Text => println!("\n  No agents found.\n"),
                }
                return Ok(());
            }
            found
        }
    };

    let mut all_diags: Vec<serde_json::Value> = Vec::new();
    let mut total_errors = 0;
    let mut total_warnings = 0;

    if matches!(format, OutputFormat::Text) {
        println!();
    }

    for agent_name in &agents {
        let diags = validate_agent(global_home, agent_name);
        let has_errors = diags.iter().any(|d| d.level == DiagnosticLevel::Fail);
        let has_warnings = diags.iter().any(|d| d.level == DiagnosticLevel::Warn);

        if has_errors {
            total_errors += 1;
        }
        if has_warnings {
            total_warnings += 1;
        }

        match format {
            OutputFormat::Json => {
                for diag in &diags {
                    all_diags.push(serde_json::json!({
                        "agent": agent_name,
                        "level": diag.level,
                        "message": diag.message,
                    }));
                }
            }
            OutputFormat::Text => {
                println!("  {agent_name}/");
                for diag in &diags {
                    println!("    {} {}", diag.tag(), diag.message);
                }
            }
        }
    }

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&all_diags)?);
        }
        OutputFormat::Text => {
            println!();
            let total = agents.len();
            let ok_count = total - total_errors;
            if total_errors == 0 && total_warnings == 0 {
                println!("  All {total} agent(s) valid.");
            } else {
                println!(
                    "  {ok_count}/{total} valid, {total_errors} with errors, {total_warnings} with warnings."
                );
            }
            println!();
        }
    }

    if total_errors > 0 {
        bail!("agent validation failed: {} error(s) found", total_errors);
    }

    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path, depth: u32) -> Result<()> {
    if depth > 10 {
        bail!("directory nesting too deep while copying {}", src.display());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path, depth + 1)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}
