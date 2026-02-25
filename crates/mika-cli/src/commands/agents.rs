use anyhow::{Result, bail};
use std::io::{self, Write};

use mika_common::agent::{self, DEFAULT_AGENT};
use mika_common::home;

use crate::cli::{AgentsArgs, AgentsCommand};

pub async fn run(args: AgentsArgs) -> Result<()> {
    let global_home = home::resolve_home_dir()?;

    // Auto-migrate on any agents subcommand
    home::migrate_to_multi_agent(&global_home)?;

    match args.command {
        AgentsCommand::List => list(&global_home),
        AgentsCommand::Create { name } => create(&global_home, &name),
        AgentsCommand::Delete { name, force } => delete(&global_home, &name, force),
        AgentsCommand::Switch { name } => switch(&global_home, &name),
        AgentsCommand::Clone { source, name } => clone(&global_home, &source, &name),
    }
}

fn list(global_home: &std::path::Path) -> Result<()> {
    let agents = agent::list_agents(global_home);
    let active = home::read_active_agent(global_home);

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
    Ok(())
}

fn create(global_home: &std::path::Path, name: &str) -> Result<()> {
    let name = agent::normalize_agent_name(name);
    agent::validate_agent_name(&name)?;

    if agent::agent_exists(global_home, &name) {
        bail!("Agent '{name}' already exists.");
    }

    // Ensure agents/ dir exists (for fresh installs that aren't legacy)
    std::fs::create_dir_all(global_home.join("agents"))?;

    home::bootstrap_agent(global_home, &name)?;

    println!("\n  Created agent '{name}'.");
    println!("  Use `mika --agent {name}` or `mika agents switch {name}` to use it.\n");
    Ok(())
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
    for filename in &["soul.md", "identity.toml", "config.toml", "heartbeat.md", "user.md"] {
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
