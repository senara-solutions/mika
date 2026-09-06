use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude::ThinkingConfig;

// -- Request types --

/// Provider-agnostic LLM request.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<LlmMessage>,
    pub tools: Option<Vec<LlmToolDefinition>>,
    pub max_tokens: u32,
    /// Extended thinking configuration (Claude-only; ignored by providers that
    /// don't support it).
    pub thinking: Option<ThinkingConfig>,
}

impl LlmRequest {
    /// Total payload size of this request in bytes (mika#2189 D5).
    ///
    /// # What it counts, and why that is the honest boundary
    ///
    /// The sum of every **caller-supplied byte** the request carries: the
    /// system prompt, every message's text and tool-call arguments and
    /// tool-result content, base64 image data, and each tool definition's
    /// name, description and JSON-schema. It deliberately does **not** count
    /// the JSON envelope a provider adapter will wrap around them — key names,
    /// braces, escaping — because that overhead is provider-specific and this
    /// number must stay comparable across the openrouter, Anthropic and Ollama
    /// rails that mika#2189's measurement spans.
    ///
    /// It is therefore a lower bound on the wire size, not the wire size. The
    /// axis it serves (does a bigger brief expire more often?) needs a
    /// *monotonic* measure, not an exact one — and a measure that changed
    /// meaning per provider would be worse than none.
    ///
    /// # Why walk the structure instead of serializing
    ///
    /// `LlmRequest` is not `Serialize`, and making it so purely to `to_string`
    /// it on **every** call would allocate a full copy of the largest object in
    /// the turn just to read its length — on the very path this ticket exists
    /// to stop overrunning its budget. This walk allocates nothing except for
    /// tool-call arguments, whose `serde_json::Value` has no length without
    /// rendering.
    pub fn payload_bytes(&self) -> usize {
        fn value_bytes(v: &Value) -> usize {
            // A `Value` has no length until it is rendered. Falling back to 0
            // on a serialization error would silently under-report a large
            // arguments blob; that failure is not reachable for the `Value`s
            // the tool layer produces, but the fallback is stated rather than
            // implied.
            serde_json::to_string(v).map(|s| s.len()).unwrap_or(0)
        }

        fn tool_result_bytes(c: &LlmToolResultContent) -> usize {
            match c {
                LlmToolResultContent::Text(t) => t.len(),
                LlmToolResultContent::Blocks(blocks) => blocks
                    .iter()
                    .map(|b| match b {
                        LlmToolResultBlock::Text(t) => t.len(),
                        LlmToolResultBlock::Image(img) => img.data.len() + img.media_type.len(),
                    })
                    .sum(),
            }
        }

        let system = self.system.as_ref().map_or(0, String::len);

        let messages: usize = self
            .messages
            .iter()
            .map(|m| match &m.content {
                LlmContent::Text(t) => t.len(),
                LlmContent::Blocks(blocks) => blocks
                    .iter()
                    .map(|b| match b {
                        LlmContentBlock::Text(t) => t.len(),
                        LlmContentBlock::Image(img) => img.data.len() + img.media_type.len(),
                        LlmContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                        } => id.len() + name.len() + value_bytes(arguments),
                        LlmContentBlock::ToolResult {
                            tool_call_id,
                            content,
                            ..
                        } => tool_call_id.len() + tool_result_bytes(content),
                    })
                    .sum(),
            })
            .sum();

        let tools: usize = self.tools.as_ref().map_or(0, |defs| {
            defs.iter()
                .map(|d| d.name.len() + d.description.len() + value_bytes(&d.parameters))
                .sum()
        });

        system + messages + tools
    }
}

/// A message in the LLM conversation.
#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: LlmContent,
}

/// Message role, provider-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmRole {
    User,
    Assistant,
    /// Tool result message (OpenAI uses `role: "tool"`;
    /// Anthropic uses `role: "user"` with `tool_result` content blocks).
    Tool,
}

/// Message content — either a plain string or a list of content blocks.
#[derive(Debug, Clone)]
pub enum LlmContent {
    Text(String),
    Blocks(Vec<LlmContentBlock>),
}

/// A single content block within a message.
#[derive(Debug, Clone)]
pub enum LlmContentBlock {
    Text(String),
    Image(LlmImage),
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        tool_call_id: String,
        content: LlmToolResultContent,
        is_error: bool,
    },
}

/// Content of a tool result — text or mixed blocks (text + images).
#[derive(Debug, Clone)]
pub enum LlmToolResultContent {
    Text(String),
    Blocks(Vec<LlmToolResultBlock>),
}

