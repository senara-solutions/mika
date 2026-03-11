use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput, index_fact};

pub struct StoreFactTool;

#[async_trait]
impl Tool for StoreFactTool {
    fn name(&self) -> &str {
        "store_fact"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "store_fact".to_string(),
            description: "Store a structured fact about a person, commitment, preference, or event. Routes to the appropriate storage by category.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "enum": ["person", "commitment", "preference", "event"],
                        "description": "Type of fact to store"
                    },
                    "name": {
                        "type": "string",
                        "description": "Person's first name, or first + last if ambiguous. No prefixes (User:, Mr., Dr.). The relationship field captures the role."
                    },
                    "relationship": {
                        "type": "string",
                        "description": "Relationship to user (person category)"
                    },
                    "notes": {
                        "type": "string",
                        "description": "Additional notes (person category)"
                    },
                    "description": {
                        "type": "string",
                        "description": "What the commitment/event is (required for commitment, event)"
                    },
                    "due_date": {
                        "type": "string",
                        "description": "ISO date for commitments, e.g. '2026-03-01'"
                    },
                    "event_date": {
                        "type": "string",
                        "description": "ISO date for events, e.g. '2026-04-15'"
                    },
                    "key": {
                        "type": "string",
                        "description": "Preference category (required for preference)"
                    },
                    "value": {
                        "type": "string",
                        "description": "Preference value (required for preference)"
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Required in reflection mode: cite a specific conversation timestamp and quote as justification for this change"
                    }
                },
                "required": ["category"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Reflection mode: require evidence field
        if let Some(err) = super::check_reflection_evidence(ctx, &input) {
            return Ok(err);
        }

        let category = input["category"].as_str().unwrap_or("");

        match category {
            "person" => store_person(&input, ctx).await,
            "commitment" => store_commitment(&input, ctx).await,
            "preference" => store_preference(&input, ctx).await,
            "event" => store_event(&input, ctx).await,
            "" => Ok(ToolOutput::error("'category' is required.")),
            other => Ok(ToolOutput::error(format!(
                "Invalid category '{other}'. Use: person, commitment, preference, event"
            ))),
        }
    }
}

/// Build reasoning string with evidence for audit log in reflection mode.
fn reflection_reasoning(ctx: &ToolContext<'_>, input: &Value) -> Option<String> {
    if ctx.is_reflection {
        input["evidence"]
            .as_str()
            .map(|e| format!("[evidence] {e}"))
    } else {
        None
    }
}

fn validate_len(field: &str, value: &str) -> Option<ToolOutput> {
    if value.len() > MAX_INPUT_LEN {
        Some(ToolOutput::error(format!(
            "'{field}' too long: {} characters (max: {MAX_INPUT_LEN})",
            value.len()
        )))
    } else {
        None
    }
}

async fn store_person(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let name = input["name"].as_str().unwrap_or("");
    if name.is_empty() {
        return Ok(ToolOutput::error("'name' is required for person category."));
    }
    if let Some(err) = validate_len("name", name) {
        return Ok(err);
    }

    let relationship = input["relationship"].as_str();
    let notes = input["notes"].as_str();

    // Capture before_value for rewind support (None = creation, Some = upsert)
    let before_value = if let Some(existing) = ctx.db.get_person(name).await? {
        Some(
            serde_json::json!({
                "name": existing.canonical_name,
                "relationship": existing.relationship,
                "notes": existing.notes,
                "mention_count": existing.mention_count,
            })
            .to_string(),
        )
    } else {
        None
    };

    let person_id = ctx.db.upsert_person(name, relationship, notes).await?;

    // Log audit event
    let target = format!("person:{name}");
    let after = format!(
        "{}{}",
        name,
        relationship.map(|r| format!(" — {r}")).unwrap_or_default()
    );
    let reasoning = reflection_reasoning(ctx, input);
    ctx.db
        .log_audit_event(
            ctx.session_id,
            "store_fact",
            &target,
            before_value.as_deref(),
            Some(&after),
            reasoning.as_deref(),
            Some(ctx.trace_id),
        )
        .await?;

    // Index for search
    let mut content = name.to_string();
    if let Some(r) = relationship {
        content.push_str(&format!(" — {r}"));
    }
    if let Some(n) = notes {
        content.push_str(&format!(". {n}"));
    }
    index_fact(ctx, "person", person_id, &content).await;

    Ok(ToolOutput::success(format!("Stored person: {name}")))
}

