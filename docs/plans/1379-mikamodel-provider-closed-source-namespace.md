---
title: "feat: MikaModel provider for closed-source internal endpoint"
type: feat
status: active
date: 2026-06-03
origin: senara-solutions/mika#1379
---

# feat: MikaModel provider for closed-source internal endpoint

## Overview

Add a new `ProviderKind::MikaModel` variant to the `LlmProvider` registry so mika-spirit can route LLM calls to a closed-source internal model. Phase 1: served by a local Ollama runtime, talked to via the existing `OllamaProvider` transport. Phase 2 (deferred): hosted endpoint behind the same wire protocol, swap via the `mikamodel_base_url` config key.

The integration must be **independent from the generic Ollama provider** so the operator can run general-purpose models (`ollama_*`) and the internal model (`mikamodel_*`) on the same agent without their config namespaces colliding. It must also **preserve the closed-source posture**: no public-source references to model lineage, training venue, or internal codenames.

## Problem Frame

The closed-source model lives in `wizzard/` and is shipped via a GGUF artifact run by Ollama. Today nothing in mika-spirit knows it exists — there's no `ProviderKind` for it, so `MIKA_LLM_PROVIDER=...` cannot select it and no `mikamodel_*` config keys flow through. The model is built and locally serving (verified end-to-end via the wizzard#1 Modelfile-template fix: `curl /api/chat` returns a structured `tool_calls` array on tool-eliciting queries), but it is unreachable from the agent loop.

Constraints:

1. **Closed-source posture (per `wizzard/CLAUDE.md`).** Public mika source must not mention the model lineage (`mikalion → mikamedia → mika`), training venue (Runpod), training data shape, or any internal codename. The integration's identifiers (variant name, config keys, env vars, default model) must be neutral and self-documenting through generic terminology only.
2. **Config independence from `Ollama`.** The same operator already uses Ollama for general-purpose local models. The internal model needs its own namespace so changing `ollama_model = "llama3"` doesn't accidentally swap the internal model under the agent.
3. **Phase-1 deployment uses local Ollama.** The only artifact on disk is GGUF, served by Ollama at `/api/chat`. We cannot deploy via vLLM or TGI today because those need safetensors, which would require a QLoRA retrain.
4. **Phase-2 swap must be a config change.** The hosted endpoint, when it exists, will be Ollama-compatible (same `/api/chat` shape). Provider plumbing added now must survive that swap with no code change.

## Requirements Trace

