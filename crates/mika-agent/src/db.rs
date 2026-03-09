use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;
use tracing::{debug, info};
use utoipa::ToSchema;

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

pub const CURRENT_SCHEMA_VERSION: i64 = 7;

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
        target_key || ': ' || COALESCE(before_value, '(none)') || ' -> ' || after_value AS summary, \
        created_at \
    FROM audit_events \
    UNION ALL \
    SELECT created_trace_id AS trace_id, created_by_session AS session_id, agent_id, \
        'task' AS event_type, action_type AS event_subtype, \
        label || ' [' || status || ']' AS summary, \
        created_at \
    FROM tasks";

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
    pub last_seen: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct TeamRow {
    pub id: String,
    pub name: String,
    pub config_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub agent_id: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub label: String,
    pub trigger_type: String,
    pub cron_expr: Option<String>,
    pub event_source: Option<String>,
    pub event_offset_secs: Option<i64>,
    pub condition_expr: Option<String>,
    pub next_fire_at: Option<i64>,
    pub timeout_at: Option<i64>,
    pub action_type: String,
    pub action_config: String,
    pub status: String,
    pub process_id: Option<i64>,
    pub input_context: Option<String>,
    pub result: Option<String>,
    pub created_by_session: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub fired_at: Option<i64>,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub agent_id: String,
    pub team_run_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub depth: i64,
    pub label: String,
    pub trigger_type: String,
    pub cron_expr: Option<String>,
    pub event_source: Option<String>,
    pub event_offset_secs: Option<i64>,
    pub condition_expr: Option<String>,
    pub next_fire_at: Option<i64>,
    pub timeout_at: Option<i64>,
    pub action_type: String,
    pub action_config: String,
    pub input_context: Option<String>,
    pub created_by_session: Option<String>,
    pub created_trace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub metadata: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CoreMemoryEntry {
    pub key: String,
    pub value: String,
    pub token_count: i32,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct Person {
    pub id: i64,
    pub canonical_name: String,
    pub relationship: Option<String>,
    pub notes: Option<String>,
    pub first_mentioned: String,
    pub last_mentioned: String,
    pub mention_count: i64,
}

#[derive(Debug, Clone)]
pub struct Commitment {
    pub id: i64,
    pub description: String,
    pub status: String,
    pub due_date: Option<String>,
    pub person_id: Option<i64>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Preference {
    pub category: String,
    pub value: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub id: i64,
    pub description: String,
    pub event_date: Option<String>,
    pub context: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditEvent {
    pub id: i64,
    pub session_id: String,
    pub tool_name: String,
    pub target_key: String,
    pub before_value: Option<String>,
    pub after_value: String,
    pub reasoning: Option<String>,
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

#[derive(Debug, Clone)]
pub struct TeamRunRow {
    pub id: String,
    pub team_name: String,
    pub goal: String,
    pub status: String,
    pub failure_reason: Option<String>,
    pub iteration: u32,
    pub max_iterations: u32,
    pub deliverable: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TeamWorkspaceEntry {
    pub id: i64,
    pub run_id: String,
    pub parent_id: Option<i64>,
    pub agent_name: Option<String>,
    pub entry_type: String,
    pub content: String,
    pub iteration: u32,
    pub created_at: i64,
}

// ===== Skill Override Types =====

/// A user override for a skill property (persists across bundled skill re-sync).
#[derive(Debug, Clone)]
pub struct SkillOverride {
    pub skill_name: String,
    pub always_on: Option<bool>,
}

// ===== Dashboard Types =====

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TimelineRow {
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: String,
    pub event_type: String,
    pub event_subtype: String,
    pub summary: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TimelineFilters {
    pub agent_id: Option<String>,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
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
        if let Some(from) = self.from {
            params.push(rusqlite::types::Value::Integer(from));
            conditions.push(format!("created_at >= ?{}", params.len()));
        }
        if let Some(to) = self.to {
            params.push(rusqlite::types::Value::Integer(to));
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
    pub last_seen: Option<i64>,
    pub created_at: i64,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionWithStats {
    pub id: String,
    pub agent_id: String,
    pub channel_type: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata: Option<String>,
    pub message_count: i64,
}

// ===== Database =====

pub struct Database {
    conn: Connection,
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
        let db = Self { conn };
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
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
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

    fn migrate(&self) -> Result<()> {
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
        Ok(())
    }

    fn migrate_v1(&self) -> Result<()> {
        info!("applying migration v1: unified task engine schema (clean slate)");

        // Drop all existing tables (clean slate — no backward compat constraint)
        let drops = [
            "DROP TABLE IF EXISTS fts_search",
            "DROP TABLE IF EXISTS vec_search",
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

        self.conn
            .execute_batch(
                "
            BEGIN;

            CREATE TABLE schema_version (
                version INTEGER NOT NULL,
                applied_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            INSERT INTO schema_version (version) VALUES (7);

            CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen INTEGER,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE teams (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
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
                started_at INTEGER NOT NULL DEFAULT (unixepoch()),
                ended_at INTEGER
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
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition')
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
                        'run_skill','invoke_orchestrator'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                fired_at INTEGER,
                completed_at INTEGER
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

            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at INTEGER NOT NULL DEFAULT (unixepoch()),
                ended_at INTEGER,
                metadata TEXT
            );
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);

            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;

            CREATE TABLE core_memory (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE people (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned INTEGER NOT NULL DEFAULT (unixepoch()),
                last_mentioned INTEGER NOT NULL DEFAULT (unixepoch()),
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
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                completed_at INTEGER
            );
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                ON commitments(agent_id, description COLLATE NOCASE, due_date)
                WHERE status = 'pending';

            CREATE TABLE preferences (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (agent_id, category)
            );

            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT NOT NULL,
                reasoning TEXT,
                trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;

            CREATE TABLE audit_event_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE (agent_id, year, month)
            );

            CREATE TABLE search_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch())
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
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);

            CREATE TABLE heartbeat_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);

            CREATE TABLE reflection_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);

            CREATE TABLE customer_config (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE failed_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE TABLE skill_overrides (
                agent_id   TEXT NOT NULL COLLATE NOCASE,
                skill_name TEXT NOT NULL COLLATE NOCASE,
                always_on  INTEGER,
                PRIMARY KEY (agent_id, skill_name)
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND action_type = 'send_message';

            CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

            -- Pre-register the default 'mika' agent
            INSERT INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');

            COMMIT;
            ",
            )
            .context("failed to create v1 schema")?;

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
    fn migrate_v3(&self) -> Result<()> {
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
                AND action_type = 'send_message';

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
            "UPDATE agents SET last_seen = unixepoch() WHERE id = ?1",
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
        let mut stmt = self
            .conn
            .prepare("SELECT skill_name, always_on FROM skill_overrides WHERE agent_id = ?1")?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
                Ok(SkillOverride {
                    skill_name: r.get(0)?,
                    always_on: r.get(1)?,
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

    /// Delete an override for a skill (revert to bundled default).
    pub fn delete_skill_override(&self, agent_id: &str, skill_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM skill_overrides WHERE agent_id = ?1 AND skill_name = ?2",
            params![agent_id, skill_name],
        )?;
        Ok(())
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
        self.conn.execute(
            "INSERT INTO tasks (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                input_context, created_by_session, created_trace_id
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17, ?18
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
            ],
        )?;
        Ok(id)
    }

    /// Insert a recurring task only if no task with the same (agent_id, label) and
    /// trigger_type='recurring' exists. Returns the task ID if created, None if already existed.
    pub fn create_recurring_task_if_absent(&self, task: NewTask) -> Result<Option<String>> {
        let id = uuid::Uuid::new_v4().to_string();
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO tasks
             (id, agent_id, team_run_id, parent_task_id, depth, label,
              trigger_type, cron_expr, event_source, event_offset_secs,
              condition_expr, next_fire_at, timeout_at, action_type,
              action_config, status, input_context, created_by_session, created_trace_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'recurring_active',?16,?17,?18)",
            params![
                id, task.agent_id, task.team_run_id, task.parent_task_id,
                task.depth, task.label, task.trigger_type, task.cron_expr,
                task.event_source, task.event_offset_secs, task.condition_expr,
                task.next_fire_at, task.timeout_at, task.action_type,
                task.action_config, task.input_context, task.created_by_session,
                task.created_trace_id
            ],
        )?;
        if n > 0 {
            Ok(Some(id))
        } else {
            Ok(None) // already existed
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
        next_fire_at: i64,
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
            "UPDATE tasks SET status = 'cancelled', updated_at = unixepoch()
             WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring'
               AND status NOT IN ('completed','failed','cancelled','expired')",
            params![agent_id, label],
        )?;
        Ok(())
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
            created_at: r.get(20)?,
            updated_at: r.get(21)?,
            fired_at: r.get(22)?,
            completed_at: r.get(23)?,
        })
    }

    const TASK_COLUMNS: &'static str = "id, agent_id, team_run_id, parent_task_id, depth, label,
         trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
         next_fire_at, timeout_at, action_type, action_config,
         status, process_id, input_context, result, created_by_session,
         created_at, updated_at, fired_at, completed_at";

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

    pub fn get_schedulable_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
               AND trigger_type != 'callback'
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
            "UPDATE tasks SET status = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// Atomically claim a task and record its fired_at in a single UPDATE.
    /// Returns true if the task was claimed (was in 'pending' or 'recurring_active' state).
    /// Returns false if the task was already claimed, cancelled, or completed.
    pub fn claim_and_fire_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'in_progress',
                              fired_at = unixepoch(),
                              updated_at = unixepoch()
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
             completed_at = unixepoch(), updated_at = unixepoch()
             WHERE id = ?2 AND agent_id = ?3 AND status IN ('pending', 'in_progress')",
            params![result, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_failed(&self, id: &str, agent_id: &str, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = 'failed', result = ?1,
             completed_at = unixepoch(), updated_at = unixepoch()
             WHERE id = ?2 AND agent_id = ?3",
            params![error, id, agent_id],
        )?;
        Ok(())
    }

    pub fn update_task_next_fire_at(&self, id: &str, next_fire_at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    /// Atomically reschedule a recurring task: set next_fire_at and status = 'recurring_active'
    /// in a single UPDATE, replacing two sequential writes.
    pub fn update_task_rescheduled(&self, id: &str, next_fire_at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, status = 'recurring_active', updated_at = unixepoch() WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    pub fn cancel_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = unixepoch()
             WHERE id = ?1 AND agent_id = ?2 AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    pub fn mark_tasks_expired(&self, now_unix: i64, agent_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'expired', updated_at = unixepoch()
             WHERE agent_id = ?2
               AND timeout_at IS NOT NULL AND timeout_at < ?1
               AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![now_unix, agent_id],
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

    /// Get all pending user-visible tasks (reminders and callbacks, excludes heartbeat/reflection).
    pub fn get_user_visible_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND (action_type = 'send_message'
                 OR (trigger_type = 'callback' AND action_type = 'resume_agent'))
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

    /// Get callback tasks that completed but have not yet been delivered to the user.
    /// Bounded by `since_unix` to avoid processing stale callbacks.
    pub fn get_undelivered_callback_tasks(
        &self,
        agent_id: &str,
        since_unix: i64,
    ) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status = 'completed'
               AND completed_at IS NOT NULL
               AND completed_at > ?2
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since_unix], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Atomically mark a completed callback task as delivered.
    /// Returns `false` if the task was already claimed (not in 'completed' status).
    pub fn mark_task_delivered(&self, id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'delivered', updated_at = unixepoch()
             WHERE id = ?1 AND status = 'completed'",
            params![id],
        )?;
        Ok(n > 0)
    }

    pub fn set_task_process_id(&self, id: &str, process_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET process_id = ?1, updated_at = unixepoch() WHERE id = ?2",
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
            "UPDATE tasks SET process_id = NULL, updated_at = unixepoch()
             WHERE id = ?1 AND agent_id = ?2",
            params![id, agent_id],
        )?;
        Ok(())
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
            "UPDATE tasks SET status = 'in_progress', fired_at = unixepoch(), updated_at = unixepoch()
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

    pub fn prune_completed_tasks(&self, older_than_secs: i64) -> Result<usize> {
        let cutoff = Utc::now().timestamp() - older_than_secs;
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE status IN ('completed','cancelled','expired','failed','delivered')
             AND completed_at IS NOT NULL AND completed_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    pub fn get_tasks_by_status(&self, agent_id: &str, statuses: &[&str]) -> Result<Vec<Task>> {
        if statuses.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: String = (1..=statuses.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {} FROM tasks WHERE agent_id = ?1 AND status IN ({}) ORDER BY created_at DESC",
            Self::TASK_COLUMNS,
            placeholders
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id.to_string())];
        for s in statuses {
            bind.push(Box::new(s.to_string()));
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
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id, agent_id, channel_type, metadata],
        )?;
        Ok(())
    }

    pub fn end_session(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at = unixepoch() WHERE id = ?1",
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

    pub fn save_message_with_metadata(
        &self,
        agent_id: &str,
        session_id: &str,
        role: &str,
        content: &str,
        metadata: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, metadata, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, agent_id, role, content, metadata, trace_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn row_to_session_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMessage> {
        Ok(SessionMessage {
            id: r.get(0)?,
            session_id: r.get(1)?,
            role: r.get(2)?,
            content: r.get(3)?,
            channel_type: r.get(4)?,
            metadata: r.get(5)?,
            created_at: r.get::<_, i64>(6)?,
        })
    }

    pub fn load_recent_messages(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND s.channel_type != 'team'
              ORDER BY m.created_at DESC, m.id DESC LIMIT ?2",
        )?;
        let mut messages = stmt
            .query_map(
                params![agent_id, limit as i64],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        messages.reverse();
        Ok(messages)
    }

    pub fn load_conversation_summary(&self, agent_id: &str) -> Result<Option<SessionMessage>> {
        self.conn
            .query_row(
                "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
                  FROM messages m JOIN sessions s ON m.session_id = s.id
                  WHERE m.agent_id = ?1 AND m.role = 'summary'
                  ORDER BY m.created_at DESC LIMIT 1",
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
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.role != 'summary' AND m.id <= ?2
              ORDER BY m.created_at ASC, m.id ASC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, cutoff_id], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn replace_with_summary(
        &self,
        agent_id: &str,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        let system_session = self.get_or_create_system_session(agent_id)?;
        self.conn.execute_batch("BEGIN")?;
        // Delete old non-summary messages up to compacted_through_id
        self.conn.execute(
            "DELETE FROM messages
             WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2",
            params![agent_id, compacted_through_id],
        )?;
        // Remove old summary
        self.conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND role = 'summary'",
            params![agent_id],
        )?;
        // Insert new summary
        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, compacted_through_id)
             VALUES (?1, ?2, 'summary', ?3, ?4)",
            params![system_session, agent_id, summary, compacted_through_id],
        )?;
        let row_id = self.conn.last_insert_rowid();
        self.conn.execute_batch("COMMIT")?;
        Ok(row_id)
    }

    pub fn load_messages_after(
        &self,
        agent_id: &str,
        after_id: i64,
    ) -> Result<Vec<SessionMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.id > ?2
              ORDER BY m.created_at ASC, m.id ASC",
        )?;
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
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.id = ?1",
        )?;
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
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.session_id = ?1
                AND (m.id >= (SELECT id FROM (SELECT id FROM messages WHERE session_id = ?1 AND id <= ?2 ORDER BY id DESC LIMIT ?3) sub ORDER BY id ASC LIMIT 1))
                AND m.id <= (SELECT id FROM (SELECT id FROM messages WHERE session_id = ?1 AND id >= ?2 ORDER BY id ASC LIMIT ?4) sub ORDER BY id DESC LIMIT 1)
              ORDER BY m.created_at ASC, m.id ASC",
        )?;
        let rows = stmt
            .query_map(
                params![session_id, target_id, before + 1, after + 1],
                Self::row_to_session_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_messages_since(
        &self,
        agent_id: &str,
        since_unix: i64,
    ) -> Result<Vec<SessionMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.agent_id = ?1 AND m.created_at >= ?2 AND m.role != 'summary'
              ORDER BY m.created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, since_unix], Self::row_to_session_message)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn last_user_message_time(&self, agent_id: &str) -> Result<Option<i64>> {
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

    pub fn get_core_memory(&self, agent_id: &str, key: &str) -> Result<Option<CoreMemoryEntry>> {
        self.conn
            .query_row(
                "SELECT key, value, token_count, datetime(updated_at, 'unixepoch')
                  FROM core_memory WHERE agent_id = ?1 AND key = ?2",
                params![agent_id, key],
                |r| {
                    Ok(CoreMemoryEntry {
                        key: r.get(0)?,
                        value: r.get(1)?,
                        token_count: r.get(2)?,
                        updated_at: r.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_all_core_memory(&self, agent_id: &str) -> Result<Vec<CoreMemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, value, token_count, datetime(updated_at, 'unixepoch')
              FROM core_memory WHERE agent_id = ?1 ORDER BY key",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
                Ok(CoreMemoryEntry {
                    key: r.get(0)?,
                    value: r.get(1)?,
                    token_count: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Insert or update a core memory entry. Returns the new token count.
    pub fn set_core_memory(&self, agent_id: &str, key: &str, value: &str) -> Result<i32> {
        let token_count = (value.len() / 4).max(1) as i32;
        self.conn.execute(
            "INSERT INTO core_memory (agent_id, key, value, token_count)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, key) DO UPDATE SET
                value = excluded.value,
                token_count = excluded.token_count,
                updated_at = unixepoch()",
            params![agent_id, key, value, token_count],
        )?;
        Ok(token_count)
    }

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

    pub fn upsert_person(
        &self,
        agent_id: &str,
        name: &str,
        relationship: Option<&str>,
        notes: Option<&str>,
    ) -> Result<i64> {
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM people WHERE agent_id = ?1 AND canonical_name = ?2",
                params![agent_id, name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE people SET relationship = COALESCE(?1, relationship),
                  notes = COALESCE(?2, notes),
                  last_mentioned = unixepoch(),
                  mention_count = mention_count + 1
                  WHERE id = ?3",
                params![relationship, notes, id],
            )?;
            Ok(id)
        } else {
            self.conn.execute(
                "INSERT INTO people (agent_id, canonical_name, relationship, notes)
                  VALUES (?1, ?2, ?3, ?4)",
                params![agent_id, name, relationship, notes],
            )?;
            Ok(self.conn.last_insert_rowid())
        }
    }

    pub fn get_person(&self, agent_id: &str, name: &str) -> Result<Option<Person>> {
        self.conn
            .query_row(
                "SELECT id, canonical_name, relationship, notes,
                         datetime(first_mentioned, 'unixepoch'),
                         datetime(last_mentioned, 'unixepoch'),
                         mention_count
                  FROM people WHERE agent_id = ?1 AND canonical_name = ?2",
                params![agent_id, name],
                |r| {
                    Ok(Person {
                        id: r.get(0)?,
                        canonical_name: r.get(1)?,
                        relationship: r.get(2)?,
                        notes: r.get(3)?,
                        first_mentioned: r.get(4)?,
                        last_mentioned: r.get(5)?,
                        mention_count: r.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_people(&self, agent_id: &str) -> Result<Vec<Person>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name, relationship, notes,
                     datetime(first_mentioned, 'unixepoch'),
                     datetime(last_mentioned, 'unixepoch'),
                     mention_count
              FROM people WHERE agent_id = ?1 ORDER BY canonical_name",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
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

    pub fn search_people(&self, agent_id: &str, query: &str) -> Result<Vec<Person>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name, relationship, notes,
                     datetime(first_mentioned, 'unixepoch'),
                     datetime(last_mentioned, 'unixepoch'),
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
                     datetime(created_at, 'unixepoch'),
                     datetime(completed_at, 'unixepoch')
              FROM commitments WHERE agent_id = ?1 AND status = ?2
              ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, status], Self::row_to_commitment)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn update_commitment_status(&self, agent_id: &str, id: i64, status: &str) -> Result<bool> {
        let completed_at = if status == "completed" {
            Some(Utc::now().timestamp())
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
                     datetime(created_at, 'unixepoch'),
                     datetime(completed_at, 'unixepoch')
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
                 value = excluded.value, updated_at = unixepoch()",
            params![agent_id, category, value],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_preference(&self, agent_id: &str, category: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM preferences WHERE agent_id = ?1 AND category = ?2",
                params![agent_id, category],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_preferences(&self, agent_id: &str) -> Result<Vec<Preference>> {
        let mut stmt = self.conn.prepare(
            "SELECT category, value, datetime(updated_at, 'unixepoch')
              FROM preferences WHERE agent_id = ?1 ORDER BY category",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
                Ok(Preference {
                    category: r.get(0)?,
                    value: r.get(1)?,
                    updated_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn search_preferences(&self, agent_id: &str, query: &str) -> Result<Vec<Preference>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT category, value, datetime(updated_at, 'unixepoch')
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

    pub fn list_events(&self, agent_id: &str) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, event_date, context, datetime(created_at, 'unixepoch')
              FROM events WHERE agent_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |r| {
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

    pub fn search_events(&self, agent_id: &str, query: &str) -> Result<Vec<Event>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT id, description, event_date, context, datetime(created_at, 'unixepoch')
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

    // ===== Audit Events =====

    #[allow(clippy::too_many_arguments)]
    pub fn log_audit_event(
        &self,
        agent_id: &str,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: &str,
        reasoning: Option<&str>,
        trace_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_events
             (agent_id, session_id, tool_name, target_key, before_value, after_value, reasoning, trace_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                agent_id,
                session_id,
                tool_name,
                target_key,
                before_value,
                after_value,
                reasoning,
                trace_id
            ],
        )?;
        Ok(())
    }

    pub fn get_audit_events(&self, agent_id: &str, session_id: &str) -> Result<Vec<AuditEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, target_key, before_value, after_value, reasoning,
                     datetime(created_at, 'unixepoch')
              FROM audit_events WHERE agent_id = ?1 AND session_id = ?2
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, session_id], |r| {
                Ok(AuditEvent {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    tool_name: r.get(2)?,
                    target_key: r.get(3)?,
                    before_value: r.get(4)?,
                    after_value: r.get(5)?,
                    reasoning: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_audit_events_since(
        &self,
        agent_id: &str,
        since_unix: i64,
    ) -> Result<Vec<AuditEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, target_key, before_value, after_value, reasoning,
                     datetime(created_at, 'unixepoch')
              FROM audit_events WHERE agent_id = ?1 AND created_at >= ?2
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, since_unix], |r| {
                Ok(AuditEvent {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    tool_name: r.get(2)?,
                    target_key: r.get(3)?,
                    before_value: r.get(4)?,
                    after_value: r.get(5)?,
                    reasoning: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn count_audit_events_for_session(&self, agent_id: &str, session_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE agent_id = ?1 AND session_id = ?2",
            params![agent_id, session_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn compact_old_audit_events(&self, agent_id: &str, days: u32) -> Result<usize> {
        let cutoff = Utc::now().timestamp() - (days as i64 * 86_400);
        let mut stmt = self.conn.prepare(
            "SELECT
                 CAST(strftime('%Y', datetime(created_at, 'unixepoch')) AS INTEGER) AS year,
                 CAST(strftime('%m', datetime(created_at, 'unixepoch')) AS INTEGER) AS month,
                 COUNT(*) AS cnt,
                 GROUP_CONCAT(
                     tool_name || ': ' || target_key || ' = ' || substr(after_value, 1, 100),
                     '; '
                 ) AS summary
             FROM audit_events
             WHERE agent_id = ?1 AND created_at < ?2
             GROUP BY year, month",
        )?;
        let groups: Vec<(i64, i64, i64, String)> = stmt
            .query_map(params![agent_id, cutoff], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if groups.is_empty() {
            return Ok(0);
        }
        let count = groups.len();
        for (year, month, event_count, summary) in groups {
            self.conn.execute(
                "INSERT OR REPLACE INTO audit_event_summaries
                 (agent_id, year, month, summary, event_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![agent_id, year, month, summary, event_count],
            )?;
        }
        self.conn.execute(
            "DELETE FROM audit_events WHERE agent_id = ?1 AND created_at < ?2",
            params![agent_id, cutoff],
        )?;
        Ok(count)
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
             WHERE agent_id = ?1 AND sent_at >= unixepoch('now') - 3600",
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
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| Utc::now().timestamp());
        let n: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends WHERE agent_id = ?1 AND sent_at >= ?2",
            params![agent_id, since_ts],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn prune_old_heartbeat_sends(&self, agent_id: &str, days: u32) -> Result<()> {
        let cutoff = Utc::now().timestamp() - (days as i64 * 86_400);
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
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| Utc::now().timestamp());
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM reflection_runs
             WHERE agent_id = ?1 AND status = 'completed' AND created_at >= ?2",
            params![agent_id, since_ts],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn prune_old_reflection_runs(&self, agent_id: &str, days: u32) -> Result<usize> {
        let cutoff = Utc::now().timestamp() - (days as i64 * 86_400);
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
            "SELECT id, text, request_id, datetime(created_at, 'unixepoch'), retry_count
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
             ON CONFLICT(agent_id, key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
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
        started_at: i64,
    ) -> Result<()> {
        // Auto-register team (team_id = team_name)
        self.conn.execute(
            "INSERT OR IGNORE INTO teams (id, name) VALUES (?1, ?1)",
            params![team_name],
        )?;
        self.conn.execute(
            "INSERT INTO team_runs (id, team_id, goal, max_iterations, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, team_name, goal, max_iterations, started_at],
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
        ended_at: Option<i64>,
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
        })
    }

    pub fn load_team_runs(&self, team_name: &str, limit: usize) -> Result<Vec<TeamRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, t.name, r.goal, r.status, r.failure_reason, r.iteration,
                     r.max_iterations, r.deliverable, r.started_at, r.ended_at
              FROM team_runs r JOIN teams t ON r.team_id = t.id
              WHERE t.name = ?1
              ORDER BY r.started_at DESC LIMIT ?2",
        )?;
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
        self.conn
            .query_row(
                "SELECT r.id, t.name, r.goal, r.status, r.failure_reason, r.iteration,
                         r.max_iterations, r.deliverable, r.started_at, r.ended_at
                  FROM team_runs r JOIN teams t ON r.team_id = t.id
                  WHERE t.name = ?1
                  ORDER BY r.started_at DESC LIMIT 1",
                params![team_name],
                Self::row_to_team_run,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn load_team_run_by_id(&self, run_id: &str) -> Result<Option<TeamRunRow>> {
        self.conn
            .query_row(
                "SELECT r.id, t.name, r.goal, r.status, r.failure_reason, r.iteration,
                         r.max_iterations, r.deliverable, r.started_at, r.ended_at
                  FROM team_runs r JOIN teams t ON r.team_id = t.id
                  WHERE r.id = ?1",
                params![run_id],
                Self::row_to_team_run,
            )
            .optional()
            .map_err(Into::into)
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
                "UPDATE search_content SET content = ?1, updated_at = unixepoch() WHERE id = ?2",
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
            "UPDATE search_content SET embedding_json = ?1, updated_at = unixepoch()
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

    pub fn get_all_facts_for_indexing(&self, agent_id: &str) -> Result<Vec<(String, i64, String)>> {
        let mut results = Vec::new();
        // People
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name || COALESCE(' - ' || relationship, '')
                        || COALESCE(': ' || notes, '')
              FROM people WHERE agent_id = ?1",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, content) in rows {
            results.push(("person".to_string(), id, content));
        }
        // Commitments
        let mut stmt = self.conn.prepare(
            "SELECT id, description || ' [' || status || ']'
              FROM commitments WHERE agent_id = ?1",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, content) in rows {
            results.push(("commitment".to_string(), id, content));
        }
        // Preferences
        let mut stmt = self.conn.prepare(
            "SELECT rowid, category || ': ' || value FROM preferences WHERE agent_id = ?1",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, content) in rows {
            results.push(("preference".to_string(), id, content));
        }
        // Events
        let mut stmt = self.conn.prepare(
            "SELECT id, description || COALESCE(': ' || context, '')
              FROM events WHERE agent_id = ?1",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![agent_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, content) in rows {
            results.push(("event".to_string(), id, content));
        }
        Ok(results)
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
    pub fn list_sessions_paginated(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
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

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT s.id, s.agent_id, s.channel_type, s.started_at, s.ended_at, s.metadata,
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
                    message_count: r.get(6)?,
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

    /// Get a single session by id.
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, agent_id, channel_type, started_at, ended_at, metadata
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
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, s.channel_type, m.metadata, m.created_at
              FROM messages m JOIN sessions s ON m.session_id = s.id
              WHERE m.session_id = ?1
              ORDER BY m.created_at ASC, m.id ASC LIMIT ?2 OFFSET ?3",
        )?;
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

    /// List audit events for an agent with pagination.
    pub fn list_audit_events_paginated(
        &self,
        agent_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, target_key, before_value, after_value, reasoning,
                     datetime(created_at, 'unixepoch')
              FROM audit_events WHERE agent_id = ?1
              ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(params![agent_id, limit as i64, offset as i64], |r| {
                Ok(AuditEvent {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    tool_name: r.get(2)?,
                    target_key: r.get(3)?,
                    before_value: r.get(4)?,
                    after_value: r.get(5)?,
                    reasoning: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count audit events for an agent.
    pub fn count_audit_events(&self, agent_id: &str) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n as u64)
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
    pub fn list_sessions_paginated_with_count(
        &self,
        agent_id: Option<&str>,
        channel_type: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<SessionWithStats>, u64)> {
        let count = self.count_sessions(agent_id, channel_type)?;
        let data = self.list_sessions_paginated(agent_id, channel_type, limit, offset)?;
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
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<AuditEvent>, u64)> {
        let count = self.count_audit_events(agent_id)?;
        let data = self.list_audit_events_paginated(agent_id, limit, offset)?;
        Ok((data, count))
    }
}

// ===== Utility Functions =====

/// Format a unix timestamp as a human-readable UTC string: "YYYY-MM-DD HH:MM:SS".
pub fn format_unix_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
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
                  VALUES (?1, 'mika', 'user', 'old', 1000)",
                params![sid],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO messages (session_id, agent_id, role, content, created_at)
                  VALUES (?1, 'mika', 'user', 'new', 2000)",
                params![sid],
            )
            .unwrap();
        let msgs = db.get_messages_since("mika", 1500).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "new");
    }

    #[test]
    fn test_last_user_message_time() {
        let (db, sid) = db_with_session();
        assert!(db.last_user_message_time("mika").unwrap().is_none());
        db.save_message("mika", &sid, "user", "hello", None)
            .unwrap();
        let ts = db.last_user_message_time("mika").unwrap();
        assert!(ts.is_some());
        assert!(ts.unwrap() > 0);
    }

    #[test]
    fn test_replace_with_summary() {
        let (db, sid) = db_with_session();
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
    fn test_log_and_get_audit_events() {
        let db = db();
        db.log_audit_event(
            "mika",
            "sess1",
            "update_core_memory",
            "user_summary",
            None,
            "New summary",
            Some("reason"),
            None,
        )
        .unwrap();
        let events = db.get_audit_events("mika", "sess1").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "update_core_memory");
    }

    #[test]
    fn test_get_audit_events_since() {
        let db = db();
        db.conn
            .execute(
                "INSERT INTO audit_events
                  (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'key', 'val', 1000)",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO audit_events
                  (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's2', 'tool', 'key', 'val', 2000)",
                [],
            )
            .unwrap();
        let evs = db.get_audit_events_since("mika", 1500).unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].session_id, "s2");
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
            1_700_000_000,
        )
        .unwrap();
        db.update_team_run(
            "run-001",
            "completed",
            None,
            1,
            Some("Done!"),
            Some(1_700_001_000),
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
        db.insert_team_run("run-001", "eng", "Goal", 3, 0).unwrap();
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
            next_fire_at: Some(9_999_999_999),
            timeout_at: None,
            action_type: "send_message".to_string(),
            action_config: r#"{"message":"hello"}"#.to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
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
        };
        let id = db.create_task(&task).unwrap();
        assert!(db.cancel_task(&id, "mika").unwrap());
        let t = db.get_task(&id, "mika").unwrap().unwrap();
        assert_eq!(t.status, "cancelled");
        // Cancelling again returns false
        assert!(!db.cancel_task(&id, "mika").unwrap());
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
    fn test_compact_old_audit_events() {
        let db = db();
        // Insert old events
        db.conn
            .execute_batch(
                "INSERT INTO audit_events (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'k1', 'v1', 1580000000);
                 INSERT INTO audit_events (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'k2', 'v2', 1580000001);",
            )
            .unwrap();
        // Insert recent event
        db.log_audit_event("mika", "s2", "tool", "k3", None, "v3", None, None)
            .unwrap();
        let compacted = db.compact_old_audit_events("mika", 30).unwrap();
        assert!(compacted > 0);
        // Old events gone, recent one stays
        let recent = db.get_audit_events("mika", "s2").unwrap();
        assert_eq!(recent.len(), 1);
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
                 VALUES ('run123', 'team1', 'test goal', unixepoch())",
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
                 VALUES ('other-run', 'team1', 'other goal', unixepoch())",
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
            next_fire_at: Some(9_999_999_999),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
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
            next_fire_at: Some(9_999_999_999),
            timeout_at: None,
            action_type: "inject_context".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
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
        }
    }

    #[test]
    fn test_get_undelivered_callback_tasks_returns_completed_only() {
        let db = db();
        let id = db.create_task(&callback_task("mika")).unwrap();

        // Pending task should not appear
        let results = db.get_undelivered_callback_tasks("mika", 0).unwrap();
        assert!(results.is_empty());

        // Complete it
        assert!(db.update_task_completed(&id, "mika", Some("done")).unwrap());

        // Now it should appear
        let results = db.get_undelivered_callback_tasks("mika", 0).unwrap();
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

        // since_unix = completed_at means "after this time", so the task at exactly
        // that time should still be included (query uses >)
        // But since completed_at == since_unix, and query is >, it should NOT appear
        let results = db
            .get_undelivered_callback_tasks("mika", completed_at)
            .unwrap();
        assert!(results.is_empty());

        // since_unix before completed_at should include it
        let results = db
            .get_undelivered_callback_tasks("mika", completed_at - 1)
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_get_undelivered_callback_tasks_excludes_other_agents() {
        let db = db();
        db.register_agent("agent_a", "Agent A", "/tmp/a").unwrap();
        db.register_agent("agent_b", "Agent B", "/tmp/b").unwrap();
        let id = db.create_task(&callback_task("agent_a")).unwrap();
        assert!(db.update_task_completed(&id, "agent_a", Some("x")).unwrap());

        let results = db.get_undelivered_callback_tasks("agent_b", 0).unwrap();
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
            "new",
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
    fn test_schema_version_is_7() {
        let db = db();
        assert_eq!(db.schema_version().unwrap(), 7);
    }
}
