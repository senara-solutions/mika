use anyhow::{Context, Result, bail};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::executor;
use mika_agent::skills::index::scan_skills_dir;
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
            show_skill_detail(&registry, &name);
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

fn show_skill_detail(registry: &SkillRegistry, name: &str) {
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
