# Plan: Native Ollama Provider (mika#1292)

## Problem

`ProviderKind::Ollama` currently routes to `OpenAiCompatibleProvider`, which hits ollama's OpenAI-compatible endpoint at `/v1/chat/completions`. The naming is misleading — operators configuring `base_url = "http://my-ollama:11434"` (without `/v1`) get a 404. The default base URL bakes in `/v1`, masking the contract. A real operator hit this on 2026-05-26.

## Goal

Replace the OpenAI-compat shim with a native ollama provider that uses `/api/chat` (ollama's native endpoint), handles ollama's response shape directly, and requires no `/v1` suffix in the base URL.

## Design Decisions

1. **No new `ProviderKind` variant.** `ProviderKind::Ollama` stays. The current OpenAI-compat-via-ollama use case is already covered by `ProviderKind::OpenAi` with a custom `base_url` — operators who want that path configure `openai` provider with `openai_base_url = "http://localhost:11434/v1"`. No need for a `ProviderKind::OpenAICompatible` — every non-Anthropic provider already IS OpenAI-compatible.

2. **Default base URL changes.** `ProviderKind::Ollama.default_base_url()` changes from `http://localhost:11434/v1` to `http://localhost:11434`. This is a **breaking change** for operators who relied on the implicit `/v1` — but since the whole point is that the implicit `/v1` was the bug, this is the fix.

3. **No streaming.** Ollama supports JSONL streaming, but `LlmProvider` trait has no streaming method. `stream: false` in all requests. Matches existing OpenAI-compat behavior.

4. **No tool calling in v1.** Ollama's native tool calling is model-dependent and has a different format from OpenAI. Ticket explicitly defers this. `supports_tool_calling()` returns `false`. The XML tool-call extraction fallback in the agent loop will still work as defense-in-depth for models that emit XML-formatted tool calls in text.

5. **No vision in v1.** Ollama supports multimodal models but the native API image format differs. Defer. `supports_vision()` returns `false`.

6. **Health check uses `/api/tags`.** Already proven in `models.rs` — the same endpoint used for model listing. Simpler and more reliable than a dummy chat completion.

7. **Model listing already works.** `models.rs` already has ollama-specific `/api/tags` handling with `/v1` stripping. After the default base URL change, the stripping logic becomes a no-op but remains harmless for operators who explicitly set a `/v1` base URL.

## Implementation Steps

### Step 1: Create `ollama.rs` provider module

**File:** `crates/mika-common/src/llm/ollama.rs`

Create a new provider implementing `LlmProvider` with:

**Struct:**
```rust
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_tokens: u32,
    log_llm_bodies: bool,
}
```

**Request types (serde):**
```rust
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,           // always false
    options: OllamaOptions, // num_predict maps to max_tokens
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,   // "system", "user", "assistant"
    content: String,
}

#[derive(Serialize)]
struct OllamaOptions {
    num_predict: u32,  // ollama's equivalent of max_tokens
}
```

**Response types (serde):**
```rust
#[derive(Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: OllamaResponseMessage,
    done: bool,
    total_duration: Option<u64>,   // nanoseconds
    eval_count: Option<u64>,       // output tokens
    prompt_eval_count: Option<u64>, // input tokens
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    role: String,
    content: String,
}
```

**Key methods:**

- `chat_url()` → `format!("{}/api/chat", self.base_url)`
- `tags_url()` → `format!("{}/api/tags", self.base_url)` (for health check)
- `to_ollama_request(&self, request: &LlmRequest) -> OllamaChatRequest` — converts provider-agnostic request to ollama format. System message goes as first message with `role: "system"`. Tool definitions are ignored (no tool calling support). `LlmContent::Blocks` with tool results/images: the agent loop gates on `supports_tool_calling()` at `agent.rs:2614` — when it returns `false`, `tools_for_request` is set to `None`, the LLM never sees tool definitions, and never produces tool calls or tool results. Therefore `LlmContent::Blocks` containing tool results are unreachable in normal operation. As defense-in-depth, if such blocks are encountered (e.g., via a manually-constructed `LlmRequest` in tests), flatten to text with a `warn!` log — this is a contract violation signal, not silent degradation (review-guide.md § Single Responsibility — contract violations surface as warnings, not silent fallbacks).
- `from_ollama_response(response: OllamaChatResponse) -> LlmResponse` — maps ollama's flat response to `LlmResponse`. `stop_reason` is always `EndTurn` (no tool use). Token counts from `eval_count`/`prompt_eval_count` map to `LlmUsage`. Extract `<think>...</think>` blocks into `reasoning` field — reuse `extract_think_block()` from `openai.rs:790-808` (already handles DeepSeek-R1, MiniMax thinking models via the OpenAI-compat path at `openai.rs:769-773`). Import as `use super::openai::extract_think_block;` (function is currently `fn` — promote to `pub(crate) fn` to enable cross-module reuse without duplication; review-guide.md § DRY).
- `send_once()` — single HTTP POST to `/api/chat` with JSON body, parse response. No auth header (ollama typically runs unauthenticated; include `Authorization: Bearer` only if `api_key` is `Some`).

**Retry logic:** Mirror the `OpenAiCompatibleProvider` pattern — 3 retries with exponential backoff (500ms, 1s, 2s). Since ollama is local, retries mainly cover transient model-loading delays (ollama loads models on first request; can timeout).

**Deadline-aware retry:** Override `send_message_with_deadline()` with the same budget-check pattern as `OpenAiCompatibleProvider` (`openai.rs:268`, `openai.rs:316`) — abort retry when remaining time < `TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS`. These constants are defined as `pub const` in `mod.rs:23-25` (shared module scope, not `openai.rs`-local) and imported via `use super::{TYPICAL_CALL_DURATION_SECS, RETRY_BUFFER_SECS}` (same pattern as `openai.rs:134`). The shared 120s budget (90s + 30s) is acceptable for ollama v1: ollama's cold-start model-loading delay is bounded by the per-request 120s `reqwest` timeout, and the deadline budget check is a retry-abort heuristic, not a per-call timeout. If local-LLM latency proves materially different in practice, a follow-up can define `OllamaProvider`-specific constants — but the shared values are correct for the initial implementation (review-guide.md § DRY — shared constants in shared module, no duplication).

**Trait implementation:**
- `provider_name()` → `"ollama"`
- `model_name()` → `&self.model`
- `max_tokens()` → `self.max_tokens`
- `supports_tool_calling()` → `false`
- `supports_vision()` → `false`
- `supports_extended_thinking()` → `false`
- `check_health()` → GET `/api/tags`, check for 200 OK

**Error mapping:** Ollama returns errors as `{"error": "message"}`. Map to `LlmError::ProviderError` for known errors, `LlmError::HttpError` for HTTP-level failures. HTTP 500 is retryable (model loading).

### Step 2: Update `mod.rs` — register module and update factory

**File:** `crates/mika-common/src/llm/mod.rs`

1. Add `pub mod ollama;` to module declarations (line ~6, after `pub mod openai;`).

2. Update `create_provider()` (`mod.rs:380`) to route `ProviderKind::Ollama` to the new provider. The current match structure (`mod.rs:385-396`) is `ProviderKind::Anthropic => { ... }` + `_ => { OpenAI-compat }`. The `ProviderKind` enum (`mod.rs:235-251`) has 11 variants: `Anthropic`, `OpenAi`, `OpenRouter`, `Groq`, `Ollama`, `Mistral`, `Google`, `DeepSeek`, `MiniMax`, `Kimi`, `Qwen`. After this change, the match becomes:
```rust
match spec.provider {
    ProviderKind::Anthropic => { /* existing, mod.rs:386-395 */ }
    ProviderKind::Ollama => {
        let base_url = spec.effective_base_url().ok_or_else(|| {
            anyhow::anyhow!("base URL is required for ollama provider")
        })?;
        let provider = ollama::OllamaProvider::new(
            base_url,
            spec.api_key.clone(),
            spec.model.clone(),
            max_tokens,
            log_llm_bodies,
        );
        Ok(Arc::new(provider))
    }
    _ => { /* existing OpenAI-compat fallback — covers OpenAi, OpenRouter, Groq, Mistral, Google, DeepSeek, MiniMax, Kimi, Qwen (9 variants) */ }
}
```
The `_ =>` arm remains correct: no `ProviderKind` variant has its own dedicated arm other than `Anthropic` and the new `Ollama`. All 9 remaining variants route to `OpenAiCompatibleProvider` via the catchall — no unreachable patterns, no shadowing (review-guide.md § Orthogonality — factory match verified exhaustive against the 11-variant enum).

3. Update `ProviderKind::Ollama.default_base_url()` from `http://localhost:11434/v1` to `http://localhost:11434`.

### Step 3: Update model listing

**File:** `crates/mika-common/src/llm/models.rs`

The existing ollama-specific `/api/tags` handling strips `/v1` from the base URL to derive the root URL. After the default changes to `http://localhost:11434`, the stripping becomes a no-op. No functional change needed, but simplify the comment to reflect the new default. The `strip_suffix("/v1")` fallback should remain for operators who override `ollama_base_url` with a `/v1` suffix.

### Step 4: Unit tests

**File:** `crates/mika-common/src/llm/ollama.rs` (inline `#[cfg(test)] mod tests`)

1. **Request conversion test:** Verify `to_ollama_request()` correctly maps `LlmRequest` → `OllamaChatRequest`:
   - System message → first message with `role: "system"`
   - User/assistant messages in order
   - `max_tokens` → `options.num_predict`
   - `stream: false`
   - Tools ignored

2. **Response conversion test:** Verify `from_ollama_response()` correctly maps:
   - `message.content` → `LlmResponseContent::Text`
   - `eval_count` / `prompt_eval_count` → `LlmUsage`
   - `<think>` block extraction → `reasoning`
   - Missing token counts → zeros

3. **Error parsing test:** Verify `{"error": "..."}` → `LlmError::ProviderError`.

4. **URL construction test:** Verify `chat_url()` produces `http://localhost:11434/api/chat`.

### Step 5: Documentation updates

1. **`crates/mika-common/CLAUDE.md`** — Update the LLM Providers section to note that Ollama now uses a native provider (not OpenAI-compatible adapter). Provider count stays at 11.

2. **Root `CLAUDE.md`** — Update `MIKA_OLLAMA_API_KEY` comment to note it's usually not needed (ollama runs unauthenticated locally). Already says this, no change needed.

3. **`.env.example`** — Verified: `.env.example` does NOT contain `MIKA_OLLAMA_BASE_URL` (grep confirms zero matches). No update required. The default base URL is encoded only in `ProviderKind::Ollama.default_base_url()` (`mod.rs:293`) — the single source of truth.

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-common/src/llm/ollama.rs` | **New** — native ollama provider (~300 lines) |
| `crates/mika-common/src/llm/openai.rs` | Promote `extract_think_block()` from `fn` to `pub(crate) fn` (line 792) |
| `crates/mika-common/src/llm/mod.rs` | Add module, update factory match arm, change default base URL |
| `crates/mika-common/src/llm/models.rs` | Comment update only (logic unchanged) |
| `crates/mika-common/CLAUDE.md` | Note native ollama provider |

**Not changed:** `.env.example` — verified it does not contain `MIKA_OLLAMA_BASE_URL`; no operator-facing config surface needs updating.

## Risk Assessment

- **Breaking change (default base URL):** Operators with `llm_provider = "ollama"` and NO explicit `ollama_base_url` get the new native endpoint automatically. This is the intended fix — but operators who explicitly want OpenAI-compat can switch to `llm_provider = "openai"` with `openai_base_url = "http://localhost:11434/v1"`.
- **Tool calling regression:** The current OpenAI-compat path supports structured tool calls via ollama's OpenAI-compat layer. The native provider does NOT. Agents relying on ollama for tool-heavy workflows will need to use the OpenAI-compat path via `ProviderKind::OpenAi`. This is acceptable because: (a) the ticket explicitly defers tool calling, (b) ollama tool calling is model-dependent and unreliable, and (c) the XML tool-call extraction fallback provides a safety net.
- **Low blast radius:** Ollama is a local-dev provider. No production Mika instances use it.

## Out of Scope

- Streaming via `/api/generate` (single-turn)
- Native tool calling
- Native vision/multimodal
- Embedding endpoint (`/api/embeddings`)
- `keep_alive` parameter tuning

## Revision history

- rev 2 (2026-05-26): addressed F1 by citing `mod.rs:23-25` as the shared constant definition location and documenting why the 120s budget is acceptable for ollama v1; addressed F2 by citing the agent loop gate at `agent.rs:2614` confirming tool-result blocks are unreachable, and changing the flatten behavior from silent to warn-logged (contract violation signal); addressed F3 by citing `openai.rs:790-808` (`extract_think_block()`) with exact line range, and specifying `pub(crate)` promotion for cross-module reuse; addressed F4 by listing all 11 `ProviderKind` variants (`mod.rs:235-251`) and confirming the updated 3-arm match (Anthropic/Ollama/catchall) is exhaustive with no shadowing; addressed F5 by verifying `.env.example` contains no `MIKA_OLLAMA_BASE_URL` entry — no update needed.
