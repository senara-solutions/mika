use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Role of a message sender.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Agent,
}

/// File content - either inline bytes (base64) or a URL reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A content part within a message or artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    #[serde(rename_all = "camelCase")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
    #[serde(rename_all = "camelCase")]
    File {
        file: FileContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
    #[serde(rename_all = "camelCase")]
    Data {
        data: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, serde_json::Value>>,
    },
}

/// Task state in the A2A state machine.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum TaskState {
    Unknown,
    Submitted,
    Working,
    InputRequired,
    AuthRequired,
    Completed,
    Failed,
    Canceled,
    Rejected,
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Submitted => "submitted",
            Self::Working => "working",
            Self::InputRequired => "input-required",
            Self::AuthRequired => "auth-required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Rejected => "rejected",
        };
        write!(f, "{s}")
    }
}

/// Current status of a task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatus {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// A message in the A2A protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub message_id: String,
    pub role: Role,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_task_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    #[serde(default = "message_kind")]
    pub kind: String,
}

fn message_kind() -> String {
    "message".to_string()
}

/// An artifact produced by an agent during task processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
}

/// A task in the A2A protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<Artifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<Message>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(default = "task_kind")]
    pub kind: String,
}

fn task_kind() -> String {
    "task".to_string()
}

/// Capabilities advertised by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,
}

/// Agent provider information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    pub organization: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A skill advertised by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_modes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_modes: Option<Vec<String>>,
}

/// Security scheme for agent authentication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecurityScheme {
    #[serde(rename = "apiKey")]
    ApiKey {
        name: String,
        #[serde(rename = "in")]
        location: String,
    },
    #[serde(rename = "http")]
    Http {
        scheme: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        bearer_format: Option<String>,
    },
    #[serde(rename = "oauth2")]
    OAuth2 { flows: serde_json::Value },
}

/// Agent Card — the public discovery document for an A2A agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub version: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,
    pub capabilities: AgentCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_schemes: Option<HashMap<String, SecurityScheme>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_requirements: Option<Vec<HashMap<String, Vec<String>>>>,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<AgentSkill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
}

/// Push notification authentication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationInfo {
    pub scheme: String,
    pub credentials: String,
}

