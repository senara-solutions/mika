pub mod kg_schema;
pub mod operational;

use anyhow::{Context, Result};
use chrono::{Duration, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Once;
use tracing::{debug, info, warn};
use utoipa::ToSchema;

pub use crate::evidence::AuditEvent;
pub use crate::task_state::tasks::*;
use crate::timestamp;

/// Register sqlite-vec as an auto-extension so every new connection gets vec0.
pub fn init_sqlite_vec() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(
            #[allow(clippy::missing_transmute_annotations)]
            std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()),
        ));
    });
}

pub const CURRENT_SCHEMA_VERSION: i64 = 44;

/// mika#1742 Problem B: refuse-to-zombie grace window for
/// [`Database::create_recurring_task_if_absent`]. Recent same-label recurring
/// rows in `'failed' | 'cancelled' | 'expired'` states within this window
/// block fresh registration and log a WARN. Grace elapses → next startup
/// re-registers automatically. Tunable at compile-time; expose via env var
/// only if operator experience surfaces a real need.
pub const RECURRING_ZOMBIE_GRACE_HOURS: u32 = 24;

/// SQLite `strftime` modifier form of [`RECURRING_ZOMBIE_GRACE_HOURS`]. Kept
/// as a `&str` const so the query stays a bindable parameter.
pub const RECURRING_ZOMBIE_GRACE_SQL: &str = "-24 hours";

/// SQL for the unified_timeline VIEW — cross-subsystem event correlation.
/// Used in both clean-slate schema creation and incremental migration.
const UNIFIED_TIMELINE_VIEW_SQL: &str = "\
    CREATE VIEW IF NOT EXISTS unified_timeline AS \
    SELECT trace_id, session_id, agent_id, 'message' AS event_type, \
        role AS event_subtype, \
        CASE WHEN length(content) > 200 THEN substr(content, 1, 200) || '...' \
             ELSE content END AS summary, \
        created_at \
    FROM messages \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'audit' AS event_type, \
        tool_name AS event_subtype, \
        target_key || ': ' || COALESCE(before_value, '(none)') || ' -> ' || COALESCE(after_value, '(none)') AS summary, \
        created_at \
    FROM audit_events \
    UNION ALL \
    SELECT COALESCE(execution_trace_id, created_trace_id) AS trace_id, created_by_session AS session_id, agent_id, \
        'task' AS event_type, action_type AS event_subtype, \
        label || ' [' || status || ']' AS summary, \
        created_at \
    FROM tasks \
    UNION ALL \
    SELECT trace_id, 'team-' || run_id AS session_id, NULL AS agent_id, \
        'team_workspace' AS event_type, entry_type AS event_subtype, \
        CASE WHEN length(content) > 200 THEN substr(content, 1, 200) || '...' \
             ELSE content END AS summary, \
        created_at \
    FROM team_workspace \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'llm_call' AS event_type, \
        provider || '/' || model AS event_subtype, \
        'tokens: ' || input_tokens || ' -> ' || output_tokens || ' (' || latency_ms || 'ms, ' || status || ')' AS summary, \
        created_at \
    FROM llm_calls \
    UNION ALL \
    SELECT trace_id, session_id, agent_id, 'tool_call' AS event_type, \
        tool_name AS event_subtype, \
        CASE WHEN success THEN 'ok' ELSE 'err' END || ' (' || latency_ms || 'ms)' AS summary, \
        created_at \
    FROM tool_calls";

/// Check if an anyhow error is a SQLite UNIQUE constraint violation.
pub fn is_unique_violation(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(e, _)) = err.downcast_ref::<rusqlite::Error>() {
        e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    } else {
        false
    }
}

pub const COMMITMENT_STATUSES: &[&str] = &["pending", "completed", "cancelled"];

pub const CORE_MEMORY_SECTIONS: &[(&str, &str)] = &[
    ("user_summary", "No information about the user yet."),
    ("self_model", "No interaction history yet."),
    ("current_priorities", "No priorities set yet."),
    ("key_people", "No people tracked yet."),
    (
        "workflows",
        "Delegate-then-forget is not allowed. Any work sent to Claude Code must have a \
         corresponding task created first (via create_task). No exceptions.",
    ),
];

pub fn core_memory_section_names() -> Vec<&'static str> {
    CORE_MEMORY_SECTIONS.iter().map(|(k, _)| *k).collect()
}

/// Returns the default self_model string for a given agent display name.
/// Centralises the format so callers don't duplicate the template.
pub fn default_self_model(display_name: &str) -> String {
    format!("I am {display_name}. No interaction history yet.")
}

// ===== Public Types =====

#[derive(Debug, Clone)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub home_dir: String,
    pub active: bool,
    pub last_seen: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TeamRow {
    pub id: String,
    pub name: String,
    pub config_path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub metadata: Option<String>,
    pub parent_session_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub agent_id: String,
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub metadata: Option<String>,
    pub trace_id: Option<String>,
    pub created_at: String,
    /// Whether this message is internal (agent-to-agent) and should be hidden from inbox mode.
    pub internal: bool,
}

/// A message row from the `task_messages` parallel narrative table (mika#974).
/// Compaction-immune — survives `replace_with_summary` indefinitely.
#[derive(Debug, Clone)]
pub struct TaskMessage {
    pub id: i64,
    pub task_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub metadata: Option<String>,
    pub trace_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoreMemoryEntry {
    pub key: String,
    pub value: String,
    pub token_count: i32,
    pub updated_at: String,
}

/// A structured fact for the dashboard (aggregated from people, commitments, preferences, events).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DashboardFact {
    pub id: i64,
    pub category: String,
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Person {
    pub id: i64,
    pub canonical_name: String,
    pub relationship: Option<String>,
    pub notes: Option<String>,
    pub first_mentioned: String,
    pub last_mentioned: String,
    pub mention_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Commitment {
    pub id: i64,
    pub description: String,
    pub status: String,
    pub due_date: Option<String>,
    pub person_id: Option<i64>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Preference {
    pub category: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Event {
    pub id: i64,
    pub description: String,
    pub event_date: Option<String>,
    pub context: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct FailedSend {
    pub id: i64,
    pub text: String,
    pub request_id: Option<String>,
    pub created_at: String,
    pub retry_count: i32,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
    pub source_type: String,
    pub source_id: Option<i64>,
    pub content: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamRunRow {
    pub id: String,
    pub team_name: String,
    pub goal: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub iteration: u32,
    pub max_iterations: u32,
    pub deliverable: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub trace_id: Option<String>,
}

// ===== Dashboard Filter Types =====

/// Filters for paginated task listing (dashboard API).
#[derive(Debug, Clone, Default)]
pub struct TaskFilters {
    pub status: Option<String>,
    pub trigger_type: Option<String>,
    pub action_type: Option<String>,
    pub agent_id: Option<String>,
    pub team_run_id_filter: Option<TeamRunIdFilter>,
    pub source: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// How to filter tasks by team_run_id.
#[derive(Debug, Clone)]
pub enum TeamRunIdFilter {
    /// team_run_id IS NULL
    Null,
    /// team_run_id IS NOT NULL
    NotNull,
    /// team_run_id = specific value
    Specific(String),
}

/// Filters for paginated team run listing (dashboard API).
#[derive(Debug, Clone, Default)]
pub struct TeamRunFilters {
    pub team_name: Option<String>,
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamWorkspaceEntry {
    pub id: i64,
    pub run_id: String,
    pub parent_id: Option<i64>,
    pub agent_name: Option<String>,
    pub entry_type: String,
    pub content: String,
    pub iteration: u32,
    pub created_at: String,
}

// ===== Team Run Summary Types =====

/// Summary of an agent's response from a previous team run.
#[derive(Debug, Clone, Serialize)]
pub struct AgentResultSummary {
    pub agent_name: String,
    pub response_preview: String,
}

/// Summary of a task's status from a previous team run.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStatusSummary {
    pub agent_id: String,
    pub label: String,
    pub status: String,
    pub task_id: String,
}

/// Enriched summary of a previous team run for context injection.
#[derive(Debug, Clone, Serialize)]
pub struct TeamRunSummary {
    pub run: TeamRunRow,
    pub agent_results: Vec<AgentResultSummary>,
    pub task_statuses: Vec<TaskStatusSummary>,
    pub pending_tasks: Vec<TaskStatusSummary>,
    pub critic_feedback: Option<String>,
}

// ===== Skill Override Types =====

/// A user override for a skill property (persists across bundled skill re-sync).
#[derive(Debug, Clone, Default)]
pub struct SkillOverride {
    pub skill_name: String,
    pub always_on: Option<bool>,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    /// Tri-state: `None` = default (enabled), `Some(false)` = disabled,
    /// `Some(true)` = explicitly enabled.
    pub enabled: Option<bool>,
    /// Lifecycle state for agent-authored skills: `staged`, `active`, `archived`.
    /// `None` for bundled/marketplace skills (they don't go through the lifecycle).
    pub lifecycle_state: Option<String>,
    /// Number of turns this skill was injected into.
    pub use_count: i64,
    /// ISO 8601 timestamp of the last turn this skill was injected into.
    pub last_used_at: Option<String>,
}

// ===== Observability Types =====

#[derive(Debug, Clone, Serialize)]
pub struct LlmCallRow {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub trace_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub latency_ms: u64,
    pub stop_reason: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub step: u32,
    /// JSON map of skill names to resolved prompt variant descriptors.
    /// `None` when no skills contributed prompts to this turn.
    pub prompt_variant: Option<String>,
    pub created_at: String,
    /// Serialized LLM response text (stripped of internal tags, capped at 50K chars).
    /// `None` for pre-v31 rows or error calls.
    pub response_text: Option<String>,
    /// Extended thinking / reasoning text (Claude-only).
    /// `None` when the provider does not support reasoning or the call errored.
    pub reasoning: Option<String>,
    /// Whether `response_text` is present (non-NULL) in the database.
    /// Set by list queries via `response_text IS NOT NULL`; detail queries derive from presence.
    pub has_response_text: bool,
    /// Whether `reasoning` is present (non-NULL) in the database.
    /// Set by list queries via `reasoning IS NOT NULL`; detail queries derive from presence.
    pub has_reasoning: bool,
    /// Estimated cost in USD, computed from token counts and provider pricing.
    /// Set to `Some` by `enrich_llm_calls_with_cost()` in dashboard handlers;
    /// `None` only for internal (non-API) use before enrichment.
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRow {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub trace_id: Option<String>,
    pub llm_call_id: Option<String>,
    pub step: u32,
    pub tool_name: String,
    pub tool_source: String,
    pub skill_name: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub success: bool,
    pub non_zero_exit: bool,
    pub latency_ms: u64,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct LlmCallFilters {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub model: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostTrendBucket {
    pub timestamp: String,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub call_count: u64,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CostTrendResponse {
    pub buckets: Vec<CostTrendBucket>,
    pub bucket_size: String,
    pub has_estimated_pricing: bool,
    pub estimated_models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CostTrendFilters {
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub bucket: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ToolCallFilters {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub tool_name: Option<String>,
    pub success: Option<bool>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Keyword search in input/output fields (uses SQL LIKE).
    pub keyword: Option<String>,
}

// ===== Dashboard Types =====

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineRow {
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub event_type: String,
    pub event_subtype: String,
    pub summary: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct TimelineFilters {
    pub agent_id: Option<String>,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl TimelineFilters {
    /// Build a WHERE clause and parameter values from the filters.
    /// Uses `rusqlite::types::Value` to preserve native types (integers pass as integers).
    pub(crate) fn to_sql(&self) -> (String, Vec<rusqlite::types::Value>) {
        let mut conditions = Vec::new();
        let mut params: Vec<rusqlite::types::Value> = Vec::new();

        if let Some(ref aid) = self.agent_id {
            params.push(rusqlite::types::Value::Text(aid.clone()));
            conditions.push(format!("agent_id = ?{}", params.len()));
        }
        if let Some(ref et) = self.event_type {
            params.push(rusqlite::types::Value::Text(et.clone()));
            conditions.push(format!("event_type = ?{}", params.len()));
        }
        if let Some(ref tid) = self.trace_id {
            params.push(rusqlite::types::Value::Text(tid.clone()));
            conditions.push(format!("trace_id = ?{}", params.len()));
        }
        if let Some(ref sid) = self.session_id {
            params.push(rusqlite::types::Value::Text(sid.clone()));
            conditions.push(format!("session_id = ?{}", params.len()));
        }
        if let Some(ref from) = self.from {
            params.push(rusqlite::types::Value::Text(from.clone()));
            conditions.push(format!("created_at >= ?{}", params.len()));
        }
        if let Some(ref to) = self.to {
            params.push(rusqlite::types::Value::Text(to.clone()));
            conditions.push(format!("created_at <= ?{}", params.len()));
        }

        let clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        (clause, params)
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentWithStats {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub home_dir: String,
    pub active: bool,
    pub last_seen: Option<String>,
    pub created_at: String,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionWithStats {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub metadata: Option<String>,
    pub task_id: Option<String>,
    pub message_count: i64,
}

/// A session row enriched with task label for the task-sessions endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct TaskSessionRow {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub task_id: Option<String>,
    pub message_count: i64,
    pub task_label: Option<String>,
}

// ===== v27 coalesce SQL (module-level for integration test access) =====

/// Generate the SQL that coalesces v26 backup table rows into the new v27
/// shared-corpus tables. Executed inside the existing migration transaction.
///
/// `docs_root` must already have single-quotes escaped (`'` → `''`).
/// `docs_root_hash` is the 16-hex-char SHA-256 prefix from `kg::config::hash_docs_root`.
pub fn v27_coalesce_sql(docs_root: &str, docs_root_hash: &str) -> String {
    format!(
        "-- Step 1: temp lookup tables for id remapping.
         -- DROP IF EXISTS first so the coalesce is retryable on the same
         -- connection if a prior attempt failed mid-transaction (TEMP tables
         -- survive ROLLBACK in SQLite — they are session-scoped, not tx-scoped).
         DROP TABLE IF EXISTS chunk_id_map;
         DROP TABLE IF EXISTS subject_entity_id_map;
         DROP TABLE IF EXISTS subject_relationship_id_map;
         CREATE TEMP TABLE chunk_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);
         CREATE TEMP TABLE subject_entity_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);
         CREATE TEMP TABLE subject_relationship_id_map (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);

         -- Step 2: INSERT winning chunks (group by source_doc_path, seq_id; take MIN(id))
         INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash, created_at, trace_id)
         SELECT '{docs_root_hash}', '{docs_root}', seq_id, source_doc_path, source_doc_hash, created_at, trace_id
         FROM kg_chunks_v26_backup
         WHERE id IN (SELECT MIN(id) FROM kg_chunks_v26_backup GROUP BY source_doc_path, seq_id);

         -- Step 3: populate chunk_id_map
         INSERT INTO chunk_id_map (old_id, new_id)
         SELECT b.id, c.id FROM kg_chunks_v26_backup b
         JOIN kg_chunks c ON c.source_doc_path = b.source_doc_path AND c.seq_id = b.seq_id AND c.docs_root_hash = '{docs_root_hash}';

         -- Step 4: INSERT winning subject entities (majority-vote by normalized entity_key)
         INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence, properties_json, created_at, trace_id)
         SELECT '{docs_root_hash}', '{docs_root}', entity_key, type, name, confidence, properties_json, created_at, trace_id
         FROM (
             SELECT b.id, b.entity_key, b.type, b.name, b.confidence, b.properties_json, b.created_at, b.trace_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY LOWER(TRIM(b.entity_key))
                        ORDER BY vote_count DESC, avg_conf DESC, b.id ASC
                    ) AS rn
             FROM kg_subject_entities_v26_backup b
             JOIN (
                 SELECT LOWER(TRIM(entity_key)) AS norm_key,
                        COUNT(DISTINCT agent_id) AS vote_count,
                        AVG(confidence) AS avg_conf
                 FROM kg_subject_entities_v26_backup
                 GROUP BY LOWER(TRIM(entity_key))
             ) agg ON LOWER(TRIM(b.entity_key)) = agg.norm_key
         )
         WHERE rn = 1;

         -- Step 5: populate subject_entity_id_map
         INSERT INTO subject_entity_id_map (old_id, new_id)
         SELECT b.id, e.id FROM kg_subject_entities_v26_backup b
         JOIN kg_subject_entities e ON LOWER(TRIM(e.entity_key)) = LOWER(TRIM(b.entity_key)) AND e.docs_root_hash = '{docs_root_hash}';

         -- Step 6: INSERT winning subject relationships (rewire FK ids, then group)
         INSERT INTO kg_subject_relationships (docs_root_hash, docs_root, from_entity_id, to_entity_id, type, confidence, properties_json, created_at, trace_id)
         SELECT '{docs_root_hash}', '{docs_root}', new_from, new_to, type, confidence, properties_json, created_at, trace_id
         FROM (
             SELECT b.id, fm.new_id AS new_from, tm.new_id AS new_to, b.type, b.confidence,
                    b.properties_json, b.created_at, b.trace_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY fm.new_id, tm.new_id, LOWER(TRIM(b.type))
                        ORDER BY vote_count DESC, avg_conf DESC, b.id ASC
                    ) AS rn
             FROM kg_subject_relationships_v26_backup b
             JOIN subject_entity_id_map fm ON fm.old_id = b.from_entity_id
             JOIN subject_entity_id_map tm ON tm.old_id = b.to_entity_id
             JOIN (
                 SELECT fm2.new_id AS nf, tm2.new_id AS nt, LOWER(TRIM(r2.type)) AS norm_type,
                        COUNT(DISTINCT r2.agent_id) AS vote_count,
                        AVG(r2.confidence) AS avg_conf
                 FROM kg_subject_relationships_v26_backup r2
                 JOIN subject_entity_id_map fm2 ON fm2.old_id = r2.from_entity_id
                 JOIN subject_entity_id_map tm2 ON tm2.old_id = r2.to_entity_id
                 GROUP BY fm2.new_id, tm2.new_id, LOWER(TRIM(r2.type))
             ) agg ON agg.nf = fm.new_id AND agg.nt = tm.new_id AND agg.norm_type = LOWER(TRIM(b.type))
         )
         WHERE rn = 1;

         -- Step 7: populate subject_relationship_id_map
         INSERT INTO subject_relationship_id_map (old_id, new_id)
         SELECT b.id, r.id
         FROM kg_subject_relationships_v26_backup b
         JOIN subject_entity_id_map fm ON fm.old_id = b.from_entity_id
         JOIN subject_entity_id_map tm ON tm.old_id = b.to_entity_id
         JOIN kg_subject_relationships r ON r.from_entity_id = fm.new_id
                                         AND r.to_entity_id = tm.new_id
                                         AND LOWER(TRIM(r.type)) = LOWER(TRIM(b.type))
                                         AND r.docs_root_hash = '{docs_root_hash}';

         -- Step 8: INSERT chunk_subjects (rewire both chunk_id and subject_entity_id)
         INSERT OR IGNORE INTO kg_chunk_subjects (docs_root_hash, docs_root, chunk_id, subject_entity_id, extraction_trace_id, created_at)
         SELECT '{docs_root_hash}', '{docs_root}', cm.new_id, em.new_id, b.extraction_trace_id, b.created_at
         FROM kg_chunk_subjects_v26_backup b
         JOIN chunk_id_map cm ON cm.old_id = b.chunk_id
         JOIN subject_entity_id_map em ON em.old_id = b.subject_entity_id;

         -- Step 9: INSERT chunk_subject_relationships (rewire chunk_id and subject_relationship_id)
         INSERT OR IGNORE INTO kg_chunk_subject_relationships (docs_root_hash, docs_root, chunk_id, subject_relationship_id, extraction_trace_id, created_at)
         SELECT '{docs_root_hash}', '{docs_root}', cm.new_id, rm.new_id, b.extraction_trace_id, b.created_at
         FROM kg_chunk_subject_relationships_v26_backup b
         JOIN chunk_id_map cm ON cm.old_id = b.chunk_id
         JOIN subject_relationship_id_map rm ON rm.old_id = b.subject_relationship_id;

         -- Step 10: INSERT extractions (first-writer-wins by MIN(id) per source_doc_path)
         INSERT OR IGNORE INTO kg_extractions (docs_root_hash, docs_root, source_doc_path, source_doc_hash, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id, created_at)
         SELECT '{docs_root_hash}', '{docs_root}', source_doc_path, source_doc_hash, extraction_model, entities_extracted, relationships_extracted, extraction_trace_id, created_at
         FROM kg_extractions_v26_backup
         WHERE id IN (SELECT MIN(id) FROM kg_extractions_v26_backup GROUP BY source_doc_path);

         -- Step 11: INSERT per-agent resolutions (rewire subject_entity_id)
         INSERT OR IGNORE INTO kg_subject_resolutions (agent_id, subject_entity_id, domain_entity_id, confidence, created_at, trace_id)
         SELECT b.agent_id, em.new_id, b.domain_entity_id, b.confidence, b.created_at, b.trace_id
         FROM kg_subject_resolutions_v26_backup b
         JOIN subject_entity_id_map em ON em.old_id = b.subject_entity_id;

         -- Step 12: INSERT per-agent resolutions_log (rewire subject_entity_id)
         INSERT OR IGNORE INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id, source_extraction_trace_id, model, duration_ms, resolved_at)
         SELECT b.agent_id, em.new_id, b.outcome, b.resolution_trace_id, b.source_extraction_trace_id, b.model, b.duration_ms, b.resolved_at
         FROM kg_resolutions_log_v26_backup b
         JOIN subject_entity_id_map em ON em.old_id = b.subject_entity_id;

         -- Step 13: DROP the 8 backup tables
         DROP TABLE IF EXISTS kg_chunk_subject_relationships_v26_backup;
         DROP TABLE IF EXISTS kg_chunk_subjects_v26_backup;
         DROP TABLE IF EXISTS kg_subject_relationships_v26_backup;
         DROP TABLE IF EXISTS kg_subject_resolutions_v26_backup;
         DROP TABLE IF EXISTS kg_resolutions_log_v26_backup;
         DROP TABLE IF EXISTS kg_subject_entities_v26_backup;
         DROP TABLE IF EXISTS kg_extractions_v26_backup;
         DROP TABLE IF EXISTS kg_chunks_v26_backup;

         -- Step 14: DROP temp lookup tables
         DROP TABLE IF EXISTS chunk_id_map;
         DROP TABLE IF EXISTS subject_entity_id_map;
         DROP TABLE IF EXISTS subject_relationship_id_map;

         -- Step 15: schema_meta coalesce marker
         INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1');
        ",
        docs_root_hash = docs_root_hash,
        docs_root = docs_root,
    )
}

// ===== Database =====

pub struct Database {
    pub(crate) conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        init_sqlite_vec();
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open SQLite at {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA auto_vacuum = INCREMENTAL;",
        )
        .context("failed to set SQLite pragmas")?;
        let mut db = Self { conn };
        // Auto-backup DB file before any destructive schema migration
        let current_ver = db.current_version().unwrap_or(0);
        if current_ver > 0 && current_ver < CURRENT_SCHEMA_VERSION {
            let backup_path = path.with_extension(format!("db.v{current_ver}-backup"));
            match std::fs::copy(path, &backup_path) {
                Ok(_) => info!(
                    from_version = current_ver,
                    backup = %backup_path.display(),
                    "auto-backed up DB before schema migration"
                ),
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Cannot auto-backup database before migration ({}). \
                         Aborting to protect data. Free disk space or manually backup '{}' before retrying.",
                        e,
                        path.display()
                    ));
                }
            }
        }
        db.migrate()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        init_sqlite_vec();
        let conn = Connection::open_in_memory().context("failed to open in-memory SQLite")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to set pragmas")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Execute raw SQL with params. Returns the number of rows changed.
    ///
    /// Intended for test fixture seeding from integration tests where `conn`
    /// is not directly accessible (`pub(crate)`).
    pub fn execute_sql(&self, sql: &str, params: &[&dyn rusqlite::types::ToSql]) -> Result<usize> {
        Ok(self.conn.execute(sql, params)?)
    }

    /// Returns the last inserted rowid.
    pub fn last_insert_rowid(&self) -> i64 {
        self.conn.last_insert_rowid()
    }

    /// Query a single scalar value. Returns `None` if no rows match.
    pub fn query_scalar<T: rusqlite::types::FromSql>(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<Option<T>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(sql, params, |row| row.get(0))
            .optional()?)
    }

    /// Query a single row and return two columns. Returns `None` if no rows match.
    pub fn query_row_2<T1: rusqlite::types::FromSql, T2: rusqlite::types::FromSql>(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<Option<(T1, T2)>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(sql, params, |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?)
    }

    fn current_version(&self) -> Result<i64> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='schema_version'",
                [],
                |row| row.get(0),
            )
            .context("failed to check schema_version existence")?;
        if !exists {
            return Ok(0);
        }
        let version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .context("failed to read schema version")?;
        Ok(version)
    }

    fn migrate(&mut self) -> Result<()> {
        let version = self.current_version()?;
        debug!(
            current_version = version,
            target_version = CURRENT_SCHEMA_VERSION,
            "checking migrations"
        );
        if version < 1 {
            if version > 0 {
                info!(from = version, to = 1, "applying clean-slate migration");
            }
            self.migrate_v1()?;
            info!(
                version = CURRENT_SCHEMA_VERSION,
                "database migrated to v{CURRENT_SCHEMA_VERSION}"
            );
        }
        if (1..3).contains(&version) {
            self.migrate_v3()?;
            info!(version = 3, "database migrated to v3");
        }
        if version == 3 {
            self.migrate_v3_to_v4()?;
            info!(version = 4, "database migrated to v4");
        }
        if version == 4 || version == 3 {
            self.migrate_v4_to_v5()?;
            info!(version = 5, "database migrated to v5");
        }
        if (3..=5).contains(&version) {
            self.migrate_v5_to_v6()?;
            info!(version = 6, "database migrated to v6");
        }
        if (3..=6).contains(&version) {
            self.migrate_v6_to_v7()?;
            info!(version = 7, "database migrated to v7");
        }
        if (3..=7).contains(&version) {
            self.migrate_v7_to_v8()?;
            info!(version = 8, "database migrated to v8");
        }
        if (3..=8).contains(&version) {
            self.migrate_v8_to_v9()?;
            info!(version = 9, "database migrated to v9");
        }
        if (3..=9).contains(&version) {
            self.migrate_v9_to_v10()?;
            info!(version = 10, "database migrated to v10");
        }
        if (3..=10).contains(&version) {
            self.migrate_v10_to_v11()?;
            info!(version = 11, "database migrated to v11");
        }
        if (3..=11).contains(&version) {
            self.migrate_v11_to_v12()?;
            info!(version = 12, "database migrated to v12");
        }
        if (3..=12).contains(&version) {
            self.migrate_v12_to_v13()?;
            info!(version = 13, "database migrated to v13");
        }
        if (3..=13).contains(&version) {
            self.migrate_v13_to_v14()?;
            info!(version = 14, "database migrated to v14");
        }
        if (3..=14).contains(&version) {
            self.migrate_v14_to_v15()?;
            info!(version = 15, "database migrated to v15");
        }
        if (3..=15).contains(&version) {
            self.migrate_v15_to_v16()?;
            info!(version = 16, "database migrated to v16");
        }
        if (3..=16).contains(&version) {
            self.migrate_v16_to_v17()?;
            info!(version = 17, "database migrated to v17");
        }
        if (3..=17).contains(&version) {
            self.migrate_v17_to_v18()?;
            info!(version = 18, "database migrated to v18");
        }
        if (3..=18).contains(&version) {
            self.migrate_v18_to_v19()?;
            info!(version = 19, "database migrated to v19");
        }
        if (3..=19).contains(&version) {
            self.migrate_v19_to_v20()?;
            info!(version = 20, "database migrated to v20");
        }
        if (3..=20).contains(&version) {
            self.migrate_v20_to_v21()?;
            info!(version = 21, "database migrated to v21");
        }
        if (3..=21).contains(&version) {
            self.migrate_v21_to_v22()?;
            info!(version = 22, "database migrated to v22");
        }
        if (3..=22).contains(&version) {
            self.migrate_v22_to_v23()?;
            info!(version = 23, "database migrated to v23");
        }
        if (3..=23).contains(&version) {
            self.migrate_v23_to_v24()?;
            info!(version = 24, "database migrated to v24");
        }
        if (3..=24).contains(&version) {
            self.migrate_v24_to_v25()?;
            info!(version = 25, "database migrated to v25");
        }
        if (3..=25).contains(&version) {
            self.migrate_v25_to_v26()?;
            info!(version = 26, "database migrated to v26");
        }
        if (3..=26).contains(&version) {
            self.migrate_v26_to_v27()?;
            info!(version = 27, "database migrated to v27");
        }

        // v27 startup guard: refuse to return a Database handle if the
        // coalesce step from #787 has not run. Fresh installs write the
        // marker in migrate_v1; existing DBs upgraded via the stub get
        // the marker only when #787's coalesce SQL runs.
        self.check_v27_coalesce_guard()?;

        if (3..=27).contains(&version) {
            self.migrate_v27_to_v28()?;
            info!(version = 28, "database migrated to v28");
        }

        if (3..=28).contains(&version) {
            self.migrate_v28_to_v29()?;
            info!(version = 29, "database migrated to v29");
        }

        if (3..=29).contains(&version) {
            self.migrate_v29_to_v30()?;
            info!(version = 30, "database migrated to v30");
        }

        if (3..=30).contains(&version) {
            self.migrate_v30_to_v31()?;
            info!(version = 31, "database migrated to v31");
        }

        if (3..=31).contains(&version) {
            self.migrate_v31_to_v32()?;
            info!(version = 32, "database migrated to v32");
        }

        if (3..=32).contains(&version) {
            self.migrate_v32_to_v33()?;
            info!(version = 33, "database migrated to v33");
        }

        if (3..=33).contains(&version) {
            self.migrate_v33_to_v34()?;
            info!(version = 34, "database migrated to v34");
        }

        if (3..=34).contains(&version) {
            self.migrate_v34_to_v35()?;
            info!(version = 35, "database migrated to v35");
        }

        if (3..=35).contains(&version) {
            self.migrate_v35_to_v36()?;
            info!(version = 36, "database migrated to v36");
        }

        if (3..=36).contains(&version) {
            self.migrate_v36_to_v37()?;
            info!(version = 37, "database migrated to v37");
        }

        if (3..=37).contains(&version) {
            self.migrate_v37_to_v38()?;
            info!(version = 38, "database migrated to v38");
        }

        if (3..=38).contains(&version) {
            self.migrate_v38_to_v39()?;
            info!(version = 39, "database migrated to v39");
        }

        if (3..=39).contains(&version) {
            self.migrate_v39_to_v40()?;
            info!(version = 40, "database migrated to v40");
        }

        if (3..=40).contains(&version) {
            self.migrate_v40_to_v41()?;
            info!(version = 41, "database migrated to v41");
        }

        if (3..=41).contains(&version) {
            self.migrate_v41_to_v42()?;
            info!(version = 42, "database migrated to v42");
        }

        if (3..=42).contains(&version) {
            self.migrate_v42_to_v43()?;
            info!(version = 43, "database migrated to v43");
        }

        if (3..=43).contains(&version) {
            self.migrate_v43_to_v44()?;
            info!(version = 44, "database migrated to v44");
        }

        Ok(())
    }

    fn migrate_v1(&mut self) -> Result<()> {
        info!("applying migration v1: unified task engine schema (clean slate)");

        // Drop all existing tables (clean slate — no backward compat constraint)
        let drops = [
            "DROP TABLE IF EXISTS fts_search",
            "DROP TABLE IF EXISTS vec_search",
            "DROP TABLE IF EXISTS tool_calls",
            "DROP TABLE IF EXISTS llm_calls",
            "DROP TABLE IF EXISTS a2a_push_notification_configs",
            "DROP TABLE IF EXISTS a2a_artifacts",
            "DROP TABLE IF EXISTS a2a_messages",
            "DROP TABLE IF EXISTS a2a_tasks",
            "DROP TABLE IF EXISTS a2a_task_map",
            "DROP TABLE IF EXISTS reminders",
            "DROP TABLE IF EXISTS heartbeat_sends",
            "DROP TABLE IF EXISTS reflection_runs",
            "DROP TABLE IF EXISTS failed_sends",
            "DROP TABLE IF EXISTS customer_config",
            "DROP TABLE IF EXISTS audit_event_summaries",
            "DROP TABLE IF EXISTS audit_events",
            "DROP TABLE IF EXISTS memory_event_summaries",
            "DROP TABLE IF EXISTS memory_events",
            "DROP TABLE IF EXISTS search_content",
            "DROP TABLE IF EXISTS events",
            "DROP TABLE IF EXISTS preferences",
            "DROP TABLE IF EXISTS commitments",
            "DROP TABLE IF EXISTS people",
            "DROP TABLE IF EXISTS core_memory",
            "DROP TABLE IF EXISTS team_workspace",
            "DROP TABLE IF EXISTS team_messages",
            "DROP TABLE IF EXISTS team_runs",
            "DROP TABLE IF EXISTS messages",
            "DROP TABLE IF EXISTS sessions",
            "DROP TABLE IF EXISTS conversations",
            "DROP TABLE IF EXISTS tasks",
            "DROP TABLE IF EXISTS teams",
            "DROP TABLE IF EXISTS agents",
            "DROP TABLE IF EXISTS schema_version",
        ];
        for drop in &drops {
            self.conn.execute_batch(drop)?;
        }

        let tx = self.conn.transaction()?;
        tx.execute_batch(
                "
            CREATE TABLE schema_version (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO schema_version (version) VALUES (44);

            -- Schema meta table for migration state tracking (v27+).
            CREATE TABLE schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            -- Fresh installs are trivially coalesce-complete (no v26 data).
            INSERT INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1');

            CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            -- Per-agent KG corpus mapping (#798). Maps agent_id to
            -- docs_root_hash for multi-corpus query fan-out.
            CREATE TABLE agent_kg_corpora (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                docs_root_hash TEXT NOT NULL,
                docs_root_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, docs_root_hash)
            );
            CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);

            CREATE TABLE teams (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE team_runs (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','cancelled','suspended')),
                failure_reason TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                max_iterations INTEGER NOT NULL DEFAULT 3,
                deliverable TEXT,
                checkpoint TEXT,
                trace_id TEXT,
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT
            );
            CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);

            CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual','a2a')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                metadata TEXT,
                type TEXT NOT NULL DEFAULT 'issue' CHECK (
                    type IN ('issue', 'milestone', 'project')
                ),
                dispatch_class TEXT CHECK (
                    dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom')
                ),
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
            CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');
            CREATE UNIQUE INDEX idx_tasks_manual_active_ref_url
                ON tasks(agent_id, reference_url)
                WHERE trigger_type = 'manual'
                AND reference_url IS NOT NULL
                AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');
            CREATE INDEX idx_tasks_dispatch_class
                ON tasks(agent_id, dispatch_class, status)
                WHERE dispatch_class IS NOT NULL;

            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                metadata TEXT,
                parent_session_id TEXT,
                task_id TEXT
            );
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;
            CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;

            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                internal INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;

            CREATE TABLE task_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                agent_id   TEXT NOT NULL,
                session_id TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                trace_id   TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_task_messages_task_created
                ON task_messages (task_id, created_at);
            CREATE INDEX idx_task_messages_agent_created
                ON task_messages (agent_id, created_at);

            CREATE TABLE core_memory (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE people (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                last_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                mention_count INTEGER NOT NULL DEFAULT 1,
                UNIQUE (agent_id, canonical_name)
            );

            CREATE TABLE commitments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','completed','cancelled')),
                due_date TEXT,
                person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                completed_at TEXT
            );
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                ON commitments(agent_id, description COLLATE NOCASE, due_date)
                WHERE status = 'pending';

            CREATE TABLE preferences (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, category)
            );

            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
            CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id) WHERE rewound_by_trace_id IS NOT NULL;

            CREATE TABLE audit_event_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, year, month)
            );

            CREATE TABLE search_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);

            CREATE TABLE team_workspace (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_workspace(id),
                agent_name TEXT,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                trace_id TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
            CREATE INDEX idx_team_ws_trace ON team_workspace(trace_id)
                WHERE trace_id IS NOT NULL;

            CREATE TABLE heartbeat_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);

            CREATE TABLE reflection_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);

            CREATE TABLE customer_config (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE failed_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE skill_overrides (
                agent_id        TEXT NOT NULL COLLATE NOCASE,
                skill_name      TEXT NOT NULL COLLATE NOCASE,
                always_on       INTEGER,
                llm_provider    TEXT,
                llm_model       TEXT,
                enabled         INTEGER,
                lifecycle_state TEXT CHECK (lifecycle_state IN ('staged', 'active', 'archived')),
                use_count       INTEGER NOT NULL DEFAULT 0,
                last_used_at    TEXT,
                PRIMARY KEY (agent_id, skill_name)
            );

            -- A2A Protocol tables: thin mapping table + genuinely new tables
            CREATE TABLE a2a_task_map (
                a2a_task_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                context_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_a2a_task_map_task ON a2a_task_map(task_id);

            CREATE TABLE a2a_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                artifact_id TEXT NOT NULL,
                name TEXT,
                description TEXT,
                parts TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE a2a_push_notification_configs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                token TEXT,
                auth_scheme TEXT,
                auth_credentials TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

            -- Observability: LLM call tracking
            CREATE TABLE llm_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                stop_reason TEXT,
                status TEXT NOT NULL DEFAULT 'success',
                error_message TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                prompt_variant TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                response_text TEXT,
                reasoning TEXT,
                system_prompt_bytes INTEGER
            );
            CREATE INDEX idx_llm_calls_trace ON llm_calls(trace_id);
            CREATE INDEX idx_llm_calls_session ON llm_calls(session_id);
            CREATE INDEX idx_llm_calls_agent_created ON llm_calls(agent_id, created_at);

            -- Observability: Tool call tracking (full I/O)
            CREATE TABLE tool_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                llm_call_id TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                tool_source TEXT NOT NULL DEFAULT 'builtin',
                skill_name TEXT,
                input TEXT,
                output TEXT,
                success INTEGER NOT NULL DEFAULT 1,
                non_zero_exit INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_tool_calls_trace ON tool_calls(trace_id);
            CREATE INDEX idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX idx_tool_calls_llm_call ON tool_calls(llm_call_id);
            CREATE INDEX idx_tool_calls_agent_created ON tool_calls(agent_id, created_at);

            -- KG domain layer (global, no agent_id)
            CREATE TABLE kg_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_key TEXT NOT NULL UNIQUE,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                CHECK (entity_key = type || ':' || name)
            );
            CREATE INDEX idx_kg_entities_type ON kg_entities(type);

            CREATE TABLE kg_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_kg_rel_from ON kg_relationships(from_entity_id, type);
            CREATE INDEX idx_kg_rel_to ON kg_relationships(to_entity_id, type);

            -- KG lexical layer (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                seq_id INTEGER NOT NULL,
                source_doc_path TEXT NOT NULL,
                source_doc_hash TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (docs_root_hash, source_doc_path, seq_id)
            );
            CREATE INDEX idx_kg_chunks_docs_root_hash_doc ON kg_chunks(docs_root_hash, source_doc_path);

            -- KG subject layer (shared by docs_root_hash — v27)
            CREATE TABLE kg_subject_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                entity_key TEXT NOT NULL,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                discovered INTEGER NOT NULL DEFAULT 0,
                discovery_reason TEXT,
                CHECK (entity_key = type || ':' || name),
                UNIQUE (docs_root_hash, entity_key)
            );
            CREATE INDEX idx_kg_subj_entities_drh_type ON kg_subject_entities(docs_root_hash, type);

            CREATE TABLE kg_subject_resolutions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (agent_id, subject_entity_id, domain_entity_id)
            );
            CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
            CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

            -- KG subject-to-subject edges / fact triples (shared by docs_root_hash — v27)
            CREATE TABLE kg_subject_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
            );
            CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(docs_root_hash, from_entity_id, type);
            CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(docs_root_hash, to_entity_id, type);
            CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(docs_root_hash, type);

            -- KG entity provenance: chunk -> subject entity (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunk_subjects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
            );
            CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(docs_root_hash, chunk_id);
            CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash, subject_entity_id);
            CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(docs_root_hash, extraction_trace_id);

            -- KG relationship provenance: chunk -> subject relationship (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunk_subject_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
            );
            CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(docs_root_hash, chunk_id);
            CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(docs_root_hash, subject_relationship_id);

            -- KG extraction tracking (shared by docs_root_hash — v27; first-writer-wins)
            CREATE TABLE kg_extractions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                source_doc_path TEXT NOT NULL,
                source_doc_hash TEXT,
                extraction_model TEXT NOT NULL,
                entities_extracted INTEGER NOT NULL DEFAULT 0,
                relationships_extracted INTEGER NOT NULL DEFAULT 0,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, source_doc_path)
            );
            CREATE INDEX idx_kg_extractions_drh ON kg_extractions(docs_root_hash);

            -- KG resolution tracking
            CREATE TABLE kg_resolutions_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                outcome TEXT NOT NULL CHECK (outcome IN (
                    'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                    'no_match', 'no_candidate_of_type',
                    'skipped_discovered_type', 'skipped_discovered_subject',
                    'skipped_no_llm', 'error'
                )),
                resolution_trace_id TEXT NOT NULL,
                source_extraction_trace_id TEXT,
                model TEXT,
                duration_ms INTEGER,
                resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, subject_entity_id)
            );
            CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

            -- KG invalidation markers (#961): ephemeral sidecar for tracking
            -- entities whose no_match resolution log rows were deleted by
            -- domain-graph rebuild invalidation (#960).
            CREATE TABLE kg_invalidated_no_match (
                subject_entity_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (subject_entity_id, agent_id)
            );

            -- Operational ledger (#1262): canonical operational-item store.
            CREATE TABLE operational_items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
                title TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
                owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
                owner_name TEXT,
                priority REAL NOT NULL DEFAULT 0.0,
                user_importance REAL NOT NULL DEFAULT 0.0,
                due_at TEXT,
                blocked_by TEXT,
                next_action TEXT,
                evidence_refs TEXT NOT NULL DEFAULT '[]',
                confidence REAL NOT NULL DEFAULT 1.0,
                source_table TEXT,
                source_id TEXT,
                agent_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
            CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
            CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
            CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
            CREATE UNIQUE INDEX idx_operational_items_source_unique
                ON operational_items(agent_id, source_table, source_id)
                WHERE source_table IS NOT NULL AND source_id IS NOT NULL;

            -- Auto-pull circuit-breaker stats (mika#1363)
            CREATE TABLE auto_pull_stats (
                repo_full_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                last_auto_pull_at TEXT,
                last_failure_at TEXT,
                PRIMARY KEY (repo_full_name, issue_number)
            );

            -- Permission-decision provenance ledger (mika#1733 AC4)
            CREATE TABLE permission_decisions (
                id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_summary TEXT,
                classifier_verdict TEXT NOT NULL
                    CHECK (classifier_verdict IN ('approved', 'denied', 'held')),
                operator_decision TEXT
                    CHECK (operator_decision IN ('approve', 'deny')),
                override_used INTEGER NOT NULL DEFAULT 0
                    CHECK (override_used IN (0, 1)),
                decision_authority TEXT NOT NULL
                    CHECK (decision_authority IN ('strict', 'override')),
                tenant_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_permission_decisions_request_id
                ON permission_decisions(request_id);
            CREATE INDEX idx_permission_decisions_created_at
                ON permission_decisions(created_at DESC);

            -- Pre-register the default 'mika' agent
            INSERT INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');
            ",
            )
            .context("failed to create v1 schema")?;
        tx.commit()?;

        // Unified timeline VIEW (uses shared constant)
        self.conn
            .execute_batch(UNIFIED_TIMELINE_VIEW_SQL)
            .context("failed to create unified_timeline view")?;

        // Virtual tables must be outside transactions
        let _ = self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
                 USING fts5(content, content='search_content', content_rowid='id');
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_search
                 USING vec0(embedding float[512]);",
        );

        Ok(())
    }

    /// Migration v3: Sessions + Messages schema redesign (clean-slate).
    ///
    /// Single user, no data to preserve. Drop and recreate via migrate_v1.
    fn migrate_v3(&mut self) -> Result<()> {
        info!("applying migration v3: sessions + messages schema redesign (clean slate)");
        self.migrate_v1()
    }

    /// Migration v3 → v4: Add duplicate-prevention indexes to existing v3 databases.
    ///
    /// New databases already get these indexes via `migrate_v1`, but databases
    /// created at v3 before these indexes were added need them applied retroactively.
    fn migrate_v3_to_v4(&self) -> Result<()> {
        info!("migrating database schema v3 → v4 (duplicate-prevention indexes)");
        self.conn.execute_batch(
            "DROP INDEX IF EXISTS idx_tasks_unique_recurring;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');

             CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

             -- Rebuild commitments table: replace inline UNIQUE(agent_id, description)
             -- with partial unique index scoped to pending status
             CREATE TABLE commitments_new (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 description TEXT NOT NULL COLLATE NOCASE,
                 status TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending','completed','cancelled')),
                 due_date TEXT,
                 person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                 created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                 completed_at INTEGER
             );
             INSERT INTO commitments_new SELECT * FROM commitments;
             DROP TABLE commitments;
             ALTER TABLE commitments_new RENAME TO commitments;
             CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                 ON commitments(agent_id, description COLLATE NOCASE, due_date)
                 WHERE status = 'pending';

             PRAGMA user_version = 4;",
        )?;
        // Update the schema_version table to reflect v4
        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (4)", [])?;
        Ok(())
    }

    /// Migration v4 → v5: Rename memory_events → audit_events, add trace_id columns,
    /// create unified_timeline VIEW.
    ///
    /// Idempotent: each step checks existence before acting, since `ALTER TABLE RENAME TO`
    /// auto-commits outside transactions in SQLite and a crash mid-migration could leave
    /// partial state.
    fn migrate_v4_to_v5(&self) -> Result<()> {
        info!("migrating database schema v4 → v5 (orthogonal observability)");

        // 1. Rename memory_events → audit_events (idempotent)
        let has_old: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_events'",
            [],
            |r| r.get(0),
        )?;
        if has_old {
            self.conn
                .execute_batch("ALTER TABLE memory_events RENAME TO audit_events")?;
        }

        // 2. Recreate indexes with new names
        self.conn.execute_batch(
            "DROP INDEX IF EXISTS idx_memev_agent_created;
             DROP INDEX IF EXISTS idx_memev_session;
             CREATE INDEX IF NOT EXISTS idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_events(session_id);",
        )?;

        // 3. Rename memory_event_summaries → audit_event_summaries (idempotent)
        let has_old_summaries: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_event_summaries'",
            [],
            |r| r.get(0),
        )?;
        if has_old_summaries {
            self.conn.execute_batch(
                "ALTER TABLE memory_event_summaries RENAME TO audit_event_summaries",
            )?;
        }

        // 4. Add trace_id columns (idempotent — ALTER TABLE ADD COLUMN is a no-op if exists)
        // We check column existence via pragma to avoid "duplicate column" errors on re-run.
        if !self.column_exists("messages", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE messages ADD COLUMN trace_id TEXT")?;
        }
        if !self.column_exists("tasks", "created_trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE tasks ADD COLUMN created_trace_id TEXT")?;
        }
        if !self.column_exists("audit_events", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE audit_events ADD COLUMN trace_id TEXT")?;
        }
        if !self.column_exists("team_workspace", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE team_workspace ADD COLUMN trace_id TEXT")?;
        }

        // 5. Create partial indexes on trace_id columns
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;",
        )?;

        // 6. Create unified_timeline VIEW (uses shared constant)
        self.conn
            .execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // 7. Update schema version
        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (5)", [])?;

        Ok(())
    }

    fn migrate_v5_to_v6(&self) -> Result<()> {
        info!("migrating database schema v5 → v6 (people mention_count)");

        if !self.column_exists("people", "mention_count")? {
            self.conn.execute_batch(
                "ALTER TABLE people ADD COLUMN mention_count INTEGER NOT NULL DEFAULT 1",
            )?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (6)", [])?;

        Ok(())
    }

    fn migrate_v6_to_v7(&self) -> Result<()> {
        info!("migrating database schema v6 → v7 (skill_overrides table)");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS skill_overrides (
                agent_id   TEXT NOT NULL COLLATE NOCASE,
                skill_name TEXT NOT NULL COLLATE NOCASE,
                always_on  INTEGER,
                PRIMARY KEY (agent_id, skill_name)
            )",
        )?;

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (7)", [])?;

        Ok(())
    }

    fn migrate_v7_to_v8(&mut self) -> Result<()> {
        info!(
            "migrating database schema v7 → v8 (tasks: manual trigger_type, blocked status, none action_type, reference_url, source)"
        );

        // SQLite cannot ALTER CHECK constraints, so we must rebuild the tasks table.
        // Entire migration wrapped in a transaction to prevent partial state on crash.
        //
        // PRAGMA foreign_keys must be OFF during the table rebuild because the INSERT
        // copies self-referencing parent_task_id rows, and ALTER TABLE RENAME validates
        // FK references. Also disable FK checks to avoid issues with the temporary table.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop the unified_timeline VIEW first — it references the `tasks` table.
        // SQLite 3.25+ validates all views/triggers during ALTER TABLE RENAME,
        // so the view must not exist when we rename tasks_new → tasks.
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at INTEGER,
                timeout_at INTEGER,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                fired_at INTEGER,
                completed_at INTEGER
            );

            INSERT INTO tasks_new (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                status, process_id, input_context, result,
                created_by_session, created_trace_id,
                created_at, updated_at, fired_at, completed_at
            )
            SELECT
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                status, process_id, input_context, result,
                created_by_session, created_trace_id,
                created_at, updated_at, fired_at, completed_at
            FROM tasks;

            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;",
        )?;

        // Recreate all indexes (still within the transaction)
        tx.execute_batch(
            "CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
             CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
             CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');
             CREATE INDEX IF NOT EXISTS idx_tasks_callback_delivery
                ON tasks(agent_id, completed_at)
                WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');
             CREATE INDEX IF NOT EXISTS idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');",
        )?;

        // Recreate unified_timeline VIEW (was dropped before table rebuild)
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (8)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v8_to_v9(&mut self) -> Result<()> {
        info!(
            "migrating database schema v8 → v9 (rewind: nullable after_value, rewound_by_trace_id)"
        );

        // Rebuild audit_events to make after_value nullable and add rewound_by_trace_id.
        // SQLite cannot ALTER a NOT NULL constraint, so we must rebuild the table.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop the unified_timeline VIEW — it references audit_events.
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        tx.execute_batch(
            "CREATE TABLE audit_events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            INSERT INTO audit_events_new (id, agent_id, session_id, tool_name, target_key,
                before_value, after_value, reasoning, trace_id, created_at)
            SELECT id, agent_id, session_id, tool_name, target_key,
                before_value, after_value, reasoning, trace_id, created_at
            FROM audit_events;

            DROP TABLE audit_events;
            ALTER TABLE audit_events_new RENAME TO audit_events;",
        )?;

        // Recreate existing indexes + new rewound index
        tx.execute_batch(
            "CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at);
             CREATE INDEX idx_audit_session ON audit_events(session_id);
             CREATE INDEX idx_audit_trace ON audit_events(trace_id)
                 WHERE trace_id IS NOT NULL;
             CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id)
                 WHERE rewound_by_trace_id IS NOT NULL;",
        )?;

        // Recreate unified_timeline VIEW
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (9)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v9_to_v10(&mut self) -> Result<()> {
        info!(
            "migrating database schema v9 → v10 (team_runs.trace_id, unified_timeline + team_workspace)"
        );

        // Hoist column_exists check before creating the transaction (borrow checker constraint).
        let has_trace_id = self.column_exists("team_runs", "trace_id")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Add trace_id column to team_runs (idempotent guard for crash recovery)
        if !has_trace_id {
            tx.execute_batch("ALTER TABLE team_runs ADD COLUMN trace_id TEXT;")?;
        }

        // Recreate unified_timeline VIEW with team_workspace union
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // Add partial index on team_workspace.trace_id (matches other timeline tables)
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_team_ws_trace ON team_workspace(trace_id)
                 WHERE trace_id IS NOT NULL;",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (10)", [])?;

        tx.commit()?;
        Ok(())
    }

    fn migrate_v10_to_v11(&mut self) -> Result<()> {
        info!(
            "migrating database schema v10 → v11 (tasks.execution_trace_id, sessions.parent_session_id)"
        );

        // Hoist column_exists checks before creating the transaction (borrow checker constraint).
        let has_exec_trace = self.column_exists("tasks", "execution_trace_id")?;
        let has_parent_session = self.column_exists("sessions", "parent_session_id")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Add execution_trace_id column to tasks (idempotent guard)
        if !has_exec_trace {
            tx.execute_batch("ALTER TABLE tasks ADD COLUMN execution_trace_id TEXT;")?;
        }

        // Add parent_session_id column to sessions (idempotent guard)
        if !has_parent_session {
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;")?;
        }

        // Partial indexes for new columns
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;",
        )?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;",
        )?;

        // Recreate unified_timeline VIEW with COALESCE for execution_trace_id
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (11)", [])?;

        tx.commit()?;
        Ok(())
    }

    fn migrate_v11_to_v12(&mut self) -> Result<()> {
        info!("migrating database schema v11 → v12 (INTEGER timestamps → ISO 8601 TEXT)");

        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop views that reference tables we're rebuilding
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        // --- agents ---
        tx.execute_batch(
            "CREATE TABLE agents_new (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO agents_new SELECT id, name, home_dir, active,
                CASE WHEN last_seen IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', last_seen, 'unixepoch') ELSE NULL END,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM agents;
            DROP TABLE agents;
            ALTER TABLE agents_new RENAME TO agents;")?;

        // --- teams ---
        tx.execute_batch(
            "CREATE TABLE teams_new (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO teams_new SELECT id, name, config_path,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM teams;
            DROP TABLE teams;
            ALTER TABLE teams_new RENAME TO teams;",
        )?;

        // --- team_runs ---
        tx.execute_batch(
            "CREATE TABLE team_runs_new (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','cancelled','suspended')),
                failure_reason TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                max_iterations INTEGER NOT NULL DEFAULT 3,
                deliverable TEXT,
                checkpoint TEXT,
                trace_id TEXT,
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT
            );
            INSERT INTO team_runs_new SELECT id, team_id, goal, status, failure_reason,
                iteration, max_iterations, deliverable, checkpoint, trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
                CASE WHEN ended_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch') ELSE NULL END
            FROM team_runs;
            DROP TABLE team_runs;
            ALTER TABLE team_runs_new RENAME TO team_runs;
            CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);")?;

        // --- sessions (must be before messages due to FK) ---
        tx.execute_batch(
            "CREATE TABLE sessions_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                metadata TEXT,
                parent_session_id TEXT
            );
            INSERT INTO sessions_new SELECT id, agent_id, channel_type,
                strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
                CASE WHEN ended_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch') ELSE NULL END,
                metadata, parent_session_id
            FROM sessions;
            DROP TABLE sessions;
            ALTER TABLE sessions_new RENAME TO sessions;
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
            CREATE INDEX idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;")?;

        // --- tasks ---
        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            INSERT INTO tasks_new SELECT id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                CASE WHEN next_fire_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', next_fire_at, 'unixepoch') ELSE NULL END,
                CASE WHEN timeout_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', timeout_at, 'unixepoch') ELSE NULL END,
                action_type, action_config, status, process_id, input_context, result,
                reference_url, source, created_by_session, created_trace_id, execution_trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch'),
                CASE WHEN fired_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', fired_at, 'unixepoch') ELSE NULL END,
                CASE WHEN completed_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', completed_at, 'unixepoch') ELSE NULL END
            FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;")?;

        // Recreate task indexes
        tx.execute_batch(
            "CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
             CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at) WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_schedulable ON tasks(agent_id, next_fire_at ASC) WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
             CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
             CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
             CREATE INDEX idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
             CREATE INDEX idx_tasks_manual_active ON tasks(agent_id, created_at DESC) WHERE trigger_type = 'manual' AND status IN ('pending', 'in_progress', 'blocked');
             CREATE UNIQUE INDEX idx_tasks_unique_recurring ON tasks(agent_id, label COLLATE NOCASE) WHERE trigger_type = 'recurring' AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
             CREATE UNIQUE INDEX idx_tasks_unique_reminder ON tasks(agent_id, label COLLATE NOCASE) WHERE status IN ('pending', 'in_progress', 'recurring_active') AND (action_type = 'send_message' OR action_type = 'resume_agent') AND trigger_type NOT IN ('callback');
             CREATE INDEX idx_tasks_callback_delivery ON tasks(agent_id, completed_at) WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');")?;

        // --- messages ---
        tx.execute_batch(
            "CREATE TABLE messages_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO messages_new SELECT id, session_id, agent_id, role, content, metadata,
                trace_id, compacted_through_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM messages;
            DROP TABLE messages;
            ALTER TABLE messages_new RENAME TO messages;
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;")?;

        // --- core_memory ---
        tx.execute_batch(
            "CREATE TABLE core_memory_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );
            INSERT INTO core_memory_new SELECT agent_id, key, value, token_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM core_memory;
            DROP TABLE core_memory;
            ALTER TABLE core_memory_new RENAME TO core_memory;",
        )?;

        // --- people ---
        tx.execute_batch(
            "CREATE TABLE people_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                last_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                mention_count INTEGER NOT NULL DEFAULT 1,
                UNIQUE (agent_id, canonical_name)
            );
            INSERT INTO people_new SELECT id, agent_id, canonical_name, relationship, notes,
                strftime('%Y-%m-%dT%H:%M:%SZ', first_mentioned, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', last_mentioned, 'unixepoch'),
                mention_count
            FROM people;
            DROP TABLE people;
            ALTER TABLE people_new RENAME TO people;",
        )?;

        // --- commitments ---
        tx.execute_batch(
            "CREATE TABLE commitments_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','completed','cancelled')),
                due_date TEXT,
                person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                completed_at TEXT
            );
            INSERT INTO commitments_new SELECT id, agent_id, description, status, due_date, person_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                CASE WHEN completed_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', completed_at, 'unixepoch') ELSE NULL END
            FROM commitments;
            DROP TABLE commitments;
            ALTER TABLE commitments_new RENAME TO commitments;
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);
            CREATE UNIQUE INDEX idx_commitments_unique_pending ON commitments(agent_id, description COLLATE NOCASE, due_date) WHERE status = 'pending';")?;

        // --- preferences ---
        tx.execute_batch(
            "CREATE TABLE preferences_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, category)
            );
            INSERT INTO preferences_new SELECT agent_id, category, value,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM preferences;
            DROP TABLE preferences;
            ALTER TABLE preferences_new RENAME TO preferences;",
        )?;

        // --- events ---
        tx.execute_batch(
            "CREATE TABLE events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO events_new SELECT id, agent_id, description, event_date, context,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM events;
            DROP TABLE events;
            ALTER TABLE events_new RENAME TO events;
            CREATE UNIQUE INDEX idx_events_unique_description ON events(agent_id, description COLLATE NOCASE, event_date) WHERE event_date IS NOT NULL;")?;

        // --- audit_events ---
        tx.execute_batch(
            "CREATE TABLE audit_events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO audit_events_new SELECT id, agent_id, session_id, tool_name,
                target_key, before_value, after_value, reasoning, trace_id, rewound_by_trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM audit_events;
            DROP TABLE audit_events;
            ALTER TABLE audit_events_new RENAME TO audit_events;
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
            CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id) WHERE rewound_by_trace_id IS NOT NULL;")?;

        // --- audit_event_summaries ---
        tx.execute_batch(
            "CREATE TABLE audit_event_summaries_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, year, month)
            );
            INSERT INTO audit_event_summaries_new SELECT id, agent_id, year, month,
                summary, event_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM audit_event_summaries;
            DROP TABLE audit_event_summaries;
            ALTER TABLE audit_event_summaries_new RENAME TO audit_event_summaries;",
        )?;

        // --- search_content ---
        tx.execute_batch(
            "CREATE TABLE search_content_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO search_content_new SELECT id, agent_id, source_type, source_id, content,
                embedding_json,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM search_content;
            DROP TABLE search_content;
            ALTER TABLE search_content_new RENAME TO search_content;
            CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);",
        )?;

        // --- team_workspace ---
        tx.execute_batch(
            "CREATE TABLE team_workspace_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_workspace_new(id),
                agent_name TEXT,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                trace_id TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO team_workspace_new SELECT id, run_id, parent_id, agent_name,
                entry_type, content, trace_id, iteration,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM team_workspace;
            DROP TABLE team_workspace;
            ALTER TABLE team_workspace_new RENAME TO team_workspace;
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
            CREATE INDEX idx_team_ws_trace ON team_workspace(trace_id) WHERE trace_id IS NOT NULL;",
        )?;

        // --- heartbeat_sends ---
        tx.execute_batch(
            "CREATE TABLE heartbeat_sends_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO heartbeat_sends_new SELECT id, agent_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', sent_at, 'unixepoch')
            FROM heartbeat_sends;
            DROP TABLE heartbeat_sends;
            ALTER TABLE heartbeat_sends_new RENAME TO heartbeat_sends;
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);",
        )?;

        // --- reflection_runs ---
        tx.execute_batch(
            "CREATE TABLE reflection_runs_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO reflection_runs_new SELECT id, agent_id, status, changes_made, summary,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM reflection_runs;
            DROP TABLE reflection_runs;
            ALTER TABLE reflection_runs_new RENAME TO reflection_runs;
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);",
        )?;

        // --- customer_config ---
        tx.execute_batch(
            "CREATE TABLE customer_config_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );
            INSERT INTO customer_config_new SELECT agent_id, key, value,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM customer_config;
            DROP TABLE customer_config;
            ALTER TABLE customer_config_new RENAME TO customer_config;",
        )?;

        // --- failed_sends ---
        tx.execute_batch(
            "CREATE TABLE failed_sends_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO failed_sends_new SELECT id, agent_id, text, request_id, retry_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM failed_sends;
            DROP TABLE failed_sends;
            ALTER TABLE failed_sends_new RENAME TO failed_sends;",
        )?;

        // --- schema_version ---
        tx.execute_batch(
            "CREATE TABLE schema_version_new (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO schema_version_new SELECT version,
                strftime('%Y-%m-%dT%H:%M:%SZ', applied_at, 'unixepoch')
            FROM schema_version;
            DROP TABLE schema_version;
            ALTER TABLE schema_version_new RENAME TO schema_version;",
        )?;

        // Recreate unified_timeline VIEW
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // Record migration
        tx.execute("INSERT INTO schema_version (version) VALUES (12)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v12_to_v13(&mut self) -> Result<()> {
        info!("migrating database schema v12 → v13 (A2A orthogonal persistence)");

        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop view first — it references the tasks table we're about to rebuild
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        // Rebuild tasks table to add 'a2a' to trigger_type CHECK constraint
        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual','a2a')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            INSERT INTO tasks_new SELECT * FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;
            CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
            CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
            CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
            CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');
            CREATE UNIQUE INDEX idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
            CREATE UNIQUE INDEX idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');")?;

        // Create thin mapping table
        tx.execute_batch(
            "CREATE TABLE a2a_task_map (
                a2a_task_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                context_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_a2a_task_map_task ON a2a_task_map(task_id);",
        )?;

        // Create A2A tables with FK to mapping table
        tx.execute_batch(
            "CREATE TABLE a2a_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                artifact_id TEXT NOT NULL,
                name TEXT,
                description TEXT,
                parts TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE TABLE a2a_push_notification_configs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                token TEXT,
                auth_scheme TEXT,
                auth_credentials TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );",
        )?;

        // Recreate unified_timeline VIEW (tasks table was rebuilt)
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (13)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v13_to_v14(&self) -> Result<()> {
        info!("migrating database schema v13 → v14 (task metadata column)");

        // Simple ALTER TABLE — SQLite supports adding nullable columns without table rebuild.
        // Idempotent: check if column already exists before adding.
        if !self.column_exists("tasks", "metadata")? {
            self.conn
                .execute_batch("ALTER TABLE tasks ADD COLUMN metadata TEXT;")?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (14)", [])?;

        Ok(())
    }

    fn migrate_v14_to_v15(&self) -> Result<()> {
        info!("migrating database schema v14 → v15 (llm_calls + tool_calls tables)");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS llm_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                stop_reason TEXT,
                status TEXT NOT NULL DEFAULT 'success',
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_llm_calls_trace ON llm_calls(trace_id);
            CREATE INDEX IF NOT EXISTS idx_llm_calls_session ON llm_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_llm_calls_agent_created ON llm_calls(agent_id, created_at);

            CREATE TABLE IF NOT EXISTS tool_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                llm_call_id TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                tool_source TEXT NOT NULL DEFAULT 'builtin',
                skill_name TEXT,
                input TEXT,
                output TEXT,
                success INTEGER NOT NULL DEFAULT 1,
                non_zero_exit INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_tool_calls_trace ON tool_calls(trace_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_llm_call ON tool_calls(llm_call_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_agent_created ON tool_calls(agent_id, created_at);",
        )?;

        // Recreate unified_timeline VIEW with new UNION ALL legs
        self.conn
            .execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (15)", [])?;

        Ok(())
    }

    fn migrate_v15_to_v16(&self) -> Result<()> {
        info!("migrating database schema v15 → v16 (add step column to llm_calls)");

        if !self.column_exists("llm_calls", "step")? {
            self.conn.execute_batch(
                "ALTER TABLE llm_calls ADD COLUMN step INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (16)", [])?;

        Ok(())
    }

    fn migrate_v16_to_v17(&mut self) -> Result<()> {
        info!("migrating database schema v16 → v17 (add task dedup index on reference_url)");

        // Wrap in transaction for atomicity (matches pattern in migrate_v7_to_v8, etc.)
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "-- Step 1: Cancel duplicate active tasks with the same (agent_id, reference_url).
             -- Keep the earliest-created item per group (by rowid for deterministic tiebreaking
             -- when created_at timestamps collide), cancel the rest with a metadata breadcrumb.
             UPDATE tasks SET status = 'cancelled',
                 metadata = json_set(COALESCE(metadata, '{}'), '$.cancelled_reason', 'dedup_migration_v17')
             WHERE rowid IN (
                 SELECT t.rowid FROM tasks t
                 INNER JOIN (
                     SELECT agent_id, reference_url, MIN(rowid) as keeper_rowid
                     FROM tasks
                     WHERE trigger_type = 'manual'
                       AND reference_url IS NOT NULL
                       AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
                     GROUP BY agent_id, reference_url
                     HAVING COUNT(*) > 1
                 ) dups ON t.agent_id = dups.agent_id
                        AND t.reference_url = dups.reference_url
                        AND t.rowid != dups.keeper_rowid
                 WHERE t.trigger_type = 'manual'
                   AND t.status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             );

             -- Step 2: Create partial unique index. NULLs are exempt (SQLite skips NULL in
             -- unique indexes), so label-only dedup is handled at the tool level.
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_manual_active_ref_url
             ON tasks(agent_id, reference_url)
             WHERE trigger_type = 'manual'
               AND reference_url IS NOT NULL
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');

             INSERT INTO schema_version (version) VALUES (17);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v17_to_v18(&mut self) -> Result<()> {
        info!("migrating database schema v17 → v18 (widen reminder dedup index for resume_agent)");

        // The old index only covered action_type = 'send_message'. The new index covers
        // both 'send_message' and 'resume_agent' to prevent duplicate reminders regardless
        // of action type. See #363.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_tasks_unique_reminder;
             CREATE UNIQUE INDEX idx_tasks_unique_reminder
             ON tasks(agent_id, label COLLATE NOCASE)
             WHERE status IN ('pending', 'in_progress', 'recurring_active')
               AND (action_type = 'send_message' OR action_type = 'resume_agent')
               AND trigger_type NOT IN ('callback');

             INSERT INTO schema_version (version) VALUES (18);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v18_to_v19(&mut self) -> Result<()> {
        info!("migrating database schema v18 → v19 (add task_id to sessions for reverse lookup)");

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "ALTER TABLE sessions ADD COLUMN task_id TEXT;

             CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;

             -- Backfill from existing metadata JSON (cli --task-id sessions)
             UPDATE sessions SET task_id = json_extract(metadata, '$.task_id')
               WHERE json_extract(metadata, '$.task_id') IS NOT NULL AND task_id IS NULL;

             INSERT INTO schema_version (version) VALUES (19);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v19_to_v20(&mut self) -> Result<()> {
        info!("migrating database schema v19 → v20 (skill_overrides: llm_provider, llm_model)");

        // Idempotent: skip ALTER if columns already exist (defensive — re-runs).
        let has_provider = self.column_exists("skill_overrides", "llm_provider")?;
        let has_model = self.column_exists("skill_overrides", "llm_model")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_provider {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN llm_provider TEXT;\n");
        }
        if !has_model {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN llm_model TEXT;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (20);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v20_to_v21(&mut self) -> Result<()> {
        info!("migrating database schema v20 → v21 (llm_calls: prompt_variant)");

        let has_col = self.column_exists("llm_calls", "prompt_variant")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE llm_calls ADD COLUMN prompt_variant TEXT;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (21);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v21_to_v22(&mut self) -> Result<()> {
        info!("migrating database schema v21 → v22 (messages: internal flag)");

        let has_col = self.column_exists("messages", "internal")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE messages ADD COLUMN internal INTEGER NOT NULL DEFAULT 0;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (22);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v22_to_v23(&mut self) -> Result<()> {
        info!("migrating database schema v22 → v23 (tasks: type column)");

        let has_col = self.column_exists("tasks", "type")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            // SQLite 3.37+ supports CHECK constraints in ALTER TABLE ADD COLUMN.
            // The DEFAULT backfills all existing rows to 'issue', preserving behavior.
            sql.push_str(
                "ALTER TABLE tasks ADD COLUMN type TEXT NOT NULL DEFAULT 'issue' \
                 CHECK (type IN ('issue', 'milestone', 'project'));\n",
            );
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (23);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v23_to_v24(&mut self) -> Result<()> {
        info!("migrating database schema v23 → v24 (skill_overrides: enabled column)");

        let has_col = self.column_exists("skill_overrides", "enabled")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN enabled INTEGER;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (24);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    /// Migration v24 -> v25: Knowledge Graph schema tables.
    ///
    /// Adds 10 tables for the three-layer KG:
    /// - Domain layer: `kg_entities`, `kg_relationships`
    /// - Lexical layer: `kg_chunks`
    /// - Subject layer: `kg_subject_entities`, `kg_subject_resolutions`,
    ///   `kg_subject_relationships`
    /// - Provenance: `kg_chunk_subjects`, `kg_chunk_subject_relationships`
    /// - Tracking: `kg_extractions`, `kg_resolutions_log`
    fn migrate_v24_to_v25(&mut self) -> Result<()> {
        info!("migrating database schema v24 -> v25 (knowledge graph tables)");

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
                "-- KG domain layer (global, no agent_id)
                CREATE TABLE IF NOT EXISTS kg_entities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    entity_key TEXT NOT NULL UNIQUE,
                    type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    CHECK (entity_key = type || ':' || name)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_entities_type ON kg_entities(type);

                CREATE TABLE IF NOT EXISTS kg_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    type TEXT NOT NULL,
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                );
                CREATE INDEX IF NOT EXISTS idx_kg_rel_from ON kg_relationships(from_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_rel_to ON kg_relationships(to_entity_id, type);

                -- KG lexical layer (per-agent)
                CREATE TABLE IF NOT EXISTS kg_chunks (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    seq_id INTEGER NOT NULL,
                    source_doc_path TEXT NOT NULL,
                    source_doc_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, source_doc_path, seq_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_chunks_agent_doc ON kg_chunks(agent_id, source_doc_path);

                -- KG subject layer (per-agent)
                CREATE TABLE IF NOT EXISTS kg_subject_entities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    entity_key TEXT NOT NULL,
                    type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    CHECK (entity_key = type || ':' || name),
                    UNIQUE (agent_id, entity_key)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_subj_entities_agent_type ON kg_subject_entities(agent_id, type);

                CREATE TABLE IF NOT EXISTS kg_subject_resolutions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, subject_entity_id, domain_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

                -- KG subject-to-subject edges / fact triples (per-agent)
                CREATE TABLE IF NOT EXISTS kg_subject_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    type TEXT NOT NULL,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, from_entity_id, to_entity_id, type)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_from ON kg_subject_relationships(agent_id, from_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_to ON kg_subject_relationships(agent_id, to_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_type ON kg_subject_relationships(agent_id, type);

                -- KG entity provenance: chunk -> subject entity (many-to-many)
                CREATE TABLE IF NOT EXISTS kg_chunk_subjects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, chunk_id, subject_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_cs_chunk ON kg_chunk_subjects(agent_id, chunk_id);
                CREATE INDEX IF NOT EXISTS idx_kg_cs_entity ON kg_chunk_subjects(agent_id, subject_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_cs_trace ON kg_chunk_subjects(agent_id, extraction_trace_id);

                -- KG relationship provenance: chunk -> subject relationship (many-to-many)
                CREATE TABLE IF NOT EXISTS kg_chunk_subject_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                    subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, chunk_id, subject_relationship_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_csr_chunk ON kg_chunk_subject_relationships(agent_id, chunk_id);
                CREATE INDEX IF NOT EXISTS idx_kg_csr_rel ON kg_chunk_subject_relationships(agent_id, subject_relationship_id);

                -- KG extraction tracking.
                -- Historical shape as shipped at v25. `source_doc_hash` is
                -- added at v26 via ALTER TABLE in migrate_v25_to_v26 (#757);
                -- keeping migrate_v24_to_v25 as the record of v25's actual
                -- schema preserves migration immutability and means the
                -- convergence test exercises the ALTER path rather than
                -- skipping it.
                CREATE TABLE IF NOT EXISTS kg_extractions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    source_doc_path TEXT NOT NULL,
                    extraction_model TEXT NOT NULL,
                    entities_extracted INTEGER NOT NULL DEFAULT 0,
                    relationships_extracted INTEGER NOT NULL DEFAULT 0,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, source_doc_path)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_extractions_agent ON kg_extractions(agent_id);

                -- KG resolution tracking
                CREATE TABLE IF NOT EXISTS kg_resolutions_log (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    outcome TEXT NOT NULL CHECK (outcome IN (
                        'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                    )),
                    resolution_trace_id TEXT NOT NULL,
                    source_extraction_trace_id TEXT,
                    model TEXT,
                    duration_ms INTEGER,
                    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, subject_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

                INSERT INTO schema_version (version) VALUES (25);",
            )
            .context("failed to migrate v24 -> v25 (knowledge graph tables)")?;
        tx.commit()?;

        Ok(())
    }

    /// v25 -> v26: add nullable `source_doc_hash` column to `kg_extractions`
    /// so the pending-doc query can skip re-extraction when chunk content is
    /// unchanged (#757). Pre-existing rows get NULL and will re-extract once
    /// on the next run (bounded by MIKA_KG_BATCH_BUDGET), then populate the
    /// hash on success so subsequent runs are no-ops.
    ///
    /// Idempotent at the migration-chain level (the version gate in
    /// `run_migrations` prevents re-run); `column_exists` guards the inner
    /// ALTER against manual invocation.
    fn migrate_v25_to_v26(&mut self) -> Result<()> {
        info!("migrating database schema v25 -> v26 (kg_extractions.source_doc_hash)");

        if !self.column_exists("kg_extractions", "source_doc_hash")? {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "ALTER TABLE kg_extractions ADD COLUMN source_doc_hash TEXT;
                 INSERT INTO schema_version (version) VALUES (26);",
            )
            .context("failed to migrate v25 -> v26 (add kg_extractions.source_doc_hash)")?;
            tx.commit()?;
        } else {
            // Column already exists (manual re-run in a test / recovery scenario).
            // Wrap in TransactionBehavior::Immediate to match the true-branch envelope
            // (mika#1391): bare INSERT could leave column-exists + schema_version-not-bumped
            // inconsistent state on failure mid-recovery.
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO schema_version (version) VALUES (26)", [])?;
            tx.commit()?;
        }

        Ok(())
    }

    /// v26 -> v27: Schema v27 — docs_root_hash as shared-corpus primary key (#786 + #787).
    ///
    /// Two-phase migration in a single transaction:
    /// 1. **DDL** (#786): Renames six shared-layer KG tables to `*_v26_backup`,
    ///    creates fresh v27 tables keyed by `docs_root_hash` instead of `agent_id`.
    ///    Rebuilds per-agent tables to fix FK refs.
    /// 2. **Coalesce** (#787): Reads from backup tables, deduplicates across agents
    ///    via majority-vote (normalized entity_key, agent-count tiebreak), rewires
    ///    FKs via temp lookup tables, drops backups, writes `v27_coalesce_complete`
    ///    marker to `schema_meta`. `docs_root` resolved from `MIKA_KG_DOCS_ROOT`
    ///    env var or CWD fallback.
    fn migrate_v26_to_v27(&mut self) -> Result<()> {
        info!("migrating database schema v26 -> v27 (docs_root_hash shared-corpus)");

        // Idempotency guard: if kg_chunks already has docs_root_hash, we've run.
        if self.column_exists("kg_chunks", "docs_root_hash")? {
            // Already upgraded — just record the version bump.
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (27)", [])?;
            return Ok(());
        }

        // Resolve docs_root for v26 data coalescing.
        let docs_root_path = std::env::var("MIKA_KG_DOCS_ROOT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("docs")
                    .join("solutions")
            });
        let docs_root_escaped = docs_root_path.to_string_lossy().replace('\'', "''");
        let docs_root_hash = crate::kg::config::hash_docs_root(&docs_root_path);

        info!(
            docs_root = %docs_root_path.display(),
            docs_root_hash = %docs_root_hash,
            "v26->v27 coalesce: resolved docs_root for migration"
        );

        // Log pre-coalesce counts from v26 tables (before DDL renames them).
        let pre_chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_entities: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_entities", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_relationships: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_relationships", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let pre_extractions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_extractions", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_resolutions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_resolutions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let pre_res_log: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            pre_chunks,
            pre_entities,
            pre_relationships,
            pre_extractions,
            pre_resolutions,
            pre_res_log,
            "v26->v27 coalesce: pre-migration row counts"
        );

        // Generate the coalesce SQL for the resolved docs_root.
        let coalesce = v27_coalesce_sql(&docs_root_escaped, &docs_root_hash);

        // Build the full migration: DDL (rename to backup + create v27 tables) +
        // coalesce (read from backups, dedup, write to v27, drop backups) + finalize.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let sql = format!(
            "-- Create schema_meta table for migration state tracking.
                 CREATE TABLE IF NOT EXISTS schema_meta (
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );

                 -- Drop v25/v26 indexes that will be recreated (SQLite keeps
                 -- index names across ALTER TABLE RENAME, so they'd conflict).
                 DROP INDEX IF EXISTS idx_kg_chunks_agent_doc;
                 DROP INDEX IF EXISTS idx_kg_subj_entities_agent_type;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_from;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_to;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_type;
                 DROP INDEX IF EXISTS idx_kg_cs_chunk;
                 DROP INDEX IF EXISTS idx_kg_cs_entity;
                 DROP INDEX IF EXISTS idx_kg_cs_trace;
                 DROP INDEX IF EXISTS idx_kg_csr_chunk;
                 DROP INDEX IF EXISTS idx_kg_csr_rel;
                 DROP INDEX IF EXISTS idx_kg_extractions_agent;
                 DROP INDEX IF EXISTS idx_kg_resolutions_agent_subj;
                 DROP INDEX IF EXISTS idx_kg_resolutions_agent_dom;
                 DROP INDEX IF EXISTS idx_kg_res_log_pending;

                 -- 1. kg_chunks: rename to backup, create v27 table.
                 ALTER TABLE kg_chunks RENAME TO kg_chunks_v26_backup;
                 CREATE TABLE kg_chunks (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     seq_id INTEGER NOT NULL,
                     source_doc_path TEXT NOT NULL,
                     source_doc_hash TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (docs_root_hash, source_doc_path, seq_id)
                 );
                 CREATE INDEX idx_kg_chunks_docs_root_hash_doc ON kg_chunks(docs_root_hash, source_doc_path);

                 -- 2. kg_subject_entities: rename to backup, create v27 table.
                 ALTER TABLE kg_subject_entities RENAME TO kg_subject_entities_v26_backup;
                 CREATE TABLE kg_subject_entities (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     entity_key TEXT NOT NULL,
                     type TEXT NOT NULL,
                     name TEXT NOT NULL,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     properties_json TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     CHECK (entity_key = type || ':' || name),
                     UNIQUE (docs_root_hash, entity_key)
                 );
                 CREATE INDEX idx_kg_subj_entities_drh_type ON kg_subject_entities(docs_root_hash, type);

                 -- 3. kg_subject_relationships: rename to backup, create v27 table.
                 ALTER TABLE kg_subject_relationships RENAME TO kg_subject_relationships_v26_backup;
                 CREATE TABLE kg_subject_relationships (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     type TEXT NOT NULL,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     properties_json TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
                 );
                 CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(docs_root_hash, from_entity_id, type);
                 CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(docs_root_hash, to_entity_id, type);
                 CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(docs_root_hash, type);

                 -- 4. kg_chunk_subjects: rename to backup, create v27 table.
                 ALTER TABLE kg_chunk_subjects RENAME TO kg_chunk_subjects_v26_backup;
                 CREATE TABLE kg_chunk_subjects (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
                 );
                 CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(docs_root_hash, chunk_id);
                 CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash, subject_entity_id);
                 CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(docs_root_hash, extraction_trace_id);

                 -- 5. kg_chunk_subject_relationships: rename to backup, create v27 table.
                 ALTER TABLE kg_chunk_subject_relationships RENAME TO kg_chunk_subject_relationships_v26_backup;
                 CREATE TABLE kg_chunk_subject_relationships (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                     subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
                 );
                 CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(docs_root_hash, chunk_id);
                 CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(docs_root_hash, subject_relationship_id);

                 -- 6. kg_extractions: rename to backup, create v27 table.
                 ALTER TABLE kg_extractions RENAME TO kg_extractions_v26_backup;
                 CREATE TABLE kg_extractions (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     source_doc_path TEXT NOT NULL,
                     source_doc_hash TEXT,
                     extraction_model TEXT NOT NULL,
                     entities_extracted INTEGER NOT NULL DEFAULT 0,
                     relationships_extracted INTEGER NOT NULL DEFAULT 0,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, source_doc_path)
                 );
                 CREATE INDEX idx_kg_extractions_drh ON kg_extractions(docs_root_hash);

                 -- 7. kg_subject_resolutions: rebuild to fix FK refs broken by
                 -- kg_subject_entities rename (SQLite rewrites FK targets on
                 -- ALTER TABLE RENAME).
                 ALTER TABLE kg_subject_resolutions RENAME TO kg_subject_resolutions_v26_backup;
                 CREATE TABLE kg_subject_resolutions (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (agent_id, subject_entity_id, domain_entity_id)
                 );
                 CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
                 CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

                 -- 8. kg_resolutions_log: rebuild to fix FK refs broken by
                 -- kg_subject_entities rename.
                 ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v26_backup;
                 CREATE TABLE kg_resolutions_log (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     outcome TEXT NOT NULL CHECK (outcome IN (
                         'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                     )),
                     resolution_trace_id TEXT NOT NULL,
                     source_extraction_trace_id TEXT,
                     model TEXT,
                     duration_ms INTEGER,
                     resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (agent_id, subject_entity_id)
                 );
                 CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

                 -- v27 coalesce: read from backup tables, dedup, write to v27 tables.
                 {coalesce}

                 INSERT INTO schema_version (version) VALUES (27);"
        );

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(&sql)
            .context("failed to migrate v26 -> v27 (docs_root_hash shared-corpus)")?;
        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        // Log post-coalesce counts from the new v27 tables.
        let post_chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap_or(0);
        let post_entities: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_entities", [], |r| r.get(0))
            .unwrap_or(0);
        let post_relationships: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_relationships", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let post_extractions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_extractions", [], |r| r.get(0))
            .unwrap_or(0);
        let post_resolutions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_resolutions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let post_res_log: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            post_chunks,
            post_entities,
            post_relationships,
            post_extractions,
            post_resolutions,
            post_res_log,
            chunks_deduped = pre_chunks - post_chunks,
            entities_deduped = pre_entities - post_entities,
            relationships_deduped = pre_relationships - post_relationships,
            extractions_deduped = pre_extractions - post_extractions,
            "v26->v27 coalesce: migration complete"
        );

        if pre_chunks > 0 && post_chunks == 0 {
            warn!("v26->v27 coalesce: all chunks were lost — this may indicate a migration bug");
        }

        Ok(())
    }

    /// v27→v28: Add `agent_kg_corpora` table for multi-corpus per-agent KG (#798).
    /// Maps `agent_id → {docs_root_hash, docs_root_path}` so the query path knows
    /// which corpora to fan out across without re-deriving from identity.
    /// Backfills from existing `kg_subject_resolutions → kg_subject_entities` joins.
    fn migrate_v27_to_v28(&mut self) -> Result<()> {
        // Idempotency: skip if table already exists.
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_kg_corpora'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if exists {
            self.conn.execute(
                "UPDATE schema_version SET version = 28 WHERE version < 28",
                [],
            )?;
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE agent_kg_corpora (
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 docs_root_hash TEXT NOT NULL,
                 docs_root_path TEXT NOT NULL,
                 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 PRIMARY KEY (agent_id, docs_root_hash)
             );

             CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);

             INSERT OR IGNORE INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path)
                 SELECT DISTINCT r.agent_id, e.docs_root_hash, e.docs_root
                 FROM kg_subject_resolutions r
                 JOIN kg_subject_entities e ON e.id = r.subject_entity_id
                 WHERE e.docs_root_hash IS NOT NULL AND e.docs_root IS NOT NULL;

             UPDATE schema_version SET version = 28;",
        )?;
        tx.commit()?;

        Ok(())
    }

    /// Backfill migration: scrub secret-shaped values from existing tool_calls
    /// rows (#908). Data-only — no DDL changes. Applies `scrub_secrets()` to
    /// `input` and `output` columns, updating only rows that change.
    fn migrate_v28_to_v29(&mut self) -> Result<()> {
        use crate::secret_scrubber::scrub_secrets;

        let version = self.schema_version()?;
        if version >= 29 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Collect IDs and content of rows that have non-NULL text fields.
        let mut stmt = tx.prepare(
            "SELECT id, input, output, error_message FROM tool_calls
             WHERE input IS NOT NULL OR output IS NOT NULL OR error_message IS NOT NULL",
        )?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut updated = 0u64;
        for (id, input, output, error_msg) in &rows {
            let scrubbed_input = input.as_deref().map(scrub_secrets);
            let scrubbed_output = output.as_deref().map(scrub_secrets);
            let scrubbed_error = error_msg.as_deref().map(scrub_secrets);

            // Only UPDATE if scrubbing changed at least one field.
            let input_changed = matches!(&scrubbed_input, Some(std::borrow::Cow::Owned(_)));
            let output_changed = matches!(&scrubbed_output, Some(std::borrow::Cow::Owned(_)));
            let error_changed = matches!(&scrubbed_error, Some(std::borrow::Cow::Owned(_)));

            if input_changed || output_changed || error_changed {
                tx.execute(
                    "UPDATE tool_calls SET input = ?1, output = ?2, error_message = ?3 WHERE id = ?4",
                    params![
                        scrubbed_input.as_deref(),
                        scrubbed_output.as_deref(),
                        scrubbed_error.as_deref(),
                        id,
                    ],
                )?;
                updated += 1;
            }
        }

        tx.execute("UPDATE schema_version SET version = 29", [])?;
        tx.commit()?;

        if updated > 0 {
            info!(
                updated_rows = updated,
                total_rows = rows.len(),
                "v28→v29: scrubbed secrets from existing tool_calls rows"
            );
        }

        Ok(())
    }

    /// v29→v30: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'matched_llm_db_fallback'` (#874). Table rebuild mirroring the v26→v27
    /// shape: RENAME → CREATE → INSERT INTO ... SELECT → DROP → recreate index.
    fn migrate_v29_to_v30(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 30 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v29_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v29_backup;

             DROP TABLE kg_resolutions_log_v29_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 30;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v29→v30: expanded kg_resolutions_log outcome CHECK constraint (#874)"
        );

        Ok(())
    }

    fn migrate_v30_to_v31(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 31 {
            return Ok(());
        }

        // Columns may already exist in clean-slate DBs; check each independently
        // to handle partial-crash recovery scenarios.
        let has_response_text = self.column_exists("llm_calls", "response_text")?;
        let has_reasoning = self.column_exists("llm_calls", "reasoning")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_response_text {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN response_text TEXT;")?;
        }
        if !has_reasoning {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN reasoning TEXT;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (31)", [])?;
        tx.commit()?;

        info!("v30→v31: added response_text and reasoning columns to llm_calls (#653)");

        Ok(())
    }

    /// v31→v32: Add `kg_invalidated_no_match` sidecar table (#961).
    ///
    /// Ephemeral marker table for tracking entities whose `no_match` resolution
    /// log rows were deleted by domain-graph rebuild invalidation (#960).
    /// The resolver reads and cleans up these markers during resolution.
    fn migrate_v31_to_v32(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 32 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_invalidated_no_match (
                subject_entity_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (subject_entity_id, agent_id)
            );",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (32)", [])?;
        tx.commit()?;

        info!("v31→v32: added kg_invalidated_no_match sidecar table (#961)");

        Ok(())
    }

    fn migrate_v32_to_v33(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 33 {
            return Ok(());
        }

        // #1052: Delete kg_extractions rows with NULL source_doc_hash.
        // These are pre-v26 rows that escaped the v27 backfill due to
        // NULL = NULL being falsy in SQL. They create a deadlock: the
        // pending query says "extract me" but INSERT OR IGNORE (now
        // replaced with upsert in #1052) would skip them. Deleting makes
        // them cleanly pending for re-extraction with the new upsert.
        // Safe because kg_extractions is an idempotency marker, not data.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted: usize = tx.execute(
            "DELETE FROM kg_extractions WHERE source_doc_hash IS NULL",
            [],
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (33)", [])?;
        tx.commit()?;

        if deleted > 0 {
            info!(
                deleted = deleted,
                "v32→v33: deleted {deleted} NULL-hash kg_extractions rows (#1052)"
            );
        } else {
            info!("v32→v33: no NULL-hash kg_extractions rows to clean up (#1052)");
        }

        Ok(())
    }

    fn migrate_v33_to_v34(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 34 {
            return Ok(());
        }

        // #1001: Add dispatch_class column for per-class dispatch slot split.
        // Nullable — pre-v34 rows stay NULL, treated as 'implement' via COALESCE
        // in the dispatch guard query. CHECK constraint limits values.
        // Column-exists guard for crash-recovery and convergence-test safety.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_col: bool = tx
            .prepare("PRAGMA table_info(tasks)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "dispatch_class");
        if !has_col {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN dispatch_class TEXT
                   CHECK (dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom'));",
            )?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_dispatch_class
               ON tasks(agent_id, dispatch_class, status)
               WHERE dispatch_class IS NOT NULL;",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (34)", [])?;
        tx.commit()?;

        info!("v33→v34: added dispatch_class column to tasks (#1001)");
        Ok(())
    }

    /// v34→v35: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'no_candidate_of_type'` (#1154). Table rebuild mirroring the v29→v30
    /// shape: RENAME → CREATE → INSERT INTO ... SELECT → DROP → recreate index.
    fn migrate_v34_to_v35(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 35 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v34_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'no_candidate_of_type',
                     'skipped_discovered_type', 'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v34_backup;

             DROP TABLE kg_resolutions_log_v34_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 35;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v34→v35: expanded kg_resolutions_log outcome CHECK to include 'no_candidate_of_type' (#1154)"
        );

        Ok(())
    }

    /// v35→v36: Add `discovered` and `discovery_reason` columns to
    /// `kg_subject_entities` for roster-grounding (#1158).
    fn migrate_v35_to_v36(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 36 {
            return Ok(());
        }

        // Column-exists guards for crash-recovery safety (per v30→v31 precedent).
        let has_discovered = self.column_exists("kg_subject_entities", "discovered")?;
        let has_discovery_reason = self.column_exists("kg_subject_entities", "discovery_reason")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if !has_discovered {
            tx.execute(
                "ALTER TABLE kg_subject_entities ADD COLUMN discovered INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !has_discovery_reason {
            tx.execute(
                "ALTER TABLE kg_subject_entities ADD COLUMN discovery_reason TEXT",
                [],
            )?;
        }

        tx.execute("UPDATE schema_version SET version = 36", [])?;
        tx.commit()?;

        info!(
            "v35→v36: added discovered + discovery_reason columns to kg_subject_entities (#1158)"
        );

        Ok(())
    }

    /// v36→v37: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'skipped_discovered_subject'` (#1158). Table rebuild.
    fn migrate_v36_to_v37(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 37 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v36_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'no_candidate_of_type',
                     'skipped_discovered_type', 'skipped_discovered_subject',
                     'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v36_backup;

             DROP TABLE kg_resolutions_log_v36_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 37;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v36→v37: expanded kg_resolutions_log outcome CHECK to include 'skipped_discovered_subject' (#1158)"
        );

        Ok(())
    }

    /// v37→v38: Add `system_prompt_bytes` column to `llm_calls` (mika#1217).
    ///
    /// Per-call assembled-system-prompt byte count for context-budget
    /// observability. Nullable; pre-v38 rows stay NULL. Mirrors v30→v31's
    /// additive-nullable shape and the `column_exists` guard pattern.
    fn migrate_v37_to_v38(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 38 {
            return Ok(());
        }

        let has_column = self.column_exists("llm_calls", "system_prompt_bytes")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_column {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN system_prompt_bytes INTEGER;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (38)", [])?;
        tx.commit()?;

        info!("v37→v38: added system_prompt_bytes column to llm_calls (mika#1217)");

        Ok(())
    }

    /// v38→v39: Add `operational_items` table (mika#1262).
    ///
    /// Canonical operational-item ledger for the What's Next engine.
    /// New table with indexes, no existing table changes.
    fn migrate_v38_to_v39(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 39 {
            return Ok(());
        }

        let has_table: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='operational_items'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if !has_table {
            tx.execute_batch(
                "CREATE TABLE operational_items (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
                    title TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
                    owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
                    owner_name TEXT,
                    priority REAL NOT NULL DEFAULT 0.0,
                    user_importance REAL NOT NULL DEFAULT 0.0,
                    due_at TEXT,
                    blocked_by TEXT,
                    next_action TEXT,
                    evidence_refs TEXT NOT NULL DEFAULT '[]',
                    confidence REAL NOT NULL DEFAULT 1.0,
                    source_table TEXT,
                    source_id TEXT,
                    agent_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
                CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
                CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
                CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
                CREATE UNIQUE INDEX idx_operational_items_source_unique
                    ON operational_items(agent_id, source_table, source_id)
                    WHERE source_table IS NOT NULL AND source_id IS NOT NULL;",
            )?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (39)", [])?;
        tx.commit()?;

        info!("v38→v39: added operational_items table (mika#1262)");

        Ok(())
    }

    /// v39→v40: Delete all mika-relay agent data (mika#1193).
    ///
    /// Self-contained: explicit deletes in reverse-dependency order. Correctness
    /// does NOT depend on PRAGMA foreign_keys being ON.
    fn migrate_v39_to_v40(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 40 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Delete in reverse-dependency order: descendants before ancestors.
        // Each statement is idempotent (no-op if rows are already gone).
        tx.execute_batch(
            "-- v40: mika#1193 retire mika-relay agent.
            DELETE FROM tool_calls
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM llm_calls
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM messages
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM skill_overrides
              WHERE agent_id = 'mika-relay';

            DELETE FROM audit_events
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM operational_items
              WHERE agent_id = 'mika-relay';

            DELETE FROM tasks
              WHERE agent_id = 'mika-relay';

            DELETE FROM sessions
              WHERE agent_id = 'mika-relay';

            DELETE FROM agents
              WHERE id = 'mika-relay';",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (40)", [])?;
        tx.commit()?;

        info!("v39→v40: deleted mika-relay agent data (mika#1193)");

        Ok(())
    }

    /// v40→v41: Add `task_messages` parallel narrative table (mika#974).
    ///
    /// Additive — no existing table altered, no data touched. Safe on live DB.
    fn migrate_v40_to_v41(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 41 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v41: mika#974 task_messages parallel narrative table.
            CREATE TABLE IF NOT EXISTS task_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                agent_id   TEXT NOT NULL,
                session_id TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                trace_id   TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE INDEX IF NOT EXISTS idx_task_messages_task_created
                ON task_messages (task_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_task_messages_agent_created
                ON task_messages (agent_id, created_at);

            INSERT INTO schema_version (version) VALUES (41);",
        )?;

        tx.commit()?;

        info!("v40→v41: added task_messages table (mika#974)");

        Ok(())
    }

    /// v41→v42: Add `auto_pull_stats` circuit-breaker tracking table (mika#1363).
    fn migrate_v41_to_v42(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 42 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v42: mika#1363 auto-pull circuit-breaker stats table.
            CREATE TABLE IF NOT EXISTS auto_pull_stats (
                repo_full_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                last_auto_pull_at TEXT,
                last_failure_at TEXT,
                PRIMARY KEY (repo_full_name, issue_number)
            );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (42)", [])?;
        tx.commit()?;

        info!("v41→v42: created auto_pull_stats table (mika#1363)");

        Ok(())
    }

    /// v42→v43: Add `lifecycle_state`, `use_count`, `last_used_at` columns to
    /// `skill_overrides` for curator background task (mika#1584).
    fn migrate_v42_to_v43(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 43 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Additive ALTER TABLE with column_exists guard for crash-recovery safety.
        if !Self::column_exists_tx(&tx, "skill_overrides", "lifecycle_state")? {
            tx.execute_batch(
                "ALTER TABLE skill_overrides ADD COLUMN lifecycle_state TEXT
                 CHECK (lifecycle_state IN ('staged', 'active', 'archived'));",
            )?;
        }
        if !Self::column_exists_tx(&tx, "skill_overrides", "use_count")? {
            tx.execute_batch(
                "ALTER TABLE skill_overrides ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !Self::column_exists_tx(&tx, "skill_overrides", "last_used_at")? {
            tx.execute_batch("ALTER TABLE skill_overrides ADD COLUMN last_used_at TEXT;")?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (43)", [])?;
        tx.commit()?;

        info!(
            "v42→v43: added lifecycle_state, use_count, last_used_at to skill_overrides (mika#1584)"
        );

        Ok(())
    }

    /// v43→v44: additive `permission_decisions` provenance ledger (mika#1733 AC4).
    ///
    /// Records every operator permission decision routed through
    /// `PermissionsChannel::resolve_decision`, including the classifier
    /// verdict, operator ratification, derived `override_used` flag, and the
    /// scope (tenant/agent) at decision time. Additive-only — no rebuild of
    /// existing tables. Two indexes support the two expected query shapes:
    /// per-request lookup and time-window scans.
    fn migrate_v43_to_v44(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 44 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS permission_decisions (
                id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_summary TEXT,
                classifier_verdict TEXT NOT NULL
                    CHECK (classifier_verdict IN ('approved', 'denied', 'held')),
                operator_decision TEXT
                    CHECK (operator_decision IN ('approve', 'deny')),
                override_used INTEGER NOT NULL DEFAULT 0
                    CHECK (override_used IN (0, 1)),
                decision_authority TEXT NOT NULL
                    CHECK (decision_authority IN ('strict', 'override')),
                tenant_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_permission_decisions_request_id
                ON permission_decisions(request_id);
            CREATE INDEX IF NOT EXISTS idx_permission_decisions_created_at
                ON permission_decisions(created_at DESC);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (44)", [])?;
        tx.commit()?;

        info!("v43→v44: added permission_decisions provenance table (mika#1733 AC4)");

        Ok(())
    }

    /// Insert a permission-decision provenance record (mika#1733 AC4). All
    /// fields correspond 1:1 to the v44 schema columns. `override_used` is
    /// derived by the caller and asserted at the CHECK constraint here as
    /// defense-in-depth.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_permission_decision(
        &self,
        id: &str,
        request_id: &str,
        tool_name: &str,
        args_summary: Option<&str>,
        classifier_verdict: &str,
        operator_decision: Option<&str>,
        override_used: bool,
        decision_authority: &str,
        tenant_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO permission_decisions (
                id, request_id, tool_name, args_summary,
                classifier_verdict, operator_decision, override_used,
                decision_authority, tenant_id, agent_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                request_id,
                tool_name,
                args_summary,
                classifier_verdict,
                operator_decision,
                if override_used { 1i64 } else { 0i64 },
                decision_authority,
                tenant_id,
                agent_id,
            ],
        )?;
        Ok(())
    }

    /// Check if a column exists on a table within a transaction scope.
    fn column_exists_tx(tx: &rusqlite::Transaction<'_>, table: &str, column: &str) -> Result<bool> {
        let mut stmt = tx.prepare(&format!("PRAGMA table_info('{table}')"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .any(|name| name.as_ref().is_ok_and(|n| n == column));
        Ok(exists)
    }

    /// v27 startup guard: refuse to open the database if the coalesce step
    /// from #787 has not run. Pins to `schema_version == 27` — future v28
    /// should carry its own guard, not inherit v27's.
    fn check_v27_coalesce_guard(&self) -> Result<()> {
        let schema_version = self.schema_version()?;
        if schema_version != 27 {
            return Ok(());
        }

        // Check for schema_meta table existence first (fresh installs have it).
        let has_meta: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_meta {
            anyhow::bail!(
                "KG v27 migration incomplete — coalesce step from mika#787 has not run. \
                 Deploy #787 before starting. See mika#786 and mika#787."
            );
        }

        let has_marker: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM schema_meta WHERE key = 'v27_coalesce_complete'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_marker {
            anyhow::bail!(
                "KG v27 migration incomplete — coalesce step from mika#787 has not run. \
                 Deploy #787 before starting. See mika#786 and mika#787."
            );
        }

        Ok(())
    }

    /// Check if a column exists on a table (used for idempotent migrations).
    fn column_exists(&self, table: &'static str, column: &'static str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info('{table}')"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .any(|name| name.as_ref().is_ok_and(|n| n == column));
        Ok(exists)
    }

    // ===== Agent CRUD =====

    /// Register an agent-corpus mapping. Idempotent via INSERT OR IGNORE.
    /// Called per (agent, corpus) pair during startup lexical ingestion (#798).
    pub fn register_agent_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
        docs_root_path: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path)
             VALUES (?1, ?2, ?3)",
            params![agent_id, docs_root_hash, docs_root_path],
        )?;
        Ok(())
    }

    pub fn register_agent(&self, id: &str, name: &str, home_dir: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO agents (id, name, home_dir) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               home_dir = CASE WHEN excluded.home_dir != '' THEN excluded.home_dir ELSE agents.home_dir END,
               name = excluded.name",
            params![id, name, home_dir],
        )?;
        Ok(())
    }

    /// Get the display name for an agent, falling back to the ID if not found.
    pub fn get_agent_display_name(&self, id: &str) -> String {
        self.conn
            .query_row("SELECT name FROM agents WHERE id = ?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|_| id.to_string())
    }

    pub fn update_agent_last_seen(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agents SET last_seen = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn list_agents_db(&self) -> Result<Vec<AgentRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, home_dir, active, last_seen, created_at FROM agents ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AgentRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    home_dir: r.get(2)?,
                    active: r.get(3)?,
                    last_seen: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Skill Overrides =====

    /// Get all skill overrides for an agent.
    pub fn get_skill_overrides(&self, agent_id: &str) -> Result<Vec<SkillOverride>> {
        let mut stmt = self.conn.prepare(
            "SELECT skill_name, always_on, llm_provider, llm_model, enabled,
                    lifecycle_state, use_count, last_used_at
             FROM skill_overrides WHERE agent_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
                Ok(SkillOverride {
                    skill_name: r.get(0)?,
                    always_on: r.get(1)?,
                    llm_provider: r.get(2)?,
                    llm_model: r.get(3)?,
                    enabled: r.get(4)?,
                    lifecycle_state: r.get(5)?,
                    use_count: r.get(6)?,
                    last_used_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Set (upsert) an always_on override for a skill.
    pub fn set_skill_override(
        &self,
        agent_id: &str,
        skill_name: &str,
        always_on: bool,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skill_overrides (agent_id, skill_name, always_on)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET always_on = excluded.always_on",
            params![agent_id, skill_name, always_on],
        )?;
        Ok(())
    }

    /// Set (upsert) an LLM provider/model override for a skill.
    /// Preserves existing `always_on` via the conflict clause.
    pub fn set_skill_llm_override(
        &self,
        agent_id: &str,
        skill_name: &str,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO skill_overrides (agent_id, skill_name, llm_provider, llm_model)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET
               llm_provider = excluded.llm_provider,
               llm_model    = excluded.llm_model",
            params![agent_id, skill_name, provider, model],
        )?;
        Ok(())
    }

    /// Set (upsert) an enabled override for a skill.
    ///
    /// `enabled = false` disables the skill; `enabled = true` explicitly enables it.
    /// When setting to `true` (the default) and all other override columns are NULL,
    /// the row is deleted (default-equals-delete).
    pub fn set_skill_enabled(
        &mut self,
        agent_id: &str,
        skill_name: &str,
        enabled: bool,
    ) -> Result<()> {
        let db_val: Option<bool> = if enabled { None } else { Some(false) };
        // RAII transaction: Drop without commit() auto-rolls back, preventing
        // stuck transactions that pin the WAL snapshot (mika#636).
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO skill_overrides (agent_id, skill_name, enabled)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, skill_name) DO UPDATE SET enabled = excluded.enabled",
            params![agent_id, skill_name, db_val],
        )?;
        // Default-equals-delete: if all columns are NULL, remove the row.
        if enabled {
            tx.execute(
                "DELETE FROM skill_overrides
                  WHERE agent_id = ?1 AND skill_name = ?2
                    AND always_on IS NULL
                    AND llm_provider IS NULL
                    AND llm_model IS NULL
                    AND enabled IS NULL
                    AND lifecycle_state IS NULL",
                params![agent_id, skill_name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Set the lifecycle state for an agent-authored skill (mika#1582).
    ///
    /// Valid states: `staged`, `active`, `archived`. The CHECK constraint on
    /// the column enforces this at the SQL layer.
    pub fn set_skill_lifecycle_state(
        &mut self,
        agent_id: &str,
        skill_name: &str,
        state: &str,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let affected = tx.execute(
            "UPDATE skill_overrides SET lifecycle_state = ?3
             WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name, state],
        )?;
        if affected == 0 {
            anyhow::bail!("no skill_overrides row for agent={agent_id}, skill={skill_name}");
        }
        tx.commit()?;
        Ok(())
    }

    /// Get the lifecycle state for a skill (mika#1582).
    ///
    /// Returns `None` if no override row exists or `lifecycle_state` is NULL.
    pub fn get_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
    ) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT lifecycle_state FROM skill_overrides
             WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
            |r| r.get(0),
        );
        match result {
            Ok(state) => Ok(state),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Clear the LLM override columns for a skill. If the resulting row has no
    /// remaining override values (all columns NULL), the row is deleted.
    ///
    /// The UPDATE and prune DELETE are wrapped in an atomic transaction so a
    /// crash between them cannot leave a half-cleared row.
    pub fn delete_skill_llm_override(&mut self, agent_id: &str, skill_name: &str) -> Result<()> {
        // RAII transaction: Drop without commit() auto-rolls back, preventing
        // stuck transactions that pin the WAL snapshot (mika#636).
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE skill_overrides
                SET llm_provider = NULL, llm_model = NULL
              WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
        )?;
        tx.execute(
            "DELETE FROM skill_overrides
              WHERE agent_id = ?1 AND skill_name = ?2
                AND always_on IS NULL
                AND llm_provider IS NULL
                AND llm_model IS NULL
                AND enabled IS NULL
                AND lifecycle_state IS NULL",
            params![agent_id, skill_name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Delete an override for a skill (revert to bundled default).
    pub fn delete_skill_override(&self, agent_id: &str, skill_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM skill_overrides WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
        )?;
        Ok(())
    }

    /// Batch-increment usage counters for all injected skills in a single turn.
    /// Creates rows for skills without prior overrides via UPSERT.
    pub fn increment_skill_usage(&self, agent_id: &str, skill_names: &[String]) -> Result<()> {
        if skill_names.is_empty() {
            return Ok(());
        }
        let now = crate::timestamp::now();
        let tx = self.conn.unchecked_transaction()?;
        for name in skill_names {
            tx.execute(
                "INSERT INTO skill_overrides (agent_id, skill_name, use_count, last_used_at)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(agent_id, skill_name) DO UPDATE SET
                    use_count = use_count + 1,
                    last_used_at = ?3",
                params![agent_id, name, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Query skills eligible for archival by the curator.
    /// Only considers agent-authored skills with `lifecycle_state = 'active'`
    /// that have been idle beyond `max_idle_days`.
    pub fn get_archival_candidates(
        &self,
        agent_id: &str,
        max_idle_days: u32,
    ) -> Result<Vec<SkillOverride>> {
        let cutoff = crate::timestamp::now_minus(chrono::Duration::days(i64::from(max_idle_days)));
        let mut stmt = self.conn.prepare(
            "SELECT skill_name, always_on, llm_provider, llm_model, enabled,
                    lifecycle_state, use_count, last_used_at
             FROM skill_overrides
             WHERE agent_id = ?1
               AND lifecycle_state = 'active'
               AND (
                 (last_used_at IS NULL AND use_count = 0)
                 OR last_used_at < ?2
               )",
        )?;
        let rows = stmt
            .query_map(params![agent_id, cutoff], |r| {
                Ok(SkillOverride {
                    skill_name: r.get(0)?,
                    always_on: r.get(1)?,
                    llm_provider: r.get(2)?,
                    llm_model: r.get(3)?,
                    enabled: r.get(4)?,
                    lifecycle_state: r.get(5)?,
                    use_count: r.get(6)?,
                    last_used_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Update the lifecycle_state of a skill override.
    pub fn update_skill_lifecycle_state(
        &self,
        agent_id: &str,
        skill_name: &str,
        state: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE skill_overrides SET lifecycle_state = ?3
             WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name, state],
        )?;
        Ok(())
    }

    /// Retrieve the most recent curator proposal from audit_events.
    pub fn get_latest_curator_proposal(&self, agent_id: &str) -> Result<Option<(String, String)>> {
        self.query_row_2(
            "SELECT after_value, created_at FROM audit_events
             WHERE tool_name = 'curator_review' AND target_key = 'curator_proposal'
               AND agent_id = ?1
             ORDER BY created_at DESC LIMIT 1",
            &[&agent_id as &dyn rusqlite::types::ToSql],
        )
    }

    // ===== Team CRUD =====

    pub fn register_team(&self, id: &str, name: &str, config_path: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO teams (id, name, config_path) VALUES (?1, ?2, ?3)",
            params![id, name, config_path],
        )?;
        Ok(())
    }

    pub fn list_teams_db(&self) -> Result<Vec<TeamRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, config_path, created_at FROM teams ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(TeamRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    config_path: r.get(2)?,
                    created_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Task CRUD =====

    pub fn create_task(&self, task: &NewTask) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        // `type` defaults to "issue" when the caller passes None or an empty string.
        // The DB CHECK constraint enforces the same allowlist as VALID_TASK_TYPES.
        let task_type = task
            .r#type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(TASK_TYPE_ISSUE);
        self.conn.execute(
            "INSERT INTO tasks (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                input_context, created_by_session, created_trace_id,
                reference_url, source, metadata, type, dispatch_class
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17, ?18,
                ?19, ?20, ?21, ?22, ?23
             )",
            params![
                id,
                task.agent_id,
                task.team_run_id,
                task.parent_task_id,
                task.depth,
                task.label,
                task.trigger_type,
                task.cron_expr,
                task.event_source,
                task.event_offset_secs,
                task.condition_expr,
                task.next_fire_at,
                task.timeout_at,
                task.action_type,
                task.action_config,
                task.input_context,
                task.created_by_session,
                task.created_trace_id,
                task.reference_url,
                task.source,
                task.metadata,
                task_type,
                task.dispatch_class,
            ],
        )?;
        Ok(id)
    }

    /// Insert a recurring task only if no task with the same (agent_id, label) and
    /// trigger_type='recurring' exists. Returns the task ID if created, None if
    /// already existed or if a recent dead sibling refuses re-registration.
    ///
    /// **mika#1742 Problem B — refuse-to-zombie guard.** The partial unique index
    /// `idx_tasks_unique_recurring` explicitly excludes `'cancelled' | 'failed' |
    /// 'expired' | 'delivered'` statuses. So a bare `INSERT OR IGNORE` silently
    /// creates a fresh row whenever every prior instance died — and every
    /// mika-spirit restart re-triggers the fresh registration. That's the
    /// curator_review zombie root cause (8 dead Mika rows across 8 days per
    /// root-claude's forensic).
    ///
    /// This guard adds a pre-insert query: if any recurring row for the same
    /// `(agent_id, label)` was `failed`/`cancelled` within the last
    /// [`RECURRING_ZOMBIE_GRACE_HOURS`] window, refuse to re-register and log a
    /// `warn!` so the operator sees the surface. The operator can:
    ///
    /// - Wait for the grace window to elapse (fresh registration then proceeds).
    /// - Investigate the previous instance's failure via `mika tasks get <id>`.
    /// - Manually clear the dead row and let the next startup re-register.
    ///
    /// Non-goal here: fixing the *underlying* dispatch failure for Mika's
    /// specific `curator_review` (Problem A in the ticket). Root-claude's
    /// diagnosis notes PR#1726 (RouteFuture/dashmap wedge) likely already
    /// resolves it. Verification is a Phase-2 follow-up under the Problem-A
    /// investigation.
    pub fn create_recurring_task_if_absent(&self, task: NewTask) -> Result<Option<String>> {
        // Zombie guard — refuse to re-register a recurring label whose most
        // recent instance died in the grace window.
        let dead_sibling: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT id, status, updated_at FROM tasks
                 WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
                   AND trigger_type = 'recurring'
                   AND status IN ('failed', 'cancelled', 'expired')
                   AND updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                 ORDER BY updated_at DESC LIMIT 1",
                params![task.agent_id, task.label, RECURRING_ZOMBIE_GRACE_SQL],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((prev_id, prev_status, prev_updated)) = dead_sibling {
            tracing::warn!(
                agent_id = %task.agent_id,
                label = %task.label,
                previous_task_id = %prev_id,
                previous_status = %prev_status,
                previous_updated_at = %prev_updated,
                grace_hours = RECURRING_ZOMBIE_GRACE_HOURS,
                "mika#1742: refusing to re-register recurring task — recent same-label \
                 instance ended in a terminal-failure state. Investigate root cause \
                 (`mika tasks get {prev_id}`) before re-enabling; re-registration \
                 automatically re-attempts after the grace window elapses."
            );
            return Ok(None);
        }

        let id = uuid::Uuid::new_v4().to_string();
        let task_type = task
            .r#type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(TASK_TYPE_ISSUE);
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO tasks
             (id, agent_id, team_run_id, parent_task_id, depth, label,
              trigger_type, cron_expr, event_source, event_offset_secs,
              condition_expr, next_fire_at, timeout_at, action_type,
              action_config, status, input_context, created_by_session, created_trace_id,
              reference_url, source, metadata, type)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'recurring_active',?16,?17,?18,?19,?20,?21,?22)",
            params![
                id, task.agent_id, task.team_run_id, task.parent_task_id,
                task.depth, task.label, task.trigger_type, task.cron_expr,
                task.event_source, task.event_offset_secs, task.condition_expr,
                task.next_fire_at, task.timeout_at, task.action_type,
                task.action_config, task.input_context, task.created_by_session,
                task.created_trace_id, task.reference_url, task.source,
                task.metadata, task_type
            ],
        )?;
        if n > 0 {
            Ok(Some(id))
        } else {
            Ok(None) // already existed (unique-index conflict on an active row)
        }
    }

    /// Get the cron expression for an existing recurring task by label.
    pub fn get_recurring_task_cron(&self, agent_id: &str, label: &str) -> Result<Option<String>> {
        let cron: Option<Option<String>> = self.conn.query_row(
            "SELECT cron_expr FROM tasks WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring' AND status IN ('recurring_active', 'pending', 'in_progress') LIMIT 1",
            params![agent_id, label],
            |r| r.get(0),
        ).optional()?;
        Ok(cron.flatten())
    }

    /// Update the cron expression and next_fire_at for an existing recurring task.
    pub fn update_recurring_task_cron(
        &self,
        agent_id: &str,
        label: &str,
        new_cron: &str,
        next_fire_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET cron_expr = ?1, next_fire_at = ?2
             WHERE agent_id = ?3 AND label = ?4 AND trigger_type = 'recurring'
               AND status IN ('recurring_active', 'pending', 'in_progress')",
            params![new_cron, next_fire_at, agent_id, label],
        )?;
        Ok(())
    }

    /// Cancel a recurring task by label (e.g. when reflection is disabled in identity.toml).
    pub fn cancel_recurring_task_by_label(&self, agent_id: &str, label: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring'
               AND status NOT IN ('completed','failed','cancelled','expired')",
            params![agent_id, label],
        )?;
        Ok(())
    }

    /// Cancel active recurring tasks for agents no longer on disk (mika#1436).
    ///
    /// Called once at startup after the filesystem walk populates the agent set.
    /// Cancels (not deletes) so the audit trail via audit_events is preserved.
    /// Returns the list of (task_id, agent_id) pairs for operator observability.
    ///
    /// Agent-unscoped: operates across all agents in the DB, not filtered by
    /// this `Database` instance's implicit agent context.
    pub fn cancel_orphan_recurring_tasks(
        &self,
        known_agent_ids: &[String],
    ) -> Result<Vec<(String, String)>> {
        if known_agent_ids.is_empty() {
            return Ok(vec![]);
        }

        let tx = self.conn.unchecked_transaction()?;

        // Build a parameterized placeholder list for the NOT IN clause.
        let placeholders: Vec<String> = (1..=known_agent_ids.len())
            .map(|i| format!("?{}", i))
            .collect();
        let placeholders_str = placeholders.join(", ");

        // SELECT orphan tasks first for logging.
        let select_sql = format!(
            "SELECT id, agent_id FROM tasks
             WHERE trigger_type = 'recurring'
               AND status IN ('pending', 'recurring_active', 'in_progress')
               AND agent_id NOT IN ({placeholders_str})"
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = known_agent_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let orphans: Vec<(String, String)> = {
            let mut stmt = tx.prepare(&select_sql)?;
            stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        if orphans.is_empty() {
            tx.commit()?;
            return Ok(vec![]);
        }

        // UPDATE to cancelled.
        let update_sql = format!(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE trigger_type = 'recurring'
               AND status IN ('pending', 'recurring_active', 'in_progress')
               AND agent_id NOT IN ({placeholders_str})"
        );
        tx.execute(&update_sql, params.as_slice())?;

        tx.commit()?;
        Ok(orphans)
    }

    fn row_to_task(r: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
        Ok(Task {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            team_run_id: r.get(2)?,
            parent_task_id: r.get(3)?,
            depth: r.get(4)?,
            label: r.get(5)?,
            trigger_type: r.get(6)?,
            cron_expr: r.get(7)?,
            event_source: r.get(8)?,
            event_offset_secs: r.get(9)?,
            condition_expr: r.get(10)?,
            next_fire_at: r.get(11)?,
            timeout_at: r.get(12)?,
            action_type: r.get(13)?,
            action_config: r.get(14)?,
            status: r.get(15)?,
            process_id: r.get(16)?,
            input_context: r.get(17)?,
            result: r.get(18)?,
            created_by_session: r.get(19)?,
            created_trace_id: r.get(20)?,
            execution_trace_id: r.get(21)?,
            created_at: r.get(22)?,
            updated_at: r.get(23)?,
            fired_at: r.get(24)?,
            completed_at: r.get(25)?,
            reference_url: r.get(26)?,
            source: r.get(27)?,
            metadata: r.get(28)?,
            r#type: r.get(29)?,
            dispatch_class: r.get(30)?,
        })
    }

    const TASK_COLUMNS: &'static str = "id, agent_id, team_run_id, parent_task_id, depth, label,
         trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
         next_fire_at, timeout_at, action_type, action_config,
         status, process_id, input_context, result, created_by_session,
         created_trace_id, execution_trace_id, created_at, updated_at, fired_at, completed_at,
         reference_url, source, metadata, type, dispatch_class";

    pub fn get_task(&self, id: &str, agent_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND agent_id = ?2",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![id, agent_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Resolve a task ID prefix to matching full task IDs, scoped to the given agent.
    /// Returns up to 10 matching IDs for ambiguity reporting.
    pub fn resolve_task_id_by_prefix(&self, prefix: &str, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks WHERE id LIKE ?1 || '%' AND agent_id = ?2 ORDER BY id LIMIT 10",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![prefix, agent_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Get a manual (task) task by ID, scoped to the given agent.
    pub fn get_manual_task(&self, id: &str, agent_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![id, agent_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Walk the `parent_task_id` chain to the nearest scope root — a task with
    /// `type IN ('issue', 'milestone', 'project')`. Returns `None` if no scope
    /// ancestor exists, the starting task is not found, or the chain exceeds the
    /// depth limit (mika#974).
    ///
    /// The depth limit of 20 gives 6× headroom above the deployed maximum of
    /// N=3 (project → milestone → issue) while bounding worst-case walk cost.
    pub fn resolve_scope_root_task_id(&self, task_id: &str) -> Result<Option<String>> {
        /// Maximum parent-chain hops before giving up. Deployed task hierarchies
        /// never exceed N=3 today (project → milestone → issue). 20 gives 6×
        /// headroom against pathological chains.
        const SCOPE_ROOT_WALK_DEPTH_LIMIT: usize = 20;
        const SCOPE_TYPES: &[&str] = &[TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT];

        let mut current_id = task_id.to_owned();

        for _ in 0..SCOPE_ROOT_WALK_DEPTH_LIMIT {
            let row: Option<(String, String, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT type, trigger_type, parent_task_id FROM tasks WHERE id = ?1",
                    params![current_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;

            match row {
                // A manual task with a scope type is the scope root.
                // Callback/recurring tasks are not scope roots even if typed as 'issue'.
                Some((task_type, trigger_type, _))
                    if SCOPE_TYPES.contains(&task_type.as_str()) && trigger_type == "manual" =>
                {
                    return Ok(Some(current_id));
                }
                Some((_, _, Some(parent_id))) => {
                    current_id = parent_id;
                }
                // Task not a scope root and no parent — chain exhausted.
                Some((_, _, None)) | None => return Ok(None),
            }
        }

        // Depth limit exceeded — likely a circular chain.
        warn!(
            task_id = task_id,
            limit = SCOPE_ROOT_WALK_DEPTH_LIMIT,
            "scope_root_walk_depth_limit_exceeded"
        );
        Ok(None)
    }

    /// Get IDs of pending callback tasks for a given session.
    ///
    /// Used by `mika ask` to detect background tasks that were spawned during the
    /// agent loop but won't be consumed until TUI or server starts. See #265.
    pub fn get_pending_callbacks_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks
             WHERE created_by_session = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
             ORDER BY created_at ASC",
        )?;
        let ids = stmt
            .query_map(params![session_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Count child tasks for a given parent task (manual tasks only).
    pub fn count_child_tasks(
        &self,
        parent_task_id: &str,
        agent_id: &str,
    ) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT status, COUNT(*) FROM tasks
             WHERE parent_task_id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'
             GROUP BY status",
        )?;
        let rows = stmt.query_map(params![parent_task_id, agent_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_schedulable_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
               AND trigger_type NOT IN ('callback', 'manual')
             ORDER BY next_fire_at ASC NULLS LAST",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// Record the trace_id of the execution that ran this task.
    /// Deliberately does NOT scope by agent_id — the dispatcher may write
    /// execution_trace_id for tasks owned by different agents (cross-agent team tasks).
    pub fn update_task_execution_trace_id(&self, id: &str, trace_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET execution_trace_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![trace_id, id],
        )?;
        Ok(())
    }

    /// Atomically claim a task and record its fired_at in a single UPDATE.
    /// Returns true if the task was claimed (was in 'pending' or 'recurring_active' state).
    /// Returns false if the task was already claimed, cancelled, or completed.
    pub fn claim_and_fire_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'in_progress',
                              fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                              updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2 AND status IN ('pending', 'recurring_active')",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    pub fn update_task_completed(
        &self,
        id: &str,
        agent_id: &str,
        result: Option<&str>,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'completed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3 AND status IN ('pending', 'in_progress')",
            params![result, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_failed(&self, id: &str, agent_id: &str, error: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'failed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
             AND status NOT IN ('completed', 'failed', 'cancelled', 'expired', 'delivered')",
            params![error, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Update the dispatch class of a task (#1001).
    ///
    /// Used when a task transitions between grooming and implementation phases
    /// (e.g., after dev-groom completes and dev-pilot is about to dispatch on
    /// the same task_id per mika#996's task-reuse pattern). Idempotent —
    /// setting the same class is a no-op (updated_at still advances).
    pub fn update_task_dispatch_class(
        &self,
        id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET dispatch_class = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3",
            params![dispatch_class, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Write a dispatch-rejection reason to `tasks.result` without changing status (#1108).
    ///
    /// Used by `validate_dispatch_readiness()` to surface rejection reasons to
    /// operator-visible surfaces (`tasks.result` column). The task's status is
    /// preserved — only `result` and `updated_at` are modified. Returns `true`
    /// if the row was updated. Agent-unscoped because the caller may not know
    /// the agent_id (e.g., the unauthorized-webhook check fires before task fetch).
    pub fn write_task_dispatch_rejection(&self, id: &str, reason_json: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET result = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND trigger_type = 'manual'",
            params![reason_json, id],
        )?;
        Ok(rows > 0)
    }

    /// Promote a task from `failed` → `completed` (#958).
    ///
    /// Symmetric to `update_task_failed()`. Only transitions tasks currently
    /// in `failed` status — guarded WHERE clause prevents promotion from any
    /// other state. Returns `true` if the transition happened.
    pub fn promote_task_completed(&self, id: &str, agent_id: &str, reason: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'completed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
             AND status = 'failed'",
            params![reason, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_next_fire_at(&self, id: &str, next_fire_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    /// Atomically reschedule a recurring task: set next_fire_at and status = 'recurring_active'
    /// in a single UPDATE, replacing two sequential writes.
    pub fn update_task_rescheduled(&self, id: &str, next_fire_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, status = 'recurring_active', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    pub fn cancel_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2 AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![id, agent_id],
        )?;
        if n > 0 {
            // Cascade: cancel active callback children (mika#1011 Phase 0.7).
            // Prevents orphan-pending deferred-dispatch callbacks (and closes a
            // latent gap for immediate callbacks too). Non-callback children
            // (e.g., manual sub-tasks) are intentionally left untouched.
            self.conn.execute(
                "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE parent_task_id = ?1 AND agent_id = ?2
                   AND trigger_type = 'callback'
                   AND status IN ('pending', 'in_progress')",
                params![id, agent_id],
            )?;
        }
        Ok(n > 0)
    }

    /// Update the status of a manual (task) task. Free transitions allowed.
    /// Sets `completed_at` when transitioning to `completed`.
    /// Returns the old status for audit logging.
    pub fn update_manual_task_status(
        &self,
        task_id: &str,
        agent_id: &str,
        new_status: &str,
    ) -> Result<Option<String>> {
        let old_status: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()?;

        let Some(old) = &old_status else {
            return Ok(None);
        };

        if old == new_status {
            return Ok(Some(old.clone()));
        }

        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                    completed_at = CASE WHEN ?1 = 'completed' THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') ELSE NULL END
             WHERE id = ?2 AND agent_id = ?3 AND trigger_type = 'manual'",
            params![new_status, task_id, agent_id],
        )?;

        Ok(old_status)
    }

    /// Number of columns in TASK_COLUMNS (used for child_count ordinal in list_manual_tasks).
    const TASK_COLUMN_COUNT: usize = 31;

    /// List manual (task) tasks for an agent with optional filters.
    /// Uses parameterized NULL checks to avoid dynamic SQL construction.
    pub fn list_manual_tasks(
        &self,
        agent_id: &str,
        status_filter: Option<&str>,
        source_filter: Option<&str>,
        include_children: bool,
    ) -> Result<Vec<(Task, Option<i64>)>> {
        let child_expr = if include_children {
            "(SELECT COUNT(*) FROM tasks c WHERE c.parent_task_id = t.id)"
        } else {
            "NULL"
        };

        let sql = format!(
            "SELECT {columns}, {child_expr} AS child_count
             FROM tasks t
             WHERE t.agent_id = ?1 AND t.trigger_type = 'manual'
               AND (?2 IS NULL OR t.status = ?2)
               AND (?3 IS NULL OR t.source = ?3)
             ORDER BY t.created_at DESC LIMIT 50",
            columns = Self::TASK_COLUMNS
                .split(", ")
                .map(|c| format!("t.{c}"))
                .collect::<Vec<_>>()
                .join(", "),
        );

        let status_param: Option<String> = status_filter.map(|s| s.to_string());
        let source_param: Option<String> = source_filter.map(|s| s.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, status_param, source_param], |r| {
                let task = Self::row_to_task(r)?;
                // child_count is at ordinal Self::TASK_COLUMN_COUNT (one past last task column)
                let child_count: Option<i64> = r.get(Self::TASK_COLUMN_COUNT)?;
                Ok((task, child_count))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count audit_events matching (agent_id, tool_name, target_key) with `created_at > since`.
    ///
    /// Used by the verdict-handler's PR-keyed circuit breaker (mika#1563):
    /// counts prior `verdict_observed` events for a given PR URL in a sliding
    /// window. The check runs BEFORE task lookup, so it fires even when the
    /// task is missing or no longer in_progress — which is the convergence-loop
    /// failure mode that #1556 hit.
    ///
    /// `since` must be an ISO 8601 UTC timestamp (`%Y-%m-%dT%H:%M:%SZ`). String
    /// comparison is correct because the column format is fixed-width UTC.
    pub fn count_recent_audit_events_for_target(
        &self,
        agent_id: &str,
        tool_name: &str,
        target_key: &str,
        since: &str,
    ) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE agent_id = ?1 AND tool_name = ?2 AND target_key = ?3
               AND created_at > ?4",
            params![agent_id, tool_name, target_key, since],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Count **active** agent-created tasks in a session (for per-session cap enforcement).
    /// Only pending/in_progress/blocked items count — completed/cancelled/failed/delivered
    /// items are terminal and should not block new task creation (sprint mode).
    /// Scoped to agent_id for defense-in-depth.
    pub fn count_session_tasks(&self, agent_id: &str, session_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1 AND created_by_session = ?2 AND trigger_type = 'manual'
               AND (source IS NULL OR source != 'user_request')
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')",
            params![agent_id, session_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Find an active manual task by agent_id and reference_url.
    /// Used for dedup when `create_task` is called with a reference_url that
    /// already has an active (non-terminal) task.
    pub fn find_active_task_by_ref_url(
        &self,
        agent_id: &str,
        reference_url: &str,
    ) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND reference_url = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, reference_url], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and PR URL stored in metadata.
    /// Looks up `json_extract(metadata, '$.claude_pilot.pr_url')` for matching.
    /// Used to locate the parent task when a PR review verdict arrives.
    pub fn find_active_task_by_pr_url(&self, agent_id: &str, pr_url: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND json_extract(metadata, '$.claude_pilot.pr_url') = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, pr_url], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and branch stored in metadata.
    /// Looks up `json_extract(metadata, '$.claude_pilot.branch')` for matching.
    /// Used to locate the parent task when a PR webhook arrives before
    /// the PR URL has been recorded (in-flight tasks only have a branch).
    pub fn find_active_task_by_branch(&self, agent_id: &str, branch: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND json_extract(metadata, '$.claude_pilot.branch') = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, branch], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and label (case-insensitive).
    /// Used as a fallback dedup path when `create_task` is called without a reference_url.
    pub fn find_active_task_by_label(&self, agent_id: &str, label: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, label], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Get the depth of a task by ID (for computing child depth).
    /// Scoped to agent_id to prevent cross-agent parent linking.
    pub fn get_task_depth(&self, task_id: &str, agent_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT depth FROM tasks WHERE id = ?1 AND agent_id = ?2",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// List pending/in_progress/blocked manual tasks for prompt injection (heartbeat awareness).
    pub fn list_active_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND trigger_type = 'manual'
               AND status IN ('pending', 'in_progress', 'blocked')
             ORDER BY created_at DESC LIMIT 10",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build a task health summary for heartbeat prompt injection.
    ///
    /// Returns active manual tasks plus anomalous task states across all trigger types.
    /// Anomalies are capped at [`crate::task_engine::types::health_thresholds::MAX_ANOMALIES`].
    pub fn get_task_health_summary(&self, agent_id: &str) -> Result<TaskHealthSummary> {
        use crate::task_engine::types::health_thresholds;

        let active_tasks = self.list_active_tasks(agent_id)?;
        let now = Utc::now();
        let mut anomalies: Vec<TaskHealthAnomaly> = Vec::new();

        // Helper: query anomaly rows and return them as TaskHealthAnomaly values.
        // `describe_age` receives the 5th SELECT column (a timestamp or ignored)
        // and returns the human-readable age_description for each anomaly.
        let query_anomalies = |sql: &str,
                               sql_params: &[&dyn rusqlite::types::ToSql],
                               anomaly_type: &str,
                               describe_age: &dyn Fn(&str) -> String|
         -> Result<Vec<TaskHealthAnomaly>> {
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(sql_params, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?;
            let mut result = Vec::new();
            for row in rows {
                let (id, label, trigger_type, status, ts_col, reference_url) = row?;
                result.push(TaskHealthAnomaly {
                    task_id: id,
                    label,
                    trigger_type,
                    status,
                    anomaly_type: anomaly_type.to_string(),
                    age_description: describe_age(&ts_col),
                    reference_url,
                });
            }
            Ok(result)
        };

        // 1. Stuck callbacks: completed but not delivered for > threshold
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STUCK_CALLBACK_SECS)),
            );
            let limit = health_thresholds::MAX_ANOMALIES as i64;
            anomalies.extend(query_anomalies(
                "SELECT id, label, trigger_type, status, updated_at, reference_url
                 FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'completed'
                   AND updated_at < ?2
                 ORDER BY updated_at ASC
                 LIMIT ?3",
                &[&agent_id, &threshold as &dyn rusqlite::types::ToSql, &limit],
                "stuck_callback",
                &|ts| format!("stuck {}", format_age(ts, now)),
            )?);
        }

        // 2. Failed recurring tasks
        {
            let since = timestamp::format(&(now - Duration::seconds(86_400)));
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, updated_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'recurring'
                       AND status = 'failed'
                       AND updated_at > ?2
                     ORDER BY updated_at DESC
                     LIMIT ?3",
                    &[&agent_id, &since as &dyn rusqlite::types::ToSql, &remaining],
                    "failed_recurring",
                    &|ts| format!("failed {} ago", format_age(ts, now)),
                )?);
            }
        }

        // 3. Long-running in_progress tasks
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, fired_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND status = 'in_progress'
                       AND trigger_type != 'manual'
                       AND fired_at IS NOT NULL
                       AND fired_at < ?2
                     ORDER BY fired_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "long_running",
                    &|ts| format!("running for {}", format_age(ts, now)),
                )?);
            }
        }

        // 4. Stale blocked manual tasks
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STALE_BLOCKED_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, updated_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'manual'
                       AND status = 'blocked'
                       AND updated_at < ?2
                     ORDER BY updated_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "stale_blocked",
                    &|ts| format!("blocked for {}", format_age(ts, now)),
                )?);
            }
        }

        // 5. Stale pending manual tasks with no callback child (#583)
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STALE_PENDING_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT t.id, t.label, t.trigger_type, t.status, t.created_at, t.reference_url
                     FROM tasks t
                     WHERE t.agent_id = ?1
                       AND t.trigger_type = 'manual'
                       AND t.status = 'pending'
                       AND t.created_at < ?2
                       AND NOT EXISTS (
                           SELECT 1 FROM tasks c
                           WHERE c.parent_task_id = t.id
                             AND c.trigger_type = 'callback'
                       )
                     ORDER BY t.created_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "stale_pending",
                    &|ts| format!("pending for {}", format_age(ts, now)),
                )?);
            }
        }

        // 6. GitHub-linked manual tasks (active, with reference_url containing github.com)
        {
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, created_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'manual'
                       AND status IN ('pending', 'in_progress')
                       AND reference_url LIKE '%github.com%'
                     ORDER BY created_at DESC
                     LIMIT ?2",
                    &[&agent_id, &remaining as &dyn rusqlite::types::ToSql],
                    "github_linked",
                    &|_| "has linked GitHub PR".to_string(),
                )?);
            }
        }

        // 7. Dispatch failures: dual-signal wedge detection (#980)
        //    Signal A: >= THRESHOLD recent run_claude_pilot failures in the sliding window
        //    Signal B: stale dispatch — no run_claude_pilot attempt in > 1h while task is in_progress
        {
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len());
            if remaining > 0 {
                let window_start = timestamp::format(
                    &(now - Duration::seconds(health_thresholds::DISPATCH_FAILURE_WINDOW_SECS)),
                );
                let stale_threshold = timestamp::format(
                    &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
                );

                // Signal A: Count recent failures with session→task JOIN for correlation
                let signal_a: Option<(u32, Option<String>, Option<String>)> = self
                    .conn
                    .prepare(
                        "SELECT
                            COUNT(*) as failure_count,
                            t.id as task_id,
                            t.label as task_label
                         FROM tool_calls tc
                         LEFT JOIN sessions s ON tc.session_id = s.id
                         LEFT JOIN tasks t ON s.task_id = t.id AND t.status = 'in_progress'
                         WHERE tc.agent_id = ?1
                           AND tc.tool_name = 'run_claude_pilot'
                           AND tc.success = 0
                           AND tc.created_at >= ?2
                         GROUP BY t.id
                         ORDER BY failure_count DESC
                         LIMIT 1",
                    )?
                    .query_row(params![agent_id, &window_start], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .ok();

                // Signal B: Stale dispatch — most recent run_claude_pilot attempt is older than 1h
                // while an in_progress manual task exists
                let signal_b: Option<(String, String)> = self
                    .conn
                    .prepare(
                        "SELECT t.id, t.label
                         FROM tasks t
                         WHERE t.agent_id = ?1
                           AND t.status = 'in_progress'
                           AND t.trigger_type = 'manual'
                           AND NOT EXISTS (
                               SELECT 1 FROM tool_calls tc2
                               WHERE tc2.agent_id = ?1
                                 AND tc2.tool_name = 'run_claude_pilot'
                                 AND tc2.created_at >= ?2
                           )
                         ORDER BY t.updated_at DESC
                         LIMIT 1",
                    )?
                    .query_row(params![agent_id, &stale_threshold], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .ok();

                let mut dispatch_anomaly_fired = false;

                // Emit anomaly from Signal A (threshold met)
                if let Some((count, task_id, task_label)) = signal_a
                    && count >= health_thresholds::DISPATCH_FAILURE_THRESHOLD
                {
                    let (tid, tlabel) = match (task_id, task_label) {
                        (Some(id), Some(label)) => (id, label),
                        _ => (
                            agent_id.to_string(),
                            "run_claude_pilot dispatch".to_string(),
                        ),
                    };
                    anomalies.push(TaskHealthAnomaly {
                        task_id: tid,
                        label: tlabel,
                        trigger_type: "manual".to_string(),
                        status: "in_progress".to_string(),
                        anomaly_type: "dispatch_failures".to_string(),
                        age_description: format!("{} failures in last 2h", count),
                        reference_url: None,
                    });
                    dispatch_anomaly_fired = true;
                }

                // Emit anomaly from Signal B (stale dispatch) — only if Signal A didn't fire
                if !dispatch_anomaly_fired && let Some((task_id, task_label)) = signal_b {
                    anomalies.push(TaskHealthAnomaly {
                        task_id,
                        label: task_label,
                        trigger_type: "manual".to_string(),
                        status: "in_progress".to_string(),
                        anomaly_type: "dispatch_stale".to_string(),
                        age_description: "no dispatch attempt in >1h".to_string(),
                        reference_url: None,
                    });
                }
            }
        }

        // Cap total anomalies
        anomalies.truncate(health_thresholds::MAX_ANOMALIES);

        Ok(TaskHealthSummary {
            active_tasks,
            anomalies,
        })
    }

    pub fn mark_tasks_expired(&self, now: &str, agent_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'expired', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?2
               AND timeout_at IS NOT NULL AND timeout_at < ?1
               AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![now, agent_id],
        )?;
        Ok(n)
    }

    /// Get IDs of expired tasks whose parent is still pending (for sibling completion checks).
    pub fn get_expired_child_task_ids(&self, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tasks t
             JOIN tasks p ON t.parent_task_id = p.id
             WHERE t.agent_id = ?1
               AND t.status = 'expired'
               AND t.parent_task_id IS NOT NULL
               AND p.status = 'pending'",
        )?;
        let ids = stmt
            .query_map(params![agent_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    pub fn count_pending_tasks(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','in_progress','recurring_active')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Count active self_dev tasks for mika-dev (mika#1363 F2).
    ///
    /// Returns the number of tasks that indicate mika-dev is not idle:
    /// - status='in_progress' (currently running) or status='pending' (awaiting dispatch)
    /// - source='self_dev' (excludes system tasks: heartbeat, reflection, recurring auto_pull)
    /// - Excludes 'completed', 'failed', 'blocked', 'cancelled' (terminal states)
    pub fn count_active_self_dev_tasks(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND source = 'self_dev'
               AND status IN ('in_progress', 'pending')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// True if an active (pending/in_progress) self_dev task references this issue
    /// (mika#1824 D6). Used by the Phase 2 stuck-ready reconciler to skip tickets
    /// that already have in-flight work of their own.
    ///
    /// `issue_url` is the canonical issue URL (e.g.
    /// `https://github.com/senara-solutions/mika/issues/123`). The match is a
    /// prefix `LIKE` so the `?phase=groom` suffix variant is covered.
    pub fn has_active_self_dev_task_for_issue(
        &self,
        agent_id: &str,
        issue_url: &str,
    ) -> Result<bool> {
        let prefix = format!("{}%", issue_url);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND source = 'self_dev'
               AND status IN ('pending', 'in_progress')
               AND reference_url LIKE ?2",
            params![agent_id, prefix],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get all pending user-visible tasks (reminders and callbacks, excludes heartbeat/reflection).
    /// Returns user-visible reminder tasks (both `send_message` and `resume_agent`).
    ///
    /// Intentionally excludes `trigger_type = 'callback'` tasks: those are system-internal
    /// tasks created by long-running exec handlers, not user-created reminders. Callback
    /// delivery is handled separately by `get_undelivered_callback_tasks()` (server mode)
    /// and `poll_callback_tasks()` (CLI mode). See #363.
    pub fn get_user_visible_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND action_type IN ('send_message', 'resume_agent')
               AND trigger_type NOT IN ('callback')
               AND status IN ('pending', 'in_progress', 'recurring_active')
             ORDER BY next_fire_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Split counts of active background tasks: executing (subprocess alive) vs queued (waiting).
    /// Uses `process_id IS NOT NULL` as the discriminator for executing tasks.
    /// Used by TUI footer badge to show `[1 running, 2 queued]` instead of `[3 running]`.
    pub fn get_background_task_counts(&self, agent_id: &str) -> Result<BackgroundTaskCounts> {
        let (executing, queued): (i64, i64) = self.conn.query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN process_id IS NOT NULL THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN process_id IS NULL THEN 1 ELSE 0 END), 0)
             FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')",
            params![agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(BackgroundTaskCounts {
            executing: executing as usize,
            queued: queued as usize,
        })
    }

    /// Count active background tasks (long-running callback tasks that are pending or in-progress).
    /// Convenience wrapper that sums executing + queued counts.
    pub fn get_active_background_task_count(&self, agent_id: &str) -> Result<usize> {
        let counts = self.get_background_task_counts(agent_id)?;
        Ok(counts.executing + counts.queued)
    }

    pub fn get_inject_context_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND action_type = 'inject_context'
               AND status IN ('pending', 'in_progress')
             ORDER BY next_fire_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get callback tasks that completed or failed but have not yet been delivered to the user.
    /// Bounded by `since` (ISO 8601) to avoid processing stale callbacks.
    pub fn get_undelivered_callback_tasks(&self, agent_id: &str, since: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get callback tasks that completed or failed but have not yet been delivered,
    /// scoped to a specific session. Used by TUI to avoid cross-session leakage.
    pub fn get_undelivered_callback_tasks_for_session(
        &self,
        agent_id: &str,
        since: &str,
        session_id: &str,
    ) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
               AND created_by_session = ?3
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since, session_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Atomically mark a completed or failed callback task as delivered.
    /// Returns `false` if the task was already claimed (not in 'completed'/'failed' status).
    pub fn mark_task_delivered(&self, id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'delivered', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND status IN ('completed', 'failed')",
            params![id],
        )?;
        Ok(n > 0)
    }

    /// Find implement-class parent self_dev tasks left `in_progress` whose
    /// callback subtask delivered without producing a PR (#871).
    ///
    /// A parent is "orphaned" when:
    /// - `status = 'in_progress'`, `source = 'self_dev'`, `trigger_type = 'manual'`
    /// - Its latest callback subtask is `status = 'delivered'`
    /// - **The callback's** `dispatch_class = 'implement'` (or NULL — pre-v34 via COALESCE).
    ///   The class is keyed off the child (per-dispatch) rather than the parent
    ///   because reused parents (mika#920 pattern) carry stale class data from
    ///   their original dispatch.
    /// - The callback's `updated_at` is older than `grace_seconds` ago
    /// - Parent metadata does NOT contain `$.claude_pilot.pr_url`
    /// - No other active callback child exists (defers to #870's retry loop)
    ///
    /// Groom-class callbacks (mika#1001) are NOT reaped here — their expected
    /// artifact is a plan commit pushed to the branch, not a PR url. Groom-class
    /// leak detection is a separate follow-up (mika#1118 Option B).
    ///
    /// **Coupled pair:** `find_completable_parent_tasks_on_pr_url` is the
    /// success-side sibling (mika#1162). Any filter change here (agent_id,
    /// status, source, trigger_type, dispatch_class, sibling guard, grace
    /// window) MUST be applied symmetrically there. The two queries differ
    /// only on the `pr_url` predicate (`IS NULL` here vs `IS NOT NULL` there).
    pub fn find_orphaned_parent_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<OrphanedParentTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MIN(child.id) AS callback_task_id
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND COALESCE(child.dispatch_class, 'implement') = 'implement'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND child.status = 'delivered'
               AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND (parent.metadata IS NULL
                    OR json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NULL)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.id != child.id
                   AND sibling.status IN ('pending', 'in_progress')
               )
             GROUP BY parent.id
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(OrphanedParentTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    callback_task_id: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find parent self_dev tasks left `in_progress` after their callback
    /// subtask delivered WITH a `pr_url` (success indicator). Sibling to
    /// `find_orphaned_parent_tasks` — same JOIN shape, same guards, but
    /// inverted on the `pr_url` predicate. Used by the success-side engine
    /// backstop (mika#1162).
    ///
    /// A parent is `completable` when:
    /// - Parent is `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`
    /// - Its latest callback subtask is `status='delivered'`
    /// - **The callback's** `dispatch_class='implement'` (or NULL — pre-v34 via COALESCE)
    /// - The callback's `updated_at` is older than `grace_seconds` ago
    /// - Parent metadata HAS a non-empty `$.claude_pilot.pr_url`
    /// - No other active callback child exists (mirrors the reaper's guard)
    ///
    /// Groom-class callbacks (mika#1001) cannot trip this path because they
    /// never emit `PR:` lines — the `dispatch_class` filter is defense-in-depth.
    ///
    /// SOLE WRITER warning: this method is the only DB query that selects
    /// candidates for the `parent_completed_from_callback` audit transition.
    pub fn find_completable_parent_tasks_on_pr_url(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<CompletableParentTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MIN(child.id) AS callback_task_id,
                    json_extract(parent.metadata, '$.claude_pilot.pr_url') AS pr_url
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND COALESCE(child.dispatch_class, 'implement') = 'implement'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND child.status = 'delivered'
               AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NOT NULL
               AND json_extract(parent.metadata, '$.claude_pilot.pr_url') != ''
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.id != child.id
                   AND sibling.status IN ('pending', 'in_progress')
               )
             GROUP BY parent.id
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(CompletableParentTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    callback_task_id: row.get(3)?,
                    pr_url: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find parent self_dev **issue** tasks left `in_progress` with **zero**
    /// callback children, aged past `grace_seconds` (mika#1687).
    ///
    /// This is the zero-child complement of `find_orphaned_parent_tasks` and
    /// `find_completable_parent_tasks_on_pr_url`: both of those INNER-JOIN a
    /// delivered callback child, so a parent that reached `in_progress` without
    /// ever recording a callback child (silent pilot death — hypothesis 3 of
    /// mika#1687) produces zero rows in either and is never reaped. The
    /// `NOT EXISTS (SELECT 1 FROM tasks child …)` predicate here is the exact
    /// complement of their JOIN, so the three selection sets are disjoint by
    /// construction (they require a child; this requires none).
    ///
    /// Staleness keys on `parent.updated_at` (there is no delivered-child
    /// `updated_at` to key on). Scoped to `type='issue'` in v1 — milestone and
    /// project parents legitimately sit childless between child dispatches and
    /// carry their own advancement backstops (mika#991, #1218); their
    /// childless-stuck detection is a deferred follow-up.
    ///
    /// SOLE WRITER context: candidates selected here are transitioned to
    /// `failed` with the distinct `stuck_in_progress_no_callback_child` reason
    /// by `TaskEngine::reap_childless_stuck_parent_tasks`.
    pub fn find_childless_stuck_parent_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<ChildlessStuckParent>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at, parent.updated_at
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks child
                 WHERE child.parent_task_id = parent.id
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(ChildlessStuckParent {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Return ALL children of a parent task for the reaper's structured log event
    /// (`task_engine_reaper.evaluated`). Captures a point-in-time snapshot at kill
    /// time so post-incident diagnosis can see what the reaper saw (mika#1126).
    pub fn get_reaper_child_snapshot(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<ReaperChildSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, dispatch_class, status, trigger_type, action_type, updated_at, label
             FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![parent_task_id], |row| {
                Ok(ReaperChildSnapshot {
                    id: row.get(0)?,
                    dispatch_class: row.get(1)?,
                    status: row.get(2)?,
                    trigger_type: row.get(3)?,
                    action_type: row.get(4)?,
                    updated_at: row.get(5)?,
                    label: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn set_task_process_id(&self, id: &str, process_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET process_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![process_id, id],
        )?;
        Ok(())
    }

    /// Returns (task_id, process_id) pairs for expired tasks that still have a process_id set.
    pub fn get_expired_tasks_with_process_id(&self, agent_id: &str) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, process_id FROM tasks
             WHERE agent_id = ?1 AND status = 'expired' AND process_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Clear the process_id after killing an orphan process to prevent repeated kill attempts.
    pub fn clear_task_process_id(&self, id: &str, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET process_id = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2",
            params![id, agent_id],
        )?;
        Ok(())
    }

    /// Get active callback tasks that have a process_id set (#959).
    ///
    /// Returns callback tasks in `in_progress` status with a non-null process_id,
    /// used by the callback watchdog to detect dead subprocesses.
    pub fn get_active_callback_tasks_with_pid(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status = 'in_progress'
               AND process_id IS NOT NULL",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Set a single field in the task's metadata JSON (#959).
    ///
    /// Uses SQLite's `json_set()` to merge the field into existing metadata,
    /// initializing with `'{}'` if metadata is currently NULL.
    pub fn set_task_metadata_field(&self, task_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_set(COALESCE(metadata, '{}'), '$.' || ?1, ?2),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3",
            params![key, value, task_id],
        )?;
        Ok(())
    }

    /// Remove a single field from the task's metadata JSON (#959).
    ///
    /// Uses SQLite's `json_remove()` to delete the key. No-op if the key doesn't exist.
    pub fn remove_task_metadata_field(&self, task_id: &str, key: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_remove(COALESCE(metadata, '{}'), '$.' || ?1),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2",
            params![key, task_id],
        )?;
        Ok(())
    }

    /// Get a single metadata field from a task's metadata JSON as a string.
    pub fn get_task_metadata_field(&self, task_id: &str, key: &str) -> Result<Option<String>> {
        let result: Option<String> = self.conn.query_row(
            "SELECT json_extract(metadata, '$.' || ?1) FROM tasks WHERE id = ?2",
            params![key, task_id],
            |row| row.get(0),
        )?;
        Ok(result)
    }

    /// Check if all siblings of a completed task are done. If so, atomically
    /// claim the parent task for dispatch.
    ///
    /// Returns `Some(parent_id)` when the parent was claimed, `None` otherwise.
    /// Uses a single SQLite transaction to prevent races.
    pub fn try_complete_parent_on_sibling_done(&self, task_id: &str) -> Result<Option<String>> {
        let tx = self.conn.unchecked_transaction()?;

        // 1. Get parent_task_id for this task (no agent_id filter — parent-child
        //    relationships are structural, and team task trees have mixed agent_ids)
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_task_id FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let parent_id = match parent_id {
            Some(id) => id,
            None => return Ok(None),
        };

        // 2. Count incomplete siblings (same parent, not in terminal state).
        // No agent_id filter — siblings in a team task tree have different agent_ids.
        let incomplete: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1
             AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![&parent_id],
            |row| row.get(0),
        )?;

        if incomplete > 0 {
            tx.commit()?;
            return Ok(None);
        }

        // 3. Atomically claim parent task (only if still pending).
        // No agent_id filter — the parent may have a different agent_id than children.
        let changed = tx.execute(
            "UPDATE tasks SET status = 'in_progress', fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND status = 'pending'",
            params![&parent_id],
        )?;

        tx.commit()?;

        if changed > 0 {
            Ok(Some(parent_id))
        } else {
            Ok(None) // already claimed by another thread
        }
    }

    /// Get all child tasks for a given parent task.
    /// No agent_id filter — team task trees have children with different agent_ids.
    pub fn get_child_tasks(&self, parent_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY created_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![parent_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get all descendant tasks for a given root task (recursive).
    /// Returns all tasks in the subtree below root_task_id, excluding the root itself.
    /// No agent_id filter — team task trees have children with different agent_ids.
    /// Depth guard (depth <= 3) mirrors the CHECK constraint as defense-in-depth.
    pub fn get_task_descendants(&self, root_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "WITH RECURSIVE descendant_ids(id) AS (
                 SELECT id FROM tasks WHERE parent_task_id = ?1
                 UNION ALL
                 SELECT t.id FROM tasks t
                 JOIN descendant_ids d ON t.parent_task_id = d.id
                 WHERE t.depth <= 3
             )
             SELECT {cols} FROM tasks
             WHERE id IN (SELECT id FROM descendant_ids)
             ORDER BY created_at ASC",
            cols = Self::TASK_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![root_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Check if any active callback tasks exist for a task OTHER than the excluded one,
    /// filtered by dispatch class.
    ///
    /// Returns `Some((parent_task_id, callback_task_id))` if an active callback task exists
    /// whose parent differs from `excluded_parent_id` and whose dispatch class matches.
    /// Used by the per-class dispatch guard to enforce one-slot-per-class (#583, #1001).
    ///
    /// Pre-v34 rows with `dispatch_class IS NULL` are treated as `'implement'` via
    /// `COALESCE` — no application-layer NULL coercion needed (architect NF1).
    ///
    /// mika#1163: Excludes `:deferred` wrappers via `label NOT LIKE '%:deferred'`.
    /// Deferred wrappers are pending markers waiting for promotion, NOT active
    /// dispatches occupying a slot. Without this exclusion, two parents each
    /// holding a pending wrapper deadlock — every dispatch attempt from one
    /// wrapper sees the OTHER as slot-occupied and registers yet another
    /// wrapper. Mirrors the equivalent clause in `has_any_active_callback`
    /// (mika#1070), which the engine-level promotion backstop uses.
    /// Returns `(parent_task_id, callback_id, callback_label)` of the blocking
    /// callback, or `None` if no conflicting dispatch exists. The label enables
    /// callers to derive `blocker_kind` for rejection JSON (#1172 W3).
    pub fn has_active_callback_tasks_excluding(
        &self,
        excluded_parent_id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_task_id, id, label FROM tasks
             WHERE trigger_type = 'callback'
               AND status IN ('pending', 'in_progress')
               AND parent_task_id IS NOT NULL
               AND parent_task_id != ?1
               AND agent_id = ?2
               AND COALESCE(dispatch_class, 'implement') = ?3
               AND label NOT LIKE '%:deferred'
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![excluded_parent_id, agent_id, dispatch_class])?;
        if let Some(row) = rows.next()? {
            Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?)))
        } else {
            Ok(None)
        }
    }

    /// Check whether the given parent task has any active (pending/in_progress)
    /// non-deferred callback child. Used by the R9 no-op wrapper detection (#1172)
    /// to determine if a deferred wrapper completed without spawning a real dispatch.
    pub fn has_non_deferred_active_callback_child(&self, parent_task_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1
               AND trigger_type = 'callback'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'",
            params![parent_task_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Check whether a completed groom-class task exists for a given GitHub
    /// issue. Used by the dispatch-classification gate (#1620) to verify that
    /// grooming markers in an issue body were written by the autonomous
    /// `dev-groom` loop (which creates tasks with `?phase=groom` URL suffix)
    /// rather than pre-stamped by a manual `/mika-ask-arch` session.
    ///
    /// `issue_url` is the canonical issue URL without the `?phase=groom` suffix
    /// (e.g., `https://github.com/owner/repo/issues/123`). The method appends
    /// the suffix internally to match the autonomous groom flow's URL pattern.
    pub fn has_completed_groom_for_issue(&self, agent_id: &str, issue_url: &str) -> Result<bool> {
        let groom_url = format!("{}?phase=groom", issue_url);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND dispatch_class = 'groom'
               AND status IN ('completed', 'delivered')
               AND reference_url = ?2",
            params![agent_id, groom_url],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Count pending callback tasks for a given team run with depth > 1.
    /// Used to detect grandchild long-running tasks spawned during a team run.
    pub fn count_pending_callback_tasks_by_team_run(&self, team_run_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE team_run_id = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND depth > 1",
            params![team_run_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count pending deferred-dispatch callback tasks for this agent (mika#1011).
    /// Used by the executor flood-cap check before registering a new deferred callback.
    pub fn count_pending_deferred_callbacks(&self, agent_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND label = 'long_running:run_claude_pilot:deferred'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Promote the next pending deferred-dispatch callback for dispatch (FIFO).
    ///
    /// Sets `next_fire_at` to now and marks with a synthetic result so the task
    /// engine's periodic scan picks it up and routes through `dispatch_resume_agent`
    /// within one tick (~1 second). Returns `Some(task_id)` if a task was promoted,
    /// `None` if no pending deferred callback existed. Called by the dispatcher
    /// after a blocking callback completes (mika#1011).
    pub fn promote_next_deferred_callback(&self, agent_id: &str) -> Result<Option<String>> {
        // First, find the candidate task ID
        let candidate_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![agent_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(task_id) = candidate_id else {
            return Ok(None);
        };

        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?2,
                 next_fire_at = ?2,
                 updated_at = ?2
             WHERE id = ?1
               AND status = 'pending'",
            params![task_id, now],
        )?;
        Ok(if n > 0 { Some(task_id) } else { None })
    }

    /// Class-scoped sibling of `promote_next_deferred_callback`. Promotes the
    /// oldest pending deferred wrapper matching the given `dispatch_class`.
    /// Returns `Some(task_id)` if a task was promoted, `None` otherwise. Used by
    /// the periodic backstop's per-class iteration (mika#1175). Pre-v34 NULL
    /// rows treated as 'implement' via COALESCE (matches
    /// `has_active_callback_tasks_excluding` semantics).
    pub fn promote_next_deferred_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        // First, find the candidate task ID
        let candidate_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                   AND COALESCE(dispatch_class, 'implement') = ?2
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )
            .optional()?;

        let Some(task_id) = candidate_id else {
            return Ok(None);
        };

        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?2,
                 next_fire_at = ?2,
                 updated_at = ?2
             WHERE id = ?1
               AND status = 'pending'",
            params![task_id, now],
        )?;
        Ok(if n > 0 { Some(task_id) } else { None })
    }

    /// Returns true if any non-deferred callback task is in pending or in_progress
    /// status (i.e., a dispatch slot is occupied). Was used by the engine-level
    /// deferred-dispatch backstop (mika#1070). Post-mika#1175, the engine
    /// backstop calls `has_any_active_callback_for_class` per-class; this
    /// agent-wide form has no remaining production callers and is retained as
    /// a regression-test baseline + as a sibling reference for the class-scoped
    /// shape. See `has_any_active_callback_for_class` for production usage.
    pub fn has_any_active_callback(&self, agent_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Class-scoped sibling of `has_any_active_callback`. Returns `true` if any
    /// non-deferred callback task in the given `dispatch_class` is `pending` or
    /// `in_progress` (i.e., the per-class dispatch slot is occupied). Used by
    /// the periodic backstop's per-class slot check (mika#1175). Excludes
    /// `:deferred` wrappers (parity with mika#1163's symmetric exclusion).
    /// Pre-v34 NULL rows treated as 'implement' via COALESCE.
    pub fn has_any_active_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Force-promote the next pending deferred wrapper for a dispatch class,
    /// with fail-closed slot-availability semantics. Checks
    /// `has_any_active_callback_for_class()` first; if the slot is occupied,
    /// returns `RejectedSlotBusy` without mutating state. If free, promotes
    /// via `promote_next_deferred_callback_for_class()` and returns `Promoted`
    /// or `NoPendingWrapper`. Used by both the CLI verb and agent tool
    /// (mika#1453).
    ///
    /// Paired predicates: the slot check shares the same SQL predicate as
    /// `has_any_active_callback_for_class` — see mika#1163 for the
    /// asymmetric-predicate-drift failure class.
    pub fn force_promote_deferred_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<ForcePromoteResult> {
        // Slot-availability check (fail-closed) — same predicate as the
        // periodic backstop in engine.rs (mika#1163 parity).
        if self.has_any_active_callback_for_class(agent_id, dispatch_class)? {
            // Fetch the blocker's label for the rejection message.
            let blocking_label: String = self
                .conn
                .query_row(
                    "SELECT label FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'callback'
                       AND action_type = 'resume_agent'
                       AND status IN ('pending', 'in_progress')
                       AND label NOT LIKE '%:deferred'
                       AND COALESCE(dispatch_class, 'implement') = ?2
                     LIMIT 1",
                    params![agent_id, dispatch_class],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "<unknown>".to_string());

            return Ok(ForcePromoteResult::RejectedSlotBusy { blocking_label });
        }

        match self.promote_next_deferred_callback_for_class(agent_id, dispatch_class)? {
            Some(task_id) => Ok(ForcePromoteResult::Promoted { task_id }),
            None => Ok(ForcePromoteResult::NoPendingWrapper),
        }
    }

    /// Returns the task ID of the active non-deferred callback occupying the
    /// per-class dispatch slot. Same SQL predicate as
    /// `has_any_active_callback_for_class` but `SELECT id LIMIT 1` instead of
    /// `SELECT COUNT(*)`. Used by the CLI override path to identify the blocker
    /// before cancellation (mika#1453).
    ///
    /// Paired predicate: shares the `:deferred` exclusion and COALESCE
    /// semantics with `has_any_active_callback_for_class` — keep in sync
    /// (mika#1163).
    pub fn find_active_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND action_type = 'resume_agent'
                   AND status IN ('pending', 'in_progress')
                   AND label NOT LIKE '%:deferred'
                   AND COALESCE(dispatch_class, 'implement') = ?2
                 LIMIT 1",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    pub fn prune_completed_tasks(&self, older_than_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(older_than_secs));
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE status IN ('completed','cancelled','expired','failed','delivered')
             AND completed_at IS NOT NULL AND completed_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    pub fn get_tasks_by_status(&self, agent_id: &str, statuses: &[&str]) -> Result<Vec<Task>> {
        self.get_tasks_by_status_and_label(agent_id, statuses, None)
    }

    /// Like `get_tasks_by_status`, but with an optional `label_contains` substring filter.
    /// When `label_contains` is `Some`, only tasks whose label contains the substring
    /// (case-sensitive) are returned. The substring is parameterized (no SQL injection).
    pub fn get_tasks_by_status_and_label(
        &self,
        agent_id: &str,
        statuses: &[&str],
        label_contains: Option<&str>,
    ) -> Result<Vec<Task>> {
        if statuses.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: String = (1..=statuses.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let label_param_idx = statuses.len() + 2; // next param index after agent_id + statuses
        let label_clause = if label_contains.is_some() {
            format!(" AND label LIKE '%' || ?{label_param_idx} || '%'")
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT {} FROM tasks WHERE agent_id = ?1 AND status IN ({}){} ORDER BY created_at DESC",
            Self::TASK_COLUMNS,
            placeholders,
            label_clause,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id.to_string())];
        for s in statuses {
            bind.push(Box::new(s.to_string()));
        }
        if let Some(lc) = label_contains {
            bind.push(Box::new(lc.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Sessions =====

    pub fn create_session(&self, id: &str, agent_id: &str, channel_type: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, ?3)",
            params![id, agent_id, channel_type],
        )?;
        Ok(())
    }

    pub fn create_session_with_metadata(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type, metadata, task_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, agent_id, channel_type, metadata, task_id],
        )?;
        Ok(())
    }

    /// Create a session with metadata, parent session reference, and optional task linkage.
    /// Used by callback and skill_run dispatchers to link back to the originating session.
    pub fn create_session_with_parent(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
        parent_session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type, metadata, parent_session_id, task_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, agent_id, channel_type, metadata, parent_session_id, task_id],
        )?;
        Ok(())
    }

    /// Create a session with metadata if it doesn't already exist (INSERT OR IGNORE).
    /// Used by team engine for per-agent sessions that may already exist on resumed runs.
    pub fn create_session_if_not_exists(
        &self,
        id: &str,
        agent_id: &str,
        channel_type: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, channel_type, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id, agent_id, channel_type, metadata],
        )?;
        Ok(())
    }

    pub fn end_session(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn get_or_create_system_session(&self, agent_id: &str) -> Result<String> {
        let id = format!("system-{agent_id}");
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, 'system')",
            params![&id, agent_id],
        )?;
        Ok(id)
    }

    /// Prune ended system/silent sessions older than `retention_secs`.
    /// Targets heartbeat, callback, skill, reflection, team, and delegate sessions.
    /// Messages are cascade-deleted via FK ON DELETE CASCADE.
    pub fn prune_old_sessions(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ?1
             AND (id LIKE 'heartbeat-%' OR id LIKE 'callback-%' OR id LIKE 'skill-%' OR id LIKE 'reflection-%' OR id LIKE 'team-%' OR id LIKE 'delegate-%' OR id LIKE 'reminder-%')",
            params![cutoff],
        )?;
        Ok(n)
    }

    // ===== LLM Calls =====

    /// Maximum stored input/output size for tool calls (50KB, measured in bytes).
    const TOOL_CALL_MAX_BYTES: usize = 50_000;

    /// Truncate a string at a UTF-8 safe boundary, avoiding panics on multi-byte characters.
    fn truncate_utf8_safe(s: &str, max_bytes: usize) -> String {
        if s.len() <= max_bytes {
            return s.to_string();
        }
        // Walk backwards from max_bytes to find a valid char boundary
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}... (truncated at {} bytes)", &s[..end], max_bytes)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_llm_call(
        &self,
        id: &str,
        agent_id: &str,
        session_id: &str,
        trace_id: Option<&str>,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: Option<u64>,
        cache_write_tokens: Option<u64>,
        latency_ms: u64,
        stop_reason: Option<&str>,
        status: &str,
        error_message: Option<&str>,
        step: u32,
        prompt_variant: Option<&str>,
        response_text: Option<&str>,
        reasoning: Option<&str>,
        system_prompt_bytes: Option<i64>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO llm_calls (id, agent_id, session_id, trace_id, provider, model,
             input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
             latency_ms, stop_reason, status, error_message, step, prompt_variant,
             response_text, reasoning, system_prompt_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                id,
                agent_id,
                session_id,
                trace_id,
                provider,
                model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                latency_ms,
                stop_reason,
                status,
                error_message,
                step,
                prompt_variant,
                response_text,
                reasoning,
                system_prompt_bytes,
            ],
        )?;
        Ok(())
    }

    // ===== Tool Calls =====

    #[allow(clippy::too_many_arguments)]
    pub fn save_tool_call(
        &self,
        id: &str,
        agent_id: &str,
        session_id: &str,
        trace_id: Option<&str>,
        llm_call_id: Option<&str>,
        step: u32,
        tool_name: &str,
        tool_source: &str,
        skill_name: Option<&str>,
        input: Option<&str>,
        output: Option<&str>,
        success: bool,
        non_zero_exit: bool,
        latency_ms: u64,
        error_message: Option<&str>,
    ) -> Result<()> {
        // Scrub secret-shaped values before persistence (#908).
        // Order: scrub → truncate → INSERT. The LLM's in-memory result is NOT
        // scrubbed — only the durable copy in tool_calls is sanitized.
        // `scrub_secrets` returns Cow::Borrowed when no secrets found (zero alloc).
        use crate::secret_scrubber::scrub_secrets;
        let scrubbed_input = input.map(scrub_secrets);
        let scrubbed_output = output.map(scrub_secrets);
        let scrubbed_error = error_message.map(scrub_secrets);

        // Truncate large inputs/outputs to prevent DB bloat.
        // Uses char_indices for UTF-8 safe boundary (byte slicing panics on multi-byte chars).
        let truncated_input = scrubbed_input
            .as_deref()
            .map(|s| Self::truncate_utf8_safe(s, Self::TOOL_CALL_MAX_BYTES));
        let truncated_output = scrubbed_output
            .as_deref()
            .map(|s| Self::truncate_utf8_safe(s, Self::TOOL_CALL_MAX_BYTES));
        self.conn.execute(
            "INSERT INTO tool_calls (id, agent_id, session_id, trace_id, llm_call_id,
             step, tool_name, tool_source, skill_name, input, output,
             success, non_zero_exit, latency_ms, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id,
                agent_id,
                session_id,
                trace_id,
                llm_call_id,
                step,
                tool_name,
                tool_source,
                skill_name,
                truncated_input,
                truncated_output,
                success,
                non_zero_exit,
                latency_ms,
                scrubbed_error.as_deref(),
            ],
        )?;
        Ok(())
    }

    /// Prune LLM calls older than `retention_secs`.
    pub fn prune_old_llm_calls(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM llm_calls WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    /// Prune tool calls older than `retention_secs`.
    pub fn prune_old_tool_calls(&self, retention_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(retention_secs));
        let n = self.conn.execute(
            "DELETE FROM tool_calls WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    // ===== LLM Call Queries (Dashboard) =====

    pub fn query_llm_calls_by_trace(&self, trace_id: &str) -> Result<Vec<LlmCallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, provider, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                    response_text IS NOT NULL, reasoning IS NOT NULL
             FROM llm_calls WHERE trace_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![trace_id], Self::row_to_llm_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn query_tool_calls_by_trace(&self, trace_id: &str) -> Result<Vec<ToolCallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls WHERE trace_id = ?1 ORDER BY created_at ASC, step ASC",
        )?;
        let rows = stmt
            .query_map(params![trace_id], Self::row_to_tool_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn query_llm_calls_by_session(
        &self,
        session_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<LlmCallRow>, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM llm_calls WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        let offset = (page.saturating_sub(1)) * per_page;
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, provider, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                    response_text IS NOT NULL, reasoning IS NOT NULL
             FROM llm_calls WHERE session_id = ?1
             ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(params![session_id, per_page, offset], Self::row_to_llm_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn query_tool_calls_by_session(
        &self,
        session_id: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<ToolCallRow>, u64)> {
        let total: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tool_calls WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        let offset = (page.saturating_sub(1)) * per_page;
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls WHERE session_id = ?1
             ORDER BY created_at DESC, step ASC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(
                params![session_id, per_page, offset],
                Self::row_to_tool_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn query_llm_calls(
        &self,
        filters: &LlmCallFilters,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<LlmCallRow>, u64)> {
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref agent_id) = filters.agent_id {
            params_vec.push(Box::new(agent_id.clone()));
            where_clauses.push(format!("agent_id = ?{}", params_vec.len()));
        }
        if let Some(ref session_id) = filters.session_id {
            params_vec.push(Box::new(session_id.clone()));
            where_clauses.push(format!("session_id = ?{}", params_vec.len()));
        }
        if let Some(ref trace_id) = filters.trace_id {
            params_vec.push(Box::new(trace_id.clone()));
            where_clauses.push(format!("trace_id = ?{}", params_vec.len()));
        }
        if let Some(ref model) = filters.model {
            params_vec.push(Box::new(model.clone()));
            where_clauses.push(format!("model = ?{}", params_vec.len()));
        }
        if let Some(ref from) = filters.from {
            params_vec.push(Box::new(from.clone()));
            where_clauses.push(format!("created_at >= ?{}", params_vec.len()));
        }
        if let Some(ref to) = filters.to {
            params_vec.push(Box::new(to.clone()));
            where_clauses.push(format!("created_at <= ?{}", params_vec.len()));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM llm_calls {where_sql}");
        let total: u64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            |r| r.get(0),
        )?;

        let offset = (page.saturating_sub(1)) * per_page;
        params_vec.push(Box::new(per_page));
        params_vec.push(Box::new(offset));
        let query_sql = format!(
            "SELECT id, agent_id, session_id, trace_id, provider, model,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                    response_text IS NOT NULL, reasoning IS NOT NULL
             FROM llm_calls {where_sql}
             ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            params_vec.len() - 1,
            params_vec.len()
        );
        let mut stmt = self.conn.prepare(&query_sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
                Self::row_to_llm_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn query_cost_trend(&self, filters: &CostTrendFilters) -> Result<CostTrendResponse> {
        use std::collections::BTreeMap;

        // Determine effective from/to.
        let effective_from = filters
            .from
            .clone()
            .unwrap_or_else(|| timestamp::now_minus(Duration::hours(24)));
        let effective_to = filters.to.clone().unwrap_or_else(timestamp::now);

        // Determine bucket size.
        let bucket_size = match filters.bucket.as_deref() {
            Some("hour") => "hour",
            Some("day") => "day",
            Some("auto") | None => {
                let from_dt = timestamp::parse(&effective_from)
                    .unwrap_or_else(|_| Utc::now() - Duration::hours(24));
                let to_dt = timestamp::parse(&effective_to).unwrap_or_else(|_| Utc::now());
                let span_secs = (to_dt - from_dt).num_seconds();
                if span_secs < 259_200 { "hour" } else { "day" }
            }
            _ => "hour",
        };

        let bucket_expr = if bucket_size == "hour" {
            "substr(created_at, 1, 13) || ':00:00Z'"
        } else {
            "substr(created_at, 1, 10) || 'T00:00:00Z'"
        };

        // Build WHERE clause.
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        params_vec.push(Box::new(effective_from));
        where_clauses.push(format!("created_at >= ?{}", params_vec.len()));

        params_vec.push(Box::new(effective_to));
        where_clauses.push(format!("created_at <= ?{}", params_vec.len()));

        if let Some(ref agent_id) = filters.agent_id {
            params_vec.push(Box::new(agent_id.clone()));
            where_clauses.push(format!("agent_id = ?{}", params_vec.len()));
        }
        if let Some(ref model) = filters.model {
            params_vec.push(Box::new(model.clone()));
            where_clauses.push(format!("model = ?{}", params_vec.len()));
        }

        let where_sql = format!("WHERE {}", where_clauses.join(" AND "));

        let sql = format!(
            "SELECT {bucket_expr} as bucket_ts,
                    agent_id, provider, model,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    SUM(COALESCE(cache_read_tokens, 0)) as total_cache_read,
                    SUM(COALESCE(cache_write_tokens, 0)) as total_cache_write,
                    COUNT(*) as call_count
             FROM llm_calls
             {where_sql}
             GROUP BY bucket_ts, agent_id, provider, model
             ORDER BY bucket_ts ASC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            |row| {
                Ok((
                    row.get::<_, String>(0)?, // bucket_ts
                    row.get::<_, String>(1)?, // agent_id
                    row.get::<_, String>(2)?, // provider
                    row.get::<_, String>(3)?, // model
                    row.get::<_, u64>(4)?,    // total_input
                    row.get::<_, u64>(5)?,    // total_output
                    row.get::<_, u64>(6)?,    // total_cache_read
                    row.get::<_, u64>(7)?,    // total_cache_write
                    row.get::<_, u64>(8)?,    // call_count
                ))
            },
        )?;

        // Aggregate by (bucket_ts, agent_id), applying per-model pricing.
        let mut has_estimated = false;
        let mut estimated_models: Vec<String> = Vec::new();

        // Key: (bucket_ts, agent_id) -> (cost_usd, input_tokens, output_tokens, call_count)
        type BucketKey = (String, Option<String>);
        type BucketValue = (f64, u64, u64, u64);
        let mut aggregated: BTreeMap<BucketKey, BucketValue> = BTreeMap::new();

        for row in rows {
            let (
                bucket_ts,
                agent_id,
                provider,
                model,
                input,
                output,
                cache_read,
                cache_write,
                count,
            ) = row?;
            let pricing = crate::pricing::get_pricing(&provider, &model);
            let cost = crate::pricing::estimate_call_cost(
                &pricing,
                input,
                output,
                Some(cache_read),
                Some(cache_write),
            );
            if crate::pricing::is_fallback_pricing(&pricing) {
                has_estimated = true;
                let model_key = format!("{provider}/{model}");
                if !estimated_models.contains(&model_key) {
                    estimated_models.push(model_key);
                }
            }

            let agent_key = Some(agent_id);
            let entry = aggregated.entry((bucket_ts, agent_key)).or_default();
            entry.0 += cost;
            entry.1 += input;
            entry.2 += output;
            entry.3 += count;
        }

        let buckets = aggregated
            .into_iter()
            .map(
                |((ts, agent_id), (cost_usd, input_tokens, output_tokens, call_count))| {
                    CostTrendBucket {
                        timestamp: ts,
                        cost_usd,
                        input_tokens,
                        output_tokens,
                        call_count,
                        agent_id,
                    }
                },
            )
            .collect();

        Ok(CostTrendResponse {
            buckets,
            bucket_size: bucket_size.to_string(),
            has_estimated_pricing: has_estimated,
            estimated_models,
        })
    }

    pub fn query_tool_calls(
        &self,
        filters: &ToolCallFilters,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<ToolCallRow>, u64)> {
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref agent_id) = filters.agent_id {
            params_vec.push(Box::new(agent_id.clone()));
            where_clauses.push(format!("agent_id = ?{}", params_vec.len()));
        }
        if let Some(ref session_id) = filters.session_id {
            params_vec.push(Box::new(session_id.clone()));
            where_clauses.push(format!("session_id = ?{}", params_vec.len()));
        }
        if let Some(ref trace_id) = filters.trace_id {
            params_vec.push(Box::new(trace_id.clone()));
            where_clauses.push(format!("trace_id = ?{}", params_vec.len()));
        }
        if let Some(ref tool_name) = filters.tool_name {
            params_vec.push(Box::new(tool_name.clone()));
            where_clauses.push(format!("tool_name = ?{}", params_vec.len()));
        }
        if let Some(success) = filters.success {
            params_vec.push(Box::new(success));
            where_clauses.push(format!("success = ?{}", params_vec.len()));
        }
        if let Some(ref from) = filters.from {
            params_vec.push(Box::new(from.clone()));
            where_clauses.push(format!("created_at >= ?{}", params_vec.len()));
        }
        if let Some(ref to) = filters.to {
            params_vec.push(Box::new(to.clone()));
            where_clauses.push(format!("created_at <= ?{}", params_vec.len()));
        }
        if let Some(ref keyword) = filters.keyword {
            // Escape LIKE metacharacters so %, _ in the keyword are treated as literals.
            let escaped = keyword
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let like_pattern = format!("%{escaped}%");
            params_vec.push(Box::new(like_pattern.clone()));
            let p1 = params_vec.len();
            params_vec.push(Box::new(like_pattern));
            let p2 = params_vec.len();
            where_clauses.push(format!(
                "(input LIKE ?{p1} ESCAPE '\\' OR output LIKE ?{p2} ESCAPE '\\')"
            ));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM tool_calls {where_sql}");
        let total: u64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
            |r| r.get(0),
        )?;

        let offset = (page.saturating_sub(1)) * per_page;
        params_vec.push(Box::new(per_page));
        params_vec.push(Box::new(offset));
        let query_sql = format!(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls {where_sql}
             ORDER BY created_at DESC, step ASC LIMIT ?{} OFFSET ?{}",
            params_vec.len() - 1,
            params_vec.len()
        );
        let mut stmt = self.conn.prepare(&query_sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_vec.iter().map(|p| p.as_ref())),
                Self::row_to_tool_call,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rows, total))
    }

    pub fn get_llm_call_by_id(&self, id: &str) -> Result<Option<LlmCallRow>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, session_id, trace_id, provider, model,
                        input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        latency_ms, stop_reason, status, error_message, step, prompt_variant, created_at,
                        response_text, reasoning
                 FROM llm_calls WHERE id = ?1",
                params![id],
                Self::row_to_llm_call_detail,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_tool_calls_by_llm_call_id(&self, llm_call_id: &str) -> Result<Vec<ToolCallRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                    step, tool_name, tool_source, skill_name,
                    input, output, success, non_zero_exit,
                    latency_ms, error_message, created_at
             FROM tool_calls WHERE llm_call_id = ?1 ORDER BY created_at ASC, step ASC",
        )?;
        let rows = stmt
            .query_map(params![llm_call_id], Self::row_to_tool_call)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn get_tool_call_by_id(&self, id: &str) -> Result<Option<ToolCallRow>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, session_id, trace_id, llm_call_id,
                        step, tool_name, tool_source, skill_name,
                        input, output, success, non_zero_exit,
                        latency_ms, error_message, created_at
                 FROM tool_calls WHERE id = ?1",
                params![id],
                Self::row_to_tool_call,
            )
            .optional()
            .map_err(Into::into)
    }

    fn row_to_llm_call(r: &rusqlite::Row<'_>) -> rusqlite::Result<LlmCallRow> {
        Ok(LlmCallRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            trace_id: r.get(3)?,
            provider: r.get(4)?,
            model: r.get(5)?,
            input_tokens: r.get(6)?,
            output_tokens: r.get(7)?,
            cache_read_tokens: r.get(8)?,
            cache_write_tokens: r.get(9)?,
            latency_ms: r.get(10)?,
            stop_reason: r.get(11)?,
            status: r.get(12)?,
            error_message: r.get(13)?,
            step: r.get(14)?,
            prompt_variant: r.get(15)?,
            created_at: r.get(16)?,
            // List queries omit response_text/reasoning for performance
            response_text: None,
            reasoning: None,
            // Boolean indicators from `response_text IS NOT NULL` / `reasoning IS NOT NULL`
            has_response_text: r.get(17)?,
            has_reasoning: r.get(18)?,
            cost_usd: None,
        })
    }

    /// Detail query includes response_text and reasoning columns.
    fn row_to_llm_call_detail(r: &rusqlite::Row<'_>) -> rusqlite::Result<LlmCallRow> {
        let response_text: Option<String> = r.get(17)?;
        let reasoning: Option<String> = r.get(18)?;
        Ok(LlmCallRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            trace_id: r.get(3)?,
            provider: r.get(4)?,
            model: r.get(5)?,
            input_tokens: r.get(6)?,
            output_tokens: r.get(7)?,
            cache_read_tokens: r.get(8)?,
            cache_write_tokens: r.get(9)?,
            latency_ms: r.get(10)?,
            stop_reason: r.get(11)?,
            status: r.get(12)?,
            error_message: r.get(13)?,
            step: r.get(14)?,
            prompt_variant: r.get(15)?,
            created_at: r.get(16)?,
            has_response_text: response_text.is_some(),
            has_reasoning: reasoning.is_some(),
            response_text,
            reasoning,
            cost_usd: None,
        })
    }

    fn row_to_tool_call(r: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRow> {
        Ok(ToolCallRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            trace_id: r.get(3)?,
            llm_call_id: r.get(4)?,
            step: r.get(5)?,
            tool_name: r.get(6)?,
            tool_source: r.get(7)?,
            skill_name: r.get(8)?,
            input: r.get(9)?,
            output: r.get(10)?,
            success: r.get(11)?,
            non_zero_exit: r.get(12)?,
            latency_ms: r.get(13)?,
            error_message: r.get(14)?,
            created_at: r.get(15)?,
        })
    }

    /// Update session metadata column.
    pub fn update_session_metadata(&self, session_id: &str, metadata: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET metadata = ?1 WHERE id = ?2",
            params![metadata, session_id],
        )?;
        Ok(())
    }

    // ===== Messages =====

    pub fn save_message(
        &self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, agent_id, role, content, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_message_with_metadata(
        &self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
        internal: bool,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, metadata, trace_id, internal)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![session_id, agent_id, role, content, metadata, trace_id, internal as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a single row into `task_messages` inside a caller-provided transaction.
    /// Used by `save_message_with_task_context` for the double-write contract (mika#974).
    #[allow(clippy::too_many_arguments)]
    fn insert_task_message_tx(
        tx: &rusqlite::Transaction<'_>,
        task_id: &str,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        tx.execute(
            "INSERT INTO task_messages (task_id, agent_id, session_id, role, content, metadata, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, agent_id, session_id, role, content, metadata, trace_id],
        )?;
        Ok(tx.last_insert_rowid())
    }

    /// Insert a single row into `task_messages` without a transaction.
    /// Used by the dispatcher to write engine-internal task narrative (e.g., callback
    /// summaries) that should NOT appear in `messages` (mika#965).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_task_message(
        &self,
        task_id: &str,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO task_messages (task_id, agent_id, session_id, role, content, metadata, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, agent_id, session_id, role, content, metadata, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Double-write: insert into `messages` AND `task_messages` in a single transaction.
    /// When `task_id` is `None`, behaves identically to `save_message_with_metadata`
    /// (no transaction overhead, no `task_messages` row).
    #[allow(clippy::too_many_arguments)]
    pub fn save_message_with_task_context(
        &mut self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
        internal: bool,
        task_id: Option<&str>,
    ) -> Result<i64> {
        match task_id {
            Some(tid) => {
                let tx = self.conn.transaction()?;
                tx.execute(
                    "INSERT INTO messages (session_id, agent_id, role, content, metadata, trace_id, internal)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![session_id, agent_id, role, content, metadata, trace_id, internal as i64],
                )?;
                let msg_id = tx.last_insert_rowid();
                Self::insert_task_message_tx(
                    &tx, tid, agent_id, session_id, role, content, metadata, trace_id,
                )?;
                tx.commit()?;
                Ok(msg_id)
            }
            None => self.save_message_with_metadata(
                agent_id, session_id, role, content, metadata, trace_id, internal,
            ),
        }
    }

    /// Load all task messages for a given task, ordered by creation time.
    /// Returns the full narrative — no limit, no compaction.
    pub fn load_task_messages(&self, task_id: &str) -> Result<Vec<TaskMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, agent_id, session_id, role, content, metadata, trace_id, created_at
             FROM task_messages
             WHERE task_id = ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![task_id], |r| {
                Ok(TaskMessage {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    session_id: r.get(3)?,
                    role: r.get(4)?,
                    content: r.get(5)?,
                    metadata: r.get(6)?,
                    trace_id: r.get(7)?,
                    created_at: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    const SESSION_MESSAGE_COLUMNS: &'static str = "m.id, m.session_id, m.agent_id, m.role, m.content, s.channel_type, m.metadata, m.trace_id, m.created_at, m.internal";

    fn row_to_session_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMessage> {
        Ok(SessionMessage {
            id: r.get(0)?,
            session_id: r.get(1)?,
            agent_id: r.get(2)?,
            role: r.get(3)?,
            content: r.get(4)?,
            channel_type: r.get(5)?,
            metadata: r.get(6)?,
            trace_id: r.get(7)?,
            created_at: r.get(8)?,
            internal: r.get::<_, i64>(9).unwrap_or(0) != 0,
        })
    }

    pub fn load_recent_messages(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionMessage>> {
        let (msgs, _) = self.load_recent_messages_filtered(agent_id, limit, false)?;
        Ok(msgs)
    }

    /// Rebuild conversation context for prompt assembly (mika#974).
    ///
    /// - `task_id = None` → existing `load_recent_messages` path (channel-mode).
    /// - `task_id = Some(tid)` → hybrid merge: load both `task_messages` (full narrative)
    ///   and `messages` (recent channel context), merge sorted by `created_at`,
    ///   dedup on `(session_id, role, content, created_at)`.
    pub fn rebuild_context(
        &self,
        agent_id: &str,
        task_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionMessage>> {
        let tid = match task_id {
            Some(t) => t,
            None => return self.load_recent_messages(agent_id, limit),
        };

        // Load channel messages (recent window).
        let channel_msgs = self.load_recent_messages(agent_id, limit)?;

        // Load task narrative (full history, no limit).
        let task_msgs = self.load_task_messages(tid)?;

        // Convert TaskMessages to SessionMessages for uniform handling.
        // task_messages don't have channel_type or internal — use sensible defaults.
        let task_as_session: Vec<SessionMessage> = task_msgs
            .into_iter()
            .map(|tm| SessionMessage {
                id: tm.id,
                session_id: tm.session_id,
                agent_id: tm.agent_id,
                role: tm.role,
                content: tm.content,
                channel_type: String::new(),
                metadata: tm.metadata,
                trace_id: tm.trace_id,
                created_at: tm.created_at,
                internal: false,
            })
            .collect();

        // Merge both sets, dedup on (session_id, role, content, created_at),
        // sort by created_at ASC.
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<SessionMessage> =
            Vec::with_capacity(channel_msgs.len() + task_as_session.len());

        for msg in task_as_session.into_iter().chain(channel_msgs.into_iter()) {
            let key = (
                msg.session_id.clone(),
                msg.role.clone(),
                msg.content.clone(),
                msg.created_at.clone(),
            );
            if seen.insert(key) {
                merged.push(msg);
            }
        }

        merged.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        Ok(merged)
    }

    /// Load recent messages with optional internal-message filtering.
    ///
    /// Returns `(visible_messages, hidden_internal_count)`. When `exclude_internal`
    /// is true, internal messages within the limit-bound window are counted but
    /// excluded from the returned Vec. The count is best-effort: it reflects
    /// internals discarded from the limit-bound window, not the total across all
    /// history. When `exclude_internal` is false, the count is always 0.
    pub fn load_recent_messages_filtered(
        &self,
        agent_id: &str,
        limit: usize,
        exclude_internal: bool,
    ) -> Result<(Vec<SessionMessage>, usize)> {
        // Always fetch without the internal filter so we can count hidden rows.
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
              ORDER BY m.created_at DESC, m.id DESC LIMIT ?2",
            Self::SESSION_MESSAGE_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let all_rows = stmt
            .query_map(
                params![agent_id, limit as i64],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        if !exclude_internal {
            let mut messages = all_rows;
            messages.reverse();
            return Ok((messages, 0));
        }

        // Partition: visible messages and count of hidden internals.
        let mut hidden_count = 0usize;
        let mut messages = Vec::with_capacity(all_rows.len());
        for msg in all_rows {
            if msg.internal {
                hidden_count += 1;
            } else {
                messages.push(msg);
            }
        }
        messages.reverse();
        Ok((messages, hidden_count))
    }

    pub fn load_conversation_summary(&self, agent_id: &str) -> Result<Option<SessionMessage>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
                      WHERE m.agent_id = ?1 AND m.role = 'summary'
                      ORDER BY m.created_at DESC LIMIT 1",
                    Self::SESSION_MESSAGE_COLUMNS
                ),
                params![agent_id],
                Self::row_to_session_message,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn count_messages(&self, agent_id: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE agent_id = ?1 AND role != 'summary'",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn load_messages_before_window(
        &self,
        agent_id: &str,
        window_size: usize,
    ) -> Result<Vec<SessionMessage>> {
        let cutoff_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM messages WHERE agent_id = ?1 AND role != 'summary'
                  ORDER BY created_at DESC, id DESC LIMIT 1 OFFSET ?2",
                params![agent_id, window_size as i64],
                |r| r.get(0),
            )
            .optional()?;
        let cutoff_id = match cutoff_id {
            Some(id) => id,
            None => return Ok(vec![]),
        };
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND m.id <= ?2
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, cutoff_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn replace_with_summary(
        &mut self,
        agent_id: &str,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let system_session = self.get_or_create_system_session(agent_id)?;
        // RAII transaction: Drop without commit() auto-rolls back, preventing
        // stuck transactions that pin the WAL snapshot (mika#636).
        let tx = self.conn.transaction()?;
        // Delete old non-summary messages up to compacted_through_id
        tx.execute(
            "DELETE FROM messages
             WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2",
            params![agent_id, compacted_through_id],
        )?;
        // Remove old summary
        tx.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND role = 'summary'",
            params![agent_id],
        )?;
        // Insert new summary (no trace_id — summaries span multiple traces)
        tx.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, compacted_through_id)
             VALUES (?1, ?2, 'summary', ?3, ?4)",
            params![system_session, agent_id, summary, compacted_through_id],
        )?;
        let row_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(row_id)
    }

    pub fn load_messages_after(
        &self,
        agent_id: &str,
        after_id: i64,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.id > ?2
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, after_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn max_message_id(&self, agent_id: &str) -> Result<i64> {
        let id: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM messages WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Load a single message by its row ID.
    pub fn get_message_by_id(&self, message_id: i64) -> Result<Option<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.id = ?1",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt
            .query_map(params![message_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.pop())
    }

    /// Load messages surrounding a given message ID within the same session.
    /// Returns up to `before` messages before and `after` messages after the target.
    pub fn get_surrounding_messages(
        &self,
        session_id: &str,
        target_id: i64,
        before: u32,
        after: u32,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.session_id = ?1
                AND (m.id >= (SELECT id FROM (SELECT id FROM messages WHERE session_id = ?1 AND id <= ?2 ORDER BY id DESC LIMIT ?3) sub ORDER BY id ASC LIMIT 1))
                AND m.id <= (SELECT id FROM (SELECT id FROM messages WHERE session_id = ?1 AND id >= ?2 ORDER BY id ASC LIMIT ?4) sub ORDER BY id DESC LIMIT 1)
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![session_id, target_id, before + 1, after + 1],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_messages_since(&self, agent_id: &str, since: &str) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.created_at >= ?2 AND m.role != 'summary'
              ORDER BY m.created_at ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn last_user_message_time(&self, agent_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT MAX(created_at) FROM messages
                  WHERE agent_id = ?1 AND role = 'user'",
                params![agent_id],
                |r| r.get(0),
            )
            .optional()
            .map(|opt| opt.flatten())
            .map_err(Into::into)
    }

    // ===== Core Memory =====

    pub fn seed_core_memory(&self, agent_id: &str, user_md_content: Option<&str>) -> Result<()> {
        for (key, default) in CORE_MEMORY_SECTIONS {
            let existing = self.get_core_memory(agent_id, key)?;
            if existing.is_none() {
                self.set_core_memory(agent_id, key, default)?;
            }
        }
        // Override self_model with agent-aware default (only if still at static default)
        if let Some(entry) = self.get_core_memory(agent_id, "self_model")?
            && entry.value == "No interaction history yet."
        {
            let display_name = self.get_agent_display_name(agent_id);
            self.set_core_memory(agent_id, "self_model", &default_self_model(&display_name))?;
        }
        if let Some(md) = user_md_content
            && !md.trim().is_empty()
        {
            self.set_core_memory(agent_id, "user_summary", md.trim())?;
        }
        Ok(())
    }

    /// Migrate legacy `persona` key to `self_model` for an agent.
    pub fn migrate_persona_to_self_model(&self, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE core_memory SET key = 'self_model' WHERE agent_id = ?1 AND key = 'persona'",
            params![agent_id],
        )?;
        Ok(())
    }

    pub fn total_core_memory_tokens(&self, agent_id: &str) -> Result<i32> {
        let n: i32 = self.conn.query_row(
            "SELECT COALESCE(SUM(token_count), 0) FROM core_memory WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    // ===== People =====

    pub fn search_people(&self, agent_id: &str, query: &str) -> Result<Vec<Person>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name, relationship, notes,
                     first_mentioned,
                     last_mentioned,
                     mention_count
              FROM people
              WHERE agent_id = ?1 AND (
                  canonical_name LIKE ?2 OR relationship LIKE ?2 OR notes LIKE ?2
              )
              ORDER BY canonical_name",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], |r| {
                Ok(Person {
                    id: r.get(0)?,
                    canonical_name: r.get(1)?,
                    relationship: r.get(2)?,
                    notes: r.get(3)?,
                    first_mentioned: r.get(4)?,
                    last_mentioned: r.get(5)?,
                    mention_count: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Commitments =====

    pub fn add_commitment(
        &self,
        agent_id: &str,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO commitments (agent_id, description, due_date, person_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, description, due_date, person_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn row_to_commitment(r: &rusqlite::Row<'_>) -> rusqlite::Result<Commitment> {
        Ok(Commitment {
            id: r.get(0)?,
            description: r.get(1)?,
            status: r.get(2)?,
            due_date: r.get(3)?,
            person_id: r.get(4)?,
            created_at: r.get(5)?,
            completed_at: r.get(6)?,
        })
    }

    pub fn list_commitments(&self, agent_id: &str, status: &str) -> Result<Vec<Commitment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, status, due_date, person_id,
                     created_at,
                     completed_at
              FROM commitments WHERE agent_id = ?1 AND status = ?2
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, status], Self::row_to_commitment)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn update_commitment_status(&self, agent_id: &str, id: i64, status: &str) -> Result<bool> {
        let completed_at: Option<String> = if status == "completed" {
            Some(timestamp::now())
        } else {
            None
        };
        let n = self.conn.execute(
            "UPDATE commitments SET status = ?1, completed_at = ?2
             WHERE agent_id = ?3 AND id = ?4",
            params![status, completed_at, agent_id, id],
        )?;
        Ok(n > 0)
    }

    pub fn get_commitment_status(&self, agent_id: &str, id: i64) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT status FROM commitments WHERE agent_id = ?1 AND id = ?2",
                params![agent_id, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_commitment_details(
        &self,
        agent_id: &str,
        id: i64,
    ) -> Result<Option<(String, Option<String>)>> {
        self.conn
            .query_row(
                "SELECT description, due_date FROM commitments WHERE agent_id = ?1 AND id = ?2",
                params![agent_id, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn search_commitments(&self, agent_id: &str, query: &str) -> Result<Vec<Commitment>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, description, status, due_date, person_id,
                     created_at,
                     completed_at
              FROM commitments WHERE agent_id = ?1 AND description LIKE ?2
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], Self::row_to_commitment)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Preferences =====

    pub fn set_preference(&self, agent_id: &str, category: &str, value: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO preferences (agent_id, category, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, category) DO UPDATE SET
                 value = excluded.value, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![agent_id, category, value],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_preferences(&self, agent_id: &str, query: &str) -> Result<Vec<Preference>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT category, value, updated_at
              FROM preferences WHERE agent_id = ?1 AND (category LIKE ?2 OR value LIKE ?2)
              ORDER BY category",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], |r| {
                Ok(Preference {
                    category: r.get(0)?,
                    value: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Events =====

    pub fn add_event(
        &self,
        agent_id: &str,
        description: &str,
        event_date: Option<&str>,
        context: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO events (agent_id, description, event_date, context) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, description, event_date, context],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn search_events(&self, agent_id: &str, query: &str) -> Result<Vec<Event>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, description, event_date, context, created_at
              FROM events WHERE agent_id = ?1 AND (description LIKE ?2 OR context LIKE ?2)
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, pattern], |r| {
                Ok(Event {
                    id: r.get(0)?,
                    description: r.get(1)?,
                    event_date: r.get(2)?,
                    context: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Auto-Pull Stats (mika#1363) =====

    /// Get the failure count for a specific issue in `auto_pull_stats`.
    /// Returns 0 if no row exists.
    pub fn get_auto_pull_failure_count(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(failure_count, 0) FROM auto_pull_stats
                 WHERE repo_full_name = ?1 AND issue_number = ?2",
                params![repo_full_name, issue_number as i64],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n)
    }

    /// Record an auto-pull event: upsert the row with `last_auto_pull_at = now`.
    pub fn record_auto_pull(&self, repo_full_name: &str, issue_number: u64) -> Result<()> {
        let now = crate::timestamp::now();
        self.conn.execute(
            "INSERT INTO auto_pull_stats (repo_full_name, issue_number, failure_count, last_auto_pull_at)
             VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(repo_full_name, issue_number) DO UPDATE SET last_auto_pull_at = ?3",
            params![repo_full_name, issue_number as i64, now],
        )?;
        Ok(())
    }

    /// Increment the failure counter for a ticket (circuit-breaker).
    pub fn increment_auto_pull_failure(
        &self,
        repo_full_name: &str,
        issue_number: u64,
    ) -> Result<()> {
        let now = crate::timestamp::now();
        self.conn.execute(
            "INSERT INTO auto_pull_stats (repo_full_name, issue_number, failure_count, last_failure_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(repo_full_name, issue_number)
             DO UPDATE SET failure_count = failure_count + 1, last_failure_at = ?3",
            params![repo_full_name, issue_number as i64, now],
        )?;
        Ok(())
    }

    /// Reset the failure counter for a ticket (on success or operator-driven ready).
    pub fn reset_auto_pull_failure(&self, repo_full_name: &str, issue_number: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE auto_pull_stats SET failure_count = 0, last_failure_at = NULL
             WHERE repo_full_name = ?1 AND issue_number = ?2",
            params![repo_full_name, issue_number as i64],
        )?;
        Ok(())
    }

    /// Count `update_core_memory` audit events in the most recent non-system session.
    pub fn count_core_memory_edits_latest_session(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE agent_id = ?1
               AND tool_name = 'update_core_memory'
               AND session_id = (
                   SELECT id FROM sessions
                   WHERE agent_id = ?1
                     AND id NOT LIKE 'system-%'
                   ORDER BY started_at DESC
                   LIMIT 1
               )",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    // ===== Rewind =====

    /// Get messages after a given message ID within a session.
    pub fn get_messages_after_id(
        &self,
        agent_id: &str,
        session_id: &str,
        after_id: i64,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.session_id = ?2 AND m.id > ?3
              ORDER BY m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![agent_id, session_id, after_id],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get the compaction boundary — the highest message ID that has been compacted.
    pub fn get_compaction_boundary(&self, agent_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT compacted_through_id FROM messages
                  WHERE agent_id = ?1 AND role = 'summary'
                  ORDER BY id DESC LIMIT 1",
                params![agent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Delete messages after a given ID in a session. Returns count deleted.
    pub fn delete_messages_after_id(
        &self,
        agent_id: &str,
        session_id: &str,
        after_id: i64,
    ) -> Result<usize> {
        let deleted = self.conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND session_id = ?2 AND id > ?3",
            params![agent_id, session_id, after_id],
        )?;
        Ok(deleted)
    }

    /// Delete rewind context marker messages from a session.
    /// Called before injecting a new marker to prevent accumulation during rapid rewinds.
    pub fn delete_rewind_markers(&self, agent_id: &str, session_id: &str) -> Result<usize> {
        let pattern = format!("{}%", crate::rewind::REWIND_MARKER_PREFIX);
        let deleted = self.conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND session_id = ?2 \
             AND role = 'system' AND content LIKE ?3",
            params![agent_id, session_id, pattern],
        )?;
        Ok(deleted)
    }

    /// Delete a commitment by description. Returns true if deleted.
    pub fn delete_commitment_by_description(
        &self,
        agent_id: &str,
        description: &str,
    ) -> Result<bool> {
        let commitment_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM commitments WHERE agent_id = ?1 AND description = ?2 COLLATE NOCASE",
                params![agent_id, description],
                |r| r.get(0),
            )
            .optional()?;
        let Some(cid) = commitment_id else {
            return Ok(false);
        };
        self.delete_search_content(agent_id, "commitment", cid)?;
        let deleted = self.conn.execute(
            "DELETE FROM commitments WHERE id = ?1 AND agent_id = ?2",
            params![cid, agent_id],
        )?;
        Ok(deleted > 0)
    }

    /// Get tasks created by the given trace_ids (via created_trace_id column).
    /// Returns tasks ordered by created_at DESC.
    pub fn get_tasks_by_trace_ids(
        &self,
        agent_id: &str,
        trace_ids: &[String],
    ) -> Result<Vec<Task>> {
        if trace_ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: Vec<String> = (0..trace_ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "SELECT {} FROM tasks WHERE agent_id = ?1 AND created_trace_id IN ({}) ORDER BY created_at DESC",
            Self::TASK_COLUMNS,
            placeholders.join(", ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(agent_id.to_string()));
        for tid in trace_ids {
            params_vec.push(Box::new(tid.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(&*param_refs, Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Delete a task by ID. Returns true if deleted.
    /// Only deletes tasks that are not in a terminal actioned state.
    pub fn delete_task_by_id(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE id = ?1 AND agent_id = ?2",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    // ===== Heartbeat =====

    pub fn record_heartbeat_send(&self, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO heartbeat_sends (agent_id) VALUES (?1)",
            params![agent_id],
        )?;
        Ok(())
    }

    pub fn count_heartbeat_sends_last_hour(&self, agent_id: &str) -> Result<u32> {
        let n: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends
             WHERE agent_id = ?1 AND sent_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn count_heartbeat_sends_today(&self, agent_id: &str, timezone: &str) -> Result<u32> {
        let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);
        let midnight_local = now_local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default();
        let since_ts = tz
            .from_local_datetime(&midnight_local)
            .earliest()
            .map(|dt| timestamp::format(&dt.with_timezone(&Utc)))
            .unwrap_or_else(timestamp::now);
        let n: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends WHERE agent_id = ?1 AND sent_at >= ?2",
            params![agent_id, since_ts],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn prune_old_heartbeat_sends(&self, agent_id: &str, days: u32) -> Result<()> {
        let cutoff = timestamp::now_minus(Duration::days(days as i64));
        self.conn.execute(
            "DELETE FROM heartbeat_sends WHERE agent_id = ?1 AND sent_at < ?2",
            params![agent_id, cutoff],
        )?;
        Ok(())
    }

    // ===== Reflection =====

    pub fn record_reflection_run(
        &self,
        agent_id: &str,
        status: &str,
        changes_made: i64,
        summary: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reflection_runs (agent_id, status, changes_made, summary)
             VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, status, changes_made, summary],
        )?;
        Ok(())
    }

    pub fn last_reflection_run_today(&self, agent_id: &str, timezone: &str) -> Result<bool> {
        let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);
        let midnight_local = now_local
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default();
        let since_ts = tz
            .from_local_datetime(&midnight_local)
            .earliest()
            .map(|dt| timestamp::format(&dt.with_timezone(&Utc)))
            .unwrap_or_else(timestamp::now);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM reflection_runs
             WHERE agent_id = ?1 AND status = 'completed' AND created_at >= ?2",
            params![agent_id, since_ts],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn prune_old_reflection_runs(&self, agent_id: &str, days: u32) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::days(days as i64));
        let n = self.conn.execute(
            "DELETE FROM reflection_runs WHERE agent_id = ?1 AND created_at < ?2",
            params![agent_id, cutoff],
        )?;
        Ok(n)
    }

    // ===== Failed Sends =====

    pub fn save_failed_send(
        &self,
        agent_id: &str,
        text: &str,
        request_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO failed_sends (agent_id, text, request_id) VALUES (?1, ?2, ?3)",
            params![agent_id, text, request_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_pending_failed_sends(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<FailedSend>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, request_id, created_at, retry_count
              FROM failed_sends WHERE agent_id = ?1
              ORDER BY created_at ASC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![agent_id, limit as i64], |r| {
                Ok(FailedSend {
                    id: r.get(0)?,
                    text: r.get(1)?,
                    request_id: r.get(2)?,
                    created_at: r.get(3)?,
                    retry_count: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn delete_failed_send(&self, agent_id: &str, id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM failed_sends WHERE agent_id = ?1 AND id = ?2",
            params![agent_id, id],
        )?;
        Ok(())
    }

    pub fn increment_failed_send_retry(&self, agent_id: &str, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE failed_sends SET retry_count = retry_count + 1 WHERE agent_id = ?1 AND id = ?2",
            params![agent_id, id],
        )?;
        Ok(())
    }

    // ===== Customer Config =====

    pub fn get_customer_config(&self, agent_id: &str, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM customer_config WHERE agent_id = ?1 AND key = ?2",
                params![agent_id, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_customer_config(&self, agent_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO customer_config (agent_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent_id, key) DO UPDATE SET value = excluded.value, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![agent_id, key, value],
        )?;
        Ok(())
    }

    pub fn list_customer_config(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM customer_config WHERE agent_id = ?1 ORDER BY key")?;
        let rows = stmt
            .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Team Runs =====

    pub fn insert_team_run(
        &self,
        run_id: &str,
        team_name: &str,
        goal: &str,
        max_iterations: u32,
        started_at: &str,
        trace_id: Option<&str>,
    ) -> Result<()> {
        // Auto-register team (team_id = team_name)
        self.conn.execute(
            "INSERT OR IGNORE INTO teams (id, name) VALUES (?1, ?1)",
            params![team_name],
        )?;
        self.conn.execute(
            "INSERT INTO team_runs (id, team_id, goal, max_iterations, started_at, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run_id,
                team_name,
                goal,
                max_iterations,
                started_at,
                trace_id
            ],
        )?;
        Ok(())
    }

    pub fn update_team_run(
        &self,
        run_id: &str,
        status: &str,
        failure_reason: Option<&str>,
        iteration: u32,
        deliverable: Option<&str>,
        ended_at: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE team_runs SET status = ?1, failure_reason = ?2, iteration = ?3,
             deliverable = ?4, ended_at = ?5
             WHERE id = ?6",
            params![
                status,
                failure_reason,
                iteration,
                deliverable,
                ended_at,
                run_id
            ],
        )?;
        Ok(())
    }

    /// Find team runs stuck in `status='running'` past `threshold_secs` whose
    /// child sessions show no recent liveness (mika#1652).
    ///
    /// A run is orphaned when no terminal-state writer ran (process died, the
    /// `run_team` future was dropped mid-flight at the tool timeout, etc.), so
    /// the row never left `running` and its team slot stays held. Liveness is
    /// proven by any `llm_calls` or `tool_calls` activity on a `team-<id>%`
    /// session within `liveness_threshold_secs` — the mika#959 `NOT EXISTS`
    /// watchdog pattern, adapted for non-process entities (team child sessions
    /// are LLM-call/tool-call rows, not subprocesses, so liveness is row
    /// recency rather than a `/proc/<pid>/stat` check).
    ///
    /// The `team-<id>%` LIKE prefix matches both the orchestrator session
    /// (`team-<id>`) and per-member sessions (`team-<id>-<agent>`); run ids are
    /// UUIDs, so the prefix never bleeds across runs.
    pub fn find_stuck_team_runs(
        &self,
        threshold_secs: i64,
        liveness_threshold_secs: i64,
    ) -> Result<Vec<TeamRunRow>> {
        let stuck_modifier = format!("-{threshold_secs} seconds");
        let liveness_modifier = format!("-{liveness_threshold_secs} seconds");
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE r.status = 'running'
                AND r.started_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
                AND NOT EXISTS (
                  SELECT 1 FROM llm_calls lc
                  WHERE lc.session_id LIKE 'team-' || r.id || '%'
                    AND lc.created_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                )
                AND NOT EXISTS (
                  SELECT 1 FROM tool_calls tc
                  WHERE tc.session_id LIKE 'team-' || r.id || '%'
                    AND tc.created_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                )
              ORDER BY r.started_at",
            Self::TEAM_RUN_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![stuck_modifier, liveness_modifier],
                Self::row_to_team_run,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Transition a team run to a terminal status idempotently (mika#1652).
    ///
    /// The `WHERE status = 'running'` guard prevents double-transition and
    /// loses cleanly to the normal finalizer (`update_team_run`) or an operator
    /// cancel that won the race. Returns `true` when a row actually changed.
    ///
    /// `status` is `'failed'` for reaper-initiated transitions (system-level
    /// failure detection); `'cancelled'` is reserved for operator-initiated
    /// termination. Both are permitted by the `team_runs.status` CHECK
    /// constraint.
    pub fn transition_team_run_terminal(
        &self,
        team_run_id: &str,
        status: &str,
        failure_reason: &str,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE team_runs
             SET status = ?1,
                 failure_reason = ?2,
                 ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3 AND status = 'running'",
            params![status, failure_reason, team_run_id],
        )?;
        Ok(changed > 0)
    }

    /// Suspend a team run, saving a serialized checkpoint for later resume.
    pub fn suspend_team_run(&self, run_id: &str, checkpoint: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE team_runs SET status = 'suspended', checkpoint = ?1 WHERE id = ?2",
            params![checkpoint, run_id],
        )?;
        Ok(())
    }

    /// Load the serialized checkpoint from a suspended team run.
    pub fn load_team_run_checkpoint(&self, run_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT checkpoint FROM team_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
            .map(|o| o.flatten())
            .map_err(Into::into)
    }

    /// Resume a suspended team run (set status back to 'running').
    pub fn resume_team_run_status(&self, run_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE team_runs SET status = 'running', checkpoint = NULL WHERE id = ?1",
            params![run_id],
        )?;
        Ok(())
    }

    /// Load the trace_id from a team run (for resume continuity).
    pub fn load_team_run_trace_id(&self, run_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT trace_id FROM team_runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
            .map(|o| o.flatten())
            .map_err(Into::into)
    }

    const TEAM_RUN_COLUMNS: &'static str = "r.id, t.name, r.goal, r.status, r.failure_reason,
         r.iteration, r.max_iterations, r.deliverable, r.started_at, r.ended_at, r.trace_id";

    fn row_to_team_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<TeamRunRow> {
        Ok(TeamRunRow {
            id: r.get(0)?,
            team_name: r.get(1)?,
            goal: r.get(2)?,
            status: r.get(3)?,
            failure_reason: r.get(4)?,
            iteration: r.get::<_, u32>(5)?,
            max_iterations: r.get::<_, u32>(6)?,
            deliverable: r.get(7)?,
            started_at: r.get(8)?,
            ended_at: r.get(9)?,
            trace_id: r.get(10)?,
        })
    }

    pub fn load_team_runs(&self, team_name: &str, limit: usize) -> Result<Vec<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1
              ORDER BY r.started_at DESC LIMIT ?2",
            Self::TEAM_RUN_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![team_name, limit as i64], Self::row_to_team_run)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn load_team_runs_for_prompt(
        &self,
        team_name: &str,
        limit: usize,
        max_text_len: usize,
    ) -> Result<Vec<TeamRunRow>> {
        let mut runs = self.load_team_runs(team_name, limit)?;
        for run in &mut runs {
            if let Some(ref d) = run.deliverable
                && d.len() > max_text_len
            {
                run.deliverable = Some(format!("{}...", &d[..max_text_len]));
            }
        }
        Ok(runs)
    }

    pub fn load_latest_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1
              ORDER BY r.started_at DESC LIMIT 1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![team_name], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    pub fn load_team_run_by_id(&self, run_id: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE r.id = ?1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![run_id], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    /// Load the most recent team run that is not running or cancelled.
    /// Returns completed, failed, or suspended runs only.
    pub fn get_last_completed_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1 COLLATE NOCASE
                AND r.status IN ('completed', 'failed', 'suspended')
              ORDER BY r.started_at DESC LIMIT 1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![team_name], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    /// Load the most recent finished team run (completed, failed, or cancelled).
    /// Excludes running and suspended runs — only truly finished runs are returned.
    pub fn get_last_finished_team_run(&self, team_name: &str) -> Result<Option<TeamRunRow>> {
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1 COLLATE NOCASE
                AND r.status NOT IN ('running', 'suspended')
              ORDER BY r.started_at DESC LIMIT 1",
            Self::TEAM_RUN_COLUMNS,
        );
        self.conn
            .query_row(&sql, params![team_name], Self::row_to_team_run)
            .optional()
            .map_err(Into::into)
    }

    /// Build an enriched summary of a team run for context injection.
    /// Queries team_workspace (assignments, critic), messages (agent responses),
    /// and tasks (statuses) for the given run.
    pub fn get_team_run_summary(&self, run_id: &str) -> Result<Option<TeamRunSummary>> {
        let run = match self.load_team_run_by_id(run_id)? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Get agent names from assignment entries
        let agent_names: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT agent_name FROM team_workspace
                 WHERE run_id = ?1 AND entry_type = 'assignment' AND agent_name IS NOT NULL",
            )?;
            stmt.query_map(params![run_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };

        // Get the last assistant message for each agent (via team session IDs)
        let mut agent_results = Vec::new();
        for agent_name in &agent_names {
            let session_id = format!("team-{}-{}", run_id, agent_name);
            let response: Option<String> = self
                .conn
                .query_row(
                    "SELECT content FROM messages
                     WHERE session_id = ?1 AND role = 'assistant'
                     ORDER BY created_at DESC LIMIT 1",
                    params![session_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(content) = response {
                let preview = truncate_chars(&content, 200);
                agent_results.push(AgentResultSummary {
                    agent_name: agent_name.clone(),
                    response_preview: preview,
                });
            }
        }

        // Cap at 5 agents to keep context concise
        agent_results.truncate(5);

        // Get task statuses
        let all_tasks: Vec<TaskStatusSummary> = {
            let mut stmt = self.conn.prepare(
                "SELECT agent_id, label, status, id FROM tasks
                 WHERE team_run_id = ?1",
            )?;
            stmt.query_map(params![run_id], |r| {
                Ok(TaskStatusSummary {
                    agent_id: r.get(0)?,
                    label: r.get(1)?,
                    status: r.get(2)?,
                    task_id: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?
        };

        let (pending_tasks, task_statuses): (Vec<_>, Vec<_>) = all_tasks
            .into_iter()
            .partition(|t| t.status == "pending" || t.status == "in_progress");

        // Get critic feedback (final iteration only)
        let critic_feedback: Option<String> = self
            .conn
            .query_row(
                "SELECT content FROM team_workspace
                 WHERE run_id = ?1 AND entry_type = 'critic'
                 ORDER BY iteration DESC, created_at DESC LIMIT 1",
                params![run_id],
                |r| r.get(0),
            )
            .optional()?
            .map(|c: String| truncate_chars(&c, 200));

        Ok(Some(TeamRunSummary {
            run,
            agent_results,
            task_statuses,
            pending_tasks,
            critic_feedback,
        }))
    }

    /// Convenience method: get the enriched summary for the most recent
    /// completed/failed/suspended run for a team. Returns None if no such run exists.
    pub fn get_last_completed_team_run_summary(
        &self,
        team_name: &str,
    ) -> Result<Option<TeamRunSummary>> {
        match self.get_last_completed_team_run(team_name)? {
            Some(prev) => self.get_team_run_summary(&prev.id),
            None => Ok(None),
        }
    }

    // ===== Team Workspace =====

    #[allow(clippy::too_many_arguments)]
    pub fn insert_team_workspace_entry(
        &self,
        run_id: &str,
        parent_id: Option<i64>,
        agent_name: Option<&str>,
        entry_type: &str,
        content: &str,
        iteration: u32,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO team_workspace (run_id, parent_id, agent_name, entry_type, content, iteration, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![run_id, parent_id, agent_name, entry_type, content, iteration, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn load_assignment_entry_ids(
        &self,
        run_id: &str,
        iteration: u32,
    ) -> Result<HashMap<String, i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_name, id FROM team_workspace
             WHERE run_id = ?1 AND iteration = ?2 AND entry_type = 'assignment'
             AND agent_name IS NOT NULL",
        )?;
        let rows: Vec<(String, i64)> = stmt
            .query_map(params![run_id, iteration], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows.into_iter().collect())
    }

    pub fn load_team_workspace(&self, run_id: &str) -> Result<Vec<TeamWorkspaceEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, parent_id, agent_name,
                     entry_type, content, iteration, created_at
              FROM team_workspace WHERE run_id = ?1
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![run_id], |r| {
                Ok(TeamWorkspaceEntry {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    agent_name: r.get(3)?,
                    entry_type: r.get(4)?,
                    content: r.get(5)?,
                    iteration: r.get::<_, u32>(6)?,
                    created_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Search / Layer 3 =====

    pub fn index_content(
        &self,
        agent_id: &str,
        source_type: &str,
        source_id: Option<i64>,
        content: &str,
    ) -> Result<i64> {
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, content FROM search_content
                  WHERE agent_id = ?1 AND source_type = ?2 AND source_id IS ?3",
                params![agent_id, source_type, source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((existing_id, old_content)) = existing {
            self.conn.execute(
                "UPDATE search_content SET content = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                params![content, existing_id],
            )?;
            // Update FTS index
            let _ = self.conn.execute(
                "INSERT INTO fts_search(fts_search, rowid, content) VALUES ('delete', ?1, ?2)",
                params![existing_id, old_content],
            );
            let _ = self.conn.execute(
                "INSERT INTO fts_search(rowid, content) VALUES (?1, ?2)",
                params![existing_id, content],
            );
            Ok(existing_id)
        } else {
            self.conn.execute(
                "INSERT INTO search_content (agent_id, source_type, source_id, content)
                  VALUES (?1, ?2, ?3, ?4)",
                params![agent_id, source_type, source_id, content],
            )?;
            let new_id = self.conn.last_insert_rowid();
            let _ = self.conn.execute(
                "INSERT INTO fts_search(rowid, content) VALUES (?1, ?2)",
                params![new_id, content],
            );
            Ok(new_id)
        }
    }

    pub fn index_embedding(&self, content_id: i64, embedding: &[f32]) -> Result<()> {
        let emb_json = serde_json::to_string(embedding)?;
        self.conn.execute(
            "UPDATE search_content SET embedding_json = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2",
            params![emb_json, content_id],
        )?;
        // Upsert into vec0 table
        let _ = self.conn.execute(
            "INSERT INTO vec_search(rowid, embedding) VALUES (?1, ?2)
             ON CONFLICT(rowid) DO UPDATE SET embedding = excluded.embedding",
            params![content_id, emb_json],
        );
        Ok(())
    }

    pub fn delete_search_content(
        &self,
        agent_id: &str,
        source_type: &str,
        source_id: i64,
    ) -> Result<()> {
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, content FROM search_content
                  WHERE agent_id = ?1 AND source_type = ?2 AND source_id = ?3",
                params![agent_id, source_type, source_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, content)) = existing {
            let _ = self.conn.execute(
                "INSERT INTO fts_search(fts_search, rowid, content) VALUES ('delete', ?1, ?2)",
                params![id, content],
            );
            let _ = self
                .conn
                .execute("DELETE FROM vec_search WHERE rowid = ?1", params![id]);
            self.conn
                .execute("DELETE FROM search_content WHERE id = ?1", params![id])?;
        }
        Ok(())
    }

    /// Returns search content rows that have no embedding yet (for backfill).
    pub fn get_unembedded_content(&self, agent_id: &str) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content FROM search_content
             WHERE agent_id = ?1 AND embedding_json IS NULL",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn count_search_content(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM search_content WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn fts_search(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT sc.id, sc.source_type, sc.source_id, sc.content, -rank AS score
              FROM fts_search
              JOIN search_content sc ON fts_search.rowid = sc.id
              WHERE fts_search MATCH ?1 AND sc.agent_id = ?2
                AND (?3 IS NULL OR sc.source_type = ?3)
              ORDER BY rank LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(
                params![query, agent_id, source_type_filter, limit as i64],
                |r| {
                    Ok(SearchResult {
                        id: r.get(0)?,
                        source_type: r.get(1)?,
                        source_id: r.get(2)?,
                        content: r.get(3)?,
                        score: r.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    fn vec_search_internal(
        &self,
        agent_id: &str,
        embedding: &[f32],
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let emb_json = serde_json::to_string(embedding)?;
        let mut stmt = self.conn.prepare(
            "SELECT sc.id, sc.source_type, sc.source_id, sc.content, knn.distance
              FROM (
                  SELECT rowid, distance FROM vec_search
                  WHERE embedding MATCH ?1
                  ORDER BY distance LIMIT ?4
              ) knn
              JOIN search_content sc ON knn.rowid = sc.id
              WHERE sc.agent_id = ?2 AND (?3 IS NULL OR sc.source_type = ?3)
              ORDER BY knn.distance",
        )?;
        let rows = stmt
            .query_map(
                params![emb_json, agent_id, source_type_filter, (limit * 3) as i64],
                |r| {
                    let dist: f64 = r.get(4)?;
                    Ok(SearchResult {
                        id: r.get(0)?,
                        source_type: r.get(1)?,
                        source_id: r.get(2)?,
                        content: r.get(3)?,
                        score: 1.0 / (1.0 + dist),
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn hybrid_search(
        &self,
        agent_id: &str,
        fts_query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let fts = self
            .fts_search(agent_id, fts_query, limit * 2, source_type_filter)
            .unwrap_or_default();
        let vec_results = embedding
            .and_then(|e| {
                self.vec_search_internal(agent_id, e, limit * 2, source_type_filter)
                    .ok()
            })
            .unwrap_or_default();

        if vec_results.is_empty() {
            return Ok(fts.into_iter().take(limit).collect());
        }

        const K: f64 = 60.0;
        let mut scores: HashMap<i64, f64> = HashMap::new();
        for (rank, r) in fts.iter().enumerate() {
            *scores.entry(r.id).or_default() += 1.0 / (K + rank as f64 + 1.0);
        }
        for (rank, r) in vec_results.iter().enumerate() {
            *scores.entry(r.id).or_default() += 1.0 / (K + rank as f64 + 1.0);
        }
        let mut all: HashMap<i64, SearchResult> = HashMap::new();
        for r in fts.into_iter().chain(vec_results.into_iter()) {
            all.entry(r.id).or_insert(r);
        }
        let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(ranked
            .into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                all.remove(&id).map(|mut r| {
                    r.score = score;
                    r
                })
            })
            .collect())
    }

    // ===== Utilities =====

    pub fn schema_version(&self) -> Result<i64> {
        self.current_version()
    }

    pub fn db_size_bytes(&self) -> Result<u64> {
        let page_size: i64 = self.conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let page_count: i64 = self.conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        Ok((page_size * page_count) as u64)
    }

    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM;")?;
        Ok(())
    }

    // ===== Dashboard Queries (unscoped, cross-agent) =====

    /// Query the unified_timeline VIEW with optional filters and pagination.
    pub fn query_timeline(
        &self,
        filters: &TimelineFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<TimelineRow>> {
        let (where_clause, params) = filters.to_sql();
        let sql = format!(
            "SELECT trace_id, session_id, agent_id, event_type, event_subtype, summary, created_at \
             FROM unified_timeline {} ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            where_clause,
            params.len() + 1,
            params.len() + 2,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut all_params: Vec<rusqlite::types::Value> = params;
        all_params.push(rusqlite::types::Value::Integer(limit as i64));
        all_params.push(rusqlite::types::Value::Integer(offset as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params
            .iter()
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(&*param_refs, |r| {
                Ok(TimelineRow {
                    trace_id: r.get(0)?,
                    session_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    event_type: r.get(3)?,
                    event_subtype: r.get(4)?,
                    summary: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count total rows in unified_timeline matching filters.
    pub fn query_timeline_count(&self, filters: &TimelineFilters) -> Result<u64> {
        let (where_clause, params) = filters.to_sql();
        let sql = format!("SELECT COUNT(*) FROM unified_timeline {}", where_clause);
        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params
            .iter()
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();
        let count: i64 = stmt.query_row(&*param_refs, |r| r.get(0))?;
        Ok(count as u64)
    }

    /// Get all events for a specific trace_id from the unified_timeline VIEW.
    pub fn query_timeline_by_trace(&self, trace_id: &str) -> Result<Vec<TimelineRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT trace_id, session_id, agent_id, event_type, event_subtype, summary, created_at \
             FROM unified_timeline WHERE trace_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![trace_id], |r| {
                Ok(TimelineRow {
                    trace_id: r.get(0)?,
                    session_id: r.get(1)?,
                    agent_id: r.get(2)?,
                    event_type: r.get(3)?,
                    event_subtype: r.get(4)?,
                    summary: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get full messages for a specific trace_id (for rich trace detail rendering).
    pub fn get_messages_by_trace_id(&self, trace_id: &str) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.trace_id = ?1
              ORDER BY m.created_at ASC, m.id ASC",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![trace_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// List all agents with message count.
    pub fn list_agents_with_stats(&self) -> Result<Vec<AgentWithStats>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.id, a.name, a.home_dir, a.active, a.last_seen, a.created_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.agent_id = a.id AND m.role != 'summary') as msg_count
             FROM agents a ORDER BY a.name",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AgentWithStats {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    home_dir: r.get(2)?,
                    active: r.get(3)?,
                    last_seen: r.get(4)?,
                    created_at: r.get(5)?,
                    message_count: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get a single agent with stats by id or name.
    pub fn get_agent_with_stats(&self, agent_id: &str) -> Result<Option<AgentWithStats>> {
        self.conn
            .query_row(
                "SELECT a.id, a.name, a.home_dir, a.active, a.last_seen, a.created_at,
                        (SELECT COUNT(*) FROM messages m WHERE m.agent_id = a.id AND m.role != 'summary') as msg_count
                 FROM agents a WHERE a.id = ?1 OR a.name = ?1",
                params![agent_id],
                |r| {
                    Ok(AgentWithStats {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        home_dir: r.get(2)?,
                        active: r.get(3)?,
                        last_seen: r.get(4)?,
                        created_at: r.get(5)?,
                        message_count: r.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// List sessions with optional filters and pagination.
    #[allow(clippy::too_many_arguments)]
    pub fn list_sessions_paginated(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        session_id: Option<&str>,
        task_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionWithStats>> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        if let Some(aid) = agent_id {
            param_values.push(aid.to_string());
            conditions.push(format!("s.agent_id = ?{}", param_values.len()));
        }
        if let Some(ct) = channel_type {
            param_values.push(ct.to_string());
            conditions.push(format!("s.channel_type = ?{}", param_values.len()));
        }
        if let Some(sid) = session_id {
            let sanitized: String = sid.chars().filter(|c| *c != '%' && *c != '_').collect();
            if !sanitized.is_empty() {
                param_values.push(format!("{}%", sanitized));
                conditions.push(format!("s.id LIKE ?{}", param_values.len()));
            }
        }
        if let Some(tid) = task_id {
            param_values.push(tid.to_string());
            conditions.push(format!(
                "COALESCE(s.task_id, json_extract(s.metadata, '$.task_id')) = ?{}",
                param_values.len()
            ));
        }
        if let Some(f) = from {
            param_values.push(f.to_string());
            conditions.push(format!("s.started_at >= ?{}", param_values.len()));
        }
        if let Some(t) = to {
            param_values.push(t.to_string());
            conditions.push(format!("s.started_at <= ?{}", param_values.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT s.id, s.agent_id, s.channel_type, s.started_at, s.ended_at, s.metadata, s.task_id,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count
             FROM sessions s {} ORDER BY s.started_at DESC LIMIT ?{} OFFSET ?{}",
            where_clause,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            param_values.into_iter().map(|s| Box::new(s) as _).collect();
        all_params.push(Box::new(limit));
        all_params.push(Box::new(offset));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();

        let rows = stmt
            .query_map(&*param_refs, |r| {
                Ok(SessionWithStats {
                    id: r.get(0)?,
                    agent_id: r.get(1)?,
                    channel_type: r.get(2)?,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    metadata: r.get(5)?,
                    task_id: r.get(6)?,
                    message_count: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count sessions matching optional filters.
    pub fn count_sessions(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        session_id: Option<&str>,
        task_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<u64> {
        let mut conditions = Vec::new();
        let mut param_values: Vec<String> = Vec::new();

        if let Some(aid) = agent_id {
            param_values.push(aid.to_string());
            conditions.push(format!("agent_id = ?{}", param_values.len()));
        }
        if let Some(ct) = channel_type {
            param_values.push(ct.to_string());
            conditions.push(format!("channel_type = ?{}", param_values.len()));
        }
        if let Some(sid) = session_id {
            let sanitized: String = sid.chars().filter(|c| *c != '%' && *c != '_').collect();
            if !sanitized.is_empty() {
                param_values.push(format!("{}%", sanitized));
                conditions.push(format!("id LIKE ?{}", param_values.len()));
            }
        }
        if let Some(tid) = task_id {
            param_values.push(tid.to_string());
            conditions.push(format!(
                "COALESCE(task_id, json_extract(metadata, '$.task_id')) = ?{}",
                param_values.len()
            ));
        }
        if let Some(f) = from {
            param_values.push(f.to_string());
            conditions.push(format!("started_at >= ?{}", param_values.len()));
        }
        if let Some(t) = to {
            param_values.push(t.to_string());
            conditions.push(format!("started_at <= ?{}", param_values.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!("SELECT COUNT(*) FROM sessions {}", where_clause);
        let mut stmt = self.conn.prepare(&sql)?;
        let boxed: Vec<Box<dyn rusqlite::types::ToSql>> =
            param_values.into_iter().map(|s| Box::new(s) as _).collect();
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = boxed.iter().map(|p| &**p).collect();
        let count: i64 = stmt.query_row(&*param_refs, |r| r.get(0))?;
        Ok(count as u64)
    }

    /// Get all sessions linked to a task tree (the given task + all descendants).
    /// Returns sessions where `task_id` (or legacy `json_extract(metadata, '$.task_id')`)
    /// matches any task ID in the tree. Includes message count for display.
    pub fn get_sessions_for_task_tree(&self, root_task_id: &str) -> Result<Vec<TaskSessionRow>> {
        // Collect all task IDs in the tree: root + all descendants
        let mut task_ids = vec![root_task_id.to_string()];
        {
            let descendants = self.get_task_descendants(root_task_id)?;
            task_ids.extend(descendants.into_iter().map(|t| t.id));
        }

        let placeholders: String = (1..=task_ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT s.id, s.agent_id, s.channel_type, s.started_at, s.ended_at, s.task_id,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) as msg_count,
                    t.label as task_label
             FROM sessions s
             LEFT JOIN tasks t ON t.id = COALESCE(s.task_id, json_extract(s.metadata, '$.task_id'))
             WHERE COALESCE(s.task_id, json_extract(s.metadata, '$.task_id')) IN ({placeholders})
             ORDER BY s.started_at DESC"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<Box<dyn rusqlite::types::ToSql>> = task_ids
            .into_iter()
            .map(|s| Box::new(s) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();

        let rows = stmt
            .query_map(&*refs, |r| {
                Ok(TaskSessionRow {
                    id: r.get(0)?,
                    agent_id: r.get(1)?,
                    channel_type: r.get(2)?,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    task_id: r.get(5)?,
                    message_count: r.get(6)?,
                    task_label: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(rows)
    }

    /// Get a single session by id.
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, channel_type, started_at, ended_at, metadata, parent_session_id, task_id
                 FROM sessions WHERE id = ?1",
                params![session_id],
                |r| {
                    Ok(Session {
                        id: r.get(0)?,
                        agent_id: r.get(1)?,
                        channel_type: r.get(2)?,
                        started_at: r.get(3)?,
                        ended_at: r.get(4)?,
                        metadata: r.get(5)?,
                        parent_session_id: r.get(6)?,
                        task_id: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Get the most recent ended CLI session for an agent.
    ///
    /// Scoped to `channel_type = 'cli'`, excludes system sessions (`system-*`),
    /// delegate sessions (`delegate-*`), child sessions (non-NULL `parent_session_id`),
    /// and active sessions (`ended_at IS NULL`). Returns the most recently started
    /// ended session, or `None` if no matching session exists.
    pub fn get_last_cli_session_for_agent(&self, agent_id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, channel_type, started_at, ended_at, metadata, parent_session_id, task_id
                 FROM sessions
                 WHERE agent_id = ?1
                   AND channel_type = 'cli'
                   AND ended_at IS NOT NULL
                   AND id NOT LIKE 'system-%'
                   AND id NOT LIKE 'delegate-%'
                   AND parent_session_id IS NULL
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![agent_id],
                |r| {
                    Ok(Session {
                        id: r.get(0)?,
                        agent_id: r.get(1)?,
                        channel_type: r.get(2)?,
                        started_at: r.get(3)?,
                        ended_at: r.get(4)?,
                        metadata: r.get(5)?,
                        parent_session_id: r.get(6)?,
                        task_id: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Load messages for a session with pagination.
    pub fn load_session_messages_paginated(
        &self,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionMessage>> {
        let sql = format!(
            "SELECT {} FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.session_id = ?1
              ORDER BY m.created_at ASC, m.id ASC LIMIT ?2 OFFSET ?3",
            Self::SESSION_MESSAGE_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![session_id, limit as i64, offset as i64],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count messages in a session.
    pub fn count_session_messages(&self, session_id: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// List audit events for an agent with pagination and optional filters.
    pub fn list_audit_events_paginated(
        &self,
        agent_id: &str,
        tool_name: Option<&str>,
        target_key: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>> {
        let sql = format!(
            "SELECT {} FROM audit_events WHERE agent_id = ?1 AND (?2 IS NULL OR tool_name = ?2) AND (?3 IS NULL OR target_key = ?3) ORDER BY created_at DESC LIMIT ?4 OFFSET ?5",
            Self::AUDIT_EVENT_COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                params![agent_id, tool_name, target_key, limit as i64, offset as i64],
                Self::row_to_audit_event,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // ===== Dashboard: Paginated Task Listing =====

    /// Cross-agent task lookup — raw `&str`, for correlation and observability only.
    ///
    /// Does NOT enforce agent ownership. For agent-scoped tool paths, use
    /// [`crate::tools::AgentScopedTaskId`] + [`crate::tools::validate_task_exists`]
    /// instead — the newtype makes the ownership invariant compile-checked (mika#755).
    ///
    /// Current callers: `mika ask --task-id` correlation branch at
    /// `crates/mika-cli/src/commands/ask.rs`, dashboard task detail endpoint.
    pub fn get_task_unscoped(&self, id: &str) -> Result<Option<Task>> {
        let sql = format!("SELECT {} FROM tasks WHERE id = ?1", Self::TASK_COLUMNS);
        self.conn
            .query_row(&sql, params![id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Count tasks matching filters (dashboard).
    pub fn count_tasks_filtered(&self, filters: &TaskFilters) -> Result<u64> {
        let (where_clause, param_values) = Self::build_task_filter_sql(filters);
        let sql = format!("SELECT COUNT(*) FROM tasks {where_clause}");

        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        self.conn
            .query_row(&sql, params.as_slice(), |r| r.get::<_, i64>(0))
            .map(|c| c as u64)
            .map_err(Into::into)
    }

    /// List tasks matching filters with pagination (dashboard).
    pub fn list_tasks_paginated(
        &self,
        filters: &TaskFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Task>> {
        let (where_clause, param_values) = Self::build_task_filter_sql(filters);
        let sql = format!(
            "SELECT {} FROM tasks {where_clause} ORDER BY updated_at DESC LIMIT ?{} OFFSET ?{}",
            Self::TASK_COLUMNS,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
            .into_iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> { Box::new(v) })
            .collect();
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(param_refs.as_slice(), Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build WHERE clause and params for task filters.
    /// Push a comma-separated filter value as an `IN (?,?,...?)` clause.
    fn push_csv_in_clause(
        field: &str,
        csv: &str,
        conditions: &mut Vec<String>,
        params: &mut Vec<String>,
    ) {
        let values: Vec<&str> = csv.split(',').map(|s| s.trim()).take(20).collect();
        let placeholders: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", params.len() + i + 1))
            .collect();
        conditions.push(format!("{field} IN ({})", placeholders.join(",")));
        for v in values {
            params.push(v.to_string());
        }
    }

    fn build_task_filter_sql(filters: &TaskFilters) -> (String, Vec<String>) {
        let mut conditions = Vec::new();
        let mut params = Vec::new();

        if let Some(ref status) = filters.status {
            Self::push_csv_in_clause("status", status, &mut conditions, &mut params);
        }

        if let Some(ref trigger_type) = filters.trigger_type {
            Self::push_csv_in_clause("trigger_type", trigger_type, &mut conditions, &mut params);
        }

        if let Some(ref action_type) = filters.action_type {
            Self::push_csv_in_clause("action_type", action_type, &mut conditions, &mut params);
        }

        if let Some(ref agent_id) = filters.agent_id {
            params.push(agent_id.clone());
            conditions.push(format!("agent_id = ?{}", params.len()));
        }

        if let Some(ref filter) = filters.team_run_id_filter {
            match filter {
                TeamRunIdFilter::Null => conditions.push("team_run_id IS NULL".to_string()),
                TeamRunIdFilter::NotNull => conditions.push("team_run_id IS NOT NULL".to_string()),
                TeamRunIdFilter::Specific(id) => {
                    params.push(id.clone());
                    conditions.push(format!("team_run_id = ?{}", params.len()));
                }
            }
        }

        if let Some(ref source) = filters.source {
            params.push(source.clone());
            conditions.push(format!("source = ?{}", params.len()));
        }

        if let Some(ref from) = filters.from {
            params.push(from.clone());
            conditions.push(format!("created_at >= ?{}", params.len()));
        }

        if let Some(ref to) = filters.to {
            params.push(to.clone());
            conditions.push(format!("created_at <= ?{}", params.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        (where_clause, params)
    }

    // ===== Dashboard: Paginated Team Run Listing =====

    /// Count team runs matching filters (dashboard).
    pub fn count_team_runs_filtered(&self, filters: &TeamRunFilters) -> Result<u64> {
        let (where_clause, param_values) = Self::build_team_run_filter_sql(filters);
        let sql = format!(
            "SELECT COUNT(*) FROM team_runs r JOIN teams t ON r.team_id = t.id {where_clause}"
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = param_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        self.conn
            .query_row(&sql, params.as_slice(), |r| r.get::<_, i64>(0))
            .map(|c| c as u64)
            .map_err(Into::into)
    }

    /// List team runs matching filters with pagination (dashboard).
    pub fn list_team_runs_paginated(
        &self,
        filters: &TeamRunFilters,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<TeamRunRow>> {
        let (where_clause, param_values) = Self::build_team_run_filter_sql(filters);
        let sql = format!(
            "SELECT {} FROM team_runs r JOIN teams t ON r.team_id = t.id
             {where_clause} ORDER BY r.started_at DESC LIMIT ?{} OFFSET ?{}",
            Self::TEAM_RUN_COLUMNS,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
            .into_iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> { Box::new(v) })
            .collect();
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(param_refs.as_slice(), Self::row_to_team_run)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build WHERE clause and params for team run filters.
    fn build_team_run_filter_sql(filters: &TeamRunFilters) -> (String, Vec<String>) {
        let mut conditions = Vec::new();
        let mut params = Vec::new();

        if let Some(ref team_name) = filters.team_name {
            params.push(team_name.clone());
            conditions.push(format!("t.name = ?{}", params.len()));
        }

        if let Some(ref status) = filters.status {
            Self::push_csv_in_clause("r.status", status, &mut conditions, &mut params);
        }

        if let Some(ref from) = filters.from {
            params.push(from.to_string());
            conditions.push(format!("r.started_at >= ?{}", params.len()));
        }

        if let Some(ref to) = filters.to {
            params.push(to.to_string());
            conditions.push(format!("r.started_at <= ?{}", params.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        (where_clause, params)
    }

    // ===== Combined data+count queries (single DB round-trip) =====

    /// Query timeline data and count in a single closure (avoids TOCTOU race).
    pub fn query_timeline_with_count(
        &self,
        filters: &TimelineFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<TimelineRow>, u64)> {
        let count = self.query_timeline_count(filters)?;
        let data = self.query_timeline(filters, limit, offset)?;
        Ok((data, count))
    }

    /// List sessions and count in a single closure.
    #[allow(clippy::too_many_arguments)]
    pub fn list_sessions_paginated_with_count(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        session_id: Option<&str>,
        task_id: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionWithStats>, u64)> {
        let count = self.count_sessions(agent_id, channel_type, session_id, task_id, from, to)?;
        let data = self.list_sessions_paginated(
            agent_id,
            channel_type,
            session_id,
            task_id,
            from,
            to,
            limit,
            offset,
        )?;
        Ok((data, count))
    }

    /// Load session messages and count in a single closure.
    pub fn load_session_messages_paginated_with_count(
        &self,
        session_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionMessage>, u64)> {
        let count = self.count_session_messages(session_id)?;
        let data = self.load_session_messages_paginated(session_id, limit, offset)?;
        Ok((data, count))
    }

    /// List audit events and count in a single closure.
    pub fn list_audit_events_paginated_with_count(
        &self,
        agent_id: &str,
        tool_name: Option<&str>,
        target_key: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<AuditEvent>, u64)> {
        let count = self.count_audit_events(agent_id, tool_name, target_key)?;
        let data =
            self.list_audit_events_paginated(agent_id, tool_name, target_key, limit, offset)?;
        Ok((data, count))
    }

    /// List tasks and count in a single closure.
    pub fn list_tasks_paginated_with_count(
        &self,
        filters: &TaskFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<Task>, u64)> {
        let count = self.count_tasks_filtered(filters)?;
        let data = self.list_tasks_paginated(filters, limit, offset)?;
        Ok((data, count))
    }

    /// List team runs and count in a single closure.
    pub fn list_team_runs_paginated_with_count(
        &self,
        filters: &TeamRunFilters,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<TeamRunRow>, u64)> {
        let count = self.count_team_runs_filtered(filters)?;
        let data = self.list_team_runs_paginated(filters, limit, offset)?;
        Ok((data, count))
    }

    // ===== Dashboard: Dev Runs (tasks with dev-run sources) =====

    /// Update the metadata JSON on a manual (task) task.
    /// Only works on `trigger_type='manual'` tasks. Returns false if not found.
    pub fn update_task_metadata(&self, task_id: &str, metadata_json: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET metadata = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND trigger_type = 'manual'",
            params![metadata_json, task_id],
        )?;
        Ok(rows > 0)
    }

    /// Get a single dev run (task with a dev-run source) by ID — unscoped by agent_id.
    pub fn get_dev_run(&self, task_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND trigger_type = 'manual' AND source IN ('self_dev', 'github_issue')",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![task_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// List dev runs (tasks with dev-run sources) with pagination and count.
    pub fn list_dev_runs_paginated_with_count(
        &self,
        status: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<Task>, u64)> {
        let mut conditions = vec![
            "trigger_type = 'manual'".to_string(),
            "source IN ('self_dev', 'github_issue')".to_string(),
        ];
        let mut param_values: Vec<String> = Vec::new();

        if let Some(s) = status {
            param_values.push(s.to_string());
            conditions.push(format!("status = ?{}", param_values.len()));
        }
        if let Some(f) = from {
            param_values.push(f.to_string());
            conditions.push(format!("created_at >= ?{}", param_values.len()));
        }
        if let Some(t) = to {
            param_values.push(t.to_string());
            conditions.push(format!("created_at <= ?{}", param_values.len()));
        }

        let where_clause = format!("WHERE {}", conditions.join(" AND "));

        let count_sql = format!("SELECT COUNT(*) FROM tasks {}", where_clause);
        let data_sql = format!(
            "SELECT {} FROM tasks {} ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
            Self::TASK_COLUMNS,
            where_clause,
            param_values.len() + 1,
            param_values.len() + 2,
        );

        let mut count_stmt = self.conn.prepare(&count_sql)?;
        let boxed_count: Vec<Box<dyn rusqlite::types::ToSql>> = param_values
            .iter()
            .map(|s| Box::new(s.clone()) as _)
            .collect();
        let count_refs: Vec<&dyn rusqlite::types::ToSql> =
            boxed_count.iter().map(|p| &**p).collect();
        let count: u64 = count_stmt.query_row(&*count_refs, |r| r.get::<_, i64>(0))? as u64;

        let mut data_stmt = self.conn.prepare(&data_sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            param_values.into_iter().map(|s| Box::new(s) as _).collect();
        all_params.push(Box::new(limit as i64));
        all_params.push(Box::new(offset as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();
        let data: Vec<Task> = data_stmt
            .query_map(&*param_refs, Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;

        Ok((data, count))
    }

    // ── KG corpus queries (#778) ──────────────────────────────────────────

    /// Count `kg_chunks` rows for a given `docs_root_hash`.
    ///
    /// Used as a "has this corpus been ingested before?" proxy at agent startup.
    /// Returns 0 for a never-ingested corpus (drift WARN).
    ///
    /// **Write-order dependency:** this is a reliable proxy ONLY because the
    /// lexical ingestor writes `kg_chunks` atomically with (or before)
    /// `kg_extractions` in the composed-write transaction. If a future edit
    /// writes `kg_extractions` before `kg_chunks`, this proxy becomes stale.
    /// See `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`.
    pub fn count_chunks_for_docs_root_hash(&self, docs_root_hash: &str) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM kg_chunks WHERE docs_root_hash = ?1",
            params![docs_root_hash],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }
}

// ===== Utility Functions =====

/// Format an ISO 8601 timestamp as a human-readable UTC string: "YYYY-MM-DD HH:MM:SS".
pub fn format_ts(ts: &str) -> String {
    crate::timestamp::parse(ts)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|_| ts.to_string())
}

/// Return today's midnight UTC time for the given IANA timezone string.
/// Falls back to UTC if the timezone string is unrecognised.
pub fn today_midnight_utc(timezone: &str) -> chrono::DateTime<chrono::Utc> {
    let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let now_local = chrono::Utc::now().with_timezone(&tz);
    let midnight_local = now_local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default();
    tz.from_local_datetime(&midnight_local)
        .earliest()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
}

/// Truncate a string to `max_chars` characters, appending "..." if truncated.
pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

/// Format the age of a timestamp relative to `now` as a human-readable string.
///
/// Examples: "3h 22m", "5d", "45m", "2d 4h"
fn format_age(timestamp_str: &str, now: chrono::DateTime<Utc>) -> String {
    let ts = match timestamp::parse(timestamp_str) {
        Ok(t) => t,
        Err(_) => return "unknown".to_string(),
    };
    let secs = now.signed_duration_since(ts).num_seconds().max(0);
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;

    if days > 0 && hours > 0 {
        format!("{days}d {hours}h")
    } else if days > 0 {
        format!("{days}d")
    } else if hours > 0 && mins > 0 {
        format!("{hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        "< 1m".to_string()
    }
}

// ===== Agent Reset helpers =====

/// Per-table counts of rows deleted (or that would be deleted) by an agent reset.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ResetAgentCounts {
    pub sessions: u64,
    pub messages: u64,
    pub core_memory: u64,
    pub llm_calls: u64,
    pub tool_calls: u64,
    pub audit_events: u64,
    pub audit_event_summaries: u64,
    pub people: u64,
    pub commitments: u64,
    pub preferences: u64,
    pub events: u64,
    pub search_content: u64,
    pub tasks: u64,
    pub kg_subject_resolutions: u64,
    pub kg_resolutions_log: u64,
    pub agent_kg_corpora: u64,
    pub kg_invalidated_no_match: u64,
    pub skill_overrides: u64,
    pub operational_items: u64,
    pub heartbeat_sends: u64,
    pub reflection_runs: u64,
    pub customer_config: u64,
    pub failed_sends: u64,
    // Shared KG tables (only deleted if no other agent shares the corpus)
    pub kg_chunks: u64,
    pub kg_subject_entities: u64,
    pub kg_subject_relationships: u64,
    pub kg_chunk_subjects: u64,
    pub kg_chunk_subject_relationships: u64,
    pub kg_extractions: u64,
}

impl ResetAgentCounts {
    /// Total rows across all tables.
    pub fn total(&self) -> u64 {
        self.sessions
            + self.messages
            + self.core_memory
            + self.llm_calls
            + self.tool_calls
            + self.audit_events
            + self.audit_event_summaries
            + self.people
            + self.commitments
            + self.preferences
            + self.events
            + self.search_content
            + self.tasks
            + self.kg_subject_resolutions
            + self.kg_resolutions_log
            + self.agent_kg_corpora
            + self.kg_invalidated_no_match
            + self.skill_overrides
            + self.operational_items
            + self.heartbeat_sends
            + self.reflection_runs
            + self.customer_config
            + self.failed_sends
            + self.kg_chunks
            + self.kg_subject_entities
            + self.kg_subject_relationships
            + self.kg_chunk_subjects
            + self.kg_chunk_subject_relationships
            + self.kg_extractions
    }
}

impl Database {
    /// Count per-table rows that would be deleted by `reset_agent_state`.
    /// Used for `--dry-run` preview.
    pub fn count_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts> {
        // Verify agent exists
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )?;
        if !exists {
            anyhow::bail!("Agent '{agent_id}' not found in database");
        }

        let count = |table: &str, col: &str, val: &str| -> Result<u64> {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE {col} = ?1");
            let n = self
                .conn
                .query_row(&sql, params![val], |r| r.get::<_, i64>(0))?;
            Ok(n as u64)
        };

        let mut counts = ResetAgentCounts {
            sessions: count("sessions", "agent_id", agent_id)?,
            messages: count("messages", "agent_id", agent_id)?,
            core_memory: count("core_memory", "agent_id", agent_id)?,
            llm_calls: count("llm_calls", "agent_id", agent_id)?,
            tool_calls: count("tool_calls", "agent_id", agent_id)?,
            audit_events: count("audit_events", "agent_id", agent_id)?,
            audit_event_summaries: count("audit_event_summaries", "agent_id", agent_id)?,
            people: count("people", "agent_id", agent_id)?,
            commitments: count("commitments", "agent_id", agent_id)?,
            preferences: count("preferences", "agent_id", agent_id)?,
            events: count("events", "agent_id", agent_id)?,
            search_content: count("search_content", "agent_id", agent_id)?,
            tasks: count("tasks", "agent_id", agent_id)?,
            kg_subject_resolutions: count("kg_subject_resolutions", "agent_id", agent_id)?,
            kg_resolutions_log: count("kg_resolutions_log", "agent_id", agent_id)?,
            agent_kg_corpora: count("agent_kg_corpora", "agent_id", agent_id)?,
            kg_invalidated_no_match: count("kg_invalidated_no_match", "agent_id", agent_id)?,
            skill_overrides: count("skill_overrides", "agent_id", agent_id)?,
            operational_items: count("operational_items", "agent_id", agent_id)?,
            heartbeat_sends: count("heartbeat_sends", "agent_id", agent_id)?,
            reflection_runs: count("reflection_runs", "agent_id", agent_id)?,
            customer_config: count("customer_config", "agent_id", agent_id)?,
            failed_sends: count("failed_sends", "agent_id", agent_id)?,
            ..Default::default()
        };

        // Shared KG tables: count rows that would be deleted
        // (only if no other agent shares the same docs_root_hash)
        let corpora = self.list_agent_corpora(agent_id)?;
        for (hash, _path) in &corpora {
            let other_refs: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM agent_kg_corpora WHERE docs_root_hash = ?1 AND agent_id != ?2",
                params![hash, agent_id],
                |r| r.get(0),
            )?;
            if other_refs == 0 {
                // This agent is the sole owner — these would be deleted
                let shared_tables = [
                    ("kg_chunks", &mut counts.kg_chunks),
                    ("kg_subject_entities", &mut counts.kg_subject_entities),
                    (
                        "kg_subject_relationships",
                        &mut counts.kg_subject_relationships,
                    ),
                    ("kg_chunk_subjects", &mut counts.kg_chunk_subjects),
                    (
                        "kg_chunk_subject_relationships",
                        &mut counts.kg_chunk_subject_relationships,
                    ),
                    ("kg_extractions", &mut counts.kg_extractions),
                ];
                for (table, field) in shared_tables {
                    let sql = format!("SELECT COUNT(*) FROM {table} WHERE docs_root_hash = ?1");
                    let n: i64 = self.conn.query_row(&sql, params![hash], |r| r.get(0))?;
                    *field += n as u64;
                }
            }
        }

        Ok(counts)
    }

    /// Get active tasks for an agent (used by the active-task guard).
    /// Returns `(task_id, status)` pairs for tasks in active states.
    pub fn get_active_tasks_for_agent(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, status FROM tasks
             WHERE agent_id = ?1
               AND status IN ('pending', 'in_progress', 'recurring_active')",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete all per-agent state while preserving the agent row itself.
    ///
    /// Caller is responsible for the active-task guard and confirmation prompt.
    /// After this call, the agent is in a freshly-provisioned state: zero rows
    /// in all child tables, agent row preserved, `identity.toml` untouched.
    pub fn reset_agent_state(&self, agent_id: &str) -> Result<ResetAgentCounts> {
        // Verify agent exists
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )?;
        if !exists {
            anyhow::bail!("Agent '{agent_id}' not found in database");
        }

        let tx = self.conn.unchecked_transaction()?;
        let mut counts = ResetAgentCounts::default();

        // Helper: delete from a table by agent_id and record count
        macro_rules! delete_by_agent {
            ($table:expr, $field:ident) => {
                tx.execute(
                    &format!("DELETE FROM {} WHERE agent_id = ?1", $table),
                    params![agent_id],
                )?;
                counts.$field = tx.changes();
            };
        }

        // -- Category 1: Conversation --
        delete_by_agent!("messages", messages);
        delete_by_agent!("sessions", sessions);
        delete_by_agent!("llm_calls", llm_calls);
        delete_by_agent!("tool_calls", tool_calls);

        // -- Category 2: Memory --
        delete_by_agent!("core_memory", core_memory);
        delete_by_agent!("people", people);
        delete_by_agent!("commitments", commitments);
        delete_by_agent!("preferences", preferences);
        delete_by_agent!("events", events);
        delete_by_agent!("search_content", search_content);

        // -- Category 3: Audit --
        delete_by_agent!("audit_events", audit_events);
        delete_by_agent!("audit_event_summaries", audit_event_summaries);

        // -- Category 4: Tasks --
        delete_by_agent!("tasks", tasks);

        // -- Category 5: Operations --
        delete_by_agent!("heartbeat_sends", heartbeat_sends);
        delete_by_agent!("reflection_runs", reflection_runs);
        delete_by_agent!("customer_config", customer_config);
        delete_by_agent!("failed_sends", failed_sends);

        // -- Category 6: KG per-agent --
        delete_by_agent!("kg_subject_resolutions", kg_subject_resolutions);
        delete_by_agent!("kg_resolutions_log", kg_resolutions_log);
        delete_by_agent!("kg_invalidated_no_match", kg_invalidated_no_match);

        // -- Category 7: KG shared (conditional — only if no other agent shares the corpus) --
        // Query corpora BEFORE deleting agent_kg_corpora rows
        let corpora: Vec<(String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT docs_root_hash, docs_root_path FROM agent_kg_corpora WHERE agent_id = ?1",
            )?;
            stmt.query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };

        for (hash, _path) in &corpora {
            let other_refs: i64 = tx.query_row(
                "SELECT COUNT(*) FROM agent_kg_corpora WHERE docs_root_hash = ?1 AND agent_id != ?2",
                params![hash, agent_id],
                |r| r.get(0),
            )?;
            if other_refs == 0 {
                // Sole owner — delete shared-layer rows in FK-safe order
                let shared_tables = [
                    "kg_chunk_subject_relationships",
                    "kg_chunk_subjects",
                    "kg_subject_relationships",
                    "kg_subject_entities",
                    "kg_extractions",
                    "kg_chunks",
                ];
                for table in &shared_tables {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE docs_root_hash = ?1"),
                        params![hash],
                    )?;
                    let deleted = tx.changes();
                    match *table {
                        "kg_chunk_subject_relationships" => {
                            counts.kg_chunk_subject_relationships += deleted
                        }
                        "kg_chunk_subjects" => counts.kg_chunk_subjects += deleted,
                        "kg_subject_relationships" => counts.kg_subject_relationships += deleted,
                        "kg_subject_entities" => counts.kg_subject_entities += deleted,
                        "kg_extractions" => counts.kg_extractions += deleted,
                        "kg_chunks" => counts.kg_chunks += deleted,
                        _ => {}
                    }
                }
            }
        }

        // Now delete the agent_kg_corpora mapping rows
        delete_by_agent!("agent_kg_corpora", agent_kg_corpora);

        // -- Category 8: Skills --
        delete_by_agent!("skill_overrides", skill_overrides);

        // -- Category 9: Operational ledger --
        delete_by_agent!("operational_items", operational_items);

        tx.commit()?;

        // -- Category 10: Post-transaction FTS5 rebuild --
        // External-content FTS5 table needs explicit rebuild after base table changes.
        // If the rebuild fails, the data is already deleted (transaction committed).
        // Log a warning but return success — the FTS index will self-heal on next
        // startup when search_content is re-indexed.
        if let Err(e) = self
            .conn
            .execute("INSERT INTO fts_search(fts_search) VALUES('rebuild')", [])
        {
            tracing::warn!(error = %e, agent_id, "FTS5 rebuild failed after agent reset — index may be stale");
        }

        Ok(counts)
    }
}

// ===== KG CLI helpers =====

/// Counts of rows deleted by a KG purge operation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KgPurgeCounts {
    pub resolutions_deleted: u64,
    pub resolution_log_deleted: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_layer_deleted: Option<Vec<(String, u64)>>,
}

impl Database {
    /// Count rows in a KG table matching a key column.
    pub fn kg_count_rows(&self, table: &str, key_col: &str, key_val: &str) -> Result<u64> {
        // Allowlist tables and columns to prevent SQL injection
        let valid_tables = [
            "kg_chunks",
            "kg_subject_entities",
            "kg_subject_relationships",
            "kg_chunk_subjects",
            "kg_chunk_subject_relationships",
            "kg_extractions",
            "kg_subject_resolutions",
            "kg_resolutions_log",
            "kg_entities",
        ];
        let valid_cols = ["docs_root_hash", "agent_id"];

        if !valid_tables.contains(&table) {
            anyhow::bail!("Invalid KG table: {table}");
        }
        if !valid_cols.contains(&key_col) {
            anyhow::bail!("Invalid KG key column: {key_col}");
        }

        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {key_col} = ?1");
        let count = self
            .conn
            .query_row(&sql, params![key_val], |r| r.get::<_, i64>(0))?;
        Ok(count as u64)
    }

    /// Count resolved subject entities for an agent within a specific corpus.
    ///
    /// Joins `kg_subject_resolutions` through `kg_subject_entities` to scope
    /// the count to a single `docs_root_hash`. Used by `mika kg status` to
    /// display per-corpus resolution coverage for multi-corpus agents (#877).
    pub fn kg_count_resolved_for_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
    ) -> Result<u64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM kg_subject_resolutions sr \
             JOIN kg_subject_entities se ON sr.subject_entity_id = se.id \
             WHERE sr.agent_id = ?1 AND se.docs_root_hash = ?2",
            params![agent_id, docs_root_hash],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(count as u64)
    }

    /// Count resolver-actionable pending subjects for an agent within a corpus (#999).
    ///
    /// Mirrors `entity_resolver::count_pending_for_corpus`: only counts subject
    /// entities of the five resolver-actionable types (`skill`, `tool`, `agent`,
    /// `problem_type`, `concept`) that have no resolution log row OR whose
    /// source extraction trace_id diverges from the latest one.
    ///
    /// Subject-graph-only types (`pattern`, `failure_mode`, `solution_path`)
    /// are intentionally excluded — they have no canonical domain projection
    /// and the resolver never touches them. Showing them as "pending" misleads
    /// the operator into thinking there is actionable backlog when there is
    /// none.
    pub fn kg_count_pending_resolver_actionable_for_corpus(
        &self,
        agent_id: &str,
        docs_root_hash: &str,
    ) -> Result<u64> {
        let sql = "SELECT COUNT(*)
             FROM kg_subject_entities e
             LEFT JOIN kg_resolutions_log r
                 ON r.subject_entity_id = e.id AND r.agent_id = ?1
             WHERE e.docs_root_hash = ?2
               AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')
               AND (
                 r.id IS NULL
                 OR r.source_extraction_trace_id != (
                     SELECT cs.extraction_trace_id
                     FROM kg_chunk_subjects cs
                     WHERE cs.subject_entity_id = e.id
                     ORDER BY cs.created_at DESC LIMIT 1
                 )
               )";
        let count = self
            .conn
            .query_row(sql, params![agent_id, docs_root_hash], |r| {
                r.get::<_, i64>(0)
            })?;
        Ok(count as u64)
    }

    /// Get the most recent extraction timestamp for a docs_root_hash.
    pub fn kg_last_extraction(&self, docs_root_hash: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT MAX(created_at) FROM kg_extractions WHERE docs_root_hash = ?1",
                params![docs_root_hash],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(Into::into)
    }

    /// Detect drift: find distinct docs_root_hash values the agent's
    /// resolutions point at via the subject->chunk->corpus chain.
    pub fn kg_observed_hashes(&self, agent_id: &str) -> Result<Vec<String>> {
        let sql = r#"
            SELECT DISTINCT c.docs_root_hash
            FROM kg_subject_resolutions r
            JOIN kg_subject_entities se ON se.id = r.subject_entity_id
            JOIN kg_chunk_subjects cs ON cs.subject_entity_id = se.id
            JOIN kg_chunks c ON c.id = cs.chunk_id
            WHERE r.agent_id = ?1
        "#;
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![agent_id], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Transactional purge of an agent's KG state.
    ///
    /// **Caller MUST verify no other agent references the `docs_root_hash`
    /// before passing `force_delete_shared = true`.** The helper does not
    /// re-verify — this flag is a pre-authorization, not operator intent.
    pub fn purge_kg_for_agent(
        &self,
        agent_id: &str,
        force_delete_shared: bool,
        docs_root_hash: Option<&str>,
    ) -> Result<KgPurgeCounts> {
        let tx = self.conn.unchecked_transaction()?;

        // Step 1: Delete per-agent resolutions
        tx.execute(
            "DELETE FROM kg_subject_resolutions WHERE agent_id = ?1",
            params![agent_id],
        )?;
        let resolutions_deleted = tx.changes();

        // Step 2: Delete per-agent resolution log
        tx.execute(
            "DELETE FROM kg_resolutions_log WHERE agent_id = ?1",
            params![agent_id],
        )?;
        let resolution_log_deleted = tx.changes();

        // Step 3: Optionally delete shared-layer rows
        let shared_layer_deleted = if force_delete_shared {
            if let Some(hash) = docs_root_hash {
                // Delete in FK-safe order (children before parents)
                let tables = [
                    "kg_chunk_subject_relationships",
                    "kg_chunk_subjects",
                    "kg_subject_relationships",
                    "kg_subject_entities",
                    "kg_extractions",
                    "kg_chunks",
                ];
                let mut deleted = Vec::new();
                for table in &tables {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE docs_root_hash = ?1"),
                        params![hash],
                    )?;
                    let count = tx.changes();
                    if count > 0 {
                        deleted.push((table.to_string(), count));
                    }
                }
                Some(deleted)
            } else {
                None
            }
        } else {
            None
        };

        tx.commit()?;

        Ok(KgPurgeCounts {
            resolutions_deleted,
            resolution_log_deleted,
            shared_layer_deleted,
        })
    }

    /// Run orphan FK check for KG validate.
    /// Returns (count, example_id) of orphan rows.
    pub fn kg_check_orphan_fk(
        &self,
        source_table: &str,
        fk_col: &str,
        target_table: &str,
    ) -> Result<(u64, Option<i64>)> {
        // Allowlist for safety
        let valid_tables = [
            "kg_chunks",
            "kg_subject_entities",
            "kg_subject_relationships",
            "kg_chunk_subjects",
            "kg_chunk_subject_relationships",
            "kg_subject_resolutions",
            "kg_resolutions_log",
            "kg_entities",
            "kg_extractions",
        ];
        let valid_cols = [
            "chunk_id",
            "subject_entity_id",
            "subject_relationship_id",
            "domain_entity_id",
        ];

        if !valid_tables.contains(&source_table) || !valid_tables.contains(&target_table) {
            anyhow::bail!("Invalid KG table in orphan check");
        }
        if !valid_cols.contains(&fk_col) {
            anyhow::bail!("Invalid KG FK column: {fk_col}");
        }

        let sql = format!(
            "SELECT COUNT(*), MIN(id) FROM {source_table} WHERE {fk_col} NOT IN (SELECT id FROM {target_table})"
        );
        let (count, example) = self.conn.query_row(&sql, [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
        })?;
        Ok((count as u64, example))
    }

    /// Rolling-window outcome stats from `kg_resolutions_log` (#1077).
    ///
    /// When `docs_root_hash` is `Some`, scopes to a single corpus via JOIN
    /// through `kg_subject_entities`. When `None`, returns agent-wide stats
    /// (no JOIN needed).
    ///
    /// The `attempted` denominator excludes structural skips and errors per
    /// design decision D6 — only outcomes representing genuine resolution
    /// attempts are counted.
    pub fn kg_resolution_outcome_stats(
        &self,
        agent_id: &str,
        docs_root_hash: Option<&str>,
        window_days: u32,
    ) -> Result<kg_schema::ResolutionOutcomeStats> {
        let window_param = format!("-{window_days} days");

        if let Some(hash) = docs_root_hash {
            // Per-corpus: JOIN through kg_subject_entities for docs_root_hash filter.
            // COALESCE guards against NULL when no rows match (SUM returns NULL on empty set).
            let sql = r#"
                SELECT
                    COUNT(*) as total,
                    COALESCE(SUM(CASE WHEN rl.outcome IN ('matched_exact','matched_llm','matched_llm_db_fallback','no_match','no_candidate_of_type') THEN 1 ELSE 0 END), 0) as attempted,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'no_match' THEN 1 ELSE 0 END), 0) as no_match,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'no_candidate_of_type' THEN 1 ELSE 0 END), 0) as no_candidate_of_type,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'matched_exact' THEN 1 ELSE 0 END), 0) as matched_exact,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'matched_llm' THEN 1 ELSE 0 END), 0) as matched_llm,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'matched_llm_db_fallback' THEN 1 ELSE 0 END), 0) as matched_llm_db_fallback,
                    COALESCE(SUM(CASE WHEN rl.outcome IN ('skipped_no_llm','skipped_discovered_type','skipped_discovered_subject') THEN 1 ELSE 0 END), 0) as skipped,
                    COALESCE(SUM(CASE WHEN rl.outcome = 'error' THEN 1 ELSE 0 END), 0) as errors
                FROM kg_resolutions_log rl
                JOIN kg_subject_entities se ON se.id = rl.subject_entity_id
                WHERE rl.agent_id = ?1
                  AND se.docs_root_hash = ?2
                  AND rl.resolved_at >= datetime('now', ?3)
            "#;
            self.conn
                .query_row(sql, params![agent_id, hash, window_param], |row| {
                    Ok(kg_schema::ResolutionOutcomeStats {
                        total: row.get::<_, i64>(0)? as u64,
                        attempted: row.get::<_, i64>(1)? as u64,
                        no_match: row.get::<_, i64>(2)? as u64,
                        no_candidate_of_type: row.get::<_, i64>(3)? as u64,
                        matched_exact: row.get::<_, i64>(4)? as u64,
                        matched_llm: row.get::<_, i64>(5)? as u64,
                        matched_llm_db_fallback: row.get::<_, i64>(6)? as u64,
                        skipped: row.get::<_, i64>(7)? as u64,
                        errors: row.get::<_, i64>(8)? as u64,
                    })
                })
                .map_err(Into::into)
        } else {
            // Agent-wide: no JOIN needed.
            // COALESCE guards against NULL when no rows match (SUM returns NULL on empty set).
            let sql = r#"
                SELECT
                    COUNT(*) as total,
                    COALESCE(SUM(CASE WHEN outcome IN ('matched_exact','matched_llm','matched_llm_db_fallback','no_match','no_candidate_of_type') THEN 1 ELSE 0 END), 0) as attempted,
                    COALESCE(SUM(CASE WHEN outcome = 'no_match' THEN 1 ELSE 0 END), 0) as no_match,
                    COALESCE(SUM(CASE WHEN outcome = 'no_candidate_of_type' THEN 1 ELSE 0 END), 0) as no_candidate_of_type,
                    COALESCE(SUM(CASE WHEN outcome = 'matched_exact' THEN 1 ELSE 0 END), 0) as matched_exact,
                    COALESCE(SUM(CASE WHEN outcome = 'matched_llm' THEN 1 ELSE 0 END), 0) as matched_llm,
                    COALESCE(SUM(CASE WHEN outcome = 'matched_llm_db_fallback' THEN 1 ELSE 0 END), 0) as matched_llm_db_fallback,
                    COALESCE(SUM(CASE WHEN outcome IN ('skipped_no_llm','skipped_discovered_type','skipped_discovered_subject') THEN 1 ELSE 0 END), 0) as skipped,
                    COALESCE(SUM(CASE WHEN outcome = 'error' THEN 1 ELSE 0 END), 0) as errors
                FROM kg_resolutions_log
                WHERE agent_id = ?1
                  AND resolved_at >= datetime('now', ?2)
            "#;
            self.conn
                .query_row(sql, params![agent_id, window_param], |row| {
                    Ok(kg_schema::ResolutionOutcomeStats {
                        total: row.get::<_, i64>(0)? as u64,
                        attempted: row.get::<_, i64>(1)? as u64,
                        no_match: row.get::<_, i64>(2)? as u64,
                        no_candidate_of_type: row.get::<_, i64>(3)? as u64,
                        matched_exact: row.get::<_, i64>(4)? as u64,
                        matched_llm: row.get::<_, i64>(5)? as u64,
                        matched_llm_db_fallback: row.get::<_, i64>(6)? as u64,
                        skipped: row.get::<_, i64>(7)? as u64,
                        errors: row.get::<_, i64>(8)? as u64,
                    })
                })
                .map_err(Into::into)
        }
    }

    /// Count kg_chunks rows with NULL source_doc_hash.
    pub fn kg_count_null_hash(&self) -> Result<u64> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM kg_chunks WHERE source_doc_hash IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(count as u64)
    }
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn db_with_session() -> (Database, String) {
        let db = db();
        let session_id = "test-session".to_string();
        db.create_session(&session_id, "mika", "cli").unwrap();
        (db, session_id)
    }

    #[test]
    fn test_open_in_memory_creates_schema() {
        let db = db();
        let version = db.schema_version().unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_default_agent_registered() {
        let db = db();
        let agents = db.list_agents_db().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "mika");
    }

    #[test]
    fn test_v3_tables_exist() {
        let db = db();
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
                  AND name IN ('agents','teams','tasks','sessions','messages','core_memory',
                               'people','commitments','preferences','events',
                               'audit_events','audit_event_summaries','search_content',
                               'team_runs','team_workspace','heartbeat_sends',
                               'reflection_runs','customer_config','failed_sends')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 19);
    }

    #[test]
    fn test_no_reminders_table() {
        let db = db();
        let exists: bool = db
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='reminders'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!exists);
    }

    #[test]
    fn test_save_and_load_messages() {
        let (db, sid) = db_with_session();
        db.save_message("mika", &sid, "user", "Hello!", None)
            .unwrap();
        db.save_message("mika", &sid, "assistant", "Hi!", None)
            .unwrap();
        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[0].session_id, sid);
    }

    #[test]
    fn test_load_recent_messages_limit() {
        let (db, sid) = db_with_session();
        for i in 0..5 {
            db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
                .unwrap();
        }
        let msgs = db.load_recent_messages("mika", 3).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].content, "msg 4");
    }

    #[test]
    fn test_load_messages_after() {
        let (db, sid) = db_with_session();
        db.save_message("mika", &sid, "user", "msg 1", None)
            .unwrap();
        db.save_message("mika", &sid, "user", "msg 2", None)
            .unwrap();
        db.save_message("mika", &sid, "user", "msg 3", None)
            .unwrap();

        let all = db.load_messages_after("mika", 0).unwrap();
        assert_eq!(all.len(), 3);

        let first_id = all[0].id;
        let after = db.load_messages_after("mika", first_id).unwrap();
        assert_eq!(after.len(), 2);
        for msg in &after {
            assert!(msg.id > first_id);
        }
    }

    #[test]
    fn test_session_channel_type_via_join() {
        let db = db();
        db.create_session("tg-session", "mika", "telegram").unwrap();
        db.create_session("cli-session", "mika", "cli").unwrap();
        db.save_message("mika", "tg-session", "user", "telegram msg", None)
            .unwrap();
        db.save_message("mika", "cli-session", "user", "cli msg", None)
            .unwrap();

        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 2);
        // Channel type comes from session JOIN
        assert!(msgs.iter().any(|m| m.channel_type == "telegram"));
        assert!(msgs.iter().any(|m| m.channel_type == "cli"));
    }

    #[test]
    fn test_load_recent_messages_excludes_team() {
        let db = db();
        db.create_session("team-session", "mika", "team").unwrap();
        db.create_session("cli-session", "mika", "cli").unwrap();
        db.save_message("mika", "team-session", "assistant", "team msg", None)
            .unwrap();
        db.save_message("mika", "cli-session", "user", "cli msg", None)
            .unwrap();

        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "cli msg");
        assert_eq!(msgs[0].channel_type, "cli");
    }

    #[test]
    fn test_load_recent_messages_includes_telegram() {
        let db = db();
        db.create_session("tg-session", "mika", "telegram").unwrap();
        db.save_message("mika", "tg-session", "user", "hello from telegram", None)
            .unwrap();

        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello from telegram");
        assert_eq!(msgs[0].channel_type, "telegram");
    }

    #[test]
    fn test_load_recent_messages_mixed_channels() {
        let db = db();
        db.create_session("cli-session", "mika", "cli").unwrap();
        db.create_session("tg-session", "mika", "telegram").unwrap();
        db.create_session("team-session", "mika", "team").unwrap();

        db.save_message("mika", "cli-session", "user", "cli 1", None)
            .unwrap();
        db.save_message("mika", "team-session", "assistant", "team 1", None)
            .unwrap();
        db.save_message("mika", "tg-session", "user", "tg 1", None)
            .unwrap();
        db.save_message("mika", "team-session", "assistant", "team 2", None)
            .unwrap();
        db.save_message("mika", "cli-session", "user", "cli 2", None)
            .unwrap();

        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 3);
        // Team messages excluded, cli and telegram present in chronological order
        assert!(msgs.iter().all(|m| m.channel_type != "team"));
        assert!(msgs.iter().any(|m| m.channel_type == "cli"));
        assert!(msgs.iter().any(|m| m.channel_type == "telegram"));
        // Chronological order (reversed from DESC query)
        assert_eq!(msgs[0].content, "cli 1");
        assert_eq!(msgs[1].content, "tg 1");
        assert_eq!(msgs[2].content, "cli 2");
    }

    #[test]
    fn test_get_messages_since() {
        let (db, sid) = db_with_session();
        db.conn
            .execute(
                "INSERT INTO messages (session_id, agent_id, role, content, created_at)
                  VALUES (?1, 'mika', 'user', 'old', '2020-01-01T00:00:00Z')",
                params![sid],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO messages (session_id, agent_id, role, content, created_at)
                  VALUES (?1, 'mika', 'user', 'new', '2025-01-01T00:00:00Z')",
                params![sid],
            )
            .unwrap();
        let msgs = db
            .get_messages_since("mika", "2024-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "new");
    }

    #[test]
    fn test_get_messages_by_trace_id() {
        let (db, sid) = db_with_session();
        let trace = "aaaa0000bbbb1111cccc2222dddd3333";
        db.save_message_with_metadata("mika", &sid, "user", "traced msg", None, Some(trace), false)
            .unwrap();
        db.save_message("mika", &sid, "assistant", "no trace", None)
            .unwrap();

        let msgs = db.get_messages_by_trace_id(trace).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "traced msg");
        assert_eq!(msgs[0].trace_id.as_deref(), Some(trace));

        let empty = db.get_messages_by_trace_id("nonexistent").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_last_user_message_time() {
        let (db, sid) = db_with_session();
        assert!(db.last_user_message_time("mika").unwrap().is_none());
        db.save_message("mika", &sid, "user", "hello", None)
            .unwrap();
        let ts = db.last_user_message_time("mika").unwrap();
        assert!(ts.is_some());
        assert!(!ts.unwrap().is_empty());
    }

    #[test]
    fn test_replace_with_summary() {
        let (mut db, sid) = db_with_session();
        let id1 = db.save_message("mika", &sid, "user", "msg1", None).unwrap();
        db.save_message("mika", &sid, "assistant", "reply1", None)
            .unwrap();
        db.replace_with_summary("mika", "Summary text", id1)
            .unwrap();
        let summary = db.load_conversation_summary("mika").unwrap().unwrap();
        assert_eq!(summary.role, "summary");
        assert_eq!(summary.content, "Summary text");
        let count = db.count_messages("mika").unwrap();
        assert_eq!(count, 1); // only the second message remains + summary excluded
    }

    #[test]
    fn test_count_messages_excludes_summary() {
        let (db, sid) = db_with_session();
        db.save_message("mika", &sid, "user", "a", None).unwrap();
        db.save_message("mika", &sid, "assistant", "b", None)
            .unwrap();
        let sys_session = db.get_or_create_system_session("mika").unwrap();
        db.conn
            .execute(
                "INSERT INTO messages (session_id, agent_id, role, content)
                  VALUES (?1, 'mika', 'summary', 'S')",
                params![sys_session],
            )
            .unwrap();
        assert_eq!(db.count_messages("mika").unwrap(), 2);
    }

    #[test]
    fn test_core_memory_set_and_get() {
        let db = db();
        db.set_core_memory("mika", "user_summary", "Alice").unwrap();
        let entry = db.get_core_memory("mika", "user_summary").unwrap().unwrap();
        assert_eq!(entry.value, "Alice");
    }

    #[test]
    fn test_seed_core_memory() {
        let db = db();
        db.seed_core_memory("mika", None).unwrap();
        let entries = db.get_all_core_memory("mika").unwrap();
        assert_eq!(entries.len(), CORE_MEMORY_SECTIONS.len());
    }

    #[test]
    fn test_seed_core_memory_custom_user_summary() {
        let db = db();
        db.seed_core_memory("mika", Some("Bob")).unwrap();
        let entry = db.get_core_memory("mika", "user_summary").unwrap().unwrap();
        assert_eq!(entry.value, "Bob");
    }

    #[test]
    fn test_upsert_and_get_person() {
        let db = db();
        db.upsert_person("mika", "Alice", Some("colleague"), Some("Works at Acme"))
            .unwrap();
        let p = db.get_person("mika", "Alice").unwrap().unwrap();
        assert_eq!(p.canonical_name, "Alice");
        assert_eq!(p.relationship.unwrap(), "colleague");
    }

    #[test]
    fn test_add_commitment_and_list() {
        let db = db();
        db.add_commitment("mika", "Write report", Some("2026-04-01"), None)
            .unwrap();
        let items = db.list_commitments("mika", "pending").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].description, "Write report");
    }

    #[test]
    fn test_update_commitment_status() {
        let db = db();
        let id = db.add_commitment("mika", "Task A", None, None).unwrap();
        assert!(
            db.update_commitment_status("mika", id, "completed")
                .unwrap()
        );
        let status = db.get_commitment_status("mika", id).unwrap().unwrap();
        assert_eq!(status, "completed");
    }

    #[test]
    fn test_set_and_get_preference() {
        let db = db();
        db.set_preference("mika", "timezone", "UTC").unwrap();
        let v = db.get_preference("mika", "timezone").unwrap().unwrap();
        assert_eq!(v, "UTC");
    }

    #[test]
    fn test_add_and_list_events() {
        let db = db();
        db.add_event("mika", "Team meeting", Some("2026-04-15"), None)
            .unwrap();
        let evts = db.list_events("mika").unwrap();
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].description, "Team meeting");
    }

    #[test]
    fn test_record_and_count_heartbeat_sends() {
        let db = db();
        db.record_heartbeat_send("mika").unwrap();
        db.record_heartbeat_send("mika").unwrap();
        let count = db.count_heartbeat_sends_last_hour("mika").unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_prune_old_heartbeat_sends() {
        let db = db();
        // Insert old record manually
        db.conn
            .execute(
                "INSERT INTO heartbeat_sends (agent_id, sent_at) VALUES ('mika', 1000)",
                [],
            )
            .unwrap();
        db.record_heartbeat_send("mika").unwrap();
        db.prune_old_heartbeat_sends("mika", 30).unwrap();
        let count = db.count_heartbeat_sends_last_hour("mika").unwrap();
        // Old entry gone, recent one stays
        assert!(count <= 1);
    }

    #[test]
    fn test_record_reflection_run() {
        let db = db();
        db.record_reflection_run("mika", "completed", 3, Some("Updated 3 keys"))
            .unwrap();
    }

    #[test]
    fn test_save_and_get_failed_send() {
        let db = db();
        let id = db
            .save_failed_send("mika", "Failed message", Some("req-1"))
            .unwrap();
        let pending = db.get_pending_failed_sends("mika", 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "Failed message");
        db.delete_failed_send("mika", id).unwrap();
        let pending = db.get_pending_failed_sends("mika", 10).unwrap();
        assert!(pending.is_empty());
    }

    #[test]
    fn test_customer_config() {
        let db = db();
        db.set_customer_config("mika", "timezone", "America/New_York")
            .unwrap();
        let v = db.get_customer_config("mika", "timezone").unwrap().unwrap();
        assert_eq!(v, "America/New_York");
        let all = db.list_customer_config("mika").unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_team_runs_insert_and_load() {
        let db = db();
        db.insert_team_run(
            "run-001",
            "engineering",
            "Build feature X",
            3,
            "2023-11-14T22:13:20Z",
            None,
        )
        .unwrap();
        db.update_team_run(
            "run-001",
            "completed",
            None,
            1,
            Some("Done!"),
            Some("2023-11-14T22:30:00Z"),
        )
        .unwrap();
        let runs = db.load_team_runs("engineering", 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].goal, "Build feature X");
        assert_eq!(runs[0].status, "completed");
    }

    #[test]
    fn test_team_workspace_insert_and_load() {
        let db = db();
        db.insert_team_run("run-001", "eng", "Goal", 3, "2020-01-01T00:00:00Z", None)
            .unwrap();
        let id = db
            .insert_team_workspace_entry("run-001", None, Some("mika"), "plan", "Do this", 1, None)
            .unwrap();
        let entries = db.load_team_workspace("run-001").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].entry_type, "plan");
    }

    #[test]
    fn test_create_and_get_task() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Send reminder".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: r#"{"message":"hello"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(t.label, "Send reminder");
        assert_eq!(t.trigger_type, "time");
        assert_eq!(t.status, "pending");
    }

    #[test]
    fn test_cancel_task() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Cancelable".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        assert!(db.cancel_task(&id, "mika").unwrap());
        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(t.status, "cancelled");
        // Cancelling again returns false
        assert!(!db.cancel_task(&id, "mika").unwrap());
    }

    #[test]
    fn test_resolve_task_id_by_prefix_unique_match() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Prefix test".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        // Use the first 12 chars as prefix (same as tasks list display)
        let prefix = &id[..12];
        let matches = db.resolve_task_id_by_prefix(prefix, "mika").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], id);
    }

    #[test]
    fn test_resolve_task_id_by_prefix_exact_match() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Full UUID test".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        // Full UUID also works via prefix match
        let matches = db.resolve_task_id_by_prefix(&id, "mika").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], id);
    }

    #[test]
    fn test_resolve_task_id_by_prefix_no_match() {
        let db = db();
        let matches = db
            .resolve_task_id_by_prefix("nonexistent-prefix", "mika")
            .unwrap();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_resolve_task_id_by_prefix_scoped_to_agent() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();
        let task = NewTask {
            agent_id: "agent-a".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "Agent A task".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        let prefix = &id[..12];
        // Same prefix, different agent — should not match
        let matches = db.resolve_task_id_by_prefix(prefix, "agent-b").unwrap();
        assert!(matches.is_empty());
        // Correct agent — should match
        let matches = db.resolve_task_id_by_prefix(prefix, "agent-a").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], id);
    }

    #[test]
    fn test_register_agent() {
        let db = db();
        db.register_agent("agent2", "Secondary Agent", "/home/agent2")
            .unwrap();
        let agents = db.list_agents_db().unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[test]
    fn test_register_agent_upserts_home_dir() {
        let db = db();
        // "mika" was pre-registered with empty home_dir by schema init
        let agents = db.list_agents_db().unwrap();
        let mika = agents.iter().find(|a| a.id == "mika").unwrap();
        assert_eq!(mika.home_dir, "");

        // Re-register with a real path — should update
        db.register_agent("mika", "Mika", "/home/user/.mika/agents/mika")
            .unwrap();
        let agents = db.list_agents_db().unwrap();
        let mika = agents.iter().find(|a| a.id == "mika").unwrap();
        assert_eq!(mika.home_dir, "/home/user/.mika/agents/mika");

        // Re-register with empty string — should NOT overwrite home_dir
        db.register_agent("mika", "Mika", "").unwrap();
        let agents = db.list_agents_db().unwrap();
        let mika = agents.iter().find(|a| a.id == "mika").unwrap();
        assert_eq!(mika.home_dir, "/home/user/.mika/agents/mika");
    }

    #[test]
    fn test_register_agent_upserts_name() {
        let db = db();
        // "mika" was pre-registered by schema init with name = "Mika"
        let name = db.get_agent_display_name("mika");
        assert_eq!(name, "Mika");

        // Re-register with a proper display name — should update
        db.register_agent("mika", "Mika ✨", "/home/user/.mika/agents/mika")
            .unwrap();
        let name = db.get_agent_display_name("mika");
        assert_eq!(name, "Mika ✨");

        // Re-register with a different name — should update again
        db.register_agent("mika", "My Assistant", "").unwrap();
        let name = db.get_agent_display_name("mika");
        assert_eq!(name, "My Assistant");
    }

    #[test]
    fn test_register_team() {
        let db = db();
        db.register_team("eng", "Engineering", "/teams/eng.toml")
            .unwrap();
        let teams = db.list_teams_db().unwrap();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].id, "eng");
    }

    #[test]
    fn test_fts_search_agent_isolation() {
        let db = db();
        db.register_agent("other", "Other", "").unwrap();
        db.index_content("mika", "person", Some(1), "Alice in Wonderland")
            .unwrap();
        db.index_content("other", "person", Some(1), "Bob the Builder")
            .unwrap();
        let results = db.fts_search("mika", "Alice", 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Alice"));
    }

    #[test]
    fn test_get_unembedded_content() {
        let db = db();
        // Index two content rows — both start with embedding_json = NULL
        let id1 = db
            .index_content("mika", "person", Some(1), "Alice in Wonderland")
            .unwrap();
        let id2 = db
            .index_content("mika", "person", Some(2), "Bob the Builder")
            .unwrap();

        // Both should be returned as unembedded
        let unembedded = db.get_unembedded_content("mika").unwrap();
        assert_eq!(unembedded.len(), 2);

        // Store an embedding for the first one
        db.index_embedding(id1, &[0.1; 512]).unwrap();

        // Now only the second should be unembedded
        let unembedded = db.get_unembedded_content("mika").unwrap();
        assert_eq!(unembedded.len(), 1);
        assert_eq!(unembedded[0].0, id2);
        assert_eq!(unembedded[0].1, "Bob the Builder");

        // Store embedding for the second — none left
        db.index_embedding(id2, &[0.2; 512]).unwrap();
        let unembedded = db.get_unembedded_content("mika").unwrap();
        assert!(unembedded.is_empty());
    }

    #[test]
    fn test_get_unembedded_content_agent_isolation() {
        let db = db();
        db.register_agent("other", "Other", "").unwrap();
        db.index_content("mika", "person", Some(1), "Alice")
            .unwrap();
        db.index_content("other", "person", Some(1), "Bob").unwrap();

        // Each agent sees only its own unembedded content
        let mika_unembedded = db.get_unembedded_content("mika").unwrap();
        assert_eq!(mika_unembedded.len(), 1);
        assert_eq!(mika_unembedded[0].1, "Alice");

        let other_unembedded = db.get_unembedded_content("other").unwrap();
        assert_eq!(other_unembedded.len(), 1);
        assert_eq!(other_unembedded[0].1, "Bob");
    }

    #[test]
    fn test_get_all_facts_for_indexing() {
        let db = db();
        db.upsert_person("mika", "Alice", Some("friend"), None)
            .unwrap();
        db.add_commitment("mika", "Write docs", None, None).unwrap();
        db.set_preference("mika", "theme", "dark").unwrap();
        let facts = db.get_all_facts_for_indexing("mika").unwrap();
        assert_eq!(facts.len(), 3);
        let types: Vec<&str> = facts.iter().map(|(t, _, _)| t.as_str()).collect();
        assert!(types.contains(&"person"));
        assert!(types.contains(&"commitment"));
        assert!(types.contains(&"preference"));
    }

    #[test]
    fn test_load_messages_before_window() {
        let (db, sid) = db_with_session();
        for i in 0..5 {
            db.save_message("mika", &sid, "user", &format!("msg {i}"), None)
                .unwrap();
        }
        let before = db.load_messages_before_window("mika", 2).unwrap();
        // Window is last 2, so before window is first 3
        assert_eq!(before.len(), 3);
        assert_eq!(before[0].content, "msg 0");
    }

    #[test]
    fn test_count_pending_tasks() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "T1".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        db.create_task(&task).unwrap();
        assert_eq!(db.count_pending_tasks("mika").unwrap(), 1);
    }

    fn make_task(label: &str) -> NewTask {
        NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[test]
    fn test_sibling_completion_all_done_fires_parent() {
        let db = db();
        let parent_id = db.create_task(&make_task("parent")).unwrap();

        let mut c1 = make_task("child1");
        c1.parent_task_id = Some(parent_id.clone());
        c1.depth = 1;
        let c1_id = db.create_task(&c1).unwrap();

        let mut c2 = make_task("child2");
        c2.parent_task_id = Some(parent_id.clone());
        c2.depth = 1;
        let c2_id = db.create_task(&c2).unwrap();

        let mut c3 = make_task("child3");
        c3.parent_task_id = Some(parent_id.clone());
        c3.depth = 1;
        let c3_id = db.create_task(&c3).unwrap();

        // Complete 2 of 3 — parent should NOT fire
        db.update_task_completed(&c1_id, "mika", Some("done"))
            .unwrap();
        db.update_task_completed(&c2_id, "mika", Some("done"))
            .unwrap();
        assert_eq!(
            db.try_complete_parent_on_sibling_done(&c2_id).unwrap(),
            None
        );

        // Complete the 3rd — parent should fire
        db.update_task_completed(&c3_id, "mika", Some("done"))
            .unwrap();
        let result = db.try_complete_parent_on_sibling_done(&c3_id).unwrap();
        assert_eq!(result, Some(parent_id));
    }

    #[test]
    fn test_sibling_completion_no_parent_returns_none() {
        let db = db();
        let task_id = db.create_task(&make_task("orphan")).unwrap();
        assert_eq!(
            db.try_complete_parent_on_sibling_done(&task_id).unwrap(),
            None
        );
    }

    #[test]
    fn test_sibling_completion_failed_child_counts_as_done() {
        let db = db();
        let parent_id = db.create_task(&make_task("parent")).unwrap();

        let mut c1 = make_task("child1");
        c1.parent_task_id = Some(parent_id.clone());
        let c1_id = db.create_task(&c1).unwrap();

        let mut c2 = make_task("child2");
        c2.parent_task_id = Some(parent_id.clone());
        let c2_id = db.create_task(&c2).unwrap();

        // One completed, one failed — both are "done"
        db.update_task_completed(&c1_id, "mika", Some("ok"))
            .unwrap();
        db.update_task_failed(&c2_id, "mika", "error").unwrap();
        let result = db.try_complete_parent_on_sibling_done(&c2_id).unwrap();
        assert_eq!(result, Some(parent_id));
    }

    #[test]
    fn test_get_child_tasks() {
        let db = db();
        let parent_id = db.create_task(&make_task("parent")).unwrap();

        let mut c1 = make_task("child1");
        c1.parent_task_id = Some(parent_id.clone());
        db.create_task(&c1).unwrap();

        let mut c2 = make_task("child2");
        c2.parent_task_id = Some(parent_id.clone());
        db.create_task(&c2).unwrap();

        let children = db.get_child_tasks(&parent_id).unwrap();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn test_get_task_descendants_three_levels() {
        let db = db();
        let root_id = db.create_task(&make_task("root")).unwrap();

        let mut child = make_task("child");
        child.parent_task_id = Some(root_id.clone());
        child.depth = 1;
        let child_id = db.create_task(&child).unwrap();

        let mut grandchild = make_task("grandchild");
        grandchild.parent_task_id = Some(child_id.clone());
        grandchild.depth = 2;
        let grandchild_id = db.create_task(&grandchild).unwrap();

        let mut great_grandchild = make_task("great-grandchild");
        great_grandchild.parent_task_id = Some(grandchild_id.clone());
        great_grandchild.depth = 3;
        db.create_task(&great_grandchild).unwrap();

        let descendants = db.get_task_descendants(&root_id).unwrap();
        assert_eq!(descendants.len(), 3);
        // Verify all descendants are present (ordering depends on created_at which may be identical)
        let labels: Vec<&str> = descendants.iter().map(|t| t.label.as_str()).collect();
        assert!(labels.contains(&"child"));
        assert!(labels.contains(&"grandchild"));
        assert!(labels.contains(&"great-grandchild"));
    }

    #[test]
    fn test_get_task_descendants_excludes_root() {
        let db = db();
        let root_id = db.create_task(&make_task("root")).unwrap();

        let mut child = make_task("child");
        child.parent_task_id = Some(root_id.clone());
        child.depth = 1;
        db.create_task(&child).unwrap();

        let descendants = db.get_task_descendants(&root_id).unwrap();
        assert_eq!(descendants.len(), 1);
        assert!(descendants.iter().all(|t| t.id != root_id));
    }

    #[test]
    fn test_get_task_descendants_no_children() {
        let db = db();
        let root_id = db.create_task(&make_task("root")).unwrap();

        let descendants = db.get_task_descendants(&root_id).unwrap();
        assert!(descendants.is_empty());
    }

    #[test]
    fn test_get_task_descendants_multi_branch() {
        let db = db();
        let root_id = db.create_task(&make_task("root")).unwrap();

        // 3 children, each with 2 grandchildren = 9 descendants total
        for i in 0..3 {
            let mut child = make_task(&format!("child-{i}"));
            child.parent_task_id = Some(root_id.clone());
            child.depth = 1;
            let child_id = db.create_task(&child).unwrap();

            for j in 0..2 {
                let mut gc = make_task(&format!("gc-{i}-{j}"));
                gc.parent_task_id = Some(child_id.clone());
                gc.depth = 2;
                db.create_task(&gc).unwrap();
            }
        }

        let descendants = db.get_task_descendants(&root_id).unwrap();
        assert_eq!(descendants.len(), 9);
    }

    #[test]
    fn test_get_task_descendants_cross_agent() {
        let db = Database::open_in_memory().unwrap();
        db.register_agent("mika", "Mika", "/tmp").unwrap();
        db.register_agent("other-agent", "Other", "/tmp").unwrap();

        let root_id = db.create_task(&make_task("root")).unwrap();

        let mut child = make_task("child");
        child.parent_task_id = Some(root_id.clone());
        child.depth = 1;
        child.agent_id = "other-agent".to_string();
        db.create_task(&child).unwrap();

        let descendants = db.get_task_descendants(&root_id).unwrap();
        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].agent_id, "other-agent");
    }

    #[test]
    fn test_count_pending_callback_tasks_by_team_run() {
        let db = Database::open_in_memory().unwrap();
        db.register_agent("mika", "Mika", "/tmp").unwrap();

        // Insert team + team runs so FK constraints are satisfied
        db.conn
            .execute(
                "INSERT OR IGNORE INTO teams (id, name) VALUES ('team1', 'team1')",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO team_runs (id, team_id, goal, started_at)
                 VALUES ('run123', 'team1', 'test goal', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
                [],
            )
            .unwrap();

        // Create a pending callback task with matching team_run_id and depth > 1
        let mut t = make_task("grandchild");
        t.trigger_type = "callback".to_string();
        t.depth = 2;
        t.team_run_id = Some("run123".to_string());
        db.create_task(&t).unwrap();

        // Create a completed callback task (should NOT be counted)
        let mut t2 = make_task("done-grandchild");
        t2.trigger_type = "callback".to_string();
        t2.depth = 2;
        t2.team_run_id = Some("run123".to_string());
        let t2_id = db.create_task(&t2).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed' WHERE id = ?1",
                params![t2_id],
            )
            .unwrap();

        // Create a depth=1 callback (should NOT be counted -- only depth > 1)
        let mut t3 = make_task("child-not-grandchild");
        t3.trigger_type = "callback".to_string();
        t3.depth = 1;
        t3.team_run_id = Some("run123".to_string());
        db.create_task(&t3).unwrap();

        // Create a pending callback for a DIFFERENT team run (should NOT be counted)
        db.conn
            .execute(
                "INSERT INTO team_runs (id, team_id, goal, started_at)
                 VALUES ('other-run', 'team1', 'other goal', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
                [],
            )
            .unwrap();
        let mut t4 = make_task("other-run-grandchild");
        t4.trigger_type = "callback".to_string();
        t4.depth = 2;
        t4.team_run_id = Some("other-run".to_string());
        db.create_task(&t4).unwrap();

        let count = db
            .count_pending_callback_tasks_by_team_run("run123")
            .unwrap();
        assert_eq!(count, 1); // only the pending depth=2 grandchild for run123
    }

    #[test]
    fn test_get_expired_child_task_ids() {
        let db = Database::open_in_memory().unwrap();
        db.register_agent("mika", "Mika", "/tmp").unwrap();

        // Parent still pending — its expired child SHOULD appear
        let parent_id = db.create_task(&make_task("parent")).unwrap();

        let mut c = make_task("expired-child");
        c.parent_task_id = Some(parent_id.clone());
        let c_id = db.create_task(&c).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'expired' WHERE id = ?1",
                params![c_id],
            )
            .unwrap();

        // Expired task without parent (should NOT appear)
        let o_id = db.create_task(&make_task("expired-orphan")).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'expired' WHERE id = ?1",
                params![o_id],
            )
            .unwrap();

        // Parent already completed — its expired child should NOT appear
        let done_parent_id = db.create_task(&make_task("done-parent")).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed' WHERE id = ?1",
                params![done_parent_id],
            )
            .unwrap();

        let mut c2 = make_task("expired-child-done-parent");
        c2.parent_task_id = Some(done_parent_id.clone());
        let c2_id = db.create_task(&c2).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'expired' WHERE id = ?1",
                params![c2_id],
            )
            .unwrap();

        let ids = db.get_expired_child_task_ids("mika").unwrap();
        assert_eq!(
            ids.len(),
            1,
            "should only return expired children whose parent is still pending"
        );
        assert_eq!(ids[0], c_id);
    }

    // ── mika#1742 Problem B — refuse-to-zombie guard on
    //    create_recurring_task_if_absent ────────────────────────────────

    fn zombie_recurring_task(agent_id: &str, label: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "recurring".to_string(),
            cron_expr: Some("0 0 * * * *".to_string()),
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    /// Fresh install (no prior rows) → registration succeeds.
    #[test]
    fn zombie_guard_fresh_install_registers() {
        let db = db();
        let id = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        assert!(id.is_some());
    }

    /// Existing `recurring_active` row → returns `None` (unchanged
    /// idempotency, driven by the partial unique index).
    #[test]
    fn zombie_guard_active_row_still_idempotent() {
        let db = db();
        db.create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        let second = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        assert!(
            second.is_none(),
            "second call must NOT create a duplicate active row"
        );
    }

    /// Recent `failed` row within the grace window → refuse to re-register.
    /// This IS the mika#1742 root-cause fix.
    #[test]
    fn zombie_guard_recent_failed_refuses_registration() {
        let db = db();
        let first = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap()
            .unwrap();
        // Mark it failed 1 hour ago.
        db.conn
            .execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
                params![first],
            )
            .unwrap();

        let retry = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        assert!(
            retry.is_none(),
            "recent-failed sibling must block fresh registration (mika#1742)"
        );
    }

    /// Recent `cancelled` row within the grace window → refuse.
    #[test]
    fn zombie_guard_recent_cancelled_refuses_registration() {
        let db = db();
        let first = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap()
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'cancelled',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2 hours')
                 WHERE id = ?1",
                params![first],
            )
            .unwrap();
        let retry = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        assert!(retry.is_none());
    }

    /// Old dead row (outside the grace window) → allow fresh registration.
    /// The grace has elapsed; operator has had time to notice / act.
    #[test]
    fn zombie_guard_expired_grace_allows_registration() {
        let db = db();
        let first = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap()
            .unwrap();
        // Push the failure well outside the 24h window.
        db.conn
            .execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-72 hours')
                 WHERE id = ?1",
                params![first],
            )
            .unwrap();
        let retry = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        assert!(
            retry.is_some(),
            "outside grace window must allow re-registration"
        );
    }

    /// Dead row for a DIFFERENT label must not block registration.
    #[test]
    fn zombie_guard_scoped_to_label() {
        let db = db();
        let first = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "other_label"))
            .unwrap()
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
                params![first],
            )
            .unwrap();
        let curator = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap();
        assert!(curator.is_some(), "guard must scope to (agent_id, label)");
    }

    /// Dead row for a DIFFERENT agent must not block registration.
    #[test]
    fn zombie_guard_scoped_to_agent() {
        let db = db();
        // Test fixture's migrate_v1 pre-registers 'mika' only; register the
        // second agent explicitly so the tasks FK holds.
        db.register_agent("mika-arch", "Mika Arch", "/tmp/mika-arch")
            .unwrap();
        let mika_first = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika", "curator_review"))
            .unwrap()
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE id = ?1",
                params![mika_first],
            )
            .unwrap();
        // Different agent — must succeed.
        let arch = db
            .create_recurring_task_if_absent(zombie_recurring_task("mika-arch", "curator_review"))
            .unwrap();
        assert!(arch.is_some(), "guard must scope to agent_id");
    }

    /// Consts stay in sync: the SQL modifier's magnitude must match
    /// [`RECURRING_ZOMBIE_GRACE_HOURS`] so operators reading logs and the
    /// SQL query agree on the same window.
    #[test]
    fn zombie_guard_grace_consts_stay_in_sync() {
        let expected = format!("-{RECURRING_ZOMBIE_GRACE_HOURS} hours");
        assert_eq!(
            RECURRING_ZOMBIE_GRACE_SQL, expected,
            "RECURRING_ZOMBIE_GRACE_SQL must match RECURRING_ZOMBIE_GRACE_HOURS"
        );
    }

    #[test]
    fn test_cancelled_recurring_task_allows_re_creation() {
        let db = db();

        // Step 1: Create a recurring task
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "heartbeat".to_string(),
            trigger_type: "recurring".to_string(),
            cron_expr: Some("0 0 * * * *".to_string()),
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id1 = db.create_task(&task).unwrap();
        assert!(!id1.is_empty());

        // Step 2: Cancel it
        assert!(db.cancel_task(&id1, "mika").unwrap());
        let t = db.get_task(&id1, "mika").unwrap().unwrap();
        assert_eq!(t.status, "cancelled");

        // Step 3: Create another recurring task with the same label — should succeed
        let task2 = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "heartbeat".to_string(),
            trigger_type: "recurring".to_string(),
            cron_expr: Some("0 0 * * * *".to_string()),
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id2 = db.create_task(&task2).unwrap();
        assert!(!id2.is_empty());
        assert_ne!(id1, id2, "should be a new task, not the old cancelled one");

        // Verify the new task is pending
        let t2 = db.get_task(&id2, "mika").unwrap().unwrap();
        assert_eq!(t2.status, "pending");
        assert_eq!(t2.label, "heartbeat");
    }

    fn callback_task(agent_id: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "analyze_codebase".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[test]
    fn test_get_undelivered_callback_tasks_returns_completed() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // Pending task should not appear
        let results = db
            .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
            .unwrap();
        assert!(results.is_empty());

        // Complete it
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

        // Now it should appear
        let results = db
            .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
    }

    #[test]
    fn test_get_undelivered_callback_tasks_since_boundary() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

        // Get the completed_at value
        let task = db.get_task(&id, "mika").unwrap().unwrap();
        let completed_at = task.completed_at.unwrap();

        // since = completed_at means "after this time", so the task at exactly
        // that time should still be included (query uses >)
        // But since completed_at == since, and query is >, it should NOT appear
        let results = db
            .get_undelivered_callback_tasks("mika", &completed_at)
            .unwrap();
        assert!(results.is_empty());

        // since before completed_at should include it
        let before = {
            let dt = timestamp::parse(&completed_at).unwrap();
            timestamp::format(&(dt - Duration::seconds(1)))
        };
        let results = db.get_undelivered_callback_tasks("mika", &before).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_get_undelivered_callback_tasks_excludes_other_agents() {
        let db = db();
        db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();
        let id = db.create_task(&callback_task("agent_a")).unwrap();
        assert!(db.update_task_completed(&id, "agent_a", Some("x")).unwrap());

        let results = db
            .get_undelivered_callback_tasks("agent_b", "1970-01-01T00:00:00Z")
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_undelivered_callback_tasks_for_session_scoped() {
        let db = db();
        // Create a task in session_a
        let mut task = callback_task("mika");
        task.created_by_session = Some("session_a".to_string());
        let id = db.create_task(&task).unwrap();
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

        // Session A should see it
        let results = db
            .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_a")
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);

        // Session B should NOT see it
        let results = db
            .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_b")
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_undelivered_callback_tasks_for_session_excludes_no_session() {
        let db = db();
        // Task with no session (created_by_session = None) should not appear
        let id = db.create_task(&callback_task("mika")).unwrap();
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

        let results = db
            .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_a")
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_mark_task_delivered_claims_completed_task() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();
        assert!(
            db.update_task_completed(&id, "mika", Some("result"))
                .unwrap()
        );

        // First claim succeeds
        assert!(db.mark_task_delivered(&id).unwrap());

        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "delivered");
    }

    #[test]
    fn test_mark_task_delivered_double_claim_rejected() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();
        assert!(
            db.update_task_completed(&id, "mika", Some("result"))
                .unwrap()
        );

        // First claim
        assert!(db.mark_task_delivered(&id).unwrap());
        // Second claim returns false (already delivered)
        assert!(!db.mark_task_delivered(&id).unwrap());
    }

    #[test]
    fn test_mark_task_delivered_rejects_non_completed_task() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // Task is still pending, should not be claimable
        assert!(!db.mark_task_delivered(&id).unwrap());
    }

    #[test]
    fn test_get_undelivered_callback_tasks_returns_failed_tasks() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // Mark it as failed (simulates background monitor detecting non-zero exit)
        db.update_task_failed(&id, "mika", "Process exited with code 1: error output")
            .unwrap();

        // Failed task should appear in undelivered callbacks
        let results = db
            .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
        assert_eq!(results[0].status, "failed");
        assert_eq!(
            results[0].result.as_deref(),
            Some("Process exited with code 1: error output")
        );
    }

    #[test]
    fn test_get_undelivered_callback_tasks_returns_both_completed_and_failed() {
        let db = db();
        let id1 = db.create_task(&callback_task("mika")).unwrap();
        let id2 = db.create_task(&callback_task("mika")).unwrap();

        // Complete one, fail the other
        assert!(
            db.update_task_completed(&id1, "mika", Some("success"))
                .unwrap()
        );
        db.update_task_failed(&id2, "mika", "Process exited with code 128: fatal error")
            .unwrap();

        // Both should appear, ordered by completed_at
        let results = db
            .get_undelivered_callback_tasks("mika", "1970-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(results.len(), 2);
        // Both tasks present (order depends on completed_at which is set to 'now' for both)
        let statuses: Vec<&str> = results.iter().map(|t| t.status.as_str()).collect();
        assert!(statuses.contains(&"completed"));
        assert!(statuses.contains(&"failed"));
    }

    #[test]
    fn test_get_undelivered_callback_tasks_for_session_returns_failed() {
        let db = db();
        let mut task = callback_task("mika");
        task.created_by_session = Some("session_a".to_string());
        let id = db.create_task(&task).unwrap();
        db.update_task_failed(&id, "mika", "crash").unwrap();

        // Session A should see the failed task
        let results = db
            .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_a")
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "failed");

        // Session B should not
        let results = db
            .get_undelivered_callback_tasks_for_session("mika", "1970-01-01T00:00:00Z", "session_b")
            .unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_active_background_task_count_counts_pending_and_in_progress() {
        let db = db();
        // No callback tasks → count is 0
        assert_eq!(db.get_active_background_task_count("mika").unwrap(), 0);

        // Create a pending callback task → count is 1
        let id1 = db.create_task(&callback_task("mika")).unwrap();
        assert_eq!(db.get_active_background_task_count("mika").unwrap(), 1);

        // Create a second pending callback task → count is 2
        let mut task2 = callback_task("mika");
        task2.label = "build_project".to_string();
        let _id2 = db.create_task(&task2).unwrap();
        assert_eq!(db.get_active_background_task_count("mika").unwrap(), 2);

        // Complete the first task → count drops to 1
        assert!(
            db.update_task_completed(&id1, "mika", Some("done"))
                .unwrap()
        );
        assert_eq!(db.get_active_background_task_count("mika").unwrap(), 1);
    }

    #[test]
    fn test_get_active_background_task_count_excludes_other_agents() {
        let db = db();
        db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

        let _id = db.create_task(&callback_task("agent_a")).unwrap();

        // agent_a has 1, agent_b has 0
        assert_eq!(db.get_active_background_task_count("agent_a").unwrap(), 1);
        assert_eq!(db.get_active_background_task_count("agent_b").unwrap(), 0);
    }

    #[test]
    fn test_get_active_background_task_count_excludes_non_callback_tasks() {
        let db = db();
        // Create a non-callback task (e.g., a reminder)
        let reminder = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "morning reminder".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: r#"{"text":"hello"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let _id = db.create_task(&reminder).unwrap();

        // Reminder should NOT count as a background task
        assert_eq!(db.get_active_background_task_count("mika").unwrap(), 0);
    }

    #[test]
    fn test_get_background_task_counts_splits_executing_and_queued() {
        let db = db();

        // No callback tasks → both zero
        let counts = db.get_background_task_counts("mika").unwrap();
        assert_eq!(
            counts,
            BackgroundTaskCounts {
                executing: 0,
                queued: 0
            }
        );

        // Create two pending callback tasks (no process_id) → both queued
        let id1 = db.create_task(&callback_task("mika")).unwrap();
        let mut task2 = callback_task("mika");
        task2.label = "build_project".to_string();
        let _id2 = db.create_task(&task2).unwrap();
        let counts = db.get_background_task_counts("mika").unwrap();
        assert_eq!(
            counts,
            BackgroundTaskCounts {
                executing: 0,
                queued: 2
            }
        );

        // Set process_id on task1 → 1 executing, 1 queued
        db.set_task_process_id(&id1, Some(12345)).unwrap();
        let counts = db.get_background_task_counts("mika").unwrap();
        assert_eq!(
            counts,
            BackgroundTaskCounts {
                executing: 1,
                queued: 1
            }
        );

        // Backward compat: total still matches
        assert_eq!(db.get_active_background_task_count("mika").unwrap(), 2);
    }

    #[test]
    fn test_get_background_task_counts_all_executing() {
        let db = db();

        let id1 = db.create_task(&callback_task("mika")).unwrap();
        let mut task2 = callback_task("mika");
        task2.label = "build_project".to_string();
        let id2 = db.create_task(&task2).unwrap();

        db.set_task_process_id(&id1, Some(100)).unwrap();
        db.set_task_process_id(&id2, Some(200)).unwrap();

        let counts = db.get_background_task_counts("mika").unwrap();
        assert_eq!(
            counts,
            BackgroundTaskCounts {
                executing: 2,
                queued: 0
            }
        );
    }

    #[test]
    fn test_get_background_task_counts_excludes_terminal_status() {
        let db = db();

        let id1 = db.create_task(&callback_task("mika")).unwrap();
        db.set_task_process_id(&id1, Some(100)).unwrap();

        // Complete the task — should not appear in either bucket
        db.update_task_completed(&id1, "mika", Some("done"))
            .unwrap();
        let counts = db.get_background_task_counts("mika").unwrap();
        assert_eq!(
            counts,
            BackgroundTaskCounts {
                executing: 0,
                queued: 0
            }
        );
    }

    #[test]
    fn test_mark_task_delivered_claims_failed_task() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();
        db.update_task_failed(&id, "mika", "exit code 1").unwrap();

        // Claim the failed task
        assert!(db.mark_task_delivered(&id).unwrap());

        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "delivered");
    }

    #[test]
    fn test_mark_task_delivered_failed_double_claim_rejected() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();
        db.update_task_failed(&id, "mika", "exit code 1").unwrap();

        // First claim
        assert!(db.mark_task_delivered(&id).unwrap());
        // Second claim returns false (already delivered)
        assert!(!db.mark_task_delivered(&id).unwrap());
    }

    // -- find_orphaned_parent_tasks tests (#871) --

    /// Helper: create a parent self_dev task (manual, in_progress, source=self_dev)
    /// and a delivered callback child. Returns (parent_id, child_id).
    fn create_orphaned_parent_setup(db: &Database) -> (String, String) {
        // Parent: manual, in_progress, source=self_dev
        let mut parent = new_task("mika", "Implement mika#868", "manual", "none");
        parent.source = Some("self_dev".to_string());
        let parent_id = db.create_task(&parent).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![parent_id],
            )
            .unwrap();

        // Child: callback, resume_agent, delivered
        let mut child = callback_task("mika");
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();
        // Complete then deliver the child
        assert!(
            db.update_task_completed(&child_id, "mika", Some("done"))
                .unwrap()
        );
        assert!(db.mark_task_delivered(&child_id).unwrap());

        (parent_id, child_id)
    }

    #[test]
    fn test_find_orphaned_parent_tasks_failure_path() {
        let db = db();
        let (parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Backdate the child's updated_at so it's past the grace period (700s > 600s)
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].id, parent_id);
        assert_eq!(orphans[0].callback_task_id, child_id);
    }

    #[test]
    fn test_find_orphaned_parent_tasks_happy_path_pr_url_present() {
        let db = db();
        let (parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Set pr_url on parent metadata — reaper should NOT match
        let meta = r#"{"claude_pilot": {"pr_url": "https://github.com/x/y/pull/1"}}"#;
        db.conn
            .execute(
                "UPDATE tasks SET metadata = ?1 WHERE id = ?2",
                params![meta, parent_id],
            )
            .unwrap();

        // Backdate child past grace
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert!(
            orphans.is_empty(),
            "parent with pr_url should not be reaped"
        );
    }

    #[test]
    fn test_find_orphaned_parent_tasks_grace_period_not_elapsed() {
        let db = db();
        let (_parent_id, _child_id) = create_orphaned_parent_setup(&db);

        // Child was just delivered (updated_at = now), well within 600s grace
        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert!(
            orphans.is_empty(),
            "parent within grace period should not be reaped"
        );
    }

    #[test]
    fn test_find_orphaned_parent_tasks_active_sibling_defers() {
        let db = db();
        let (parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Backdate the delivered child past grace
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // Add a sibling callback child that's still in_progress (e.g., #870 retry)
        let mut sibling = callback_task("mika");
        sibling.parent_task_id = Some(parent_id.clone());
        let sibling_id = db.create_task(&sibling).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![sibling_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert!(
            orphans.is_empty(),
            "parent with active sibling callback should not be reaped"
        );
    }

    #[test]
    fn test_find_orphaned_parent_tasks_excludes_other_agents() {
        let db = db();
        db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

        // Create orphaned parent under agent_a
        let mut parent = new_task("agent_a", "Implement task", "manual", "none");
        parent.source = Some("self_dev".to_string());
        let parent_id = db.create_task(&parent).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![parent_id],
            )
            .unwrap();

        let mut child = NewTask {
            agent_id: "agent_a".to_string(),
            ..callback_task("agent_a")
        };
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();
        assert!(
            db.update_task_completed(&child_id, "agent_a", Some("done"))
                .unwrap()
        );
        assert!(db.mark_task_delivered(&child_id).unwrap());
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // agent_b should see nothing
        let orphans = db.find_orphaned_parent_tasks("agent_b", 600).unwrap();
        assert!(orphans.is_empty());

        // agent_a should see the orphan
        let orphans = db.find_orphaned_parent_tasks("agent_a", 600).unwrap();
        assert_eq!(orphans.len(), 1);
    }

    /// mika#1118 v2 — groom-class CALLBACKS must NOT trigger reaping of their
    /// parent. The class is keyed off the child (per-dispatch) because reused
    /// parents (mika#920) carry stale class data.
    #[test]
    fn test_find_orphaned_parent_tasks_groom_class_not_reaped() {
        let db = db();
        let (_parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Set CHILD callback dispatch_class to 'groom' (the per-dispatch class)
        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // Backdate child past grace (would otherwise trigger reaping)
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert!(
            orphans.is_empty(),
            "parent with groom-class callback should not be reaped — grooming produces plan commits, not PRs"
        );
    }

    /// mika#1118 v2 — implement-class callbacks still trigger reaping when the
    /// dispatch completes without producing a PR. Confirms the per-child filter
    /// does not regress the original #871 behavior.
    #[test]
    fn test_find_orphaned_parent_tasks_implement_class_still_reaped() {
        let db = db();
        let (parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Set CHILD callback dispatch_class explicitly to 'implement'
        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'implement' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // Backdate child past grace
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].id, parent_id);
        assert_eq!(orphans[0].callback_task_id, child_id);
    }

    /// mika#1118 v2 — NULL dispatch_class on the CHILD callback (pre-v34 rows)
    /// is treated as 'implement' via COALESCE. Preserves backward compatibility
    /// with rows created before the v33→v34 migration added the column.
    #[test]
    fn test_find_orphaned_parent_tasks_null_dispatch_class_treated_as_implement() {
        let db = db();
        let (parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Helper does NOT set dispatch_class — defaults to NULL on both parent
        // and child. Verify the CHILD's column is actually NULL.
        let class: Option<String> = db
            .conn
            .query_row(
                "SELECT dispatch_class FROM tasks WHERE id = ?1",
                params![child_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            class.is_none(),
            "test setup invariant: child dispatch_class should be NULL pre-v34-style"
        );

        // Backdate child past grace
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert_eq!(
            orphans.len(),
            1,
            "NULL child dispatch_class must still be reaped (COALESCE -> 'implement')"
        );
        assert_eq!(orphans[0].id, parent_id);
    }

    /// mika#1118 v2 — the key regression case: reused parent (mika#920) with
    /// NULL `parent.dispatch_class` but groom-class `child.dispatch_class`.
    /// v1 would have reaped because it checked the parent's class. v2 must NOT
    /// reap because the child's class is the per-dispatch authority.
    #[test]
    fn test_find_orphaned_parent_tasks_stale_parent_class_doesnt_drive_reaping() {
        let db = db();
        let (_parent_id, child_id) = create_orphaned_parent_setup(&db);

        // Parent stays NULL (simulates reused parent from pre-v34 or mika#920).
        // Child is the fresh groom-class dispatch.
        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // Backdate child past grace
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert!(
            orphans.is_empty(),
            "stale parent class must not override the fresh child class — v1 regression"
        );
    }

    /// mika#1126 — H3 scenario: parent with two children where one has NULL
    /// dispatch_class (treated as 'implement') and one has 'groom'. The NULL
    /// child matches the reaper filter, so the parent IS reaped.
    #[test]
    fn test_find_orphaned_parent_tasks_mixed_children_groom_and_null() {
        let db = db();
        let (parent_id, child_a_id) = create_orphaned_parent_setup(&db);

        // child_a: NULL dispatch_class (default from helper), callback, delivered
        // Already set up by helper. Backdate past grace.
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_a_id],
            )
            .unwrap();

        // child_b: groom dispatch_class, callback, delivered
        let mut child_b = callback_task("mika");
        child_b.parent_task_id = Some(parent_id.clone());
        child_b.dispatch_class = Some("groom".to_string());
        let child_b_id = db.create_task(&child_b).unwrap();
        assert!(
            db.update_task_completed(&child_b_id, "mika", Some("done"))
                .unwrap()
        );
        assert!(db.mark_task_delivered(&child_b_id).unwrap());
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_b_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert_eq!(
            orphans.len(),
            1,
            "parent with mixed children (NULL + groom) should be reaped — the NULL child matches"
        );
        assert_eq!(orphans[0].id, parent_id);
    }

    /// mika#1126 — parent with ONLY groom-class children should NOT be reaped.
    /// Both children have dispatch_class='groom'; no implement-class child exists.
    #[test]
    fn test_find_orphaned_parent_tasks_only_groom_children_not_reaped() {
        let db = db();
        let (parent_id, child_a_id) = create_orphaned_parent_setup(&db);

        // Set child_a to groom class
        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
                params![child_a_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_a_id],
            )
            .unwrap();

        // child_b: also groom class
        let mut child_b = callback_task("mika");
        child_b.parent_task_id = Some(parent_id.clone());
        child_b.dispatch_class = Some("groom".to_string());
        let child_b_id = db.create_task(&child_b).unwrap();
        assert!(
            db.update_task_completed(&child_b_id, "mika", Some("done"))
                .unwrap()
        );
        assert!(db.mark_task_delivered(&child_b_id).unwrap());
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_b_id],
            )
            .unwrap();

        let orphans = db.find_orphaned_parent_tasks("mika", 600).unwrap();
        assert!(
            orphans.is_empty(),
            "parent with only groom-class children should not be reaped"
        );
    }

    // -- find_childless_stuck_parent_tasks tests (mika#1687) --

    /// Create a childless self_dev **issue** parent left `in_progress`, with
    /// `updated_at` aged `age_secs` into the past. Returns `parent_id`. The
    /// zero-child complement of `create_orphaned_parent_setup` — no callback
    /// child is spawned (the silent-pilot-death signature).
    fn create_childless_stuck_parent(db: &Database, age_secs: i64) -> String {
        let mut parent = new_task("mika", "Implement mika#1687", "manual", "none");
        parent.source = Some("self_dev".to_string());
        // type defaults to 'issue' via SQL DEFAULT (r#type: None).
        let parent_id = db.create_task(&parent).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                 WHERE id = ?1",
                params![parent_id, format!("-{age_secs} seconds")],
            )
            .unwrap();
        parent_id
    }

    #[test]
    fn test_find_childless_stuck_parent_selects_qualifying() {
        let db = db();
        let parent_id = create_childless_stuck_parent(&db, 2000);

        let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0].id, parent_id);
        assert_eq!(stuck[0].agent_id, "mika");
        assert!(!stuck[0].created_at.is_empty());
        assert!(!stuck[0].updated_at.is_empty());
    }

    #[test]
    fn test_find_childless_stuck_parent_excludes_parent_with_any_child() {
        let db = db();
        let parent_id = create_childless_stuck_parent(&db, 2000);

        // Add a callback child that is merely `pending` (not delivered). The
        // NOT EXISTS predicate excludes on ANY child row — this is the exact
        // complement of the orphan reaper's `delivered`-child INNER JOIN.
        let mut child = callback_task("mika");
        child.parent_task_id = Some(parent_id.clone());
        db.create_task(&child).unwrap();

        let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
        assert!(
            stuck.is_empty(),
            "parent with any child (even pending) must not be reaped"
        );
    }

    #[test]
    fn test_find_childless_stuck_parent_excludes_younger_than_grace() {
        let db = db();
        create_childless_stuck_parent(&db, 100);

        let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
        assert!(
            stuck.is_empty(),
            "parent younger than grace must not be reaped"
        );
    }

    #[test]
    fn test_find_childless_stuck_parent_excludes_milestone_and_project() {
        let db = db();

        for task_type in ["milestone", "project"] {
            let mut parent = new_task("mika", "Milestone parent", "manual", "none");
            parent.source = Some("self_dev".to_string());
            parent.r#type = Some(task_type.to_string());
            let parent_id = db.create_task(&parent).unwrap();
            db.conn
                .execute(
                    "UPDATE tasks SET status = 'in_progress',
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                     WHERE id = ?1",
                    params![parent_id],
                )
                .unwrap();
        }

        let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
        assert!(
            stuck.is_empty(),
            "milestone/project parents are out of scope for v1 (D2)"
        );
    }

    #[test]
    fn test_find_childless_stuck_parent_excludes_wrong_status_source_trigger() {
        let db = db();

        // (a) pending (never reached in_progress): backdate but leave pending.
        let mut pending = new_task("mika", "Pending", "manual", "none");
        pending.source = Some("self_dev".to_string());
        let pending_id = db.create_task(&pending).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
                params![pending_id],
            )
            .unwrap();

        // (b) non-self_dev source, in_progress, aged.
        let mut other_source = new_task("mika", "Other source", "manual", "none");
        other_source.source = Some("webhook".to_string());
        let other_source_id = db.create_task(&other_source).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
                params![other_source_id],
            )
            .unwrap();

        // (c) non-manual trigger (recurring), self_dev, in_progress, aged.
        let mut recurring = new_task("mika", "Recurring", "recurring", "none");
        recurring.source = Some("self_dev".to_string());
        let recurring_id = db.create_task(&recurring).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
                params![recurring_id],
            )
            .unwrap();

        let stuck = db.find_childless_stuck_parent_tasks("mika", 1800).unwrap();
        assert!(
            stuck.is_empty(),
            "pending / non-self_dev / non-manual parents must not be reaped"
        );
    }

    #[test]
    fn test_find_childless_stuck_parent_excludes_other_agents() {
        let db = db();
        db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

        let mut parent = new_task("agent_a", "Implement task", "manual", "none");
        parent.source = Some("self_dev".to_string());
        let parent_id = db.create_task(&parent).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2000 seconds')
                 WHERE id = ?1",
                params![parent_id],
            )
            .unwrap();

        // agent_b sees nothing; agent_a sees its own stuck parent.
        assert!(
            db.find_childless_stuck_parent_tasks("agent_b", 1800)
                .unwrap()
                .is_empty()
        );
        let stuck = db
            .find_childless_stuck_parent_tasks("agent_a", 1800)
            .unwrap();
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0].id, parent_id);
    }

    /// Terminal-state race: once the childless parent is transitioned out of
    /// `in_progress`, the guarded `update_task_failed` no-ops (Ok(false)) and
    /// the query stops selecting it — the reaper never double-writes (R7).
    #[test]
    fn test_find_childless_stuck_parent_terminal_state_race() {
        let db = db();
        let parent_id = create_childless_stuck_parent(&db, 2000);

        // First reap succeeds.
        assert!(
            db.update_task_failed(&parent_id, "mika", "stuck_in_progress_no_callback_child")
                .unwrap()
        );

        // No longer selected (left in_progress).
        assert!(
            db.find_childless_stuck_parent_tasks("mika", 1800)
                .unwrap()
                .is_empty()
        );

        // A second guarded write no-ops rather than overwriting.
        assert!(
            !db.update_task_failed(&parent_id, "mika", "should not overwrite")
                .unwrap()
        );
        let task = db.get_task(&parent_id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "failed");
        assert_eq!(
            task.result.as_deref(),
            Some("stuck_in_progress_no_callback_child")
        );
    }

    /// mika#1126 — get_reaper_child_snapshot returns ALL children of a parent.
    #[test]
    fn test_get_reaper_child_snapshot_returns_all_children() {
        let db = db();
        let (parent_id, child_a_id) = create_orphaned_parent_setup(&db);

        // Add a second groom-class child
        let mut child_b = callback_task("mika");
        child_b.parent_task_id = Some(parent_id.clone());
        child_b.dispatch_class = Some("groom".to_string());
        let child_b_id = db.create_task(&child_b).unwrap();

        let snapshot = db.get_reaper_child_snapshot(&parent_id).unwrap();
        assert_eq!(snapshot.len(), 2, "snapshot should return all children");

        let ids: Vec<&str> = snapshot.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&child_a_id.as_str()));
        assert!(ids.contains(&child_b_id.as_str()));

        // Verify the groom child has dispatch_class populated
        let groom_child = snapshot.iter().find(|s| s.id == child_b_id).unwrap();
        assert_eq!(groom_child.dispatch_class.as_deref(), Some("groom"));
        assert_eq!(groom_child.trigger_type, "callback");
        assert_eq!(groom_child.action_type, "resume_agent");
    }

    // -- find_completable_parent_tasks_on_pr_url tests (mika#1162) --

    /// Helper: like `create_orphaned_parent_setup` but stamps a `pr_url` on the
    /// parent's metadata to make it a success-side candidate.
    fn create_completable_parent_setup(db: &Database, pr_url: &str) -> (String, String) {
        let (parent_id, child_id) = create_orphaned_parent_setup(db);
        let meta = format!(r#"{{"claude_pilot":{{"pr_url":"{pr_url}"}}}}"#);
        db.conn
            .execute(
                "UPDATE tasks SET metadata = ?1 WHERE id = ?2",
                params![meta, parent_id],
            )
            .unwrap();
        (parent_id, child_id)
    }

    #[test]
    fn test_find_completable_parent_tasks_happy_path() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1234";
        let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        // Backdate the child's updated_at past the grace period
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, parent_id);
        assert_eq!(candidates[0].callback_task_id, child_id);
        assert_eq!(candidates[0].pr_url, pr_url);
    }

    #[test]
    fn test_find_completable_parent_tasks_no_pr_url_excluded() {
        // Mirror of the reaper's happy path — when pr_url is absent the
        // completer must NOT match. (The reaper handles that case.)
        let db = db();
        let (_parent_id, child_id) = create_orphaned_parent_setup(&db);

        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "parent without pr_url is reaper territory, not completer"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_empty_pr_url_excluded() {
        let db = db();
        let (_parent_id, child_id) = create_completable_parent_setup(&db, "");

        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "empty pr_url string must not trip the completer"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_grace_period_not_elapsed() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";
        let (_parent_id, _child_id) = create_completable_parent_setup(&db, pr_url);

        // Child was just delivered, well within the 600s grace window
        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "parent within grace period should not be auto-completed yet"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_child_in_progress_excluded() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";

        // Custom setup: parent ready, but child is `in_progress` (not delivered).
        let mut parent = new_task("mika", "Implement mika#1162", "manual", "none");
        parent.source = Some("self_dev".to_string());
        let parent_id = db.create_task(&parent).unwrap();
        let meta = format!(r#"{{"claude_pilot":{{"pr_url":"{pr_url}"}}}}"#);
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress', metadata = ?1 WHERE id = ?2",
                params![meta, parent_id],
            )
            .unwrap();

        let mut child = callback_task("mika");
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "child must be `delivered`, not `in_progress`"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_groom_class_excluded() {
        // Defense-in-depth: groom-class callbacks don't emit `PR:` lines, but
        // the dispatch_class filter must still exclude them. Mirror of the
        // reaper's same-named guard.
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";
        let (_parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "groom-class callbacks must not trip the success-side completer"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_implement_class_matched() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";
        let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'implement' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, parent_id);
    }

    #[test]
    fn test_find_completable_parent_tasks_null_dispatch_class_treated_as_implement() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";
        let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        // Helper leaves dispatch_class NULL (pre-v34 row shape)
        let class: Option<String> = db
            .conn
            .query_row(
                "SELECT dispatch_class FROM tasks WHERE id = ?1",
                params![child_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(class.is_none(), "test invariant: pre-v34 shape");

        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "NULL dispatch_class must still match (COALESCE -> 'implement')"
        );
        assert_eq!(candidates[0].id, parent_id);
    }

    #[test]
    fn test_find_completable_parent_tasks_parent_not_in_progress() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";
        let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        // Parent is already completed (race with the inline path).
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed' WHERE id = ?1",
                params![parent_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "parent not in `in_progress` must not be re-completed"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_active_sibling_defers() {
        let db = db();
        let pr_url = "https://github.com/owner/repo/pull/1";
        let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // Another callback child is still in_progress
        let mut sibling = callback_task("mika");
        sibling.parent_task_id = Some(parent_id.clone());
        let sibling_id = db.create_task(&sibling).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![sibling_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert!(
            candidates.is_empty(),
            "parent with active sibling callback should wait for sibling completion"
        );
    }

    #[test]
    fn test_find_completable_parent_tasks_excludes_other_agents() {
        let db = db();
        db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();

        let mut parent = new_task("agent_a", "Implement mika#1162", "manual", "none");
        parent.source = Some("self_dev".to_string());
        let parent_id = db.create_task(&parent).unwrap();
        let meta = r#"{"claude_pilot":{"pr_url":"https://github.com/x/y/pull/1"}}"#;
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress', metadata = ?1 WHERE id = ?2",
                params![meta, parent_id],
            )
            .unwrap();

        let mut child = NewTask {
            agent_id: "agent_a".to_string(),
            ..callback_task("agent_a")
        };
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();
        assert!(
            db.update_task_completed(&child_id, "agent_a", Some("done"))
                .unwrap()
        );
        assert!(db.mark_task_delivered(&child_id).unwrap());
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        // agent_b should see nothing
        let candidates = db
            .find_completable_parent_tasks_on_pr_url("agent_b", 600)
            .unwrap();
        assert!(candidates.is_empty());

        // agent_a should see the candidate
        let candidates = db
            .find_completable_parent_tasks_on_pr_url("agent_a", 600)
            .unwrap();
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn test_find_completable_parent_tasks_stale_parent_class_doesnt_drive_selection() {
        // mika#1162 v2 — symmetric to test_find_orphaned_parent_tasks_stale_parent_class_*.
        // mika#920 task-reuse pattern means a parent's dispatch_class can be
        // stale relative to the most recent child callback. The selection MUST
        // key off the CHILD's dispatch_class, not the parent's. Set parent class
        // to 'groom' (stale, would have caused false-negative if we keyed off
        // parent) and child to 'implement' (the fresh dispatch). Parent must be
        // selected.
        let db = db();
        let pr_url = "https://github.com/x/y/pull/1";
        let (parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        // Set PARENT class to 'groom' (stale data from a prior dispatch)
        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'groom' WHERE id = ?1",
                params![parent_id],
            )
            .unwrap();
        // Set CHILD class to 'implement' (the fresh per-dispatch authority)
        db.conn
            .execute(
                "UPDATE tasks SET dispatch_class = 'implement' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert_eq!(
            candidates.len(),
            1,
            "stale parent class must not override the fresh child class — must select on child.dispatch_class"
        );
        assert_eq!(candidates[0].id, parent_id);
    }

    #[test]
    fn test_find_completable_parent_tasks_returns_pr_url_field() {
        // Defense-in-depth: confirm the pr_url comes through unchanged from
        // json_extract. The engine completer relies on this for its audit log.
        let db = db();
        let pr_url = "https://github.com/senara-solutions/mika/pull/1158";
        let (_parent_id, child_id) = create_completable_parent_setup(&db, pr_url);

        db.conn
            .execute(
                "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-700 seconds') WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let candidates = db
            .find_completable_parent_tasks_on_pr_url("mika", 600)
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pr_url, pr_url);
    }

    #[test]
    fn test_update_task_failed_guards_terminal_states() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // Complete the task first
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

        // Attempting to fail an already-completed task should return Ok(false)
        let updated = db
            .update_task_failed(&id, "mika", "should not overwrite")
            .unwrap();
        assert!(!updated);

        // Verify the task status is still 'completed' (not overwritten to 'failed')
        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.result.as_deref(), Some("done"));
    }

    #[test]
    fn test_promote_task_completed_from_failed() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // First fail the task
        assert!(
            db.update_task_failed(&id, "mika", "initial_failure")
                .unwrap()
        );
        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "failed");

        // Promote from failed → completed
        let promoted = db
            .promote_task_completed(&id, "mika", "retry_success (pr_url: https://example.com)")
            .unwrap();
        assert!(promoted);

        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(
            task.result.as_deref(),
            Some("retry_success (pr_url: https://example.com)")
        );
        assert!(task.completed_at.is_some());
    }

    #[test]
    fn test_promote_task_completed_noop_for_non_failed() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // Task starts in pending — promote should be a no-op
        let promoted = db
            .promote_task_completed(&id, "mika", "should not fire")
            .unwrap();
        assert!(!promoted);
        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "pending");

        // Complete the task — promote should still be a no-op
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());
        let promoted = db
            .promote_task_completed(&id, "mika", "should not fire")
            .unwrap();
        assert!(!promoted);
        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "completed");
        assert_eq!(task.result.as_deref(), Some("done"));
    }

    #[test]
    fn test_promote_task_completed_wrong_agent() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();
        assert!(db.update_task_failed(&id, "mika", "failure").unwrap());

        // Wrong agent_id — should return false
        let promoted = db
            .promote_task_completed(&id, "wrong-agent", "retry_success")
            .unwrap();
        assert!(!promoted);
        let task = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(task.status, "failed");
    }

    #[test]
    fn test_promote_task_completed_nonexistent() {
        let db = db();
        let promoted = db
            .promote_task_completed("nonexistent-id", "mika", "retry_success")
            .unwrap();
        assert!(!promoted);
    }

    #[test]
    fn test_is_unique_violation_only_catches_unique() {
        use rusqlite::ffi;

        // UNIQUE violation should match
        let unique_err = rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CONSTRAINT_UNIQUE),
            Some("UNIQUE constraint failed".to_string()),
        );
        assert!(is_unique_violation(&anyhow::Error::from(unique_err)));

        // NOT NULL violation should NOT match
        let notnull_err = rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CONSTRAINT_NOTNULL),
            Some("NOT NULL constraint failed".to_string()),
        );
        assert!(!is_unique_violation(&anyhow::Error::from(notnull_err)));
    }

    #[test]
    fn test_trace_id_propagation() {
        let (db, sid) = db_with_session();
        let trace = "abcd1234abcd1234abcd1234abcd1234";

        // Save message with trace_id
        db.save_message("mika", &sid, "user", "traced msg", Some(trace))
            .unwrap();

        // Log audit event with trace_id
        db.log_audit_event(
            "mika",
            &sid,
            "store_fact",
            "person:Alice",
            None,
            Some("new"),
            None,
            Some(trace),
        )
        .unwrap();

        // Create task with trace_id
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "traced-task".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(sid.clone()),
            created_trace_id: Some(trace.to_string()),
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        db.create_task(&task).unwrap();

        // Query unified_timeline for this trace_id
        let rows: Vec<(String, String)> = db
            .conn
            .prepare("SELECT event_type, summary FROM unified_timeline WHERE trace_id = ?1 ORDER BY event_type")
            .unwrap()
            .query_map([trace], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            rows.len(),
            3,
            "expected message + audit + task in unified_timeline"
        );
        let types: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
        assert!(types.contains(&"audit"), "missing audit event");
        assert!(types.contains(&"message"), "missing message");
        assert!(types.contains(&"task"), "missing task");
    }

    #[test]
    fn test_unified_timeline_includes_null_trace_id() {
        let (db, sid) = db_with_session();

        // Save message without trace_id (legacy behavior)
        db.save_message("mika", &sid, "user", "legacy msg", None)
            .unwrap();

        // Query for NULL trace_id rows
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM unified_timeline WHERE trace_id IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert!(
            count >= 1,
            "legacy rows with NULL trace_id should appear in unified_timeline"
        );
    }

    // ===== Skill Override Tests =====

    #[test]
    fn test_skill_override_crud() {
        let db = db();

        // Initially empty
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert!(overrides.is_empty());

        // Set an override
        db.set_skill_override("mika", "web-search", true).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].skill_name, "web-search");
        assert_eq!(overrides[0].always_on, Some(true));

        // Update the override
        db.set_skill_override("mika", "web-search", false).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].always_on, Some(false));

        // Delete the override
        db.delete_skill_override("mika", "web-search").unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert!(overrides.is_empty());
    }

    #[test]
    fn test_skill_llm_override_round_trip() {
        let mut db = db();

        // Set LLM override on a skill with no existing row.
        db.set_skill_llm_override("mika", "qa-review", "anthropic", "claude-sonnet-4-6")
            .unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].skill_name, "qa-review");
        assert_eq!(overrides[0].always_on, None);
        assert_eq!(overrides[0].llm_provider.as_deref(), Some("anthropic"));
        assert_eq!(overrides[0].llm_model.as_deref(), Some("claude-sonnet-4-6"));

        // Upsert to a different model.
        db.set_skill_llm_override("mika", "qa-review", "deepseek", "deepseek-chat")
            .unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].llm_provider.as_deref(), Some("deepseek"));
        assert_eq!(overrides[0].llm_model.as_deref(), Some("deepseek-chat"));

        // Clearing LLM columns prunes a row with no other overrides.
        db.delete_skill_llm_override("mika", "qa-review").unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert!(overrides.is_empty(), "fully-NULL row should be pruned");
    }

    #[test]
    fn test_skill_llm_override_preserves_always_on() {
        let mut db = db();

        // Start with always_on set.
        db.set_skill_override("mika", "qa-review", true).unwrap();
        // Layer an LLM override on top.
        db.set_skill_llm_override("mika", "qa-review", "anthropic", "claude-sonnet-4-6")
            .unwrap();

        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].always_on, Some(true));
        assert_eq!(overrides[0].llm_provider.as_deref(), Some("anthropic"));

        // Clearing LLM columns must NOT delete the row — always_on still set.
        db.delete_skill_llm_override("mika", "qa-review").unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].always_on, Some(true));
        assert_eq!(overrides[0].llm_provider, None);
        assert_eq!(overrides[0].llm_model, None);
    }

    #[test]
    fn test_skill_llm_override_case_insensitive_and_per_agent() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "").unwrap();
        db.register_agent("agent-b", "Agent B", "").unwrap();

        db.set_skill_llm_override("agent-a", "QA-Review", "anthropic", "claude-sonnet-4-6")
            .unwrap();
        db.set_skill_llm_override("agent-b", "qa-review", "deepseek", "deepseek-chat")
            .unwrap();

        let a = db.get_skill_overrides("AGENT-A").unwrap();
        let b = db.get_skill_overrides("agent-b").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].llm_provider.as_deref(), Some("anthropic"));
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].llm_provider.as_deref(), Some("deepseek"));
    }

    #[test]
    fn test_skill_override_per_agent_isolation() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "").unwrap();
        db.register_agent("agent-b", "Agent B", "").unwrap();

        db.set_skill_override("agent-a", "shell-exec", true)
            .unwrap();
        db.set_skill_override("agent-b", "shell-exec", false)
            .unwrap();

        let a_overrides = db.get_skill_overrides("agent-a").unwrap();
        let b_overrides = db.get_skill_overrides("agent-b").unwrap();
        assert_eq!(a_overrides[0].always_on, Some(true));
        assert_eq!(b_overrides[0].always_on, Some(false));
    }

    #[test]
    fn test_skill_override_case_insensitive() {
        let db = db();

        db.set_skill_override("mika", "Web-Search", true).unwrap();

        // Query with different case should find it
        let overrides = db.get_skill_overrides("MIKA").unwrap();
        assert_eq!(overrides.len(), 1);

        // Upsert with different case should update (not create duplicate)
        db.set_skill_override("MIKA", "web-search", false).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].always_on, Some(false));
    }

    #[test]
    fn test_skill_override_delete_nonexistent_is_noop() {
        let db = db();
        // Should not error
        db.delete_skill_override("mika", "nonexistent").unwrap();
    }

    #[test]
    fn test_set_skill_enabled_disable() {
        let mut db = db();
        db.set_skill_enabled("mika", "foo", false).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
        assert_eq!(ov.enabled, Some(false));
    }

    #[test]
    fn test_set_skill_enabled_enable_deletes_row() {
        let mut db = db();
        // Disable first
        db.set_skill_enabled("mika", "foo", false).unwrap();
        assert_eq!(db.get_skill_overrides("mika").unwrap().len(), 1);
        // Enable (default) — row should be deleted (default-equals-delete)
        db.set_skill_enabled("mika", "foo", true).unwrap();
        assert!(db.get_skill_overrides("mika").unwrap().is_empty());
    }

    #[test]
    fn test_set_skill_enabled_preserves_always_on() {
        let mut db = db();
        // Set always_on first
        db.set_skill_override("mika", "foo", true).unwrap();
        // Now disable
        db.set_skill_enabled("mika", "foo", false).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
        assert_eq!(ov.always_on, Some(true));
        assert_eq!(ov.enabled, Some(false));
    }

    /// Seed a `skill_overrides` row with explicit curator-relevant columns for
    /// the archival-candidate query tests (AC9–AC12). Bypasses the upsert
    /// helpers so `lifecycle_state` and `last_used_at` can be set to arbitrary
    /// values (the helpers only manage `always_on`/`llm_*`/`enabled`).
    fn seed_curator_skill_row(
        db: &Database,
        agent_id: &str,
        skill_name: &str,
        lifecycle_state: Option<&str>,
        use_count: i64,
        last_used_at: Option<&str>,
    ) {
        db.conn
            .execute(
                "INSERT INTO skill_overrides
                    (agent_id, skill_name, lifecycle_state, use_count, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    agent_id,
                    skill_name,
                    lifecycle_state,
                    use_count,
                    last_used_at
                ],
            )
            .unwrap();
    }

    #[test]
    fn test_v43_migration_adds_curator_columns() {
        // Mechanically enforce the schema the curator candidate query depends
        // on: migrate_v42_to_v43 must add lifecycle_state, use_count, and
        // last_used_at to skill_overrides. This dependency is self-contained in
        // mika#1584 — the migration adds all three columns here, so the query is
        // not coupled to any other PR's schema. (qa#1624: schema dependency
        // mechanically enforced.)
        let db = db();
        let cols: Vec<String> = db
            .conn
            .prepare("PRAGMA table_info('skill_overrides')")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for required in ["lifecycle_state", "use_count", "last_used_at"] {
            assert!(
                cols.iter().any(|c| c == required),
                "skill_overrides missing curator column '{required}' (v43 migration); have: {cols:?}"
            );
        }
    }

    #[test]
    fn test_archival_candidates_fresh_agent_returns_zero() {
        // AC9: a fresh agent with no agent-authored skills yields zero
        // candidates from the curator candidate query.
        let db = db();
        let candidates = db.get_archival_candidates("mika", 30).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_archival_candidates_staged_skill_excluded() {
        // AC10: a staged (not yet promoted) skill is excluded even when idle —
        // the query only considers lifecycle_state = 'active'.
        let db = db();
        let idle = crate::timestamp::now_minus(chrono::Duration::days(60));
        seed_curator_skill_row(&db, "mika", "staged-skill", Some("staged"), 0, Some(&idle));
        let candidates = db.get_archival_candidates("mika", 30).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_archival_candidates_idle_active_skill_returned() {
        // AC11: a promoted+active skill idle beyond the threshold (last used 40
        // days ago, threshold 30) is returned as exactly one candidate.
        let db = db();
        let idle = crate::timestamp::now_minus(chrono::Duration::days(40));
        seed_curator_skill_row(&db, "mika", "idle-skill", Some("active"), 3, Some(&idle));
        let candidates = db.get_archival_candidates("mika", 30).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].skill_name, "idle-skill");
        assert_eq!(candidates[0].lifecycle_state.as_deref(), Some("active"));
    }

    #[test]
    fn test_archival_candidates_bundled_null_lifecycle_excluded() {
        // AC12: a bundled/marketplace skill (NULL lifecycle_state, never used)
        // is excluded by construction — NULL never equals 'active'.
        let db = db();
        seed_curator_skill_row(&db, "mika", "bundled-skill", None, 0, None);
        let candidates = db.get_archival_candidates("mika", 30).unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_set_skill_enabled_enable_with_always_on_keeps_row() {
        let mut db = db();
        // Set both always_on and disabled
        db.set_skill_override("mika", "foo", true).unwrap();
        db.set_skill_enabled("mika", "foo", false).unwrap();
        // Now enable — row should remain because always_on is non-NULL
        db.set_skill_enabled("mika", "foo", true).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
        assert_eq!(ov.always_on, Some(true));
        assert_eq!(ov.enabled, None); // Cleared to NULL
    }

    #[test]
    fn test_set_skill_enabled_preserves_llm_override() {
        let mut db = db();
        db.set_skill_llm_override("mika", "foo", "anthropic", "claude-sonnet-4-6")
            .unwrap();
        db.set_skill_enabled("mika", "foo", false).unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
        assert_eq!(ov.llm_provider.as_deref(), Some("anthropic"));
        assert_eq!(ov.enabled, Some(false));
    }

    #[test]
    fn test_set_skill_enabled_round_trip() {
        let mut db = db();
        // Disable → enable → state is clean
        db.set_skill_enabled("mika", "bar", false).unwrap();
        assert_eq!(db.get_skill_overrides("mika").unwrap().len(), 1);
        db.set_skill_enabled("mika", "bar", true).unwrap();
        assert!(db.get_skill_overrides("mika").unwrap().is_empty());
    }

    #[test]
    fn test_enabled_column_exists_in_skill_overrides() {
        let db = db();
        assert!(db.column_exists("skill_overrides", "enabled").unwrap());
    }

    #[test]
    fn test_delete_skill_llm_override_with_enabled_keeps_row() {
        let mut db = db();
        db.set_skill_llm_override("mika", "foo", "anthropic", "claude-sonnet-4-6")
            .unwrap();
        db.set_skill_enabled("mika", "foo", false).unwrap();
        // Delete LLM override — row should remain because enabled is non-NULL
        db.delete_skill_llm_override("mika", "foo").unwrap();
        let overrides = db.get_skill_overrides("mika").unwrap();
        let ov = overrides.iter().find(|o| o.skill_name == "foo").unwrap();
        assert_eq!(ov.llm_provider, None);
        assert_eq!(ov.enabled, Some(false));
    }

    #[test]
    fn test_schema_version_is_current() {
        let db = db();
        assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_execution_trace_id_column_exists() {
        let db = db();
        assert!(db.column_exists("tasks", "execution_trace_id").unwrap());
    }

    #[test]
    fn test_parent_session_id_column_exists() {
        let db = db();
        assert!(db.column_exists("sessions", "parent_session_id").unwrap());
    }

    // ===== `tasks.type` column tests (issue #595, schema v23) =====

    #[test]
    fn test_tasks_type_column_exists() {
        let db = db();
        assert!(db.column_exists("tasks", "type").unwrap());
    }

    #[test]
    fn test_tasks_type_defaults_to_issue() {
        // Inserting via NewTask with r#type: None should backfill to 'issue'
        // via the SQL DEFAULT.
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "default-type".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(t.r#type, "issue");
    }

    #[test]
    fn test_tasks_type_round_trips_milestone() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "milestone-type".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: Some("milestone".to_string()),
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();
        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(t.r#type, "milestone");
    }

    #[test]
    fn test_tasks_type_check_constraint_rejects_invalid() {
        // Direct INSERT bypassing the tool boundary should still be blocked by
        // the SQLite CHECK constraint.
        let db = db();
        let result = db.conn.execute(
            "INSERT INTO tasks (id, agent_id, label, trigger_type, action_type, type)
             VALUES ('00000000-0000-0000-0000-000000000099', 'mika', 'bad', 'manual', 'none', 'epic')",
            [],
        );
        assert!(result.is_err(), "CHECK should reject 'epic'");
    }

    #[test]
    fn test_update_task_execution_trace_id() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test-exec-trace".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: Some("created-trace-aaa".to_string()),
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();

        // Initially execution_trace_id is None
        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert!(t.execution_trace_id.is_none());

        // Write execution trace_id
        db.update_task_execution_trace_id(&id, "exec-trace-bbb")
            .unwrap();

        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(t.execution_trace_id.as_deref(), Some("exec-trace-bbb"));
        // created_trace_id should be unchanged
        assert_eq!(t.created_trace_id.as_deref(), Some("created-trace-aaa"));
    }

    #[test]
    fn test_update_task_execution_trace_id_cross_agent() {
        // Verify execution_trace_id update does NOT scope by agent_id
        let db = db();
        db.register_agent("other", "Other", "").unwrap();

        let task = NewTask {
            agent_id: "other".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "cross-agent-test".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();

        // The "mika" agent can write execution_trace_id on a task owned by "other"
        db.update_task_execution_trace_id(&id, "cross-trace-123")
            .unwrap();

        let t = db.get_task(&id, "other").unwrap().unwrap();
        assert_eq!(t.execution_trace_id.as_deref(), Some("cross-trace-123"));
    }

    #[test]
    fn test_create_session_with_parent() {
        let db = db();
        // Create a parent session first
        db.create_session("parent-sess", "mika", "cli").unwrap();

        // Create a child session with parent reference and task linkage
        db.create_session_with_parent(
            "child-sess",
            "mika",
            "system",
            Some(r#"{"trigger": "callback"}"#),
            Some("parent-sess"),
            Some("task-123"),
        )
        .unwrap();

        let session = db.get_session("child-sess").unwrap().unwrap();
        assert_eq!(session.parent_session_id.as_deref(), Some("parent-sess"));
        assert_eq!(session.task_id.as_deref(), Some("task-123"));
        assert_eq!(
            session.metadata.as_deref(),
            Some(r#"{"trigger": "callback"}"#)
        );
    }

    #[test]
    fn test_create_session_with_parent_none() {
        let db = db();
        db.create_session_with_parent(
            "no-parent-sess",
            "mika",
            "system",
            Some(r#"{"trigger": "heartbeat"}"#),
            None,
            None,
        )
        .unwrap();

        let session = db.get_session("no-parent-sess").unwrap().unwrap();
        assert!(session.parent_session_id.is_none());
        assert!(session.task_id.is_none());
    }

    #[test]
    fn test_create_session_with_metadata_and_task_id() {
        let db = db();
        db.create_session_with_metadata(
            "meta-sess",
            "mika",
            "cli",
            Some(r#"{"task_id": "task-abc"}"#),
            Some("task-abc"),
        )
        .unwrap();

        let session = db.get_session("meta-sess").unwrap().unwrap();
        assert_eq!(session.task_id.as_deref(), Some("task-abc"));
        assert_eq!(
            session.metadata.as_deref(),
            Some(r#"{"task_id": "task-abc"}"#)
        );
    }

    #[test]
    fn test_get_sessions_for_task_tree() {
        let db = db();

        // Create a parent task
        let parent_id = db
            .create_task(&new_task("mika", "parent-work-item", "manual", "none"))
            .unwrap();

        // Create a child task
        let mut child = new_task("mika", "child-callback", "callback", "resume_agent");
        child.parent_task_id = Some(parent_id.clone());
        child.depth = 1;
        let child_id = db.create_task(&child).unwrap();

        // Create sessions linked to the task tree
        db.create_session_with_parent("sess-parent", "mika", "cli", None, None, Some(&parent_id))
            .unwrap();
        db.create_session_with_parent(
            "sess-child",
            "mika",
            "system",
            Some(r#"{"trigger": "callback"}"#),
            Some("sess-parent"),
            Some(&child_id),
        )
        .unwrap();
        // Unrelated session (no task_id)
        db.create_session("sess-unrelated", "mika", "cli").unwrap();

        let sessions = db.get_sessions_for_task_tree(&parent_id).unwrap();
        assert_eq!(sessions.len(), 2);

        // Both sessions should be found
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"sess-parent"));
        assert!(ids.contains(&"sess-child"));

        // Verify task labels are joined
        let parent_sess = sessions.iter().find(|s| s.id == "sess-parent").unwrap();
        assert_eq!(parent_sess.task_label.as_deref(), Some("parent-work-item"));
        let child_sess = sessions.iter().find(|s| s.id == "sess-child").unwrap();
        assert_eq!(child_sess.task_label.as_deref(), Some("child-callback"));
    }

    #[test]
    fn test_sessions_for_task_tree_backfill_compat() {
        let db = db();

        // Simulate a pre-v19 session with task_id only in metadata JSON
        let task_id = db
            .create_task(&new_task("mika", "legacy-task", "manual", "none"))
            .unwrap();

        // Create session with task_id only in metadata (legacy path)
        db.create_session_with_metadata(
            "legacy-sess",
            "mika",
            "cli",
            Some(&format!(r#"{{"task_id": "{task_id}"}}"#)),
            None, // no task_id column
        )
        .unwrap();

        // The COALESCE in the query should still find it
        let sessions = db.get_sessions_for_task_tree(&task_id).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "legacy-sess");
    }

    #[test]
    fn test_sessions_for_task_tree_includes_deep_descendants() {
        let db = db();

        // Create a 3-level tree: root → child → grandchild
        let root_id = db
            .create_task(&new_task("mika", "root-task", "manual", "none"))
            .unwrap();
        let mut child = new_task("mika", "child-task", "callback", "resume_agent");
        child.parent_task_id = Some(root_id.clone());
        child.depth = 1;
        let child_id = db.create_task(&child).unwrap();
        let mut grandchild = new_task("mika", "grandchild-task", "callback", "resume_agent");
        grandchild.parent_task_id = Some(child_id.clone());
        grandchild.depth = 2;
        let grandchild_id = db.create_task(&grandchild).unwrap();

        // Sessions at each level
        db.create_session_with_parent("sess-root", "mika", "cli", None, None, Some(&root_id))
            .unwrap();
        db.create_session_with_parent("sess-child", "mika", "system", None, None, Some(&child_id))
            .unwrap();
        db.create_session_with_parent(
            "sess-grandchild",
            "mika",
            "system",
            None,
            None,
            Some(&grandchild_id),
        )
        .unwrap();
        // Unrelated session
        db.create_session("sess-unrelated", "mika", "cli").unwrap();

        let sessions = db.get_sessions_for_task_tree(&root_id).unwrap();
        assert_eq!(sessions.len(), 3);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"sess-root"));
        assert!(ids.contains(&"sess-child"));
        assert!(ids.contains(&"sess-grandchild"));
    }

    #[test]
    fn test_list_sessions_paginated_coalesce_task_id() {
        let db = db();

        let task_id = db
            .create_task(&new_task("mika", "coalesce-test", "manual", "none"))
            .unwrap();

        // Session with task_id in column
        db.create_session_with_metadata("col-sess", "mika", "cli", None, Some(&task_id))
            .unwrap();
        // Session with task_id only in metadata (legacy)
        db.create_session_with_metadata(
            "meta-sess-2",
            "mika",
            "cli",
            Some(&format!(r#"{{"task_id": "{task_id}"}}"#)),
            None,
        )
        .unwrap();
        // Unrelated session
        db.create_session("other-sess", "mika", "cli").unwrap();

        let sessions = db
            .list_sessions_paginated(None, None, None, Some(&task_id), None, None, 50, 0)
            .unwrap();
        assert_eq!(sessions.len(), 2);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"col-sess"));
        assert!(ids.contains(&"meta-sess-2"));
    }

    #[test]
    fn test_sessions_time_range_filter() {
        let db = db();
        // Create sessions with known timestamps
        db.create_session("s1", "mika", "cli").unwrap();
        db.create_session("s2", "mika", "cli").unwrap();

        // Query with from/to set to a window that includes all sessions (now-1h to now+1h)
        let from_ts = crate::timestamp::now_minus(chrono::Duration::seconds(3600));
        let to_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
        let sessions = db
            .list_sessions_paginated(None, None, None, None, Some(&from_ts), Some(&to_ts), 50, 0)
            .unwrap();
        assert_eq!(sessions.len(), 2);

        // Query with from set to the far future — should return no sessions
        let future_ts = "2099-01-01T00:00:00Z";
        let sessions = db
            .list_sessions_paginated(None, None, None, None, Some(future_ts), None, 50, 0)
            .unwrap();
        assert!(sessions.is_empty());

        // Count should also respect the filter
        let count = db
            .count_sessions(None, None, None, None, Some(&from_ts), Some(&to_ts))
            .unwrap();
        assert_eq!(count, 2);
        let count = db
            .count_sessions(None, None, None, None, Some(future_ts), None)
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_tasks_time_range_filter() {
        let db = db();
        db.create_task(&new_task("mika", "task-a", "manual", "none"))
            .unwrap();
        db.create_task(&new_task("mika", "task-b", "manual", "none"))
            .unwrap();

        // Build filters with a wide time range
        let from_ts = crate::timestamp::now_minus(chrono::Duration::seconds(3600));
        let to_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
        let filters = TaskFilters {
            from: Some(from_ts),
            to: Some(to_ts),
            ..Default::default()
        };
        let (tasks, count) = db.list_tasks_paginated_with_count(&filters, 50, 0).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(count, 2);

        // Narrow to the far future — no matches
        let filters = TaskFilters {
            from: Some("2099-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        let (tasks, count) = db.list_tasks_paginated_with_count(&filters, 50, 0).unwrap();
        assert!(tasks.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn test_dev_runs_time_range_filter() {
        let db = db();
        // Create dev-run-shaped tasks (trigger_type=manual, source=self_dev)
        let mut t = new_task("mika", "dev-run-1", "manual", "none");
        t.source = Some("self_dev".to_string());
        db.create_task(&t).unwrap();
        let mut t2 = new_task("mika", "dev-run-2", "manual", "none");
        t2.source = Some("self_dev".to_string());
        db.create_task(&t2).unwrap();

        // Wide time range — should get both
        let from_ts = crate::timestamp::now_minus(chrono::Duration::seconds(3600));
        let to_ts = crate::timestamp::now_plus(chrono::Duration::seconds(3600));
        let (runs, count) = db
            .list_dev_runs_paginated_with_count(None, Some(&from_ts), Some(&to_ts), 50, 0)
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(count, 2);

        // Far future — no matches
        let (runs, count) = db
            .list_dev_runs_paginated_with_count(None, Some("2099-01-01T00:00:00Z"), None, 50, 0)
            .unwrap();
        assert!(runs.is_empty());
        assert_eq!(count, 0);
    }

    #[test]
    fn test_unified_timeline_uses_execution_trace_id() {
        let db = db();
        let task = NewTask {
            agent_id: "mika".to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "timeline-test".to_string(),
            trigger_type: "time".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: Some("created-trace-111".to_string()),
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        let id = db.create_task(&task).unwrap();

        // Before execution: unified_timeline should show created_trace_id
        let rows: Vec<(Option<String>,)> = db
            .conn
            .prepare("SELECT trace_id FROM unified_timeline WHERE event_type = 'task' AND summary LIKE 'timeline-test%'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?,)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.as_deref(), Some("created-trace-111"));

        // After execution: unified_timeline should prefer execution_trace_id
        db.update_task_execution_trace_id(&id, "exec-trace-222")
            .unwrap();

        let rows: Vec<(Option<String>,)> = db
            .conn
            .prepare("SELECT trace_id FROM unified_timeline WHERE event_type = 'task' AND summary LIKE 'timeline-test%'")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?,)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.as_deref(), Some("exec-trace-222"));
    }

    #[test]
    fn test_get_last_completed_team_run_no_runs() {
        let db = db();
        let result = db.get_last_completed_team_run("nonexistent-team").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_completed_team_run_filters_running() {
        let db = db();
        let run_id = "run-filter-1";
        db.insert_team_run(
            run_id,
            "filter-team",
            "test goal",
            3,
            "2020-01-01T00:00:00Z",
            None,
        )
        .unwrap();

        // Run is in "running" status — should not be returned
        let result = db.get_last_completed_team_run("filter-team").unwrap();
        assert!(result.is_none());

        // Complete the run
        db.update_team_run(
            run_id,
            "completed",
            None,
            2,
            Some("deliverable"),
            Some("2020-01-01T00:01:00Z"),
        )
        .unwrap();

        let result = db.get_last_completed_team_run("filter-team").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, run_id);
    }

    #[test]
    fn test_get_last_finished_team_run_excludes_suspended() {
        let db = db();
        // Insert a running run
        db.insert_team_run(
            "run-finished-1",
            "finished-team",
            "running goal",
            3,
            "2020-01-01T00:00:00Z",
            None,
        )
        .unwrap();

        // Running → should not be returned
        let result = db.get_last_finished_team_run("finished-team").unwrap();
        assert!(result.is_none());

        // Suspend it → still should not be returned
        db.update_team_run("run-finished-1", "suspended", None, 1, None, None)
            .unwrap();
        let result = db.get_last_finished_team_run("finished-team").unwrap();
        assert!(result.is_none());

        // Insert a completed run
        db.insert_team_run(
            "run-finished-2",
            "finished-team",
            "completed goal",
            3,
            "2020-01-01T00:01:00Z",
            None,
        )
        .unwrap();
        db.update_team_run(
            "run-finished-2",
            "completed",
            None,
            2,
            Some("done"),
            Some("2020-01-01T00:02:00Z"),
        )
        .unwrap();

        let result = db.get_last_finished_team_run("finished-team").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "run-finished-2");

        // Insert a cancelled run (newer) — should also be returned
        db.insert_team_run(
            "run-finished-3",
            "finished-team",
            "cancelled goal",
            3,
            "2020-01-01T00:03:00Z",
            None,
        )
        .unwrap();
        db.update_team_run(
            "run-finished-3",
            "cancelled",
            None,
            0,
            None,
            Some("2020-01-01T00:03:30Z"),
        )
        .unwrap();

        let result = db.get_last_finished_team_run("finished-team").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "run-finished-3");
    }

    #[test]
    fn test_get_team_run_summary_basic() {
        let db = db();
        let run_id = "run-summary-1";
        db.insert_team_run(
            run_id,
            "summary-team",
            "test goal for summary",
            3,
            "2020-01-01T00:00:00Z",
            None,
        )
        .unwrap();
        db.update_team_run(
            run_id,
            "completed",
            None,
            1,
            Some("final output"),
            Some("2020-01-01T00:01:00Z"),
        )
        .unwrap();

        let summary = db.get_team_run_summary(run_id).unwrap().unwrap();
        assert_eq!(summary.run.id, run_id);
        assert_eq!(summary.run.goal, "test goal for summary");
        assert_eq!(summary.run.deliverable.as_deref(), Some("final output"));
        assert!(summary.agent_results.is_empty());
        assert!(summary.task_statuses.is_empty());
        assert!(summary.pending_tasks.is_empty());
        assert!(summary.critic_feedback.is_none());
    }

    #[test]
    fn test_get_team_run_summary_with_critic() {
        let db = db();
        let run_id = "run-critic-1";
        db.insert_team_run(
            run_id,
            "critic-team",
            "critic test",
            3,
            "2020-01-01T00:00:00Z",
            None,
        )
        .unwrap();

        // Add critic feedback
        db.insert_team_workspace_entry(
            run_id,
            None,
            Some("critic"),
            "critic",
            "Needs improvement in error handling",
            1,
            None,
        )
        .unwrap();

        let summary = db.get_team_run_summary(run_id).unwrap().unwrap();
        assert_eq!(
            summary.critic_feedback.as_deref(),
            Some("Needs improvement in error handling")
        );
    }

    #[test]
    fn test_get_team_run_summary_not_found() {
        let db = db();
        let result = db.get_team_run_summary("nonexistent-run-id").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_truncate_chars_short() {
        assert_eq!(super::truncate_chars("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_chars_exact() {
        assert_eq!(super::truncate_chars("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_chars_long() {
        assert_eq!(super::truncate_chars("hello world", 5), "hello...");
    }

    #[test]
    fn test_create_session_if_not_exists() {
        let db = db();
        // First call creates the session
        db.create_session_if_not_exists(
            "test-idempotent",
            "mika",
            "team",
            Some(r#"{"trigger":"team"}"#),
        )
        .unwrap();
        // Second call should not error (INSERT OR IGNORE)
        db.create_session_if_not_exists(
            "test-idempotent",
            "mika",
            "team",
            Some(r#"{"trigger":"team"}"#),
        )
        .unwrap();
        // Verify session exists
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'test-idempotent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_end_session_sets_ended_at() {
        let db = db();
        db.create_session("end-test", "mika", "system").unwrap();
        // ended_at should be NULL initially
        let ended: Option<String> = db
            .conn
            .query_row(
                "SELECT ended_at FROM sessions WHERE id = 'end-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ended.is_none());
        // End the session
        db.end_session("end-test").unwrap();
        let ended: Option<String> = db
            .conn
            .query_row(
                "SELECT ended_at FROM sessions WHERE id = 'end-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ended.is_some());
    }

    #[test]
    fn test_prune_old_sessions() {
        let db = db();
        // Create two ended sessions and one active (not ended)
        db.create_session("heartbeat-old", "mika", "system")
            .unwrap();
        db.create_session("heartbeat-new", "mika", "system")
            .unwrap();
        db.create_session("heartbeat-active", "mika", "system")
            .unwrap();
        // Save a message to heartbeat-old to verify cascade delete
        db.save_message("mika", "heartbeat-old", "user", "test msg", None)
            .unwrap();

        // End two sessions, but make one "old" by backdating ended_at
        db.end_session("heartbeat-old").unwrap();
        db.end_session("heartbeat-new").unwrap();
        // Backdate heartbeat-old to 10 days ago
        db.conn
            .execute(
                "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = 'heartbeat-old'",
                [],
            )
            .unwrap();

        // Prune with 7-day retention
        let pruned = db.prune_old_sessions(7 * 24 * 60 * 60).unwrap();
        assert_eq!(pruned, 1); // Only heartbeat-old should be pruned

        // Verify heartbeat-old is gone (and its message cascaded)
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'heartbeat-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);

        let msg_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = 'heartbeat-old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(msg_count, 0);

        // Verify heartbeat-new still exists (ended recently)
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'heartbeat-new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify heartbeat-active still exists (not ended)
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'heartbeat-active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_prune_targets_correct_prefixes() {
        let db = db();
        // Create sessions with various prefixes
        for prefix in &[
            "heartbeat-1",
            "callback-1",
            "skill-test-1",
            "reflection-2026-01-01",
            "team-run1-agent1",
        ] {
            db.create_session(prefix, "mika", "system").unwrap();
            db.end_session(prefix).unwrap();
            // Backdate to 10 days ago
            db.conn
                .execute(
                    &format!(
                        "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = '{prefix}'"
                    ),
                    [],
                )
                .unwrap();
        }
        // Delegate session uses "delegate" channel (not "system")
        db.create_session("delegate-task-1", "mika", "delegate")
            .unwrap();
        db.end_session("delegate-task-1").unwrap();
        db.conn
            .execute(
                "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = 'delegate-task-1'",
                [],
            )
            .unwrap();
        // Also create a regular CLI session that should NOT be pruned
        db.create_session("cli-session", "mika", "cli").unwrap();
        db.end_session("cli-session").unwrap();
        db.conn
            .execute(
                "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 days') WHERE id = 'cli-session'",
                [],
            )
            .unwrap();

        let pruned = db.prune_old_sessions(7 * 24 * 60 * 60).unwrap();
        assert_eq!(pruned, 6); // All prefixed sessions pruned, CLI session preserved

        // CLI session should still exist
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'cli-session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // -- Task health summary tests --

    fn new_task(agent: &str, label: &str, trigger: &str, action: &str) -> NewTask {
        NewTask {
            agent_id: agent.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: trigger.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[test]
    fn test_health_summary_empty_no_anomalies() {
        let db = db();
        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(summary.active_tasks.is_empty());
        assert!(summary.anomalies.is_empty());
    }

    #[test]
    fn test_health_summary_active_tasks() {
        let db = db();
        let task = NewTask {
            reference_url: Some("https://github.com/org/repo/issues/1".to_string()),
            ..new_task("mika", "Fix bug", "manual", "none")
        };
        db.create_task(&task).unwrap();
        let summary = db.get_task_health_summary("mika").unwrap();
        assert_eq!(summary.active_tasks.len(), 1);
        assert_eq!(summary.active_tasks[0].label, "Fix bug");
        // Also detected as github_linked anomaly
        assert!(
            summary
                .anomalies
                .iter()
                .any(|a| a.anomaly_type == "github_linked")
        );
    }

    #[test]
    fn test_health_summary_stuck_callback() {
        let db = db();
        let id = db
            .create_task(&new_task(
                "mika",
                "Build deploy",
                "callback",
                "resume_agent",
            ))
            .unwrap();
        // Mark as completed with a timestamp >10 min ago
        let old_time = timestamp::format(&(Utc::now() - Duration::seconds(700)));
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![old_time, id],
            )
            .unwrap();

        let summary = db.get_task_health_summary("mika").unwrap();
        assert_eq!(summary.anomalies.len(), 1);
        assert_eq!(summary.anomalies[0].anomaly_type, "stuck_callback");
        assert_eq!(summary.anomalies[0].label, "Build deploy");
        assert!(summary.anomalies[0].age_description.starts_with("stuck "));
    }

    #[test]
    fn test_health_summary_stuck_callback_not_triggered_within_threshold() {
        let db = db();
        let id = db
            .create_task(&new_task(
                "mika",
                "Recent callback",
                "callback",
                "resume_agent",
            ))
            .unwrap();
        // Mark as completed just 2 minutes ago (within 10 min threshold)
        let recent_time = timestamp::format(&(Utc::now() - Duration::seconds(120)));
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![recent_time, id],
            )
            .unwrap();

        let summary = db.get_task_health_summary("mika").unwrap();
        // Should NOT appear as stuck_callback
        assert!(
            summary
                .anomalies
                .iter()
                .all(|a| a.anomaly_type != "stuck_callback")
        );
    }

    #[test]
    fn test_health_summary_failed_recurring() {
        let db = db();
        let id = db
            .create_task(&new_task("mika", "Heartbeat", "recurring", "run_skill"))
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'failed' WHERE id = ?1",
                params![id],
            )
            .unwrap();

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .any(|a| a.anomaly_type == "failed_recurring")
        );
    }

    #[test]
    fn test_health_summary_long_running() {
        let db = db();
        let id = db
            .create_task(&new_task("mika", "Slow task", "time", "run_skill"))
            .unwrap();
        let old_time = timestamp::format(&(Utc::now() - Duration::seconds(7200)));
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress', fired_at = ?1 WHERE id = ?2",
                params![old_time, id],
            )
            .unwrap();

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .any(|a| a.anomaly_type == "long_running")
        );
    }

    #[test]
    fn test_health_summary_stale_blocked() {
        let db = db();
        let id = db
            .create_task(&new_task("mika", "Blocked item", "manual", "none"))
            .unwrap();
        let old_time = timestamp::format(&(Utc::now() - Duration::seconds(90_000)));
        db.conn
            .execute(
                "UPDATE tasks SET status = 'blocked', updated_at = ?1 WHERE id = ?2",
                params![old_time, id],
            )
            .unwrap();

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .any(|a| a.anomaly_type == "stale_blocked")
        );
    }

    #[test]
    fn test_health_summary_agent_scoping() {
        let db = db();
        // Register a second agent
        db.conn
            .execute(
                "INSERT OR IGNORE INTO agents (id, name) VALUES ('other', 'Other')",
                [],
            )
            .unwrap();
        // Create a stuck callback for "other" agent
        let id = db
            .create_task(&new_task(
                "other",
                "Other build",
                "callback",
                "resume_agent",
            ))
            .unwrap();
        let old_time = timestamp::format(&(Utc::now() - Duration::seconds(700)));
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                params![old_time, id],
            )
            .unwrap();

        // Mika should see no anomalies
        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(summary.anomalies.is_empty());

        // Other should see the stuck callback
        let summary = db.get_task_health_summary("other").unwrap();
        assert_eq!(summary.anomalies.len(), 1);
        assert_eq!(summary.anomalies[0].anomaly_type, "stuck_callback");
    }

    #[test]
    fn test_health_summary_anomaly_cap() {
        let db = db();
        // Create 15 stuck callbacks — only 10 should be returned
        for i in 0..15 {
            let id = db
                .create_task(&new_task(
                    "mika",
                    &format!("Build {i}"),
                    "callback",
                    "resume_agent",
                ))
                .unwrap();
            let old_time = timestamp::format(&(Utc::now() - Duration::seconds(700 + i * 60)));
            db.conn
                .execute(
                    "UPDATE tasks SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                    params![old_time, id],
                )
                .unwrap();
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(summary.anomalies.len() <= 10);
    }

    #[test]
    fn test_format_age_hours_minutes() {
        let now = Utc::now();
        let ts = timestamp::format(&(now - Duration::seconds(7200 + 1320)));
        assert_eq!(format_age(&ts, now), "2h 22m");
    }

    #[test]
    fn test_format_age_days() {
        let now = Utc::now();
        let ts = timestamp::format(&(now - Duration::seconds(86_400 * 5)));
        assert_eq!(format_age(&ts, now), "5d");
    }

    #[test]
    fn test_format_age_invalid_timestamp() {
        let now = Utc::now();
        assert_eq!(format_age("not-a-timestamp", now), "unknown");
    }

    // -- Dispatch failure anomaly tests (#980) --

    /// Helper: insert a tool_call row directly for testing anomaly #7.
    fn insert_tool_call(
        db: &Database,
        agent_id: &str,
        session_id: &str,
        tool_name: &str,
        success: bool,
        created_at: &str,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        db.conn
            .execute(
                "INSERT INTO tool_calls (id, agent_id, session_id, tool_name, tool_source, success, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'builtin', ?5, ?6)",
                params![id, agent_id, session_id, tool_name, success as i32, created_at],
            )
            .unwrap();
    }

    #[test]
    fn test_dispatch_failures_below_threshold_no_anomaly() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // 2 recent failures — below threshold of 3
        let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .all(|a| a.anomaly_type != "dispatch_failures"),
            "should not fire dispatch_failures with only 2 failures"
        );
    }

    #[test]
    fn test_dispatch_failures_at_threshold() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // 3 recent failures — at threshold
        let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
        for _ in 0..3 {
            insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        let anomaly = summary
            .anomalies
            .iter()
            .find(|a| a.anomaly_type == "dispatch_failures");
        assert!(
            anomaly.is_some(),
            "should fire dispatch_failures at threshold 3"
        );
        assert_eq!(anomaly.unwrap().age_description, "3 failures in last 2h");
    }

    #[test]
    fn test_dispatch_failures_above_threshold_shows_count() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // 6 recent failures — above threshold
        let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
        for _ in 0..6 {
            insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        let anomaly = summary
            .anomalies
            .iter()
            .find(|a| a.anomaly_type == "dispatch_failures")
            .expect("should fire dispatch_failures");
        assert_eq!(anomaly.age_description, "6 failures in last 2h");
    }

    #[test]
    fn test_dispatch_failures_outside_window_no_anomaly() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // 3 failures older than 2h window
        let old = timestamp::format(&(Utc::now() - Duration::seconds(8000)));
        for _ in 0..3 {
            insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &old);
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .all(|a| a.anomaly_type != "dispatch_failures"),
            "failures outside 2h window should not trigger dispatch_failures"
        );
    }

    #[test]
    fn test_dispatch_failures_mixed_success_counts_only_failures() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
        // 2 failures + 3 successes — only 2 failures, below threshold
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .all(|a| a.anomaly_type != "dispatch_failures"),
            "should count only failures, not successes"
        );
    }

    #[test]
    fn test_dispatch_failures_task_correlation_via_session_join() {
        let db = db();
        let session_id = "task-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // Create an in_progress manual task and link session to it
        let task_id = db
            .create_task(&new_task("mika", "Fix issue #42", "manual", "none"))
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![task_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE sessions SET task_id = ?1 WHERE id = ?2",
                params![task_id, session_id],
            )
            .unwrap();

        // 3 failures in that session
        let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
        for _ in 0..3 {
            insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        let anomaly = summary
            .anomalies
            .iter()
            .find(|a| a.anomaly_type == "dispatch_failures")
            .expect("should fire dispatch_failures with task correlation");
        assert_eq!(anomaly.task_id, task_id);
        assert_eq!(anomaly.label, "Fix issue #42");
    }

    #[test]
    fn test_dispatch_stale_fires_when_no_recent_dispatch() {
        let db = db();

        // Create an in_progress manual task
        let task_id = db
            .create_task(&new_task("mika", "Stale work item", "manual", "none"))
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![task_id],
            )
            .unwrap();

        // No run_claude_pilot calls at all → stale dispatch
        let summary = db.get_task_health_summary("mika").unwrap();
        let anomaly = summary
            .anomalies
            .iter()
            .find(|a| a.anomaly_type == "dispatch_stale");
        assert!(
            anomaly.is_some(),
            "should fire dispatch_stale when no dispatch attempt in >1h"
        );
        assert_eq!(anomaly.unwrap().task_id, task_id);
        assert_eq!(
            anomaly.unwrap().age_description,
            "no dispatch attempt in >1h"
        );
    }

    #[test]
    fn test_dispatch_stale_not_fired_when_recent_dispatch_exists() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // Create an in_progress manual task
        let task_id = db
            .create_task(&new_task("mika", "Active work item", "manual", "none"))
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![task_id],
            )
            .unwrap();

        // Recent successful dispatch within 1h
        let recent = timestamp::format(&(Utc::now() - Duration::seconds(1800)));
        insert_tool_call(&db, "mika", session_id, "run_claude_pilot", true, &recent);

        let summary = db.get_task_health_summary("mika").unwrap();
        assert!(
            summary
                .anomalies
                .iter()
                .all(|a| a.anomaly_type != "dispatch_stale"),
            "should not fire dispatch_stale when recent dispatch exists"
        );
    }

    #[test]
    fn test_dispatch_signal_a_suppresses_signal_b() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // Create an in_progress manual task
        let task_id = db
            .create_task(&new_task("mika", "Work item", "manual", "none"))
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![task_id],
            )
            .unwrap();

        // 3 recent failures (Signal A fires) but also no recent dispatch (Signal B would fire)
        // Only Signal A should appear
        let recent = timestamp::format(&(Utc::now() - Duration::seconds(600)));
        for _ in 0..3 {
            insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &recent);
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        let dispatch_anomalies: Vec<_> = summary
            .anomalies
            .iter()
            .filter(|a| a.anomaly_type == "dispatch_failures" || a.anomaly_type == "dispatch_stale")
            .collect();
        assert_eq!(
            dispatch_anomalies.len(),
            1,
            "Signal A should suppress Signal B"
        );
        assert_eq!(dispatch_anomalies[0].anomaly_type, "dispatch_failures");
    }

    #[test]
    fn test_dispatch_stale_fires_when_failures_aged_out() {
        let db = db();
        let session_id = "test-session";
        db.create_session(session_id, "mika", "cli").unwrap();

        // Create an in_progress manual task
        let task_id = db
            .create_task(&new_task("mika", "Aged out work", "manual", "none"))
            .unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![task_id],
            )
            .unwrap();

        // 3 failures older than 2h window (aged out of Signal A)
        // AND older than 1h (stale for Signal B)
        let old = timestamp::format(&(Utc::now() - Duration::seconds(8000)));
        for _ in 0..3 {
            insert_tool_call(&db, "mika", session_id, "run_claude_pilot", false, &old);
        }

        let summary = db.get_task_health_summary("mika").unwrap();
        // Signal A should NOT fire (aged out of 2h window)
        assert!(
            summary
                .anomalies
                .iter()
                .all(|a| a.anomaly_type != "dispatch_failures"),
            "Signal A should not fire for aged-out failures"
        );
        // Signal B SHOULD fire (no recent dispatch in >1h, in_progress task exists)
        let stale = summary
            .anomalies
            .iter()
            .find(|a| a.anomaly_type == "dispatch_stale");
        assert!(
            stale.is_some(),
            "Signal B (dispatch_stale) should fire when failures aged out — this is the aging defense"
        );
        assert_eq!(stale.unwrap().task_id, task_id);
    }

    // ===== Internal message tests (#494) =====

    #[test]
    fn test_internal_column_exists() {
        let db = db();
        assert!(db.column_exists("messages", "internal").unwrap());
    }

    #[test]
    fn test_save_internal_message() {
        let (db, sid) = db_with_session();
        db.save_message_with_metadata("mika", &sid, "assistant", "internal msg", None, None, true)
            .unwrap();
        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].internal);
        assert_eq!(msgs[0].content, "internal msg");
    }

    #[test]
    fn test_save_internal_message_with_metadata() {
        let (db, sid) = db_with_session();
        db.save_message_with_metadata(
            "mika",
            &sid,
            "assistant",
            "internal with meta",
            Some(r#"{"tool_calls":[]}"#),
            None,
            true,
        )
        .unwrap();
        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].internal);
        assert!(msgs[0].metadata.is_some());
    }

    #[test]
    fn test_save_message_defaults_not_internal() {
        let (db, sid) = db_with_session();
        db.save_message("mika", &sid, "user", "hello", None)
            .unwrap();
        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(!msgs[0].internal);
    }

    #[test]
    fn test_load_recent_messages_filtered_excludes_internal() {
        let (db, sid) = db_with_session();
        db.save_message("mika", &sid, "user", "visible 1", None)
            .unwrap();
        db.save_message_with_metadata("mika", &sid, "assistant", "hidden", None, None, true)
            .unwrap();
        db.save_message("mika", &sid, "assistant", "visible 2", None)
            .unwrap();

        // Without filter: all 3, hidden count 0
        let (all, hidden) = db.load_recent_messages_filtered("mika", 10, false).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(hidden, 0);

        // With filter: only 2 visible, 1 hidden
        let (visible, hidden) = db.load_recent_messages_filtered("mika", 10, true).unwrap();
        assert_eq!(visible.len(), 2);
        assert_eq!(hidden, 1);
        assert!(visible.iter().all(|m| !m.internal));
    }

    #[test]
    fn test_load_recent_messages_filtered_hidden_count_empty() {
        let (db, _sid) = db_with_session();
        let (msgs, hidden) = db.load_recent_messages_filtered("mika", 20, true).unwrap();
        assert!(msgs.is_empty());
        assert_eq!(hidden, 0);
    }

    #[test]
    fn test_load_recent_messages_filtered_hidden_count_limit() {
        let (db, sid) = db_with_session();
        // Create 5 internal + 5 visible messages
        for i in 0..5 {
            db.save_message_with_metadata(
                "mika",
                &sid,
                "user",
                &format!("internal {i}"),
                None,
                None,
                true,
            )
            .unwrap();
            db.save_message("mika", &sid, "assistant", &format!("visible {i}"), None)
                .unwrap();
        }

        // Limit 10 fetches all 10 rows from DB; 5 visible returned, 5 hidden counted
        let (visible, hidden) = db.load_recent_messages_filtered("mika", 10, true).unwrap();
        assert_eq!(visible.len(), 5);
        assert_eq!(hidden, 5);

        // Without filter: all 10 returned, 0 hidden
        let (all, hidden) = db.load_recent_messages_filtered("mika", 10, false).unwrap();
        assert_eq!(all.len(), 10);
        assert_eq!(hidden, 0);
    }

    // -- find_active_task_by_pr_url tests --

    #[test]
    fn test_find_active_task_by_pr_url_found() {
        let db = db();
        let pr_url = "https://github.com/senara-solutions/mika/pull/42";
        let task = new_task("mika", "Implement feature", "manual", "none");
        let id = db.create_task(&task).unwrap();
        let meta =
            r#"{"claude_pilot":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
        db.update_task_metadata(&id, meta).unwrap();

        let found = db.find_active_task_by_pr_url("mika", pr_url).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, id);
    }

    #[test]
    fn test_find_active_task_by_pr_url_not_found() {
        let db = db();
        let task = new_task("mika", "Implement feature", "manual", "none");
        let id = db.create_task(&task).unwrap();
        let meta =
            r#"{"claude_pilot":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
        db.update_task_metadata(&id, meta).unwrap();

        let found = db
            .find_active_task_by_pr_url("mika", "https://github.com/senara-solutions/mika/pull/99")
            .unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_find_active_task_by_pr_url_completed_not_returned() {
        let db = db();
        let task = new_task("mika", "Done feature", "manual", "none");
        let id = db.create_task(&task).unwrap();
        let meta =
            r#"{"claude_pilot":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
        db.update_task_metadata(&id, meta).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed' WHERE id = ?1",
                params![id],
            )
            .unwrap();

        let found = db
            .find_active_task_by_pr_url("mika", "https://github.com/senara-solutions/mika/pull/42")
            .unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_find_active_task_by_pr_url_wrong_metadata_path() {
        let db = db();
        let task = new_task("mika", "Wrong path", "manual", "none");
        let id = db.create_task(&task).unwrap();
        // pr_url in a different metadata path (not under claude_pilot)
        let meta = r#"{"other":{"pr_url":"https://github.com/senara-solutions/mika/pull/42"}}"#;
        db.update_task_metadata(&id, meta).unwrap();

        let found = db
            .find_active_task_by_pr_url("mika", "https://github.com/senara-solutions/mika/pull/42")
            .unwrap();
        assert!(found.is_none());
    }

    // -- find_active_task_by_branch tests --

    #[test]
    fn test_find_active_task_by_branch_found() {
        let db = db();
        let branch = "feat/test";
        let task = new_task("mika", "Implement feature", "manual", "none");
        let id = db.create_task(&task).unwrap();
        let meta = r#"{"claude_pilot":{"branch":"feat/test"}}"#;
        db.update_task_metadata(&id, meta).unwrap();

        let found = db.find_active_task_by_branch("mika", branch).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, id);
    }

    #[test]
    fn test_find_active_task_by_branch_not_found() {
        let db = db();
        let task = new_task("mika", "Implement feature", "manual", "none");
        let id = db.create_task(&task).unwrap();
        let meta = r#"{"claude_pilot":{"branch":"feat/test"}}"#;
        db.update_task_metadata(&id, meta).unwrap();

        let found = db
            .find_active_task_by_branch("mika", "feat/other-branch")
            .unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_find_active_task_by_branch_completed_not_returned() {
        let db = db();
        let task = new_task("mika", "Done feature", "manual", "none");
        let id = db.create_task(&task).unwrap();
        let meta = r#"{"claude_pilot":{"branch":"feat/test"}}"#;
        db.update_task_metadata(&id, meta).unwrap();
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed' WHERE id = ?1",
                params![id],
            )
            .unwrap();

        let found = db.find_active_task_by_branch("mika", "feat/test").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_find_active_task_by_branch_wrong_path() {
        let db = db();
        let task = new_task("mika", "Wrong path", "manual", "none");
        let id = db.create_task(&task).unwrap();
        // branch in a different metadata path (not under claude_pilot)
        let meta = r#"{"other":{"branch":"feat/test"}}"#;
        db.update_task_metadata(&id, meta).unwrap();

        let found = db.find_active_task_by_branch("mika", "feat/test").unwrap();
        assert!(found.is_none());
    }

    // ===== KG Schema Migration Tests (v24 → v25 forward-test harness) =====

    /// A structural fingerprint of a SQLite table, including columns, indexes,
    /// and foreign keys. Used for migration convergence testing.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TableSnapshot {
        name: String,
        columns: Vec<ColumnInfo>,
        indexes: Vec<IndexInfo>,
        foreign_keys: Vec<ForeignKeyInfo>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct ColumnInfo {
        name: String,
        col_type: String,
        not_null: bool,
        default_value: Option<String>,
        pk: i32,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct IndexInfo {
        name: String,
        unique: bool,
        columns: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct ForeignKeyInfo {
        from_col: String,
        to_table: String,
        to_col: String,
        on_delete: String,
    }

    /// Snapshot the full structural schema of a database for comparison.
    fn snapshot_schema(conn: &rusqlite::Connection) -> Vec<TableSnapshot> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        let table_names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        let mut snapshots = Vec::new();
        for name in table_names {
            // Skip virtual tables (fts_search, vec_search) — they don't support PRAGMA introspection
            // Skip v26 backup tables left by migrate_v26_to_v27 (pending #787 coalesce)
            if name == "fts_search"
                || name == "vec_search"
                || name.starts_with("fts_search_")
                || name.starts_with("vec_search_")
                || name.ends_with("_v26_backup")
            {
                continue;
            }

            let mut columns: Vec<ColumnInfo> = {
                let mut s = conn
                    .prepare(&format!("PRAGMA table_info('{name}')"))
                    .unwrap();
                s.query_map([], |r| {
                    Ok(ColumnInfo {
                        name: r.get(1)?,
                        col_type: r.get(2)?,
                        not_null: r.get::<_, bool>(3)?,
                        default_value: r.get(4)?,
                        pk: r.get(5)?,
                    })
                })
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
            };
            columns.sort();

            let mut indexes: Vec<IndexInfo> = {
                let mut s = conn
                    .prepare(&format!("PRAGMA index_list('{name}')"))
                    .unwrap();
                let raw: Vec<(String, bool)> = s
                    .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect();
                raw.into_iter()
                    .map(|(idx_name, unique)| {
                        let mut si = conn
                            .prepare(&format!("PRAGMA index_info('{idx_name}')"))
                            .unwrap();
                        let cols: Vec<String> = si
                            .query_map([], |r| r.get(2))
                            .unwrap()
                            .map(|r| r.unwrap())
                            .collect();
                        IndexInfo {
                            name: idx_name,
                            unique,
                            columns: cols,
                        }
                    })
                    .collect()
            };
            indexes.sort();

            let mut foreign_keys: Vec<ForeignKeyInfo> = {
                let mut s = conn
                    .prepare(&format!("PRAGMA foreign_key_list('{name}')"))
                    .unwrap();
                s.query_map([], |r| {
                    Ok(ForeignKeyInfo {
                        from_col: r.get(3)?,
                        to_table: r.get(2)?,
                        to_col: r.get(4)?,
                        on_delete: r.get(6)?,
                    })
                })
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
            };
            foreign_keys.sort();

            snapshots.push(TableSnapshot {
                name,
                columns,
                indexes,
                foreign_keys,
            });
        }
        snapshots.sort_by(|a, b| a.name.cmp(&b.name));
        snapshots
    }

    /// All ten KG tables added in v25.
    const KG_TABLES: &[&str] = &[
        "kg_entities",
        "kg_relationships",
        "kg_chunks",
        "kg_subject_entities",
        "kg_subject_resolutions",
        "kg_subject_relationships",
        "kg_chunk_subjects",
        "kg_chunk_subject_relationships",
        "kg_extractions",
        "kg_resolutions_log",
    ];

    fn table_exists(conn: &rusqlite::Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    fn index_exists(conn: &rusqlite::Connection, index: &str) -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?1",
            [index],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            > 0
    }

    #[test]
    fn test_v24_to_v25_migration_adds_kg_tables() {
        let db = db();
        // Fresh DB starts at v25 via migrate_v1 — tables should exist
        for table in KG_TABLES {
            assert!(
                table_exists(&db.conn, table),
                "KG table '{table}' should exist after fresh DB creation"
            );
        }

        // Verify key indexes exist
        let expected_indexes = [
            "idx_kg_entities_type",
            "idx_kg_rel_from",
            "idx_kg_rel_to",
            "idx_kg_chunks_docs_root_hash_doc",
            "idx_kg_subj_entities_drh_type",
            "idx_kg_resolutions_agent_subj",
            "idx_kg_resolutions_agent_dom",
            "idx_kg_subj_rel_from",
            "idx_kg_subj_rel_to",
            "idx_kg_subj_rel_type",
            "idx_kg_cs_chunk",
            "idx_kg_cs_entity",
            "idx_kg_cs_trace",
            "idx_kg_csr_chunk",
            "idx_kg_csr_rel",
            "idx_kg_extractions_drh",
            "idx_kg_res_log_pending",
        ];
        for idx in expected_indexes {
            assert!(index_exists(&db.conn, idx), "KG index '{idx}' should exist");
        }
    }

    #[test]
    fn test_v25_entity_key_check_constraint() {
        let db = db();
        // Valid entity — should succeed
        db.conn
            .execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('skill:self-dev', 'skill', 'self-dev')",
                [],
            )
            .expect("valid entity should insert");

        // Invalid entity — entity_key doesn't match type:name
        let result = db.conn.execute(
            "INSERT INTO kg_entities (entity_key, type, name) VALUES ('wrong-key', 'skill', 'self-dev')",
            [],
        );
        assert!(
            result.is_err(),
            "entity_key CHECK constraint should reject mismatched key"
        );
    }

    #[test]
    fn test_v25_subject_entity_confidence_constraint() {
        let db = db();
        db.conn
            .execute(
                "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES ('0000000000000000', '/test', 'failure_mode:oom', 'failure_mode', 'oom', 0.85)",
                [],
            )
            .expect("valid confidence should insert");

        // Confidence > 1.0 should fail
        let result = db.conn.execute(
            "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES ('0000000000000000', '/test', 'failure_mode:crash', 'failure_mode', 'crash', 1.5)",
            [],
        );
        assert!(
            result.is_err(),
            "confidence > 1.0 should be rejected by CHECK constraint"
        );

        // Confidence < 0.0 should fail
        let result = db.conn.execute(
            "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES ('0000000000000000', '/test', 'failure_mode:hang', 'failure_mode', 'hang', -0.1)",
            [],
        );
        assert!(
            result.is_err(),
            "confidence < 0.0 should be rejected by CHECK constraint"
        );
    }

    #[test]
    fn test_v25_kg_chunks_agent_cascade_delete() {
        let db = db();
        // Enable FK enforcement
        db.conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        db.conn
            .execute(
                "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                 VALUES ('0000000000000000', '/test', 0, 'docs/test.md', 'abc123hash')",
                [],
            )
            .unwrap();

        let count: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM kg_chunks WHERE docs_root_hash = '0000000000000000'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Delete the agent — v27 shared-layer chunks should NOT cascade
        db.conn
            .execute("DELETE FROM agents WHERE id = 'mika'", [])
            .unwrap();

        let count: i64 = db
            .conn
            .query_row("SELECT count(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "kg_chunks should persist after agent deletion (v27 shared-layer)"
        );
    }

    #[test]
    fn test_v25_kg_relationships_entity_cascade_delete() {
        let db = db();
        db.conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        db.conn
            .execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('skill:a', 'skill', 'a')",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO kg_entities (entity_key, type, name) VALUES ('tool:b', 'tool', 'b')",
                [],
            )
            .unwrap();

        let from_id: i64 = db
            .conn
            .query_row(
                "SELECT id FROM kg_entities WHERE entity_key = 'skill:a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let to_id: i64 = db
            .conn
            .query_row(
                "SELECT id FROM kg_entities WHERE entity_key = 'tool:b'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO kg_relationships (from_entity_id, to_entity_id, type) VALUES (?1, ?2, 'PROVIDES')",
                rusqlite::params![from_id, to_id],
            )
            .unwrap();

        // Delete from_entity — relationship should cascade
        db.conn
            .execute("DELETE FROM kg_entities WHERE entity_key = 'skill:a'", [])
            .unwrap();

        let count: i64 = db
            .conn
            .query_row("SELECT count(*) FROM kg_relationships", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "kg_relationships should cascade-delete when from_entity is deleted"
        );
    }

    #[test]
    fn test_v25_resolutions_log_outcome_check() {
        let db = db();
        db.conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();

        // Valid outcome
        db.conn
            .execute(
                "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
                 VALUES ('mika', 1, 'matched_exact', 'trace-1')",
                [],
            )
            .expect("valid outcome should insert");

        // Invalid outcome
        let result = db.conn.execute(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 2, 'invalid_outcome', 'trace-2')",
            [],
        );
        assert!(
            result.is_err(),
            "invalid outcome should be rejected by CHECK constraint"
        );
    }

    #[test]
    fn test_v1_and_incremental_schemas_converge() {
        // DB1: fresh install via migrate_v1 (reaches CURRENT_SCHEMA_VERSION directly).
        let db1 = Database::open_in_memory().unwrap();
        let snap1 = snapshot_schema(&db1.conn);

        // DB2: simulate a v24 DB, then migrate incrementally through v25,
        // v26 (#757), and v27 (#786). Strategy: create a fresh DB, extract all non-KG DDL from
        // sqlite_master, replay it on a new connection to get a v24 DB, then
        // run migrate_v24_to_v25 and migrate_v25_to_v26 and compare schemas.
        let fresh = Database::open_in_memory().unwrap();
        let mut stmt = fresh
            .conn
            .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid")
            .unwrap();
        let ddl_stmts: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        drop(stmt);

        init_sqlite_vec();
        let conn2 = rusqlite::Connection::open_in_memory().unwrap();
        conn2.execute_batch("PRAGMA foreign_keys = ON;").unwrap();

        // Replay all DDL except KG tables, virtual tables, views, and schema_meta
        conn2.execute_batch("BEGIN;").unwrap();
        for ddl in &ddl_stmts {
            let lower = ddl.to_lowercase();
            if lower.contains("kg_entities")
                || lower.contains("kg_relationships")
                || lower.contains("kg_chunks")
                || lower.contains("kg_subject_")
                || lower.contains("kg_chunk_subject")
                || lower.contains("kg_extractions")
                || lower.contains("kg_resolutions_log")
                || lower.contains("agent_kg_corpora")
                || lower.contains("idx_kg_")
                || lower.contains("idx_agent_kg_corpora")
                || lower.contains("schema_meta")
                || lower.contains("operational_items")
                || lower.contains("auto_pull_stats")
            {
                continue;
            }
            if lower.contains("fts5") || lower.contains("vec0") {
                continue;
            }
            if lower.contains("unified_timeline") {
                continue;
            }
            if lower.contains("sqlite_sequence") {
                continue;
            }
            conn2.execute_batch(ddl).unwrap_or_else(|e| {
                if !e.to_string().contains("already exists") {
                    panic!("DDL failed: {ddl}: {e}");
                }
            });
        }
        conn2.execute_batch("COMMIT;").unwrap();

        // Recreate view and virtual tables
        conn2.execute_batch(UNIFIED_TIMELINE_VIEW_SQL).unwrap();
        let _ = conn2.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
                 USING fts5(content, content='search_content', content_rowid='id');
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_search
                 USING vec0(embedding float[512]);",
        );

        // Set version to 24 and insert default agent
        conn2
            .execute_batch(
                "DELETE FROM schema_version WHERE version > 24;
                 INSERT OR IGNORE INTO schema_version (version) VALUES (24);
                 INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');",
            )
            .unwrap();

        // Verify v24 state: KG tables should not exist
        let v24_version: i64 = conn2
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            v24_version, 24,
            "DB2 should be at v24 before incremental migration"
        );
        assert!(
            !table_exists(&conn2, "kg_entities"),
            "kg_entities should not exist at v24"
        );

        // Run incremental migrations: v24 -> v25 -> v26 -> v27 -> v28 -> v29
        let mut db2 = Database { conn: conn2 };
        db2.migrate_v24_to_v25().unwrap();
        db2.migrate_v25_to_v26().unwrap();
        db2.migrate_v26_to_v27().unwrap();

        // Insert the v27 coalesce marker so check_v27_coalesce_guard() passes.
        // In production this is written by the #787 coalesce step; in tests we
        // short-circuit it so the convergence comparison can proceed.
        db2.conn
            .execute(
                "INSERT OR IGNORE INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1')",
                [],
            )
            .unwrap();

        db2.migrate_v27_to_v28().unwrap();
        db2.migrate_v28_to_v29().unwrap();
        db2.migrate_v29_to_v30().unwrap();
        db2.migrate_v30_to_v31().unwrap();
        db2.migrate_v31_to_v32().unwrap();
        db2.migrate_v32_to_v33().unwrap();
        db2.migrate_v33_to_v34().unwrap();
        db2.migrate_v34_to_v35().unwrap();
        db2.migrate_v35_to_v36().unwrap();
        db2.migrate_v36_to_v37().unwrap();
        db2.migrate_v37_to_v38().unwrap();
        db2.migrate_v38_to_v39().unwrap();
        db2.migrate_v39_to_v40().unwrap();
        db2.migrate_v40_to_v41().unwrap();
        db2.migrate_v41_to_v42().unwrap();
        db2.migrate_v42_to_v43().unwrap();
        db2.migrate_v43_to_v44().unwrap();

        let final_version: i64 = db2
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            final_version, CURRENT_SCHEMA_VERSION,
            "DB2 should be at CURRENT_SCHEMA_VERSION after incremental migrations"
        );

        // #757: kg_extractions.source_doc_hash must exist after v27 migration.
        assert!(
            db2.column_exists("kg_extractions", "source_doc_hash")
                .unwrap(),
            "kg_extractions.source_doc_hash should exist after v27 migration"
        );

        // #786: kg_chunks.docs_root_hash must exist after v27 migration.
        assert!(
            db2.column_exists("kg_chunks", "docs_root_hash").unwrap(),
            "kg_chunks.docs_root_hash should exist after v27 migration"
        );

        // #798: agent_kg_corpora must exist after v28 migration.
        assert!(
            table_exists(&db2.conn, "agent_kg_corpora"),
            "agent_kg_corpora should exist after v28 migration"
        );

        let snap2 = snapshot_schema(&db2.conn);

        // Compare schemas structurally
        assert_eq!(
            snap1.len(),
            snap2.len(),
            "Table count mismatch: v1 has {} tables, incremental has {}\nv1: {:?}\nincremental: {:?}",
            snap1.len(),
            snap2.len(),
            snap1.iter().map(|t| &t.name).collect::<Vec<_>>(),
            snap2.iter().map(|t| &t.name).collect::<Vec<_>>(),
        );

        for (t1, t2) in snap1.iter().zip(snap2.iter()) {
            assert_eq!(t1.name, t2.name, "Table name mismatch");
            assert_eq!(
                t1.columns, t2.columns,
                "Column mismatch in table '{}'",
                t1.name
            );
            assert_eq!(
                t1.foreign_keys, t2.foreign_keys,
                "Foreign key mismatch in table '{}'",
                t1.name
            );
            assert_eq!(
                t1.indexes, t2.indexes,
                "Index mismatch in table '{}'",
                t1.name
            );
        }
    }

    #[test]
    fn test_v25_kg_chunks_unique_constraint() {
        let db = db();
        db.conn
            .execute(
                "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                 VALUES ('0000000000000000', '/test', 0, 'docs/test.md', 'hash1')",
                [],
            )
            .unwrap();

        // Duplicate (docs_root_hash, source_doc_path, seq_id) should fail
        let result = db.conn.execute(
            "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
             VALUES ('0000000000000000', '/test', 0, 'docs/test.md', 'hash2')",
            [],
        );
        assert!(
            result.is_err(),
            "duplicate (docs_root_hash, source_doc_path, seq_id) should violate UNIQUE constraint"
        );

        // Different seq_id should succeed
        db.conn
            .execute(
                "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                 VALUES ('0000000000000000', '/test', 1, 'docs/test.md', 'hash1')",
                [],
            )
            .expect("different seq_id should be allowed");
    }

    #[test]
    fn test_v25_subject_entity_agent_key_unique() {
        let db = db();
        db.conn
            .execute(
                "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
                 VALUES ('0000000000000000', '/test', 'failure_mode:oom', 'failure_mode', 'oom', 0.9)",
                [],
            )
            .unwrap();

        // Duplicate (docs_root_hash, entity_key) should fail
        let result = db.conn.execute(
            "INSERT INTO kg_subject_entities (docs_root_hash, docs_root, entity_key, type, name, confidence) \
             VALUES ('0000000000000000', '/test', 'failure_mode:oom', 'failure_mode', 'oom', 0.8)",
            [],
        );
        assert!(
            result.is_err(),
            "duplicate (docs_root_hash, entity_key) should violate UNIQUE constraint"
        );
    }

    // ── count_chunks_for_docs_root_hash tests (#778) ──────────────────────

    #[test]
    fn count_chunks_for_docs_root_hash_returns_zero_for_unknown() {
        let db = db();
        let count = db
            .count_chunks_for_docs_root_hash("unknown_hash_0000")
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn count_chunks_for_docs_root_hash_returns_correct_count() {
        let db = db();
        let hash = "abc1234567890abc";
        // Insert 3 chunks with the same docs_root_hash.
        for seq in 0..3 {
            db.conn
                .execute(
                    "INSERT INTO kg_chunks (docs_root_hash, docs_root, seq_id, source_doc_path, source_doc_hash) \
                     VALUES (?1, '/test', ?2, 'docs/test.md', 'dochash')",
                    rusqlite::params![hash, seq],
                )
                .unwrap();
        }
        let count = db.count_chunks_for_docs_root_hash(hash).unwrap();
        assert_eq!(count, 3);

        // Different hash should still return 0.
        let other = db
            .count_chunks_for_docs_root_hash("other_hash_000000")
            .unwrap();
        assert_eq!(other, 0);
    }

    // --- Transaction RAII tests (mika#636) ---

    /// Test 1: A committed RAII transaction persists writes visible to a
    /// separate connection.
    #[test]
    fn test_transaction_commit_persists() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        // Connection A: write inside a committed transaction.
        {
            let mut conn_a = Connection::open(path).unwrap();
            conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            conn_a
                .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);")
                .unwrap();

            let tx = conn_a.transaction().unwrap();
            tx.execute("INSERT INTO t (val) VALUES ('hello')", [])
                .unwrap();
            tx.commit().unwrap();
        }

        // Connection B: verify the row is visible.
        let conn_b = Connection::open(path).unwrap();
        let count: i64 = conn_b
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Test 2: A transaction dropped without commit() auto-rolls back — the
    /// write is invisible to a separate connection.
    #[test]
    fn test_transaction_drop_without_commit_rolls_back() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        // Connection A: write inside a transaction that is dropped (not committed).
        {
            let mut conn_a = Connection::open(path).unwrap();
            conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            conn_a
                .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT);")
                .unwrap();

            let tx = conn_a.transaction().unwrap();
            tx.execute("INSERT INTO t (val) VALUES ('dropped')", [])
                .unwrap();
            // Intentionally NOT calling tx.commit() — drop triggers ROLLBACK.
            drop(tx);

            // Even on the same connection, the row should not be visible.
            let count: i64 = conn_a
                .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "row should be rolled back on same connection");
        }

        // Connection B: also invisible.
        let conn_b = Connection::open(path).unwrap();
        let count: i64 = conn_b
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "row should be rolled back for other connections");
    }

    /// Test 3: RAII Transaction rollback preserves prior state when an error
    /// occurs mid-transaction. Uses the same DEFERRED transaction pattern as
    /// `replace_with_summary` to verify that `Transaction::drop` rolls back
    /// partial writes.
    #[test]
    fn test_replace_with_summary_rollback_on_error() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        let mut conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, val TEXT NOT NULL);
             INSERT INTO t (val) VALUES ('original');",
        )
        .unwrap();

        // Verify baseline.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Simulate a transaction that partially succeeds then fails.
        // This mirrors the replace_with_summary pattern: DELETE + INSERT
        // where the INSERT can fail.
        let result: Result<(), rusqlite::Error> = (|| {
            let tx = conn.transaction()?;
            // First operation succeeds — deletes the original row.
            tx.execute("DELETE FROM t WHERE val = 'original'", [])?;
            // Second operation fails — NOT NULL constraint violation.
            tx.execute("INSERT INTO t (val) VALUES (NULL)", [])?;
            tx.commit()?;
            Ok(())
        })();

        assert!(
            result.is_err(),
            "INSERT NULL should violate NOT NULL constraint"
        );

        // The original row should still be present — RAII Transaction::drop
        // rolled back the DELETE when the INSERT failed.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "original row should be preserved after rollback");

        let val: String = conn
            .query_row("SELECT val FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            val, "original",
            "original value should be preserved after rollback"
        );
    }

    /// Test 6: Cross-connection staleness regression test.
    ///
    /// Opens two connections to the same WAL-mode DB with `cache=private`
    /// (disables in-process shared cache to simulate cross-process visibility).
    /// Connection A writes a session. Connection B reads sessions. Without
    /// the WAL checkpoint fix, B would see stale data if A's transaction was
    /// held; with the fix + checkpoint, B sees fresh data.
    #[test]
    fn test_dashboard_sees_writes_from_separate_connection() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        // Open Connection A with cache=private to simulate cross-process isolation.
        let uri_a = format!("file:{}?cache=private", path);
        let mut conn_a = Connection::open_with_flags(
            &uri_a,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();
        conn_a.execute_batch("PRAGMA journal_mode=WAL;").unwrap();

        // Create schema (minimal: agents + sessions tables).
        conn_a
            .execute_batch(
                "CREATE TABLE agents (
                     id TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     home_dir TEXT NOT NULL DEFAULT '',
                     active BOOLEAN NOT NULL DEFAULT 1,
                     last_seen TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 );
                 CREATE TABLE sessions (
                     id TEXT PRIMARY KEY,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     channel_type TEXT NOT NULL DEFAULT 'cli',
                     started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     ended_at TEXT,
                     metadata TEXT,
                     parent_session_id TEXT,
                     task_id TEXT
                 );
                 INSERT INTO agents (id, name) VALUES ('test', 'test');
                 INSERT INTO sessions (id, agent_id, channel_type) VALUES ('s1', 'test', 'cli');",
            )
            .unwrap();

        // Open Connection B with cache=private (simulates dashboard reader).
        let uri_b = format!("file:{}?cache=private", path);
        let conn_b = Connection::open_with_flags(
            &uri_b,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .unwrap();

        // B sees the initial session.
        let count_before: i64 = conn_b
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, 1);

        // A writes a new session (committed transaction, RAII style).
        {
            let tx = conn_a.transaction().unwrap();
            tx.execute(
                "INSERT INTO sessions (id, agent_id, channel_type) VALUES ('s2', 'test', 'cli')",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
        }

        // Without a checkpoint, B may still see stale data due to WAL snapshot.
        // Run PASSIVE checkpoint on A's connection to advance the snapshot.
        conn_a
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .unwrap();

        // B should now see the new session after the checkpoint.
        let count_after: i64 = conn_b
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count_after, 2,
            "dashboard connection should see writes after WAL checkpoint"
        );
    }

    // ===== Secret scrubbing at save_tool_call boundary (#908) =====

    #[test]
    fn test_save_tool_call_scrubs_secrets_in_output() {
        let db = db();
        db.save_tool_call(
            "tc-1",
            "mika",
            "test-session",
            None,
            None,
            0,
            "read_agent_file",
            "builtin",
            None,
            Some(r#"{"path":".env"}"#),
            Some("MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef\nMIKA_LOG_FORMAT=json"),
            true,
            false,
            100,
            None,
        )
        .unwrap();

        let (input, output): (Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT input, output FROM tool_calls WHERE id = 'tc-1'",
                [],
                |row: &rusqlite::Row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        // Output should have secret redacted but non-secret preserved
        let output = output.unwrap();
        assert!(
            !output.contains("github_pat_11CBQ5"),
            "secret should be redacted in output: {output}"
        );
        assert!(
            output.contains("MIKA_GITHUB_TOKEN=<REDACTED>"),
            "env var assignment should be redacted: {output}"
        );
        assert!(
            output.contains("MIKA_LOG_FORMAT=json"),
            "non-secret env var should be preserved: {output}"
        );

        // Input should be unchanged (no secrets)
        let input = input.unwrap();
        assert_eq!(input, r#"{"path":".env"}"#);
    }

    #[test]
    fn test_save_tool_call_scrubs_secrets_in_input() {
        let db = db();
        db.save_tool_call(
            "tc-2",
            "mika",
            "test-session",
            None,
            None,
            0,
            "run_shell",
            "builtin",
            None,
            Some(r#"{"command":"echo ghp_ABCDEFghij1234567890"}"#),
            Some("ghp_ABCDEFghij1234567890"),
            true,
            false,
            50,
            None,
        )
        .unwrap();

        let (input, output): (Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT input, output FROM tool_calls WHERE id = 'tc-2'",
                [],
                |row: &rusqlite::Row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        let input = input.unwrap();
        assert!(
            !input.contains("ghp_ABCDEFghij"),
            "secret should be redacted in input: {input}"
        );
        assert!(
            input.contains("ghp_<REDACTED>"),
            "should have redaction marker: {input}"
        );

        let output = output.unwrap();
        assert_eq!(output, "ghp_<REDACTED>");
    }

    #[test]
    fn test_save_tool_call_preserves_clean_content() {
        let db = db();
        let clean_output = "File contents:\nname = mika\nversion = 0.5.0";
        db.save_tool_call(
            "tc-3",
            "mika",
            "test-session",
            None,
            None,
            0,
            "read_agent_file",
            "builtin",
            None,
            Some(r#"{"path":"Cargo.toml"}"#),
            Some(clean_output),
            true,
            false,
            30,
            None,
        )
        .unwrap();

        let output: String = db
            .conn
            .query_row(
                "SELECT output FROM tool_calls WHERE id = 'tc-3'",
                [],
                |row: &rusqlite::Row| row.get(0),
            )
            .unwrap();

        assert_eq!(output, clean_output, "clean content should be unchanged");
    }

    #[test]
    fn test_save_tool_call_scrubs_secrets_in_error_message() {
        let db = db();
        // When a tool fails, error_message carries the same content as output.
        // Both must be scrubbed.
        let secret_error =
            "Error reading .env: MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef";
        db.save_tool_call(
            "tc-err",
            "mika",
            "test-session",
            None,
            None,
            0,
            "read_agent_file",
            "builtin",
            None,
            Some(r#"{"path":".env"}"#),
            Some(secret_error),
            false,
            false,
            100,
            Some(secret_error),
        )
        .unwrap();

        let (output, err_msg): (Option<String>, Option<String>) = db
            .conn
            .query_row(
                "SELECT output, error_message FROM tool_calls WHERE id = 'tc-err'",
                [],
                |row: &rusqlite::Row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        let output = output.unwrap();
        let err_msg = err_msg.unwrap();
        assert!(
            !output.contains("github_pat_11CBQ5"),
            "secret in output should be redacted: {output}"
        );
        assert!(
            !err_msg.contains("github_pat_11CBQ5"),
            "secret in error_message should be redacted: {err_msg}"
        );
    }

    #[test]
    fn test_migrate_v28_to_v29_scrubs_existing_rows() {
        let mut db = db();

        // Temporarily set schema to 28 so migration will run
        db.conn
            .execute("UPDATE schema_version SET version = 28", [])
            .unwrap();

        // Insert a row with a secret directly (bypassing the scrubber via raw SQL)
        db.conn
            .execute(
                "INSERT INTO tool_calls (id, agent_id, session_id, step, tool_name, tool_source, input, output, success, non_zero_exit, latency_ms)
                 VALUES ('old-1', 'mika', 'test-session', 0, 'read_agent_file', 'builtin', '{\"path\":\".env\"}', 'MIKA_GITHUB_TOKEN=github_pat_11CBQ5ABC1234567890abcdef', 1, 0, 100)",
                [],
            )
            .unwrap();

        // Insert a clean row that should not be modified
        db.conn
            .execute(
                "INSERT INTO tool_calls (id, agent_id, session_id, step, tool_name, tool_source, input, output, success, non_zero_exit, latency_ms)
                 VALUES ('old-2', 'mika', 'test-session', 1, 'search_memory', 'builtin', '{\"query\":\"test\"}', 'No results found', 1, 0, 50)",
                [],
            )
            .unwrap();

        // Run migration
        db.migrate_v28_to_v29().unwrap();

        // Verify version bumped
        assert_eq!(db.schema_version().unwrap(), 29);

        // Verify secret row was scrubbed
        let output: String = db
            .conn
            .query_row(
                "SELECT output FROM tool_calls WHERE id = 'old-1'",
                [],
                |row: &rusqlite::Row| row.get(0),
            )
            .unwrap();
        assert!(
            !output.contains("github_pat_11CBQ5"),
            "migration should have scrubbed secret: {output}"
        );
        assert!(
            output.contains("<REDACTED>"),
            "migration should have redacted: {output}"
        );

        // Verify clean row was unchanged
        let clean_output: String = db
            .conn
            .query_row(
                "SELECT output FROM tool_calls WHERE id = 'old-2'",
                [],
                |row: &rusqlite::Row| row.get(0),
            )
            .unwrap();
        assert_eq!(clean_output, "No results found");
    }

    #[test]
    fn test_migrate_v28_to_v29_idempotent() {
        let mut db = db();
        // DB is already at CURRENT_SCHEMA_VERSION (db() creates at latest)
        // Running migration again should be a no-op
        db.migrate_v28_to_v29().unwrap();
        assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_migrate_v29_to_v30_expands_check_constraint() {
        let mut db = db();

        // Seed subject entities so FK constraints are satisfied.
        // The db() helper creates fresh schema with kg_subject_entities table.
        db.conn
            .execute_batch(
                "INSERT INTO kg_subject_entities (id, docs_root_hash, docs_root, entity_key, type, name, confidence, trace_id)
                 VALUES (1, 'abcd1234', '/docs', 'skill:test1', 'skill', 'test1', 0.9, 'trace-1');
                 INSERT INTO kg_subject_entities (id, docs_root_hash, docs_root, entity_key, type, name, confidence, trace_id)
                 VALUES (2, 'abcd1234', '/docs', 'skill:test2', 'skill', 'test2', 0.9, 'trace-2');
                 INSERT INTO kg_subject_entities (id, docs_root_hash, docs_root, entity_key, type, name, confidence, trace_id)
                 VALUES (3, 'abcd1234', '/docs', 'skill:test3', 'skill', 'test3', 0.9, 'trace-3');",
            )
            .unwrap();

        // Temporarily set schema to 29 so migration will run.
        // We need to rebuild the table with the old CHECK constraint first.
        db.conn
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP INDEX IF EXISTS idx_kg_res_log_pending;
                 ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_old;
                 CREATE TABLE kg_resolutions_log (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     outcome TEXT NOT NULL CHECK (outcome IN (
                         'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                     )),
                     resolution_trace_id TEXT NOT NULL,
                     source_extraction_trace_id TEXT,
                     model TEXT,
                     duration_ms INTEGER,
                     resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (agent_id, subject_entity_id)
                 );
                 INSERT INTO kg_resolutions_log SELECT * FROM kg_resolutions_log_old;
                 DROP TABLE kg_resolutions_log_old;
                 CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);
                 PRAGMA foreign_keys = ON;
                 UPDATE schema_version SET version = 29;",
            )
            .unwrap();

        // Verify matched_llm_db_fallback is REJECTED before migration.
        let insert_result = db.conn.execute(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 1, 'matched_llm_db_fallback', 'trace-pre')",
            [],
        );
        assert!(
            insert_result.is_err(),
            "v29 schema should reject matched_llm_db_fallback"
        );

        // Seed a row with an existing outcome to verify preservation.
        db.conn
            .execute(
                "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
                 VALUES ('mika', 1, 'matched_exact', 'trace-seed')",
                [],
            )
            .unwrap();

        let count_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap();

        // Run migration.
        db.migrate_v29_to_v30().unwrap();

        // Verify version bumped.
        assert_eq!(db.schema_version().unwrap(), 30);

        // Verify row count preserved.
        let count_after: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, count_after, "row count should be preserved");

        // Verify matched_llm_db_fallback is ACCEPTED after migration.
        // Use a different subject_entity_id to avoid UNIQUE violation.
        let insert_result = db.conn.execute(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 2, 'matched_llm_db_fallback', 'trace-post')",
            [],
        );
        assert!(
            insert_result.is_ok(),
            "v30 schema should accept matched_llm_db_fallback: {:?}",
            insert_result.err()
        );

        // Verify invalid values still rejected.
        let invalid_result = db.conn.execute(
            "INSERT INTO kg_resolutions_log (agent_id, subject_entity_id, outcome, resolution_trace_id) \
             VALUES ('mika', 3, 'invalid_value', 'trace-invalid')",
            [],
        );
        assert!(
            invalid_result.is_err(),
            "invalid outcome should still be rejected"
        );
    }

    #[test]
    fn test_migrate_v29_to_v30_idempotent() {
        let mut db = db();
        // DB is already at CURRENT_SCHEMA_VERSION (db() creates at latest)
        // Running migration again should be a no-op
        db.migrate_v29_to_v30().unwrap();
        assert_eq!(db.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    // -- Callback watchdog DB helpers (#959) --

    /// Helper: create a parent task and a callback child task, returning (parent_id, child_id).
    fn create_callback_task_pair(db: &Database, agent: &str, label: &str) -> (String, String) {
        let parent = new_task(agent, &format!("{label}-parent"), "manual", "none");
        let parent_id = db.create_task(&parent).unwrap();
        let mut child = new_task(agent, label, "callback", "resume_agent");
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();
        (parent_id, child_id)
    }

    #[test]
    fn test_get_active_callback_tasks_with_pid() {
        let db = db();
        let (_parent_id, child_id) = create_callback_task_pair(&db, "mika", "callback-task");
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress', process_id = 12345 WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let results = db.get_active_callback_tasks_with_pid("mika").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, child_id);
        assert_eq!(results[0].process_id, Some(12345));
    }

    #[test]
    fn test_get_active_callback_tasks_with_pid_excludes_completed() {
        let db = db();
        let (_parent_id, child_id) = create_callback_task_pair(&db, "mika", "done-callback");
        db.conn
            .execute(
                "UPDATE tasks SET status = 'completed', process_id = 12345 WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let results = db.get_active_callback_tasks_with_pid("mika").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_active_callback_tasks_with_pid_excludes_no_pid() {
        let db = db();
        let (_parent_id, child_id) = create_callback_task_pair(&db, "mika", "no-pid-callback");
        db.conn
            .execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
                params![child_id],
            )
            .unwrap();

        let results = db.get_active_callback_tasks_with_pid("mika").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_set_task_metadata_field_new() {
        let db = db();
        let task = new_task("mika", "meta-test", "manual", "none");
        let id = db.create_task(&task).unwrap();

        db.set_task_metadata_field(&id, "process_start_time", "999888")
            .unwrap();

        let val = db
            .get_task_metadata_field(&id, "process_start_time")
            .unwrap();
        assert_eq!(val, Some("999888".to_string()));
    }

    #[test]
    fn test_set_task_metadata_field_existing_metadata() {
        let db = db();
        let task = new_task("mika", "meta-test-2", "manual", "none");
        let id = db.create_task(&task).unwrap();

        // Set initial metadata
        db.update_task_metadata(&id, r#"{"existing":"value"}"#)
            .unwrap();

        // Add a new field
        db.set_task_metadata_field(&id, "first_dead_at", "2026-05-05T00:00:00Z")
            .unwrap();

        // Both fields should exist
        let val = db.get_task_metadata_field(&id, "existing").unwrap();
        assert_eq!(val, Some("value".to_string()));
        let val = db.get_task_metadata_field(&id, "first_dead_at").unwrap();
        assert_eq!(val, Some("2026-05-05T00:00:00Z".to_string()));
    }

    #[test]
    fn test_get_task_metadata_field_missing_key() {
        let db = db();
        let task = new_task("mika", "meta-test-3", "manual", "none");
        let id = db.create_task(&task).unwrap();

        let val = db.get_task_metadata_field(&id, "nonexistent").unwrap();
        assert_eq!(val, None);
    }

    // -- mika#1011: cancel_task cascade tests --

    #[test]
    fn test_cancel_task_cascades_to_active_callback_children() {
        let db = db();
        // Create parent task
        let parent = new_task("mika", "parent-work-item", "manual", "none");
        let parent_id = db.create_task(&parent).unwrap();

        // Create callback child (pending)
        let mut child = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();

        // Create deferred callback child (pending)
        let mut deferred = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        deferred.parent_task_id = Some(parent_id.clone());
        let deferred_id = db.create_task(&deferred).unwrap();

        // Create a non-callback child (manual sub-task) — should NOT be cancelled
        let mut manual_child = new_task("mika", "sub-task", "manual", "none");
        manual_child.parent_task_id = Some(parent_id.clone());
        let manual_child_id = db.create_task(&manual_child).unwrap();

        // Cancel parent
        assert!(db.cancel_task(&parent_id, "mika").unwrap());

        // Callback children should be cancelled
        let child_task = db.get_task_unscoped(&child_id).unwrap().unwrap();
        assert_eq!(
            child_task.status, "cancelled",
            "callback child should be cascaded"
        );

        let deferred_task = db.get_task_unscoped(&deferred_id).unwrap().unwrap();
        assert_eq!(
            deferred_task.status, "cancelled",
            "deferred callback should be cascaded"
        );

        // Non-callback child should NOT be cancelled
        let manual_task = db.get_task_unscoped(&manual_child_id).unwrap().unwrap();
        assert_eq!(
            manual_task.status, "pending",
            "manual child should not be affected"
        );
    }

    #[test]
    fn test_cancel_task_does_not_cascade_to_completed_callback() {
        let db = db();
        let parent = new_task("mika", "parent", "manual", "none");
        let parent_id = db.create_task(&parent).unwrap();

        let mut child = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        child.parent_task_id = Some(parent_id.clone());
        let child_id = db.create_task(&child).unwrap();
        // Mark child as completed
        db.update_task_completed(&child_id, "mika", Some("done"))
            .unwrap();

        // Cancel parent
        assert!(db.cancel_task(&parent_id, "mika").unwrap());

        // Completed child should NOT be affected
        let child_task = db.get_task_unscoped(&child_id).unwrap().unwrap();
        assert_eq!(
            child_task.status, "completed",
            "completed callback should not be cancelled"
        );
    }

    // -- mika#1011: deferred callback DB helper tests --

    #[test]
    fn test_count_pending_deferred_callbacks() {
        let db = db();
        // No deferred callbacks initially
        assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 0);

        // Create parent tasks (FK requirement)
        let p1_id = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();
        let p2_id = db
            .create_task(&new_task("mika", "p2", "manual", "none"))
            .unwrap();
        let p3_id = db
            .create_task(&new_task("mika", "p3", "manual", "none"))
            .unwrap();

        // Create a deferred callback
        let mut task = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        task.parent_task_id = Some(p1_id.clone());
        db.create_task(&task).unwrap();

        assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 1);

        // Create another
        let mut task2 = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        task2.parent_task_id = Some(p2_id.clone());
        db.create_task(&task2).unwrap();

        assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 2);

        // Non-deferred callback should not count
        let mut regular = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        regular.parent_task_id = Some(p3_id.clone());
        db.create_task(&regular).unwrap();

        assert_eq!(db.count_pending_deferred_callbacks("mika").unwrap(), 2);
    }

    #[test]
    fn test_promote_next_deferred_callback_fifo() {
        let db = db();

        // No deferred callbacks → returns None
        assert!(db.promote_next_deferred_callback("mika").unwrap().is_none());

        // Create parent tasks (FK requirement)
        let p1_id = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();
        let p2_id = db
            .create_task(&new_task("mika", "p2", "manual", "none"))
            .unwrap();

        // Create two deferred callbacks
        let mut task1 = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        task1.parent_task_id = Some(p1_id.clone());
        let id1 = db.create_task(&task1).unwrap();

        let mut task2 = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        task2.parent_task_id = Some(p2_id.clone());
        let id2 = db.create_task(&task2).unwrap();

        // Promote first (FIFO)
        assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());

        // First should be completed, second still pending
        let t1 = db.get_task_unscoped(&id1).unwrap().unwrap();
        assert_eq!(
            t1.status, "completed",
            "first deferred should be promoted to completed"
        );
        assert!(t1.result.is_some(), "promoted task should have a result");

        let t2 = db.get_task_unscoped(&id2).unwrap().unwrap();
        assert_eq!(
            t2.status, "pending",
            "second deferred should still be pending"
        );

        // Promote second
        assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());
        let t2 = db.get_task_unscoped(&id2).unwrap().unwrap();
        assert_eq!(t2.status, "completed");

        // No more → returns None
        assert!(db.promote_next_deferred_callback("mika").unwrap().is_none());
    }

    /// mika#1175 — Class-scoped sibling of `test_promote_next_deferred_callback_fifo`.
    /// Verifies that `promote_next_deferred_callback_for_class` filters by
    /// `dispatch_class`, that NULL-class rows are treated as `'implement'` via
    /// `COALESCE`, and that FIFO is preserved within a class.
    #[test]
    fn test_promote_next_deferred_callback_for_class_filters_by_class() {
        let db = db();

        // No deferred callbacks → both class predicates return None.
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "implement")
                .unwrap()
                .is_none()
        );
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "groom")
                .unwrap()
                .is_none()
        );

        // Parent tasks (FK requirement).
        let p_impl = db
            .create_task(&new_task("mika", "p_impl", "manual", "none"))
            .unwrap();
        let p_groom = db
            .create_task(&new_task("mika", "p_groom", "manual", "none"))
            .unwrap();
        let p_null = db
            .create_task(&new_task("mika", "p_null", "manual", "none"))
            .unwrap();

        // Three deferred wrappers: implement, groom, NULL (pre-v34 row).
        let mut w_impl = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        w_impl.parent_task_id = Some(p_impl.clone());
        w_impl.dispatch_class = Some("implement".to_string());
        let id_impl = db.create_task(&w_impl).unwrap();

        let mut w_groom = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        w_groom.parent_task_id = Some(p_groom.clone());
        w_groom.dispatch_class = Some("groom".to_string());
        let id_groom = db.create_task(&w_groom).unwrap();

        let mut w_null = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        w_null.parent_task_id = Some(p_null.clone());
        w_null.dispatch_class = None; // pre-v34 NULL row
        let id_null = db.create_task(&w_null).unwrap();

        // Promote groom: only the groom wrapper transitions.
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "groom")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            db.get_task_unscoped(&id_groom).unwrap().unwrap().status,
            "completed"
        );
        assert_eq!(
            db.get_task_unscoped(&id_impl).unwrap().unwrap().status,
            "pending",
            "implement wrapper must not transition on a groom promotion"
        );
        assert_eq!(
            db.get_task_unscoped(&id_null).unwrap().unwrap().status,
            "pending",
            "NULL-class wrapper must not transition on a groom promotion"
        );

        // No more groom wrappers pending.
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "groom")
                .unwrap()
                .is_none()
        );

        // First implement promotion: one of (implement, NULL) transitions
        // (FIFO within the implement+NULL class group via COALESCE).
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "implement")
                .unwrap()
                .is_some()
        );
        let after_first_impl = (
            db.get_task_unscoped(&id_impl).unwrap().unwrap().status,
            db.get_task_unscoped(&id_null).unwrap().unwrap().status,
        );
        assert!(
            matches!(
                after_first_impl,
                (ref a, ref b) if (a == "completed" && b == "pending") || (a == "pending" && b == "completed")
            ),
            "exactly one of (implement, NULL) must transition on first implement promotion, got {after_first_impl:?}"
        );

        // Second implement promotion: the remaining wrapper transitions
        // (NULL is matched by 'implement' via COALESCE).
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "implement")
                .unwrap()
                .is_some()
        );
        assert_eq!(
            db.get_task_unscoped(&id_impl).unwrap().unwrap().status,
            "completed",
            "implement wrapper must be completed after second implement promotion"
        );
        assert_eq!(
            db.get_task_unscoped(&id_null).unwrap().unwrap().status,
            "completed",
            "NULL-class wrapper must be completed after second implement \
             promotion (COALESCE treats NULL as 'implement')"
        );

        // Drained.
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "implement")
                .unwrap()
                .is_none()
        );
        assert!(
            db.promote_next_deferred_callback_for_class("mika", "groom")
                .unwrap()
                .is_none()
        );
    }

    /// mika#1070 — Regression test: chain promotion works after anti-cascade
    /// guard removal. Simulates the full lifecycle:
    /// 1. Blocking callback completes → promotes wrapper W1
    /// 2. W1's DeferredDispatch turn completes (mark delivered) → promotes W2
    #[test]
    fn test_deferred_dispatch_chain_promotion() {
        let db = db();

        // Create parent tasks
        let p1 = db
            .create_task(&new_task("mika", "host1", "manual", "none"))
            .unwrap();
        let p2 = db
            .create_task(&new_task("mika", "host2", "manual", "none"))
            .unwrap();

        // Create two deferred wrappers
        let mut w1 = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        w1.parent_task_id = Some(p1.clone());
        let w1_id = db.create_task(&w1).unwrap();

        let mut w2 = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        w2.parent_task_id = Some(p2.clone());
        let w2_id = db.create_task(&w2).unwrap();

        // Step 1: Promote W1 (simulates blocking callback completion)
        assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());
        let t1 = db.get_task_unscoped(&w1_id).unwrap().unwrap();
        assert_eq!(t1.status, "completed");
        assert!(t1.completed_at.is_some());

        // W1 should be returned by get_undelivered_callback_tasks
        let since = crate::timestamp::now_minus(chrono::Duration::hours(1));
        let undelivered = db.get_undelivered_callback_tasks("mika", &since).unwrap();
        assert!(
            undelivered.iter().any(|t| t.id == w1_id),
            "promoted W1 should be in undelivered callbacks"
        );

        // Step 2: Mark W1 as delivered (simulates DeferredDispatch turn completion)
        assert!(db.mark_task_delivered(&w1_id).unwrap());

        // Step 3: Chain promotion — promote W2 (this was blocked by the
        // anti-cascade guard before mika#1070)
        assert!(db.promote_next_deferred_callback("mika").unwrap().is_some());
        let t2 = db.get_task_unscoped(&w2_id).unwrap().unwrap();
        assert_eq!(
            t2.status, "completed",
            "W2 should be promoted via chain promotion"
        );

        // W2 should be in undelivered callbacks
        let undelivered = db.get_undelivered_callback_tasks("mika", &since).unwrap();
        assert!(
            undelivered.iter().any(|t| t.id == w2_id),
            "promoted W2 should be in undelivered callbacks"
        );
    }

    /// mika#1070 — Regression test: has_any_active_callback correctly identifies
    /// active non-deferred callbacks and excludes deferred wrappers.
    #[test]
    fn test_has_any_active_callback() {
        let db = db();

        // No callbacks at all → false
        assert!(!db.has_any_active_callback("mika").unwrap());

        // Create parent task
        let p1 = db
            .create_task(&new_task("mika", "host1", "manual", "none"))
            .unwrap();

        // Add a deferred wrapper (should NOT count as active)
        let mut deferred = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        deferred.parent_task_id = Some(p1.clone());
        db.create_task(&deferred).unwrap();
        assert!(
            !db.has_any_active_callback("mika").unwrap(),
            "deferred wrapper should not count as active callback"
        );

        // Add a regular callback (SHOULD count as active)
        let p2 = db
            .create_task(&new_task("mika", "host2", "manual", "none"))
            .unwrap();
        let mut regular = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        regular.parent_task_id = Some(p2.clone());
        let reg_id = db.create_task(&regular).unwrap();
        assert!(
            db.has_any_active_callback("mika").unwrap(),
            "regular pending callback should count as active"
        );

        // Complete then deliver → no more active
        db.update_task_completed(&reg_id, "mika", Some("done"))
            .unwrap();
        db.mark_task_delivered(&reg_id).unwrap();
        assert!(
            !db.has_any_active_callback("mika").unwrap(),
            "delivered callback should not count as active"
        );
    }

    /// mika#1175 — Class-scoped sibling of `test_has_any_active_callback`.
    /// Verifies that `has_any_active_callback_for_class` is scoped to the given
    /// `dispatch_class`, that `:deferred` wrappers are excluded in both classes
    /// (parity with mika#1163), and that NULL-class rows are matched by the
    /// `'implement'` predicate via `COALESCE`.
    #[test]
    fn test_has_any_active_callback_for_class_class_scoped() {
        let db = db();

        // Empty DB → both predicates false.
        assert!(
            !db.has_any_active_callback_for_class("mika", "implement")
                .unwrap()
        );
        assert!(
            !db.has_any_active_callback_for_class("mika", "groom")
                .unwrap()
        );

        // Active non-deferred implement callback.
        let p_impl = db
            .create_task(&new_task("mika", "p_impl", "manual", "none"))
            .unwrap();
        let mut active_impl = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        active_impl.parent_task_id = Some(p_impl.clone());
        active_impl.dispatch_class = Some("implement".to_string());
        db.create_task(&active_impl).unwrap();

        assert!(
            db.has_any_active_callback_for_class("mika", "implement")
                .unwrap(),
            "implement-class predicate must detect the active implement callback"
        );
        assert!(
            !db.has_any_active_callback_for_class("mika", "groom")
                .unwrap(),
            "groom-class predicate must not see the implement callback"
        );

        // Add `:deferred` wrappers in BOTH classes — must not flip either predicate.
        let p_def_impl = db
            .create_task(&new_task("mika", "p_def_impl", "manual", "none"))
            .unwrap();
        let mut def_impl = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        def_impl.parent_task_id = Some(p_def_impl.clone());
        def_impl.dispatch_class = Some("implement".to_string());
        db.create_task(&def_impl).unwrap();

        let p_def_groom = db
            .create_task(&new_task("mika", "p_def_groom", "manual", "none"))
            .unwrap();
        let mut def_groom = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        def_groom.parent_task_id = Some(p_def_groom.clone());
        def_groom.dispatch_class = Some("groom".to_string());
        db.create_task(&def_groom).unwrap();

        assert!(
            db.has_any_active_callback_for_class("mika", "implement")
                .unwrap(),
            "implement predicate unchanged by :deferred wrappers (mika#1163 parity)"
        );
        assert!(
            !db.has_any_active_callback_for_class("mika", "groom")
                .unwrap(),
            "groom predicate unchanged by :deferred wrappers (mika#1163 parity)"
        );

        // NULL-class active callback must be matched by `"implement"` via COALESCE.
        // Use a fresh agent so the assertion is independent of the rows above.
        db.register_agent("mika-null", "mika-null", "").unwrap();
        let p_null = db
            .create_task(&new_task("mika-null", "p_null", "manual", "none"))
            .unwrap();
        let mut active_null = new_task(
            "mika-null",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        active_null.parent_task_id = Some(p_null.clone());
        active_null.dispatch_class = None;
        db.create_task(&active_null).unwrap();

        assert!(
            db.has_any_active_callback_for_class("mika-null", "implement")
                .unwrap(),
            "NULL-class active callback must be matched by 'implement' predicate \
             (COALESCE matches mika#1163 slot-guard semantics)"
        );
        assert!(
            !db.has_any_active_callback_for_class("mika-null", "groom")
                .unwrap(),
            "NULL-class active callback must NOT be matched by 'groom' predicate"
        );
    }

    /// mika#1163 — Regression test: `has_active_callback_tasks_excluding` must
    /// exclude `:deferred` wrappers when looking for "slot occupied" evidence.
    ///
    /// The sibling predicate `has_any_active_callback` (mika#1070) already
    /// excludes `:deferred` rows; this test pins the same semantics on the
    /// per-class slot predicate used by `validate_dispatch_readiness`. Without
    /// the `label NOT LIKE '%:deferred'` clause, two parents each holding a
    /// pending deferred wrapper deadlock: every dispatch attempt from one
    /// wrapper sees the OTHER wrapper as an active dispatch, so neither ever
    /// promotes through `run_claude_pilot`.
    #[test]
    fn test_has_active_callback_tasks_excluding_ignores_deferred_wrappers() {
        let db = db();

        // Parent A with one pending deferred wrapper as a callback child.
        let p_a = db
            .create_task(&new_task("mika", "host_a", "manual", "none"))
            .unwrap();
        let mut w_a = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        w_a.parent_task_id = Some(p_a.clone());
        db.create_task(&w_a).unwrap();

        // Querying for any other parent: a `:deferred` wrapper is NOT an
        // active dispatch, so the predicate must return None.
        let result = db
            .has_active_callback_tasks_excluding("other-task", "mika", "implement")
            .unwrap();
        assert!(
            result.is_none(),
            "pending :deferred wrapper must not count as an active dispatch \
             (mika#1163 — was previously detected as slot-occupied, deadlocking \
             every cross-parent dispatch attempt)"
        );

        // Add Parent B with a REAL (non-deferred) pending callback. This IS
        // an active dispatch and MUST be detected.
        let p_b = db
            .create_task(&new_task("mika", "host_b", "manual", "none"))
            .unwrap();
        let mut real_b = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        real_b.parent_task_id = Some(p_b.clone());
        let real_b_id = db.create_task(&real_b).unwrap();

        // Querying from a third parent: should find Parent B's real callback,
        // proving the exclusion is wrapper-only, not blanket.
        let result = db
            .has_active_callback_tasks_excluding("other-task", "mika", "implement")
            .unwrap();
        let (parent_id, callback_id, callback_label) = result.expect(
            "real (non-deferred) pending callback MUST still be detected as an \
             active dispatch — exclusion is narrowly scoped to :deferred wrappers",
        );
        assert_eq!(parent_id, p_b, "should match Parent B (the real dispatch)");
        assert_eq!(callback_id, real_b_id);
        assert_eq!(callback_label, "long_running:run_claude_pilot");

        // Mixed-state: querying from Parent B itself excludes B's own callback
        // via the parent_task_id != ?1 clause, and the only remaining row is
        // A's deferred wrapper. Result must be None.
        let result = db
            .has_active_callback_tasks_excluding(&p_b, "mika", "implement")
            .unwrap();
        assert!(
            result.is_none(),
            "Parent B's query: own callback excluded by parent filter, A's \
             wrapper excluded by :deferred filter — no blocking dispatch"
        );

        // Suffix-anchor pin: the `%:deferred` LIKE pattern matches END of the
        // label, not arbitrary substring. A hypothetical future label that
        // contains `:deferred` mid-string (e.g., `:deferred:retry`) is NOT
        // excluded by the wildcard — it would be counted as an active
        // dispatch. This pins the convention so a refactor that shifts the
        // suffix convention has to update the SQL clause deliberately.
        let p_d = db
            .create_task(&new_task("mika", "host_d", "manual", "none"))
            .unwrap();
        let mut suffix_variant = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred:retry",
            "callback",
            "resume_agent",
        );
        suffix_variant.parent_task_id = Some(p_d.clone());
        let suffix_variant_id = db.create_task(&suffix_variant).unwrap();
        // Delete the existing real callback for Parent B so this assertion
        // can isolate the suffix-variant row's behavior.
        db.cancel_task(&real_b_id, "mika").unwrap();
        let result = db
            .has_active_callback_tasks_excluding("other-task", "mika", "implement")
            .unwrap();
        let (parent_id, callback_id, _callback_label) = result.expect(
            "label `:deferred:retry` is NOT a suffix match for `%:deferred` — \
             must still be counted as an active dispatch",
        );
        assert_eq!(parent_id, p_d, "suffix-variant row should be the blocker");
        assert_eq!(callback_id, suffix_variant_id);

        // Forward-compat for in_progress deferred wrappers: today no code path
        // sets a `:deferred` row to `in_progress`, but the SQL `status IN
        // ('pending', 'in_progress')` clause catches both. Verify the
        // exclusion also holds for in_progress wrappers so a future code path
        // that flips a wrapper to in_progress doesn't accidentally reintroduce
        // the deadlock.
        db.update_task_status(&suffix_variant_id, "cancelled")
            .unwrap(); // clean up suffix-variant first
        let p_e = db
            .create_task(&new_task("mika", "host_e", "manual", "none"))
            .unwrap();
        let mut deferred_in_progress = new_task(
            "mika",
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        deferred_in_progress.parent_task_id = Some(p_e.clone());
        let dip_id = db.create_task(&deferred_in_progress).unwrap();
        db.update_task_status(&dip_id, "in_progress").unwrap();
        let result = db
            .has_active_callback_tasks_excluding("other-task", "mika", "implement")
            .unwrap();
        assert!(
            result.is_none(),
            "in_progress :deferred wrapper must also be excluded (forward-compat \
             — today no code path sets a wrapper to in_progress, but the SQL \
             status filter covers both pending+in_progress, so the exclusion \
             contract must hold for both)"
        );
    }

    /// mika#1070 — Regression test: AgentBusy recovery keeps callback in
    /// 'completed' status so dispatch_undelivered_callbacks can find it.
    /// The old behavior reset to 'pending', which stranded the callback.
    #[test]
    fn test_agent_busy_callback_stays_completed() {
        let db = db();

        let p1 = db
            .create_task(&new_task("mika", "host1", "manual", "none"))
            .unwrap();

        let mut cb = new_task(
            "mika",
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        cb.parent_task_id = Some(p1.clone());
        let cb_id = db.create_task(&cb).unwrap();

        // Simulate external completion (webhook handler sets completed)
        db.update_task_completed(&cb_id, "mika", Some("done"))
            .unwrap();
        let t = db.get_task_unscoped(&cb_id).unwrap().unwrap();
        assert_eq!(t.status, "completed");

        // Simulate AgentBusy: keep completed, just set next_fire_at for retry
        let retry_at = crate::timestamp::now_plus(chrono::Duration::seconds(30));
        db.update_task_next_fire_at(&cb_id, &retry_at).unwrap();

        // The task should still be found by get_undelivered_callback_tasks
        let since = crate::timestamp::now_minus(chrono::Duration::hours(1));
        let undelivered = db.get_undelivered_callback_tasks("mika", &since).unwrap();
        assert!(
            undelivered.iter().any(|t| t.id == cb_id),
            "AgentBusy callback should remain in completed status and be findable"
        );

        // Verify it has next_fire_at set (for retry delay guard in engine)
        let t = db.get_task_unscoped(&cb_id).unwrap().unwrap();
        assert!(
            t.next_fire_at.is_some(),
            "AgentBusy callback should have next_fire_at for retry delay"
        );
    }

    // ===== Agent Reset tests (#964) =====

    /// Helper to create a manual task for the given agent.
    fn make_manual_task(agent_id: &str, label: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[test]
    fn test_reset_agent_empty() {
        // Reset an agent with no state — should be idempotent, all counts 0
        let db = db();
        let agent_id = "test-agent";
        db.register_agent(agent_id, "Test Agent", "/tmp/test")
            .unwrap();

        // First reset: all zeros
        let counts = db.reset_agent_state(agent_id).unwrap();
        assert_eq!(counts.total(), 0);

        // Agent row still exists
        let agents = db.list_agents_db().unwrap();
        assert!(agents.iter().any(|a| a.id == agent_id));

        // Second reset: still idempotent
        let counts2 = db.reset_agent_state(agent_id).unwrap();
        assert_eq!(counts2.total(), 0);
    }

    #[test]
    fn test_reset_agent_populated() {
        let db = db();
        let agent_id = "test-agent";
        db.register_agent(agent_id, "Test Agent", "/tmp/test")
            .unwrap();

        // Insert data into multiple child tables
        let sid = "test-reset-session";
        db.create_session(sid, agent_id, "cli").unwrap();
        db.save_message(agent_id, sid, "user", "hello", None)
            .unwrap();
        db.save_message(agent_id, sid, "assistant", "hi", None)
            .unwrap();
        db.set_core_memory(agent_id, "user_summary", "test user")
            .unwrap();

        // Create a task
        let task = make_manual_task(agent_id, "test task");
        db.create_task(&task).unwrap();

        // Create a person fact
        db.conn
            .execute(
                "INSERT INTO people (agent_id, canonical_name, relationship, notes)
                 VALUES (?1, 'Alice', 'colleague', 'test')",
                params![agent_id],
            )
            .unwrap();

        // Create a heartbeat_sends entry
        db.conn
            .execute(
                "INSERT INTO heartbeat_sends (agent_id, sent_at)
                 VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
                params![agent_id],
            )
            .unwrap();

        // Create a customer_config entry
        db.conn
            .execute(
                "INSERT INTO customer_config (agent_id, key, value)
                 VALUES (?1, 'test_key', 'test_value')",
                params![agent_id],
            )
            .unwrap();

        // Verify data exists before reset
        let pre_counts = db.count_agent_state(agent_id).unwrap();
        assert!(pre_counts.sessions > 0);
        assert!(pre_counts.messages > 0);
        assert!(pre_counts.core_memory > 0);
        assert!(pre_counts.tasks > 0);
        assert!(pre_counts.people > 0);
        assert!(pre_counts.heartbeat_sends > 0);
        assert!(pre_counts.customer_config > 0);

        // Reset
        let counts = db.reset_agent_state(agent_id).unwrap();
        assert!(counts.total() > 0, "Should have deleted rows");

        // Verify all child tables are empty for this agent
        let post_counts = db.count_agent_state(agent_id).unwrap();
        assert_eq!(post_counts.total(), 0, "All child tables should be empty");

        // Agent row still exists
        let agents = db.list_agents_db().unwrap();
        assert!(
            agents.iter().any(|a| a.id == agent_id),
            "Agent row must survive reset"
        );
    }

    #[test]
    fn test_reset_agent_active_task_guard() {
        let db = db();
        let agent_id = "test-agent";
        db.register_agent(agent_id, "Test Agent", "/tmp/test")
            .unwrap();

        // Create an in_progress task
        let mut task = make_manual_task(agent_id, "active work");
        task.trigger_type = "manual".to_string();
        let task_id = db.create_task(&task).unwrap();

        // Transition to in_progress
        db.update_task_status(&task_id, "in_progress").unwrap();

        // Active-task guard should find it
        let active = db.get_active_tasks_for_agent(agent_id).unwrap();
        assert!(!active.is_empty(), "Should detect active task");
        assert!(
            active.iter().any(|(id, _)| id == &task_id),
            "Should include the in_progress task"
        );
    }

    #[test]
    fn test_reset_agent_nonexistent() {
        let db = db();
        let result = db.reset_agent_state("nonexistent-agent");
        assert!(result.is_err(), "Should fail for nonexistent agent");
    }

    #[test]
    fn test_count_agent_state_nonexistent() {
        let db = db();
        let result = db.count_agent_state("nonexistent-agent");
        assert!(result.is_err(), "Should fail for nonexistent agent");
    }

    // --- kg_resolution_outcome_stats integration tests (#1077) ---

    /// Helper: seed a kg_subject_entity row and return its id.
    fn seed_subject_entity(db: &Database, docs_root_hash: &str, name: &str) -> i64 {
        let entity_key = format!("concept:{name}");
        db.conn
            .execute(
                "INSERT INTO kg_subject_entities (name, type, entity_key, docs_root_hash, docs_root, confidence, created_at) \
                 VALUES (?1, 'concept', ?2, ?3, '/test/path', 0.9, datetime('now'))",
                params![name, entity_key, docs_root_hash],
            )
            .unwrap();
        db.conn.last_insert_rowid()
    }

    /// Helper: seed a kg_resolutions_log row with a given outcome and age.
    fn seed_resolution_log(
        db: &Database,
        agent_id: &str,
        subject_entity_id: i64,
        outcome: &str,
        days_ago: i32,
    ) {
        let resolved_at = if days_ago == 0 {
            "datetime('now')".to_string()
        } else {
            format!("datetime('now', '-{days_ago} days')")
        };
        db.conn
            .execute(
                &format!(
                    "INSERT INTO kg_resolutions_log \
                     (agent_id, subject_entity_id, outcome, resolved_at, source_extraction_trace_id, resolution_trace_id) \
                     VALUES (?1, ?2, ?3, {resolved_at}, 'trace-test', 'trace-test')"
                ),
                params![agent_id, subject_entity_id, outcome],
            )
            .unwrap();
    }

    #[test]
    fn test_kg_outcome_stats_empty_returns_zeros() {
        let db = db();
        let stats = db.kg_resolution_outcome_stats("mika", None, 7).unwrap();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.attempted, 0);
        assert_eq!(stats.no_match, 0);
        assert_eq!(stats.no_match_rate(), 0.0);
    }

    #[test]
    fn test_kg_outcome_stats_agent_wide() {
        let db = db();
        let hash = "testhash1234";
        let agent = "mika"; // Default agent registered by schema init.

        // Seed entities
        let e1 = seed_subject_entity(&db, hash, "entity-1");
        let e2 = seed_subject_entity(&db, hash, "entity-2");
        let e3 = seed_subject_entity(&db, hash, "entity-3");
        let e4 = seed_subject_entity(&db, hash, "entity-4");
        let e5 = seed_subject_entity(&db, hash, "entity-5");

        // Seed outcomes within 7-day window
        seed_resolution_log(&db, agent, e1, "no_match", 1);
        seed_resolution_log(&db, agent, e2, "matched_exact", 2);
        seed_resolution_log(&db, agent, e3, "matched_llm", 3);
        seed_resolution_log(&db, agent, e4, "skipped_no_llm", 1);
        seed_resolution_log(&db, agent, e5, "error", 1);

        let stats = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();

        assert_eq!(stats.total, 5);
        assert_eq!(stats.attempted, 3); // no_match + matched_exact + matched_llm
        assert_eq!(stats.no_match, 1);
        assert_eq!(stats.matched_exact, 1);
        assert_eq!(stats.matched_llm, 1);
        assert_eq!(stats.skipped, 1);
        assert_eq!(stats.errors, 1);
        // no_match_rate = 1/3 ≈ 0.333
        assert!(
            (stats.no_match_rate() - 1.0 / 3.0).abs() < 1e-10,
            "Expected ~0.333, got {}",
            stats.no_match_rate()
        );
    }

    #[test]
    fn test_kg_outcome_stats_per_corpus() {
        let db = db();
        let hash_a = "hash_corpus_a";
        let hash_b = "hash_corpus_b";
        let agent = "mika";

        // Corpus A: 2 no_match, 1 matched
        let a1 = seed_subject_entity(&db, hash_a, "a-entity-1");
        let a2 = seed_subject_entity(&db, hash_a, "a-entity-2");
        let a3 = seed_subject_entity(&db, hash_a, "a-entity-3");
        seed_resolution_log(&db, agent, a1, "no_match", 1);
        seed_resolution_log(&db, agent, a2, "no_match", 2);
        seed_resolution_log(&db, agent, a3, "matched_exact", 1);

        // Corpus B: 0 no_match, 2 matched
        let b1 = seed_subject_entity(&db, hash_b, "b-entity-1");
        let b2 = seed_subject_entity(&db, hash_b, "b-entity-2");
        seed_resolution_log(&db, agent, b1, "matched_llm", 1);
        seed_resolution_log(&db, agent, b2, "matched_exact", 3);

        // Per-corpus A
        let stats_a = db
            .kg_resolution_outcome_stats(agent, Some(hash_a), 7)
            .unwrap();
        assert_eq!(stats_a.attempted, 3);
        assert_eq!(stats_a.no_match, 2);
        assert!((stats_a.no_match_rate() - 2.0 / 3.0).abs() < 1e-10);

        // Per-corpus B
        let stats_b = db
            .kg_resolution_outcome_stats(agent, Some(hash_b), 7)
            .unwrap();
        assert_eq!(stats_b.attempted, 2);
        assert_eq!(stats_b.no_match, 0);
        assert_eq!(stats_b.no_match_rate(), 0.0);

        // Agent-wide includes both corpora
        let stats_all = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();
        assert_eq!(stats_all.attempted, 5);
        assert_eq!(stats_all.no_match, 2);
    }

    #[test]
    fn test_kg_outcome_stats_window_excludes_old_rows() {
        let db = db();
        let hash = "windowhash";
        let agent = "mika";

        // 3-day-old row — inside 7-day window
        let e1 = seed_subject_entity(&db, hash, "recent-entity");
        seed_resolution_log(&db, agent, e1, "no_match", 3);

        // 8-day-old row — outside 7-day window
        let e2 = seed_subject_entity(&db, hash, "old-entity");
        seed_resolution_log(&db, agent, e2, "no_match", 8);

        let stats = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();
        assert_eq!(stats.total, 1, "8-day-old row should be excluded");
        assert_eq!(stats.no_match, 1);
    }

    #[test]
    fn test_kg_outcome_stats_all_outcome_types() {
        let db = db();
        let hash = "alloutcomes";

        let outcomes = [
            "matched_exact",
            "matched_llm",
            "matched_llm_db_fallback",
            "no_match",
            "no_candidate_of_type",
            "skipped_no_llm",
            "skipped_discovered_type",
            "skipped_discovered_subject",
            "error",
        ];
        let agent = "mika";
        for (i, outcome) in outcomes.iter().enumerate() {
            let eid = seed_subject_entity(&db, hash, &format!("e-{i}"));
            seed_resolution_log(&db, agent, eid, outcome, 1);
        }

        let stats = db.kg_resolution_outcome_stats(agent, None, 7).unwrap();
        assert_eq!(stats.total, 9);
        assert_eq!(stats.attempted, 5); // matched_exact + matched_llm + matched_llm_db_fallback + no_match + no_candidate_of_type
        assert_eq!(stats.skipped, 3); // skipped_no_llm + skipped_discovered_type + skipped_discovered_subject
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.matched_exact, 1);
        assert_eq!(stats.matched_llm, 1);
        assert_eq!(stats.matched_llm_db_fallback, 1);
        assert_eq!(stats.no_match, 1);
        assert_eq!(stats.no_candidate_of_type, 1);
    }

    // ===== task_messages (mika#974) =====

    /// Helper to create a manual task for task_messages tests.
    fn create_test_task(db: &Database, task_id: &str, task_type: &str, parent_id: Option<&str>) {
        db.conn
            .execute(
                "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type, parent_task_id)
                 VALUES (?1, 'mika', 0, 'test', 'manual', 'none', '{}', 'pending', ?2, ?3)",
                params![task_id, task_type, parent_id],
            )
            .unwrap();
    }

    #[test]
    fn test_double_write_tagged_event() {
        let (mut db, sid) = db_with_session();
        let task_id = "task-root-123";
        create_test_task(&db, task_id, "issue", None);

        let msg_id = db
            .save_message_with_task_context(
                "mika",
                &sid,
                "assistant",
                "Hello from task",
                None,
                None,
                false,
                Some(task_id),
            )
            .unwrap();
        assert!(msg_id > 0);

        // Verify row in messages
        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello from task");

        // Verify row in task_messages
        let task_msgs = db.load_task_messages(task_id).unwrap();
        assert_eq!(task_msgs.len(), 1);
        assert_eq!(task_msgs[0].content, "Hello from task");
        assert_eq!(task_msgs[0].task_id, task_id);
        assert_eq!(task_msgs[0].session_id, sid);
        assert_eq!(task_msgs[0].role, "assistant");
    }

    #[test]
    fn test_single_write_untagged_event() {
        let (mut db, sid) = db_with_session();

        db.save_message_with_task_context(
            "mika",
            &sid,
            "user",
            "No task context",
            None,
            None,
            false,
            None,
        )
        .unwrap();

        // Verify row in messages
        let msgs = db.load_recent_messages("mika", 10).unwrap();
        assert_eq!(msgs.len(), 1);

        // Verify task_messages is empty
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM task_messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_double_write_transaction_atomicity() {
        // Verify that if the task_messages INSERT would fail, messages INSERT
        // also rolls back. We test by trying to write with a task_id and verifying
        // both tables are consistent.
        let (mut db, sid) = db_with_session();

        // Write two tagged messages, verify both tables have exactly 2 rows.
        db.save_message_with_task_context(
            "mika",
            &sid,
            "user",
            "msg1",
            None,
            None,
            false,
            Some("t1"),
        )
        .unwrap();
        db.save_message_with_task_context(
            "mika",
            &sid,
            "assistant",
            "msg2",
            None,
            None,
            false,
            Some("t1"),
        )
        .unwrap();

        let msg_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE agent_id = 'mika'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let task_msg_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM task_messages WHERE task_id = 't1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(msg_count, 2);
        assert_eq!(task_msg_count, 2);
    }

    #[test]
    fn test_scope_root_walk() {
        let db = db();
        // Build tree: project → milestone → issue
        create_test_task(&db, "proj-1", "project", None);
        create_test_task(&db, "ms-1", "milestone", Some("proj-1"));
        create_test_task(&db, "issue-1", "issue", Some("ms-1"));
        // Callback child (not a scope type)
        db.conn
            .execute(
                "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type, parent_task_id)
                 VALUES ('cb-1', 'mika', 0, 'callback', 'callback', 'resume_agent', '{}', 'completed', 'issue', 'issue-1')",
                [],
            )
            .unwrap();

        // From callback child → should resolve to issue-1 (first scope root)
        let root = db.resolve_scope_root_task_id("cb-1").unwrap();
        assert_eq!(root, Some("issue-1".to_string()));

        // From issue → should resolve to itself
        let root = db.resolve_scope_root_task_id("issue-1").unwrap();
        assert_eq!(root, Some("issue-1".to_string()));

        // From milestone → should resolve to itself
        let root = db.resolve_scope_root_task_id("ms-1").unwrap();
        assert_eq!(root, Some("ms-1".to_string()));

        // From project → should resolve to itself
        let root = db.resolve_scope_root_task_id("proj-1").unwrap();
        assert_eq!(root, Some("proj-1".to_string()));
    }

    #[test]
    fn test_malformed_parent_chain() {
        let db = db();
        // Create a non-manual callback task with no parent. Its type is 'issue'
        // but it's a callback — not a scope root. The chain exhausts without
        // finding a manual scope root.
        db.conn
            .execute(
                "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type)
                 VALUES ('orphan-1', 'mika', 0, 'orphan', 'callback', 'resume_agent', '{}', 'pending', 'issue')",
                [],
            )
            .unwrap();

        // Should return None — callback tasks are not scope roots
        let root = db.resolve_scope_root_task_id("orphan-1").unwrap();
        assert_eq!(root, None);
    }

    #[test]
    fn test_scope_root_walk_depth_limit_21_hops() {
        let db = db();
        // Build a chain of 21 callback tasks (non-scope-root), no scope root reachable.
        // The depth limit is 20, so at hop 21 the guard fires and returns None.
        let mut prev_id: Option<String> = None;
        for i in 0..21 {
            let id = format!("chain-{i}");
            db.conn
                .execute(
                    "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type, parent_task_id)
                     VALUES (?1, 'mika', 0, 'chain', 'callback', 'resume_agent', '{}', 'pending', 'issue', ?2)",
                    params![id, prev_id],
                )
                .unwrap();
            prev_id = Some(id);
        }

        // Walk from the deepest node (chain-20). The chain has 21 nodes,
        // all callback (not scope roots). After 20 hops the depth limit fires.
        let root = db.resolve_scope_root_task_id("chain-20").unwrap();
        assert_eq!(
            root, None,
            "21-hop chain must hit the depth limit and return None"
        );

        // Verify a 20-hop chain with a scope root at the end DOES resolve.
        // Create a manual project at the root.
        create_test_task(&db, "scope-root", "project", None);
        // Re-parent chain-0 to point at the scope root.
        db.conn
            .execute(
                "UPDATE tasks SET parent_task_id = 'scope-root' WHERE id = 'chain-0'",
                [],
            )
            .unwrap();

        // From chain-18 (19 hops through chain + 1 hop to scope-root = 20 iterations).
        // The loop runs for 0..20, so 20 iterations fit exactly.
        let root = db.resolve_scope_root_task_id("chain-18").unwrap();
        assert_eq!(
            root,
            Some("scope-root".to_string()),
            "chain with scope-root reachable within 20 iterations should resolve"
        );

        // From chain-19 (20 hops through chain + 1 hop to scope-root = 21 iterations).
        // Exceeds the 20-iteration limit — depth guard fires.
        let root = db.resolve_scope_root_task_id("chain-19").unwrap();
        assert_eq!(
            root, None,
            "chain requiring 21 iterations must exceed depth limit"
        );

        // From chain-20 (21 hops through chain + 1 hop = 22 iterations): also exceeds.
        let root = db.resolve_scope_root_task_id("chain-20").unwrap();
        assert_eq!(
            root, None,
            "chain requiring 22 iterations must exceed depth limit"
        );
    }

    #[test]
    fn test_scope_root_walk_nonexistent_task() {
        let db = db();
        let root = db.resolve_scope_root_task_id("does-not-exist").unwrap();
        assert_eq!(root, None);
    }

    #[test]
    fn test_task_messages_survive_compaction() {
        let (mut db, sid) = db_with_session();
        let task_id = "task-survive-compaction";

        // Insert several tagged messages
        for i in 0..5 {
            db.save_message_with_task_context(
                "mika",
                &sid,
                if i % 2 == 0 { "user" } else { "assistant" },
                &format!("msg {i}"),
                None,
                None,
                false,
                Some(task_id),
            )
            .unwrap();
        }

        // Verify both tables have 5 rows
        let msgs = db.load_recent_messages("mika", 20).unwrap();
        assert_eq!(msgs.len(), 5);
        let task_msgs = db.load_task_messages(task_id).unwrap();
        assert_eq!(task_msgs.len(), 5);

        // Run compaction — this deletes from messages but NOT from task_messages
        let last_id = msgs.iter().map(|m| m.id).max().unwrap();
        db.replace_with_summary("mika", "Summary of task work", last_id)
            .unwrap();

        // messages should now have zero non-summary messages
        // (load_recent_messages filters out role='summary')
        let msgs_after = db.load_recent_messages("mika", 20).unwrap();
        assert_eq!(msgs_after.len(), 0);

        // But the summary exists in the DB
        let summary = db.load_conversation_summary("mika").unwrap();
        assert!(summary.is_some());

        // task_messages should still have all 5 rows — the structural guarantee
        let task_msgs_after = db.load_task_messages(task_id).unwrap();
        assert_eq!(task_msgs_after.len(), 5);
        for (i, tm) in task_msgs_after.iter().enumerate() {
            assert_eq!(tm.content, format!("msg {i}"));
        }
    }

    #[test]
    fn test_load_task_messages_ordered() {
        let (mut db, sid) = db_with_session();
        let task_id = "task-order";

        db.save_message_with_task_context(
            "mika",
            &sid,
            "user",
            "first",
            None,
            None,
            false,
            Some(task_id),
        )
        .unwrap();
        db.save_message_with_task_context(
            "mika",
            &sid,
            "assistant",
            "second",
            None,
            None,
            false,
            Some(task_id),
        )
        .unwrap();
        db.save_message_with_task_context(
            "mika",
            &sid,
            "user",
            "third",
            None,
            None,
            false,
            Some(task_id),
        )
        .unwrap();

        let task_msgs = db.load_task_messages(task_id).unwrap();
        assert_eq!(task_msgs.len(), 3);
        assert_eq!(task_msgs[0].content, "first");
        assert_eq!(task_msgs[1].content, "second");
        assert_eq!(task_msgs[2].content, "third");
    }

    #[test]
    fn test_insert_task_message_standalone() {
        let (db, sid) = db_with_session();
        let task_id = "task-standalone";

        let row_id = db
            .insert_task_message(
                task_id,
                "mika",
                &sid,
                "system",
                "Callback completed: session=abc, turns=5",
                Some(r#"{"claude_pilot":{"session_id":"abc","turns":5}}"#),
                Some("trace-123"),
            )
            .unwrap();
        assert!(row_id > 0);

        let msgs = db.load_task_messages(task_id).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].task_id, task_id);
        assert_eq!(msgs[0].agent_id, "mika");
        assert_eq!(msgs[0].session_id, sid);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "Callback completed: session=abc, turns=5");
        assert!(
            msgs[0]
                .metadata
                .as_deref()
                .unwrap()
                .contains("claude_pilot")
        );
        assert_eq!(msgs[0].trace_id.as_deref(), Some("trace-123"));
    }

    #[test]
    fn test_insert_task_message_with_none_optional_fields() {
        let (db, sid) = db_with_session();
        let task_id = "task-none-fields";

        let row_id = db
            .insert_task_message(task_id, "mika", &sid, "system", "summary", None, None)
            .unwrap();
        assert!(row_id > 0);

        let msgs = db.load_task_messages(task_id).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].metadata.is_none());
        assert!(msgs[0].trace_id.is_none());
    }

    /// Happy-path replay: simulate a milestone dispatch with multiple children.
    /// Verify that messages from dispatch session (turn 1), child callback (turn 2),
    /// and subsequent dispatch (turn 3) are all present in task_messages sorted by
    /// created_at. Verify rebuild_context in task-mode surfaces the full narrative
    /// from a different (callback) session.
    #[test]
    fn test_happy_path_multi_session_replay() {
        let mut db = db();
        let scope_root = "milestone-1";
        create_test_task(&db, scope_root, "milestone", None);

        // Three sessions simulating dispatch → callback → re-dispatch lifecycle.
        let sid_dispatch1 = "session-dispatch-1";
        let sid_callback = "session-callback";
        let sid_dispatch2 = "session-dispatch-2";
        db.create_session(sid_dispatch1, "mika", "cli").unwrap();
        db.create_session(sid_callback, "mika", "cli").unwrap();
        db.create_session(sid_dispatch2, "mika", "cli").unwrap();

        // Turn 1: dispatch session — orchestrator dispatches child issue #1.
        // Use explicit created_at to guarantee ordering across sessions.
        db.save_message_with_task_context(
            "mika",
            sid_dispatch1,
            "user",
            "dispatch child issue #1",
            None,
            None,
            false,
            Some(scope_root),
        )
        .unwrap();
        db.save_message_with_task_context(
            "mika",
            sid_dispatch1,
            "assistant",
            "dispatched child #1, advance to item #2",
            None,
            None,
            false,
            Some(scope_root),
        )
        .unwrap();

        // Turn 2: callback session — child #1 completes.
        db.save_message_with_task_context(
            "mika",
            sid_callback,
            "user",
            "[callback: child #1 completed]",
            None,
            None,
            false,
            Some(scope_root),
        )
        .unwrap();
        db.save_message_with_task_context(
            "mika",
            sid_callback,
            "assistant",
            "child #1 done, updating status",
            None,
            None,
            false,
            Some(scope_root),
        )
        .unwrap();

        // Turn 3: re-dispatch session — orchestrator dispatches child issue #2.
        db.save_message_with_task_context(
            "mika",
            sid_dispatch2,
            "user",
            "dispatch child issue #2",
            None,
            None,
            false,
            Some(scope_root),
        )
        .unwrap();
        db.save_message_with_task_context(
            "mika",
            sid_dispatch2,
            "assistant",
            "dispatched child #2",
            None,
            None,
            false,
            Some(scope_root),
        )
        .unwrap();

        // Verify: load_task_messages returns all 6 messages across 3 sessions,
        // in created_at order.
        let task_msgs = db.load_task_messages(scope_root).unwrap();
        assert_eq!(
            task_msgs.len(),
            6,
            "all messages across all sessions must be present"
        );
        assert_eq!(task_msgs[0].content, "dispatch child issue #1");
        assert_eq!(
            task_msgs[1].content,
            "dispatched child #1, advance to item #2"
        );
        assert_eq!(task_msgs[2].content, "[callback: child #1 completed]");
        assert_eq!(task_msgs[3].content, "child #1 done, updating status");
        assert_eq!(task_msgs[4].content, "dispatch child issue #2");
        assert_eq!(task_msgs[5].content, "dispatched child #2");

        // Verify: sessions span 3 distinct session IDs.
        let session_ids: std::collections::HashSet<&str> =
            task_msgs.iter().map(|m| m.session_id.as_str()).collect();
        assert_eq!(session_ids.len(), 3);

        // Verify: rebuild_context from the callback session with task-mode
        // surfaces the "advance to item #2" intent from the dispatch session.
        let ctx = db.rebuild_context("mika", Some(scope_root), 20).unwrap();
        assert!(
            ctx.iter().any(|m| m.content.contains("advance to item #2")),
            "rebuild_context in task-mode must surface cross-session dispatch intent"
        );

        // Verify: messages table also has rows (double-write contract).
        let channel_msgs = db.load_recent_messages("mika", 100).unwrap();
        assert_eq!(
            channel_msgs.len(),
            6,
            "messages table should also have all 6 rows"
        );

        // Verify: after compaction, task_messages still has all 6 rows.
        let last_id = channel_msgs.iter().map(|m| m.id).max().unwrap();
        db.replace_with_summary("mika", "Summary of milestone work", last_id)
            .unwrap();
        let channel_msgs_after = db.load_recent_messages("mika", 100).unwrap();
        assert_eq!(channel_msgs_after.len(), 0, "channel messages compacted");
        let task_msgs_after = db.load_task_messages(scope_root).unwrap();
        assert_eq!(
            task_msgs_after.len(),
            6,
            "task narrative survives compaction"
        );

        // Verify: rebuild_context still surfaces full narrative post-compaction.
        let ctx_after = db.rebuild_context("mika", Some(scope_root), 20).unwrap();
        assert_eq!(
            ctx_after.len(),
            6,
            "task-mode rebuild returns full narrative post-compaction"
        );
        assert!(
            ctx_after
                .iter()
                .any(|m| m.content.contains("advance to item #2"))
        );
    }
    // ── Force-promote regression tests (mika#1453) ──────────────────────

    /// Helper: create a deferred callback with a specific dispatch class.
    fn deferred_wrapper(agent: &str, class: &str) -> NewTask {
        let mut t = new_task(
            agent,
            "long_running:run_claude_pilot:deferred",
            "callback",
            "resume_agent",
        );
        t.dispatch_class = Some(class.to_string());
        t
    }

    /// Helper: create a real (non-deferred) callback with a specific dispatch class.
    fn real_callback(agent: &str, class: &str) -> NewTask {
        let mut t = new_task(
            agent,
            "long_running:run_claude_pilot",
            "callback",
            "resume_agent",
        );
        t.dispatch_class = Some(class.to_string());
        t
    }

    /// AC5a (mika#1453): 2 pending wrappers + 1 in-flight real callback →
    /// force-promote WITHOUT override → RejectedSlotBusy, no state mutation.
    #[test]
    fn test_force_promote_rejected_slot_busy_no_state_mutation() {
        let db = db();

        // Parent tasks (FK requirement for callbacks).
        let p1 = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();
        let p2 = db
            .create_task(&new_task("mika", "p2", "manual", "none"))
            .unwrap();
        let p3 = db
            .create_task(&new_task("mika", "p3", "manual", "none"))
            .unwrap();

        // 2 pending deferred wrappers.
        let mut d1 = deferred_wrapper("mika", "implement");
        d1.parent_task_id = Some(p1.clone());
        let d1_id = db.create_task(&d1).unwrap();

        let mut d2 = deferred_wrapper("mika", "implement");
        d2.parent_task_id = Some(p2.clone());
        let d2_id = db.create_task(&d2).unwrap();

        // 1 in-flight real callback (pending status occupies the slot).
        let mut rc = real_callback("mika", "implement");
        rc.parent_task_id = Some(p3.clone());
        let rc_id = db.create_task(&rc).unwrap();
        // Transition to in_progress to simulate a dispatched subprocess.
        db.claim_and_fire_task(&rc_id, "mika").unwrap();

        // Force-promote should be rejected.
        let result = db
            .force_promote_deferred_for_class("mika", "implement")
            .unwrap();
        assert!(
            matches!(result, ForcePromoteResult::RejectedSlotBusy { .. }),
            "Expected RejectedSlotBusy, got {result:?}"
        );

        // Both deferred wrappers should still be pending (no state mutation).
        let t1 = db.get_task_unscoped(&d1_id).unwrap().unwrap();
        assert_eq!(
            t1.status, "pending",
            "deferred wrapper 1 should remain pending"
        );
        let t2 = db.get_task_unscoped(&d2_id).unwrap().unwrap();
        assert_eq!(
            t2.status, "pending",
            "deferred wrapper 2 should remain pending"
        );

        // Real callback should still be in_progress.
        let rc_task = db.get_task_unscoped(&rc_id).unwrap().unwrap();
        assert_eq!(
            rc_task.status, "in_progress",
            "real callback should remain in_progress"
        );

        // AC5a audit assertion: simulate the caller emitting the rejection
        // audit event (the tool/CLI layer is responsible for logging), then
        // verify exactly one row with the expected event type was stored.
        db.log_audit_event(
            "mika",
            "test-session",
            "deferred_dispatch_force_promote_rejected_slot_busy",
            "dispatch_class:implement",
            None,
            Some("rejected_slot_busy"),
            None,
            None,
        )
        .unwrap();
        let audit_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE tool_name = ?1",
                params!["deferred_dispatch_force_promote_rejected_slot_busy"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            audit_count, 1,
            "exactly one rejected_slot_busy audit event should be emitted"
        );
    }

    /// AC5b (mika#1453): Same setup; cancel the blocker, then force-promote →
    /// oldest deferred wrapper promoted, per-class invariant holds (≤1 active
    /// real callback).
    #[test]
    fn test_force_promote_override_cancel_then_promote() {
        let db = db();

        // Parent tasks (FK requirement for callbacks).
        let p1 = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();
        let p2 = db
            .create_task(&new_task("mika", "p2", "manual", "none"))
            .unwrap();
        let p3 = db
            .create_task(&new_task("mika", "p3", "manual", "none"))
            .unwrap();

        // 2 pending deferred wrappers.
        let mut d1 = deferred_wrapper("mika", "implement");
        d1.parent_task_id = Some(p1.clone());
        let d1_id = db.create_task(&d1).unwrap();

        let mut d2 = deferred_wrapper("mika", "implement");
        d2.parent_task_id = Some(p2.clone());
        let d2_id = db.create_task(&d2).unwrap();

        // 1 in-flight real callback.
        let mut rc = real_callback("mika", "implement");
        rc.parent_task_id = Some(p3.clone());
        let rc_id = db.create_task(&rc).unwrap();
        db.claim_and_fire_task(&rc_id, "mika").unwrap();

        // Step 1: Confirm slot is busy.
        let result = db
            .force_promote_deferred_for_class("mika", "implement")
            .unwrap();
        assert!(matches!(
            result,
            ForcePromoteResult::RejectedSlotBusy { .. }
        ));

        // Step 2: Find and identify the blocker.
        let blocker = db
            .find_active_callback_for_class("mika", "implement")
            .unwrap();
        assert_eq!(blocker.as_deref(), Some(rc_id.as_str()));

        // Step 3: Cancel the blocker (simulating `cancel_task_and_kill`).
        db.cancel_task(&rc_id, "mika").unwrap();

        // Step 4: Retry force-promote — should succeed now.
        let result = db
            .force_promote_deferred_for_class("mika", "implement")
            .unwrap();
        match &result {
            ForcePromoteResult::Promoted { task_id } => {
                assert_eq!(
                    task_id, &d1_id,
                    "oldest deferred wrapper should be promoted first"
                );
            }
            other => panic!("Expected Promoted, got {other:?}"),
        }

        // Step 5: Verify only the first wrapper was promoted.
        let t1 = db.get_task_unscoped(&d1_id).unwrap().unwrap();
        assert_eq!(
            t1.status, "completed",
            "first wrapper should be promoted (completed)"
        );

        let t2 = db.get_task_unscoped(&d2_id).unwrap().unwrap();
        assert_eq!(t2.status, "pending", "second wrapper should remain pending");

        // Step 6: Verify per-class invariant — no real callback active.
        assert!(
            !db.has_any_active_callback_for_class("mika", "implement")
                .unwrap(),
            "no active real callback should remain after cancel"
        );
    }

    #[test]
    fn test_force_promote_slot_free_promoted() {
        let db = db();
        let p1 = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();

        let mut d1 = deferred_wrapper("mika", "implement");
        d1.parent_task_id = Some(p1.clone());
        let d1_id = db.create_task(&d1).unwrap();

        let result = db
            .force_promote_deferred_for_class("mika", "implement")
            .unwrap();
        match &result {
            ForcePromoteResult::Promoted { task_id } => {
                assert_eq!(task_id, &d1_id);
            }
            other => panic!("Expected Promoted, got {other:?}"),
        }
    }

    #[test]
    fn test_force_promote_slot_free_no_pending_wrapper() {
        let db = db();
        let result = db
            .force_promote_deferred_for_class("mika", "implement")
            .unwrap();
        assert!(
            matches!(result, ForcePromoteResult::NoPendingWrapper),
            "Expected NoPendingWrapper, got {result:?}"
        );
    }

    #[test]
    fn test_find_active_callback_for_class_excludes_deferred() {
        let db = db();
        let p1 = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();

        // Only a deferred wrapper exists — should return None.
        let mut d1 = deferred_wrapper("mika", "implement");
        d1.parent_task_id = Some(p1.clone());
        db.create_task(&d1).unwrap();

        let blocker = db
            .find_active_callback_for_class("mika", "implement")
            .unwrap();
        assert!(
            blocker.is_none(),
            "deferred wrappers should not count as slot occupiers"
        );
    }

    #[test]
    fn test_find_active_callback_for_class_returns_real_callback() {
        let db = db();
        let p1 = db
            .create_task(&new_task("mika", "p1", "manual", "none"))
            .unwrap();

        let mut rc = real_callback("mika", "implement");
        rc.parent_task_id = Some(p1.clone());
        let rc_id = db.create_task(&rc).unwrap();

        let blocker = db
            .find_active_callback_for_class("mika", "implement")
            .unwrap();
        assert_eq!(blocker.as_deref(), Some(rc_id.as_str()));
    }

    // --- cancel_orphan_recurring_tasks (mika#1436) ---

    fn recurring_task(agent: &str, label: &str) -> NewTask {
        NewTask {
            agent_id: agent.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: label.to_string(),
            trigger_type: "recurring".to_string(),
            cron_expr: Some("0 0 * * * *".to_string()),
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        }
    }

    #[test]
    fn test_cancel_orphan_recurring_tasks_one_orphan() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();

        let id_a = db
            .create_task(&recurring_task("agent-a", "heartbeat"))
            .unwrap();
        let id_b = db
            .create_task(&recurring_task("agent-b", "heartbeat"))
            .unwrap();

        // Only agent-a is known (agent-b was deleted from disk).
        let orphans = db
            .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
            .unwrap();

        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].0, id_b);
        assert_eq!(orphans[0].1, "agent-b");

        // agent-a's task is unchanged (create_task sets status to 'pending').
        let ta = db.get_task(&id_a, "agent-a").unwrap().unwrap();
        assert_eq!(ta.status, "pending");

        // agent-b's task is cancelled.
        let tb = db.get_task(&id_b, "agent-b").unwrap().unwrap();
        assert_eq!(tb.status, "cancelled");
    }

    #[test]
    fn test_cancel_orphan_recurring_tasks_no_orphans() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();

        db.create_task(&recurring_task("agent-a", "heartbeat"))
            .unwrap();

        let orphans = db
            .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
            .unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn test_cancel_orphan_recurring_tasks_multiple_orphans_same_agent() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("mika-relay", "Relay", "/tmp/relay")
            .unwrap();

        db.create_task(&recurring_task("agent-a", "heartbeat"))
            .unwrap();
        let r1 = db
            .create_task(&recurring_task("mika-relay", "heartbeat"))
            .unwrap();
        let r2 = db
            .create_task(&recurring_task("mika-relay", "reflection"))
            .unwrap();

        let orphans = db
            .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
            .unwrap();

        assert_eq!(orphans.len(), 2);
        let orphan_ids: Vec<&str> = orphans.iter().map(|(id, _)| id.as_str()).collect();
        assert!(orphan_ids.contains(&r1.as_str()));
        assert!(orphan_ids.contains(&r2.as_str()));

        // Both relay tasks cancelled.
        let t1 = db.get_task(&r1, "mika-relay").unwrap().unwrap();
        let t2 = db.get_task(&r2, "mika-relay").unwrap().unwrap();
        assert_eq!(t1.status, "cancelled");
        assert_eq!(t2.status, "cancelled");
    }

    #[test]
    fn test_cancel_orphan_recurring_tasks_already_cancelled_idempotent() {
        let db = db();
        db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();

        let id = db
            .create_task(&recurring_task("agent-b", "heartbeat"))
            .unwrap();
        // Pre-cancel the task.
        db.cancel_task(&id, "agent-b").unwrap();

        let orphans = db
            .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
            .unwrap();
        // Already cancelled — not in the active set, so not returned.
        assert!(orphans.is_empty());
    }

    #[test]
    fn test_cancel_orphan_recurring_tasks_empty_known_set_returns_empty() {
        let db = db();
        db.register_agent("agent-a", "Agent A", "/tmp/a").unwrap();
        db.create_task(&recurring_task("agent-a", "heartbeat"))
            .unwrap();

        // Empty known set triggers early return (safety guard).
        let orphans = db.cancel_orphan_recurring_tasks(&[]).unwrap();
        assert!(orphans.is_empty());

        // Task should still be active (no mutation on empty set).
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE agent_id = 'agent-a' AND status IN ('pending', 'recurring_active')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_cancel_orphan_recurring_tasks_skips_manual_tasks() {
        let db = db();
        db.register_agent("agent-b", "Agent B", "/tmp/b").unwrap();

        // Create a manual task for agent-b (should NOT be cancelled).
        let manual_id = db
            .create_task(&new_task("agent-b", "some-work", "manual", "none"))
            .unwrap();

        // Create a recurring task for agent-b (should be cancelled).
        let recurring_id = db
            .create_task(&recurring_task("agent-b", "heartbeat"))
            .unwrap();

        let orphans = db
            .cancel_orphan_recurring_tasks(&["agent-a".to_string()])
            .unwrap();

        // Only the recurring task is cancelled.
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].0, recurring_id);

        // Manual task is still pending.
        let mt = db.get_task(&manual_id, "agent-b").unwrap().unwrap();
        assert_eq!(mt.status, "pending");
    }

    #[test]
    fn test_get_last_cli_session_returns_none_when_empty() {
        let db = db();
        let result = db.get_last_cli_session_for_agent("mika").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_cli_session_returns_most_recent_ended() {
        let db = db();
        // Create two ended CLI sessions with distinct started_at timestamps
        db.create_session("s1", "mika", "cli").unwrap();
        db.end_session("s1").unwrap();
        // Manually set s1 to an earlier timestamp to ensure deterministic ordering
        db.conn
            .execute(
                "UPDATE sessions SET started_at = '2026-01-01T00:00:00Z' WHERE id = 's1'",
                [],
            )
            .unwrap();
        db.create_session("s2", "mika", "cli").unwrap();
        db.end_session("s2").unwrap();

        let result = db.get_last_cli_session_for_agent("mika").unwrap().unwrap();
        assert_eq!(result.id, "s2");
    }

    #[test]
    fn test_get_last_cli_session_excludes_non_cli_channels() {
        let db = db();
        db.create_session("tg1", "mika", "telegram").unwrap();
        db.end_session("tg1").unwrap();
        db.create_session("wh1", "mika", "webhook").unwrap();
        db.end_session("wh1").unwrap();

        let result = db.get_last_cli_session_for_agent("mika").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_cli_session_excludes_system_sessions() {
        let db = db();
        db.create_session("system-mika", "mika", "cli").unwrap();
        db.end_session("system-mika").unwrap();

        let result = db.get_last_cli_session_for_agent("mika").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_cli_session_excludes_delegate_sessions() {
        let db = db();
        db.create_session("delegate-abc", "mika", "cli").unwrap();
        db.end_session("delegate-abc").unwrap();

        let result = db.get_last_cli_session_for_agent("mika").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_last_cli_session_excludes_child_sessions() {
        let db = db();
        // Parent session
        db.create_session("parent", "mika", "cli").unwrap();
        db.end_session("parent").unwrap();
        // Child session with parent_session_id set
        db.create_session_with_parent("child", "mika", "cli", None, Some("parent"), None)
            .unwrap();
        db.end_session("child").unwrap();

        let result = db.get_last_cli_session_for_agent("mika").unwrap().unwrap();
        // Only the parent (no parent_session_id) is returned
        assert_eq!(result.id, "parent");
    }

    #[test]
    fn test_get_last_cli_session_excludes_active_sessions() {
        let db = db();
        // Active session (ended_at IS NULL)
        db.create_session("active", "mika", "cli").unwrap();

        let result = db.get_last_cli_session_for_agent("mika").unwrap();
        assert!(result.is_none());
    }

    // --- has_completed_groom_for_issue (#1620) ---

    fn groom_task(agent_id: &str, reference_url: &str) -> NewTask {
        NewTask {
            agent_id: agent_id.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "groom senara-solutions/mika#123".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: Some(reference_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("groom".to_string()),
        }
    }

    #[test]
    fn test_groom_cross_check_no_task_returns_false() {
        let db = db();
        let result = db
            .has_completed_groom_for_issue(
                "mika",
                "https://github.com/senara-solutions/mika/issues/123",
            )
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn test_groom_cross_check_completed_task_returns_true() {
        let db = db();
        let issue_url = "https://github.com/senara-solutions/mika/issues/123";
        let groom_url = format!("{}?phase=groom", issue_url);
        let task = groom_task("mika", &groom_url);
        let id = db.create_task(&task).unwrap();
        db.update_task_status(&id, "completed").unwrap();

        let result = db.has_completed_groom_for_issue("mika", issue_url).unwrap();
        assert!(result);
    }

    #[test]
    fn test_groom_cross_check_delivered_task_returns_true() {
        let db = db();
        let issue_url = "https://github.com/senara-solutions/mika/issues/123";
        let groom_url = format!("{}?phase=groom", issue_url);
        let task = groom_task("mika", &groom_url);
        let id = db.create_task(&task).unwrap();
        db.update_task_status(&id, "completed").unwrap();
        db.update_task_status(&id, "delivered").unwrap();

        let result = db.has_completed_groom_for_issue("mika", issue_url).unwrap();
        assert!(result);
    }

    #[test]
    fn test_groom_cross_check_pending_task_returns_false() {
        let db = db();
        let issue_url = "https://github.com/senara-solutions/mika/issues/123";
        let groom_url = format!("{}?phase=groom", issue_url);
        let task = groom_task("mika", &groom_url);
        db.create_task(&task).unwrap();
        // Status is "pending" (default) — should not satisfy the gate

        let result = db.has_completed_groom_for_issue("mika", issue_url).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_groom_cross_check_implement_class_returns_false() {
        let db = db();
        let issue_url = "https://github.com/senara-solutions/mika/issues/123";
        let groom_url = format!("{}?phase=groom", issue_url);
        let mut task = groom_task("mika", &groom_url);
        task.dispatch_class = Some("implement".to_string());
        let id = db.create_task(&task).unwrap();
        db.update_task_status(&id, "completed").unwrap();

        let result = db.has_completed_groom_for_issue("mika", issue_url).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_groom_cross_check_different_agent_returns_false() {
        let db = db();
        db.register_agent("other-agent", "Other", "").unwrap();
        let issue_url = "https://github.com/senara-solutions/mika/issues/123";
        let groom_url = format!("{}?phase=groom", issue_url);
        let task = groom_task("other-agent", &groom_url);
        let id = db.create_task(&task).unwrap();
        db.update_task_status(&id, "completed").unwrap();

        // Query for "mika" agent — should not find the other-agent's task
        let result = db.has_completed_groom_for_issue("mika", issue_url).unwrap();
        assert!(!result);
    }
}
