//! Agent execution trace for post-run assertions.

use mika_agent::agent::AgentOutput;
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{LlmCallRow, ToolCallRow};
use mika_common::llm::LlmRequest;
use mika_common::llm::mock::MockLlmProvider;

/// Complete execution trace of an agent run.
///
/// Assembled from:
/// - `AgentOutput` (final text, thinking, usage)
/// - `llm_calls` SQLite table (LLM API call records)
/// - `tool_calls` SQLite table (tool execution records)
/// - `MockLlmProvider::captured_requests()` (full LLM request payloads)
pub struct AgentTrace {
    pub output: AgentOutput,
    pub llm_calls: Vec<LlmCallRow>,
    pub tool_calls: Vec<ToolCallRow>,
    pub captured_requests: Vec<LlmRequest>,
    /// Number of LLM API calls made during the run (one per agent loop iteration).
    pub llm_call_count: usize,
}

impl AgentTrace {
    /// Build an `AgentTrace` by querying the DB and reading captured mock requests.
    ///
    /// When `mock_provider` is `None` (real-provider tests), `captured_requests` will be empty.
    ///
    /// All DB writes by `run_agent()` are synchronous — LLM calls and tool calls are
    /// persisted before `run_agent()` returns. `AgentTrace::from_run` can query them
    /// immediately without waiting or polling.
    pub async fn from_run(
        db: &AsyncDatabase,
        trace_id: &str,
        mock_provider: Option<&MockLlmProvider>,
        output: AgentOutput,
    ) -> anyhow::Result<Self> {
        let llm_calls = db.query_llm_calls_by_trace(trace_id).await?;
        let tool_calls = db.query_tool_calls_by_trace(trace_id).await?;
        let captured_requests = mock_provider
            .map(|m| m.captured_requests())
            .unwrap_or_default();
        let llm_call_count = llm_calls.len();

        Ok(Self {
            output,
            llm_calls,
            tool_calls,
            captured_requests,
            llm_call_count,
        })
    }

    /// Get the names of all tools that were called during the run.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tool_calls
            .iter()
            .map(|tc| tc.tool_name.as_str())
            .collect()
    }

    /// Get tool calls filtered by name.
    pub fn calls_for_tool(&self, name: &str) -> Vec<&ToolCallRow> {
        self.tool_calls
            .iter()
            .filter(|tc| tc.tool_name == name)
            .collect()
    }
}
