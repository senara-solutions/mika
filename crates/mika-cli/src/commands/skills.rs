use anyhow::{Context, Result, bail};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::executor;
use mika_agent::skills::git;
use mika_agent::skills::index::scan_skills_dir;
use mika_agent::skills::install;
use mika_agent::skills::marketplace;
use mika_common::home;
use std::path::Path;

use crate::cli::{SkillsArgs, SkillsCommand};

pub async fn run(args: SkillsArgs, agent_name: &str) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;
    let agent_home = home::resolve_agent_home(&global_home, agent_name);
    let skills_dir = agent_home.join("skills");

    match args.command {
        None | Some(SkillsCommand::List) => {
            let registry = SkillRegistry::from_dir(&skills_dir);
            list_skills(&registry);
        }
        Some(SkillsCommand::Info { name }) => {
            let registry = SkillRegistry::from_dir(&skills_dir);
            show_skill_detail(&registry, &name, &agent_home);
        }
        Some(SkillsCommand::Create { name }) => {
            create_skill(&skills_dir, &name)?;
        }
        Some(SkillsCommand::Test { skill, tool, input }) => {
            test_skill_tool(&skills_dir, &skill, &tool, &input).await?;
        }
        Some(SkillsCommand::Enable { name }) => {
            toggle_skill(&skills_dir, &name, true)?;
        }
        Some(SkillsCommand::Disable { name }) => {
            toggle_skill(&skills_dir, &name, false)?;
        }
        Some(SkillsCommand::Install { source, name }) => {
            install_skill(&agent_home, &skills_dir, &source, name.as_deref())?;
        }
        Some(SkillsCommand::Uninstall { name }) => {
            uninstall_skill(&agent_home, &skills_dir, &name)?;
        }
        Some(SkillsCommand::Update { name }) => {
            update_skills(&agent_home, &skills_dir, name.as_deref())?;
        }
    }
    Ok(())
}

fn list_skills(registry: &SkillRegistry) {
    let skills = registry.skills();
    if skills.is_empty() {
        println!("\n  No skills loaded.\n");
        println!("  Create one with: mika skills create <name>");
        return;
    }
    println!("\n  Skills ({}):", skills.len());
    for entry in skills {
        let tool_count = entry.skill_tools.len();
        let tools_desc = if tool_count > 0 {
            format!("{} tools", tool_count)
        } else {
            "no tools".to_string()
        };
        let always_on = if entry.manifest.skill.always_on {
            " [always on]"
        } else {
            ""
        };
        let status = if entry.enabled { "" } else { " [disabled]" };
        println!(
            "    {} ({}) — {}{}{}",
            entry.manifest.skill.name,
            tools_desc,
            entry.manifest.skill.description,
            always_on,
            status
        );
    }
    println!();
}

fn show_skill_detail(registry: &SkillRegistry, name: &str, agent_home: &Path) {
    let skills = registry.skills();
    let found = skills
        .iter()
        .find(|s| s.manifest.skill.name.eq_ignore_ascii_case(name));
    match found {
        Some(entry) => {
            let m = &entry.manifest;
            let keywords = if m.triggers.keywords.is_empty() {
                "none".to_string()
            } else {
                m.triggers.keywords.join(", ")
            };
            println!();
            println!("  Skill: {}", m.skill.name);
            println!("    Description: {}", m.skill.description);
            println!(
                "    Version:     {}",
                if m.skill.version.is_empty() {
                    "unset"
                } else {
                    &m.skill.version
                }
            );
            println!("    Always on:   {}", m.skill.always_on);
            println!("    Enabled:     {}", entry.enabled);
            println!("    Timeout:     {}s", m.skill.timeout_secs);
            println!("    Keywords:    {keywords}");
            if !entry.skill_tools.is_empty() {
                println!("    Tools:");
                for st in &entry.skill_tools {
                    let handler_type = match &st.handler {
                        mika_agent::skills::manifest::ToolHandler::Exec { command } => {
                            format!("exec: {command}")
                        }
                        mika_agent::skills::manifest::ToolHandler::Http { url, method } => {
                            format!("{method} {url}")
                        }
                        mika_agent::skills::manifest::ToolHandler::Builtin { function } => {
                            format!("builtin: {function}")
                        }
                    };
                    println!("      - {} ({})", st.definition.name, handler_type);
                }
            }
            println!("    Path:        {}", entry.dir.display());

            // Show marketplace metadata if applicable
            let lock = marketplace::read_lock(agent_home);
            if let Some(mp_entry) = lock.skills.get(&m.skill.name) {
                println!("    Source:      {}", mp_entry.url);
                println!("    Repo path:   {}", mp_entry.path);
                println!("    Commit:      {}", mp_entry.commit);
                println!("    Installed:   {}", mp_entry.installed_at);
                println!("    Updated:     {}", mp_entry.updated_at);
            }
            println!();
        }
        None => {
            println!("\n  No skill found with name '{name}'.\n");
        }
    }
}

