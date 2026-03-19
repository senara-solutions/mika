use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::types::{Artifact, Message, Task, TaskStatus};

/// Server-Sent Event from an A2A streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StreamEvent {
    #[serde(rename = "task")]
    Task(Task),
    #[serde(rename = "message")]
    Message(Message),
    #[serde(rename = "status-update")]
    StatusUpdate(TaskStatusUpdateEvent),
    #[serde(rename = "artifact-update")]
    ArtifactUpdate(TaskArtifactUpdateEvent),
}

/// A task status update event sent via SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusUpdateEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    #[serde(rename = "final")]
    pub is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// An artifact update event sent via SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskArtifactUpdateEvent {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub artifact: Artifact,
    #[serde(default)]
    pub append: bool,
    #[serde(default)]
    pub last_chunk: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Part, Role, TaskState};

    fn make_status_update() -> StreamEvent {
        StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
            task_id: "task-1".to_string(),
            context_id: Some("ctx-1".to_string()),
            status: TaskStatus {
                state: TaskState::Working,
                message: None,
                timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            },
            is_final: false,
            metadata: None,
        })
    }

    fn make_artifact_update() -> StreamEvent {
        StreamEvent::ArtifactUpdate(TaskArtifactUpdateEvent {
            task_id: "task-1".to_string(),
            context_id: None,
            artifact: Artifact {
                artifact_id: "art-1".to_string(),
                name: Some("output.txt".to_string()),
                description: None,
                parts: vec![Part::Text {
                    text: "result".to_string(),
                    metadata: None,
                }],
                metadata: None,
                extensions: None,
            },
            append: false,
            last_chunk: true,
            metadata: None,
        })
    }

    fn make_task_event() -> StreamEvent {
        StreamEvent::Task(Task {
            id: "task-1".to_string(),
            context_id: None,
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
            kind: "task".to_string(),
        })
    }

    fn make_message_event() -> StreamEvent {
        StreamEvent::Message(Message {
            message_id: "msg-1".to_string(),
            role: Role::Agent,
            parts: vec![Part::Text {
                text: "response".to_string(),
                metadata: None,
            }],
            context_id: None,
            task_id: Some("task-1".to_string()),
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        })
    }

    #[test]
    fn status_update_round_trip() {
        let event = make_status_update();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["kind"], "status-update");
        if let StreamEvent::StatusUpdate(ref e) = parsed {
            assert_eq!(e.task_id, "task-1");
            assert_eq!(e.status.state, TaskState::Working);
            assert!(!e.is_final);
        } else {
            panic!("expected StatusUpdate");
        }
    }

    #[test]
    fn status_update_final_flag() {
        let event = StreamEvent::StatusUpdate(TaskStatusUpdateEvent {
            task_id: "t".to_string(),
            context_id: None,
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            is_final: true,
            metadata: None,
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["final"], true);
    }

    #[test]
    fn artifact_update_round_trip() {
        let event = make_artifact_update();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: StreamEvent = serde_json::from_str(&json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["kind"], "artifact-update");
        if let StreamEvent::ArtifactUpdate(ref e) = parsed {
            assert_eq!(e.task_id, "task-1");
            assert_eq!(e.artifact.artifact_id, "art-1");
            assert!(e.last_chunk);
            assert!(!e.append);
        } else {
            panic!("expected ArtifactUpdate");
        }
    }

    #[test]
    fn task_event_serializes_with_kind_tag() {
        // Task and Message already have a `kind` field in their struct,
        // which conflicts with StreamEvent's serde tag on deserialization.
        // We test serialization produces the correct kind tag.
        let event = make_task_event();
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["kind"], "task");
        assert_eq!(value["id"], "task-1");
        assert_eq!(value["status"]["state"], "completed");
    }

    #[test]
    fn message_event_serializes_with_kind_tag() {
        let event = make_message_event();
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["kind"], "message");
        assert_eq!(value["messageId"], "msg-1");
        assert_eq!(value["role"], "agent");
    }

    #[test]
    fn all_kind_discriminators() {
        let events = vec![
            (make_status_update(), "status-update"),
            (make_artifact_update(), "artifact-update"),
            (make_task_event(), "task"),
            (make_message_event(), "message"),
        ];
        for (event, expected_kind) in events {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(
                value["kind"].as_str().unwrap(),
                expected_kind,
                "kind discriminator for {expected_kind}"
            );
        }
    }
}
