---
module: crates/mika-common/src/llm/mod.rs
tags: [llm, provider, ollama, closed-source, integration]
problem_type: integration-surface-design
category: architecture-patterns
---

# MikaModel as a separate ProviderKind with the Ollama transport

## Problem

The agent loop needed a way to route LLM calls to a closed-source internal model, with the following constraints:

1. **No closed-source identifiers in public mika source.** No model lineage names, no hosted-venue references, no internal codenames. The integration must be self-documenting through neutral terminology only.
2. **Independent operator config.** The operator already uses Ollama for general-purpose local models; the internal model needs its own namespace so the two don't stomp each other (different model name, different base URL, possibly different auth in the future).
3. **Phase-1 deployment uses local Ollama.** The model artifact is a GGUF served by an Ollama runtime today. Migrating to a hosted endpoint is planned but not yet possible (only GGUF on disk; safetensors would need a retrain).
4. **Future swap to a hosted endpoint must be a config change, not a code change.** Whatever provider plumbing we add now should not need rework when the deployment venue changes.

## Solution

A new `ProviderKind::MikaModel` variant that routes through the existing `OllamaProvider` transport. Same `/api/chat` wire protocol and native tool-calling support as the `Ollama` variant; everything else (config namespace, defaults, operator-visible identifiers) is distinct.

- **Config namespace:** `mikamodel_model`, `mikamodel_api_key` (`SecretString`), `mikamodel_base_url`. Env vars: `MIKA_MIKAMODEL_{MODEL,API_KEY,BASE_URL}`.
- **Defaults:** `default_model = "mika"`, `default_base_url = "http://localhost:11434"` (matches local Ollama for phase 1), `max_output_tokens = 131_072` (matches Ollama's no-hard-cap profile).
- **Dispatch:** `create_provider()` builds an `OllamaProvider` for the `MikaModel` variant — identical construction shape to the `Ollama` arm. Phase 2 (hosted endpoint) overrides `mikamodel_base_url` and keeps the same `OllamaProvider`; no code change needed as long as the upstream stays Ollama-compatible.

## Why a separate `ProviderKind` (not reusing `Ollama`)

1. **Independent config.** Reusing `Ollama` would force the operator to choose between general-purpose Ollama use and internal-model use. A separate namespace lets the two coexist on the same agent: `ollama_model = "llama3"`, `mikamodel_model = "mika"`, switch with `llm_provider`.
2. **Closed-source posture.** A separate variant named `MikaModel` is neutral and self-documenting in public source. Putting the internal model behind the generic `Ollama` label would require leaking model identity into per-agent config to disambiguate the two use cases at runtime — exactly the leak we want to avoid.
3. **Future-proof for the hosted swap.** When phase 2 lands, only `mikamodel_base_url` changes. Provider plumbing, config keys, and operator-facing semantics stay stable.
4. **Audit trail.** `provider_fields()` and `set_provider_model()` already exhaust over `ProviderKind`. A separate variant means the compiler enforces that any future plumbing (e.g., prompt-caching telemetry per provider, deadline-aware retry tuning) covers MikaModel explicitly — no accidental aliasing onto generic Ollama behavior.

## Why not `OpenAiCompatibleProvider`

Considered. Rejected because the only artifact on disk is GGUF, served by Ollama, which uses `/api/chat` not `/v1/chat/completions`. Routing MikaModel through `OpenAiCompatibleProvider` would require either deploying vLLM or TGI (which needs safetensors — not available without a QLoRA retrain) or proxying through a translation shim. The Ollama transport is what works today; the open question of which inference server to use in production can be answered later without revisiting the provider plumbing.

## Why not a dedicated provider impl

A new Rust `LlmProvider` impl just for MikaModel would duplicate everything `OllamaProvider` already does correctly: `/api/chat` shape, synthetic tool-call IDs, the same deadline-aware retry, the same error-mapping. The variant-with-shared-transport pattern (`MikaModel` and `Ollama` both build `OllamaProvider`) gives us a distinct config surface without duplicate code or duplicate maintenance.

## Closed-source posture

The PR maintains the posture under `wizzard/CLAUDE.md`:

- All committed files use `MikaModel` only.
- No mention of model lineage, training data, GGUF format, or quantization in public source.
- The internal endpoint deployment venue (local Ollama, future hosted) is not referenced; defaults match Ollama because the transport is Ollama, not because of any closed-source detail.
- Internal cross-references to wizzard remain in the closed `wizzard/` repo.

## Verification

1. Local Ollama already has `mika:latest` from prior Modelfile-template work.
2. `MIKA_LLM_PROVIDER=mikamodel mika ask "hello"` → routes via `OllamaProvider` to `localhost:11434` → text response.
3. `MIKA_LLM_PROVIDER=mikamodel mika ask "<tool-eliciting query>"` → returns a structured tool call (validates the prior Modelfile-template fix end-to-end through the agent loop).
4. `mika config show` lists the three `mikamodel_*` keys; `mikamodel_api_key` masked as `[SET]`/`[UNSET]`.
5. `cargo test -p mika-common --lib` — 363 passed (361 prior + 2 new MikaModel-specific tests, plus 3 existing tests extended for the 11 → 12 variant count).

## Out of scope

- **Hosted serverless deployment.** Phase 2; separate ticket once safetensors exist (requires a QLoRA retrain).
- **Vision support on MikaModel.** Deferred along with the general Ollama vision-support gap.
- **Prompt-caching telemetry.** Ollama upstream doesn't report cache metrics; nothing to wire.

## Related

- mika#1379 — tracking issue
- crates/mika-common/CLAUDE.md — provider count updated 11 → 12
