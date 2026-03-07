use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use super::index::ResolvedSkillTool;
use super::manifest::ToolHandler;
use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use crate::task_engine::types::{action_type, trigger_type};
use crate::tools::{ImageData, ToolOutput};

/// Maximum output size from a skill tool (10,000 characters).
const MAX_OUTPUT_LEN: usize = 10_000;

/// Scrub all `MIKA_*` environment variables from a tokio Command (defense-in-depth).
///
/// Prevents leaking secrets like `MIKA_ANTHROPIC_API_KEY`, `MIKA_INTERNAL_TOKEN`,
/// and `MIKA_OPENAI_API_KEY` to child processes.
fn scrub_mika_env_vars(cmd: &mut tokio::process::Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
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
) -> ToolOutput {
    // Check for long-running exec handler
    if let ToolHandler::Exec {
        command,
        long_running: true,
        estimated_duration_secs,
    } = &skill_tool.handler
        && let Some(ctx) = long_running_ctx
    {
        return execute_long_running(skill_tool, command, input, *estimated_duration_secs, ctx)
            .await;
    }

    let timeout = Duration::from_secs(timeout_secs);
    match tokio::time::timeout(timeout, execute_inner(skill_tool, input)).await {
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
/// - Returns stdout on success, stderr + exit code on failure
/// - Detects `__mika_v1` envelope for image-bearing results
async fn execute_exec(
    command: &str,
    skill_dir: &std::path::Path,
    tool_name: &str,
    input: serde_json::Value,
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

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // Log successful output for debugging silent failures
        // Use char-boundary-safe slicing to avoid panics on multi-byte UTF-8
        let stdout_end = {
            let mut b = stdout.len().min(200);
            while b > 0 && !stdout.is_char_boundary(b) {
                b -= 1;
            }
            b
        };
        tracing::debug!(
            tool = %tool_name,
            stdout_len = stdout.len(),
            stdout_preview = %&stdout[..stdout_end],
            "skill exec succeeded"
        );
        if !stderr.trim().is_empty() {
            let stderr_end = {
                let mut b = stderr.len().min(500);
                while b > 0 && !stderr.is_char_boundary(b) {
                    b -= 1;
                }
                b
            };
            tracing::debug!(
                tool = %tool_name,
                stderr = %&stderr[..stderr_end],
                "skill exec stderr on success"
            );
        }

        // Try to parse as __mika_v1 image envelope
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        let exit_code = output.status.code().unwrap_or(-1);
        Ok(ToolOutput::error(format!(
            "Process exited with code {exit_code}: {}",
            truncate_output(&stderr)
        )))
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

/// Execute a long-running exec handler by creating a callback task and spawning
/// the subprocess in the background. Returns immediately with the task ID.
async fn execute_long_running(
    skill_tool: &ResolvedSkillTool,
    command: &str,
    input: serde_json::Value,
    estimated_duration_secs: Option<u64>,
    ctx: &LongRunningContext,
) -> ToolOutput {
    let estimated = estimated_duration_secs.unwrap_or(3600);
    let timeout_secs = (estimated * 3).clamp(600, 7_776_000); // 10min..90days
    let now = chrono::Utc::now().timestamp();

    let task = NewTask {
        agent_id: ctx.db.agent_id.clone(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: format!("long_running:{}", skill_tool.definition.name),
        trigger_type: trigger_type::CALLBACK.to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: Some(now + timeout_secs as i64),
        action_type: action_type::RESUME_AGENT.to_string(),
        action_config: "{}".to_string(),
        input_context: Some(serde_json::to_string(&input).unwrap_or_default()),
        created_by_session: Some(ctx.session_id.clone()),
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

    spawn_long_running_exec(
        cmd_path,
        skill_tool.skill_dir.clone(),
        enriched_input,
        task_id.clone(),
        ctx.db.clone(),
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
                if let Err(db_err) = db
                    .update_task_failed(&task_id, &format!("wait failed: {e}"))
                    .await
                {
                    warn!(task_id = %task_id, error = %db_err, "failed to mark wait-failed task in DB");
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
            let exit_code = status.code().unwrap_or(-1);
            let err_msg = format!(
                "Process exited with code {exit_code}: {}",
                truncate_output(&stderr_text)
            );
            warn!(task_id = %task_id, exit_code, "long-running exec failed");
            if let Err(db_err) = db.update_task_failed(&task_id, &err_msg).await {
                warn!(task_id = %task_id, error = %db_err, "failed to mark long-running exec failure in DB");
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
            execute_skill_tool(&tool, serde_json::json!({"query": "test"}), 30, None).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(output.content.contains("hello from handler"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_failure() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            &tmp.path().join("fail.sh"),
            "#!/bin/sh\necho 'error msg' >&2\nexit 1",
        );

        let tool = make_exec_tool(tmp.path(), "fail.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None).await;
        assert!(output.is_error);
        assert!(output.content.contains("exit"));
        assert!(output.content.contains("error msg"));
    }

    #[tokio::test]
    async fn test_exec_handler_missing_command() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = make_exec_tool(tmp.path(), "nonexistent.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 2, None).await;
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
        let output = execute_skill_tool(&tool, input.clone(), 30, None).await;
        assert!(!output.is_error);
        // The output should contain the JSON input
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed, input);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_exec_handler_command_with_quotes() {
        let (_tmp, tool) = setup_shell_exec_handler();
        let input = serde_json::json!({"command": "echo \"hello world\""});
        let output = execute_skill_tool(&tool, input, 30, None).await;
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
        let output = execute_skill_tool(&tool, input, 30, None).await;
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
        let output = execute_skill_tool(&tool, input, 30, None).await;
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
        let output = execute_skill_tool(&tool, input, 30, None).await;
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
        let output = execute_skill_tool(&tool, input, 30, None).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 5, None).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, None).await;
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
        let ctx = LongRunningContext {
            db: async_db.clone(),
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
        };

        let output =
            execute_skill_tool(&tool, serde_json::json!({"query": "test"}), 30, Some(&ctx)).await;

        assert!(!output.is_error, "unexpected error: {}", output.content);
        assert!(
            output.content.contains("Task submitted"),
            "expected task submission message, got: {}",
            output.content
        );

        // Verify a callback task was created
        let tasks = async_db
            .get_tasks_by_status(vec!["pending".to_string()])
            .await
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].trigger_type, "callback");
        assert_eq!(tasks[0].action_type, "resume_agent");
        assert!(tasks[0].label.starts_with("long_running:"));
        assert!(tasks[0].timeout_at.is_some());
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
        };

        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, Some(&ctx)).await;
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
        let ctx = LongRunningContext {
            db: async_db.clone(),
            agent_name: "mika".to_string(),
            session_id: "test-session".to_string(),
        };

        let output = execute_skill_tool(&tool, serde_json::json!({}), 30, Some(&ctx)).await;

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
        assert!(tasks[0].result.as_ref().unwrap().contains("exit"));
    }
}