- **R1.** A new `ProviderKind` variant resolvable via `MIKA_LLM_PROVIDER=mikamodel`, `mika config set llm_provider mikamodel`, and TOML `llm_provider = "mikamodel"`.
- **R2.** Three config keys: `mikamodel_model` (File-backed), `mikamodel_api_key` (Env-backed, `SecretString`-wrapped, masked as `[SET]` in `mika config show`), `mikamodel_base_url` (File-backed). Env vars: `MIKA_MIKAMODEL_MODEL`, `MIKA_MIKAMODEL_API_KEY`, `MIKA_MIKAMODEL_BASE_URL`.
- **R3.** Defaults that make the local Ollama setup work without operator config: `default_model = "mika"`, `default_base_url = "http://localhost:11434"`. `max_output_tokens = 131_072` matching the Ollama profile.
- **R4.** `create_provider()` dispatches `MikaModel` → `OllamaProvider`. Wire protocol identical to the `Ollama` arm. Tool calling natively supported (verified via wizzard#1 end-to-end test).
- **R5.** `provider_fields()`, `set_provider_model()`, `get_effective_value()`, `test_defaults()` all cover the new variant (compiler-enforced exhaustiveness).
- **R6.** `validation.rs` accepts `mikamodel_model` (non-empty) and `mikamodel_base_url` (valid http/https URL).
- **R7.** Default dotenv template (`home.rs`) lists `mikamodel` as a valid provider option with a commented example.
- **R8.** Public CLAUDE.md files updated: provider count `11 → 12`, MikaModel added to enumeration with a short note explaining the separate-variant choice.
- **R9.** No mention of model lineage, training venue, GGUF artifact, or QLoRA in any committed file. Self-documenting through neutral terminology only.
- **R10.** Unit tests cover the variant defaults, the `FromStr` round-trip, and the TOML deserialize path. Existing tests covering `ALL.len()` and the display enumeration extended for the new count.

## Proposed Solution

**Pattern:** variant-with-shared-transport. A separate `ProviderKind::MikaModel` variant with its own config namespace, dispatched via `create_provider()` to the existing `OllamaProvider`. No new `LlmProvider` impl.

### Touchpoints

1. **`crates/mika-common/src/llm/mod.rs`**
   - Add `MikaModel` variant to `ProviderKind` enum with `#[serde(rename = "mikamodel")]`.
   - Add to `ProviderKind::ALL` (count 11 → 12).
   - Extend 6 helper methods: `config_prefix → "mikamodel"`, `default_base_url → Some("http://localhost:11434")`, `max_output_tokens → 131_072`, `model_names_contain_slash → false`, `default_model → "mika"`, `FromStr` accepts `"mikamodel"`.
   - Add `ProviderKind::MikaModel` arm to `create_provider()`: build an `OllamaProvider` identically to the `Ollama` arm.

2. **`crates/mika-common/src/config.rs`**
   - 3 `ConfigKeyInfo` entries with `MIKA_MIKAMODEL_*` env binding. `mikamodel_api_key` flagged `secret: true`, `backend: ConfigBackend::Env`.
   - 3 `Settings` fields: `mikamodel_model: Option<String>`, `mikamodel_api_key: Option<SecretString>`, `mikamodel_base_url: Option<String>`.
   - Match arms in `get_effective_value` (api_key masked `[SET]`), `provider_fields` (returns the triple), `set_provider_model`, `test_defaults` (all `None`).

3. **`crates/mika-common/src/validation.rs`**
   - `mikamodel_model` added to the model-field validator (non-empty string).
   - `mikamodel_base_url` added to the URL validator (http/https scheme).

4. **`crates/mika-common/src/home.rs`**
   - `mikamodel` appended to the `llm_provider` provider list in the default config template.
   - `# mikamodel_model = "mika"` commented example in the per-provider-model section.

5. **`crates/mika-common/CLAUDE.md` + `CLAUDE.md`**
   - Provider count `11 → 12`, MikaModel added to the enumeration. Short explanation that MikaModel and Ollama share the transport but live in distinct config namespaces.

6. **Tests (in `mika-common/src/llm/mod.rs`)**
   - `test_mikamodel_defaults` — verifies `config_prefix`, `default_base_url`, `default_model`, `max_output_tokens`, `model_names_contain_slash`.
   - `test_provider_kind_deserialize_mikamodel` — TOML `provider = "mikamodel"` round-trip.
   - Extend `test_provider_kind_display`, `test_provider_kind_from_str`, `test_provider_kind_all` for the new count.

## Key Technical Decisions

### Why a separate `ProviderKind` instead of reusing `Ollama`

Considered. Rejected because:

1. **Config namespace collision.** Reusing `Ollama` would force the operator to choose between general-purpose Ollama use and internal-model use at any moment. With `mikamodel_*` separate, both can be configured independently and `MIKA_LLM_PROVIDER` switches between them without disturbing the other's state.
2. **Closed-source identity leak avoidance.** With shared `Ollama`, the internal-model identity would have to be expressed at runtime — operator-visible config or per-agent overrides — which is exactly the leak we want to avoid. A separate variant carries the identity in the public type system through a neutral name.
3. **Compiler-enforced future-proofing.** `provider_fields()`, `set_provider_model()`, `provider_kind` deserialize, and any future per-provider plumbing (prompt-caching telemetry, deadline tuning, etc.) all exhaust over `ProviderKind`. A separate variant guarantees the compiler flags any future plumbing that forgets MikaModel — no accidental aliasing.

### Why not `OpenAiCompatibleProvider`

The only artifact on disk is a format served by Ollama at `/api/chat` (not OpenAI's `/v1/chat/completions`). Routing MikaModel through `OpenAiCompatibleProvider` would require either:

- Deploying vLLM or TGI (both need safetensors — would require a retrain to recover).
- Proxying through a translation shim.

Neither is justified today. The Ollama transport is what works. The hosted-endpoint phase-2 can choose its own inference server as long as it stays Ollama-compatible; if a future migration to OpenAI-compatible is desired, the variant can be re-dispatched then. Adding that complexity preemptively violates the "implementability" check from the milestone discipline.

### Why no new `LlmProvider` impl

A dedicated Rust impl just for MikaModel would duplicate everything `OllamaProvider` already does correctly — `/api/chat` request shape, synthetic tool-call IDs, the shared deadline-aware retry plumbing, the shared error-mapping. The variant-with-shared-transport pattern gives a distinct config surface without duplicate code or duplicate maintenance.

## Scope Boundaries

### In scope

- New variant + dispatch + config + validation + tests (this PR).
- Public CLAUDE.md count + enumeration update.
- A `docs/solutions/` compound doc capturing the same design rationale for post-merge reference.

### Out of scope

- **Hosted serverless deployment.** Phase 2; separate ticket. Requires either safetensors (which would require a retrain) or Ollama-on-Runpod (a different infrastructure decision).
- **Vision support on MikaModel.** Deferred along with the general Ollama vision-support gap.
- **Prompt-caching telemetry.** Ollama upstream doesn't report cache metrics; nothing to wire on either the `Ollama` or `MikaModel` paths.
- **Calibration suite for MikaModel.** The `make calibrate-<role>` framework is anchored on production-grade base models. MikaModel is a deployment artifact rather than a calibrated base — calibration is the operator's call when the model is promoted into a role.

## Closed-source posture (verification)

- All committed files use `MikaModel` only.
- No mention of model lineage (`mikalion`, `mikamedia`, or the lineage progression).
- No mention of training data shape, GGUF format, quantization, or PEFT/QLoRA.
- No mention of Runpod or any hosted-deployment venue.
- Defaults match Ollama because the transport is Ollama, not because of any closed-source detail.
- Cross-references to `wizzard/` (the closed-source repo) stay in the closed repo. Public mika source has no path-level reference to wizzard.

## Verification

### Build verification

1. `cargo check --workspace` clean.
2. `cargo clippy -p mika-common --no-deps -- -D warnings` clean.
3. `cargo test -p mika-common --lib` — 363 passed (361 prior + 2 new dedicated MikaModel tests; 3 existing tests extended for the 11 → 12 count).
4. Pre-commit hooks (`no-large-files`, `rust-fmt`, `rust-clippy`) all green.

### Behavioral verification (post-merge, local)

1. Local Ollama already has `mika:latest` registered (built from the GGUF in `out/mika-qlora/` via the canonical `scripts/build-mika-modelfile.sh` flow from wizzard#1).
2. `MIKA_LLM_PROVIDER=mikamodel mika ask "hello"` routes via `OllamaProvider` to `http://localhost:11434/api/chat` and returns a text response.
3. `MIKA_LLM_PROVIDER=mikamodel mika ask "<tool-eliciting query>"` returns a structured `tool_calls` array (validates the wizzard#1 Modelfile-template fix end-to-end through the agent loop, not just the curl-level test).
4. `mika config show` lists `mikamodel_model`, `mikamodel_api_key`, `mikamodel_base_url`. The `api_key` value is masked as `[SET]` if set, omitted otherwise.

## Risks

- **None at the provider plumbing layer.** The dispatch routes to an already-deployed and battle-tested `OllamaProvider`. The diff adds capacity (new variant + config namespace) rather than altering the behavior of any existing provider path.
- **Operator misconfiguration risk** (set `llm_provider = "mikamodel"` with no local Ollama running): degrades to a connection-refused error from `OllamaProvider`. Existing UX matches the `Ollama` variant; no new failure mode.
- **Phase-2 swap risk** (deferred). The hosted endpoint must be Ollama-compatible at `/api/chat`. If we later decide on a non-Ollama-compatible deployment, the dispatch can be re-routed without disrupting config keys or operator-visible behavior.

## Out of scope reminder

This plan ships **the provider plumbing**. It does not:

- Deploy the model to any hosted venue.
- Retrain the model in any form.
- Modify behavior of any existing provider.
- Expose any closed-source identifier in public source.
