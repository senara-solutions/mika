use anyhow::Result;
use mika_a2a::types::{
    Artifact, AuthenticationInfo, Message, Part, Task, TaskPushNotificationConfig, TaskState,
    TaskStatus,
};

use crate::db::Database;
use crate::timestamp;

impl Database {
    // === A2A Tasks ===

    pub fn a2a_create_task(&self, id: &str, context_id: Option<&str>) -> Result<()> {
        let now = timestamp::now();
        self.conn.execute(
            "INSERT INTO a2a_tasks (id, context_id, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, context_id, "submitted", &now, &now],
        )?;
        Ok(())
    }

    pub fn a2a_get_task_state(&self, id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT state FROM a2a_tasks WHERE id = ?1")?;
        let result = stmt.query_row(rusqlite::params![id], |row| row.get::<_, String>(0));
        match result {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn a2a_update_task_state(&self, id: &str, state: &str) -> Result<()> {
        let now = timestamp::now();
        let rows = self.conn.execute(
            "UPDATE a2a_tasks SET state = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![state, &now, id],
        )?;
        if rows == 0 {
            anyhow::bail!("a2a task not found: {id}");
        }
        Ok(())
    }

    pub fn a2a_update_task_metadata(&self, id: &str, metadata: Option<&str>) -> Result<()> {
        let now = timestamp::now();
        self.conn.execute(
            "UPDATE a2a_tasks SET metadata = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![metadata, &now, id],
        )?;
        Ok(())
    }

    // === A2A Messages ===

    pub fn a2a_insert_message(&self, task_id: &str, message: &Message) -> Result<()> {
        let now = timestamp::now();
        let parts_json = serde_json::to_string(&message.parts)?;
        let metadata_json = message
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let role = match message.role {
            mika_a2a::types::Role::User => "user",
            mika_a2a::types::Role::Agent => "agent",
        };
        self.conn.execute(
            "INSERT INTO a2a_messages (task_id, message_id, role, parts, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![task_id, &message.message_id, role, &parts_json, &metadata_json, &now],
        )?;
        Ok(())
    }

    pub fn a2a_get_messages(&self, task_id: &str, limit: Option<i32>) -> Result<Vec<Message>> {
        let sql = match limit {
            Some(n) => format!(
                "SELECT message_id, role, parts, metadata FROM a2a_messages WHERE task_id = ?1 ORDER BY id ASC LIMIT {n}"
            ),
            None => "SELECT message_id, role, parts, metadata FROM a2a_messages WHERE task_id = ?1 ORDER BY id ASC".to_string(),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;

        let mut messages = Vec::new();
        for row in rows {
            let (message_id, role, parts_json, metadata_json) = row?;
            let role = match role.as_str() {
                "agent" => mika_a2a::types::Role::Agent,
                _ => mika_a2a::types::Role::User,
            };
            let parts: Vec<Part> = serde_json::from_str(&parts_json)?;
            let metadata = metadata_json
                .map(|m| serde_json::from_str(&m))
                .transpose()?;
            messages.push(Message {
                message_id,
                role,
                parts,
                context_id: None,
                task_id: Some(task_id.to_string()),
                metadata,
                reference_task_ids: None,
                extensions: None,
                kind: "message".to_string(),
            });
        }
        Ok(messages)
    }

    // === A2A Artifacts ===

    pub fn a2a_insert_artifact(&self, task_id: &str, artifact: &Artifact) -> Result<()> {
        let now = timestamp::now();
        let parts_json = serde_json::to_string(&artifact.parts)?;
        let metadata_json = artifact
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn.execute(
            "INSERT OR REPLACE INTO a2a_artifacts (task_id, artifact_id, name, description, parts, metadata, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![task_id, &artifact.artifact_id, &artifact.name, &artifact.description, &parts_json, &metadata_json, &now],
        )?;
        Ok(())
    }

    pub fn a2a_get_artifacts(&self, task_id: &str) -> Result<Vec<Artifact>> {
        let mut stmt = self.conn.prepare(
            "SELECT artifact_id, name, description, parts, metadata FROM a2a_artifacts WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![task_id], |row| {
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

    // === A2A Push Notification Configs ===

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

    pub fn a2a_list_push_configs(&self, task_id: &str) -> Result<Vec<TaskPushNotificationConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, url, token, auth_scheme, auth_credentials FROM a2a_push_notification_configs WHERE task_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![task_id], |row| {
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

    /// Build a complete A2A Task from the database tables.
    pub fn a2a_build_task(&self, id: &str, history_length: Option<i32>) -> Result<Option<Task>> {
        let state_opt = self.a2a_get_task_state(id)?;
        let state_str = match state_opt {
            Some(s) => s,
            None => return Ok(None),
        };

        let state: TaskState = serde_json::from_value(serde_json::Value::String(state_str))?;
        let messages = self.a2a_get_messages(id, history_length)?;
        let artifacts = self.a2a_get_artifacts(id)?;

        // Get context_id and metadata from a2a_tasks
        let mut stmt = self
            .conn
            .prepare("SELECT context_id, metadata, updated_at FROM a2a_tasks WHERE id = ?1")?;
        let (context_id, metadata_json, updated_at): (Option<String>, Option<String>, String) =
            stmt.query_row(rusqlite::params![id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

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
            id: id.to_string(),
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
        db.a2a_create_task("t1", Some("ctx-1")).unwrap();
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
        db.a2a_create_task("t2", None).unwrap();
        let state = db.a2a_get_task_state("t2").unwrap();
        assert_eq!(state, Some("submitted".to_string()));
    }

    #[test]
    fn update_task_state() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();
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
        db.a2a_create_task("t1", None).unwrap();

        let msg1 = make_message("m1", mika_a2a::types::Role::User, "hello");
        let msg2 = make_message("m2", mika_a2a::types::Role::Agent, "hi there");

        db.a2a_insert_message("t1", &msg1).unwrap();
        db.a2a_insert_message("t1", &msg2).unwrap();

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
    fn get_messages_with_limit() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

        for i in 0..5 {
            let msg = make_message(
                &format!("m{i}"),
                mika_a2a::types::Role::User,
                &format!("msg {i}"),
            );
            db.a2a_insert_message("t1", &msg).unwrap();
        }

        let messages = db.a2a_get_messages("t1", Some(3)).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].message_id, "m0");
        assert_eq!(messages[2].message_id, "m2");
    }

    #[test]
    fn get_messages_empty() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();
        let messages = db.a2a_get_messages("t1", None).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn insert_and_retrieve_artifacts() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

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
        db.a2a_create_task("t1", None).unwrap();
        let artifacts = db.a2a_get_artifacts("t1").unwrap();
        assert!(artifacts.is_empty());
    }

    #[test]
    fn multiple_artifacts_for_task() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

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
        db.a2a_create_task("t1", None).unwrap();

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
        db.a2a_create_task("t1", None).unwrap();

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
        db.a2a_create_task("t1", None).unwrap();

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
        db.a2a_create_task("t1", None).unwrap();
        let configs = db.a2a_list_push_configs("t1").unwrap();
        assert!(configs.is_empty());
    }

    #[test]
    fn delete_push_config() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

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
        db.a2a_create_task("t1", Some("ctx-1")).unwrap();

        let msg = make_message("m1", mika_a2a::types::Role::User, "request");
        db.a2a_insert_message("t1", &msg).unwrap();

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
        db.a2a_create_task("t1", None).unwrap();
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
        db.a2a_create_task("t1", None).unwrap();

        let user_msg = make_message("m1", mika_a2a::types::Role::User, "request");
        let agent_msg = make_message("m2", mika_a2a::types::Role::Agent, "response");
        db.a2a_insert_message("t1", &user_msg).unwrap();
        db.a2a_insert_message("t1", &agent_msg).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        let status_msg = task.status.message.unwrap();
        assert_eq!(status_msg.message_id, "m2");
        assert_eq!(status_msg.role, mika_a2a::types::Role::Agent);
    }

    #[test]
    fn build_task_with_history_length() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

        for i in 0..5 {
            let msg = make_message(
                &format!("m{i}"),
                mika_a2a::types::Role::User,
                &format!("msg {i}"),
            );
            db.a2a_insert_message("t1", &msg).unwrap();
        }

        let task = db.a2a_build_task("t1", Some(2)).unwrap().unwrap();
        let history = task.history.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn build_task_empty_history_and_artifacts_are_none() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        assert!(task.history.is_none());
        assert!(task.artifacts.is_none());
    }

    #[test]
    fn update_task_metadata() {
        let db = db();
        db.a2a_create_task("t1", None).unwrap();

        let meta = r#"{"key":"value"}"#;
        db.a2a_update_task_metadata("t1", Some(meta)).unwrap();

        let task = db.a2a_build_task("t1", None).unwrap().unwrap();
        let metadata = task.metadata.unwrap();
        assert_eq!(metadata["key"], serde_json::json!("value"));
    }
}
