use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use tokio_stream::Stream;
use tracing::{debug, warn};

use crate::error::A2aError;
use crate::jsonrpc::{JsonRpcRequest, JsonRpcResponse};
use crate::params::{MessageSendParams, TaskIdParams, TaskQueryParams};
use crate::streaming::StreamEvent;
use crate::types::{AgentCard, Task};

/// HTTP client for calling remote A2A agents.
pub struct A2aClient {
    http: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
}

impl A2aClient {
    /// Create a new A2A client.
    pub fn new(base_url: impl Into<String>, auth_token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
            auth_token,
        }
    }

    /// Create a client with a shared reqwest::Client.
    pub fn with_http_client(
        http: reqwest::Client,
        base_url: impl Into<String>,
        auth_token: Option<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            auth_token,
        }
    }

    /// Fetch the agent card from the well-known URL.
    pub async fn get_agent_card(&self) -> Result<AgentCard, A2aError> {
        let url = format!("{}/.well-known/agent.json", self.base_url);
        debug!(url = %url, "fetching agent card");

        let resp = self.http.get(&url).send().await?.error_for_status()?;
        let card: AgentCard = resp.json().await?;
        Ok(card)
    }

    /// Send a message synchronously (message/send).
    pub async fn send_message(&self, params: MessageSendParams) -> Result<Task, A2aError> {
        let request = self.build_jsonrpc_request("message/send", &params)?;
        let response = self.post_jsonrpc(&request).await?;

        match response.error {
            Some(err) => Err(A2aError::InvalidJsonRpc(format!(
                "JSON-RPC error {}: {}",
                err.code, err.message
            ))),
            None => {
                let result = response
                    .result
                    .ok_or_else(|| A2aError::InvalidJsonRpc("missing result".to_string()))?;
                let task: Task = serde_json::from_value(result)?;
                Ok(task)
            }
        }
    }

    /// Send a message with streaming (message/stream). Returns a stream of events.
    pub async fn send_message_streaming(
        &self,
        params: MessageSendParams,
    ) -> Result<impl Stream<Item = Result<StreamEvent, A2aError>>, A2aError> {
        let request = self.build_jsonrpc_request("message/stream", &params)?;
        let body = serde_json::to_string(&request)?;

        let mut req = self
            .http
            .post(&self.base_url)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .body(body);

        if let Some(ref token) = self.auth_token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }

        let resp = req.send().await?.error_for_status()?;
        let byte_stream = resp.bytes_stream();

        Ok(parse_sse_stream(byte_stream))
    }

    /// Get a task by ID.
    pub async fn get_task(
        &self,
        id: impl Into<String>,
        history_length: Option<i32>,
    ) -> Result<Task, A2aError> {
        let params = TaskQueryParams {
            id: id.into(),
            history_length,
        };
        let request = self.build_jsonrpc_request("tasks/get", &params)?;
        let response = self.post_jsonrpc(&request).await?;

        match response.error {
            Some(err) => Err(A2aError::InvalidJsonRpc(format!(
                "JSON-RPC error {}: {}",
                err.code, err.message
            ))),
            None => {
                let result = response
                    .result
                    .ok_or_else(|| A2aError::InvalidJsonRpc("missing result".to_string()))?;
                let task: Task = serde_json::from_value(result)?;
                Ok(task)
            }
        }
    }

    /// Cancel a task.
    pub async fn cancel_task(&self, id: impl Into<String>) -> Result<Task, A2aError> {
        let params = TaskIdParams { id: id.into() };
        let request = self.build_jsonrpc_request("tasks/cancel", &params)?;
        let response = self.post_jsonrpc(&request).await?;

        match response.error {
            Some(err) => Err(A2aError::InvalidJsonRpc(format!(
                "JSON-RPC error {}: {}",
                err.code, err.message
            ))),
            None => {
                let result = response
                    .result
                    .ok_or_else(|| A2aError::InvalidJsonRpc("missing result".to_string()))?;
                let task: Task = serde_json::from_value(result)?;
                Ok(task)
            }
        }
    }

    fn build_jsonrpc_request<T: serde::Serialize>(
        &self,
        method: &str,
        params: &T,
    ) -> Result<JsonRpcRequest, A2aError> {
        Ok(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params: serde_json::to_value(params)?,
            id: Some(crate::jsonrpc::JsonRpcId::String(
                uuid::Uuid::new_v4().to_string(),
            )),
        })
    }

    async fn post_jsonrpc(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, A2aError> {
        let mut req = self
            .http
            .post(&self.base_url)
            .header(CONTENT_TYPE, "application/json")
            .json(request);

        if let Some(ref token) = self.auth_token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }

        debug!(method = %request.method, "sending JSON-RPC request");
        let resp = req.send().await?.error_for_status()?;
        let response: JsonRpcResponse = resp.json().await?;
        Ok(response)
    }
}

/// Parse an SSE byte stream into a stream of A2A events.
///
/// Spawns a background task that reads chunks from the byte stream,
/// parses SSE frames, and sends parsed events through a channel.
fn parse_sse_stream(
    byte_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent, A2aError>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent, A2aError>>(64);

    tokio::spawn(async move {
        use tokio_stream::StreamExt as _;

        let mut buffer = String::new();
        tokio::pin!(byte_stream);

        while let Some(chunk) = byte_stream.next().await {
            let events = match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));

                    let mut events = Vec::new();
                    while let Some(pos) = buffer.find("\n\n") {
                        let event_str = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        // Parse SSE data lines
                        let data: String = event_str
                            .lines()
                            .filter_map(|line| line.strip_prefix("data: "))
                            .collect::<Vec<_>>()
                            .join("\n");

                        if !data.is_empty() {
                            match serde_json::from_str::<StreamEvent>(&data) {
                                Ok(event) => events.push(Ok(event)),
                                Err(e) => {
                                    warn!(error = %e, data = %data, "failed to parse SSE event");
                                    events.push(Err(A2aError::SerializationError(e)));
                                }
                            }
                        }
                    }
                    events
                }
                Err(e) => vec![Err(A2aError::ClientError(e))],
            };

            for event in events {
                if tx.send(event).await.is_err() {
                    return; // receiver dropped
                }
            }
        }
    });

    tokio_stream::wrappers::ReceiverStream::new(rx)
}