fn create_skill(skills_dir: &Path, name: &str) -> Result<()> {
    let skill_dir = skills_dir.join(name);
    if skill_dir.exists() {
        bail!("Skill '{name}' already exists at {}", skill_dir.display());
    }

    std::fs::create_dir_all(&skill_dir)
        .with_context(|| format!("failed to create {}", skill_dir.display()))?;

    // Write skill.toml
    let skill_toml = format!(
        r#"[skill]
name = "{name}"
description = "Description of {name} skill"
version = "0.1.0"
always_on = false
timeout_secs = 30

[triggers]
keywords = ["{name}"]
"#
    );
    std::fs::write(skill_dir.join("skill.toml"), &skill_toml)?;

    // Write tools.json
    let tools_json = format!(
        r#"[
  {{
    "name": "{name_under}_action",
    "description": "Perform the {name} action",
    "input_schema": {{
      "type": "object",
      "properties": {{
        "query": {{
          "type": "string",
          "description": "Input query"
        }}
      }},
      "required": ["query"]
    }},
    "handler": {{
      "type": "exec",
      "command": "handlers/run.sh"
    }}
  }}
]
"#,
        name_under = name.replace('-', "_")
    );
    std::fs::write(skill_dir.join("tools.json"), &tools_json)?;

    // Write system_prompt.md
    let prompt =
        format!("Use the {name} tools when the user asks about topics related to {name}.\n");
    std::fs::write(skill_dir.join("system_prompt.md"), &prompt)?;

    // Write handler script
    let handlers_dir = skill_dir.join("handlers");
    std::fs::create_dir_all(&handlers_dir)?;
    let handler = r#"#!/bin/sh
# Skill handler script
# Input: JSON on stdin
# Output: text on stdout

# Read input
INPUT=$(cat)

# Extract query field
QUERY=$(echo "$INPUT" | grep -o '"query":"[^"]*"' | head -1 | cut -d'"' -f4)

echo "TODO: implement handler for query: $QUERY"
"#;
    let handler_path = handlers_dir.join("run.sh");
    std::fs::write(&handler_path, handler)?;

    // Make handler executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&handler_path, std::fs::Permissions::from_mode(0o755))?;
    }

    println!("\n  Created skill '{name}' at {}", skill_dir.display());
    println!("  Files:");
    println!("    skill.toml       — skill manifest");
    println!("    tools.json       — tool definitions");
    println!("    system_prompt.md — prompt snippet");
    println!("    handlers/run.sh  — handler script");
    println!();
    println!(
        "  Test with: mika skills test {name} {}_action",
        name.replace('-', "_")
    );
    println!();
    Ok(())
}

async fn test_skill_tool(
    skills_dir: &Path,
    skill_name: &str,
    tool_name: &str,
    input_json: &str,
) -> Result<()> {
    let entries = scan_skills_dir(skills_dir);
    let entry = entries
        .iter()
        .find(|e| e.manifest.skill.name.eq_ignore_ascii_case(skill_name))
        .ok_or_else(|| anyhow::anyhow!("Skill '{skill_name}' not found"))?;

    let skill_tool = entry
        .skill_tools
        .iter()
        .find(|t| t.definition.name == tool_name)
        .ok_or_else(|| {
            let available: Vec<&str> = entry
                .skill_tools
                .iter()
                .map(|t| t.definition.name.as_str())
                .collect();
            if available.is_empty() {
                anyhow::anyhow!("Skill '{skill_name}' has no tools defined in tools.json")
            } else {
                anyhow::anyhow!(
                    "Tool '{tool_name}' not found in skill '{skill_name}'. Available: {}",
                    available.join(", ")
                )
            }
        })?;

    let input: serde_json::Value = serde_json::from_str(input_json)
        .with_context(|| format!("Invalid JSON input: {input_json}"))?;

    println!("\n  Testing {skill_name}/{tool_name}...\n");

    let output =
        executor::execute_skill_tool(skill_tool, input, entry.manifest.skill.timeout_secs).await;

    if output.is_error {
        println!("  [ERROR] {}", output.content);
    } else {
        println!("  [OK] {}", output.content);
    }
    println!();
    Ok(())
}

fn toggle_skill(skills_dir: &Path, name: &str, enable: bool) -> Result<()> {
    let skill_dir = skills_dir.join(name);
    if !skill_dir.is_dir() {
        bail!("Skill '{name}' not found at {}", skill_dir.display());
    }

    let marker = skill_dir.join(".disabled");
    if enable {
        if marker.exists() {
            std::fs::remove_file(&marker)
                .with_context(|| format!("failed to remove {}", marker.display()))?;
            println!("\n  Enabled skill '{name}'.\n");
        } else {
            println!("\n  Skill '{name}' is already enabled.\n");
        }
    } else if !marker.exists() {
        std::fs::write(&marker, "")
            .with_context(|| format!("failed to create {}", marker.display()))?;
        println!("\n  Disabled skill '{name}'.\n");
    } else {
        println!("\n  Skill '{name}' is already disabled.\n");
    }
    Ok(())
}

