// READ-ONLY INVARIANT: Skills must never write to their own directory at runtime.
// This is critical for `--link` mode where the skill directory is a symlink to the
// author's source directory. Writing back would silently modify the author's source
// files, creating shared-mutable-state bugs. All skill output goes to stdout/stderr.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

use super::index::ResolvedSkillTool;
use super::manifest::ToolHandler;
use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use crate::task_engine::types::{action_type, trigger_type};
use crate::tools::{GitHubRef, ImageData, ToolOutput, parse_github_ref};

/// Maximum output size from a skill tool (10,000 characters).
const MAX_OUTPUT_LEN: usize = 10_000;

/// Non-`MIKA_*` env vars that must also be scrubbed from child processes.
///
/// `GH_TOKEN` is removed to prevent identity collision: if it leaked from
/// `~/.mika/.env` via dotenvy, it would override the host's `gh auth` identity
/// in ALL child processes. `run_gh` explicitly re-injects the correct platform
/// token AFTER this scrub. See issue #380.
const EXTRA_SCRUB_VARS: &[&str] = &["GH_TOKEN"];

/// Scrub all `MIKA_*` environment variables (and [`EXTRA_SCRUB_VARS`]) from a
/// tokio Command (defense-in-depth).
///
/// Prevents leaking secrets like `MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`,
/// and `MIKA_OPENAI_API_KEY` to child processes.
pub(crate) fn scrub_mika_env_vars(cmd: &mut tokio::process::Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }
    for key in EXTRA_SCRUB_VARS {
        cmd.env_remove(key);
    }
}

/// Scrub all `MIKA_*` environment variables (and [`EXTRA_SCRUB_VARS`]) from a
/// std Command (defense-in-depth).
///
/// Same as [`scrub_mika_env_vars`] but for synchronous `std::process::Command`.
pub(crate) fn scrub_mika_env_vars_std(cmd: &mut std::process::Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }
    for key in EXTRA_SCRUB_VARS {
        cmd.env_remove(key);
    }
}

/// Maximum raw image file size (5 MB).
const MAX_IMAGE_SIZE: u64 = 5 * 1024 * 1024;

/// Maximum number of images per tool result.
const MAX_IMAGES_PER_RESULT: usize = 5;

// -- Mika envelope protocol for image-bearing tool results --

/// Top-level JSON envelope output by exec handlers that return images.
///
/// Scripts output `{"__mika_v1": {"text": "...", "images": ["/path/to/img.png"]}}`.
#[derive(Deserialize)]
struct MikaEnvelope {
    __mika_v1: MikaOutput,
}

#[derive(Deserialize)]
struct MikaOutput {
    text: String,
    #[serde(default)]
    images: Vec<String>,
}

/// Context for spawning long-running background exec handlers.
///
/// When present and the handler has `long_running: true`, the executor creates
/// a callback task and spawns the subprocess in the background instead of
/// blocking the agent loop.
pub struct LongRunningContext {
    pub db: AsyncDatabase,
    pub agent_name: String,
    pub session_id: String,
    pub trace_id: String,
    /// Per-turn dispatch counter (#583). Only one long-running dispatch is
    /// permitted per agent turn. Atomic for interior mutability through `&self`.
    pub dispatch_count: AtomicU32,
}

/// Execute a skill tool with the appropriate handler.
///
/// Applies a per-skill timeout wrapping the inner execution.
/// If `long_running_ctx` is Some and the handler is `Exec { long_running: true }`,
/// the subprocess is spawned in the background with a callback task.
pub async fn execute_skill_tool(
    skill_tool: &ResolvedSkillTool,
    input: serde_json::Value,
    timeout_secs: u64,
    long_running_ctx: Option<&LongRunningContext>,
    github_token: Option<&str>,
) -> ToolOutput {
    // Check for long-running exec handler
    if let ToolHandler::Exec {
        command,
        long_running: true,
        estimated_duration_secs,
    } = &skill_tool.handler
        && let Some(ctx) = long_running_ctx
    {
        return execute_long_running(
            skill_tool,
            command,
            input,
            *estimated_duration_secs,
            ctx,
            github_token,
        )
        .await;
    }

    // Refuse long-running tools when no long-running context is available
    // (callback turns, silent mode, CLI test). The sync exec path does not
    // inject __mika_task_id/__mika_agent, so the handler would crash with
    // a cryptic error. Return an explicit error instead (#537).
    if matches!(
        &skill_tool.handler,
        ToolHandler::Exec {
            long_running: true,
            ..
        }
    ) && long_running_ctx.is_none()
    {
        warn!(
            tool = %skill_tool.definition.name,
            "long-running tool invoked without long_running_ctx"
        );
        return ToolOutput::error(format!(
            "Tool '{}' is declared long_running but cannot run in the current context \
             (callback turn, silent mode, or CLI test). Long-running tools require a \
             conversation-mode turn with an active task engine.",
            skill_tool.definition.name
        ));
    }

    let timeout = Duration::from_secs(timeout_secs);
    match tokio::time::timeout(timeout, execute_inner(skill_tool, input, github_token)).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            warn!(
                tool = %skill_tool.definition.name,
                error = %e,
                "skill tool execution failed"
            );
            ToolOutput::error(format!("Skill tool error: {e}"))
        }
        Err(_) => {
            warn!(
                tool = %skill_tool.definition.name,
                timeout_secs,
                "skill tool timed out"
            );
            ToolOutput::error(format!(
                "Skill tool '{}' timed out after {timeout_secs}s",
                skill_tool.definition.name
            ))
        }
    }
}

async fn execute_inner(
    skill_tool: &ResolvedSkillTool,
    input: serde_json::Value,
    github_token: Option<&str>,
) -> Result<ToolOutput> {
    tracing::info!(
        tool = %skill_tool.definition.name,
        input = %input,
        "executing skill tool"
    );
    match &skill_tool.handler {
        ToolHandler::Exec { command, .. } => {
            execute_exec(
                command,
                &skill_tool.skill_dir,
                &skill_tool.definition.name,
                input,
                github_token,
            )
            .await
        }
        ToolHandler::Http { url, method } => execute_http(url, method, input).await,
        ToolHandler::Builtin { .. } => {
            // Builtin handlers are dispatched directly from agent.rs, not through executor.
            // This path should never be reached.
            bail!("Builtin handlers must be dispatched from the agent loop, not the executor")
        }
    }
}

/// Attempt to parse exec handler stdout as a `__mika_v1` image envelope.
///
/// Returns `Some(MikaOutput)` if the output is valid JSON with the sentinel key,
/// `None` otherwise (plain text output — backward compatible).
fn try_parse_envelope(stdout: &str) -> Option<MikaOutput> {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') || !trimmed.contains(r#""__mika_v1""#) {
        return None;
    }
    serde_json::from_str::<MikaEnvelope>(trimmed)
        .ok()
        .map(|e| e.__mika_v1)
}

/// Read an image file from disk, validate it, and return base64-encoded data.
///
/// Security checks:
/// - Canonicalizes path (resolves symlinks)
/// - Verifies regular file (rejects devices, sockets, etc.)
/// - Enforces 5 MB size limit via metadata pre-check AND capped read (TOCTOU-safe)
/// - Magic-byte validation for supported image types (JPEG, PNG, GIF, WebP)
async fn read_and_validate_image(path: &str) -> Result<ImageData, String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        use std::fs;
        use std::io::Read;

        let canonical = fs::canonicalize(&path)
            .map_err(|e| format!("cannot resolve image path '{}': {}", path, e))?;

        let metadata = fs::metadata(&canonical)
            .map_err(|e| format!("cannot read image '{}': {}", canonical.display(), e))?;

        if !metadata.is_file() {
            return Err(format!("not a regular file: {}", canonical.display()));
        }

        if metadata.len() > MAX_IMAGE_SIZE {
            return Err(format!(
                "image too large: {} bytes (max {} bytes)",
                metadata.len(),
                MAX_IMAGE_SIZE
            ));
        }

        // Use capped read to prevent TOCTOU race (file could grow between metadata and read)
        let file = fs::File::open(&canonical)
            .map_err(|e| format!("cannot open image '{}': {}", canonical.display(), e))?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_IMAGE_SIZE + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("cannot read image '{}': {}", canonical.display(), e))?;

        if bytes.len() as u64 > MAX_IMAGE_SIZE {
            return Err(format!(
                "image too large: {} bytes (max {} bytes)",
                bytes.len(),
                MAX_IMAGE_SIZE
            ));
        }

        let media_type = detect_image_type(&bytes)
            .ok_or_else(|| format!("not a supported image type: {}", canonical.display()))?;

        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);

        Ok(ImageData {
            media_type: media_type.to_string(),
            data,
        })
    })
    .await
    .map_err(|e| format!("image read task panicked: {e}"))?
}

