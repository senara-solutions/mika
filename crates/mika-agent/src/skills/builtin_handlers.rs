//! Dispatch for builtin skill tool handlers.
//!
//! Builtin handlers are Rust functions invoked directly by the agent loop,
//! without spawning a subprocess or making an HTTP call. They have access
//! to `ToolContext` (for home_dir, etc.) and return `ToolOutput`.

use std::fmt::Write;
use std::sync::LazyLock;

use crate::tools::{ToolContext, ToolOutput};

/// Embedded OpenAPI spec for the agent (mika-server) HTTP API.
static AGENT_API_SPEC: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/openapi/mika-server.yaml"));

/// Embedded architecture overview document.
static ARCHITECTURE_OVERVIEW: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/architecture.md"));

/// Embedded documentation files (topic → content).
static DOC_CONFIGURATION: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/configuration.md"));
static DOC_DEPLOYMENT: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/deployment.md"));
static DOC_GETTING_STARTED: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/getting-started.md"));
static DOC_SKILLS: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/skills.md"));
static DOC_RUNTIME_STRUCTURE: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/runtime-structure.md"));
static DOC_SLASH_COMMANDS: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/slash-commands.md"));

/// Known builtin function names, used for startup validation.
pub const KNOWN_BUILTINS: &[&str] = &["get_documentation", "web_search"];

/// Maximum output size from a builtin handler (matches executor::MAX_OUTPUT_LEN).
const MAX_OUTPUT_LEN: usize = 10_000;

/// Maximum response body size from Brave Search API (1MB).
const MAX_SEARCH_RESPONSE_BYTES: usize = 1_024 * 1_024;

/// Shared HTTP client for builtin handlers (connection pooling, TLS reuse).
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("failed to build HTTP client")
});

/// Dispatch a builtin handler by function name.
///
/// Returns `ToolOutput` directly — no subprocess, no HTTP call.
/// Output is truncated to `MAX_OUTPUT_LEN` for consistency with exec/http handlers.
pub async fn execute(
    function: &str,
    input: serde_json::Value,
    ctx: &ToolContext<'_>,
) -> ToolOutput {
    let mut output = match function {
        "get_documentation" => get_documentation(&input, ctx).await,
        "web_search" => web_search(&input, ctx).await,
        _ => ToolOutput::error(format!("Unknown builtin function: {function}")),
    };
    truncate_output(&mut output);
    output
}

/// Truncate output content to MAX_OUTPUT_LEN characters.
fn truncate_output(output: &mut ToolOutput) {
    if output.content.len() > MAX_OUTPUT_LEN {
        output.content.truncate(MAX_OUTPUT_LEN);
        output.content.push_str("\n... (truncated at 10000 chars)");
    }
}

/// Unified documentation handler — returns docs for any supported topic.
///
/// Input: `{"topic": "architecture" | "api-spec" | "cli-reference" | "configuration" | ...}`
///
/// Most topics return embedded (compile-time) content. The `cli-reference` topic
/// reads from disk because it's generated at runtime by the CLI binary.
async fn get_documentation(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    let topic = input.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    match topic {
        "architecture" => ToolOutput::success(ARCHITECTURE_OVERVIEW.to_string()),
        "api-spec" => ToolOutput::success(AGENT_API_SPEC.to_string()),
        "cli-reference" => {
            let path = ctx.home_dir.join("cli-reference.md");
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => ToolOutput::success(content),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "failed to read CLI reference");
                    ToolOutput::error(
                        "CLI reference not found. Run the `mika` CLI once to generate it."
                            .to_string(),
                    )
                }
            }
        }
        "configuration" => ToolOutput::success(DOC_CONFIGURATION.to_string()),
        "deployment" => ToolOutput::success(DOC_DEPLOYMENT.to_string()),
        "getting-started" => ToolOutput::success(DOC_GETTING_STARTED.to_string()),
        "runtime-structure" => ToolOutput::success(DOC_RUNTIME_STRUCTURE.to_string()),
        "skills" => ToolOutput::success(DOC_SKILLS.to_string()),
        "slash-commands" => ToolOutput::success(DOC_SLASH_COMMANDS.to_string()),
        _ => ToolOutput::error(
            "Invalid topic. Use one of: architecture, api-spec, cli-reference, configuration, deployment, getting-started, runtime-structure, skills, slash-commands."
                .to_string(),
        ),
    }
}

