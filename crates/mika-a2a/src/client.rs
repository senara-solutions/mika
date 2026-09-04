use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tracing::debug;

use crate::error::A2aError;
use crate::jsonrpc::{JsonRpcRequest, JsonRpcResponse, TASK_NOT_FOUND};
use crate::params::{MessageSendParams, TaskQueryParams};
use crate::types::Task;

/// Default budget for a single A2A exchange.
///
/// `reqwest::Client::new()` applies no request timeout at all, so before
/// mika#2036 this client had no timeout *policy* — only whatever the OS and the
/// peer happened to do, which is not a decision.
///
/// 300 s is measured, not generous. During the founding incident the longest
/// generation that was delivered successfully took 114 s; 300 s leaves a 2.6x
/// margin for a heavier generation or a degraded network while staying well
/// short of an interval a caller would read as a hang.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Budget for a recovery read (`tasks/get`) issued after a failed exchange.
///
/// A recovery is a database read, not a generation, so it must not inherit the
/// generation-sized [`DEFAULT_TIMEOUT`]: a caller already past one failure
/// should not wait another five minutes to learn whether its answer survived.
pub const RECOVERY_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP client for calling remote A2A agents.
pub struct A2aClient {
    http: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
    timeout: Duration,
}

impl A2aClient {
    /// Create a new A2A client with the [`DEFAULT_TIMEOUT`] budget.
    ///
    /// The signature is unchanged from before mika#2036 — both existing call
    /// sites (`mika-cli/src/remote_ask.rs`, `mika-agent/src/tools/a2a_call.rs`)
    /// compile untouched and simply gain a timeout they did not have.
    pub fn new(base_url: impl Into<String>, auth_token: Option<String>) -> Self {
        Self::with_timeout(base_url, auth_token, DEFAULT_TIMEOUT)
    }

    /// Create a client with an explicit budget.
    ///
    /// A 10 KB plan review and a health probe do not have the same needs, but no
    /// caller should be forced to migrate to say so.
    pub fn with_timeout(
        base_url: impl Into<String>,
        auth_token: Option<String>,
        timeout: Duration,
    ) -> Self {
        // `reqwest::Client::new()` is itself `builder().build().expect(...)`, so
        // this expect adds no new failure mode. Falling back to a default client
        // would be worse than panicking: it would silently drop the timeout and
        // report a budget the client is not actually keeping — precisely the
        // class of lie mika#2036 exists to remove.
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            http,
            base_url: base_url.into(),
            auth_token,
            timeout,
        }
    }

    /// The budget this client actually enforces.
    ///
    /// Exposed so an error message can name the interval that was really spent
    /// instead of quoting a constant the client may not be using.
    pub fn timeout(&self) -> Duration {
        self.timeout
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

    /// Fetch a task by id (`tasks/get`).
    ///
    /// `Ok(None)` means the server has no such task — the JSON-RPC
    /// `TASK_NOT_FOUND` code, which is an *answer* ("nothing was created under
    /// that name") rather than a failure, and the caller needs to tell those
    /// apart. Every other JSON-RPC error surfaces as
    /// [`A2aError::InvalidJsonRpc`].
    ///
    /// The Mika server also accepts a caller-supplied `context_id` here, which
    /// is what makes recovery possible at all: the task id is minted server-side
    /// (`Uuid::new_v4`), so a caller that lost the `message/send` response never
    /// learned it (mika#2036).
    pub async fn get_task(
        &self,
        id: &str,
        history_length: Option<i32>,
    ) -> Result<Option<Task>, A2aError> {
        let params = TaskQueryParams {
            id: id.to_string(),
            history_length,
        };
        let request = self.build_jsonrpc_request("tasks/get", &params)?;
        let response = self.post_jsonrpc(&request).await?;

        match response.error {
            Some(err) if err.code == TASK_NOT_FOUND => Ok(None),
            Some(err) => Err(A2aError::InvalidJsonRpc(format!(
                "JSON-RPC error {}: {}",
                err.code, err.message
            ))),
            None => {
                let result = response
                    .result
                    .ok_or_else(|| A2aError::InvalidJsonRpc("missing result".to_string()))?;
                let task: Task = serde_json::from_value(result)?;
                Ok(Some(task))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// AC2: the budget is a named decision in the code, not `reqwest`'s default
    /// (which is *no* timeout at all).
    #[test]
    fn default_timeout_is_the_measured_300_seconds() {
        assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(300));
    }

    /// A recovery read must not inherit the generation-sized budget.
    #[test]
    fn recovery_budget_is_shorter_than_the_generation_budget() {
        assert!(RECOVERY_TIMEOUT < DEFAULT_TIMEOUT);
    }

    /// AC2: `new` keeps its signature and gains the default; `with_timeout`
    /// overrides it. Two clients built with different budgets must actually
    /// carry them — a `timeout()` that returned a constant would pass a
    /// single-client test and fail this one.
    #[test]
    fn clients_carry_the_budget_they_were_built_with() {
        let default = A2aClient::new("http://x/a2a/agent", None);
        assert_eq!(default.timeout(), DEFAULT_TIMEOUT);

        let short = A2aClient::with_timeout("http://x/a2a/agent", None, Duration::from_secs(2));
        let long = A2aClient::with_timeout("http://x/a2a/agent", None, Duration::from_secs(120));
        assert_eq!(short.timeout(), Duration::from_secs(2));
        assert_eq!(long.timeout(), Duration::from_secs(120));
        assert_ne!(short.timeout(), long.timeout());
    }
}
