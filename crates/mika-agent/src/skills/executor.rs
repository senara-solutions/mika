use std::time::Duration;

use anyhow::{Result, bail};
use tokio::io::AsyncWriteExt;
use tracing::warn;

use super::index::ResolvedSkillTool;
use super::manifest::ToolHandler;
use crate::tools::ToolOutput;

/// Maximum output size from a skill tool (10,000 characters).
const MAX_OUTPUT_LEN: usize = 10_000;

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
    match &skill_tool.handler {
        ToolHandler::Exec { command } => execute_exec(command, &skill_tool.skill_dir, input).await,
        ToolHandler::Http { url, method } => execute_http(url, method, input).await,
    }
}

/// Execute an exec-type handler by spawning a subprocess.
///
/// - Resolves the command path relative to the skill directory
/// - Pipes input JSON to stdin
/// - Returns stdout on success, stderr + exit code on failure
async fn execute_exec(
    command: &str,
    skill_dir: &std::path::Path,
    input: serde_json::Value,
) -> Result<ToolOutput> {
    // Resolve command relative to skill directory
    let cmd_path = skill_dir.join(command);
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
        Ok(ToolOutput::success(truncate_output(&stdout)))
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
        let truncated = &s[..MAX_OUTPUT_LEN];
        format!("{truncated}\n... (truncated at {MAX_OUTPUT_LEN} chars)")
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
}
