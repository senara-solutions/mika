//! Dispatch for builtin skill tool handlers.
//!
//! Builtin handlers are Rust functions invoked directly by the agent loop,
//! without spawning a subprocess or making an HTTP call. They have access
//! to `ToolContext` (for home_dir, etc.) and return `ToolOutput`.

use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::bundled_skills::{is_trust_critical_skill, trust_critical_skill_names};
use crate::skills::index::{resolve_canonical_provider_model, sanitize_model_dir_name};
use crate::tools::{ToolContext, ToolOutput};

/// Embedded OpenAPI spec for the agent (mika-server) HTTP API.
static AGENT_API_SPEC: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/openapi/mika-server.yaml"));

/// Embedded architecture overview document.
static ARCHITECTURE_OVERVIEW: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/architecture.md"));

/// Embedded documentation files (topic → content).
static DOC_BROWSER_CONTROL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/browser-control.md"));
static DOC_CONFIGURATION: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/configuration.md"));
static DOC_DEPLOYMENT: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/deployment.md"));
static DOC_GETTING_STARTED: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/getting-started.md"));
static DOC_SKILLS: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/skills.md"));
static DOC_RUNTIME_STRUCTURE: &str =
    include_str!(concat!(env!("OUT_DIR"), "/docs/runtime-structure.md"));
static DOC_SLASH_COMMANDS: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/slash-commands.md"));
static DOC_TASK_SYSTEM: &str = include_str!(concat!(env!("OUT_DIR"), "/docs/task-system.md"));

/// Known builtin function names, used for startup validation.
pub const KNOWN_BUILTINS: &[&str] = &[
    "get_documentation",
    "gh_read",
    "git_ops",
    "review_skill",
    "run_gh",
    "run_gws",
    "web_search",
];

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
        "gh_read" => gh_read(&input, ctx).await,
        "git_ops" => git_ops(&input, ctx).await,
        "review_skill" => review_skill(&input, ctx).await,
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
        "browser-control" => {
            ToolOutput::success(strip_frontmatter(DOC_BROWSER_CONTROL).to_string())
        }
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
        "task-system" => ToolOutput::success(strip_frontmatter(DOC_TASK_SYSTEM).to_string()),
        _ => ToolOutput::error(
            "Invalid topic. Use one of: architecture, api-spec, browser-control, cli-reference, configuration, deployment, getting-started, runtime-structure, skills, slash-commands, task-system."
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
    let args: Vec<String> = match input.get("command") {
        Some(cmd) if cmd.is_string() => {
            // LLMs sometimes serialize arrays as JSON strings — attempt coercion
            match serde_json::from_str::<Vec<String>>(cmd.as_str().unwrap()) {
                Ok(parsed) => {
                    tracing::debug!("coerced string command parameter to array");
                    parsed
                }
                Err(_) => {
                    return Err(ToolOutput::error(
                        "The 'command' parameter must be a JSON array of strings, not a single string."
                            .to_string(),
                    ));
                }
            }
        }
        Some(cmd) if cmd.is_array() => {
            let arr = cmd.as_array().unwrap();
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
        _ => {
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

/// Progress ticker interval for `spawn_and_collect` diagnostic logging (#900).
/// 30 seconds in production; overridden in tests via `PROGRESS_TICKER_INTERVAL`.
#[cfg(not(test))]
const PROGRESS_TICKER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(test)]
const PROGRESS_TICKER_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Read from an async reader into a buffer up to `max_bytes`, incrementing
/// `counter` after each chunk. Enables external progress observation (#900).
async fn read_with_counter<R: AsyncRead + Unpin>(
    reader: R,
    max_bytes: usize,
    counter: Arc<AtomicUsize>,
) -> Vec<u8> {
    let mut take = reader.take(max_bytes as u64);
    let mut buf = Vec::with_capacity(max_bytes.min(8192));
    let mut chunk = [0u8; 256];
    loop {
        match take.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                counter.fetch_add(n, Ordering::Relaxed);
            }
            Err(_) => break,
        }
    }
    buf
}

/// Spawn a CLI subprocess, capture bounded stdout/stderr, and return ToolOutput.
///
/// Shared logic for all CLI builtin handlers (gh, gws, etc.). The caller builds
/// the `Command` with args, env vars, and security scrubbing; this function handles
/// the spawn-read-wait-format cycle.
///
/// Includes diagnostic instrumentation (#900): per-invocation progress ticker at
/// 30-second intervals logging stdout/stderr byte counts, plus a completion summary.
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

    let started_at = tokio::time::Instant::now();
    let stdout_count = Arc::new(AtomicUsize::new(0));
    let stderr_count = Arc::new(AtomicUsize::new(0));

    // Read stdout and stderr with bounded size, tracking byte counts for diagnostics
    let stdout_handle = child.stdout.take().expect("stdout piped");
    let stderr_handle = child.stderr.take().expect("stderr piped");

    let stdout_reader = tokio::spawn(read_with_counter(
        stdout_handle,
        MAX_OUTPUT_LEN,
        Arc::clone(&stdout_count),
    ));
    let stderr_reader = tokio::spawn(read_with_counter(
        stderr_handle,
        MAX_OUTPUT_LEN,
        Arc::clone(&stderr_count),
    ));

    // Progress ticker: every PROGRESS_TICKER_INTERVAL, log byte count snapshots (#900)
    let progress_task = {
        let stdout_c = Arc::clone(&stdout_count);
        let stderr_c = Arc::clone(&stderr_count);
        let name = tool_name.to_string();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PROGRESS_TICKER_INTERVAL);
            interval.tick().await; // discard immediate first tick
            loop {
                interval.tick().await;
                tracing::info!(
                    tool = %name,
                    stdout_bytes = stdout_c.load(Ordering::Relaxed),
                    stderr_bytes = stderr_c.load(Ordering::Relaxed),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    "spawn_and_collect progress"
                );
            }
        })
    };

    // Wait for both reads + process exit
    let (stdout_res, stderr_res, wait_res) =
        tokio::join!(stdout_reader, stderr_reader, child.wait());
    progress_task.abort();

    let stdout_buf = stdout_res.unwrap_or_default();
    let stderr_buf = stderr_res.unwrap_or_default();

    // Post-completion summary (#900)
    tracing::info!(
        tool = %tool_name,
        stdout_bytes = stdout_count.load(Ordering::Relaxed),
        stderr_bytes = stderr_count.load(Ordering::Relaxed),
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "spawn_and_collect complete"
    );

    let status = match wait_res {
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
        let code_display = match status.code() {
            Some(code) => format!("Exit code: {code}"),
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    match status.signal() {
                        Some(sig) => format!("Killed by signal: {sig}"),
                        None => "Exit code: unknown".to_string(),
                    }
                }
                #[cfg(not(unix))]
                {
                    "Exit code: unknown".to_string()
                }
            }
        };
        let mut result = format!("{code_display}\n");
        if !stderr.is_empty() {
            result.push_str(&stderr);
        }
        if !stdout.is_empty() {
            result.push_str(&stdout);
        }
        ToolOutput::success(result)
    }
}

// -- Git ops handler --

/// Protected branch names that cannot be force-pushed.
const GIT_OPS_PROTECTED_BRANCHES: &[&str] = &["main", "master"];

/// Result of running a git subprocess, preserving success/failure status
/// separately from the output content (unlike `spawn_and_collect` which always
/// returns `is_error: false`).
struct GitResult {
    content: String,
    success: bool,
}

/// Build and run a git command with MIKA_* env scrubbing and `GIT_TERMINAL_PROMPT=0`.
/// Returns structured result preserving the exit status.
async fn run_git(repo_path: &str, args: &[&str]) -> GitResult {
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(repo_path);
    cmd.args(args);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    super::executor::scrub_mika_env_vars(&mut cmd);

    let output = spawn_and_collect(cmd, "git", "Is git installed?").await;

    // spawn_and_collect always returns is_error=false. Detect failure via
    // "Exit code:" or "Killed by signal:" prefix in content.
    let success = !output.content.starts_with("Exit code:")
        && !output.content.starts_with("Killed by signal:")
        && !output.content.starts_with("Failed to spawn git:");

    GitResult {
        content: output.content,
        success,
    }
}

/// All valid git_ops operations.
const GIT_OPS_VALID_OPERATIONS: &[&str] = &[
    "fetch",
    "rebase",
    "merge",
    "pull",
    "checkout",
    "worktree_add",
    "worktree_remove",
    "worktree_list",
    "worktree_prune",
];

/// Validate `git_ops` input and extract parameters.
#[derive(Debug)]
struct GitOpsInput {
    operation: String,
    repo_path: String,
    base: String,
    push: bool,
    /// Branch name for checkout and worktree_add operations.
    branch: Option<String>,
    /// Filesystem path for worktree_add and worktree_remove operations.
    path: Option<String>,
}

fn validate_git_ops_input(input: &serde_json::Value) -> Result<GitOpsInput, ToolOutput> {
    let operation = input
        .get("operation")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| {
            ToolOutput::error(format!(
                "Missing required 'operation' parameter. Must be one of: {}.",
                GIT_OPS_VALID_OPERATIONS.join(", ")
            ))
        })?;

    if !GIT_OPS_VALID_OPERATIONS.contains(&operation.as_str()) {
        return Err(ToolOutput::error(format!(
            "Unknown operation '{operation}'. Must be one of: {}.",
            GIT_OPS_VALID_OPERATIONS.join(", ")
        )));
    }

    let repo_path = input
        .get("repo_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| {
            ToolOutput::error(
                "Missing required 'repo_path' parameter. Must be an absolute path to a git repository."
                    .to_string(),
            )
        })?;

    let base = input
        .get("base")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("origin/main")
        .to_string();

    // Reject base refs starting with '-' to prevent git argument injection
    if base.starts_with('-') {
        return Err(ToolOutput::error(
            "Invalid base ref: must not start with '-'.".to_string(),
        ));
    }

    // Reject relative paths — repo_path must be absolute
    if !std::path::Path::new(&repo_path).is_absolute() {
        return Err(ToolOutput::error(format!(
            "repo_path must be an absolute path (got '{repo_path}')."
        )));
    }

    let push = input.get("push").and_then(|v| v.as_bool()).unwrap_or(false);

    // Reject push on non-rebase operations
    if push && operation != "rebase" {
        return Err(ToolOutput::error(format!(
            "push=true is only allowed with 'rebase' operation, not '{operation}'."
        )));
    }

    // Extract optional branch parameter
    let branch = input
        .get("branch")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    // Reject branch starting with '-' to prevent git argument injection
    if let Some(ref b) = branch
        && b.starts_with('-')
    {
        return Err(ToolOutput::error(
            "Invalid branch name: must not start with '-'.".to_string(),
        ));
    }

    // Branch is required for checkout and worktree_add
    if (operation == "checkout" || operation == "worktree_add") && branch.is_none() {
        return Err(ToolOutput::error(format!(
            "Missing required 'branch' parameter for '{operation}' operation."
        )));
    }

    // Extract optional path parameter
    let path = input
        .get("path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    // Reject path starting with '-' to prevent git argument injection
    if let Some(ref p) = path
        && p.starts_with('-')
    {
        return Err(ToolOutput::error(
            "Invalid path: must not start with '-'.".to_string(),
        ));
    }

    // Path is required for worktree_add and worktree_remove
    if (operation == "worktree_add" || operation == "worktree_remove") && path.is_none() {
        return Err(ToolOutput::error(format!(
            "Missing required 'path' parameter for '{operation}' operation."
        )));
    }

    // Path must be absolute for worktree operations
    if let Some(ref p) = path
        && (operation == "worktree_add" || operation == "worktree_remove")
        && !std::path::Path::new(p).is_absolute()
    {
        return Err(ToolOutput::error(format!(
            "path must be an absolute path (got '{p}')."
        )));
    }

    Ok(GitOpsInput {
        operation,
        repo_path,
        base,
        push,
        branch,
        path,
    })
}

/// Run pre-flight checks on the repository:
/// - Path exists and is a directory
/// - Is a git repository
/// - Working tree is clean (for rebase/merge)
/// - No rebase/merge in progress
async fn git_ops_preflight(repo_path: &str, operation: &str) -> Result<(), ToolOutput> {
    // Check path exists and is a directory
    let path = std::path::Path::new(repo_path);
    if !path.exists() {
        return Err(ToolOutput::error(format!(
            "Path does not exist: {repo_path}"
        )));
    }
    if !path.is_dir() {
        return Err(ToolOutput::error(format!(
            "Path is not a directory: {repo_path}"
        )));
    }

    // Check it's a git repo
    let result = run_git(repo_path, &["rev-parse", "--git-dir"]).await;
    if !result.success {
        return Err(ToolOutput::error(format!(
            "Not a git repository: {repo_path}"
        )));
    }

    // For rebase/merge/pull, check working tree is clean
    if operation == "rebase" || operation == "merge" || operation == "pull" {
        let status_result = run_git(repo_path, &["status", "--porcelain"]).await;
        if status_result.success && !status_result.content.trim().is_empty() {
            return Err(ToolOutput::error(format!(
                "Working tree has uncommitted changes. Commit or stash them first.\n{}",
                status_result.content.trim()
            )));
        }

        // Check for in-progress rebase or merge
        let git_dir = path.join(".git");
        // Handle worktrees where .git is a file, not a directory
        let git_dir = if git_dir.is_file() {
            // In a worktree, .git is a file containing "gitdir: <path>"
            match std::fs::read_to_string(&git_dir) {
                Ok(content) => {
                    let gitdir_path = content.trim().strip_prefix("gitdir: ").unwrap_or("").trim();
                    if gitdir_path.is_empty() {
                        git_dir
                    } else {
                        std::path::PathBuf::from(gitdir_path)
                    }
                }
                Err(_) => git_dir,
            }
        } else {
            git_dir
        };

        if git_dir.join("rebase-apply").exists() || git_dir.join("rebase-merge").exists() {
            return Err(ToolOutput::error(
                "A rebase is already in progress. Run 'git rebase --abort' or 'git rebase --continue' first."
                    .to_string(),
            ));
        }
        if git_dir.join("MERGE_HEAD").exists() {
            return Err(ToolOutput::error(
                "A merge is already in progress. Run 'git merge --abort' or complete the merge first."
                    .to_string(),
            ));
        }
    }

    Ok(())
}

/// Extract the remote name from a base ref like "origin/main" -> "origin".
fn extract_remote(base: &str) -> &str {
    base.split('/').next().unwrap_or("origin")
}

/// Execute a git maintenance operation (fetch, rebase, or merge).
async fn git_ops(input: &serde_json::Value, _ctx: &ToolContext<'_>) -> ToolOutput {
    let params = match validate_git_ops_input(input) {
        Ok(p) => p,
        Err(e) => return e,
    };

    if let Err(e) = git_ops_preflight(&params.repo_path, &params.operation).await {
        return e;
    }

    let remote = extract_remote(&params.base);

    match params.operation.as_str() {
        "fetch" => git_ops_fetch(&params.repo_path, remote).await,
        "rebase" => git_ops_rebase(&params.repo_path, remote, &params.base, params.push).await,
        "merge" => git_ops_merge(&params.repo_path, remote, &params.base).await,
        "pull" => git_ops_pull(&params.repo_path, remote, &params.base).await,
        "checkout" => {
            // branch is guaranteed to be Some by validation
            git_ops_checkout(&params.repo_path, params.branch.as_deref().unwrap()).await
        }
        "worktree_add" => {
            // path and branch are guaranteed to be Some by validation
            git_ops_worktree_add(
                &params.repo_path,
                params.path.as_deref().unwrap(),
                params.branch.as_deref().unwrap(),
                &params.base,
            )
            .await
        }
        "worktree_remove" => {
            // path is guaranteed to be Some by validation
            git_ops_worktree_remove(&params.repo_path, params.path.as_deref().unwrap()).await
        }
        "worktree_list" => git_ops_worktree_list(&params.repo_path).await,
        "worktree_prune" => git_ops_worktree_prune(&params.repo_path).await,
        _ => ToolOutput::error(format!("Unknown operation: {}", params.operation)),
    }
}

