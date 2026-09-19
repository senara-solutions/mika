pub mod client;
pub mod error;
pub mod jsonrpc;
pub mod params;
pub mod render;
pub mod state_machine;
pub mod streaming;
pub mod types;

pub use error::A2aError;
pub use jsonrpc::{A2aMethod, JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse};
pub use params::{
    CALLER_SESSION_ID_KEY, EFFECTIVE_MODEL_KEY, MODEL_OVERRIDE_KEY, MessageSendParams,
    ONLY_SKILLS_KEY, SendMessageConfiguration, TaskIdParams, TaskQueryParams, attested_model,
};
pub use render::{EmptyKind, TaskRenderEmpty, render_task_text};
pub use state_machine::TaskStateMachine;
pub use streaming::{
    StreamEvent, StreamEventSender, TaskArtifactUpdateEvent, TaskStatusUpdateEvent,
    ToolCallResultEvent, ToolCallStartEvent, ToolCallStreamContext,
};
pub use types::{
    AgentCapabilities, AgentCard, AgentProvider, AgentSkill, Artifact, AuthenticationInfo,
    FileContent, Message, Part, Role, Task, TaskPushNotificationConfig, TaskState, TaskStatus,
};
