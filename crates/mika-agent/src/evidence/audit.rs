//! Audit-event ledger: `AuditEvent` struct and `Database` methods for
//! the tool-call audit trail that backs the rewind path and dashboard surfaces.

use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use utoipa::ToSchema;

use crate::db::Database;
use crate::timestamp;
use chrono::Duration;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditEvent {
    pub id: i64,
    pub agent_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub target_key: String,
    pub before_value: Option<String>,
    pub after_value: Option<String>,
    pub reasoning: Option<String>,
    pub trace_id: Option<String>,
    pub rewound_by_trace_id: Option<String>,
    pub created_at: String,
}

/// `tool_name` carried by every auth-boundary audit row (mika#1949 AC2).
///
/// One literal, one place. A call site that spelled it by hand would be
/// invisible to `WHERE tool_name = 'auth_boundary'` on the day it mattered.
pub const AUTH_BOUNDARY_TOOL_NAME: &str = "auth_boundary";

/// `session_id` carried by auth-boundary rows.
///
/// A boundary failure belongs to no conversation — it happens on the wire,
/// often before any session exists. `audit_events.session_id` is `NOT NULL`
/// with no foreign key, so a well-known constant is the honest filler: it
/// says "not session-scoped" instead of borrowing an unrelated session id and
/// making the row look like it came from a turn.
pub const AUTH_BOUNDARY_SESSION_ID: &str = "auth-boundary";

impl Database {
    // ===== Audit Events =====