async fn store_commitment(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let description = input["description"].as_str().unwrap_or("");
    if description.is_empty() {
        return Ok(ToolOutput::error(
            "'description' is required for commitment category.",
        ));
    }
    if let Some(err) = validate_len("description", description) {
        return Ok(err);
    }

    let due_date = input["due_date"].as_str();

    // Look up person if name provided
    let person_id = if let Some(name) = input["name"].as_str() {
        ctx.db.get_person(name).await?.map(|p| p.id)
    } else {
        None
    };

    let commitment_id = match ctx
        .db
        .add_commitment(description, due_date, person_id)
        .await
    {
        Ok(id) => id,
        Err(e) if crate::db::is_unique_violation(&e) => {
            let detail = if let Ok(commitments) = ctx.db.list_commitments("pending").await {
                commitments
                    .iter()
                    .find(|c| {
                        c.description.eq_ignore_ascii_case(description)
                            && c.due_date.as_deref() == due_date
                    })
                    .map(|c| {
                        let due_info = c
                            .due_date
                            .as_deref()
                            .map(|d| format!(" (due: {d})"))
                            .unwrap_or_default();
                        format!(
                            "A commitment '{}'{} already exists (id: {}). No duplicate created.",
                            c.description, due_info, c.id
                        )
                    })
            } else {
                None
            };
            return Ok(ToolOutput::success(detail.unwrap_or_else(|| {
                "A similar commitment already exists. No duplicate created.".to_string()
            })));
        }
        Err(e) => return Err(e),
    };

    let target = format!("commitment:{description}");
    let reasoning = reflection_reasoning(ctx, input);
    ctx.db
        .log_audit_event(
            ctx.session_id,
            "store_fact",
            &target,
            None,
            Some(description),
            reasoning.as_deref(),
            Some(ctx.trace_id),
        )
        .await?;

    // Index for search
    let mut content = description.to_string();
    if let Some(d) = due_date {
        content.push_str(&format!(" (due: {d})"));
    }
    content.push_str(", status: pending");
    index_fact(ctx, "commitment", commitment_id, &content).await;

    let due_info = due_date.map(|d| format!(" (due: {d})")).unwrap_or_default();
    Ok(ToolOutput::success(format!(
        "Stored commitment: \"{description}\"{due_info}"
    )))
}

async fn store_preference(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let key = input["key"].as_str().unwrap_or("");
    let value = input["value"].as_str().unwrap_or("");

    if key.is_empty() || value.is_empty() {
        return Ok(ToolOutput::error(
            "'key' and 'value' are required for preference category.",
        ));
    }
    if let Some(err) = validate_len("key", key) {
        return Ok(err);
    }
    if let Some(err) = validate_len("value", value) {
        return Ok(err);
    }

    // Capture before_value for rewind support (None = creation, Some = upsert)
    let before_value = ctx.db.get_preference(key).await?;

    let pref_id = ctx.db.set_preference(key, value).await?;

    let target = format!("preference:{key}");
    let reasoning = reflection_reasoning(ctx, input);
    ctx.db
        .log_audit_event(
            ctx.session_id,
            "store_fact",
            &target,
            before_value.as_deref(),
            Some(value),
            reasoning.as_deref(),
            Some(ctx.trace_id),
        )
        .await?;

    // Index for search
    let content = format!("{key}: {value}");
    index_fact(ctx, "preference", pref_id, &content).await;

    Ok(ToolOutput::success(format!(
        "Stored preference [{key}]: {value}"
    )))
}

