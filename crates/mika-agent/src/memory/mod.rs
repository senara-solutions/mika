//! Core memory, structured facts, search, and KG bridges.
//!
//! Owns Database query/write methods for:
//! - Core memory (key-value identity blocks injected per-turn)
//! - People (entity records used in conversation context)
//! - Preferences (operator-stamped routing hints)
//! - Events (factual record of state-of-world entries)
//! - Facts (semantic memory store)
//! - KG corpora bridges (integration with `crate::kg`'s ingestion + resolution)
//!
//! Per Foundation §6: reads `OperationalItem` to inform retrieval;
//! writes Evidence on persistence ops.

use anyhow::{Result, anyhow};
use chrono::Duration;
use rusqlite::OptionalExtension;
use rusqlite::params;
use sha2::{Digest, Sha256};
use tracing::warn;
use unicode_normalization::UnicodeNormalization;

use crate::db::{
    CoreMemoryEntry, DashboardFact, Database, Event, Person, Preference, RecordOutcome,
    ServedContent,
};
use crate::timestamp;

/// Categories accepted by the `served_content` ledger (mika#1867). Mirrors
/// the SQL CHECK constraint on `served_content.category`. Kept as a Rust-side
/// allowlist so the tool can reject invalid categories with a helpful error
/// before the DB round-trip and the `Duplicate` shape can be returned cleanly.
pub const SERVED_CONTENT_CATEGORIES: &[&str] = &[
    "proverb",
    "quote",
    "joke",
    "poem",
    "recommendation",
    "story",
    "fact",
];

/// Default `check_already_served` lookback window (mika#1867 A5). 90 days
/// covers seasonal-recall patterns (Al's founding incident was 6 days) without
/// unbounded history growth.
pub const SERVED_CONTENT_DEFAULT_WINDOW_DAYS: i64 = 90;

/// Compute the normalized SHA-256 hex hash used to dedup content-serve rows
/// (mika#1867 A4). Normalization defends against semantically-identical
/// content hashing differently under common Unicode + typographic variance:
///
/// 1. NFKC normalization — folds compatibility variants (fullwidth/ligatures)
///    and composes decomposed accents (NFD `e` + U+0301 → NFC `é`), so the
///    same word typed with pre-composed vs decomposed accents hashes equal.
/// 2. Zero-width character strip — U+200B/U+200C/U+200D/U+FEFF/U+2060.
///    LLM output occasionally carries these; the human never sees them but
///    the hash would.
/// 3. Typographic quote fold — U+2018/U+2019 → ASCII `'`, U+201C/U+201D →
///    ASCII `"`. Same glyph to the reader, distinct bytes.
/// 4. Trailing punctuation trim — a trailing `.`, `?`, `!`, `…`, or any ASCII
///    punctuation is elided so `"proverb X"` and `"proverb X."` collide.
/// 5. Lowercase.
/// 6. Whitespace collapse — any contiguous whitespace becomes a single space.
///
/// Interior punctuation is preserved for a fair first cut — fuzzy dedup
/// (AC6, `content_signature`) is the follow-up.
pub fn compute_content_hash(content: &str) -> String {
    // NFKC first — folds compatibility variants + composes accents.
    let nfkc: String = content.nfkc().collect();
    // Drop invisibles + normalize typographic quotes in one pass.
    let mut cleaned = String::with_capacity(nfkc.len());
    for c in nfkc.chars() {
        match c {
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{2060}' => continue,
            '\u{2018}' | '\u{2019}' => cleaned.push('\''),
            '\u{201C}' | '\u{201D}' => cleaned.push('"'),
            _ => cleaned.push(c),
        }
    }
    // Trim trailing punctuation, then lowercase.
    let trimmed = cleaned
        .trim_end_matches(|c: char| c.is_ascii_punctuation() || matches!(c, '…' | '!' | '?' | '.'))
        .to_lowercase();
    let normalized: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    let hash = Sha256::digest(normalized.as_bytes());
    format!("{hash:x}")
}

impl Database {
    // === Core memory ===

