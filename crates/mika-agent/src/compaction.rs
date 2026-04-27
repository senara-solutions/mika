use anyhow::{Context, Result};
use mika_common::llm::{LlmContent, LlmMessage, LlmProvider, LlmRequest, LlmRole};
use tracing::{debug, info, warn};

use crate::async_db::AsyncDatabase;
use crate::db::SessionMessage;

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
pub async fn maybe_compact(db: &AsyncDatabase, llm: &dyn LlmProvider) -> Result<()> {
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

    let mut summary_text = summarize_messages(llm, batch, existing_summary.as_ref()).await?;

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
    llm: &dyn LlmProvider,
    messages: &[SessionMessage],
    existing_summary: Option<&SessionMessage>,
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

    let request = LlmRequest {
        model: llm.model_name().to_string(),
        max_tokens: 1024,
        system: Some(SUMMARIZATION_SYSTEM_PROMPT.to_string()),
        messages: vec![LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(user_prompt),
        }],
        tools: None,
        thinking: None,
    };

    let response = llm
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
    let entries: Vec<String> = calls
        .iter()
        .filter_map(|c| {
            let name = c.get("name").and_then(|n| n.as_str())?;
            let ok = c.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
            Some(if ok {
                name.to_string()
            } else {
                format!("{name}(err)")
            })
        })
        .collect();
    if entries.is_empty() {
        return String::new();
    }
    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = entries
        .into_iter()
        .filter(|n| seen.insert(n.clone()))
        .collect();
    format!(" [used: {}]", unique.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::test_db;

    fn test_db_with_session() -> (crate::db::Database, String) {
        let db = test_db();
        let sid = "test-session".to_string();
        db.create_session(&sid, "mika", "cli").unwrap();
        (db, sid)
    }

    #[test]
    fn test_compaction_skips_below_threshold() {
        let (db, sid) = test_db_with_session();
        for i in 0..10 {
            db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
                .unwrap();
        }
        assert!(db.count_messages("mika").unwrap() <= COMPACTION_THRESHOLD);
    }

    #[test]
    fn test_compaction_identifies_old_messages() {
        let (db, sid) = test_db_with_session();
        for i in 0..(COMPACTION_THRESHOLD + 10) {
            db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
                .unwrap();
        }

        let total = db.count_messages("mika").unwrap();
        assert!(total > COMPACTION_THRESHOLD);

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        assert_eq!(old.len(), total - CONTEXT_WINDOW);
    }

    #[test]
    fn test_replace_with_summary_preserves_recent() {
        let (mut db, sid) = test_db_with_session();
        for i in 0..60 {
            db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
                .unwrap();
        }

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        let highest_id = old.last().unwrap().id;

        db.replace_with_summary("mika", "Compacted summary", highest_id)
            .unwrap();

        let recent = db.load_recent_messages("mika", 30).unwrap();
        assert_eq!(recent.len(), CONTEXT_WINDOW);
        assert_eq!(recent[0].content, "msg 40");

        let summary = db.load_conversation_summary("mika").unwrap().unwrap();
        assert_eq!(summary.content, "Compacted summary");
    }

    #[test]
    fn test_incremental_compaction() {
        let (mut db, sid) = test_db_with_session();

        for i in 0..60 {
            db.save_message("mika", &sid, "user", &format!("batch1 msg {i}"), None)
                .unwrap();
        }

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        let highest_id = old.last().unwrap().id;
        db.replace_with_summary("mika", "First summary", highest_id)
            .unwrap();

        for i in 0..40 {
            db.save_message("mika", &sid, "user", &format!("batch2 msg {i}"), None)
                .unwrap();
        }

        let total = db.count_messages("mika").unwrap();
        assert_eq!(total, 60);

        let old = db
            .load_messages_before_window("mika", CONTEXT_WINDOW)
            .unwrap();
        assert_eq!(old.len(), 40);
        let highest_id = old.last().unwrap().id;
        db.replace_with_summary("mika", "Merged summary", highest_id)
            .unwrap();

        let remaining = db.load_recent_messages("mika", 100).unwrap();
        assert_eq!(remaining.len(), CONTEXT_WINDOW);

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

    #[test]
    fn extract_tool_names_shows_failure() {
        let meta = r#"{"tool_calls":[{"name":"write_agent_file","step":0,"success":true},{"name":"read_agent_file","step":1,"success":false}]}"#;
        assert_eq!(
            extract_tool_names(&Some(meta.to_string())),
            " [used: write_agent_file, read_agent_file(err)]"
        );
    }

    #[test]
    fn extract_tool_names_deduplicates_with_failure() {
        let meta = r#"{"tool_calls":[{"name":"search_memory","step":0,"success":true},{"name":"search_memory","step":1,"success":false}]}"#;
        // First occurrence wins in dedup
        assert_eq!(
            extract_tool_names(&Some(meta.to_string())),
            " [used: search_memory, search_memory(err)]"
        );
    }
}
