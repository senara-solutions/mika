//! Dispatch for builtin skill tool handlers.
//!
//! Builtin handlers are Rust functions invoked directly by the agent loop,
//! without spawning a subprocess or making an HTTP call. They have access
//! to `ToolContext` (for home_dir, etc.) and return `ToolOutput`.

use std::fmt::Write;
use std::sync::LazyLock;

use tokio::io::AsyncReadExt;

use crate::skills::index::sanitize_model_dir_name;
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

/// Validate `git_ops` input and extract parameters.
#[derive(Debug)]
struct GitOpsInput {
    operation: String,
    repo_path: String,
    base: String,
    push: bool,
}

fn validate_git_ops_input(input: &serde_json::Value) -> Result<GitOpsInput, ToolOutput> {
    let operation = input
        .get("operation")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| {
            ToolOutput::error(
                "Missing required 'operation' parameter. Must be one of: fetch, rebase, merge."
                    .to_string(),
            )
        })?;

    if !["fetch", "rebase", "merge"].contains(&operation.as_str()) {
        return Err(ToolOutput::error(format!(
            "Unknown operation '{operation}'. Must be one of: fetch, rebase, merge."
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

    Ok(GitOpsInput {
        operation,
        repo_path,
        base,
        push,
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

    // For rebase/merge, check working tree is clean
    if operation == "rebase" || operation == "merge" {
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

// ---------------------------------------------------------------------------
// review_skill — gather skill prompt data for model-tuned variant generation
// ---------------------------------------------------------------------------

/// Maximum size for a root prompt included in the response (characters).
const MAX_PROMPT_IN_RESPONSE: usize = 8_000;

/// Resolve the canonical (provider, model) tuple for variant directory naming.
///
/// For aggregator providers (e.g. OpenRouter) whose model names contain a slash
/// (`anthropic/claude-sonnet-4`), extracts the real provider and model so that
/// variants are filed under the canonical provider directory.
///
/// For direct providers the inputs are returned as-is.
fn resolve_canonical_provider_model<'a>(
    provider_name: &'a str,
    model_name: &'a str,
) -> (&'a str, &'a str) {
    let Ok(kind) = provider_name.parse::<mika_common::llm::ProviderKind>() else {
        return (provider_name, model_name);
    };

    if kind.model_names_contain_slash()
        && let Some((real_provider, real_model)) = model_name.split_once('/')
        && !real_provider.is_empty()
        && !real_model.is_empty()
    {
        return (real_provider, real_model);
    }

    (provider_name, model_name)
}

/// Gather skill prompt data and resolve variant paths for model-tuned variant
/// generation.  Returns structured JSON so the agent loop can perform the
/// creative prompt adaptation and write the result via `write_agent_file`.
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

    // --- Resolve canonical provider / model --------------------------------
    let (canonical_provider, canonical_model) =
        resolve_canonical_provider_model(ctx.provider_name, ctx.model_name);
    let sanitized_model = sanitize_model_dir_name(canonical_model);

    let skills_dir = ctx.home_dir.join("skills");

    // --- Batch mode --------------------------------------------------------
    if skill_name == "*" {
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
    )
    .await
}

/// Handle a single-skill review_skill invocation.
async fn review_skill_single(
    skills_dir: &std::path::Path,
    skill_name: &str,
    canonical_provider: &str,
    canonical_model: &str,
    sanitized_model: &str,
    dry_run: bool,
    force: bool,
) -> ToolOutput {
    let skill_dir = skills_dir.join(skill_name);

    // Existence check
    if !skill_dir.exists() {
        return ToolOutput::error(format!(
            "Skill '{skill_name}' not found. Check the name with `list_agent_files` \
             at path 'skills/'."
        ));
    }

    // Linked-skill check (symlink → refuse)
    match std::fs::symlink_metadata(&skill_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return ToolOutput::error(format!(
                "Skill '{skill_name}' is installed with --link. Variants cannot be \
                 written to linked skills (read-only invariant). Unlink first with \
                 `mika skills uninstall {skill_name}` then reinstall without --link."
            ));
        }
        Err(e) => {
            return ToolOutput::error(format!(
                "Cannot read skill directory for '{skill_name}': {e}"
            ));
        }
        _ => {}
    }

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

    // Read tools.json (optional — prompt-only skills may not have one)
    let tools_path = skill_dir.join("tools.json");
    let tools_json = std::fs::read_to_string(&tools_path).unwrap_or_else(|_| "[]".to_string());

    // Compute variant path (relative to agent home for write_agent_file)
    let variant_rel =
        format!("skills/{skill_name}/{canonical_provider}/{sanitized_model}/system_prompt.md");
    let variant_abs = skills_dir.parent().unwrap_or(skills_dir).join(&variant_rel);

    // Check existing variant
    let existing_variant = std::fs::read_to_string(&variant_abs).ok();

    if existing_variant.is_some() && !force {
        let result = serde_json::json!({
            "skill_name": skill_name,
            "provider": canonical_provider,
            "model": canonical_model,
            "variant_path": variant_rel,
            "skipped": true,
            "reason": "variant already exists (use force=true to overwrite)",
            "dry_run": dry_run,
        });
        return ToolOutput::success(serde_json::to_string_pretty(&result).unwrap());
    }

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

    let result = serde_json::json!({
        "skill_name": skill_name,
        "root_prompt": prompt_for_response,
        "tools_json": tools_json,
        "provider": canonical_provider,
        "model": canonical_model,
        "variant_path": variant_rel,
        "existing_variant": existing_variant,
        "dry_run": dry_run,
        "skipped": false,
    });

    ToolOutput::success(serde_json::to_string_pretty(&result).unwrap())
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

        // Check existing variant
        let variant_path = path
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
    async fn test_review_skill_linked_skill_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skills_dir = home.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        // Create a real directory and symlink to it
        let real_dir = tmp.path().join("real-skill");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("system_prompt.md"), "test prompt").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, skills_dir.join("linked-skill")).unwrap();
        #[cfg(not(unix))]
        {
            // Skip on non-unix
            return;
        }
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "linked-skill"}), &ctx).await;
        assert!(output.is_error);
        assert!(output.content.contains("--link"));
        assert!(output.content.contains("read-only invariant"));
    }

    #[tokio::test]
    async fn test_review_skill_single_happy_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/web-search");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Search the web.").unwrap();
        std::fs::write(skill_dir.join("tools.json"), r#"[{"name": "web_search"}]"#).unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "web-search"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["skill_name"], "web-search");
        assert_eq!(parsed["provider"], "anthropic");
        assert_eq!(parsed["model"], "claude-sonnet-4-6");
        assert_eq!(parsed["root_prompt"], "Search the web.");
        assert_eq!(parsed["skipped"], false);
        assert!(
            parsed["variant_path"]
                .as_str()
                .unwrap()
                .contains("anthropic/claude-sonnet-4-6/system_prompt.md")
        );
    }

    #[tokio::test]
    async fn test_review_skill_existing_variant_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/web-search");
        let variant_dir = skill_dir.join("anthropic/claude-sonnet-4-6");
        std::fs::create_dir_all(&variant_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Search the web.").unwrap();
        std::fs::write(variant_dir.join("system_prompt.md"), "Existing variant.").unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(&serde_json::json!({"skill_name": "web-search"}), &ctx).await;
        assert!(!output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed["skipped"], true);
        assert!(
            parsed["reason"]
                .as_str()
                .unwrap()
                .contains("already exists")
        );
    }

    #[tokio::test]
    async fn test_review_skill_existing_variant_force() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let skill_dir = home.join("skills/web-search");
        let variant_dir = skill_dir.join("anthropic/claude-sonnet-4-6");
        std::fs::create_dir_all(&variant_dir).unwrap();
        std::fs::write(skill_dir.join("system_prompt.md"), "Search the web.").unwrap();
        std::fs::write(variant_dir.join("system_prompt.md"), "Existing variant.").unwrap();
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_home(home);
        let output = review_skill(
            &serde_json::json!({"skill_name": "web-search", "force": true}),
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
}
