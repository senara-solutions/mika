use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Once;
use tracing::{debug, info};

/// Register sqlite-vec as an auto-extension so every new connection gets vec0.
/// Idempotent — uses an internal Once guard so multiple calls are safe.
pub fn init_sqlite_vec() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: sqlite3_auto_extension requires a function pointer matching
        // the sqlite3 extension init signature. sqlite_vec::sqlite3_vec_init
        // is provided by the sqlite-vec crate and matches this signature.
        // This must run before any DB connections are opened.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(
                #[allow(clippy::missing_transmute_annotations)]
                std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()),
            ));
        }
    });
}

const CURRENT_SCHEMA_VERSION: i64 = 8;

/// Canonical list of valid commitment statuses at the database level.
/// "pending" is the default for new commitments; "completed" and "cancelled" are terminal states.
pub const COMMITMENT_STATUSES: &[&str] = &["pending", "completed", "cancelled"];

/// Canonical list of core memory section names and their default values.
/// Used by seed_core_memory, CLI reset, update_core_memory validation, and prompt assembly.
pub const CORE_MEMORY_SECTIONS: &[(&str, &str)] = &[
    ("user_summary", "New user. No information yet."),
    ("persona", "Mika -- personal AI executive assistant."),
    (
        "current_priorities",
        "Get to know the user and understand their needs.",
    ),
    ("key_people", "No one tracked yet."),
];

/// Returns just the section names from CORE_MEMORY_SECTIONS.
pub fn core_memory_section_names() -> Vec<&'static str> {
    CORE_MEMORY_SECTIONS.iter().map(|(k, _)| *k).collect()
}

