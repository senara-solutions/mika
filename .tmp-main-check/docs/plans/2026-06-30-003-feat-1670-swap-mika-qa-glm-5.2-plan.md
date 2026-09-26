---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: senara-solutions/mika#1670
plan_type: feat
created: 2026-06-30
---

# feat: Swap mika-qa base model to zai/glm-5.2 (mika#1670)

> **Target repo:** mika · **Issue:** senara-solutions/mika#1670 · **Branch:** `feat/1670/well-known-agents-swap-mika-qa-base`

---

## Summary

Switch the well-known `mika-qa` agent's base LLM from its current default (inherited Anthropic) to **`zai/glm-5.2`** via the native Z.AI provider (mika#1657), mirroring the `MIKA_DEV_CONFIG` precedent (mika#1633). The swap is gated by a passing mika-qa calibration run (5/5 PASS, 2026-06-30 11:15 UTC) per the mika#1190 calibration-required-before-swap rule. Ships the source-of-truth `config_toml` change, a TOML-validity test mirroring mika-dev's, the committed calibration evidence, and a runtime config apply so the live agent picks up the swap without a fresh provision.

---

## Problem Frame

**WHY.** mika-qa is the **fabrication-catching layer** of the autonomous loop — mika-dev on glm-5.2 is workable *because* mika-qa catches its fabrications. Putting mika-qa itself on a glm-5.2 base is a cost reduction (native Z.AI bypasses OpenRouter margin) and a 1M-context unlock for the qa layer, but it must not weaken the catcher. The mika-qa calibration suite (mika#1632, 5 scenarios: verdict format precision, per-AC enumeration, absence-claim grounding, wip-rescue skip, no-fabricated-fix) was designed for exactly this risk class. All 5 PASS → glm-5.2 can hold the qa contract.

**Evidence.**
- Calibration run: `target/eval-calibration/mika-qa-20260630-111518.{json,md}` — 5/5 PASS, single-shot, 3488 in / 4539 out tokens, 84.1s wall (verified present in worktree).
- Source precedent: `crates/mika-agent/src/well_known_agents.rs:166` (`MIKA_DEV_CONFIG`), `:103` (`config_toml: Some(MIKA_DEV_CONFIG)`), `:1860` (`test_mika_dev_config_toml_is_valid_toml`).
- Current state: `MIKA_QA` at `well_known_agents.rs:180` has `config_toml: None` (`:187`).
- Native provider env: `MIKA_ZAI_API_KEY` / `MIKA_ZAI_MODEL` (default `glm-5.2`) — root `CLAUDE.md` Environment Variables.
- Directory convention: `docs/eval/calibration/mika-dev-1633/mika-dev-glm-5.2-post-1633.{json,md}` (verified present).

**Divergence from mika-dev's config (intentional).** `MIKA_DEV_CONFIG` uses `llm_provider = "openrouter"` + `openrouter_model = "z-ai/glm-5.2"` (source still reflects the pre-native-provider era; runtime is on native zai — a known drift, out of scope per the ticket). This swap uses the **native** provider directly: `llm_provider = "zai"` + `zai_model = "glm-5.2"`. That is the current-correct shape and what the calibration run exercised (`"model": "zai/glm-5.2"` in the artifact).

---

## Requirements

- **R1.** Add a `MIKA_QA_CONFIG` const string with `llm_provider = "zai"` and `zai_model = "glm-5.2"`, parallel to `MIKA_DEV_CONFIG`. (AC1)
- **R2.** Set `MIKA_QA.config_toml = Some(MIKA_QA_CONFIG)` (currently `None`). (AC2)
- **R3.** Add a TOML-validity test mirroring `test_mika_dev_config_toml_is_valid_toml`, asserting provider/model fields. (AC3)
- **R4.** Commit the calibration artifacts under `docs/eval/calibration/mika-qa-1632/`. (AC4)
- **R5.** Apply the swap to the running agent's runtime config (`~/.mika/agents/mika-qa/config.toml`) so the live mika-qa picks up zai/glm-5.2 — verified by `mika --agent mika-qa ask "what model are you running?"`. (AC5)
- **R6.** No regression in existing happy-path qa-review behavior shape; calibration remains green on subsequent runs. (AC6)

---

## Key Technical Decisions

- **KTD1 — Native `zai` provider, not openrouter.** Use `llm_provider = "zai"` + `zai_model = "glm-5.2"`. Matches the provider the calibration run actually exercised and the current-correct routing (mika#1657). Do **not** copy mika-dev's openrouter shape — that would route through a different provider than what was calibrated.
- **KTD2 — `llm_max_tokens = 16384`.** The ticket specifies 16384 for mika-qa (vs mika-dev's 8192). qa-review emits per-AC enumeration + finding lists; the calibration's `absence_claim_grounding` scenario alone produced 1276 output tokens. 16384 leaves headroom; glm-5.2's context budget supports it. Carry the ticket's value.
- **KTD3 — Artifact filename mirrors the `-post-<ticket>` convention.** Name the committed files `mika-qa-glm-5.2-post-1632.{json,md}` (mirroring `mika-dev-glm-5.2-post-1633.{json,md}`), under `docs/eval/calibration/mika-qa-1632/`. The `1632` directory/suffix ties to the calibration-suite ticket (mika#1632), exactly as mika-dev-1633 ties to mika#1633. This is a deliberate, consistency-driven read of the ticket's looser `mika-qa-glm-5.2-post-calibration` suggestion — AC4 only pins the directory, not the filename.
- **KTD4 — Const placement + doc comment.** Place `MIKA_QA_CONFIG` adjacent to `MIKA_QA` (after the `MIKA_QA_IDENTITY` block or beside `MIKA_DEV_CONFIG`), with a doc comment citing mika#1670 + the 5/5 calibration result, mirroring the `MIKA_DEV_CONFIG` comment at `:164-165`.
- **KTD5 — Runtime apply is a manual operator step, documented but not code.** AC5 (`~/.mika/agents/mika-qa/config.toml`) is a deploy-time runtime mutation outside the Rust source. The reconcile path (`reconcile_well_known_config`, `well_known_agents.rs:658`) writes spec `config_toml` to the runtime file on provision when the spec defines a config — so after this change + a provision/deploy cycle the runtime file converges automatically. The plan documents the explicit apply + verification (AC5) as the Verification Contract, not as an Implementation Unit.

---

## Implementation Units

### U1. Add `MIKA_QA_CONFIG` and wire it onto the `MIKA_QA` spec

- **Goal:** Source-of-truth model swap for mika-qa. (R1, R2)
- **Dependencies:** none
- **Files:** `crates/mika-agent/src/well_known_agents.rs`
- **Approach:**
  - Add `const MIKA_QA_CONFIG: &str` with a doc comment mirroring `MIKA_DEV_CONFIG` (`:164-173`):
    ```toml
    # Mika QA — fabrication-catching review agent.
    # Base model switched to zai/glm-5.2 per mika#1670 calibration evidence (5/5 PASS).

    llm_provider = "zai"
    zai_model = "glm-5.2"
    llm_max_tokens = 16384
    log_level = "info"
    ```
  - Change `MIKA_QA.config_toml` from `None` (`:187`) to `Some(MIKA_QA_CONFIG)`.
- **Patterns to follow:** `MIKA_DEV_CONFIG` const + `MIKA_DEV.config_toml: Some(...)` at `:103`.
- **Test scenarios:** covered by U2. (Config wiring is exercised by the reconcile tests that already key off `spec.config_toml` — see `test_reconcile_well_known_config_*` at `:2816`+; no behavioral branch added here beyond `None → Some`.)
- **Verification:** `cargo build -p mika-agent` compiles; the const parses (U2 test).

### U2. TOML-validity test for `MIKA_QA_CONFIG`

- **Goal:** Regression-gate the swapped provider/model fields. (R3)
- **Dependencies:** U1
- **Files:** `crates/mika-agent/src/well_known_agents.rs` (`#[cfg(test)] mod tests`)
- **Approach:** Add `test_mika_qa_config_toml_is_valid_toml` mirroring `test_mika_dev_config_toml_is_valid_toml` (`:1859-1865`):
  - parse `MIKA_QA_CONFIG` via `toml::from_str`,
  - assert `llm_provider == "zai"`,
  - assert `zai_model == "glm-5.2"`.
- **Patterns to follow:** `test_mika_dev_config_toml_is_valid_toml` verbatim shape.
- **Test scenarios:**
  - Happy path: `MIKA_QA_CONFIG` parses as valid TOML and both field assertions hold.
  - (Implicit regression) a malformed const or wrong provider/model value fails the assertion — this is the guard's purpose.
- **Verification:** `cargo test -p mika-agent test_mika_qa_config_toml_is_valid_toml` passes.

### U3. Commit calibration evidence under `docs/eval/calibration/mika-qa-1632/`

- **Goal:** Durable, in-repo calibration record for the swap. (R4)
- **Dependencies:** none
- **Files:**
  - `docs/eval/calibration/mika-qa-1632/mika-qa-glm-5.2-post-1632.json` (copy of `target/eval-calibration/mika-qa-20260630-111518.json`)
  - `docs/eval/calibration/mika-qa-1632/mika-qa-glm-5.2-post-1632.md` (copy of `target/eval-calibration/mika-qa-20260630-111518.md`)
- **Approach:** `git`-tracked copy of the ephemeral `target/` artifacts into the committed calibration tree, byte-identical, mirroring `mika-dev-1633/`.
- **Test scenarios:** `Test expectation: none — committed evidence files, no behavioral change.`
- **Verification:** both files present in `git status`; content matches the `target/` originals (`diff` clean).

### U4. Doc-sync touchpoint check (no doc edits expected)

- **Goal:** Confirm no `docs/`-as-single-source-of-truth file needs a sync run for this change. (R6 hygiene)
- **Dependencies:** U1–U3
- **Files:** none expected
- **Approach:** This change touches Rust source + a new `docs/eval/calibration/` tree only. `docs/eval/calibration/` is **not** in the `crates/mika-agent/docs/` build-time include set (that path covers `architecture/`, `configuration/`, etc., not calibration evidence), so `scripts/sync-agent-docs.sh` + the CI `docs-sync` job are not triggered. Confirm by checking the build.rs include surface; if calibration docs were unexpectedly in scope, run the sync script.
- **Test scenarios:** `Test expectation: none — verification-only unit.`
- **Verification:** CI `docs-sync` job green on the PR (no unsynced-doc failure).

---

## Verification Contract

Run in the worktree before PR:

1. `cargo build -p mika-agent` — compiles with `config_toml: Some(MIKA_QA_CONFIG)`.
2. `cargo test -p mika-agent test_mika_qa_config_toml_is_valid_toml` — new test passes. (AC3)
3. `cargo test -p mika-agent well_known` — no regression in well-known-agent tests (allowlist parity, reconcile, etc.). (AC6)
4. `cargo clippy -p mika-agent` / `cargo fmt --check` — clean.
5. Artifact presence: `docs/eval/calibration/mika-qa-1632/mika-qa-glm-5.2-post-1632.{json,md}` tracked; byte-identical to `target/eval-calibration/mika-qa-20260630-111518.{json,md}`. (AC4)

**Runtime-apply verification (AC5 — deploy-time, operator/post-merge step, not gating the PR build):**

6. After `make deploy` (or `MIKA_DISABLE_AGENT_PROVISIONING` unset so reconcile writes the spec config), `~/.mika/agents/mika-qa/config.toml` shows `llm_provider = "zai"` + `zai_model = "glm-5.2"`.
7. `mika --agent mika-qa ask "what model are you running?"` confirms zai/glm-5.2.
8. A confirmatory `make calibrate-mika-qa MODEL=zai/glm-5.2` re-run stays 5/5 (AC6 — drift check).

---

## Definition of Done

- [ ] `MIKA_QA_CONFIG` const added with `llm_provider = "zai"` + `zai_model = "glm-5.2"` + `llm_max_tokens = 16384`.
- [ ] `MIKA_QA.config_toml = Some(MIKA_QA_CONFIG)`.
- [ ] `test_mika_qa_config_toml_is_valid_toml` added and passing.
- [ ] Calibration artifacts committed under `docs/eval/calibration/mika-qa-1632/`.
- [ ] `cargo build`, `cargo test` (well-known scope), `clippy`, `fmt` all clean.
- [ ] PR body documents the AC5 runtime-apply + verification steps as the post-merge/deploy action.

## Acceptance criteria

- [ ] AC1. `MIKA_QA_CONFIG` constant added with `llm_provider = "zai"` + `zai_model = "glm-5.2"`.
- [ ] AC2. `MIKA_QA.config_toml = Some(MIKA_QA_CONFIG)`.
- [ ] AC3. Validation test passes (mirrors mika-dev's test).
- [ ] AC4. Calibration artifacts committed under `docs/eval/calibration/mika-qa-1632/`.
- [ ] AC5. After `make deploy`, running `mika --agent mika-qa ask "what model are you running?"` confirms zai/glm-5.2.
- [ ] AC6. Existing happy-path qa-review behaviors unchanged in shape; no regression in calibration on subsequent runs.

---

## Scope Boundaries

**In scope:** source `config_toml` swap, validity test, committed calibration evidence, runtime-apply documentation/verification.

### Deferred to Follow-Up Work
- **mika-qa variant generation** (skill-review-tuned prompts for the 6 prompt-heavy skills: qa-review, qa-review-build-callback, build-mika, deploy-mika, self-check, dev-handsoff) — POST-swap, separate workstream. Per `feedback_skill_variant_gen_tied_to_runtime_model`: variant generation persists to the agent's runtime model and must be done **post-swap**, never as pre-swap prep.
- **Refactor `MIKA_DEV_CONFIG` openrouter → native zai** to fix the source/runtime drift — separate ticket if worth fixing.

---

## Sources & Research

- `crates/mika-agent/src/well_known_agents.rs` — `MIKA_DEV_CONFIG` (`:166`), `MIKA_DEV.config_toml` (`:103`), `MIKA_QA` (`:180`, `config_toml: None` at `:187`), `test_mika_dev_config_toml_is_valid_toml` (`:1859`), reconcile tests (`:2816`+).
- `target/eval-calibration/mika-qa-20260630-111518.{json,md}` — calibration evidence (5/5 PASS).
- `docs/eval/calibration/mika-dev-1633/` — directory + filename convention precedent.
- Root `CLAUDE.md` § Environment Variables (`MIKA_ZAI_API_KEY`/`MIKA_ZAI_MODEL`), § Model calibration (#1190).
- `crates/mika-agent/CLAUDE.md` § Evaluation — Model Calibration (#1190): mika-qa 5-scenario suite.
- Lineage: mika#1632 (calibration suite, PR #1644), mika#1190 (framework), mika#1633 (mika-dev swap, PR #1635), mika#1657 (native Z.AI provider).
