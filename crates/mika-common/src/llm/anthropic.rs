use anyhow::Result;
use async_trait::async_trait;
use tracing::warn;

use crate::claude::{
    ClaudeClient, ContentBlock, ImageSource, Message, MessageContent, MessagesRequest,
    ToolDefinition, ToolResultBlock, ToolResultBody,
};

use super::error::LlmError;
use super::types::*;
use super::LlmProvider;

/// Anthropic provider wrapping the existing `ClaudeClient`.
pub struct AnthropicProvider {
    client: ClaudeClient,
}

impl AnthropicProvider {
    pub fn new(api_key: Option<String>, model: String, max_tokens: u32) -> Result<Self> {
        let client = ClaudeClient::new(api_key, model, max_tokens)?;
        Ok(Self { client })
    }

    /// Create a dummy provider that cannot make API calls.
    pub fn dummy() -> Self {
        Self {
            client: ClaudeClient::dummy(),
        }
    }

    /// Access the underlying `ClaudeClient` for cases that need Anthropic-specific
    /// functionality (e.g., investigation panel SSE which uses its own request building).
    pub fn inner(&self) -> &ClaudeClient {
        &self.client
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        let anthropic_request = to_anthropic_request(request);

        let response = self
            .client
            .send_message(&anthropic_request)
            .await
            .map_err(|e| LlmError::ProviderError(e.to_string()))?;

        Ok(from_anthropic_response(response))
    }

    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn model_name(&self) -> &str {
        &self.client.model
    }

    fn max_tokens(&self) -> u32 {
        self.client.max_tokens
    }

    fn supports_tool_calling(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        true
    }

    fn supports_extended_thinking(&self) -> bool {
        true
    }

    async fn check_health(&self) -> Result<(), LlmError> {
        let request = MessagesRequest {
            model: self.client.model.clone(),
            max_tokens: 1,
            system: None,
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::Text("hi".to_string()),
            }],
            tools: None,
            thinking: None,
        };
        self.client
            .send_message(&request)
            .await
            .map_err(|e| LlmError::ProviderError(e.to_string()))?;
        Ok(())
    }
}

// -- Translation: LlmRequest → MessagesRequest --

fn to_anthropic_request(req: &LlmRequest) -> MessagesRequest {
    let messages = req.messages.iter().map(to_anthropic_message).collect();

    let tools = req.tools.as_ref().map(|tools| {
        tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect()
    });

    MessagesRequest {
        model: req.model.clone(),
        max_tokens: req.max_tokens,
        system: req.system.clone(),
        messages,
        tools,
        thinking: req.thinking.clone(),
    }
}

fn to_anthropic_message(msg: &LlmMessage) -> Message {
    match msg.role {
        LlmRole::User => Message {
            role: "user".to_string(),
            content: to_anthropic_content(&msg.content),
        },
        LlmRole::Assistant => Message {
            role: "assistant".to_string(),
            content: to_anthropic_content(&msg.content),
        },
        LlmRole::Tool => {
            // Anthropic uses role="user" with tool_result content blocks
            Message {
                role: "user".to_string(),
                content: to_anthropic_content(&msg.content),
            }
        }
    }
}

fn to_anthropic_content(content: &LlmContent) -> MessageContent {
    match content {
        LlmContent::Text(t) => MessageContent::Text(t.clone()),
        LlmContent::Blocks(blocks) => {
            let anthropic_blocks: Vec<ContentBlock> =
                blocks.iter().map(to_anthropic_block).collect();
            MessageContent::Blocks(anthropic_blocks)
        }
    }
}

fn to_anthropic_block(block: &LlmContentBlock) -> ContentBlock {
    match block {
        LlmContentBlock::Text(t) => ContentBlock::Text { text: t.clone() },
        LlmContentBlock::Image(img) => ContentBlock::Image {
            source: ImageSource {
                source_type: "base64".to_string(),
                media_type: img.media_type.clone(),
                data: img.data.clone(),
            },
        },
        LlmContentBlock::ToolCall {
            id,
            name,
            arguments,
        } => ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: arguments.clone(),
        },
        LlmContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => ContentBlock::ToolResult {
            tool_use_id: tool_call_id.clone(),
            content: to_anthropic_tool_result_body(content),
            is_error: if *is_error { Some(true) } else { None },
        },
    }
}

