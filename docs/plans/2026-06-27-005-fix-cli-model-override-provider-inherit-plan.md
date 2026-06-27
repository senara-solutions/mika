---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: senara-solutions/mika#1591
branch: fix/1591/cli-mika-ask-model-prefix-routing
groomed: mika-arch session aa53117b (F1/F2/F3 verified)
created: 2026-06-27
type: fix
---

# fix(cli): `mika ask/chat --model <prefix>/<id>` must inherit the agent's configured llm_provider

## Summary

The `--model` launch flag on `mika ask` and `mika chat` re-dispatches to a native LLM
provider whenever the model id carries a `prefix/` that happens to match a provider name
(e.g. `qwen/qwen3.7-max`). That native provider usually has no API key configured, so the
request fails with a bare HTTP 401 "no API key" — even though the same model id routes
correctly through the agent's configured provider (e.g. `openrouter`, which *requires* the
vendor-prefixed id). This fix makes `--model` a **model-id-only override**: the provider is
always inherited from the agent's configured `llm_provider`, never re-dispatched from the
model-name prefix.

## Problem Frame

`AppContext::override_model()` (`crates/mika-cli/src/init.rs:83-107`) parses the `--model`
value and, when it finds a `prefix/rest` shape whose prefix parses to a `ProviderKind`,
**switches `settings.llm_provider` to that native provider** and strips the prefix from the
model id:

```rust
if let Ok(provider) = prefix.parse::<ProviderKind>() {     // "qwen" parses as native provider
    self.db_ctx.settings.llm_provider = provider;          // BUG: abandons the agent's provider
    self.db_ctx.settings.set_provider_model(provider, Some(model_name)); // BUG: strips "qwen/"
    ...
```

Two distinct defects:
1. **Provider re-dispatch.** `--model qwen/qwen3.7-max` against an `openrouter` agent switches
   to native Qwen (no key) → 401, instead of routing through OpenRouter.
2. **Prefix stripping.** OpenRouter model ids *are* `vendor/model` (its own default is
   `anthropic/claude-sonnet-4`), so stripping the `qwen/` prefix would mangle the id even if
   the provider were inherited.

`override_model` is called from exactly two sites — `crates/mika-cli/src/commands/ask.rs:85`
and `crates/mika-cli/src/commands/chat.rs:553` — both the `--model` launch flag. The TUI
`/model` slash command and the `mika model` subcommand are **separate code paths** and are
out of scope: their `provider/model` cross-provider switching is intentional and must not
change.

## Requirements

- **R1** (AC1): `--model qwen/qwen3.7-max` with agent `llm_provider = openrouter` routes
  through OpenRouter with the full id `qwen/qwen3.7-max` and succeeds (no 401).
- **R2** (AC3): `--model qwen/qwen3.7-max` with agent `llm_provider = qwen` strips the
  matching `qwen/` prefix → model id `qwen3.7-max`, routes through native Qwen.
- **R3** (AC4): `--model claude-sonnet-4-6-20250514` with agent `llm_provider = anthropic`
  routes through Anthropic, no prefix change (non-prefixed ids unaffected — current behavior).
- **R4** (AC2): when the inherited provider has no API key configured (and requires one), the
  error names the provider and model id, e.g. `Provider 'openrouter' has no API key
  configured. Cannot route model 'qwen/qwen3.7-max'.` — not a bare 401 "no API key".
- **R5** (AC5/AC6): document the new `--model` semantics in `mika ask --help` and
  `crates/mika-cli/CLAUDE.md`.
- **R6** (AC7): unit tests for the parse contract, including the openrouter/qwen/anthropic cases.

## Key Technical Decisions

