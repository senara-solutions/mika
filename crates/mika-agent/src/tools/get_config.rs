use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct GetConfigTool;

#[async_trait]
impl Tool for GetConfigTool {
    fn name(&self) -> &str {
        "get_config"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_config".to_string(),
            description: "Read customer configuration settings. Returns all configured values (e.g. timezone, chat_id).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let configs = ctx.db.list_customer_config().await?;

        if configs.is_empty() {
            return Ok(ToolOutput::success("No configuration values set."));
        }

        let mut output = String::from("Customer configuration:\n");
        for (key, value) in &configs {
            output.push_str(&format!("  {key} = {value}\n"));
        }

        Ok(ToolOutput::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_get_config_empty() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetConfigTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No configuration values set"));
    }

    #[tokio::test]
    async fn test_get_config_returns_entries() {
        let harness = TestHarness::new();
        harness
            .db
            .set_customer_config("timezone", "Asia/Singapore")
            .await
            .unwrap();
        harness
            .db
            .set_customer_config("chat_id", "123456789")
            .await
            .unwrap();

        let ctx = harness.ctx();
        let tool = GetConfigTool;

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("timezone = Asia/Singapore"));
        assert!(result.content.contains("chat_id = 123456789"));
    }
}
