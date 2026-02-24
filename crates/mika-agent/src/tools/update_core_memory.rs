use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::sync::atomic::Ordering;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::{CORE_MEMORY_SECTIONS, core_memory_section_names};

const MAX_TOKENS_PER_BLOCK: i32 = 500;
const MAX_CORE_MEMORY_EDITS_PER_SESSION: u32 = 3;

pub struct UpdateCoreMemoryTool;

#[async_trait(?Send)]
impl Tool for UpdateCoreMemoryTool {
    fn name(&self) -> &str {
        "update_core_memory"
    }

    fn definition(&self) -> ToolDefinition {
        let section_names = core_memory_section_names();
        let section_list = section_names.join(", ");
        ToolDefinition {
            name: "update_core_memory".to_string(),
            description: format!(
                "Update your persistent core memory blocks. Core memory is always visible in the system prompt. You have {} blocks: {section_list}. Each block is limited to ~500 tokens.",
                section_names.len()
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "enum": section_names,
                        "description": "Which core memory block to update"
                    },
                    "action": {
                        "type": "string",
                        "enum": ["replace", "append", "remove_line", "reset"],
                        "description": "How to modify the block: replace (full replacement), append (add to end), remove_line (remove first line containing the content), reset (restore block to its default value — content field is ignored)"
                    },
                    "content": {
                        "type": "string",
                        "description": "New content (for replace/append) or text to find and remove (for remove_line). Not required for reset."
                    },
                    "reasoning": {
                        "type": "string",
                        "description": "Why you are making this change (recorded in audit log)"
                    }
                },
                "required": ["section", "action", "reasoning"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let section = input["section"].as_str().unwrap_or("");
        let action = input["action"].as_str().unwrap_or("");
        let content = input["content"].as_str().unwrap_or("");
        let reasoning = input["reasoning"].as_str().unwrap_or("");

        // Validate required fields (content is not required for "reset")
        let content_required = action != "reset";
        if section.is_empty()
            || action.is_empty()
            || (content_required && content.is_empty())
            || reasoning.is_empty()
        {
            return Ok(ToolOutput::error(
                "Required fields missing: section, action, reasoning (and content for non-reset actions).",
            ));
        }

        // Validate section
        let allowed_names = core_memory_section_names();
        if !allowed_names.contains(&section) {
            return Ok(ToolOutput::error(format!(
                "Invalid section '{section}'. Allowed: {}",
                allowed_names.join(", ")
            )));
        }

        // Validate action
        if !["replace", "append", "remove_line", "reset"].contains(&action) {
            return Ok(ToolOutput::error(format!(
                "Invalid action '{action}'. Allowed: replace, append, remove_line, reset"
            )));
        }

        // Input length check
        if content.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "Content too long: {} characters (max: {MAX_INPUT_LEN})",
                content.len()
            )));
        }

        // Rate limit check (onboarding sessions are exempt)
        let current_edits = ctx.core_memory_edit_count.load(Ordering::Relaxed);
        if current_edits >= MAX_CORE_MEMORY_EDITS_PER_SESSION && !ctx.is_onboarding {
            return Ok(ToolOutput::error(format!(
                "Core memory edit limit ({MAX_CORE_MEMORY_EDITS_PER_SESSION}) reached for this session. Focus on using your existing knowledge."
            )));
        }

        // Get existing value (for before snapshot and action logic)
        let existing = ctx.db.get_core_memory(section).await?;
        let before_value = existing.as_ref().map(|e| e.value.as_str());

        // Compute new value based on action
        let new_value = match action {
            "replace" => content.to_string(),
            "append" => {
                let base = before_value.unwrap_or("");
                if base.is_empty() {
                    content.to_string()
                } else {
                    format!("{base}\n{content}")
                }
            }
            "remove_line" => {
                let base = before_value.unwrap_or("");
                let search_lower = content.to_lowercase();
                let mut found = false;
                let lines: Vec<&str> = base
                    .lines()
                    .filter(|line| {
                        if !found && line.to_lowercase().contains(&search_lower) {
                            found = true;
                            false // remove this line
                        } else {
                            true
                        }
                    })
                    .collect();

                if !found {
                    return Ok(ToolOutput::error(format!(
                        "No line containing '{content}' found in {section}."
                    )));
                }

                lines.join("\n")
            }
            "reset" => {
                let default_value = CORE_MEMORY_SECTIONS
                    .iter()
                    .find(|(k, _)| *k == section)
                    .map(|(_, v)| *v)
                    .expect("section already validated above");
                default_value.to_string()
            }
            _ => unreachable!(), // Already validated above
        };

        // Per-block token limit check
        let new_tokens = (new_value.len() / 4) as i32;
        if new_tokens > MAX_TOKENS_PER_BLOCK {
            return Ok(ToolOutput::error(format!(
                "Block '{section}' would be ~{new_tokens} tokens (max: {MAX_TOKENS_PER_BLOCK}). Please shorten the content."
            )));
        }

        // Write to DB
        ctx.db.set_core_memory(section, &new_value).await?;

        // Log audit event
        ctx.db.log_memory_event(
            ctx.session_id,
            "update_core_memory",
            section,
            before_value,
            &new_value,
            Some(reasoning),
        ).await?;

        // Increment edit counter
        ctx.core_memory_edit_count.fetch_add(1, Ordering::Relaxed);

        Ok(ToolOutput::success(format!(
            "Updated core memory '{section}' ({action}). Block size: ~{new_tokens}/{MAX_TOKENS_PER_BLOCK} tokens."
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{test_async_db, test_ctx_with_onboarding as test_ctx};
    use std::sync::atomic::AtomicU32;

    fn make_input(section: &str, action: &str, content: &str, reasoning: &str) -> Value {
        serde_json::json!({
            "section": section,
            "action": action,
            "content": content,
            "reasoning": reasoning
        })
    }

    #[tokio::test]
    async fn test_reject_invalid_section() {
        let db = test_async_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(make_input("invalid_key", "replace", "val", "reason"), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid section"));
    }

    #[tokio::test]
    async fn test_replace_action() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input(
                    "persona",
                    "replace",
                    "Mika — sharp, proactive EA.",
                    "Updated persona",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = db.get_core_memory("persona").await.unwrap().unwrap();
        assert_eq!(entry.value, "Mika — sharp, proactive EA.");
    }

    #[tokio::test]
    async fn test_append_action() {
        let db = test_async_db();
        db.set_core_memory("key_people", "Alice — CTO").await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input("key_people", "append", "Bob — VP Engineering", "Met Bob"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = db.get_core_memory("key_people").await.unwrap().unwrap();
        assert_eq!(entry.value, "Alice — CTO\nBob — VP Engineering");
    }

    #[tokio::test]
    async fn test_append_exceeds_block_limit() {
        let db = test_async_db();
        // Set a block near the limit (~500 tokens = ~2000 chars)
        db.set_core_memory("persona", &"x".repeat(1900)).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input("persona", "append", &"y".repeat(200), "Extending persona"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("tokens"));
    }

    #[tokio::test]
    async fn test_remove_line_action() {
        let db = test_async_db();
        db.set_core_memory("key_people", "Alice — CTO\nBob — VP Eng\nCarol — PM")
            .await
            .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input("key_people", "remove_line", "Bob", "Bob left the company"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = db.get_core_memory("key_people").await.unwrap().unwrap();
        assert_eq!(entry.value, "Alice — CTO\nCarol — PM");
    }

    #[tokio::test]
    async fn test_remove_line_no_match() {
        let db = test_async_db();
        db.set_core_memory("key_people", "Alice — CTO")
            .await
            .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input("key_people", "remove_line", "Bob", "Trying to remove Bob"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("No line containing"));
    }

    #[tokio::test]
    async fn test_rate_limit_triggers() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        // Make 3 successful edits
        for i in 0..3 {
            let result = tool
                .execute(
                    make_input("persona", "replace", &format!("Edit {i}"), "Testing"),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!result.is_error, "Edit {i} should succeed");
        }

        // 4th edit should be rate limited
        let result = tool
            .execute(make_input("persona", "replace", "Edit 3", "One more"), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("edit limit"));
    }

    #[tokio::test]
    async fn test_rate_limit_exempt_during_onboarding() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, true); // onboarding = true
        let tool = UpdateCoreMemoryTool;

        // Make 4 edits — all should succeed during onboarding
        for i in 0..4 {
            let result = tool
                .execute(
                    make_input(
                        "persona",
                        "replace",
                        &format!("Onboarding edit {i}"),
                        "Seeding",
                    ),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!result.is_error, "Onboarding edit {i} should succeed");
        }
    }

    #[tokio::test]
    async fn test_audit_event_logged() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        tool.execute(
            make_input(
                "user_summary",
                "replace",
                "Alice, CEO of Acme.",
                "User introduced herself",
            ),
            &ctx,
        )
        .await
        .unwrap();

        let events = db.get_memory_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "update_core_memory");
        assert_eq!(events[0].target_key, "user_summary");
        assert_eq!(
            events[0].before_value,
            Some("New user. No information yet.".to_string())
        );
        assert_eq!(events[0].after_value, "Alice, CEO of Acme.");
        assert_eq!(
            events[0].reasoning,
            Some("User introduced herself".to_string())
        );
    }

    #[tokio::test]
    async fn test_per_block_token_limit() {
        let db = test_async_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        // 500 tokens * 4 chars/token = 2000 chars. Slightly over should fail.
        let result = tool
            .execute(
                make_input("persona", "replace", &"x".repeat(2004), "Testing limit"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("tokens"));
    }

    #[tokio::test]
    async fn test_reset_action() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter, false);
        let tool = UpdateCoreMemoryTool;

        // Overwrite user_summary with a custom value
        let result = tool
            .execute(
                make_input(
                    "user_summary",
                    "replace",
                    "Alice, CEO of Acme Corp.",
                    "User introduced herself",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = db.get_core_memory("user_summary").await.unwrap().unwrap();
        assert_eq!(entry.value, "Alice, CEO of Acme Corp.");

        // Reset user_summary back to its default (content field omitted)
        let result = tool
            .execute(
                serde_json::json!({
                    "section": "user_summary",
                    "action": "reset",
                    "reasoning": "User asked to start fresh"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("reset"));

        // Verify it is back to the default value from CORE_MEMORY_SECTIONS
        let entry = db.get_core_memory("user_summary").await.unwrap().unwrap();
        assert_eq!(entry.value, "New user. No information yet.");
    }
}