/// Detect image type from magic bytes. Returns MIME type or None.
fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("image/png")
    } else if bytes.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        Some("image/gif")
    } else if bytes.len() >= 12
        && bytes[..4] == [0x52, 0x49, 0x46, 0x46]
        && bytes[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        Some("image/webp")
    } else {
        None
    }
}

/// Process image file paths from a Mika envelope, returning validated images
/// and any error notes for paths that couldn't be loaded.
async fn process_envelope_images(image_paths: &[String]) -> (Vec<ImageData>, Vec<String>) {
    let mut images = Vec::new();
    let mut errors = Vec::new();

    for (i, path) in image_paths.iter().enumerate() {
        if i >= MAX_IMAGES_PER_RESULT {
            errors.push(format!(
                "skipped {} image(s): max {} per result",
                image_paths.len() - MAX_IMAGES_PER_RESULT,
                MAX_IMAGES_PER_RESULT
            ));
            break;
        }
        match read_and_validate_image(path).await {
            Ok(img) => images.push(img),
            Err(e) => {
                warn!(path, error = %e, "failed to load envelope image");
                errors.push(e);
            }
        }
    }

    (images, errors)
}

/// Execute an exec-type handler by spawning a subprocess.
///
/// - Resolves the command path relative to the skill directory
/// - Pipes input JSON to stdin
/// - Returns stdout regardless of exit code; prefixes `Exit code: N` on non-zero
/// - Detects `__mika_v1` envelope for image-bearing results (exit 0 only)
async fn execute_exec(
    command: &str,
    skill_dir: &std::path::Path,
    tool_name: &str,
    input: serde_json::Value,
    github_token: Option<&str>,
) -> Result<ToolOutput> {
    // Resolve command relative to skill directory
    let cmd_path = skill_dir.join(command);
    info!(command = %cmd_path.display(), "executing skill command");
    if !cmd_path.exists() {
        bail!(
            "handler command not found: {} (resolved to {})",
            command,
            cmd_path.display()
        );
    }

    let mut child = loop {
        let mut cmd = tokio::process::Command::new(&cmd_path);
        cmd.current_dir(skill_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .kill_on_drop(true);
        scrub_mika_env_vars(&mut cmd);
        // Re-inject agent's GitHub token for platform identity separation.
        // Same pattern as builtin run_gh handler (builtin_handlers.rs).
        if let Some(token) = github_token {
            cmd.env("GH_TOKEN", token);
        }
        match cmd.spawn() {
            Ok(child) => break child,
            Err(e) if e.raw_os_error() == Some(26 /* ETXTBSY */) => {
                // ETXTBSY — another process has the file open for writing.
                // Retry after a brief yield (common fork+exec race).
                tokio::task::yield_now().await;
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    };

    // Write input JSON to stdin and close.
    // Ignore BrokenPipe — the child may exit without reading stdin.
    if let Some(mut stdin) = child.stdin.take() {
        let input_bytes = serde_json::to_vec(&input)?;
        match stdin.write_all(&input_bytes).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(e) => return Err(e.into()),
        }
        // stdin is dropped here, closing the pipe
    }

    let output = child.wait_with_output().await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Log output for debugging (regardless of exit code)
    // Use char-boundary-safe slicing to avoid panics on multi-byte UTF-8
    let stdout_end = {
        let mut b = stdout.len().min(200);
        while b > 0 && !stdout.is_char_boundary(b) {
            b -= 1;
        }
        b
    };
    debug!(
        tool = %tool_name,
        exit_success = output.status.success(),
        stdout_len = stdout.len(),
        stdout_preview = %&stdout[..stdout_end],
        "skill exec output"
    );
    if !stderr.trim().is_empty() {
        let stderr_end = {
            let mut b = stderr.len().min(500);
            while b > 0 && !stderr.is_char_boundary(b) {
                b -= 1;
            }
            b
        };
        debug!(
            tool = %tool_name,
            stderr = %&stderr[..stderr_end],
            "skill exec stderr"
        );
    }

    if output.status.success() {
        // Exit 0: parse envelope and return stdout
        if let Some(envelope) = try_parse_envelope(&stdout) {
            let (images, errors) = process_envelope_images(&envelope.images).await;
            let mut text = truncate_output(&envelope.text);
            if !errors.is_empty() {
                text.push_str("\n[image errors: ");
                text.push_str(&errors.join("; "));
                text.push(']');
            }
            if images.is_empty() {
                Ok(ToolOutput::success(text))
            } else {
                Ok(ToolOutput::success_with_images(text, images))
            }
        } else {
            Ok(ToolOutput::success(truncate_output(&stdout)))
        }
    } else {
        // Non-zero exit: return success with exit code prefix.
        // The agent decides whether the exit code represents a real failure —
        // many tools (grep, linters, health checks) use non-zero to signal
        // status, not errors.
        let code_display = match output.status.code() {
            Some(code) => format!("Exit code: {code}"),
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    match output.status.signal() {
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

        // Combine stdout and stderr. Append stderr only if it has content
        // not already in stdout (run.sh merges them with 2>&1).
        let mut combined = stdout.to_string();
        let stderr_trimmed = stderr.trim();
        if !stderr_trimmed.is_empty() && stderr_trimmed != stdout.trim() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(stderr_trimmed);
        }

        let truncated = truncate_output(&combined);
        Ok(ToolOutput::success(format!("{code_display}\n{truncated}")))
    }
}

/// Execute an HTTP-type handler by making an HTTP request.
///
/// - POST/PUT: sends input as JSON body
/// - GET: sends input as query parameters
async fn execute_http(url: &str, method: &str, input: serde_json::Value) -> Result<ToolOutput> {
    let client = reqwest::Client::new();

    let request = match method.to_uppercase().as_str() {
        "GET" => {
            // For GET, serialize input object as query params
            let mut req = client.get(url);
            if let serde_json::Value::Object(map) = &input {
                let params: Vec<(String, String)> = map
                    .iter()
                    .map(|(k, v)| {
                        let val = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        (k.clone(), val)
                    })
                    .collect();
                req = req.query(&params);
            }
            req
        }
        "POST" => client.post(url).json(&input),
        "PUT" => client.put(url).json(&input),
        other => bail!("unsupported HTTP method: {other}"),
    };

    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;

    if status.is_success() {
        Ok(ToolOutput::success(truncate_output(&body)))
    } else {
        Ok(ToolOutput::error(format!(
            "HTTP {}: {}",
            status.as_u16(),
            truncate_output(&body)
        )))
    }
}

/// Truncate output to MAX_OUTPUT_LEN characters.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_OUTPUT_LEN {
        s.to_string()
    } else {
        let mut boundary = MAX_OUTPUT_LEN;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!(
            "{}\n... (truncated at {MAX_OUTPUT_LEN} chars)",
            &s[..boundary]
        )
    }
}

use crate::github_graphql::fetch_open_blockers;

/// Validate that a task is in a dispatchable state for long-running execution.
///
/// Stricter than `validate_task()` (which also allows `blocked` for delegation).
/// Long-running dispatch only permits `pending` and `in_progress`, and rejects if an
/// active callback child task already exists (double-dispatch prevention).
///
/// Returns `Err(json_error_string)` on rejection, `Ok(status)` with the task's
/// current status if dispatch may proceed.
async fn validate_dispatch_readiness(
    db: &AsyncDatabase,
    task_id: &str,
    github_token: Option<&str>,
) -> Result<String, String> {
    // Re-fetch the task to get the full struct (validate_task confirmed existence)
    let task = match db.get_task(task_id).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            // Should not happen after validate_task, but defense-in-depth
            return Err(serde_json::json!({
                "error": "task_not_found",
                "task_id": task_id,
                "reason": "Task does not exist in the database"
            })
            .to_string());
        }
        Err(e) => {
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to fetch task for dispatch check: {e}")
            })
            .to_string());
        }
    };

    // Only pending and in_progress are dispatchable
    if !matches!(task.status.as_str(), "pending" | "in_progress") {
        let pr_url = extract_pr_url(&task.metadata);
        return Err(serde_json::json!({
            "error": "task_not_dispatchable",
            "task_id": task_id,
            "current_status": task.status,
            "pr_url": pr_url,
            "reason": format!(
                "Task is in '{}' state and cannot be dispatched. \
                 Only 'pending' and 'in_progress' tasks can be dispatched.",
                task.status
            )
        })
        .to_string());
    }

    // Check for active callback children (double-dispatch prevention)
    match db.get_child_tasks(task_id).await {
        Ok(children) => {
            let active_callback = children.iter().find(|c| {
                c.trigger_type == "callback"
                    && matches!(c.status.as_str(), "pending" | "in_progress")
            });
            if let Some(child) = active_callback {
                let pr_url = extract_pr_url(&task.metadata);
                return Err(serde_json::json!({
                    "error": "task_active_dispatch",
                    "task_id": task_id,
                    "current_status": task.status,
                    "active_child_id": child.id,
                    "active_child_status": child.status,
                    "pr_url": pr_url,
                    "reason": format!(
                        "Task already has an active dispatch (callback task '{}' \
                         in '{}' status). Wait for it to complete or cancel it before \
                         dispatching again.",
                        child.id, child.status
                    )
                })
                .to_string());
            }
        }
        Err(e) => {
            // Fail-closed: if we can't check children, reject dispatch
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to check active dispatches for task: {e}")
            })
            .to_string());
        }
    }

    // Global dispatch guard (#583): reject if ANY other task has an active
    // callback child. Enforces single-session-at-a-time across all tasks.
    match db.has_active_callback_tasks_excluding(task_id).await {
        Ok(Some((blocking_parent_id, blocking_callback_id))) => {
            return Err(serde_json::json!({
                "error": "global_dispatch_active",
                "task_id": task_id,
                "blocking_task_id": blocking_parent_id,
                "blocking_callback_id": blocking_callback_id,
                "reason": format!(
                    "Another task ('{}') already has an active dispatch \
                     (callback task '{}'). Only one long-running dispatch may be \
                     active at a time. Wait for it to complete or cancel it before \
                     dispatching again.",
                    blocking_parent_id, blocking_callback_id
                )
            })
            .to_string());
        }
        Ok(None) => { /* No conflicting dispatch — proceed */ }
        Err(e) => {
            // Fail-closed: if we can't check global state, reject dispatch
            return Err(serde_json::json!({
                "error": "dispatch_check_failed",
                "task_id": task_id,
                "reason": format!("Failed to check global dispatch state: {e}")
            })
            .to_string());
        }
    }

    // Blocked-by guard (#713): reject dispatch if the ticket's GitHub blockers
    // are still open. This is the most expensive check (external API call) so it
    // runs last, after all cheap DB checks have passed.
    if let Some(GitHubRef::Issue {
        owner,
        repo,
        number,
    }) = task.reference_url.as_deref().and_then(parse_github_ref)
    {
        match github_token {
            Some(token) => match fetch_open_blockers(token, &owner, &repo, number).await {
                Ok(blockers) if !blockers.is_empty() => {
                    return Err(serde_json::json!({
                        "error": "dispatch_blocked_by",
                        "task_id": task_id,
                        "blocking_issues": blockers,
                        "message": format!(
                            "ticket #{number} is blocked by {} which {} still open",
                            blockers.iter().map(|n| format!("#{n}")).collect::<Vec<_>>().join(", "),
                            if blockers.len() == 1 { "is" } else { "are" }
                        )
                    })
                    .to_string());
                }
                Ok(_) => { /* No open blockers — proceed */ }
                Err(e) => {
                    // Fail-closed: if we can't verify blocker state, reject dispatch
                    warn!(
                        task_id = task_id,
                        error = %e,
                        "blocked-by check failed, rejecting dispatch"
                    );
                    return Err(serde_json::json!({
                        "error": "dispatch_check_failed",
                        "task_id": task_id,
                        "reason": format!("Failed to check blocked-by status: {e}")
                    })
                    .to_string());
                }
            },
            None => {
                warn!(
                    task_id = task_id,
                    "Skipping blocked-by check: no GitHub token configured"
                );
            }
        }
    }

    Ok(task.status.clone())
}