/// Push notification configuration for a task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskPushNotificationConfig {
    pub id: String,
    pub task_id: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<AuthenticationInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_text_round_trip() {
        let part = Part::Text {
            text: "hello".to_string(),
            metadata: None,
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(json["text"], "hello");
        let deserialized: Part = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, part);
    }

    #[test]
    fn part_text_with_metadata_round_trip() {
        let mut meta = HashMap::new();
        meta.insert("key".to_string(), serde_json::json!("value"));
        let part = Part::Text {
            text: "hi".to_string(),
            metadata: Some(meta),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(json["metadata"]["key"], "value");
        let deserialized: Part = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, part);
    }

    #[test]
    fn part_file_round_trip() {
        let part = Part::File {
            file: FileContent {
                name: Some("test.txt".to_string()),
                mime_type: Some("text/plain".to_string()),
                bytes: Some("aGVsbG8=".to_string()),
                url: None,
            },
            metadata: None,
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["kind"], "file");
        assert_eq!(json["file"]["name"], "test.txt");
        assert_eq!(json["file"]["mimeType"], "text/plain");
        let deserialized: Part = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, part);
    }

    #[test]
    fn part_file_url_round_trip() {
        let part = Part::File {
            file: FileContent {
                name: None,
                mime_type: None,
                bytes: None,
                url: Some("https://example.com/file.pdf".to_string()),
            },
            metadata: None,
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["kind"], "file");
        assert_eq!(json["file"]["url"], "https://example.com/file.pdf");
        // Optional fields should be absent
        assert!(json["file"].get("name").is_none());
        let deserialized: Part = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, part);
    }

    #[test]
    fn part_data_round_trip() {
        let part = Part::Data {
            data: serde_json::json!({"numbers": [1, 2, 3]}),
            metadata: None,
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["kind"], "data");
        assert_eq!(json["data"]["numbers"], serde_json::json!([1, 2, 3]));
        let deserialized: Part = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, part);
    }

    #[test]
    fn message_round_trip() {
        let msg = Message {
            message_id: "msg-1".to_string(),
            role: Role::User,
            parts: vec![Part::Text {
                text: "hello agent".to_string(),
                metadata: None,
            }],
            context_id: Some("ctx-1".to_string()),
            task_id: None,
            metadata: None,
            reference_task_ids: None,
            extensions: None,
            kind: "message".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn message_role_serialization() {
        let json = serde_json::to_value(Role::User).unwrap();
        assert_eq!(json, "user");
        let json = serde_json::to_value(Role::Agent).unwrap();
        assert_eq!(json, "agent");
    }

    #[test]
    fn message_kind_defaults() {
        // When kind is missing from JSON, it should default to "message"
        let json = serde_json::json!({
            "messageId": "m1",
            "role": "agent",
            "parts": [{"kind": "text", "text": "hi"}]
        });
        let msg: Message = serde_json::from_value(json).unwrap();
        assert_eq!(msg.kind, "message");
    }

    #[test]
    fn task_round_trip() {
        let task = Task {
            id: "task-1".to_string(),
            context_id: Some("ctx-1".to_string()),
            status: TaskStatus {
                state: TaskState::Working,
                message: None,
                timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            },
            artifacts: None,
            history: Some(vec![Message {
                message_id: "msg-1".to_string(),
                role: Role::User,
                parts: vec![Part::Text {
                    text: "do something".to_string(),
                    metadata: None,
                }],
                context_id: None,
                task_id: Some("task-1".to_string()),
                metadata: None,
                reference_task_ids: None,
                extensions: None,
                kind: "message".to_string(),
            }]),
            metadata: None,
            kind: "task".to_string(),
        };
        let json = serde_json::to_string(&task).unwrap();
        let deserialized: Task =
            serde_json::from_value(serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(deserialized, task);
    }

    #[test]
    fn task_kind_defaults() {
        let json = serde_json::json!({
            "id": "t1",
            "status": {"state": "submitted"}
        });
        let task: Task = serde_json::from_value(json).unwrap();
        assert_eq!(task.kind, "task");
    }

    #[test]
    fn task_state_kebab_case_serialization() {
        let cases = [
            (TaskState::Unknown, "unknown"),
            (TaskState::Submitted, "submitted"),
            (TaskState::Working, "working"),
            (TaskState::InputRequired, "input-required"),
            (TaskState::AuthRequired, "auth-required"),
            (TaskState::Completed, "completed"),
            (TaskState::Failed, "failed"),
            (TaskState::Canceled, "canceled"),
            (TaskState::Rejected, "rejected"),
        ];
        for (state, expected) in &cases {
            let json = serde_json::to_value(state).unwrap();
            assert_eq!(
                json.as_str().unwrap(),
                *expected,
                "serialization of {state:?}"
            );
            let deserialized: TaskState = serde_json::from_value(json).unwrap();
            assert_eq!(deserialized, *state, "deserialization of {expected}");
        }
    }

    #[test]
    fn task_state_display() {
        assert_eq!(TaskState::InputRequired.to_string(), "input-required");
        assert_eq!(TaskState::AuthRequired.to_string(), "auth-required");
        assert_eq!(TaskState::Completed.to_string(), "completed");
    }

    #[test]
    fn agent_card_round_trip() {
        let card = AgentCard {
            name: "test-agent".to_string(),
            description: "A test agent".to_string(),
            version: "1.0.0".to_string(),
            url: "https://example.com/a2a/test-agent".to_string(),
            provider: Some(AgentProvider {
                organization: "TestOrg".to_string(),
                url: Some("https://testorg.com".to_string()),
            }),
            capabilities: AgentCapabilities {
                streaming: Some(true),
                push_notifications: Some(false),
            },
            security_schemes: None,
            security_requirements: None,
            default_input_modes: vec!["text/plain".to_string()],
            default_output_modes: vec!["text/plain".to_string()],
            skills: vec![AgentSkill {
                id: "summarize".to_string(),
                name: "Summarize".to_string(),
                description: "Summarizes text".to_string(),
                tags: vec!["nlp".to_string()],
                examples: Some(vec!["Summarize this document".to_string()]),
                input_modes: None,
                output_modes: None,
            }],
            icon_url: None,
            documentation_url: None,
        };
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, card);
    }

    #[test]
    fn agent_card_camel_case_fields() {
        let card = AgentCard {
            name: "a".to_string(),
            description: "d".to_string(),
            version: "1".to_string(),
            url: "http://x".to_string(),
            provider: None,
            capabilities: AgentCapabilities {
                streaming: Some(true),
                push_notifications: Some(true),
            },
            security_schemes: None,
            security_requirements: None,
            default_input_modes: vec![],
            default_output_modes: vec![],
            skills: vec![],
            icon_url: None,
            documentation_url: None,
        };
        let json = serde_json::to_value(&card).unwrap();
        // Verify camelCase field names
        assert!(json.get("defaultInputModes").is_some());
        assert!(json.get("defaultOutputModes").is_some());
        assert!(json["capabilities"].get("pushNotifications").is_some());
    }

    #[test]
    fn task_push_notification_config_round_trip() {
        let config = TaskPushNotificationConfig {
            id: "cfg-1".to_string(),
            task_id: "task-1".to_string(),
            url: "https://webhook.example.com/notify".to_string(),
            token: Some("secret-token".to_string()),
            authentication: Some(AuthenticationInfo {
                scheme: "Bearer".to_string(),
                credentials: "my-jwt-token".to_string(),
            }),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: TaskPushNotificationConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn task_push_notification_config_minimal() {
        let config = TaskPushNotificationConfig {
            id: "cfg-2".to_string(),
            task_id: "task-2".to_string(),
            url: "https://example.com".to_string(),
            token: None,
            authentication: None,
        };
        let json = serde_json::to_value(&config).unwrap();
        // Optional fields should be absent
        assert!(json.get("token").is_none());
        assert!(json.get("authentication").is_none());
        let deserialized: TaskPushNotificationConfig = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn artifact_round_trip() {
        let artifact = Artifact {
            artifact_id: "art-1".to_string(),
            name: Some("output.txt".to_string()),
            description: Some("Generated output".to_string()),
            parts: vec![Part::Text {
                text: "result data".to_string(),
                metadata: None,
            }],
            metadata: None,
            extensions: None,
        };
        let json = serde_json::to_string(&artifact).unwrap();
        let deserialized: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, artifact);
    }

    #[test]
    fn security_scheme_api_key() {
        let scheme = SecurityScheme::ApiKey {
            name: "x-api-key".to_string(),
            location: "header".to_string(),
        };
        let json = serde_json::to_value(&scheme).unwrap();
        assert_eq!(json["type"], "apiKey");
        assert_eq!(json["name"], "x-api-key");
        assert_eq!(json["in"], "header");
        let deserialized: SecurityScheme = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, scheme);
    }

    #[test]
    fn security_scheme_http_bearer() {
        let scheme = SecurityScheme::Http {
            scheme: "bearer".to_string(),
            bearer_format: Some("JWT".to_string()),
        };
        let json = serde_json::to_value(&scheme).unwrap();
        assert_eq!(json["type"], "http");
        assert_eq!(json["scheme"], "bearer");
        // bearer_format uses camelCase from the enum-level rename_all
        let bf_key = if json.get("bearerFormat").is_some() {
            "bearerFormat"
        } else {
            "bearer_format"
        };
        assert_eq!(json[bf_key], "JWT");
        let deserialized: SecurityScheme = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, scheme);
    }
}
