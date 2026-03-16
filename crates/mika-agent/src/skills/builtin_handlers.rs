//! Dispatch for builtin skill tool handlers.
//!
//! Builtin handlers are Rust functions invoked directly by the agent loop,
//! without spawning a subprocess or making an HTTP call. They have access
//! to `ToolContext` (for home_dir, etc.) and return `ToolOutput`.

use std::fmt::Write;
use std::sync::LazyLock;

use tokio::io::AsyncReadExt;

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
pub const KNOWN_BUILTINS: &[&str] = &["get_documentation", "run_gh", "run_gws", "web_search"];

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
        "run_gh" => run_gh(&input, ctx).await,
        "run_gws" => run_gws(&input, ctx).await,
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

/// Strip YAML frontmatter from a markdown document.
///
/// If the content starts with `---\n`, everything up to and including the next
/// `---\n` (or `---\r\n`) line is removed. Leading whitespace after the
/// frontmatter block is also trimmed.
fn strip_frontmatter(content: &str) -> &str {
    let after_open = if let Some(rest) = content.strip_prefix("---\r\n") {
        rest
    } else if let Some(rest) = content.strip_prefix("---\n") {
        rest
    } else {
        return content;
    };
    // Find the closing `---` on its own line
    if let Some(rest) = after_open.strip_prefix("---\n") {
        rest.trim_start()
    } else if let Some(rest) = after_open.strip_prefix("---\r\n") {
        rest.trim_start()
    } else if let Some(close) = after_open.find("\n---\n") {
        after_open[close + 5..].trim_start()
    } else if let Some(close) = after_open.find("\n---\r\n") {
        after_open[close + 6..].trim_start()
    } else {
        // No closing delimiter — return content unchanged
        content
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
        "architecture" => ToolOutput::success(strip_frontmatter(ARCHITECTURE_OVERVIEW).to_string()),
        "api-spec" => ToolOutput::success(AGENT_API_SPEC.to_string()),
        "cli-reference" => {
            let path = ctx.home_dir.join("cli-reference.md");
            match tokio::fs::read_to_string(&path).await {
                Ok(content) => ToolOutput::success(strip_frontmatter(&content).to_string()),
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "failed to read CLI reference");
                    ToolOutput::error(
                        "CLI reference not found. Run the `mika` CLI once to generate it."
                            .to_string(),
                    )
                }
            }
        }
        "configuration" => ToolOutput::success(strip_frontmatter(DOC_CONFIGURATION).to_string()),
        "deployment" => ToolOutput::success(strip_frontmatter(DOC_DEPLOYMENT).to_string()),
        "getting-started" => {
            ToolOutput::success(strip_frontmatter(DOC_GETTING_STARTED).to_string())
        }
        "runtime-structure" => {
            ToolOutput::success(strip_frontmatter(DOC_RUNTIME_STRUCTURE).to_string())
        }
        "skills" => ToolOutput::success(strip_frontmatter(DOC_SKILLS).to_string()),
        "slash-commands" => {
            ToolOutput::success(strip_frontmatter(DOC_SLASH_COMMANDS).to_string())
        }
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

// -- Shared CLI helpers --

/// Parse and validate a CLI command array from JSON input.
///
/// Shared validation steps: string rejection, array parsing, empty check, length limit.
/// Returns the validated args for handler-specific checks (allowlist, blocked flags).
fn parse_command_array(input: &serde_json::Value) -> Result<Vec<String>, ToolOutput> {
    // Reject string format with a migration hint
    if input.get("command").is_some_and(|v| v.is_string()) {
        return Err(ToolOutput::error(
            "The 'command' parameter must be a JSON array of strings, not a single string."
                .to_string(),
        ));
    }

    let args: Vec<String> = match input.get("command").and_then(|v| v.as_array()) {
        Some(arr) => {
            let mut args = Vec::with_capacity(arr.len());
            for item in arr {
                match item.as_str() {
                    Some(s) => args.push(s.to_string()),
                    None => {
                        return Err(ToolOutput::error(
                            "All elements in 'command' must be strings.".to_string(),
                        ));
                    }
                }
            }
            args
        }
        None => {
            return Err(ToolOutput::error(
                "Missing or invalid 'command' parameter.".to_string(),
            ));
        }
    };

    if args.is_empty() {
        return Err(ToolOutput::error(
            "Command array must not be empty.".to_string(),
        ));
    }

    // Enforce total input length limit
    let total_len: usize = args.iter().map(|s| s.len()).sum();
    if total_len > 10_000 {
        return Err(ToolOutput::error(
            "Command too long (max 10000 characters total).".to_string(),
        ));
    }

    Ok(args)
}

