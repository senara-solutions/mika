use crate::db::CoreMemoryEntry;
use serde::Deserialize;
use std::path::Path;

/// Agent identity loaded from ~/.mika/identity.toml.
#[derive(Debug, Deserialize, Clone)]
pub struct Identity {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_emoji")]
    pub emoji: String,
}

fn default_name() -> String {
    "Mika".to_string()
}

fn default_emoji() -> String {
    "✦".to_string()
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: default_name(),
            emoji: default_emoji(),
        }
    }
}

/// Load identity from ~/.mika/identity.toml.
/// Returns defaults if file is missing or invalid.
pub fn load_identity(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Identity::default(),
    }
}

/// Context needed to build the system prompt.
pub struct PromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub is_onboarding: bool,
}

const ONBOARDING_PROMPT: &str = "\
## First Session
This is your first conversation with the user. Introduce yourself briefly and warmly. \
Ask who they are and what they're working on. Use update_core_memory to seed all \
four blocks (persona, user_summary, current_priorities, key_people) from their \
responses. Keep it to 2-3 natural exchanges, then transition to being helpful \
with whatever they need.";

/// Build the system prompt from context.
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    // Soul content (personality baseline from soul.md)
    if !ctx.soul_content.is_empty() {
        prompt.push_str(ctx.soul_content);
        prompt.push_str("\n\n");
    }

    // Identity
    prompt.push_str(&format!("## Identity\nYou are {}.\n\n", ctx.identity.name));

    // Core Memory
    prompt.push_str("## Core Memory\n");
    prompt.push_str(
        "These are your persistent memory blocks. Update them using the update_core_memory tool.\n\n",
    );

    for entry in ctx.core_memory {
        prompt.push_str(&format!("### {}\n{}\n\n", entry.key, entry.value));
    }

    // Instructions
    prompt.push_str("## Instructions\n");
    prompt.push_str("- Update your core memory when you learn important things about the user.\n");
    prompt.push_str(
        "- Track people, commitments, preferences, and events using the appropriate tools.\n",
    );
    prompt.push_str("- Never fabricate information. If you don't know something, say so.\n");
    prompt.push_str("- You have 4 memory blocks (persona, user_summary, current_priorities, key_people),\n  each limited to ~500 tokens. Be concise and prioritize what matters most.\n");

    // Onboarding prompt (only on first session)
    if ctx.is_onboarding {
        prompt.push('\n');
        prompt.push_str(ONBOARDING_PROMPT);
        prompt.push('\n');
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> Identity {
        Identity {
            name: "Mika".to_string(),
            emoji: "✦".to_string(),
        }
    }

    fn test_core_memory() -> Vec<CoreMemoryEntry> {
        vec![
            CoreMemoryEntry {
                key: "user_summary".to_string(),
                value: "Loves coffee.".to_string(),
                token_count: 3,
                updated_at: "2026-01-01".to_string(),
            },
            CoreMemoryEntry {
                key: "persona".to_string(),
                value: "Mika — assistant.".to_string(),
                token_count: 4,
                updated_at: "2026-01-01".to_string(),
            },
        ]
    }

    #[test]
    fn test_prompt_includes_soul_content() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "You are a sharp, proactive executive assistant.",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.starts_with("You are a sharp, proactive executive assistant."));
    }

    #[test]
    fn test_prompt_includes_all_core_memory_blocks() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("### user_summary"));
        assert!(prompt.contains("Loves coffee."));
        assert!(prompt.contains("### persona"));
        assert!(prompt.contains("Mika — assistant."));
    }

    #[test]
    fn test_prompt_includes_identity_name() {
        let identity = Identity {
            name: "TestBot".to_string(),
            emoji: "🤖".to_string(),
        };
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("You are TestBot."));
    }

    #[test]
    fn test_onboarding_prompt_injected() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: true,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## First Session"));
        assert!(prompt.contains("Introduce yourself briefly"));
    }

    #[test]
    fn test_no_onboarding_for_returning_user() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## First Session"));
    }

    #[test]
    fn test_load_identity_parses_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Agent X\"\nemoji = \"🕶\"\n",
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Agent X");
        assert_eq!(identity.emoji, "🕶");
    }

    #[test]
    fn test_load_identity_defaults_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Mika");
        assert_eq!(identity.emoji, "✦");
    }

    #[test]
    fn test_prompt_empty_soul_no_extra_whitespace() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
        };

        let prompt = build_system_prompt(&ctx);
        // Should start directly with Identity section when soul is empty
        assert!(prompt.starts_with("## Identity"));
    }
}
