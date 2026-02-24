use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;
use tracing::{debug, info};

const CURRENT_SCHEMA_VERSION: i64 = 4;

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

    pub fn conn(&self) -> &Connection {
        &self.conn
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

        info!(version = CURRENT_SCHEMA_VERSION, "database migrated");
        Ok(())
    }

    fn migrate_v4(&self) -> Result<()> {
        info!("applying migration v4: plaintext schema (encryption removed)");

        // Drop all existing tables if they exist (pre-production, no data to preserve)
        self.conn
            .execute_batch(
                "
            DROP TABLE IF EXISTS memory_events;
            DROP TABLE IF EXISTS events;
            DROP TABLE IF EXISTS preferences;
            DROP TABLE IF EXISTS commitments;
            DROP TABLE IF EXISTS people;
            DROP TABLE IF EXISTS core_memory;
            DROP TABLE IF EXISTS conversations;
            DROP TABLE IF EXISTS schema_version;
            ",
            )
            .context("failed to drop old tables")?;

        self.conn
            .execute_batch(
                "
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
            CREATE INDEX idx_conv_created ON conversations(created_at);

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
            ",
            )
            .context("failed to apply migration v4")?;

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
    pub fn load_recent_messages(&self, limit: usize) -> Result<Vec<ConversationMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, channel_type, created_at
             FROM conversations ORDER BY id DESC LIMIT ?1",
        )?;

        let mut messages: Vec<ConversationMessage> = stmt
            .query_map([limit as i64], |row| {
                Ok(ConversationMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    channel_type: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read conversation row");
                    None
                }
            })
            .collect();

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
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read core_memory row");
                    None
                }
            })
            .collect();

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
    /// If user_md_content is provided, use it for user_summary.
    pub fn seed_core_memory(&self, user_md_content: Option<&str>) -> Result<()> {
        let user_summary = user_md_content.unwrap_or("New user. No information yet.");
        self.set_core_memory("user_summary", user_summary)?;
        self.set_core_memory("persona", "Mika -- personal AI executive assistant.")?;
        self.set_core_memory(
            "current_priorities",
            "Get to know the user and understand their needs.",
        )?;
        self.set_core_memory("key_people", "No one tracked yet.")?;
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
            "SELECT id FROM people WHERE canonical_name = ?1 COLLATE NOCASE",
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
                 FROM people WHERE canonical_name = ?1 COLLATE NOCASE",
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
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read people row");
                    None
                }
            })
            .collect();

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
            "SELECT id FROM commitments WHERE description = ?1 COLLATE NOCASE",
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
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read commitment row");
                    None
                }
            })
            .collect();

        Ok(commitments)
    }

    /// Update commitment status.
    pub fn update_commitment_status(&self, id: i64, status: &str) -> Result<()> {
        const VALID_STATUSES: &[&str] = &["pending", "completed", "cancelled"];
        if !VALID_STATUSES.contains(&status) {
            anyhow::bail!("invalid commitment status: {status}");
        }

        if status == "completed" {
            self.conn.execute(
                "UPDATE commitments SET status = ?1, completed_at = datetime('now') WHERE id = ?2",
                rusqlite::params![status, id],
            )?;
        } else {
            self.conn.execute(
                "UPDATE commitments SET status = ?1, completed_at = NULL WHERE id = ?2",
                rusqlite::params![status, id],
            )?;
        }
        Ok(())
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
                "SELECT value FROM preferences WHERE category = ?1 COLLATE NOCASE",
                [category],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
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
            .filter_map(|r| match r {
                Ok(row) => Some(row),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read memory_event row");
                    None
                }
            })
            .collect();

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
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn test_migration_creates_tables() {
        let db = test_db();
        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 4);
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
        assert_eq!(version, 4);
    }

    #[test]
    fn test_conversation_roundtrip() {
        let db = test_db();
        db.save_message("user", "Hello Mika!", "telegram").unwrap();
        db.save_message("assistant", "Hi! How can I help?", "telegram")
            .unwrap();

        let messages = db.load_recent_messages(10).unwrap();
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
        let messages = db.load_recent_messages(5).unwrap();
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
}
