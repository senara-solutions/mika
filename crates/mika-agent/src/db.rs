use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;
use tracing::{debug, info};

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

pub const CURRENT_SCHEMA_VERSION: i64 = 1;

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
}

#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub metadata: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct MemoryEvent {
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
pub struct TeamMessageRow {
    pub id: i64,
    pub run_id: String,
    pub parent_id: Option<i64>,
    pub agent_name: Option<String>,
    pub message_type: String,
    pub content: String,
    pub iteration: u32,
    pub created_at: i64,
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
            info!(version = 1, "database migrated to v1");
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
            "DROP TABLE IF EXISTS memory_event_summaries",
            "DROP TABLE IF EXISTS memory_events",
            "DROP TABLE IF EXISTS search_content",
            "DROP TABLE IF EXISTS events",
            "DROP TABLE IF EXISTS preferences",
            "DROP TABLE IF EXISTS commitments",
            "DROP TABLE IF EXISTS people",
            "DROP TABLE IF EXISTS core_memory",
            "DROP TABLE IF EXISTS team_messages",
            "DROP TABLE IF EXISTS team_runs",
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
            INSERT INTO schema_version (version) VALUES (1);

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
                               'cancelled','expired','recurring_active')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                created_by_session TEXT,
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

            CREATE TABLE conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary')),
                content TEXT NOT NULL,
                channel_type TEXT NOT NULL DEFAULT 'telegram',
                metadata TEXT,
                compacted_through_id INTEGER,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_conv_agent_created ON conversations(agent_id, created_at DESC);

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
                completed_at INTEGER,
                UNIQUE (agent_id, description)
            );
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);

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

            CREATE TABLE memory_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT NOT NULL,
                reasoning TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_memev_agent_created ON memory_events(agent_id, created_at DESC);
            CREATE INDEX idx_memev_session ON memory_events(session_id);

            CREATE TABLE memory_event_summaries (
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

            CREATE TABLE team_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_messages(id),
                agent_name TEXT,
                message_type TEXT NOT NULL,
                content TEXT NOT NULL,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );
            CREATE INDEX idx_team_msg_run ON team_messages(run_id, created_at);

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

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label)
                WHERE trigger_type = 'recurring';

            -- Pre-register the default 'mika' agent
            INSERT INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');

            COMMIT;
            ",
            )
            .context("failed to create v1 schema")?;

        // Virtual tables must be outside transactions
        let _ = self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
                 USING fts5(content, content='search_content', content_rowid='id');
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_search
                 USING vec0(embedding float[512]);",
        );

        Ok(())
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
                input_context, created_by_session
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17
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
              action_config, status, input_context, created_by_session)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'recurring_active',?16,?17)",
            params![
                id, task.agent_id, task.team_run_id, task.parent_task_id,
                task.depth, task.label, task.trigger_type, task.cron_expr,
                task.event_source, task.event_offset_secs, task.condition_expr,
                task.next_fire_at, task.timeout_at, task.action_type,
                task.action_config, task.input_context, task.created_by_session
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
             WHERE id = ?1 AND agent_id = ?2 AND status NOT IN ('completed','failed','cancelled','expired')",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    pub fn mark_tasks_expired(&self, now_unix: i64, agent_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'expired', updated_at = unixepoch()
             WHERE agent_id = ?2
               AND timeout_at IS NOT NULL AND timeout_at < ?1
               AND status NOT IN ('completed','failed','cancelled','expired')",
            params![now_unix, agent_id],
        )?;
        Ok(n)
    }

    /// Get IDs of expired tasks whose parent is still pending (for sibling completion checks).
    pub fn get_expired_child_task_ids(&self, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tasks t
             JOIN tasks p ON t.parent_task_id = p.id AND p.agent_id = t.agent_id
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

    pub fn get_pending_reminder_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type IN ('time', 'recurring')
               AND action_type = 'send_message'
               AND status IN ('pending', 'recurring_active')
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
    pub fn try_complete_parent_on_sibling_done(
        &self,
        task_id: &str,
        agent_id: &str,
    ) -> Result<Option<String>> {
        let tx = self.conn.unchecked_transaction()?;

        // 1. Get parent_task_id for this task
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_task_id FROM tasks WHERE id = ?1 AND agent_id = ?2",
                params![task_id, agent_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let parent_id = match parent_id {
            Some(id) => id,
            None => return Ok(None),
        };

        // 2. Count incomplete siblings (same parent, not in terminal state)
        let incomplete: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1 AND agent_id = ?2
             AND status NOT IN ('completed','failed','cancelled','expired')",
            params![&parent_id, agent_id],
            |row| row.get(0),
        )?;

        if incomplete > 0 {
            tx.commit()?;
            return Ok(None);
        }

        // 3. Atomically claim parent task (only if still pending)
        let changed = tx.execute(
            "UPDATE tasks SET status = 'in_progress', fired_at = unixepoch(), updated_at = unixepoch()
             WHERE id = ?1 AND agent_id = ?2 AND status = 'pending'",
            params![&parent_id, agent_id],
        )?;

        tx.commit()?;

        if changed > 0 {
            Ok(Some(parent_id))
        } else {
            Ok(None) // already claimed by another thread
        }
    }

    /// Get all child tasks for a given parent task.
    pub fn get_child_tasks(&self, parent_task_id: &str, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE parent_task_id = ?1 AND agent_id = ?2
             ORDER BY created_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![parent_task_id, agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count pending callback tasks for a given team run with depth > 1.
    /// Used to detect grandchild long-running tasks spawned during a team run.
    pub fn count_pending_callback_tasks_by_team_run(
        &self,
        team_run_id: &str,
        agent_id: &str,
    ) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND team_run_id = ?2
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND depth > 1",
            params![agent_id, team_run_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn prune_completed_tasks(&self, older_than_secs: i64) -> Result<usize> {
        let cutoff = Utc::now().timestamp() - older_than_secs;
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE status IN ('completed','cancelled','expired','failed')
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

    // ===== Conversation =====

    pub fn save_message(
        &self,
        agent_id: &str,
        role: &str,
        content: &str,
        channel_type: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO conversations (agent_id, role, content, channel_type)
             VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, role, content, channel_type],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn save_message_with_metadata(
        &self,
        agent_id: &str,
        role: &str,
        content: &str,
        channel_type: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO conversations (agent_id, role, content, channel_type, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![agent_id, role, content, channel_type, metadata],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn row_to_conversation_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationMessage> {
        Ok(ConversationMessage {
            id: r.get(0)?,
            role: r.get(1)?,
            content: r.get(2)?,
            channel_type: r.get(3)?,
            metadata: r.get(4)?,
            created_at: r.get::<_, i64>(5)?,
        })
    }

    pub fn load_recent_messages(
        &self,
        agent_id: &str,
        limit: usize,
        channel_types: Option<&[&str]>,
    ) -> Result<Vec<ConversationMessage>> {
        let mut messages = if let Some(types) = channel_types {
            let placeholders: String = (1..=types.len())
                .map(|i| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT id, role, content, channel_type, metadata, created_at
                  FROM conversations
                  WHERE agent_id = ?1 AND channel_type IN ({})
                  ORDER BY created_at DESC, id DESC LIMIT ?2",
                placeholders
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(agent_id.to_string()), Box::new(limit as i64)];
            for t in types {
                bind.push(Box::new(t.to_string()));
            }
            let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
            stmt.query_map(refs.as_slice(), Self::row_to_conversation_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, role, content, channel_type, metadata, created_at
                  FROM conversations WHERE agent_id = ?1 AND role != 'summary'
                  ORDER BY created_at DESC, id DESC LIMIT ?2",
            )?;
            stmt.query_map(
                params![agent_id, limit as i64],
                Self::row_to_conversation_message,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        messages.reverse();
        Ok(messages)
    }

    pub fn load_conversation_summary(&self, agent_id: &str) -> Result<Option<ConversationMessage>> {
        self.conn
            .query_row(
                "SELECT id, role, content, channel_type, metadata, created_at
                  FROM conversations WHERE agent_id = ?1 AND role = 'summary'
                  ORDER BY created_at DESC LIMIT 1",
                params![agent_id],
                Self::row_to_conversation_message,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn count_messages(&self, agent_id: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM conversations WHERE agent_id = ?1 AND role != 'summary'",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn load_messages_before_window(
        &self,
        agent_id: &str,
        window_size: usize,
    ) -> Result<Vec<ConversationMessage>> {
        let cutoff_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM conversations WHERE agent_id = ?1 AND role != 'summary'
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
            "SELECT id, role, content, channel_type, metadata, created_at
              FROM conversations
              WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2
              ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(
                params![agent_id, cutoff_id],
                Self::row_to_conversation_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn replace_with_summary(
        &self,
        agent_id: &str,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        self.conn.execute_batch("BEGIN")?;
        // Delete old non-summary messages up to compacted_through_id
        self.conn.execute(
            "DELETE FROM conversations
             WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2",
            params![agent_id, compacted_through_id],
        )?;
        // Remove old summary
        self.conn.execute(
            "DELETE FROM conversations WHERE agent_id = ?1 AND role = 'summary'",
            params![agent_id],
        )?;
        // Insert new summary
        self.conn.execute(
            "INSERT INTO conversations (agent_id, role, content, channel_type, compacted_through_id)
             VALUES (?1, 'summary', ?2, 'cli', ?3)",
            params![agent_id, summary, compacted_through_id],
        )?;
        let row_id = self.conn.last_insert_rowid();
        self.conn.execute_batch("COMMIT")?;
        Ok(row_id)
    }

    pub fn load_messages_after(
        &self,
        agent_id: &str,
        after_id: i64,
        channel_types: Option<&[&str]>,
    ) -> Result<Vec<ConversationMessage>> {
        if let Some(types) = channel_types {
            let placeholders: String = (0..types.len())
                .map(|i| format!("?{}", i + 3))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT id, role, content, channel_type, metadata, created_at
                  FROM conversations
                  WHERE agent_id = ?1 AND id > ?2 AND channel_type IN ({})
                  ORDER BY created_at ASC",
                placeholders
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(agent_id.to_string()), Box::new(after_id)];
            for t in types {
                bind.push(Box::new(t.to_string()));
            }
            let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
            let rows = stmt
                .query_map(refs.as_slice(), Self::row_to_conversation_message)?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, role, content, channel_type, metadata, created_at
                  FROM conversations WHERE agent_id = ?1 AND id > ?2
                  ORDER BY created_at ASC",
            )?;
            let rows = stmt
                .query_map(
                    params![agent_id, after_id],
                    Self::row_to_conversation_message,
                )?
                .collect::<rusqlite::Result<_>>()?;
            Ok(rows)
        }
    }

    pub fn max_message_id(&self, agent_id: &str) -> Result<i64> {
        let id: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM conversations WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn get_conversations_since(
        &self,
        agent_id: &str,
        since_unix: i64,
    ) -> Result<Vec<ConversationMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, channel_type, metadata, created_at
              FROM conversations
              WHERE agent_id = ?1 AND created_at >= ?2 AND role != 'summary'
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(
                params![agent_id, since_unix],
                Self::row_to_conversation_message,
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn last_user_message_time(&self, agent_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT MAX(created_at) FROM conversations
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
            self.set_core_memory(
                agent_id,
                "self_model",
                &format!("I am {display_name}. No interaction history yet."),
            )?;
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
                  last_mentioned = unixepoch()
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
                         datetime(last_mentioned, 'unixepoch')
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
                     datetime(last_mentioned, 'unixepoch')
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
                     datetime(last_mentioned, 'unixepoch')
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
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, description) DO UPDATE SET
                 due_date = COALESCE(?3, due_date),
                 person_id = COALESCE(?4, person_id)",
            params![agent_id, description, due_date, person_id],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM commitments WHERE agent_id = ?1 AND description = ?2",
            params![agent_id, description],
            |r| r.get(0),
        )?;
        Ok(id)
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

    // ===== Memory Events (Audit Log) =====

    #[allow(clippy::too_many_arguments)]
    pub fn log_memory_event(
        &self,
        agent_id: &str,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: &str,
        reasoning: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO memory_events
             (agent_id, session_id, tool_name, target_key, before_value, after_value, reasoning)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                agent_id,
                session_id,
                tool_name,
                target_key,
                before_value,
                after_value,
                reasoning
            ],
        )?;
        Ok(())
    }

    pub fn get_memory_events(&self, agent_id: &str, session_id: &str) -> Result<Vec<MemoryEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, target_key, before_value, after_value, reasoning,
                     datetime(created_at, 'unixepoch')
              FROM memory_events WHERE agent_id = ?1 AND session_id = ?2
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, session_id], |r| {
                Ok(MemoryEvent {
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

    pub fn get_memory_events_since(
        &self,
        agent_id: &str,
        since_unix: i64,
    ) -> Result<Vec<MemoryEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, target_key, before_value, after_value, reasoning,
                     datetime(created_at, 'unixepoch')
              FROM memory_events WHERE agent_id = ?1 AND created_at >= ?2
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, since_unix], |r| {
                Ok(MemoryEvent {
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

    pub fn count_memory_events_for_session(&self, agent_id: &str, session_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memory_events WHERE agent_id = ?1 AND session_id = ?2",
            params![agent_id, session_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    pub fn compact_old_memory_events(&self, agent_id: &str, days: u32) -> Result<usize> {
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
             FROM memory_events
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
                "INSERT OR REPLACE INTO memory_event_summaries
                 (agent_id, year, month, summary, event_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![agent_id, year, month, summary, event_count],
            )?;
        }
        self.conn.execute(
            "DELETE FROM memory_events WHERE agent_id = ?1 AND created_at < ?2",
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

    // ===== Team Messages =====

    pub fn insert_team_message(
        &self,
        run_id: &str,
        parent_id: Option<i64>,
        agent_name: Option<&str>,
        message_type: &str,
        content: &str,
        iteration: u32,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO team_messages (run_id, parent_id, agent_name, message_type, content, iteration)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![run_id, parent_id, agent_name, message_type, content, iteration],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn load_assignment_msg_ids(
        &self,
        run_id: &str,
        iteration: u32,
    ) -> Result<HashMap<String, i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_name, id FROM team_messages
             WHERE run_id = ?1 AND iteration = ?2 AND message_type = 'assignment'
             AND agent_name IS NOT NULL",
        )?;
        let rows: Vec<(String, i64)> = stmt
            .query_map(params![run_id, iteration], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows.into_iter().collect())
    }

    pub fn load_team_messages(&self, run_id: &str) -> Result<Vec<TeamMessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, run_id, parent_id, agent_name,
                     message_type, content, iteration, created_at
              FROM team_messages WHERE run_id = ?1
              ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![run_id], |r| {
                Ok(TeamMessageRow {
                    id: r.get(0)?,
                    run_id: r.get(1)?,
                    parent_id: r.get(2)?,
                    agent_name: r.get(3)?,
                    message_type: r.get(4)?,
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
    fn test_v1_tables_exist() {
        let db = db();
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'
                  AND name IN ('agents','teams','tasks','conversations','core_memory',
                               'people','commitments','preferences','events',
                               'memory_events','memory_event_summaries','search_content',
                               'team_runs','team_messages','heartbeat_sends',
                               'reflection_runs','customer_config','failed_sends')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 18);
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
        let db = db();
        db.save_message("mika", "user", "Hello!", "cli").unwrap();
        db.save_message("mika", "assistant", "Hi!", "cli").unwrap();
        let msgs = db.load_recent_messages("mika", 10, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn test_load_recent_messages_limit() {
        let db = db();
        for i in 0..5 {
            db.save_message("mika", "user", &format!("msg {i}"), "cli")
                .unwrap();
        }
        let msgs = db.load_recent_messages("mika", 3, None).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].content, "msg 4");
    }

    #[test]
    fn test_load_recent_messages_channel_filter() {
        let db = db();
        db.save_message("mika", "user", "telegram msg", "telegram")
            .unwrap();
        db.save_message("mika", "user", "cli msg", "cli").unwrap();
        let tg = db
            .load_recent_messages("mika", 10, Some(&["telegram"]))
            .unwrap();
        assert_eq!(tg.len(), 1);
        assert_eq!(tg[0].content, "telegram msg");
    }

    #[test]
    fn test_load_messages_after_with_channel_filter() {
        let db = db();
        // Insert messages across different channels
        db.save_message("mika", "user", "telegram msg 1", "telegram")
            .unwrap();
        db.save_message("mika", "user", "cli msg 1", "cli").unwrap();
        db.save_message("mika", "user", "api msg 1", "api").unwrap();
        db.save_message("mika", "user", "telegram msg 2", "telegram")
            .unwrap();
        db.save_message("mika", "user", "cli msg 2", "cli").unwrap();

        // Filter to telegram + cli only (should exclude api)
        let msgs = db
            .load_messages_after("mika", 0, Some(&["telegram", "cli"]))
            .unwrap();
        assert_eq!(msgs.len(), 4);
        for msg in &msgs {
            assert!(
                msg.channel_type == "telegram" || msg.channel_type == "cli",
                "unexpected channel_type: {}",
                msg.channel_type
            );
        }

        // No filter returns all messages
        let all = db.load_messages_after("mika", 0, None).unwrap();
        assert_eq!(all.len(), 5);

        // after_id filtering works with channel filter
        let first_id = msgs[0].id;
        let after = db
            .load_messages_after("mika", first_id, Some(&["cli"]))
            .unwrap();
        // Should get only cli messages after first_id
        for msg in &after {
            assert_eq!(msg.channel_type, "cli");
            assert!(msg.id > first_id);
        }
    }

    #[test]
    fn test_get_conversations_since() {
        let db = db();
        // Insert directly with a known timestamp
        db.conn
            .execute(
                "INSERT INTO conversations (agent_id, role, content, channel_type, created_at)
                  VALUES ('mika', 'user', 'old', 'cli', 1000)",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO conversations (agent_id, role, content, channel_type, created_at)
                  VALUES ('mika', 'user', 'new', 'cli', 2000)",
                [],
            )
            .unwrap();
        let msgs = db.get_conversations_since("mika", 1500).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "new");
    }

    #[test]
    fn test_last_user_message_time() {
        let db = db();
        assert!(db.last_user_message_time("mika").unwrap().is_none());
        db.save_message("mika", "user", "hello", "cli").unwrap();
        let ts = db.last_user_message_time("mika").unwrap();
        assert!(ts.is_some());
        assert!(ts.unwrap() > 0);
    }

    #[test]
    fn test_replace_with_summary() {
        let db = db();
        let id1 = db.save_message("mika", "user", "msg1", "cli").unwrap();
        db.save_message("mika", "assistant", "reply1", "cli")
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
        let db = db();
        db.save_message("mika", "user", "a", "cli").unwrap();
        db.save_message("mika", "assistant", "b", "cli").unwrap();
        db.conn
            .execute(
                "INSERT INTO conversations (agent_id, role, content, channel_type)
                  VALUES ('mika', 'summary', 'S', 'cli')",
                [],
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
    fn test_log_and_get_memory_events() {
        let db = db();
        db.log_memory_event(
            "mika",
            "sess1",
            "update_core_memory",
            "user_summary",
            None,
            "New summary",
            Some("reason"),
        )
        .unwrap();
        let events = db.get_memory_events("mika", "sess1").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, "update_core_memory");
    }

    #[test]
    fn test_get_memory_events_since() {
        let db = db();
        db.conn
            .execute(
                "INSERT INTO memory_events
                  (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'key', 'val', 1000)",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO memory_events
                  (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's2', 'tool', 'key', 'val', 2000)",
                [],
            )
            .unwrap();
        let evs = db.get_memory_events_since("mika", 1500).unwrap();
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
    fn test_team_messages_insert_and_load() {
        let db = db();
        db.insert_team_run("run-001", "eng", "Goal", 3, 0).unwrap();
        let id = db
            .insert_team_message("run-001", None, Some("mika"), "plan", "Do this", 1)
            .unwrap();
        let msgs = db.load_team_messages("run-001").unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, id);
        assert_eq!(msgs[0].message_type, "plan");
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
    fn test_compact_old_memory_events() {
        let db = db();
        // Insert old events
        db.conn
            .execute_batch(
                "INSERT INTO memory_events (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'k1', 'v1', 1580000000);
                 INSERT INTO memory_events (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'k2', 'v2', 1580000001);",
            )
            .unwrap();
        // Insert recent event
        db.log_memory_event("mika", "s2", "tool", "k3", None, "v3", None)
            .unwrap();
        let compacted = db.compact_old_memory_events("mika", 30).unwrap();
        assert!(compacted > 0);
        // Old events gone, recent one stays
        let recent = db.get_memory_events("mika", "s2").unwrap();
        assert_eq!(recent.len(), 1);
    }

    #[test]
    fn test_load_messages_before_window() {
        let db = db();
        for i in 0..5 {
            db.save_message("mika", "user", &format!("msg {i}"), "cli")
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
            db.try_complete_parent_on_sibling_done(&c2_id, "mika")
                .unwrap(),
            None
        );

        // Complete the 3rd — parent should fire
        db.update_task_completed(&c3_id, "mika", Some("done"))
            .unwrap();
        let result = db
            .try_complete_parent_on_sibling_done(&c3_id, "mika")
            .unwrap();
        assert_eq!(result, Some(parent_id));
    }

    #[test]
    fn test_sibling_completion_no_parent_returns_none() {
        let db = db();
        let task_id = db.create_task(&make_task("orphan")).unwrap();
        assert_eq!(
            db.try_complete_parent_on_sibling_done(&task_id, "mika")
                .unwrap(),
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
        let result = db
            .try_complete_parent_on_sibling_done(&c2_id, "mika")
            .unwrap();
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

        let children = db.get_child_tasks(&parent_id, "mika").unwrap();
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
            .count_pending_callback_tasks_by_team_run("run123", "mika")
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
}
