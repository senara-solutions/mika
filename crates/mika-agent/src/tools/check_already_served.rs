//! `check_already_served` tool (mika#1867).
//!
//! Query the served-content ledger for a given person + category. Call BEFORE
//! generating content (proverb, quote, joke, poem, story, recommendation,
//! fact) to see what has already been served. Returns the last 3 serves with
//! 200-char snippets so the LLM can steer away from repeats.
//!
//! `person_id: i64` is required — matches `record_served_content` (architect
//! F2 arbitration). No `person_name` fallback.

use anyhow::Result;
use async_trait::async_trait;
use chrono::Duration;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};
use crate::memory::{SERVED_CONTENT_CATEGORIES, SERVED_CONTENT_DEFAULT_WINDOW_DAYS};
use crate::timestamp;

/// Snippet length cap in the tool response (mika#1867 Q3 architect arbitration).
const SNIPPET_MAX_CHARS: usize = 200;
/// Number of most-recent serves returned (mika#1867 Q3 architect arbitration).
const CHECK_LIMIT: usize = 3;

pub struct CheckAlreadyServedTool;

fn person_id_required_error() -> ToolOutput {
    ToolOutput::error(
        serde_json::json!({
            "error": "person_id_required",
            "hint": "Resolve person_id via search_memory(query=\"<name>\", category=\"person\") — or store_fact(category=\"person\", key=\"<name>\", value=\"<name>\") then search_memory to read back the id — before checking served content."
        })
        .to_string(),
    )
}

fn snippet(content: &str) -> String {
    // Char-safe truncation.
    let mut out = String::with_capacity(SNIPPET_MAX_CHARS.min(content.len()));
    for (i, c) in content.chars().enumerate() {
        if i >= SNIPPET_MAX_CHARS {
            break;
        }
        out.push(c);
    }
    out
}

