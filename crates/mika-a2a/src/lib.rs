pub mod client;
pub mod error;
pub mod jsonrpc;
pub mod params;
pub mod state_machine;
pub mod streaming;
pub mod types;

pub use error::A2aError;
pub use jsonrpc::{A2aMethod, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse};
pub use params::{MessageSendParams, SendMessageConfiguration, TaskIdParams, TaskQueryParams};
pub use state_machine::TaskStateMachine;
pub use streaming::{StreamEvent, TaskArtifactUpdateEvent, TaskStatusUpdateEvent};
pub use types::{
    AgentCapabilities, AgentCard, AgentProvider, AgentSkill, Artifact, AuthenticationInfo,
    FileContent, Message, Part, Role, Task, TaskPushNotificationConfig, TaskState, TaskStatus,
};
