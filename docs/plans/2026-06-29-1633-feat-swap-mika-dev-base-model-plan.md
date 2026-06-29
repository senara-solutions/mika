# Plan: Swap mika-dev base model to openrouter/z-ai/glm-5.2

**Ticket:** mika#1633
**Type:** Enhancement (agent-core)
**Branch:** `feat/1633/agents-swap-mika-dev-base-model-claude`

## Problem

mika-dev currently uses the global default LLM provider (anthropic/claude-sonnet-4-6 via `Settings`), costing ~$3/M output tokens. With 249 calls/hour observed, that's ~$36/day on mika-dev alone against a $153 remaining Anthropic budget. Switching to `openrouter/z-ai/glm-5.2` reduces cost by 50-100×. Calibration gate (mika#1190) is satisfied: 100% pass (5/5 scenarios).

## Requirements

1. mika-dev provisions with `openrouter/z-ai/glm-5.2` as its base model instead of inheriting the global anthropic default.
2. Existing agents on disk receive the new config on next restart (reconciliation).
3. Calibration baseline updated to reflect the new model.
4. All existing tests pass; new test validates the config constant.

## Design

### Why config.toml, not identity.toml

The issue body suggests editing `MIKA_DEV_IDENTITY`. However, `identity.toml` and `config.toml` are deserialized into **separate Rust types** with no schema overlap:

- **`identity.toml`** → `Identity` struct (via `prompt::load_identity()`) — carries agent metadata: `[skills].allowlist`, `[kg]`, `[tools].disabled`, `[context]`, `name`, `emoji`. No LLM provider fields exist in this schema.
- **`config.toml`** → `Settings` struct (via `config-rs`) — carries runtime config: `llm_provider: ProviderKind`, `openrouter_model: Option<String>`, `llm_max_tokens`, `log_level`, API keys, etc.

`llm_provider` and `openrouter_model` are `Settings` fields (defined in `crates/mika-common/src/config.rs:601-625`). Placing them in `identity.toml` would silently fail — `config-rs` does not read `identity.toml`, and `prompt::load_identity()` does not deserialize `Settings` fields. The existing mika-arch precedent (`MIKA_ARCH_CONFIG`) confirms this separation: mika-arch's openrouter/kimi override lives in `config_toml`, not in identity.

**Conclusion:** `config.toml` is the sole vehicle for per-agent LLM provider config. The existing `reconcile_well_known_identity()` cannot propagate this change because it operates on `identity.toml` only.

> Citation: review-guide.md § KISS — the simplest correct path is the one that uses the existing type system rather than fighting it.

### Approach: Add `config_toml` to MIKA_DEV spec + minimal reconciliation

The `WellKnownAgent` struct already supports a `config_toml: Option<&'static str>` field that writes an agent-specific `config.toml` on first creation. mika-arch uses this pattern (`MIKA_ARCH_CONFIG`) to override the global provider to openrouter/kimi.

**Change:** Add a `MIKA_DEV_CONFIG` constant and set `config_toml: Some(MIKA_DEV_CONFIG)` on the `MIKA_DEV` static.

### Reconciliation for existing agents

The current `reconcile_well_known_identity()` only reconciles `identity.toml` — it does NOT reconcile `config.toml`. The `config_toml` field is only written on first agent creation (line ~748 of `well_known_agents.rs`).

For existing mika-dev agents, the on-disk `config.toml` will remain unchanged after deploy. Without reconciliation, the model swap is a no-op for any existing installation — the new constant is dead code until the agent directory is deleted and re-provisioned.

### Why reconciliation is in-scope, not scope creep

The reconciliation is **not** general-purpose infrastructure — it is the minimum mechanism required for the model swap to take effect. Without it, AC1 ("new identity provisions openrouter/z-ai/glm-5.2") is only satisfied for fresh installations. The issue body assumes "auto-provisioning will rewrite config.toml on next restart" — this assumption is wrong, and the reconciliation fixes it for this specific case.

The function is scoped to `provision_well_known_agents` (the same call site as identity reconciliation) and follows the identical pattern: compare on-disk content with spec content, overwrite if different. It is ~20 lines of code, not a new subsystem.

> Citation: review-guide.md § Single Responsibility — the single responsibility here is "make mika-dev use openrouter/z-ai/glm-5.2." The constant change and the reconciliation are two steps of the same concern, not two independent concerns. One without the other does not deliver the AC.
> Citation: review-guide.md § YAGNI — the reconciliation is right-sized: it fires only when `spec.config_toml` is `Some` and on-disk content differs. Agents without a spec config (mika-qa, mika-relay) are unaffected.

### Implementation

Add a `reconcile_well_known_config` function called from `provision_well_known_agents`, after `reconcile_well_known_identity`. When `spec.config_toml` is `Some(content)`, compare with on-disk `config.toml`. If they differ, overwrite with atomic tmp+rename (matching identity reconcile pattern). Log the change.

## Changes

### File 1: `crates/mika-agent/src/well_known_agents.rs`

1. **Add `MIKA_DEV_CONFIG` constant** (after `MIKA_DEV_IDENTITY`, ~line 161):
   ```rust
   const MIKA_DEV_CONFIG: &str = r#"# Mika Dev — autonomous development agent.
   # Base model switched to glm-5.2 per mika#1633 (cost reduction).
   
   llm_provider = "openrouter"
   openrouter_model = "z-ai/glm-5.2"
   llm_max_tokens = 8192
   log_level = "info"
   "#;
   ```

2. **Update `MIKA_DEV` static** — change `config_toml: None` to `config_toml: Some(MIKA_DEV_CONFIG)` (line 103).