/// Spawn a CLI subprocess, capture bounded stdout/stderr, and return ToolOutput.
///
/// Shared logic for all CLI builtin handlers (gh, gws, etc.). The caller builds
/// the `Command` with args, env vars, and security scrubbing; this function handles
/// the spawn-read-wait-format cycle.
async fn spawn_and_collect(
    mut cmd: tokio::process::Command,
    tool_name: &str,
    install_hint: &str,
) -> ToolOutput {
    cmd.kill_on_drop(true);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ToolOutput::error(format!("Failed to spawn {tool_name}: {e}. {install_hint}"));
        }
    };

    // Read stdout and stderr with bounded size to prevent memory exhaustion
    let stdout_handle = child.stdout.take().expect("stdout piped");
    let stderr_handle = child.stderr.take().expect("stderr piped");
    let mut stdout_buf = Vec::with_capacity(MAX_OUTPUT_LEN);
    let mut stderr_buf = Vec::with_capacity(MAX_OUTPUT_LEN);

    let mut stdout_take = stdout_handle.take(MAX_OUTPUT_LEN as u64);
    let mut stderr_take = stderr_handle.take(MAX_OUTPUT_LEN as u64);
    let (stdout_res, stderr_res) = tokio::join!(
        stdout_take.read_to_end(&mut stdout_buf),
        stderr_take.read_to_end(&mut stderr_buf),
    );
    stdout_res.ok();
    stderr_res.ok();

    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => {
            return ToolOutput::error(format!("Failed to execute {tool_name}: {e}"));
        }
    };

    let stdout = String::from_utf8_lossy(&stdout_buf);
    let stderr = String::from_utf8_lossy(&stderr_buf);

    if status.success() {
        ToolOutput::success(stdout.into_owned())
    } else {
        let code = status.code().unwrap_or(-1);
        let mut result = format!("Exit code: {code}\n");
        if !stderr.is_empty() {
            result.push_str(&stderr);
        }
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        ToolOutput::success(result)
    }
}

// -- GitHub CLI handler --

/// Allowed top-level `gh` subcommands.
const GH_ALLOWED_SUBCOMMANDS: &[&str] = &[
    "pr",
    "issue",
    "run",
    "workflow",
    "release",
    "repo",
    "search",
    "label",
    "milestone",
    "project",
];

/// Validated `run_gh` input — command args and optional repo.
#[derive(Debug)]
struct GhArgs {
    args: Vec<String>,
    repo: Option<String>,
}

/// Validate and parse `run_gh` input into structured args.
///
/// Checks: shared parse + allowlist + repo-smuggling.
fn validate_gh_input(input: &serde_json::Value) -> Result<GhArgs, ToolOutput> {
    let args = parse_command_array(input)?;

    // Validate subcommand against allowlist
    let subcommand = &args[0];
    if !GH_ALLOWED_SUBCOMMANDS.contains(&subcommand.as_str()) {
        return Err(ToolOutput::error(format!(
            "gh subcommand '{subcommand}' is not allowed. \
             Permitted: {}.",
            GH_ALLOWED_SUBCOMMANDS.join(", ")
        )));
    }

    // Reject --repo / -R smuggling in the command array (including --repo=value form)
    if args
        .iter()
        .any(|s| s == "--repo" || s == "-R" || s.starts_with("--repo="))
    {
        return Err(ToolOutput::error(
            "Do not include --repo in the command array. Use the separate 'repo' parameter instead."
                .to_string(),
        ));
    }

    let repo = input
        .get("repo")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    Ok(GhArgs { args, repo })
}

/// Execute a GitHub CLI (`gh`) command with safe argument passing.
///
/// Input: `{"command": ["pr", "list", "--state", "open"], "repo": "owner/repo"}`
///
/// Unlike the old shell-script handler, arguments are passed as an array to avoid
/// shell word-splitting issues with quoted multi-word values.
async fn run_gh(input: &serde_json::Value, _ctx: &ToolContext<'_>) -> ToolOutput {
    let gh_args = match validate_gh_input(input) {
        Ok(args) => args,
        Err(err) => return err,
    };

    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(&gh_args.args);

    if let Some(ref repo) = gh_args.repo {
        cmd.arg("--repo").arg(repo);
    }

    cmd.env("GH_PROMPT_DISABLED", "1");
    super::executor::scrub_mika_env_vars(&mut cmd);

    spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?").await
}