- **KTD1 — Introduce a pure helper `parse_model_override`.** Signature:
  `fn parse_model_override(model: &str, configured: ProviderKind) -> (ProviderKind, String)`.
  It resolves aliases first (`crate::cli::resolve_model_alias`), then:
  - if the resolved value has a `prefix/rest` shape **and** `prefix.parse::<ProviderKind>()`
    succeeds **and** that `ProviderKind == configured` → return `(configured, rest)` (R2);
  - otherwise → return `(configured, resolved)` (R1, R3).
  The returned provider is **always** `configured` — no prefix re-dispatch. Pure (no `&self`,
  no I/O) so it is directly unit-testable per R6. Lives in `crates/mika-cli/src/init.rs`
  alongside `override_model` (it needs `resolve_model_alias` from `crate::cli` and
  `ProviderKind` from `mika_common::llm`).
- **KTD2 — `override_model` becomes a thin caller.** It reads the agent's current
  `settings.llm_provider` as `configured`, calls `parse_model_override`, sets the model on
  that provider via `set_provider_model(configured, Some(model_id))`, and rebuilds the
  provider with `make_llm_provider()`. It never assigns `settings.llm_provider`. This single
  change fixes both `ask` and `chat` callers.
- **KTD3 — AC2 pre-flight key check in the override path.** After resolving `(provider,
  model_id)` and before/at provider construction, if the inherited provider **requires an API
  key** and none is configured, bail with the named error from R4. "Requires a key" excludes
  local providers (`Ollama`, `MikaModel` — localhost base URL, key optional per config docs).
  Reuse `provider_fields(provider)` (`crates/mika-common/src/config.rs:1043`) to read the
  configured key. Exact predicate for "requires key" is an execution-time detail (see Deferred);
  the simplest correct form is "has a non-localhost default base URL and key is absent/empty".
  Scope the check to the `--model` override path so unrelated default-provider startup is
  untouched.

## Implementation Units

### U1. `parse_model_override` helper + rewire `override_model`

- **Goal:** Provider is always inherited; matching-prefix-only stripping. Fixes R1, R2, R3.
- **Requirements:** R1, R2, R3.
- **Dependencies:** none.
- **Files:**
  - `crates/mika-cli/src/init.rs` — add `parse_model_override`, rewrite `override_model` body.
- **Approach:** Implement `parse_model_override` per KTD1. Rewrite `override_model` (KTD2) to:
  `let configured = self.db_ctx.settings.llm_provider; let (provider, model_id) =
  parse_model_override(model, configured);` then `set_provider_model(provider, Some(model_id))`
  + `make_llm_provider()`. Delete the `settings.llm_provider = provider` reassignment and the
  prefix-find branch. Keep the alias resolution (now inside the helper). `provider` returned
  always equals `configured`, so `set_provider_model` targets the agent's provider.
- **Patterns to follow:** existing `override_model` structure; `make_provider_from_model_string`
  (`config.rs:1245`) for the `split_once('/')` + `parse::<ProviderKind>()` idiom.
- **Test scenarios** (R6 / AC7) — `#[cfg(test)] mod tests` in `init.rs`:
  - `parse_model_override("qwen/qwen3.7-max", OpenRouter)` → `(OpenRouter, "qwen/qwen3.7-max")` (Covers AC1).
  - `parse_model_override("qwen/qwen3.7-max", Qwen)` → `(Qwen, "qwen3.7-max")` (Covers AC3).
  - `parse_model_override("claude-sonnet-4-6-20250514", Anthropic)` → `(Anthropic, "claude-sonnet-4-6-20250514")` (Covers AC4).
  - Alias resolution: a known alias (e.g. `sonnet`) resolves before prefix logic and inherits `configured`.
  - Cross-vendor non-matching prefix with a non-OpenRouter configured provider keeps the full id and the configured provider (no re-dispatch) — guards against regression of the old behavior.
- **Verification:** `cargo test -p mika-cli` passes the new cases; `cargo clippy -p mika-cli` clean.

### U2. AC2 — named error when inherited provider lacks a required API key

- **Goal:** Replace the bare 401 with a provider+model-named error in the `--model` path. Fixes R4.
- **Requirements:** R4.
- **Dependencies:** U1 (uses the resolved `(provider, model_id)`).
- **Files:**
  - `crates/mika-cli/src/init.rs` — add the pre-flight key check in `override_model`.