/// A block within a tool result (text or image).
#[derive(Debug, Clone)]
pub enum LlmToolResultBlock {
    Text(String),
    Image(LlmImage),
}

/// A base64-encoded image.
#[derive(Debug, Clone)]
pub struct LlmImage {
    pub media_type: String,
    pub data: String,
}

// -- Tool definition --

/// Provider-agnostic tool/function definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema describing the tool's parameters.
    pub parameters: Value,
}

// -- Response types --

/// Provider-agnostic LLM response.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<LlmResponseContent>,
    /// Extended thinking / reasoning text (provider-specific; `None` if unsupported).
    pub reasoning: Option<String>,
    pub stop_reason: LlmStopReason,
    pub usage: LlmUsage,
}

impl LlmResponse {
    /// Extract joined text from all text content blocks.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                LlmResponseContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Extended thinking / reasoning text, if the provider surfaced any.
    ///
    /// Populated from a dedicated `reasoning_content` field (reasoning-mode
    /// models on OpenAI-compatible APIs) or from `<think>…</think>` extraction;
    /// `None` when the provider surfaced no reasoning.
    pub fn reasoning(&self) -> Option<&str> {
        self.reasoning.as_deref()
    }

    /// Extract all tool call content blocks.
    pub fn tool_calls(&self) -> Vec<&LlmResponseContent> {
        self.content
            .iter()
            .filter(|c| matches!(c, LlmResponseContent::ToolCall { .. }))
            .collect()
    }

    /// Whether the response contains any tool calls.
    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|c| matches!(c, LlmResponseContent::ToolCall { .. }))
    }
}

/// A content element in the LLM response.
#[derive(Debug, Clone)]
pub enum LlmResponseContent {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
}

/// Why the LLM stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmStopReason {
    /// Natural end of response.
    EndTurn,
    /// Model wants to call one or more tools.
    ToolUse,
    /// Hit the max_tokens limit.
    MaxTokens,
    /// Content was filtered by safety systems.
    ContentFilter,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Anthropic-only: tokens used to create prompt cache.
    pub cache_creation_input_tokens: Option<u64>,
    /// Anthropic-only: tokens read from prompt cache.
    pub cache_read_input_tokens: Option<u64>,
}

// -- Conversions from/to existing claude.rs types --

impl From<crate::claude::ToolDefinition> for LlmToolDefinition {
    fn from(td: crate::claude::ToolDefinition) -> Self {
        Self {
            name: td.name,
            description: td.description,
            parameters: td.input_schema,
        }
    }
}

impl From<LlmToolDefinition> for crate::claude::ToolDefinition {
    fn from(ltd: LlmToolDefinition) -> Self {
        Self {
            name: ltd.name,
            description: ltd.description,
            input_schema: ltd.parameters,
        }
    }
}

impl From<crate::claude::ImageSource> for LlmImage {
    fn from(src: crate::claude::ImageSource) -> Self {
        Self {
            media_type: src.media_type,
            data: src.data,
        }
    }
}

impl From<LlmImage> for crate::claude::ImageSource {
    fn from(img: LlmImage) -> Self {
        Self {
            source_type: "base64".to_string(),
            media_type: img.media_type,
            data: img.data,
        }
    }
}

impl From<crate::claude::Usage> for LlmUsage {
    fn from(u: crate::claude::Usage) -> Self {
        Self {
            input_tokens: u64::from(u.input_tokens),
            output_tokens: u64::from(u.output_tokens),
            cache_creation_input_tokens: u.cache_creation_input_tokens.map(u64::from),
            cache_read_input_tokens: u.cache_read_input_tokens.map(u64::from),
        }
    }
}

impl From<crate::claude::StopReason> for LlmStopReason {
    fn from(sr: crate::claude::StopReason) -> Self {
        match sr {
            crate::claude::StopReason::EndTurn => LlmStopReason::EndTurn,
            crate::claude::StopReason::ToolUse => LlmStopReason::ToolUse,
            crate::claude::StopReason::MaxTokens => LlmStopReason::MaxTokens,
            crate::claude::StopReason::StopSequence => LlmStopReason::EndTurn,
        }
    }
}

