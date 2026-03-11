//! Conversation rewind engine — preview and execute message deletion
//! with automatic reversal of memory/fact mutations via the audit log.

use anyhow::{Result, bail};

use crate::async_db::AsyncDatabase;
use crate::db::{AuditEvent, SessionMessage, Task};

/// Summary of a message that will be deleted.
#[derive(Debug, Clone)]
pub struct MessagePreview {
    pub id: i64,
    pub role: String,
    pub content_snippet: String,
    pub trace_id: Option<String>,
}

/// What action will be taken to reverse an audit event.
#[derive(Debug, Clone, PartialEq)]
pub enum ReversalAction {
    /// Update: restore before_value
    Restore,
    /// Creation: delete the created record
    Delete,
    /// Deletion: re-insert from before_value (not yet needed — no delete tools exist)
    Reinsert,
    /// Cannot reverse (missing data)
    Skip,
}

/// Preview of a single reversal operation.
#[derive(Debug, Clone)]
pub struct ReversalPreview {
    pub audit_event_id: i64,
    pub tool_name: String,
    pub target_key: String,
    pub action: ReversalAction,
    pub description: String,
}

/// Preview of a task that will be affected by the rewind.
#[derive(Debug, Clone)]
pub struct TaskPreview {
    pub task_id: String,
    pub label: String,
    /// true = task is in a terminal/actioned state and will be cancelled instead of deleted
    pub actioned: bool,
}

/// Full preview of a rewind operation.
#[derive(Debug)]
pub struct RewindPreview {
    pub messages: Vec<MessagePreview>,
    pub reversals: Vec<ReversalPreview>,
    pub tasks: Vec<TaskPreview>,
    pub warnings: Vec<String>,
    pub blocked: bool,
    pub block_reason: Option<String>,
}

/// Result of executing a rewind.
#[derive(Debug)]
pub struct RewindResult {
    pub messages_deleted: usize,
    pub reversals_applied: usize,
    pub reversals_skipped: usize,
    pub tasks_deleted: usize,
    pub tasks_cancelled: usize,
    pub warnings: Vec<String>,
    pub rewind_trace_id: String,
}

/// Find the N most recent exchanges (by distinct user-role trace_ids) in the session,
/// then collect ALL messages that share those trace_ids.
/// Returns `(anchor_message_id, trace_ids)` — the anchor is the message just before
/// the rewind range (i.e., rewind deletes everything after anchor).
pub async fn find_recent_exchanges(
    db: &AsyncDatabase,
    session_id: &str,
    count: usize,
) -> Result<Option<(i64, Vec<String>)>> {
    // Get all messages in this session, ordered newest first
    let messages = db.get_messages_after_id(session_id, 0).await?;
    if messages.is_empty() {
        return Ok(None);
    }

    // Collect distinct trace_ids from user-role messages, newest first
    let mut user_trace_ids: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for msg in messages.iter().rev() {
        if msg.role == "user"
            && let Some(ref tid) = msg.trace_id
            && seen.insert(tid.clone())
        {
            user_trace_ids.push(tid.clone());
            if user_trace_ids.len() >= count {
                break;
            }
        }
    }

    if user_trace_ids.is_empty() {
        return Ok(None);
    }

    // Find the earliest message with any of these trace_ids
    let trace_set: std::collections::HashSet<&str> =
        user_trace_ids.iter().map(|s| s.as_str()).collect();
    let mut earliest_id = i64::MAX;
    for msg in &messages {
        if let Some(ref tid) = msg.trace_id
            && trace_set.contains(tid.as_str())
            && msg.id < earliest_id
        {
            earliest_id = msg.id;
        }
    }

    // Anchor = the message just before the earliest
    let anchor_id = earliest_id - 1;

    Ok(Some((anchor_id, user_trace_ids)))
}

