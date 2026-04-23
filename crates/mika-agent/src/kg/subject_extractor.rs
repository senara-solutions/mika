//! # Subject Graph Extractor
//!
//! Per-agent LLM-based extraction of named entities and fact triples from
//! previously-ingested documents (#690). For each document, the extractor:
//!
//! 1. Reads the full doc from disk.
//! 2. Reads chunk boundaries from `kg_chunks` for the doc.
//! 3. Builds a prompt with chunk boundary markers (`[CHUNK N]`).
//! 4. Calls the LLM to extract entities and relationships.
//! 5. Validates output against approved types and constraints.
//! 6. Writes entities, relationships, and provenance in a single transaction.
//!
//! ## Sole-Writer Contract
//!
//! This module is the sole writer of `kg_subject_entities`,
//! `kg_subject_relationships`, `kg_chunk_subjects`,
//! `kg_chunk_subject_relationships`, and `kg_extractions` rows.
//! No other code path writes these.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use mika_common::llm::{LlmMessage, LlmProvider, LlmRequest, LlmRole};

use crate::async_db::AsyncDatabase;

// ---------------------------------------------------------------------------
// Constants — approved types and relationship constraints
// ---------------------------------------------------------------------------

/// Approved subject entity types.
pub const APPROVED_ENTITY_TYPES: &[&str] = &[
    "skill",
    "tool",
    "agent",
    "problem_type",
    "solution_path",
    "failure_mode",
    "pattern",
];

/// Approved relationship types with from/to type constraints.
pub const APPROVED_RELATIONSHIP_TYPES: &[RelationshipConstraint] = &[
    RelationshipConstraint {
        rel_type: "SOLVED_BY",
        from_types: &["problem_type"],
        to_types: &["solution_path"],
    },
    RelationshipConstraint {
        rel_type: "USES",
        from_types: &["solution_path"],
        to_types: &["skill"],
    },
    RelationshipConstraint {
        rel_type: "CALLS",
        from_types: &["solution_path"],
        to_types: &["tool"],
    },
    RelationshipConstraint {
        rel_type: "INDICATES",
        from_types: &["failure_mode"],
        to_types: &["problem_type"],
    },
    RelationshipConstraint {
        rel_type: "PREVENTS",
        from_types: &["pattern"],
        to_types: &["problem_type"],
    },
    RelationshipConstraint {
        rel_type: "CAUSED_BY",
        from_types: &["problem_type"],
        to_types: &["problem_type"],
    },
    RelationshipConstraint {
        rel_type: "MENTIONS",
        // any type can mention an agent
        from_types: &[
            "skill",
            "tool",
            "agent",
            "problem_type",
            "solution_path",
            "failure_mode",
            "pattern",
        ],
        to_types: &["agent"],
    },
];

/// Relationship type constraint.
pub struct RelationshipConstraint {
    pub rel_type: &'static str,
    pub from_types: &'static [&'static str],
    pub to_types: &'static [&'static str],
}

/// Session ID sentinel for audit events emitted during extraction.
const EXTRACTION_SESSION_ID: &str = "extraction";

// ---------------------------------------------------------------------------
// Extraction output types (D4 — what the LLM produces)
// ---------------------------------------------------------------------------

/// Raw LLM extraction output — deserialized from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionOutput {
    pub entities: Vec<ExtractedEntity>,
    pub relationships: Vec<ExtractedRelationship>,
}

/// A single extracted entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub chunk_indices: Vec<usize>,
    pub confidence: f64,
}

/// A single extracted relationship.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelationship {
    pub from_type: String,
    pub from_name: String,
    pub to_type: String,
    pub to_name: String,
    #[serde(rename = "type")]
    pub rel_type: String,
    pub chunk_indices: Vec<usize>,
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// Validation (Unit 3)
// ---------------------------------------------------------------------------