fn install_skill(
    agent_home: &Path,
    skills_dir: &Path,
    source: &str,
    alias: Option<&str>,
) -> Result<()> {
    // Resolve URL
    let url = git::resolve_url(source)?;
    println!("\n  Installing from {url}...");

    // Check git is available
    git::check_git()?;

    // Clone to temp dir
    let tmp = git::clone_to_temp(&url)?;

    // Get commit hash
    let commit = git::get_head_commit(tmp.path())?;

    // Scan for skills
    let candidates = marketplace::scan_repo_for_skills(tmp.path());
    if candidates.is_empty() {
        bail!("No valid skills found in repository. Ensure the repo contains a skill.toml file.");
    }

    // Ensure skills dir exists
    std::fs::create_dir_all(skills_dir)
        .with_context(|| format!("failed to create {}", skills_dir.display()))?;

    // Select skill(s) to install
    let selected = if candidates.len() == 1 {
        vec![&candidates[0]]
    } else {
        // --name can only be used with a single skill
        if alias.is_some() {
            bail!(
                "Multiple skills found in repo. --name can only be used when installing a single skill.\n\
                 Found skills: {}",
                candidates
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        select_skills_interactive(&candidates)?
    };

    if selected.is_empty() {
        println!("  No skills selected.\n");
        return Ok(());
    }

    for candidate in &selected {
        let result = install::install_skill(
            agent_home,
            skills_dir,
            candidate,
            if selected.len() == 1 { alias } else { None },
            &url,
            &commit,
        )?;

        println!(
            "  Installed '{}' (commit: {})",
            result.name,
            &result.commit[..12.min(result.commit.len())]
        );

        if result.has_exec_handlers {
            println!("\n  WARNING: This skill contains exec handlers that run shell commands.");
            println!("  Review the source before use: {}\n", result.url);
        }
    }

    println!();
    Ok(())
}

/// Interactive multi-skill selection using dialoguer.
fn select_skills_interactive(
    candidates: &[marketplace::SkillCandidate],
) -> Result<Vec<&marketplace::SkillCandidate>> {
    // Check if we're in a TTY
    if !atty_check() {
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        bail!(
            "Multiple skills found but not running in a terminal.\n\
             Found: {}\n\
             Re-run with a specific skill name or use --name.",
            names.join(", ")
        );
    }

    let items: Vec<String> = candidates
        .iter()
        .map(|c| format!("{} — {} (at: {})", c.name, c.description, c.relative_path))
        .collect();

    println!("\n  Multiple skills found in repository:");

    let selections = dialoguer::MultiSelect::new()
        .with_prompt("  Select skills to install")
        .items(&items)
        .interact()?;

    Ok(selections.iter().map(|&i| &candidates[i]).collect())
}

/// Simple TTY check.
fn atty_check() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stdin())
}

fn uninstall_skill(agent_home: &Path, skills_dir: &Path, name: &str) -> Result<()> {
    install::uninstall_skill(agent_home, skills_dir, name)?;
    println!("\n  Uninstalled '{name}'.\n");
    Ok(())
}

fn update_skills(agent_home: &Path, skills_dir: &Path, name: Option<&str>) -> Result<()> {
    let lock = marketplace::read_lock(agent_home);

    if lock.skills.is_empty() {
        println!("\n  No marketplace skills installed.\n");
        return Ok(());
    }

    match name {
        Some(name) => {
            // Update single skill
            if !lock.skills.contains_key(name) {
                bail!("Skill '{name}' is not a marketplace-installed skill.");
            }
            println!("\n  Updating '{name}'...");
            match install::update_skill(agent_home, skills_dir, name)? {
                Some(result) => {
                    println!(
                        "  {}: updated ({} -> {})",
                        result.name,
                        &result.old_commit[..12.min(result.old_commit.len())],
                        &result.new_commit[..12.min(result.new_commit.len())]
                    );
                }
                None => {
                    println!("  {name}: already up to date.");
                }
            }
        }
        None => {
            // Update all marketplace skills
            let names: Vec<String> = lock.skills.keys().cloned().collect();
            println!("\n  Updating {} marketplace skill(s)...", names.len());

            let mut updated = 0;
            let mut up_to_date = 0;
            let mut failed: Vec<(String, String)> = Vec::new();

            for skill_name in &names {
                match install::update_skill(agent_home, skills_dir, skill_name) {
                    Ok(Some(result)) => {
                        println!(
                            "  {}: updated ({} -> {})",
                            result.name,
                            &result.old_commit[..12.min(result.old_commit.len())],
                            &result.new_commit[..12.min(result.new_commit.len())]
                        );
                        updated += 1;
                    }
                    Ok(None) => {
                        up_to_date += 1;
                    }
                    Err(e) => {
                        println!("  {skill_name}: failed ({e})");
                        failed.push((skill_name.clone(), e.to_string()));
                    }
                }
            }

            println!();
            if updated > 0 {
                println!("  Updated {updated} skill(s).");
            }
            if up_to_date > 0 {
                println!("  {up_to_date} skill(s) already up to date.");
            }
            if !failed.is_empty() {
                println!("  {} skill(s) failed to update.", failed.len());
            }
        }
    }

    println!();
    Ok(())
}
