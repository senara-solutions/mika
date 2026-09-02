use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;

use mika_a2a::types::{
    Artifact, AuthenticationInfo, Message, Part, Task, TaskPushNotificationConfig, TaskState,
    TaskStatus,
};

use rusqlite::OptionalExtension;

use crate::db::Database;
use crate::timestamp;

/// Typed representation of the A2A message metadata stored in the `metadata`
/// column of the `messages` table. Deserializing directly into this struct
/// avoids an intermediate `serde_json::Value` and the deep `.clone()` calls
/// that were previously needed to extract individual fields.
#[derive(Deserialize)]
struct A2aMessageMeta {
    #[serde(default)]
    a2a_message_id: String,
    #[serde(default)]
    a2a_parts: Option<Vec<Part>>,
    #[serde(default)]
    a2a_metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Map A2A state strings to internal task status values.
fn a2a_state_to_task_status(state: &str) -> &'static str {
    match state {
        "submitted" => "pending",
        "working" => "in_progress",
        "completed" => "completed",
        "failed" => "failed",
        "canceled" => "cancelled",
        _ => "pending",
    }
}

/// Map internal task status to A2A state strings.
fn task_status_to_a2a_state(status: &str) -> &'static str {
    match status {
        "pending" => "submitted",
        "in_progress" => "working",
        "completed" | "delivered" => "completed",
        "failed" | "expired" => "failed",
        "cancelled" => "canceled",
        "blocked" => "working",
        _ => "submitted",
    }
}

