//! Agent execution trace for post-run assertions.

use mika_agent::agent::AgentOutput;
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{LlmCallRow, ToolCallRow};
use mika_common::llm::mock::MockLlmProvider;
use mika_common::llm::{LlmProvider, LlmRequest};

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
    pub steps: usize,
    pub provider: String,
    pub model: String,
}

impl AgentTrace {
    /// Build an `AgentTrace` by querying the DB and reading captured mock requests.
    pub async fn from_run(
        db: &AsyncDatabase,
        trace_id: &str,
        mock_provider: &MockLlmProvider,
        output: AgentOutput,
    ) -> anyhow::Result<Self> {
        let llm_calls = db.query_llm_calls_by_trace(trace_id).await?;
        let tool_calls = db.query_tool_calls_by_trace(trace_id).await?;
        let captured_requests = mock_provider.captured_requests();
        let steps = llm_calls.len();

        Ok(Self {
            output,
            llm_calls,
            tool_calls,
            captured_requests,
            steps,
            provider: mock_provider.provider_name().to_string(),
            model: mock_provider.model_name().to_string(),
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
