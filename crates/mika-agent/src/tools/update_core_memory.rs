use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::sync::atomic::Ordering;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::{CORE_MEMORY_SECTIONS, core_memory_section_names, default_self_model};

const MAX_TOKENS_PER_BLOCK: i32 = 500;
const MAX_CORE_MEMORY_EDITS_PER_SESSION: u32 = 3;
const MAX_CORE_MEMORY_EDITS_REFLECTION: u32 = 5;

pub struct UpdateCoreMemoryTool;

#[async_trait]
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
                "Update your persistent core memory blocks. Core memory is always visible in your system prompt. \
                You have {} blocks: {section_list}. Each block is limited to ~500 tokens. \
                All three parameters 'section', 'action', and 'reasoning' are REQUIRED. \
                The 'content' parameter is also required unless action is 'reset'.",
                section_names.len()
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "section": {
                        "type": "string",
                        "enum": section_names,
                        "description": format!("REQUIRED. Which core memory block to update. Must be one of: {section_list}")
                    },
                    "action": {
                        "type": "string",
                        "enum": ["replace", "append", "remove_line", "reset"],
                        "description": "REQUIRED. How to modify the block. Must be one of: 'replace' (overwrite entire block), 'append' (add text to end of block), 'remove_line' (remove first line containing the content string), 'reset' (restore block to default value, content parameter is ignored)"
                    },
                    "content": {
                        "type": "string",
                        "description": "The text content. Required for 'replace' (new block content), 'append' (text to add), and 'remove_line' (substring to match). Not required when action is 'reset'."
                    },
                    "reasoning": {
                        "type": "string",
                        "description": "REQUIRED. A brief explanation of why you are making this change. This is recorded in the audit log."
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Only required in reflection mode. Cite a specific conversation timestamp and quote as justification for this change."
                    }
                },
                "required": ["section", "action", "reasoning"],
                "additionalProperties": false
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let section = input["section"].as_str().unwrap_or("");
        let action = input["action"].as_str().unwrap_or("");
        let content = input["content"].as_str().unwrap_or("");
        // Accept `reason` as an alias for `reasoning` to accommodate tokenization
        // quirks in some LLMs (e.g., minimax/minimax-m2.7 consistently truncates
        // the key to `reason`). Canonical `reasoning` wins when both are present.
        // The alias is intentionally undocumented in the tool schema — we do not
        // want to advertise the misspelling. See issue #488.
        let reasoning_canonical = input["reasoning"].as_str();
        let reasoning = reasoning_canonical
            .or_else(|| input["reason"].as_str())
            .unwrap_or("");
        if reasoning_canonical.is_none() && input.get("reason").is_some() {
            tracing::debug!(
                target: "mika::tools",
                model = ?ctx.model_name,
                provider = ?ctx.provider_name,
                "update_core_memory: accepted 'reason' as alias for 'reasoning'"
            );
        }

        // Validate required fields with specific error messages
        let content_required = action != "reset" && !action.is_empty();
        let mut missing = Vec::new();
        if section.is_empty() {
            missing.push("section");
        }
        if action.is_empty() {
            missing.push("action");
        }
        if reasoning.is_empty() {
            missing.push("reasoning");
        }
        if content_required && content.is_empty() {
            missing.push("content");
        }
        if !missing.is_empty() {
            let allowed_sections = core_memory_section_names().join(", ");
            return Ok(ToolOutput::error(format!(
                "Missing required parameter(s): {}. \
                You must provide: 'section' (one of: {allowed_sections}), \
                'action' (one of: replace, append, remove_line, reset), \
                'reasoning' (why you are making this change), \
                and 'content' (the text, required unless action is 'reset'). \
                Example: {{\"section\": \"user_summary\", \"action\": \"replace\", \"content\": \"New info\", \"reasoning\": \"User shared this\"}}",
                missing.join(", ")
            )));
        }

        // Reflection mode: require evidence field
        if let Some(err) = super::check_reflection_evidence(ctx, &input) {
            return Ok(err);
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
        let max_edits = if ctx.is_reflection {
            MAX_CORE_MEMORY_EDITS_REFLECTION
        } else {
            MAX_CORE_MEMORY_EDITS_PER_SESSION
        };
        let current_edits = ctx.core_memory_edit_count.load(Ordering::Relaxed);
        if current_edits >= max_edits && !ctx.is_onboarding {
            return Ok(ToolOutput::error(format!(
                "Core memory edit limit ({max_edits}) reached for this session. Focus on using your existing knowledge."
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
                let static_default = CORE_MEMORY_SECTIONS
                    .iter()
                    .find(|(k, _)| *k == section)
                    .map(|(_, v)| *v)
                    .expect("section already validated above");
                if section == "self_model" {
                    let display_name = ctx.db.get_agent_display_name().await;
                    default_self_model(&display_name)
                } else {
                    static_default.to_string()
                }
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

        // Log audit event (include evidence in reasoning when in reflection mode)
        let audit_reasoning = if ctx.is_reflection {
            let evidence = input["evidence"].as_str().unwrap_or("");
            format!("{reasoning} [evidence] {evidence}")
        } else {
            reasoning.to_string()
        };
        ctx.db
            .log_audit_event(
                ctx.session_id,
                "update_core_memory",
                section,
                before_value,
                Some(&*new_value),
                Some(&audit_reasoning),
                Some(ctx.trace_id),
            )
            .await?;

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
    use crate::test_utils::test_helpers::TestHarness;

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
        let harness = TestHarness::new();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input(
                    "self_model",
                    "replace",
                    "Mika — sharp, proactive EA.",
                    "Updated persona",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = harness
            .db
            .get_core_memory("self_model")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.value, "Mika — sharp, proactive EA.");
    }

    #[tokio::test]
    async fn test_append_action() {
        let harness = TestHarness::new();
        harness
            .db
            .set_core_memory("key_people", "Alice — CTO")
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input("key_people", "append", "Bob — VP Engineering", "Met Bob"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = harness
            .db
            .get_core_memory("key_people")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.value, "Alice — CTO\nBob — VP Engineering");
    }

    #[tokio::test]
    async fn test_append_exceeds_block_limit() {
        let harness = TestHarness::new();
        // Set a block near the limit (~500 tokens = ~2000 chars)
        harness
            .db
            .set_core_memory("self_model", &"x".repeat(1900))
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input(
                    "self_model",
                    "append",
                    &"y".repeat(200),
                    "Extending persona",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("tokens"));
    }

    #[tokio::test]
    async fn test_remove_line_action() {
        let harness = TestHarness::new();
        harness
            .db
            .set_core_memory("key_people", "Alice — CTO\nBob — VP Eng\nCarol — PM")
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                make_input("key_people", "remove_line", "Bob", "Bob left the company"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let entry = harness
            .db
            .get_core_memory("key_people")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.value, "Alice — CTO\nCarol — PM");
    }

    #[tokio::test]
    async fn test_remove_line_no_match() {
        let harness = TestHarness::new();
        harness
            .db
            .set_core_memory("key_people", "Alice — CTO")
            .await
            .unwrap();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // Make 3 successful edits
        for i in 0..3 {
            let result = tool
                .execute(
                    make_input("self_model", "replace", &format!("Edit {i}"), "Testing"),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!result.is_error, "Edit {i} should succeed");
        }

        // 4th edit should be rate limited
        let result = tool
            .execute(
                make_input("self_model", "replace", "Edit 3", "One more"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("edit limit"));
    }

    #[tokio::test]
    async fn test_rate_limit_exempt_during_onboarding() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx_with_onboarding(true);
        let tool = UpdateCoreMemoryTool;

        // Make 4 edits — all should succeed during onboarding
        for i in 0..4 {
            let result = tool
                .execute(
                    make_input(
                        "self_model",
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
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
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

        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "update_core_memory");
        assert_eq!(events[0].target_key, "user_summary");
        assert_eq!(
            events[0].before_value,
            Some("No information about the user yet.".to_string())
        );
        assert_eq!(
            events[0].after_value,
            Some("Alice, CEO of Acme.".to_string())
        );
        assert_eq!(
            events[0].reasoning,
            Some("User introduced herself".to_string())
        );
    }

    #[tokio::test]
    async fn test_per_block_token_limit() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // 500 tokens * 4 chars/token = 2000 chars. Slightly over should fail.
        let result = tool
            .execute(
                make_input("self_model", "replace", &"x".repeat(2004), "Testing limit"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("tokens"));
    }

    #[tokio::test]
    async fn test_reflection_requires_evidence() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx_with_reflection();
        let tool = UpdateCoreMemoryTool;

        // Without evidence field → rejected
        let result = tool
            .execute(
                make_input(
                    "self_model",
                    "replace",
                    "Updated persona",
                    "Reflection update",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("evidence"));
    }

    #[tokio::test]
    async fn test_reflection_with_evidence_succeeds() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx_with_reflection();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "section": "self_model",
                    "action": "replace",
                    "content": "Updated persona",
                    "reasoning": "Reflection update",
                    "evidence": "User said 'I prefer a more formal tone' at 14:30"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_reflection_edit_cap_is_five() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx_with_reflection();
        let tool = UpdateCoreMemoryTool;

        let make_reflection_input = |i: u32| {
            serde_json::json!({
                "section": "self_model",
                "action": "replace",
                "content": format!("Edit {i}"),
                "reasoning": "Reflection",
                "evidence": format!("Evidence for edit {i}")
            })
        };

        // Make 5 successful edits
        for i in 0..5 {
            let result = tool.execute(make_reflection_input(i), &ctx).await.unwrap();
            assert!(!result.is_error, "Reflection edit {i} should succeed");
        }

        // 6th edit should be rate limited
        let result = tool.execute(make_reflection_input(5), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("edit limit"));
    }

    #[tokio::test]
    async fn test_reflection_audit_includes_evidence() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx_with_reflection();
        let tool = UpdateCoreMemoryTool;

        tool.execute(
            serde_json::json!({
                "section": "user_summary",
                "action": "replace",
                "content": "Alice, CEO",
                "reasoning": "Promoting from facts",
                "evidence": "User said 'I am the CEO of Acme' at 10:15"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        let reasoning = events[0].reasoning.as_deref().unwrap();
        assert!(reasoning.contains("[evidence]"));
        assert!(reasoning.contains("CEO of Acme"));
    }

    #[tokio::test]
    async fn test_missing_section_and_reasoning_lists_both() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // Simulate what MiniMax-M2.5 sends: only action + content, missing section + reasoning
        let result = tool
            .execute(
                serde_json::json!({
                    "action": "replace",
                    "content": "Some text"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("section"));
        assert!(result.content.contains("reasoning"));
        assert!(result.content.contains("Missing required parameter"));
        // Should include an example call
        assert!(result.content.contains("Example:"));
    }

    #[tokio::test]
    async fn test_missing_only_section_specific_error() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "action": "replace",
                    "content": "Some text",
                    "reasoning": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("section"));
        // Should NOT list reasoning as missing since it was provided
        assert!(
            !result
                .content
                .starts_with("Missing required parameter(s): reasoning")
        );
    }

    #[tokio::test]
    async fn test_missing_content_for_non_reset_action() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "section": "user_summary",
                    "action": "replace",
                    "reasoning": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("content"));
        assert!(result.content.contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn test_all_fields_missing_lists_all() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("section"));
        assert!(result.content.contains("action"));
        assert!(result.content.contains("reasoning"));
    }

    #[tokio::test]
    async fn test_reason_alias_accepted_when_only_reason_provided() {
        // Regression for #488: minimax/minimax-m2.7 truncates `reasoning` to
        // `reason` in tool input JSON. The engine should accept the alias.
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "section": "user_summary",
                    "action": "replace",
                    "content": "Alice, CEO of Acme Corp.",
                    "reason": "User introduced herself"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "expected success, got: {}",
            result.content
        );

        let entry = harness
            .db
            .get_core_memory("user_summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.value, "Alice, CEO of Acme Corp.");

        // Audit event should record the aliased reasoning text.
        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        let reasoning = events[0].reasoning.as_deref().unwrap();
        assert!(reasoning.contains("User introduced herself"));
    }

    #[tokio::test]
    async fn test_reasoning_wins_when_both_reason_and_reasoning_provided() {
        // When both canonical `reasoning` and alias `reason` are present,
        // `reasoning` must win (canonical field takes precedence).
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "section": "user_summary",
                    "action": "replace",
                    "content": "Bob",
                    "reasoning": "canonical value",
                    "reason": "alias value"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        let reasoning = events[0].reasoning.as_deref().unwrap();
        assert!(
            reasoning.contains("canonical value"),
            "expected canonical 'reasoning' to win, got: {reasoning}"
        );
        assert!(
            !reasoning.contains("alias value"),
            "alias 'reason' should be ignored when canonical is present, got: {reasoning}"
        );
    }

    #[tokio::test]
    async fn test_reset_action() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
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

        let entry = harness
            .db
            .get_core_memory("user_summary")
            .await
            .unwrap()
            .unwrap();
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
        let entry = harness
            .db
            .get_core_memory("user_summary")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.value, "No information about the user yet.");
    }
}
