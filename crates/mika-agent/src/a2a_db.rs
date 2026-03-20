use anyhow::Result;
use mika_a2a::types::{
    Artifact, AuthenticationInfo, Message, Part, Task, TaskPushNotificationConfig, TaskState,
    TaskStatus,
};

use crate::db::Database;
use crate::timestamp;

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
fn extract_text_from_parts(parts: &[Part]) -> String {
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

    /// Create an A2A task: inserts into `tasks`, `sessions`, and `a2a_task_map`.
    /// Returns the session_id for use with the agent loop.
    pub fn a2a_create_task(
        &self,
        a2a_task_id: &str,
        agent_id: &str,
        context_id: Option<&str>,
    ) -> Result<String> {
        let now = timestamp::now();
        let task_id = uuid::Uuid::new_v4().to_string();
        let session_id = format!("a2a-{a2a_task_id}");

        // Create session for this A2A interaction
        self.conn.execute(
            "INSERT INTO sessions (id, agent_id, channel_type) VALUES (?1, ?2, 'a2a')",
            rusqlite::params![&session_id, agent_id],
        )?;

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

    /// Update A2A task metadata (stored in the tasks.result column as JSON).
    pub fn a2a_update_task_metadata(
        &self,
        a2a_task_id: &str,
        metadata: Option<&str>,
    ) -> Result<()> {
        let now = timestamp::now();
        self.conn.execute(
            "UPDATE tasks SET result = ?1, updated_at = ?2
             WHERE id = (SELECT task_id FROM a2a_task_map WHERE a2a_task_id = ?3)",
            rusqlite::params![metadata, &now, a2a_task_id],
        )?;
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

        let sql = match limit {
            Some(n) => format!(
                "SELECT role, content, metadata FROM messages
                 WHERE session_id = ?1 AND role IN ('user', 'assistant')
                 ORDER BY id ASC LIMIT {n}"
            ),
            None => "SELECT role, content, metadata FROM messages
                     WHERE session_id = ?1 AND role IN ('user', 'assistant')
                     ORDER BY id ASC"
                .to_string(),
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![&session_id], |row| {
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
                if let Ok(meta) = serde_json::from_str::<serde_json::Value>(meta_str) {
                    let mid = meta
                        .get("a2a_message_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let p = meta
                        .get("a2a_parts")
                        .and_then(|v| serde_json::from_value::<Vec<Part>>(v.clone()).ok())
                        .unwrap_or_else(|| {
                            vec![Part::Text {
                                text: content.clone(),
                                metadata: None,
                            }]
                        });
                    let am = meta.get("a2a_metadata").and_then(|v| {
                        if v.is_null() {
                            None
                        } else {
                            serde_json::from_value(v.clone()).ok()
                        }
                    });
                    (mid, p, am)
                } else {
                    (
                        String::new(),
                        vec![Part::Text {
                            text: content,
                            metadata: None,
                        }],
                        None,
                    )
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
        db.a2a_create_task("t1", "mika", Some("ctx-1")).unwrap();
        let state = db.a2a_get_task_state("t1").unwrap();
        assert_eq!(state, Some("submitted".to_string()));
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
        db.a2a_create_task("t2", "mika", None).unwrap();
        let state = db.a2a_get_task_state("t2").unwrap();
        assert_eq!(state, Some("submitted".to_string()));
    }

    #[test]
    fn create_task_creates_internal_task_row() {
        let db = db();
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        let session_id = db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();
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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();
        let messages = db.a2a_get_messages("t1", None).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn insert_and_retrieve_artifacts() {
        let db = db();
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();
        let artifacts = db.a2a_get_artifacts("t1").unwrap();
        assert!(artifacts.is_empty());
    }

    #[test]
    fn multiple_artifacts_for_task() {
        let db = db();
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();
        let configs = db.a2a_list_push_configs("t1").unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn delete_push_config() {
        let db = db();
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", Some("ctx-1")).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();
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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        assert!(task.history.is_none());
        assert!(task.artifacts.is_none());
    }

    #[test]
    fn update_task_metadata() {
        let db = db();
        db.a2a_create_task("t1", "mika", None).unwrap();

        let meta = r#"{"key":"value"}"#;
        db.a2a_update_task_metadata("t1", Some(meta)).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        let metadata = task.metadata.unwrap();
        assert_eq!(metadata["key"], serde_json::json!("value"));
    }

    #[test]
    fn a2a_task_visible_in_unified_timeline() {
        let db = db();
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        db.a2a_create_task("t1", "mika", None).unwrap();

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
        let session_id = db.a2a_create_task("t1", "mika", None).unwrap();
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
