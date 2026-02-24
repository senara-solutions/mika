use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::{debug, info};

const CURRENT_SCHEMA_VERSION: i64 = 5;

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
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
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

    // -- Preferences (Layer 2) --

    /// Upsert a preference (case-insensitive category).
    pub fn set_preference(&self, category: &str, value: &str) -> Result<()> {
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
        Ok(())
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

    /// Query reminders with an additional WHERE clause appended after `status = 'pending'`.
    fn query_reminders(&self, extra_where: &str) -> Result<Vec<Reminder>> {
        let sql = format!(
            "SELECT id, fire_at, message, status, created_at, delivered_at \
             FROM reminders WHERE status = 'pending'{} ORDER BY fire_at ASC",
            extra_where
        );
        let mut stmt = self.conn.prepare(&sql)?;
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
        self.query_reminders("")
    }

    /// Get pending reminders whose fire_at is in the future.
    pub fn get_future_reminders(&self) -> Result<Vec<Reminder>> {
        self.query_reminders(" AND fire_at > datetime('now')")
    }

    /// Get pending reminders whose fire_at is at or past now (ready to deliver).
    pub fn get_past_due_reminders(&self) -> Result<Vec<Reminder>> {
        self.query_reminders(" AND fire_at <= datetime('now')")
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

    // -- Heartbeat Rate Limiting --

    /// Record a heartbeat send for rate limiting.
    pub fn record_heartbeat_send(&self) -> Result<()> {
        self.conn
            .execute("INSERT INTO heartbeat_sends DEFAULT VALUES", [])?;
        Ok(())
    }

    /// Count heartbeat sends in the current day for a given timezone.
    /// Uses SQLite date functions to compute "today" in the user's timezone.
    pub fn count_heartbeat_sends_today(&self, timezone_offset: &str) -> Result<u32> {
        // timezone_offset is like "+08:00" or "-05:00"
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends
             WHERE date(sent_at, ?1) = date('now', ?1)",
            [timezone_offset],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count heartbeat sends in the last hour (UTC).
    pub fn count_heartbeat_sends_last_hour(&self) -> Result<u32> {
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM heartbeat_sends
             WHERE sent_at >= datetime('now', '-1 hour')",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Delete heartbeat sends older than `days` days.
    pub fn prune_old_heartbeat_sends(&self, days: u32) -> Result<()> {
        let modifier = format!("-{days} days");
        self.conn.execute(
            "DELETE FROM heartbeat_sends WHERE sent_at < datetime('now', ?1)",
            [&modifier],
        )?;
        Ok(())
    }

    // -- Conversation Compaction --

    /// Save a conversation summary row. Returns the new row ID.
    pub fn save_conversation_summary(
        &self,
        summary: &str,
        compacted_through_id: i64,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO conversations (role, content, channel_type, compacted_through_id)
             VALUES ('summary', ?1, 'system', ?2)",
            rusqlite::params![summary, compacted_through_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete messages up to and including `through_id`, excluding summary rows.
    /// Returns the number of deleted rows.
    pub fn delete_compacted_messages(&self, through_id: i64) -> Result<u32> {
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
        self.conn.execute("BEGIN", [])?;
        let result = (|| {
            // Delete old summary rows (we keep only the latest)
            self.conn
                .execute("DELETE FROM conversations WHERE role = 'summary'", [])?;
            self.delete_compacted_messages(compacted_through_id)?;
            self.save_conversation_summary(summary, compacted_through_id)
        })();
        match &result {
            Ok(_) => self.conn.execute("COMMIT", [])?,
            Err(_) => self.conn.execute("ROLLBACK", [])?,
        };
        result
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

    // -- Failed Sends --

    /// Record a failed outbound send for retry.
    pub fn record_failed_send(&self, text: &str, request_id: Option<&str>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO failed_sends (text, request_id) VALUES (?1, ?2)",
            rusqlite::params![text, request_id],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get all failed sends for retry.
    pub fn get_failed_sends(&self) -> Result<Vec<FailedSend>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, request_id, created_at, retry_count
             FROM failed_sends ORDER BY created_at ASC",
        )?;
        let sends = stmt
            .query_map([], |row| {
                Ok(FailedSend {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    request_id: row.get(2)?,
                    created_at: row.get(3)?,
                    retry_count: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(sends)
    }

    /// Remove a failed send after successful retry.
    pub fn delete_failed_send(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM failed_sends WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Increment the retry count for a failed send.
    pub fn increment_failed_send_retry(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE failed_sends SET retry_count = retry_count + 1 WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Get the timestamp of the last user message (for heartbeat staleness check).
    pub fn last_user_message_time(&self) -> Result<Option<String>> {
        let time = self
            .conn
            .query_row(
                "SELECT created_at FROM conversations
                 WHERE role = 'user' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(time)
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
}

// -- Public types --

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
pub struct FailedSend {
    pub id: i64,
    pub text: String,
    pub request_id: Option<String>,
    pub created_at: String,
    pub retry_count: i32,
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
        assert_eq!(version, 5);
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
        assert_eq!(version, 5);
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

    // -- v5 migration tests --

    #[test]
    fn test_v5_tables_exist() {
        let db = test_db();
        // Verify all v5 tables exist by querying sqlite_master
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

    // -- Heartbeat rate limiting tests --

    #[test]
    fn test_heartbeat_send_counting() {
        let db = test_db();

        assert_eq!(db.count_heartbeat_sends_last_hour().unwrap(), 0);

        db.record_heartbeat_send().unwrap();
        db.record_heartbeat_send().unwrap();

        assert_eq!(db.count_heartbeat_sends_last_hour().unwrap(), 2);
    }

    #[test]
    fn test_heartbeat_sends_today() {
        let db = test_db();
        db.record_heartbeat_send().unwrap();

        // UTC offset "+00:00" should see the send on "today"
        let count = db.count_heartbeat_sends_today("+00:00").unwrap();
        assert_eq!(count, 1);
    }

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
        db.record_heartbeat_send().unwrap(); // recent one

        assert_eq!(db.count_heartbeat_sends_last_hour().unwrap(), 1);

        // Prune sends older than 7 days
        db.prune_old_heartbeat_sends(7).unwrap();

        // The old one should be gone, but the count_last_hour still sees 1
        let total: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM heartbeat_sends", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total, 1);
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

    // -- Failed sends tests --

    #[test]
    fn test_failed_send_lifecycle() {
        let db = test_db();

        let id = db
            .record_failed_send("Hello user!", Some("req-123"))
            .unwrap();
        assert!(id > 0);

        let sends = db.get_failed_sends().unwrap();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].text, "Hello user!");
        assert_eq!(sends[0].request_id, Some("req-123".to_string()));
        assert_eq!(sends[0].retry_count, 0);

        // Increment retry
        db.increment_failed_send_retry(id).unwrap();
        let sends = db.get_failed_sends().unwrap();
        assert_eq!(sends[0].retry_count, 1);

        // Delete after successful retry
        db.delete_failed_send(id).unwrap();
        let sends = db.get_failed_sends().unwrap();
        assert_eq!(sends.len(), 0);
    }

    // -- Last user message time --

    #[test]
    fn test_last_user_message_time() {
        let db = test_db();

        // No messages yet
        assert!(db.last_user_message_time().unwrap().is_none());

        db.save_message("user", "hello", "cli").unwrap();
        db.save_message("assistant", "hi", "cli").unwrap();

        // Should return the user message time (not assistant)
        let time = db.last_user_message_time().unwrap().unwrap();
        assert!(!time.is_empty());
    }
}