// -- Helpers --

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

    /// Minimal request carrying only a system prompt and one user message.
    fn req(system: Option<&str>, messages: Vec<LlmMessage>) -> LlmRequest {
        LlmRequest {
            model: "m".into(),
            system: system.map(str::to_owned),
            messages,
            tools: None,
            max_tokens: 1024,
            thinking: None,
        }
    }

    fn user(text: &str) -> LlmMessage {
        LlmMessage {
            role: LlmRole::User,
            content: LlmContent::Text(text.into()),
        }
    }

    /// The core property (mika#2189 D5): the measure counts the caller-supplied
    /// payload and nothing else, so it is comparable across provider rails.
    #[test]
    fn payload_bytes_sums_system_and_messages() {
        assert_eq!(req(None, vec![]).payload_bytes(), 0);
        assert_eq!(req(Some("abcde"), vec![]).payload_bytes(), 5);
        assert_eq!(req(Some("abcde"), vec![user("xyz")]).payload_bytes(), 8);
        assert_eq!(
            req(None, vec![user("ab"), user("cde")]).payload_bytes(),
            5,
            "every message counts, not just the last"
        );
    }

    /// Multi-byte text is counted in **bytes**, not chars — the column is named
    /// `request_bytes` and a French-language brief must not read smaller than an
    /// English one of the same wire size.
    #[test]
    fn payload_bytes_counts_bytes_not_chars() {
        let r = req(None, vec![user("é")]);
        assert_eq!(r.payload_bytes(), 2);
    }

    /// Images are the largest thing a request can carry, and a request that
    /// carries one is exactly the kind that times out. Excluding them would make
    /// the size-vs-expiry axis blind to its most extreme cases.
    #[test]
    fn payload_bytes_counts_image_data() {
        let r = req(
            None,
            vec![LlmMessage {
                role: LlmRole::User,
                content: LlmContent::Blocks(vec![
                    LlmContentBlock::Text("hi".into()),
                    LlmContentBlock::Image(LlmImage {
                        media_type: "image/png".into(), // 9
                        data: "AAAA".into(),            // 4
                    }),
                ]),
            }],
        );
        assert_eq!(r.payload_bytes(), 2 + 9 + 4);
    }

    /// Tool definitions travel on every single call and dominate a small turn;
    /// omitting them would under-report the fixed cost each request pays.
    #[test]
    fn payload_bytes_counts_tool_definitions() {
        let schema = serde_json::json!({"type": "object"});
        let schema_len = serde_json::to_string(&schema).unwrap().len();
        let mut r = req(None, vec![]);
        r.tools = Some(vec![LlmToolDefinition {
            name: "search".into(),      // 6
            description: "Find".into(), // 4
            parameters: schema,
        }]);
        assert_eq!(r.payload_bytes(), 6 + 4 + schema_len);
    }

    /// Tool results are where a long agent turn actually grows — a turn at step
    /// 15 is mostly accumulated tool output, which is the mechanism by which a
    /// pass drifts over its plafond.
    #[test]
    fn payload_bytes_counts_tool_calls_and_results() {
        let args = serde_json::json!({"q": "x"});
        let args_len = serde_json::to_string(&args).unwrap().len();
        let r = req(
            None,
            vec![
                LlmMessage {
                    role: LlmRole::Assistant,
                    content: LlmContent::Blocks(vec![LlmContentBlock::ToolCall {
                        id: "id1".into(),      // 3
                        name: "search".into(), // 6
                        arguments: args,
                    }]),
                },
                LlmMessage {
                    role: LlmRole::Tool,
                    content: LlmContent::Blocks(vec![LlmContentBlock::ToolResult {
                        tool_call_id: "id1".into(),                           // 3
                        content: LlmToolResultContent::Text("result".into()), // 6
                        is_error: false,
                    }]),
                },
            ],
        );
        assert_eq!(r.payload_bytes(), 3 + 6 + args_len + 3 + 6);
    }

    /// Monotonicity is the property the axis actually rests on: a strictly
    /// larger brief must never measure smaller. Exactness is not claimed (the
    /// JSON envelope is excluded on purpose); ordering is.
    #[test]
    fn payload_bytes_is_monotonic_in_content() {
        let small = req(Some("sys"), vec![user("a")]);
        let large = req(Some("sys"), vec![user("a"), user("bbbbbbbbbb")]);
        assert!(large.payload_bytes() > small.payload_bytes());
    }

    #[test]
    fn test_tool_definition_roundtrip() {
        let claude_td = crate::claude::ToolDefinition {
            name: "search".into(),
            description: "Search memory".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        };

        let llm_td: LlmToolDefinition = claude_td.clone().into();
        assert_eq!(llm_td.name, "search");
        assert_eq!(llm_td.description, "Search memory");
        assert_eq!(llm_td.parameters, claude_td.input_schema);

        let back: crate::claude::ToolDefinition = llm_td.into();
        assert_eq!(back.name, claude_td.name);
        assert_eq!(back.input_schema, claude_td.input_schema);
    }

    #[test]
    fn test_image_roundtrip() {
        let src = crate::claude::ImageSource {
            source_type: "base64".into(),
            media_type: "image/png".into(),
            data: "iVBOR...".into(),
        };
        let img: LlmImage = src.into();
        assert_eq!(img.media_type, "image/png");
        assert_eq!(img.data, "iVBOR...");

        let back: crate::claude::ImageSource = img.into();
        assert_eq!(back.source_type, "base64");
        assert_eq!(back.media_type, "image/png");
    }

    #[test]
    fn test_usage_conversion() {
        let usage = crate::claude::Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: Some(200),
            cache_read_input_tokens: None,
        };
        let llm: LlmUsage = usage.into();
        assert_eq!(llm.input_tokens, 100);
        assert_eq!(llm.output_tokens, 50);
        assert_eq!(llm.cache_creation_input_tokens, Some(200));
        assert!(llm.cache_read_input_tokens.is_none());
    }

    #[test]
    fn test_stop_reason_conversion() {
        assert_eq!(
            LlmStopReason::from(crate::claude::StopReason::EndTurn),
            LlmStopReason::EndTurn
        );
        assert_eq!(
            LlmStopReason::from(crate::claude::StopReason::ToolUse),
            LlmStopReason::ToolUse
        );
        assert_eq!(
            LlmStopReason::from(crate::claude::StopReason::MaxTokens),
            LlmStopReason::MaxTokens
        );
        assert_eq!(
            LlmStopReason::from(crate::claude::StopReason::StopSequence),
            LlmStopReason::EndTurn
        );
    }

    #[test]
    fn test_llm_response_text() {
        let resp = LlmResponse {
            content: vec![
                LlmResponseContent::Text("Hello ".into()),
                LlmResponseContent::ToolCall {
                    id: "tc_1".into(),
                    name: "search".into(),
                    arguments: serde_json::json!({}),
                },
                LlmResponseContent::Text("world".into()),
            ],
            reasoning: None,
            stop_reason: LlmStopReason::EndTurn,
            usage: LlmUsage::default(),
        };
        assert_eq!(resp.text(), "Hello world");
        assert!(resp.has_tool_calls());
        assert_eq!(resp.tool_calls().len(), 1);
    }

    /// AC #4 (#984): Assert the Rust → provider tool-definition conversion
    /// preserves the `required` field in `input_schema`/`parameters`.
    ///
    /// The conversion is a direct copy (`parameters: td.input_schema`), but this
    /// test guards against a future refactor that might strip or transform the schema.
    #[test]
    fn test_tool_definition_conversion_preserves_required_field() {
        let claude_td = crate::claude::ToolDefinition {
            name: "run_claude_pilot".into(),
            description: "Dispatch claude-pilot session".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "enum": ["dev-pilot", "dev-groom"] },
                    "prompt": { "type": "string" },
                    "task_id": { "type": "string" }
                },
                "required": ["skill", "prompt", "task_id"]
            }),
        };

        // Convert to provider-agnostic LlmToolDefinition
        let llm_td: LlmToolDefinition = claude_td.clone().into();

        // Assert `required` is preserved in the `parameters` field
        let required = llm_td
            .parameters
            .get("required")
            .expect("'required' field missing after conversion to LlmToolDefinition");
        let required_arr = required
            .as_array()
            .expect("'required' is not an array after conversion");
        let required_strs: Vec<&str> = required_arr.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(
            required_strs,
            vec!["skill", "prompt", "task_id"],
            "required field values changed during conversion"
        );

        // Also test serialization round-trip (serde_json::to_value → from_value)
        let serialized = serde_json::to_value(&llm_td).unwrap();
        let params = serialized.get("parameters").unwrap();
        let ser_required = params
            .get("required")
            .expect("'required' missing from serialized LlmToolDefinition");
        assert_eq!(
            ser_required,
            &serde_json::json!(["skill", "prompt", "task_id"]),
            "required field lost during serialization"
        );

        // Round-trip back to ToolDefinition
        let back: crate::claude::ToolDefinition = llm_td.into();
        let back_required = back
            .input_schema
            .get("required")
            .expect("'required' missing after roundtrip back to ToolDefinition");
        assert_eq!(
            back_required,
            &serde_json::json!(["skill", "prompt", "task_id"]),
            "required field lost during roundtrip"
        );
    }
}
