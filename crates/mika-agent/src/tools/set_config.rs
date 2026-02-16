use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{MAX_INPUT_LEN, Tool, ToolContext, ToolOutput};
use crate::config_keys::{
    SETTABLE_CONFIG_KEYS, is_settable_key, settable_keys_display, validate_config_value,
};

pub struct SetConfigTool;

#[async_trait]
impl Tool for SetConfigTool {
    fn name(&self) -> &str {
        "set_config"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "set_config".to_string(),
            description: format!(
                "Set a customer configuration value. Valid keys: {}.",
                settable_keys_display()
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Configuration key to set",
                        "enum": serde_json::Value::Array(
                            SETTABLE_CONFIG_KEYS.iter()
                                .map(|k| serde_json::Value::String(k.to_string()))
                                .collect()
                        )
                    },
                    "value": {
                        "type": "string",
                        "description": "Value to set for the key (e.g. 'Asia/Singapore' for timezone, '123456789' for chat_id, 'low'/'medium'/'high'/'off' for thinking_level)"
                    }
                },
                "required": ["key", "value"]
            }),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        let key = input["key"].as_str().unwrap_or("");
        let value = input["value"].as_str().unwrap_or("");

        // Validate non-empty
        if key.is_empty() {
            return Ok(ToolOutput::error("'key' is required."));
        }
        if value.is_empty() {
            return Ok(ToolOutput::error("'value' is required."));
        }

        // Validate input length
        if key.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'key' too long: {} characters (max: {MAX_INPUT_LEN})",
                key.len()
            )));
        }
        if value.len() > MAX_INPUT_LEN {
            return Ok(ToolOutput::error(format!(
                "'value' too long: {} characters (max: {MAX_INPUT_LEN})",
                value.len()
            )));
        }

        // Validate key is in the allowlist
        if !is_settable_key(key) {
            return Ok(ToolOutput::error(format!(
                "Unknown config key: '{key}'. Valid keys: {}",
                settable_keys_display()
            )));
        }

        // Validate value per key type
        if let Err(msg) = validate_config_value(key, value) {
            return Ok(ToolOutput::error(msg));
        }

        ctx.db.set_customer_config(key, value).await?;

        Ok(ToolOutput::success(format!("Set {key} = {value}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn test_set_config_timezone() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "key": "timezone",
                    "value": "Asia/Singapore"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Set timezone = Asia/Singapore"));

        // Verify it was persisted
        let stored = harness
            .db
            .get_customer_config("timezone")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, "Asia/Singapore");
    }

    #[tokio::test]
    async fn test_set_config_chat_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "key": "chat_id",
                    "value": "-987654321"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Set chat_id = -987654321"));

        let stored = harness
            .db
            .get_customer_config("chat_id")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, "-987654321");
    }

    #[tokio::test]
    async fn test_set_config_invalid_key() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "key": "api_key",
                    "value": "sk-secret"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Unknown config key"));
    }

    #[tokio::test]
    async fn test_set_config_invalid_timezone() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "key": "timezone",
                    "value": "Not/A/Timezone"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid timezone"));
    }

    #[tokio::test]
    async fn test_set_config_invalid_chat_id() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "key": "chat_id",
                    "value": "not-a-number"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Invalid chat_id"));
    }

    #[tokio::test]
    async fn test_set_config_missing_key() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "value": "Asia/Singapore"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'key' is required"));
    }

    #[tokio::test]
    async fn test_set_config_missing_value() {
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = SetConfigTool;

        let result = tool
            .execute(
                serde_json::json!({
                    "key": "timezone"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("'value' is required"));
    }
}
