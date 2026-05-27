# Plan: Native Ollama Provider — Tool Calling Support

**Ticket:** mika#1305
**Type:** feature (enhancement to existing provider)
**Target:** `crates/mika-common/src/llm/ollama.rs`
**Reference:** `crates/mika-common/src/llm/openai.rs` (tool calling pattern)

## Problem

The native ollama provider (`OllamaProvider`) shipped in mika#1292 returns `supports_tool_calling() = false` and drops tools from requests. When Mika runs on the native ollama provider, the agent loop injects tool definitions as text in the system prompt. Weak local models then regurgitate the tool schemas as plain text, breaking the assistant UX entirely.

Ollama's `/api/chat` endpoint supports tool calling natively. The provider should serialize tools into requests, parse `tool_calls` from responses, and handle `role: "tool"` messages for tool results.

## Ollama Tool Calling Wire Format

Key differences from OpenAI format (confirmed from Ollama API docs):

### Request — tools field
Same shape as OpenAI: `{"type": "function", "function": {"name", "description", "parameters"}}`.

### Response — tool_calls in message
Similar to OpenAI but:
- **No `id` field** on tool calls (OpenAI has `id: "call_xxx"`)
- **`arguments` is a JSON object** (not a JSON string like OpenAI)

```json
{
  "message": {
    "role": "assistant",
    "content": "",
    "tool_calls": [{ "function": { "name": "get_weather", "arguments": {"city": "Tokyo"} } }]
  }
}
```

### Tool results — role: "tool"
- No `tool_call_id` (unlike OpenAI)

```json
{"role": "tool", "content": "result text"}
```

## Implementation Steps

### Step 1: Add Ollama tool wire types

Add new structs to `ollama.rs` for the tool-related wire format:

```rust
/// Tool definition sent in the request (same shape as OpenAI).
#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OllamaFunctionDef,
}

#[derive(Serialize)]
struct OllamaFunctionDef {
    name: String,
    description: String,
    parameters: Value,
}

/// Tool call returned in the response message.
#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Debug, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: Value, // Ollama returns a JSON object, not a string
}
```

### Step 2: Add tools field to OllamaChatRequest

```rust
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    options: OllamaOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaTool>>,
}
```

### Step 3: Add tool_calls field to OllamaResponseMessage

```rust
#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    role: String,
    content: String,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}
```

### Step 4: Update OllamaMessage to support tool results

The `OllamaMessage` needs to handle three roles (system, user/assistant, tool). Currently it's a flat struct with `role` + `content`. Extend it:

```rust
#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
    // Tool calls from assistant messages (echoed back in conversation history)
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaToolCall>>,
}
```

**Note:** `OllamaToolCall` needs both `Serialize` and `Deserialize` since it appears in both request (echoed assistant messages) and response.

### Step 5: Convert LlmToolDefinition → OllamaTool in to_ollama_request()

In `to_ollama_request()`, convert `request.tools` to `Option<Vec<OllamaTool>>`:

```rust
let tools = request.tools.as_ref().map(|tools| {
    tools.iter().map(|t| OllamaTool {
        tool_type: "function".into(),
        function: OllamaFunctionDef {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.clone(),
        },
    }).collect()
});
```

Set the `tools` field on `OllamaChatRequest`.

### Step 6: Handle assistant messages with tool calls in to_ollama_request()

Update the `LlmRole::Assistant` arm in `to_ollama_request()` to extract `LlmContentBlock::ToolCall` blocks and serialize them as `OllamaToolCall` on the message's `tool_calls` field — same pattern as `openai.rs` lines 519–565.

### Step 7: Handle LlmRole::Tool messages in to_ollama_request()

Replace the current `LlmRole::Tool` warning-and-skip with actual conversion. For each `ToolResult` block in the message content, emit an `OllamaMessage` with `role: "tool"`:

```rust
LlmRole::Tool => {
    match &msg.content {
        LlmContent::Blocks(blocks) => {
            for block in blocks {
                if let LlmContentBlock::ToolResult { content, .. } = block {
                    let text = match content {
                        LlmToolResultContent::Text(t) => t.clone(),
                        LlmToolResultContent::Blocks(parts) => {
                            parts.iter().filter_map(|p| match p {
                                LlmToolResultBlock::Text(t) => Some(t.as_str()),
                                _ => None,
                            }).collect::<Vec<_>>().join("")
                        }
                    };
                    messages.push(OllamaMessage {
                        role: "tool".into(),
                        content: text,
                        tool_calls: None,
                    });
                }
            }
        }
        LlmContent::Text(t) => {
            messages.push(OllamaMessage {
                role: "tool".into(),
                content: t.clone(),
                tool_calls: None,
            });
        }
    }
}
```

