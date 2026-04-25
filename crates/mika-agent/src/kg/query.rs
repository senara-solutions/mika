//! # KG Query Engine
//!
//! Read-only graph traversal for the knowledge graph. Implements hybrid entry-path
//! resolution (direct domain match, subject match, semantic search), recursive CTE
//! traversal, result ranking, and agent context enrichment.
//!
//! See `docs/plans/2026-04-21-008-feat-kg-query-tool-plan.md` for design decisions.
//! Key decisions: D1 (parallel entry paths), D2 (traversal algorithm), D3 (ranking),
//! D4 (return shape), D5 (agent context: annotate, don't filter), D6 (compact default).

use std::collections::{HashMap, HashSet};

use rusqlite::OptionalExtension;

use crate::async_db::AsyncDatabase;
use crate::db::kg_schema::KG_CHUNK_SOURCE_TYPE;
use serde::{Deserialize, Serialize};

// ── Types ───────────────────────────────────────────────────────────────────

/// Input to the KG query engine.
#[derive(Debug, Clone, Deserialize)]
pub struct KgQueryInput {
    /// Free-text question — triggers entry-path resolution (paths A-C).
    pub question: Option<String>,
    /// Explicit traversal starting point and parameters.
    pub traversal: Option<TraversalInput>,
    /// Agent ID for scoped queries and context enrichment.
    /// Used for per-agent tables (resolutions) and skill enablement context.
    pub agent_id: Option<String>,
    /// Shared-corpus hash for filtering shared-layer tables (v27).
    /// When set, subject entities, chunks, and provenance tables are scoped
    /// to this corpus. When `None`, shared-layer queries are unscoped.
    /// Deprecated (#798): use `docs_root_hashes` for multi-corpus agents.
    pub docs_root_hash: Option<String>,
    /// Multi-corpus hash filter (#798). When non-empty, shared-layer queries
    /// use `IN (?, ?, ...)` instead of `= ?`. Takes precedence over
    /// `docs_root_hash` when both are set. Empty vec means "no filter".
    #[serde(default)]
    pub docs_root_hashes: Vec<String>,
    /// Include chunk prose text in results (default: false).
    #[serde(default)]
    pub include_context: bool,
    /// Max entities to return (default: 20).
    pub result_limit: Option<usize>,
}

/// Explicit traversal parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct TraversalInput {
    /// Starting entity_key (e.g., "problem_type:ci_failure").
    pub start: Option<String>,
    /// Relationship types to follow (default: SOLVED_BY, PROVIDES, DEPENDS_ON, USES, CALLS).
    pub follow: Option<Vec<String>>,
    /// Max traversal depth (default: 2, cap: 4).
    pub max_depth: Option<u32>,
}

/// Query result returned to the tool.
#[derive(Debug, Clone, Serialize)]
pub struct KgQueryResult {
    pub status: KgQueryStatus,
    pub entries: Vec<KgResultEntry>,
    pub chunks: Vec<KgChunkResult>,
    pub entry_method: Option<String>,
}

/// Three-value status for #692 fallback logic.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum KgQueryStatus {
    Ok,
    StartingEntityMissing,
    TraversalEmpty,
}

/// A single entity in the query result.
#[derive(Debug, Clone, Serialize)]
pub struct KgResultEntry {
    pub entity_key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub layer: String,
    pub hop: u32,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_context: Option<AgentContext>,
    pub edges_out: Vec<KgEdge>,
}

/// An edge from an entity to another.
#[derive(Debug, Clone, Serialize)]
pub struct KgEdge {
    #[serde(rename = "type")]
    pub edge_type: String,
    pub target: String,
    pub confidence: f64,
}

/// Agent-scoped metadata on an entity (D5: annotate, don't filter).
#[derive(Debug, Clone, Serialize)]
pub struct AgentContext {
    pub enabled: bool,
}

/// Chunk text included when `include_context` is true.
#[derive(Debug, Clone, Serialize)]
pub struct KgChunkResult {
    pub chunk_id: i64,
    pub source_doc_path: String,
    pub content: Option<String>,
}

/// Internal representation of a found starting entity.
#[derive(Debug, Clone)]
struct EntryEntity {
    entity_id: i64,
    entity_key: String,
    name: String,
    entity_type: String,
    layer: Layer,
    confidence: f64,
    entry_method: &'static str,
}

/// Internal representation of a traversed entity (post-CTE).
#[derive(Debug, Clone)]
struct TraversedEntity {
    entity_id: i64,
    entity_key: String,
    name: String,
    entity_type: String,
    layer: Layer,
    hop: u32,
    cumulative_confidence: f64,
}

/// Internal representation of a traversed edge.
#[derive(Debug, Clone)]
struct TraversedEdge {
    from_entity_id: i64,
    #[allow(dead_code)]
    to_entity_id: i64,
    to_entity_key: String,
    edge_type: String,
    confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Layer {
    Domain,
    Subject,
}

impl Layer {
    fn as_str(&self) -> &'static str {
        match self {
            Layer::Domain => "domain",
            Layer::Subject => "subject",
        }
    }
}

// ── Default constants ───────────────────────────────────────────────────────

const DEFAULT_FOLLOW: &[&str] = &["SOLVED_BY", "PROVIDES", "DEPENDS_ON", "USES", "CALLS"];
const DEFAULT_MAX_DEPTH: u32 = 2;
const MAX_DEPTH_CAP: u32 = 4;
const DEFAULT_ENTRY_TOP_K: usize = 5;
const MAX_RESULT_ENTITIES: usize = 20;
const MAX_EDGES: usize = 30;
const MAX_CHUNKS: usize = 10;

// ── Public API ──────────────────────────────────────────────────────────────

