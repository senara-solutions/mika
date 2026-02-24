use crate::db::{CORE_MEMORY_SECTIONS, Commitment, CoreMemoryEntry};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::fmt::Write;
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
    pub current_utc: DateTime<Utc>,
    pub timezone: Option<String>,
}

fn onboarding_prompt() -> String {
    let section_names: Vec<&str> = CORE_MEMORY_SECTIONS.iter().map(|(k, _)| *k).collect();
    format!(
        "## First Session\n\
         This is your first conversation with the user. Introduce yourself briefly and warmly. \
         Ask who they are and what they're working on. Use update_core_memory to seed all \
         {} blocks ({}) from their \
         responses. Keep it to 2-3 natural exchanges, then transition to being helpful \
         with whatever they need.",
        section_names.len(),
        section_names.join(", ")
    )
}

/// Build the system prompt from context.
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    // Soul content (personality baseline from soul.md)
    if !ctx.soul_content.is_empty() {
        prompt.push_str(ctx.soul_content);
        prompt.push_str("\n\n");
    }

    // Identity
    write!(prompt, "## Identity\nYou are {}.\n\n", ctx.identity.name).unwrap();

    // Current Time
    prompt.push_str("## Current Time\n");
    writeln!(
        prompt,
        "UTC: {}",
        ctx.current_utc.format("%Y-%m-%dT%H:%M:%SZ")
    )
    .unwrap();
    if let Some(tz) = &ctx.timezone {
        writeln!(prompt, "User timezone: {tz}").unwrap();
    }
    prompt.push('\n');

    // Core Memory
    prompt.push_str("## Core Memory\n");
    prompt.push_str(
        "These are your persistent memory blocks. Update them using the update_core_memory tool.\n\n",
    );

    for entry in ctx.core_memory {
        write!(prompt, "### {}\n{}\n\n", entry.key, entry.value).unwrap();
    }

    // Instructions
    prompt.push_str("## Instructions\n");
    prompt.push_str("- Update your core memory when you learn important things about the user.\n");
    prompt.push_str(
        "- Track people, commitments, preferences, and events using the appropriate tools.\n",
    );
    prompt.push_str("- Never fabricate information. If you don't know something, say so.\n");
    prompt.push_str(
        "- You can create reminders with create_reminder (requires ISO 8601 datetime in UTC). \
         Use the current time shown above to compute future times.\n",
    );
    prompt.push_str(
        "- You can list and cancel reminders with list_reminders and cancel_reminder.\n",
    );
    prompt.push_str(
        "- Mark commitments as completed or cancelled using the update_fact tool.\n",
    );
    prompt.push_str(
        "- You can reset a core memory section to its default value using update_core_memory with the reset action.\n",
    );
    let section_names: Vec<&str> = CORE_MEMORY_SECTIONS.iter().map(|(k, _)| *k).collect();
    write!(
        prompt,
        "- You have {} memory blocks ({}),\n  each limited to ~500 tokens. Be concise and prioritize what matters most.\n",
        section_names.len(),
        section_names.join(", ")
    )
    .unwrap();

    // Onboarding prompt (only on first session)
    if ctx.is_onboarding {
        prompt.push('\n');
        prompt.push_str(&onboarding_prompt());
        prompt.push('\n');
    }

    prompt
}

/// Context for building a silent mode (heartbeat/reminder) system prompt.
pub struct SilentPromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub pending_commitments: &'a [Commitment],
    pub trigger_context: &'a str,
    pub current_utc: DateTime<Utc>,
    pub timezone: Option<String>,
}

