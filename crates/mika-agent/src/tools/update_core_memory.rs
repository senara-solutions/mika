use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;
use std::sync::atomic::Ordering;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::{CORE_MEMORY_SECTIONS, core_memory_section_names, default_self_model};

const MAX_TOKENS_PER_BLOCK: i32 = 500;
// The per-session cap counts UPDATES to already-customized blocks. First-writes
// (writes that promote a block from its default value to a customized value) are
// exempt from this cap AND do not increment `core_memory_edit_count` — bootstrap
// work should always be allowed, even when it spans multiple sessions. Raised from
// 3 to 5 (matches block count and the reflection cap) so a steady-state session
// with genuine multi-block refinements is not truncated. See mika#1782.
const MAX_CORE_MEMORY_EDITS_PER_SESSION: u32 = 5;
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
                Rate limit: up to {MAX_CORE_MEMORY_EDITS_PER_SESSION} updates per session, plus one \
                'first-write' per block that is still at its default value (bootstrap writes are exempt \
                from the cap). \
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

        // Get existing value (for before snapshot, action logic, and first-write detection).
        // Fetched BEFORE the rate-limit check because first-write status gates the cap
        // (writes that promote a default block to a customized value are exempt — mika#1782).
        let existing = ctx.db.get_core_memory(section).await?;
        let before_value = existing.as_ref().map(|e| e.value.as_str());

        // First-write detection: a write is a "first-write" when the block is currently
        // absent OR still holds its canonical default value. First-writes are bootstrap-shape
        // work — they should always be allowed regardless of the per-session cap, and they
        // do not consume the update budget. Non-first-writes (updates to already-customized
        // blocks) are what the cap is protecting against (runaway churn on a long chat).
        let is_first_write = is_write_from_default(ctx, section, before_value).await;

        // Rate limit check (onboarding sessions and first-writes are exempt)
        let max_edits = if ctx.is_reflection {
            MAX_CORE_MEMORY_EDITS_REFLECTION
        } else {
            MAX_CORE_MEMORY_EDITS_PER_SESSION
        };
        let current_edits = ctx.core_memory_edit_count.load(Ordering::Relaxed);
        if current_edits >= max_edits && !ctx.is_onboarding && !is_first_write {
            return Ok(ToolOutput::error(format!(
                "Core memory edit limit ({max_edits}) reached for this session. \
                You can still write to blocks that are at their default value \
                (first-writes are exempt from the cap)."
            )));
        }

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
            "reset" => resolve_default_for_section(ctx, section).await,
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

        // Increment edit counter — only for updates (writes to already-customized blocks).
        // First-writes (block was at its default value) don't consume the per-session budget:
        // bootstrap must be able to complete even across multiple sessions. See mika#1782.
        if !is_first_write {
            ctx.core_memory_edit_count.fetch_add(1, Ordering::Relaxed);
        }

        Ok(ToolOutput::success(format!(
            "Updated core memory '{section}' ({action}). Block size: ~{new_tokens}/{MAX_TOKENS_PER_BLOCK} tokens."
        )))
    }
}

/// Single source of truth for the canonical default value of a core-memory section.
/// `self_model` is per-agent (formatted with the agent display name via
/// `default_self_model`); all other sections read from the static `CORE_MEMORY_SECTIONS`
/// array. Used by BOTH the `reset` action (writes the returned string) AND the
/// first-write detector below (compares before-value against the returned string) so
/// the two sites cannot drift.
///
/// Returns `None` only for section names not in `CORE_MEMORY_SECTIONS`. Upstream
/// validation rejects that path in production code; the `Option` shape lets the
/// first-write detector fall back safely (see `is_write_from_default`).
async fn resolve_default_for_section_checked(
    ctx: &ToolContext<'_>,
    section: &str,
) -> Option<String> {
    if section == "self_model" {
        let display_name = ctx.db.get_agent_display_name().await;
        return Some(default_self_model(&display_name));
    }
    CORE_MEMORY_SECTIONS
        .iter()
        .find(|(k, _)| *k == section)
        .map(|(_, v)| (*v).to_string())
}

