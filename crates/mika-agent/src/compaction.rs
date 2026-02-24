use anyhow::{Context, Result};
use mika_common::claude::{ClaudeClient, Message, MessageContent, MessagesRequest};
use tracing::{debug, info, warn};

use crate::db::{ConversationMessage, Database};

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
pub async fn maybe_compact(db: &Database, claude: &ClaudeClient) -> Result<()> {
    let total = db.count_messages()?;
    if total <= COMPACTION_THRESHOLD {
        debug!(
            total,
            threshold = COMPACTION_THRESHOLD,
            "compaction not needed"
        );
        return Ok(());
    }

    let existing_summary = db.load_conversation_summary()?;
    let old_messages = db.load_messages_before_window(CONTEXT_WINDOW)?;
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

    info!(
        old_count = batch.len(),
        total, "compacting conversation"
    );

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
    db.replace_with_summary(&summary_text, highest_id)?;

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
        let msg_chars = msg.role.len() + 2 + msg.content.len() + 1; // "role: content\n"
        if char_count + msg_chars > MAX_COMPACTION_INPUT_CHARS {
            break;
        }
        char_count += msg_chars;
        included += 1;
        user_prompt.push_str(&msg.role);
        user_prompt.push_str(": ");
        user_prompt.push_str(&msg.content);
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
    };

    let response = claude
        .send_message(&request)
        .await
        .context("summarization API call failed")?;

    Ok(response.text())
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
            db.save_message("user", &format!("msg {i}"), "cli").unwrap();
        }

        // We can't call maybe_compact in a sync test without a ClaudeClient,
        // but we can verify the threshold check logic
        assert!(db.count_messages().unwrap() <= COMPACTION_THRESHOLD);
    }

    #[test]
    fn test_compaction_identifies_old_messages() {
        let db = test_db();
        // Insert COMPACTION_THRESHOLD + 10 messages
        for i in 0..(COMPACTION_THRESHOLD + 10) {
            db.save_message("user", &format!("msg {i}"), "cli").unwrap();
        }

        let total = db.count_messages().unwrap();
        assert!(total > COMPACTION_THRESHOLD);

        let old = db.load_messages_before_window(CONTEXT_WINDOW).unwrap();
        // Should have (total - CONTEXT_WINDOW) messages to compact
        assert_eq!(old.len(), total - CONTEXT_WINDOW);
    }

    #[test]
    fn test_replace_with_summary_preserves_recent() {
        let db = test_db();
        for i in 0..60 {
            db.save_message("user", &format!("msg {i}"), "cli").unwrap();
        }

        let old = db.load_messages_before_window(CONTEXT_WINDOW).unwrap();
        let highest_id = old.last().unwrap().id;

        db.replace_with_summary("Compacted summary", highest_id)
            .unwrap();

        // Recent messages should still be there
        let recent = db.load_recent_messages(30, None).unwrap();
        assert_eq!(recent.len(), CONTEXT_WINDOW);
        assert_eq!(recent[0].content, "msg 40");

        // Summary should exist
        let summary = db.load_conversation_summary().unwrap().unwrap();
        assert_eq!(summary.content, "Compacted summary");
    }

    #[test]
    fn test_incremental_compaction() {
        let db = test_db();

        // First round: 60 messages
        for i in 0..60 {
            db.save_message("user", &format!("batch1 msg {i}"), "cli")
                .unwrap();
        }

        let old = db.load_messages_before_window(CONTEXT_WINDOW).unwrap();
        let highest_id = old.last().unwrap().id;
        db.replace_with_summary("First summary", highest_id)
            .unwrap();

        // Add more messages to trigger second compaction
        for i in 0..40 {
            db.save_message("user", &format!("batch2 msg {i}"), "cli")
                .unwrap();
        }

        // Should now have 60 messages (20 kept + 40 new)
        let total = db.count_messages().unwrap();
        assert_eq!(total, 60);

        // Second compaction
        let old = db.load_messages_before_window(CONTEXT_WINDOW).unwrap();
        assert_eq!(old.len(), 40);
        let highest_id = old.last().unwrap().id;
        db.replace_with_summary("Merged summary", highest_id)
            .unwrap();

        // After second compaction, only CONTEXT_WINDOW messages remain
        let remaining = db.load_recent_messages(100, None).unwrap();
        assert_eq!(remaining.len(), CONTEXT_WINDOW);

        // Summary is the latest one
        let summary = db.load_conversation_summary().unwrap().unwrap();
        assert_eq!(summary.content, "Merged summary");
    }
}