### Step 8: Parse tool_calls from response in from_ollama_response()

Update `from_ollama_response()` to check `response.message.tool_calls` and convert them to `LlmResponseContent::ToolCall`:

```rust
// After existing text/reasoning extraction...
if let Some(tool_calls) = response.message.tool_calls {
    for tc in tool_calls {
        // Generate a synthetic ID since Ollama doesn't provide one.
        // The agent loop uses tool_call IDs to correlate results;
        // a unique-enough ID per response is sufficient.
        let id = format!("ollama_tc_{}", uuid_or_counter);
        content.push(LlmResponseContent::ToolCall {
            id,
            name: tc.function.name,
            arguments: tc.function.arguments, // Already a Value (not a string)
        });
    }
}
```

**ID generation:** Ollama doesn't return tool call IDs. The agent loop needs IDs to correlate `ToolResult` blocks back. Options:
- Use `uuid::Uuid::new_v4()` — `uuid` is already a dependency of `mika-common`
- Use a simple counter format: `ollama_tc_{index}`

Prefer `format!("ollama_tc_{index}")` (zero-indexed per response) for determinism in tests. The agent loop only needs uniqueness within a single response turn — cross-turn uniqueness isn't required since tool results are correlated by position in the same turn.

**Stop reason:** When tool_calls are present, set `stop_reason` to `LlmStopReason::ToolUse` instead of `EndTurn`.

### Step 9: Flip supports_tool_calling() to true

```rust
fn supports_tool_calling(&self) -> bool {
    true
}
```

### Step 10: Update tests

1. **Remove `test_to_ollama_request_tools_ignored`** — this test asserts the old behavior (tools dropped).
2. **Add `test_to_ollama_request_with_tools`** — verify tools are serialized in the request with correct `type: "function"` shape.
3. **Add `test_to_ollama_request_assistant_with_tool_calls`** — verify assistant messages with `ToolCall` blocks produce `tool_calls` field on the serialized message.
4. **Add `test_to_ollama_request_tool_result_messages`** — verify `LlmRole::Tool` messages with `ToolResult` blocks produce `role: "tool"` messages.
5. **Add `test_from_ollama_response_tool_calls`** — deserialize a response with `tool_calls` and verify `LlmResponseContent::ToolCall` items with correct names, arguments, and synthetic IDs.
6. **Add `test_from_ollama_response_tool_calls_stop_reason`** — verify `stop_reason` is `ToolUse` when tool_calls are present.
7. **Add `test_from_ollama_response_tool_calls_with_text`** — verify mixed text + tool_calls response produces both `Text` and `ToolCall` content.
8. **Update `test_provider_capabilities`** — flip `supports_tool_calling()` assertion to `true`.
9. **Update `test_to_ollama_request_blocks_flatten_to_text`** — keep this for user/assistant text blocks (still valid).

### Step 11: Update doc comment on OllamaProvider

Remove "Does not support tool calling" from the doc comment on the `OllamaProvider` struct.

## Non-goals

- **Vision support** — remains deferred (separate ticket scope).
- **Model-capability detection** — the provider sends `tools` unconditionally. Models that don't support tool calling simply won't emit `tool_calls` in the response, and the agent loop handles that gracefully (same as OpenAI provider behavior).
- **Streaming** — remains `stream: false` (existing provider contract).
- **XML tool call extraction** — the `extract_xml_tool_calls()` fallback in `openai.rs` is not needed here. If ollama models emit XML-style tool calls as text instead of structured `tool_calls`, that's a model-level issue outside this provider's scope. The existing `detect_text_based_tool_call()` defense-in-depth in `agent.rs` handles this at the agent loop level.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/llm/ollama.rs` | All implementation: wire types, request/response conversion, capability flag, tests |

## Risk Assessment

**Low risk.** Single-file change, well-bounded by the existing `LlmProvider` trait contract. The OpenAI provider's tool calling implementation is a proven reference pattern. The main novelty is Ollama's missing `id` field on tool calls (synthetic ID generation) and `arguments` being a JSON object instead of a string (simpler than OpenAI — no parsing needed).

**Backward compatibility:** No breaking changes. The provider previously didn't support tools; now it does. The agent loop already handles both tool-calling and non-tool-calling providers via the `supports_tool_calling()` gate.