/// Execute a KG query. Read-only — no mutations.
pub async fn query_knowledge_graph(
    db: &AsyncDatabase,
    input: &KgQueryInput,
) -> anyhow::Result<KgQueryResult> {
    // Validate: exactly one of question or traversal.start
    let has_question = input
        .question
        .as_ref()
        .is_some_and(|q| !q.trim().is_empty());
    let has_start = input
        .traversal
        .as_ref()
        .and_then(|t| t.start.as_ref())
        .is_some_and(|s| !s.trim().is_empty());

    if has_question && has_start {
        anyhow::bail!("Exactly one of 'question' or 'traversal.start' must be provided, not both");
    }
    if !has_question && !has_start {
        anyhow::bail!("Either 'question' or 'traversal.start' must be provided");
    }

    let follow_types = input
        .traversal
        .as_ref()
        .and_then(|t| t.follow.as_ref())
        .map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .unwrap_or_else(|| DEFAULT_FOLLOW.to_vec());

    let max_depth = input
        .traversal
        .as_ref()
        .and_then(|t| t.max_depth)
        .unwrap_or(DEFAULT_MAX_DEPTH)
        .min(MAX_DEPTH_CAP);

    let result_limit = input
        .result_limit
        .unwrap_or(MAX_RESULT_ENTITIES)
        .min(MAX_RESULT_ENTITIES);

    let agent_id = input.agent_id.as_deref();
    // Effective docs_root_hash: prefer multi-corpus list, fall back to singular.
    // For internal use, we keep the Option<&str> API for back-compat with all
    // the existing query functions. Multi-corpus IN-list support is the next step.
    let docs_root_hash = if !input.docs_root_hashes.is_empty() {
        // For now, the query functions accept Option<&str>. When multi-corpus
        // is fully wired, they'll accept &[String]. First corpus is used for
        // back-compat with single-corpus callers while the IN-list migration
        // proceeds incrementally.
        input.docs_root_hashes.first().map(|s| s.as_str())
    } else {
        input.docs_root_hash.as_deref()
    };

    // Phase 1: Find starting entities
    let (starting_entities, entry_method) = if has_start {
        let start_key = input
            .traversal
            .as_ref()
            .unwrap()
            .start
            .as_ref()
            .unwrap()
            .trim();
        let entities = find_by_entity_key(db, start_key, docs_root_hash).await?;
        let method = if entities.is_empty() {
            None
        } else {
            Some(entities[0].entry_method)
        };
        (entities, method)
    } else {
        let question = input.question.as_ref().unwrap().trim();
        resolve_entry_paths(db, question, agent_id, docs_root_hash).await?
    };

    if starting_entities.is_empty() {
        return Ok(KgQueryResult {
            status: KgQueryStatus::StartingEntityMissing,
            entries: vec![],
            chunks: vec![],
            entry_method: None,
        });
    }

    // Phase 2: If max_depth == 0, return starting entities only
    if max_depth == 0 {
        let mut entries = Vec::new();
        for e in starting_entities.iter().take(result_limit) {
            let ctx = if let Some(aid) = agent_id {
                enrich_agent_context(db, &e.entity_key, &e.entity_type, aid).await?
            } else {
                None
            };
            entries.push(build_result_entry(e, 0, e.confidence, ctx, vec![]));
        }

        return Ok(KgQueryResult {
            status: KgQueryStatus::Ok,
            entries,
            chunks: vec![],
            entry_method: entry_method.map(|s| s.to_string()),
        });
    }

    // Phase 3: Graph traversal
    let (traversed, edges) = traverse_graph(
        db,
        &starting_entities,
        &follow_types,
        max_depth,
        docs_root_hash,
    )
    .await?;

    let has_traversed_neighbors = traversed.iter().any(|e| e.hop > 0);
    if !has_traversed_neighbors {
        // Starting entities found but no edges to follow
        let entries = starting_entities
            .iter()
            .take(result_limit)
            .map(|e| build_result_entry(e, 0, e.confidence, None, vec![]))
            .collect();

        return Ok(KgQueryResult {
            status: KgQueryStatus::TraversalEmpty,
            entries,
            chunks: vec![],
            entry_method: entry_method.map(|s| s.to_string()),
        });
    }

    // Phase 4: Rank and budget results
    let ranked = rank_and_budget(traversed, result_limit);

    // Phase 5: Build edges_out per entity
    let entries = build_entries_with_edges(&ranked, &edges, agent_id, db).await?;

    // Phase 6: Optionally include chunk context
    let chunks = if input.include_context {
        collect_chunks(db, &ranked, docs_root_hash).await?
    } else {
        vec![]
    };

    Ok(KgQueryResult {
        status: KgQueryStatus::Ok,
        entries,
        chunks,
        entry_method: entry_method.map(|s| s.to_string()),
    })
}

// ── Entry Path Resolution (Unit 1) ─────────────────────────────────────────

