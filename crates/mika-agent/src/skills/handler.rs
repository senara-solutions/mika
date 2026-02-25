use std::collections::HashMap;
use std::time::Duration;

use tracing::warn;

use crate::tools::ToolOutput;

use super::manifest::Handler;

/// Execute a tool call via a skill handler (exec or http).
///
/// Builtin handlers are dispatched through `ToolRegistry` in the agent loop,
/// so reaching this function with a Builtin handler is a logic error.
pub async fn execute_skill_tool(
    handler: &Handler,
    tool_name: &str,
    input: serde_json::Value,
    timeout: Duration,
) -> ToolOutput {
    match handler {
        Handler::Builtin { .. } => {
            warn!(tool = %tool_name, "execute_skill_tool called for builtin handler");
            ToolOutput::error(format!(
                "Internal error: builtin tool '{tool_name}' routed to skill handler"
            ))
        }
        Handler::Exec { command, args, .. } => {
            execute_exec(command, args, tool_name, &input, timeout).await
        }
        Handler::Http { url, headers, .. } => {
            execute_http(url, headers, tool_name, &input, timeout).await
        }
    }
}

async fn execute_exec(
    command: &str,
    args: &[String],
    tool_name: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> ToolOutput {
    let input_json = input.to_string();

    let result = tokio::time::timeout(timeout, async {
        tokio::process::Command::new(command)
            .args(args)
            .arg(tool_name)
            .env("MIKA_TOOL_INPUT", &input_json)
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if output.status.success() {
                ToolOutput::success(stdout)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let msg = if stderr.is_empty() {
                    format!("Command exited with {}", output.status)
                } else {
                    format!("Command failed: {stderr}")
                };
                ToolOutput::error(msg)
            }
        }
        Ok(Err(e)) => {
            warn!(command, tool = %tool_name, error = %e, "exec handler failed to spawn");
            ToolOutput::error(format!("Failed to execute command: {e}"))
        }
        Err(_) => {
            warn!(command, tool = %tool_name, timeout_secs = timeout.as_secs(), "exec handler timed out");
            ToolOutput::error(format!(
                "Tool '{tool_name}' timed out after {}s",
                timeout.as_secs()
            ))
        }
    }
}

async fn execute_http(
    url: &str,
    headers: &HashMap<String, String>,
    tool_name: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> ToolOutput {
    let client = reqwest::Client::new();
    let mut request = client.post(url).timeout(timeout).json(&serde_json::json!({
        "tool_name": tool_name,
        "input": input,
    }));

    for (key, value) in headers {
        request = request.header(key, value);
    }

    match request.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                ToolOutput::success(body)
            } else {
                ToolOutput::error(format!("HTTP {status}: {body}"))
            }
        }
        Err(e) => {
            if e.is_timeout() {
                ToolOutput::error(format!(
                    "Tool '{tool_name}' timed out after {}s",
                    timeout.as_secs()
                ))
            } else {
                warn!(url, tool = %tool_name, error = %e, "http handler failed");
                ToolOutput::error(format!("HTTP request failed: {e}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exec_echo_script() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("echo_tool.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"result: $1 $MIKA_TOOL_INPUT\"").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let handler = Handler::Exec {
            command: script.to_string_lossy().to_string(),
            args: vec![],
            tools: vec!["test_tool".to_string()],
        };

        let output = execute_skill_tool(
            &handler,
            "test_tool",
            serde_json::json!({"key": "val"}),
            Duration::from_secs(5),
        )
        .await;

        assert!(!output.is_error);
        assert!(output.content.contains("test_tool"));
    }

    #[tokio::test]
    async fn test_exec_nonzero_exit() {
        let handler = Handler::Exec {
            command: "false".to_string(),
            args: vec![],
            tools: vec!["fail_tool".to_string()],
        };

        let output = execute_skill_tool(
            &handler,
            "fail_tool",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;

        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_exec_timeout() {
        // Use sh -c so the tool_name arg appended by execute_exec doesn't break the sleep
        let handler = Handler::Exec {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), "sleep 10".to_string()],
            tools: vec!["slow_tool".to_string()],
        };

        let output = execute_skill_tool(
            &handler,
            "slow_tool",
            serde_json::json!({}),
            Duration::from_millis(100),
        )
        .await;

        assert!(output.is_error);
        assert!(output.content.contains("timed out"));
    }

    #[tokio::test]
    async fn test_http_unreachable() {
        let handler = Handler::Http {
            url: "http://127.0.0.1:1".to_string(),
            tools: vec!["remote_tool".to_string()],
            headers: HashMap::new(),
        };

        let output = execute_skill_tool(
            &handler,
            "remote_tool",
            serde_json::json!({}),
            Duration::from_secs(2),
        )
        .await;

        assert!(output.is_error);
    }

    #[tokio::test]
    async fn test_builtin_returns_error() {
        let handler = Handler::Builtin {
            tools: vec!["store_fact".to_string()],
        };

        let output = execute_skill_tool(
            &handler,
            "store_fact",
            serde_json::json!({}),
            Duration::from_secs(5),
        )
        .await;

        assert!(output.is_error);
        assert!(output.content.contains("builtin"));
    }
}
