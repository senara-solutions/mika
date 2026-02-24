use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct SearchMemoryTool;

#[async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_memory".to_string(),
            description: "Search your stored facts across all categories (people, commitments, preferences, events, reminders, core memory). Uses case-insensitive substring matching.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search term to find across stored facts"
                    },
                    "category": {
                        "type": "string",
                        "enum": ["all", "person", "commitment", "preference", "event", "reminder", "core_memory"],
                        "description": "Category to search in (default: all)"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let query = input["query"].as_str().unwrap_or("");
        if query.is_empty() {
            return Ok(ToolOutput::error("'query' is required."));
        }
        if query.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'query' too long: {} characters (max: {MAX_INPUT_LEN})",
                query.len()
            )));
        }

        let category = input["category"].as_str().unwrap_or("all");
        let query_lower = query.to_lowercase();

        let mut results = Vec::new();

        // Search core memory (no LIKE index, still in-memory filter — tiny table)
        if category == "all" || category == "core_memory" {
            let entries = ctx.db.get_all_core_memory().await?;
            for entry in entries {
                if entry.key.to_lowercase().contains(&query_lower)
                    || entry.value.to_lowercase().contains(&query_lower)
                {
                    results.push(format!("[core_memory] {}: {}", entry.key, entry.value));
                }
            }
        }

        // Search people (SQL LIKE)
        if category == "all" || category == "person" {
            let people = ctx.db.search_people(query).await?;
            for person in people {
                let mut desc = format!("[person] {} (id:{})", person.canonical_name, person.id);
                if let Some(ref rel) = person.relationship {
                    desc.push_str(&format!(" — {rel}"));
                }
                if let Some(ref notes) = person.notes {
                    desc.push_str(&format!(" | {notes}"));
                }
                results.push(desc);
            }
        }

        // Search commitments (SQL LIKE across all statuses)
        if category == "all" || category == "commitment" {
            let commitments = ctx.db.search_commitments(query).await?;
            for c in commitments {
                let mut desc = format!(
                    "[commitment] {} (id:{}, status:{})",
                    c.description, c.id, c.status
                );
                if let Some(ref due) = c.due_date {
                    desc.push_str(&format!(" due:{due}"));
                }
                results.push(desc);
            }
        }

        // Search preferences (SQL LIKE)
        if category == "all" || category == "preference" {
            let prefs = ctx.db.search_preferences(query).await?;
            for pref in prefs {
                results.push(format!("[preference] {}: {}", pref.category, pref.value));
            }
        }

        // Search events (SQL LIKE)
        if category == "all" || category == "event" {
            let events = ctx.db.search_events(query).await?;
            for event in events {
                let mut desc = format!(
                    "[event] {} (id:{}",
                    event.description, event.id
                );
                if let Some(ref date) = event.event_date {
                    desc.push_str(&format!(", {date}"));
                }
                desc.push(')');
                if let Some(ref context) = event.context {
                    if !context.is_empty() {
                        desc.push_str(&format!(" — {context}"));
                    }
                }
                results.push(desc);
            }
        }

        // Search reminders (SQL LIKE)
        if category == "all" || category == "reminder" {
            let reminders = ctx.db.search_reminders(query).await?;
            for r in reminders {
                results.push(format!(
                    "[reminder] #{}: \"{}\" at {} (created: {})",
                    r.id, r.message, r.fire_at, r.created_at
                ));
            }
        }

        if results.is_empty() {
            Ok(ToolOutput::success(format!(
                "No results found for \"{query}\" in {category}."
            )))
        } else {
            let count = results.len();
            let body = results.join("\n");
            Ok(ToolOutput::success(format!(
                "Found {count} result(s) for \"{query}\":\n{body}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::{test_async_db, test_ctx};
    use std::sync::atomic::AtomicU32;

    #[tokio::test]
    async fn test_search_finds_person() {
        let db = test_async_db();
        db.upsert_person("Alice Chen", Some("CTO"), Some("Likes coffee"))
            .await
            .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        let result = tool
            .execute(serde_json::json!({"query": "Alice"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Alice Chen"));
        assert!(result.content.contains("[person]"));
    }

    #[tokio::test]
    async fn test_search_finds_commitment() {
        let db = test_async_db();
        db.add_commitment("Review Q4 budget", Some("2026-03-01"), None)
            .await
            .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        let result = tool
            .execute(serde_json::json!({"query": "budget"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Q4 budget"));
        assert!(result.content.contains("[commitment]"));
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let db = test_async_db();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        let result = tool
            .execute(serde_json::json!({"query": "nonexistent"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No results found"));
    }

    #[tokio::test]
    async fn test_search_filters_by_category() {
        let db = test_async_db();
        db.upsert_person("Alice", None, None).await.unwrap();
        db.add_commitment("Call Alice", None, None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        // Search only in person category
        let result = tool
            .execute(
                serde_json::json!({"query": "Alice", "category": "person"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.content.contains("[person]"));
        assert!(!result.content.contains("[commitment]"));
    }

    #[tokio::test]
    async fn test_search_core_memory() {
        let db = test_async_db();
        db.seed_core_memory(None).await.unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "user", "category": "core_memory"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.content.contains("[core_memory]"));
        assert!(result.content.contains("user_summary"));
    }

    #[tokio::test]
    async fn test_search_finds_preference_by_value_substring() {
        let db = test_async_db();
        db.set_preference("Food", "No shellfish, prefers sushi")
            .await
            .unwrap();
        db.set_preference("Meeting time", "Morning, before 10am")
            .await
            .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        // Search by value substring
        let result = tool
            .execute(serde_json::json!({"query": "shellfish"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[preference]"));
        assert!(result.content.contains("Food"));

        // Search by partial category
        let result = tool
            .execute(
                serde_json::json!({"query": "meeting", "category": "preference"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Meeting time"));
        assert!(result.content.contains("Morning"));
    }

    #[tokio::test]
    async fn test_search_event_includes_context() {
        let db = test_async_db();
        db.add_event(
            "Team offsite in Bali",
            Some("2026-06-01"),
            Some("annual planning retreat"),
        )
        .await
        .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        let result = tool
            .execute(serde_json::json!({"query": "Bali"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Team offsite in Bali"));
        assert!(result.content.contains("annual planning retreat"));
    }

    #[tokio::test]
    async fn test_search_finds_event_by_description() {
        let db = test_async_db();
        db.add_event(
            "Board meeting with investors",
            Some("2026-03-15"),
            Some("quarterly review"),
        )
        .await
        .unwrap();
        let counter = AtomicU32::new(0);
        let ctx = test_ctx(&db, &counter);
        let tool = SearchMemoryTool;

        // Search by description substring
        let result = tool
            .execute(serde_json::json!({"query": "investors"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[event]"));
        assert!(result.content.contains("Board meeting with investors"));
        assert!(result.content.contains("2026-03-15"));

        // Search with event category filter
        let result = tool
            .execute(
                serde_json::json!({"query": "board", "category": "event"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[event]"));
        assert!(!result.content.contains("[person]"));
    }
}
