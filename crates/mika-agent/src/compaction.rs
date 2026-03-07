use anyhow::{Context, Result};
use mika_common::claude::{ClaudeClient, Message, MessageContent, MessagesRequest};
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::ConversationMessage;

const COMPACTION_THRESHOLD: usize = 50;
const CONTEXT_WINDOW: usize = 20;
const MAX_COMPACTION_BATCH: usize = 100;
const MAX_SUMMARY_CHARS: usize = 4000;
const MAX_COMPACTION_INPUT_CHARS: usize = 50_000;

const SUMMARIZATION_SYSTEM_PROMPT: &str = "\
You are summarizing a conversation between an AI executive assistant and their user.
Preserve: key decisions, action items, commitments, user preferences, important facts about people.
Discard: pleasantries, small talk, repeated information.
Keep the summary concise (under 500 tokens). Use bullet points.
If there is an existing summary, merge it with the new information.";

/// Check if compaction is needed and perform it if so.
/// Called after each agent turn completes.
pub async fn maybe_compact(db: &AsyncDatabase, claude: &ClaudeClient) -> Result<()> {
    let total = db.count_messages().await?;
    if total <= COMPACTION_THRESHOLD {
        debug!(
            total,
            threshold = COMPACTION_THRESHOLD,
            "compaction not needed"
        );
        return Ok(());
    }

    let existing_summary = db.load_conversation_summary().await?;
    let old_messages = db.load_messages_before_window(CONTEXT_WINDOW).await?;
    if old_messages.is_empty() {
        debug!("no messages outside context window to compact");
        return Ok(());
    }

    // Cap batch size to prevent sending too much to the summarization API
    let batch = if old_messages.len() > MAX_COMPACTION_BATCH {
        warn!(
            total = old_messages.len(),
            batch = MAX_COMPACTION_BATCH,
            "capping compaction batch size"
        );
        &old_messages[..MAX_COMPACTION_BATCH]
    } else {
        &old_messages
    };

    info!(old_count = batch.len(), total, "compacting conversation");

    let mut summary_text = summarize_messages(claude, batch, existing_summary.as_ref()).await?;

    // Truncate summary if it exceeds the size guard
    if summary_text.len() > MAX_SUMMARY_CHARS {
        warn!(
            len = summary_text.len(),
            max = MAX_SUMMARY_CHARS,
            "truncating oversized summary"
        );
        summary_text.truncate(MAX_SUMMARY_CHARS);
        // Ensure we don't cut in the middle of a multi-byte char
        while !summary_text.is_char_boundary(summary_text.len()) {
            summary_text.pop();
        }
    }

    let highest_id = batch.last().map(|m| m.id).unwrap_or(0);
    db.replace_with_summary(&summary_text, highest_id).await?;

    info!(compacted_through_id = highest_id, "compaction complete");
    Ok(())
}

/// Call Claude to summarize a batch of old messages, optionally merging with
/// an existing summary.
async fn summarize_messages(
    claude: &ClaudeClient,
    messages: &[ConversationMessage],
    existing_summary: Option<&ConversationMessage>,
) -> Result<String> {
    let mut user_prompt = String::with_capacity(2048);

    if let Some(summary) = existing_summary {
        user_prompt.push_str("## Existing Summary\n");
        user_prompt.push_str(&summary.content);
        user_prompt.push_str("\n\n");
    }

    user_prompt.push_str("## Messages to Summarize\n");
    let mut char_count = 0usize;
    let mut included = 0usize;
    for msg in messages {
        // Append tool names from metadata so summaries mention tool usage
        let tool_suffix = extract_tool_names(&msg.metadata);
        let msg_chars = msg.role.len() + 2 + msg.content.len() + tool_suffix.len() + 1;
        if char_count + msg_chars > MAX_COMPACTION_INPUT_CHARS {
            break;
        }
        char_count += msg_chars;
        included += 1;
        user_prompt.push_str(&msg.role);
        user_prompt.push_str(": ");
        user_prompt.push_str(&msg.content);
        if !tool_suffix.is_empty() {
            user_prompt.push_str(&tool_suffix);
        }
        user_prompt.push('\n');
    }
    if included < messages.len() {
        warn!(
            total = messages.len(),
            included,
            char_budget = MAX_COMPACTION_INPUT_CHARS,
            "truncated compaction input to stay within character budget"
        );
    }

    user_prompt.push_str("\nPlease produce a concise bullet-point summary.");

    let request = MessagesRequest {
        model: claude.model.clone(),
        max_tokens: 1024,
        system: Some(SUMMARIZATION_SYSTEM_PROMPT.to_string()),
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text(user_prompt),
        }],
        tools: None,
        thinking: None,
    };

    let response = claude
        .send_message(&request)
        .await
        .context("summarization API call failed")?;

    Ok(response.text())
}