/// Extract text content from A2A message parts.
pub(crate) fn extract_text_from_parts(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Database {
    // === A2A Tasks (via tasks + a2a_task_map) ===

    /// Maximum accepted length of a wire-supplied caller session id, in bytes.
    ///
    /// The id is emitted as a structured `tracing` field on every `turn_usage`
    /// event, so it is shape-checked before it reaches a log line or a query.
    const MAX_CALLER_SESSION_ID_LEN: usize = 200;

    /// Whether a wire-supplied caller session id may be adopted as this task's
    /// agent-loop session (mika#2070).
    ///
    /// True only when the id is well-formed AND names an existing session row
    /// owned by `agent_id`. A request may *name* a session, never conjure one:
    /// refusing an unknown id — rather than inserting it — is what keeps a value
    /// arriving over the network from creating rows or attaching turns to another
    /// agent's history. Every rejection is silent and falls back to the minted
    /// `a2a-<task_id>` session, because the caller needs the turn more than it
    /// needs the correlation.
    fn caller_session_is_adoptable(&self, candidate: &str, agent_id: &str) -> bool {
        if candidate.is_empty()
            || candidate.len() > Self::MAX_CALLER_SESSION_ID_LEN
            || candidate.chars().any(char::is_control)
        {
            // Length only — a malformed id must not itself reach a log field.
            tracing::debug!(
                len = candidate.len(),
                "caller session id refused: malformed"
            );
            return false;
        }
        let owner: Option<String> = match self
            .conn
            .query_row(
                "SELECT agent_id FROM sessions WHERE id = ?1",
                rusqlite::params![candidate],
                |row| row.get(0),
            )
            .optional()
        {
            Ok(owner) => owner,
            Err(e) => {
                // A lookup failure refuses the id rather than failing the turn,
                // but it is not the same thing as "no such session" — say so, or
                // a broken sessions table reads as a fleet of uncorrelated runs.
                tracing::warn!(
                    error = %e,
                    "caller session lookup failed; falling back to a minted session"
                );
                return false;
            }
        };
        match owner.as_deref() {
            Some(o) if o == agent_id => true,
            Some(o) => {
                tracing::debug!(
                    session_id = candidate,
                    owner = o,
                    agent_id,
                    "caller session id refused: owned by another agent"
                );
                false
            }
            None => {
                // Worth saying out loud: from the log alone, a refused id and a
                // client that sent nothing both look like a minted session.
                tracing::debug!(
                    session_id = candidate,
                    "caller session id refused: no such session"
                );
                false
            }
        }
    }

    /// Create an A2A task: inserts into `tasks`, `sessions`, and `a2a_task_map`.
    /// Returns the session_id for use with the agent loop.
    ///
    /// `caller_session_id` is the sender's own session id, carried across
    /// `message/send` request metadata (mika#2070). When this agent already owns
    /// that session row it becomes the returned session, so the agent loop — and
    /// the `turn_usage` event it emits — runs under the caller's session instead
    /// of a freshly minted one. This is a lookup, not an insert: the CLI and the
    /// spirit daemon share one container database, so a local caller's row is
    /// already present by the time the request lands. Anything else — absent,
    /// malformed, unknown, or owned by another agent — falls back to
    /// `a2a-<a2a_task_id>` exactly as before.
    pub fn a2a_create_task(
        &self,
        a2a_task_id: &str,
        agent_id: &str,
        context_id: Option<&str>,
        caller_session_id: Option<&str>,
    ) -> Result<String> {
        let now = timestamp::now();
        let task_id = uuid::Uuid::new_v4().to_string();

        let adopted = caller_session_id
            .filter(|sid| self.caller_session_is_adoptable(sid, agent_id))
            .map(str::to_string);
        let session_id = match adopted {
            Some(sid) => sid,
            None => {
                let minted = format!("a2a-{a2a_task_id}");
                // Create session for this A2A interaction
                self.conn.execute(
                    "INSERT INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, 'a2a')",
                    rusqlite::params![&minted, agent_id],
                )?;
                minted
            }
        };

        // Create task row in the unified tasks table
        self.conn.execute(
            "INSERT INTO tasks (id, agent_id, label, trigger_type, action_type, status,
                created_by_session, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'a2a', 'resume_agent', 'pending', ?4, ?5, ?5)",
            rusqlite::params![
                &task_id,
                agent_id,
                format!("A2A task {a2a_task_id}"),
                &session_id,
                &now,
            ],
        )?;

        // Create mapping row
        self.conn.execute(
            "INSERT INTO a2a_task_map (a2a_task_id, task_id, session_id, context_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![a2a_task_id, &task_id, &session_id, context_id, &now],
        )?;

        Ok(session_id)
    }

    /// Get the A2A state for a task (maps internal status to A2A state).
    pub fn a2a_get_task_state(&self, a2a_task_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.status FROM tasks t
             JOIN a2a_task_map m ON m.task_id = t.id
             WHERE m.a2a_task_id = ?1",
        )?;
        let result = stmt.query_row(rusqlite::params![a2a_task_id], |row| {
            row.get::<_, String>(0)
        });
        match result {
            Ok(status) => Ok(Some(task_status_to_a2a_state(&status).to_string())),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update an A2A task's state (maps A2A state to internal status).
    pub fn a2a_update_task_state(&self, a2a_task_id: &str, a2a_state: &str) -> Result<()> {
        let internal_status = a2a_state_to_task_status(a2a_state);
        let now = timestamp::now();

        let completed_at = if internal_status == "completed"
            || internal_status == "failed"
            || internal_status == "cancelled"
        {
            Some(&now)
        } else {
            None
        };

        let rows = self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2, completed_at = ?3
             WHERE id = (SELECT task_id FROM a2a_task_map WHERE a2a_task_id = ?4)",
            rusqlite::params![internal_status, &now, completed_at, a2a_task_id],
        )?;
        if rows == 0 {
            anyhow::bail!("a2a task not found: {a2a_task_id}");
        }
        Ok(())
    }

    // === A2A Messages (via messages table) ===

    /// Insert an A2A message into the unified messages table.
    pub fn a2a_insert_message(
        &self,
        a2a_task_id: &str,
        agent_id: &str,
        message: &Message,
    ) -> Result<()> {
        // Look up the session_id from the mapping
        let session_id: String = self.conn.query_row(
            "SELECT session_id FROM a2a_task_map WHERE a2a_task_id = ?1",
            rusqlite::params![a2a_task_id],
            |row| row.get(0),
        )?;

        // Map A2A role to Mika role
        let role = match message.role {
            mika_a2a::types::Role::User => "user",
            mika_a2a::types::Role::Agent => "assistant",
        };

        // Extract text content for the content column
        let content = extract_text_from_parts(&message.parts);

        // Store original A2A parts + message metadata in the metadata column
        let a2a_meta = serde_json::json!({
            "a2a_message_id": message.message_id,
            "a2a_parts": message.parts,
            "a2a_metadata": message.metadata,
            "a2a_task_id": a2a_task_id,
        });
        let metadata_str = serde_json::to_string(&a2a_meta)?;

        self.conn.execute(
            "INSERT INTO messages (session_id, agent_id, role, content, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![&session_id, agent_id, role, &content, &metadata_str],
        )?;
        Ok(())
    }

    /// Get A2A messages for a task by reading from the messages table.
    pub fn a2a_get_messages(&self, a2a_task_id: &str, limit: Option<i32>) -> Result<Vec<Message>> {
        // Look up the session_id
        let session_id: String = match self.conn.query_row(
            "SELECT session_id FROM a2a_task_map WHERE a2a_task_id = ?1",
            rusqlite::params![a2a_task_id],
            |row| row.get(0),
        ) {
            Ok(sid) => sid,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        // Scope to THIS task, not to the whole session (mika#2070). A session
        // used to belong to exactly one A2A task, so `session_id` alone was an
        // exact filter. Caller-session adoption makes that many-to-one: a second
        // `mika ask --session-id S` would otherwise return the first turn's
        // messages too, and `render_task_parts` would print both agent replies
        // concatenated. The grooming retry in `skills/bundled/_shared/dispatch-lib.sh`
        // reuses one session on purpose, so this is a live path, not a corner.
        //
        // Two producers write a task's messages, and each stamps the task
        // differently: the agent loop passes the A2A task id as `trace_id`
        // (`run_a2a_agent` sets `AgentParams.trace_id`), while `a2a_insert_message`
        // records it under `metadata.a2a_task_id`. Match either, so no historical
        // row is dropped from a task that predates adoption.
        const TASK_SCOPE: &str = "session_id = ?1 AND role IN ('user', 'assistant') \
             AND (trace_id = ?2 OR json_extract(metadata, '$.a2a_task_id') = ?2)";
        let sql = match limit {
            Some(n) => format!(
                "SELECT role, content, metadata FROM messages
                 WHERE {TASK_SCOPE} ORDER BY id ASC LIMIT {n}"
            ),
            None => format!(
                "SELECT role, content, metadata FROM messages
                 WHERE {TASK_SCOPE} ORDER BY id ASC"
            ),
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![&session_id, a2a_task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;

        let mut messages = Vec::new();
        for row in rows {
            let (role_str, content, metadata_json) = row?;

            let role = match role_str.as_str() {
                "assistant" => mika_a2a::types::Role::Agent,
                _ => mika_a2a::types::Role::User,
            };

            // Try to recover original A2A parts from metadata, fall back to text part
            let (message_id, parts, a2a_metadata) = if let Some(ref meta_str) = metadata_json {
                match serde_json::from_str::<A2aMessageMeta>(meta_str) {
                    Ok(meta) => {
                        let parts = meta.a2a_parts.unwrap_or_else(|| {
                            vec![Part::Text {
                                text: content.clone(),
                                metadata: None,
                            }]
                        });
                        (meta.a2a_message_id, parts, meta.a2a_metadata)
                    }
                    Err(_) => (
                        String::new(),
                        vec![Part::Text {
                            text: content,
                            metadata: None,
                        }],
                        None,
                    ),
                }
            } else {
                (
                    String::new(),
                    vec![Part::Text {
                        text: content,
                        metadata: None,
                    }],
                    None,
                )
            };

            messages.push(Message {
                message_id,
                role,
                parts,
                context_id: None,
                task_id: Some(a2a_task_id.to_string()),
                metadata: a2a_metadata,
                reference_task_ids: None,
                extensions: None,
                kind: "message".to_string(),
            });
        }
        Ok(messages)
    }

    // === A2A Artifacts (genuinely new — kept as-is) ===

    pub fn a2a_insert_artifact(&self, a2a_task_id: &str, artifact: &Artifact) -> Result<()> {
        let now = timestamp::now();
        let parts_json = serde_json::to_string(&artifact.parts)?;
        let metadata_json = artifact
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn.execute(
            "INSERT OR REPLACE INTO a2a_artifacts (task_id, artifact_id, name, description, parts, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![a2a_task_id, &artifact.artifact_id, &artifact.name, &artifact.description, &parts_json, &metadata_json, &now],
        )?;
        Ok(())
    }

    pub fn a2a_get_artifacts(&self, a2a_task_id: &str) -> Result<Vec<Artifact>> {
        let mut stmt = self.conn.prepare(
            "SELECT artifact_id, name, description, parts, metadata FROM a2a_artifacts WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![a2a_task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;

        let mut artifacts = Vec::new();
        for row in rows {
            let (artifact_id, name, description, parts_json, metadata_json) = row?;
            let parts: Vec<Part> = serde_json::from_str(&parts_json)?;
            let metadata = metadata_json
                .map(|m| serde_json::from_str(&m))
                .transpose()?;
            artifacts.push(Artifact {
                artifact_id,
                name,
                description,
                parts,
                metadata,
                extensions: None,
            });
        }
        Ok(artifacts)
    }

    // === A2A Push Notification Configs (genuinely new — kept as-is) ===

    pub fn a2a_set_push_config(&self, config: &TaskPushNotificationConfig) -> Result<()> {
        let now = timestamp::now();
        let (auth_scheme, auth_credentials) = match &config.authentication {
            Some(auth) => (Some(auth.scheme.clone()), Some(auth.credentials.clone())),
            None => (None, None),
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO a2a_push_notification_configs (id, task_id, url, token, auth_scheme, auth_credentials, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![&config.id, &config.task_id, &config.url, &config.token, &auth_scheme, &auth_credentials, &now],
        )?;
        Ok(())
    }

    pub fn a2a_get_push_config(&self, id: &str) -> Result<Option<TaskPushNotificationConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, url, token, auth_scheme, auth_credentials FROM a2a_push_notification_configs WHERE id = ?1",
        )?;
        let result = stmt.query_row(rusqlite::params![id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        });
        match result {
            Ok((id, task_id, url, token, auth_scheme, auth_credentials)) => {
                let authentication = match (auth_scheme, auth_credentials) {
                    (Some(scheme), Some(credentials)) => Some(AuthenticationInfo {
                        scheme,
                        credentials,
                    }),
                    _ => None,
                };
                Ok(Some(TaskPushNotificationConfig {
                    id,
                    task_id,
                    url,
                    token,
                    authentication,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn a2a_list_push_configs(
        &self,
        a2a_task_id: &str,
    ) -> Result<Vec<TaskPushNotificationConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, url, token, auth_scheme, auth_credentials FROM a2a_push_notification_configs WHERE task_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![a2a_task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;

        let mut configs = Vec::new();
        for row in rows {
            let (id, task_id, url, token, auth_scheme, auth_credentials) = row?;
            let authentication = match (auth_scheme, auth_credentials) {
                (Some(scheme), Some(credentials)) => Some(AuthenticationInfo {
                    scheme,
                    credentials,
                }),
                _ => None,
            };
            configs.push(TaskPushNotificationConfig {
                id,
                task_id,
                url,
                token,
                authentication,
            });
        }
        Ok(configs)
    }

    pub fn a2a_delete_push_config(&self, id: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "DELETE FROM a2a_push_notification_configs WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(rows > 0)
    }

    // === A2A Task Assembly ===

    /// Build a complete A2A Task from the unified tables.
    pub fn a2a_build_task(
        &self,
        a2a_task_id: &str,
        history_length: Option<i32>,
    ) -> Result<Option<Task>> {
        // Look up mapping + task status
        let mut stmt = self.conn.prepare(
            "SELECT t.status, m.context_id, t.result, t.updated_at
             FROM a2a_task_map m
             JOIN tasks t ON t.id = m.task_id
             WHERE m.a2a_task_id = ?1",
        )?;
        let row = match stmt.query_row(rusqlite::params![a2a_task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        }) {
            Ok(r) => r,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let (status, context_id, metadata_json, updated_at) = row;

        let a2a_state_str = task_status_to_a2a_state(&status);
        let state: TaskState =
            serde_json::from_value(serde_json::Value::String(a2a_state_str.to_string()))?;

        let messages = self.a2a_get_messages(a2a_task_id, history_length)?;
        let artifacts = self.a2a_get_artifacts(a2a_task_id)?;

        let metadata = metadata_json
            .map(|m| serde_json::from_str(&m))
            .transpose()?;

        // Build status with last agent message
        let last_agent_message = messages
            .iter()
            .rev()
            .find(|m| m.role == mika_a2a::types::Role::Agent)
            .cloned();
        let status = TaskStatus {
            state,
            message: last_agent_message,
            timestamp: Some(updated_at),
        };

        Ok(Some(Task {
            id: a2a_task_id.to_string(),
            context_id,
            status,
            artifacts: if artifacts.is_empty() {
                None
            } else {
                Some(artifacts)
            },
            history: if messages.is_empty() {
                None
            } else {
                Some(messages)
            },
            metadata,
            kind: "task".to_string(),
        }))
    }

    /// Get the session_id for an A2A task (for use with the agent loop).
    pub fn a2a_get_session_id(&self, a2a_task_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id FROM a2a_task_map WHERE a2a_task_id = ?1")?;
        let result = stmt.query_row(rusqlite::params![a2a_task_id], |row| {
            row.get::<_, String>(0)
        });
        match result {
            Ok(sid) => Ok(Some(sid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn make_message(id: &str, role: mika_a2a::types::Role, text: &str) -> Message {
        Message {
            message_id: id.to_string(),
            role,
            parts: vec![Part::Text {
                text: text.to_string(),
                metadata: None,
            }],
            context_id: None,
            task_id: None,
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        }
    }

    #[test]
    fn create_and_get_task_state() {
        let db = db();
        db.a2a_create_task("t1", "mika", Some("ctx-1"), None)
            .unwrap();
        let state = db.a2a_get_task_state("t1").unwrap();
        assert_eq!(state, Some("submitted".to_string()));
    }

    // --- mika#2070: caller session adoption -----------------------------------
    //
    // Since mika#1727 the CLI is a thin A2A client, so `turn_usage` carried the
    // minted `a2a-<task_id>` session and a run's turns could only be recovered by
    // time slice. These cover the one rule that fixes it: adopt the caller's
    // session when this agent already owns the row, refuse everything else
    // silently.

    /// Seed a CLI-shaped session row the way `mika ask` does before dispatching.
    fn seed_caller_session(db: &Database, session_id: &str, agent_id: &str) {
        if agent_id != "mika" {
            db.register_agent(agent_id, agent_id, "").unwrap();
        }
        db.create_session(session_id, agent_id, "cli").unwrap();
    }

    fn session_count(db: &Database) -> i64 {
        db.conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap()
    }

    fn mapped_session(db: &Database, a2a_task_id: &str) -> String {
        db.conn
            .query_row(
                "SELECT session_id FROM a2a_task_map WHERE a2a_task_id = ?1",
                rusqlite::params![a2a_task_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn caller_session_owned_by_agent_is_adopted() {
        let db = db();
        seed_caller_session(&db, "rt005-c1-r7", "mika");
        let before = session_count(&db);

        let session_id = db
            .a2a_create_task("t1", "mika", None, Some("rt005-c1-r7"))
            .unwrap();

        assert_eq!(session_id, "rt005-c1-r7");
        // Adoption is a lookup, not an insert — no second session row appears.
        assert_eq!(session_count(&db), before);
        assert_eq!(mapped_session(&db, "t1"), "rt005-c1-r7");
    }

    #[test]
    fn caller_session_owned_by_another_agent_is_refused() {
        let db = db();
        seed_caller_session(&db, "other-agents-session", "mika-dev");

        let session_id = db
            .a2a_create_task("t1", "mika", None, Some("other-agents-session"))
            .unwrap();

        assert_eq!(session_id, "a2a-t1");
        assert_eq!(mapped_session(&db, "t1"), "a2a-t1");
        // The other agent's session keeps its owner — no turns get attached to it.
        let owner: String = db
            .conn
            .query_row(
                "SELECT agent_id FROM sessions WHERE id = 'other-agents-session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, "mika-dev");
    }

    #[test]
    fn unknown_caller_session_is_refused_and_not_created() {
        let db = db();

        let session_id = db
            .a2a_create_task("t1", "mika", None, Some("does-not-exist"))
            .unwrap();

        assert_eq!(session_id, "a2a-t1");
        // A request may name a session, never conjure one.
        let exists: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = 'does-not-exist'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }

    #[test]
    fn malformed_caller_session_ids_are_refused() {
        let long_id = "x".repeat(201);
        for candidate in ["has\nnewline", "has\ttab", long_id.as_str()] {
            let db = db();
            // Seed a row that this agent DOES own under the malformed id, so the
            // ownership lookup would say yes. Only the shape check can refuse it
            // — otherwise this test would pass with the shape check deleted, and
            // a newline would reach the `turn_usage` log line as a field value.
            seed_caller_session(&db, candidate, "mika");
            let session_id = db
                .a2a_create_task("t1", "mika", None, Some(candidate))
                .unwrap();
            assert_eq!(session_id, "a2a-t1", "candidate {candidate:?} was adopted");
        }
    }

    #[test]
    fn an_empty_caller_session_id_is_refused() {
        // Split out: an empty id cannot be seeded as a session row the way the
        // other malformed candidates can, so it gets its own case.
        let db = db();
        let session_id = db.a2a_create_task("t1", "mika", None, Some("")).unwrap();
        assert_eq!(session_id, "a2a-t1");
    }

    #[test]
    fn a_caller_session_id_at_the_length_limit_is_adopted() {
        // The boundary itself: 200 bytes is accepted, 201 is not. Without this,
        // an off-by-one in the bound (`>=` for `>`) would ship green.
        let db = db();
        let at_limit = "x".repeat(200);
        seed_caller_session(&db, &at_limit, "mika");
        let session_id = db
            .a2a_create_task("t1", "mika", None, Some(&at_limit))
            .unwrap();
        assert_eq!(session_id, at_limit);
    }

    #[test]
    fn an_adopted_session_returns_only_the_current_tasks_messages() {
        // The regression adoption introduces if history is read by session:
        // `mika ask --session-id S` twice would make the second Task carry the
        // first turn's messages, and `render_task_parts` would print both agent
        // replies concatenated. `skills/bundled/_shared/dispatch-lib.sh` reuses
        // one session on its grooming retry, so this is a live path.
        let db = db();
        seed_caller_session(&db, "shared-session", "mika");

        db.a2a_create_task("task-1", "mika", None, Some("shared-session"))
            .unwrap();
        // The agent loop stamps each message with the A2A task id as trace_id.
        db.save_message(
            "mika",
            "shared-session",
            "user",
            "first ask",
            Some("task-1"),
        )
        .unwrap();
        db.save_message(
            "mika",
            "shared-session",
            "assistant",
            "first reply",
            Some("task-1"),
        )
        .unwrap();

        db.a2a_create_task("task-2", "mika", None, Some("shared-session"))
            .unwrap();
        db.save_message(
            "mika",
            "shared-session",
            "user",
            "second ask",
            Some("task-2"),
        )
        .unwrap();
        db.save_message(
            "mika",
            "shared-session",
            "assistant",
            "second reply",
            Some("task-2"),
        )
        .unwrap();

        let second = db.a2a_get_messages("task-2", None).unwrap();
        let texts: Vec<String> = second
            .iter()
            .map(|m| extract_text_from_parts(&m.parts))
            .collect();
        assert_eq!(texts, vec!["second ask", "second reply"]);

        // And the first task still reads back exactly its own turn.
        let first = db.a2a_get_messages("task-1", None).unwrap();
        let texts: Vec<String> = first
            .iter()
            .map(|m| extract_text_from_parts(&m.parts))
            .collect();
        assert_eq!(texts, vec!["first ask", "first reply"]);
    }

    #[test]
    fn absent_caller_session_keeps_prior_behavior() {
        let db = db();
        let session_id = db.a2a_create_task("t1", "mika", None, None).unwrap();

        assert_eq!(session_id, "a2a-t1");
        let channel: String = db
            .conn
            .query_row(
                "SELECT channel_type FROM sessions WHERE id = 'a2a-t1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(channel, "a2a");
    }

    #[test]
    fn get_task_state_nonexistent() {
        let db = db();
        let state = db.a2a_get_task_state("nonexistent").unwrap();
        assert!(state.is_none());
    }

    #[test]
    fn create_task_without_context() {
        let db = db();
        db.a2a_create_task("t2", "mika", None, None).unwrap();
        let state = db.a2a_get_task_state("t2").unwrap();
        assert_eq!(state, Some("submitted".to_string()));
    }

    #[test]
    fn create_task_creates_internal_task_row() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        // Verify a row exists in the tasks table with trigger_type='a2a'
        let mut stmt = db
            .conn
            .prepare("SELECT trigger_type, action_type, status FROM tasks WHERE id = (SELECT task_id FROM a2a_task_map WHERE a2a_task_id = 't1')")
            .unwrap();
        let (trigger, action, status): (String, String, String) = stmt
            .query_row([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        assert_eq!(trigger, "a2a");
        assert_eq!(action, "resume_agent");
        assert_eq!(status, "pending");
    }

    #[test]
    fn create_task_creates_session() {
        let db = db();
        let session_id = db.a2a_create_task("t1", "mika", None, None).unwrap();

        // Verify the session exists with channel_type='a2a'
        let channel: String = db
            .conn
            .query_row(
                "SELECT channel_type FROM sessions WHERE id = ?1",
                rusqlite::params![&session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(channel, "a2a");
    }

    #[test]
    fn update_task_state() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();
        db.a2a_update_task_state("t1", "working").unwrap();
        let state = db.a2a_get_task_state("t1").unwrap();
        assert_eq!(state, Some("working".to_string()));

        db.a2a_update_task_state("t1", "completed").unwrap();
        let state = db.a2a_get_task_state("t1").unwrap();
        assert_eq!(state, Some("completed".to_string()));
    }

    #[test]
    fn update_task_state_nonexistent_fails() {
        let db = db();
        let result = db.a2a_update_task_state("nonexistent", "working");
        assert!(result.is_err());
    }

    #[test]
    fn insert_and_retrieve_messages() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let msg1 = make_message("m1", mika_a2a::types::Role::User, "hello");
        let msg2 = make_message("m2", mika_a2a::types::Role::Agent, "hi there");

        db.a2a_insert_message("t1", "mika", &msg1).unwrap();
        db.a2a_insert_message("t1", "mika", &msg2).unwrap();

        let messages = db.a2a_get_messages("t1", None).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message_id, "m1");
        assert_eq!(messages[0].role, mika_a2a::types::Role::User);
        assert_eq!(messages[1].message_id, "m2");
        assert_eq!(messages[1].role, mika_a2a::types::Role::Agent);

        // Verify parts survived round-trip
        if let Part::Text { ref text, .. } = messages[0].parts[0] {
            assert_eq!(text, "hello");
        } else {
            panic!("expected Text part");
        }
    }

    #[test]
    fn messages_stored_in_unified_messages_table() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let msg = make_message("m1", mika_a2a::types::Role::User, "hello");
        db.a2a_insert_message("t1", "mika", &msg).unwrap();

        // Verify the message is in the messages table
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = 'a2a-t1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_messages_with_limit() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        for i in 0..5 {
            let msg = make_message(
                &format!("m{i}"),
                mika_a2a::types::Role::User,
                &format!("msg {i}"),
            );
            db.a2a_insert_message("t1", "mika", &msg).unwrap();
        }

        let messages = db.a2a_get_messages("t1", Some(3)).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].message_id, "m0");
        assert_eq!(messages[2].message_id, "m2");
    }

    #[test]
    fn get_messages_empty() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();
        let messages = db.a2a_get_messages("t1", None).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn insert_and_retrieve_artifacts() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let artifact = Artifact {
            artifact_id: "a1".to_string(),
            name: Some("output.txt".to_string()),
            description: Some("A result file".to_string()),
            parts: vec![Part::Text {
                text: "file content".to_string(),
                metadata: None,
            }],
            metadata: None,
            extensions: None,
        };
        db.a2a_insert_artifact("t1", &artifact).unwrap();

        let artifacts = db.a2a_get_artifacts("t1").unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_id, "a1");
        assert_eq!(artifacts[0].name, Some("output.txt".to_string()));
        assert_eq!(artifacts[0].description, Some("A result file".to_string()));
    }

    #[test]
    fn get_artifacts_empty() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();
        let artifacts = db.a2a_get_artifacts("t1").unwrap();
        assert!(artifacts.is_empty());
    }

    #[test]
    fn multiple_artifacts_for_task() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let artifact1 = Artifact {
            artifact_id: "a1".to_string(),
            name: Some("first".to_string()),
            description: None,
            parts: vec![Part::Text {
                text: "content 1".to_string(),
                metadata: None,
            }],
            metadata: None,
            extensions: None,
        };
        let artifact2 = Artifact {
            artifact_id: "a2".to_string(),
            name: Some("second".to_string()),
            description: None,
            parts: vec![Part::Text {
                text: "content 2".to_string(),
                metadata: None,
            }],
            metadata: None,
            extensions: None,
        };
        db.a2a_insert_artifact("t1", &artifact1).unwrap();
        db.a2a_insert_artifact("t1", &artifact2).unwrap();

        let artifacts = db.a2a_get_artifacts("t1").unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].artifact_id, "a1");
        assert_eq!(artifacts[1].artifact_id, "a2");
    }

    #[test]
    fn set_get_push_config() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let config = TaskPushNotificationConfig {
            id: "cfg-1".to_string(),
            task_id: "t1".to_string(),
            url: "https://webhook.example.com".to_string(),
            token: Some("tok-123".to_string()),
            authentication: Some(AuthenticationInfo {
                scheme: "Bearer".to_string(),
                credentials: "jwt-token".to_string(),
            }),
        };
        db.a2a_set_push_config(&config).unwrap();

        let retrieved = db.a2a_get_push_config("cfg-1").unwrap().unwrap();
        assert_eq!(retrieved.id, "cfg-1");
        assert_eq!(retrieved.task_id, "t1");
        assert_eq!(retrieved.url, "https://webhook.example.com");
        assert_eq!(retrieved.token, Some("tok-123".to_string()));
        let auth = retrieved.authentication.unwrap();
        assert_eq!(auth.scheme, "Bearer");
        assert_eq!(auth.credentials, "jwt-token");
    }

    #[test]
    fn get_push_config_nonexistent() {
        let db = db();
        let result = db.a2a_get_push_config("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn set_push_config_no_auth() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let config = TaskPushNotificationConfig {
            id: "cfg-2".to_string(),
            task_id: "t1".to_string(),
            url: "https://example.com".to_string(),
            token: None,
            authentication: None,
        };
        db.a2a_set_push_config(&config).unwrap();

        let retrieved = db.a2a_get_push_config("cfg-2").unwrap().unwrap();
        assert!(retrieved.authentication.is_none());
        assert!(retrieved.token.is_none());
    }

    #[test]
    fn list_push_configs() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        for i in 0..3 {
            let config = TaskPushNotificationConfig {
                id: format!("cfg-{i}"),
                task_id: "t1".to_string(),
                url: format!("https://example.com/{i}"),
                token: None,
                authentication: None,
            };
            db.a2a_set_push_config(&config).unwrap();
        }

        let configs = db.a2a_list_push_configs("t1").unwrap();
        assert_eq!(configs.len(), 3);
    }

    #[test]
    fn list_push_configs_empty() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();
        let configs = db.a2a_list_push_configs("t1").unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn delete_push_config() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let config = TaskPushNotificationConfig {
            id: "cfg-1".to_string(),
            task_id: "t1".to_string(),
            url: "https://example.com".to_string(),
            token: None,
            authentication: None,
        };
        db.a2a_set_push_config(&config).unwrap();

        let deleted = db.a2a_delete_push_config("cfg-1").unwrap();
        assert!(deleted);

        let retrieved = db.a2a_get_push_config("cfg-1").unwrap();
        assert!(retrieved.is_none());
    }

    #[test]
    fn delete_push_config_nonexistent() {
        let db = db();
        let deleted = db.a2a_delete_push_config("nonexistent").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn build_task_basic() {
        let db = db();
        db.a2a_create_task("t1", "mika", Some("ctx-1"), None)
            .unwrap();

        let msg = make_message("m1", mika_a2a::types::Role::User, "request");
        db.a2a_insert_message("t1", "mika", &msg).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        assert_eq!(task.id, "t1");
        assert_eq!(task.context_id, Some("ctx-1".to_string()));
        assert_eq!(task.status.state, TaskState::Submitted);
        assert_eq!(task.kind, "task");

        let history = task.history.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].message_id, "m1");
    }

    #[test]
    fn build_task_nonexistent() {
        let db = db();
        let task = db.a2a_build_task("nonexistent", None).unwrap();
        assert!(task.is_none());
    }

    #[test]
    fn build_task_with_artifacts() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();
        db.a2a_update_task_state("t1", "completed").unwrap();

        let artifact = Artifact {
            artifact_id: "a1".to_string(),
            name: Some("result".to_string()),
            description: None,
            parts: vec![Part::Text {
                text: "output".to_string(),
                metadata: None,
            }],
            metadata: None,
            extensions: None,
        };
        db.a2a_insert_artifact("t1", &artifact).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        assert_eq!(task.status.state, TaskState::Completed);
        let artifacts = task.artifacts.unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_id, "a1");
    }

    #[test]
    fn build_task_status_has_last_agent_message() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let user_msg = make_message("m1", mika_a2a::types::Role::User, "request");
        let agent_msg = make_message("m2", mika_a2a::types::Role::Agent, "response");
        db.a2a_insert_message("t1", "mika", &user_msg).unwrap();
        db.a2a_insert_message("t1", "mika", &agent_msg).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        let status_msg = task.status.message.unwrap();
        assert_eq!(status_msg.message_id, "m2");
        assert_eq!(status_msg.role, mika_a2a::types::Role::Agent);
    }

    #[test]
    fn build_task_with_history_length() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        for i in 0..5 {
            let msg = make_message(
                &format!("m{i}"),
                mika_a2a::types::Role::User,
                &format!("msg {i}"),
            );
            db.a2a_insert_message("t1", "mika", &msg).unwrap();
        }

        let task = db.a2a_build_task("t1", Some(2)).unwrap().unwrap();
        let history = task.history.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn build_task_empty_history_and_artifacts_are_none() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        assert!(task.history.is_none());
        assert!(task.artifacts.is_none());
    }

    #[test]
    fn a2a_task_visible_in_unified_timeline() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        // Verify the task appears in unified_timeline via the tasks leg
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM unified_timeline WHERE event_type = 'task' AND summary LIKE '%A2A task t1%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn a2a_messages_visible_in_unified_timeline() {
        let db = db();
        db.a2a_create_task("t1", "mika", None, None).unwrap();

        let msg = make_message("m1", mika_a2a::types::Role::User, "hello from a2a");
        db.a2a_insert_message("t1", "mika", &msg).unwrap();

        // Verify the message appears in unified_timeline via the messages leg
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM unified_timeline WHERE event_type = 'message' AND summary LIKE '%hello from a2a%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_session_id() {
        let db = db();
        let session_id = db.a2a_create_task("t1", "mika", None, None).unwrap();
        let retrieved = db.a2a_get_session_id("t1").unwrap().unwrap();
        assert_eq!(session_id, retrieved);
    }

    #[test]
    fn get_session_id_nonexistent() {
        let db = db();
        let result = db.a2a_get_session_id("nonexistent").unwrap();
        assert!(result.is_none());
    }
}