fn to_anthropic_tool_result_body(content: &LlmToolResultContent) -> ToolResultBody {
    match content {
        LlmToolResultContent::Text(t) => ToolResultBody::Text(t.clone()),
        LlmToolResultContent::Blocks(blocks) => {
            let result_blocks = blocks
                .iter()
                .map(|b| match b {
                    LlmToolResultBlock::Text(t) => ToolResultBlock::Text { text: t.clone() },
                    LlmToolResultBlock::Image(img) => ToolResultBlock::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: img.media_type.clone(),
                            data: img.data.clone(),
                        },
                    },
                })
                .collect();
            ToolResultBody::Blocks(result_blocks)
        }
    }
}

// -- Translation: MessagesResponse → LlmResponse --

fn from_anthropic_response(resp: crate::claude::MessagesResponse) -> LlmResponse {
    let reasoning = resp.thinking();

    let content: Vec<LlmResponseContent> = resp
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(LlmResponseContent::Text(text.clone())),
            ContentBlock::ToolUse { id, name, input } => Some(LlmResponseContent::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: input.clone(),
            }),
            ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => None,
            ContentBlock::ToolResult { .. } => {
                warn!("unexpected tool_result in assistant response");
                None
            }
        })
        .collect();

    let stop_reason = LlmStopReason::from(resp.stop_reason);
    let usage = LlmUsage::from(resp.usage);

    LlmResponse {
        content,
        reasoning,
        stop_reason,
        usage,
    }
}

// -- Translation: LlmResponse content back to LlmMessage (for building conversation history) --

