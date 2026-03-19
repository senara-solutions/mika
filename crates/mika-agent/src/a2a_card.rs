use mika_a2a::types::{AgentCapabilities, AgentCard, AgentProvider, AgentSkill, SecurityScheme};
use std::collections::HashMap;

use crate::skills::SkillRegistry;

/// Build an A2A Agent Card from an agent's skill registry.
pub fn build_agent_card(
    agent_name: &str,
    description: &str,
    skills: &SkillRegistry,
    base_url: &str,
) -> AgentCard {
    let a2a_skills: Vec<AgentSkill> = skills
        .skills()
        .iter()
        .map(|entry| {
            let manifest = &entry.manifest;
            AgentSkill {
                id: manifest.skill.name.clone(),
                name: manifest.skill.name.clone(),
                description: manifest.skill.description.clone(),
                tags: manifest.triggers.keywords.clone(),
                examples: None,
                input_modes: None,
                output_modes: None,
            }
        })
        .collect();

    let mut security_schemes = HashMap::new();
    security_schemes.insert(
        "apiKey".to_string(),
        SecurityScheme::ApiKey {
            name: "x-api-key".to_string(),
            location: "header".to_string(),
        },
    );

    AgentCard {
        name: agent_name.to_string(),
        description: description.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        url: format!("{base_url}/a2a/{agent_name}"),
        provider: Some(AgentProvider {
            organization: "Mika".to_string(),
            url: None,
        }),
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(true),
        },
        security_schemes: Some(security_schemes),
        security_requirements: None,
        default_input_modes: vec!["text/plain".to_string()],
        default_output_modes: vec!["text/plain".to_string()],
        skills: a2a_skills,
        icon_url: None,
        documentation_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::{SkillInfo, SkillManifest, Triggers};

    fn empty_registry() -> SkillRegistry {
        let tmp = tempfile::tempdir().unwrap();
        SkillRegistry::from_dir(tmp.path())
    }

    fn registry_with_skill(name: &str, desc: &str, keywords: Vec<String>) -> SkillRegistry {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let manifest = SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: desc.to_string(),
                version: "1.0.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
            },
            triggers: Triggers {
                keywords: keywords.clone(),
            },
        };
        let toml_str = toml::to_string(&manifest).unwrap();
        std::fs::write(skill_dir.join("skill.toml"), toml_str).unwrap();
        SkillRegistry::from_dir(tmp.path())
    }

    #[test]
    fn build_card_empty_skills() {
        let registry = empty_registry();
        let card = build_agent_card(
            "test-agent",
            "Test agent description",
            &registry,
            "http://localhost:8080",
        );

        assert_eq!(card.name, "test-agent");
        assert_eq!(card.description, "Test agent description");
        assert_eq!(card.url, "http://localhost:8080/a2a/test-agent");
        assert!(!card.version.is_empty());
        assert!(card.skills.is_empty());
    }

    #[test]
    fn build_card_capabilities() {
        let registry = empty_registry();
        let card = build_agent_card("agent", "desc", &registry, "http://localhost");

        assert_eq!(card.capabilities.streaming, Some(true));
        assert_eq!(card.capabilities.push_notifications, Some(true));
    }

    #[test]
    fn build_card_default_modes() {
        let registry = empty_registry();
        let card = build_agent_card("agent", "desc", &registry, "http://localhost");

        assert_eq!(card.default_input_modes, vec!["text/plain"]);
        assert_eq!(card.default_output_modes, vec!["text/plain"]);
    }

    #[test]
    fn build_card_provider() {
        let registry = empty_registry();
        let card = build_agent_card("agent", "desc", &registry, "http://localhost");

        let provider = card.provider.unwrap();
        assert_eq!(provider.organization, "Mika");
    }

    #[test]
    fn build_card_security_schemes() {
        let registry = empty_registry();
        let card = build_agent_card("agent", "desc", &registry, "http://localhost");

        let schemes = card.security_schemes.unwrap();
        let api_key = schemes.get("apiKey").unwrap();
        match api_key {
            SecurityScheme::ApiKey { name, location } => {
                assert_eq!(name, "x-api-key");
                assert_eq!(location, "header");
            }
            _ => panic!("expected ApiKey security scheme"),
        }
    }

    #[test]
    fn build_card_with_skills() {
        let registry = registry_with_skill(
            "summarize",
            "Summarizes documents",
            vec!["summary".to_string(), "tldr".to_string()],
        );
        let card = build_agent_card("agent", "desc", &registry, "http://localhost");

        assert_eq!(card.skills.len(), 1);
        let skill = &card.skills[0];
        assert_eq!(skill.id, "summarize");
        assert_eq!(skill.name, "summarize");
        assert_eq!(skill.description, "Summarizes documents");
        assert_eq!(skill.tags, vec!["summary", "tldr"]);
    }

    #[test]
    fn build_card_url_format() {
        let registry = empty_registry();
        let card = build_agent_card("my-agent", "desc", &registry, "https://api.example.com");
        assert_eq!(card.url, "https://api.example.com/a2a/my-agent");
    }
}