/// Allowed top-level `gws` service subcommands.
const GWS_ALLOWED_SUBCOMMANDS: &[&str] = &["gmail", "calendar", "drive"];

/// Flags that must not appear in the `run_gws` command array (prevent credential/config smuggling).
const GWS_BLOCKED_FLAGS: &[&str] = &["--token", "--credentials-file", "--config", "--config-dir"];

/// Validate and parse `run_gws` input into structured args.
///
/// Checks: shared parse + allowlist + flag-smuggling.
fn validate_gws_input(input: &serde_json::Value) -> Result<Vec<String>, ToolOutput> {
    let args = parse_command_array(input)?;

    // Validate service subcommand against allowlist
    let subcommand = &args[0];
    if !GWS_ALLOWED_SUBCOMMANDS.contains(&subcommand.as_str()) {
        return Err(ToolOutput::error(format!(
            "gws service '{subcommand}' is not allowed. \
             Permitted: {}.",
            GWS_ALLOWED_SUBCOMMANDS.join(", ")
        )));
    }

    // Reject credential/config flag smuggling in the command array (including --flag=value form)
    for flag in GWS_BLOCKED_FLAGS {
        if args
            .iter()
            .any(|s| s == *flag || s.starts_with(&format!("{flag}=")))
        {
            return Err(ToolOutput::error(format!(
                "Do not include {flag} in the command array. \
                 Authentication and configuration are handled automatically."
            )));
        }
    }

    Ok(args)
}

