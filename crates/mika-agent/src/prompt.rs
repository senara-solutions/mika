use crate::db::{CoreMemoryEntry, Database};
use anyhow::Result;

/// Build the system prompt from core memory.
/// Core memory is always included in the system prompt so the agent has persistent context.
pub fn build_system_prompt(core_memory: &[CoreMemoryEntry]) -> String {
    let mut prompt = String::with_capacity(4096);

    prompt.push_str("You are Mika, a personal AI executive assistant.\n\n");
    prompt.push_str("## Core Memory\n");
    prompt.push_str("This is your persistent memory about the user. You can update it using the update_core_memory tool.\n\n");

    for entry in core_memory {
        prompt.push_str(&format!("### {}\n{}\n\n", entry.key, entry.value));
    }

    prompt.push_str("## Instructions\n");
    prompt.push_str("- Be direct, concise, and professional.\n");
    prompt.push_str("- Remember what the user tells you by updating your core memory.\n");
    prompt.push_str("- Track people, commitments, preferences, and events using the appropriate tools.\n");
    prompt.push_str("- When the user mentions a person, use the upsert_person tool to remember them.\n");
    prompt.push_str("- When the user makes or mentions a commitment, use the add_commitment tool.\n");
    prompt.push_str("- When you learn a preference, use the set_preference tool.\n");
    prompt.push_str("- Never fabricate information. If you don't know something, say so.\n");

    prompt
}

/// Load core memory from the database and build the system prompt.
pub fn load_system_prompt(db: &Database) -> Result<String> {
    let core_memory = db.get_all_core_memory()?;
    Ok(build_system_prompt(&core_memory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_empty() {
        let prompt = build_system_prompt(&[]);
        assert!(prompt.contains("You are Mika"));
        assert!(prompt.contains("Core Memory"));
    }

    #[test]
    fn test_build_system_prompt_with_memory() {
        let entries = vec![
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
        ];

        let prompt = build_system_prompt(&entries);
        assert!(prompt.contains("### user_summary"));
        assert!(prompt.contains("Loves coffee."));
        assert!(prompt.contains("### persona"));
    }
}
