use anyhow::{Result, bail};
use dialoguer::{Confirm, Input, MultiSelect, Select};

use mika_common::llm::{LlmContent, LlmMessage, LlmProvider, LlmRequest, LlmRole};
use mika_common::team::TeamAgent;

// -- Agent wizard --

pub struct AgentWizardResult {
    pub display_name: String,
    pub emoji: String,
    pub specialization: String,
    pub communication_style: String,
}

pub fn run_agent_wizard(name: &str) -> Result<AgentWizardResult> {
    println!("\n  Setting up agent '{name}'\n");

    let default_display = capitalize(name);
    let display_name: String = Input::new()
        .with_prompt("  Display name")
        .default(default_display)
        .interact_text()?;

    let emoji: String = Input::new()
        .with_prompt("  Emoji")
        .default("✦".to_string())
        .interact_text()?;

    let specialization: String = Input::new()
        .with_prompt("  Specialization (what this agent focuses on, Enter to skip)")
        .default(String::new())
        .show_default(false)
        .interact_text()?;

    let style_presets = [
        "Professional and concise",
        "Friendly and conversational",
        "Technical and detailed",
        "Custom...",
    ];

    let style_idx = Select::new()
        .with_prompt("  Communication style")
        .items(&style_presets)
        .default(0)
        .interact()?;

    let communication_style = if style_idx == 3 {
        Input::<String>::new()
            .with_prompt("  Describe the communication style")
            .interact_text()?
    } else {
        style_presets[style_idx].to_string()
    };

    Ok(AgentWizardResult {
        display_name,
        emoji,
        specialization,
        communication_style,
    })
}

// -- Team wizard --

pub struct TeamWizardResult {
    pub orchestrator: String,
    pub agents: Vec<TeamAgent>,
    pub max_iterations: u32,
}

pub fn run_team_wizard(name: &str, agents: &[String]) -> Result<TeamWizardResult> {
    println!("\n  Setting up team '{name}'\n");

    // Pick orchestrator
    let orch_idx = Select::new()
        .with_prompt("  Orchestrator agent")
        .items(agents)
        .default(0)
        .interact()?;
    let orchestrator = agents[orch_idx].clone();

    // Pick members from remaining agents
    let remaining: Vec<&String> = agents.iter().filter(|a| **a != orchestrator).collect();

    if remaining.is_empty() {
        bail!("No agents available as team members (need at least 1 besides the orchestrator).");
    }

    let member_indices = MultiSelect::new()
        .with_prompt("  Select team members (Space to toggle, Enter to confirm)")
        .items(&remaining)
        .interact()?;

    if member_indices.is_empty() {
        bail!("A team needs at least 1 member besides the orchestrator.");
    }

    // Collect role and mandate for each member
    let mut team_agents = Vec::new();
    for &idx in &member_indices {
        let agent_name = remaining[idx];
        println!();

        let role: String = Input::new()
            .with_prompt(format!("  Role for '{agent_name}'"))
            .default("specialist".to_string())
            .interact_text()?;

        let mandate: String = Input::new()
            .with_prompt(format!("  Mandate for '{agent_name}'"))
            .default("Complete assigned tasks".to_string())
            .interact_text()?;

        team_agents.push(TeamAgent {
            name: agent_name.clone(),
            role,
            mandate,
        });
    }

    let max_iterations: u32 = Input::new()
        .with_prompt("  Max iterations")
        .default(3)
        .interact_text()?;

    // Print summary
    println!("\n  Team '{name}' summary:");
    println!("    Orchestrator: {orchestrator}");
    for agent in &team_agents {
        println!("    {} ({}): {}", agent.name, agent.role, agent.mandate);
    }
    println!("    Max iterations: {max_iterations}");
    println!();

    let confirmed = Confirm::new()
        .with_prompt("  Create this team?")
        .default(true)
        .interact()?;

    if !confirmed {
        bail!("Team creation cancelled.");
    }

    Ok(TeamWizardResult {
        orchestrator,
        agents: team_agents,
        max_iterations,
    })
}

// -- Soul generation --

pub async fn generate_soul_md(
    provider: &dyn LlmProvider,
    name: &str,
    specialization: &str,
    style: &str,
) -> Option<String> {
    let system = format!(
        "Generate a soul.md personality file for an AI agent named {name}. \
         Specialization: {specialization}. Communication style: {style}. \
         Structure: ## Personality, ## Communication style, ## Proactive behaviors, ## Boundaries. \
         Under 300 words. Output Markdown only, no code fences."
    );

    let request = LlmRequest {
        model: provider.model_name().to_string(),
        system: Some(system),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(format!("Create the soul.md for the agent '{name}'.")),
        }],
        tools: None,
        max_tokens: 1024,
        thinking: None,
    };

    match provider.send_message(&request).await {
        Ok(response) => {
            let text = response.text();
            if text.is_empty() { None } else { Some(text) }
        }
        Err(_) => None,
    }
}

pub fn template_soul_md(name: &str, specialization: &str, style: &str) -> String {
    format!(
        "# {name}\n\
         \n\
         ## Personality\n\
         You are {name}, an AI assistant specializing in {specialization}.\n\
         You are knowledgeable, reliable, and focused on delivering results.\n\
         \n\
         ## Communication style\n\
         - {style}\n\
         - Lead with the answer, then context if needed\n\
         - Match the user's energy — brief if they're brief, detailed if they ask\n\
         - Push back respectfully when something doesn't make sense\n\
         \n\
         ## Proactive behaviors\n\
         - Surface relevant information before being asked\n\
         - Flag potential issues early\n\
         - Suggest improvements when you see opportunities\n\
         \n\
         ## Boundaries\n\
         - Never pretend to have done something you haven't\n\
         - Say \"I don't know\" when you don't know\n\
         - Ask for clarification rather than guess on high-stakes decisions\n"
    )
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("mika"), "Mika");
        assert_eq!(capitalize("work-agent"), "Work-agent");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("A"), "A");
    }

    #[test]
    fn test_template_soul_md_contains_name() {
        let soul = template_soul_md("Atlas", "project management", "Professional and concise");
        assert!(soul.contains("# Atlas"));
        assert!(soul.contains("project management"));
        assert!(soul.contains("Professional and concise"));
    }
}
