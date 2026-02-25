use anyhow::Result;
use mika_common::claude::ToolDefinition;
use std::path::Path;

/// Load the prompt snippet for a skill (`system_prompt.md`).
/// Returns an empty string if the file doesn't exist.
pub async fn load_prompt_snippet(skill_dir: &Path) -> String {
    let path = skill_dir.join("system_prompt.md");
    tokio::fs::read_to_string(&path).await.unwrap_or_default()
}

/// Load external tool definitions from `tools.json`.
/// Returns an empty vec if the file doesn't exist.
pub async fn load_tool_definitions(skill_dir: &Path) -> Result<Vec<ToolDefinition>> {
    let path = skill_dir.join("tools.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => {
            let defs: Vec<ToolDefinition> = serde_json::from_str(&content)?;
            Ok(defs)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_prompt_snippet() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("system_prompt.md"),
            "Use memory tools wisely.",
        )
        .unwrap();

        let snippet = load_prompt_snippet(tmp.path()).await;
        assert_eq!(snippet, "Use memory tools wisely.");
    }

    #[tokio::test]
    async fn test_load_prompt_snippet_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let snippet = load_prompt_snippet(tmp.path()).await;
        assert_eq!(snippet, "");
    }

    #[tokio::test]
    async fn test_load_tool_definitions() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("tools.json"),
            r#"[{
                "name": "get_weather",
                "description": "Get current weather",
                "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}
            }]"#,
        )
        .unwrap();

        let defs = load_tool_definitions(tmp.path()).await.unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "get_weather");
    }

    #[tokio::test]
    async fn test_load_tool_definitions_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let defs = load_tool_definitions(tmp.path()).await.unwrap();
        assert!(defs.is_empty());
    }
}
