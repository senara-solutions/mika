use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use super::index::ResolvedSkillTool;
use super::manifest::ToolHandler;
use crate::tools::{ImageData, ToolOutput};

/// Maximum output size from a skill tool (10,000 characters).
const MAX_OUTPUT_LEN: usize = 10_000;

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

/// Execute a skill tool with the appropriate handler.
///
/// Applies a per-skill timeout wrapping the inner execution.
pub async fn execute_skill_tool(
    skill_tool: &ResolvedSkillTool,
    input: serde_json::Value,
    timeout_secs: u64,
) -> ToolOutput {
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
        ToolHandler::Exec { command } => {
            execute_exec(command, &skill_tool.skill_dir, &skill_tool.definition.name, input).await
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

    let mut child = tokio::process::Command::new(&cmd_path)
        .current_dir(skill_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .kill_on_drop(true)
        .spawn()?;

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
        tracing::debug!(
            tool = %tool_name,
            stdout_len = stdout.len(),
            stdout_preview = %&stdout[..stdout.len().min(200)],
            "skill exec succeeded"
        );
        if !stderr.trim().is_empty() {
            tracing::debug!(
                tool = %tool_name,
                stderr = %&stderr[..stderr.len().min(500)],
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
        format!("{}\n... (truncated at {MAX_OUTPUT_LEN} chars)", &s[..boundary])
    }
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
        writer.into_inner().unwrap().sync_all().unwrap();
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
        let output = execute_skill_tool(&tool, serde_json::json!({"query": "test"}), 30).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30).await;
        assert!(output.is_error);
        assert!(output.content.contains("exit"));
        assert!(output.content.contains("error msg"));
    }

    #[tokio::test]
    async fn test_exec_handler_missing_command() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = make_exec_tool(tmp.path(), "nonexistent.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 2).await;
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
        let output = execute_skill_tool(&tool, input.clone(), 30).await;
        assert!(!output.is_error);
        // The output should contain the JSON input
        let parsed: serde_json::Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(parsed, input);
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 5).await;
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
        std::fs::write(tmp.path(), &png_bytes).unwrap();
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
            std::fs::write(&p, &png_header).unwrap();
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
        std::fs::write(&img_path, &png_header).unwrap();

        // Script that outputs a Mika envelope
        let script = format!(
            "#!/bin/sh\nprintf '{{\"__mika_v1\":{{\"text\":\"Screenshot taken.\",\"images\":[\"{}\"]}}}}'\n",
            img_path.display()
        );
        let handler_dir = tmp.path().join("handlers");
        fs::create_dir_all(&handler_dir).unwrap();
        write_script(&handler_dir.join("screenshot.sh"), &script);

        let tool = make_exec_tool(tmp.path(), "handlers/screenshot.sh");
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30).await;
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
        let output = execute_skill_tool(&tool, serde_json::json!({}), 30).await;
        assert!(!output.is_error, "unexpected error: {}", output.content);
        // Both vars should be empty because env_remove strips them
        assert_eq!(output.content.trim(), "TMUX= TMUX_PANE=");

        // Clean up env
        unsafe {
            std::env::remove_var("TMUX");
            std::env::remove_var("TMUX_PANE");
        }
    }
}
