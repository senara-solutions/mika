//! `get_active_llm` builtin tool (mika#1815).
//!
//! Returns the runtime-active LLM provider and model powering the current
//! agent turn. Ground truth is `ToolContext.provider_name` /
//! `ToolContext.model_name`, populated at turn-start from the live
//! `LlmProvider` instance — the same source that populates the system
//! prompt's `## Runtime` section, so the two channels cannot drift.
//!
//! Purpose: when a user asks "which model / LLM are you using?" and the
//! agent is genuinely asked to "go check" (verb: VERIFY, not INFER), this
//! tool returns the value verbatim instead of forcing the agent to
//! confabulate one from commented-out config lines or defaults. Fabricated
//! model names are the founding incident for mika#1815 (Al testeur,
//! 2026-07-20).
//!
//! Read-only classification is enforced in
//! `crate::tools::classification::is_read_tool`, and the tool name is
//! registered in `BUILTIN_TOOL_NAMES` so the parity test
//! `test_every_builtin_tool_has_explicit_classification` covers it.

use anyhow::Result;
use async_trait::async_trait;
use mika_common::claude::ToolDefinition;
use serde_json::Value;

use super::{Tool, ToolContext, ToolOutput};

pub struct GetActiveLlmTool;

#[async_trait]
impl Tool for GetActiveLlmTool {
    fn name(&self) -> &str {
        "get_active_llm"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_active_llm".to_string(),
            description:
                "Return the runtime-active LLM provider and model powering this agent turn. \
                 Use this when the user asks about your model / LLM / provider and you want to \
                 VERIFY (not INFER). The system prompt's `## Runtime` section carries the same \
                 information — this tool is the on-demand verifier when a user explicitly asks \
                 you to 'go check'. Never fabricate a model name from commented-out config lines \
                 or defaults; either quote this tool's output verbatim or say you cannot reliably \
                 determine your model."
                    .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        Ok(ToolOutput::success(format!(
            "Active LLM: provider=`{}`, model=`{}` (runtime source: live LlmProvider instance).",
            ctx.provider_name, ctx.model_name,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::test_helpers::TestHarness;

    #[tokio::test]
    async fn returns_provider_and_model_from_context() {
        // TestHarness ctx defaults to provider=anthropic / model=claude-sonnet-4-6.
        let harness = TestHarness::new();
        let ctx = harness.ctx();
        let tool = GetActiveLlmTool;

        let out = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!out.is_error);
        assert!(
            out.content.contains("provider=`anthropic`"),
            "expected provider verbatim in output, got: {}",
            out.content
        );
        assert!(
            out.content.contains("model=`claude-sonnet-4-6`"),
            "expected model verbatim in output, got: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn returns_ground_truth_for_zai_glm() {
        // Founding-incident fixture: Mika actually runs on GLM z.ai but
        // confabulated Anthropic. Prove the tool returns the ground truth
        // when the runtime is zai/glm-5.2.
        let harness = TestHarness::new();
        let ctx = harness.ctx_with_llm("zai", "glm-5.2");
        let tool = GetActiveLlmTool;

        let out = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(!out.is_error);
        assert!(
            out.content.contains("provider=`zai`"),
            "expected provider=`zai` verbatim, got: {}",
            out.content
        );
        assert!(
            out.content.contains("model=`glm-5.2`"),
            "expected model=`glm-5.2` verbatim, got: {}",
            out.content
        );
        // Anti-fabrication cross-check: the tool must NOT invent an
        // Anthropic answer just because that's the training-data mode.
        assert!(
            !out.content.contains("anthropic"),
            "output must not contain the wrong provider, got: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn tool_definition_lists_verify_verb() {
        // The Self-Identity Discipline section keys off "VERIFY" as the
        // verb that means read-the-source. The tool description must call
        // it out so the LLM binds "va voir" / "check" to this tool.
        let tool = GetActiveLlmTool;
        let def = tool.definition();
        assert_eq!(def.name, "get_active_llm");
        assert!(
            def.description.contains("VERIFY"),
            "tool description must anchor the VERIFY verb, got: {}",
            def.description
        );
        assert!(
            def.description.contains("Runtime"),
            "tool description must reference the ## Runtime prompt section, got: {}",
            def.description
        );
    }
}