3. **Add config.toml reconciliation** — extend `provision_well_known_agents` to reconcile `config.toml` for existing agents. After `reconcile_well_known_identity(home_dir, spec, settings)` (line 704), add:
   ```rust
   reconcile_well_known_config(home_dir, spec);
   ```
   
   New function `reconcile_well_known_config`:
   - If `spec.config_toml` is `None`, skip (no spec-defined config).
   - Read on-disk `config.toml`. If content matches spec, skip (idempotent).
   - If content differs, overwrite with atomic tmp+rename pattern (matching identity reconcile).
   - Log `config_reconcile.updated` or `config_reconcile.unchanged`.

4. **Add test `test_mika_dev_config_toml_is_valid_toml`** — mirrors `test_mika_arch_config_toml_is_valid_toml`:
   ```rust
   #[test]
   fn test_mika_dev_config_toml_is_valid_toml() {
       let config: toml::Value =
           toml::from_str(MIKA_DEV_CONFIG).expect("MIKA_DEV_CONFIG should be valid TOML");
       assert_eq!(config["llm_provider"].as_str(), Some("openrouter"));
       assert_eq!(config["openrouter_model"].as_str(), Some("z-ai/glm-5.2"));
   }
   ```

5. **Add test `test_reconcile_config_toml_for_mika_dev`** — provisions mika-dev, then calls reconcile, verifies config.toml is written.

6. **Add test `test_reconcile_config_toml_idempotent`** — two reconcile calls, second should be no-op.

### File 2: `docs/eval/calibration/baselines/`

Create `docs/eval/calibration/baselines/` directory (currently missing) and add the glm-5.2 baseline:

- `docs/eval/calibration/mika-dev-1633/mika-dev-glm-5.2-post-1633.json` — calibration artifact from the run documented in the issue body (100% pass, 5/5).
- `docs/eval/calibration/mika-dev-1633/mika-dev-glm-5.2-post-1633.md` — markdown summary.

Note: The `Makefile` references `docs/eval/calibration/baselines/latest.json` which doesn't exist. The existing baseline is at `docs/eval/calibration/mika-dev-1221/`. Follow the existing convention of issue-scoped directories rather than the `baselines/` path. The `--baseline` flag is optional in the calibrate binary.

## Verification Contract

| Check | Method |
|-------|--------|
| `MIKA_DEV_CONFIG` is valid TOML with correct provider/model | Unit test `test_mika_dev_config_toml_is_valid_toml` |
| Config reconciliation works for existing agents | Unit test `test_reconcile_config_toml_for_mika_dev` |
| Config reconciliation is idempotent | Unit test `test_reconcile_config_toml_idempotent` |
| Existing tests pass (allowlist count = 25, etc.) | `cargo test -p mika-agent` |
| `cargo clippy` clean | `cargo clippy --all-targets` |
| Calibration: 100% pass on glm-5.2 | Artifact from issue #1633 body (pre-merge evidence) |

## Definition of Done

- `MIKA_DEV` spec updated with `config_toml: Some(MIKA_DEV_CONFIG)` pointing to `openrouter/z-ai/glm-5.2`.
- Config.toml reconciliation ensures existing agents pick up the change on restart.
- Calibration artifact committed alongside the plan.
- All `cargo test` and `cargo clippy` pass.

## Acceptance criteria

- AC1 — `crates/mika-agent/src/well_known_agents.rs` constant updated; new identity provisions `openrouter/z-ai/glm-5.2`.
- AC2 — `make calibrate-mika-dev MODEL=openrouter/z-ai/glm-5.2` is included in PR body output (re-run on PR HEAD, must remain 100% pass).
- AC3 — Baseline file at `docs/eval/calibration/` updated to reference the new model.
- AC4 — Post-deploy: 1-hour observation window. `llm_calls` table shows mika-dev calls going to `openrouter` provider, NOT anthropic. Zero new fabrication-guard hits attributable to model swap (compare `guard.*` events 1h pre vs 1h post deploy).
- AC5 — Post-deploy: at least one full autonomous PR cycle (dispatch → impl → qa pass → merge) completes successfully on the new model. Capture the dispatched task ID in PR comment.

## Risks

- **Config reconciliation scope creep:** The new `reconcile_well_known_config` is narrow — only overwrites when `spec.config_toml` is `Some`. Agents without a spec config (mika-qa, mika-relay, mika-test) are unaffected.
- **Operator override loss:** If an operator manually edited mika-dev's `config.toml`, the reconciler will overwrite it. This is the intended behavior — code-owned config wins for well-known agents (same pattern as identity reconciliation).
- **Rollback:** Revert this PR → `config_toml` goes back to `None` → reconciler stops overwriting → operator manually restores the default config, or the old config.toml remains until the agent dir is re-provisioned.

## Out of scope

- mika-qa swap (mika#1632 — separate calibration suite needed).
- mika-arch model change.
- `Makefile` `--baseline` path fix (the `baselines/latest.json` path doesn't exist; tracked separately).

## Revision history

- rev 2 (2026-06-29): addressed F1 by adding "Why config.toml, not identity.toml" section documenting the structural reason `identity.toml` cannot carry LLM provider config (`llm_provider`/`openrouter_model` are `Settings` fields deserialized from `config.toml` via `config-rs`, not `Identity` fields from `identity.toml`); reframed reconciliation as in-scope prerequisite (not scope creep) with citations to review-guide.md § Single Responsibility, § YAGNI, and § KISS; removed the Option A/Option B decision matrix in favor of a single justified approach.