/// Extract `pr_url` from a task's metadata JSON.
///
/// Looks for `claude_pilot.pr_url` (nested) or `pr_url` (top-level).
fn extract_pr_url(metadata: &Option<String>) -> Option<String> {
    let meta = metadata.as_deref()?;
    let parsed: serde_json::Value = serde_json::from_str(meta).ok()?;

    // Try nested claude_pilot.pr_url first
    if let Some(url) = parsed
        .get("claude_pilot")
        .and_then(|cp| cp.get("pr_url"))
        .and_then(|v| v.as_str())
    {
        return Some(url.to_string());
    }

    // Fallback to top-level pr_url
    parsed
        .get("pr_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Execute a long-running exec handler by creating a callback task and spawning
/// the subprocess in the background. Returns immediately with the task ID.
async fn execute_long_running(
    skill_tool: &ResolvedSkillTool,
    command: &str,
    input: serde_json::Value,
    estimated_duration_secs: Option<u64>,
    ctx: &LongRunningContext,
    github_token: Option<&str>,
) -> ToolOutput {
    // Validate task_id — long-running tasks require tracked tasks.
    // The agent passes the task UUID via the `task_id` input field.
    // See mika#596 / mika-skills#151.
    let task_id = input.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(err) = crate::tools::validate_task(&ctx.db, task_id).await {
        return ToolOutput::error(err);
    }

    // Dispatch-readiness guard (#525): stricter than validate_task() which also
    // allows `blocked` (needed by delegate_task). Long-running dispatch only permits
    // `pending` and `in_progress`. Returns the current status on success to avoid
    // a redundant DB read in the auto-transition below.
    let wi_status = match validate_dispatch_readiness(&ctx.db, task_id, github_token).await {
        Ok(status) => status,
        Err(err) => return ToolOutput::error(err),
    };

    // Per-turn dispatch cap (#583): only one long-running dispatch per agent turn.
    // Check first without incrementing — the actual increment happens right before
    // spawn to avoid leaving the counter stuck at 1 if create_task or path validation fails.
    if ctx.dispatch_count.load(Ordering::Relaxed) > 0 {
        return ToolOutput::error(
            serde_json::json!({
                "error": "dispatch_limit_exceeded",
                "task_id": task_id,
                "dispatches_this_turn": ctx.dispatch_count.load(Ordering::Relaxed),
                "reason": "Only one long-running dispatch is permitted per agent turn. \
                           A dispatch has already been launched in this turn. Wait for the \
                           current dispatch to complete via callback before launching another."
            })
            .to_string(),
        );
    }

    let estimated = estimated_duration_secs.unwrap_or(3600);
    let timeout_secs = (estimated * 3).clamp(600, 7_776_000); // 10min..90days

    // Auto-transition pending tasks to in_progress on dispatch (#525).
    // Closes the TOCTOU window where two dispatches to a pending item both pass.
    if wi_status == "pending" {
        if let Err(e) = ctx
            .db
            .update_manual_task_status(task_id, "in_progress")
            .await
        {
            // Non-fatal: the callback child creation provides a secondary guard
            warn!(
                task_id,
                error = %e,
                "failed to auto-transition task to in_progress"
            );
        } else {
            info!(
                task_id,
                "auto-transitioned task from pending to in_progress on dispatch"
            );
        }
    }

    // Link callback task to parent task via parent_task_id for task tree correlation.
    // Same canonical source as the validation above — agent passes via `task_id`.
    let parent_task_id = input
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let task = NewTask {
        agent_id: ctx.db.agent_id.clone(),
        team_run_id: None,
        parent_task_id,
        depth: 0,
        label: format!("long_running:{}", skill_tool.definition.name),
        trigger_type: trigger_type::CALLBACK.to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: Some(crate::timestamp::now_plus(chrono::Duration::seconds(
            timeout_secs as i64,
        ))),
        action_type: action_type::RESUME_AGENT.to_string(),
        action_config: "{}".to_string(),
        input_context: Some(serde_json::to_string(&input).unwrap_or_default()),
        created_by_session: Some(ctx.session_id.clone()),
        created_trace_id: Some(ctx.trace_id.clone()),
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
    };

    let task_id = match ctx.db.create_task(task).await {
        Ok(id) => id,
        Err(e) => {
            return ToolOutput::error(format!("Failed to create callback task: {e}"));
        }
    };

    let cmd_path = skill_tool.skill_dir.join(command);
    if !cmd_path.exists() {
        let _ = ctx
            .db
            .update_task_failed(
                &task_id,
                &format!("handler not found: {}", cmd_path.display()),
            )
            .await;
        return ToolOutput::error(format!(
            "handler command not found: {} (resolved to {})",
            command,
            cmd_path.display()
        ));
    }

    // Inject task metadata into input for the subprocess
    let mut enriched_input = input;
    if let serde_json::Value::Object(ref mut map) = enriched_input {
        map.insert(
            "__mika_task_id".to_string(),
            serde_json::Value::String(task_id.clone()),
        );
        map.insert(
            "__mika_agent".to_string(),
            serde_json::Value::String(ctx.agent_name.clone()),
        );
    }

    // Increment dispatch counter right before spawn — after all validation
    // and task creation succeeded. This ensures the counter stays at 0 if
    // any early error path returns before we actually launch the subprocess.
    ctx.dispatch_count.fetch_add(1, Ordering::Relaxed);

    spawn_long_running_exec(
        cmd_path,
        skill_tool.skill_dir.clone(),
        enriched_input,
        task_id.clone(),
        ctx.db.clone(),
        github_token.map(|s| s.to_string()),
    );

    ToolOutput::success(format!(
        "Task submitted (long-running). ID: {task_id}\n\
         The subprocess is running in the background. \
         Results will be delivered via callback when complete."
    ))
}

/// Spawn a monitored background task for a long-running exec handler.
///
/// The subprocess runs with `kill_on_drop(false)` so it survives if the
/// parent agent task ends. A monitor task records the PID and handles
/// failure (non-zero exit → task marked failed).
fn spawn_long_running_exec(
    cmd_path: PathBuf,
    skill_dir: PathBuf,
    input: serde_json::Value,
    task_id: String,
    db: AsyncDatabase,
    github_token: Option<String>,
) {
    tokio::spawn(async move {
        let mut cmd = tokio::process::Command::new(&cmd_path);
        cmd.current_dir(&skill_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .kill_on_drop(false);
        scrub_mika_env_vars(&mut cmd);
        if let Some(ref token) = github_token {
            cmd.env("GH_TOKEN", token);
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "failed to spawn long-running exec");
                if let Err(db_err) = db
                    .update_task_failed(&task_id, &format!("spawn failed: {e}"))
                    .await
                {
                    warn!(task_id = %task_id, error = %db_err, "failed to mark spawn-failed task in DB");
                }
                return;
            }
        };

        // Record PID
        if let Some(pid) = child.id()
            && let Err(e) = db.set_task_process_id(&task_id, Some(pid as i64)).await
        {
            warn!(task_id = %task_id, error = %e, "failed to record process ID for long-running task");
        }

        // Write input JSON to stdin
        if let Some(mut stdin) = child.stdin.take() {
            let input_bytes = serde_json::to_vec(&input).unwrap_or_default();
            match stdin.write_all(&input_bytes).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(e) => {
                    warn!(task_id = %task_id, error = %e, "failed to write stdin to long-running exec");
                }
            }
        }

        // Take stderr handle so we can read it (capped) on failure.
        let stderr_handle = child.stderr.take();

        // Wait for the subprocess to finish — only handle failure here.
        // On success, the script itself should call `mika ask --task-id`
        // to deliver results via the callback mechanism.
        let status = match child.wait().await {
            Ok(s) => s,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "failed to wait on long-running exec");
                match db
                    .update_task_failed(&task_id, &format!("wait failed: {e}"))
                    .await
                {
                    Ok(true) => {
                        warn!(task_id = %task_id, error = %e, "long-running exec wait failed")
                    }
                    Ok(false) => {
                        info!(task_id = %task_id, "long-running exec wait failed but task already in terminal state")
                    }
                    Err(db_err) => {
                        warn!(task_id = %task_id, error = %db_err, "failed to mark wait-failed task in DB")
                    }
                }
                return;
            }
        };

        if !status.success() {
            let mut stderr_text = String::new();
            if let Some(mut stderr) = stderr_handle {
                use tokio::io::AsyncReadExt;
                let mut buf = Vec::with_capacity(MAX_OUTPUT_LEN);
                AsyncReadExt::take(&mut stderr, MAX_OUTPUT_LEN as u64)
                    .read_to_end(&mut buf)
                    .await
                    .ok();
                stderr_text = String::from_utf8_lossy(&buf).to_string();
            }
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
            let err_msg = format!("Process {code_display}: {}", truncate_output(&stderr_text));
            match db.update_task_failed(&task_id, &err_msg).await {
                Ok(true) => warn!(task_id = %task_id, %code_display, "long-running exec failed"),
                Ok(false) => {
                    info!(task_id = %task_id, %code_display, "long-running exec exited but task already in terminal state")
                }
                Err(db_err) => {
                    warn!(task_id = %task_id, error = %db_err, "failed to mark long-running exec failure in DB")
                }
            }
        }
        // If success, the script called `mika ask --task-id` which completes the task.
        // If the script didn't call it, the task will eventually expire via timeout_at.
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::manifest::ToolHandler;
    use mika_common::claude::ToolDefinition;
    use std::fs;
    use std::path::PathBuf;

    /// Write a script file and make it executable, with fsync to avoid races.
    fn write_script(path: &std::path::Path, content: &str) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut writer = std::io::BufWriter::new(file);
        writer.write_all(content.as_bytes()).unwrap();
        let file = writer.into_inner().unwrap();
        file.sync_all().unwrap();
        drop(file); // Explicitly close before chmod
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn make_exec_tool(skill_dir: &std::path::Path, command: &str) -> ResolvedSkillTool {
        ResolvedSkillTool {
            definition: ToolDefinition {
                name: "test_tool".to_string(),
                description: "Test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: command.to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: skill_dir.to_path_buf(),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_success() {
        let tmp = tempfile::tempdir().unwrap();
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(
            &handler_dir.join("handler.sh"),
            "#!/bin/sh\necho 'hello from handler'",
        );

        let tool = make_exec_tool(tmp.path(), "handlers/handler.sh");
        let output =
            execute_skill_tool(&tool, serde_json::json!({"query": "test"}), 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("hello from handler"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_returns_output() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("fail.sh"),
            "#!/bin/sh\necho 'error msg' >&2\nexit 1",
        );

        let tool = make_exec_tool(tmp.path(), "fail.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        // Non-zero exit is NOT a tool error — the process ran to completion
        assert!(!output.is_error, "non-zero exit should not be is_error");
        assert!(
            output.content.contains("Exit code: 1"),
            "should contain exit code, got: {}",
            output.content
        );
        assert!(
            output.content.contains("error msg"),
            "should contain stderr output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_with_stdout() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("status.sh"),
            "#!/bin/sh\necho 'CRITICAL: disk usage 95%'\nexit 2",
        );

        let tool = make_exec_tool(tmp.path(), "status.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(
            !output.is_error,
            "non-zero exit should not be is_error, got: {}",
            output.content
        );
        assert!(
            output.content.contains("Exit code: 2"),
            "should contain exit code 2, got: {}",
            output.content
        );
        assert!(
            output.content.contains("CRITICAL: disk usage 95%"),
            "should contain stdout, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_empty_output() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("silent_fail.sh"), "#!/bin/sh\nexit 3");

        let tool = make_exec_tool(tmp.path(), "silent_fail.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error, "non-zero exit should not be is_error");
        assert!(
            output.content.contains("Exit code: 3"),
            "should contain exit code 3, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_via_run_sh() {
        // Regression test for the double problem: run.sh merges stderr into stdout
        // via 2>&1, so on non-zero exit the executor must read stdout (not stderr).
        let (_tmp, tool) = setup_shell_exec_handler();
        let input = serde_json::json!({"command": "echo 'health check output' && exit 2"});
        let output = execute_skill_tool(&tool, input, 30, None, None).await;
        assert!(
            !output.is_error,
            "non-zero exit via run.sh should not be is_error, got: {}",
            output.content
        );
        assert!(
            output.content.contains("Exit code: 2"),
            "should contain exit code 2, got: {}",
            output.content
        );
        assert!(
            output.content.contains("health check output"),
            "should contain stdout from run.sh, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_exit_zero_unchanged() {
        // Exit 0 should NOT have an exit code prefix
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("ok.sh"), "#!/bin/sh\necho 'all good'");

        let tool = make_exec_tool(tmp.path(), "ok.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error);
        assert!(
            !output.content.contains("Exit code:"),
            "exit 0 should not have exit code prefix, got: {}",
            output.content
        );
        assert!(output.content.contains("all good"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_nonzero_exit_stdout_and_stderr() {
        // When stdout and stderr have different content, both should appear
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("both.sh"),
            "#!/bin/sh\necho 'stdout line'\necho 'stderr line' >&2\nexit 1",
        );

        let tool = make_exec_tool(tmp.path(), "both.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error);
        assert!(output.content.contains("Exit code: 1"));
        assert!(
            output.content.contains("stdout line"),
            "should contain stdout, got: {}",
            output.content
        );
        assert!(
            output.content.contains("stderr line"),
            "should contain stderr, got: {}",
            output.content
        );
    }

    #[tokio::test]
    async fn test_exec_handler_missing_command() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = make_exec_tool(tmp.path(), "nonexistent.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(output.is_error);
        assert!(output.content.contains("not found"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("slow.sh"),
            "#!/bin/sh\nsleep 60\necho done",
        );

        let tool = make_exec_tool(tmp.path(), "slow.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 2, None, None).await;
        assert!(
            output.is_error,
            "expected timeout error, got: {}",
            output.content
        );
        assert!(output.content.contains("timed out"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_reads_stdin() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("echo_input.sh"), "#!/bin/sh\ncat");

        let tool = make_exec_tool(tmp.path(), "echo_input.sh");
        let input = serde_json::json!({"query": "hello world"});
        let output = execute_skill_tool(&tool, input.clone(), 30, None, None).await;
        assert!(!output.is_error);
        // The output should contain the JSON input
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed, input);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_command_with_quotes() {
        let (_tmp, tool) = setup_shell_exec_handler();
        let input = serde_json::json!({"command": "echo \"hello world\""});
        let output = execute_skill_tool(&tool, input, 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("hello world"),
            "expected 'hello world' in output, got: {}",
            output.content
        );
    }

    /// Helper to create a temp dir with the real shell-exec handler script.
    fn setup_shell_exec_handler() -> (tempfile::TempDir, ResolvedSkillTool) {
        let tmp = tempfile::tempdir().unwrap();
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(
            &handler_dir.join("run.sh"),
            include_str!("../../templates/skills/shell-exec/handlers/run.sh"),
        );
        let tool = make_exec_tool(tmp.path(), "handlers/run.sh");
        (tmp, tool)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_css_hash_chars() {
        let (_tmp, tool) = setup_shell_exec_handler();
        // CSS selectors and hex colors contain # which must survive JSON → jq → eval
        let input = serde_json::json!({"command": "echo '#custom-relay { color: #a6e3a1; }'"});
        let output = execute_skill_tool(&tool, input, 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("#custom-relay"),
            "expected CSS selector in output, got: {}",
            output.content
        );
        assert!(
            output.content.contains("#a6e3a1"),
            "expected hex color in output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_heredoc_multiline() {
        let (_tmp, tool) = setup_shell_exec_handler();
        // Multi-line command with heredoc: \n in JSON must become real newlines
        let input = serde_json::json!({
            "command": "cat << 'EOF'\n#selector { color: #fff; }\nEOF"
        });
        let output = execute_skill_tool(&tool, input, 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("#selector"),
            "expected CSS selector from heredoc, got: {}",
            output.content
        );
        assert!(
            output.content.contains("#fff"),
            "expected hex color from heredoc, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_sed_with_hash() {
        let (tmp, tool) = setup_shell_exec_handler();
        // Create a temp CSS file, then use sed to replace a selector with #
        let css_file = tmp.path().join("test.css");
        fs::write(&css_file, "#old-selector { color: red; }\n").unwrap();
        let cmd = format!(
            "sed 's/#old-selector/#new-selector/' '{}' && echo done",
            css_file.display()
        );
        let input = serde_json::json!({"command": cmd});
        let output = execute_skill_tool(&tool, input, 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("#new-selector"),
            "expected sed replacement with # in output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_backslash_in_printf() {
        let (_tmp, tool) = setup_shell_exec_handler();
        // printf with \n format specifiers: backslashes must survive JSON → jq → eval → printf
        let input = serde_json::json!({"command": "printf 'line1\\nline2\\n'"});
        let output = execute_skill_tool(&tool, input, 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("line1"),
            "expected line1 in output, got: {}",
            output.content
        );
        assert!(
            output.content.contains("line2"),
            "expected line2 in output, got: {}",
            output.content
        );
    }

    #[test]
    fn test_truncate_output() {
        assert_eq!(truncate_output("short"), "short");

        let long = "x".repeat(MAX_OUTPUT_LEN + 100);
        let truncated = truncate_output(&long);
        assert!(truncated.len() < long.len());
        assert!(truncated.contains("truncated"));
    }

    #[tokio::test]
    async fn test_http_handler_unsupported_method() {
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "test_tool".to_string(),
                description: "Test".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Http {
                url: "http://localhost:9999".to_string(),
                method: "DELETE".to_string(),
            },
            skill_dir: PathBuf::from("/tmp"),
        };
        let output = execute_skill_tool(&tool, serde_json::json!({}), 5, None, None).await;
        assert!(output.is_error);
        assert!(output.content.contains("unsupported HTTP method"));
    }

    // -- Envelope protocol tests --

    #[test]
    fn test_try_parse_envelope_valid() {
        let json = r#"{"__mika_v1": {"text": "Screenshot taken.", "images": ["/tmp/shot.png"]}}"#;
        let env = try_parse_envelope(json).unwrap();
        assert_eq!(env.text, "Screenshot taken.");
        assert_eq!(env.images, vec!["/tmp/shot.png"]);
    }

    #[test]
    fn test_try_parse_envelope_no_images() {
        let json = r#"{"__mika_v1": {"text": "Done."}}"#;
        let env = try_parse_envelope(json).unwrap();
        assert_eq!(env.text, "Done.");
        assert!(env.images.is_empty());
    }

    #[test]
    fn test_try_parse_envelope_plain_text() {
        assert!(try_parse_envelope("hello world").is_none());
    }

    #[test]
    fn test_try_parse_envelope_pretty_printed() {
        // jq without -c produces pretty-printed JSON — must still parse
        let json = "{\n  \"__mika_v1\": {\n    \"text\": \"Image file: /tmp/shot.png (image/png)\",\n    \"images\": [\n      \"/tmp/shot.png\"\n    ]\n  }\n}";
        let env = try_parse_envelope(json).unwrap();
        assert_eq!(env.text, "Image file: /tmp/shot.png (image/png)");
        assert_eq!(env.images, vec!["/tmp/shot.png"]);
    }

    #[test]
    fn test_try_parse_envelope_other_json() {
        // JSON without sentinel key — treated as plain text
        assert!(try_parse_envelope(r#"{"result": "ok"}"#).is_none());
    }

    #[test]
    fn test_try_parse_envelope_invalid_json() {
        assert!(try_parse_envelope("{invalid").is_none());
    }

    // -- Magic byte detection tests --

    #[test]
    fn test_detect_image_type_jpeg() {
        assert_eq!(
            detect_image_type(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_detect_image_type_png() {
        assert_eq!(
            detect_image_type(&[0x89, 0x50, 0x4E, 0x47, 0x0D]),
            Some("image/png")
        );
    }

    #[test]
    fn test_detect_image_type_gif() {
        assert_eq!(
            detect_image_type(&[0x47, 0x49, 0x46, 0x38, 0x39]),
            Some("image/gif")
        );
    }

    #[test]
    fn test_detect_image_type_webp() {
        let mut bytes = vec![0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00];
        bytes.extend_from_slice(&[0x57, 0x45, 0x42, 0x50]);
        assert_eq!(detect_image_type(&bytes), Some("image/webp"));
    }

    #[test]
    fn test_detect_image_type_unknown() {
        assert_eq!(detect_image_type(&[0x00, 0x01, 0x02, 0x03]), None);
    }

    #[test]
    fn test_detect_image_type_too_short() {
        assert_eq!(detect_image_type(&[0xFF, 0xD8]), None);
    }

    // -- Image file validation tests --

    #[tokio::test]
    async fn test_read_and_validate_image_nonexistent() {
        let err = read_and_validate_image("/tmp/nonexistent_abc123.png")
            .await
            .unwrap_err();
        assert!(err.contains("cannot resolve"));
    }

    #[tokio::test]
    async fn test_read_and_validate_image_not_image() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"this is plain text").unwrap();
        let err = read_and_validate_image(tmp.path().to_str().unwrap())
            .await
            .unwrap_err();
        assert!(err.contains("not a supported image type"));
    }

    #[tokio::test]
    async fn test_read_and_validate_image_valid_png() {
        let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        // Minimal valid PNG header + IHDR
        let png_bytes = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, // IHDR length
            0x49, 0x48, 0x44, 0x52, // IHDR
        ];
        std::fs::write(tmp.path(), png_bytes).unwrap();
        let img = read_and_validate_image(tmp.path().to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(img.media_type, "image/png");
        assert!(!img.data.is_empty());
    }

    #[tokio::test]
    async fn test_process_envelope_images_respects_max() {
        // Create 7 temp PNG files — only 5 should be processed
        let dir = tempfile::tempdir().unwrap();
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut paths = Vec::new();
        for i in 0..7 {
            let p = dir.path().join(format!("img{i}.png"));
            std::fs::write(&p, png_header).unwrap();
            paths.push(p.to_str().unwrap().to_string());
        }
        let (images, errors) = process_envelope_images(&paths).await;
        assert_eq!(images.len(), 5);
        assert!(!errors.is_empty());
        assert!(errors.last().unwrap().contains("skipped"));
    }

    // -- Exec handler with envelope integration test --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_with_image_envelope() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a fake PNG image
        let img_path = tmp.path().join("screenshot.png");
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        std::fs::write(&img_path, png_header).unwrap();

        // Script that outputs a Mika envelope
        let script = format!(
            "#!/bin/sh\nprintf '{{\"__mika_v1\":{{\"text\":\"Screenshot taken.\",\"images\":[\"{}\"]}}}}'\n",
            img_path.display()
        );
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(&handler_dir.join("screenshot.sh"), &script);

        let tool = make_exec_tool(tmp.path(), "handlers/screenshot.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("Screenshot taken."));
        assert_eq!(output.images.len(), 1);
        assert_eq!(output.images[0].media_type, "image/png");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_plain_text_backward_compat() {
        // Plain text output should still work as before (no envelope)
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("plain.sh"),
            "#!/bin/sh\necho 'just plain text'",
        );

        let tool = make_exec_tool(tmp.path(), "plain.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error);
        assert!(output.content.contains("just plain text"));
        assert!(output.images.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_strips_tmux_env() {
        let tmp = tempfile::tempdir().unwrap();
        // Script that prints the TMUX env var (empty if stripped)
        write_script(
            &tmp.path().join("check_env.sh"),
            "#!/bin/sh\nprintf 'TMUX=%s TMUX_PANE=%s' \"$TMUX\" \"$TMUX_PANE\"",
        );

        // Set TMUX in the current process environment
        // Safety: we're in a test with controlled env access
        unsafe {
            std::env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
            std::env::set_var("TMUX_PANE", "%0");
        }

        let tool = make_exec_tool(tmp.path(), "check_env.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        // Both vars should be empty because env_remove strips them
        assert_eq!(output.content.trim(), "TMUX= TMUX_PANE=");

        // Clean up env
        unsafe {
            std::env::remove_var("TMUX");
            std::env::remove_var("TMUX_PANE");
        }
    }

    // -- Long-running exec tests --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_missing_task_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("analyze.sh"), "#!/bin/sh\necho done");

        let tool = make_long_running_tool(tmp.path(), "analyze.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let ctx = LongRunningContext {
            db: async_db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"query": "test"}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        assert!(
            output.content.contains("create a task first"),
            "expected task error, got: {}",
            output.content
        );
    }

    use crate::test_utils::test_helpers::create_test_task;

    fn make_long_running_tool(skill_dir: &std::path::Path, command: &str) -> ResolvedSkillTool {
        ResolvedSkillTool {
            definition: ToolDefinition {
                name: "long_test".to_string(),
                description: "Long-running test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: command.to_string(),
                long_running: true,
                estimated_duration_secs: Some(60),
            },
            skill_dir: skill_dir.to_path_buf(),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_creates_callback_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("analyze.sh"), "#!/bin/sh\necho done");

        let tool = make_long_running_tool(tmp.path(), "analyze.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;
        let ctx = LongRunningContext {
            db: async_db.clone(),
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"query": "test", "task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("Task submitted"),
            "expected task submission message, got: {}",
            output.content
        );

        // Verify a callback task was created (2 tasks total: parent task + callback)
        let tasks = async_db
            .get_tasks_by_status(vec!["pending".to_string()])
            .await
            .unwrap();
        // Task is pending, callback task is also pending
        let callback_tasks: Vec<_> = tasks
            .iter()
            .filter(|t| t.trigger_type == "callback")
            .collect();
        assert_eq!(callback_tasks.len(), 1);
        assert_eq!(callback_tasks[0].action_type, "resume_agent");
        assert!(callback_tasks[0].label.starts_with("long_running:"));
        assert!(callback_tasks[0].timeout_at.is_some());
        assert_eq!(
            callback_tasks[0].parent_task_id.as_deref(),
            Some(wi_id.as_str()),
            "callback task should link to parent task via parent_task_id"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_false_blocks_normally() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("handler.sh"),
            "#!/bin/sh\necho 'sync result'",
        );

        // long_running: false — should execute synchronously even with LongRunningContext
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "sync_test".to_string(),
                description: "Sync test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: "handler.sh".to_string(),
                long_running: false,
                estimated_duration_secs: None,
            },
            skill_dir: tmp.path().to_path_buf(),
        };

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let ctx = LongRunningContext {
            db: async_db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
        };

        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, Some(&ctx), None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("sync result"),
            "expected sync output, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_long_running_failure_marks_task_failed() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("fail.sh"),
            "#!/bin/sh\necho 'error msg' >&2\nexit 1",
        );

        let tool = make_long_running_tool(tmp.path(), "fail.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;
        let ctx = LongRunningContext {
            db: async_db.clone(),
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
        };

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Should return success immediately (task submitted)
        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Wait briefly for the background monitor to detect the failure
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let tasks = async_db
            .get_tasks_by_status(vec!["failed".to_string()])
            .await
            .unwrap();
        assert_eq!(
            tasks.len(),
            1,
            "expected 1 failed task, got {}",
            tasks.len()
        );
        let result_text = tasks[0].result.as_ref().unwrap();
        assert!(
            result_text.contains("Exit code: 1"),
            "expected 'Exit code: 1' in result, got: {result_text}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_receives_gh_token() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("check_token.sh"),
            "#!/bin/sh\necho \"GH_TOKEN=$GH_TOKEN\"",
        );

        let tool = make_exec_tool(tmp.path(), "check_token.sh");

        // With github_token provided — should appear in child env
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({}),
            30,
            None,
            Some("ghp_test_token_123"),
        )
        .await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("GH_TOKEN=ghp_test_token_123"),
            "expected GH_TOKEN to be injected, got: {}",
            output.content
        );

        // Without github_token — GH_TOKEN should be absent (scrubbed)
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            !output.content.contains("GH_TOKEN=ghp_"),
            "expected GH_TOKEN to not contain a token when github_token is None, got: {}",
            output.content
        );
    }

    /// Regression test for #537: a long_running tool with no long_running_ctx
    /// must return an explicit error instead of silently falling through to
    /// the sync exec path (which lacks __mika_task_id injection).
    #[tokio::test]
    async fn test_long_running_tool_without_context_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = ResolvedSkillTool {
            definition: ToolDefinition {
                name: "run_claude_pilot".to_string(),
                description: "Long-running test tool".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
            },
            handler: ToolHandler::Exec {
                command: "./handlers/run.sh".to_string(),
                long_running: true,
                estimated_duration_secs: Some(300),
            },
            skill_dir: tmp.path().to_path_buf(),
        };

        // Pass None for long_running_ctx — simulates callback turn / silent mode / CLI test
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None, None).await;

        assert!(output.is_error, "expected error, got: {}", output.content);
        assert!(
            output.content.contains("run_claude_pilot"),
            "error should name the tool: {}",
            output.content
        );
        assert!(
            output.content.contains("long_running"),
            "error should mention long_running: {}",
            output.content
        );
        assert!(
            output.content.contains("cannot run in the current context"),
            "error should explain the context restriction: {}",
            output.content
        );
    }

    // -- Dispatch-readiness guard tests (#525) --

    /// Helper: create a task and transition it to the given status.
    async fn create_task_with_status(db: &crate::async_db::AsyncDatabase, status: &str) -> String {
        let wi_id = create_test_task(db).await;
        if status != "pending" {
            // pending -> blocked or in_progress are valid transitions;
            // pending -> completed or cancelled are also valid.
            db.update_manual_task_status(&wi_id, status).await.unwrap();
        }
        wi_id
    }

    /// Helper: create a callback child task under a parent task.
    async fn create_callback_child(
        db: &crate::async_db::AsyncDatabase,
        parent_task_id: &str,
        status: &str,
    ) -> String {
        use crate::task_engine::types::{action_type, trigger_type};
        let task = crate::db::NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: Some(parent_task_id.to_string()),
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: trigger_type::CALLBACK.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::RESUME_AGENT.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
        };
        let child_id = db.create_task(task).await.unwrap();
        if status != "pending" {
            // Transition the child to the requested status via direct DB update
            db.update_task_status(&child_id, status).await.unwrap();
        }
        child_id
    }

    fn make_lr_ctx(db: crate::async_db::AsyncDatabase) -> LongRunningContext {
        LongRunningContext {
            db,
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
            trace_id: "00000000000000000000000000000000".to_string(),
            dispatch_count: AtomicU32::new(0),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_blocked_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "blocked").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_not_dispatchable");
        assert_eq!(parsed["current_status"], "blocked");
        assert_eq!(parsed["task_id"], wi_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_completed_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "completed").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Caught by validate_task() (first-pass) — not a JSON error
        assert!(output.is_error);
        assert!(
            output.content.contains("not an active task"),
            "expected validate_task rejection, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_cancelled_task() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "cancelled").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Caught by validate_task() (first-pass)
        assert!(output.is_error);
        assert!(
            output.content.contains("not an active task"),
            "expected validate_task rejection, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_nonexistent_task_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": "00000000-0000-0000-0000-000000000000"}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        assert!(
            output.content.contains("task_not_found"),
            "expected task_not_found error for valid-format-but-nonexistent UUID, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_active_callback_child() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let child_id = create_callback_child(&async_db, &wi_id, "pending").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
        assert_eq!(parsed["active_child_id"], child_id);
        assert_eq!(parsed["task_id"], wi_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_rejects_in_progress_callback_child() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let _child_id = create_callback_child(&async_db, &wi_id, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_allows_with_only_completed_callback_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        // Create completed and failed callback children — should not block
        create_callback_child(&async_db, &wi_id, "completed").await;
        create_callback_child(&async_db, &wi_id, "failed").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Should proceed to dispatch — guard must not reject.
        // If is_error, verify it's NOT a dispatch guard rejection.
        if output.is_error {
            assert!(
                !output.content.contains("task_not_dispatchable")
                    && !output.content.contains("task_active_dispatch"),
                "dispatch guard should not reject, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_allows_cancelled_callback_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        // Cancelled callback child — should not block re-dispatch
        create_callback_child(&async_db, &wi_id, "cancelled").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        if output.is_error {
            assert!(
                !output.content.contains("task_not_dispatchable")
                    && !output.content.contains("task_active_dispatch"),
                "dispatch guard should not reject for cancelled child, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_ignores_non_callback_children() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;

        // Create a non-callback child (e.g., resume_agent with manual trigger)
        let non_callback = crate::db::NewTask {
            agent_id: async_db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: Some(wi_id.clone()),
            depth: 0,
            label: "delegate:some-agent".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
        };
        async_db.create_task(non_callback).await.unwrap();

        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Should proceed — non-callback children don't block
        if output.is_error {
            assert!(
                !output.content.contains("task_not_dispatchable")
                    && !output.content.contains("task_active_dispatch"),
                "dispatch guard should not reject for non-callback children, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_mixed_children_one_active_rejects() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        // One completed, one still pending — should block
        create_callback_child(&async_db, &wi_id, "completed").await;
        create_callback_child(&async_db, &wi_id, "pending").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_auto_transitions_pending_to_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;

        // Verify starts as pending
        let task_before = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(task_before.status, "pending");

        let ctx = make_lr_ctx(async_db.clone());

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Verify task transitioned to in_progress
        let task_after = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(
            task_after.status, "in_progress",
            "task should auto-transition to in_progress on dispatch"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_no_transition_for_already_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db.clone());

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(!output.is_error, "unexpected error: {}", output.content);

        // Should still be in_progress (no redundant transition)
        let task = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(task.status, "in_progress");
    }

    // -- Integration test: PR #522 race scenario replay (#525) --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pr522_replay_active_dispatch_with_pr_url() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Create in_progress task with PR URL in metadata
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let metadata = serde_json::json!({
            "claude_pilot": {
                "pr_url": "https://github.com/senara-solutions/mika/pull/522",
                "branch": "feat/522/some-feature"
            }
        });
        async_db
            .update_task_metadata(&wi_id, &metadata.to_string())
            .await
            .unwrap();

        // Simulate active claude-pilot session (pending callback child)
        create_callback_child(&async_db, &wi_id, "pending").await;

        // Count tasks before attempted dispatch
        let tasks_before = async_db.get_child_tasks(&wi_id).await.unwrap().len();

        let ctx = make_lr_ctx(async_db.clone());

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Should be rejected
        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
        assert_eq!(
            parsed["pr_url"],
            "https://github.com/senara-solutions/mika/pull/522"
        );

        // Verify no new callback task was created
        let tasks_after = async_db.get_child_tasks(&wi_id).await.unwrap().len();
        assert_eq!(
            tasks_before, tasks_after,
            "no new callback task should be created when dispatch is rejected"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pr522_replay_no_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi_id, "pending").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "task_active_dispatch");
        assert!(
            parsed["pr_url"].is_null(),
            "pr_url should be null when no metadata"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_pr522_replay_retry_after_child_completes() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_status(&async_db, "in_progress").await;
        let child_id = create_callback_child(&async_db, &wi_id, "pending").await;

        // First attempt: should be rejected
        let ctx = make_lr_ctx(async_db.clone());
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;
        assert!(output.is_error);

        // Complete the child task
        async_db
            .update_task_status(&child_id, "completed")
            .await
            .unwrap();

        // Retry: should succeed now
        let ctx = make_lr_ctx(async_db.clone());
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(
            !output.is_error,
            "retry after child completion should succeed, got: {}",
            output.content
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_dispatch_guard_double_dispatch_pending_item() {
        // Regression: two sequential dispatches to a pending task.
        // First should succeed (and auto-transition to in_progress + create callback).
        // Second should be rejected by active-child check.
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_test_task(&async_db).await;

        // First dispatch: should succeed
        let ctx = make_lr_ctx(async_db.clone());
        let output1 = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;
        assert!(
            !output1.is_error,
            "first dispatch should succeed: {}",
            output1.content
        );

        // Verify first dispatch created exactly one callback child
        let children = async_db.get_child_tasks(&wi_id).await.unwrap();
        let callback_children: Vec<_> = children
            .iter()
            .filter(|c| c.trigger_type == "callback")
            .collect();
        assert_eq!(
            callback_children.len(),
            1,
            "first dispatch must create exactly one callback child"
        );

        // Second dispatch: should be rejected (active callback child from first dispatch)
        let ctx = make_lr_ctx(async_db.clone());
        let output2 = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_id}),
            30,
            Some(&ctx),
            None,
        )
        .await;
        assert!(output2.is_error, "second dispatch should be rejected");
        let parsed: serde_json::Value = serde_json::from_str(&output2.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output2.content));
        assert_eq!(parsed["error"], "task_active_dispatch");

        // Verify task is in_progress (auto-transitioned by first dispatch)
        let task = async_db.get_task(&wi_id).await.unwrap().unwrap();
        assert_eq!(task.status, "in_progress");
    }

    // ---- Global dispatch guard tests (#583) ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_global_dispatch_guard_rejects_when_other_task_has_active_callback() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Create task A with an active callback child
        let wi_a = create_task_with_status(&async_db, "in_progress").await;
        let _callback_a = create_callback_child(&async_db, &wi_a, "pending").await;

        // Create task B — attempting to dispatch on this should be blocked
        let wi_b = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_b}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        assert!(output.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "global_dispatch_active");
        assert_eq!(parsed["blocking_task_id"], wi_a);
        assert_eq!(parsed["task_id"], wi_b);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_global_dispatch_guard_allows_when_no_other_active_callbacks() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // Task A has only completed callbacks
        let wi_a = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi_a, "completed").await;

        // Dispatch on task B should succeed
        let wi_b = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi_b}),
            30,
            Some(&ctx),
            None,
        )
        .await;

        // Should NOT be a global_dispatch_active error
        if output.is_error {
            assert!(
                !output.content.contains("global_dispatch_active"),
                "global dispatch guard should not reject when other callbacks are completed, got: {}",
                output.content
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_global_dispatch_guard_allows_same_task_callback() {
        // The global guard should NOT block dispatch on the same task —
        // that's already handled by the per-task guard.
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi, "pending").await;

        // Check the DB method directly — should return None since the only
        // active callback belongs to the excluded parent
        let result = async_db
            .has_active_callback_tasks_excluding(&wi)
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "should not find active callbacks for the same task"
        );
    }

    // ---- Per-turn dispatch counter tests (#583) ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_turn_dispatch_counter_rejects_second_dispatch() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(&tmp.path().join("run.sh"), "#!/bin/sh\necho done");
        let tool = make_long_running_tool(tmp.path(), "run.sh");

        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi = create_task_with_status(&async_db, "in_progress").await;
        let ctx = make_lr_ctx(async_db);

        // Simulate that a dispatch already happened this turn by setting the counter
        ctx.dispatch_count.store(1, Ordering::Relaxed);

        // Second dispatch should be rejected by the per-turn counter
        let output = execute_skill_tool(
            &tool,
            serde_json::json!({"task_id": wi}),
            30,
            Some(&ctx),
            None,
        )
        .await;
        assert!(output.is_error, "second dispatch should be rejected");
        let parsed: serde_json::Value = serde_json::from_str(&output.content)
            .unwrap_or_else(|_| panic!("expected JSON error, got: {}", output.content));
        assert_eq!(parsed["error"], "dispatch_limit_exceeded");
        // Counter should still be 1 (not incremented on rejection)
        assert_eq!(ctx.dispatch_count.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_turn_dispatch_counter_resets_with_new_context() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        // First context with counter at 1
        let ctx1 = make_lr_ctx(async_db.clone());
        ctx1.dispatch_count.store(1, Ordering::Relaxed);
        assert_eq!(ctx1.dispatch_count.load(Ordering::Relaxed), 1);

        // New context should start at 0
        let ctx2 = make_lr_ctx(async_db);
        assert_eq!(ctx2.dispatch_count.load(Ordering::Relaxed), 0);
    }

    // ---- DB method tests for has_active_callback_tasks_excluding (#583) ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_has_active_callback_excluding_returns_none_when_empty() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let result = async_db
            .has_active_callback_tasks_excluding("nonexistent")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_has_active_callback_excluding_ignores_terminal_states() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;
        create_callback_child(&async_db, &wi, "completed").await;
        create_callback_child(&async_db, &wi, "failed").await;
        create_callback_child(&async_db, &wi, "cancelled").await;

        let result = async_db
            .has_active_callback_tasks_excluding("other-task")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "should not detect terminal-state callbacks as active"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_has_active_callback_excluding_finds_active_for_different_parent() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");

        let wi = create_task_with_status(&async_db, "in_progress").await;
        let callback_id = create_callback_child(&async_db, &wi, "pending").await;

        let result = async_db
            .has_active_callback_tasks_excluding("different-parent")
            .await
            .unwrap();
        assert!(result.is_some());
        let (parent_id, found_callback_id) = result.unwrap();
        assert_eq!(parent_id, wi);
        assert_eq!(found_callback_id, callback_id);
    }

    // -- Blocked-by guard tests (#713) --

    /// Helper: create a task with `reference_url` set and transition to a given status.
    async fn create_task_with_ref_url(
        db: &crate::async_db::AsyncDatabase,
        status: &str,
        reference_url: Option<&str>,
    ) -> String {
        use crate::db::NewTask;
        use crate::task_engine::types::{action_type, trigger_type};

        let task = NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test task with ref".to_string(),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: reference_url.map(|u| u.to_string()),
            source: None,
            metadata: None,
            r#type: None,
        };
        let id = db.create_task(task).await.unwrap();
        if status != "pending" {
            db.update_manual_task_status(&id, status).await.unwrap();
        }
        id
    }

    #[tokio::test]
    async fn test_blocked_by_guard_skips_when_no_reference_url() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(&async_db, "in_progress", None).await;

        // No reference_url → blocked-by check skipped, dispatch proceeds
        let result = validate_dispatch_readiness(&async_db, &wi_id, Some("fake-token")).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn test_blocked_by_guard_skips_when_reference_is_pr() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(
            &async_db,
            "in_progress",
            Some("https://github.com/senara-solutions/mika/pull/100"),
        )
        .await;

        // PR reference → blocked-by check skipped (only issues have blockedBy)
        let result = validate_dispatch_readiness(&async_db, &wi_id, Some("fake-token")).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn test_blocked_by_guard_skips_when_no_github_token() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let async_db = crate::async_db::AsyncDatabase::new_with_agent(db, "mika");
        let wi_id = create_task_with_ref_url(
            &async_db,
            "in_progress",
            Some("https://github.com/senara-solutions/mika/issues/713"),
        )
        .await;

        // No token → blocked-by check skipped (fail-open), dispatch proceeds
        let result = validate_dispatch_readiness(&async_db, &wi_id, None).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }
}
