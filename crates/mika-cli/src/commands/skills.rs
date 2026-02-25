use anyhow::Result;
use mika_agent::skills::SkillRegistry;
use mika_common::home;

pub async fn run(name: Option<String>, agent_name: &str) -> Result<()> {
    let global_home = home::resolve_home_dir()?;
    home::migrate_to_multi_agent(&global_home)?;
    let agent_home = home::resolve_agent_home(&global_home, agent_name);
    let skills_dir = agent_home.join("skills");
    let registry = SkillRegistry::from_dir(&skills_dir);

    if let Some(name) = name {
        show_skill_detail(&registry, &name);
    } else {
        list_skills(&registry);
    }
    Ok(())
}

fn list_skills(registry: &SkillRegistry) {
    let skills = registry.skills();
    if skills.is_empty() {
        println!("\n  No skills loaded.\n");
        return;
    }
    println!("\n  Loaded skills ({}):", skills.len());
    for entry in skills {
        let handler_type = match &entry.manifest.handler {
            mika_agent::skills::manifest::Handler::Builtin { .. } => "builtin",
        };
        let always_on = if entry.manifest.options.always_on {
            " [always on]"
        } else {
            ""
        };
        println!(
            "    {} ({}) — {}{}",
            entry.manifest.name, handler_type, entry.manifest.description, always_on
        );
    }
    println!();
}

fn show_skill_detail(registry: &SkillRegistry, name: &str) {
    let skills = registry.skills();
    let found = skills
        .iter()
        .find(|s| s.manifest.name.eq_ignore_ascii_case(name));
    match found {
        Some(entry) => {
            let m = &entry.manifest;
            let handler_desc = match &m.handler {
                mika_agent::skills::manifest::Handler::Builtin { tools } => {
                    format!("builtin (tools: {})", tools.join(", "))
                }
            };
            let keywords = if m.triggers.keywords.is_empty() {
                "none".to_string()
            } else {
                m.triggers.keywords.join(", ")
            };
            println!();
            println!("  Skill: {}", m.name);
            println!("    Description: {}", m.description);
            println!("    Handler:     {handler_desc}");
            println!("    Always on:   {}", m.options.always_on);
            println!("    Timeout:     {}s", m.options.timeout_secs);
            println!("    Keywords:    {keywords}");
            println!("    Path:        {}", entry.dir.display());
            println!();
        }
        None => {
            println!("\n  No skill found with name '{name}'.\n");
        }
    }
}