/// Variant that panics on unknown section — used by the `reset` action where the
/// section has already been validated upstream.
async fn resolve_default_for_section(ctx: &ToolContext<'_>, section: &str) -> String {
    resolve_default_for_section_checked(ctx, section)
        .await
        .expect("section already validated above")
}

/// Determine whether a pending write is a "first-write" — i.e., the target block is
/// currently absent OR still holds its canonical default value. First-writes are exempt
/// from the per-session cap AND do not consume the update budget (mika#1782).
async fn is_write_from_default(
    ctx: &ToolContext<'_>,
    section: &str,
    before_value: Option<&str>,
) -> bool {
    let Some(current) = before_value else {
        // Block absent in DB → definitionally a first-write.
        return true;
    };

    match resolve_default_for_section_checked(ctx, section).await {
        Some(default) => current == default,
        // Unknown section — validation upstream rejects this path, but be conservative:
        // treat as non-first-write so the cap still applies.
        None => false,
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
        // Post-mika#1782 semantics:
        // - First-write to a block (before_value == default) is exempt from the cap AND
        //   does not increment the edit counter.
        // - Updates (before_value != default) count against the cap
        //   (MAX_CORE_MEMORY_EDITS_PER_SESSION = 5).
        //
        // So for a single block: 1 first-write (exempt) + 5 updates all succeed, and the
        // 6th write (which would be the 5th update) fires the cap.
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // 1 first-write + 5 updates = 6 successful writes.
        for i in 0..6 {
            let result = tool
                .execute(
                    make_input("self_model", "replace", &format!("Edit {i}"), "Testing"),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(
                !result.is_error,
                "Edit {i} should succeed (1 first-write + 5 updates within cap). Got: {}",
                result.content
            );
        }

        // 7th write is the 6th update; cap fires (5 updates max per session).
        let result = tool
            .execute(
                make_input("self_model", "replace", "Edit 6", "One more"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            result.is_error,
            "6th update should be rate limited. Got: {}",
            result.content
        );
        assert!(result.content.contains("edit limit"));
        // The corrective error message must name the first-write escape hatch so the
        // model can steer toward writing to still-default blocks.
        assert!(
            result.content.contains("default value"),
            "cap-hit message should mention the first-write exemption. Got: {}",
            result.content
        );
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
        // Post-mika#1782 semantics for reflection mode:
        // - Reflection cap = MAX_CORE_MEMORY_EDITS_REFLECTION = 5 updates per session.
        // - First-write to a default block is still exempt from the cap (same rule
        //   as conversation mode) and does not increment the counter.
        // - For a single block targeted repeatedly: 1 first-write (exempt) + 5 updates
        //   succeed; the 7th write (6th update) fires the cap.
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

        // 1 first-write + 5 updates = 6 successful writes.
        for i in 0..6 {
            let result = tool.execute(make_reflection_input(i), &ctx).await.unwrap();
            assert!(
                !result.is_error,
                "Reflection edit {i} should succeed (1 first-write + 5 updates within cap). Got: {}",
                result.content
            );
        }

        // 7th write (6th update) fires the reflection cap.
        let result = tool.execute(make_reflection_input(6), &ctx).await.unwrap();
        assert!(
            result.is_error,
            "6th reflection update should be rate limited. Got: {}",
            result.content
        );
        assert!(result.content.contains("edit limit"));
        // Reflection path shares the cap-hit error string with conversation mode; the
        // first-write escape hatch is load-bearing in the fix design and must survive
        // in both surfaces (mika#1782).
        assert!(
            result.content.contains("default value"),
            "reflection cap-hit message should also name the first-write exemption. Got: {}",
            result.content
        );
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

    // -- mika#1782 regression tests: first-write exemption + cap raise --

    /// Reproduces the founding incident (Vincent's cloud Mika, 2026-07-17):
    /// non-onboarding session, agent writes all 5 default core-memory blocks in one turn.
    /// Pre-fix: 4th write fires the cap (MAX_CORE_MEMORY_EDITS_PER_SESSION = 3, and
    /// `is_onboarding=false` because user_summary was customized in the previous
    /// crashed session). Post-fix: all 5 first-writes succeed regardless of cap.
    ///
    /// Failure signal: setting `MAX_CORE_MEMORY_EDITS_PER_SESSION` back to 3 alone does
    /// NOT re-fire this test — the first-write exemption still lets all 5 through. To
    /// re-fire it, both changes must revert (constant + exemption).
    #[tokio::test]
    async fn test_bootstrap_five_blocks_from_default_succeeds() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx(); // is_onboarding = false — the load-bearing detail
        let tool = UpdateCoreMemoryTool;

        // The five default core-memory blocks — mirrors Vincent's onboarding turn.
        let bootstrap_writes = [
            ("user_summary", "Vincent — the user. Speaks French."),
            (
                "self_model",
                "I am Mika, Vincent's executive assistant. First contact 2026-07-17.",
            ),
            ("current_priorities", "Awaiting Vincent's guidance."),
            ("key_people", "Vincent — the user. Speaks French."),
            (
                "workflows",
                "Delegate-then-forget is not allowed. Any work sent to Claude Code must \
                 have a corresponding task created first (via create_task). No exceptions.",
            ),
        ];

        for (section, content) in &bootstrap_writes {
            let result = tool
                .execute(
                    make_input(section, "replace", content, "Onboarding: seed block"),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(
                !result.is_error,
                "First-write of {section} should succeed (bootstrap exemption). Got: {}",
                result.content
            );
        }

        // Confirm every block was actually written — the fix must not silently drop writes.
        for (section, expected) in &bootstrap_writes {
            let entry = harness
                .db
                .get_core_memory(section)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("block {section} should be present after bootstrap"));
            assert_eq!(
                entry.value, *expected,
                "block {section} contents diverge from what was written"
            );
        }
    }

    /// After the update cap is exhausted, a subsequent write to a still-default block
    /// should still succeed via the first-write exemption. Proves the exemption is
    /// orthogonal to the update-cap budget.
    #[tokio::test]
    async fn test_first_write_exempt_after_cap_hit() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // Burn the update cap on one block. First iteration is a first-write (exempt);
        // remaining 5 iterations are updates. That's 5 cap-counted updates — cap now full.
        for i in 0..6 {
            let result = tool
                .execute(
                    make_input(
                        "self_model",
                        "replace",
                        &format!("Iteration {i}"),
                        "Refining",
                    ),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(
                !result.is_error,
                "iteration {i} should succeed. Got: {}",
                result.content
            );
        }

        // Confirm the cap is actually exhausted: another update to self_model must fail.
        let refused = tool
            .execute(
                make_input("self_model", "replace", "Blocked", "Cap should fire"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(refused.is_error, "6th update should be capped");
        assert!(refused.content.contains("edit limit"));

        // A write to workflows — still at default — must succeed via first-write exemption.
        let first_write = tool
            .execute(
                make_input(
                    "workflows",
                    "replace",
                    "Custom workflow instructions.",
                    "First write to workflows",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !first_write.is_error,
            "first-write to still-default workflows should succeed even after cap hit. Got: {}",
            first_write.content
        );
    }

    /// Runaway-update protection is preserved: after 5 first-writes (which don't count),
    /// the 6th write to an already-customized block still fires the cap after 5 updates.
    #[tokio::test]
    async fn test_updates_still_capped_after_bootstrap() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // Five first-writes across the five blocks — none consume budget.
        for (section, content) in &[
            ("user_summary", "Vincent"),
            ("self_model", "I am Mika. Started 2026-07-17."),
            ("current_priorities", "Awaiting guidance"),
            ("key_people", "Vincent"),
            ("workflows", "Custom workflow"),
        ] {
            let r = tool
                .execute(make_input(section, "replace", content, "Bootstrap"), &ctx)
                .await
                .unwrap();
            assert!(!r.is_error, "bootstrap of {section} should succeed");
        }

        // Now do 5 updates (all to already-customized blocks). All should succeed
        // because MAX_CORE_MEMORY_EDITS_PER_SESSION == 5.
        for i in 0..5 {
            let r = tool
                .execute(
                    make_input(
                        "user_summary",
                        "replace",
                        &format!("Vincent — refinement {i}"),
                        "Refining",
                    ),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!r.is_error, "update #{i} should succeed within cap");
        }

        // 6th update fires the cap.
        let capped = tool
            .execute(
                make_input(
                    "user_summary",
                    "replace",
                    "Vincent — refinement 5",
                    "One more",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            capped.is_error,
            "6th update should be capped even after bootstrap. Got: {}",
            capped.content
        );
        assert!(capped.content.contains("edit limit"));
    }

    /// `reset` on a customized block counts against the cap; the re-write after reset
    /// is a first-write (block is back at default) and is exempt. This is intentional
    /// — the test documents the semantic so future readers don't get surprised.
    #[tokio::test]
    async fn test_reset_then_update_counts_as_update() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // First-write (exempt).
        let r = tool
            .execute(
                make_input("user_summary", "replace", "Vincent", "First write"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!r.is_error);

        // Reset back to default. before_value != default → is_first_write = false →
        // counts against the cap. Counter goes to 1.
        let r = tool
            .execute(
                serde_json::json!({
                    "section": "user_summary",
                    "action": "reset",
                    "reasoning": "Start fresh"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!r.is_error, "reset should succeed. Got: {}", r.content);

        // Now user_summary is back at default. Re-write it → treated as a first-write
        // again (exempt from cap, does not increment counter).
        let r = tool
            .execute(
                make_input("user_summary", "replace", "Vincent v2", "Second life"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !r.is_error,
            "re-write after reset is a first-write and should succeed. Got: {}",
            r.content
        );

        // Counter should be at 1 (only the reset counted). We can therefore burn 4 more
        // updates before hitting the cap of 5.
        for i in 0..4 {
            let r = tool
                .execute(
                    make_input(
                        "user_summary",
                        "replace",
                        &format!("update {i}"),
                        "Refining",
                    ),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!r.is_error, "post-reset update #{i} should succeed");
        }

        // Counter is at 5 now (1 reset + 4 loop updates = 5 counted updates); this next
        // attempt would be the 6th cap-counted operation, which fires the guard.
        let capped = tool
            .execute(
                make_input("user_summary", "replace", "capped", "One more"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            capped.is_error,
            "6th cap-counted operation should be blocked. Got: {}",
            capped.content
        );
    }

    /// `self_model`'s default is formatted with the agent display name via
    /// `default_self_model`. The first-write detector must use the same source of truth
    /// or it will misclassify writes on agents whose display name differs from "mika".
    #[tokio::test]
    async fn test_first_write_detection_uses_per_agent_self_model_default() {
        // Custom agent — display name is "operator", not "mika".
        let harness = TestHarness::with_agent("operator");
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = UpdateCoreMemoryTool;

        // The seeded self_model for "operator" should be "I am operator. No interaction
        // history yet." — writing over it must be classified as a first-write.
        // Verify by burning 5 non-self_model updates first (fill the cap) then writing
        // self_model — it should still succeed via first-write exemption.
        for i in 0..5 {
            // Each iteration burns one cap-counted update on user_summary.
            let payload = if i == 0 {
                // First-write to user_summary — exempt.
                "Custom user".to_string()
            } else {
                format!("Refinement {i}")
            };
            let r = tool
                .execute(
                    make_input("user_summary", "replace", &payload, "Setup"),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!r.is_error, "setup iteration {i} should succeed");
        }
        // Counter is now at 4 (i=1..=4 were updates; i=0 was first-write, exempt).
        // Add one more update to hit exactly 5.
        let r = tool
            .execute(
                make_input("user_summary", "replace", "Refinement 5", "Fill cap"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!r.is_error);

        // Cap is now at 5 (full). An update to user_summary would fail.
        let refused = tool
            .execute(
                make_input("user_summary", "replace", "capped", "Should fail"),
                &ctx,
            )
            .await
            .unwrap();
        assert!(refused.is_error);

        // But self_model is still at its per-agent default ("I am operator. …") →
        // first-write exemption fires → succeeds.
        let first_write = tool
            .execute(
                make_input(
                    "self_model",
                    "replace",
                    "I am operator, tuned for calm precision.",
                    "First-write to self_model on non-default agent",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert!(
            !first_write.is_error,
            "first-write to self_model on custom-name agent should succeed. Got: {}",
            first_write.content
        );
    }
}