/// Execute a Google Workspace CLI (`gws`) command with safe argument passing.
///
/// Input: `{"command": ["gmail", "messages", "list", "--params", "{\"maxResults\": 5}"]}`
///
/// Uses `gws`'s native keyring-based authentication (set up via `gws auth login`).
async fn run_gws(input: &serde_json::Value, _ctx: &ToolContext<'_>) -> ToolOutput {
    let args = match validate_gws_input(input) {
        Ok(args) => args,
        Err(err) => return err,
    };

    let mut cmd = tokio::process::Command::new("gws");
    cmd.args(&args);

    super::executor::scrub_mika_env_vars(&mut cmd);

    spawn_and_collect(
        cmd,
        "gws",
        "Is the Google Workspace CLI installed? \
         Install via: cargo install --git https://github.com/googleworkspace/cli --locked",
    )
    .await
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

    #[test]
    fn test_run_gh_in_known_builtins() {
        assert!(KNOWN_BUILTINS.contains(&"run_gh"));
    }

    #[tokio::test]
    async fn test_run_gh_empty_command() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"command": []});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("must not be empty"));
    }

    #[tokio::test]
    async fn test_run_gh_missing_command() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Missing"));
    }

    #[tokio::test]
    async fn test_run_gh_disallowed_subcommand() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"command": ["auth", "login"]});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("'auth' is not allowed"));
        assert!(output.content.contains("Permitted:"));
    }

    #[test]
    fn test_run_gh_allowlist_accepts_valid() {
        for sub in &[
            "pr",
            "issue",
            "run",
            "workflow",
            "release",
            "repo",
            "search",
            "label",
            "milestone",
            "project",
        ] {
            let input = serde_json::json!({"command": [sub, "list"]});
            let result = validate_gh_input(&input);
            assert!(result.is_ok(), "subcommand '{sub}' should be allowed");
        }
    }

    #[test]
    fn test_run_gh_repo_flag_appended() {
        let input = serde_json::json!({"command": ["pr", "list"], "repo": "owner/repo"});
        let gh_args = validate_gh_input(&input).unwrap();
        assert_eq!(gh_args.args, vec!["pr", "list"]);
        assert_eq!(gh_args.repo.as_deref(), Some("owner/repo"));
    }

    #[test]
    fn test_run_gh_repo_not_appended_when_empty() {
        let input = serde_json::json!({"command": ["issue", "list"]});
        let gh_args = validate_gh_input(&input).unwrap();
        assert_eq!(gh_args.args, vec!["issue", "list"]);
        assert!(gh_args.repo.is_none());
    }

    #[tokio::test]
    async fn test_run_gh_string_command_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"command": "pr list --state open"});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("JSON array of strings"));
    }

    #[tokio::test]
    async fn test_run_gh_repo_flag_smuggling() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"command": ["pr", "list", "--repo", "evil/repo"]});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Do not include --repo"));
        assert!(output.content.contains("'repo' parameter"));
    }

    #[tokio::test]
    async fn test_run_gh_repo_shorthand_smuggling() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"command": ["pr", "list", "-R", "evil/repo"]});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Do not include --repo"));
    }

    #[tokio::test]
    async fn test_run_gh_command_too_long() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let long_arg = "x".repeat(10_001);
        let input = serde_json::json!({"command": ["pr", long_arg]});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Command too long"));
    }

    #[tokio::test]
    async fn test_run_gh_non_string_array_element() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let input = serde_json::json!({"command": ["pr", 42]});
        let output = run_gh(&input, &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("must be strings"));
    }

    // -- run_gws tests --

    #[test]
    fn test_run_gws_in_known_builtins() {
        assert!(KNOWN_BUILTINS.contains(&"run_gws"));
    }

    #[test]
    fn test_validate_gws_input_string_rejected() {
        let input = serde_json::json!({"command": "gmail messages list"});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("JSON array of strings"));
    }

    #[test]
    fn test_validate_gws_input_empty_array() {
        let input = serde_json::json!({"command": []});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must not be empty"));
    }

    #[test]
    fn test_validate_gws_input_missing_command() {
        let input = serde_json::json!({});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing"));
    }

    #[test]
    fn test_validate_gws_input_allowed_subcommands() {
        for sub in &["gmail", "calendar", "drive"] {
            let input = serde_json::json!({"command": [sub, "messages", "list"]});
            let result = validate_gws_input(&input);
            assert!(result.is_ok(), "service '{sub}' should be allowed");
        }
    }

    #[test]
    fn test_validate_gws_input_disallowed_subcommands() {
        for sub in &[
            "auth", "config", "admin", "chat", "docs", "sheets", "schema",
        ] {
            let input = serde_json::json!({"command": [sub, "list"]});
            let result = validate_gws_input(&input);
            assert!(result.is_err(), "service '{sub}' should be blocked");
            let err = result.unwrap_err();
            assert!(err.content.contains("is not allowed"));
            assert!(err.content.contains("Permitted:"));
        }
    }

    #[test]
    fn test_validate_gws_input_token_smuggling() {
        let input =
            serde_json::json!({"command": ["gmail", "messages", "list", "--token", "evil"]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("--token"));
        assert!(err.content.contains("handled automatically"));
    }

    #[test]
    fn test_validate_gws_input_credentials_file_smuggling() {
        let input =
            serde_json::json!({"command": ["gmail", "+send", "--credentials-file", "/etc/creds"]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("--credentials-file"));
    }

    #[test]
    fn test_validate_gws_input_config_smuggling() {
        let input = serde_json::json!({"command": ["drive", "files", "list", "--config", "/tmp"]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("--config"));
    }

    #[test]
    fn test_validate_gws_input_config_dir_smuggling() {
        let input =
            serde_json::json!({"command": ["calendar", "events", "list", "--config-dir", "/tmp"]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("--config-dir"));
    }

    #[test]
    fn test_validate_gws_input_length_limit() {
        let long_arg = "x".repeat(10_001);
        let input = serde_json::json!({"command": ["gmail", long_arg]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Command too long"));
    }

    #[test]
    fn test_validate_gws_input_non_string_elements() {
        let input = serde_json::json!({"command": ["gmail", 42, true]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must be strings"));
    }

    #[test]
    fn test_validate_gws_input_token_equals_smuggling() {
        let input = serde_json::json!({"command": ["gmail", "messages", "list", "--token=evil"]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("--token"));
        assert!(err.content.contains("handled automatically"));
    }

    #[test]
    fn test_validate_gws_input_credentials_file_equals_smuggling() {
        let input =
            serde_json::json!({"command": ["gmail", "+send", "--credentials-file=/etc/creds"]});
        let result = validate_gws_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("--credentials-file"));
    }

    #[test]
    fn test_run_gh_repo_equals_smuggling() {
        let input = serde_json::json!({"command": ["pr", "list", "--repo=evil/repo"]});
        let result = validate_gh_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Do not include --repo"));
    }

    #[test]
    fn test_strip_frontmatter_with_frontmatter() {
        let content = "---\ntitle: Test\ndescription: A test doc\n---\n\n# Hello World\n";
        assert_eq!(strip_frontmatter(content), "# Hello World\n");
    }

    #[test]
    fn test_strip_frontmatter_without_frontmatter() {
        let content = "# Hello World\n\nSome content here.";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn test_strip_frontmatter_no_closing_delimiter() {
        let content = "---\ntitle: Test\nno closing delimiter";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn test_strip_frontmatter_empty_frontmatter() {
        let content = "---\n---\n\n# Doc\n";
        assert_eq!(strip_frontmatter(content), "# Doc\n");
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