    pub fn get_core_memory(&self, agent_id: &str, key: &str) -> Result<Option<CoreMemoryEntry>> {
        self.conn
            .query_row(
                "SELECT key, value, token_count, updated_at
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
            "SELECT key, value, token_count, updated_at
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
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
            params![agent_id, key, value, token_count],
        )?;
        Ok(token_count)
    }

    // === People ===

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
                  last_mentioned = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
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
                         first_mentioned,
                         last_mentioned,
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
                     first_mentioned,
                     last_mentioned,
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

    /// Delete a person by canonical name. Unlinks commitments first (sets person_id = NULL).
    /// Returns true if a person was deleted.
    pub fn delete_person_by_name(&self, agent_id: &str, name: &str) -> Result<bool> {
        // Unlink commitments from this person
        let person_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM people WHERE agent_id = ?1 AND canonical_name = ?2",
                params![agent_id, name],
                |r| r.get(0),
            )
            .optional()?;
        let Some(pid) = person_id else {
            return Ok(false);
        };
        self.conn.execute(
            "UPDATE commitments SET person_id = NULL WHERE person_id = ?1",
            params![pid],
        )?;
        // Delete search content
        self.delete_search_content(agent_id, "person", pid)?;
        // Delete the person
        let deleted = self.conn.execute(
            "DELETE FROM people WHERE agent_id = ?1 AND canonical_name = ?2",
            params![agent_id, name],
        )?;
        Ok(deleted > 0)
    }

    // === Preferences ===

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
            "SELECT category, value, updated_at
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

    /// Delete a preference by category. Returns true if deleted.
    pub fn delete_preference(&self, agent_id: &str, category: &str) -> Result<bool> {
        // Clean up search content — preference content is "{category}: {value}".
        // source_id from set_preference (last_insert_rowid) can be unreliable for upserts,
        // so we find the search_content row by content prefix match instead.
        let content_prefix = format!("{category}: ");
        let existing: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT id, content FROM search_content
                  WHERE agent_id = ?1 AND source_type = 'preference' AND content LIKE ?2 || '%'",
                params![agent_id, content_prefix],
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
        let deleted = self.conn.execute(
            "DELETE FROM preferences WHERE agent_id = ?1 AND category = ?2",
            params![agent_id, category],
        )?;
        Ok(deleted > 0)
    }

    // === Events ===

    pub fn list_events(&self, agent_id: &str) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, description, event_date, context, created_at
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

    /// Delete an event by description. Returns true if deleted.
    pub fn delete_event_by_description(&self, agent_id: &str, description: &str) -> Result<bool> {
        let event_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM events WHERE agent_id = ?1 AND description = ?2 COLLATE NOCASE",
                params![agent_id, description],
                |r| r.get(0),
            )
            .optional()?;
        let Some(eid) = event_id else {
            return Ok(false);
        };
        self.delete_search_content(agent_id, "event", eid)?;
        let deleted = self.conn.execute(
            "DELETE FROM events WHERE id = ?1 AND agent_id = ?2",
            params![eid, agent_id],
        )?;
        Ok(deleted > 0)
    }

    // === Facts ===

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

    /// Returns paginated structured facts for the dashboard with total count.
    pub fn list_facts_paginated_with_count(
        &self,
        agent_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<DashboardFact>, u64)> {
        let count: u64 = self.conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM people WHERE agent_id = ?1) +
                (SELECT COUNT(*) FROM commitments WHERE agent_id = ?1) +
                (SELECT COUNT(*) FROM preferences WHERE agent_id = ?1) +
                (SELECT COUNT(*) FROM events WHERE agent_id = ?1)",
            params![agent_id],
            |r| r.get(0),
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT id, category, key, value, updated_at FROM (
                SELECT id, 'People' AS category, canonical_name AS key,
                    CASE WHEN relationship IS NOT NULL AND notes IS NOT NULL THEN relationship || ': ' || notes WHEN relationship IS NOT NULL THEN relationship WHEN notes IS NOT NULL THEN notes ELSE '' END AS value,
                    last_mentioned AS updated_at
                FROM people WHERE agent_id = ?1
                UNION ALL
                SELECT id, 'Commitments' AS category, description AS key,
                    '[' || status || ']' || COALESCE(' due: ' || due_date, '') AS value,
                    created_at AS updated_at
                FROM commitments WHERE agent_id = ?1
                UNION ALL
                SELECT rowid AS id, 'Preferences' AS category, category AS key,
                    value AS value, updated_at AS updated_at
                FROM preferences WHERE agent_id = ?1
                UNION ALL
                SELECT id, 'Events' AS category, description AS key,
                    COALESCE(context, '') AS value, created_at AS updated_at
                FROM events WHERE agent_id = ?1
            ) ORDER BY updated_at DESC LIMIT ?2 OFFSET ?3",
        )?;

        let data = stmt
            .query_map(params![agent_id, limit, offset], |r| {
                Ok(DashboardFact {
                    id: r.get(0)?,
                    category: r.get(1)?,
                    key: r.get(2)?,
                    value: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok((data, count))
    }

    // === Served-content ledger (mika#1867) ===

    /// Record that Mika has served a specific piece of content to a specific
    /// person in a specific category (mika#1867). Idempotent on
    /// `(agent_id, person_id, content_hash)` — a duplicate write returns
    /// [`RecordOutcome::Duplicate`] with the pre-existing row's id and
    /// `served_at`, so the LLM can regenerate with a different item per the
    /// AC5 retry-with-avoid loop.
    ///
    /// Errors on FK violation (person_id does not exist) with context naming
    /// the missing id. Emits a `warn!` on duplicate for observability.
    pub fn record_served_content(
        &mut self,
        agent_id: &str,
        person_id: i64,
        category: &str,
        content_text: &str,
        session_id: Option<&str>,
    ) -> Result<RecordOutcome> {
        let content_hash = compute_content_hash(content_text);
        let rows = match self.conn.execute(
            "INSERT INTO served_content
                (agent_id, person_id, category, content_text, content_hash, session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(agent_id, person_id, content_hash) DO NOTHING",
            params![
                agent_id,
                person_id,
                category,
                content_text,
                content_hash,
                session_id,
            ],
        ) {
            Ok(n) => n,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("FOREIGN KEY") {
                    return Err(anyhow!(
                        "person_id={person_id} not found in people table: {e}"
                    ));
                }
                return Err(e.into());
            }
        };

        if rows == 1 {
            Ok(RecordOutcome::Inserted {
                id: self.conn.last_insert_rowid(),
            })
        } else {
            // Duplicate: look up the pre-existing row.
            let (existing_id, prior_served_at): (i64, String) = self.conn.query_row(
                "SELECT id, served_at FROM served_content
                  WHERE agent_id = ?1 AND person_id = ?2 AND content_hash = ?3",
                params![agent_id, person_id, content_hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            warn!(
                target: "mika::otel",
                event = "served_content.duplicate_write",
                agent_id = agent_id,
                person_id = person_id,
                category = category,
                existing_id = existing_id,
                prior_served_at = %prior_served_at,
                "duplicate served-content write detected (mika#1867)"
            );
            Ok(RecordOutcome::Duplicate {
                existing_id,
                prior_served_at,
            })
        }
    }

    /// List served-content rows for `(agent_id, person_id, category)` filtered
    /// by `since` (ISO 8601 lower bound). When `since` is `None`, defaults to
    /// [`SERVED_CONTENT_DEFAULT_WINDOW_DAYS`] ago. Ordered by `served_at DESC`.
    ///
    /// When `content_hash` is `Some`, filters via the `idx_served_content_hash`
    /// index at the SQL layer — this avoids the /ce:review P1-7 LIMIT-then-
    /// filter false-negative where a caller with a targeted-hash query would
    /// miss a match that happened to sit older than the top-N most-recent
    /// slice.
    pub fn list_served_content(
        &self,
        agent_id: &str,
        person_id: i64,
        category: &str,
        since: Option<&str>,
        limit: usize,
        content_hash: Option<&str>,
    ) -> Result<Vec<ServedContent>> {
        let default_since;
        let since_ts = match since {
            Some(s) => s,
            None => {
                default_since =
                    timestamp::now_minus(Duration::days(SERVED_CONTENT_DEFAULT_WINDOW_DAYS));
                &default_since
            }
        };

        let rows = if let Some(h) = content_hash {
            let mut stmt = self.conn.prepare(
                "SELECT id, agent_id, person_id, category, content_text,
                         content_hash, content_signature, served_at, session_id
                  FROM served_content
                  WHERE agent_id = ?1 AND person_id = ?2 AND category = ?3
                    AND served_at >= ?4 AND content_hash = ?5
                  ORDER BY served_at DESC
                  LIMIT ?6",
            )?;
            stmt.query_map(
                params![agent_id, person_id, category, since_ts, h, limit as i64],
                |r| {
                    Ok(ServedContent {
                        id: r.get(0)?,
                        agent_id: r.get(1)?,
                        person_id: r.get(2)?,
                        category: r.get(3)?,
                        content_text: r.get(4)?,
                        content_hash: r.get(5)?,
                        content_signature: r.get(6)?,
                        served_at: r.get(7)?,
                        session_id: r.get(8)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, agent_id, person_id, category, content_text,
                         content_hash, content_signature, served_at, session_id
                  FROM served_content
                  WHERE agent_id = ?1 AND person_id = ?2 AND category = ?3
                    AND served_at >= ?4
                  ORDER BY served_at DESC
                  LIMIT ?5",
            )?;
            stmt.query_map(
                params![agent_id, person_id, category, since_ts, limit as i64],
                |r| {
                    Ok(ServedContent {
                        id: r.get(0)?,
                        agent_id: r.get(1)?,
                        person_id: r.get(2)?,
                        category: r.get(3)?,
                        content_text: r.get(4)?,
                        content_hash: r.get(5)?,
                        content_signature: r.get(6)?,
                        served_at: r.get(7)?,
                        session_id: r.get(8)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?
        };
        Ok(rows)
    }

    // === KG bridges ===

    /// List all corpora registered for an agent.
    /// Returns `(docs_root_hash, docs_root_path)` pairs.
    pub fn list_agent_corpora(&self, agent_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT docs_root_hash, docs_root_path FROM agent_kg_corpora WHERE agent_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn seed_person(db: &Database) -> i64 {
        db.upsert_person("mika", "Al", Some("tester"), None)
            .unwrap()
    }

    #[test]
    fn test_compute_content_hash_case_insensitive() {
        let a = compute_content_hash("Avant l'éveil, couper du bois.");
        let b = compute_content_hash("avant l'éveil, couper du BOIS.");
        assert_eq!(a, b);
    }

    #[test]
    fn test_compute_content_hash_whitespace_insensitive() {
        let a = compute_content_hash("Avant l'éveil, couper du bois.");
        let b = compute_content_hash("Avant l'éveil,  couper du bois.");
        let c = compute_content_hash("Avant\tl'éveil,\ncouper du bois.");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn test_compute_content_hash_punctuation_preserved() {
        // Interior punctuation still discriminates (`,` mid-phrase). Trailing
        // punctuation is elided per test_hash_trailing_punct_ignored below.
        let a = compute_content_hash("Hello world");
        let b = compute_content_hash("Hello, world");
        assert_ne!(a, b);
    }

    // -------- /ce:review P1-4 hash-normalization regressions --------

    #[test]
    fn test_hash_nfc_vs_nfd_equivalent() {
        // NFC: pre-composed `é` (U+00E9). NFD: `e` + combining acute (U+0301).
        // NFKC folds the two to the same byte sequence so they hash equal.
        let nfc = "café";
        let nfd = "cafe\u{0301}";
        assert_eq!(compute_content_hash(nfc), compute_content_hash(nfd));
    }

    #[test]
    fn test_hash_straight_vs_curly_quotes_equivalent() {
        // Typographic right single quote (U+2019) folds to ASCII apostrophe.
        // Same visible glyph, distinct bytes pre-fold.
        let ascii = "l'éveil";
        let curly = "l\u{2019}éveil";
        assert_eq!(compute_content_hash(ascii), compute_content_hash(curly));
    }

    #[test]
    fn test_hash_trailing_punct_ignored() {
        // Trailing `.` / `…` / `!` / `?` / ASCII-punct all elide.
        let base = compute_content_hash("proverb X");
        assert_eq!(base, compute_content_hash("proverb X."));
        assert_eq!(base, compute_content_hash("proverb X\u{2026}")); // …
        assert_eq!(base, compute_content_hash("proverb X!"));
        assert_eq!(base, compute_content_hash("proverb X?"));
    }

    #[test]
    fn test_hash_zero_width_stripped() {
        // U+200B is zero-width space — invisible to the reader but present
        // in bytes. Must be stripped before hashing.
        let dirty = "proverb\u{200B}X";
        let clean = "proverbX";
        assert_eq!(compute_content_hash(dirty), compute_content_hash(clean));
    }

    #[test]
    fn test_record_served_content_inserted_then_duplicate() {
        let mut db = Database::open_in_memory().unwrap();
        let pid = seed_person(&db);

        let out1 = db
            .record_served_content("mika", pid, "proverb", "Hello world", None)
            .unwrap();
        let inserted_id = match out1 {
            RecordOutcome::Inserted { id } => id,
            _ => panic!("expected Inserted"),
        };

        let out2 = db
            .record_served_content("mika", pid, "proverb", "hello  world", None)
            .unwrap();
        match out2 {
            RecordOutcome::Duplicate {
                existing_id,
                prior_served_at,
            } => {
                assert_eq!(existing_id, inserted_id);
                assert!(!prior_served_at.is_empty());
            }
            _ => panic!("expected Duplicate"),
        }
    }

    #[test]
    fn test_record_served_content_fk_violation() {
        let mut db = Database::open_in_memory().unwrap();
        let result = db.record_served_content("mika", 99999, "proverb", "content", None);
        let err = result.expect_err("expected FK violation");
        let msg = format!("{err}");
        assert!(msg.contains("not found in people table"), "got: {msg}");
    }

    #[test]
    fn test_list_served_content_window() {
        let mut db = Database::open_in_memory().unwrap();
        let pid = seed_person(&db);

        db.record_served_content("mika", pid, "proverb", "Recent proverb", None)
            .unwrap();
        db.record_served_content("mika", pid, "proverb", "Ancient proverb", None)
            .unwrap();

        // Age the ancient row 100 days back.
        db.conn
            .execute(
                "UPDATE served_content SET served_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-100 days') WHERE content_text = 'Ancient proverb'",
                [],
            )
            .unwrap();

        // Default 90-day window returns 1.
        let default = db
            .list_served_content("mika", pid, "proverb", None, 10, None)
            .unwrap();
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].content_text, "Recent proverb");

        // Explicit 200-day window returns both.
        let since_200 = crate::timestamp::now_minus(chrono::Duration::days(200));
        let all = db
            .list_served_content("mika", pid, "proverb", Some(&since_200), 10, None)
            .unwrap();
        assert_eq!(all.len(), 2);

        // 1-day window when both are old-ish (recent is < 1 day so still returned).
        let since_1 = crate::timestamp::now_minus(chrono::Duration::days(1));
        let recent_only = db
            .list_served_content("mika", pid, "proverb", Some(&since_1), 10, None)
            .unwrap();
        assert_eq!(recent_only.len(), 1);
    }

    #[test]
    fn test_list_served_content_category_filter() {
        let mut db = Database::open_in_memory().unwrap();
        let pid = seed_person(&db);
        db.record_served_content("mika", pid, "proverb", "a proverb", None)
            .unwrap();
        db.record_served_content("mika", pid, "joke", "a joke", None)
            .unwrap();

        let proverbs = db
            .list_served_content("mika", pid, "proverb", None, 10, None)
            .unwrap();
        assert_eq!(proverbs.len(), 1);
        assert_eq!(proverbs[0].category, "proverb");
    }

    #[test]
    fn test_list_served_content_ordering_and_limit() {
        let mut db = Database::open_in_memory().unwrap();
        let pid = seed_person(&db);
        db.record_served_content("mika", pid, "quote", "first", None)
            .unwrap();
        db.record_served_content("mika", pid, "quote", "second", None)
            .unwrap();
        db.record_served_content("mika", pid, "quote", "third", None)
            .unwrap();

        let rows = db
            .list_served_content("mika", pid, "quote", None, 2, None)
            .unwrap();
        assert_eq!(rows.len(), 2);
    }

    // -------- Al founding-incident reproduction (mika#1867) --------

    const PROVERB_X: &str = "Avant l'éveil, couper du bois, porter de l'eau. \
                             Après l'éveil, couper du bois, porter de l'eau.";
    const PROVERB_Y: &str =
        "Le maître dit : ne cherche pas la vérité, cesse seulement de chérir tes opinions.";

    #[test]
    fn test_al_six_day_apart_avoids_repeat() {
        // Al (Vietnam tester) via Telegram 2026-07-28: same zen proverb served
        // on 22 July and 28 July (6 days apart). This walks through the fix:
        //   turn 1 (t): check → empty; record → Inserted.
        //   simulate 6 calendar days of drift.
        //   turn 2 (t+6d): check → sees hash-X; record different content Y → Inserted.
        let mut db = Database::open_in_memory().unwrap();
        let al = db
            .upsert_person("mika", "Al", Some("tester"), Some("Vietnam"))
            .unwrap();

        // Turn 1: fresh check + insert.
        let prior = db
            .list_served_content("mika", al, "proverb", None, 3, None)
            .unwrap();
        assert!(prior.is_empty(), "turn 1 pre-check must be empty");
        match db
            .record_served_content("mika", al, "proverb", PROVERB_X, None)
            .unwrap()
        {
            RecordOutcome::Inserted { .. } => {}
            other => panic!("turn 1 must Insert, got {other:?}"),
        }

        // Shift stored row back 6 days.
        db.conn
            .execute(
                "UPDATE served_content
                    SET served_at = strftime('%Y-%m-%dT%H:%M:%SZ', served_at, '-6 days')",
                [],
            )
            .unwrap();

        // Turn 2 (t+6d).
        let prior = db
            .list_served_content("mika", al, "proverb", None, 3, None)
            .unwrap();
        assert_eq!(
            prior.len(),
            1,
            "turn 2 pre-check must see the 6-day-old serve"
        );
        assert_eq!(prior[0].content_hash, compute_content_hash(PROVERB_X));

        // LLM sees the check result and generates a distinct proverb.
        match db
            .record_served_content("mika", al, "proverb", PROVERB_Y, None)
            .unwrap()
        {
            RecordOutcome::Inserted { .. } => {}
            other => panic!("turn 2 must Insert distinct content, got {other:?}"),
        }

        let all = db
            .list_served_content("mika", al, "proverb", None, 10, None)
            .unwrap();
        assert_eq!(all.len(), 2);
        let hashes: std::collections::HashSet<_> =
            all.iter().map(|r| r.content_hash.clone()).collect();
        assert_eq!(hashes.len(), 2, "two distinct hashes must be stored");
        assert!(hashes.contains(&compute_content_hash(PROVERB_X)));
        assert!(hashes.contains(&compute_content_hash(PROVERB_Y)));
    }

    #[test]
    fn test_al_retry_with_avoid_three_collisions_no_extra_persist() {
        // AC5: a runaway LLM that regenerates the same hash 3 times in a row
        // must not silently re-persist. Structural gate: ON CONFLICT DO NOTHING.
        let mut db = Database::open_in_memory().unwrap();
        let al = db
            .upsert_person("mika", "Al", Some("tester"), Some("Vietnam"))
            .unwrap();

        // Prime with X.
        db.record_served_content("mika", al, "proverb", PROVERB_X, None)
            .unwrap();

        for attempt in 1..=3 {
            match db
                .record_served_content("mika", al, "proverb", PROVERB_X, None)
                .unwrap()
            {
                RecordOutcome::Duplicate { .. } => {}
                other => panic!("retry attempt {attempt} must be Duplicate, got {other:?}"),
            }
        }

        // Only the initial X row should persist.
        let all = db
            .list_served_content("mika", al, "proverb", None, 10, None)
            .unwrap();
        assert_eq!(all.len(), 1, "no extra rows should have been written");
        assert_eq!(all[0].content_hash, compute_content_hash(PROVERB_X));
    }

    /// /ce:review P1-5 — RC-A per-user filter test. If a future refactor
    /// drops `AND person_id = ?` from `list_served_content` (e.g. copies the
    /// history-fetch semantics that key on agent only), Alice's ledger read
    /// would contaminate with Bob's serves. This test catches that class
    /// structurally.
    #[test]
    fn test_list_served_content_isolates_by_person_id() {
        let mut db = Database::open_in_memory().unwrap();
        let agent_id = "mika";
        let alice_id = db.upsert_person(agent_id, "Alice", None, None).unwrap();
        let bob_id = db.upsert_person(agent_id, "Bob", None, None).unwrap();

        db.record_served_content(agent_id, alice_id, "proverb", "Content X for Alice", None)
            .unwrap();
        db.record_served_content(agent_id, bob_id, "proverb", "Content Y for Bob", None)
            .unwrap();

        // Alice sees only Alice's serve.
        let alice_items = db
            .list_served_content(agent_id, alice_id, "proverb", None, 10, None)
            .unwrap();
        assert_eq!(
            alice_items.len(),
            1,
            "Alice ledger must not include Bob's serve"
        );
        assert!(
            alice_items[0].content_text.contains("Alice"),
            "Alice ledger returned wrong person's content: {}",
            alice_items[0].content_text
        );

        // Bob sees only Bob's serve.
        let bob_items = db
            .list_served_content(agent_id, bob_id, "proverb", None, 10, None)
            .unwrap();
        assert_eq!(
            bob_items.len(),
            1,
            "Bob ledger must not include Alice's serve"
        );
        assert!(
            bob_items[0].content_text.contains("Bob"),
            "Bob ledger returned wrong person's content: {}",
            bob_items[0].content_text
        );
    }

    /// /ce:review P2-5 — AC10 audit-events silence assertion.
    ///
    /// AC10 says: on duplicate detection, `record_served_content` emits a
    /// `warn!` (grepable via `served_content.duplicate_write`) and does NOT
    /// write an `audit_events` row. The audit-events silence is the
    /// load-bearing half — a duplicate-write is a high-volume low-signal
    /// event that should never bloat the audit trail. This test asserts the
    /// count is unchanged; the warn assertion is omitted because the repo
    /// does not have a shared tracing-test harness (see NOTE below).
    ///
    /// NOTE: if a tracing capture util is added later
    /// (`tracing_test::traced_test` or a shared `TestSubscriber`), extend
    /// this test to also assert the `served_content.duplicate_write` event
    /// fired. The structural silence check is the primary invariant.
    #[test]
    fn test_ac10_duplicate_write_no_audit_event() {
        let mut db = Database::open_in_memory().unwrap();
        let agent_id = "mika";
        let person_id = db.upsert_person(agent_id, "Alice", None, None).unwrap();

        // First insert — Inserted.
        let first = db
            .record_served_content(agent_id, person_id, "proverb", "Same content", None)
            .unwrap();
        assert!(matches!(first, RecordOutcome::Inserted { .. }));

        // Baseline audit_events count.
        let audit_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();

        // Second insert — Duplicate. MUST warn but MUST NOT touch audit_events.
        let second = db
            .record_served_content(agent_id, person_id, "proverb", "Same content", None)
            .unwrap();
        assert!(matches!(second, RecordOutcome::Duplicate { .. }));

        let audit_after: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            audit_before, audit_after,
            "AC10: record_served_content duplicate MUST NOT write audit_events"
        );
    }
}