/// Find starting entities by exact entity_key lookup.
async fn find_by_entity_key(
    db: &AsyncDatabase,
    entity_key: &str,
    docs_root_hash: Option<&str>,
) -> anyhow::Result<Vec<EntryEntity>> {
    let key = entity_key.to_owned();
    let drh = docs_root_hash.map(|s| s.to_owned());
    let mut results = Vec::new();

    // Path A: domain entity exact match
    let domain_key = key.clone();
    let domain_results: Vec<(i64, String, String, String)> = db
        .with_db(move |db| {
            let conn = &db.conn;
            let mut stmt = conn.prepare(
                "SELECT id, entity_key, name, type FROM kg_entities \
                 WHERE LOWER(entity_key) = LOWER(?1)",
            )?;
            let rows = stmt
                .query_map([&domain_key], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;

    for (id, ek, name, etype) in domain_results {
        results.push(EntryEntity {
            entity_id: id,
            entity_key: ek,
            name,
            entity_type: etype,
            layer: Layer::Domain,
            confidence: 1.0,
            entry_method: "direct_entity_key",
        });
    }

    // Path B: subject entity exact match (if docs_root_hash provided)
    if let Some(ref hash) = drh {
        let key_b = key.clone();
        let hash_b = hash.clone();
        let subject_results: Vec<(i64, String, String, String, f64)> = db
            .with_db(move |db| {
                let conn = &db.conn;
                let mut stmt = conn.prepare(
                    "SELECT id, entity_key, name, type, confidence FROM kg_subject_entities \
                     WHERE docs_root_hash = ?1 AND LOWER(entity_key) = LOWER(?2)",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![&hash_b, &key_b], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, f64>(4)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;

        for (id, ek, name, etype, conf) in subject_results {
            results.push(EntryEntity {
                entity_id: id,
                entity_key: ek,
                name,
                entity_type: etype,
                layer: Layer::Subject,
                confidence: conf,
                entry_method: "direct_entity_key",
            });
        }
    }

    Ok(results)
}

/// Resolve entry paths A-C in parallel, merge and dedupe.
async fn resolve_entry_paths(
    db: &AsyncDatabase,
    question: &str,
    agent_id: Option<&str>,
    docs_root_hash: Option<&str>,
) -> anyhow::Result<(Vec<EntryEntity>, Option<&'static str>)> {
    // Run paths A, B, C and merge
    let mut all_entries = Vec::new();

    // Path A: Direct domain entity match (case-insensitive LIKE on name and entity_key)
    let path_a = find_domain_entities_by_name(db, question).await?;
    all_entries.extend(path_a);

    // Path B: Subject entity match (if docs_root_hash)
    if let Some(drh) = docs_root_hash {
        let path_b = find_subject_entities_by_name(db, question, drh).await?;
        all_entries.extend(path_b);
    }

    // Path C: Semantic search via chunks -> chunk_subjects -> subject entities -> resolve
    let path_c = find_entities_via_chunks(db, question, docs_root_hash, agent_id).await?;
    all_entries.extend(path_c);

    if all_entries.is_empty() {
        return Ok((vec![], None));
    }

    // Dedupe: keep highest confidence per (layer, entity_id)
    let mut seen: HashMap<(Layer, i64), usize> = HashMap::new();
    let mut deduped: Vec<EntryEntity> = Vec::new();

    for entry in all_entries {
        let key = (entry.layer, entry.entity_id);
        if let Some(&idx) = seen.get(&key) {
            if entry.confidence > deduped[idx].confidence {
                deduped[idx] = entry;
            }
        } else {
            seen.insert(key, deduped.len());
            deduped.push(entry);
        }
    }

    // Sort by confidence descending, take top-K
    deduped.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    deduped.truncate(DEFAULT_ENTRY_TOP_K);

    let method = deduped.first().map(|e| e.entry_method);
    Ok((deduped, method))
}

/// Path A: Find domain entities whose name or entity_key matches the query.
async fn find_domain_entities_by_name(
    db: &AsyncDatabase,
    question: &str,
) -> anyhow::Result<Vec<EntryEntity>> {
    let q = question.to_lowercase();
    let q_like = format!("%{q}%");
    let q_for_closure = q.clone();

    let rows: Vec<(i64, String, String, String)> = db
        .with_db(move |db| {
            let conn = &db.conn;
            let mut stmt = conn.prepare(
                "SELECT id, entity_key, name, type FROM kg_entities \
                 WHERE LOWER(name) LIKE ?1 OR LOWER(entity_key) = ?2 \
                 LIMIT 20",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![&q_like, &q_for_closure], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, ek, name, etype)| {
            let conf = if ek.to_lowercase() == q || name.to_lowercase() == q {
                1.0
            } else {
                0.8
            };
            EntryEntity {
                entity_id: id,
                entity_key: ek,
                name,
                entity_type: etype,
                layer: Layer::Domain,
                confidence: conf,
                entry_method: "path_a_direct_match",
            }
        })
        .collect())
}

/// Path B: Find subject entities whose name or entity_key matches the query.
async fn find_subject_entities_by_name(
    db: &AsyncDatabase,
    question: &str,
    docs_root_hash: &str,
) -> anyhow::Result<Vec<EntryEntity>> {
    let q = question.to_lowercase();
    let q_like = format!("%{q}%");
    let q_for_closure = q.clone();
    let drh = docs_root_hash.to_owned();

    let rows: Vec<(i64, String, String, String, f64)> = db
        .with_db(move |db| {
            let conn = &db.conn;
            let mut stmt = conn.prepare(
                "SELECT id, entity_key, name, type, confidence FROM kg_subject_entities \
                 WHERE docs_root_hash = ?1 AND (LOWER(name) LIKE ?2 OR LOWER(entity_key) = ?3) \
                 LIMIT 20",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![&drh, &q_like, &q_for_closure], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, f64>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, ek, name, etype, conf)| {
            let entry_conf = if ek.to_lowercase() == q || name.to_lowercase() == q {
                0.9 * conf
            } else {
                0.7 * conf
            };
            EntryEntity {
                entity_id: id,
                entity_key: ek,
                name,
                entity_type: etype,
                layer: Layer::Subject,
                confidence: entry_conf,
                entry_method: "path_b_subject_match",
            }
        })
        .collect())
}

/// Path C: Find entities via semantic search on KG chunks.
async fn find_entities_via_chunks(
    db: &AsyncDatabase,
    question: &str,
    docs_root_hash: Option<&str>,
    agent_id: Option<&str>,
) -> anyhow::Result<Vec<EntryEntity>> {
    // Use hybrid search scoped to kg_chunk source type
    let search_results = db
        .hybrid_search(question, None, 10, Some(KG_CHUNK_SOURCE_TYPE))
        .await
        .unwrap_or_default();

    if search_results.is_empty() {
        return Ok(vec![]);
    }

    // Get chunk IDs from search results (source_id is the kg_chunks.id)
    let chunk_ids: Vec<i64> = search_results.iter().filter_map(|r| r.source_id).collect();

    if chunk_ids.is_empty() {
        return Ok(vec![]);
    }

    // Find subject entities linked to these chunks via kg_chunk_subjects
    let drh = docs_root_hash.map(|s| s.to_owned());
    let chunk_scores: HashMap<i64, f64> = search_results
        .iter()
        .filter_map(|r| r.source_id.map(|id| (id, r.score)))
        .collect();

    let subject_entities: Vec<(i64, String, String, String, f64, i64)> = db
        .with_db(move |db| {
            let conn = &db.conn;
            let placeholders: String = chunk_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT DISTINCT se.id, se.entity_key, se.name, se.type, se.confidence, cs.chunk_id \
                 FROM kg_chunk_subjects cs \
                 JOIN kg_subject_entities se ON se.id = cs.subject_entity_id \
                 WHERE cs.chunk_id IN ({placeholders}){hash_filter}",
                hash_filter = if drh.is_some() {
                    " AND cs.docs_root_hash = ?"
                } else {
                    ""
                },
            );
            let mut stmt = conn.prepare(&sql)?;

            let mut param_idx = 1;
            for cid in &chunk_ids {
                stmt.raw_bind_parameter(param_idx, cid)?;
                param_idx += 1;
            }
            if let Some(ref h) = drh {
                stmt.raw_bind_parameter(param_idx, h.as_str())?;
            }

            let mut rows = Vec::new();
            let mut raw_rows = stmt.raw_query();
            while let Some(row) = raw_rows.next()? {
                rows.push((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, i64>(5)?,
                ));
            }
            Ok(rows)
        })
        .await?;

    let mut entries = Vec::new();
    let mut seen_subject_ids: HashSet<i64> = HashSet::new();

    for (se_id, se_key, se_name, se_type, se_conf, chunk_id) in &subject_entities {
        if !seen_subject_ids.insert(*se_id) {
            continue;
        }
        let search_score = chunk_scores.get(chunk_id).copied().unwrap_or(0.5);

        // Try to resolve subject entity to domain via kg_subject_resolutions
        let resolved = resolve_subject_to_domain(db, *se_id, agent_id).await?;

        if let Some((domain_id, domain_key, domain_name, domain_type, res_conf)) = resolved {
            entries.push(EntryEntity {
                entity_id: domain_id,
                entity_key: domain_key,
                name: domain_name,
                entity_type: domain_type,
                layer: Layer::Domain,
                confidence: search_score * res_conf * se_conf,
                entry_method: "path_c_semantic_search",
            });
        } else {
            entries.push(EntryEntity {
                entity_id: *se_id,
                entity_key: se_key.clone(),
                name: se_name.clone(),
                entity_type: se_type.clone(),
                layer: Layer::Subject,
                confidence: search_score * se_conf,
                entry_method: "path_c_semantic_search",
            });
        }
    }

    Ok(entries)
}

/// Resolve a subject entity to its domain counterpart via kg_subject_resolutions.
async fn resolve_subject_to_domain(
    db: &AsyncDatabase,
    subject_entity_id: i64,
    agent_id: Option<&str>,
) -> anyhow::Result<Option<(i64, String, String, String, f64)>> {
    let aid = agent_id.map(|s| s.to_owned());
    let se_id = subject_entity_id;

    db.with_db(move |db| {
        let conn = &db.conn;
        let mut stmt = conn.prepare(
            "SELECT e.id, e.entity_key, e.name, e.type, r.confidence \
             FROM kg_subject_resolutions r \
             JOIN kg_entities e ON e.id = r.domain_entity_id \
             WHERE r.subject_entity_id = ?1 \
             AND (?2 IS NULL OR r.agent_id = ?2) \
             ORDER BY r.confidence DESC \
             LIMIT 1",
        )?;
        let result = stmt
            .query_row(rusqlite::params![se_id, &aid], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            })
            .optional()?;
        Ok(result)
    })
    .await
}

// ── Graph Traversal (Unit 2) ────────────────────────────────────────────────

/// Traverse the graph from starting entities via recursive CTE.
async fn traverse_graph(
    db: &AsyncDatabase,
    starting: &[EntryEntity],
    follow_types: &[&str],
    max_depth: u32,
    docs_root_hash: Option<&str>,
) -> anyhow::Result<(Vec<TraversedEntity>, Vec<TraversedEdge>)> {
    let mut all_entities: Vec<TraversedEntity> = Vec::new();
    let mut all_edges: Vec<TraversedEdge> = Vec::new();
    let mut seen_entity_ids: HashSet<(Layer, i64)> = HashSet::new();

    // Add starting entities at hop 0
    for entry in starting {
        if seen_entity_ids.insert((entry.layer, entry.entity_id)) {
            all_entities.push(TraversedEntity {
                entity_id: entry.entity_id,
                entity_key: entry.entity_key.clone(),
                name: entry.name.clone(),
                entity_type: entry.entity_type.clone(),
                layer: entry.layer,
                hop: 0,
                cumulative_confidence: entry.confidence,
            });
        }
    }

    // Traverse domain layer edges
    let domain_starts: Vec<(i64, f64)> = starting
        .iter()
        .filter(|e| e.layer == Layer::Domain)
        .map(|e| (e.entity_id, e.confidence))
        .collect();

    if !domain_starts.is_empty() {
        let (entities, edges) =
            traverse_domain_layer(db, &domain_starts, follow_types, max_depth).await?;
        for e in entities {
            if seen_entity_ids.insert((Layer::Domain, e.entity_id)) {
                all_entities.push(e);
            }
        }
        all_edges.extend(edges);
    }

    // Traverse subject layer edges (if docs_root_hash provided)
    if let Some(drh) = docs_root_hash {
        let subject_starts: Vec<(i64, f64)> = starting
            .iter()
            .filter(|e| e.layer == Layer::Subject)
            .map(|e| (e.entity_id, e.confidence))
            .collect();

        if !subject_starts.is_empty() {
            let (entities, edges) =
                traverse_subject_layer(db, &subject_starts, follow_types, max_depth, drh).await?;
            for e in entities {
                if seen_entity_ids.insert((Layer::Subject, e.entity_id)) {
                    all_entities.push(e);
                }
            }
            all_edges.extend(edges);
        }
    }

    Ok((all_entities, all_edges))
}

/// Traverse domain layer using recursive CTE on kg_relationships.
async fn traverse_domain_layer(
    db: &AsyncDatabase,
    starts: &[(i64, f64)],
    follow_types: &[&str],
    max_depth: u32,
) -> anyhow::Result<(Vec<TraversedEntity>, Vec<TraversedEdge>)> {
    let start_ids: Vec<i64> = starts.iter().map(|(id, _)| *id).collect();
    let types: Vec<String> = follow_types.iter().map(|s| s.to_string()).collect();
    let depth = max_depth;

    db.with_db(move |db| {
        let conn = &db.conn;

        // Build the recursive CTE
        let id_placeholders: String = start_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let type_placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        // All parameters use positional ? — bind order: start_ids..., depth, types...
        let sql = format!(
            "WITH RECURSIVE traverse(entity_id, hop, cumulative_conf, path) AS ( \
                SELECT id, 0, 1.0, CAST(id AS TEXT) \
                FROM kg_entities WHERE id IN ({id_placeholders}) \
              UNION ALL \
                SELECT r.to_entity_id, t.hop + 1, t.cumulative_conf * 1.0, \
                       t.path || ',' || CAST(r.to_entity_id AS TEXT) \
                FROM kg_relationships r \
                JOIN traverse t ON t.entity_id = r.from_entity_id \
                WHERE t.hop < ? \
                  AND r.type IN ({type_placeholders}) \
                  AND INSTR(',' || t.path || ',', ',' || CAST(r.to_entity_id AS TEXT) || ',') = 0 \
            ) \
            SELECT DISTINCT t.entity_id, t.hop, t.cumulative_conf, \
                   e.entity_key, e.name, e.type \
            FROM traverse t \
            JOIN kg_entities e ON e.id = t.entity_id \
            WHERE t.hop > 0 \
            ORDER BY t.hop ASC, t.cumulative_conf DESC"
        );

        let mut stmt = conn.prepare(&sql)?;

        let mut param_idx = 1;
        for id in &start_ids {
            stmt.raw_bind_parameter(param_idx, id)?;
            param_idx += 1;
        }
        stmt.raw_bind_parameter(param_idx, depth)?;
        param_idx += 1;
        for t in &types {
            stmt.raw_bind_parameter(param_idx, t.as_str())?;
            param_idx += 1;
        }

        let mut entities = Vec::new();
        let mut raw_rows = stmt.raw_query();
        while let Some(row) = raw_rows.next()? {
            entities.push(TraversedEntity {
                entity_id: row.get::<_, i64>(0)?,
                hop: row.get::<_, u32>(1)?,
                cumulative_confidence: row.get::<_, f64>(2)?,
                entity_key: row.get::<_, String>(3)?,
                name: row.get::<_, String>(4)?,
                entity_type: row.get::<_, String>(5)?,
                layer: Layer::Domain,
            });
        }
        drop(raw_rows);
        drop(stmt);

        // Fetch edges
        let all_entity_ids: Vec<i64> = {
            let mut ids: Vec<i64> = start_ids.clone();
            ids.extend(entities.iter().map(|e| e.entity_id));
            ids.sort();
            ids.dedup();
            ids
        };

        let eid_placeholders: String = all_entity_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let edge_sql = format!(
            "SELECT r.from_entity_id, r.to_entity_id, e.entity_key, r.type \
             FROM kg_relationships r \
             JOIN kg_entities e ON e.id = r.to_entity_id \
             WHERE r.from_entity_id IN ({eid_placeholders}) \
               AND r.to_entity_id IN ({eid_placeholders}) \
               AND r.type IN ({type_placeholders})"
        );

        let mut stmt = conn.prepare(&edge_sql)?;
        let mut param_idx = 1;
        for id in &all_entity_ids {
            stmt.raw_bind_parameter(param_idx, id)?;
            param_idx += 1;
        }
        for id in &all_entity_ids {
            stmt.raw_bind_parameter(param_idx, id)?;
            param_idx += 1;
        }
        for t in &types {
            stmt.raw_bind_parameter(param_idx, t.as_str())?;
            param_idx += 1;
        }

        let mut edges = Vec::new();
        let mut raw_rows = stmt.raw_query();
        while let Some(row) = raw_rows.next()? {
            edges.push(TraversedEdge {
                from_entity_id: row.get::<_, i64>(0)?,
                to_entity_id: row.get::<_, i64>(1)?,
                to_entity_key: row.get::<_, String>(2)?,
                edge_type: row.get::<_, String>(3)?,
                confidence: 1.0, // domain edges don't have confidence
            });
        }

        Ok((entities, edges))
    })
    .await
}

/// Traverse subject layer using recursive CTE on kg_subject_relationships.
async fn traverse_subject_layer(
    db: &AsyncDatabase,
    starts: &[(i64, f64)],
    follow_types: &[&str],
    max_depth: u32,
    docs_root_hash: &str,
) -> anyhow::Result<(Vec<TraversedEntity>, Vec<TraversedEdge>)> {
    let start_ids: Vec<i64> = starts.iter().map(|(id, _)| *id).collect();
    let types: Vec<String> = follow_types.iter().map(|s| s.to_string()).collect();
    let depth = max_depth;
    let drh = docs_root_hash.to_owned();

    db.with_db(move |db| {
        let conn = &db.conn;

        let id_placeholders: String = start_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let type_placeholders: String = types.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        // All parameters use positional ? — bind order: start_ids..., drh, depth, types..., drh
        let sql = format!(
            "WITH RECURSIVE traverse(entity_id, hop, cumulative_conf, path) AS ( \
                SELECT id, 0, confidence, CAST(id AS TEXT) \
                FROM kg_subject_entities WHERE id IN ({id_placeholders}) AND docs_root_hash = ? \
              UNION ALL \
                SELECT r.to_entity_id, t.hop + 1, t.cumulative_conf * r.confidence, \
                       t.path || ',' || CAST(r.to_entity_id AS TEXT) \
                FROM kg_subject_relationships r \
                JOIN traverse t ON t.entity_id = r.from_entity_id \
                WHERE t.hop < ? \
                  AND r.type IN ({type_placeholders}) \
                  AND r.docs_root_hash = ? \
                  AND INSTR(',' || t.path || ',', ',' || CAST(r.to_entity_id AS TEXT) || ',') = 0 \
            ) \
            SELECT DISTINCT t.entity_id, t.hop, t.cumulative_conf, \
                   e.entity_key, e.name, e.type \
            FROM traverse t \
            JOIN kg_subject_entities e ON e.id = t.entity_id \
            WHERE t.hop > 0 \
            ORDER BY t.hop ASC, t.cumulative_conf DESC"
        );

        let mut stmt = conn.prepare(&sql)?;

        let mut param_idx = 1;
        for id in &start_ids {
            stmt.raw_bind_parameter(param_idx, id)?;
            param_idx += 1;
        }
        stmt.raw_bind_parameter(param_idx, drh.as_str())?;
        param_idx += 1;
        stmt.raw_bind_parameter(param_idx, depth)?;
        param_idx += 1;
        for t in &types {
            stmt.raw_bind_parameter(param_idx, t.as_str())?;
            param_idx += 1;
        }
        stmt.raw_bind_parameter(param_idx, drh.as_str())?;

        let mut entities = Vec::new();
        let mut raw_rows = stmt.raw_query();
        while let Some(row) = raw_rows.next()? {
            entities.push(TraversedEntity {
                entity_id: row.get::<_, i64>(0)?,
                hop: row.get::<_, u32>(1)?,
                cumulative_confidence: row.get::<_, f64>(2)?,
                entity_key: row.get::<_, String>(3)?,
                name: row.get::<_, String>(4)?,
                entity_type: row.get::<_, String>(5)?,
                layer: Layer::Subject,
            });
        }
        drop(raw_rows);
        drop(stmt);

        // Fetch edges
        let all_entity_ids: Vec<i64> = {
            let mut ids: Vec<i64> = start_ids.clone();
            ids.extend(entities.iter().map(|e| e.entity_id));
            ids.sort();
            ids.dedup();
            ids
        };

        let eid_placeholders: String = all_entity_ids
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let edge_sql = format!(
            "SELECT r.from_entity_id, r.to_entity_id, e.entity_key, r.type, r.confidence \
             FROM kg_subject_relationships r \
             JOIN kg_subject_entities e ON e.id = r.to_entity_id \
             WHERE r.from_entity_id IN ({eid_placeholders}) \
               AND r.to_entity_id IN ({eid_placeholders}) \
               AND r.type IN ({type_placeholders}) \
               AND r.docs_root_hash = ?"
        );

        let mut stmt = conn.prepare(&edge_sql)?;
        let mut param_idx = 1;
        for id in &all_entity_ids {
            stmt.raw_bind_parameter(param_idx, id)?;
            param_idx += 1;
        }
        for id in &all_entity_ids {
            stmt.raw_bind_parameter(param_idx, id)?;
            param_idx += 1;
        }
        for t in &types {
            stmt.raw_bind_parameter(param_idx, t.as_str())?;
            param_idx += 1;
        }
        stmt.raw_bind_parameter(param_idx, drh.as_str())?;

        let mut edges = Vec::new();
        let mut raw_rows = stmt.raw_query();
        while let Some(row) = raw_rows.next()? {
            edges.push(TraversedEdge {
                from_entity_id: row.get::<_, i64>(0)?,
                to_entity_id: row.get::<_, i64>(1)?,
                to_entity_key: row.get::<_, String>(2)?,
                edge_type: row.get::<_, String>(3)?,
                confidence: row.get::<_, f64>(4)?,
            });
        }

        Ok((entities, edges))
    })
    .await
}

// ── Ranking and Budgeting (Unit 3) ──────────────────────────────────────────

/// Rank results by (hop ASC, cumulative_confidence DESC).
/// Apply result budget caps.
fn rank_and_budget(mut entities: Vec<TraversedEntity>, limit: usize) -> Vec<TraversedEntity> {
    // Lexicographic sort: hop ascending, then confidence descending
    entities.sort_by(|a, b| {
        a.hop.cmp(&b.hop).then_with(|| {
            b.cumulative_confidence
                .partial_cmp(&a.cumulative_confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    entities.truncate(limit.min(MAX_RESULT_ENTITIES));
    entities
}

// ── Agent Context Enrichment (Unit 4) ───────────────────────────────────────

/// Enrich entities with agent context (skill enablement state).
async fn enrich_agent_context(
    db: &AsyncDatabase,
    entity_key: &str,
    entity_type: &str,
    agent_id: &str,
) -> anyhow::Result<Option<AgentContext>> {
    if entity_type != "skill" {
        return Ok(None);
    }

    // Extract skill name from entity_key "skill:<name>"
    let skill_name = entity_key.strip_prefix("skill:").unwrap_or(entity_key);

    let name = skill_name.to_owned();
    let aid = agent_id.to_owned();

    let enabled: bool = db
        .with_db(move |db| {
            let conn = &db.conn;
            let result: Option<Option<i64>> = conn
                .query_row(
                    "SELECT enabled FROM skill_overrides \
                     WHERE skill_name = ?1 AND agent_id = ?2",
                    rusqlite::params![&name, &aid],
                    |row| row.get(0),
                )
                .optional()?;

            // Tri-state: NULL -> default enabled (true), 0 -> disabled, 1 -> enabled
            // No row -> default enabled (true)
            Ok(!matches!(result, Some(Some(0))))
        })
        .await?;

    Ok(Some(AgentContext { enabled }))
}

// ── Result Assembly ─────────────────────────────────────────────────────────

fn build_result_entry(
    entry: &EntryEntity,
    hop: u32,
    confidence: f64,
    agent_context: Option<AgentContext>,
    edges_out: Vec<KgEdge>,
) -> KgResultEntry {
    KgResultEntry {
        entity_key: entry.entity_key.clone(),
        name: entry.name.clone(),
        entity_type: entry.entity_type.clone(),
        layer: entry.layer.as_str().to_string(),
        hop,
        confidence,
        agent_context,
        edges_out,
    }
}

/// Build KgResultEntry list with edges_out and agent context.
async fn build_entries_with_edges(
    entities: &[TraversedEntity],
    edges: &[TraversedEdge],
    agent_id: Option<&str>,
    db: &AsyncDatabase,
) -> anyhow::Result<Vec<KgResultEntry>> {
    // Group edges by from_entity_id
    let mut edges_by_from: HashMap<i64, Vec<&TraversedEdge>> = HashMap::new();
    for edge in edges {
        edges_by_from
            .entry(edge.from_entity_id)
            .or_default()
            .push(edge);
    }

    let mut results = Vec::new();
    let mut edge_count = 0;

    for entity in entities {
        let entity_edges: Vec<KgEdge> = edges_by_from
            .get(&entity.entity_id)
            .map(|es| {
                es.iter()
                    .take(MAX_EDGES.saturating_sub(edge_count))
                    .map(|e| {
                        edge_count += 1;
                        KgEdge {
                            edge_type: e.edge_type.clone(),
                            target: e.to_entity_key.clone(),
                            confidence: e.confidence,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let agent_context = if let Some(aid) = agent_id {
            enrich_agent_context(db, &entity.entity_key, &entity.entity_type, aid).await?
        } else {
            None
        };

        results.push(KgResultEntry {
            entity_key: entity.entity_key.clone(),
            name: entity.name.clone(),
            entity_type: entity.entity_type.clone(),
            layer: entity.layer.as_str().to_string(),
            hop: entity.hop,
            confidence: entity.cumulative_confidence,
            agent_context,
            edges_out: entity_edges,
        });
    }

    Ok(results)
}

/// Collect chunk context when include_context is true.
async fn collect_chunks(
    db: &AsyncDatabase,
    entities: &[TraversedEntity],
    docs_root_hash: Option<&str>,
) -> anyhow::Result<Vec<KgChunkResult>> {
    // Find subject entity IDs in the result set
    let subject_entity_ids: Vec<i64> = entities
        .iter()
        .filter(|e| e.layer == Layer::Subject)
        .map(|e| e.entity_id)
        .collect();

    if subject_entity_ids.is_empty() {
        return Ok(vec![]);
    }

    let drh = docs_root_hash.map(|s| s.to_owned());

    let chunks: Vec<KgChunkResult> = db
        .with_db(move |db| {
            let conn = &db.conn;
            let placeholders: String = subject_entity_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");

            let sql = format!(
                "SELECT DISTINCT c.id, c.source_doc_path, sc.content \
                 FROM kg_chunk_subjects cs \
                 JOIN kg_chunks c ON c.id = cs.chunk_id \
                 LEFT JOIN search_content sc ON sc.source_type = 'kg_chunk' AND sc.source_id = c.id \
                 WHERE cs.subject_entity_id IN ({placeholders}){hash_filter} \
                 LIMIT ?",
                hash_filter = if drh.is_some() {
                    " AND cs.docs_root_hash = ?"
                } else {
                    ""
                },
            );

            let mut stmt = conn.prepare(&sql)?;
            let mut param_idx = 1;
            for id in &subject_entity_ids {
                stmt.raw_bind_parameter(param_idx, id)?;
                param_idx += 1;
            }
            if let Some(ref h) = drh {
                stmt.raw_bind_parameter(param_idx, h.as_str())?;
                param_idx += 1;
            }
            stmt.raw_bind_parameter(param_idx, MAX_CHUNKS as i64)?;

            let mut results = Vec::new();
            let mut raw_rows = stmt.raw_query();
            while let Some(row) = raw_rows.next()? {
                results.push(KgChunkResult {
                    chunk_id: row.get::<_, i64>(0)?,
                    source_doc_path: row.get::<_, String>(1)?,
                    content: row.get::<_, Option<String>>(2)?,
                });
            }
            Ok(results)
        })
        .await?;

    Ok(chunks)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::async_db::AsyncDatabase;
    use crate::db::Database;

    /// Create an in-memory AsyncDatabase with a test agent.
    fn test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        AsyncDatabase::new_with_agent(db, "mika")
    }

    /// Seed domain entities and relationships for tests.
    async fn seed_domain_graph(db: &AsyncDatabase) {
        db.with_db(|db| {
            let conn = &db.conn;

            conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('problem_type:ci_failure', 'problem_type', 'ci_failure')",
                [],
            )?;
            let ci_failure_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('skill:build-mika', 'skill', 'build-mika')",
                [],
            )?;
            let build_mika_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('tool:run_gh', 'tool', 'run_gh')",
                [],
            )?;
            let run_gh_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (?1, ?2, 'SOLVED_BY')",
                rusqlite::params![ci_failure_id, build_mika_id],
            )?;
            conn.execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (?1, ?2, 'PROVIDES')",
                rusqlite::params![build_mika_id, run_gh_id],
            )?;

            Ok(())
        })
        .await
        .unwrap();
    }

    /// Test docs_root_hash constant for inline tests.
    const TEST_DRH: &str = "0000000000000000";
    const TEST_DR: &str = "/test/docs/solutions";

    /// Seed subject entities and relationships for tests.
    async fn seed_subject_graph(db: &AsyncDatabase) {
        db.with_db(|db| {
            let conn = &db.conn;

            conn.execute(
                "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES (?1, ?2, 'solution_path:webhook_ci_handler', 'solution_path', 'webhook_ci_handler', 0.85)",
                rusqlite::params![TEST_DRH, TEST_DR],
            )?;
            let sp_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES (?1, ?2, 'failure_mode:flaky_test', 'failure_mode', 'flaky_test', 0.9)",
                rusqlite::params![TEST_DRH, TEST_DR],
            )?;
            let fm_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO kg_subject_relationships (docs_root_hash, docs_root, from_entity_id, to_entity_id, type, confidence) \
                 VALUES (?1, ?2, ?3, ?4, 'SOLVED_BY', 0.8)",
                rusqlite::params![TEST_DRH, TEST_DR, sp_id, fm_id],
            )?;

            Ok(())
        })
        .await
        .unwrap();
    }

    // ── Unit 1: Entry path resolution tests ─────────────────────────────

    #[tokio::test]
    async fn test_path_a_exact_domain_match() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: Some("ci_failure".to_string()),
            traversal: None,
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        assert!(!result.entries.is_empty());
        assert_eq!(result.entries[0].entity_key, "problem_type:ci_failure");
        assert_eq!(result.entry_method.as_deref(), Some("path_a_direct_match"));
    }

    #[tokio::test]
    async fn test_path_a_like_match() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: Some("ci".to_string()),
            traversal: None,
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        // Should find ci_failure via LIKE %ci%
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "problem_type:ci_failure")
        );
    }

    #[tokio::test]
    async fn test_path_b_subject_match() {
        let db = test_db();
        seed_subject_graph(&db).await;

        let input = KgQueryInput {
            question: Some("webhook_ci_handler".to_string()),
            traversal: None,
            agent_id: Some("mika".to_string()),
            docs_root_hash: Some(TEST_DRH.to_string()),
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "solution_path:webhook_ci_handler")
        );
    }

    #[tokio::test]
    async fn test_direct_entity_key_lookup() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(1),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "problem_type:ci_failure")
        );
    }

    #[tokio::test]
    async fn test_all_paths_miss_returns_starting_entity_missing() {
        let db = test_db();
        // No KG data seeded

        let input = KgQueryInput {
            question: Some("nonexistent_entity".to_string()),
            traversal: None,
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::StartingEntityMissing);
        assert!(result.entries.is_empty());
    }

    #[tokio::test]
    async fn test_dedup_same_entity_keeps_highest_confidence() {
        let db = test_db();
        seed_domain_graph(&db).await;

        // "build-mika" will match both by name (path A) and potentially path C
        // At minimum, path A should find it
        let input = KgQueryInput {
            question: Some("build-mika".to_string()),
            traversal: None,
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        // Should have deduped — only one entry for build-mika
        let build_entries: Vec<_> = result
            .entries
            .iter()
            .filter(|e| e.entity_key == "skill:build-mika")
            .collect();
        assert_eq!(build_entries.len(), 1);
    }

    // ── Unit 2: Graph traversal tests ───────────────────────────────────

    #[tokio::test]
    async fn test_traversal_depth_1() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(1),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        // Should have ci_failure (hop 0) and build-mika (hop 1)
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "problem_type:ci_failure" && e.hop == 0)
        );
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "skill:build-mika" && e.hop == 1)
        );
        // run_gh should NOT be at depth 1 (it's 2 hops away)
        assert!(
            !result
                .entries
                .iter()
                .any(|e| e.entity_key == "tool:run_gh" && e.hop == 1)
        );
    }

    #[tokio::test]
    async fn test_traversal_depth_2() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(2),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        // Now run_gh should appear at hop 2
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "tool:run_gh" && e.hop == 2)
        );
    }

    #[tokio::test]
    async fn test_edge_type_filter() {
        let db = test_db();
        seed_domain_graph(&db).await;

        // Only follow PROVIDES — ci_failure has SOLVED_BY, not PROVIDES
        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: Some(vec!["PROVIDES".to_string()]),
                max_depth: Some(2),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        // Should be traversal_empty since ci_failure has no PROVIDES edges
        assert_eq!(result.status, KgQueryStatus::TraversalEmpty);
    }

    #[tokio::test]
    async fn test_subject_layer_traversal() {
        let db = test_db();
        seed_subject_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("solution_path:webhook_ci_handler".to_string()),
                follow: Some(vec!["SOLVED_BY".to_string()]),
                max_depth: Some(1),
            }),
            agent_id: Some("mika".to_string()),
            docs_root_hash: Some(TEST_DRH.to_string()),
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        assert!(
            result
                .entries
                .iter()
                .any(|e| e.entity_key == "failure_mode:flaky_test")
        );
    }

    #[tokio::test]
    async fn test_max_depth_zero_returns_starting_only() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(0),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].entity_key, "problem_type:ci_failure");
    }

    #[tokio::test]
    async fn test_entity_found_no_edges_returns_traversal_empty() {
        let db = test_db();
        // Create an isolated entity with no relationships
        db.with_db(|db| {
            db.conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('agent:lonely', 'agent', 'lonely')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("agent:lonely".to_string()),
                follow: None,
                max_depth: Some(2),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::TraversalEmpty);
    }

    // ── Unit 3: Ranking tests ───────────────────────────────────────────

    #[tokio::test]
    async fn test_ranking_distance_first() {
        // Entities closer to entry point should rank higher
        let entities = vec![
            TraversedEntity {
                entity_id: 1,
                entity_key: "a:far".to_string(),
                name: "far".to_string(),
                entity_type: "a".to_string(),
                layer: Layer::Domain,
                hop: 2,
                cumulative_confidence: 1.0,
            },
            TraversedEntity {
                entity_id: 2,
                entity_key: "a:near".to_string(),
                name: "near".to_string(),
                entity_type: "a".to_string(),
                layer: Layer::Domain,
                hop: 1,
                cumulative_confidence: 0.5,
            },
        ];

        let ranked = rank_and_budget(entities, 20);
        assert_eq!(ranked[0].entity_key, "a:near"); // hop 1 before hop 2
        assert_eq!(ranked[1].entity_key, "a:far");
    }

    #[tokio::test]
    async fn test_ranking_confidence_within_same_hop() {
        let entities = vec![
            TraversedEntity {
                entity_id: 1,
                entity_key: "a:low".to_string(),
                name: "low".to_string(),
                entity_type: "a".to_string(),
                layer: Layer::Domain,
                hop: 1,
                cumulative_confidence: 0.5,
            },
            TraversedEntity {
                entity_id: 2,
                entity_key: "a:high".to_string(),
                name: "high".to_string(),
                entity_type: "a".to_string(),
                layer: Layer::Domain,
                hop: 1,
                cumulative_confidence: 0.9,
            },
        ];

        let ranked = rank_and_budget(entities, 20);
        assert_eq!(ranked[0].entity_key, "a:high"); // higher confidence first
    }

    #[tokio::test]
    async fn test_result_budget_limits() {
        let mut entities = Vec::new();
        for i in 0..30 {
            entities.push(TraversedEntity {
                entity_id: i,
                entity_key: format!("a:e{i}"),
                name: format!("e{i}"),
                entity_type: "a".to_string(),
                layer: Layer::Domain,
                hop: 0,
                cumulative_confidence: 1.0,
            });
        }

        let ranked = rank_and_budget(entities, 20);
        assert!(ranked.len() <= MAX_RESULT_ENTITIES);
    }

    // ── Unit 4: Agent context enrichment tests ──────────────────────────

    #[tokio::test]
    async fn test_agent_context_skill_enabled_by_default() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("skill:build-mika".to_string()),
                follow: None,
                max_depth: Some(0),
            }),
            agent_id: Some("mika".to_string()),
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);

        let skill_entry = result
            .entries
            .iter()
            .find(|e| e.entity_key == "skill:build-mika")
            .unwrap();
        assert!(skill_entry.agent_context.is_some());
        assert!(skill_entry.agent_context.as_ref().unwrap().enabled);
    }

    #[tokio::test]
    async fn test_agent_context_skill_disabled() {
        let db = test_db();
        seed_domain_graph(&db).await;

        // Disable the skill via skill_overrides
        db.with_db(|db| {
            db.conn.execute(
                "INSERT INTO skill_overrides (agent_id, skill_name, enabled) VALUES ('mika', 'build-mika', 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("skill:build-mika".to_string()),
                follow: None,
                max_depth: Some(0),
            }),
            agent_id: Some("mika".to_string()),
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();

        let skill_entry = result
            .entries
            .iter()
            .find(|e| e.entity_key == "skill:build-mika")
            .unwrap();
        assert!(skill_entry.agent_context.is_some());
        assert!(!skill_entry.agent_context.as_ref().unwrap().enabled);
    }

    #[tokio::test]
    async fn test_no_agent_context_without_agent_id() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("skill:build-mika".to_string()),
                follow: None,
                max_depth: Some(0),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();

        let skill_entry = result
            .entries
            .iter()
            .find(|e| e.entity_key == "skill:build-mika")
            .unwrap();
        assert!(skill_entry.agent_context.is_none());
    }

    #[tokio::test]
    async fn test_no_agent_context_for_non_skill_entities() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(0),
            }),
            agent_id: Some("mika".to_string()),
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();

        let entry = &result.entries[0];
        assert_eq!(entry.entity_type, "problem_type");
        assert!(entry.agent_context.is_none());
    }

    // ── Unit 5: Validation tests ────────────────────────────────────────

    #[tokio::test]
    async fn test_both_question_and_start_errors() {
        let db = test_db();

        let input = KgQueryInput {
            question: Some("test".to_string()),
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: None,
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Exactly one"));
    }

    #[tokio::test]
    async fn test_neither_question_nor_start_errors() {
        let db = test_db();

        let input = KgQueryInput {
            question: None,
            traversal: None,
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be provided"));
    }

    #[tokio::test]
    async fn test_max_depth_capped_at_4() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(100), // should be capped to 4
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        // Should not error — just caps at 4
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
    }

    // ── Edges in results tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_edges_out_populated() {
        let db = test_db();
        seed_domain_graph(&db).await;

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("problem_type:ci_failure".to_string()),
                follow: None,
                max_depth: Some(2),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();

        let ci_entry = result
            .entries
            .iter()
            .find(|e| e.entity_key == "problem_type:ci_failure")
            .unwrap();
        assert!(!ci_entry.edges_out.is_empty());
        assert!(
            ci_entry
                .edges_out
                .iter()
                .any(|e| e.edge_type == "SOLVED_BY" && e.target == "skill:build-mika")
        );
    }

    // ── Cycle detection tests ──────────────────────────────────────────

    #[tokio::test]
    async fn test_cycle_detection_prevents_infinite_loop() {
        let db = test_db();
        db.with_db(|db| {
            let conn = &db.conn;
            conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('a:one', 'a', 'one')",
                [],
            )?;
            let id_one = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('a:two', 'a', 'two')",
                [],
            )?;
            let id_two = conn.last_insert_rowid();
            // Create a cycle: one -> two -> one
            conn.execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (?1, ?2, 'USES')",
                rusqlite::params![id_one, id_two],
            )?;
            conn.execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (?1, ?2, 'USES')",
                rusqlite::params![id_two, id_one],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("a:one".to_string()),
                follow: Some(vec!["USES".to_string()]),
                max_depth: Some(4),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        // Should have exactly 2 entities (one + two), not infinite
        assert_eq!(result.entries.len(), 2);
    }

    #[tokio::test]
    async fn test_cycle_detection_no_substring_false_positives() {
        // Regression test: entity ID 2 should not be falsely blocked when
        // path contains "12" (INSTR("12", "2") != 0 with naive check)
        let db = test_db();
        db.with_db(|db| {
            let conn = &db.conn;
            // Insert entities with IDs that will be sequential
            conn.execute(
                "INSERT INTO kg_entities (id, entity_key, type, name) VALUES (11, 'a:eleven', 'a', 'eleven')",
                [],
            )?;
            conn.execute(
                "INSERT INTO kg_entities (id, entity_key, type, name) VALUES (12, 'a:twelve', 'a', 'twelve')",
                [],
            )?;
            conn.execute(
                "INSERT INTO kg_entities (id, entity_key, type, name) VALUES (2, 'a:two', 'a', 'two')",
                [],
            )?;
            // eleven -> twelve -> two (path "11,12" should NOT block entity 2)
            conn.execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (11, 12, 'USES')",
                [],
            )?;
            conn.execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (12, 2, 'USES')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let input = KgQueryInput {
            question: None,
            traversal: Some(TraversalInput {
                start: Some("a:eleven".to_string()),
                follow: Some(vec!["USES".to_string()]),
                max_depth: Some(2),
            }),
            agent_id: None,
            docs_root_hash: None,
            docs_root_hashes: vec![],
            include_context: false,
            result_limit: None,
        };
        let result = query_knowledge_graph(&db, &input).await.unwrap();
        assert_eq!(result.status, KgQueryStatus::Ok);
        // Entity 2 must be reachable despite path "11,12" containing substring "2"
        assert!(
            result.entries.iter().any(|e| e.entity_key == "a:two"),
            "Entity 'a:two' (ID 2) should not be blocked by substring match in path '11,12'"
        );
        assert_eq!(result.entries.len(), 3); // eleven, twelve, two
    }
}