/// Convert an `LlmResponse`'s content into `LlmContentBlock`s suitable for
/// pushing back as an assistant message in the conversation history.
pub fn response_content_to_blocks(content: &[LlmResponseContent]) -> Vec<LlmContentBlock> {
    content
        .iter()
        .map(|c| match c {
            LlmResponseContent::Text(t) => LlmContentBlock::Text(t.clone()),
            LlmResponseContent::ToolCall {
                id,
                name,
                arguments,
            } => LlmContentBlock::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::StopReason;
    use serde_json::json;

    #[test]
    fn test_to_anthropic_request_basic() {
        let req = LlmRequest {
            model: "claude-sonnet-4-6".into(),
            system: Some("You are Mika.".into()),
            messages: vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Text("Hello".into()),
            }],
            tools: None,
            max_tokens: 4096,
            thinking: None,
        };

        let anthropic = to_anthropic_request(&req);
        assert_eq!(anthropic.model, "claude-sonnet-4-6");
        assert_eq!(anthropic.max_tokens, 4096);
        assert_eq!(anthropic.system, Some("You are Mika.".into()));
        assert_eq!(anthropic.messages.len(), 1);
        assert_eq!(anthropic.messages[0].role, "user");
    }

    #[test]
    fn test_to_anthropic_request_with_tools() {
        let req = LlmRequest {
            model: "claude-sonnet-4-6".into(),
            system: None,
            messages: vec![],
            tools: Some(vec![LlmToolDefinition {
                name: "search".into(),
                description: "Search memory".into(),
                parameters: json!({ "type": "object" }),
            }]),
            max_tokens: 4096,
            thinking: None,
        };

        let anthropic = to_anthropic_request(&req);
        let tools = anthropic.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[0].input_schema, json!({ "type": "object" }));
    }

    #[test]
    fn test_to_anthropic_tool_result_message() {
        let msg = LlmMessage {
            role: LlmRole::Tool,
            content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                tool_call_id: "tc_1".into(),
                content: LlmToolResultContent::Text("result text".into()),
                is_error: false,
            }]),
        };

        let anthropic = to_anthropic_message(&msg);
        // Tool messages become user role in Anthropic
        assert_eq!(anthropic.role, "user");
        match anthropic.content {
            MessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 1);
                match &blocks[0] {
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        assert_eq!(tool_use_id, "tc_1");
                        assert!(matches!(content, ToolResultBody::Text(t) if t == "result text"));
                        assert!(is_error.is_none()); // false → None
                    }
                    _ => panic!("expected ToolResult"),
                }
            }
            _ => panic!("expected Blocks"),
        }
    }

    #[test]
    fn test_from_anthropic_response_text() {
        let resp = crate::claude::MessagesResponse {
            id: "msg_01".into(),
            content: vec![ContentBlock::Text {
                text: "Hello!".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: crate::claude::Usage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        };

        let llm = from_anthropic_response(resp);
        assert_eq!(llm.text(), "Hello!");
        assert_eq!(llm.stop_reason, LlmStopReason::EndTurn);
        assert!(!llm.has_tool_calls());
        assert_eq!(llm.usage.input_tokens, 10);
    }

    #[test]
    fn test_from_anthropic_response_tool_use() {
        let resp = crate::claude::MessagesResponse {
            id: "msg_02".into(),
            content: vec![
                ContentBlock::Text {
                    text: "Let me search.".into(),
                },
                ContentBlock::ToolUse {
                    id: "tu_1".into(),
                    name: "search_memory".into(),
                    input: json!({"query": "test"}),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage: crate::claude::Usage {
                input_tokens: 50,
                output_tokens: 30,
                cache_creation_input_tokens: Some(100),
                cache_read_input_tokens: None,
            },
        };

        let llm = from_anthropic_response(resp);
        assert_eq!(llm.stop_reason, LlmStopReason::ToolUse);
        assert!(llm.has_tool_calls());
        assert_eq!(llm.tool_calls().len(), 1);
        assert_eq!(llm.text(), "Let me search.");
        assert_eq!(llm.usage.cache_creation_input_tokens, Some(100));
    }

    #[test]
    fn test_from_anthropic_response_thinking() {
        let resp = crate::claude::MessagesResponse {
            id: "msg_03".into(),
            content: vec![
                ContentBlock::Thinking {
                    thinking: "Let me think...".into(),
                    signature: Some("sig".into()),
                },
                ContentBlock::Text {
                    text: "The answer is 42.".into(),
                },
            ],
            stop_reason: StopReason::EndTurn,
            usage: crate::claude::Usage {
                input_tokens: 100,
                output_tokens: 50,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        };

        let llm = from_anthropic_response(resp);
        assert_eq!(llm.text(), "The answer is 42.");
        assert_eq!(llm.reasoning, Some("Let me think...".into()));
    }

    #[test]
    fn test_response_content_to_blocks() {
        let content = vec![
            LlmResponseContent::Text("Hello".into()),
            LlmResponseContent::ToolCall {
                id: "tc_1".into(),
                name: "search".into(),
                arguments: json!({}),
            },
        ];
        let blocks = response_content_to_blocks(&content);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], LlmContentBlock::Text(t) if t == "Hello"));
        assert!(matches!(&blocks[1], LlmContentBlock::ToolCall { name, .. } if name == "search"));
    }

    #[test]
    fn test_image_translation() {
        let msg = LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Blocks(vec![
                LlmContentBlock::Image(LlmImage {
                    media_type: "image/png".into(),
                    data: "iVBOR...".into(),
                }),
                LlmContentBlock::Text("What's in this image?".into()),
            ]),
        };

        let anthropic = to_anthropic_message(&msg);
        match anthropic.content {
            MessageContent::Blocks(blocks) => {
                assert_eq!(blocks.len(), 2);
                match &blocks[0] {
                    ContentBlock::Image { source } => {
                        assert_eq!(source.source_type, "base64");
                        assert_eq!(source.media_type, "image/png");
                    }
                    _ => panic!("expected Image block"),
                }
            }
            _ => panic!("expected Blocks"),
        }
    }
}
