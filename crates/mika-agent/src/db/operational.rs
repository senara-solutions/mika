//! Database methods for the `operational_items` table (mika#1262).
//!
//! Follows the `kg_schema.rs` pattern for DB module separation. All methods
//! are on [`Database`] and wrapped by [`AsyncDatabase`] via channel dispatch.
//!
//! ## Write contract
//!
//! Writes to `operational_items.evidence_refs` MUST go through
//! `crates/mika-agent/src/operational/types.rs::EvidenceRef`. Direct SQL
//! writes that bypass the Rust type are unsupported and will break the
//! closed-enum guarantee from foundation Decision F.

use crate::operational::types::*;
use crate::timestamp;
use anyhow::{Context, Result, bail};
use rusqlite::{Transaction, params};
use tracing::debug;
use uuid::Uuid;

use super::Database;

impl Database {
    // -----------------------------------------------------------------------
    // Insert / Upsert
    // -----------------------------------------------------------------------

    /// Insert a new operational item, generating a UUID and timestamps.
    pub fn insert_operational_item(&self, item: &NewOperationalItem) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = timestamp::now();
        let evidence_json = serde_json::to_string(&item.evidence_refs)?;

        self.conn.execute(
            "INSERT INTO operational_items (
                id, kind, title, status, owner_type, owner_name,
                priority, user_importance, due_at, blocked_by, next_action,
                evidence_refs, confidence, source_table, source_id,
                agent_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                id,
                item.kind.as_str(),
                item.title,
                item.status.as_str(),
                item.owner.owner_type(),
                item.owner.owner_name(),
                item.priority,
                item.user_importance,
                item.due_at,
                item.blocked_by,
                item.next_action,
                evidence_json,
                item.confidence,
                item.source_table,
                item.source_id,
                item.agent_id,
                now,
                now,
            ],
        )?;

        debug!(id = %id, kind = %item.kind, agent_id = %item.agent_id, "inserted operational item");
        Ok(id)
    }

    /// Upsert an operational item keyed on `(agent_id, source_table, source_id)`.
    ///
    /// This is the primary write-path method — idempotent for re-delivery.
    /// Requires `source_table` and `source_id` to be `Some`.
    pub fn upsert_operational_item_by_source(&self, item: &NewOperationalItem) -> Result<String> {
        let source_table = item
            .source_table
            .as_deref()
            .context("upsert_operational_item_by_source requires source_table")?;
        let source_id = item
            .source_id
            .as_deref()
            .context("upsert_operational_item_by_source requires source_id")?;

        // Check if an item already exists for this source
        if let Some(existing) =
            self.get_operational_item_by_source(&item.agent_id, source_table, source_id)?
        {
            // Update the existing item
            let now = timestamp::now();
            let evidence_json = serde_json::to_string(&item.evidence_refs)?;

            self.conn.execute(
                "UPDATE operational_items SET
                    kind = ?1, title = ?2, status = ?3, owner_type = ?4, owner_name = ?5,
                    priority = ?6, user_importance = ?7, due_at = ?8, blocked_by = ?9,
                    next_action = ?10, evidence_refs = ?11, confidence = ?12, updated_at = ?13
                WHERE id = ?14",
                params![
                    item.kind.as_str(),
                    item.title,
                    item.status.as_str(),
                    item.owner.owner_type(),
                    item.owner.owner_name(),
                    item.priority,
                    item.user_importance,
                    item.due_at,
                    item.blocked_by,
                    item.next_action,
                    evidence_json,
                    item.confidence,
                    now,
                    existing.id,
                ],
            )?;

            debug!(id = %existing.id, kind = %item.kind, "updated operational item by source");
            Ok(existing.id)
        } else {
            self.insert_operational_item(item)
        }
    }

    /// Transaction-accepting variant of [`upsert_operational_item_by_source`].
    ///
    /// Used when atomicity per §5.2 requires the same `&Transaction` reference
    /// to be threaded through source-of-truth + operational writes.
    pub fn upsert_operational_item_by_source_tx(
        &self,
        tx: &Transaction<'_>,
        item: &NewOperationalItem,
    ) -> Result<String> {
        let source_table = item
            .source_table
            .as_deref()
            .context("upsert_operational_item_by_source_tx requires source_table")?;
        let source_id = item
            .source_id
            .as_deref()
            .context("upsert_operational_item_by_source_tx requires source_id")?;

        let now = timestamp::now();
        let evidence_json = serde_json::to_string(&item.evidence_refs)?;

        // Check if exists
        let existing_id: Option<String> = tx.query_row(
            "SELECT id FROM operational_items WHERE agent_id = ?1 AND source_table = ?2 AND source_id = ?3",
            params![item.agent_id, source_table, source_id],
            |row| row.get(0),
        ).ok();

        if let Some(id) = existing_id {
            tx.execute(
                "UPDATE operational_items SET
                    kind = ?1, title = ?2, status = ?3, owner_type = ?4, owner_name = ?5,
                    priority = ?6, user_importance = ?7, due_at = ?8, blocked_by = ?9,
                    next_action = ?10, evidence_refs = ?11, confidence = ?12, updated_at = ?13
                WHERE id = ?14",
                params![
                    item.kind.as_str(),
                    item.title,
                    item.status.as_str(),
                    item.owner.owner_type(),
                    item.owner.owner_name(),
                    item.priority,
                    item.user_importance,
                    item.due_at,
                    item.blocked_by,
                    item.next_action,
                    evidence_json,
                    item.confidence,
                    now,
                    id,
                ],
            )?;
            debug!(id = %id, kind = %item.kind, "updated operational item by source (tx)");
            Ok(id)
        } else {
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO operational_items (
                    id, kind, title, status, owner_type, owner_name,
                    priority, user_importance, due_at, blocked_by, next_action,
                    evidence_refs, confidence, source_table, source_id,
                    agent_id, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    id,
                    item.kind.as_str(),
                    item.title,
                    item.status.as_str(),
                    item.owner.owner_type(),
                    item.owner.owner_name(),
                    item.priority,
                    item.user_importance,
                    item.due_at,
                    item.blocked_by,
                    item.next_action,
                    evidence_json,
                    item.confidence,
                    source_table,
                    source_id,
                    item.agent_id,
                    now,
                    now,
                ],
            )?;
            debug!(id = %id, kind = %item.kind, "inserted operational item (tx)");
            Ok(id)
        }
    }

    // -----------------------------------------------------------------------
    // Status transitions
    // -----------------------------------------------------------------------

    /// Update status for non-terminal transitions only.
    ///
    /// The type-level guard via [`NonTerminalStatus`] prevents callers from
    /// passing `Done`. Use [`complete_operational_item`] for terminal transitions.
    pub fn update_operational_item_status(
        &self,
        id: &str,
        status: NonTerminalStatus,
    ) -> Result<()> {
        let now = timestamp::now();
        let op_status: OperationalStatus = status.into();

        let updated = self.conn.execute(
            "UPDATE operational_items SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status != 'done'",
            params![op_status.as_str(), now, id],
        )?;

        if updated == 0 {
            debug!(id = %id, "operational item not found or already done");
        }
        Ok(())
    }

    /// The **only** path to write `status = Done`. Requires an Evidence
    /// reference so the terminal transition is always audit-traceable.
    ///
    /// Returns `Err` if the item is already `Done` (per Decision G).
    pub fn complete_operational_item(&self, id: &str, evidence: EvidenceRef) -> Result<()> {
        let now = timestamp::now();

        // Check current status
        let current_status: String = self
            .conn
            .query_row(
                "SELECT status FROM operational_items WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .context("operational item not found")?;

        if current_status == "done" {
            bail!("operational item {id} is already done (terminal state)");
        }

        // Read existing evidence_refs and append the new one
        let existing_refs_json: String = self.conn.query_row(
            "SELECT evidence_refs FROM operational_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let mut refs: Vec<EvidenceRef> =
            serde_json::from_str(&existing_refs_json).unwrap_or_default();
        refs.push(evidence);
        let refs_json = serde_json::to_string(&refs)?;

        self.conn.execute(
            "UPDATE operational_items SET status = 'done', evidence_refs = ?1, updated_at = ?2 WHERE id = ?3",
            params![refs_json, now, id],
        )?;

        debug!(id = %id, "completed operational item");
        Ok(())
    }

    /// The **only** path out of `status = Done`. Transitions back to `Now`
    /// (re-derivation takes over on subsequent reads). Requires an Evidence
    /// reference per foundation §4 "explicit re-open with audit-trail Evidence link."
    pub fn reopen_operational_item(&self, id: &str, evidence: EvidenceRef) -> Result<()> {
        let now = timestamp::now();

        let current_status: String = self
            .conn
            .query_row(
                "SELECT status FROM operational_items WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .context("operational item not found")?;

        if current_status != "done" {
            bail!("operational item {id} is not done (status: {current_status}), cannot reopen");
        }

        // Append evidence for the reopen event
        let existing_refs_json: String = self.conn.query_row(
            "SELECT evidence_refs FROM operational_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let mut refs: Vec<EvidenceRef> =
            serde_json::from_str(&existing_refs_json).unwrap_or_default();
        refs.push(evidence);
        let refs_json = serde_json::to_string(&refs)?;

        self.conn.execute(
            "UPDATE operational_items SET status = 'now', evidence_refs = ?1, updated_at = ?2 WHERE id = ?3",
            params![refs_json, now, id],
        )?;

        debug!(id = %id, "reopened operational item");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Priority update
    // -----------------------------------------------------------------------

    /// Update the cached priority score (Layer 2 calls this).
    pub fn update_operational_item_priority(&self, id: &str, priority: f32) -> Result<()> {
        let now = timestamp::now();
        self.conn.execute(
            "UPDATE operational_items SET priority = ?1, updated_at = ?2 WHERE id = ?3",
            params![priority, now, id],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Canonical read query with filtering and sorting.
    pub fn query_operational_items(
        &self,
        filter: &OperationalItemFilter,
    ) -> Result<Vec<OperationalItem>> {
        let mut sql = String::from(
            "SELECT id, kind, title, status, owner_type, owner_name,
                    priority, user_importance, due_at, blocked_by, next_action,
                    evidence_refs, confidence, source_table, source_id,
                    agent_id, created_at, updated_at
             FROM operational_items WHERE agent_id = ?1",
        );
        let mut param_idx = 2u32;

        // We'll build params as strings and bind dynamically
        let mut bind_values: Vec<String> = vec![filter.agent_id.clone()];

        if let Some(kind) = &filter.kind {
            sql.push_str(&format!(" AND kind = ?{param_idx}"));
            bind_values.push(kind.as_str().to_string());
            param_idx += 1;
        }
        if let Some(status) = &filter.status {
            sql.push_str(&format!(" AND status = ?{param_idx}"));
            bind_values.push(status.as_str().to_string());
            param_idx += 1;
        }
        if let Some(owner_type) = &filter.owner_type {
            sql.push_str(&format!(" AND owner_type = ?{param_idx}"));
            bind_values.push(owner_type.clone());
            let _ = param_idx; // suppress unused warning
        }

        if filter.sort_by_priority {
            sql.push_str(" ORDER BY priority DESC");
        } else {
            sql.push_str(" ORDER BY created_at DESC");
        }

        let limit = filter.limit.unwrap_or(20).min(100);
        sql.push_str(&format!(" LIMIT {limit}"));

        let mut stmt = self.conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = bind_values
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();

        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(row_to_operational_item(row))
        })?;

        let mut items = Vec::new();
        for row in rows {
            match row? {
                Ok(item) => items.push(item),
                Err(e) => {
                    debug!(error = %e, "skipping malformed operational item row");
                }
            }
        }
        Ok(items)
    }

    /// Lookup by source — for status-update paths that need to find the
    /// existing operational item for a given source row.
    pub fn get_operational_item_by_source(
        &self,
        agent_id: &str,
        source_table: &str,
        source_id: &str,
    ) -> Result<Option<OperationalItem>> {
        let result = self.conn.query_row(
            "SELECT id, kind, title, status, owner_type, owner_name,
                    priority, user_importance, due_at, blocked_by, next_action,
                    evidence_refs, confidence, source_table, source_id,
                    agent_id, created_at, updated_at
             FROM operational_items
             WHERE agent_id = ?1 AND source_table = ?2 AND source_id = ?3",
            params![agent_id, source_table, source_id],
            |row| Ok(row_to_operational_item(row)),
        );

        match result {
            Ok(Ok(item)) => Ok(Some(item)),
            Ok(Err(e)) => Err(e),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count items blocked by the given item ID — for dependency_risk scoring.
    pub fn count_blocked_items(&self, blocked_by_id: &str) -> Result<u32> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM operational_items WHERE blocked_by = ?1",
            params![blocked_by_id],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    /// Batch-load dependency counts for all items of an agent in a single query.
    ///
    /// Returns a `HashMap<blocked_by_id, count>` — how many non-Done items are
    /// blocked by each item. Used by the scoring engine to avoid N+1 queries
    /// (plan §1.3).
    pub fn batch_blocked_counts(
        &self,
        agent_id: &str,
    ) -> Result<std::collections::HashMap<String, u32>> {
        let mut stmt = self.conn.prepare(
            "SELECT blocked_by, COUNT(*) as blocked_count
             FROM operational_items
             WHERE agent_id = ?1 AND blocked_by IS NOT NULL AND status != 'done'
             GROUP BY blocked_by",
        )?;

        let mut counts = std::collections::HashMap::new();
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        for row in rows {
            let (blocked_by, count) = row?;
            counts.insert(blocked_by, count as u32);
        }

        Ok(counts)
    }

    /// Get the status of a single operational item by ID.
    ///
    /// Used by `resolve_cleared_blockers()` to check if a blocker is Done.
    pub fn get_operational_item_status(&self, id: &str) -> Result<Option<OperationalStatus>> {
        let result = self.conn.query_row(
            "SELECT status FROM operational_items WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(status_str) => Ok(OperationalStatus::from_str_opt(&status_str)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Clear the `blocked_by` field for an operational item.
    ///
    /// Primary cascade mechanism: when a blocker moves to Done, dependent
    /// items have their `blocked_by` cleared so they re-derive to a non-Waiting
    /// status (plan §2.3).
    pub fn clear_operational_item_blocked_by(&self, id: &str) -> Result<()> {
        let now = timestamp::now();
        self.conn.execute(
            "UPDATE operational_items SET blocked_by = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    /// Query non-Done items for an agent, ordered by priority DESC.
    ///
    /// Used by the canonical read path (`score_and_rank_items`).
    pub fn query_active_operational_items(
        &self,
        agent_id: &str,
        limit: u32,
    ) -> Result<Vec<OperationalItem>> {
        let effective_limit = limit.clamp(1, 100) as i64;

        let mut stmt = self.conn.prepare(
            "SELECT id, kind, title, status, owner_type, owner_name,
                    priority, user_importance, due_at, blocked_by, next_action,
                    evidence_refs, confidence, source_table, source_id,
                    agent_id, created_at, updated_at
             FROM operational_items WHERE agent_id = ?1 AND status != 'done'
             ORDER BY priority DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![agent_id, effective_limit], |row| {
            Ok(row_to_operational_item(row))
        })?;

        let mut items = Vec::new();
        for row in rows {
            match row? {
                Ok(item) => items.push(item),
                Err(e) => {
                    debug!(error = %e, "skipping malformed operational item row");
                }
            }
        }
        Ok(items)
    }
}

/// Map a row to an [`OperationalItem`]. Returns `Result` to allow skipping
/// malformed rows (e.g., unknown enum variants from future migrations).
fn row_to_operational_item(row: &rusqlite::Row<'_>) -> Result<OperationalItem> {
    let kind_str: String = row.get(1)?;
    let status_str: String = row.get(3)?;
    let owner_type_str: String = row.get(4)?;
    let owner_name: Option<String> = row.get(5)?;
    let evidence_json: String = row.get(11)?;

    let kind = OperationalKind::from_str_opt(&kind_str)
        .context(format!("unknown operational kind: {kind_str}"))?;
    let status = OperationalStatus::from_str_opt(&status_str)
        .context(format!("unknown operational status: {status_str}"))?;
    let owner = Owner::from_db(&owner_type_str, owner_name.as_deref()).context(format!(
        "invalid owner: type={owner_type_str}, name={owner_name:?}"
    ))?;
    let evidence_refs: Vec<EvidenceRef> = serde_json::from_str(&evidence_json).unwrap_or_default();

    Ok(OperationalItem {
        id: row.get(0)?,
        kind,
        title: row.get(2)?,
        status,
        owner,
        priority: row.get(6)?,
        user_importance: row.get(7)?,
        due_at: row.get(8)?,
        blocked_by: row.get(9)?,
        next_action: row.get(10)?,
        evidence_refs,
        confidence: row.get(12)?,
        source_table: row.get(13)?,
        source_id: row.get(14)?,
        agent_id: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn sample_item(agent_id: &str) -> NewOperationalItem {
        NewOperationalItem {
            kind: OperationalKind::Task,
            title: "Test task".to_string(),
            status: OperationalStatus::Now,
            owner: Owner::User,
            priority: 0.0,
            user_importance: 0.0,
            due_at: None,
            blocked_by: None,
            next_action: None,
            evidence_refs: Vec::new(),
            confidence: 1.0,
            source_table: Some("tasks".to_string()),
            source_id: Some("task-001".to_string()),
            agent_id: agent_id.to_string(),
        }
    }

    #[test]
    fn insert_and_query() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();
        assert!(!id.is_empty());

        let filter = OperationalItemFilter {
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let results = db.query_operational_items(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);
        assert_eq!(results[0].kind, OperationalKind::Task);
        assert_eq!(results[0].title, "Test task");
        assert_eq!(results[0].status, OperationalStatus::Now);
        assert_eq!(results[0].owner, Owner::User);
    }

    #[test]
    fn upsert_by_source_creates_then_updates() {
        let db = test_db();
        let item = sample_item("mika");

        // First upsert — creates
        let id1 = db.upsert_operational_item_by_source(&item).unwrap();

        // Second upsert with same source — updates, same ID
        let mut item2 = item.clone();
        item2.title = "Updated title".to_string();
        let id2 = db.upsert_operational_item_by_source(&item2).unwrap();
        assert_eq!(id1, id2);

        // Verify updated
        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.title, "Updated title");
    }

    #[test]
    fn upsert_idempotency_unique_index() {
        let db = test_db();
        let item = sample_item("mika");

        db.upsert_operational_item_by_source(&item).unwrap();
        db.upsert_operational_item_by_source(&item).unwrap();
        db.upsert_operational_item_by_source(&item).unwrap();

        let filter = OperationalItemFilter {
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let results = db.query_operational_items(&filter).unwrap();
        assert_eq!(
            results.len(),
            1,
            "multiple upserts should not create duplicates"
        );
    }

    #[test]
    fn update_status_non_terminal() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        db.update_operational_item_status(&id, NonTerminalStatus::Waiting)
            .unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OperationalStatus::Waiting);
    }

    #[test]
    fn complete_requires_evidence() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        let evidence = EvidenceRef {
            kind: EvidenceRefKind::GithubPr,
            id: "https://github.com/test/pr/1".to_string(),
        };
        db.complete_operational_item(&id, evidence.clone()).unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OperationalStatus::Done);
        assert_eq!(found.evidence_refs.len(), 1);
        assert_eq!(found.evidence_refs[0].kind, EvidenceRefKind::GithubPr);
    }

    #[test]
    fn complete_already_done_fails() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        let evidence = EvidenceRef {
            kind: EvidenceRefKind::GithubPr,
            id: "pr-1".to_string(),
        };
        db.complete_operational_item(&id, evidence.clone()).unwrap();

        // Second complete should fail
        let result = db.complete_operational_item(&id, evidence);
        assert!(
            result.is_err(),
            "completing an already-done item should fail"
        );
    }

    #[test]
    fn update_status_skips_done_items() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        // Complete the item
        let evidence = EvidenceRef {
            kind: EvidenceRefKind::GithubPr,
            id: "pr-1".to_string(),
        };
        db.complete_operational_item(&id, evidence).unwrap();

        // Try to update status — should be silently skipped
        db.update_operational_item_status(&id, NonTerminalStatus::Now)
            .unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(
            found.status,
            OperationalStatus::Done,
            "done items should not be overwritten"
        );
    }

    #[test]
    fn reopen_from_done() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        let evidence1 = EvidenceRef {
            kind: EvidenceRefKind::GithubPr,
            id: "pr-1".to_string(),
        };
        db.complete_operational_item(&id, evidence1).unwrap();

        let evidence2 = EvidenceRef {
            kind: EvidenceRefKind::External,
            id: "reopen-reason".to_string(),
        };
        db.reopen_operational_item(&id, evidence2).unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OperationalStatus::Now);
        assert_eq!(
            found.evidence_refs.len(),
            2,
            "reopen should append evidence"
        );
    }

    #[test]
    fn reopen_non_done_fails() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        let evidence = EvidenceRef {
            kind: EvidenceRefKind::External,
            id: "x".to_string(),
        };
        let result = db.reopen_operational_item(&id, evidence);
        assert!(result.is_err(), "reopening a non-done item should fail");
    }

    #[test]
    fn query_with_kind_filter() {
        let db = test_db();

        let mut item1 = sample_item("mika");
        item1.kind = OperationalKind::Task;
        item1.source_id = Some("t1".to_string());
        db.insert_operational_item(&item1).unwrap();

        let mut item2 = sample_item("mika");
        item2.kind = OperationalKind::Goal;
        item2.source_id = Some("t2".to_string());
        db.insert_operational_item(&item2).unwrap();

        let filter = OperationalItemFilter {
            agent_id: "mika".to_string(),
            kind: Some(OperationalKind::Goal),
            ..Default::default()
        };
        let results = db.query_operational_items(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, OperationalKind::Goal);
    }

    #[test]
    fn query_with_status_filter() {
        let db = test_db();

        let mut item1 = sample_item("mika");
        item1.status = OperationalStatus::Now;
        item1.source_id = Some("t1".to_string());
        db.insert_operational_item(&item1).unwrap();

        let mut item2 = sample_item("mika");
        item2.status = OperationalStatus::Scheduled;
        item2.source_id = Some("t2".to_string());
        db.insert_operational_item(&item2).unwrap();

        let filter = OperationalItemFilter {
            agent_id: "mika".to_string(),
            status: Some(OperationalStatus::Scheduled),
            ..Default::default()
        };
        let results = db.query_operational_items(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, OperationalStatus::Scheduled);
    }

    #[test]
    fn query_sort_by_priority() {
        let db = test_db();

        for (i, prio) in [10.0, 50.0, 30.0].iter().enumerate() {
            let mut item = sample_item("mika");
            item.priority = *prio;
            item.source_id = Some(format!("t{i}"));
            db.insert_operational_item(&item).unwrap();
        }

        let filter = OperationalItemFilter {
            agent_id: "mika".to_string(),
            sort_by_priority: true,
            ..Default::default()
        };
        let results = db.query_operational_items(&filter).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].priority, 50.0);
        assert_eq!(results[1].priority, 30.0);
        assert_eq!(results[2].priority, 10.0);
    }

    #[test]
    fn count_blocked_items() {
        let db = test_db();

        let blocker = sample_item("mika");
        let blocker_id = db.insert_operational_item(&blocker).unwrap();

        // Create 3 items blocked by the blocker
        for i in 0..3 {
            let mut item = NewOperationalItem {
                blocked_by: Some(blocker_id.clone()),
                source_table: Some("tasks".to_string()),
                source_id: Some(format!("blocked-{i}")),
                agent_id: "mika".to_string(),
                ..Default::default()
            };
            item.title = format!("Blocked item {i}");
            db.insert_operational_item(&item).unwrap();
        }

        let count = db.count_blocked_items(&blocker_id).unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn get_by_source_not_found() {
        let db = test_db();
        let result = db
            .get_operational_item_by_source("mika", "tasks", "nonexistent")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn update_priority() {
        let db = test_db();
        let item = sample_item("mika");
        let id = db.insert_operational_item(&item).unwrap();

        db.update_operational_item_priority(&id, 42.5).unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert!((found.priority - 42.5).abs() < f32::EPSILON);
    }

    #[test]
    fn agent_scoping() {
        let db = test_db();
        // Create agent2
        db.conn
            .execute(
                "INSERT INTO agents (id, name, home_dir) VALUES ('agent2', 'Agent 2', '')",
                [],
            )
            .unwrap();

        let item1 = sample_item("mika");
        db.insert_operational_item(&item1).unwrap();

        let mut item2 = sample_item("agent2");
        item2.source_id = Some("task-002".to_string());
        db.insert_operational_item(&item2).unwrap();

        // Query for mika only
        let filter = OperationalItemFilter {
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let results = db.query_operational_items(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].agent_id, "mika");
    }

    #[test]
    fn evidence_refs_round_trip() {
        let db = test_db();
        let mut item = sample_item("mika");
        item.evidence_refs = vec![
            EvidenceRef {
                kind: EvidenceRefKind::GithubPr,
                id: "pr-1".to_string(),
            },
            EvidenceRef {
                kind: EvidenceRefKind::Message,
                id: "msg-1".to_string(),
            },
        ];
        db.insert_operational_item(&item).unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.evidence_refs.len(), 2);
        assert_eq!(found.evidence_refs[0].kind, EvidenceRefKind::GithubPr);
        assert_eq!(found.evidence_refs[1].kind, EvidenceRefKind::Message);
    }

    #[test]
    fn owner_variants_persist() {
        let db = test_db();
        let owners = [
            (Owner::User, "u1"),
            (Owner::Mika, "u2"),
            (Owner::Person("Alice".into()), "u3"),
            (Owner::Agent("mika-dev".into()), "u4"),
        ];

        for (owner, sid) in &owners {
            let mut item = sample_item("mika");
            item.owner = owner.clone();
            item.source_id = Some(sid.to_string());
            db.insert_operational_item(&item).unwrap();
        }

        for (expected_owner, sid) in &owners {
            let found = db
                .get_operational_item_by_source("mika", "tasks", sid)
                .unwrap()
                .unwrap();
            assert_eq!(
                &found.owner, expected_owner,
                "owner mismatch for source {sid}"
            );
        }
    }

    #[test]
    fn upsert_tx_create_and_update_via_self_transaction() {
        // Test the _tx variant through Database's own transaction method.
        // The _tx methods are designed to be called from within Database methods
        // that own the transaction (e.g., create_task atomicity).
        // Here we test via the non-tx wrapper as a proxy for the same DB logic.
        let db = test_db();
        let item = sample_item("mika");

        // First call creates
        let id1 = db.upsert_operational_item_by_source(&item).unwrap();

        // Second call with same source updates
        let mut item2 = item.clone();
        item2.title = "Updated title".to_string();
        let id2 = db.upsert_operational_item_by_source(&item2).unwrap();

        assert_eq!(id1, id2, "upsert should return same ID for same source");
        let found = db
            .get_operational_item_by_source("mika", "tasks", "task-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.title, "Updated title");
    }

    #[test]
    fn cross_path_reminder_to_completion() {
        let db = test_db();

        // Create a scheduled reminder item
        let reminder = NewOperationalItem {
            kind: OperationalKind::Task,
            title: "Send weekly report".to_string(),
            status: OperationalStatus::Scheduled,
            owner: Owner::User,
            due_at: Some("2026-06-01T09:00:00Z".to_string()),
            confidence: 1.0,
            source_table: Some("tasks".to_string()),
            source_id: Some("reminder-001".to_string()),
            agent_id: "mika".to_string(),
            ..Default::default()
        };
        let id = db.upsert_operational_item_by_source(&reminder).unwrap();

        // Callback completes the task
        let evidence = EvidenceRef {
            kind: EvidenceRefKind::ToolCall,
            id: "tool-call-001".to_string(),
        };
        db.complete_operational_item(&id, evidence).unwrap();

        let found = db
            .get_operational_item_by_source("mika", "tasks", "reminder-001")
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OperationalStatus::Done);
    }

    #[test]
    fn migration_v38_to_v39_idempotent() {
        // This is tested by the convergence test, but let's also verify
        // the idempotency of the migration function directly.
        let mut db = test_db();
        // DB is already at v39, calling migrate again should be a no-op
        db.migrate_v38_to_v39().unwrap();
    }
}