/// Preview a rewind operation without executing it.
pub async fn preview_rewind(
    db: &AsyncDatabase,
    session_id: &str,
    after_message_id: i64,
) -> Result<RewindPreview> {
    let mut warnings = Vec::new();
    let mut blocked = false;
    let mut block_reason = None;

    // Boundary check: compaction
    if let Some(boundary) = db.get_compaction_boundary().await?
        && after_message_id <= boundary
    {
        return Ok(RewindPreview {
            messages: vec![],
            reversals: vec![],
            tasks: vec![],
            warnings: vec![],
            blocked: true,
            block_reason: Some(format!(
                "Cannot rewind past compaction boundary (message {after_message_id} \
                 has been compacted through message {boundary})."
            )),
        });
    }

    // Get messages to delete
    let messages = db
        .get_messages_after_id(session_id, after_message_id)
        .await?;
    if messages.is_empty() {
        return Ok(RewindPreview {
            messages: vec![],
            reversals: vec![],
            tasks: vec![],
            warnings: vec![],
            blocked: true,
            block_reason: Some("No messages found after the specified message ID.".to_string()),
        });
    }

    // Check for NULL trace_ids
    let null_trace_count = messages.iter().filter(|m| m.trace_id.is_none()).count();
    if null_trace_count > 0 {
        blocked = true;
        block_reason = Some(format!(
            "Cannot rewind: {null_trace_count} message(s) in the range have no trace_id \
             (pre-migration messages). These cannot be grouped or reversed."
        ));
        // Still build the preview for display purposes
    }

    // Collect unique trace_ids
    let trace_ids: Vec<String> = messages
        .iter()
        .filter_map(|m| m.trace_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Get audit events for those trace_ids
    let audit_events = if !trace_ids.is_empty() {
        db.get_audit_events_by_trace_ids(trace_ids.clone()).await?
    } else {
        vec![]
    };

    // Build message previews
    let message_previews: Vec<MessagePreview> = messages
        .iter()
        .map(|m| MessagePreview {
            id: m.id,
            role: m.role.clone(),
            content_snippet: truncate_content(&m.content, 100),
            trace_id: m.trace_id.clone(),
        })
        .collect();

    // Build reversal previews
    let reversals = build_reversal_previews(&audit_events);

    // Check for irreversible side effects
    check_irreversible_effects(&audit_events, &messages, &mut warnings);

    // Get tasks created during these turns
    let tasks = if !trace_ids.is_empty() {
        db.get_tasks_by_trace_ids(&trace_ids).await?
    } else {
        vec![]
    };
    let task_previews = build_task_previews(&tasks, db).await?;

    // Warn about actioned tasks
    for tp in &task_previews {
        if tp.actioned {
            warnings.push(format!(
                "Task '{}' ({}) has been actioned — will be cancelled instead of deleted.",
                tp.label, tp.task_id
            ));
        }
    }

    // Warn if audit events are missing for some trace_ids
    let audited_traces: std::collections::HashSet<&str> = audit_events
        .iter()
        .filter_map(|e| e.trace_id.as_deref())
        .collect();
    let unaudited: Vec<&str> = trace_ids
        .iter()
        .filter(|t| !audited_traces.contains(t.as_str()))
        .map(|s| s.as_str())
        .collect();
    if !unaudited.is_empty() {
        warnings.push(format!(
            "No audit events found for {} trace(s). Memory changes from those turns \
             (if any) cannot be reversed.",
            unaudited.len()
        ));
    }

    Ok(RewindPreview {
        messages: message_previews,
        reversals,
        tasks: task_previews,
        warnings,
        blocked,
        block_reason,
    })
}

/// Execute a rewind: reverse memory mutations, delete messages, clean up tasks.
/// Returns an error if the rewind is blocked.
pub async fn execute_rewind(
    db: &AsyncDatabase,
    session_id: &str,
    after_message_id: i64,
) -> Result<RewindResult> {
    // Preview first to check for blockers
    let preview = preview_rewind(db, session_id, after_message_id).await?;
    if preview.blocked {
        bail!(
            "Rewind blocked: {}",
            preview.block_reason.unwrap_or_default()
        );
    }

    let rewind_trace_id = mika_common::trace::generate_trace_id();
    let mut warnings = preview.warnings;

    // Collect trace_ids from messages being deleted
    let trace_ids: Vec<String> = preview
        .messages
        .iter()
        .filter_map(|m| m.trace_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Get fresh audit events (ordered by id DESC — reverse chronological)
    let audit_events = if !trace_ids.is_empty() {
        db.get_audit_events_by_trace_ids(trace_ids.clone()).await?
    } else {
        vec![]
    };

    // Apply reversals in reverse chronological order (already ordered by id DESC)
    let mut applied = 0usize;
    let mut skipped = 0usize;
    for evt in &audit_events {
        match apply_reversal(db, evt).await {
            Ok(true) => applied += 1,
            Ok(false) => {
                skipped += 1;
                warnings.push(format!(
                    "Skipped reversal for {} target '{}' — target not found or unchanged.",
                    evt.tool_name, evt.target_key
                ));
            }
            Err(e) => {
                skipped += 1;
                warnings.push(format!(
                    "Error reversing {} target '{}': {e}",
                    evt.tool_name, evt.target_key
                ));
            }
        }
    }

    // Handle tasks created during rewound turns
    let tasks = if !trace_ids.is_empty() {
        db.get_tasks_by_trace_ids(&trace_ids).await?
    } else {
        vec![]
    };
    let mut tasks_deleted = 0usize;
    let mut tasks_cancelled = 0usize;
    for task in &tasks {
        if is_task_actioned(task, db).await? {
            if db.cancel_task(&task.id).await? {
                tasks_cancelled += 1;
            }
        } else if db.delete_task_by_id(&task.id).await? {
            tasks_deleted += 1;
        }
    }

    // Delete messages
    let messages_deleted = db
        .delete_messages_after_id(session_id, after_message_id)
        .await?;

    // Mark audit events as rewound
    if !trace_ids.is_empty() {
        db.mark_audit_events_rewound(trace_ids, &rewind_trace_id)
            .await?;
    }

    // Log the rewind itself as an audit event
    let target = format!("rewind:after_msg_{after_message_id}");
    let after_val = format!(
        "deleted {messages_deleted} messages, reversed {applied} mutations, \
         {tasks_deleted} tasks deleted, {tasks_cancelled} tasks cancelled"
    );
    db.log_audit_event(
        session_id,
        "rewind",
        &target,
        None,
        Some(&after_val),
        None,
        Some(&rewind_trace_id),
    )
    .await?;

    Ok(RewindResult {
        messages_deleted,
        reversals_applied: applied,
        reversals_skipped: skipped,
        tasks_deleted,
        tasks_cancelled,
        warnings,
        rewind_trace_id,
    })
}

/// Apply a single reversal based on the audit event's tool_name, target_key, and before_value.
/// Returns Ok(true) if applied, Ok(false) if skipped (target missing).
async fn apply_reversal(db: &AsyncDatabase, evt: &AuditEvent) -> Result<bool> {
    match (
        evt.before_value.as_deref(),
        evt.after_value.as_deref(),
        evt.tool_name.as_str(),
    ) {
        // Update: restore before_value
        (Some(before), Some(_), "update_core_memory") => {
            // target_key is the section name
            db.set_core_memory(&evt.target_key, before).await?;
            Ok(true)
        }

        // store_fact creation: delete the created record (before_value is None)
        (None, Some(_), "store_fact") => apply_store_fact_deletion(db, &evt.target_key).await,

        // store_fact update: restore before_value
        (Some(before), Some(_), "store_fact") => {
            apply_store_fact_restore(db, &evt.target_key, before).await
        }

        // update_fact (commitment status change): restore previous status
        (Some(before_status), Some(_), "update_fact") => {
            if let Some(id) = parse_id_from_target_key(&evt.target_key, "commitment:") {
                db.update_commitment_status(id, before_status).await?;
                Ok(true)
            } else {
                Ok(false)
            }
        }

        // Task creation: handled separately in execute_rewind
        (_, _, "create_work_item" | "create_reminder") => Ok(true),

        // update_work_item_status: restore previous status
        (Some(before_status), Some(_), "update_work_item_status") => {
            // target_key format is "task:{id}" or "work_item:{id}"
            if let Some(task_id) = parse_string_from_target_key(&evt.target_key) {
                db.update_manual_task_status(&task_id, before_status)
                    .await?;
                Ok(true)
            } else {
                Ok(false)
            }
        }

        // No before or after: nothing to reverse
        (None, None, _) => Ok(true),

        // Unrecognized tool: skip
        _ => Ok(false),
    }
}

/// Delete a record created by store_fact, based on target_key pattern.
async fn apply_store_fact_deletion(db: &AsyncDatabase, target_key: &str) -> Result<bool> {
    if let Some(name) = target_key.strip_prefix("person:") {
        Ok(db.delete_person_by_name(name).await?)
    } else if let Some(key) = target_key.strip_prefix("preference:") {
        Ok(db.delete_preference(key).await?)
    } else if let Some(desc) = target_key.strip_prefix("commitment:") {
        Ok(db.delete_commitment_by_description(desc).await?)
    } else if let Some(desc) = target_key.strip_prefix("event:") {
        Ok(db.delete_event_by_description(desc).await?)
    } else {
        Ok(false)
    }
}

/// Restore a record modified by store_fact to its before_value.
async fn apply_store_fact_restore(
    db: &AsyncDatabase,
    target_key: &str,
    before: &str,
) -> Result<bool> {
    if target_key.starts_with("person:") {
        // before is JSON: {"name": ..., "relationship": ..., "notes": ...}
        // Restore the person by re-upserting from the JSON fields
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(before) {
            let name = json["name"].as_str().unwrap_or("");
            let relationship = json["relationship"].as_str();
            let notes = json["notes"].as_str();
            if !name.is_empty() {
                db.upsert_person(name, relationship, notes).await?;
                return Ok(true);
            }
        }
        Ok(false)
    } else if let Some(key) = target_key.strip_prefix("preference:") {
        // before is the preference value
        db.set_preference(key, before).await?;
        Ok(true)
    } else {
        // Commitment/event updates via store_fact aren't common;
        // those use update_fact instead. Skip gracefully.
        Ok(false)
    }
}

/// Parse a numeric ID from a target_key like "commitment:42"
fn parse_id_from_target_key(target_key: &str, prefix: &str) -> Option<i64> {
    target_key.strip_prefix(prefix)?.parse().ok()
}

/// Parse a string ID from a target_key like "task:uuid" or "work_item:uuid"
fn parse_string_from_target_key(target_key: &str) -> Option<String> {
    target_key
        .strip_prefix("task:")
        .or_else(|| target_key.strip_prefix("work_item:"))
        .map(|id| id.to_string())
}

/// Check if a task has been actioned (terminal state or has children).
async fn is_task_actioned(task: &Task, db: &AsyncDatabase) -> Result<bool> {
    let terminal = matches!(
        task.status.as_str(),
        "completed" | "delivered" | "failed" | "expired"
    );
    if terminal {
        return Ok(true);
    }
    // Check for child tasks
    let children = db.get_child_tasks(&task.id).await?;
    Ok(!children.is_empty())
}

fn build_reversal_previews(audit_events: &[AuditEvent]) -> Vec<ReversalPreview> {
    audit_events
        .iter()
        .map(|evt| {
            let (action, description) = describe_reversal(evt);
            ReversalPreview {
                audit_event_id: evt.id,
                tool_name: evt.tool_name.clone(),
                target_key: evt.target_key.clone(),
                action,
                description,
            }
        })
        .collect()
}

fn describe_reversal(evt: &AuditEvent) -> (ReversalAction, String) {
    match (
        evt.before_value.as_deref(),
        evt.after_value.as_deref(),
        evt.tool_name.as_str(),
    ) {
        (Some(before), Some(_), "update_core_memory") => (
            ReversalAction::Restore,
            format!(
                "Restore core_memory '{}' to: {}",
                evt.target_key,
                truncate_content(before, 60)
            ),
        ),

        (None, Some(_), "store_fact") => (
            ReversalAction::Delete,
            format!("Delete {} (was created)", evt.target_key),
        ),

        (Some(before), Some(_), "store_fact") => (
            ReversalAction::Restore,
            format!(
                "Restore {} to previous value: {}",
                evt.target_key,
                truncate_content(before, 60)
            ),
        ),

        (Some(before), Some(_), "update_fact") => (
            ReversalAction::Restore,
            format!("Restore {} status to '{before}'", evt.target_key),
        ),

        (Some(before), Some(_), "update_work_item_status") => (
            ReversalAction::Restore,
            format!("Restore {} status to '{before}'", evt.target_key),
        ),

        (_, _, "create_work_item" | "create_reminder") => (
            ReversalAction::Delete,
            format!("Delete task from {}", evt.target_key),
        ),

        (None, None, _) => (
            ReversalAction::Skip,
            "No-op (no values recorded)".to_string(),
        ),

        _ => (
            ReversalAction::Skip,
            format!("Cannot reverse '{}' on '{}'", evt.tool_name, evt.target_key),
        ),
    }
}

async fn build_task_previews(tasks: &[Task], db: &AsyncDatabase) -> Result<Vec<TaskPreview>> {
    let mut previews = Vec::new();
    for task in tasks {
        let actioned = is_task_actioned(task, db).await?;
        previews.push(TaskPreview {
            task_id: task.id.clone(),
            label: task.label.clone(),
            actioned,
        });
    }
    Ok(previews)
}

fn check_irreversible_effects(
    audit_events: &[AuditEvent],
    messages: &[SessionMessage],
    warnings: &mut Vec<String>,
) {
    // Check for send_message in audit events
    if audit_events.iter().any(|e| e.tool_name == "send_message") {
        warnings.push(
            "A message was sent externally (e.g., via Telegram) during these turns. \
             It cannot be unsent."
                .to_string(),
        );
    }

    // Check for file writes
    if audit_events
        .iter()
        .any(|e| e.tool_name == "write_file" || e.tool_name == "write_workspace")
    {
        warnings
            .push("Files were written during these turns. They will not be reverted.".to_string());
    }

    // Check for exec handler tools in message metadata
    let has_exec = messages.iter().any(|m| {
        m.metadata
            .as_deref()
            .map(|md| md.contains("\"handler\":\"exec\"") || md.contains("skill:"))
            .unwrap_or(false)
    });
    if has_exec {
        warnings.push(
            "External skill scripts were executed during these turns. \
             Their side effects cannot be reversed."
                .to_string(),
        );
    }
}

fn truncate_content(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{truncated}...")
    }
}

/// Format a RewindPreview for display in TUI/dashboard.
pub fn format_preview(preview: &RewindPreview) -> String {
    let mut lines = Vec::new();

    if preview.blocked {
        if let Some(ref reason) = preview.block_reason {
            lines.push(format!("BLOCKED: {reason}"));
        }
        return lines.join("\n");
    }

    lines.push(format!("Rewinding {} message(s):", preview.messages.len()));

    if !preview.reversals.is_empty() {
        lines.push(String::new());
        lines.push("Memory changes to reverse:".to_string());
        for rev in &preview.reversals {
            let icon = match rev.action {
                ReversalAction::Restore => "↩",
                ReversalAction::Delete => "🗑",
                ReversalAction::Reinsert => "➕",
                ReversalAction::Skip => "⏭",
            };
            lines.push(format!("  {icon} {}", rev.description));
        }
    }

    if !preview.tasks.is_empty() {
        lines.push(String::new());
        lines.push("Tasks affected:".to_string());
        for tp in &preview.tasks {
            if tp.actioned {
                lines.push(format!("  ⚠ Cancel '{}' (already actioned)", tp.label));
            } else {
                lines.push(format!("  🗑 Delete '{}'", tp.label));
            }
        }
    }

    if !preview.warnings.is_empty() {
        lines.push(String::new());
        lines.push("Warnings:".to_string());
        for w in &preview.warnings {
            lines.push(format!("  ⚠ {w}"));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_preview_empty_session() {
        let harness = TestHarness::new();
        let preview = preview_rewind(&harness.db, "test-session", 0)
            .await
            .unwrap();
        assert!(preview.blocked);
        assert!(preview.block_reason.unwrap().contains("No messages found"));
    }

    #[tokio::test]
    async fn test_rewind_core_memory_change() {
        let harness = TestHarness::new();
        let db = &harness.db;

        // Set initial core memory
        db.set_core_memory("user_summary", "Original value")
            .await
            .unwrap();

        // Save a message with a trace_id
        let trace_id = "test-trace-001";
        db.save_message("test-session", "user", "Hello", Some(trace_id))
            .await
            .unwrap();

        // Agent changes core memory and logs audit
        db.set_core_memory("user_summary", "Updated value")
            .await
            .unwrap();
        db.log_audit_event(
            "test-session",
            "update_core_memory",
            "user_summary",
            Some("Original value"),
            Some("Updated value"),
            None,
            Some(trace_id),
        )
        .await
        .unwrap();

        db.save_message("test-session", "assistant", "Done", Some(trace_id))
            .await
            .unwrap();

        // Preview: should show the reversal
        let first_msg = db.get_messages_after_id("test-session", 0).await.unwrap();
        let anchor_id = first_msg[0].id - 1; // before the user message

        let preview = preview_rewind(db, "test-session", anchor_id).await.unwrap();
        assert!(!preview.blocked);
        assert_eq!(preview.messages.len(), 2);
        assert_eq!(preview.reversals.len(), 1);
        assert_eq!(preview.reversals[0].action, ReversalAction::Restore);

        // Execute
        let result = execute_rewind(db, "test-session", anchor_id).await.unwrap();
        assert_eq!(result.messages_deleted, 2);
        assert_eq!(result.reversals_applied, 1);

        // Verify core memory was restored
        let entry = db.get_core_memory("user_summary").await.unwrap();
        assert_eq!(entry.map(|e| e.value), Some("Original value".to_string()));
    }

    #[tokio::test]
    async fn test_rewind_store_fact_person_creation() {
        let harness = TestHarness::new();
        let db = &harness.db;

        let trace_id = "test-trace-002";
        db.save_message("test-session", "user", "Remember Sarah", Some(trace_id))
            .await
            .unwrap();

        // Agent creates a person
        db.upsert_person("Sarah", Some("friend"), Some("Met at conference"))
            .await
            .unwrap();
        db.log_audit_event(
            "test-session",
            "store_fact",
            "person:Sarah",
            None,
            Some("Sarah (friend): Met at conference"),
            None,
            Some(trace_id),
        )
        .await
        .unwrap();

        db.save_message("test-session", "assistant", "Noted Sarah", Some(trace_id))
            .await
            .unwrap();

        let messages = db.get_messages_after_id("test-session", 0).await.unwrap();
        let anchor_id = messages[0].id - 1;

        // Preview
        let preview = preview_rewind(db, "test-session", anchor_id).await.unwrap();
        assert!(!preview.blocked);
        assert_eq!(preview.reversals.len(), 1);
        assert_eq!(preview.reversals[0].action, ReversalAction::Delete);

        // Execute
        let result = execute_rewind(db, "test-session", anchor_id).await.unwrap();
        assert_eq!(result.reversals_applied, 1);

        // Verify person was deleted
        let person = db.get_person("Sarah").await.unwrap();
        assert!(person.is_none());
    }

    #[tokio::test]
    async fn test_rewind_multiple_exchanges() {
        let harness = TestHarness::new();
        let db = &harness.db;

        // Exchange 1
        let trace1 = "trace-ex1";
        db.save_message("test-session", "user", "Set priority", Some(trace1))
            .await
            .unwrap();
        db.set_core_memory("current_priorities", "Priority A")
            .await
            .unwrap();
        db.log_audit_event(
            "test-session",
            "update_core_memory",
            "current_priorities",
            Some("No priorities yet."),
            Some("Priority A"),
            None,
            Some(trace1),
        )
        .await
        .unwrap();
        db.save_message("test-session", "assistant", "Set to A", Some(trace1))
            .await
            .unwrap();

        // Exchange 2
        let trace2 = "trace-ex2";
        db.save_message("test-session", "user", "Update priority", Some(trace2))
            .await
            .unwrap();
        db.set_core_memory("current_priorities", "Priority B")
            .await
            .unwrap();
        db.log_audit_event(
            "test-session",
            "update_core_memory",
            "current_priorities",
            Some("Priority A"),
            Some("Priority B"),
            None,
            Some(trace2),
        )
        .await
        .unwrap();
        db.save_message("test-session", "assistant", "Set to B", Some(trace2))
            .await
            .unwrap();

        // Rewind both exchanges: anchor before exchange 1
        let messages = db.get_messages_after_id("test-session", 0).await.unwrap();
        let anchor_id = messages[0].id - 1;

        let result = execute_rewind(db, "test-session", anchor_id).await.unwrap();
        assert_eq!(result.messages_deleted, 4);
        assert_eq!(result.reversals_applied, 2);

        // Core memory should be back to original
        let entry = db.get_core_memory("current_priorities").await.unwrap();
        assert_eq!(
            entry.map(|e| e.value),
            Some("No priorities yet.".to_string())
        );
    }

    #[tokio::test]
    async fn test_rewind_blocked_past_compaction() {
        let harness = TestHarness::new();
        let db = &harness.db;

        // Simulate compaction: save several messages and then compact
        for i in 0..5 {
            db.save_message(
                "test-session",
                if i % 2 == 0 { "user" } else { "assistant" },
                &format!("Message {i}"),
                Some(&format!("trace-{i}")),
            )
            .await
            .unwrap();
        }

        // Do a compaction
        let messages = db.get_messages_after_id("test-session", 0).await.unwrap();
        let through_id = messages[2].id; // compact through message 3
        db.replace_with_summary("Summary of first 3 messages", through_id)
            .await
            .unwrap();

        // Try to rewind to before compaction boundary
        let preview = preview_rewind(db, "test-session", 0).await.unwrap();
        assert!(preview.blocked);
        assert!(preview.block_reason.unwrap().contains("compaction"));
    }

    #[tokio::test]
    async fn test_rewind_logs_audit_event() {
        let harness = TestHarness::new();
        let db = &harness.db;

        let trace_id = "test-trace-audit";
        db.save_message("test-session", "user", "Test", Some(trace_id))
            .await
            .unwrap();
        db.save_message("test-session", "assistant", "Reply", Some(trace_id))
            .await
            .unwrap();

        let messages = db.get_messages_after_id("test-session", 0).await.unwrap();
        let anchor_id = messages[0].id - 1;

        let result = execute_rewind(db, "test-session", anchor_id).await.unwrap();

        // Check that the rewind itself was logged
        let events = db.get_audit_events("test-session").await.unwrap();
        let rewind_event = events.iter().find(|e| e.tool_name == "rewind");
        assert!(rewind_event.is_some());
        let re = rewind_event.unwrap();
        assert!(re.target_key.starts_with("rewind:"));
        assert_eq!(
            re.trace_id.as_deref(),
            Some(result.rewind_trace_id.as_str())
        );
    }

    #[tokio::test]
    async fn test_rewind_marks_audit_events_rewound() {
        let harness = TestHarness::new();
        let db = &harness.db;

        let trace_id = "test-trace-mark";
        db.save_message("test-session", "user", "Test", Some(trace_id))
            .await
            .unwrap();
        db.set_core_memory("user_summary", "Changed").await.unwrap();
        db.log_audit_event(
            "test-session",
            "update_core_memory",
            "user_summary",
            Some("Original"),
            Some("Changed"),
            None,
            Some(trace_id),
        )
        .await
        .unwrap();
        db.save_message("test-session", "assistant", "Done", Some(trace_id))
            .await
            .unwrap();

        let messages = db.get_messages_after_id("test-session", 0).await.unwrap();
        let anchor_id = messages[0].id - 1;

        let result = execute_rewind(db, "test-session", anchor_id).await.unwrap();

        // The original audit event should be marked as rewound
        let all_events = db.get_audit_events("test-session").await.unwrap();
        let rewound = all_events
            .iter()
            .find(|e| e.tool_name == "update_core_memory");
        assert!(rewound.is_some());
        let re = rewound.unwrap();
        assert_eq!(
            re.rewound_by_trace_id.as_deref(),
            Some(result.rewind_trace_id.as_str())
        );
    }

    #[tokio::test]
    async fn test_find_recent_exchanges() {
        let harness = TestHarness::new();
        let db = &harness.db;

        // Create 3 exchanges
        for i in 1..=3 {
            let trace = format!("trace-{i}");
            db.save_message(
                "test-session",
                "user",
                &format!("User msg {i}"),
                Some(&trace),
            )
            .await
            .unwrap();
            db.save_message(
                "test-session",
                "assistant",
                &format!("Reply {i}"),
                Some(&trace),
            )
            .await
            .unwrap();
        }

        // Find last 2 exchanges
        let result = find_recent_exchanges(db, "test-session", 2).await.unwrap();
        assert!(result.is_some());
        let (anchor_id, trace_ids) = result.unwrap();
        assert_eq!(trace_ids.len(), 2);
        assert!(trace_ids.contains(&"trace-3".to_string()));
        assert!(trace_ids.contains(&"trace-2".to_string()));

        // Messages after anchor should be the last 2 exchanges (4 messages)
        let msgs = db
            .get_messages_after_id("test-session", anchor_id)
            .await
            .unwrap();
        assert_eq!(msgs.len(), 4);
    }

    #[test]
    fn test_truncate_content() {
        assert_eq!(truncate_content("short", 10), "short");
        assert_eq!(truncate_content("a long string here", 6), "a long...");
    }

    #[test]
    fn test_parse_id_from_target_key() {
        assert_eq!(
            parse_id_from_target_key("commitment:42", "commitment:"),
            Some(42)
        );
        assert_eq!(parse_id_from_target_key("person:John", "commitment:"), None);
        assert_eq!(
            parse_id_from_target_key("commitment:abc", "commitment:"),
            None
        );
    }

    #[test]
    fn test_parse_string_from_target_key() {
        assert_eq!(
            parse_string_from_target_key("task:abc-123"),
            Some("abc-123".to_string())
        );
        assert_eq!(
            parse_string_from_target_key("work_item:xyz"),
            Some("xyz".to_string())
        );
        assert_eq!(parse_string_from_target_key("person:John"), None);
    }

    #[test]
    fn test_format_preview_blocked() {
        let preview = RewindPreview {
            messages: vec![],
            reversals: vec![],
            tasks: vec![],
            warnings: vec![],
            blocked: true,
            block_reason: Some("Test block reason".to_string()),
        };
        let output = format_preview(&preview);
        assert!(output.contains("BLOCKED: Test block reason"));
    }
}