/// Extract tool names from metadata JSON for inclusion in compaction input.
/// Returns a short suffix like " [used: search_memory, store_fact]" or empty string.
fn extract_tool_names(metadata: &Option<String>) -> String {
    let Some(json) = metadata else {
        return String::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json) else {
        return String::new();
    };
    let Some(calls) = parsed.get("tool_calls").and_then(|v| v.as_array()) else {
        return String::new();
    };
    if calls.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = calls
        .iter()
        .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
        .collect();
    if names.is_empty() {
        return String::new();
    }
    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<&str> = names.into_iter().filter(|n| seen.insert(*n)).collect();
    format!(" [used: {}]", unique.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::test_db;

    #[test]
    fn test_compaction_skips_below_threshold() {
        let db = test_db();
        // Insert fewer than COMPACTION_THRESHOLD messages
        for i in 0..10 {
            db.save_message("mika", "user", &format!("msg {i}"), "cli")
                .unwrap();
        }

        // We can't call maybe_compact in a sync test without a ClaudeClient,
        // but we can verify the threshold check logic
        assert!(db.count_messages("mika").unwrap() <= COMPACTION_THRESHOLD);
    }

    #[test]
    fn test_compaction_identifies_old_messages() {
        let db = test_db();
        // Insert COMPACTION_THRESHOLD + 10 messages
        for i in 0..(COMPACTION_THRESHOLD + 10) {
            db.save_message("mika", "user", &format!("msg {i}"), "cli")
                .unwrap();
        }

        let total = db.count_messages("mika").unwrap();
        assert!(total > COMPACTION_THRESHOLD);

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        // Should have (total - CONTEXT_WINDOW) messages to compact
        assert_eq!(old.len(), total - CONTEXT_WINDOW);
    }

    #[test]
    fn test_replace_with_summary_preserves_recent() {
        let db = test_db();
        for i in 0..60 {
            db.save_message("mika", "user", &format!("msg {i}"), "cli")
                .unwrap();
        }

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        let highest_id = old.last().unwrap().id;

        db.replace_with_summary("mika", "Compacted summary", highest_id)
            .unwrap();

        // Recent messages should still be there
        let recent = db.load_recent_messages("mika", 30, None).unwrap();
        assert_eq!(recent.len(), CONTEXT_WINDOW);
        assert_eq!(recent[0].content, "msg 40");

        // Summary should exist
        let summary = db.load_conversation_summary("mika").unwrap().unwrap();
        assert_eq!(summary.content, "Compacted summary");
    }

    #[test]
    fn test_incremental_compaction() {
        let db = test_db();

        // First round: 60 messages
        for i in 0..60 {
            db.save_message("mika", "user", &format!("batch1 msg {i}"), "cli")
                .unwrap();
        }

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        let highest_id = old.last().unwrap().id;
        db.replace_with_summary("mika", "First summary", highest_id)
            .unwrap();

        // Add more messages to trigger second compaction
        for i in 0..40 {
            db.save_message("mika", "user", &format!("batch2 msg {i}"), "cli")
                .unwrap();
        }

        // Should now have 60 messages (20 kept + 40 new)
        let total = db.count_messages("mika").unwrap();
        assert_eq!(total, 60);

        // Second compaction
        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        assert_eq!(old.len(), 40);
        let highest_id = old.last().unwrap().id;
        db.replace_with_summary("mika", "Merged summary", highest_id)
            .unwrap();

        // After second compaction, only CONTEXT_WINDOW messages remain
        let remaining = db.load_recent_messages("mika", 100, None).unwrap();
        assert_eq!(remaining.len(), CONTEXT_WINDOW);

        // Summary is the latest one
        let summary = db.load_conversation_summary("mika").unwrap().unwrap();
        assert_eq!(summary.content, "Merged summary");
    }

    #[test]
    fn extract_tool_names_none_metadata() {
        assert_eq!(extract_tool_names(&None), "");
    }

    #[test]
    fn extract_tool_names_invalid_json() {
        assert_eq!(extract_tool_names(&Some("not json".to_string())), "");
    }

    #[test]
    fn extract_tool_names_empty_calls() {
        assert_eq!(
            extract_tool_names(&Some(r#"{"tool_calls":[]}"#.to_string())),
            ""
        );
    }

    #[test]
    fn extract_tool_names_single_tool() {
        let meta = r#"{"tool_calls":[{"name":"search_memory","step":0}]}"#;
        assert_eq!(
            extract_tool_names(&Some(meta.to_string())),
            " [used: search_memory]"
        );
    }

    #[test]
    fn extract_tool_names_deduplicates() {
        let meta = r#"{"tool_calls":[{"name":"search_memory","step":0},{"name":"search_memory","step":1},{"name":"store_fact","step":2}]}"#;
        assert_eq!(
            extract_tool_names(&Some(meta.to_string())),
            " [used: search_memory, store_fact]"
        );
    }
}