async fn store_event(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let description = input["description"].as_str().unwrap_or("");
    if description.is_empty() {
        return Ok(ToolOutput::error(
            "'description' is required for event category.",
        ));
    }
    if let Some(err) = validate_len("description", description) {
        return Ok(err);
    }

    let event_date = input["event_date"].as_str();
    let notes = input["notes"].as_str();

    let event_id = match ctx.db.add_event(description, event_date, notes).await {
        Ok(id) => id,
        Err(e) if crate::db::is_unique_violation(&e) => {
            // Query for existing event to provide details
            let detail = if let Ok(events) = ctx.db.list_events().await {
                events
                    .iter()
                    .find(|ev| {
                        ev.description.eq_ignore_ascii_case(description)
                            && ev.event_date.as_deref() == event_date
                    })
                    .map(|ev| {
                        let date_info = ev
                            .event_date
                            .as_deref()
                            .map(|d| format!(" on {d}"))
                            .unwrap_or_default();
                        format!(
                            "An event '{}'{} already exists (id: {}). No duplicate created.",
                            ev.description, date_info, ev.id
                        )
                    })
            } else {
                None
            };
            return Ok(ToolOutput::success(detail.unwrap_or_else(|| {
                "A similar event already exists. No duplicate created.".to_string()
            })));
        }
        Err(e) => return Err(e),
    };

    let target = format!("event:{description}");
    let reasoning = reflection_reasoning(ctx, input);
    ctx.db
        .log_audit_event(
            ctx.session_id,
            "store_fact",
            &target,
            None,
            Some(description),
            reasoning.as_deref(),
            Some(ctx.trace_id),
        )
        .await?;

    // Index for search
    let mut content = description.to_string();
    if let Some(d) = event_date {
        content.push_str(&format!(" on {d}"));
    }
    if let Some(n) = notes {
        content.push_str(&format!(". {n}"));
    }
    index_fact(ctx, "event", event_id, &content).await;

    Ok(ToolOutput::success(format!(
        "Stored event: \"{description}\""
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_store_person() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "person",
                    "name": "Alice Chen",
                    "relationship": "CTO",
                    "notes": "Prefers morning standups"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Alice Chen"));

        let person = harness.db.get_person("Alice Chen").await.unwrap().unwrap();
        assert_eq!(person.relationship, Some("CTO".to_string()));
    }

    #[tokio::test]
    async fn test_store_commitment() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "commitment",
                    "description": "Review Q4 budget",
                    "due_date": "2026-03-01"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Q4 budget"));

        let commitments = harness.db.list_commitments("pending").await.unwrap();
        assert_eq!(commitments.len(), 1);
    }

    #[tokio::test]
    async fn test_store_preference() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "preference",
                    "key": "meeting_time",
                    "value": "Morning, before 10am"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let pref = harness
            .db
            .get_preference("meeting_time")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pref, "Morning, before 10am");
    }

    #[tokio::test]
    async fn test_store_event_with_event_date() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "event",
                    "description": "Board meeting",
                    "event_date": "2026-04-15"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Board meeting"));
    }

    #[tokio::test]
    async fn test_store_event_blocks_duplicate() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;
        let input = serde_json::json!({
            "category": "event",
            "description": "Board meeting",
            "event_date": "2026-04-15"
        });

        // First creation succeeds
        let result = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Board meeting"));

        // Second creation with same description is blocked — with details
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already exists"));
        assert!(
            result.content.contains("Board meeting"),
            "should contain event description"
        );
        assert!(
            result.content.contains("on 2026-04-15"),
            "should contain event date"
        );
        assert!(result.content.contains("id:"), "should contain event id");

        let events = harness.db.list_events().await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_store_event_allows_different_description() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        tool.execute(
            serde_json::json!({
                "category": "event",
                "description": "Board meeting",
                "event_date": "2026-04-15"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "event",
                    "description": "Team offsite",
                    "event_date": "2026-05-01"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Team offsite"));

        let events = harness.db.list_events().await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_store_event_allows_duplicate_without_date() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;
        let input = serde_json::json!({
            "category": "event",
            "description": "Interesting observation"
        });

        // First creation succeeds
        let result = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!result.is_error);

        // Second creation with same description but no date also succeeds
        // (partial index only covers dated events)
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Interesting observation"));

        let events = harness.db.list_events().await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_store_fact_logs_audit() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        tool.execute(
            serde_json::json!({
                "category": "person",
                "name": "Bob"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let events = harness.db.get_audit_events("test-session").await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "store_fact");
        assert!(events[0].target_key.starts_with("person:"));
    }

    #[tokio::test]
    async fn test_reflection_requires_evidence() {
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_reflection();
        let tool = StoreFactTool;

        // Without evidence → rejected
        let result = tool
            .execute(
                serde_json::json!({
                    "category": "person",
                    "name": "Alice"
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
        let ctx = harness.ctx_with_reflection();
        let tool = StoreFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "person",
                    "name": "Alice",
                    "relationship": "CTO",
                    "evidence": "User mentioned Alice as CTO in 3 conversations"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_store_commitment_blocks_duplicate() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;
        let input = serde_json::json!({
            "category": "commitment",
            "description": "Call Sarah",
            "due_date": "2026-03-15"
        });

        // First creation succeeds
        let result = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Call Sarah"));

        // Second creation with same description + due_date is blocked
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("already exists"));
        assert!(result.content.contains("Call Sarah"));
        assert!(result.content.contains("due: 2026-03-15"));
        assert!(
            result.content.contains("id:"),
            "should contain commitment id"
        );

        let commitments = harness.db.list_commitments("pending").await.unwrap();
        assert_eq!(commitments.len(), 1);
    }

    #[tokio::test]
    async fn test_store_commitment_allows_different_due_date() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        tool.execute(
            serde_json::json!({
                "category": "commitment",
                "description": "Call Sarah",
                "due_date": "2026-03-15"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "commitment",
                    "description": "Call Sarah",
                    "due_date": "2026-04-01"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);

        let commitments = harness.db.list_commitments("pending").await.unwrap();
        assert_eq!(commitments.len(), 2);
    }

    #[tokio::test]
    async fn test_store_commitment_allows_after_completed() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;
        let input = serde_json::json!({
            "category": "commitment",
            "description": "Call Sarah",
            "due_date": "2026-03-15"
        });

        // Create and complete
        let result = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!result.is_error);
        let commitments = harness.db.list_commitments("pending").await.unwrap();
        assert_eq!(commitments.len(), 1);
        let id = commitments[0].id;
        harness
            .db
            .update_commitment_status(id, "completed")
            .await
            .unwrap();

        // Create same again — succeeds because the first is completed
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Call Sarah"));

        let commitments = harness.db.list_commitments("pending").await.unwrap();
        assert_eq!(commitments.len(), 1);
    }

    #[tokio::test]
    async fn test_store_commitment_allows_duplicate_without_due_date() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;
        let input = serde_json::json!({
            "category": "commitment",
            "description": "Call Sarah"
        });

        // First creation
        let result = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!result.is_error);

        // Second creation with no due_date also succeeds (NULL != NULL in SQLite UNIQUE)
        let result = tool.execute(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Call Sarah"));

        let commitments = harness.db.list_commitments("pending").await.unwrap();
        assert_eq!(commitments.len(), 2);
    }

    #[tokio::test]
    async fn test_store_fact_missing_required() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = StoreFactTool;

        // Person without name
        let result = tool
            .execute(serde_json::json!({"category": "person"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'name' is required"));

        // Preference without key/value
        let result = tool
            .execute(serde_json::json!({"category": "preference"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
    }
}