    #[allow(clippy::too_many_arguments)]
    pub fn log_audit_event(
        &self,
        agent_id: &str,
        session_id: &str,
        tool_name: &str,
        target_key: &str,
        before_value: Option<&str>,
        after_value: Option<&str>,
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

    /// Record one cross-boundary authentication failure in the audit ledger
    /// (mika#1949, Porte 3).
    ///
    /// # Row shape
    ///
    /// | column | value |
    /// |---|---|
    /// | `tool_name` | [`AUTH_BOUNDARY_TOOL_NAME`] — always `auth_boundary` |
    /// | `session_id` | [`AUTH_BOUNDARY_SESSION_ID`] |
    /// | `target_key` | `<from>_to_<to>`, from [`AuthBoundaryError::boundary_key`] |
    /// | `after_value` | the failure kind: `missing`/`empty`/`invalid`/`rejected`/`unreachable` |
    /// | `reasoning` | JSON `{"token_name","kind","from","to"}` |
    /// | `before_value` | always `NULL` — there is no prior state to name |
    ///
    /// # Why callers ignore the `Result`
    ///
    /// This returns `Result` so that a test can assert the write happened, and
    /// so a caller can log the failure. **No caller may let it change the
    /// request's outcome** (mika#1949 R5): the authentication verdict is
    /// already decided by the time this runs, and an audit ledger that could
    /// turn a refusal into a 500 would be a new failure mode bolted onto an
    /// observability feature. Call sites log at `warn` and continue —
    /// `crate::auth_boundary_ledger` is the fire-and-forget wrapper that does
    /// exactly that.
    ///
    /// # No token value, ever
    ///
    /// Every field written here comes from [`AuthBoundaryError`], which has no
    /// field that can hold a secret. See its module docs for the invariant and
    /// the test that pins it.
    pub fn record_auth_boundary(
        &self,
        agent_id: &str,
        err: &mika_common::auth_boundary::AuthBoundaryError,
    ) -> Result<()> {
        let reasoning = serde_json::json!({
            "token_name": err.token_name,
            "kind": err.kind.as_str(),
            "from": err.from,
            "to": err.to,
        })
        .to_string();
        self.log_audit_event(
            agent_id,
            AUTH_BOUNDARY_SESSION_ID,
            AUTH_BOUNDARY_TOOL_NAME,
            &err.boundary_key(),
            None,
            Some(err.kind.as_str()),
            Some(&reasoning),
            None,
        )
    }

    /// Standard column list for audit event queries.
    pub(crate) const AUDIT_EVENT_COLS: &str = "id, agent_id, session_id, tool_name, target_key, before_value, after_value, reasoning, trace_id, rewound_by_trace_id, created_at";

    pub(crate) fn row_to_audit_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEvent> {
        Ok(AuditEvent {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            session_id: r.get(2)?,
            tool_name: r.get(3)?,
            target_key: r.get(4)?,
            before_value: r.get(5)?,
            after_value: r.get(6)?,
            reasoning: r.get(7)?,
            trace_id: r.get(8)?,
            rewound_by_trace_id: r.get(9)?,
            created_at: r.get(10)?,
        })
    }

    pub fn get_audit_events(&self, agent_id: &str, session_id: &str) -> Result<Vec<AuditEvent>> {
        let sql = format!(
            "SELECT {} FROM audit_events WHERE agent_id = ?1 AND session_id = ?2 ORDER BY created_at ASC",
            Self::AUDIT_EVENT_COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, session_id], Self::row_to_audit_event)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_audit_events_since(&self, agent_id: &str, since: &str) -> Result<Vec<AuditEvent>> {
        let sql = format!(
            "SELECT {} FROM audit_events WHERE agent_id = ?1 AND created_at >= ?2 ORDER BY created_at ASC",
            Self::AUDIT_EVENT_COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since], Self::row_to_audit_event)?
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
        let cutoff = timestamp::now_minus(Duration::days(days as i64));
        let mut stmt = self.conn.prepare(
            "SELECT
                 CAST(strftime('%Y', created_at) AS INTEGER) AS year,
                 CAST(strftime('%m', created_at) AS INTEGER) AS month,
                 COUNT(*) AS cnt,
                 GROUP_CONCAT(
                     tool_name || ': ' || target_key || ' = ' || substr(COALESCE(after_value, '(none)'), 1, 100),
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

    /// Get audit events by a list of trace_ids, ordered by id DESC for reverse-chronological reversal.
    /// Excludes events already marked as rewound.
    pub fn get_audit_events_by_trace_ids(
        &self,
        agent_id: &str,
        trace_ids: &[String],
    ) -> Result<Vec<AuditEvent>> {
        if trace_ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: Vec<String> = (0..trace_ids.len())
            .map(|i| format!("?{}", i + 2))
            .collect();
        let sql = format!(
            "SELECT {} FROM audit_events WHERE agent_id = ?1 AND trace_id IN ({}) AND rewound_by_trace_id IS NULL ORDER BY id DESC",
            Self::AUDIT_EVENT_COLS,
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
            .query_map(&*param_refs, Self::row_to_audit_event)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Mark audit events as rewound by setting rewound_by_trace_id.
    pub fn mark_audit_events_rewound(
        &self,
        agent_id: &str,
        trace_ids: &[String],
        rewind_trace_id: &str,
    ) -> Result<usize> {
        if trace_ids.is_empty() {
            return Ok(0);
        }
        let placeholders: Vec<String> = (0..trace_ids.len())
            .map(|i| format!("?{}", i + 3))
            .collect();
        let sql = format!(
            "UPDATE audit_events SET rewound_by_trace_id = ?1
             WHERE agent_id = ?2 AND trace_id IN ({}) AND rewound_by_trace_id IS NULL",
            placeholders.join(", ")
        );
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(rewind_trace_id.to_string()));
        params_vec.push(Box::new(agent_id.to_string()));
        for tid in trace_ids {
            params_vec.push(Box::new(tid.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let updated = self.conn.execute(&sql, &*param_refs)?;
        Ok(updated)
    }

    /// Count audit events for an agent with optional filters.
    pub fn count_audit_events(
        &self,
        agent_id: &str,
        tool_name: Option<&str>,
        target_key: Option<&str>,
    ) -> Result<u64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM audit_events WHERE agent_id = ?1 AND (?2 IS NULL OR tool_name = ?2) AND (?3 IS NULL OR target_key = ?3)",
            params![agent_id, tool_name, target_key],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use mika_common::auth_boundary::{AuthBoundaryError, AuthBoundaryKind};

    fn db() -> Database {
        Database::open_in_memory().unwrap()
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
            Some("New summary"),
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
                  VALUES ('mika', 's1', 'tool', 'key', 'val', '2020-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO audit_events
                  (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's2', 'tool', 'key', 'val', '2025-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        let evs = db
            .get_audit_events_since("mika", "2024-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].session_id, "s2");
    }

    #[test]
    fn test_compact_old_audit_events() {
        let db = db();
        // Insert old events
        db.conn
            .execute_batch(
                "INSERT INTO audit_events (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'k1', 'v1', '2020-01-01T00:00:00Z');
                 INSERT INTO audit_events (agent_id, session_id, tool_name, target_key, after_value, created_at)
                  VALUES ('mika', 's1', 'tool', 'k2', 'v2', '2020-01-01T00:00:01Z');",
            )
            .unwrap();
        // Insert recent event
        db.log_audit_event("mika", "s2", "tool", "k3", None, Some("v3"), None, None)
            .unwrap();
        let compacted = db.compact_old_audit_events("mika", 30).unwrap();
        assert!(compacted > 0);
        // Old events gone, recent one stays
        let recent = db.get_audit_events("mika", "s2").unwrap();
        assert_eq!(recent.len(), 1);
    }

    #[test]
    fn auth_boundary_row_carries_the_documented_tool_name_and_target_key() {
        let db = db();
        let err = AuthBoundaryError::new(
            "MIKA_INTERNAL_TOKEN",
            "gateway",
            "spirit",
            AuthBoundaryKind::Rejected,
        );
        db.record_auth_boundary("mika", &err).unwrap();

        let events = db
            .get_audit_events("mika", super::AUTH_BOUNDARY_SESSION_ID)
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tool_name, super::AUTH_BOUNDARY_TOOL_NAME);
        assert_eq!(events[0].target_key, "gateway_to_spirit");
        assert_eq!(events[0].after_value.as_deref(), Some("rejected"));
    }

    #[test]
    fn auth_boundary_reasoning_names_the_token_and_the_outcome() {
        let db = db();
        let err = AuthBoundaryError::new("INTERNAL_TOKEN", "cm", "spirit", AuthBoundaryKind::Empty);
        db.record_auth_boundary("mika", &err).unwrap();

        let events = db
            .get_audit_events("mika", super::AUTH_BOUNDARY_SESSION_ID)
            .unwrap();
        let reasoning = events[0].reasoning.clone().expect("reasoning present");
        let parsed: serde_json::Value = serde_json::from_str(&reasoning).unwrap();
        assert_eq!(parsed["token_name"], "INTERNAL_TOKEN");
        assert_eq!(parsed["kind"], "empty");
        assert_eq!(parsed["from"], "cm");
        assert_eq!(parsed["to"], "spirit");
    }

    /// Negative control for the one invariant of mika#1949: the row names the
    /// token but can never carry its value. Built with a secret-shaped *name*
    /// so any extra copy of it in the row would show up as a second match.
    #[test]
    fn auth_boundary_row_never_carries_a_token_value() {
        let db = db();
        let secret_shaped = "b".repeat(64);
        let err = AuthBoundaryError::new(
            secret_shaped.clone(),
            "manager",
            "delivery",
            AuthBoundaryKind::Missing,
        );
        db.record_auth_boundary("mika", &err).unwrap();

        let events = db
            .get_audit_events("mika", super::AUTH_BOUNDARY_SESSION_ID)
            .unwrap();
        let row = serde_json::to_string(&events[0]).unwrap();
        // Exactly one occurrence: the `token_name` slot inside `reasoning`.
        assert_eq!(row.matches(&secret_shaped).count(), 1, "{row}");
        assert_eq!(events[0].before_value, None);
    }
}
