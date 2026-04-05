use anyhow::{Context, Result, bail};
use mika_agent::bundled_skills;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::executor;
use mika_agent::skills::git;
use mika_agent::skills::index::{DiagnosticLevel, scan_skills_dir, validate_skill};
use mika_agent::skills::install;
use mika_agent::skills::marketplace;
use mika_agent::tools::create_skill::validate_skill_name;
use mika_common::home;
use std::io::IsTerminal;
use std::path::Path;

use crate::cli::{SkillsArgs, SkillsCommand};

pub async fn run(args: SkillsArgs, agent_name: &str) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;
    let agent_home = home::resolve_agent_home(&global_home, agent_name);
    let skills_dir = agent_home.join("skills");

    match args.command {
        None => {
            let registry = SkillRegistry::from_dir(&skills_dir);
            list_skills(&registry, &agent_home, &crate::cli::OutputFormat::Text)?;
        }
        Some(SkillsCommand::List { format }) => {
            let registry = SkillRegistry::from_dir(&skills_dir);
            list_skills(&registry, &agent_home, &format)?;
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
        Some(SkillsCommand::Install { source, name, link }) => {
            install_skill(&agent_home, &skills_dir, &source, name.as_deref(), link)?;
        }
        Some(SkillsCommand::Uninstall { name, remove_deps }) => {
            uninstall_skill(&agent_home, &skills_dir, &name, remove_deps)?;
        }
        Some(SkillsCommand::Update { name }) => {
            update_skills(&agent_home, &skills_dir, name.as_deref())?;
        }
        Some(SkillsCommand::Validate { name, format }) => {
            validate_skills(&skills_dir, name.as_deref(), &format)?;
        }
    }
    Ok(())
}

