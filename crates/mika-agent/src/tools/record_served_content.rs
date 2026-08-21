//! `record_served_content` tool (mika#1867).
//!
//! Ledger a piece of content Mika has served to a specific person, keyed by
//! exact-match hash. Prevents re-serving the same content on future requests.
//! Founding incident: Al (Vietnam tester) 2026-07-28 — same zen proverb served
//! twice, 6 days apart.
//!
//! `person_id: i64` is required — architect F2 arbitration explicitly rejects
//! `person_name` fallback (silent mis-attribution risk in multi-`Al` scenarios).
//! The caller resolves `person_id` via `list_people` or by upserting via
//! `store_fact(category="person")` and reading back the row.

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_PAYLOAD_BYTES, Tool, ToolContext, ToolOutput};
use crate::db::RecordOutcome;
use crate::memory::{SERVED_CONTENT_CATEGORIES, compute_content_hash};

pub struct RecordServedContentTool;

fn person_id_required_error() -> ToolOutput {
    ToolOutput::error(
        serde_json::json!({
            "error": "person_id_required",
            "hint": "Resolve person_id via list_people or upsert_person before recording served content."
        })
        .to_string(),
    )
}

#[async_trait]
impl Tool for RecordServedContentTool {
    fn name(&self) -> &str {
        "record_served_content"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "record_served_content".to_string(),
            description:
                "Ledger a piece of content Mika has served to a specific person, keyed by \
                 exact-match hash. Prevents re-serving the same content on future requests. \
                 Categories: proverb, quote, joke, poem, recommendation, story, fact. \
                 Requires person_id (INTEGER) — resolve via list_people or store_fact(category=\"person\") first."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "person_id": {
                        "type": "integer",
                        "description": "The people.id of the person the content was served to. Resolve via list_people or upsert_person."
                    },
                    "person_name": {
                        "type": "string",
                        "description": "REJECTED — pass person_id instead. Accepted only to surface a helpful error."
                    },
                    "category": {
                        "type": "string",
                        "enum": ["proverb","quote","joke","poem","recommendation","story","fact"],
                        "description": "The bounded content class Mika served."
                    },
                    "content": {
                        "type": "string",
                        "description": "The exact text of the content served. Used to compute the dedup hash."
                    },
                    "signature": {
                        "type": "string",
                        "description": "Reserved for v2 fuzzy dedup (AC6). Accepted but not currently persisted."
                    }
                },
                "required": ["person_id", "category", "content"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        // Reject person_name inputs explicitly (architect F2 arbitration).
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

        let content = input["content"].as_str().unwrap_or("");
        if content.trim().is_empty() {
            return Ok(ToolOutput::error(
                "'content' is required and must be non-empty.",
            ));
        }
        if content.len() > MAX_PAYLOAD_BYTES {
            return Ok(ToolOutput::error(format!(
                "'content' too long: {} bytes (max: {MAX_PAYLOAD_BYTES})",
                content.len()
            )));
        }

        let session_id = ctx.session_id.to_string();
        let outcome = match ctx
            .db
            .record_served_content(
                person_id,
                category.to_string(),
                content.to_string(),
                Some(session_id),
            )
            .await
        {
            Ok(o) => o,
            Err(e) => {
                // FK violations surface as structured JSON so the LLM can self-correct
                // by upserting the person and retrying.
                let msg = e.to_string();
                if msg.contains("not found in people table") {
                    return Ok(ToolOutput::error(
                        serde_json::json!({
                            "error": "person_not_found",
                            "person_id": person_id,
                            "hint": "person_id does not exist in the people table. Upsert via store_fact(category=\"person\") and re-read via list_people."
                        })
                        .to_string(),
                    ));
                }
                return Err(e);
            }
        };

        let hash = compute_content_hash(content);
        match outcome {
            RecordOutcome::Inserted { id } => Ok(ToolOutput::success(
                serde_json::json!({
                    "status": "recorded",
                    "id": id,
                    "content_hash": hash,
                })
                .to_string(),
            )),
            RecordOutcome::Duplicate {
                existing_id,
                prior_served_at,
            } => Ok(ToolOutput::success(
                serde_json::json!({
                    "status": "duplicate",
                    "existing_id": existing_id,
                    "prior_served_at": prior_served_at,
                    "retry_hint": format!(
                        "Content already served on {prior_served_at}. Regenerate with a different item."
                    ),
                })
                .to_string(),
            )),
        }
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

    #[tokio::test]
    async fn test_record_inserted_then_duplicate() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;
        let ctx = harness.ctx();
        let tool = RecordServedContentTool;

        let input = serde_json::json!({
            "person_id": person_id,
            "category": "proverb",
            "content": "Avant l'éveil, couper du bois, porter de l'eau."
        });

        let out = tool.execute(input.clone(), &ctx).await.unwrap();
        assert!(!out.is_error, "first call should succeed: {}", out.content);
        assert!(out.content.contains("\"status\":\"recorded\""));
        assert!(out.content.contains("\"id\""));
        assert!(out.content.contains("\"content_hash\""));

        // Second call → duplicate (still success, but flagged).
        let out = tool.execute(input, &ctx).await.unwrap();
        assert!(!out.is_error);
        assert!(out.content.contains("\"status\":\"duplicate\""));
        assert!(out.content.contains("\"prior_served_at\""));
        assert!(out.content.contains("retry_hint"));
    }

    #[tokio::test]
    async fn test_missing_person_id_rejects() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RecordServedContentTool;

        // person_name only → rejected
        let out = tool
            .execute(
                serde_json::json!({
                    "person_name": "Al",
                    "category": "proverb",
                    "content": "Some proverb."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("person_id_required"));

        // Missing both → rejected
        let out = tool
            .execute(
                serde_json::json!({
                    "category": "proverb",
                    "content": "Some proverb."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("person_id_required"));
    }

    #[tokio::test]
    async fn test_invalid_category_rejects() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;
        let ctx = harness.ctx();
        let tool = RecordServedContentTool;

        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": person_id,
                    "category": "riddle",
                    "content": "Some content."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("Invalid category"));
    }

    #[tokio::test]
    async fn test_nonexistent_person_id_errors() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = RecordServedContentTool;

        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": 99999,
                    "category": "proverb",
                    "content": "Some content."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("person_not_found"));
    }

    #[tokio::test]
    async fn test_empty_content_rejects() {
        let harness = TestHarness::new();
        let person_id = seed_person(&harness).await;
        let ctx = harness.ctx();
        let tool = RecordServedContentTool;

        let out = tool
            .execute(
                serde_json::json!({
                    "person_id": person_id,
                    "category": "proverb",
                    "content": "   "
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content.contains("content"));
    }
}
