use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tracing::debug;

use crate::error::A2aError;
use crate::jsonrpc::{JsonRpcRequest, JsonRpcResponse};
use crate::params::MessageSendParams;
use crate::types::Task;

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
