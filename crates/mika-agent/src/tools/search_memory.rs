use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::db::core_memory_section_names;

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
            description: "Search your stored facts across all categories (people, commitments, preferences, events, reminders). Uses semantic search when available, with full-text and keyword fallback. Note: core_memory is auto-injected into your system prompt — search there first before using this tool for core_memory content.".to_string(),
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

        // Redirect: core_memory is auto-injected into the system prompt every turn.
        // Querying the DB for it is always redundant.
        if category == "core_memory" {
            tracing::info!(
                tool = "search_memory",
                redirect_reason = "core_memory_in_prompt",
                query = query,
                "context redundancy redirect: core_memory already in system prompt"
            );
            return Ok(ToolOutput::error(
                "core_memory is auto-injected into your system prompt on every turn — \
                 the content is already in the 'Core Memory' block above. \
                 Search there directly instead of querying the database. \
                 If you need to modify core_memory, use the update_core_memory tool.",
            ));
        }

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

        // Soft hint: if the query matches a core_memory section name, remind
        // the agent that this data is already in the system prompt.
        let core_memory_hint = if category == "all" {
            let query_lower = query.to_lowercase();
            core_memory_section_names()
                .iter()
                .find(|section| section.to_lowercase() == query_lower)
                .map(|section| {
                    format!(
                        "Hint: '{section}' is a core_memory section already in your system prompt. \
                         Check the 'Core Memory' block above first."
                    )
                })
        } else {
            None
        };

        if results.is_empty() {
            let mut msg = format!("No results found for \"{query}\" in {category}.");
            if let Some(hint) = core_memory_hint {
                msg = format!("{hint}\n\n{msg}");
            }
            Ok(ToolOutput::success(msg))
        } else {
            let count = results.len();
            let body = results.join("\n");
            let mut msg = format!("Found {count} result(s) for \"{query}\":\n{body}");
            if let Some(hint) = core_memory_hint {
                msg = format!("{hint}\n\n{msg}");
            }
            Ok(ToolOutput::success(msg))
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

/// Search pending reminder tasks by label.
async fn search_reminders(
    ctx: &ToolContext<'_>,
    query: &str,
    results: &mut Vec<String>,
) -> Result<()> {
    let query_lower = query.to_lowercase();
    let tasks = ctx.db.get_user_visible_tasks().await?;
    for t in tasks {
        if t.label.to_lowercase().contains(&query_lower) {
            let short_id = &t.id[..8.min(t.id.len())];
            let fire_at = t
                .next_fire_at
                .map(|s| crate::db::format_ts(&s))
                .unwrap_or_else(|| "unknown".to_string());
            results.push(format!(
                "[reminder] {}: \"{}\" at {}",
                short_id, t.label, fire_at
            ));
        }
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

    // --- Context redundancy redirect tests ---

    #[tokio::test]
    async fn test_search_core_memory_category_redirected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "self_model", "category": "core_memory"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("auto-injected"),
            "expected redirect message, got: {}",
            result.content
        );
        assert!(result.content.contains("update_core_memory"));
    }

    #[tokio::test]
    async fn test_search_person_category_not_redirected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "Alice", "category": "person"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_search_all_category_not_redirected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "Alice", "category": "all"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_search_empty_query_validation_before_redirect() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "", "category": "core_memory"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        // Should be the validation error, not the redirect
        assert!(
            result.content.contains("required"),
            "expected validation error, got: {}",
            result.content
        );
    }

    // --- Core memory heading hint tests ---

    #[tokio::test]
    async fn test_search_all_with_section_name_query_has_hint() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "self_model", "category": "all"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("Hint:"),
            "expected hint, got: {}",
            result.content
        );
        assert!(result.content.contains("core_memory section"));
    }

    #[tokio::test]
    async fn test_search_all_with_non_section_query_no_hint() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "Alice", "category": "all"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.content.contains("Hint:"));
    }

    #[tokio::test]
    async fn test_search_all_section_name_case_insensitive() {
        let harness = TestHarness::new();
        harness.db.seed_core_memory(None).await.unwrap();
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "Self_Model", "category": "all"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("Hint:"),
            "expected hint for case-insensitive match, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_search_all_section_name_no_results_still_has_hint() {
        let harness = TestHarness::new();
        // Don't seed core memory — query will find no results
        let ctx = harness.ctx();
        let tool = SearchMemoryTool;

        let result = tool
            .execute(
                serde_json::json!({"query": "user_summary", "category": "all"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("Hint:"),
            "expected hint even with no results, got: {}",
            result.content
        );
        assert!(result.content.contains("No results found"));
    }

    // --- Existing tests ---

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
        // category="core_memory" is now redirected — core memory is always in the prompt.
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
        assert!(result.is_error);
        assert!(result.content.contains("auto-injected"));
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