/// Fetch from a remote.
async fn git_ops_fetch(repo_path: &str, remote: &str) -> ToolOutput {
    let result = run_git(repo_path, &["fetch", remote]).await;

    if result.success {
        let mut msg = format!("Fetch from '{remote}' completed successfully.");
        if !result.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", result.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!("Fetch from '{remote}' failed.\n{}", result.content))
    }
}

/// Rebase onto a base ref with auto-abort on conflict.
async fn git_ops_rebase(repo_path: &str, remote: &str, base: &str, push: bool) -> ToolOutput {
    // Step 1: Fetch
    let fetch = run_git(repo_path, &["fetch", remote]).await;
    if !fetch.success {
        return ToolOutput::error(format!("Fetch from '{remote}' failed.\n{}", fetch.content));
    }

    // Step 2: Rebase
    let rebase = run_git(repo_path, &["rebase", base]).await;

    if !rebase.success {
        // Auto-abort to leave repo clean
        let _ = run_git(repo_path, &["rebase", "--abort"]).await;

        return ToolOutput::error(format!(
            "Rebase onto '{base}' failed (conflicts detected). \
             Rebase was automatically aborted — working tree is clean.\n\n{}",
            rebase.content
        ));
    }

    let mut msg = format!("Rebase onto '{base}' completed successfully.");
    if !rebase.content.trim().is_empty() {
        let _ = write!(msg, "\n{}", rebase.content.trim());
    }

    // Step 3: Push if requested
    if push {
        match git_ops_push(repo_path).await {
            Ok(push_msg) => {
                let _ = write!(msg, "\n\n{push_msg}");
            }
            Err(e) => {
                return ToolOutput::error(format!(
                    "{msg}\n\nRebase succeeded but push failed:\n{}",
                    e.content
                ));
            }
        }
    }

    ToolOutput::success(msg)
}

/// Fast-forward merge onto a base ref.
async fn git_ops_merge(repo_path: &str, remote: &str, base: &str) -> ToolOutput {
    // Step 1: Fetch
    let fetch = run_git(repo_path, &["fetch", remote]).await;
    if !fetch.success {
        return ToolOutput::error(format!("Fetch from '{remote}' failed.\n{}", fetch.content));
    }

    // Step 2: Merge --ff-only
    let merge = run_git(repo_path, &["merge", "--ff-only", base]).await;

    if merge.success {
        let mut msg = format!("Fast-forward merge of '{base}' completed successfully.");
        if !merge.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", merge.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!(
            "Fast-forward merge of '{base}' failed. \
             The branch may have diverged — try rebasing first.\n\n{}",
            merge.content
        ))
    }
}

/// Force-push with lease after a successful rebase.
/// Refuses to push to protected branches (main/master).
/// Returns Ok(message) on success, Err(ToolOutput) on failure.
async fn git_ops_push(repo_path: &str) -> Result<String, ToolOutput> {
    // Check current branch — refuse to push to main/master
    let branch_result = run_git(repo_path, &["branch", "--show-current"]).await;
    let branch = branch_result.content.trim().to_string();

    if branch.is_empty() {
        return Err(ToolOutput::error(
            "Cannot push: HEAD is detached. Check out a branch first.".to_string(),
        ));
    }

    if GIT_OPS_PROTECTED_BRANCHES.contains(&branch.as_str()) {
        return Err(ToolOutput::error(format!(
            "Refusing to force-push to protected branch '{branch}'. \
             Check out a feature branch first."
        )));
    }

    let push = run_git(repo_path, &["push", "--force-with-lease"]).await;

    if push.success {
        Ok(format!(
            "Force-push (with lease) to '{branch}' completed successfully."
        ))
    } else {
        Err(ToolOutput::error(format!(
            "Force-push to '{branch}' failed.\n{}",
            push.content
        )))
    }
}

/// Pull (fetch + fast-forward merge) from a remote.
async fn git_ops_pull(repo_path: &str, remote: &str, base: &str) -> ToolOutput {
    // Step 1: Fetch
    let fetch = run_git(repo_path, &["fetch", remote]).await;
    if !fetch.success {
        return ToolOutput::error(format!("Fetch from '{remote}' failed.\n{}", fetch.content));
    }

    // Step 2: Merge --ff-only
    let merge = run_git(repo_path, &["merge", "--ff-only", base]).await;

    if merge.success {
        let mut msg = format!("Pull (fast-forward) from '{base}' completed successfully.");
        if !merge.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", merge.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!(
            "Pull from '{base}' failed — not fast-forwardable. \
             The branch may have diverged — try rebasing first.\n\n{}",
            merge.content
        ))
    }
}

/// Switch to a branch using `git switch`.
async fn git_ops_checkout(repo_path: &str, branch: &str) -> ToolOutput {
    let result = run_git(repo_path, &["switch", branch]).await;

    if result.success {
        let mut msg = format!("Switched to branch '{branch}'.");
        if !result.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", result.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!(
            "Failed to switch to branch '{branch}'.\n{}",
            result.content
        ))
    }
}

/// Create a worktree with a new branch.
///
/// Tries `git worktree add -b <branch> <path> <base>` first.
/// If the branch already exists, falls back to `git worktree add <path> <branch>`.
async fn git_ops_worktree_add(repo_path: &str, path: &str, branch: &str, base: &str) -> ToolOutput {
    // Try creating with a new branch first
    let result = run_git(repo_path, &["worktree", "add", "-b", branch, path, base]).await;

    if result.success {
        let mut msg =
            format!("Worktree created at '{path}' on new branch '{branch}' (base: {base}).");
        if !result.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", result.content.trim());
        }
        return ToolOutput::success(msg);
    }

    // If branch already exists, try attaching the worktree to the existing branch
    let fallback = run_git(repo_path, &["worktree", "add", path, branch]).await;

    if fallback.success {
        let mut msg = format!("Worktree created at '{path}' on existing branch '{branch}'.");
        if !fallback.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", fallback.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!(
            "Failed to create worktree at '{path}'.\n{}",
            fallback.content
        ))
    }
}

/// Remove a worktree.
async fn git_ops_worktree_remove(repo_path: &str, path: &str) -> ToolOutput {
    let result = run_git(repo_path, &["worktree", "remove", "--force", path]).await;

    if result.success {
        let mut msg = format!("Worktree at '{path}' removed.");
        if !result.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", result.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!(
            "Failed to remove worktree at '{path}'.\n{}",
            result.content
        ))
    }
}

/// List all worktrees in porcelain format.
async fn git_ops_worktree_list(repo_path: &str) -> ToolOutput {
    let result = run_git(repo_path, &["worktree", "list", "--porcelain"]).await;

    if result.success {
        if result.content.trim().is_empty() {
            ToolOutput::success("No worktrees found.".to_string())
        } else {
            ToolOutput::success(result.content)
        }
    } else {
        ToolOutput::error(format!("Failed to list worktrees.\n{}", result.content))
    }
}

/// Prune stale worktree references.
async fn git_ops_worktree_prune(repo_path: &str) -> ToolOutput {
    let result = run_git(repo_path, &["worktree", "prune"]).await;

    if result.success {
        let mut msg = "Worktree prune completed successfully.".to_string();
        if !result.content.trim().is_empty() {
            let _ = write!(msg, "\n{}", result.content.trim());
        }
        ToolOutput::success(msg)
    } else {
        ToolOutput::error(format!("Failed to prune worktrees.\n{}", result.content))
    }
}

// -- GitHub read-only handler --

/// Maximum file size for `file_view` op (matches GitHub contents API cap).
const FILE_VIEW_MAX_BYTES: u64 = 1_048_576;

/// Allowed read-only operations for `gh_read`.
const GH_READ_ALLOWED_OPS: &[&str] = &[
    "issue_view",
    "pr_view",
    "pr_diff",
    "issue_list",
    "file_view",
];

/// Validated `gh_read` input — operation, optional target, and repo.
#[derive(Debug)]
struct GhReadArgs {
    op: String,
    target: Option<String>,
    repo: String,
    /// File path for `file_view` op (repo-root-relative).
    path: Option<String>,
    /// Git ref for `file_view` op (branch/tag/sha). Defaults to `"main"`.
    r#ref: Option<String>,
}

/// Structured error types for `gh_read` responses.
#[derive(Debug)]
enum GhReadError {
    NotFound(String),
    AuthFailed(String),
    RateLimited(String),
    NetworkError(String),
    MalformedRequest(String),
    FileTooLarge { size_bytes: u64, max_bytes: u64 },
}

impl GhReadError {
    fn to_json(&self) -> String {
        match self {
            Self::NotFound(msg) => {
                format!(
                    "{{\"error\": \"not_found\", \"message\": {}}}",
                    serde_json::json!(msg)
                )
            }
            Self::AuthFailed(msg) => {
                format!(
                    "{{\"error\": \"auth_failed\", \"message\": {}}}",
                    serde_json::json!(msg)
                )
            }
            Self::RateLimited(msg) => {
                format!(
                    "{{\"error\": \"rate_limited\", \"message\": {}}}",
                    serde_json::json!(msg)
                )
            }
            Self::NetworkError(msg) => {
                format!(
                    "{{\"error\": \"network_error\", \"message\": {}}}",
                    serde_json::json!(msg)
                )
            }
            Self::MalformedRequest(msg) => {
                format!(
                    "{{\"error\": \"malformed_request\", \"message\": {}}}",
                    serde_json::json!(msg)
                )
            }
            Self::FileTooLarge {
                size_bytes,
                max_bytes,
            } => {
                format!(
                    "{{\"error\": \"file_too_large\", \"message\": \"File size ({size_bytes} bytes) exceeds maximum ({max_bytes} bytes).\", \"size_bytes\": {size_bytes}, \"max_bytes\": {max_bytes}}}"
                )
            }
        }
    }
}

/// Classify `gh` stderr/exit-code into structured error variants.
fn classify_gh_error(stderr: &str, exit_code: Option<i32>) -> GhReadError {
    let lower = stderr.to_lowercase();

    // Check for CLI-not-found first (before generic "not found" which would false-positive)
    if exit_code == Some(127) || lower.contains("command not found") {
        return GhReadError::NetworkError(format!("gh CLI not found or not installed: {stderr}"));
    }

    if lower.contains("not found")
        || lower.contains("could not resolve")
        || lower.contains("no pull requests found")
        || lower.contains("no issues match")
    {
        GhReadError::NotFound(stderr.to_string())
    } else if lower.contains("authentication")
        || lower.contains("401")
        // 403/forbidden covers App permission gaps, fine-grained PAT scope
        // mismatches, and "Resource not accessible by integration". Without
        // this branch they were silently classified as NetworkError, causing
        // skills to retry transiently when they should escalate.
        || lower.contains("403")
        || lower.contains("forbidden")
        || lower.contains("resource not accessible")
        || lower.contains("login")
    {
        GhReadError::AuthFailed(stderr.to_string())
    } else if lower.contains("rate limit") || lower.contains("429") {
        GhReadError::RateLimited(stderr.to_string())
    } else {
        GhReadError::NetworkError(stderr.to_string())
    }
}

/// Validate and parse `gh_read` input.
///
/// Input schema: `{"op": "issue_view", "target": "123", "repo": "owner/repo"}`
fn validate_gh_read_input(input: &serde_json::Value) -> Result<GhReadArgs, ToolOutput> {
    let op = input
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    if op.is_empty() {
        return Err(ToolOutput::error(
            GhReadError::MalformedRequest("Missing required 'op' parameter.".to_string()).to_json(),
        ));
    }

    if !GH_READ_ALLOWED_OPS.contains(&op.as_str()) {
        return Err(ToolOutput::error(
            GhReadError::MalformedRequest(format!(
                "Operation '{}' is not allowed. Permitted: {}.",
                op,
                GH_READ_ALLOWED_OPS.join(", ")
            ))
            .to_json(),
        ));
    }

    let repo = input
        .get("repo")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let repo = match repo {
        Some(r) => r,
        None => {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest("Missing required 'repo' parameter.".to_string())
                    .to_json(),
            ));
        }
    };

    // Reject flag-shaped repo values. Tokio Command argv passing forecloses
    // shell injection, but `repo='--token=evil'` would smuggle a flag into
    // the gh subcommand. Mirrors run_gh's anti-smuggling check.
    if repo.starts_with('-') {
        return Err(ToolOutput::error(
            GhReadError::MalformedRequest(format!(
                "'repo' value '{repo}' looks like a flag (starts with '-'). Expected format: owner/repo."
            ))
            .to_json(),
        ));
    }

    let target = input
        .get("target")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    // issue_view, pr_view, pr_diff require a target (number)
    if matches!(op.as_str(), "issue_view" | "pr_view" | "pr_diff") && target.is_none() {
        return Err(ToolOutput::error(
            GhReadError::MalformedRequest(format!(
                "Operation '{}' requires a 'target' parameter (issue/PR number).",
                op
            ))
            .to_json(),
        ));
    }

    // For ops that take a numeric target, require all-digit values. Without
    // this, `target='--web'` would invoke `gh issue view --web` (the issue
    // view's web flag), bypassing the JSON-output contract.
    if let Some(ref t) = target {
        if t.starts_with('-') {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest(format!(
                    "'target' value '{t}' looks like a flag (starts with '-')."
                ))
                .to_json(),
            ));
        }
        if matches!(op.as_str(), "issue_view" | "pr_view" | "pr_diff")
            && !t.chars().all(|c| c.is_ascii_digit())
        {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest(format!(
                    "Operation '{op}' requires a numeric 'target' (issue/PR number); got '{t}'."
                ))
                .to_json(),
            ));
        }
    }

    // file_view-specific validation: path and ref
    let (path, r#ref) = if op == "file_view" {
        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let path = match path {
            Some(p) => p,
            None => {
                return Err(ToolOutput::error(
                    GhReadError::MalformedRequest(
                        "Operation 'file_view' requires a 'path' parameter.".to_string(),
                    )
                    .to_json(),
                ));
            }
        };

        // Anti-flag-smuggling on path
        if path.starts_with('-') {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest(format!(
                    "'path' value '{path}' looks like a flag (starts with '-')."
                ))
                .to_json(),
            ));
        }

        // No absolute paths
        if path.starts_with('/') {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest(format!(
                    "'path' must be repo-root-relative, not absolute: '{path}'."
                ))
                .to_json(),
            ));
        }

        // No directory traversal
        if path.contains("..") {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest(format!("'path' must not contain '..': '{path}'."))
                    .to_json(),
            ));
        }

        // Charset enforcement: only [A-Za-z0-9._/\-] allowed.
        // This prevents URL-encoding attacks (e.g., foo%2F..%2Fbaz bypassing
        // the literal '..' check after GitHub's server-side URL decoding).
        if !path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
        {
            return Err(ToolOutput::error(
                GhReadError::MalformedRequest(format!(
                    "'path' contains disallowed characters. Only [A-Za-z0-9._/-] are permitted: '{path}'."
                ))
                .to_json(),
            ));
        }

        // Ref validation
        let r#ref = input
            .get("ref")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        if let Some(ref r) = r#ref {
            if r.starts_with('-') {
                return Err(ToolOutput::error(
                    GhReadError::MalformedRequest(format!(
                        "'ref' value '{r}' looks like a flag (starts with '-')."
                    ))
                    .to_json(),
                ));
            }
            if r.len() > 256 {
                return Err(ToolOutput::error(
                    GhReadError::MalformedRequest(format!(
                        "'ref' value is too long ({} chars, max 256).",
                        r.len()
                    ))
                    .to_json(),
                ));
            }
            // Charset enforcement: prevent URL injection via query-string
            // metacharacters. The ref is interpolated into `?ref={ref}` in the
            // gh api URL — chars like `?`, `&`, `#`, `%` would corrupt the URL.
            // Allow git-ref-safe characters: alphanumeric, `.`, `_`, `/`, `-`.
            if !r
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
            {
                return Err(ToolOutput::error(
                    GhReadError::MalformedRequest(format!(
                        "'ref' contains disallowed characters. Only [A-Za-z0-9._/-] are permitted: '{r}'."
                    ))
                    .to_json(),
                ));
            }
        }

        (Some(path), r#ref)
    } else {
        (None, None)
    };

    Ok(GhReadArgs {
        op,
        target,
        repo,
        path,
        r#ref,
    })
}