/// Search the web using the Brave Search API.
///
/// Input: `{"query": "search terms"}`
/// Requires `brave_api_key` in config or `MIKA_BRAVE_API_KEY` environment variable.
async fn web_search(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q.trim(),
        _ => return ToolOutput::error("Missing or empty 'query' parameter.".to_string()),
    };

    // Enforce input length limit (shared tool validation convention)
    if query.len() > 10_000 {
        return ToolOutput::error("Query too long (max 10000 characters).".to_string());
    }

    let api_key = match ctx.brave_api_key {
        Some(key) if !key.trim().is_empty() => key.to_string(),
        _ => {
            return ToolOutput::error(
                "Brave Search API key not configured. \
                 Set brave_api_key in ~/.mika/config.toml or MIKA_BRAVE_API_KEY env var. \
                 Get a free key at https://brave.com/search/api/"
                    .to_string(),
            );
        }
    };

    let resp = match HTTP_CLIENT
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", &api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", "5")])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return ToolOutput::error("Search request timed out (15s).".to_string());
        }
        Err(_) => {
            return ToolOutput::error("Search request failed.".to_string());
        }
    };

    let status = resp.status();
    if !status.is_success() {
        let msg = match status.as_u16() {
            401 => "Invalid API key. Check MIKA_BRAVE_API_KEY.".to_string(),
            429 => "Search rate limit exceeded. Try again later.".to_string(),
            _ => format!("Search API returned HTTP {status}."),
        };
        return ToolOutput::error(msg);
    }

    // Limit response body size to prevent memory exhaustion
    let content_length = resp.content_length().unwrap_or(0) as usize;
    if content_length > MAX_SEARCH_RESPONSE_BYTES {
        return ToolOutput::error("Search response too large.".to_string());
    }

    let bytes = match resp.bytes().await {
        Ok(b) if b.len() > MAX_SEARCH_RESPONSE_BYTES => {
            return ToolOutput::error("Search response too large.".to_string());
        }
        Ok(b) => b,
        Err(_) => return ToolOutput::error("Failed to read search response.".to_string()),
    };

    let body: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return ToolOutput::error("Failed to parse search response.".to_string()),
    };

    format_brave_results(&body, query)
}