/// Per-customer SQLite database.
/// Data-at-rest encryption is provided by Kubernetes encrypted volumes.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Open (or create) a per-customer SQLite database at `path`.
    /// Runs migrations automatically.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open SQLite at {}", path.display()))?;

        // Performance pragmas
        // auto_vacuum = INCREMENTAL must be set before first table creation for new DBs.
        // For existing DBs it requires a full VACUUM to take effect (handled by migration).
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA auto_vacuum = INCREMENTAL;",
        )
        .context("failed to set SQLite pragmas")?;

        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory SQLite")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to set pragmas")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    // -- Schema migrations --

    fn current_version(&self) -> Result<i64> {
        // schema_version table may not exist yet
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

        if version >= CURRENT_SCHEMA_VERSION {
            return Ok(());
        }

        if version < 4 {
            self.migrate_v4()?;
        }
        if version < 5 {
            self.migrate_v5()?;
        }
        if version < 6 {
            self.migrate_v6()?;
        }
        if version < 7 {
            self.migrate_v7()?;
        }
        if version < 8 {
            self.migrate_v8()?;
        }

        info!(version = CURRENT_SCHEMA_VERSION, "database migrated");
        Ok(())
    }

    fn migrate_v4(&self) -> Result<()> {
        info!("applying migration v4: plaintext schema (encryption removed)");

        self.conn
            .execute_batch(
                "
            BEGIN;

            -- Drop all existing tables if they exist (pre-production, no data to preserve)
            DROP TABLE IF EXISTS memory_events;
            DROP TABLE IF EXISTS events;
            DROP TABLE IF EXISTS preferences;
            DROP TABLE IF EXISTS commitments;
            DROP TABLE IF EXISTS people;
            DROP TABLE IF EXISTS core_memory;
            DROP TABLE IF EXISTS conversations;
            DROP TABLE IF EXISTS schema_version;

            -- Schema version tracking
            CREATE TABLE schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Conversations (message history)
            CREATE TABLE conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                channel_type TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Core memory (Layer 1)
            CREATE TABLE core_memory (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- People (Layer 2)
            CREATE TABLE people (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                canonical_name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (datetime('now')),
                last_mentioned TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Commitments (Layer 2)
            CREATE TABLE commitments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                description TEXT NOT NULL UNIQUE COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending',
                due_date TEXT,
                person_id INTEGER REFERENCES people(id),
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
            );
            CREATE INDEX idx_commit_status ON commitments(status);
            CREATE INDEX idx_commit_due ON commitments(due_date);

            -- Preferences (Layer 2)
            CREATE TABLE preferences (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                category TEXT NOT NULL UNIQUE COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Events (Layer 2)
            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_events_date ON events(event_date);

            -- Memory events (audit log)
            CREATE TABLE memory_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_memory_events_session ON memory_events(session_id);

            -- Record version
            INSERT INTO schema_version (version) VALUES (4);

            COMMIT;
            ",
            )
            .context("failed to apply migration v4")?;

        Ok(())
    }

    fn migrate_v5(&self) -> Result<()> {
        info!("applying migration v5: compaction, reminders, heartbeat, customer_config");

        self.conn
            .execute_batch(
                "
            BEGIN;

            -- Compaction support: track which messages a summary covers
            ALTER TABLE conversations ADD COLUMN compacted_through_id INTEGER;

            -- Reminders (persisted Tokio timer state)
            CREATE TABLE IF NOT EXISTS reminders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                fire_at TEXT NOT NULL,
                message TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                delivered_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_reminders_status_fire_at ON reminders(status, fire_at);

            -- Heartbeat send rate limiting
            CREATE TABLE IF NOT EXISTS heartbeat_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sent_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_heartbeat_sends_sent_at ON heartbeat_sends(sent_at);

            -- Indexes on conversations for hot-path queries
            CREATE INDEX IF NOT EXISTS idx_conversations_role ON conversations(role, id);
            CREATE INDEX IF NOT EXISTS idx_conversations_channel_type ON conversations(channel_type, id);

            -- Customer config (timezone, chat_id for outbound)
            CREATE TABLE IF NOT EXISTS customer_config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- Failed outbound sends (retry queue for /send failures)
            CREATE TABLE IF NOT EXISTS failed_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                request_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                retry_count INTEGER NOT NULL DEFAULT 0
            );

            INSERT INTO schema_version (version) VALUES (5);

            COMMIT;
            ",
            )
            .context("failed to apply migration v5")?;

        Ok(())
    }

    fn migrate_v6(&self) -> Result<()> {
        info!("applying migration v6: memory_event_summaries for tiered retention");

        self.conn
            .execute_batch(
                "
            BEGIN;

            CREATE TABLE IF NOT EXISTS memory_event_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                tool_counts TEXT NOT NULL,
                category_counts TEXT NOT NULL,
                total_mutations INTEGER NOT NULL,
                top_targets TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(year, month)
            );

            INSERT INTO schema_version (version) VALUES (6);

            COMMIT;
            ",
            )
            .context("failed to apply migration v6")?;

        Ok(())
    }

    fn migrate_v7(&self) -> Result<()> {
        info!("applying migration v7: index on memory_events.created_at");

        self.conn
            .execute_batch(
                "
            BEGIN;

            CREATE INDEX IF NOT EXISTS idx_memory_events_created_at ON memory_events(created_at);

            INSERT INTO schema_version (version) VALUES (7);

            COMMIT;
            ",
            )
            .context("failed to apply migration v7")?;

        Ok(())
    }

    fn migrate_v8(&self) -> Result<()> {
        info!(
            "applying migration v8: Layer 3 search tables (search_content, vec_search, fts_search)"
        );

        // search_content and fts_search in one transaction
        self.conn
            .execute_batch(
                "
            BEGIN;

            -- Unified content table for all searchable text
            CREATE TABLE search_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_search_content_source ON search_content(source_type, source_id);

            -- FTS5 keyword index (BM25 ranking)
            CREATE VIRTUAL TABLE fts_search USING fts5(
                content,
                content_id UNINDEXED,
                source_type UNINDEXED,
                tokenize='porter unicode61'
            );

            INSERT INTO schema_version (version) VALUES (8);

            COMMIT;
            ",
            )
            .context("failed to apply migration v8 (search_content + fts_search)")?;

        // vec0 virtual table must be created outside the transaction
        // (sqlite-vec limitation with virtual tables)
        self.conn
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS vec_search USING vec0(
                    content_id INTEGER PRIMARY KEY,
                    embedding float[512]
                );",
            )
            .context("failed to create vec_search table")?;

        Ok(())
    }

    // -- Conversations --

    /// Save a conversation message.
    pub fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO conversations (role, content, channel_type) VALUES (?1, ?2, ?3)",
                rusqlite::params![role, content, channel_type],
            )
            .context("failed to insert conversation")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Load the N most recent messages, oldest first.
    /// If `channel_types` is Some, only messages with matching channel_type are returned.
    /// If None, all messages are returned (used by compaction).
    /// Summary rows (role = 'summary') are always excluded — use `load_conversation_summary()`.
    pub fn load_recent_messages(
        &self,
        limit: usize,
        channel_types: Option<&[&str]>,
    ) -> Result<Vec<ConversationMessage>> {
        let (sql, params) = match channel_types {
            Some(types) if !types.is_empty() => {
                let placeholders: Vec<String> =
                    (0..types.len()).map(|i| format!("?{}", i + 2)).collect();
                let sql = format!(
                    "SELECT id, role, content, channel_type, created_at
                     FROM conversations
                     WHERE role != 'summary' AND channel_type IN ({})
                     ORDER BY id DESC LIMIT ?1",
                    placeholders.join(", ")
                );
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(limit as i64)];
                for t in types {
                    params.push(Box::new(t.to_string()));
                }
                (sql, params)
            }
            _ => {
                let sql = "SELECT id, role, content, channel_type, created_at
                     FROM conversations
                     WHERE role != 'summary'
                     ORDER BY id DESC LIMIT ?1"
                    .to_string();
                let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(limit as i64)];
                (sql, params)
            }
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut messages: Vec<ConversationMessage> = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(ConversationMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    channel_type: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        // Reverse to oldest-first order
        messages.reverse();
        Ok(messages)
    }

    // -- Core Memory --

    /// Get a core memory value by key.
    pub fn get_core_memory(&self, key: &str) -> Result<Option<CoreMemoryEntry>> {
        let entry = self
            .conn
            .query_row(
                "SELECT key, value, token_count, updated_at FROM core_memory WHERE key = ?1",
                [key],
                |row| {
                    Ok(CoreMemoryEntry {
                        key: row.get(0)?,
                        value: row.get(1)?,
                        token_count: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(entry)
    }

    /// Get all core memory entries.
    pub fn get_all_core_memory(&self) -> Result<Vec<CoreMemoryEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value, token_count, updated_at FROM core_memory ORDER BY key")?;

        let entries: Vec<CoreMemoryEntry> = stmt
            .query_map([], |row| {
                Ok(CoreMemoryEntry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    token_count: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(entries)
    }

    /// Upsert a core memory entry.
    /// Returns token count. Approximate: chars / 4.
    pub fn set_core_memory(&self, key: &str, value: &str) -> Result<i32> {
        let token_count = (value.len() / 4) as i32;

        self.conn
            .execute(
                "INSERT INTO core_memory (key, value, token_count, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    token_count = excluded.token_count,
                    updated_at = excluded.updated_at",
                rusqlite::params![key, value, token_count],
            )
            .context("failed to upsert core memory")?;

        Ok(token_count)
    }

    /// Total token count across all core memory entries.
    pub fn total_core_memory_tokens(&self) -> Result<i32> {
        let total: i32 = self.conn.query_row(
            "SELECT COALESCE(SUM(token_count), 0) FROM core_memory",
            [],
            |row| row.get(0),
        )?;
        Ok(total)
    }

    /// Seed default core memory for a new customer.
    /// If user_md_content is provided, use it for user_summary instead of the default.
    pub fn seed_core_memory(&self, user_md_content: Option<&str>) -> Result<()> {
        for &(key, default_value) in CORE_MEMORY_SECTIONS {
            let value = if key == "user_summary" {
                user_md_content.unwrap_or(default_value)
            } else {
                default_value
            };
            self.set_core_memory(key, value)?;
        }
        Ok(())
    }

    // -- People (Layer 2) --

    /// Insert or update a person. Returns their ID.
    pub fn upsert_person(
        &self,
        name: &str,
        relationship: Option<&str>,
        notes: Option<&str>,
    ) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO people (canonical_name, relationship, notes, last_mentioned)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(canonical_name) DO UPDATE SET
                    relationship = COALESCE(excluded.relationship, people.relationship),
                    notes = COALESCE(excluded.notes, people.notes),
                    last_mentioned = excluded.last_mentioned",
                rusqlite::params![name, relationship, notes],
            )
            .context("failed to upsert person")?;

        let id = self.conn.query_row(
            "SELECT id FROM people WHERE canonical_name = ?1",
            [name],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Get a person by name (case-insensitive).
    pub fn get_person(&self, name: &str) -> Result<Option<Person>> {
        let person = self
            .conn
            .query_row(
                "SELECT id, canonical_name, relationship, notes, first_mentioned, last_mentioned
                 FROM people WHERE canonical_name = ?1",
                [name],
                |row| {
                    Ok(Person {
                        id: row.get(0)?,
                        canonical_name: row.get(1)?,
                        relationship: row.get(2)?,
                        notes: row.get(3)?,
                        first_mentioned: row.get(4)?,
                        last_mentioned: row.get(5)?,
                    })
                },
            )
            .optional()?;
        Ok(person)
    }

    /// List all people.
    pub fn list_people(&self) -> Result<Vec<Person>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name, relationship, notes, first_mentioned, last_mentioned
             FROM people ORDER BY last_mentioned DESC",
        )?;

        let people: Vec<Person> = stmt
            .query_map([], |row| {
                Ok(Person {
                    id: row.get(0)?,
                    canonical_name: row.get(1)?,
                    relationship: row.get(2)?,
                    notes: row.get(3)?,
                    first_mentioned: row.get(4)?,
                    last_mentioned: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(people)
    }

    // -- Commitments (Layer 2) --

    /// Add a commitment. Uses case-insensitive UNIQUE for dedup.
    pub fn add_commitment(
        &self,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO commitments (description, due_date, person_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![description, due_date, person_id],
            )
            .context("failed to insert commitment")?;

        let id = self.conn.query_row(
            "SELECT id FROM commitments WHERE description = ?1",
            [description],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// List commitments by status.
    pub fn list_commitments(&self, status: &str) -> Result<Vec<Commitment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, status, due_date, person_id, created_at, completed_at
             FROM commitments WHERE status = ?1 ORDER BY due_date ASC NULLS LAST",
        )?;

        let commitments: Vec<Commitment> = stmt
            .query_map([status], |row| {
                Ok(Commitment {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    status: row.get(2)?,
                    due_date: row.get(3)?,
                    person_id: row.get(4)?,
                    created_at: row.get(5)?,
                    completed_at: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(commitments)
    }

    /// Update commitment status. Returns `true` if a row was updated, `false` if the id was not found.
    pub fn update_commitment_status(&self, id: i64, status: &str) -> Result<bool> {
        if !COMMITMENT_STATUSES.contains(&status) {
            anyhow::bail!("invalid commitment status: {status}");
        }

        let rows = if status == "completed" {
            self.conn.execute(
                "UPDATE commitments SET status = ?1, completed_at = datetime('now') WHERE id = ?2",
                rusqlite::params![status, id],
            )?
        } else {
            self.conn.execute(
                "UPDATE commitments SET status = ?1, completed_at = NULL WHERE id = ?2",
                rusqlite::params![status, id],
            )?
        };
        Ok(rows > 0)
    }

    /// Get the current status of a commitment by id.
    pub fn get_commitment_status(&self, id: i64) -> Result<Option<String>> {
        let status = self
            .conn
            .query_row(
                "SELECT status FROM commitments WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(status)
    }

    /// Get the description and due_date of a commitment by id.
    pub fn get_commitment_details(&self, id: i64) -> Result<Option<(String, Option<String>)>> {
        let result = self
            .conn
            .query_row(
                "SELECT description, due_date FROM commitments WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(result)
    }

    // -- Preferences (Layer 2) --

    /// Upsert a preference (case-insensitive category). Returns the preference ID.
    pub fn set_preference(&self, category: &str, value: &str) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO preferences (category, value, updated_at)
                 VALUES (?1, ?2, datetime('now'))
                 ON CONFLICT(category) DO UPDATE SET
                    value = excluded.value,
                    updated_at = excluded.updated_at",
                rusqlite::params![category, value],
            )
            .context("failed to upsert preference")?;
        let id = self
            .conn
            .query_row(
                "SELECT id FROM preferences WHERE category = ?1",
                [category],
                |row| row.get(0),
            )
            .context("failed to get preference id")?;
        Ok(id)
    }

    /// Get a preference by category (case-insensitive).
    pub fn get_preference(&self, category: &str) -> Result<Option<String>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE category = ?1",
                [category],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    /// List all preferences (for substring search).
    pub fn list_preferences(&self) -> Result<Vec<Preference>> {
        let mut stmt = self
            .conn
            .prepare("SELECT category, value, updated_at FROM preferences ORDER BY category")?;
        let prefs = stmt
            .query_map([], |row| {
                Ok(Preference {
                    category: row.get(0)?,
                    value: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(prefs)
    }

    // -- Events (Layer 2) --

    /// Add an event.
    pub fn add_event(
        &self,
        description: &str,
        event_date: Option<&str>,
        context: Option<&str>,
    ) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO events (description, event_date, context) VALUES (?1, ?2, ?3)",
                rusqlite::params![description, event_date, context],
            )
            .context("failed to insert event")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List all events.
    pub fn list_events(&self) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, event_date, context, created_at
             FROM events ORDER BY event_date ASC NULLS LAST",
        )?;

        let events = stmt
            .query_map([], |row| {
                Ok(Event {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    event_date: row.get(2)?,
                    context: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(events)
    }

    // -- Reminders --

    /// Add a new reminder. Returns the reminder ID.
    pub fn add_reminder(&self, fire_at: &str, message: &str) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO reminders (fire_at, message) VALUES (?1, ?2)",
                rusqlite::params![fire_at, message],
            )
            .context("failed to insert reminder")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Query reminders using a type-safe filter enum.
    fn query_reminders(&self, filter: ReminderFilter) -> Result<Vec<Reminder>> {
        let sql = match filter {
            ReminderFilter::All => {
                "SELECT id, fire_at, message, status, created_at, delivered_at \
                 FROM reminders WHERE status = 'pending' ORDER BY fire_at ASC"
            }
            ReminderFilter::Future => {
                "SELECT id, fire_at, message, status, created_at, delivered_at \
                 FROM reminders WHERE status = 'pending' AND fire_at > datetime('now') ORDER BY fire_at ASC"
            }
            ReminderFilter::PastDue => {
                "SELECT id, fire_at, message, status, created_at, delivered_at \
                 FROM reminders WHERE status = 'pending' AND fire_at <= datetime('now') ORDER BY fire_at ASC"
            }
        };
        let mut stmt = self.conn.prepare_cached(sql)?;
        let reminders = stmt
            .query_map([], |row| {
                Ok(Reminder {
                    id: row.get(0)?,
                    fire_at: row.get(1)?,
                    message: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    delivered_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(reminders)
    }

    /// Get all pending reminders (any fire_at).
    pub fn get_pending_reminders(&self) -> Result<Vec<Reminder>> {
        self.query_reminders(ReminderFilter::All)
    }

    /// Get pending reminders whose fire_at is in the future.
    pub fn get_future_reminders(&self) -> Result<Vec<Reminder>> {
        self.query_reminders(ReminderFilter::Future)
    }

    /// Get pending reminders whose fire_at is at or past now (ready to deliver).
    pub fn get_past_due_reminders(&self) -> Result<Vec<Reminder>> {
        self.query_reminders(ReminderFilter::PastDue)
    }

    /// Mark a reminder as delivered.
    pub fn mark_reminder_delivered(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET status = 'delivered', delivered_at = datetime('now') WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Mark a reminder as failed.
    pub fn mark_reminder_failed(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE reminders SET status = 'failed' WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Cancel a pending reminder. Returns true if a reminder was actually cancelled.
    pub fn cancel_reminder(&self, id: i64) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE reminders SET status = 'cancelled' WHERE id = ?1 AND status = 'pending'",
            [id],
        )?;
        Ok(rows > 0)
    }

    // -- Heartbeat Pruning --

    /// Delete heartbeat sends older than `days` days.
    pub fn prune_old_heartbeat_sends(&self, days: u32) -> Result<()> {
        let modifier = format!("-{days} days");
        self.conn.execute(
            "DELETE FROM heartbeat_sends WHERE sent_at < datetime('now', ?1)",
            [&modifier],
        )?;
        Ok(())
    }

    /// Record a heartbeat send (for rate limiting).
    pub fn record_heartbeat_send(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO heartbeat_sends (sent_at) VALUES (datetime('now'))",
            [],
        )?;
        Ok(())
    }

    /// Count heartbeat sends in the last hour.
    pub fn count_heartbeat_sends_last_hour(&self) -> Result<u32> {
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends WHERE sent_at >= datetime('now', '-1 hour')",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count heartbeat sends today in the customer's timezone.
    /// Computes the local start-of-day boundary using chrono-tz, then counts
    /// sends with `sent_at >= local_midnight_utc`.
    pub fn count_heartbeat_sends_today(&self, timezone: &str) -> Result<u32> {
        use chrono::{Datelike, NaiveDate, Utc};
        use chrono_tz::Tz;

        let tz: Tz = timezone.parse().unwrap_or(chrono_tz::UTC);

        // Current time in the customer's timezone
        let now_local = Utc::now().with_timezone(&tz);
        // Start of today in customer's local time
        let local_midnight =
            NaiveDate::from_ymd_opt(now_local.year(), now_local.month(), now_local.day())
                .unwrap_or_else(|| Utc::now().date_naive())
                .and_hms_opt(0, 0, 0)
                .expect("midnight is always valid");

        // Convert local midnight back to UTC for SQL comparison
        // Use the earliest possible time if ambiguous (e.g., DST transitions)
        let midnight_utc = local_midnight
            .and_local_timezone(tz)
            .earliest()
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        let boundary = midnight_utc.format("%Y-%m-%d %H:%M:%S").to_string();

        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends WHERE sent_at >= ?1",
            [&boundary],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Most recent user message timestamp (for heartbeat pre-filter).
    pub fn last_user_message_time(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT created_at FROM conversations \
                 WHERE role = 'user' AND channel_type IN ('cli', 'telegram') \
                 ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    // -- Failed Sends --

    /// Save a failed outbound send for later retry.
    pub fn save_failed_send(&self, text: &str, request_id: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO failed_sends (text, request_id) VALUES (?1, ?2)",
            rusqlite::params![text, request_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get oldest failed sends for flushing (max retry_count 3, max age 24h).
    pub fn get_pending_failed_sends(&self, limit: usize) -> Result<Vec<FailedSend>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, request_id, created_at, retry_count \
             FROM failed_sends \
             WHERE retry_count < 3 AND created_at > datetime('now', '-24 hours') \
             ORDER BY created_at ASC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(FailedSend {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    request_id: row.get(2)?,
                    created_at: row.get(3)?,
                    retry_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete a successfully flushed send.
    pub fn delete_failed_send(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM failed_sends WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Increment retry count on a failed flush.
    pub fn increment_failed_send_retry(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE failed_sends SET retry_count = retry_count + 1 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    // -- Conversation Compaction --

    /// Save a conversation summary row. Returns the new row ID.
    /// Only used directly by tests; production code uses `replace_with_summary`.
    #[cfg(test)]
    fn save_conversation_summary(&self, summary: &str, compacted_through_id: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO conversations (role, content, channel_type, compacted_through_id)
             VALUES ('summary', ?1, 'system', ?2)",
            rusqlite::params![summary, compacted_through_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete messages up to and including `through_id`, excluding summary rows.
    /// Returns the number of deleted rows.
    /// Only used directly by tests; production code uses `replace_with_summary`.
    #[cfg(test)]
    fn delete_compacted_messages(&self, through_id: i64) -> Result<u32> {
        let rows = self.conn.execute(
            "DELETE FROM conversations WHERE id <= ?1 AND role != 'summary'",
            [through_id],
        )?;
        Ok(rows as u32)
    }

    /// Load the most recent conversation summary (if any).
    pub fn load_conversation_summary(&self) -> Result<Option<ConversationMessage>> {
        let msg = self
            .conn
            .query_row(
                "SELECT id, role, content, channel_type, created_at
                 FROM conversations WHERE role = 'summary'
                 ORDER BY id DESC LIMIT 1",
                [],
                |row| {
                    Ok(ConversationMessage {
                        id: row.get(0)?,
                        role: row.get(1)?,
                        content: row.get(2)?,
                        channel_type: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(msg)
    }

    /// Count total non-summary messages.
    pub fn count_messages(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM conversations WHERE role != 'summary'",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Load messages older than the most recent `window_size` messages.
    /// Used by compaction to identify messages to summarize.
    pub fn load_messages_before_window(
        &self,
        window_size: usize,
    ) -> Result<Vec<ConversationMessage>> {
        // Get the ID threshold: the oldest message in the "keep" window
        let threshold_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(id) FROM (
                    SELECT id FROM conversations
                    WHERE role != 'summary'
                    ORDER BY id DESC LIMIT ?1
                )",
                [window_size as i64],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let Some(threshold_id) = threshold_id else {
            return Ok(Vec::new());
        };

        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, channel_type, created_at
             FROM conversations
             WHERE role != 'summary' AND id < ?1
             ORDER BY id ASC",
        )?;
        let messages = stmt
            .query_map([threshold_id], |row| {
                Ok(ConversationMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    channel_type: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(messages)
    }

    /// Atomically replace old messages with a summary.
    /// Deletes messages up to the highest ID in the batch and saves a summary row.
    pub fn replace_with_summary(&self, summary: &str, compacted_through_id: i64) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM conversations WHERE role = 'summary'", [])?;
        tx.execute(
            "DELETE FROM conversations WHERE id <= ?1 AND role != 'summary'",
            [compacted_through_id],
        )?;
        tx.execute(
            "INSERT INTO conversations (role, content, channel_type, compacted_through_id) VALUES ('summary', ?1, 'system', ?2)",
            rusqlite::params![summary, compacted_through_id],
        )?;
        let id = tx.query_row("SELECT last_insert_rowid()", [], |row| row.get(0))?;
        tx.commit()?;
        Ok(id)
    }

    // -- Search methods (SQL LIKE filter) --

    /// Search commitments by description substring (case-insensitive via LIKE).
    pub fn search_commitments(&self, query: &str) -> Result<Vec<Commitment>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, description, status, due_date, person_id, created_at, completed_at
             FROM commitments WHERE description LIKE '%' || ?1 || '%'
             ORDER BY due_date ASC NULLS LAST",
        )?;
        let commitments = stmt
            .query_map([query], |row| {
                Ok(Commitment {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    status: row.get(2)?,
                    due_date: row.get(3)?,
                    person_id: row.get(4)?,
                    created_at: row.get(5)?,
                    completed_at: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(commitments)
    }

    /// Search people by name, relationship, or notes substring (case-insensitive via LIKE).
    pub fn search_people(&self, query: &str) -> Result<Vec<Person>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, canonical_name, relationship, notes, first_mentioned, last_mentioned
             FROM people
             WHERE canonical_name LIKE '%' || ?1 || '%'
                OR relationship LIKE '%' || ?1 || '%'
                OR notes LIKE '%' || ?1 || '%'
             ORDER BY last_mentioned DESC",
        )?;
        let people = stmt
            .query_map([query], |row| {
                Ok(Person {
                    id: row.get(0)?,
                    canonical_name: row.get(1)?,
                    relationship: row.get(2)?,
                    notes: row.get(3)?,
                    first_mentioned: row.get(4)?,
                    last_mentioned: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(people)
    }

    /// Search preferences by category or value substring (case-insensitive via LIKE).
    pub fn search_preferences(&self, query: &str) -> Result<Vec<Preference>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT category, value, updated_at FROM preferences
             WHERE category LIKE '%' || ?1 || '%'
                OR value LIKE '%' || ?1 || '%'
             ORDER BY category",
        )?;
        let prefs = stmt
            .query_map([query], |row| {
                Ok(Preference {
                    category: row.get(0)?,
                    value: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(prefs)
    }

    /// Search events by description, date, or context substring (case-insensitive via LIKE).
    pub fn search_events(&self, query: &str) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, description, event_date, context, created_at
             FROM events
             WHERE description LIKE '%' || ?1 || '%'
                OR event_date LIKE '%' || ?1 || '%'
                OR context LIKE '%' || ?1 || '%'
             ORDER BY event_date ASC NULLS LAST",
        )?;
        let events = stmt
            .query_map([query], |row| {
                Ok(Event {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    event_date: row.get(2)?,
                    context: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(events)
    }

    /// Search pending reminders by message substring (case-insensitive via LIKE).
    pub fn search_reminders(&self, query: &str) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, fire_at, message, status, created_at, delivered_at
             FROM reminders
             WHERE status = 'pending' AND message LIKE '%' || ?1 || '%'
             ORDER BY fire_at ASC",
        )?;
        let reminders = stmt
            .query_map([query], |row| {
                Ok(Reminder {
                    id: row.get(0)?,
                    fire_at: row.get(1)?,
                    message: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    delivered_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(reminders)
    }

    // -- Layer 3: Search Indexing --

    /// Index content for search: inserts into search_content and fts_search.
    /// Returns the search_content.id for subsequent vector embedding storage.
    pub fn index_content(
        &self,
        source_type: &str,
        source_id: Option<i64>,
        content: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO search_content (source_type, source_id, content) VALUES (?1, ?2, ?3)",
            rusqlite::params![source_type, source_id, content],
        )?;
        let content_id = self.conn.last_insert_rowid();

        self.conn.execute(
            "INSERT INTO fts_search (content, content_id, source_type) VALUES (?1, ?2, ?3)",
            rusqlite::params![content, content_id, source_type],
        )?;

        Ok(content_id)
    }

    /// Store a vector embedding for a search_content row.
    pub fn index_embedding(&self, content_id: i64, embedding: &[f32]) -> Result<()> {
        let bytes: &[u8] = zerocopy::AsBytes::as_bytes(embedding);
        self.conn.execute(
            "INSERT INTO vec_search (content_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![content_id, bytes],
        )?;
        Ok(())
    }

    /// Delete search index entries for a specific source (for re-indexing on update).
    pub fn delete_search_content(&self, source_type: &str, source_id: i64) -> Result<()> {
        let subquery = "SELECT id FROM search_content WHERE source_type = ?1 AND source_id = ?2";

        self.conn.execute(
            &format!("DELETE FROM vec_search WHERE content_id IN ({subquery})"),
            rusqlite::params![source_type, source_id],
        )?;
        self.conn.execute(
            &format!("DELETE FROM fts_search WHERE content_id IN ({subquery})"),
            rusqlite::params![source_type, source_id],
        )?;
        self.conn.execute(
            "DELETE FROM search_content WHERE source_type = ?1 AND source_id = ?2",
            rusqlite::params![source_type, source_id],
        )?;

        Ok(())
    }

    /// Count total rows in search_content (used for backfill detection).
    pub fn count_search_content(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM search_content", [], |row| row.get(0))?;
        Ok(count)
    }

    // -- Layer 3: Search Queries --

    /// Sanitize a query for FTS5 MATCH by wrapping in double quotes.
    /// Escapes any embedded double quotes to prevent FTS5 syntax errors
    /// from special characters (AND, OR, NOT, parentheses, etc.).
    fn sanitize_fts5_query(query: &str) -> String {
        let escaped = query.replace('"', "\"\"");
        format!("\"{escaped}\"")
    }

    /// FTS5-only search (BM25 keyword ranking). Used when no embedding client.
    pub fn fts_search(
        &self,
        query: &str,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let safe_query = Self::sanitize_fts5_query(query);

        let base_sql = "SELECT sc.id, sc.source_type, sc.source_id, sc.content, fts.rank
                 FROM fts_search fts
                 JOIN search_content sc ON sc.id = CAST(fts.content_id AS INTEGER)
                 WHERE fts_search MATCH ?1";

        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match source_type_filter
        {
            Some(st) => (
                format!("{base_sql} AND sc.source_type = ?3 ORDER BY fts.rank LIMIT ?2"),
                vec![
                    Box::new(safe_query),
                    Box::new(limit as i64),
                    Box::new(st.to_string()),
                ],
            ),
            None => (
                format!("{base_sql} ORDER BY fts.rank LIMIT ?2"),
                vec![Box::new(safe_query), Box::new(limit as i64)],
            ),
        };
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let results = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(SearchResult {
                    id: row.get(0)?,
                    source_type: row.get(1)?,
                    source_id: row.get(2)?,
                    content: row.get(3)?,
                    score: row.get::<_, f64>(4)?.abs(), // BM25 returns negative, negate
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(results)
    }

    /// Vector-only search. Returns content_ids ranked by cosine distance.
    pub fn vec_search(&self, embedding: &[f32], limit: usize) -> Result<Vec<(i64, f64)>> {
        let bytes: &[u8] = zerocopy::AsBytes::as_bytes(embedding);
        let mut stmt = self.conn.prepare(
            "SELECT content_id, distance
             FROM vec_search
             WHERE embedding MATCH ?1 AND k = ?2
             ORDER BY distance",
        )?;
        let results = stmt
            .query_map(rusqlite::params![bytes, limit as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(results)
    }

    /// Hybrid search using Reciprocal Rank Fusion (RRF) of FTS5 + vector results.
    pub fn hybrid_search(
        &self,
        fts_query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
        source_type_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        // Get FTS5 results with rank positions
        let fts_results = self.fts_search(fts_query, limit * 2, source_type_filter)?;

        // If no embedding provided, return FTS5-only results
        let vec_results = match embedding {
            Some(emb) => self.vec_search(emb, limit * 2)?,
            None => Vec::new(),
        };

        if vec_results.is_empty() {
            // FTS5-only: just truncate to limit
            let mut results = fts_results;
            results.truncate(limit);
            return Ok(results);
        }

        // RRF merge with k=60
        const RRF_K: f64 = 60.0;
        let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();

        for (rank, result) in fts_results.iter().enumerate() {
            *scores.entry(result.id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        for (rank, (content_id, _distance)) in vec_results.iter().enumerate() {
            *scores.entry(*content_id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }

        // Sort by RRF score descending
        let mut scored: Vec<(i64, f64)> = scores.into_iter().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        // Fetch full content for top results, applying source_type filter for vec results
        let mut results = Vec::with_capacity(scored.len());
        for (content_id, score) in scored {
            if let Ok(result) = self.conn.query_row(
                "SELECT id, source_type, source_id, content FROM search_content WHERE id = ?1",
                [content_id],
                |row| {
                    Ok(SearchResult {
                        id: row.get(0)?,
                        source_type: row.get(1)?,
                        source_id: row.get(2)?,
                        content: row.get(3)?,
                        score,
                    })
                },
            ) {
                // Filter by source_type if specified (vec results weren't pre-filtered)
                if let Some(st) = source_type_filter
                    && result.source_type != st
                {
                    continue;
                }
                results.push(result);
            }
        }

        Ok(results)
    }

    /// List all facts for backfill indexing (returns source_type, source_id, content text).
    pub fn get_all_facts_for_indexing(&self) -> Result<Vec<(String, i64, String)>> {
        let mut facts = Vec::new();

        // People
        let mut stmt = self
            .conn
            .prepare("SELECT id, canonical_name, relationship, notes FROM people")?;
        let people = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let rel: Option<String> = row.get(2)?;
            let notes: Option<String> = row.get(3)?;
            let mut content = name.clone();
            if let Some(r) = rel {
                content.push_str(&format!(" — {r}"));
            }
            if let Some(n) = notes {
                content.push_str(&format!(". {n}"));
            }
            Ok((id, content))
        })?;
        for row in people {
            let (id, content) = row?;
            facts.push(("person".to_string(), id, content));
        }

        // Commitments
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, due_date, status FROM commitments")?;
        let commits = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let desc: String = row.get(1)?;
            let due: Option<String> = row.get(2)?;
            let status: String = row.get(3)?;
            let mut content = desc;
            if let Some(d) = due {
                content.push_str(&format!(" (due: {d})"));
            }
            content.push_str(&format!(", status: {status}"));
            Ok((id, content))
        })?;
        for row in commits {
            let (id, content) = row?;
            facts.push(("commitment".to_string(), id, content));
        }

        // Preferences
        let mut stmt = self
            .conn
            .prepare("SELECT id, category, value FROM preferences")?;
        let prefs = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let cat: String = row.get(1)?;
            let val: String = row.get(2)?;
            Ok((id, format!("{cat}: {val}")))
        })?;
        for row in prefs {
            let (id, content) = row?;
            facts.push(("preference".to_string(), id, content));
        }

        // Events
        let mut stmt = self
            .conn
            .prepare("SELECT id, description, event_date, context FROM events")?;
        let events = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let desc: String = row.get(1)?;
            let date: Option<String> = row.get(2)?;
            let ctx: Option<String> = row.get(3)?;
            let mut content = desc;
            if let Some(d) = date {
                content.push_str(&format!(" on {d}"));
            }
            if let Some(c) = ctx {
                content.push_str(&format!(". {c}"));
            }
            Ok((id, content))
        })?;
        for row in events {
            let (id, content) = row?;
            facts.push(("event".to_string(), id, content));
        }

        Ok(facts)
    }

    // -- Customer Config --

    /// Get a customer config value by key.
    pub fn get_customer_config(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM customer_config WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    /// Set a customer config value.
    pub fn set_customer_config(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO customer_config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// List all customer config entries.
    pub fn list_customer_config(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM customer_config ORDER BY key")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // -- Cross-Channel Queries --

    /// Load messages with id > after_id, optionally filtered by channel type.
    /// Returns messages in ascending id order.
    pub fn load_messages_after(
        &self,
        after_id: i64,
        channel_types: Option<&[&str]>,
    ) -> Result<Vec<ConversationMessage>> {
        let (sql, params) = match channel_types {
            Some(types) if !types.is_empty() => {
                let placeholders: Vec<String> =
                    (0..types.len()).map(|i| format!("?{}", i + 2)).collect();
                let sql = format!(
                    "SELECT id, role, content, channel_type, created_at
                     FROM conversations
                     WHERE id > ?1 AND role != 'summary' AND channel_type IN ({})
                     ORDER BY id ASC",
                    placeholders.join(", ")
                );
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(after_id)];
                for t in types {
                    params.push(Box::new(t.to_string()));
                }
                (sql, params)
            }
            _ => {
                let sql = "SELECT id, role, content, channel_type, created_at
                     FROM conversations
                     WHERE id > ?1 AND role != 'summary'
                     ORDER BY id ASC"
                    .to_string();
                let params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(after_id)];
                (sql, params)
            }
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let messages: Vec<ConversationMessage> = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(ConversationMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    channel_type: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(messages)
    }

    /// Get the maximum message id in the conversations table.
    /// Returns 0 if the table is empty.
    pub fn max_message_id(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM conversations",
            [],
            |row| row.get(0),
        )?)
    }

    // -- Memory Events (Audit Log) --

    /// Log a memory mutation event for auditability.
    pub fn log_memory_event(
        &self,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: &str,
        reasoning: Option<&str>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO memory_events (session_id, tool_name, target_key, before_value, after_value, reasoning)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![session_id, tool_name, target_key, before_value, after_value, reasoning],
            )
            .context("failed to log memory event")?;
        Ok(())
    }

    /// Get memory events for a session.
    pub fn get_memory_events(&self, session_id: &str) -> Result<Vec<MemoryEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, tool_name, target_key, before_value, after_value, reasoning, created_at
             FROM memory_events WHERE session_id = ?1 ORDER BY id",
        )?;

        let events: Vec<MemoryEvent> = stmt
            .query_map([session_id], |row| {
                Ok(MemoryEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    tool_name: row.get(2)?,
                    target_key: row.get(3)?,
                    before_value: row.get(4)?,
                    after_value: row.get(5)?,
                    reasoning: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(events)
    }

    // -- Tiered Retention --

    /// Compact memory events older than `days` days into monthly summaries.
    /// Uses SQL GROUP BY for aggregation (memory is O(months), not O(events)).
    /// All operations run within a single transaction for atomicity.
    /// Returns the number of raw events deleted (0 if nothing to compact).
    pub fn compact_old_memory_events(&self, days: u32) -> Result<usize> {
        use std::collections::HashMap;

        let modifier = format!("-{days} days");

        // Wrap everything in one transaction for atomicity
        let tx = self.conn.unchecked_transaction()?;

        let cutoff: String =
            tx.query_row("SELECT datetime('now', ?1)", [&modifier], |row| row.get(0))?;

        // Check if there are any events to compact (early exit avoids unnecessary work)
        let event_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM memory_events WHERE created_at < ?1",
            [&cutoff],
            |row| row.get(0),
        )?;

        if event_count == 0 {
            tx.commit()?;
            return Ok(0);
        }

        // Aggregate tool counts per month using SQL GROUP BY — O(months * tools)
        {
            let mut tool_stmt = tx.prepare(
                "SELECT CAST(strftime('%Y', created_at) AS INTEGER),
                        CAST(strftime('%m', created_at) AS INTEGER),
                        tool_name,
                        COUNT(*)
                 FROM memory_events
                 WHERE created_at < ?1
                 GROUP BY strftime('%Y', created_at), strftime('%m', created_at), tool_name",
            )?;

            let mut month_tools: HashMap<(i64, i64), HashMap<String, u32>> = HashMap::new();
            let mut rows = tool_stmt.query([&cutoff])?;
            while let Some(row) = rows.next()? {
                let year: i64 = row.get(0)?;
                let month: i64 = row.get(1)?;
                let tool_name: String = row.get(2)?;
                let count: u32 = row.get(3)?;
                month_tools
                    .entry((year, month))
                    .or_default()
                    .insert(tool_name, count);
            }
            drop(rows);
            drop(tool_stmt);

            // Aggregate category counts (category = part before ':' in target_key)
            // and target counts per month
            let mut detail_stmt = tx.prepare(
                "SELECT CAST(strftime('%Y', created_at) AS INTEGER),
                        CAST(strftime('%m', created_at) AS INTEGER),
                        target_key,
                        COUNT(*)
                 FROM memory_events
                 WHERE created_at < ?1
                 GROUP BY strftime('%Y', created_at), strftime('%m', created_at), target_key",
            )?;

            struct MonthAgg {
                category_counts: HashMap<String, u32>,
                target_counts: HashMap<String, u32>,
            }

            let mut month_details: HashMap<(i64, i64), MonthAgg> = HashMap::new();
            let mut rows = detail_stmt.query([&cutoff])?;
            while let Some(row) = rows.next()? {
                let year: i64 = row.get(0)?;
                let month: i64 = row.get(1)?;
                let target_key: String = row.get(2)?;
                let count: u32 = row.get(3)?;

                let category = target_key
                    .split(':')
                    .next()
                    .unwrap_or(&target_key)
                    .to_string();

                let agg = month_details
                    .entry((year, month))
                    .or_insert_with(|| MonthAgg {
                        category_counts: HashMap::new(),
                        target_counts: HashMap::new(),
                    });
                *agg.category_counts.entry(category).or_insert(0) += count;
                agg.target_counts.insert(target_key, count);
            }
            drop(rows);
            drop(detail_stmt);

            // Aggregate total mutations per month
            let mut total_stmt = tx.prepare(
                "SELECT CAST(strftime('%Y', created_at) AS INTEGER),
                        CAST(strftime('%m', created_at) AS INTEGER),
                        COUNT(*)
                 FROM memory_events
                 WHERE created_at < ?1
                 GROUP BY strftime('%Y', created_at), strftime('%m', created_at)",
            )?;

            let mut month_totals: HashMap<(i64, i64), u32> = HashMap::new();
            let mut rows = total_stmt.query([&cutoff])?;
            while let Some(row) = rows.next()? {
                let year: i64 = row.get(0)?;
                let month: i64 = row.get(1)?;
                let total: u32 = row.get(2)?;
                month_totals.insert((year, month), total);
            }
            drop(rows);
            drop(total_stmt);

            // Write summaries with ON CONFLICT upsert
            for (&(year, month), total) in &month_totals {
                let tool_counts = month_tools.get(&(year, month));
                let details = month_details.get(&(year, month));

                let tool_counts_json =
                    serde_json::to_string(&tool_counts.unwrap_or(&HashMap::new()))?;
                let category_counts_json = serde_json::to_string(
                    &details
                        .map(|d| &d.category_counts)
                        .unwrap_or(&HashMap::new()),
                )?;

                // Build top targets (top 10 by count)
                let top_targets_json = if let Some(d) = details {
                    let mut targets: Vec<(&String, &u32)> = d.target_counts.iter().collect();
                    targets.sort_by(|a, b| b.1.cmp(a.1));
                    targets.truncate(10);
                    let top: Vec<String> = targets
                        .iter()
                        .map(|(k, v)| format!("{}:{}", k, v))
                        .collect();
                    serde_json::to_string(&top)?
                } else {
                    "[]".to_string()
                };

                tx.execute(
                    "INSERT INTO memory_event_summaries (year, month, tool_counts, category_counts, total_mutations, top_targets)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(year, month) DO UPDATE SET
                        tool_counts = excluded.tool_counts,
                        category_counts = excluded.category_counts,
                        total_mutations = excluded.total_mutations,
                        top_targets = excluded.top_targets",
                    rusqlite::params![
                        year,
                        month,
                        tool_counts_json,
                        category_counts_json,
                        total,
                        top_targets_json
                    ],
                )?;
            }
        }

        // Batch delete all compacted raw events in one statement
        let deleted = tx.execute("DELETE FROM memory_events WHERE created_at < ?1", [&cutoff])?;

        tx.commit()?;
        Ok(deleted)
    }

    /// Get database file size using PRAGMA page_count * PRAGMA page_size.
    pub fn db_size_bytes(&self) -> Result<u64> {
        let page_count: u64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: u64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok(page_count * page_size)
    }

    /// Reclaim space after deletions using incremental auto-vacuum.
    /// Frees up to 100 pages per call, avoiding the full-db rewrite of VACUUM.
    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA incremental_vacuum(100)")?;
        Ok(())
    }

    /// Return the current schema version of the database.
    pub fn schema_version(&self) -> Result<i64> {
        self.current_version()
    }
}

// -- Public types --

/// Type-safe filter for reminder queries, replacing raw SQL string interpolation.
pub enum ReminderFilter {
    /// All pending reminders regardless of fire_at.
    All,
    /// Pending reminders with fire_at in the future.
    Future,
    /// Pending reminders with fire_at at or past now.
    PastDue,
}

#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub channel_type: String,
    pub created_at: String,
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
pub struct Reminder {
    pub id: i64,
    pub fire_at: String,
    pub message: String,
    pub status: String,
    pub created_at: String,
    pub delivered_at: Option<String>,
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

// Bring in rusqlite optional extension
use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use crate::test_utils::test_helpers::test_db;

    #[test]
    fn test_migration_creates_tables() {
        let db = test_db();
        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 8);
    }

    #[test]
    fn test_migration_idempotent() {
        let db = test_db();
        db.migrate().unwrap();
        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 8);
    }

    #[test]
    fn test_conversation_roundtrip() {
        let db = test_db();
        db.save_message("user", "Hello Mika!", "telegram").unwrap();
        db.save_message("assistant", "Hi! How can I help?", "telegram")
            .unwrap();

        let messages = db.load_recent_messages(10, None).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello Mika!");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Hi! How can I help?");
    }

    #[test]
    fn test_conversation_limit() {
        let db = test_db();
        for i in 0..20 {
            db.save_message("user", &format!("Message {i}"), "telegram")
                .unwrap();
        }
        let messages = db.load_recent_messages(5, None).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, "Message 15");
        assert_eq!(messages[4].content, "Message 19");
    }

    #[test]
    fn test_core_memory_crud() {
        let db = test_db();

        let tokens = db.set_core_memory("user_summary", "Loves coffee.").unwrap();
        assert!(tokens > 0);

        let entry = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(entry.value, "Loves coffee.");
        assert_eq!(entry.key, "user_summary");

        db.set_core_memory("user_summary", "Loves coffee and tea.")
            .unwrap();
        let entry = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(entry.value, "Loves coffee and tea.");

        assert!(db.get_core_memory("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_core_memory_seed() {
        let db = test_db();
        db.seed_core_memory(None).unwrap();

        let all = db.get_all_core_memory().unwrap();
        assert_eq!(all.len(), 4);

        let summary = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(summary.value, "New user. No information yet.");
    }

    #[test]
    fn test_core_memory_seed_with_user_md() {
        let db = test_db();
        db.seed_core_memory(Some("I'm a CEO who loves hiking"))
            .unwrap();

        let summary = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(summary.value, "I'm a CEO who loves hiking");
    }

    #[test]
    fn test_total_token_count() {
        let db = test_db();
        db.set_core_memory("a", &"x".repeat(100)).unwrap();
        db.set_core_memory("b", &"y".repeat(200)).unwrap();
        let total = db.total_core_memory_tokens().unwrap();
        assert_eq!(total, 75);
    }

    #[test]
    fn test_people_crud() {
        let db = test_db();

        let id = db
            .upsert_person("Sarah Chen", Some("colleague"), Some("VP of Engineering"))
            .unwrap();
        assert!(id > 0);

        let person = db.get_person("Sarah Chen").unwrap().unwrap();
        assert_eq!(person.canonical_name, "Sarah Chen");
        assert_eq!(person.relationship, Some("colleague".to_string()));
        assert_eq!(person.notes, Some("VP of Engineering".to_string()));

        db.upsert_person("Sarah Chen", Some("manager"), None)
            .unwrap();
        let person = db.get_person("Sarah Chen").unwrap().unwrap();
        assert_eq!(person.relationship, Some("manager".to_string()));

        assert!(db.get_person("Unknown").unwrap().is_none());
    }

    #[test]
    fn test_people_list() {
        let db = test_db();
        db.upsert_person("Alice", None, None).unwrap();
        db.upsert_person("Bob", None, None).unwrap();

        let people = db.list_people().unwrap();
        assert_eq!(people.len(), 2);
    }

    #[test]
    fn test_person_lookup_case_insensitive() {
        let db = test_db();
        db.upsert_person("Sarah Chen", Some("colleague"), None)
            .unwrap();

        // Lookup with different casing should find the same person
        let person = db.get_person("sarah chen").unwrap().unwrap();
        assert_eq!(person.canonical_name, "Sarah Chen");
        assert_eq!(person.relationship, Some("colleague".to_string()));

        let person = db.get_person("SARAH CHEN").unwrap().unwrap();
        assert_eq!(person.canonical_name, "Sarah Chen");
    }

    #[test]
    fn test_commitments() {
        let db = test_db();

        let id = db
            .add_commitment("Review Q4 budget", Some("2026-03-01"), None)
            .unwrap();
        assert!(id > 0);

        // Duplicate should be ignored (same description, case-insensitive)
        db.add_commitment("Review Q4 budget", Some("2026-03-01"), None)
            .unwrap();

        let pending = db.list_commitments("pending").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].description, "Review Q4 budget");

        db.update_commitment_status(id, "completed").unwrap();
        let pending = db.list_commitments("pending").unwrap();
        assert_eq!(pending.len(), 0);
        let completed = db.list_commitments("completed").unwrap();
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn test_commitment_dedup_case_insensitive() {
        let db = test_db();
        db.add_commitment("Review Q4 budget", Some("2026-03-01"), None)
            .unwrap();
        db.add_commitment("review q4 budget", Some("2026-03-01"), None)
            .unwrap();

        let pending = db.list_commitments("pending").unwrap();
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn test_preferences() {
        let db = test_db();

        db.set_preference("communication_style", "Direct and concise")
            .unwrap();
        let pref = db.get_preference("communication_style").unwrap().unwrap();
        assert_eq!(pref, "Direct and concise");

        db.set_preference("communication_style", "Friendly and warm")
            .unwrap();
        let pref = db.get_preference("communication_style").unwrap().unwrap();
        assert_eq!(pref, "Friendly and warm");

        assert!(db.get_preference("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_preference_case_insensitive() {
        let db = test_db();
        db.set_preference("Food", "No shellfish").unwrap();

        let pref = db.get_preference("food").unwrap().unwrap();
        assert_eq!(pref, "No shellfish");

        let pref = db.get_preference("FOOD").unwrap().unwrap();
        assert_eq!(pref, "No shellfish");
    }

    #[test]
    fn test_events() {
        let db = test_db();

        let id = db
            .add_event("Board meeting", Some("2026-03-15"), Some("business"))
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_memory_events() {
        let db = test_db();
        db.log_memory_event(
            "sess-1",
            "store_fact",
            "person:Alice",
            None,
            "Alice — colleague",
            None,
        )
        .unwrap();
        db.log_memory_event(
            "sess-1",
            "update_core_memory",
            "persona",
            Some("old"),
            "new",
            Some("user asked"),
        )
        .unwrap();

        let events = db.get_memory_events("sess-1").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].tool_name, "store_fact");
        assert_eq!(events[0].target_key, "person:Alice");
        assert_eq!(events[1].reasoning, Some("user asked".to_string()));

        // Different session returns empty
        let events = db.get_memory_events("other").unwrap();
        assert!(events.is_empty());
    }

    // -- Migration tests --

    #[test]
    fn test_v5_tables_exist() {
        let db = test_db();
        // Verify all v5+ tables exist by querying sqlite_master
        let tables: Vec<String> = {
            let mut stmt = db
                .conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(tables.contains(&"reminders".to_string()));
        assert!(tables.contains(&"heartbeat_sends".to_string()));
        assert!(tables.contains(&"customer_config".to_string()));
        assert!(tables.contains(&"failed_sends".to_string()));
        assert!(tables.contains(&"memory_event_summaries".to_string()));
    }

    #[test]
    fn test_conversations_has_compacted_through_id() {
        let db = test_db();
        // Verify the column exists by inserting a row with it
        db.conn
            .execute(
                "INSERT INTO conversations (role, content, channel_type, compacted_through_id)
                 VALUES ('summary', 'test', 'system', 42)",
                [],
            )
            .unwrap();
        let val: i64 = db
            .conn
            .query_row(
                "SELECT compacted_through_id FROM conversations WHERE role = 'summary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 42);
    }

    // -- Channel type filter tests --

    #[test]
    fn test_load_messages_channel_filter() {
        let db = test_db();
        db.save_message("user", "cli msg", "cli").unwrap();
        db.save_message("user", "telegram msg", "telegram").unwrap();
        db.save_message("user", "heartbeat msg", "heartbeat")
            .unwrap();

        // Filter to only cli + telegram
        let msgs = db
            .load_recent_messages(10, Some(&["cli", "telegram"]))
            .unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|m| m.channel_type != "heartbeat"));

        // No filter returns all
        let all = db.load_recent_messages(10, None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_load_messages_excludes_summary() {
        let db = test_db();
        db.save_message("user", "real msg", "cli").unwrap();
        db.save_conversation_summary("old summary", 0).unwrap();

        let msgs = db.load_recent_messages(10, None).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "real msg");
    }

    // -- Reminder tests --

    #[test]
    fn test_reminder_lifecycle() {
        let db = test_db();

        // Create two reminders: one in the past, one in the future
        let past_id = db
            .add_reminder("2020-01-01T00:00:00Z", "past reminder")
            .unwrap();
        let future_id = db
            .add_reminder("2099-12-31T23:59:59Z", "future reminder")
            .unwrap();
        assert!(past_id > 0);
        assert!(future_id > past_id);

        // All pending
        let pending = db.get_pending_reminders().unwrap();
        assert_eq!(pending.len(), 2);

        // Future only
        let future = db.get_future_reminders().unwrap();
        assert_eq!(future.len(), 1);
        assert_eq!(future[0].message, "future reminder");

        // Past due only
        let past_due = db.get_past_due_reminders().unwrap();
        assert_eq!(past_due.len(), 1);
        assert_eq!(past_due[0].message, "past reminder");

        // Mark past as delivered
        db.mark_reminder_delivered(past_id).unwrap();
        let pending = db.get_pending_reminders().unwrap();
        assert_eq!(pending.len(), 1);

        // Cancel future
        let cancelled = db.cancel_reminder(future_id).unwrap();
        assert!(cancelled);
        let pending = db.get_pending_reminders().unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn test_cancel_nonpending_reminder() {
        let db = test_db();
        let id = db
            .add_reminder("2020-01-01T00:00:00Z", "already delivered")
            .unwrap();
        db.mark_reminder_delivered(id).unwrap();

        // Cancelling a delivered reminder returns false
        let cancelled = db.cancel_reminder(id).unwrap();
        assert!(!cancelled);
    }

    #[test]
    fn test_mark_reminder_failed() {
        let db = test_db();
        let id = db
            .add_reminder("2020-01-01T00:00:00Z", "will fail")
            .unwrap();
        db.mark_reminder_failed(id).unwrap();

        let pending = db.get_pending_reminders().unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn test_get_pending_reminders_excludes_cancelled() {
        let db = test_db();
        db.add_reminder("2099-01-01T00:00:00Z", "active one")
            .unwrap();
        let id2 = db
            .add_reminder("2099-02-01T00:00:00Z", "will cancel")
            .unwrap();
        db.cancel_reminder(id2).unwrap();

        let active = db.get_pending_reminders().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].message, "active one");
    }

    // -- Heartbeat pruning tests --

    #[test]
    fn test_prune_heartbeat_sends() {
        let db = test_db();
        // Insert a send with an old timestamp manually
        db.conn
            .execute(
                "INSERT INTO heartbeat_sends (sent_at) VALUES (datetime('now', '-10 days'))",
                [],
            )
            .unwrap();
        // Insert a recent one directly
        db.conn
            .execute("INSERT INTO heartbeat_sends DEFAULT VALUES", [])
            .unwrap();

        let total_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM heartbeat_sends", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total_before, 2);

        // Prune sends older than 7 days
        db.prune_old_heartbeat_sends(7).unwrap();

        // The old one should be gone, recent one remains
        let total: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM heartbeat_sends", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn test_record_and_count_heartbeat_sends() {
        let db = test_db();

        assert_eq!(db.count_heartbeat_sends_last_hour().unwrap(), 0);
        assert_eq!(db.count_heartbeat_sends_today("UTC").unwrap(), 0);

        db.record_heartbeat_send().unwrap();
        db.record_heartbeat_send().unwrap();

        assert_eq!(db.count_heartbeat_sends_last_hour().unwrap(), 2);
        assert_eq!(db.count_heartbeat_sends_today("UTC").unwrap(), 2);
    }

    #[test]
    fn test_last_user_message_time() {
        let db = test_db();

        // No messages yet
        assert!(db.last_user_message_time().unwrap().is_none());

        // Add a user message
        db.save_message("user", "hello", "cli").unwrap();
        let ts = db.last_user_message_time().unwrap();
        assert!(ts.is_some());

        // Heartbeat messages should not count
        db.save_message("assistant", "hi", "heartbeat").unwrap();
        let ts2 = db.last_user_message_time().unwrap();
        assert_eq!(ts, ts2); // same as before
    }

    #[test]
    fn test_failed_send_lifecycle() {
        let db = test_db();

        // No sends initially
        let sends = db.get_pending_failed_sends(10).unwrap();
        assert!(sends.is_empty());

        // Save a failed send
        let id = db.save_failed_send("hello user", Some("req-123")).unwrap();
        assert!(id > 0);

        // Retrieve it
        let sends = db.get_pending_failed_sends(10).unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].text, "hello user");
        assert_eq!(sends[0].request_id.as_deref(), Some("req-123"));
        assert_eq!(sends[0].retry_count, 0);

        // Increment retry
        db.increment_failed_send_retry(id).unwrap();
        let sends = db.get_pending_failed_sends(10).unwrap();
        assert_eq!(sends[0].retry_count, 1);

        // Delete after successful flush
        db.delete_failed_send(id).unwrap();
        let sends = db.get_pending_failed_sends(10).unwrap();
        assert!(sends.is_empty());
    }

    #[test]
    fn test_failed_sends_respects_retry_limit() {
        let db = test_db();

        let id = db.save_failed_send("doomed msg", None).unwrap();
        db.increment_failed_send_retry(id).unwrap();
        db.increment_failed_send_retry(id).unwrap();
        db.increment_failed_send_retry(id).unwrap();

        // retry_count = 3 → should not appear in pending
        let sends = db.get_pending_failed_sends(10).unwrap();
        assert!(sends.is_empty());
    }

    // -- Compaction tests --

    #[test]
    fn test_save_and_load_summary() {
        let db = test_db();

        // Initially no summary
        assert!(db.load_conversation_summary().unwrap().is_none());

        let id = db
            .save_conversation_summary("Summary of messages 1-10", 10)
            .unwrap();
        assert!(id > 0);

        let summary = db.load_conversation_summary().unwrap().unwrap();
        assert_eq!(summary.role, "summary");
        assert_eq!(summary.content, "Summary of messages 1-10");
    }

    #[test]
    fn test_delete_compacted_messages() {
        let db = test_db();

        // Insert 5 messages
        for i in 0..5 {
            db.save_message("user", &format!("msg {i}"), "cli").unwrap();
        }
        let all = db.load_recent_messages(10, None).unwrap();
        assert_eq!(all.len(), 5);
        let third_id = all[2].id;

        // Delete messages up to and including the third
        let deleted = db.delete_compacted_messages(third_id).unwrap();
        assert_eq!(deleted, 3);

        let remaining = db.load_recent_messages(10, None).unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn test_count_messages() {
        let db = test_db();
        assert_eq!(db.count_messages().unwrap(), 0);

        db.save_message("user", "one", "cli").unwrap();
        db.save_message("assistant", "two", "cli").unwrap();
        assert_eq!(db.count_messages().unwrap(), 2);

        // Summary rows don't count
        db.save_conversation_summary("summary", 0).unwrap();
        assert_eq!(db.count_messages().unwrap(), 2);
    }

    #[test]
    fn test_load_messages_before_window() {
        let db = test_db();

        // Insert 10 messages
        for i in 0..10 {
            db.save_message("user", &format!("msg {i}"), "cli").unwrap();
        }

        // Window of 3 keeps the last 3, so 7 should be "before window"
        let old = db.load_messages_before_window(3).unwrap();
        assert_eq!(old.len(), 7);
        assert_eq!(old[0].content, "msg 0");
        assert_eq!(old[6].content, "msg 6");
    }

    #[test]
    fn test_load_messages_before_window_empty() {
        let db = test_db();

        // Fewer messages than window size
        db.save_message("user", "only one", "cli").unwrap();
        let old = db.load_messages_before_window(5).unwrap();
        assert_eq!(old.len(), 0);
    }

    #[test]
    fn test_replace_with_summary() {
        let db = test_db();

        // Insert 10 messages
        for i in 0..10 {
            db.save_message("user", &format!("msg {i}"), "cli").unwrap();
        }
        let all = db.load_recent_messages(20, None).unwrap();
        let seventh_id = all[6].id; // msg 6's id

        // Replace messages 0-6 with a summary
        let summary_id = db
            .replace_with_summary("Summary of messages 0-6", seventh_id)
            .unwrap();
        assert!(summary_id > 0);

        // Should have 3 real messages remaining
        let remaining = db.load_recent_messages(20, None).unwrap();
        assert_eq!(remaining.len(), 3);
        assert_eq!(remaining[0].content, "msg 7");

        // Summary should be loadable
        let summary = db.load_conversation_summary().unwrap().unwrap();
        assert_eq!(summary.content, "Summary of messages 0-6");
    }

    // -- Customer config tests --

    #[test]
    fn test_customer_config_crud() {
        let db = test_db();

        // Initially empty
        assert!(db.get_customer_config("timezone").unwrap().is_none());

        // Set and get
        db.set_customer_config("timezone", "+08:00").unwrap();
        let tz = db.get_customer_config("timezone").unwrap().unwrap();
        assert_eq!(tz, "+08:00");

        // Upsert
        db.set_customer_config("timezone", "-05:00").unwrap();
        let tz = db.get_customer_config("timezone").unwrap().unwrap();
        assert_eq!(tz, "-05:00");
    }

    // -- Compaction tests --

    /// Helper to insert a memory event with a specific created_at timestamp.
    fn insert_event_at(db: &super::Database, tool: &str, target: &str, created_at: &str) {
        db.conn
            .execute(
                "INSERT INTO memory_events (session_id, tool_name, target_key, after_value, created_at)
                 VALUES ('test', ?1, ?2, 'v', ?3)",
                rusqlite::params![tool, target, created_at],
            )
            .unwrap();
    }

    #[test]
    fn test_compact_no_old_events() {
        let db = test_db();
        // No events at all — should return 0
        let deleted = db.compact_old_memory_events(90).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_compact_groups_by_month() {
        let db = test_db();
        // Insert events across 3 different months, all old enough to compact
        insert_event_at(&db, "store_fact", "person:Alice", "2020-01-15 10:00:00");
        insert_event_at(&db, "store_fact", "person:Bob", "2020-01-20 10:00:00");
        insert_event_at(&db, "update_fact", "commitment:1", "2020-02-10 10:00:00");
        insert_event_at(&db, "store_fact", "event:meeting", "2020-03-05 10:00:00");

        let deleted = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted, 4);

        // Should have 3 summary rows (one per month)
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM memory_event_summaries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 3);

        // Raw events should be gone
        let remaining: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM memory_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_compact_preserves_recent_events() {
        let db = test_db();
        // Insert old event
        insert_event_at(&db, "store_fact", "person:Alice", "2020-01-15 10:00:00");
        // Insert recent event (use a far-future date so it's always "recent")
        insert_event_at(&db, "store_fact", "person:Bob", "2099-12-31 10:00:00");

        let deleted = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted, 1);

        // Recent event should still exist
        let remaining: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM memory_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_compact_returns_deletion_count() {
        let db = test_db();
        insert_event_at(&db, "store_fact", "person:A", "2020-01-01 10:00:00");
        insert_event_at(&db, "store_fact", "person:B", "2020-01-02 10:00:00");
        insert_event_at(&db, "store_fact", "person:C", "2020-01-03 10:00:00");

        let deleted = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted, 3);
    }

    #[test]
    fn test_compact_idempotent() {
        let db = test_db();
        insert_event_at(&db, "store_fact", "person:Alice", "2020-01-15 10:00:00");
        insert_event_at(&db, "update_fact", "person:Alice", "2020-01-20 10:00:00");

        // First compaction
        let deleted1 = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted1, 2);

        // Second compaction — nothing left to compact
        let deleted2 = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted2, 0);

        // Summary should still be there and unchanged
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM memory_event_summaries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_compact_summary_content() {
        let db = test_db();
        insert_event_at(&db, "store_fact", "person:Alice", "2020-03-10 10:00:00");
        insert_event_at(&db, "store_fact", "person:Bob", "2020-03-15 10:00:00");
        insert_event_at(&db, "update_fact", "commitment:1", "2020-03-20 10:00:00");

        db.compact_old_memory_events(1).unwrap();

        // Verify the summary row content
        let (tool_counts, category_counts, total, year, month): (
            String,
            String,
            i64,
            i64,
            i64,
        ) = db
            .conn
            .query_row(
                "SELECT tool_counts, category_counts, total_mutations, year, month FROM memory_event_summaries",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();

        assert_eq!(year, 2020);
        assert_eq!(month, 3);
        assert_eq!(total, 3);

        let tc: serde_json::Value = serde_json::from_str(&tool_counts).unwrap();
        assert_eq!(tc["store_fact"], 2);
        assert_eq!(tc["update_fact"], 1);

        let cc: serde_json::Value = serde_json::from_str(&category_counts).unwrap();
        assert_eq!(cc["person"], 2);
        assert_eq!(cc["commitment"], 1);
    }

    #[test]
    fn test_compact_boundary_date_excludes_recent() {
        let db = test_db();
        // Insert an event right at "now" — it should NOT be compacted even with days=0
        // because the cutoff is datetime('now', '-0 days') which equals now, and we use < not <=
        db.log_memory_event("sess-1", "store_fact", "person:Alice", None, "v", None)
            .unwrap();

        // With days=0, cutoff is essentially "now"; event created at "now" should
        // not be strictly less than cutoff
        let deleted = db.compact_old_memory_events(0).unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn test_compact_multiple_months_separate_summaries() {
        let db = test_db();
        // Events spanning 5 months
        insert_event_at(&db, "store_fact", "person:A", "2019-06-01 10:00:00");
        insert_event_at(&db, "store_fact", "person:B", "2019-07-15 10:00:00");
        insert_event_at(&db, "update_fact", "person:C", "2019-08-20 10:00:00");
        insert_event_at(&db, "store_fact", "event:meeting", "2019-09-10 10:00:00");
        insert_event_at(&db, "store_fact", "person:D", "2019-10-05 10:00:00");

        let deleted = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted, 5);

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM memory_event_summaries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 5); // one summary per month

        // Verify each month has total_mutations = 1
        let mut stmt = db
            .conn
            .prepare("SELECT year, month, total_mutations FROM memory_event_summaries ORDER BY year, month")
            .unwrap();
        let rows: Vec<(i64, i64, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 5);
        for (_, _, total) in &rows {
            assert_eq!(*total, 1);
        }
    }

    #[test]
    fn test_compact_upserts_existing_summary() {
        let db = test_db();
        // Insert events for January 2020
        insert_event_at(&db, "store_fact", "person:Alice", "2020-01-15 10:00:00");
        insert_event_at(&db, "store_fact", "person:Bob", "2020-01-20 10:00:00");

        // First compaction
        let deleted1 = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted1, 2);

        // Manually insert more events for the same month (simulates edge case)
        insert_event_at(&db, "update_fact", "person:Alice", "2020-01-25 10:00:00");

        // Second compaction should upsert (ON CONFLICT) the existing summary
        let deleted2 = db.compact_old_memory_events(1).unwrap();
        assert_eq!(deleted2, 1);

        // Should still have exactly 1 summary row for Jan 2020
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory_event_summaries WHERE year = 2020 AND month = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_heartbeat_sends_today_with_utc_timezone() {
        let db = test_db();

        assert_eq!(db.count_heartbeat_sends_today("UTC").unwrap(), 0);

        db.record_heartbeat_send().unwrap();
        assert_eq!(db.count_heartbeat_sends_today("UTC").unwrap(), 1);

        // Old send should not count for today
        db.conn
            .execute(
                "INSERT INTO heartbeat_sends (sent_at) VALUES (datetime('now', '-2 days'))",
                [],
            )
            .unwrap();
        assert_eq!(db.count_heartbeat_sends_today("UTC").unwrap(), 1);
    }

    #[test]
    fn test_heartbeat_sends_today_invalid_timezone_falls_back_to_utc() {
        let db = test_db();
        db.record_heartbeat_send().unwrap();
        // Invalid timezone should fall back to UTC and still work
        let count = db.count_heartbeat_sends_today("Invalid/Timezone").unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_v7_index_exists() {
        let db = test_db();
        // Verify the v7 index exists
        let index_exists: bool = db
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_memory_events_created_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            index_exists,
            "idx_memory_events_created_at should exist after v7 migration"
        );
    }

    #[test]
    fn test_v8_search_tables_exist() {
        let db = test_db();

        // search_content table exists
        let sc_exists: bool = db
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='search_content'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            sc_exists,
            "search_content table should exist after v8 migration"
        );

        // fts_search virtual table exists
        let fts_exists: bool = db
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='fts_search'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            fts_exists,
            "fts_search table should exist after v8 migration"
        );

        // vec_search virtual table exists
        let vec_exists: bool = db
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='vec_search'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            vec_exists,
            "vec_search table should exist after v8 migration"
        );

        // Verify vec_search works with a simple insert + query
        db.conn
            .execute(
                "INSERT INTO search_content (source_type, content) VALUES ('test', 'hello world')",
                [],
            )
            .unwrap();
        let content_id: i64 = db.conn.last_insert_rowid();

        // Insert a dummy 512-dim vector
        let zeros = vec![0.0f32; 512];
        let bytes: &[u8] = zerocopy::AsBytes::as_bytes(zeros.as_slice());
        db.conn
            .execute(
                "INSERT INTO vec_search (content_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![content_id, bytes],
            )
            .unwrap();

        // FTS5 insert
        db.conn
            .execute(
                "INSERT INTO fts_search (content, content_id, source_type) VALUES ('hello world', ?1, 'test')",
                rusqlite::params![content_id],
            )
            .unwrap();

        // FTS5 query
        let fts_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM fts_search WHERE fts_search MATCH 'hello'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1);
    }

    #[test]
    fn test_index_content_and_fts_search() {
        let db = test_db();

        db.index_content("person", Some(1), "Alice is a software engineer")
            .unwrap();
        db.index_content("commitment", Some(2), "Review quarterly budget report")
            .unwrap();
        db.index_content("event", Some(3), "Team dinner at Italian restaurant")
            .unwrap();

        // FTS5 search for "engineer"
        let results = db.fts_search("engineer", 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source_type, "person");
        assert!(results[0].content.contains("Alice"));

        // FTS5 search for "budget" (stemmed via porter)
        let results = db.fts_search("budget", 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source_type, "commitment");
    }

    #[test]
    fn test_index_content_with_embedding_and_vec_search() {
        let db = test_db();

        let cid = db
            .index_content("person", Some(1), "Alice is a software engineer")
            .unwrap();

        // Create a simple embedding (512 dims)
        let embedding: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        db.index_embedding(cid, &embedding).unwrap();

        // Vector search with similar embedding
        let query_emb: Vec<f32> = (0..512).map(|i| (i as f32 + 0.5) / 512.0).collect();
        let results = db.vec_search(&query_emb, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, cid);
    }

    #[test]
    fn test_hybrid_search_fts_only() {
        let db = test_db();

        db.index_content("person", Some(1), "Alice is a software engineer")
            .unwrap();
        db.index_content("commitment", Some(2), "Review quarterly budget report")
            .unwrap();

        // Hybrid search without embedding (FTS5-only path)
        let results = db.hybrid_search("engineer", None, 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Alice"));
    }

    #[test]
    fn test_hybrid_search_with_vectors() {
        let db = test_db();

        // Index two items with embeddings
        let cid1 = db
            .index_content("person", Some(1), "Alice is a software engineer")
            .unwrap();
        let emb1: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        db.index_embedding(cid1, &emb1).unwrap();

        let cid2 = db
            .index_content("commitment", Some(2), "budget report review")
            .unwrap();
        let emb2: Vec<f32> = (0..512).map(|i| (512 - i) as f32 / 512.0).collect();
        db.index_embedding(cid2, &emb2).unwrap();

        // Hybrid search: "engineer" should rank cid1 highest via FTS5 + vector
        let query_emb: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        let results = db
            .hybrid_search("engineer", Some(&query_emb), 10, None)
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].source_type, "person");
    }

    #[test]
    fn test_delete_search_content() {
        let db = test_db();

        let cid = db
            .index_content("person", Some(1), "Alice is a software engineer")
            .unwrap();
        let emb: Vec<f32> = vec![0.0; 512];
        db.index_embedding(cid, &emb).unwrap();

        // Verify content exists
        assert_eq!(db.count_search_content().unwrap(), 1);
        assert_eq!(db.fts_search("engineer", 10, None).unwrap().len(), 1);

        // Delete
        db.delete_search_content("person", 1).unwrap();

        // Verify everything is gone
        assert_eq!(db.count_search_content().unwrap(), 0);
        assert_eq!(db.fts_search("engineer", 10, None).unwrap().len(), 0);
        assert_eq!(db.vec_search(&emb, 10).unwrap().len(), 0);
    }

    #[test]
    fn test_get_all_facts_for_indexing() {
        let db = test_db();

        // Add a person, commitment, preference, event
        db.conn.execute(
            "INSERT INTO people (canonical_name, relationship, notes) VALUES ('Alice', 'colleague', 'works in engineering')",
            [],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO commitments (description, due_date, status) VALUES ('Review budget', '2026-03-01', 'pending')",
            [],
        ).unwrap();
        db.conn
            .execute(
                "INSERT INTO preferences (category, value) VALUES ('coffee', 'oat milk latte')",
                [],
            )
            .unwrap();
        db.conn.execute(
            "INSERT INTO events (description, event_date, context) VALUES ('Team dinner', '2026-02-20', 'Italian restaurant')",
            [],
        ).unwrap();

        let facts = db.get_all_facts_for_indexing().unwrap();
        assert_eq!(facts.len(), 4);

        // Check person content
        let person = facts.iter().find(|(t, _, _)| t == "person").unwrap();
        assert!(person.2.contains("Alice"));
        assert!(person.2.contains("colleague"));
        assert!(person.2.contains("engineering"));

        // Check commitment content
        let commit = facts.iter().find(|(t, _, _)| t == "commitment").unwrap();
        assert!(commit.2.contains("Review budget"));

        // Check preference content
        let pref = facts.iter().find(|(t, _, _)| t == "preference").unwrap();
        assert!(pref.2.contains("coffee"));
        assert!(pref.2.contains("oat milk latte"));

        // Check event content
        let event = facts.iter().find(|(t, _, _)| t == "event").unwrap();
        assert!(event.2.contains("Team dinner"));
        assert!(event.2.contains("Italian restaurant"));
    }

    // -- Cross-channel query tests --

    #[test]
    fn test_load_messages_after_basic() {
        let db = test_db();
        let id1 = db.save_message("user", "msg1", "telegram").unwrap();
        let id2 = db.save_message("assistant", "msg2", "telegram").unwrap();
        let _id3 = db.save_message("user", "msg3", "cli").unwrap();

        // Load after id1 — should get msg2 and msg3
        let msgs = db.load_messages_after(id1, None).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, id2);
        assert_eq!(msgs[0].content, "msg2");

        // Load after id1 with telegram filter — should get only msg2
        let msgs = db.load_messages_after(id1, Some(&["telegram"])).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "msg2");

        // Load after id2 with telegram filter — should be empty
        let msgs = db.load_messages_after(id2, Some(&["telegram"])).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_load_messages_after_excludes_summary() {
        let db = test_db();
        db.save_message("user", "msg1", "cli").unwrap();
        db.conn
            .execute(
                "INSERT INTO conversations (role, content, channel_type) VALUES ('summary', 'sum', 'system')",
                [],
            )
            .unwrap();
        db.save_message("user", "msg2", "telegram").unwrap();

        let msgs = db.load_messages_after(0, None).unwrap();
        assert!(msgs.iter().all(|m| m.role != "summary"));
    }

    #[test]
    fn test_load_messages_after_ascending_order() {
        let db = test_db();
        let id1 = db.save_message("user", "first", "telegram").unwrap();
        let id2 = db.save_message("user", "second", "telegram").unwrap();
        let id3 = db.save_message("user", "third", "telegram").unwrap();

        let msgs = db.load_messages_after(0, None).unwrap();
        assert_eq!(msgs.len(), 3);
        assert!(msgs[0].id < msgs[1].id);
        assert!(msgs[1].id < msgs[2].id);
        assert_eq!(msgs[0].id, id1);
        assert_eq!(msgs[1].id, id2);
        assert_eq!(msgs[2].id, id3);
    }

    #[test]
    fn test_max_message_id_empty() {
        let db = test_db();
        assert_eq!(db.max_message_id().unwrap(), 0);
    }

    #[test]
    fn test_max_message_id_populated() {
        let db = test_db();
        db.save_message("user", "msg1", "cli").unwrap();
        let id2 = db.save_message("user", "msg2", "telegram").unwrap();
        assert_eq!(db.max_message_id().unwrap(), id2);
    }

    #[test]
    fn test_list_customer_config_empty() {
        let db = test_db();
        let configs = db.list_customer_config().unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn test_list_customer_config_populated() {
        let db = test_db();
        db.set_customer_config("timezone", "Asia/Singapore")
            .unwrap();
        db.set_customer_config("chat_id", "12345").unwrap();

        let configs = db.list_customer_config().unwrap();
        assert_eq!(configs.len(), 2);
        // Ordered by key
        assert_eq!(configs[0], ("chat_id".to_string(), "12345".to_string()));
        assert_eq!(
            configs[1],
            ("timezone".to_string(), "Asia/Singapore".to_string())
        );
    }
}
