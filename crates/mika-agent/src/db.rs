use anyhow::{Context, Result};
use mika_common::crypto::EncryptionKey;
use rusqlite::Connection;
use std::path::Path;
use tracing::{debug, info};

const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Per-customer SQLite database.
/// All sensitive fields are encrypted with AES-256-GCM at the application level.
pub struct Database {
    conn: Connection,
    key: EncryptionKey,
}

impl Database {
    /// Open (or create) a per-customer SQLite database at `path`.
    /// Runs migrations automatically.
    pub fn open(path: &Path, key: EncryptionKey) -> Result<Self> {
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

        let db = Self { conn, key };
        db.migrate()?;
        Ok(db)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory(key: EncryptionKey) -> Result<Self> {
        let conn = Connection::open_in_memory().context("failed to open in-memory SQLite")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .context("failed to set pragmas")?;
        let db = Self { conn, key };
        db.migrate()?;
        Ok(db)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn key(&self) -> &EncryptionKey {
        &self.key
    }

    /// Verify the encryption key works by encrypting and decrypting a test value.
    /// Call this on startup to fail fast if the key is wrong.
    pub fn check_encryption_key(&self) -> Result<()> {
        let test = "mika-key-check";
        let encrypted = self.key.encrypt_string(test)?;
        let decrypted = self.key.decrypt_string(&encrypted)?;
        if decrypted != test {
            anyhow::bail!("encryption key check failed: roundtrip mismatch");
        }
        Ok(())
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
        debug!(current_version = version, target_version = CURRENT_SCHEMA_VERSION, "checking migrations");

        if version >= CURRENT_SCHEMA_VERSION {
            return Ok(());
        }

        if version < 1 {
            self.migrate_v1()?;
        }

        info!(version = CURRENT_SCHEMA_VERSION, "database migrated");
        Ok(())
    }

    fn migrate_v1(&self) -> Result<()> {
        info!("applying migration v1: initial schema");

        self.conn
            .execute_batch(
                "
            -- Schema version tracking
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Conversations (message history)
            CREATE TABLE IF NOT EXISTS conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content_encrypted BLOB NOT NULL,
                channel_type TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_conv_created ON conversations(created_at);

            -- Core memory (Layer 1)
            CREATE TABLE IF NOT EXISTS core_memory (
                key TEXT PRIMARY KEY,
                value_encrypted BLOB NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- People (Layer 2)
            CREATE TABLE IF NOT EXISTS people (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                canonical_name_encrypted BLOB NOT NULL,
                canonical_name_hash TEXT NOT NULL UNIQUE,
                relationship_encrypted BLOB,
                notes_encrypted BLOB,
                first_mentioned TEXT NOT NULL DEFAULT (datetime('now')),
                last_mentioned TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Commitments (Layer 2)
            CREATE TABLE IF NOT EXISTS commitments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                description_encrypted BLOB NOT NULL,
                description_hash TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL DEFAULT 'pending',
                due_date TEXT,
                person_id INTEGER REFERENCES people(id),
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_commit_status ON commitments(status);
            CREATE INDEX IF NOT EXISTS idx_commit_due ON commitments(due_date);

            -- Preferences (Layer 2)
            CREATE TABLE IF NOT EXISTS preferences (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                category_encrypted BLOB NOT NULL,
                category_hash TEXT NOT NULL UNIQUE,
                value_encrypted BLOB NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            -- Events (Layer 2)
            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                description_encrypted BLOB NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_events_date ON events(event_date);

            -- Record version
            INSERT INTO schema_version (version) VALUES (1);
            ",
            )
            .context("failed to apply migration v1")?;

        Ok(())
    }

    /// Compute HMAC-SHA256 hash for deterministic lookups.
    fn hmac_hash(&self, input: &str) -> String {
        mika_common::crypto::hmac_sha256_hex(self.key.key_bytes(), input)
    }

    // -- Conversations --

    /// Save a conversation message (encrypted).
    pub fn save_message(&self, role: &str, content: &str, channel_type: &str) -> Result<i64> {
        let encrypted = self.key.encrypt_string(content)?;
        self.conn
            .execute(
                "INSERT INTO conversations (role, content_encrypted, channel_type) VALUES (?1, ?2, ?3)",
                rusqlite::params![role, encrypted, channel_type],
            )
            .context("failed to insert conversation")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Load the N most recent messages (decrypted), oldest first.
    pub fn load_recent_messages(&self, limit: usize) -> Result<Vec<ConversationMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content_encrypted, channel_type, created_at
             FROM conversations ORDER BY id DESC LIMIT ?1",
        )?;

        let mut messages: Vec<ConversationMessage> = stmt
            .query_map([limit as i64], |row| {
                Ok(RawConversationRow {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content_encrypted: row.get(2)?,
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
            .filter_map(|raw| {
                match self.key.decrypt_string(&raw.content_encrypted) {
                    Ok(content) => Some(ConversationMessage {
                        id: raw.id,
                        role: raw.role,
                        content,
                        channel_type: raw.channel_type,
                        created_at: raw.created_at,
                    }),
                    Err(e) => {
                        tracing::warn!(row_id = raw.id, error = %e, "decryption failed for conversation row, skipping");
                        None
                    }
                }
            })
            .collect();

        // Reverse to oldest-first order
        messages.reverse();
        Ok(messages)
    }

    // -- Core Memory --

    /// Get a core memory value by key (decrypted).
    pub fn get_core_memory(&self, key: &str) -> Result<Option<CoreMemoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, value_encrypted, token_count, updated_at FROM core_memory WHERE key = ?1",
        )?;

        let entry = stmt
            .query_row([key], |row| {
                Ok(RawCoreMemoryRow {
                    key: row.get(0)?,
                    value_encrypted: row.get(1)?,
                    token_count: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .optional()?;

        match entry {
            Some(raw) => {
                let value = self.key.decrypt_string(&raw.value_encrypted)?;
                Ok(Some(CoreMemoryEntry {
                    key: raw.key,
                    value,
                    token_count: raw.token_count,
                    updated_at: raw.updated_at,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get all core memory entries (decrypted).
    pub fn get_all_core_memory(&self) -> Result<Vec<CoreMemoryEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value_encrypted, token_count, updated_at FROM core_memory ORDER BY key")?;

        let entries: Vec<CoreMemoryEntry> = stmt
            .query_map([], |row| {
                Ok(RawCoreMemoryRow {
                    key: row.get(0)?,
                    value_encrypted: row.get(1)?,
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
            .filter_map(|raw| {
                match self.key.decrypt_string(&raw.value_encrypted) {
                    Ok(value) => Some(CoreMemoryEntry {
                        key: raw.key,
                        value,
                        token_count: raw.token_count,
                        updated_at: raw.updated_at,
                    }),
                    Err(e) => {
                        tracing::warn!(key = raw.key, error = %e, "decryption failed for core_memory row, skipping");
                        None
                    }
                }
            })
            .collect();

        Ok(entries)
    }

    /// Upsert a core memory entry (encrypts value).
    /// Returns token count. Approximate: chars / 4.
    pub fn set_core_memory(&self, key: &str, value: &str) -> Result<i32> {
        let token_count = (value.len() / 4) as i32;
        let encrypted = self.key.encrypt_string(value)?;

        self.conn
            .execute(
                "INSERT INTO core_memory (key, value_encrypted, token_count, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET
                    value_encrypted = excluded.value_encrypted,
                    token_count = excluded.token_count,
                    updated_at = excluded.updated_at",
                rusqlite::params![key, encrypted, token_count],
            )
            .context("failed to upsert core memory")?;

        Ok(token_count)
    }

    /// Total token count across all core memory entries.
    pub fn total_core_memory_tokens(&self) -> Result<i32> {
        let total: i32 = self
            .conn
            .query_row("SELECT COALESCE(SUM(token_count), 0) FROM core_memory", [], |row| {
                row.get(0)
            })?;
        Ok(total)
    }

    /// Seed default core memory for a new customer.
    pub fn seed_core_memory(&self) -> Result<()> {
        self.set_core_memory("user_summary", "New user. No information yet.")?;
        self.set_core_memory("persona", "Mika -- personal AI executive assistant.")?;
        self.set_core_memory("current_goals", "Get to know the user and understand their needs.")?;
        Ok(())
    }

    // -- People (Layer 2) --

    /// Insert or update a person. Returns their ID.
    pub fn upsert_person(&self, name: &str, relationship: Option<&str>, notes: Option<&str>) -> Result<i64> {
        let name_encrypted = self.key.encrypt_string(name)?;
        let name_hash = self.hmac_hash(name);
        let relationship_encrypted = relationship
            .map(|r| self.key.encrypt_string(r))
            .transpose()?;
        let notes_encrypted = notes
            .map(|n| self.key.encrypt_string(n))
            .transpose()?;

        self.conn
            .execute(
                "INSERT INTO people (canonical_name_encrypted, canonical_name_hash, relationship_encrypted, notes_encrypted, last_mentioned)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))
                 ON CONFLICT(canonical_name_hash) DO UPDATE SET
                    canonical_name_encrypted = excluded.canonical_name_encrypted,
                    relationship_encrypted = COALESCE(excluded.relationship_encrypted, people.relationship_encrypted),
                    notes_encrypted = COALESCE(excluded.notes_encrypted, people.notes_encrypted),
                    last_mentioned = excluded.last_mentioned",
                rusqlite::params![name_encrypted, name_hash, relationship_encrypted, notes_encrypted],
            )
            .context("failed to upsert person")?;

        let id = self.conn.query_row(
            "SELECT id FROM people WHERE canonical_name_hash = ?1",
            [&name_hash],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Get a person by name (decrypted).
    pub fn get_person(&self, name: &str) -> Result<Option<Person>> {
        let name_hash = self.hmac_hash(name);
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name_encrypted, relationship_encrypted, notes_encrypted, first_mentioned, last_mentioned
             FROM people WHERE canonical_name_hash = ?1",
        )?;

        let row = stmt
            .query_row([&name_hash], |row| {
                Ok(RawPersonRow {
                    id: row.get(0)?,
                    canonical_name_encrypted: row.get(1)?,
                    relationship_encrypted: row.get(2)?,
                    notes_encrypted: row.get(3)?,
                    first_mentioned: row.get(4)?,
                    last_mentioned: row.get(5)?,
                })
            })
            .optional()?;

        match row {
            Some(raw) => {
                let canonical_name = self.key.decrypt_string(&raw.canonical_name_encrypted)?;
                let relationship = raw
                    .relationship_encrypted
                    .as_ref()
                    .map(|enc| self.key.decrypt_string(enc))
                    .transpose()?;
                let notes = raw
                    .notes_encrypted
                    .as_ref()
                    .map(|enc| self.key.decrypt_string(enc))
                    .transpose()?;
                Ok(Some(Person {
                    id: raw.id,
                    canonical_name,
                    relationship,
                    notes,
                    first_mentioned: raw.first_mentioned,
                    last_mentioned: raw.last_mentioned,
                }))
            }
            None => Ok(None),
        }
    }

    /// List all people (decrypted).
    pub fn list_people(&self) -> Result<Vec<Person>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, canonical_name_encrypted, relationship_encrypted, notes_encrypted, first_mentioned, last_mentioned
             FROM people ORDER BY last_mentioned DESC",
        )?;

        let people: Vec<Person> = stmt
            .query_map([], |row| {
                Ok(RawPersonRow {
                    id: row.get(0)?,
                    canonical_name_encrypted: row.get(1)?,
                    relationship_encrypted: row.get(2)?,
                    notes_encrypted: row.get(3)?,
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
            .filter_map(|raw| {
                let canonical_name = match self.key.decrypt_string(&raw.canonical_name_encrypted) {
                    Ok(name) => name,
                    Err(e) => {
                        tracing::warn!(row_id = raw.id, error = %e, "decryption failed for person canonical_name, skipping");
                        return None;
                    }
                };
                let relationship = match raw.relationship_encrypted.as_ref() {
                    Some(enc) => match self.key.decrypt_string(enc) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            tracing::warn!(row_id = raw.id, error = %e, "decryption failed for person relationship, skipping");
                            return None;
                        }
                    },
                    None => None,
                };
                let notes = match raw.notes_encrypted.as_ref() {
                    Some(enc) => match self.key.decrypt_string(enc) {
                        Ok(n) => Some(n),
                        Err(e) => {
                            tracing::warn!(row_id = raw.id, error = %e, "decryption failed for person notes, skipping");
                            return None;
                        }
                    },
                    None => None,
                };
                Some(Person {
                    id: raw.id,
                    canonical_name,
                    relationship,
                    notes,
                    first_mentioned: raw.first_mentioned,
                    last_mentioned: raw.last_mentioned,
                })
            })
            .collect();

        Ok(people)
    }

    // -- Commitments (Layer 2) --

    /// Add a commitment (encrypted). Uses HMAC-SHA256 for dedup.
    pub fn add_commitment(
        &self,
        description: &str,
        due_date: Option<&str>,
        person_id: Option<i64>,
    ) -> Result<i64> {
        let hash = self.hmac_hash(description);
        let encrypted = self.key.encrypt_string(description)?;

        self.conn
            .execute(
                "INSERT OR IGNORE INTO commitments (description_encrypted, description_hash, due_date, person_id)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![encrypted, hash, due_date, person_id],
            )
            .context("failed to insert commitment")?;

        let id = self.conn.query_row(
            "SELECT id FROM commitments WHERE description_hash = ?1",
            [&hash],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// List commitments by status (decrypted).
    pub fn list_commitments(&self, status: &str) -> Result<Vec<Commitment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description_encrypted, status, due_date, person_id, created_at, completed_at
             FROM commitments WHERE status = ?1 ORDER BY due_date ASC NULLS LAST",
        )?;

        let commitments: Vec<Commitment> = stmt
            .query_map([status], |row| {
                Ok(RawCommitmentRow {
                    id: row.get(0)?,
                    description_encrypted: row.get(1)?,
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
            .filter_map(|raw| {
                match self.key.decrypt_string(&raw.description_encrypted) {
                    Ok(description) => Some(Commitment {
                        id: raw.id,
                        description,
                        status: raw.status,
                        due_date: raw.due_date,
                        person_id: raw.person_id,
                        created_at: raw.created_at,
                        completed_at: raw.completed_at,
                    }),
                    Err(e) => {
                        tracing::warn!(row_id = raw.id, error = %e, "decryption failed for commitment, skipping");
                        None
                    }
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

    /// Upsert a preference (encrypted).
    pub fn set_preference(&self, category: &str, value: &str) -> Result<()> {
        let category_encrypted = self.key.encrypt_string(category)?;
        let category_hash = self.hmac_hash(category);
        let value_encrypted = self.key.encrypt_string(value)?;
        self.conn
            .execute(
                "INSERT INTO preferences (category_encrypted, category_hash, value_encrypted, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(category_hash) DO UPDATE SET
                    category_encrypted = excluded.category_encrypted,
                    value_encrypted = excluded.value_encrypted,
                    updated_at = excluded.updated_at",
                rusqlite::params![category_encrypted, category_hash, value_encrypted],
            )
            .context("failed to upsert preference")?;
        Ok(())
    }

    /// Get a preference by category (decrypted).
    pub fn get_preference(&self, category: &str) -> Result<Option<String>> {
        let category_hash = self.hmac_hash(category);
        let encrypted: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT value_encrypted FROM preferences WHERE category_hash = ?1",
                [&category_hash],
                |row| row.get(0),
            )
            .optional()?;

        match encrypted {
            Some(enc) => Ok(Some(self.key.decrypt_string(&enc)?)),
            None => Ok(None),
        }
    }

    // -- Events (Layer 2) --

    /// Add an event (encrypted description).
    pub fn add_event(&self, description: &str, event_date: Option<&str>, context: Option<&str>) -> Result<i64> {
        let encrypted = self.key.encrypt_string(description)?;
        self.conn
            .execute(
                "INSERT INTO events (description_encrypted, event_date, context) VALUES (?1, ?2, ?3)",
                rusqlite::params![encrypted, event_date, context],
            )
            .context("failed to insert event")?;
        Ok(self.conn.last_insert_rowid())
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

// -- Internal raw row types (before decryption) --

struct RawConversationRow {
    id: i64,
    role: String,
    content_encrypted: Vec<u8>,
    channel_type: String,
    created_at: String,
}

struct RawCoreMemoryRow {
    key: String,
    value_encrypted: Vec<u8>,
    token_count: i32,
    updated_at: String,
}

struct RawPersonRow {
    id: i64,
    canonical_name_encrypted: Vec<u8>,
    relationship_encrypted: Option<Vec<u8>>,
    notes_encrypted: Option<Vec<u8>>,
    first_mentioned: String,
    last_mentioned: String,
}

struct RawCommitmentRow {
    id: i64,
    description_encrypted: Vec<u8>,
    status: String,
    due_date: Option<String>,
    person_id: Option<i64>,
    created_at: String,
    completed_at: Option<String>,
}

// Bring in rusqlite optional extension
use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EncryptionKey {
        EncryptionKey::from_hex(&"01".repeat(32)).unwrap()
    }

    fn test_db() -> Database {
        Database::open_in_memory(test_key()).unwrap()
    }

    #[test]
    fn test_migration_creates_tables() {
        let db = test_db();
        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn test_migration_idempotent() {
        let db = test_db();
        // Running migrate again should be a no-op
        db.migrate().unwrap();
        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn test_conversation_roundtrip() {
        let db = test_db();
        db.save_message("user", "Hello Mika!", "telegram").unwrap();
        db.save_message("assistant", "Hi! How can I help?", "telegram").unwrap();

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
            db.save_message("user", &format!("Message {i}"), "telegram").unwrap();
        }
        let messages = db.load_recent_messages(5).unwrap();
        assert_eq!(messages.len(), 5);
        // Should be the 5 most recent, oldest first
        assert_eq!(messages[0].content, "Message 15");
        assert_eq!(messages[4].content, "Message 19");
    }

    #[test]
    fn test_core_memory_crud() {
        let db = test_db();

        // Set
        let tokens = db.set_core_memory("user_summary", "Loves coffee.").unwrap();
        assert!(tokens > 0);

        // Get
        let entry = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(entry.value, "Loves coffee.");
        assert_eq!(entry.key, "user_summary");

        // Update
        db.set_core_memory("user_summary", "Loves coffee and tea.").unwrap();
        let entry = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(entry.value, "Loves coffee and tea.");

        // Missing key
        assert!(db.get_core_memory("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_core_memory_seed() {
        let db = test_db();
        db.seed_core_memory().unwrap();

        let all = db.get_all_core_memory().unwrap();
        assert_eq!(all.len(), 3);

        let summary = db.get_core_memory("user_summary").unwrap().unwrap();
        assert_eq!(summary.value, "New user. No information yet.");
    }

    #[test]
    fn test_total_token_count() {
        let db = test_db();
        db.set_core_memory("a", &"x".repeat(100)).unwrap(); // ~25 tokens
        db.set_core_memory("b", &"y".repeat(200)).unwrap(); // ~50 tokens
        let total = db.total_core_memory_tokens().unwrap();
        assert_eq!(total, 75);
    }

    #[test]
    fn test_people_crud() {
        let db = test_db();

        let id = db.upsert_person("Sarah Chen", Some("colleague"), Some("VP of Engineering")).unwrap();
        assert!(id > 0);

        let person = db.get_person("Sarah Chen").unwrap().unwrap();
        assert_eq!(person.canonical_name, "Sarah Chen");
        assert_eq!(person.relationship, Some("colleague".to_string()));
        assert_eq!(person.notes, Some("VP of Engineering".to_string()));

        // Upsert updates relationship
        db.upsert_person("Sarah Chen", Some("manager"), None).unwrap();
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
    fn test_commitments() {
        let db = test_db();

        let id = db.add_commitment("Review Q4 budget", Some("2026-03-01"), None).unwrap();
        assert!(id > 0);

        // Duplicate should be ignored (same hash)
        db.add_commitment("Review Q4 budget", Some("2026-03-01"), None).unwrap();

        let pending = db.list_commitments("pending").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].description, "Review Q4 budget");

        // Complete it
        db.update_commitment_status(id, "completed").unwrap();
        let pending = db.list_commitments("pending").unwrap();
        assert_eq!(pending.len(), 0);
        let completed = db.list_commitments("completed").unwrap();
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn test_preferences() {
        let db = test_db();

        db.set_preference("communication_style", "Direct and concise").unwrap();
        let pref = db.get_preference("communication_style").unwrap().unwrap();
        assert_eq!(pref, "Direct and concise");

        // Update
        db.set_preference("communication_style", "Friendly and warm").unwrap();
        let pref = db.get_preference("communication_style").unwrap().unwrap();
        assert_eq!(pref, "Friendly and warm");

        // Missing
        assert!(db.get_preference("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_events() {
        let db = test_db();

        let id = db.add_event("Board meeting", Some("2026-03-15"), Some("business")).unwrap();
        assert!(id > 0);
    }

    #[test]
    fn test_hmac_sha256_hex() {
        let key_bytes = [0x01u8; 32];
        let hash = mika_common::crypto::hmac_sha256_hex(&key_bytes, "hello");
        // HMAC-SHA256 produces 64 hex chars (32 bytes)
        assert_eq!(hash.len(), 64);
        // Should be deterministic
        let hash2 = mika_common::crypto::hmac_sha256_hex(&key_bytes, "hello");
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_data_encrypted_at_rest() {
        let db = test_db();
        db.save_message("user", "Secret meeting at noon", "telegram").unwrap();

        // Read raw encrypted blob -- should NOT contain plaintext
        let raw: Vec<u8> = db
            .conn
            .query_row(
                "SELECT content_encrypted FROM conversations WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let raw_str = String::from_utf8_lossy(&raw);
        assert!(!raw_str.contains("Secret meeting"));
    }

    #[test]
    fn test_check_encryption_key() {
        let db = test_db();
        // Should succeed with a valid key
        db.check_encryption_key().unwrap();
    }

    #[test]
    fn test_people_encrypted_at_rest() {
        let db = test_db();
        db.upsert_person("Sarah Chen", Some("colleague"), Some("VP of Engineering")).unwrap();

        // Read raw canonical_name_encrypted -- should NOT contain plaintext
        let raw_name: Vec<u8> = db
            .conn
            .query_row(
                "SELECT canonical_name_encrypted FROM people WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let raw_name_str = String::from_utf8_lossy(&raw_name);
        assert!(!raw_name_str.contains("Sarah Chen"), "canonical_name should be encrypted at rest");

        // Read raw relationship_encrypted -- should NOT contain plaintext
        let raw_rel: Vec<u8> = db
            .conn
            .query_row(
                "SELECT relationship_encrypted FROM people WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let raw_rel_str = String::from_utf8_lossy(&raw_rel);
        assert!(!raw_rel_str.contains("colleague"), "relationship should be encrypted at rest");
    }

    #[test]
    fn test_preferences_encrypted_at_rest() {
        let db = test_db();
        db.set_preference("communication_style", "Direct and concise").unwrap();

        // Read raw category_encrypted -- should NOT contain plaintext
        let raw_cat: Vec<u8> = db
            .conn
            .query_row(
                "SELECT category_encrypted FROM preferences WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let raw_cat_str = String::from_utf8_lossy(&raw_cat);
        assert!(!raw_cat_str.contains("communication_style"), "category should be encrypted at rest");
    }
}
