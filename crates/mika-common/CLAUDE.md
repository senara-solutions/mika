# mika-common — Shared Library

Shared library used by all Mika crates: config, LLM providers, Claude API client, OAuth, GitHub App auth, logging, telemetry, home directory, and test utilities.

## Modules

- **Config:** config-rs with `MIKA_` prefix, `ConfigKeyInfo` registry with `ConfigBackend` enum, `get_effective_value`/`lookup_config_key` helpers. All API keys and tokens in `Settings` use `secrecy::SecretString` for compile-time exposure safety and zeroize-on-drop. Secrets exposed at accessor boundary via `.expose_secret()`. `get_effective_value()` returns `"[SET]"` for secret-flagged fields.
- **Validation:** `validation.rs` — API key format, file permissions, binary-in-PATH, config value validation
- **Dotenv:** `~/.mika/.env` load/read/write via dotenvy, `parse_dotenv` for reading without env mutation, `dotenv_to_toml` for config-rs injection
- **Claude API client:** Typed request/response, retry, prompt caching injection
- **OAuth:** PKCE token exchange (`oauth.rs` — PKCE flow, `OAuthTokenManager` with `tokio::sync::RwLock` caching, `~/.mika/oauth.json` persistence)
- **GitHub App auth:** `github_app.rs` — RS256 JWT signing, installation token exchange and caching with `tokio::sync::RwLock` double-checked locking, `GitHubApp::from_settings()` constructor (takes `&Settings`), `GitHubApp::from_credentials()` constructor (takes raw `app_id`, `private_key_b64`, `installation_id` — used by the gateway which has its own settings type), file-based token cache at `{home_dir}/github_app_token.json` for short-lived CLI processes via `installation_token_with_file_cache()` — per-agent when `--agent` flag resolves to agent home dir
- **Logging:** tracing + tracing-subscriber setup
- **Telemetry:** Feature-gated OTel export. See `crates/mika-agent/CLAUDE.md` for observability architecture.
- **Text:** `text.rs` — `safe_truncate(s, max_bytes)` for UTF-8-safe byte-budget truncation using `floor_char_boundary`. Returns `&str`, never panics on multi-byte characters. Use for log line widths, prompt size budgets, and error message previews. Distinct from `db::truncate_chars` (char-count-based, appends "...", returns `String`).
- **Home directory:** Resolution utilities
- **Model list cache:** `llm/models.rs` — `get_models()` fetches from provider `/models` API with 24h TTL file cache at `{agent_home}/cache/models/{provider}.json`, hardcoded lists for Anthropic/Google, `ModelCache`/`ModelInfo` types

## LLM Providers

Multi-provider via `LlmProvider` trait — 12 supported providers, each with its own `model`, `api_key`, and `base_url` fields in config. Active provider selected by `llm_provider` config key. Providers: Anthropic (default, Claude Sonnet 4.6), OpenAI, OpenRouter, Groq, Ollama, Mistral, Google (Gemini), DeepSeek, MiniMax, Kimi, Qwen, MikaModel. Anthropic uses its native API client. Ollama and MikaModel both use the native Ollama provider (`llm/ollama.rs`) that targets `/api/chat` — no `/v1` suffix required in the base URL, supports tool calling natively (synthetic IDs since Ollama omits them), no vision support (deferred). MikaModel is a separate `ProviderKind` so the operator can configure the internal endpoint (`mikamodel_*` keys, defaults to `http://localhost:11434` + model `"mika"`) independently from a general-purpose Ollama (`ollama_*` keys); the same transport, different config namespace. All other non-Anthropic providers use the `OpenAiCompatibleProvider` adapter.

