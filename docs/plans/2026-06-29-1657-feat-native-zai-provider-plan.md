# Plan: feat(llm) — Native Z.AI provider for direct GLM API (mika#1657)

## Problem

mika-dev routes GLM-5.2 calls through OpenRouter (`openrouter/z-ai/glm-5.2`). OpenRouter charges a 10–20% margin per call. With a Z.AI direct subscription (flat-rate), routing through OpenRouter means paying twice: the subscription fee AND the per-token markup. A native Z.AI provider eliminates the double-pay by talking to Z.AI's API directly.

## Requirements

1. Add `Zai` as a first-class `ProviderKind` variant — the 13th provider.
2. Z.AI provides an OpenAI-compatible API, so the implementation falls into the existing `OpenAiCompatibleProvider` adapter (the `_ =>` catch-all in `create_provider()`). No new provider file needed.
3. Settings wiring: `zai_api_key`, `zai_model`, `zai_base_url` fields with `MIKA_ZAI_API_KEY` / `MIKA_ZAI_MODEL` / `MIKA_ZAI_BASE_URL` env var bindings.
4. Default base URL: `https://open.z.ai/api/paas/v4` (per Z.AI docs — verify at implementation time; the ticket's `https://api.z.ai/v1` is a guess).
5. Default model: `glm-4-plus` (GLM-5.2 is the target but may not be the canonical model ID string — verify at implementation time).
6. Update docs and `.env.example`.

## Architectural analysis

### Why no new file is needed

The existing 12 providers are served by three implementations:
- `anthropic.rs` — native Anthropic API (1 provider)
- `ollama.rs` — native Ollama transport (2 providers: Ollama, MikaModel)
- `openai.rs` — OpenAI-compatible adapter (9 providers: OpenAI, OpenRouter, Groq, Mistral, Google, DeepSeek, MiniMax, Kimi, Qwen)

Z.AI's API is OpenAI-compatible (same `/chat/completions` endpoint shape, Bearer auth). It falls into the third category — the `_ =>` catch-all in `create_provider()` already routes all non-Anthropic, non-Ollama providers to `OpenAiCompatibleProvider`. Adding the `Zai` variant to `ProviderKind` is sufficient; no new Rust file is required.

### Touch-point inventory

Every new provider requires changes at exactly these 14 sites in `crates/mika-common/src/`. The pattern is mechanical and identical for all OpenAI-compatible providers added after the original nine.

## Implementation steps

### Step 1 — Add `ProviderKind::Zai` variant

**File:** `crates/mika-common/src/llm/mod.rs`

1. Add `Zai` variant to the `ProviderKind` enum (after `Qwen`, before `MikaModel`):
   ```rust
   #[serde(rename = "zai")]
   Zai,
   ```

2. Add `ProviderKind::Zai` to `ProviderKind::ALL` array (after `Qwen`, before `MikaModel`).

3. Add arm to `config_prefix()`:
   ```rust
   ProviderKind::Zai => "zai",
   ```

4. Add arm to `default_base_url()`:
   ```rust
   ProviderKind::Zai => Some("https://open.z.ai/api/paas/v4"),
   ```
   **Note:** Verify the actual endpoint from Z.AI docs at implementation time. The ticket suggested `https://api.z.ai/v1` but this needs confirmation.

5. Add arm to `max_output_tokens()`:
   ```rust
   ProviderKind::Zai => 16_384, // conservative; adjust after verifying Z.AI docs
   ```

6. Add arm to `default_model()`:
   ```rust
   ProviderKind::Zai => "glm-4-plus",
   ```
   **Note:** Verify the canonical model ID string from Z.AI docs. The ticket targets "glm-5.2" but the API model identifier may differ.

7. No change to `model_names_contain_slash()` — Z.AI model names don't contain slashes (default `false` from the non-matching `matches!` macro).

8. Add arm to `FromStr`:
   ```rust
   "zai" => Ok(ProviderKind::Zai),
   ```
   And update the error message's known-providers list to include `zai`.

9. No change to `create_provider()` — the `_ =>` catch-all already handles all OpenAI-compatible providers including `Zai`.

### Step 2 — Add Settings fields

**File:** `crates/mika-common/src/config.rs`

1. Add three fields to `Settings` struct (between the Qwen and MikaModel field groups):
   ```rust
   // -- Per-provider fields: Z.AI --
   pub zai_model: Option<String>,
   pub zai_api_key: Option<SecretString>,
   pub zai_base_url: Option<String>,
   ```

2. Add three `ConfigKeyInfo` entries to the `CONFIG_KEYS` array (between the MiniMax and MikaModel entries):
   ```rust
   ConfigKeyInfo {
       key: "zai_model",
       backend: ConfigBackend::File,
       env_var: Some("MIKA_ZAI_MODEL"),
       secret: false,
       description: "Z.AI model ID",
   },
   ConfigKeyInfo {
       key: "zai_api_key",
       backend: ConfigBackend::Env,
       env_var: Some("MIKA_ZAI_API_KEY"),
       secret: true,
       description: "Z.AI API key",
   },
   ConfigKeyInfo {
       key: "zai_base_url",
       backend: ConfigBackend::File,
       env_var: Some("MIKA_ZAI_BASE_URL"),
       secret: false,
       description: "Z.AI base URL override",
   },
   ```

3. Add `ProviderKind::Zai` arm to `provider_fields()`:
   ```rust
   ProviderKind::Zai => (
       self.zai_model.as_deref(),
       self.zai_api_key.as_ref().map(|s| s.expose_secret()),
       self.zai_base_url.as_deref(),
   ),
   ```

4. Add `ProviderKind::Zai` arm to `set_provider_model()`:
   ```rust
   ProviderKind::Zai => self.zai_model = model,
   ```

5. Add entries to `get_effective_value()`:
   ```rust
   // Per-provider: Z.AI
   "zai_model" => settings.zai_model.clone(),
   "zai_api_key" => settings.zai_api_key.as_ref().map(|_| "[SET]".to_string()),
   "zai_base_url" => settings.zai_base_url.clone(),
   ```

6. Add to `test_defaults()`:
   ```rust
   zai_model: None,
   zai_api_key: None,
   zai_base_url: None,
   ```

7. Add to `Debug` impl:
   ```rust
   .field("zai_model", &self.zai_model)
   .field("zai_api_key", &self.zai_api_key.as_ref().map(|_| "[REDACTED]"))
   .field("zai_base_url", &self.zai_base_url)
   ```

### Step 3 — Vision support decision

**File:** `crates/mika-common/src/llm/openai.rs`

Z.AI's GLM models support vision (multimodal). Add `ProviderKind::Zai` to the `supports_vision()` match:
```rust
fn supports_vision(&self) -> bool {
    matches!(
        self.provider_kind,
        ProviderKind::OpenAi
            | ProviderKind::OpenRouter
            | ProviderKind::Mistral
            | ProviderKind::Google
            | ProviderKind::DeepSeek
            | ProviderKind::Zai
    )
}
```

**Note:** Verify GLM-5.2 vision support from Z.AI docs. If unsupported, omit from the match (vision defaults to `false`).

### Step 4 — Update `.env.example`

**File:** `.env.example`

Add after the DeepSeek entry:
```bash
# MIKA_ZAI_API_KEY=...
```

### Step 5 — Update documentation

**Files to update:**

1. **`CLAUDE.md` (root):** Add `MIKA_ZAI_API_KEY` to the environment variables section. Update provider count from 12 to 13.

2. **`crates/mika-common/CLAUDE.md`:** Update the provider list paragraph to include Z.AI. Update provider count.

3. **`llm_provider` description in `ConfigKeyInfo`:** Update the description string to include `zai` in the known-providers list.

### Step 6 — Tests

**File:** `crates/mika-common/src/config.rs` (test module)

1. The existing `test_get_effective_value_returns_set_for_secrets` test should continue to pass — it tests specific keys but doesn't exhaustively test all providers. No change needed unless the test iterates `ProviderKind::ALL`.

2. Add `zai_api_key` to the secrets test if it uses an explicit list.

3. The existing `FromStr` round-trip tests (if any) will need `"zai"` added.

**File:** `crates/mika-common/src/llm/mod.rs` (test module)

1. If there are tests that iterate `ProviderKind::ALL` and check `config_prefix()` / `default_base_url()` / `default_model()` coverage, they will automatically pick up `Zai` — verify they pass.

## Verification contract

| Check | How to verify |
|-------|---------------|
| Compiles | `cargo build` succeeds with no warnings on the new code |
| Tests pass | `cargo test -p mika-common` — all existing tests pass |
| Provider dispatches | `Settings { llm_provider: "zai", zai_api_key: Some(...), zai_model: Some("glm-4-plus") }` creates a working `OpenAiCompatibleProvider` via `make_llm_provider()` |
| Config wiring | `mika config get zai_api_key` returns `[SET]` when env var is set |
| Smoke test (AC4) | `mika ask --agent mika-dev "hello"` with `llm_provider = "zai"` returns a real GLM response |
| Calibration (AC6) | `make calibrate-mika-dev MODEL=zai/glm-5.2` produces passing report |

## Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Z.AI API endpoint URL differs from assumed `https://open.z.ai/api/paas/v4` | Verify from Z.AI docs before implementing; `zai_base_url` override available as fallback |
| GLM-5.2 model ID string differs | `zai_model` config allows any model string; default can be corrected post-merge |
| Z.AI API is not fully OpenAI-compatible (different error shapes, missing fields) | `OpenAiCompatibleProvider` is battle-tested with 9 providers of varying compatibility; worst case, error messages may be less informative but calls will work |
| Z.AI subscription not yet active when PR merges | No impact — provider is dormant until `MIKA_ZAI_API_KEY` is set; zero cost when unused |

## Out of scope

- Z.AI subscription itself (operator action)
- Swapping mika-dev to use `zai` provider (separate swap PR, gated on calibration)
- Z.AI's full model catalog beyond GLM-5.2
- Extended thinking support for Z.AI (GLM models don't use `<think>` blocks)

## Definition of Done

- [ ] `ProviderKind::Zai` variant exists with all 8 method arms
- [ ] `Settings` has `zai_model`, `zai_api_key`, `zai_base_url` fields
- [ ] `ConfigKeyInfo` has 3 entries for Z.AI
- [ ] `provider_fields()`, `set_provider_model()`, `get_effective_value()`, `test_defaults()`, `Debug` impl all handle Z.AI
- [ ] `.env.example` documents `MIKA_ZAI_API_KEY`
- [ ] `CLAUDE.md` and `crates/mika-common/CLAUDE.md` updated
- [ ] `cargo build` and `cargo test -p mika-common` pass
- [ ] `cargo clippy` clean

## Acceptance criteria

- **AC1 — Provider exists.** `crates/mika-common/src/llm/mod.rs` defines `ProviderKind::Zai` with all required method arms (`config_prefix`, `default_base_url`, `max_output_tokens`, `default_model`, `FromStr`). The variant routes through the existing `OpenAiCompatibleProvider` adapter via the `_ =>` catch-all in `create_provider()`.
- **AC2 — Settings wiring.** `Settings` exposes `zai_api_key: Option<SecretString>`, `zai_model: Option<String>`, and `zai_base_url: Option<String>` fields. `MIKA_ZAI_API_KEY` env var binds to the key field via `ConfigKeyInfo`.
- **AC3 — Factory dispatch.** `Settings { llm_provider: "zai", zai_api_key: Some(...), zai_model: Some("glm-4-plus") }` creates a working `OpenAiCompatibleProvider` instance via `make_llm_provider()`.
- **AC4 — Smoke test.** `mika ask --agent mika-dev "hello"` works when mika-dev's config has `llm_provider = "zai"` + `zai_model = "glm-5.2"` + the env var set. Returns a real GLM-5.2 response (no errors).
- **AC5 — Cost/latency reads.** When swapped, a single skill-review call against self-dev (the 54KB workload that timed out on OpenRouter routing) completes inside the 120s HTTP timeout in `openai.rs` style (or document if Z.AI direct has different timeout characteristics).
- **AC6 — Calibration baseline.** Run `make calibrate-mika-dev MODEL=zai/glm-5.2` after swap; commit the JSON + markdown report under `docs/eval/calibration/mika-dev-1XXX/` per mika#1190's calibration-gates-swaps discipline.
