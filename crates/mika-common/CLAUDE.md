# mika-common — Shared Library

Shared library used by all Mika crates: config, LLM providers, Claude API client, OAuth, GitHub App auth, logging, telemetry, home directory, and test utilities.

## Modules

- **Config:** config-rs with `MIKA_` prefix, `ConfigKeyInfo` registry with `ConfigBackend` enum, `get_effective_value`/`lookup_config_key` helpers
- **Validation:** `validation.rs` — API key format, file permissions, binary-in-PATH, config value validation
- **Dotenv:** `~/.mika/.env` load/read/write via dotenvy, `parse_dotenv` for reading without env mutation, `dotenv_to_toml` for config-rs injection
- **Claude API client:** Typed request/response, retry, prompt caching injection
- **OAuth:** PKCE token exchange (`oauth.rs` — PKCE flow, `OAuthTokenManager` with `tokio::sync::RwLock` caching, `~/.mika/oauth.json` persistence)
- **GitHub App auth:** `github_app.rs` — RS256 JWT signing, installation token exchange and caching with `tokio::sync::RwLock` double-checked locking, `GitHubApp::from_settings()` constructor, file-based token cache at `{home_dir}/github_app_token.json` for short-lived CLI processes via `installation_token_with_file_cache()` — per-agent when `--agent` flag resolves to agent home dir
- **Logging:** tracing + tracing-subscriber setup
- **Telemetry:** Feature-gated OTel export. See `crates/mika-agent/CLAUDE.md` for observability architecture.
- **Home directory:** Resolution utilities
- **Model list cache:** `llm/models.rs` — `get_models()` fetches from provider `/models` API with 24h TTL file cache at `{agent_home}/cache/models/{provider}.json`, hardcoded lists for Anthropic/Google, `ModelCache`/`ModelInfo` types

## LLM Providers

Multi-provider via `LlmProvider` trait — 11 supported providers, each with its own `model`, `api_key`, and `base_url` fields in config. Active provider selected by `llm_provider` config key. Providers: Anthropic (default, Claude Sonnet 4.6), OpenAI, OpenRouter, Groq, Ollama, Mistral, Google (Gemini), DeepSeek, MiniMax, Kimi, Qwen. All non-Anthropic providers use the `OpenAiCompatibleProvider` adapter.

## Typed Errors

- `ClaudeApiError` enum with HTTP status-code retry (429/500/529)
- Provider-agnostic `LlmError` enum (`Debug + Clone + Error`) with `HttpError`, `Transport`, `ParseError`, `ProviderError`, `UnsupportedFeature` variants

## Prompt Caching (Anthropic)

`to_anthropic_request()` in `anthropic.rs` injects `cache_control: {"type": "ephemeral"}` breakpoints on system prompt (`SystemContentBlock`) and last tool definition (`CachedToolDefinition` wrapper via `#[serde(flatten)]`). Two of four allowed breakpoints used. Cache metrics (`cache_read_input_tokens`, `cache_creation_input_tokens`) logged at info level in `ClaudeClient::send_message_inner()` and persisted to `llm_calls` table (`cache_read_tokens`, `cache_write_tokens` columns). Provider-agnostic types (`LlmRequest`, `LlmMessage`) unchanged — cache *injection* is Anthropic-specific. `OpenAiCompatibleProvider` parses `prompt_tokens_details.cached_tokens` from OpenAI-standard responses into `cache_read_input_tokens` (logged at info level, persisted to `llm_calls`); `cache_creation_input_tokens` remains `None` for OpenAI-compatible providers (not reported by the OpenAI API). OpenRouter Anthropic models go through `OpenAiCompatibleProvider` and do not get prompt caching injection but do report cache read metrics if the upstream provider auto-caches.

## Internal Tag Stripping

`strip_internal_tags()` in `mika-common::llm` removes echoed internal XML tags (`<context>`, `<callback_result>`, `<task-health>`, `<rewind_reversals>`, etc.) from LLM response text before display and persistence. Applied at `run_loop()` EndTurn extraction, continuation responses, and the `send_message` tool. Uses lazy-compiled per-tag regexes with early-exit fast path when no `<` present. Closing-tag matching tolerates malformed variants from non-Anthropic models (#453): whitespace (`< /tag>`, `</ tag>`) and bare (`tag>` without `</`). System prompt also instructs the LLM not to echo internal tags (defense-in-depth).

## XML Tool Call Extraction

`extract_xml_tool_calls()` in `mika-common::llm::openai` recovers tool calls when OpenAI-compatible providers emit them as XML text (`<function=name>args</function>` or `<tool_call>...</tool_call>`) instead of structured `tool_calls`. Integrated into `from_openai_response()` — runs only when no structured tool_calls are present, converts XML to `LlmResponseContent::ToolCall`, flips `stop_reason` to `ToolUse`. Uses lazy-compiled regexes (`LazyLock<Regex>`). Defense-in-depth: `detect_text_based_tool_call()` in `agent.rs` catches patterns that slip through Layer 1, re-prompts the LLM once (similar to `required_tools_retry_done` pattern). See #447.

## MockLlmProvider

`llm/mock.rs` — sequence-based mock for deterministic agent loop testing, gated behind `test-utils` feature (`#[cfg(any(test, feature = "test-utils"))]`). Used by the eval harness in `crates/mika-agent/tests/eval/`.
