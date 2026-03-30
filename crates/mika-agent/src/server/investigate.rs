//! Investigation panel — SSE endpoint and lightweight agent loop for
//! analyzing agent behavior from the observability dashboard.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response, sse},
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info};

use mika_common::claude::ToolDefinition;
use mika_common::llm::{
    LlmContent, LlmContentBlock, LlmMessage, LlmProvider, LlmRequest, LlmResponseContent, LlmRole,
    LlmStopReason, LlmToolResultContent, response_content_to_blocks,
};

use crate::async_db::AsyncDatabase;
use crate::db::SessionMessage;
use crate::tools::{Tool, ToolOutput, ToolRegistry};

use super::state::AppState;

// ===== Configuration =====

/// Max tool-dispatch steps per investigation.
const MAX_INVESTIGATION_STEPS: u32 = 5;
/// Total timeout for the investigation loop.
const INVESTIGATION_TIMEOUT: Duration = Duration::from_secs(120);
/// Per-tool timeout for investigation tools.
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
/// Max question length in characters.
const MAX_QUESTION_LEN: usize = 4_000;
/// Max history turns.
const MAX_HISTORY_TURNS: usize = 10;
/// Max investigation request body size (64 KB).
pub const INVESTIGATE_BODY_LIMIT: usize = 64 * 1024;

// ===== SSE Event Types =====

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
enum InvestigateEvent {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        name: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    #[serde(rename = "done")]
    Done {},
    #[serde(rename = "error")]
    Error { message: String },
}

// ===== Request Types =====