/// Validation error with details for C2.2 semantic retry reinforcement.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub message: String,
    pub details: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        for detail in &self.details {
            write!(f, "\n  - {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationError {}

/// Validate extraction output against D3/D4 constraints.
///
/// Returns the validated output with any filtered entities/relationships removed,
/// or a `ValidationError` if the output is structurally invalid.
pub fn validate_extraction_output(
    output: &ExtractionOutput,
    max_chunk_index: usize,
) -> Result<ExtractionOutput, ValidationError> {
    let mut errors = Vec::new();
    let mut valid_entities = Vec::new();
    let mut valid_entity_keys: HashSet<String> = HashSet::new();

    // Validate entities
    for (i, entity) in output.entities.iter().enumerate() {
        let mut entity_errors = Vec::new();

        if !APPROVED_ENTITY_TYPES.contains(&entity.entity_type.as_str()) {
            entity_errors.push(format!(
                "entity[{i}]: unapproved type '{}'. Approved: {:?}",
                entity.entity_type, APPROVED_ENTITY_TYPES
            ));
        }

        if entity.name.trim().is_empty() {
            entity_errors.push(format!("entity[{i}]: name is empty"));
        }

        if entity.name.contains(':') {
            entity_errors.push(format!(
                "entity[{i}]: name '{}' contains ':' (breaks entity_key convention)",
                entity.name
            ));
        }

        if entity.chunk_indices.is_empty() {
            entity_errors.push(format!("entity[{i}]: chunk_indices is empty"));
        }

        if entity
            .chunk_indices
            .iter()
            .any(|&idx| idx > max_chunk_index)
        {
            entity_errors.push(format!(
                "entity[{i}]: chunk_indices contains index > max ({max_chunk_index})"
            ));
        }

        if !(0.0..=1.0).contains(&entity.confidence) {
            entity_errors.push(format!(
                "entity[{i}]: confidence {} out of range [0.0, 1.0]",
                entity.confidence
            ));
        }

        if entity_errors.is_empty() {
            let key = format!("{}:{}", entity.entity_type, entity.name);
            valid_entity_keys.insert(key);
            valid_entities.push(entity.clone());
        } else {
            errors.extend(entity_errors);
        }
    }

    // Validate relationships
    let mut valid_relationships = Vec::new();
    for (i, rel) in output.relationships.iter().enumerate() {
        let mut rel_errors = Vec::new();

        // Check relationship type is approved
        let constraint = APPROVED_RELATIONSHIP_TYPES
            .iter()
            .find(|c| c.rel_type == rel.rel_type);

        match constraint {
            None => {
                rel_errors.push(format!(
                    "relationship[{i}]: unapproved type '{}'. Approved: {:?}",
                    rel.rel_type,
                    APPROVED_RELATIONSHIP_TYPES
                        .iter()
                        .map(|c| c.rel_type)
                        .collect::<Vec<_>>()
                ));
            }
            Some(c) => {
                // Check from/to type constraints
                if !c.from_types.contains(&rel.from_type.as_str()) {
                    rel_errors.push(format!(
                        "relationship[{i}]: '{}' from_type '{}' not in {:?}",
                        rel.rel_type, rel.from_type, c.from_types
                    ));
                }
                if !c.to_types.contains(&rel.to_type.as_str()) {
                    rel_errors.push(format!(
                        "relationship[{i}]: '{}' to_type '{}' not in {:?}",
                        rel.rel_type, rel.to_type, c.to_types
                    ));
                }
            }
        }

        // Check that referenced entities exist in the output
        let from_key = format!("{}:{}", rel.from_type, rel.from_name);
        let to_key = format!("{}:{}", rel.to_type, rel.to_name);
        if !valid_entity_keys.contains(&from_key) {
            rel_errors.push(format!(
                "relationship[{i}]: from entity '{from_key}' not in entities list"
            ));
        }
        if !valid_entity_keys.contains(&to_key) {
            rel_errors.push(format!(
                "relationship[{i}]: to entity '{to_key}' not in entities list"
            ));
        }

        if rel.chunk_indices.is_empty() {
            rel_errors.push(format!("relationship[{i}]: chunk_indices is empty"));
        }

        if !(0.0..=1.0).contains(&rel.confidence) {
            rel_errors.push(format!(
                "relationship[{i}]: confidence {} out of range [0.0, 1.0]",
                rel.confidence
            ));
        }

        if rel_errors.is_empty() {
            valid_relationships.push(rel.clone());
        } else {
            errors.extend(rel_errors);
        }
    }

    // If all entities and relationships were rejected, that's a validation error.
    // If some passed and some were filtered, return the valid subset with warnings.
    if valid_entities.is_empty() && !output.entities.is_empty() {
        return Err(ValidationError {
            message: "all entities rejected by validation".to_string(),
            details: errors,
        });
    }

    // Log warnings for filtered items but don't fail
    if !errors.is_empty() {
        warn!(
            filtered_count = errors.len(),
            event = "extraction_validation_filtered",
            "some extracted items filtered by validation"
        );
    }

    Ok(ExtractionOutput {
        entities: valid_entities,
        relationships: valid_relationships,
    })
}

// ---------------------------------------------------------------------------
// Extraction stats
// ---------------------------------------------------------------------------

/// Statistics from a single document extraction.
#[derive(Debug, Clone, Default)]
pub struct ExtractionStats {
    pub entities_extracted: usize,
    pub relationships_extracted: usize,
    pub entities_upserted: usize,
    pub relationships_upserted: usize,
    pub orphans_removed: usize,
}

/// Statistics from a batch extraction run.
#[derive(Debug, Clone, Default)]
pub struct BatchStats {
    pub docs_total: usize,
    pub docs_extracted: usize,
    pub docs_skipped: usize,
    pub docs_failed: usize,
    pub total_entities: usize,
    pub total_relationships: usize,
    pub duration_ms: u64,
    /// True when the batch stopped early because the LLM call budget was
    /// exhausted (#757). Remaining docs stay pending for the next run.
    pub aborted_budget: bool,
    /// Number of LLM calls made against the budget. A `call` here is one
    /// `extract_document` invocation (whole-doc extraction). Retries inside
    /// `call_llm_with_retry` count as the same call for budget purposes.
    pub llm_calls: u32,
}

// ---------------------------------------------------------------------------
// Previous state for re-extraction (D5)
// ---------------------------------------------------------------------------

/// Previous provenance state, captured before re-ingestion.
/// `None` for fresh extraction (first-time or compound hook).
#[derive(Debug, Clone)]
pub struct PreviousState {
    pub entity_ids: Vec<i64>,
    pub relationship_ids: Vec<i64>,
}

// ---------------------------------------------------------------------------
// SubjectExtractor (Units 2, 4, 5)
// ---------------------------------------------------------------------------

/// Sole writer of `kg_subject_entities`, `kg_subject_relationships`,
/// `kg_chunk_subjects`, and `kg_chunk_subject_relationships`.
///
/// All extraction goes through `extract_document()`. Invocation-context-agnostic.
pub struct SubjectExtractor {
    db: AsyncDatabase,
    llm: Arc<dyn LlmProvider>,
    trace_id: String,
    docs_root: PathBuf,
}

impl SubjectExtractor {
    /// Create a new extractor.
    ///
    /// - `db`: async database handle (carries `agent_id` implicitly).
    /// - `llm`: LLM provider for extraction (from `MIKA_KG_EXTRACTION_MODEL`).
    /// - `docs_root`: root directory for docs (for resolving relative paths).
    /// - `trace_id`: optional trace ID for observability.
    pub fn new(
        db: AsyncDatabase,
        llm: Arc<dyn LlmProvider>,
        docs_root: PathBuf,
        trace_id: Option<&str>,
    ) -> Self {
        let trace_id = trace_id
            .map(|s| s.to_owned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace('-', ""));
        Self {
            db,
            llm,
            trace_id,
            docs_root,
        }
    }

    /// Core extraction entry point — invocation-context-agnostic.
    ///
    /// Reads the full doc from disk, builds a prompt with chunk markers,
    /// calls the LLM, validates output, and writes to DB in a single transaction.
    pub async fn extract_document(
        &self,
        doc_path: &str,
        previous_state: Option<PreviousState>,
    ) -> Result<ExtractionStats> {
        let agent_id = self.db.agent_id.clone();

        // 1. Resolve absolute path and read doc from disk.
        let abs_path = self.resolve_doc_path(doc_path);
        let doc_text = std::fs::read_to_string(&abs_path)
            .with_context(|| format!("failed to read doc: {}", abs_path.display()))?;

        if doc_text.trim().is_empty() {
            return Ok(ExtractionStats::default());
        }

        // 2. Read chunk boundaries from kg_chunks.
        let chunks = self.get_chunk_boundaries(doc_path).await?;
        if chunks.is_empty() {
            warn!(
                trace_id = %self.trace_id,
                agent_id = %agent_id,
                doc = %doc_path,
                event = "extraction_no_chunks",
                "no chunks found for doc — skipping extraction"
            );
            return Ok(ExtractionStats::default());
        }

        let max_chunk_index = chunks.len().saturating_sub(1);

        // 3. Build prompt with chunk boundary markers.
        let annotated_text = self.build_annotated_text(&doc_text, &chunks);
        let prompt = self.build_extraction_prompt(&annotated_text);

        // 4. LLM call with C2.2 retry taxonomy.
        let start = Instant::now();
        let extraction_output = self.call_llm_with_retry(&prompt).await?;
        let latency_ms = start.elapsed().as_millis() as u64;

        // 5. Parse and validate output.
        let validated = match extraction_output {
            Some(ref output) => match validate_extraction_output(output, max_chunk_index) {
                Ok(v) => v,
                Err(e) => {
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        doc = %doc_path,
                        error = %e,
                        event = "extraction_validation_failed",
                        "extraction output failed validation — log-and-skip per C2.3"
                    );
                    return Ok(ExtractionStats::default());
                }
            },
            None => {
                // LLM returned nothing usable after retries
                return Ok(ExtractionStats::default());
            }
        };

        let stats = ExtractionStats {
            entities_extracted: validated.entities.len(),
            relationships_extracted: validated.relationships.len(),
            ..Default::default()
        };

        // 6. Store llm_calls row (C2.4).
        if let Some(ref raw_output) = extraction_output {
            self.store_llm_call(doc_path, latency_ms, raw_output).await;
        }

        // 7. Write entities, relationships, provenance in single transaction.
        let final_stats = self
            .write_extraction_results(doc_path, &validated, &chunks, previous_state)
            .await
            .with_context(|| format!("failed to write extraction results for {doc_path}"))?;

        // 8. Emit audit event (D11).
        self.emit_audit_event(doc_path, &final_stats).await;

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            doc = %doc_path,
            entities = final_stats.entities_upserted,
            relationships = final_stats.relationships_upserted,
            orphans_removed = final_stats.orphans_removed,
            latency_ms = latency_ms,
            event = "extraction_complete",
        );

        Ok(ExtractionStats {
            entities_extracted: stats.entities_extracted,
            relationships_extracted: stats.relationships_extracted,
            entities_upserted: final_stats.entities_upserted,
            relationships_upserted: final_stats.relationships_upserted,
            orphans_removed: final_stats.orphans_removed,
        })
    }

    /// Enumerate and extract all pending docs for the agent (D7), capped by
    /// `budget` LLM calls.
    ///
    /// `budget == 0` returns immediately with no LLM calls — the extraction
    /// phase is effectively disabled for this invocation. On overflow the
    /// method aborts cleanly, emits a `kg_budget_exhausted` WARN, and leaves
    /// remaining docs pending for the next run (#757 R2). The aborted flag is
    /// surfaced via `BatchStats::aborted_budget`.
    pub async fn extract_pending(&self, budget: u32) -> Result<BatchStats> {
        let start = Instant::now();
        let agent_id = self.db.agent_id.clone();
        let model_name = self.llm.model_name().to_string();

        // Query pending docs via kg_extractions tracking table.
        let pending_docs = self.get_pending_docs().await?;

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            pending_docs = pending_docs.len(),
            budget = budget,
            event = "subject_extraction_start",
        );

        if pending_docs.is_empty() {
            return Ok(BatchStats {
                duration_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            });
        }

        let mut stats = BatchStats {
            docs_total: pending_docs.len(),
            ..Default::default()
        };

        if budget == 0 {
            warn!(
                trace_id = %self.trace_id,
                agent_id = %agent_id,
                scope = "extraction",
                budget = 0,
                pending_docs = pending_docs.len(),
                event = "kg_budget_exhausted",
                "MIKA_KG_BATCH_BUDGET=0 — skipping extraction (all docs stay pending)"
            );
            stats.aborted_budget = true;
            stats.duration_ms = start.elapsed().as_millis() as u64;
            return Ok(stats);
        }

        for (i, doc_path) in pending_docs.iter().enumerate() {
            // Budget guard: abort cleanly once we've made `budget` LLM calls.
            if stats.llm_calls >= budget {
                warn!(
                    trace_id = %self.trace_id,
                    agent_id = %agent_id,
                    scope = "extraction",
                    calls_made = stats.llm_calls,
                    budget = budget,
                    remaining = pending_docs.len() - i,
                    event = "kg_budget_exhausted",
                    "extraction hit per-batch LLM call cap — remaining docs stay pending for next run"
                );
                stats.aborted_budget = true;
                break;
            }

            // Capture the per-doc hash before extraction so we can persist it
            // alongside the kg_extractions row (#757 R1). If read fails we fall
            // back to None — extraction still happens; the idempotency marker
            // will simply re-fire next run (bounded by the budget).
            let doc_hash = self.get_doc_hash(doc_path).await.unwrap_or_else(|e| {
                warn!(
                    trace_id = %self.trace_id,
                    agent_id = %agent_id,
                    doc = %doc_path,
                    error = %e,
                    event = "extraction_doc_hash_read_failed",
                );
                None
            });

            match self.extract_document(doc_path, None).await {
                Ok(doc_stats) => {
                    stats.docs_extracted += 1;
                    stats.total_entities += doc_stats.entities_upserted;
                    stats.total_relationships += doc_stats.relationships_upserted;

                    // Write kg_extractions tracking row with the hash we consumed.
                    self.record_extraction(
                        doc_path,
                        doc_hash.as_deref(),
                        &model_name,
                        doc_stats.entities_upserted,
                        doc_stats.relationships_upserted,
                    )
                    .await;
                }
                Err(e) => {
                    stats.docs_failed += 1;
                    warn!(
                        trace_id = %self.trace_id,
                        agent_id = %agent_id,
                        doc = %doc_path,
                        error = %e,
                        event = "extraction_doc_failed",
                        "extraction failed — doc stays pending per C2.3"
                    );
                }
            }

            // Every attempted doc debits the budget — a validated-as-empty
            // extraction still spent an LLM call (C2 retry taxonomy runs
            // inside extract_document regardless of outcome).
            stats.llm_calls = stats.llm_calls.saturating_add(1);

            // Progress logging every 10 docs.
            if (i + 1) % 10 == 0 {
                info!(
                    trace_id = %self.trace_id,
                    agent_id = %agent_id,
                    completed = i + 1,
                    remaining = pending_docs.len() - (i + 1),
                    duration_ms = start.elapsed().as_millis() as u64,
                    event = "subject_extraction_progress",
                );
            }
        }

        stats.docs_skipped = stats.docs_total - stats.docs_extracted - stats.docs_failed;
        stats.duration_ms = start.elapsed().as_millis() as u64;

        info!(
            trace_id = %self.trace_id,
            agent_id = %agent_id,
            completed = stats.docs_extracted,
            failed = stats.docs_failed,
            entities = stats.total_entities,
            relationships = stats.total_relationships,
            llm_calls = stats.llm_calls,
            aborted_budget = stats.aborted_budget,
            duration_ms = stats.duration_ms,
            event = "subject_extraction_complete",
        );

        Ok(stats)
    }

    /// Public wrapper for `record_extraction` — used by the ingestion
    /// orchestrator to mark a doc as extracted after compound hook extraction.
    ///
    /// Passes `source_doc_hash = None`; the orchestrator calls
    /// [`Self::get_doc_hash_public`] separately if it needs hash persistence.
    pub async fn record_extraction_public(
        &self,
        doc_path: &str,
        model: &str,
        entities: usize,
        relationships: usize,
    ) {
        let hash = self.get_doc_hash(doc_path).await.unwrap_or(None);
        self.record_extraction(doc_path, hash.as_deref(), model, entities, relationships)
            .await;
    }

    /// Capture previous provenance state before re-ingestion (D5, Phase 1).
    pub async fn capture_previous_state(&self, doc_path: &str) -> Result<PreviousState> {
        let agent_id = self.db.agent_id.clone();
        let doc_path = doc_path.to_owned();

        self.db
            .with_db(move |db| {
                let entity_ids: Vec<i64> = {
                    let mut stmt = db.conn.prepare(
                        "SELECT DISTINCT cs.subject_entity_id
                         FROM kg_chunk_subjects cs
                         JOIN kg_chunks c ON cs.chunk_id = c.id
                         WHERE c.agent_id = ?1 AND c.source_doc_path = ?2",
                    )?;
                    stmt.query_map(rusqlite::params![agent_id, doc_path], |row| row.get(0))?
                        .filter_map(|r| r.ok())
                        .collect()
                };

                let relationship_ids: Vec<i64> = {
                    let mut stmt = db.conn.prepare(
                        "SELECT DISTINCT csr.subject_relationship_id
                         FROM kg_chunk_subject_relationships csr
                         JOIN kg_chunks c ON csr.chunk_id = c.id
                         WHERE c.agent_id = ?1 AND c.source_doc_path = ?2",
                    )?;
                    stmt.query_map(rusqlite::params![agent_id, doc_path], |row| row.get(0))?
                        .filter_map(|r| r.ok())
                        .collect()
                };

                Ok(PreviousState {
                    entity_ids,
                    relationship_ids,
                })
            })
            .await
    }

    // -- Internal helpers --

    /// Resolve a repo-relative doc path to an absolute path.
    fn resolve_doc_path(&self, doc_path: &str) -> PathBuf {
        // docs_root is typically <repo>/docs/solutions. The repo root is
        // two levels up. Doc paths stored in DB are repo-relative.
        if let Some(repo_root) = self.docs_root.parent().and_then(|p| p.parent()) {
            repo_root.join(doc_path)
        } else {
            self.docs_root.join(doc_path)
        }
    }

    /// Get chunk boundaries from kg_chunks for this doc.
    async fn get_chunk_boundaries(&self, doc_path: &str) -> Result<Vec<ChunkBoundary>> {
        let agent_id = self.db.agent_id.clone();
        let doc_path = doc_path.to_owned();

        self.db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT id, seq_id FROM kg_chunks
                     WHERE agent_id = ?1 AND source_doc_path = ?2
                     ORDER BY seq_id ASC",
                )?;
                let chunks: Vec<ChunkBoundary> = stmt
                    .query_map(rusqlite::params![agent_id, doc_path], |row| {
                        Ok(ChunkBoundary {
                            id: row.get(0)?,
                            _seq_id: row.get(1)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(chunks)
            })
            .await
    }

    /// Build annotated text with `[CHUNK N]` markers.
    fn build_annotated_text(&self, doc_text: &str, chunks: &[ChunkBoundary]) -> String {
        // We need to insert chunk markers into the text. Since we don't have
        // exact byte offsets, we use the chunker's algorithm to split and
        // re-join with markers.
        let chunk_texts: Vec<super::chunker::Chunk> = super::chunker::chunk_doc(doc_text);

        let mut annotated = String::with_capacity(doc_text.len() + chunks.len() * 12);
        for (i, chunk) in chunk_texts.iter().enumerate() {
            if i > 0 {
                annotated.push('\n');
            }
            annotated.push_str(&format!("[CHUNK {i}] "));
            annotated.push_str(&chunk.text);
        }

        annotated
    }

    /// Build the extraction prompt.
    fn build_extraction_prompt(&self, annotated_text: &str) -> (String, String) {
        let system = format!(
            r#"You are a knowledge graph extraction agent. Extract named entities and fact triples from the following document.

Return ONLY valid JSON matching this schema:
{{
  "entities": [
    {{
      "type": "<entity_type>",
      "name": "<lowercase_underscore_name>",
      "description": "<brief description>",
      "chunk_indices": [<int>, ...],
      "confidence": <0.0-1.0>
    }}
  ],
  "relationships": [
    {{
      "from_type": "<entity_type>",
      "from_name": "<name>",
      "to_type": "<entity_type>",
      "to_name": "<name>",
      "type": "<relationship_type>",
      "chunk_indices": [<int>, ...],
      "confidence": <0.0-1.0>
    }}
  ]
}}

Approved entity types: {approved_entity_types}
Approved relationship types: {approved_rel_types}

Relationship type constraints:
- SOLVED_BY: from problem_type to solution_path
- USES: from solution_path to skill
- CALLS: from solution_path to tool
- INDICATES: from failure_mode to problem_type
- PREVENTS: from pattern to problem_type
- CAUSED_BY: from problem_type to problem_type
- MENTIONS: from any type to agent

Rules:
- Entity names must be lowercase with underscores (e.g., "state_drift", "self_dev")
- Entity names must NOT contain colons
- Each entity and relationship must reference chunk indices where evidence appears
- Only extract entities and relationships you are confident about
- If the document has no extractable entities, return {{"entities": [], "relationships": []}}
- Return ONLY the JSON object, no markdown fencing, no explanation"#,
            approved_entity_types = APPROVED_ENTITY_TYPES.join(", "),
            approved_rel_types = APPROVED_RELATIONSHIP_TYPES
                .iter()
                .map(|c| c.rel_type)
                .collect::<Vec<_>>()
                .join(", "),
        );

        (system, annotated_text.to_string())
    }

    /// Call the LLM with C2.2 retry taxonomy.
    async fn call_llm_with_retry(
        &self,
        prompt: &(String, String),
    ) -> Result<Option<ExtractionOutput>> {
        let (system, user_text) = prompt;

        let request = LlmRequest {
            model: self.llm.model_name().to_string(),
            system: Some(system.clone()),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: mika_common::llm::LlmContent::Text(user_text.clone()),
            }],
            tools: None,
            max_tokens: self.llm.max_tokens(),
            thinking: None,
        };

        // Attempt 1
        match self.llm.send_message(&request).await {
            Ok(response) => {
                let text = response.text_content();
                match self.parse_extraction_json(&text) {
                    Ok(output) => Ok(Some(output)),
                    Err(parse_err) => {
                        // Semantic failure — one retry with reinforcement (C2.2)
                        warn!(
                            trace_id = %self.trace_id,
                            error = %parse_err,
                            event = "extraction_parse_failed_retry",
                            "malformed JSON from LLM — retrying with reinforcement"
                        );
                        self.retry_with_reinforcement(&request, &text).await
                    }
                }
            }
            Err(e) if is_retryable_error(&e) => {
                // Transport/rate-limit retry
                for attempt in 1..=2 {
                    let backoff = std::time::Duration::from_millis(1000 * 2u64.pow(attempt));
                    tokio::time::sleep(backoff).await;
                    match self.llm.send_message(&request).await {
                        Ok(response) => {
                            let text = response.text_content();
                            return match self.parse_extraction_json(&text) {
                                Ok(output) => Ok(Some(output)),
                                Err(_) => self.retry_with_reinforcement(&request, &text).await,
                            };
                        }
                        Err(retry_err) if is_retryable_error(&retry_err) => continue,
                        Err(retry_err) => {
                            warn!(
                                trace_id = %self.trace_id,
                                error = %retry_err,
                                attempt = attempt + 1,
                                event = "extraction_transport_failed",
                            );
                            return Ok(None);
                        }
                    }
                }
                warn!(
                    trace_id = %self.trace_id,
                    event = "extraction_transport_exhausted",
                    "all transport retries exhausted — log-and-skip"
                );
                Ok(None)
            }
            Err(e) => {
                // Configuration error — do not retry (C2.2)
                warn!(
                    trace_id = %self.trace_id,
                    error = %e,
                    event = "extraction_config_error",
                    "non-retryable LLM error — log-and-skip"
                );
                Ok(None)
            }
        }
    }

    /// Retry with prompt reinforcement after semantic failure.
    async fn retry_with_reinforcement(
        &self,
        original_request: &LlmRequest,
        bad_output: &str,
    ) -> Result<Option<ExtractionOutput>> {
        let reinforcement = format!(
            "Your previous response was not valid JSON. The output was:\n{}\n\n\
             Please return ONLY a valid JSON object with \"entities\" and \"relationships\" arrays. \
             No markdown fencing, no explanation, no text outside the JSON.",
            &bad_output[..bad_output.len().min(500)]
        );

        let mut retry_request = original_request.clone();
        retry_request.messages.push(LlmMessage {
            role: LlmRole::Assistant,
            content: mika_common::llm::LlmContent::Text(bad_output.to_string()),
        });
        retry_request.messages.push(LlmMessage {
            role: LlmRole::User,
            content: mika_common::llm::LlmContent::Text(reinforcement),
        });

        match self.llm.send_message(&retry_request).await {
            Ok(response) => {
                let text = response.text_content();
                match self.parse_extraction_json(&text) {
                    Ok(output) => Ok(Some(output)),
                    Err(e) => {
                        warn!(
                            trace_id = %self.trace_id,
                            error = %e,
                            event = "extraction_semantic_exhausted",
                            "semantic retry also failed — log-and-skip per C2.3"
                        );
                        Ok(None)
                    }
                }
            }
            Err(e) => {
                warn!(
                    trace_id = %self.trace_id,
                    error = %e,
                    event = "extraction_retry_transport_failed",
                );
                Ok(None)
            }
        }
    }

    /// Parse extraction JSON from LLM response text.
    fn parse_extraction_json(&self, text: &str) -> Result<ExtractionOutput> {
        // Strip markdown code fences if present
        let cleaned = text
            .trim()
            .strip_prefix("```json")
            .or_else(|| text.trim().strip_prefix("```"))
            .unwrap_or(text.trim());
        let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

        serde_json::from_str(cleaned).with_context(|| {
            format!(
                "failed to parse extraction JSON: {}",
                &cleaned[..cleaned.len().min(200)]
            )
        })
    }

    /// Write extraction results to DB in a single transaction (D5).
    async fn write_extraction_results(
        &self,
        _doc_path: &str,
        output: &ExtractionOutput,
        chunks: &[ChunkBoundary],
        previous_state: Option<PreviousState>,
    ) -> Result<ExtractionStats> {
        let agent_id = self.db.agent_id.clone();
        let trace_id = self.trace_id.clone();
        let entities = output.entities.clone();
        let relationships = output.relationships.clone();
        let chunk_lookup: HashMap<usize, i64> =
            chunks.iter().enumerate().map(|(i, c)| (i, c.id)).collect();

        self.db
            .with_db(move |db| {
                let tx = db.conn.unchecked_transaction()?;
                let mut stats = ExtractionStats::default();

                // -- UPSERT entities --
                let mut entity_id_map: HashMap<String, i64> = HashMap::new();
                for entity in &entities {
                    let entity_key = format!("{}:{}", entity.entity_type, entity.name);
                    let props = entity
                        .description
                        .as_ref()
                        .map(|d| format!(r#"{{"description":{}}}"#, serde_json::to_string(d).unwrap_or_default()));

                    tx.execute(
                        "INSERT INTO kg_subject_entities (agent_id, entity_key, type, name, confidence, properties_json, trace_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(agent_id, entity_key) DO UPDATE SET
                            confidence = MAX(excluded.confidence, kg_subject_entities.confidence),
                            properties_json = COALESCE(excluded.properties_json, kg_subject_entities.properties_json),
                            trace_id = excluded.trace_id",
                        rusqlite::params![
                            agent_id,
                            entity_key,
                            entity.entity_type,
                            entity.name,
                            entity.confidence,
                            props,
                            trace_id,
                        ],
                    )?;

                    // Get the entity ID (whether inserted or existing).
                    let entity_id: i64 = tx.query_row(
                        "SELECT id FROM kg_subject_entities WHERE agent_id = ?1 AND entity_key = ?2",
                        rusqlite::params![agent_id, entity_key],
                        |row| row.get(0),
                    )?;
                    entity_id_map.insert(entity_key, entity_id);
                    stats.entities_upserted += 1;

                    // -- INSERT chunk-subject provenance --
                    for &chunk_idx in &entity.chunk_indices {
                        if let Some(&chunk_id) = chunk_lookup.get(&chunk_idx) {
                            tx.execute(
                                "INSERT OR IGNORE INTO kg_chunk_subjects
                                    (agent_id, chunk_id, subject_entity_id, extraction_trace_id)
                                 VALUES (?1, ?2, ?3, ?4)",
                                rusqlite::params![agent_id, chunk_id, entity_id, trace_id],
                            )?;
                        }
                    }
                }

                // -- UPSERT relationships --
                for rel in &relationships {
                    let from_key = format!("{}:{}", rel.from_type, rel.from_name);
                    let to_key = format!("{}:{}", rel.to_type, rel.to_name);

                    let from_id = match entity_id_map.get(&from_key) {
                        Some(&id) => id,
                        None => continue, // Entity filtered during validation
                    };
                    let to_id = match entity_id_map.get(&to_key) {
                        Some(&id) => id,
                        None => continue,
                    };

                    tx.execute(
                        "INSERT INTO kg_subject_relationships
                            (agent_id, from_entity_id, to_entity_id, type, confidence, properties_json, trace_id)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         ON CONFLICT(agent_id, from_entity_id, to_entity_id, type) DO UPDATE SET
                            confidence = MAX(excluded.confidence, kg_subject_relationships.confidence),
                            properties_json = COALESCE(excluded.properties_json, kg_subject_relationships.properties_json),
                            trace_id = excluded.trace_id",
                        rusqlite::params![
                            agent_id,
                            from_id,
                            to_id,
                            rel.rel_type,
                            rel.confidence,
                            Option::<String>::None,
                            trace_id,
                        ],
                    )?;

                    let rel_id: i64 = tx.query_row(
                        "SELECT id FROM kg_subject_relationships
                         WHERE agent_id = ?1 AND from_entity_id = ?2 AND to_entity_id = ?3 AND type = ?4",
                        rusqlite::params![agent_id, from_id, to_id, rel.rel_type],
                        |row| row.get(0),
                    )?;
                    stats.relationships_upserted += 1;

                    // -- INSERT chunk-relationship provenance --
                    for &chunk_idx in &rel.chunk_indices {
                        if let Some(&chunk_id) = chunk_lookup.get(&chunk_idx) {
                            tx.execute(
                                "INSERT OR IGNORE INTO kg_chunk_subject_relationships
                                    (agent_id, chunk_id, subject_relationship_id, extraction_trace_id)
                                 VALUES (?1, ?2, ?3, ?4)",
                                rusqlite::params![agent_id, chunk_id, rel_id, trace_id],
                            )?;
                        }
                    }
                }

                // -- Scoped orphan sweep (D5 phase 3, step 7-8) --
                if let Some(prev) = previous_state {
                    // Delete entities from previous set that now have zero provenance
                    if !prev.entity_ids.is_empty() {
                        let placeholders: Vec<String> =
                            prev.entity_ids.iter().map(|_| "?".to_string()).collect();
                        let sql = format!(
                            "DELETE FROM kg_subject_entities
                             WHERE id IN ({placeholders})
                               AND id NOT IN (SELECT subject_entity_id FROM kg_chunk_subjects WHERE agent_id = ?)",
                            placeholders = placeholders.join(",")
                        );
                        let mut stmt = tx.prepare(&sql)?;
                        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = prev
                            .entity_ids
                            .iter()
                            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                            .collect();
                        params.push(Box::new(agent_id.clone()));
                        stats.orphans_removed +=
                            stmt.execute(rusqlite::params_from_iter(params))? as usize;
                    }

                    // Delete relationships from previous set that now have zero provenance
                    if !prev.relationship_ids.is_empty() {
                        let placeholders: Vec<String> = prev
                            .relationship_ids
                            .iter()
                            .map(|_| "?".to_string())
                            .collect();
                        let sql = format!(
                            "DELETE FROM kg_subject_relationships
                             WHERE id IN ({placeholders})
                               AND id NOT IN (SELECT subject_relationship_id FROM kg_chunk_subject_relationships WHERE agent_id = ?)",
                            placeholders = placeholders.join(",")
                        );
                        let mut stmt = tx.prepare(&sql)?;
                        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = prev
                            .relationship_ids
                            .iter()
                            .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                            .collect();
                        params.push(Box::new(agent_id));
                        stats.orphans_removed +=
                            stmt.execute(rusqlite::params_from_iter(params))? as usize;
                    }
                }

                tx.commit()?;
                Ok(stats)
            })
            .await
    }

    /// Get pending docs (D7 + #757 content-hash check).
    ///
    /// A doc is pending when it has `kg_chunks` rows but either has no
    /// `kg_extractions` row, or the row's `source_doc_hash` disagrees with
    /// the chunk's current `source_doc_hash` (which is per-doc — all chunk
    /// rows of a given doc share one hash, see `lexical_ingestor`). The
    /// direct hash-equality predicate catches both the "never extracted"
    /// and "extracted-but-stale" cases without aggregation.
    async fn get_pending_docs(&self) -> Result<Vec<String>> {
        let agent_id = self.db.agent_id.clone();

        self.db
            .with_db(move |db| {
                let mut stmt = db.conn.prepare(
                    "SELECT DISTINCT c.source_doc_path
                     FROM kg_chunks c
                     WHERE c.agent_id = ?1
                       AND NOT EXISTS (
                         SELECT 1 FROM kg_extractions e
                         WHERE e.agent_id        = c.agent_id
                           AND e.source_doc_path = c.source_doc_path
                           AND e.source_doc_hash = c.source_doc_hash
                       )
                     ORDER BY c.source_doc_path",
                )?;
                let paths: Vec<String> = stmt
                    .query_map(rusqlite::params![agent_id], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(paths)
            })
            .await
    }

    /// Read the per-doc `source_doc_hash` from `kg_chunks` (#757).
    ///
    /// Returns `Ok(None)` when no chunks exist for the doc. Callers treat
    /// missing-hash as "write NULL into `kg_extractions.source_doc_hash`,"
    /// which the pending query rejects → the doc re-extracts once next run.
    async fn get_doc_hash(&self, doc_path: &str) -> Result<Option<String>> {
        let agent_id = self.db.agent_id.clone();
        let doc_path = doc_path.to_owned();

        self.db
            .with_db(move |db| {
                use rusqlite::OptionalExtension;
                let hash: Option<String> = db
                    .conn
                    .query_row(
                        "SELECT source_doc_hash
                         FROM kg_chunks
                         WHERE agent_id = ?1 AND source_doc_path = ?2
                         LIMIT 1",
                        rusqlite::params![agent_id, doc_path],
                        |row| row.get(0),
                    )
                    .optional()?;
                Ok(hash)
            })
            .await
    }

    /// Record a successful extraction in kg_extractions (D7 + #757 hash).
    async fn record_extraction(
        &self,
        doc_path: &str,
        source_doc_hash: Option<&str>,
        model: &str,
        entities: usize,
        relationships: usize,
    ) {
        let agent_id = self.db.agent_id.clone();
        let doc_path_owned = doc_path.to_owned();
        let hash_owned = source_doc_hash.map(str::to_owned);
        let model = model.to_owned();
        let trace_id = self.trace_id.clone();

        if let Err(e) = self
            .db
            .with_db(move |db| {
                db.conn.execute(
                    "INSERT INTO kg_extractions
                        (agent_id, source_doc_path, source_doc_hash, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(agent_id, source_doc_path) DO UPDATE SET
                        source_doc_hash = excluded.source_doc_hash,
                        extraction_model = excluded.extraction_model,
                        entities_extracted = excluded.entities_extracted,
                        relationships_extracted = excluded.relationships_extracted,
                        extraction_trace_id = excluded.extraction_trace_id,
                        created_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                    rusqlite::params![agent_id, doc_path_owned, hash_owned, model, entities as i64, relationships as i64, trace_id],
                )?;
                Ok(())
            })
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                error = %e,
                event = "extraction_record_failed",
            );
        }
    }

    /// Store an llm_calls row (C2.4).
    async fn store_llm_call(&self, _doc_path: &str, latency_ms: u64, _output: &ExtractionOutput) {
        let call_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        let provider = self.llm.provider_name().to_string();
        let model = self.llm.model_name().to_string();

        if let Err(e) = self
            .db
            .save_llm_call(
                &call_id,
                EXTRACTION_SESSION_ID,
                Some(&self.trace_id),
                &provider,
                &model,
                0, // input_tokens not available from extraction calls
                0, // output_tokens not available
                None,
                None,
                latency_ms,
                Some("end_turn"),
                "success",
                None,
                0,
                Some("kg_extraction"),
            )
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                error = %e,
                event = "extraction_llm_call_record_failed",
            );
        }
    }

    /// Emit a per-document audit event (C3.3).
    async fn emit_audit_event(&self, doc_path: &str, stats: &ExtractionStats) {
        let target_key = format!("kg_subject:{doc_path}");
        let after_value = format!(
            r#"{{"entities_extracted":{},"relationships_extracted":{},"entities_upserted":{},"relationships_upserted":{},"orphans_removed":{}}}"#,
            stats.entities_extracted,
            stats.relationships_extracted,
            stats.entities_upserted,
            stats.relationships_upserted,
            stats.orphans_removed,
        );

        if let Err(e) = self
            .db
            .log_audit_event(
                EXTRACTION_SESSION_ID,
                "extract_subject_entities",
                &target_key,
                None,
                Some(&after_value),
                Some("subject graph extraction"),
                Some(&self.trace_id),
            )
            .await
        {
            warn!(
                trace_id = %self.trace_id,
                doc = %doc_path,
                error = %e,
                event = "extraction_audit_event_failed",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Helper types and functions
// ---------------------------------------------------------------------------

/// Chunk boundary info from `kg_chunks`.
#[derive(Debug, Clone)]
struct ChunkBoundary {
    id: i64,
    _seq_id: i64,
}

/// Check if an LLM error is retryable (transport/rate-limit).
fn is_retryable_error(e: &mika_common::llm::LlmError) -> bool {
    e.is_retryable()
}

/// Helper trait for extracting text content from LlmResponse.
trait LlmResponseExt {
    fn text_content(&self) -> String;
}

impl LlmResponseExt for mika_common::llm::LlmResponse {
    fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                mika_common::llm::LlmResponseContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_empty_output() {
        let output = ExtractionOutput {
            entities: vec![],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert!(validated.entities.is_empty());
        assert!(validated.relationships.is_empty());
    }

    #[test]
    fn validate_valid_entity() {
        let output = ExtractionOutput {
            entities: vec![ExtractedEntity {
                entity_type: "problem_type".to_string(),
                name: "fabrication".to_string(),
                description: Some("LLM generates ungrounded content".to_string()),
                chunk_indices: vec![0, 2],
                confidence: 0.9,
            }],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().entities.len(), 1);
    }

    #[test]
    fn validate_rejects_unapproved_entity_type() {
        let output = ExtractionOutput {
            entities: vec![ExtractedEntity {
                entity_type: "unknown_type".to_string(),
                name: "test".to_string(),
                description: None,
                chunk_indices: vec![0],
                confidence: 0.8,
            }],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        // All entities rejected → error
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_colon_in_name() {
        let output = ExtractionOutput {
            entities: vec![ExtractedEntity {
                entity_type: "problem_type".to_string(),
                name: "bad:name".to_string(),
                description: None,
                chunk_indices: vec![0],
                confidence: 0.8,
            }],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_empty_chunk_indices() {
        let output = ExtractionOutput {
            entities: vec![ExtractedEntity {
                entity_type: "problem_type".to_string(),
                name: "test".to_string(),
                description: None,
                chunk_indices: vec![],
                confidence: 0.8,
            }],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_out_of_range_confidence() {
        let output = ExtractionOutput {
            entities: vec![ExtractedEntity {
                entity_type: "problem_type".to_string(),
                name: "test".to_string(),
                description: None,
                chunk_indices: vec![0],
                confidence: 1.5,
            }],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_err());
    }

    #[test]
    fn validate_valid_relationship() {
        let output = ExtractionOutput {
            entities: vec![
                ExtractedEntity {
                    entity_type: "problem_type".to_string(),
                    name: "fabrication".to_string(),
                    description: None,
                    chunk_indices: vec![0],
                    confidence: 0.9,
                },
                ExtractedEntity {
                    entity_type: "solution_path".to_string(),
                    name: "grounding_validation".to_string(),
                    description: None,
                    chunk_indices: vec![1],
                    confidence: 0.85,
                },
            ],
            relationships: vec![ExtractedRelationship {
                from_type: "problem_type".to_string(),
                from_name: "fabrication".to_string(),
                to_type: "solution_path".to_string(),
                to_name: "grounding_validation".to_string(),
                rel_type: "SOLVED_BY".to_string(),
                chunk_indices: vec![0, 1],
                confidence: 0.85,
            }],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert_eq!(validated.relationships.len(), 1);
    }

    #[test]
    fn validate_rejects_wrong_relationship_direction() {
        let output = ExtractionOutput {
            entities: vec![
                ExtractedEntity {
                    entity_type: "problem_type".to_string(),
                    name: "fabrication".to_string(),
                    description: None,
                    chunk_indices: vec![0],
                    confidence: 0.9,
                },
                ExtractedEntity {
                    entity_type: "solution_path".to_string(),
                    name: "fix".to_string(),
                    description: None,
                    chunk_indices: vec![1],
                    confidence: 0.85,
                },
            ],
            relationships: vec![ExtractedRelationship {
                // SOLVED_BY should go from problem_type to solution_path, not reversed
                from_type: "solution_path".to_string(),
                from_name: "fix".to_string(),
                to_type: "problem_type".to_string(),
                to_name: "fabrication".to_string(),
                rel_type: "SOLVED_BY".to_string(),
                chunk_indices: vec![0, 1],
                confidence: 0.85,
            }],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_ok());
        // Relationship filtered, entities survive
        let validated = result.unwrap();
        assert_eq!(validated.entities.len(), 2);
        assert_eq!(validated.relationships.len(), 0);
    }

    #[test]
    fn validate_rejects_dangling_relationship() {
        let output = ExtractionOutput {
            entities: vec![ExtractedEntity {
                entity_type: "problem_type".to_string(),
                name: "fabrication".to_string(),
                description: None,
                chunk_indices: vec![0],
                confidence: 0.9,
            }],
            relationships: vec![ExtractedRelationship {
                from_type: "problem_type".to_string(),
                from_name: "fabrication".to_string(),
                to_type: "solution_path".to_string(),
                to_name: "nonexistent".to_string(), // Not in entities
                rel_type: "SOLVED_BY".to_string(),
                chunk_indices: vec![0],
                confidence: 0.85,
            }],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert_eq!(validated.entities.len(), 1);
        assert_eq!(validated.relationships.len(), 0); // Filtered
    }

    #[test]
    fn validate_filters_partial_bad_entities() {
        let output = ExtractionOutput {
            entities: vec![
                ExtractedEntity {
                    entity_type: "problem_type".to_string(),
                    name: "good_entity".to_string(),
                    description: None,
                    chunk_indices: vec![0],
                    confidence: 0.9,
                },
                ExtractedEntity {
                    entity_type: "bad_type".to_string(),
                    name: "bad_entity".to_string(),
                    description: None,
                    chunk_indices: vec![1],
                    confidence: 0.8,
                },
            ],
            relationships: vec![],
        };
        let result = validate_extraction_output(&output, 5);
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert_eq!(validated.entities.len(), 1);
        assert_eq!(validated.entities[0].name, "good_entity");
    }

    #[test]
    fn parse_extraction_json_strips_markdown() {
        // Test the JSON parsing logic directly
        let json = r#"```json
{"entities": [], "relationships": []}
```"#;
        let cleaned = json
            .trim()
            .strip_prefix("```json")
            .or_else(|| json.trim().strip_prefix("```"))
            .unwrap_or(json.trim());
        let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();
        let result: Result<ExtractionOutput, _> = serde_json::from_str(cleaned);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.entities.is_empty());
        assert!(output.relationships.is_empty());
    }

    #[test]
    fn parse_extraction_json_plain() {
        let json = r#"{"entities": [], "relationships": []}"#;
        let result: Result<ExtractionOutput, _> = serde_json::from_str(json);
        assert!(result.is_ok());
    }
}