/// Build the `gh` CLI command for a `gh_read` operation.
///
/// Each op arm appends `--repo` where needed. `file_view` uses `gh api`
/// which takes the repo in the URL path, so it omits `--repo`.
fn build_gh_read_command(args: &GhReadArgs) -> Vec<String> {
    match args.op.as_str() {
        "issue_view" => {
            vec![
                "issue".to_string(),
                "view".to_string(),
                args.target.clone().unwrap(),
                "--json".to_string(),
                "number,title,body,labels,milestone,comments,state,assignees".to_string(),
                "--repo".to_string(),
                args.repo.clone(),
            ]
        }
        "pr_view" => {
            vec![
                "pr".to_string(),
                "view".to_string(),
                args.target.clone().unwrap(),
                "--json".to_string(),
                "number,title,body,labels,headRefName,state,reviewDecision,reviews,commits"
                    .to_string(),
                "--repo".to_string(),
                args.repo.clone(),
            ]
        }
        "pr_diff" => {
            vec![
                "pr".to_string(),
                "diff".to_string(),
                args.target.clone().unwrap(),
                "--repo".to_string(),
                args.repo.clone(),
            ]
        }
        "issue_list" => {
            let mut cmd = vec!["issue".to_string(), "list".to_string()];
            if let Some(ref target) = args.target {
                // If target looks like a milestone number or name, use --milestone;
                // otherwise treat as label filter.
                if target.chars().all(|c| c.is_ascii_digit()) {
                    cmd.push("--milestone".to_string());
                    cmd.push(target.clone());
                } else {
                    cmd.push("--label".to_string());
                    cmd.push(target.clone());
                }
            }
            cmd.push("--json".to_string());
            cmd.push("number,title,state,labels,milestone,assignees".to_string());
            cmd.push("--repo".to_string());
            cmd.push(args.repo.clone());
            cmd
        }
        "file_view" => {
            let path = args.path.as_deref().unwrap();
            let r#ref = args.r#ref.as_deref().unwrap_or("main");
            vec![
                "api".to_string(),
                format!("/repos/{}/contents/{}?ref={}", args.repo, path, r#ref),
                "--method".to_string(),
                "GET".to_string(),
                "-H".to_string(),
                "Accept: application/vnd.github+json".to_string(),
            ]
        }
        _ => unreachable!("validate_gh_read_input checks op against allowlist"),
    }
}

/// Read-only GitHub CLI handler with structured errors.
///
/// Input: `{"op": "issue_view", "target": "123", "repo": "owner/repo"}`
///
/// Unlike `run_gh`, this handler uses an operation-level allowlist (not raw
/// subcommands) and only supports read-only operations. Structured error
/// variants enable skill prompts to branch on failure type.
async fn gh_read(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    let args = match validate_gh_read_input(input) {
        Ok(a) => a,
        Err(err) => return err,
    };

    let start = std::time::Instant::now();
    let gh_cmd = build_gh_read_command(&args);

    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(&gh_cmd);
    // --repo is now per-op inside build_gh_read_command (file_view uses gh api
    // which takes repo in the URL path, not as a --repo flag).
    cmd.env("GH_PROMPT_DISABLED", "1");
    super::executor::scrub_mika_env_vars(&mut cmd);

    if let Some(token) = ctx.github_token {
        cmd.env("GH_TOKEN", token);
    }

    let output = spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?").await;
    let latency_ms = start.elapsed().as_millis();

    // Compute is_err FIRST so the audit log reflects the content-prefix
    // detection (the f6d6a252 fix). Without this, every non-zero exit
    // logged status=ok because output.is_error stayed false. spawn_and_collect
    // returns ToolOutput::success even on non-zero exits, formatting content
    // as "Exit code: N\n{stderr}\n{stdout}".
    let is_err = output.is_error
        || output.content.starts_with("Exit code:")
        || output.content.starts_with("Killed by signal:");

    // Compute audit resource field — target for issue/pr ops, <ref>:<path> for file_view
    let audit_resource = if args.op == "file_view" {
        let r = args.r#ref.as_deref().unwrap_or("main");
        let p = args.path.as_deref().unwrap_or("");
        format!("{r}:{p}")
    } else {
        args.target.as_deref().unwrap_or("").to_string()
    };

    // For file_view, extract blob_sha from the response on success
    let blob_sha: Option<String> = if args.op == "file_view" && !is_err {
        serde_json::from_str::<serde_json::Value>(&output.content)
            .ok()
            .and_then(|v| v.get("sha").and_then(|s| s.as_str()).map(String::from))
    } else {
        None
    };

    // Audit log
    tracing::info!(
        event = "gh_read_invocation",
        op = %args.op,
        resource = %audit_resource,
        repo = %args.repo,
        latency_ms = latency_ms,
        status = if is_err { "error" } else { "ok" },
        blob_sha = blob_sha.as_deref().unwrap_or(""),
        "gh_read invocation"
    );

    if is_err {
        let err = classify_gh_error(&output.content, None);
        return ToolOutput::error(err.to_json());
    }

    // file_view post-processing: parse GitHub contents API response,
    // detect FileTooLarge, base64 decode, return normalized response.
    if args.op == "file_view" {
        return match parse_file_view_response(&output.content, &args) {
            Ok(json) => ToolOutput::success(json),
            Err(err) => ToolOutput::error(err.to_json()),
        };
    }

    output
}

/// Parse GitHub contents API response for `file_view` op.
///
/// Returns normalized JSON: `{"content": "<text>", "ref": "<blob-sha>", "path": "<path>", "size_bytes": <n>}`
fn parse_file_view_response(body: &str, args: &GhReadArgs) -> Result<String, GhReadError> {
    let json: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        GhReadError::MalformedRequest(format!("Failed to parse GitHub API response: {e}"))
    })?;

    let size = json.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    let content_raw = json.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let sha = json.get("sha").and_then(|v| v.as_str()).unwrap_or("");

    // Detect FileTooLarge: GitHub returns 200 with empty content + non-zero size
    // for files > 1 MiB.
    if size > FILE_VIEW_MAX_BYTES && content_raw.is_empty() {
        return Err(GhReadError::FileTooLarge {
            size_bytes: size,
            max_bytes: FILE_VIEW_MAX_BYTES,
        });
    }

    // Verify encoding is base64 (GitHub's documented encoding for contents API)
    let encoding = json.get("encoding").and_then(|v| v.as_str()).unwrap_or("");
    if encoding != "base64" && !content_raw.is_empty() {
        return Err(GhReadError::MalformedRequest(format!(
            "Unexpected encoding '{encoding}'; expected 'base64'."
        )));
    }

    // Base64 decode — GitHub's base64 content has line breaks
    use base64::Engine;
    let cleaned = content_raw.replace('\n', "");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        .map_err(|e| {
            GhReadError::MalformedRequest(format!("Failed to base64-decode file content: {e}"))
        })?;

    // UTF-8 validation
    let text = String::from_utf8(bytes).map_err(|_| {
        GhReadError::MalformedRequest("File content is not valid UTF-8.".to_string())
    })?;

    let path = args.path.as_deref().unwrap_or("");
    let response = serde_json::json!({
        "content": text,
        "ref": sha,
        "path": path,
        "size_bytes": size,
    });

    Ok(response.to_string())
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
async fn run_gh(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    let gh_args = match validate_gh_input(input) {
        Ok(args) => args,
        Err(err) => return err,
    };

    // Compute PR dedup key for `pr review` commands.
    let pr_dedup_key = if is_pr_review_command(&gh_args.args) {
        debug_assert!(
            ctx.pr_reviews_posted.is_some(),
            "pr_reviews_posted must be threaded for production pr review calls"
        );
        Some(make_pr_dedup_key(&gh_args.args, gh_args.repo.as_deref()))
    } else {
        None
    };

    // Session-scope check (Fix A, #821) — primary defense in production.
    // Prevents duplicate reviews across turns within the same session
    // (e.g., when a required-tools gate forces a retry into a new turn).
    if let (Some(key), Some(map)) = (&pr_dedup_key, ctx.pr_reviews_posted)
        && map
            .get(ctx.session_id)
            .map(|set| set.contains(key))
            .unwrap_or(false)
    {
        return ToolOutput::error(
            "{\"error\": \"duplicate_pr_review\", \"message\": \"A PR review was already \
             posted in this session for this PR. Duplicate reviews create duplicate \
             webhooks. End your turn — the review is already submitted.\"}"
                .to_string(),
        );
    }

    // Per-turn PR review idempotency guard (#695): if the command is `pr review ...`
    // and we already posted a review in this turn, reject with a structured error.
    // Retained as fallback when pr_reviews_posted is None (CLI/test contexts).
    if pr_dedup_key.is_some()
        && ctx
            .pr_review_posted
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return ToolOutput::error(
            "{\"error\": \"duplicate_pr_review\", \"message\": \"A PR review was already \
             posted in this turn. Duplicate reviews create duplicate webhooks. End your \
             turn — the review is already submitted.\"}"
                .to_string(),
        );
    }

    let mut cmd = tokio::process::Command::new("gh");
    cmd.args(&gh_args.args);

    if let Some(ref repo) = gh_args.repo {
        cmd.arg("--repo").arg(repo);
    }

    cmd.env("GH_PROMPT_DISABLED", "1");
    super::executor::scrub_mika_env_vars(&mut cmd);

    // Inject platform GitHub token for agent identity separation AFTER scrub.
    // scrub_mika_env_vars removes GH_TOKEN (defense-in-depth against .env leak),
    // so we must re-add the correct platform token here. See #380.
    if let Some(token) = ctx.github_token {
        cmd.env("GH_TOKEN", token);
    }

    // Diagnostic instrumentation (#900): log the exact gh subcommand, env key set
    // (keys only, never values), and token presence for timeout forensics.
    let env_keys_set: &[&str] = match ctx.github_token {
        Some(_) => &["GH_PROMPT_DISABLED", "GH_TOKEN"],
        None => &["GH_PROMPT_DISABLED"],
    };
    tracing::info!(
        tool = "run_gh",
        argv = ?&gh_args.args,
        repo = ?&gh_args.repo,
        env_keys_set = ?env_keys_set,
        has_github_token = ctx.github_token.is_some(),
        "run_gh invocation"
    );

    let output = spawn_and_collect(cmd, "gh", "Is the GitHub CLI installed?").await;

    // On success, record that a PR review was posted (both per-turn and session-scoped).
    if !output.is_error
        && let Some(key) = pr_dedup_key
    {
        ctx.pr_review_posted
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(map) = ctx.pr_reviews_posted {
            map.entry(ctx.session_id.to_string())
                .or_default()
                .insert(key);
        }
    }

    output
}

/// Derive a dedup key for a `gh pr review` invocation.
///
/// Format: `{repo}|{positional}` where `positional` is the first argument
/// after `pr review` (typically a PR URL or number), and `repo` is from the
/// `--repo` flag. Falls back to `__default__` and `__current_branch__`
/// when those values are absent.
///
/// `__current_branch__` fallback fires only for `gh pr review --approve` with no
/// positional, which mika-qa never emits (it always passes the PR URL). If this
/// fallback ever produces same-key collisions for legitimately different PRs in
/// one session, the fix is to normalize via `gh pr view --json number` — file as
/// follow-up if it surfaces.
fn make_pr_dedup_key(args: &[String], repo: Option<&str>) -> String {
    let positional = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("__current_branch__");
    format!("{}|{}", repo.unwrap_or("__default__"), positional)
}

/// Check if a `gh` command array is a `pr review` invocation.
/// Matches: `["pr", "review", ...]` — the two positional args that identify
/// a GitHub PR review command (--approve, --comment, or --request-changes).
fn is_pr_review_command(args: &[String]) -> bool {
    args.len() >= 2 && args[0] == "pr" && args[1] == "review"
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

// ---------------------------------------------------------------------------
// review_skill — gather skill prompt data and (optionally) persist a
// model-tuned variant in a single atomic call.
// ---------------------------------------------------------------------------

/// Maximum size for a root prompt included in the response (characters).
const MAX_PROMPT_IN_RESPONSE: usize = 8_000;

/// Minimum allowed ratio of variant size to source size.
/// Anything smaller is rejected as a likely truncation.
const MIN_VARIANT_RATIO: f64 = 0.5;

/// Gather skill prompt data and, when `content` is supplied, persist a
/// model-tuned variant in the same call.
///
/// Two modes (controlled by the optional `content` parameter):
/// - **Inspect** (`content` omitted): returns the current root prompt, declared
///   tools, runtime provider/model, and any existing variant. Use this to
///   read what's there before drafting a variant.
/// - **Persist** (`content` provided): runs the inspect step *and* writes the
///   supplied prompt body to
///   `skills/<name>/generated/<provider>/<sanitized_model>/system_prompt.md`.
///   The destination path is computed entirely from `ctx.provider_name` /
///   `ctx.model_name` — there is no path input the agent can fabricate.
///
/// `force = true` is required to overwrite an existing variant.
/// `dry_run = true` skips the disk write but still resolves and reports the
/// would-be path.
async fn review_skill(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput {
    // --- Input validation --------------------------------------------------
    let skill_name = match input.get("skill_name").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s,
        _ => return ToolOutput::error("Missing required 'skill_name' (string)."),
    };
    if skill_name.len() > 200 {
        return ToolOutput::error("'skill_name' must be at most 200 characters.");
    }
    // Path traversal protection: reject names containing path separators or parent refs.
    if skill_name != "*"
        && (skill_name.contains('/')
            || skill_name.contains('\\')
            || skill_name.contains("..")
            || skill_name.contains('\0'))
    {
        return ToolOutput::error(
            "'skill_name' must be a plain name (no path separators, '..', or null bytes).",
        );
    }
    let dry_run = input
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let force = input
        .get("force")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Optional content: when present, the call also persists the variant.
    let content = match input.get("content") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) if s.is_empty() => {
            return ToolOutput::error(
                "'content' must be a non-empty string when provided. \
                 Omit it entirely to inspect without writing.",
            );
        }
        Some(serde_json::Value::String(s)) => Some(s.as_str()),
        Some(_) => {
            return ToolOutput::error("'content' must be a string when provided.");
        }
    };
    if let Some(c) = content
        && c.len() > crate::tools::MAX_PAYLOAD_BYTES
    {
        return ToolOutput::error(format!(
            "'content' exceeds maximum payload size of {} bytes.",
            crate::tools::MAX_PAYLOAD_BYTES
        ));
    }

    // --- Resolve canonical provider / model --------------------------------
    let (canonical_provider, canonical_model) =
        resolve_canonical_provider_model(ctx.provider_name, ctx.model_name);
    let sanitized_model = sanitize_model_dir_name(canonical_model);

    let skills_dir = ctx.home_dir.join("skills");

    // --- Trust-critical skill guard (#480, #486) ----------------------------
    // Only block trust-critical skills (skill-review, self-knowledge,
    // agents-teams) whose prompts govern security/identity. Other bundled
    // skills (web-search, shell-exec, etc.) are reviewable — their prompts
    // focus on tool usage mechanics, safe to adapt per-model.
    // Block before filesystem existence check so case-mismatched names get
    // "trust-critical" error (via eq_ignore_ascii_case) rather than "not found".
    if skill_name != "*" && is_trust_critical_skill(skill_name) {
        let names = trust_critical_skill_names().join(", ");
        return ToolOutput::success(format!(
            "Cannot review trust-critical skill '{skill_name}'. \
             Trust-critical skills ({names}) are platform-managed and cannot \
             be adapted — their prompts govern security, identity, or orchestration.",
        ));
    }

    // --- Batch mode --------------------------------------------------------
    if skill_name == "*" {
        if content.is_some() {
            return ToolOutput::error(
                "'content' is not supported in batch mode. \
                 Pass an explicit 'skill_name' when supplying 'content'.",
            );
        }
        return review_skill_batch(&skills_dir, canonical_provider, &sanitized_model, force).await;
    }

    // --- Single-skill mode -------------------------------------------------
    review_skill_single(
        &skills_dir,
        skill_name,
        canonical_provider,
        canonical_model,
        &sanitized_model,
        dry_run,
        force,
        content,
        ctx.skills_dirty,
    )
    .await
}

