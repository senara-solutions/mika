use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};

pub struct SearchMemoryTool;

/// Categories that are indexed in the search_content table.
const INDEXED_CATEGORIES: &[&str] = &["person", "commitment", "preference", "event"];

#[async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_memory".to_string(),
            description: "Search your stored facts across all categories (people, commitments, preferences, events, reminders, core memory). Uses semantic search when available, with full-text and keyword fallback.".to_string(),
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

        let mut results = Vec::new();

        // Non-indexed categories: always use LIKE
        if category == "all" || category == "core_memory" {
            search_core_memory(ctx, query, &mut results).await?;
        }
        if category == "all" || category == "reminder" {
            search_reminders(ctx, query, &mut results).await?;
        }

        // Indexed categories: try hybrid search, fall back to LIKE
        let use_hybrid = category == "all" || INDEXED_CATEGORIES.contains(&category);
        if use_hybrid {
            let source_type_filter = if category == "all" {
                None
            } else {
                Some(category)
            };

            let hybrid_results = run_hybrid_search(ctx, query, source_type_filter).await;

            if !hybrid_results.is_empty() {
                // Use hybrid search results
                for r in hybrid_results {
                    results.push(format!("[{}] {}", r.source_type, r.content));
                }
            } else {
                // Fallback to LIKE-based search (index may not be populated yet)
                search_like_fallback(ctx, query, category, &mut results).await?;
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

/// Run hybrid search (FTS5 + optional vector embedding).
async fn run_hybrid_search(
    ctx: &ToolContext<'_>,
    query: &str,
    source_type_filter: Option<&str>,
) -> Vec<crate::db::SearchResult> {
    // Generate query embedding if client is available
    let embedding = if let Some(client) = ctx.embedding_client {
        match client.embed(query).await {
            Ok(emb) => Some(emb),
            Err(e) => {
                tracing::warn!(error = %e, "failed to generate query embedding, using FTS5 only");
                None
            }
        }
    } else {
        None
    };

    match ctx
        .db
        .hybrid_search(query, embedding, 20, source_type_filter)
        .await
    {
        Ok(results) => results,
        Err(e) => {
            tracing::warn!(error = %e, "hybrid search failed, falling back to LIKE");
            Vec::new()
        }
    }
}

/// Search core memory (always in-memory filter, tiny table).
async fn search_core_memory(
    ctx: &ToolContext<'_>,
    query: &str,
    results: &mut Vec<String>,
) -> Result<()> {
    let query_lower = query.to_lowercase();
    let entries = ctx.db.get_all_core_memory().await?;
    for entry in entries {
        if entry.key.to_lowercase().contains(&query_lower)
            || entry.value.to_lowercase().contains(&query_lower)
        {
            results.push(format!("[core_memory] {}: {}", entry.key, entry.value));
        }
    }
    Ok(())
}

/// Search reminders via SQL LIKE.
async fn search_reminders(
    ctx: &ToolContext<'_>,
    query: &str,
    results: &mut Vec<String>,
) -> Result<()> {
    let reminders = ctx.db.search_reminders(query).await?;
    for r in reminders {
        results.push(format!(
            "[reminder] #{}: \"{}\" at {} (created: {})",
            r.id, r.message, r.display_fire_at(), r.created_at
        ));
    }
    Ok(())
}

/// LIKE-based fallback for indexed categories (used when search index is empty).
async fn search_like_fallback(
    ctx: &ToolContext<'_>,
    query: &str,
    category: &str,
    results: &mut Vec<String>,
) -> Result<()> {
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

    if category == "all" || category == "preference" {
        let prefs = ctx.db.search_preferences(query).await?;
        for pref in prefs {
            results.push(format!("[preference] {}: {}", pref.category, pref.value));
        }
    }

    if category == "all" || category == "event" {
        let events = ctx.db.search_events(query).await?;
        for event in events {
            let mut desc = format!("[event] {} (id:{}", event.description, event.id);
            if let Some(ref date) = event.event_date {
                desc.push_str(&format!(", {date}"));
            }
            desc.push(')');
            if let Some(ref context) = event.context
                && !context.is_empty()
            {
                desc.push_str(&format!(" — {context}"));
            }
            results.push(desc);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use crate::tools::index_fact;

    #[tokio::test]
    async fn test_search_finds_person() {
        let harness = TestHarness::new();
        harness
            .db
            .upsert_person("Alice Chen", Some("CTO"), Some("Likes coffee"))
            .await
            .unwrap();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness
            .db
            .add_commitment("Review Q4 budget", Some("2026-03-01"), None)
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(serde_json::json!({"query": "budget"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("budget"));
        assert!(result.content.contains("[commitment]"));
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness.db.upsert_person("Alice", None, None).await.unwrap();
        harness
            .db
            .add_commitment("Call Alice", None, None)
            .await
            .unwrap();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness
            .db
            .set_preference("Food", "No shellfish, prefers sushi")
            .await
            .unwrap();
        harness
            .db
            .set_preference("Meeting time", "Morning, before 10am")
            .await
            .unwrap();
        let ctx = harness.ctx();
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
        let harness = TestHarness::new();
        harness
            .db
            .add_event(
                "Team offsite in Bali",
                Some("2026-06-01"),
                Some("annual planning retreat"),
            )
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(serde_json::json!({"query": "Bali"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Bali"));
    }

    #[tokio::test]
    async fn test_search_finds_event_by_description() {
        let harness = TestHarness::new();
        harness
            .db
            .add_event(
                "Board meeting with investors",
                Some("2026-03-15"),
                Some("quarterly review"),
            )
            .await
            .unwrap();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        // Search by description substring
        let result = tool
            .execute(serde_json::json!({"query": "investors"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("investors"));

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

    #[tokio::test]
    async fn test_search_uses_hybrid_when_indexed() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // Index a fact directly into search_content (simulating store_fact behavior)
        index_fact(&ctx, "person", 1, "Alice Chen — CTO. Likes morning coffee").await;

        let tool = SearchMemoryTool;
        let result = tool
            .execute(serde_json::json!({"query": "coffee"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("[person]"));
        assert!(result.content.contains("Alice Chen"));
    }

    #[tokio::test]
    async fn test_search_hybrid_with_category_filter() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // Index facts of different types
        index_fact(&ctx, "person", 1, "Alice Chen — CTO").await;
        index_fact(&ctx, "commitment", 1, "Review Alice's proposal").await;

        let tool = SearchMemoryTool;

        // Filter to person only
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
}
