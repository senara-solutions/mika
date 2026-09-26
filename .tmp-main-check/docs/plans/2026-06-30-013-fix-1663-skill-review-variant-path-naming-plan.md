---
issue: 1663
type: fix
date: 2026-06-30
---

# Plan — fix(skill-review): variant path provider-name normalization (mika#1663)

## Problem

`review_skill` writes variants to `generated/<provider>/<sanitized_model>/system_prompt.md`. The `<provider>` segment comes from `resolve_canonical_provider_model(ctx.provider_name, ctx.model_name)` in `crates/mika-agent/src/skills/index.rs`. When the agent is configured via OpenRouter (`llm_provider = "openrouter"`, `model = "z-ai/glm-5.2"`), the resolver splits the slash to produce `("z-ai", "glm-5.2")` — taking the OpenRouter-namespace string as the canonical provider name.

The runtime variant loader (`crates/mika-agent/src/skills/index.rs::scan_generated_variants`) validates directory names by parsing them as `ProviderKind` via `from_str`, which **only accepts the config-key-prefix form**: `"zai"`, not `"z-ai"`. Directory `generated/z-ai/glm-5.2/` fails the parse gate → variant silently skipped → root prompt loaded instead.

**Hard evidence (body-cited, codebase-verified):**
- `crates/mika-common/src/llm/mod.rs::FromStr for ProviderKind` accepts `"zai"` only; `"z-ai"` returns `Err`.
- `crates/mika-common/src/llm/mod.rs::ProviderKind::config_prefix()` returns `"zai"` for `ProviderKind::ZAi` (used by `Display` impl).
- `crates/mika-agent/src/skills/index.rs::resolve_canonical_provider_model` splits OpenRouter `"z-ai/glm-5.2"` into `("z-ai", "glm-5.2")` with no normalization step.
- Live state: 6 variants under `~/.mika/skills/<skill>/generated/zai/glm-5.2/` work correctly (mika-dev is now on `llm_provider = "zai"` native after mika#1657 deploy + operator-CC manual `mv z-ai zai`). Pre-fix state was `generated/z-ai/`.

This bug surfaces whenever the operator switches from `openrouter/z-ai/glm-5.2` (which writes to `z-ai/`) to native `zai/glm-5.2` (which reads from `zai/`) — variants generated under OpenRouter routing become inert.

## Architectural lineage

- mika#1633 — glm-5.2 swap (introduced OpenRouter routing of z-ai/glm-5.2).
- mika#1657 — Z.AI native provider (introduced `ProviderKind::ZAi` + `"zai"` config key, surfacing the naming-mismatch).
- mika#1190 — calibration discipline (variants are core to per-model-tuned cost-reduction; this bug silently nullifies it).

## Fix shape (Option A — writer normalization, architect-confirmable)

The body proposes "Fix the writer" — compute the variant directory using `ProviderKind::from_str().config_prefix()` (or equivalent canonical form). Concrete steps:

1. **Extend `ProviderKind::from_str` to accept the `"z-ai"` alias.** Currently the match-arm only accepts `"zai"`. Add `"z-ai"` as an alternate matching to `ProviderKind::ZAi`. This is the OpenRouter-namespace-tolerance layer.
2. **Normalize `resolve_canonical_provider_model`'s OpenRouter-split output.** After splitting, attempt `ProviderKind::from_str(real_provider)`; on success, replace `real_provider` with the parsed kind's `config_prefix()`. On failure, return the raw split (legacy behavior, fail-open).
3. **The variant-loader (`scan_generated_variants`) stays as-is** — it accepts only `ProviderKind::from_str`-parseable directory names, which after Step 1 includes both `"zai"` and (transitively, via the writer normalization) NEVER `"z-ai"` because Step 2 collapses it before the path is written.

The body's stated bug is about path-write inconsistency. The root cause is actually the **OpenRouter→canonical name normalization missing in the resolver**, which the FromStr alias + resolver normalization together fix.

## Implementation outline

1. **Edit `crates/mika-common/src/llm/mod.rs::FromStr for ProviderKind`**: add `"z-ai" => Ok(ProviderKind::ZAi)` arm. Document as "OpenRouter-namespace alias." If other providers have similar dash-or-no-dash variation in OpenRouter's namespace (`deepseek` vs `deepseek-r1` patterns, `qwen` vs `qwen-2-72b`), audit and add aliases in this same PR. (Likely zero or one — focused commit.)

2. **Edit `crates/mika-agent/src/skills/index.rs::resolve_canonical_provider_model`**: after the OpenRouter split path, normalize the `real_provider` half via `ProviderKind::from_str(real_provider).map(|k| k.config_prefix())` — on success use that, on failure leave unchanged. This means even if a future OpenRouter alias is added (`x-y`) and `FromStr` knows it, the variant path keeps using the canonical key.

3. **One-shot data migration:** add a startup hook (or operator CLI command) that renames existing `generated/z-ai/` directories to `generated/zai/`. Architect-bearing — could be:
   - **3a** Inline in `seed_bundled_skills_if_needed()` or skill registry init (fires once per restart, idempotent).
   - **3b** A `mika skills migrate` CLI command operators run manually.
   - **3c** Just document the manual `mv` command in the PR body; let operators run it.
   The body explicitly mentions AC2 wants "auto-migrate or one-shot rename migration documented." Lean (3a) for cleanest UX; (3c) is the minimal-risk option. Plan defers shape to architect.

4. **Unit test (AC3):** assert that for `ctx.provider_name = "openrouter"`, `ctx.model_name = "z-ai/glm-5.2"`, the resolved canonical provider is `"zai"` (not `"z-ai"`). Test added to `crates/mika-agent/src/skills/index.rs`'s `#[cfg(test)] mod tests` (mirrors existing `test_resolve_canonical_provider_model_openrouter` style at line 4469).

5. **Smoke test (AC4):** after PR merge + deploy, run `mika ask --agent mika-dev "use review_skill to inspect dev-groom"` from an OpenRouter-routed agent (if any still exist) OR document the expected behavior: variant writes to `generated/zai/glm-5.2/`. Next `mika ask` confirms variant loads via `per_skill_bytes` log line.

## Acceptance criteria

- **AC1** — `review_skill` writes variants to `generated/zai/glm-5.2/` (not `z-ai`) when called from an agent routed via OpenRouter (`openrouter/z-ai/glm-5.2`) AND when called from an agent on native `zai`. Both paths converge on `zai/`.

- **AC2** — Existing variants under `z-ai/` auto-migrate via Step 3a (inline auto-migrate in `seed_bundled_skills_if_needed()`, idempotent walk-and-mv). Architect-confirmed shape.

- **AC2b (architect sharpening)** — OpenRouter↔canonical-name audit documented in PR body. Implementer checks explicitly: `deepseek/` (vs config `deepseek-r1`/`deepseek-chat`), `qwen/` (vs `qwen-3-*`), `google/` (vs `google-2-*`), `anthropic/` (vs `claude-*`). Each entry: either "no mismatch (OpenRouter and config use the same name)" or "mismatch — alias added: `<openrouter>` → `<config>`." If more than one additional alias needs adding beyond z-ai/zai, surface to architect — likely a scope-expansion signal.

- **AC3** — Unit test in `crates/mika-agent/src/skills/index.rs` asserts `resolve_canonical_provider_model("openrouter", "z-ai/glm-5.2")` returns `("zai", "glm-5.2")`. Mirror the existing `test_resolve_canonical_provider_model_openrouter` test style.

- **AC4** — Post-fix smoke: `mika ask --agent <openrouter-routed-agent> "use review_skill to inspect dev-groom"` writes to `generated/zai/glm-5.2/`. On next `mika ask` from any agent (zai-native OR openrouter-routed), `per_skill_bytes` log line shows variant size, not root size. Verified via log inspection in PR body.

## Out of scope

- **`refusal_regression` calibration `max_tokens` issue** — different ticket (mika#1665), different root cause.
- **`reasoning_content` surface in OpenAI-compatible adapter for Z.AI** — different ticket.
- **Audit of other OpenRouter aliases for dash-vs-no-dash mismatch.** Plan §1 says "audit in this PR" — that audit may turn up zero or one additional alias. If more than two surface, file a separate sweep ticket; don't expand scope here.

## Files involved

- `crates/mika-common/src/llm/mod.rs::FromStr for ProviderKind` — Step 1 (add `"z-ai"` alias)
- `crates/mika-agent/src/skills/index.rs::resolve_canonical_provider_model` — Step 2 (normalize OpenRouter-split provider via config_prefix)
- `crates/mika-agent/src/skills/index.rs` `#[cfg(test)] mod tests` — Step 4 (unit test, AC3)
- `crates/mika-agent/src/server/mod.rs` OR new CLI command — Step 3 (data migration, architect-bearing)
- No skill prompt changes; no schema migration

## Verification

- **Static:** `cargo build --release` clean. `cargo test -p mika-common` covers FromStr round-trip. `cargo test -p mika-agent skills::index` covers resolver normalization.
- **Live (AC4):** run `mika ask --agent mika-dev "list known skills"` (any read-only call) and check `~/.mika/agents/mika-dev/logs/mika.log.<date>` for `system_prompt_assembled` event. `per_skill_bytes` for any variant-having skill should match the variant file size, not the root size.
- **Data migration verification (AC2):** after restart with migration applied, `ls ~/.mika/skills/*/generated/` returns directories named `zai/` (or other ProviderKind-canonical keys), zero `z-ai/` directories.

## References

- mika#1633 — glm-5.2 swap (introduced OpenRouter routing of z-ai/glm-5.2)
- mika#1657 — Z.AI native provider (introduced ProviderKind::ZAi + "zai" config key)
- mika#1190 — calibration discipline (variants are load-bearing for per-model tuning)
- `crates/mika-common/src/llm/mod.rs:372-376` — Display impl uses config_prefix → "zai"
- `crates/mika-common/src/llm/mod.rs:378-398` — FromStr impl accepts only "zai" today
- `crates/mika-agent/src/skills/index.rs::resolve_canonical_provider_model` — the splitter that produces "z-ai" without normalization
- mika-dev's `system_prompt_assembled` log evidence (per_skill_bytes 1623 vs 3292 — body-cited)