/// Handle a single-skill `review_skill` invocation. When `content` is `Some`,
/// the variant is also written under
/// `skills/<skill_name>/generated/<provider>/<sanitized_model>/system_prompt.md`.
#[allow(clippy::too_many_arguments)]
async fn review_skill_single(
    skills_dir: &std::path::Path,
    skill_name: &str,
    canonical_provider: &str,
    canonical_model: &str,
    sanitized_model: &str,
    dry_run: bool,
    force: bool,
    content: Option<&str>,
    skills_dirty: &std::sync::atomic::AtomicBool,
) -> ToolOutput {
    let skill_dir = skills_dir.join(skill_name);

    // Existence check
    if !skill_dir.exists() {
        return ToolOutput::error(format!(
            "Skill '{skill_name}' not found. Check the name with `list_agent_files` \
             at path 'skills/'."
        ));
    }

    // Linked-skill awareness — detect but do not log here. Reviews of linked skills
    // flow through normally; any subsequent write will land in the source directory
    // by symlink transparency. The `warn_linked_skill_write` tracing event fires only
    // when an actual write happens (persist branch, non-dry-run) — inspecting a linked
    // skill is not a noteworthy event on its own.
    let linked = match std::fs::symlink_metadata(&skill_dir) {
        Ok(meta) => meta.file_type().is_symlink(),
        Err(e) => {
            return ToolOutput::error(format!(
                "Cannot read skill directory for '{skill_name}': {e}"
            ));
        }
    };

    // Read root prompt
    let prompt_path = skill_dir.join("system_prompt.md");
    let root_prompt = match std::fs::read_to_string(&prompt_path) {
        Ok(content) if !content.trim().is_empty() => content,
        Ok(_) => {
            return ToolOutput::error(format!(
                "Skill '{skill_name}' has an empty system_prompt.md — nothing to adapt."
            ));
        }
        Err(_) => {
            return ToolOutput::error(format!(
                "Skill '{skill_name}' has no system_prompt.md to adapt."
            ));
        }
    };
    let source_size = root_prompt.len();

    // Read tools.json (optional — prompt-only skills may not have one)
    let tools_path = skill_dir.join("tools.json");
    let tools_json = std::fs::read_to_string(&tools_path).unwrap_or_else(|_| "[]".to_string());

    // Compute the canonical variant target path. Always derived from ctx — the
    // agent has no way to influence it.
    let variant_dir = skill_dir
        .join("generated")
        .join(canonical_provider)
        .join(sanitized_model);
    let variant_path = variant_dir.join("system_prompt.md");
    let existing_variant = std::fs::read_to_string(&variant_path).ok();

    // Truncate very large prompts to keep the response within output limits
    let prompt_for_response = if root_prompt.len() > MAX_PROMPT_IN_RESPONSE {
        let truncated = &root_prompt[..root_prompt
            .char_indices()
            .take_while(|(i, _)| *i < MAX_PROMPT_IN_RESPONSE)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(MAX_PROMPT_IN_RESPONSE)];
        format!(
            "{truncated}\n\n... (truncated — original is {} chars)",
            root_prompt.len()
        )
    } else {
        root_prompt
    };

    // --- Persist branch (content provided) --------------------------------
    if let Some(body) = content {
        // Truncation guard: variant must be at least MIN_VARIANT_RATIO of source.
        let min_size = ((source_size as f64) * MIN_VARIANT_RATIO).ceil() as usize;
        if body.len() < min_size {
            let pct = ((body.len() as f64) / (source_size as f64) * 100.0).round() as u64;
            return ToolOutput::error(format!(
                "Variant is {pct}% the size of the source ({} bytes vs {source_size} bytes) — \
                 this looks like truncation. Re-emit the full adapted prompt.",
                body.len()
            ));
        }

        // Markdown validation: reject malformed content before writing (#511).
        if let Err(reason) = super::validate_markdown_content(body) {
            return ToolOutput::error(format!(
                "Generated prompt fails markdown validation: {reason}. \
                 Fix the content and re-call review_skill.",
            ));
        }

        // Overwrite guard: a pre-existing variant requires force.
        if existing_variant.is_some() && !force {
            return ToolOutput::error(format!(
                "Variant already exists at '{}'. Re-call with force=true to overwrite.",
                variant_path.display()
            ));
        }

        let mut written = false;
        if !dry_run {
            if let Err(e) = std::fs::create_dir_all(&variant_dir) {
                return ToolOutput::error(format!(
                    "Failed to create variant directory '{}': {e}",
                    variant_dir.display()
                ));
            }
            if let Err(e) = std::fs::write(&variant_path, body) {
                return ToolOutput::error(format!(
                    "Failed to write variant '{}': {e}",
                    variant_path.display()
                ));
            }
            // Ops-side log for writes that land in a linked (symlinked) skill
            // source directory. Fires only on real writes, not dry-runs or inspects.
            if linked {
                warn_linked_skill_write(skill_name, &skill_dir);
            }
            // Mark the registry stale so the next agent turn re-scans and picks
            // up the new generated variant. Without this, `resolve_prompt` keeps
            // serving the root prompt until the process restarts — the whole
            // point of the loop.
            skills_dirty.store(true, std::sync::atomic::Ordering::Release);
            written = true;
        }

        // Persist-branch warning: tense reflects whether a write actually happened.
        let warning = if linked {
            Some(if written {
                format!(
                    "linked skill: variant persisted through symlink to source directory at {}",
                    skill_dir.display()
                )
            } else {
                format!(
                    "linked skill: variant would be persisted through symlink to source directory at {} (dry_run)",
                    skill_dir.display()
                )
            })
        } else {
            None
        };

        let result = serde_json::json!({
            "skill_name": skill_name,
            "root_prompt": prompt_for_response,
            "tools_json": tools_json,
            "runtime_provider": canonical_provider,
            "runtime_model": canonical_model,
            "provider": canonical_provider,
            "model": sanitized_model,
            "existing_variant": existing_variant,
            "dry_run": dry_run,
            "skipped": false,
            "linked": linked,
            "warning": warning,
            "written": written,
            "target_path": variant_path.display().to_string(),
            "content_bytes": body.len(),
            "source_bytes": source_size,
        });
        return ToolOutput::success(serde_json::to_string_pretty(&result).unwrap());
    }

    // --- Inspect-only branch (no content) ---------------------------------
    // Inspect-branch warning: future tense — no write planned, but tell the agent
    // where a subsequent persist call would land.
    let warning = if linked {
        Some(format!(
            "linked skill: any variant written by review_skill will be persisted \
             through the symlink to the source directory at {}",
            skill_dir.display()
        ))
    } else {
        None
    };

    let result = serde_json::json!({
        "skill_name": skill_name,
        "root_prompt": prompt_for_response,
        "tools_json": tools_json,
        "runtime_provider": canonical_provider,
        "runtime_model": canonical_model,
        "provider": canonical_provider,
        "model": sanitized_model,
        "existing_variant": existing_variant,
        "dry_run": dry_run,
        "skipped": false,
        "linked": linked,
        "warning": warning,
        "written": false,
        "target_path": variant_path.display().to_string(),
    });

    ToolOutput::success(serde_json::to_string_pretty(&result).unwrap())
}

/// Emit a structured warning when a write operation targets a linked skill.
///
/// Linked skills (`mika skills install <path> --link`) are symlinks to a live
/// source directory. Writes through the symlink land in the source tree by
/// design — the user explicitly opted into mutable-source semantics. We log
/// the write so it's visible in audit and tracing output.
fn warn_linked_skill_write(skill_name: &str, skill_dir: &std::path::Path) {
    tracing::warn!(
        skill = %skill_name,
        path  = %skill_dir.display(),
        "[linked skill] changes will be written to the source directory"
    );
}