#[derive(Debug, Deserialize)]
pub struct InvestigateRequest {
    pub message_id: i64,
    pub tool_call_index: Option<usize>,
    pub question: String,
    #[serde(default)]
    pub history: Vec<HistoryTurn>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryTurn {
    pub role: String,
    pub content: String,
}

// ===== Investigation Tools =====

/// Tool: query the unified timeline.
struct QueryTimelineTool;

#[async_trait::async_trait]
impl Tool for QueryTimelineTool {
    fn name(&self) -> &str {
        "query_timeline"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "query_timeline".to_string(),
            description: "Query the unified event timeline. Returns messages, audit events, and tasks across all agents.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "event_type": { "type": "string", "description": "Filter by event type (message, audit, task)" },
                    "trace_id": { "type": "string", "description": "Filter by trace ID" },
                    "limit": { "type": "integer", "description": "Max results (default 20, max 50)" }
                },
                "required": []
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &crate::tools::ToolContext<'_>,
    ) -> Result<ToolOutput> {
        // Access the investigation context from the db field (it's the dashboard_db)
        let agent_id = input["agent_id"].as_str().map(|s| s.to_owned());
        let event_type = input["event_type"].as_str().map(|s| s.to_owned());
        let trace_id = input["trace_id"].as_str().map(|s| s.to_owned());
        let limit = input["limit"].as_u64().unwrap_or(20).min(50) as u32;

        let filters = crate::db::TimelineFilters {
            agent_id,
            event_type,
            trace_id,
            session_id: None,
            from: None,
            to: None,
        };

        let rows = ctx.db.query_timeline(filters, limit, 0).await?;
        let mut output = format!("Found {} timeline events:\n\n", rows.len());
        for row in &rows {
            output.push_str(&format!(
                "- [{}] {} / {} | agent={} | {}\n",
                row.created_at,
                row.event_type,
                row.event_subtype,
                row.agent_id.as_deref().unwrap_or("-"),
                row.summary.as_deref().unwrap_or("(no summary)"),
            ));
        }
        Ok(ToolOutput::success(output))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Tool: query messages from a session.
struct QueryMessagesTool;

#[async_trait::async_trait]
impl Tool for QueryMessagesTool {
    fn name(&self) -> &str {
        "query_messages"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "query_messages".to_string(),
            description: "Load messages from a specific session. Returns role, content, metadata, and timestamps.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session ID to query" },
                    "limit": { "type": "integer", "description": "Max messages (default 20, max 50)" },
                    "offset": { "type": "integer", "description": "Offset for pagination (default 0)" }
                },
                "required": ["session_id"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &crate::tools::ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let session_id = input["session_id"].as_str().unwrap_or("").to_owned();
        if session_id.is_empty() {
            return Ok(ToolOutput::error("session_id is required"));
        }
        let limit = input["limit"].as_u64().unwrap_or(20).min(50) as u32;
        let offset = input["offset"].as_u64().unwrap_or(0) as u32;

        let msgs = ctx
            .db
            .load_session_messages_paginated(&session_id, limit, offset)
            .await?;
        let mut output = format!(
            "Found {} messages in session {}:\n\n",
            msgs.len(),
            session_id
        );
        for m in &msgs {
            let content_preview = if m.content.len() > 200 {
                let truncated: String = m.content.chars().take(200).collect();
                format!("{truncated}...")
            } else {
                m.content.clone()
            };
            output.push_str(&format!(
                "- [id={}, ts={}] {}: {}\n",
                m.id, m.created_at, m.role, content_preview,
            ));
            if let Some(meta) = &m.metadata {
                output.push_str(&format!("  metadata: {}\n", meta));
            }
        }
        Ok(ToolOutput::success(output))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Tool: query audit events (memory mutations).
struct QueryAuditEventsTool;

#[async_trait::async_trait]
impl Tool for QueryAuditEventsTool {
    fn name(&self) -> &str {
        "query_audit_events"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "query_audit_events".to_string(),
            description: "Query memory mutation audit log for an agent. Shows what was changed, when, and why.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID to query" },
                    "limit": { "type": "integer", "description": "Max events (default 20, max 50)" }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &crate::tools::ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let agent_id = input["agent_id"].as_str().unwrap_or("").to_owned();
        if agent_id.is_empty() {
            return Ok(ToolOutput::error("agent_id is required"));
        }
        let limit = input["limit"].as_u64().unwrap_or(20).min(50) as u32;

        let events = ctx
            .db
            .list_audit_events_paginated(&agent_id, limit, 0)
            .await?;
        let mut output = format!(
            "Found {} audit events for agent {}:\n\n",
            events.len(),
            agent_id
        );
        for e in &events {
            output.push_str(&format!(
                "- [{}] {} on '{}': {} → {}\n",
                e.created_at,
                e.tool_name,
                e.target_key,
                e.before_value.as_deref().unwrap_or("(none)"),
                {
                    let av = e.after_value.as_deref().unwrap_or("(none)");
                    if av.len() > 100 {
                        let truncated: String = av.chars().take(100).collect();
                        format!("{truncated}...")
                    } else {
                        av.to_string()
                    }
                },
            ));
            if let Some(r) = &e.reasoning {
                output.push_str(&format!("  reason: {}\n", r));
            }
        }
        Ok(ToolOutput::success(output))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Tool: full-text search across agent memory (FTS5, no vectors).
struct SearchMemoryTool;

#[async_trait::async_trait]
impl Tool for SearchMemoryTool {
    fn name(&self) -> &str {
        "search_memory"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_memory".to_string(),
            description: "Full-text search across an agent's structured memory (facts, people, preferences, events).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent whose memory to search" },
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["agent_id", "query"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &crate::tools::ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let agent_id = input["agent_id"].as_str().unwrap_or("").to_owned();
        let query = input["query"].as_str().unwrap_or("").to_owned();
        if agent_id.is_empty() || query.is_empty() {
            return Ok(ToolOutput::error("agent_id and query are required"));
        }

        // We need the agent-specific DB for memory search. Access via tool_ctx home_dir hack:
        // Instead, we look it up from the session_id (which carries the investigation context).
        // For now, use the dashboard_db's FTS search if the agent is the scoped one.
        let results = ctx.db.fts_search(&query, 20, None).await?;
        let mut output = format!(
            "Found {} memory results for '{}':\n\n",
            results.len(),
            query
        );
        for r in &results {
            output.push_str(&format!(
                "- [{}] (score={:.2}) {}\n",
                r.source_type, r.score, r.content,
            ));
        }
        if results.is_empty() {
            output.push_str("No matching memory entries found.\n");
        }
        Ok(ToolOutput::success(output))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Tool: get agent info (stats, core memory, soul).
struct GetAgentInfoTool {
    agents: Arc<std::collections::HashMap<String, Arc<super::state::AgentState>>>,
}

#[async_trait::async_trait]
impl Tool for GetAgentInfoTool {
    fn name(&self) -> &str {
        "get_agent_info"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_agent_info".to_string(),
            description: "Get agent identity, stats, core memory, and soul/persona. Helps understand an agent's configuration and personality.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID to look up" }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &crate::tools::ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let agent_id = input["agent_id"].as_str().unwrap_or("").to_owned();
        if agent_id.is_empty() {
            return Ok(ToolOutput::error("agent_id is required"));
        }

        // Get stats from dashboard DB
        let stats = ctx.db.get_agent_with_stats(&agent_id).await?;
        let stats = match stats {
            Some(s) => s,
            None => return Ok(ToolOutput::error(format!("agent '{}' not found", agent_id))),
        };

        let mut output = format!(
            "Agent: {} ({})\nActive: {}\nMessages: {}\nCreated: {}\n",
            stats.name, stats.id, stats.active, stats.message_count, stats.created_at,
        );

        // Get core memory and soul from per-agent state
        let normalized = mika_common::agent::normalize_agent_name(&agent_id);
        if let Some(agent_state) = self.agents.get(&normalized) {
            // Core memory
            if let Ok(entries) = agent_state.db.get_all_core_memory().await {
                output.push_str("\nCore Memory:\n");
                for e in &entries {
                    output.push_str(&format!("  {}: {}\n", e.key, e.value));
                }
            }
            // Soul
            let soul_path = agent_state.home_dir.join("soul.md");
            if let Ok(soul) = tokio::fs::read_to_string(&soul_path).await {
                let preview = if soul.len() > 500 {
                    let truncated: String = soul.chars().take(500).collect();
                    format!("{truncated}...")
                } else {
                    soul
                };
                output.push_str(&format!("\nSoul:\n{}\n", preview));
            }
        }

        Ok(ToolOutput::success(output))
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Tool: create a GitHub issue via the GitHub API.
struct CreateGithubIssueTool {
    http_client: reqwest::Client,
    github_token: String,
    github_repo: String,
}

#[async_trait::async_trait]
impl Tool for CreateGithubIssueTool {
    fn name(&self) -> &str {
        "create_github_issue"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_github_issue".to_string(),
            description: "Create a GitHub issue to track a finding from this investigation. The issue will automatically include investigation context (session ID, agent ID, trace ID).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Issue title — concise summary of the finding" },
                    "body": { "type": "string", "description": "Issue body in Markdown — detailed description, steps to reproduce, expected vs actual behavior" },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Labels to apply. Use: type (bug/enhancement/documentation/question), priority (p0-critical/p1-important/p2-normal/p3-nice-to-have), component (agent-core/tui/team-engine/skill/gateway/infrastructure/dashboard)"
                    }
                },
                "required": ["title", "body"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &crate::tools::ToolContext<'_>,
    ) -> Result<ToolOutput> {
        let title = input["title"].as_str().unwrap_or("").to_owned();
        let body_input = input["body"].as_str().unwrap_or("").to_owned();
        let labels: Vec<String> = input["labels"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        if title.is_empty() {
            return Ok(ToolOutput::error("title is required"));
        }
        if title.len() > 256 {
            return Ok(ToolOutput::error("title exceeds 256 character limit"));
        }
        if body_input.is_empty() {
            return Ok(ToolOutput::error("body is required"));
        }
        if body_input.len() > 10_000 {
            return Ok(ToolOutput::error("body exceeds 10,000 character limit"));
        }

        // Append investigation context footer
        let body = format!(
            "{body_input}\n\n---\n\
             **Investigation Context**\n\
             - Session: `{}`\n\
             - Trace: `{}`\n\
             \n_Created from the Mika observability dashboard_",
            ctx.session_id, ctx.trace_id,
        );

        let url = format!("https://api.github.com/repos/{}/issues", self.github_repo);

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.github_token))
            .header("User-Agent", "mika-dashboard")
            .header("Accept", "application/vnd.github+json")
            .json(&serde_json::json!({
                "title": title,
                "body": body,
                "labels": labels,
            }))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reach GitHub API: {e}"))?;

        let status = response.status();
        if status.is_success() {
            let resp_body: serde_json::Value = response
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to parse GitHub response: {e}"))?;
            let number = resp_body["number"].as_u64().unwrap_or(0);
            let html_url = resp_body["html_url"].as_str().unwrap_or("(unknown URL)");
            Ok(ToolOutput::success(format!(
                "Created issue #{number}: {html_url}"
            )))
        } else {
            let error_body = response.text().await.unwrap_or_default();
            let msg = match status.as_u16() {
                401 => "GitHub token is invalid or expired".to_string(),
                403 => "GitHub token lacks required permissions (needs 'repo' scope)".to_string(),
                404 => format!(
                    "Repository '{}' not found or not accessible",
                    self.github_repo
                ),
                422 => {
                    // Extract validation message from GitHub response
                    let parsed: serde_json::Value =
                        serde_json::from_str(&error_body).unwrap_or_default();
                    let gh_msg = parsed["message"].as_str().unwrap_or("Validation failed");
                    format!("GitHub validation error: {gh_msg}")
                }
                429 => "GitHub API rate limit exceeded, try again later".to_string(),
                _ => format!("GitHub API error (HTTP {status}): {error_body}"),
            };
            Ok(ToolOutput::error(msg))
        }
    }

    fn timeout_secs(&self) -> Option<u64> {
        Some(10)
    }
}

/// Configuration for building investigation tools.
pub struct InvestigationToolsConfig {
    pub agents: Arc<std::collections::HashMap<String, Arc<super::state::AgentState>>>,
    pub http_client: reqwest::Client,
    pub investigate_github_token: Option<String>,
    pub github_repo: Option<String>,
}

/// Build the investigation tool registry.
fn build_investigation_tools(config: InvestigationToolsConfig) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(QueryTimelineTool));
    registry.register(Box::new(QueryMessagesTool));
    registry.register(Box::new(QueryAuditEventsTool));
    registry.register(Box::new(SearchMemoryTool));
    registry.register(Box::new(GetAgentInfoTool {
        agents: config.agents,
    }));

    // Conditionally register GitHub issue creation tool
    if let (Some(token), Some(repo)) = (config.investigate_github_token, config.github_repo)
        && !token.is_empty()
        && !repo.is_empty()
    {
        // Validate owner/repo format (exactly one slash, not at edges)
        if repo.matches('/').count() == 1 && !repo.starts_with('/') && !repo.ends_with('/') {
            registry.register(Box::new(CreateGithubIssueTool {
                http_client: config.http_client,
                github_token: token,
                github_repo: repo,
            }));
        } else {
            tracing::warn!(
                "MIKA_GITHUB_REPO must be in 'owner/repo' format — GitHub issue tool not registered"
            );
        }
    }

    registry
}

// ===== Context Loading =====

/// Tool call extracted from message metadata.
#[derive(Debug, Deserialize)]
struct ToolCallMeta {
    step: u32,
    name: String,
    input_summary: String,
    output_summary: String,
    success: bool,
}

/// Build the system prompt and context for an investigation.
async fn build_investigation_context(
    db: &AsyncDatabase,
    agents: &std::collections::HashMap<String, Arc<super::state::AgentState>>,
    message: &SessionMessage,
    tool_call_index: Option<usize>,
    has_github_tool: bool,
) -> Result<String> {
    // Get agent_id from session
    let session = db
        .get_session(&message.session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;
    let agent_id = &session.agent_id;

    // Load surrounding messages (5 before, 3 after)
    let surrounding = db
        .get_surrounding_messages(&message.session_id, message.id, 5, 3)
        .await?;

    let mut prompt = String::from(
        "You are an investigation assistant analyzing Mika agent behavior. \
         Mika is written in Rust — any code suggestions in issues should use Rust idioms and crates. \
         You have read-only access to the database to help answer questions \
         about why the agent made certain decisions.\n\n\
         Use your tools to dig deeper when needed — query the timeline, load messages, \
         check audit events, or search memory to provide thorough answers.\n\n",
    );

    if has_github_tool {
        prompt.push_str(
            "You can also create GitHub issues to track findings using the `create_github_issue` tool. \
             Use it when the user asks to create an issue or when you identify a significant bug worth tracking.\n\n\
             When creating issues:\n\
             - Apply labels from the Mika taxonomy:\n\
               - **Type** (pick one): `bug`, `enhancement`, `documentation`, `question`\n\
               - **Priority** (pick one): `p0-critical`, `p1-important`, `p2-normal`, `p3-nice-to-have`\n\
               - **Component** (pick one or more): `agent-core`, `tui`, `team-engine`, `skill`, `gateway`, `infrastructure`, `dashboard`\n\
             - Always include an **Acceptance Criteria** section at the end of the issue body. \
               List concrete, testable conditions that define when the issue is resolved. \
               Include both the failing case that triggered the issue and any equivalent forms \
               (e.g. if `~/file.txt` fails, acceptance criteria should cover both `~/file.txt` \
               and `/home/user/file.txt` working correctly).\n\n",
        );
    }

    prompt.push_str(&format!(
        "## Agent Under Investigation\nAgent: {}\nSession: {}\n\n",
        agent_id, message.session_id
    ));

    // Include core memory if available
    let normalized = mika_common::agent::normalize_agent_name(agent_id);
    if let Some(agent_state) = agents.get(&normalized)
        && let Ok(entries) = agent_state.db.get_all_core_memory().await
        && !entries.is_empty()
    {
        prompt.push_str("## Core Memory\n");
        for e in &entries {
            prompt.push_str(&format!("- {}: {}\n", e.key, e.value));
        }
        prompt.push('\n');
    }

    // Include surrounding messages as context
    prompt.push_str("## Conversation Context\n");
    for m in &surrounding {
        let content_preview = if m.content.len() > 500 {
            let truncated: String = m.content.chars().take(500).collect();
            format!("{truncated}...")
        } else {
            m.content.clone()
        };
        prompt.push_str(&format!("[{}] {}: {}\n", m.id, m.role, content_preview));
        if let Some(meta) = &m.metadata {
            prompt.push_str(&format!("  tool_calls: {}\n", meta));
        }
    }
    prompt.push('\n');

    // Include specific tool call if requested
    if let Some(idx) = tool_call_index
        && let Some(meta) = &message.metadata
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(meta)
        && let Some(calls) = parsed["tool_calls"].as_array()
        && let Some(tc) = calls.get(idx)
        && let Ok(tc) = serde_json::from_value::<ToolCallMeta>(tc.clone())
    {
        prompt.push_str(&format!(
            "## Specific Tool Call Under Investigation\n\
                                 Tool: {}\nStep: {}\nInput: {}\nOutput: {}\nSuccess: {}\n\n",
            tc.name, tc.step, tc.input_summary, tc.output_summary, tc.success,
        ));
    }

    Ok(prompt)
}

// ===== Investigation Agent Loop =====

/// Parameters for running an investigation agent loop.
struct InvestigationParams {
    system_prompt: String,
    messages: Vec<LlmMessage>,
    tx: mpsc::Sender<Result<sse::Event, Infallible>>,
    db: AsyncDatabase,
    session_id: String,
    trace_id: String,
}

/// Run the investigation agent loop, sending SSE events via the channel.
async fn run_investigation(
    llm: &dyn LlmProvider,
    tools: &ToolRegistry,
    params: InvestigationParams,
) {
    let InvestigationParams {
        system_prompt,
        mut messages,
        tx,
        db,
        session_id: investigation_session_id,
        trace_id: investigation_trace_id,
    } = params;
    let tool_defs: Vec<_> = tools
        .definitions()
        .iter()
        .map(|d| d.clone().into())
        .collect();
    let edit_count = std::sync::atomic::AtomicU32::new(0);
    let skills_dirty = std::sync::atomic::AtomicBool::new(false);
    // Create a minimal ToolContext for investigation tools.
    // session_id and trace_id carry the investigation context for the
    // create_github_issue tool to embed in issue bodies.
    let tool_ctx = crate::tools::ToolContext {
        db: &db,
        session_id: &investigation_session_id,
        trace_id: &investigation_trace_id,
        home_dir: std::path::Path::new("/tmp"),
        global_home_dir: None,
        core_memory_edit_count: &edit_count,
        is_onboarding: false,
        message_sender: None,
        embedding_client: None,
        brave_api_key: None,
        github_token: None,
        skills_dirty: &skills_dirty,
        is_reflection: false,
        is_task_context: false,
        is_callback_turn: false,
    };

    for _step in 0..MAX_INVESTIGATION_STEPS {
        let request = LlmRequest {
            model: llm.model_name().to_string(),
            max_tokens: 4096,
            system: Some(system_prompt.clone()),
            messages: messages.clone(),
            tools: Some(tool_defs.clone()),
            thinking: None,
        };

        let response =
            match tokio::time::timeout(INVESTIGATION_TIMEOUT, llm.send_message(&request)).await {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) => {
                    let _ = send_event(
                        &tx,
                        InvestigateEvent::Error {
                            message: format!("LLM error: {e}"),
                        },
                    )
                    .await;
                    let _ = send_event(&tx, InvestigateEvent::Done {}).await;
                    return;
                }
                Err(_) => {
                    let _ = send_event(
                        &tx,
                        InvestigateEvent::Error {
                            message: "Investigation timed out".to_string(),
                        },
                    )
                    .await;
                    let _ = send_event(&tx, InvestigateEvent::Done {}).await;
                    return;
                }
            };

        // Collect text and tool_use blocks
        let mut text_parts = Vec::new();
        let mut tool_uses = Vec::new();

        for block in &response.content {
            match block {
                LlmResponseContent::Text(text) => {
                    text_parts.push(text.clone());
                }
                LlmResponseContent::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    tool_uses.push((id.clone(), name.clone(), arguments.clone()));
                }
            }
        }

        // Send any text as a delta
        if !text_parts.is_empty() {
            let full_text = text_parts.join("");
            if send_event(&tx, InvestigateEvent::TextDelta { text: full_text })
                .await
                .is_err()
            {
                return; // Client disconnected
            }
        }

        // If stop_reason is end_turn (no tool calls), we're done
        if response.stop_reason == LlmStopReason::EndTurn || tool_uses.is_empty() {
            let _ = send_event(&tx, InvestigateEvent::Done {}).await;
            return;
        }

        // Dispatch tool calls
        let mut tool_results = Vec::new();
        for (id, name, input) in &tool_uses {
            // Notify client that a tool is running
            if send_event(
                &tx,
                InvestigateEvent::ToolUse {
                    name: name.clone(),
                    status: "running".to_string(),
                    summary: None,
                },
            )
            .await
            .is_err()
            {
                return;
            }

            let output = if let Some(tool) = tools.get(name) {
                match tokio::time::timeout(TOOL_TIMEOUT, tool.execute(input.clone(), &tool_ctx))
                    .await
                {
                    Ok(Ok(out)) => out,
                    Ok(Err(e)) => ToolOutput::error(format!("Tool error: {e}")),
                    Err(_) => ToolOutput::error("Tool timed out"),
                }
            } else {
                ToolOutput::error(format!("Unknown tool: {name}"))
            };

            let summary = if output.content.len() > 100 {
                let truncated: String = output.content.chars().take(100).collect();
                format!("{truncated}...")
            } else {
                output.content.clone()
            };

            // Notify client that tool completed
            if send_event(
                &tx,
                InvestigateEvent::ToolUse {
                    name: name.clone(),
                    status: "completed".to_string(),
                    summary: Some(summary),
                },
            )
            .await
            .is_err()
            {
                return;
            }

            tool_results.push(LlmContentBlock::ToolResult {
                tool_call_id: id.clone(),
                content: LlmToolResultContent::Text(output.content),
                is_error: output.is_error,
            });
        }

        // Add assistant response + tool results to message history for next turn
        messages.push(LlmMessage {
            role: LlmRole::Assistant,
            content: LlmContent::Blocks(response_content_to_blocks(&response.content)),
        });
        messages.push(LlmMessage {
            role: LlmRole::Tool,
            content: LlmContent::Blocks(tool_results),
        });
    }

    // Exceeded max steps
    let _ = send_event(
        &tx,
        InvestigateEvent::TextDelta {
            text: "\n\n[Investigation reached the maximum number of tool steps]".to_string(),
        },
    )
    .await;
    let _ = send_event(&tx, InvestigateEvent::Done {}).await;
}

/// Helper to send an SSE event, returning Err if the client disconnected.
async fn send_event(
    tx: &mpsc::Sender<Result<sse::Event, Infallible>>,
    event: InvestigateEvent,
) -> Result<(), ()> {
    let event_type = match &event {
        InvestigateEvent::TextDelta { .. } => "text_delta",
        InvestigateEvent::ToolUse { .. } => "tool_use",
        InvestigateEvent::Done { .. } => "done",
        InvestigateEvent::Error { .. } => "error",
    };
    let data = serde_json::to_string(&event).unwrap_or_default();
    let sse_event = sse::Event::default().event(event_type).data(data);
    tx.send(Ok(sse_event)).await.map_err(|_| ())
}

// ===== HTTP Handler =====

/// POST /api/v1/investigate — start an investigation SSE stream.
pub async fn handle_investigate(
    State(state): State<AppState>,
    Json(req): Json<InvestigateRequest>,
) -> Result<Response, Response> {
    // --- Validate request ---
    if req.question.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "question is required"})),
        )
            .into_response());
    }
    if req.question.len() > MAX_QUESTION_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("question exceeds {} chars", MAX_QUESTION_LEN)})),
        )
            .into_response());
    }

    // --- Load target message ---
    let message = state
        .dashboard_db
        .get_message_by_id(req.message_id)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to load message");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal server error"})),
            )
                .into_response()
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("message {} not found", req.message_id)})),
            )
                .into_response()
        })?;

    // --- Validate tool_call_index ---
    if let Some(idx) = req.tool_call_index {
        let valid = message
            .metadata
            .as_ref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v["tool_calls"].as_array().map(|a| idx < a.len()))
            .unwrap_or(false);
        if !valid {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("tool_call_index {} out of range", idx)})),
            )
                .into_response());
        }
    }

    // --- Acquire investigation lock (non-blocking) ---
    if state.investigation_lock.try_lock().is_err() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error": "another investigation is already running"})),
        )
            .into_response());
    }

    // --- Check GitHub tool availability ---
    let has_github =
        state.settings.investigate_github_token.is_some() && state.settings.github_repo.is_some();

    // --- Build context ---
    let system_prompt = build_investigation_context(
        &state.dashboard_db,
        &state.agents,
        &message,
        req.tool_call_index,
        has_github,
    )
    .await
    .map_err(|e| {
        error!(error = %e, "failed to build investigation context");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "failed to build investigation context"})),
        )
            .into_response()
    })?;

    // --- Lazy-init investigation tools ---
    // NOTE: Tool registry is immutable after first initialization. GitHub tool
    // availability is determined at the first investigation request. Config changes
    // (e.g., setting MIKA_INVESTIGATE_GITHUB_TOKEN) require a server restart to take effect.
    let tools = state
        .investigation_tools
        .get_or_init(|| async {
            Arc::new(build_investigation_tools(InvestigationToolsConfig {
                agents: state.agents.clone(),
                http_client: state.http_client.clone(),
                investigate_github_token: state.settings.investigate_github_token.clone(),
                github_repo: state.settings.github_repo.clone(),
            }))
        })
        .await
        .clone();

    // --- Build messages array ---
    let mut messages: Vec<LlmMessage> = Vec::new();

    // Add history turns (capped)
    let history = if req.history.len() > MAX_HISTORY_TURNS {
        &req.history[req.history.len() - MAX_HISTORY_TURNS..]
    } else {
        &req.history
    };
    for turn in history {
        let role = match turn.role.as_str() {
            "assistant" => LlmRole::Assistant,
            _ => LlmRole::User,
        };
        messages.push(LlmMessage {
            role,
            content: LlmContent::Text(turn.content.clone()),
        });
    }

    // Add current question
    messages.push(LlmMessage {
        role: LlmRole::User,
        content: LlmContent::Text(req.question),
    });

    // --- Start SSE stream ---
    let (tx, rx) = mpsc::channel::<Result<sse::Event, Infallible>>(64);
    // Investigation panel is not agent-scoped — use the default agent's LLM provider.
    // The default agent is guaranteed to exist (validated in run_server).
    let llm = state
        .resolve_agent(&state.default_agent)
        .expect("default agent must exist")
        .llm
        .clone();
    let db = state.dashboard_db.clone();
    let agents = state.agents.clone();
    let lock = state.investigation_lock.clone();
    let investigation_session_id = message.session_id.clone();
    let investigation_trace_id = message.trace_id.clone().unwrap_or_default();

    info!(message_id = req.message_id, "starting investigation");

    tokio::spawn(async move {
        // Hold the lock for the entire investigation duration
        let _guard = lock.lock().await;
        let _agents = agents; // keep alive for the duration
        run_investigation(
            llm.as_ref(),
            &tools,
            InvestigationParams {
                system_prompt,
                messages,
                tx,
                db,
                session_id: investigation_session_id,
                trace_id: investigation_trace_id,
            },
        )
        .await;
    });

    let stream = ReceiverStream::new(rx);
    let sse = sse::Sse::new(stream).keep_alive(
        sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    );

    Ok(sse.into_response())
}