**Deadline-aware retry (#939):** `LlmProvider` exposes `send_message_with_deadline(request, deadline: Option<Instant>)` with a default implementation that ignores the deadline and delegates to `send_message`. Both `AnthropicProvider` and `OpenAiCompatibleProvider` override the method to check the remaining deadline budget before each retry — when remaining < `TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` (120s, shared constants in `llm/mod.rs`), the retry chain aborts immediately with a "deadline budget insufficient" error context. Callers without deadline visibility pass `None` and get unchanged behavior. The agent loop (`crates/mika-agent/src/agent.rs`) is the primary caller with `Some(deadline)`.

## Typed Errors

- `ClaudeApiError` enum with HTTP status-code retry (429/500/529); `BillingError` variant for non-retriable Anthropic HTTP 400 billing rejections (detected via `error.type == "invalid_request_error"` + message prefix, logs at `error!`, surfaces billing URL in error chain)
- Provider-agnostic `LlmError` enum (`Debug + Clone + Error`) with `HttpError`, `Transport`, `ParseError`, `ProviderError`, `UnsupportedFeature` variants

## Prompt Caching (Anthropic)

`to_anthropic_request()` in `anthropic.rs` injects `cache_control: {"type": "ephemeral"}` breakpoints on system prompt (`SystemContentBlock`) and last tool definition (`CachedToolDefinition` wrapper via `#[serde(flatten)]`). Two of four allowed breakpoints used. Cache metrics (`cache_read_input_tokens`, `cache_creation_input_tokens`) logged at info level in `ClaudeClient::send_message_inner()` and persisted to `llm_calls` table (`cache_read_tokens`, `cache_write_tokens` columns). Provider-agnostic types (`LlmRequest`, `LlmMessage`) unchanged — cache *injection* is Anthropic-specific. `OpenAiCompatibleProvider` parses `prompt_tokens_details.cached_tokens` from OpenAI-standard responses into `cache_read_input_tokens` (logged at info level, persisted to `llm_calls`); `cache_creation_input_tokens` remains `None` for OpenAI-compatible providers (not reported by the OpenAI API). OpenRouter Anthropic models go through `OpenAiCompatibleProvider` and do not get prompt caching injection but do report cache read metrics if the upstream provider auto-caches.

## Internal Tag Stripping

`strip_internal_tags()` in `mika-common::llm` removes echoed internal XML tags (`<context>`, `<callback_result>`, `<task-health>`, `<rewind_reversals>`, etc.) from LLM response text before display and persistence. Applied at `run_loop()` EndTurn extraction, continuation responses, and the `send_message` tool. Uses lazy-compiled per-tag regexes with early-exit fast path when no `<` present. Closing-tag matching tolerates malformed variants from non-Anthropic models (#453): whitespace (`< /tag>`, `</ tag>`) and bare (`tag>` without `</`). System prompt also instructs the LLM not to echo internal tags (defense-in-depth).

## XML Tool Call Extraction

`extract_xml_tool_calls()` in `mika-common::llm::openai` recovers tool calls when OpenAI-compatible providers emit them as XML text (`<function=name>args</function>` or `<tool_call>...</tool_call>`) instead of structured `tool_calls`. Integrated into `from_openai_response()` — runs only when no structured tool_calls are present, converts XML to `LlmResponseContent::ToolCall`, flips `stop_reason` to `ToolUse`. Uses lazy-compiled regexes (`LazyLock<Regex>`). Defense-in-depth: `detect_text_based_tool_call()` in `agent.rs` catches patterns that slip through Layer 1, re-prompts the LLM once (similar to `required_tools_retry_done` pattern). See #447.

## MockLlmProvider

`llm/mock.rs` — sequence-based mock for deterministic agent loop testing, gated behind `test-utils` feature (`#[cfg(any(test, feature = "test-utils"))]`). Used by the eval harness in `crates/mika-agent/tests/eval/`. Responses stored in `Mutex<Vec<MockResponse>>` for dynamic replacement via `clear_and_set()` — enables tests that need to seed DB data (generating IDs) before configuring mock responses that reference those IDs. Must be called before `.run()` (not safe during concurrent access). `MockLlmProviderBuilder::health_error(LlmError)` configures `check_health()` to return the given error on every call (default: `Ok(())`), enabling degraded-provider scenario testing.

**`MockResponse::Delayed { sleep_ms, inner }` variant (#848):** wraps an inner response with a virtual `tokio::time::sleep(Duration::from_millis(sleep_ms))` before resolving. Pair with `tokio::time::pause()` + `start_paused = true` on the test to drive the clock without wall-clock waits — used by the deadline-during-LLM-call eval scenarios in `tests/eval/test_deadline_in_flight_llm_call.rs`. Always uses `tokio::time::sleep` (never `std::thread::sleep`, which would block the runtime and defeat virtual-time control). Nesting `Delayed` inside `Delayed` panics at resolve time; push multiple `Delayed` entries onto the response sequence instead. Helper constructor: `delayed_response(sleep_ms, inner)`.

`Settings::test_defaults()` — minimal `Settings` constructor for deterministic tests, gated behind `test-utils` feature. Lives on `Settings` itself so new field additions produce a compile error at the canonical location. Used by the eval harness and `mika-agent::test_utils::dummy_settings()` (which delegates to it).
