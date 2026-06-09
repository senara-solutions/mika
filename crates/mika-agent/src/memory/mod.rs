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

use anyhow::Result;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::db::{CoreMemoryEntry, DashboardFact, Database, Event, Person, Preference};

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