/// Build a system prompt for silent mode (heartbeat/reminder).
/// The agent's text output is NOT delivered — it must use send_message to contact the user.
pub fn build_silent_prompt(ctx: &SilentPromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    // Soul content
    if !ctx.soul_content.is_empty() {
        prompt.push_str(ctx.soul_content);
        prompt.push_str("\n\n");
    }

    // Identity
    write!(prompt, "## Identity\nYou are {}.\n\n", ctx.identity.name).unwrap();

    // Current Time
    prompt.push_str("## Current Time\n");
    writeln!(
        prompt,
        "UTC: {}",
        ctx.current_utc.format("%Y-%m-%dT%H:%M:%SZ")
    )
    .unwrap();
    if let Some(tz) = &ctx.timezone {
        writeln!(prompt, "User timezone: {tz}").unwrap();
    }
    prompt.push('\n');

    // Core Memory
    prompt.push_str("## Core Memory\n");
    for entry in ctx.core_memory {
        write!(prompt, "### {}\n{}\n\n", entry.key, entry.value).unwrap();
    }

    // Pending commitments
    if !ctx.pending_commitments.is_empty() {
        prompt.push_str("## Pending Commitments\n");
        for c in ctx.pending_commitments {
            let due = c.due_date.as_deref().unwrap_or("no due date");
            writeln!(prompt, "- {} (due: {})", c.description, due).unwrap();
        }
        prompt.push('\n');
    }

    // Silent mode instructions
    prompt.push_str("## Silent Mode\n");
    prompt.push_str(
        "You are in SILENT MODE. Your text output is NOT delivered to the user.\n\
         Use the send_message tool to contact the user. If you have nothing worthwhile \
         to say, simply respond with a brief internal note and do NOT call send_message.\n\n",
    );

    // Available tools summary
    prompt.push_str("## Available Tools\n");
    prompt.push_str(
        "You have access to all tools. Use them as appropriate:\n\
         - search_memory / store_fact / update_core_memory / update_fact: Read and update the user's memory\n\
         - create_reminder / list_reminders / cancel_reminder: Manage reminders\n\
         - send_message: Contact the user (required in silent mode for output)\n\n",
    );

    // Trigger-specific context
    prompt.push_str("## Trigger\n");
    prompt.push_str(ctx.trigger_context);
    prompt.push('\n');

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

    fn test_time() -> DateTime<Utc> {
        "2026-02-24T12:00:00Z".parse().unwrap()
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
            current_utc: test_time(),
            timezone: None,
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
            current_utc: test_time(),
            timezone: None,
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
            current_utc: test_time(),
            timezone: None,
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
            current_utc: test_time(),
            timezone: None,
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
            current_utc: test_time(),
            timezone: None,
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
            current_utc: test_time(),
            timezone: None,
        };

        let prompt = build_system_prompt(&ctx);
        // Should start directly with Identity section when soul is empty
        assert!(prompt.starts_with("## Identity"));
    }

    #[test]
    fn test_silent_prompt_heartbeat() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            pending_commitments: &[],
            trigger_context: "This is a HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Silent Mode"));
        assert!(prompt.contains("send_message"));
        assert!(prompt.contains("HEARTBEAT check-in"));
        assert!(prompt.contains("## Core Memory"));
    }

    #[test]
    fn test_silent_prompt_reminder() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "REMINDER: Call the dentist",
            current_utc: test_time(),
            timezone: None,
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Call the dentist"));
        assert!(prompt.contains("NOT delivered"));
    }

    #[test]
    fn test_silent_prompt_includes_commitments() {
        use crate::db::Commitment;
        let identity = test_identity();
        let commitments = vec![Commitment {
            id: 1,
            description: "Review budget".to_string(),
            status: "pending".to_string(),
            due_date: Some("2026-03-01".to_string()),
            person_id: None,
            created_at: "2026-02-24".to_string(),
            completed_at: None,
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &commitments,
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Review budget"));
        assert!(prompt.contains("2026-03-01"));
    }

    #[test]
    fn test_prompt_includes_current_time() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Current Time"));
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(!prompt.contains("User timezone"));
    }

    #[test]
    fn test_prompt_includes_timezone_when_set() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: Some("+08:00".to_string()),
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(prompt.contains("User timezone: +08:00"));
    }

    #[test]
    fn test_silent_prompt_includes_current_time() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: Some("-05:00".to_string()),
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Current Time"));
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(prompt.contains("User timezone: -05:00"));
    }
}
