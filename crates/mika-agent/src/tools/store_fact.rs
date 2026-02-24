use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct StoreFactTool;

#[async_trait(?Send)]
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
                        "description": "Person's full name (required for person category)"
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
                        "description": "ISO date, e.g. '2026-03-01' (commitment, event)"
                    },
                    "key": {
                        "type": "string",
                        "description": "Preference category (required for preference)"
                    },
                    "value": {
                        "type": "string",
                        "description": "Preference value (required for preference)"
                    }
                },
                "required": ["category"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let category = input["category"].as_str().unwrap_or("");

        match category {
            "person" => store_person(&input, ctx),
            "commitment" => store_commitment(&input, ctx),
            "preference" => store_preference(&input, ctx),
            "event" => store_event(&input, ctx),
            "" => Ok(ToolOutput::error("'category' is required.")),
            other => Ok(ToolOutput::error(format!(
                "Invalid category '{other}'. Use: person, commitment, preference, event"
            ))),
        }
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

fn store_person(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let name = input["name"].as_str().unwrap_or("");
    if name.is_empty() {
        return Ok(ToolOutput::error("'name' is required for person category."));
    }
    if let Some(err) = validate_len("name", name) {
        return Ok(err);
    }

    let relationship = input["relationship"].as_str();
    let notes = input["notes"].as_str();

    ctx.db.upsert_person(name, relationship, notes)?;

    // Log audit event
    let target = format!("person:{name}");
    let after = format!(
        "{}{}",
        name,
        relationship.map(|r| format!(" — {r}")).unwrap_or_default()
    );
    ctx.db
        .log_memory_event(ctx.session_id, "store_fact", &target, None, &after, None)?;

    Ok(ToolOutput::success(format!("Stored person: {name}")))
}

fn store_commitment(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
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
        ctx.db.get_person(name)?.map(|p| p.id)
    } else {
        None
    };

    ctx.db.add_commitment(description, due_date, person_id)?;

    let target = format!("commitment:{description}");
    ctx.db.log_memory_event(
        ctx.session_id,
        "store_fact",
        &target,
        None,
        description,
        None,
    )?;

    let due_info = due_date.map(|d| format!(" (due: {d})")).unwrap_or_default();
    Ok(ToolOutput::success(format!(
        "Stored commitment: \"{description}\"{due_info}"
    )))
}

fn store_preference(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
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

    ctx.db.set_preference(key, value)?;

    let target = format!("preference:{key}");
    ctx.db
        .log_memory_event(ctx.session_id, "store_fact", &target, None, value, None)?;

    Ok(ToolOutput::success(format!(
        "Stored preference [{key}]: {value}"
    )))
}

fn store_event(input: &Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
    let description = input["description"].as_str().unwrap_or("");
    if description.is_empty() {
        return Ok(ToolOutput::error(
            "'description' is required for event category.",
        ));
    }
    if let Some(err) = validate_len("description", description) {
        return Ok(err);
    }

    let due_date = input["due_date"].as_str();
    let notes = input["notes"].as_str();

    ctx.db.add_event(description, due_date, notes)?;

    let target = format!("event:{description}");
    ctx.db.log_memory_event(
        ctx.session_id,
        "store_fact",
        &target,
        None,
        description,
        None,
    )?;

    Ok(ToolOutput::success(format!(
        "Stored event: \"{description}\""
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::sync::atomic::AtomicU32;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn test_ctx<'a>(db: &'a Database, edit_count: &'a AtomicU32) -> ToolContext<'a> {
        static HOME_DIR: &str = "/tmp/mika-test";
        ToolContext {
            db,
            session_id: "test-session",
            home_dir: std::path::Path::new(HOME_DIR),
            core_memory_edit_count: edit_count,
            is_onboarding: false,
        }
    }

    #[tokio::test]
    async fn test_store_person() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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

        let person = db.get_person("Alice Chen").unwrap().unwrap();
        assert_eq!(person.relationship, Some("CTO".to_string()));
    }

    #[tokio::test]
    async fn test_store_commitment() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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

        let commitments = db.list_commitments("pending").unwrap();
        assert_eq!(commitments.len(), 1);
    }

    #[tokio::test]
    async fn test_store_preference() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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

        let pref = db.get_preference("meeting_time").unwrap().unwrap();
        assert_eq!(pref, "Morning, before 10am");
    }

    #[tokio::test]
    async fn test_store_event() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = StoreFactTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "category": "event",
                    "description": "Board meeting",
                    "due_date": "2026-04-15"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Board meeting"));
    }

    #[tokio::test]
    async fn test_store_fact_logs_audit() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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

        let events = db.get_memory_events("test-session").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "store_fact");
        assert!(events[0].target_key.starts_with("person:"));
    }

    #[tokio::test]
    async fn test_store_fact_missing_required() {
        let db = test_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
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