/// Format Brave Search API results into concise, LLM-friendly text.
fn format_brave_results(body: &serde_json::Value, query: &str) -> ToolOutput {
    let results = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array());

    let results = match results {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            return ToolOutput::success(format!("No results found for \"{query}\"."));
        }
    };

    let mut out = format!("Search results for \"{query}\":\n");

    for (i, result) in results.iter().enumerate().take(5) {
        let title = result.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let url = result.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let description = result
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let _ = writeln!(out);
        let _ = writeln!(out, "{}. {}", i + 1, title);
        let _ = writeln!(out, "   URL: {url}");
        if !description.is_empty() {
            let _ = writeln!(out, "   {description}");
        }
    }

    ToolOutput::success(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_get_documentation_all_embedded_topics() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        for topic in &[
            "architecture",
            "api-spec",
            "configuration",
            "deployment",
            "getting-started",
            "runtime-structure",
            "skills",
            "slash-commands",
        ] {
            let input = serde_json::json!({"topic": topic});
            let output = get_documentation(&input, &ctx).await;
            assert!(!output.is_error, "topic {topic} should succeed");
            assert!(
                !output.content.is_empty(),
                "topic {topic} should have content"
            );
        }
    }

    #[tokio::test]
    async fn test_get_documentation_cli_reference() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"topic": "cli-reference"});
        let output = get_documentation(&input, &ctx).await;
        // In test env, the file won't exist — expect a helpful error
        assert!(output.is_error);
        assert!(output.content.contains("CLI reference not found"));
    }

    #[tokio::test]
    async fn test_get_documentation_invalid_topic() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"topic": "nonexistent"});
        let output = get_documentation(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Invalid topic"));
    }

    #[tokio::test]
    async fn test_get_documentation_missing_topic() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({});
        let output = get_documentation(&input, &ctx).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_execute_unknown_function_returns_error() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = execute("nonexistent_function", serde_json::json!({}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Unknown builtin function"));
        assert!(output.content.contains("nonexistent_function"));
    }

    #[test]
    fn test_truncate_output_short() {
        let mut output = ToolOutput::success("short content");
        truncate_output(&mut output);
        assert_eq!(output.content, "short content");
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "x".repeat(MAX_OUTPUT_LEN + 500);
        let mut output = ToolOutput::success(long);
        truncate_output(&mut output);
        assert!(output.content.contains("truncated"));
        assert!(output.content.len() < MAX_OUTPUT_LEN + 100);
    }

    #[tokio::test]
    async fn test_web_search_missing_query() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = web_search(&serde_json::json!({}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Missing or empty"));
    }

    #[tokio::test]
    async fn test_web_search_empty_query() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = web_search(&serde_json::json!({"query": "  "}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Missing or empty"));
    }

    #[tokio::test]
    async fn test_web_search_query_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let long_query = "x".repeat(10_001);
        let output = web_search(&serde_json::json!({"query": long_query}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("too long"));
    }

    #[test]
    fn test_format_brave_results_empty() {
        let body = serde_json::json!({"web": {"results": []}});
        let output = format_brave_results(&body, "test");
        assert!(!output.is_error);
        assert!(output.content.contains("No results found"));
    }

    #[test]
    fn test_format_brave_results_with_data() {
        let body = serde_json::json!({
            "web": {
                "results": [
                    {
                        "title": "Test Result",
                        "url": "https://example.com",
                        "description": "A test description"
                    }
                ]
            }
        });
        let output = format_brave_results(&body, "test query");
        assert!(!output.is_error);
        assert!(output.content.contains("Test Result"));
        assert!(output.content.contains("https://example.com"));
        assert!(output.content.contains("A test description"));
        assert!(output.content.contains("test query"));
    }

    #[test]
    fn test_format_brave_results_no_web_key() {
        let body = serde_json::json!({"query": {}});
        let output = format_brave_results(&body, "test");
        assert!(!output.is_error);
        assert!(output.content.contains("No results found"));
    }

    #[tokio::test]
    async fn test_web_search_no_api_key() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        // Default test context has brave_api_key: None
        let output = web_search(&serde_json::json!({"query": "test"}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("not configured"));
        assert!(output.content.contains("config.toml"));
    }

    #[test]
    fn test_web_search_in_known_builtins() {
        assert!(KNOWN_BUILTINS.contains(&"web_search"));
    }

    #[test]
    fn test_get_documentation_in_known_builtins() {
        assert!(KNOWN_BUILTINS.contains(&"get_documentation"));
    }

    /// Verify crate-local fallback copies match workspace-root docs.
    /// Run with: cargo test -p mika-agent -- --ignored verify_crate_local_docs_in_sync
    #[test]
    #[ignore]
    fn verify_crate_local_docs_in_sync() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let workspace_docs = std::path::Path::new(manifest_dir).join("../../docs");
        let crate_docs = std::path::Path::new(manifest_dir).join("docs");

        let files = [
            "architecture.md",
            "configuration.md",
            "deployment.md",
            "getting-started.md",
            "runtime-structure.md",
            "skills.md",
            "slash-commands.md",
            "openapi/mika-server.yaml",
        ];

        for file in &files {
            let ws = std::fs::read_to_string(workspace_docs.join(file))
                .unwrap_or_else(|e| panic!("workspace docs/{file}: {e}"));
            let cr = std::fs::read_to_string(crate_docs.join(file))
                .unwrap_or_else(|e| panic!("crate docs/{file}: {e}"));
            assert_eq!(
                ws, cr,
                "docs/{file} differs from crates/mika-agent/docs/{file}. \
                 Run scripts/sync-agent-docs.sh to fix."
            );
        }
    }
}