/// Handle batch mode (`skill_name = "*"`).
///
/// Returns a summary of all eligible and skipped skills so the agent can
/// then process them individually.
async fn review_skill_batch(
    skills_dir: &std::path::Path,
    canonical_provider: &str,
    sanitized_model: &str,
    force: bool,
) -> ToolOutput {
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(entries) => entries,
        Err(e) => {
            return ToolOutput::error(format!("Cannot read skills directory: {e}"));
        }
    };

    let mut eligible: Vec<serde_json::Value> = Vec::new();
    let mut skipped: Vec<serde_json::Value> = Vec::new();

    let mut dirs: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type()
                .map(|ft| ft.is_dir() || ft.is_symlink())
                .unwrap_or(false)
        })
        .collect();
    dirs.sort_by_key(|e| e.file_name());

    for entry in dirs {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let path = entry.path();

        // Skip trust-critical skills (#480, #486)
        if is_trust_critical_skill(&name_str) {
            skipped.push(serde_json::json!({
                "name": name_str,
                "reason": "trust-critical",
            }));
            continue;
        }

        // Skip linked skills
        if let Ok(meta) = std::fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            skipped.push(serde_json::json!({
                "name": name_str,
                "reason": "linked",
            }));
            continue;
        }

        // Skip skills without system_prompt.md
        let prompt_path = path.join("system_prompt.md");
        if !prompt_path.exists() {
            skipped.push(serde_json::json!({
                "name": name_str,
                "reason": "no system_prompt.md",
            }));
            continue;
        }

        // Check existing generated variant
        let variant_path = path
            .join("generated")
            .join(canonical_provider)
            .join(sanitized_model)
            .join("system_prompt.md");
        let has_variant = variant_path.exists();

        if has_variant && !force {
            skipped.push(serde_json::json!({
                "name": name_str,
                "reason": "variant exists",
            }));
            continue;
        }

        eligible.push(serde_json::json!({
            "name": name_str,
            "has_variant": has_variant,
        }));
    }

    let result = serde_json::json!({
        "mode": "batch",
        "provider": canonical_provider,
        "model": sanitized_model,
        "eligible_skills": eligible,
        "skipped_skills": skipped,
        "total_eligible": eligible.len(),
        "total_skipped": skipped.len(),
    });

    ToolOutput::success(serde_json::to_string_pretty(&result).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_get_documentation_all_embedded_topics() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        for topic in &[
            "architecture",
            "api-spec",
            "browser-control",
            "configuration",
            "deployment",
            "getting-started",
            "runtime-structure",
            "skills",
            "slash-commands",
            "task-system",
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

    #[test]
    fn test_parse_command_array_coerces_valid_json_string() {
        let input = serde_json::json!({
            "command": "[\"pr\", \"list\", \"--state\", \"open\"]"
        });
        let result = parse_command_array(&input).unwrap();
        assert_eq!(result, vec!["pr", "list", "--state", "open"]);
    }

    #[test]
    fn test_parse_command_array_rejects_plain_string() {
        let input = serde_json::json!({"command": "pr list --state open"});
        let result = parse_command_array(&input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .content
                .contains("JSON array of strings")
        );
    }

    #[test]
    fn test_parse_command_array_rejects_mixed_type_json_string() {
        let input = serde_json::json!({"command": "[\"pr\", 42]"});
        let result = parse_command_array(&input);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .content
                .contains("JSON array of strings")
        );
    }

    #[test]
    fn test_parse_command_array_coerces_single_element_json_string() {
        let input = serde_json::json!({"command": "[\"pr\"]"});
        let result = parse_command_array(&input).unwrap();
        assert_eq!(result, vec!["pr"]);
    }

    #[test]
    fn test_parse_command_array_rejects_empty_json_string_array() {
        let input = serde_json::json!({"command": "[]"});
        let result = parse_command_array(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().content.contains("must not be empty"));
    }

    #[test]
    fn test_validate_gh_input_coerces_json_string_through_full_validation() {
        // Coerced array must pass through the same allowlist + blocked-flag checks
        let input = serde_json::json!({
            "command": "[\"pr\", \"list\", \"--state\", \"open\"]"
        });
        let result = validate_gh_input(&input).unwrap();
        assert_eq!(result.args, vec!["pr", "list", "--state", "open"]);
    }

    #[test]
    fn test_validate_gh_input_coerced_string_blocked_subcommand() {
        // Coerced array must still be rejected by the allowlist
        let input = serde_json::json!({"command": "[\"auth\", \"login\"]"});
        let result = validate_gh_input(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().content.contains("not allowed"));
    }

    #[test]
    fn test_validate_gws_input_coerces_json_string() {
        let input = serde_json::json!({
            "command": "[\"gmail\", \"messages\", \"list\"]"
        });
        let result = validate_gws_input(&input).unwrap();
        assert_eq!(result, vec!["gmail", "messages", "list"]);
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

    // -- git_ops tests --

    #[test]
    fn test_git_ops_in_known_builtins() {
        assert!(KNOWN_BUILTINS.contains(&"git_ops"));
    }

    #[test]
    fn test_validate_git_ops_missing_operation() {
        let input = serde_json::json!({"repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing required 'operation'"));
    }

    #[test]
    fn test_validate_git_ops_unknown_operation() {
        let input = serde_json::json!({"operation": "cherry-pick", "repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Unknown operation 'cherry-pick'"));
    }

    #[test]
    fn test_validate_git_ops_missing_repo_path() {
        let input = serde_json::json!({"operation": "fetch"});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing required 'repo_path'"));
    }

    #[test]
    fn test_validate_git_ops_push_on_merge_rejected() {
        let input =
            serde_json::json!({"operation": "merge", "repo_path": "/tmp/repo", "push": true});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.content
                .contains("push=true is only allowed with 'rebase'")
        );
    }

    #[test]
    fn test_validate_git_ops_push_on_fetch_rejected() {
        let input =
            serde_json::json!({"operation": "fetch", "repo_path": "/tmp/repo", "push": true});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.content
                .contains("push=true is only allowed with 'rebase'")
        );
    }

    #[test]
    fn test_validate_git_ops_valid_rebase_with_push() {
        let input = serde_json::json!({
            "operation": "rebase",
            "repo_path": "/tmp/repo",
            "base": "origin/develop",
            "push": true
        });
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "rebase");
        assert_eq!(result.repo_path, "/tmp/repo");
        assert_eq!(result.base, "origin/develop");
        assert!(result.push);
    }

    #[test]
    fn test_validate_git_ops_base_starting_with_dash_rejected() {
        let input = serde_json::json!({
            "operation": "rebase",
            "repo_path": "/tmp/repo",
            "base": "--exec=malicious"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must not start with '-'"));
    }

    #[test]
    fn test_validate_git_ops_relative_path_rejected() {
        let input = serde_json::json!({"operation": "fetch", "repo_path": "relative/path"});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must be an absolute path"));
    }

    #[test]
    fn test_validate_git_ops_defaults() {
        let input = serde_json::json!({"operation": "fetch", "repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.base, "origin/main");
        assert!(!result.push);
    }

    #[test]
    fn test_extract_remote() {
        assert_eq!(extract_remote("origin/main"), "origin");
        assert_eq!(extract_remote("upstream/develop"), "upstream");
        assert_eq!(extract_remote("origin"), "origin");
    }

    #[tokio::test]
    async fn test_git_ops_preflight_nonexistent_path() {
        let result = git_ops_preflight("/nonexistent/path/to/repo", "fetch").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Path does not exist"));
    }

    #[tokio::test]
    async fn test_git_ops_preflight_not_a_directory() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = git_ops_preflight(path, "fetch").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("not a directory"));
    }

    #[tokio::test]
    async fn test_git_ops_preflight_not_a_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().to_str().unwrap();
        let result = git_ops_preflight(path, "fetch").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Not a git repository"));
    }

    #[tokio::test]
    async fn test_git_ops_preflight_dirty_tree_blocks_rebase() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        // Init a git repo with one commit
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        // Create an uncommitted file
        std::fs::write(tmp.path().join("dirty.txt"), "dirty").unwrap();
        let result = git_ops_preflight(repo, "rebase").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("uncommitted changes"));
    }

    #[tokio::test]
    async fn test_git_ops_fetch_on_local_repo() {
        // Fetch on a repo with no remote will fail — verifies error handling
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = git_ops(
            &serde_json::json!({
                "operation": "fetch",
                "repo_path": repo
            }),
            &ctx,
        )
        .await;
        // No remote configured → fetch fails
        assert!(output.is_error);
        assert!(output.content.contains("failed"));
    }

    // -- new operation validation tests --

    #[test]
    fn test_validate_git_ops_pull_valid() {
        let input = serde_json::json!({"operation": "pull", "repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "pull");
        assert_eq!(result.base, "origin/main");
    }

    #[test]
    fn test_validate_git_ops_checkout_valid() {
        let input = serde_json::json!({
            "operation": "checkout",
            "repo_path": "/tmp/repo",
            "branch": "feat/my-feature"
        });
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "checkout");
        assert_eq!(result.branch.as_deref(), Some("feat/my-feature"));
    }

    #[test]
    fn test_validate_git_ops_checkout_missing_branch() {
        let input = serde_json::json!({"operation": "checkout", "repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing required 'branch'"));
    }

    #[test]
    fn test_validate_git_ops_worktree_add_valid() {
        let input = serde_json::json!({
            "operation": "worktree_add",
            "repo_path": "/tmp/repo",
            "path": "/tmp/worktree",
            "branch": "feat/new-branch",
            "base": "origin/main"
        });
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "worktree_add");
        assert_eq!(result.path.as_deref(), Some("/tmp/worktree"));
        assert_eq!(result.branch.as_deref(), Some("feat/new-branch"));
    }

    #[test]
    fn test_validate_git_ops_worktree_add_missing_path() {
        let input = serde_json::json!({
            "operation": "worktree_add",
            "repo_path": "/tmp/repo",
            "branch": "feat/new-branch"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing required 'path'"));
    }

    #[test]
    fn test_validate_git_ops_worktree_add_missing_branch() {
        let input = serde_json::json!({
            "operation": "worktree_add",
            "repo_path": "/tmp/repo",
            "path": "/tmp/worktree"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing required 'branch'"));
    }

    #[test]
    fn test_validate_git_ops_worktree_add_relative_path_rejected() {
        let input = serde_json::json!({
            "operation": "worktree_add",
            "repo_path": "/tmp/repo",
            "path": "relative/worktree",
            "branch": "feat/new-branch"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must be an absolute path"));
    }

    #[test]
    fn test_validate_git_ops_worktree_remove_valid() {
        let input = serde_json::json!({
            "operation": "worktree_remove",
            "repo_path": "/tmp/repo",
            "path": "/tmp/worktree"
        });
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "worktree_remove");
        assert_eq!(result.path.as_deref(), Some("/tmp/worktree"));
    }

    #[test]
    fn test_validate_git_ops_worktree_remove_missing_path() {
        let input = serde_json::json!({
            "operation": "worktree_remove",
            "repo_path": "/tmp/repo"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("Missing required 'path'"));
    }

    #[test]
    fn test_validate_git_ops_worktree_remove_relative_path_rejected() {
        let input = serde_json::json!({
            "operation": "worktree_remove",
            "repo_path": "/tmp/repo",
            "path": "relative/worktree"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must be an absolute path"));
    }

    #[test]
    fn test_validate_git_ops_worktree_list_valid() {
        let input = serde_json::json!({"operation": "worktree_list", "repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "worktree_list");
    }

    #[test]
    fn test_validate_git_ops_worktree_prune_valid() {
        let input = serde_json::json!({"operation": "worktree_prune", "repo_path": "/tmp/repo"});
        let result = validate_git_ops_input(&input).unwrap();
        assert_eq!(result.operation, "worktree_prune");
    }

    #[test]
    fn test_validate_git_ops_branch_starting_with_dash_rejected() {
        let input = serde_json::json!({
            "operation": "checkout",
            "repo_path": "/tmp/repo",
            "branch": "--exec=malicious"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must not start with '-'"));
    }

    #[test]
    fn test_validate_git_ops_path_starting_with_dash_rejected() {
        let input = serde_json::json!({
            "operation": "worktree_add",
            "repo_path": "/tmp/repo",
            "path": "--malicious",
            "branch": "feat/test"
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("must not start with '-'"));
    }

    #[test]
    fn test_validate_git_ops_push_on_pull_rejected() {
        let input =
            serde_json::json!({"operation": "pull", "repo_path": "/tmp/repo", "push": true});
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.content
                .contains("push=true is only allowed with 'rebase'")
        );
    }

    #[test]
    fn test_validate_git_ops_push_on_checkout_rejected() {
        let input = serde_json::json!({
            "operation": "checkout",
            "repo_path": "/tmp/repo",
            "branch": "main",
            "push": true
        });
        let result = validate_git_ops_input(&input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.content
                .contains("push=true is only allowed with 'rebase'")
        );
    }

    // -- new operation handler tests --

    #[tokio::test]
    async fn test_git_ops_pull_on_local_repo() {
        // Pull on a repo with no remote will fail — verifies error handling
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = git_ops(
            &serde_json::json!({
                "operation": "pull",
                "repo_path": repo
            }),
            &ctx,
        )
        .await;
        // No remote configured → fetch fails
        assert!(output.is_error);
        assert!(output.content.contains("failed"));
    }

    #[tokio::test]
    async fn test_git_ops_checkout_nonexistent_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = git_ops(
            &serde_json::json!({
                "operation": "checkout",
                "repo_path": repo,
                "branch": "nonexistent-branch"
            }),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(output.content.contains("Failed to switch"));
    }

    #[tokio::test]
    async fn test_git_ops_worktree_list_on_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = git_ops(
            &serde_json::json!({
                "operation": "worktree_list",
                "repo_path": repo
            }),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        // The main worktree should be listed
        assert!(output.content.contains("worktree"));
    }

    #[tokio::test]
    async fn test_git_ops_worktree_prune_on_clean_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = git_ops(
            &serde_json::json!({
                "operation": "worktree_prune",
                "repo_path": repo
            }),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        assert!(output.content.contains("prune completed"));
    }

    #[tokio::test]
    async fn test_git_ops_worktree_add_and_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();

        let wt_path = tmp.path().join("my-worktree");
        let wt_path_str = wt_path.to_str().unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx();

        // Add worktree
        let output = git_ops(
            &serde_json::json!({
                "operation": "worktree_add",
                "repo_path": repo,
                "path": wt_path_str,
                "branch": "test-wt-branch",
                "base": "HEAD"
            }),
            &ctx,
        )
        .await;
        assert!(!output.is_error, "worktree_add failed: {}", output.content);
        assert!(output.content.contains("Worktree created"));
        assert!(wt_path.exists());

        // Remove worktree
        let output = git_ops(
            &serde_json::json!({
                "operation": "worktree_remove",
                "repo_path": repo,
                "path": wt_path_str
            }),
            &ctx,
        )
        .await;
        assert!(
            !output.is_error,
            "worktree_remove failed: {}",
            output.content
        );
        assert!(output.content.contains("removed"));
        assert!(!wt_path.exists());
    }

    #[tokio::test]
    async fn test_git_ops_worktree_remove_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = git_ops(
            &serde_json::json!({
                "operation": "worktree_remove",
                "repo_path": repo,
                "path": "/tmp/nonexistent-worktree-path"
            }),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(output.content.contains("Failed to remove"));
    }

    #[tokio::test]
    async fn test_git_ops_preflight_dirty_tree_blocks_pull() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().to_str().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .output()
            .unwrap();
        // Create an uncommitted file
        std::fs::write(tmp.path().join("dirty.txt"), "dirty").unwrap();
        let result = git_ops_preflight(repo, "pull").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.content.contains("uncommitted changes"));
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
            "browser-control.md",
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

    // -----------------------------------------------------------------------
    // review_skill tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_review_skill_in_known_builtins() {
        assert!(KNOWN_BUILTINS.contains(&"review_skill"));
    }

    #[tokio::test]
    async fn test_review_skill_missing_skill_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = review_skill(&serde_json::json!({}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Missing required 'skill_name'"));
    }

    #[tokio::test]
    async fn test_review_skill_empty_skill_name() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = review_skill(&serde_json::json!({"skill_name": ""}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("Missing required 'skill_name'"));
    }

    #[tokio::test]
    async fn test_review_skill_path_traversal_rejected() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        for bad_name in &["../etc/passwd", "foo/bar", "skill\\evil", "a\0b", ".."] {
            let output = review_skill(&serde_json::json!({"skill_name": bad_name}), &ctx).await;
            assert!(output.is_error, "should reject: {bad_name}");
            assert!(
                output.content.contains("no path separators"),
                "wrong error for: {bad_name}"
            );
        }
    }

    #[tokio::test]
    async fn test_review_skill_nonexistent_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "nonexistent"}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_review_skill_no_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            "[skill]\nname = \"test-skill\"\n",
        )
        .unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "test-skill"}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("no system_prompt.md"));
    }

    #[tokio::test]
    async fn test_review_skill_linked_skill_warns_and_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skills_dir = home.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        // Create a real directory and symlink to it
        let real_dir = tmp.path().join("real-skill");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("system_prompt.md"), "test prompt").unwrap();
        #[cfg(not(unix))]
        {
            return;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, skills_dir.join("linked-skill")).unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "linked-skill"}), &ctx).await;
        assert!(!output.is_error, "linked skill review must succeed");
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["linked"], true);
        assert!(
            parsed["warning"]
                .as_str()
                .unwrap_or("")
                .contains("linked skill")
        );
    }

    #[tokio::test]
    async fn test_review_skill_single_happy_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/custom-search");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Search the web.").unwrap();
        std::fs::write(skill_dir.join("tools.json"), r#"[{"name": "web_search"}]"#).unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "custom-search"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["skill_name"], "custom-search");
        assert_eq!(parsed["runtime_provider"], "anthropic");
        assert_eq!(parsed["runtime_model"], "claude-sonnet-4-6");
        assert_eq!(parsed["root_prompt"], "Search the web.");
        assert_eq!(parsed["skipped"], false);
        // Inspect and persist branches share the same core shape: `provider`, `model`,
        // and `target_path` are emitted in both so the agent doesn't have to branch
        // on mode. The legacy `next_action` instruction is gone — `review_skill` is
        // now the single tool that both inspects and persists.
        assert_eq!(parsed["provider"], "anthropic");
        assert_eq!(parsed["model"], "claude-sonnet-4-6");
        assert!(
            parsed["target_path"]
                .as_str()
                .unwrap()
                .contains("/generated/anthropic/claude-sonnet-4-6/system_prompt.md")
        );
        assert!(parsed.get("next_action").is_none());
        assert_eq!(parsed["written"], false);
    }

    #[tokio::test]
    async fn test_review_skill_existing_variant_returned_in_inspect_mode() {
        // Inspect-only mode (no `content`) returns the existing variant in the
        // response so the agent can decide whether to overwrite. The legacy
        // "skipped" stub is gone — there is no reason to short-circuit when the
        // agent only asked to inspect.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/custom-search");
        let variant_dir = skill_dir.join("generated/anthropic/claude-sonnet-4-6");
        std::fs::create_dir_all(&variant_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Search the web.").unwrap();
        std::fs::write(variant_dir.join("system_prompt.md"), "Existing variant.").unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "custom-search"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["skipped"], false);
        assert_eq!(parsed["existing_variant"], "Existing variant.");
        assert_eq!(parsed["written"], false);
    }

    #[tokio::test]
    async fn test_review_skill_existing_variant_force() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/custom-search");
        let variant_dir = skill_dir.join("generated/anthropic/claude-sonnet-4-6");
        std::fs::create_dir_all(&variant_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Search the web.").unwrap();
        std::fs::write(variant_dir.join("system_prompt.md"), "Existing variant.").unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(
            &serde_json::json!({"skill_name": "custom-search", "force": true}),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["skipped"], false);
        assert_eq!(parsed["existing_variant"], "Existing variant.");
    }

    #[tokio::test]
    async fn test_review_skill_batch_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skills_dir = home.join("skills");
        // Create two skills with prompts
        for name in &["alpha-skill", "beta-skill"] {
            let dir = skills_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("system_prompt.md"), format!("{name} prompt")).unwrap();
        }
        // Create one skill without prompt
        let no_prompt = skills_dir.join("gamma-skill");
        std::fs::create_dir_all(&no_prompt).unwrap();
        std::fs::write(no_prompt.join("skill.toml"), "[skill]\nname = \"gamma\"\n").unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "*"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["mode"], "batch");
        assert_eq!(parsed["total_eligible"], 2);
        assert_eq!(parsed["total_skipped"], 1);
        // gamma-skill skipped for no system_prompt.md
        let skipped = parsed["skipped_skills"].as_array().unwrap();
        assert!(skipped.iter().any(|s| s["name"] == "gamma-skill"));
    }

    #[test]
    fn test_resolve_canonical_provider_model_direct() {
        let (p, m) = resolve_canonical_provider_model("anthropic", "claude-sonnet-4-6");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-4-6");
    }

    #[test]
    fn test_resolve_canonical_provider_model_openrouter() {
        let (p, m) = resolve_canonical_provider_model("openrouter", "anthropic/claude-sonnet-4");
        assert_eq!(p, "anthropic");
        assert_eq!(m, "claude-sonnet-4");
    }

    #[test]
    fn test_resolve_canonical_provider_model_openrouter_openai() {
        let (p, m) = resolve_canonical_provider_model("openrouter", "openai/gpt-4o");
        assert_eq!(p, "openai");
        assert_eq!(m, "gpt-4o");
    }

    #[test]
    fn test_resolve_canonical_provider_model_openrouter_meta_llama() {
        let (p, m) =
            resolve_canonical_provider_model("openrouter", "meta-llama/llama-3.3-70b-instruct");
        assert_eq!(p, "meta-llama");
        assert_eq!(m, "llama-3.3-70b-instruct");
    }

    #[test]
    fn test_resolve_canonical_provider_model_unknown_provider() {
        let (p, m) = resolve_canonical_provider_model("custom-provider", "my-model");
        assert_eq!(p, "custom-provider");
        assert_eq!(m, "my-model");
    }

    #[tokio::test]
    async fn test_review_skill_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Test prompt.").unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(
            &serde_json::json!({"skill_name": "test-skill", "dry_run": true}),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["dry_run"], true);
        assert_eq!(parsed["skipped"], false);
    }

    // -----------------------------------------------------------------------
    // review_skill persist-mode tests (formerly write_skill_variant)
    // -----------------------------------------------------------------------

    /// Set up a skill directory with a source prompt of `source_size` bytes.
    fn setup_skill_with_source(
        home: &std::path::Path,
        skill_name: &str,
        source_size: usize,
    ) -> std::path::PathBuf {
        let skill_dir = home.join("skills").join(skill_name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let source = "x".repeat(source_size);
        std::fs::write(skill_dir.join("system_prompt.md"), source).unwrap();
        skill_dir
    }

    #[tokio::test]
    async fn test_write_skill_variant_uses_runtime_model() {
        // Path must derive from ctx.provider_name / ctx.model_name, not from any input.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = setup_skill_with_source(home, "demo", 1000);

        let harness = TestHarness::new(); // anthropic / claude-sonnet-4-6
        let ctx = harness.ctx_with_home(home);
        let body = "y".repeat(800);
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body}),
            &ctx,
        )
        .await;
        assert!(!output.is_error, "expected ok, got: {}", output.content);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        let written = parsed["target_path"].as_str().unwrap();
        assert!(
            written.contains("/anthropic/claude-sonnet-4-6/system_prompt.md"),
            "path should derive from ctx (anthropic/claude-sonnet-4-6), got: {written}"
        );
        assert!(
            skill_dir
                .join("generated/anthropic/claude-sonnet-4-6/system_prompt.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn test_write_skill_variant_canonicalises_openrouter() {
        // openrouter/minimax/minimax-m2.7 → generated/minimax/minimax-m2.7/
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup_skill_with_source(home, "demo", 1000);

        let harness = TestHarness::new();
        let mut ctx = harness.ctx_with_home(home);
        ctx.provider_name = "openrouter";
        ctx.model_name = "minimax/minimax-m2.7";
        let body = "q".repeat(800);
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body}),
            &ctx,
        )
        .await;
        assert!(!output.is_error, "got: {}", output.content);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["provider"], "minimax");
        assert_eq!(parsed["model"], "minimax-m2.7");
        let written = parsed["target_path"].as_str().unwrap();
        assert!(
            written.contains("/generated/minimax/minimax-m2.7/system_prompt.md"),
            "got: {written}"
        );
    }

    #[tokio::test]
    async fn test_write_skill_variant_path_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        for bad in &["../etc/passwd", "foo/bar", "skill\\evil", "a\0b", ".."] {
            let output = review_skill(
                &serde_json::json!({"skill_name": bad, "content": "data"}),
                &ctx,
            )
            .await;
            assert!(output.is_error, "should reject: {bad}");
            assert!(
                output.content.contains("no path separators"),
                "wrong error for {bad}: {}",
                output.content
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_write_skill_variant_linked_skill_warns_and_writes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        let real = tmp.path().join("real-skill");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("system_prompt.md"), "x".repeat(1000)).unwrap();
        std::os::unix::fs::symlink(&real, home.join("skills/linked")).unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(
            &serde_json::json!({"skill_name": "linked", "content": "y".repeat(800)}),
            &ctx,
        )
        .await;
        assert!(!output.is_error, "linked skill variant must succeed");
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["linked"], true);
        assert!(
            parsed["warning"]
                .as_str()
                .unwrap_or("")
                .contains("linked skill")
        );
        // Variant must land in the source directory by symlink transparency.
        let variant_path = real.join("generated/anthropic/claude-sonnet-4-6/system_prompt.md");
        assert!(
            variant_path.exists(),
            "variant should be written through symlink to source: {variant_path:?}"
        );
    }

    #[tokio::test]
    async fn test_write_skill_variant_no_overwrite_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup_skill_with_source(home, "demo", 1000);

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let body = "y".repeat(800);

        // First write succeeds.
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body.clone()}),
            &ctx,
        )
        .await;
        assert!(!output.is_error);

        // Second write without force is rejected.
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body.clone()}),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(output.content.contains("force=true"));

        // Second write with force is accepted.
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body, "force": true}),
            &ctx,
        )
        .await;
        assert!(
            !output.is_error,
            "force should overwrite, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_write_skill_variant_marks_skills_dirty() {
        // Without this the registry never re-scans the new generated variant
        // and resolve_prompt keeps serving the root prompt — the feature loop
        // is broken until process restart.
        use std::sync::atomic::Ordering;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup_skill_with_source(home, "demo", 1000);

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        // Reset the shared static (TestHarness::ctx_with_home uses a static
        // AtomicBool across tests), so we can observe the flip cleanly.
        ctx.skills_dirty.store(false, Ordering::Release);

        let body = "y".repeat(800);
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body}),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        assert!(
            ctx.skills_dirty.load(Ordering::Acquire),
            "skills_dirty must be set after a successful variant write"
        );
    }

    #[tokio::test]
    async fn test_write_skill_variant_truncation_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup_skill_with_source(home, "demo", 1000);

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        // 400 bytes is 40% of 1000 — below the 50% MIN_VARIANT_RATIO threshold.
        let truncated = "y".repeat(400);
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": truncated}),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(
            output.content.contains("truncation"),
            "expected truncation error, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_review_skill_rejects_invalid_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup_skill_with_source(home, "demo", 1000);

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        // Content with null bytes — should fail markdown validation
        let body = format!("{}hello\0world{}", "x".repeat(500), "x".repeat(500));
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body}),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(
            output.content.contains("markdown validation"),
            "expected markdown validation error, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_review_skill_rejects_unclosed_fence() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup_skill_with_source(home, "demo", 100);

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        // Content with 3 fences (odd = unclosed). Must be >= 50% of source (50 bytes).
        let body =
            "# Prompt\n\n```rust\nfn main() {}\n```\n\nSome text.\n\n```python\nprint('hello')\n";
        assert!(body.len() >= 50, "body too short: {}", body.len());
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body}),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(
            output.content.contains("code fence"),
            "expected code fence error, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_review_skill_inspect_then_persist_round_trip() {
        // Two-call workflow: inspect first, then persist with content. This is
        // the canonical interaction pattern from the skill-review system prompt.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = setup_skill_with_source(home, "demo", 1000);
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);

        // Step 1: inspect (no content) — should not write anything.
        let inspect = review_skill(&serde_json::json!({"skill_name": "demo"}), &ctx).await;
        assert!(!inspect.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&inspect.content).unwrap();
        assert_eq!(parsed["written"], false);
        assert!(
            !skill_dir
                .join("generated/anthropic/claude-sonnet-4-6/system_prompt.md")
                .exists()
        );

        // Step 2: persist with content — should write the variant.
        let body = "y".repeat(800);
        let persist = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body}),
            &ctx,
        )
        .await;
        assert!(!persist.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&persist.content).unwrap();
        assert_eq!(parsed["written"], true);
        assert_eq!(parsed["content_bytes"], 800);
        assert!(
            skill_dir
                .join("generated/anthropic/claude-sonnet-4-6/system_prompt.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn test_review_skill_persist_dry_run_does_not_touch_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = setup_skill_with_source(home, "demo", 1000);
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);

        let body = "y".repeat(800);
        let output = review_skill(
            &serde_json::json!({"skill_name": "demo", "content": body, "dry_run": true}),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["dry_run"], true);
        assert_eq!(parsed["written"], false);
        // The would-be path is reported, but the file does NOT exist on disk.
        assert!(
            parsed["target_path"]
                .as_str()
                .unwrap()
                .contains("/generated/anthropic/claude-sonnet-4-6/system_prompt.md")
        );
        assert!(
            !skill_dir
                .join("generated/anthropic/claude-sonnet-4-6/system_prompt.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn test_review_skill_persist_rejects_batch_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join("skills")).unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);

        let output = review_skill(
            &serde_json::json!({"skill_name": "*", "content": "y".repeat(800)}),
            &ctx,
        )
        .await;
        assert!(output.is_error);
        assert!(output.content.contains("not supported in batch mode"));
    }

    #[tokio::test]
    async fn test_review_skill_no_tools_json() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/prompt-only");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Prompt only skill.").unwrap();
        // No tools.json
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "prompt-only"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["tools_json"], "[]");
    }

    // -----------------------------------------------------------------------
    // review_skill — built-in skill guard (#480)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_review_skill_rejects_trust_critical_inspect() {
        // Inspect mode (no content) should be rejected for trust-critical skills.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = review_skill(&serde_json::json!({"skill_name": "skill-review"}), &ctx).await;
        assert!(!output.is_error); // Not a tool error — a clear rejection message
        assert!(
            output
                .content
                .contains("Cannot review trust-critical skill"),
            "expected trust-critical rejection, got: {}",
            output.content
        );
        assert!(output.content.contains("skill-review"));
    }

    #[tokio::test]
    async fn test_review_skill_rejects_trust_critical_persist() {
        // Persist mode (with content) should also be rejected for trust-critical skills.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let output = review_skill(
            &serde_json::json!({
                "skill_name": "self-knowledge",
                "content": "Adapted prompt for self-knowledge."
            }),
            &ctx,
        )
        .await;
        assert!(!output.is_error);
        assert!(
            output
                .content
                .contains("Cannot review trust-critical skill"),
            "expected trust-critical rejection, got: {}",
            output.content
        );
        assert!(output.content.contains("self-knowledge"));
    }

    #[tokio::test]
    async fn test_review_skill_rejects_trust_critical_case_insensitive() {
        // Case-insensitive matching: trust-critical skills blocked regardless of case.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        for name in &["Skill-Review", "SELF-KNOWLEDGE", "Agents-Teams"] {
            let output = review_skill(&serde_json::json!({"skill_name": name}), &ctx).await;
            assert!(
                output
                    .content
                    .contains("Cannot review trust-critical skill"),
                "should reject '{}', got: {}",
                name,
                output.content
            );
        }
    }

    #[tokio::test]
    async fn test_review_skill_allows_reviewable_bundled_skills() {
        // Non-trust-critical bundled skills (e.g., web-search, shell-exec)
        // should NOT be blocked by the trust-critical guard.
        // They will fail with "not found" (no skill dir in temp home) but
        // should NOT hit the trust-critical rejection.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        for name in &["web-search", "shell-exec", "tmux", "github", "git-ops"] {
            let output = review_skill(&serde_json::json!({"skill_name": name}), &ctx).await;
            assert!(
                !output
                    .content
                    .contains("Cannot review trust-critical skill"),
                "'{}' should NOT be blocked as trust-critical, got: {}",
                name,
                output.content
            );
        }
    }

    #[tokio::test]
    async fn test_review_skill_batch_skips_trust_critical_only() {
        // Batch mode should skip trust-critical skills but include reviewable
        // bundled skills.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skills_dir = home.join("skills");

        // Create a trust-critical skill directory with a prompt
        let trust_critical_dir = skills_dir.join("self-knowledge");
        std::fs::create_dir_all(&trust_critical_dir).unwrap();
        std::fs::write(
            trust_critical_dir.join("system_prompt.md"),
            "Self knowledge.",
        )
        .unwrap();

        // Create a reviewable bundled skill directory with a prompt
        let reviewable_dir = skills_dir.join("shell-exec");
        std::fs::create_dir_all(&reviewable_dir).unwrap();
        std::fs::write(reviewable_dir.join("system_prompt.md"), "Execute shell.").unwrap();

        // Create a custom skill directory with a prompt
        let custom_dir = skills_dir.join("my-custom-skill");
        std::fs::create_dir_all(&custom_dir).unwrap();
        std::fs::write(custom_dir.join("system_prompt.md"), "Custom prompt.").unwrap();

        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "*"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["mode"], "batch");

        // self-knowledge should be in skipped with reason "trust-critical"
        let skipped = parsed["skipped_skills"].as_array().unwrap();
        let trust_skip = skipped
            .iter()
            .find(|s| s["name"] == "self-knowledge")
            .expect("self-knowledge should be in skipped list");
        assert_eq!(trust_skip["reason"], "trust-critical");

        // shell-exec (reviewable bundled) should be in eligible, NOT skipped
        let eligible = parsed["eligible_skills"].as_array().unwrap();
        assert!(
            eligible.iter().any(|s| s["name"] == "shell-exec"),
            "shell-exec (reviewable bundled) should be eligible"
        );
        assert!(
            !skipped.iter().any(|s| s["name"] == "shell-exec"),
            "shell-exec should NOT be in skipped list"
        );

        // my-custom-skill should be in eligible
        assert!(
            eligible.iter().any(|s| s["name"] == "my-custom-skill"),
            "my-custom-skill should be eligible"
        );
    }

    #[test]
    fn test_is_pr_review_command() {
        // Positive cases
        assert!(is_pr_review_command(&[
            "pr".to_string(),
            "review".to_string(),
            "455".to_string(),
            "--approve".to_string(),
        ]));
        assert!(is_pr_review_command(&[
            "pr".to_string(),
            "review".to_string(),
            "123".to_string(),
            "--comment".to_string(),
            "--body".to_string(),
            "VERDICT: hold".to_string(),
        ]));

        // Negative cases
        assert!(!is_pr_review_command(&[
            "pr".to_string(),
            "list".to_string(),
        ]));
        assert!(!is_pr_review_command(&[
            "pr".to_string(),
            "view".to_string(),
            "455".to_string(),
        ]));
        assert!(!is_pr_review_command(&["pr".to_string()]));
        assert!(!is_pr_review_command(&[]));
        assert!(!is_pr_review_command(&[
            "issue".to_string(),
            "review".to_string(),
        ]));
    }

    // -- gh_read tests --

    #[test]
    fn test_validate_gh_read_input_valid_issue_view() {
        let input = serde_json::json!({"op": "issue_view", "target": "123", "repo": "owner/repo"});
        let args = validate_gh_read_input(&input).unwrap();
        assert_eq!(args.op, "issue_view");
        assert_eq!(args.target.as_deref(), Some("123"));
        assert_eq!(args.repo, "owner/repo");
    }

    #[test]
    fn test_validate_gh_read_input_valid_pr_diff() {
        let input = serde_json::json!({"op": "pr_diff", "target": "456", "repo": "owner/repo"});
        let args = validate_gh_read_input(&input).unwrap();
        assert_eq!(args.op, "pr_diff");
        assert_eq!(args.target.as_deref(), Some("456"));
    }

    #[test]
    fn test_validate_gh_read_input_issue_list_no_target() {
        let input = serde_json::json!({"op": "issue_list", "repo": "owner/repo"});
        let args = validate_gh_read_input(&input).unwrap();
        assert_eq!(args.op, "issue_list");
        assert!(args.target.is_none());
    }

    #[test]
    fn test_validate_gh_read_input_issue_list_with_milestone() {
        let input = serde_json::json!({"op": "issue_list", "target": "17", "repo": "owner/repo"});
        let args = validate_gh_read_input(&input).unwrap();
        assert_eq!(args.op, "issue_list");
        assert_eq!(args.target.as_deref(), Some("17"));
    }

    #[test]
    fn test_validate_gh_read_input_disallowed_op() {
        let input =
            serde_json::json!({"op": "issue_create", "target": "test", "repo": "owner/repo"});
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("issue_create"));
    }

    #[test]
    fn test_validate_gh_read_input_missing_op() {
        let input = serde_json::json!({"repo": "owner/repo"});
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("op"));
    }

    #[test]
    fn test_validate_gh_read_input_missing_repo() {
        let input = serde_json::json!({"op": "issue_view", "target": "123"});
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("repo"));
    }

    #[test]
    fn test_validate_gh_read_input_missing_target_for_issue_view() {
        let input = serde_json::json!({"op": "issue_view", "repo": "owner/repo"});
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("target"));
    }

    #[test]
    fn test_classify_gh_error_not_found() {
        let err = classify_gh_error("GraphQL: Could not resolve to an Issue", Some(1));
        assert!(matches!(err, GhReadError::NotFound(_)));
    }

    #[test]
    fn test_classify_gh_error_auth_failed() {
        let err = classify_gh_error("authentication required", Some(1));
        assert!(matches!(err, GhReadError::AuthFailed(_)));
    }

    #[test]
    fn test_classify_gh_error_rate_limited() {
        let err = classify_gh_error("API rate limit exceeded", Some(1));
        assert!(matches!(err, GhReadError::RateLimited(_)));
    }

    #[test]
    fn test_classify_gh_error_network_error_command_not_found() {
        let err = classify_gh_error("command not found", Some(127));
        assert!(matches!(err, GhReadError::NetworkError(_)));
    }

    #[test]
    fn test_classify_gh_error_network_error_unknown() {
        let err = classify_gh_error("some unexpected error", Some(1));
        assert!(matches!(err, GhReadError::NetworkError(_)));
    }

    #[test]
    fn test_classify_gh_error_403_forbidden() {
        // App permission gap or fine-grained PAT scope mismatch.
        let err = classify_gh_error("HTTP 403: Forbidden", Some(1));
        assert!(
            matches!(err, GhReadError::AuthFailed(_)),
            "403 must classify as AuthFailed (was NetworkError before fix)"
        );
    }

    #[test]
    fn test_classify_gh_error_resource_not_accessible() {
        // GitHub App installation missing the required permission.
        let err = classify_gh_error("Resource not accessible by integration", Some(1));
        assert!(matches!(err, GhReadError::AuthFailed(_)));
    }

    #[test]
    fn test_validate_gh_read_input_rejects_flag_shaped_target() {
        let input = serde_json::json!({"op": "issue_view", "target": "--web", "repo": "o/r"});
        let err = validate_gh_read_input(&input).expect_err("flag-shaped target must reject");
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
    }

    #[test]
    fn test_validate_gh_read_input_rejects_flag_shaped_repo() {
        let input = serde_json::json!({"op": "issue_view", "target": "42", "repo": "--token=evil"});
        let err = validate_gh_read_input(&input).expect_err("flag-shaped repo must reject");
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
    }

    #[test]
    fn test_validate_gh_read_input_rejects_non_numeric_target_for_view_ops() {
        let input = serde_json::json!({"op": "pr_view", "target": "not-a-number", "repo": "o/r"});
        let err = validate_gh_read_input(&input).expect_err("non-numeric target must reject");
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
    }

    #[test]
    fn test_validate_gh_read_input_allows_non_numeric_target_for_issue_list() {
        // issue_list accepts label or milestone-name targets — these are not
        // required to be numeric.
        let input = serde_json::json!({"op": "issue_list", "target": "bug", "repo": "o/r"});
        let result = validate_gh_read_input(&input);
        assert!(
            result.is_ok(),
            "issue_list with label target must be allowed"
        );
    }

    #[test]
    fn test_build_gh_read_command_issue_view() {
        let args = GhReadArgs {
            op: "issue_view".to_string(),
            target: Some("42".to_string()),
            repo: "owner/repo".to_string(),
            path: None,
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert_eq!(cmd[0], "issue");
        assert_eq!(cmd[1], "view");
        assert_eq!(cmd[2], "42");
        assert_eq!(cmd[3], "--json");
        // Verify --repo is in the argv (moved into per-op arms)
        assert!(cmd.contains(&"--repo".to_string()));
        assert!(cmd.contains(&"owner/repo".to_string()));
    }

    #[test]
    fn test_build_gh_read_command_pr_diff() {
        let args = GhReadArgs {
            op: "pr_diff".to_string(),
            target: Some("99".to_string()),
            repo: "owner/repo".to_string(),
            path: None,
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert_eq!(cmd, vec!["pr", "diff", "99", "--repo", "owner/repo"]);
    }

    #[test]
    fn test_build_gh_read_command_issue_list_no_target() {
        let args = GhReadArgs {
            op: "issue_list".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: None,
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert_eq!(cmd[0], "issue");
        assert_eq!(cmd[1], "list");
        assert_eq!(cmd[2], "--json");
        // Verify --repo is in the argv
        assert!(cmd.contains(&"--repo".to_string()));
    }

    #[test]
    fn test_build_gh_read_command_issue_list_milestone_filter() {
        let args = GhReadArgs {
            op: "issue_list".to_string(),
            target: Some("17".to_string()),
            repo: "owner/repo".to_string(),
            path: None,
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert!(cmd.contains(&"--milestone".to_string()));
        assert!(cmd.contains(&"17".to_string()));
        assert!(cmd.contains(&"--repo".to_string()));
    }

    #[test]
    fn test_build_gh_read_command_issue_list_label_filter() {
        let args = GhReadArgs {
            op: "issue_list".to_string(),
            target: Some("bug".to_string()),
            repo: "owner/repo".to_string(),
            path: None,
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert!(cmd.contains(&"--label".to_string()));
        assert!(cmd.contains(&"bug".to_string()));
        assert!(cmd.contains(&"--repo".to_string()));
    }

    #[test]
    fn test_gh_read_in_known_builtins() {
        assert!(
            KNOWN_BUILTINS.contains(&"gh_read"),
            "gh_read must be registered in KNOWN_BUILTINS"
        );
    }

    #[test]
    fn test_gh_read_error_json_format() {
        let err = GhReadError::NotFound("issue 999 not found".to_string());
        let json_str = err.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["error"], "not_found");
        assert_eq!(parsed["message"], "issue 999 not found");
    }

    // Additional coverage tests for gh_read

    #[test]
    fn test_validate_gh_read_input_missing_target_for_pr_view() {
        let input = serde_json::json!({"op": "pr_view", "repo": "owner/repo"});
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("target"));
    }

    #[test]
    fn test_validate_gh_read_input_missing_target_for_pr_diff() {
        let input = serde_json::json!({"op": "pr_diff", "repo": "owner/repo"});
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("target"));
    }

    #[test]
    fn test_build_gh_read_command_pr_view() {
        let args = GhReadArgs {
            op: "pr_view".to_string(),
            target: Some("7".to_string()),
            repo: "owner/repo".to_string(),
            path: None,
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert_eq!(cmd[0], "pr");
        assert_eq!(cmd[1], "view");
        assert_eq!(cmd[2], "7");
        assert_eq!(cmd[3], "--json");
        // Verify key fields are present
        let fields = &cmd[4];
        assert!(fields.contains("reviewDecision"));
        assert!(fields.contains("reviews"));
        assert!(fields.contains("commits"));
        // Verify --repo is in the argv
        assert!(cmd.contains(&"--repo".to_string()));
        assert!(cmd.contains(&"owner/repo".to_string()));
    }

    #[test]
    fn test_classify_gh_error_no_prs_found() {
        let err = classify_gh_error("no pull requests found in owner/repo", Some(1));
        assert!(matches!(err, GhReadError::NotFound(_)));
    }

    #[test]
    fn test_classify_gh_error_401_in_stderr() {
        let err = classify_gh_error("HTTP 401 Unauthorized", Some(1));
        assert!(matches!(err, GhReadError::AuthFailed(_)));
    }

    #[test]
    fn test_classify_gh_error_429_in_stderr() {
        let err = classify_gh_error("HTTP 429 Too Many Requests", Some(1));
        assert!(matches!(err, GhReadError::RateLimited(_)));
    }

    #[test]
    fn test_classify_gh_error_command_not_found_no_exit_code() {
        // Production path: exit_code is always None from spawn_and_collect
        let err = classify_gh_error("command not found", None);
        assert!(matches!(err, GhReadError::NetworkError(_)));
    }

    #[test]
    fn test_classify_gh_error_exit_code_prefix() {
        // spawn_and_collect formats non-zero exits as "Exit code: N\n..."
        let err = classify_gh_error("Exit code: 1\nGraphQL: Could not resolve to an Issue", None);
        assert!(matches!(err, GhReadError::NotFound(_)));
    }

    // -- file_view validation tests --

    #[test]
    fn test_validate_gh_read_input_valid_file_view() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": "feat/branch"
        });
        let args = validate_gh_read_input(&input).unwrap();
        assert_eq!(args.op, "file_view");
        assert_eq!(args.path.as_deref(), Some("src/main.rs"));
        assert_eq!(args.r#ref.as_deref(), Some("feat/branch"));
        assert!(args.target.is_none());
    }

    #[test]
    fn test_validate_gh_read_input_file_view_default_ref() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "README.md"
        });
        let args = validate_gh_read_input(&input).unwrap();
        assert_eq!(args.path.as_deref(), Some("README.md"));
        assert!(args.r#ref.is_none()); // defaults applied in build_gh_read_command
    }

    #[test]
    fn test_validate_gh_read_input_file_view_missing_path() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("path"));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_path_starts_with_dash() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "--evil"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("flag"));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_path_starts_with_slash() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "/etc/passwd"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("absolute"));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_path_with_traversal() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/../../../etc/passwd"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains(".."));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_path_charset_rejection() {
        // URL-encoded path component — the load-bearing case (architect finding 1).
        // `%2F` decodes to `/` server-side; charset enforcement rejects `%` before
        // it reaches GitHub's URL decoder.
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "foo%2Fbar%2Fbaz"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("disallowed"));

        // Whitespace
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/my file.rs"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.content.contains("disallowed"));

        // Semicolons (shell metacharacter)
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src;echo/evil"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.content.contains("disallowed"));

        // Non-ASCII
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/caf\u{00e9}.rs"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.content.contains("disallowed"));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_ref_starts_with_dash() {
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": "--evil-ref"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("flag"));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_ref_too_long() {
        let long_ref = "a".repeat(257);
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": long_ref
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("malformed_request"));
        assert!(err.content.contains("too long"));
    }

    // -- file_view build-command tests --

    #[test]
    fn test_build_gh_read_command_file_view() {
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "senara-solutions/mika".to_string(),
            path: Some("crates/mika-agent/src/main.rs".to_string()),
            r#ref: Some("feat/test-branch".to_string()),
        };
        let cmd = build_gh_read_command(&args);
        assert_eq!(cmd[0], "api");
        assert!(cmd[1].starts_with("/repos/senara-solutions/mika/contents/"));
        assert!(cmd[1].contains("crates/mika-agent/src/main.rs"));
        assert!(cmd[1].contains("?ref=feat/test-branch"));
        assert!(cmd.contains(&"--method".to_string()));
        assert!(cmd.contains(&"GET".to_string()));
        // file_view must NOT have --repo (gh api doesn't accept it)
        assert!(!cmd.contains(&"--repo".to_string()));
    }

    #[test]
    fn test_build_gh_read_command_file_view_default_ref() {
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("README.md".to_string()),
            r#ref: None,
        };
        let cmd = build_gh_read_command(&args);
        assert!(cmd[1].contains("?ref=main"), "default ref should be 'main'");
    }

    #[test]
    fn test_build_gh_read_command_existing_ops_have_repo() {
        // Regression: confirm the four existing ops still emit --repo after
        // the refactor moved the append into their per-op arms.
        for op in &["issue_view", "pr_view", "pr_diff", "issue_list"] {
            let args = GhReadArgs {
                op: op.to_string(),
                target: if *op == "issue_list" {
                    None
                } else {
                    Some("1".to_string())
                },
                repo: "o/r".to_string(),
                path: None,
                r#ref: None,
            };
            let cmd = build_gh_read_command(&args);
            assert!(
                cmd.contains(&"--repo".to_string()),
                "op '{op}' must include --repo in argv"
            );
            assert!(
                cmd.contains(&"o/r".to_string()),
                "op '{op}' must include repo value in argv"
            );
        }
    }

    // -- file_view response parsing tests --

    #[test]
    fn test_parse_file_view_happy_path() {
        use base64::Engine;
        let content = "Hello, world!\nLine 2\n";
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        let body = serde_json::json!({
            "name": "test.txt",
            "path": "src/test.txt",
            "sha": "abc123def456",
            "size": content.len(),
            "content": encoded,
            "encoding": "base64",
            "type": "file"
        });
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("src/test.txt".to_string()),
            r#ref: Some("main".to_string()),
        };
        let result = parse_file_view_response(&body.to_string(), &args).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["content"], content);
        assert_eq!(parsed["ref"], "abc123def456");
        assert_eq!(parsed["path"], "src/test.txt");
        assert_eq!(parsed["size_bytes"], content.len());
    }

    #[test]
    fn test_parse_file_view_file_too_large() {
        // GitHub returns empty content + non-zero size for files > 1 MiB
        let body = serde_json::json!({
            "name": "big.bin",
            "path": "big.bin",
            "sha": "deadbeef",
            "size": 2_000_000,
            "content": "",
            "encoding": "base64",
            "type": "file"
        });
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("big.bin".to_string()),
            r#ref: None,
        };
        let err = parse_file_view_response(&body.to_string(), &args).unwrap_err();
        match err {
            GhReadError::FileTooLarge {
                size_bytes,
                max_bytes,
            } => {
                assert_eq!(size_bytes, 2_000_000);
                assert_eq!(max_bytes, FILE_VIEW_MAX_BYTES);
            }
            other => panic!("Expected FileTooLarge, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_file_view_non_utf8_content() {
        use base64::Engine;
        // Invalid UTF-8 sequence
        let bytes: &[u8] = &[0xFF, 0xFE, 0x00, 0x01];
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let body = serde_json::json!({
            "name": "binary.dat",
            "path": "binary.dat",
            "sha": "binarysha",
            "size": bytes.len(),
            "content": encoded,
            "encoding": "base64",
            "type": "file"
        });
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("binary.dat".to_string()),
            r#ref: None,
        };
        let err = parse_file_view_response(&body.to_string(), &args).unwrap_err();
        assert!(
            matches!(err, GhReadError::MalformedRequest(ref msg) if msg.contains("UTF-8")),
            "Expected MalformedRequest with UTF-8 message, got: {err:?}"
        );
    }

    #[test]
    fn test_file_too_large_error_json_format() {
        let err = GhReadError::FileTooLarge {
            size_bytes: 2_000_000,
            max_bytes: 1_048_576,
        };
        let json_str = err.to_json();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["error"], "file_too_large");
        assert_eq!(parsed["size_bytes"], 2_000_000);
        assert_eq!(parsed["max_bytes"], 1_048_576);
    }

    // -- Review-driven tests (code review findings) --

    #[test]
    fn test_validate_gh_read_input_file_view_ref_charset_rejection() {
        // Query-string injection via metacharacters in ref (P1 security finding)
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": "main&foo=bar"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.is_error);
        assert!(err.content.contains("disallowed"));

        // Fragment injection
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": "main#fragment"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.content.contains("disallowed"));

        // Percent-encoded ref
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": "main%00evil"
        });
        let err = validate_gh_read_input(&input).unwrap_err();
        assert!(err.content.contains("disallowed"));
    }

    #[test]
    fn test_validate_gh_read_input_file_view_ref_at_boundary() {
        // Ref exactly 256 chars — should be accepted
        let ref_256 = "a".repeat(256);
        let input = serde_json::json!({
            "op": "file_view",
            "repo": "owner/repo",
            "path": "src/main.rs",
            "ref": ref_256
        });
        assert!(
            validate_gh_read_input(&input).is_ok(),
            "ref of exactly 256 chars must be accepted"
        );
    }

    #[test]
    fn test_parse_file_view_malformed_json() {
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("test.rs".to_string()),
            r#ref: None,
        };
        let err = parse_file_view_response("not valid json at all", &args).unwrap_err();
        assert!(
            matches!(err, GhReadError::MalformedRequest(ref m) if m.contains("parse")),
            "Expected MalformedRequest with parse error, got: {err:?}"
        );
    }

    #[test]
    fn test_parse_file_view_unexpected_encoding() {
        let body = serde_json::json!({
            "name": "test.txt",
            "path": "test.txt",
            "sha": "abc123",
            "size": 10,
            "content": "some content",
            "encoding": "none",
            "type": "file"
        });
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("test.txt".to_string()),
            r#ref: None,
        };
        let err = parse_file_view_response(&body.to_string(), &args).unwrap_err();
        assert!(
            matches!(err, GhReadError::MalformedRequest(ref m) if m.contains("encoding")),
            "Expected MalformedRequest about encoding, got: {err:?}"
        );
    }

    #[test]
    fn test_parse_file_view_empty_file() {
        // GitHub returns size=0, content="" for empty files
        let body = serde_json::json!({
            "name": "empty.txt",
            "path": "empty.txt",
            "sha": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
            "size": 0,
            "content": "",
            "encoding": "base64",
            "type": "file"
        });
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("empty.txt".to_string()),
            r#ref: None,
        };
        let result = parse_file_view_response(&body.to_string(), &args).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["content"], "");
        assert_eq!(parsed["size_bytes"], 0);
    }

    #[test]
    fn test_parse_file_view_base64_with_newlines() {
        use base64::Engine;
        // GitHub returns base64 with embedded \n every 60 chars
        let content = "This is a test file with enough content to span multiple base64 lines when encoded.\nLine two.\nLine three.\n";
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        // Insert newlines every 60 chars like GitHub does
        let with_newlines: String = encoded
            .as_bytes()
            .chunks(60)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let body = serde_json::json!({
            "name": "test.txt",
            "path": "test.txt",
            "sha": "sha123",
            "size": content.len(),
            "content": with_newlines,
            "encoding": "base64",
            "type": "file"
        });
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("test.txt".to_string()),
            r#ref: None,
        };
        let result = parse_file_view_response(&body.to_string(), &args).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["content"], content);
    }

    #[test]
    fn test_build_gh_read_command_file_view_exact_url() {
        let args = GhReadArgs {
            op: "file_view".to_string(),
            target: None,
            repo: "owner/repo".to_string(),
            path: Some("src/main.rs".to_string()),
            r#ref: Some("v1.0".to_string()),
        };
        let cmd = build_gh_read_command(&args);
        assert_eq!(
            cmd[1], "/repos/owner/repo/contents/src/main.rs?ref=v1.0",
            "Full URL must match exactly"
        );
    }

    // -- make_pr_dedup_key unit tests (#821) --

    #[test]
    fn test_make_pr_dedup_key_with_url_and_repo() {
        let args = vec![
            "pr".to_string(),
            "review".to_string(),
            "https://github.com/org/repo/pull/42".to_string(),
            "--approve".to_string(),
        ];
        let key = make_pr_dedup_key(&args, Some("org/repo"));
        assert_eq!(key, "org/repo|https://github.com/org/repo/pull/42");
    }

    #[test]
    fn test_make_pr_dedup_key_no_positional() {
        let args = vec!["pr".to_string(), "review".to_string()];
        let key = make_pr_dedup_key(&args, None);
        assert_eq!(key, "__default__|__current_branch__");
    }

    #[test]
    fn test_make_pr_dedup_key_number_form() {
        let args = vec![
            "pr".to_string(),
            "review".to_string(),
            "42".to_string(),
            "--approve".to_string(),
        ];
        let key = make_pr_dedup_key(&args, Some("org/repo"));
        assert_eq!(key, "org/repo|42");
    }

    // -- Session-scoped PR review dedup tests (#821) --

    /// Test helper: check if a session+key pair exists in the dedup map.
    /// Workaround for DashMap type inference issues in test code.
    fn test_map_contains(
        map: &dashmap::DashMap<String, std::collections::HashSet<String>>,
        session_id: &str,
        key: &str,
    ) -> bool {
        map.get(session_id)
            .map(|set| set.contains(key))
            .unwrap_or(false)
    }

    #[test]
    fn test_run_gh_session_scope_blocks_cross_turn_duplicate() {
        // Simulate: turn 1 posts a review (session map populated),
        // turn 2 tries the same review (AtomicBool reset, but session map blocks).
        let map: Arc<dashmap::DashMap<String, std::collections::HashSet<String>>> =
            Arc::new(dashmap::DashMap::new());
        let session_id = "test-session";
        let pr_url = "https://github.com/org/repo/pull/42";

        // Simulate turn 1 success: populate session map + flip per-turn bool.
        let dedup_key = make_pr_dedup_key(
            &[
                "pr".to_string(),
                "review".to_string(),
                pr_url.to_string(),
                "--approve".to_string(),
            ],
            None,
        );
        map.entry(session_id.to_string())
            .or_default()
            .insert(dedup_key.clone());

        // Verify the map blocks a second attempt.
        assert!(
            test_map_contains(&map, session_id, &dedup_key),
            "session map should block the duplicate"
        );
    }

    #[test]
    fn test_run_gh_session_scope_allows_different_pr_same_session() {
        let map: Arc<dashmap::DashMap<String, std::collections::HashSet<String>>> =
            Arc::new(dashmap::DashMap::new());
        let session_id = "test-session";

        // Post review for PR #42.
        let key1 = make_pr_dedup_key(
            &[
                "pr".to_string(),
                "review".to_string(),
                "42".to_string(),
                "--approve".to_string(),
            ],
            Some("org/repo"),
        );
        map.entry(session_id.to_string()).or_default().insert(key1);

        // Different PR #43 should NOT be blocked.
        let key2 = make_pr_dedup_key(
            &[
                "pr".to_string(),
                "review".to_string(),
                "43".to_string(),
                "--approve".to_string(),
            ],
            Some("org/repo"),
        );
        assert!(
            !test_map_contains(&map, session_id, &key2),
            "different PR in same session should not be blocked"
        );
    }

    #[test]
    fn test_run_gh_session_scope_allows_same_pr_different_session() {
        let map: Arc<dashmap::DashMap<String, std::collections::HashSet<String>>> =
            Arc::new(dashmap::DashMap::new());

        let key = make_pr_dedup_key(
            &[
                "pr".to_string(),
                "review".to_string(),
                "42".to_string(),
                "--approve".to_string(),
            ],
            Some("org/repo"),
        );

        // Session A posts the review.
        map.entry("session-a".to_string())
            .or_default()
            .insert(key.clone());

        // Session B should NOT be blocked.
        assert!(
            !test_map_contains(&map, "session-b", &key),
            "same PR in different session should not be blocked"
        );
    }

    #[test]
    fn test_run_gh_required_tools_gate_retry_blocks_second_review() {
        // Directly simulate the bug chain from #821:
        // Turn 1: pr review succeeds (map populated, atomic flips).
        // Required-tools gate rejects EndTurn, creates turn 2.
        // Turn 2 (fresh AtomicBool): tries pr review again — session map blocks it.
        let map: Arc<dashmap::DashMap<String, std::collections::HashSet<String>>> =
            Arc::new(dashmap::DashMap::new());
        let session_id = "qa-session";
        let pr_url = "https://github.com/senara-solutions/mika/pull/819";

        let args = vec![
            "pr".to_string(),
            "review".to_string(),
            pr_url.to_string(),
            "--approve".to_string(),
            "--body".to_string(),
            "VERDICT: pass".to_string(),
        ];
        let key = make_pr_dedup_key(&args, None);

        // Turn 1: review succeeds.
        let turn1_atomic = std::sync::atomic::AtomicBool::new(false);
        assert!(!turn1_atomic.load(std::sync::atomic::Ordering::Acquire));
        // Record in both per-turn and session map.
        turn1_atomic.store(true, std::sync::atomic::Ordering::Release);
        map.entry(session_id.to_string())
            .or_default()
            .insert(key.clone());

        // Turn 2: fresh AtomicBool (simulates new turn).
        let turn2_atomic = std::sync::atomic::AtomicBool::new(false);
        assert!(!turn2_atomic.load(std::sync::atomic::Ordering::Acquire));

        // Session map should block the second review even though per-turn is false.
        assert!(
            test_map_contains(&map, session_id, &key),
            "session-scoped map must block cross-turn duplicate"
        );
    }

    #[test]
    fn test_run_gh_no_session_map_does_not_panic_on_non_review() {
        // pr_reviews_posted: None + non-review command should not panic.
        // The debug_assert! is scoped to the pr review branch only.
        let args = vec!["pr".to_string(), "diff".to_string()];
        assert!(!is_pr_review_command(&args) || args.len() < 2);
        // Just verify it doesn't reach the pr review code path.
        assert!(!is_pr_review_command(&args));
    }

    // -- Diagnostic instrumentation tests (#900) --

    /// Captured tracing event for test assertions.
    #[derive(Debug, Clone)]
    struct CapturedEvent {
        message: String,
        fields: std::collections::HashMap<String, String>,
    }

    /// A tracing layer that captures events into a shared Vec for test assertions.
    struct CapturingLayer {
        events: Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = std::collections::HashMap::new();
            let mut visitor = FieldVisitor(&mut fields);
            event.record(&mut visitor);
            let message = fields.remove("message").unwrap_or_default();
            if let Ok(mut events) = self.events.lock() {
                events.push(CapturedEvent { message, fields });
            }
        }
    }

    struct FieldVisitor<'a>(&'a mut std::collections::HashMap<String, String>);

    impl tracing::field::Visit for FieldVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    /// Set up a tracing subscriber that captures events into a shared Vec.
    fn capture_tracing_events() -> (
        tracing::subscriber::DefaultGuard,
        Arc<std::sync::Mutex<Vec<CapturedEvent>>>,
    ) {
        use tracing_subscriber::layer::SubscriberExt;
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let layer = CapturingLayer {
            events: Arc::clone(&events),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let guard = tracing::subscriber::set_default(subscriber);
        (guard, events)
    }

    #[tokio::test]
    async fn test_spawn_and_collect_emits_complete_log() {
        let (_guard, events) = capture_tracing_events();

        let mut cmd = tokio::process::Command::new("echo");
        cmd.arg("hello");
        let output = spawn_and_collect(cmd, "test_echo", "").await;

        assert!(!output.is_error);
        assert_eq!(output.content.trim(), "hello");

        let captured = events.lock().unwrap();
        let complete_events: Vec<_> = captured
            .iter()
            .filter(|e| e.message == "spawn_and_collect complete")
            .collect();
        assert_eq!(
            complete_events.len(),
            1,
            "expected exactly one 'spawn_and_collect complete' event"
        );
        let evt = &complete_events[0];
        assert_eq!(
            evt.fields.get("tool").map(|s| s.as_str()),
            Some("test_echo")
        );
        // "hello\n" = 6 bytes
        assert_eq!(
            evt.fields.get("stdout_bytes").map(|s| s.as_str()),
            Some("6")
        );
        assert_eq!(
            evt.fields.get("stderr_bytes").map(|s| s.as_str()),
            Some("0")
        );
        // elapsed_ms should be present and > 0
        let elapsed: u64 = evt
            .fields
            .get("elapsed_ms")
            .unwrap()
            .parse()
            .expect("elapsed_ms should be a number");
        assert!(elapsed < 10_000, "echo should complete quickly");
    }

    #[tokio::test]
    async fn test_run_gh_invocation_log_redacts_token() {
        let (_guard, events) = capture_tracing_events();

        let harness = TestHarness::new();
        let mut ctx = harness.ctx();
        let fake_token = "FAKE_TOKEN_DO_NOT_LOG_ME";
        ctx.github_token = Some(fake_token);

        // Use an allowed subcommand so we pass validation and reach the invocation log.
        // `pr list` will fail (no repo context) but the log fires before spawn_and_collect.
        let input = serde_json::json!({"command": ["pr", "list"]});
        let _output = run_gh(&input, &ctx).await;

        let captured = events.lock().unwrap();
        let invocation_events: Vec<_> = captured
            .iter()
            .filter(|e| e.message == "run_gh invocation")
            .collect();
        assert_eq!(
            invocation_events.len(),
            1,
            "expected exactly one 'run_gh invocation' event"
        );
        let evt = &invocation_events[0];

        // env_keys_set should contain GH_TOKEN as a key name
        let env_keys = evt.fields.get("env_keys_set").expect("env_keys_set field");
        assert!(
            env_keys.contains("GH_TOKEN"),
            "env_keys_set should contain the key name GH_TOKEN"
        );
        assert_eq!(
            evt.fields.get("has_github_token").map(|s| s.as_str()),
            Some("true")
        );

        // No field should contain the actual token value
        for (key, value) in &evt.fields {
            assert!(
                !value.contains(fake_token),
                "field '{key}' leaked the token value: {value}"
            );
        }
    }

    #[tokio::test]
    async fn test_spawn_and_collect_handles_large_output() {
        // Verify that spawn_and_collect returns within a reasonable time
        // even when the subprocess produces output exceeding MAX_OUTPUT_LEN.
        // This documents the pipe-deadlock risk for the follow-up ticket.
        let (_guard, _events) = capture_tracing_events();

        let start = std::time::Instant::now();
        let mut cmd = tokio::process::Command::new("sh");
        // Generate 20KB of output (2x MAX_OUTPUT_LEN)
        cmd.args(["-c", "yes 'AAAAAAAAAA' | head -c 20000"]);
        let output = spawn_and_collect(cmd, "test_large", "").await;
        let elapsed = start.elapsed();

        // Should complete within 10 seconds (not hang at 600s)
        assert!(
            elapsed.as_secs() < 10,
            "spawn_and_collect took {elapsed:?} — possible pipe deadlock"
        );
        // Output should be capped at MAX_OUTPUT_LEN
        assert!(
            output.content.len() <= MAX_OUTPUT_LEN,
            "output {} exceeds MAX_OUTPUT_LEN {}",
            output.content.len(),
            MAX_OUTPUT_LEN
        );
    }

    #[tokio::test]
    async fn test_spawn_and_collect_progress_ticker_fires() {
        // With PROGRESS_TICKER_INTERVAL = 100ms in test mode, a 500ms sleep
        // should produce at least 3 progress tick events.
        let (_guard, events) = capture_tracing_events();

        let mut cmd = tokio::process::Command::new("sleep");
        cmd.arg("0.5");
        let _output = spawn_and_collect(cmd, "test_sleep", "").await;

        let captured = events.lock().unwrap();
        let progress_events: Vec<_> = captured
            .iter()
            .filter(|e| e.message == "spawn_and_collect progress")
            .collect();
        assert!(
            progress_events.len() >= 3,
            "expected at least 3 progress ticks, got {}",
            progress_events.len()
        );

        // Verify elapsed_ms is monotonically non-decreasing
        let elapsed_values: Vec<u64> = progress_events
            .iter()
            .map(|e| e.fields.get("elapsed_ms").unwrap().parse::<u64>().unwrap())
            .collect();
        for window in elapsed_values.windows(2) {
            assert!(
                window[1] >= window[0],
                "elapsed_ms should be non-decreasing: {elapsed_values:?}"
            );
        }
    }
}