fn list_skills(
    registry: &SkillRegistry,
    agent_home: &Path,
    format: &crate::cli::OutputFormat,
) -> Result<()> {
    let skills = registry.skills();
    if skills.is_empty() {
        match format {
            crate::cli::OutputFormat::Json => println!("[]"),
            crate::cli::OutputFormat::Text => {
                println!("\n  No skills loaded.\n");
                println!("  Create one with: mika skills create <name>");
            }
        }
        return Ok(());
    }
    let lock = marketplace::read_lock(agent_home);

    match format {
        crate::cli::OutputFormat::Json => {
            let entries: Vec<serde_json::Value> = skills
                .iter()
                .map(|entry| {
                    let name = &entry.manifest.skill.name;
                    let origin = if bundled_skills::is_bundled_skill(name) {
                        "built-in"
                    } else if let Some(mp_entry) = lock.skills.get(name.as_str()) {
                        if mp_entry.linked {
                            "marketplace/linked"
                        } else {
                            "marketplace"
                        }
                    } else {
                        "custom"
                    };
                    let llm_override = if !entry.manifest.llm.is_empty() {
                        let provider = entry.manifest.llm.provider.as_deref().unwrap_or("*");
                        let model = entry.manifest.llm.model.as_deref().unwrap_or("default");
                        Some(format!("{provider}/{model}"))
                    } else {
                        None
                    };
                    serde_json::json!({
                        "name": name,
                        "description": entry.manifest.skill.description,
                        "origin": origin,
                        "enabled": entry.enabled,
                        "always_on": entry.manifest.skill.always_on,
                        "tools": entry.skill_tools.len(),
                        "variants": entry.variant_count(),
                        "llm_override": llm_override,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&entries)?);
        }
        crate::cli::OutputFormat::Text => {
            println!("\n  Skills ({}):", skills.len());
            for entry in skills {
                let name = &entry.manifest.skill.name;
                let tool_count = entry.skill_tools.len();
                let tools_desc = if tool_count > 0 {
                    format!("{} tools", tool_count)
                } else {
                    "no tools".to_string()
                };
                let origin = if bundled_skills::is_bundled_skill(name) {
                    " [built-in]"
                } else if let Some(mp_entry) = lock.skills.get(name.as_str()) {
                    if mp_entry.linked {
                        " [marketplace/linked]"
                    } else {
                        " [marketplace]"
                    }
                } else {
                    " [custom]"
                };
                let always_on = if entry.manifest.skill.always_on {
                    " [always on]"
                } else {
                    ""
                };
                let status = if entry.enabled { "" } else { " [disabled]" };
                let variants = {
                    let count = entry.variant_count();
                    if count > 0 {
                        format!(" [variants: {count}]")
                    } else {
                        String::new()
                    }
                };
                let llm_badge = if !entry.manifest.llm.is_empty() {
                    let provider = entry.manifest.llm.provider.as_deref().unwrap_or("*");
                    let model = entry.manifest.llm.model.as_deref().unwrap_or("default");
                    format!(" [llm: {provider}/{model}]")
                } else {
                    String::new()
                };
                println!(
                    "    {} ({}) — {}{}{}{}{}{}",
                    name,
                    tools_desc,
                    entry.manifest.skill.description,
                    origin,
                    always_on,
                    status,
                    variants,
                    llm_badge
                );
            }
            println!();
        }
    }
    Ok(())
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
                        mika_agent::skills::manifest::ToolHandler::Exec { command, .. } => {
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

            // Show LLM override if configured
            if !entry.manifest.llm.is_empty() {
                let provider = entry
                    .manifest
                    .llm
                    .provider
                    .as_deref()
                    .unwrap_or("(agent default)");
                let model = entry
                    .manifest
                    .llm
                    .model
                    .as_deref()
                    .unwrap_or("(provider default)");
                println!("    LLM Override:");
                println!("      Provider:  {}", provider);
                println!("      Model:     {}", model);
            }

            // Show provider and model variants
            let providers = entry.variant_providers();
            if !providers.is_empty() || entry.variant_count() > 0 {
                println!("    Variants:    {} total", entry.variant_count());
                for provider in &providers {
                    let has_override = entry.provider_overrides.contains_key(*provider);
                    if has_override {
                        println!("      - {} (overrides)", provider);
                    } else {
                        println!("      - {}", provider);
                    }

                    // Show model variants under this provider
                    let models = entry.variant_models(provider);
                    for model in &models {
                        let model_key = format!("{provider}/{model}");
                        let has_model_prompt = entry.model_prompts.contains_key(&model_key);
                        let has_model_override = entry.model_overrides.contains_key(&model_key);
                        let model_parts: Vec<&str> = [
                            if has_model_prompt {
                                Some("prompt")
                            } else {
                                None
                            },
                            if has_model_override {
                                Some("overrides")
                            } else {
                                None
                            },
                        ]
                        .into_iter()
                        .flatten()
                        .collect();
                        println!("        └─ {} ({})", model, model_parts.join(", "));
                    }
                }
            }

            // Show marketplace metadata if applicable
            let lock = marketplace::read_lock(agent_home);
            if let Some(mp_entry) = lock.skills.get(&m.skill.name) {
                if mp_entry.linked {
                    println!("    Source:      {} (linked)", mp_entry.url);
                } else if marketplace::is_local_source(mp_entry) {
                    println!("    Source:      {} (local copy)", mp_entry.url);
                } else {
                    println!("    Source:      {}", mp_entry.url);
                    println!("    Repo path:   {}", mp_entry.path);
                    println!("    Commit:      {}", mp_entry.commit);
                }
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

command -v jq >/dev/null 2>&1 || { echo "Error: jq is required" >&2; exit 1; }

INPUT=$(cat)
QUERY=$(printf '%s\n' "$INPUT" | jq -r '.query // empty')

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
    let scan = scan_skills_dir(skills_dir);
    let entry = scan
        .entries
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
        executor::execute_skill_tool(skill_tool, input, entry.manifest.skill.timeout_secs, None)
            .await;

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
    link: bool,
) -> Result<()> {
    use mika_agent::skills::git::SourceKind;

    // Resolve source (local path or git URL)
    let source_kind = git::resolve_source(source)?;

    // Reject --link with git sources
    if link && let SourceKind::Git(url) = &source_kind {
        bail!("--link requires a local path source (got: {url})");
    }

    // Ensure skills dir exists
    std::fs::create_dir_all(skills_dir)
        .with_context(|| format!("failed to create {}", skills_dir.display()))?;

    match source_kind {
        SourceKind::Git(url) => {
            install_from_git(agent_home, skills_dir, &url, alias)?;
        }
        SourceKind::Local(path) => {
            install_from_local(agent_home, skills_dir, &path, alias, link)?;
        }
    }

    println!();
    Ok(())
}

fn install_from_git(
    agent_home: &Path,
    skills_dir: &Path,
    url: &str,
    alias: Option<&str>,
) -> Result<()> {
    println!("\n  Installing from {url}...");

    git::check_git()?;
    let tmp = git::clone_to_temp(url)?;
    let commit = git::get_head_commit(tmp.path())?;

    let candidates = marketplace::scan_repo_for_skills(tmp.path());
    if candidates.is_empty() {
        bail!("No valid skills found in repository. Ensure the repo contains a skill.toml file.");
    }

    let selected = if candidates.len() == 1 {
        vec![&candidates[0]]
    } else {
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
        println!("  No skills selected.");
        return Ok(());
    }

    // Resolve dependencies
    let deps = install::resolve_dependencies(&selected, &candidates, skills_dir)?;

    // Install dependencies first (in BFS order — leaves before roots)
    for dep in &deps {
        let dep_candidate = &dep.candidate;

        if dep_candidate.has_exec_handlers {
            println!(
                "\n  WARNING: Dependency '{}' contains exec handlers that run shell commands.",
                dep_candidate.name
            );
            println!("  Review the source before use: {url}");
            if std::io::stdin().is_terminal() {
                let confirm = dialoguer::Confirm::new()
                    .with_prompt("  Continue with installation?")
                    .default(false)
                    .interact()?;
                if !confirm {
                    bail!(
                        "Installation cancelled: dependency '{}' requires exec handlers.",
                        dep_candidate.name
                    );
                }
            }
        }

        let result =
            install::install_skill(agent_home, skills_dir, dep_candidate, None, url, &commit)?;

        // Mark as dependency in the lock file
        install::mark_as_dependency(agent_home, &result.name, &dep.required_by)?;

        println!(
            "  Installing dependency: {} (required by {})",
            result.name, dep.required_by
        );
    }

    // Install primary skills
    for candidate in &selected {
        if candidate.has_exec_handlers {
            println!("\n  WARNING: This skill contains exec handlers that run shell commands.");
            println!("  Review the source before use: {url}");
            if std::io::stdin().is_terminal() {
                let confirm = dialoguer::Confirm::new()
                    .with_prompt("  Continue with installation?")
                    .default(false)
                    .interact()?;
                if !confirm {
                    println!("  Installation cancelled.");
                    continue;
                }
            }
        }

        let result = install::install_skill(
            agent_home,
            skills_dir,
            candidate,
            if selected.len() == 1 { alias } else { None },
            url,
            &commit,
        )?;

        println!(
            "  Installed '{}' (commit: {})",
            result.name,
            &result.commit[..12.min(result.commit.len())]
        );
    }

    Ok(())
}

fn install_from_local(
    agent_home: &Path,
    skills_dir: &Path,
    path: &Path,
    alias: Option<&str>,
    link: bool,
) -> Result<()> {
    let mode = if link { "linking" } else { "copying" };
    println!("\n  Installing from {} ({mode})...", path.display());

    let candidates = marketplace::scan_repo_for_skills(path);
    if candidates.is_empty() {
        bail!(
            "No valid skills found at {}. Ensure the directory contains a skill.toml file.",
            path.display()
        );
    }

    let selected = if candidates.len() == 1 {
        vec![&candidates[0]]
    } else {
        if alias.is_some() {
            bail!(
                "Multiple skills found. --name can only be used when installing a single skill.\n\
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
        println!("  No skills selected.");
        return Ok(());
    }

    let file_url = format!("file://{}", path.display());

    // Build the full pool of available candidates for dependency resolution.
    // If the source path has a parent directory, scan it too so that sibling
    // skills (e.g. mika-skills/build-mika alongside mika-skills/self-dev) are
    // discoverable as dependencies.
    let mut available_candidates = candidates.clone();
    if let Some(parent) = path.parent().filter(|p| *p != path) {
        let siblings = marketplace::scan_repo_for_skills(parent);
        for sibling in siblings {
            let already_present = available_candidates
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&sibling.name));
            if !already_present {
                available_candidates.push(sibling);
            }
        }
    }

    // Resolve dependencies
    let deps = install::resolve_dependencies(&selected, &available_candidates, skills_dir)?;

    // Install dependencies first (in BFS order — leaves before roots)
    for dep in &deps {
        let dep_candidate = &dep.candidate;

        if dep_candidate.has_exec_handlers {
            println!(
                "\n  WARNING: Dependency '{}' contains exec handlers that run shell commands.",
                dep_candidate.name
            );
            println!(
                "  Review the source at: {}",
                dep_candidate.absolute_path.display()
            );
            if link {
                println!("  NOTE: With --link, handler scripts can be modified at any time.");
            }
            if std::io::stdin().is_terminal() {
                let confirm = dialoguer::Confirm::new()
                    .with_prompt("  Continue with installation?")
                    .default(false)
                    .interact()?;
                if !confirm {
                    bail!(
                        "Installation cancelled: dependency '{}' requires exec handlers.",
                        dep_candidate.name
                    );
                }
            }
        }

        // --link propagates to deps from the same source directory
        let dep_file_url = format!(
            "file://{}",
            dep_candidate
                .absolute_path
                .parent()
                .unwrap_or(path)
                .display()
        );
        let result = if link {
            install::install_skill_linked(
                agent_home,
                skills_dir,
                dep_candidate,
                None,
                &dep_file_url,
            )?
        } else {
            install::install_skill(
                agent_home,
                skills_dir,
                dep_candidate,
                None,
                &dep_file_url,
                "",
            )?
        };

        // Mark as dependency in the lock file
        install::mark_as_dependency(agent_home, &result.name, &dep.required_by)?;

        if result.linked {
            println!(
                "  Installing dependency: {} (required by {}, linked)",
                result.name, dep.required_by
            );
        } else {
            println!(
                "  Installing dependency: {} (required by {})",
                result.name, dep.required_by
            );
        }
    }

    // Install primary skills
    for candidate in &selected {
        if candidate.has_exec_handlers {
            println!("\n  WARNING: This skill contains exec handlers that run shell commands.");
            println!(
                "  Review the source at: {}",
                candidate.absolute_path.display()
            );
            if link {
                println!("  NOTE: With --link, handler scripts can be modified at any time.");
            }
            if std::io::stdin().is_terminal() {
                let confirm = dialoguer::Confirm::new()
                    .with_prompt("  Continue with installation?")
                    .default(false)
                    .interact()?;
                if !confirm {
                    println!("  Installation cancelled.");
                    continue;
                }
            }
        }

        let install_alias = if selected.len() == 1 { alias } else { None };

        let result = if link {
            install::install_skill_linked(
                agent_home,
                skills_dir,
                candidate,
                install_alias,
                &file_url,
            )?
        } else {
            install::install_skill(
                agent_home,
                skills_dir,
                candidate,
                install_alias,
                &file_url,
                "",
            )?
        };

        if result.linked {
            println!("  Installed '{}' (linked)", result.name);
        } else {
            println!("  Installed '{}' (local copy)", result.name);
        }
    }

    Ok(())
}

/// Interactive multi-skill selection using dialoguer.
fn select_skills_interactive(
    candidates: &[marketplace::SkillCandidate],
) -> Result<Vec<&marketplace::SkillCandidate>> {
    // Check if we're in a TTY
    if !std::io::stdin().is_terminal() {
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

fn uninstall_skill(
    agent_home: &Path,
    skills_dir: &Path,
    name: &str,
    remove_deps: bool,
) -> Result<()> {
    // Find orphans before uninstalling (need lock state before removal)
    let orphans = install::find_orphaned_deps(agent_home, name);

    // Uninstall the primary skill
    install::uninstall_skill(agent_home, skills_dir, name)?;
    println!("\n  Uninstalled '{name}'.");

    // Clean up dependency tracking
    install::remove_from_dependency_lists(agent_home, name)?;

    // Handle orphaned dependencies
    if !orphans.is_empty() {
        let is_tty = std::io::stdin().is_terminal();

        let mut to_remove: Vec<String> = Vec::new();

        for orphan in &orphans {
            if remove_deps {
                to_remove.push(orphan.clone());
            } else if is_tty {
                let confirm = dialoguer::Confirm::new()
                    .with_prompt(format!(
                        "  Dependency \"{orphan}\" is no longer needed. Remove?"
                    ))
                    .default(false)
                    .interact()?;
                if confirm {
                    to_remove.push(orphan.clone());
                }
            } else {
                println!(
                    "  Orphaned dependency \"{orphan}\" kept (non-interactive mode). Use --remove-deps to remove."
                );
            }
        }

        // Cascading removal: removing an orphan may orphan its own deps
        while let Some(orphan_name) = to_remove.pop() {
            let sub_orphans = install::find_orphaned_deps(agent_home, &orphan_name);
            if let Err(e) = install::uninstall_skill(agent_home, skills_dir, &orphan_name) {
                println!("  Warning: failed to remove orphaned dependency \"{orphan_name}\": {e}");
                continue;
            }
            install::remove_from_dependency_lists(agent_home, &orphan_name)?;
            println!("  Removed orphaned dependency: {orphan_name}");

            // Enqueue any newly-orphaned transitive deps
            for sub in sub_orphans {
                // Check it's still actually orphaned after the lock update
                let current_orphans = install::find_orphaned_deps(agent_home, &orphan_name);
                if current_orphans.contains(&sub) {
                    to_remove.push(sub);
                }
            }
        }
    }

    println!();
    Ok(())
}

fn update_skills(agent_home: &Path, skills_dir: &Path, name: Option<&str>) -> Result<()> {
    use mika_agent::skills::install::UpdateResult;

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
                UpdateResult::Updated {
                    name,
                    old_commit,
                    new_commit,
                } => {
                    if old_commit.is_empty() {
                        // Local source — no commit hashes
                        println!("  {name}: updated (re-copied from source).");
                    } else if old_commit == new_commit {
                        println!("  {name}: already up to date.");
                    } else {
                        println!(
                            "  {}: updated ({} -> {})",
                            name,
                            &old_commit[..12.min(old_commit.len())],
                            &new_commit[..12.min(new_commit.len())]
                        );
                    }
                }
                UpdateResult::LinkedNoOp { name } => {
                    println!("  {name}: linked — source changes are always current.");
                }
            }
        }
        None => {
            // Update all marketplace skills
            let names: Vec<String> = lock.skills.keys().cloned().collect();
            println!("\n  Updating {} marketplace skill(s)...", names.len());

            let mut updated = 0;
            let mut up_to_date = 0;
            let mut linked_skipped = 0;
            let mut failed: Vec<(String, String)> = Vec::new();

            for skill_name in &names {
                match install::update_skill(agent_home, skills_dir, skill_name) {
                    Ok(UpdateResult::Updated {
                        ref name,
                        ref old_commit,
                        ref new_commit,
                    }) => {
                        if old_commit.is_empty() {
                            println!("  {name}: updated (re-copied from source).");
                            updated += 1;
                        } else if old_commit == new_commit {
                            up_to_date += 1;
                        } else {
                            println!(
                                "  {}: updated ({} -> {})",
                                name,
                                &old_commit[..12.min(old_commit.len())],
                                &new_commit[..12.min(new_commit.len())]
                            );
                            updated += 1;
                        }
                    }
                    Ok(UpdateResult::LinkedNoOp { .. }) => {
                        linked_skipped += 1;
                    }
                    Err(e) => {
                        println!("  {skill_name}: failed ({e})");
                        failed.push((skill_name.clone(), e.to_string()));
                    }
                }
            }

            println!();
            let total = names.len();
            println!(
                "  Updated {updated}/{total} skills. Linked (no-op): {linked_skipped}. Up to date: {up_to_date}. Failed: {}.",
                failed.len()
            );
        }
    }

    println!();
    Ok(())
}

fn validate_skills(
    skills_dir: &Path,
    name: Option<&str>,
    format: &crate::cli::OutputFormat,
) -> Result<()> {
    if !skills_dir.is_dir() {
        match format {
            crate::cli::OutputFormat::Json => println!("[]"),
            crate::cli::OutputFormat::Text => {
                println!(
                    "\n  No skills directory found at {}\n",
                    skills_dir.display()
                );
            }
        }
        return Ok(());
    }

    let dirs: Vec<_> = match name {
        Some(n) => {
            if let Err(e) = validate_skill_name(n) {
                match format {
                    crate::cli::OutputFormat::Json => println!("[]"),
                    crate::cli::OutputFormat::Text => println!("\n  Invalid skill name: {e}\n"),
                }
                return Ok(());
            }
            let skill_dir = skills_dir.join(n);
            if !skill_dir.is_dir() {
                match format {
                    crate::cli::OutputFormat::Json => println!("[]"),
                    crate::cli::OutputFormat::Text => {
                        println!("\n  Skill '{n}' not found at {}\n", skill_dir.display());
                    }
                }
                return Ok(());
            }
            vec![(n.to_string(), skill_dir)]
        }
        None => {
            let mut found = Vec::new();
            if let Ok(rd) = std::fs::read_dir(skills_dir) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    if path.is_dir()
                        && let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
                    {
                        found.push((dir_name.to_string(), path));
                    }
                }
            }
            found.sort_by(|a, b| a.0.cmp(&b.0));
            found
        }
    };

    if dirs.is_empty() {
        match format {
            crate::cli::OutputFormat::Json => println!("[]"),
            crate::cli::OutputFormat::Text => println!("\n  No skills found to validate.\n"),
        }
        return Ok(());
    }

    let mut all_diags: Vec<serde_json::Value> = Vec::new();
    let mut total_errors = 0;
    let mut total_warnings = 0;

    if matches!(format, crate::cli::OutputFormat::Text) {
        println!();
    }

    for (dir_name, dir_path) in &dirs {
        let diags = validate_skill(dir_path);
        let has_errors = diags.iter().any(|d| d.level == DiagnosticLevel::Fail);
        let has_warnings = diags.iter().any(|d| d.level == DiagnosticLevel::Warn);

        if has_errors {
            total_errors += 1;
        }
        if has_warnings {
            total_warnings += 1;
        }

        match format {
            crate::cli::OutputFormat::Json => {
                for diag in &diags {
                    all_diags.push(serde_json::json!({
                        "skill": dir_name,
                        "level": diag.level,
                        "message": diag.message,
                    }));
                }
            }
            crate::cli::OutputFormat::Text => {
                println!("  {dir_name}/");
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
        crate::cli::OutputFormat::Text => {
            println!();
            let total = dirs.len();
            let ok_count = total - total_errors;
            if total_errors == 0 && total_warnings == 0 {
                println!("  All {total} skill(s) valid.");
            } else {
                println!(
                    "  {ok_count}/{total} valid, {total_errors} with errors, {total_warnings} with warnings."
                );
            }
            println!();
        }
    }

    if total_errors > 0 {
        bail!("skill validation failed: {} error(s) found", total_errors);
    }

    Ok(())
}