#[async_trait]
impl Tool for CheckAlreadyServedTool {
    fn name(&self) -> &str {
        "check_already_served"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_already_served".to_string(),
            description: format!(
                "Query the served-content ledger for a given person + category. Call BEFORE \
                 generating content (proverb, quote, joke, poem, story, recommendation, fact) \
                 to see what has already been served. Returns the last {CHECK_LIMIT} serves with \
                 {SNIPPET_MAX_CHARS}-char snippets. Requires person_id (INTEGER) — resolve via \
                 search_memory(query=\"<name>\", category=\"person\") or store_fact(category=\"person\", key=\"<name>\", value=\"<name>\") + follow-up search_memory."
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "person_id": {
                        "type": "integer",
                        "description": "The people.id of the person to query. Resolve via search_memory(query=\"<name>\", category=\"person\") or store_fact(category=\"person\", key=\"<name>\", value=\"<name>\") + follow-up search_memory."
                    },
                    "person_name": {
                        "type": "string",
                        "description": "REJECTED — pass person_id instead. Accepted only to surface a helpful error."
                    },
                    "category": {
                        "type": "string",
                        "enum": ["proverb","quote","joke","poem","recommendation","story","fact"],
                        "description": "The bounded content class to check."
                    },
                    "days": {
                        "type": "integer",
                        "description": "Lookback window in days. Defaults to 90.",
                        "minimum": 1
                    },
                    "content_hash": {
                        "type": "string",
                        "description": "Optional exact-hash filter. When set, only items with a matching hash are returned."
                    }
                },
                "required": ["person_id", "category"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        if input.get("person_id").is_none() && input.get("person_name").is_some() {
            return Ok(person_id_required_error());
        }

        let person_id = match input.get("person_id").and_then(|v| v.as_i64()) {
            Some(id) if id > 0 => id,
            _ => return Ok(person_id_required_error()),
        };

        let category = input["category"].as_str().unwrap_or("");
        if category.is_empty() {
            return Ok(ToolOutput::error("'category' is required."));
        }
        if !SERVED_CONTENT_CATEGORIES.contains(&category) {
            return Ok(ToolOutput::error(format!(
                "Invalid category '{category}'. Use one of: {}",
                SERVED_CONTENT_CATEGORIES.join(", ")
            )));
        }

        let days: i64 = input
            .get("days")
            .and_then(|v| v.as_i64())
            .filter(|d| *d > 0)
            .unwrap_or(SERVED_CONTENT_DEFAULT_WINDOW_DAYS);
        let since = timestamp::now_minus(Duration::days(days));

        let hash_filter = input.get("content_hash").and_then(|v| v.as_str());

        let mut items = ctx
            .db
            .list_served_content(person_id, category.to_string(), Some(since), CHECK_LIMIT)
            .await?;

        if let Some(h) = hash_filter {
            items.retain(|row| row.content_hash == h);
        }

        let json_items: Vec<serde_json::Value> = items
            .iter()
            .map(|r| {
                serde_json::json!({
                    "served_at": r.served_at,
                    "content_hash": r.content_hash,
                    "snippet": snippet(&r.content_text),
                })
            })
            .collect();

        Ok(ToolOutput::success(
            serde_json::json!({
                "served_count": json_items.len(),
                "window_days": days,
                "items": json_items,
            })
            .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    async fn seed_person(harness: &TestHarness) -> i64 {
        harness
            .db
            .upsert_person("Al", Some("tester"), None)
            .await
            .unwrap()
    }

    async fn record(harness: &TestHarness, person_id: i64, category: &str, content: &str) {
        harness
            .db
            .record_served_content(
                person_id,
                category.to_string(),
                content.to_string(),
                Some("test-session".to_string()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_returns_serves_within_window() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;

        record(&harness, person_id, "proverb", "First proverb.").await;
        record(&harness, person_id, "proverb", "Second proverb.").await;
        record(&harness, person_id, "proverb", "Third proverb.").await;

        let ctx = harness.ctx();
        let tool = CheckAlreadyServedTool;
        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": person_id,
                    "category": "proverb"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("\"served_count\":3"));
        assert!(out.content.contains("First proverb.") || out.content.contains("Third proverb."));
    }

    #[tokio::test]
    async fn test_narrow_window_returns_none() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;
        record(&harness, person_id, "proverb", "Old proverb.").await;

        // Shift the served_at 200 days back so a days=1 query excludes it.
        // We use an UPDATE on the underlying table for the test fixture.
        harness
            .db
            .with_db(move |db| {
                db.conn
                    .execute(
                        "UPDATE served_content SET served_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-200 days')",
                        [],
                    )
                    .map(|_| ())
                    .map_err(Into::into)
            })
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = CheckAlreadyServedTool;
        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": person_id,
                    "category": "proverb",
                    "days": 1
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("\"served_count\":0"));
    }

    #[tokio::test]
    async fn test_content_hash_filter() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;

        record(&harness, person_id, "proverb", "Match this.").await;
        record(&harness, person_id, "proverb", "Ignore this.").await;

        let expected_hash = crate::memory::compute_content_hash("Match this.");

        let ctx = harness.ctx();
        let tool = CheckAlreadyServedTool;
        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": person_id,
                    "category": "proverb",
                    "content_hash": expected_hash
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("\"served_count\":1"));
        assert!(out.content.contains("Match this."));
        assert!(!out.content.contains("Ignore this."));
    }

    #[tokio::test]
    async fn test_snippet_truncation() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;
        // > 200 char content
        let long: String = "a".repeat(500);
        record(&harness, person_id, "quote", &long).await;

        let ctx = harness.ctx();
        let tool = CheckAlreadyServedTool;
        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": person_id,
                    "category": "quote"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!out.is_error);
        // Snippet should be 200 chars, not 500.
        let parsed: serde_json::Value = serde_json::from_str(&out.content).unwrap();
        let snippet = parsed["items"][0]["snippet"].as_str().unwrap();
        assert_eq!(snippet.len(), 200);
    }

    #[tokio::test]
    async fn test_missing_person_id_rejects() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = CheckAlreadyServedTool;

        let out = tool
            .execute(
                serde_json::json!({
                    "person_name": "Al",
                    "category": "proverb"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("person_id_required"));
    }
}