- **Approach:** Per KTD3, after resolving `(provider, model_id)`, read the configured key via
  `self.db_ctx.settings.provider_fields(provider)`. If the provider requires a key and the key
  is `None`/empty, `anyhow::bail!("Provider '{provider}' has no API key configured. Cannot
  route model '{model_id}'.")`. Use `ProviderKind`'s `Display` (`as_str`) for the provider name.
  Local providers (Ollama, MikaModel) are exempt.
- **Patterns to follow:** `anyhow::bail!` usage already in `ask.rs`; `provider_fields` return
  shape `(model, api_key, base_url)` at `config.rs:1043`.
- **Execution note:** confirm the exact "requires a key" predicate against `ProviderKind`
  helpers at implementation time — prefer an existing method over a hardcoded match if one
  exists; otherwise exempt `Ollama`/`MikaModel` explicitly and treat all others as key-requiring.
- **Test scenarios** (R4):
  - With a `Settings` whose `openrouter` key is unset, the override path for an openrouter
    agent + `--model qwen/qwen3.7-max` returns an error whose message contains both
    `openrouter` and `qwen/qwen3.7-max`. (Use `Settings::test_defaults()` to construct.)
  - A local provider (Ollama) with no key does NOT trip the check.
- **Verification:** `cargo test -p mika-cli` covers the named-error and exemption cases.

### U3. Docs — `--help` text + crate CLAUDE.md (AC5/AC6)

- **Goal:** Document `--model` as model-id-only, provider inherited. Fixes R5.
- **Requirements:** R5.
- **Dependencies:** U1 (final semantics settled).
- **Files:**
  - `crates/mika-cli/src/cli.rs` — the clap `--model` arg help/doc-comment (find the `model`
    arg on the Ask/Chat subcommands; help text feeds `mika ask --help`).
  - `crates/mika-cli/CLAUDE.md` — the `mika ask` `--model` description: note it is a
    model-id-only override; provider always inherited from agent config; no name-prefix
    re-dispatch.
- **Approach:** Update help to: "Overrides the model id for this request only (not persisted).
  Routes through the agent's configured llm_provider — does not re-dispatch based on model
  name prefix." Mirror in CLAUDE.md. Keep wording consistent across both surfaces.
- **Test scenarios:** Test expectation: none — documentation/help-string change. (Optional:
  an existing `--help` snapshot test, if present, will need its expected text updated.)
- **Verification:** `cargo build -p mika-cli` succeeds; `mika ask --help` shows the new text;
  CLAUDE.md edit reads coherently next to the existing `--model` paragraph.

## Scope Boundaries

In scope: the `--model` launch-flag path for `mika ask` and `mika chat` via `override_model`;
the named-error improvement on that path; help + CLAUDE.md docs; unit tests.

Out of scope (from issue body):
- A `--provider` flag (Shape B).
- Prefix-disambiguation table / `--model-route` opt-in (Shape C).
- The TUI `/model` slash command and `mika model` subcommand (separate paths; cross-provider
  switching there is intentional).
- Persistent config paths (`config.toml`, `skill_overrides`) — already route correctly.
- Calibration gating for one-shot `--model` overrides.

## Deferred to Implementation

- Exact "provider requires an API key" predicate (existing `ProviderKind` helper vs. explicit
  `Ollama`/`MikaModel` exemption).
- Whether any existing `--help` snapshot/golden test exists that must be updated for U3.

## Verification Contract

- `cargo test -p mika-cli` green, including the new `parse_model_override` and AC2 cases.
- `cargo clippy -p mika-cli --all-targets` clean.
- `cargo build` workspace-wide succeeds.
- Manual (optional, requires keys): `mika ask --agent <openrouter-agent> --model
  qwen/qwen3.7-max "ping"` returns a model response, not a 401.

## Definition of Done

All of R1–R6 met; AC1–AC7 from mika#1591 satisfied; U1–U3 landed with tests; pipeline
verification (`scripts/verify-pipeline.sh`) passes; PR opened with `Closes #1591`.
