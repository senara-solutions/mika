use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, index_fact};

/// Tool-level allowed statuses: only terminal states (not "pending").
/// See also `crate::db::COMMITMENT_STATUSES` for the full DB-level list.
const TOOL_ALLOWED_STATUSES: &[&str] = &["completed", "cancelled"];

pub struct UpdateFactTool;

#[async_trait]
impl Tool for UpdateFactTool {
    fn name(&self) -> &str {
        "update_fact"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_fact".to_string(),
            description: "Update an existing stored fact. Currently supports updating commitment status (e.g. marking as completed or cancelled).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The ID of the fact to update (from search_memory results)"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["commitment"],
                        "description": "Category of fact to update"
                    },
                    "updates": {
                        "type": "object",
                        "description": "Fields to update",
                        "properties": {
                            "status": {
                                "type": "string",
                                "enum": ["completed", "cancelled"],
                                "description": "New status for the commitment"
                            }
                        },
                        "required": ["status"]
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Required in reflection mode: cite a specific conversation timestamp and quote as justification for this change"
                    }
                },
                "required": ["id", "category", "updates"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Reflection mode: require evidence field
        if let Some(err) = super::check_reflection_evidence(ctx, &input) {
            return Ok(err);
        }

        let category = input["category"].as_str().unwrap_or("");
        let id = input["id"].as_i64().unwrap_or(0);

        if category.is_empty() {
            return Ok(ToolOutput::error("'category' is required."));
        }
        if category.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'category' too long: {} characters (max: {MAX_INPUT_LEN})",
                category.len()
            )));
        }
        if id <= 0 {
            return Ok(ToolOutput::error(
                "'id' is required and must be a positive integer.",
            ));
        }

        match category {
            "commitment" => update_commitment(&input, id, ctx).await,
            other => Ok(ToolOutput::error(format!(
                "Invalid category '{other}'. Currently supported: commitment"
            ))),
        }
    }
}

async fn update_commitment(input: &Value, id: i64, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let updates = &input["updates"];
    let status = updates["status"].as_str().unwrap_or("");

    if status.is_empty() {
        return Ok(ToolOutput::error(
            "'updates.status' is required for commitment category.",
        ));
    }

    if status.len() > MAX_INPUT_LEN {
        return Ok(ToolOutput::error(format!(
            "'updates.status' too long: {} characters (max: {MAX_INPUT_LEN})",
            status.len()
        )));
    }

    if !TOOL_ALLOWED_STATUSES.contains(&status) {
        return Ok(ToolOutput::error(format!(
            "Invalid status '{status}'. Allowed: {}",
            TOOL_ALLOWED_STATUSES.join(", ")
        )));
    }

    // Capture before_value for audit log before performing the update
    let before_status = ctx.db.get_commitment_status(id).await?;

    let updated = ctx.db.update_commitment_status(id, status).await?;
    if !updated {
        return Ok(ToolOutput::error(format!(
            "Commitment with id {id} not found."
        )));
    }

    // Log audit event with before_value
    let target = format!("commitment:{id}");
    let after = format!("status -> {status}");
    let reasoning = if ctx.is_reflection {
        input["evidence"]
            .as_str()
            .map(|e| format!("[evidence] {e}"))
    } else {
        None
    };
    ctx.db
        .log_audit_event(
            ctx.session_id,
            "update_fact",
            &target,
            before_status.as_deref(),
            &after,
            reasoning.as_deref(),
            None,
        )
        .await?;

    // Re-index for search with updated status
    if let Ok(Some((desc, due_date))) = ctx.db.get_commitment_details(id).await {
        let mut content = desc;
        if let Some(d) = due_date {
            content.push_str(&format!(" (due: {d})"));
        }
        content.push_str(&format!(", status: {status}"));
        index_fact(ctx, "commitment", id, &content).await;
    }

    Ok(ToolOutput::success(format!(
        "Updated commitment (id:{id}) status to '{status}'."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_update_commitment_completed() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_commitment("Review Q4 budget", Some("2026-03-01"), None)
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": id,
                    "category": "commitment",
                    "updates": { "status": "completed" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("completed"));

        let commitments = harness.db.list_commitments("completed").await.unwrap();
        assert_eq!(commitments.len(), 1);
        assert_eq!(commitments[0].description, "Review Q4 budget");
    }

    #[tokio::test]
    async fn test_update_commitment_cancelled() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_commitment("Cancel this task", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": id,
                    "category": "commitment",
                    "updates": { "status": "cancelled" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let commitments = harness.db.list_commitments("cancelled").await.unwrap();
        assert_eq!(commitments.len(), 1);
    }

    #[tokio::test]
    async fn test_update_commitment_invalid_status() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_commitment("Some task", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": id,
                    "category": "commitment",
                    "updates": { "status": "pending" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid status"));
    }

    #[tokio::test]
    async fn test_reflection_requires_evidence() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_commitment("Test task", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx_with_reflection();
        let tool = UpdateFactTool;

        // Without evidence → rejected
        let result = tool
            .execute(
                serde_json::json!({
                    "id": id,
                    "category": "commitment",
                    "updates": { "status": "completed" }
                }),
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
        let id = harness
            .db
            .add_commitment("Test task", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx_with_reflection();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": id,
                    "category": "commitment",
                    "updates": { "status": "completed" },
                    "evidence": "User said task was done at 15:00"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_update_fact_missing_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "commitment",
                    "updates": { "status": "completed" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));
    }

    #[tokio::test]
    async fn test_update_fact_invalid_category() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": 1,
                    "category": "person",
                    "updates": { "status": "completed" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid category"));
    }

    #[tokio::test]
    async fn test_update_fact_logs_audit() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_commitment("Audit test task", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        tool.execute(
            serde_json::json!({
                "id": id,
                "category": "commitment",
                "updates": { "status": "completed" }
            }),
            &ctx,
        )
        .await
        .unwrap();

        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "update_fact");
        assert!(events[0].target_key.starts_with("commitment:"));
        assert!(events[0].after_value.contains("completed"));
    }

    #[tokio::test]
    async fn test_update_fact_negative_id_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": -5,
                    "category": "commitment",
                    "updates": { "status": "completed" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result
                .content
                .contains("'id' is required and must be a positive integer")
        );
    }

    #[tokio::test]
    async fn test_update_fact_nonexistent_commitment() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "id": 99999,
                    "category": "commitment",
                    "updates": { "status": "completed" }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_update_fact_audit_captures_before_value() {
        let harness = TestHarness::new();
        let id = harness
            .db
            .add_commitment("Before-value test", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = UpdateFactTool;

        tool.execute(
            serde_json::json!({
                "id": id,
                "category": "commitment",
                "updates": { "status": "completed" }
            }),
            &ctx,
        )
        .await
        .unwrap();

        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].before_value.as_deref(),
            Some("pending"),
            "before_value should capture the previous status"
        );
        assert!(events[0].after_value.contains("completed"));
    }
}
