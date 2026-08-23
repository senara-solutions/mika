# Plan — fix(agent-honesty): stop Mika confabulating its own LLM model

**Status:** DRAFT
**Date:** 2026-08-22
**Ticket:** mika#1815
**Owner:** mika-orchestrator (Vincent + Claude Code, co-creators)
**Class:** Agent-honesty bug fix (prompt + tool + doctrine)
**Cross-refs:** mika#1784 (image-ingestion honesty — contrast case), memory `reference_mika_uses_glm_zai`, mika#1798 (non-transit doctrine — same surface: Mika must KNOW its own configuration)

## Why

Al (family tester, 2026-07-20) asked Mika "quel LLM utilises-tu ?". Mika first answered honestly ("je ne sais pas avec certitude … 11 providers … je ne sais pas lequel Vincent a configuré"), then Al said "oui, va voir", and Mika returned with a **confident wrong answer**:

> « Je tourne sur Anthropic — le modèle par défaut… la ligne anthropic_model est commentée, donc c'est le défaut. Vu la date et le contexte, c'est très probablement Claude Sonnet 4. »

Three layers of failure, verbatim from the ticket body:

1. **Verbe faux.** Mika said "je vais **VÉRIFIER** ma config" but delivered an **INFÉRENCE** (« très probablement », « vu la date et le contexte », reasoning from a commented-out line). It **did not read its runtime.**
2. **Inférence fausse.** Mika runs on **GLM (z.ai)**, not Anthropic/Claude (per memory `reference_mika_uses_glm_zai`, the z.ai single-provider flip). "Je tourne sur Anthropic / Claude Sonnet 4" **contradicts the real setup** → confabulation of a false fact **about itself**.
3. **Incohérence interne.** "je ne sais pas avec certitude" → then "je tourne sur Anthropic" with confidence. Guess disguised as observation.

**Root cause (verified in codebase, not inferred):**

- `Settings::active_llm_config()` (`crates/mika-common/src/config.rs:1281`) and `Settings::active_model_display()` (line 1313) already resolve the runtime-active provider + model deterministically. `LlmProvider` trait (`crates/mika-common/src/llm/mod.rs:213-238`) exposes `provider_name()` + `model_name()` on every provider impl. Ground truth exists inside the process at every turn.
- **Ground truth is never exposed to the agent's prompt.** `PromptContext` (`crates/mika-agent/src/prompt.rs:557`) has no field carrying provider/model. `write_identity_section()` renders `## Identity\nYou are {name}.` and stops. Compact-provider prompt is identical shape.
- **No tool exposes runtime LLM identity.** `default_tools()` (`crates/mika-agent/src/tools/mod.rs:801`) registers `get_config` (customer-config KV table only), `read_agent_file` (filesystem), etc. `ToolContext` holds `provider_name` + `model_name` (line 128-133) but no tool surfaces them. The agent must therefore guess.

The virtue Mika showed on mika#1784 (image-ingestion honesty — "je ne vois pas l'image") is missing on self-identity. **Anti-fabrication must apply self-referentially.**

## What

Four coordinated changes: (1) inject ground-truth model identity into the system prompt on every turn, (2) add a `get_active_llm` builtin tool that returns the same ground truth on demand, (3) add prompt-level "Self-Identity Discipline" rules that block confabulation and mandate the fact-vs-inference verb split, (4) two anti-confabulation snapshot tests that pin the discipline shape.

### 1. `PromptContext` extension — carry active LLM identity through prompt assembly

**Files:**
- `crates/mika-common/src/config.rs` — no change (`active_llm_config` + `active_model_display` already there)
- `crates/mika-agent/src/prompt.rs` — extend `PromptContext` + write a new `## Runtime` section
- `crates/mika-agent/src/agent_loop/mod.rs` — populate the new fields at both `PromptContext` construction sites (line 3007, line 4536)

**Change shape (`prompt.rs`):**

```rust
pub struct PromptContext<'a> {
    // ... existing fields ...
    /// Runtime LLM identity — provider + model of the LLM currently powering
    /// this turn. Populated from `llm.provider_name()` / `llm.model_name()` at
    /// the agent-loop turn-start (matches Settings::active_llm_config, but sourced
    /// from the live `LlmProvider` instance for one-source-of-truth).
    /// This is ground truth for "which LLM am I?"; the Self-Identity section
    /// consumes it directly.
    pub runtime_provider: &'a str,
    pub runtime_model: &'a str,
}

fn write_runtime_section(prompt: &mut String, provider: &str, model: &str) {
    prompt.push_str("## Runtime\n");
    writeln!(prompt, "You are currently running on provider `{provider}` model `{model}`.").unwrap();
    prompt.push_str(
        "This is the ground truth for questions about your own LLM/model. \
         Do NOT infer your model from commented-out config lines, defaults, or \"probably\" reasoning. \
         If a user asks which model you use, quote this line verbatim.\n\n",
    );
}
```

- `build_system_prompt` writes the Runtime section after `write_identity_section` and before `write_time_section` (so identity + runtime read as a coherent block).
- `build_compact_system_prompt` also renders a one-line variant: `## Runtime\nProvider `zai`, model `glm-5.2`.\n\n` — the compact provider is `mikamodel` today, but the runtime rule applies uniformly.
- Both prompt-build callsites in `agent_loop/mod.rs` (line 3007, line 4536) populate the new fields from `llm.provider_name()` / `llm.model_name()`. The `SilentPromptContext` gets the same fields + same section (heartbeat/reflection can also be asked "what model are you?").

### 2. `get_active_llm` builtin tool — on-demand ground-truth introspection

**File:** new `crates/mika-agent/src/tools/get_active_llm.rs`; register in `default_tools()` (`crates/mika-agent/src/tools/mod.rs:801`).

**Shape (mirrors `get_config` — small, no-arg, read-only):**

```rust
pub struct GetActiveLlmTool;

#[async_trait]
impl Tool for GetActiveLlmTool {
    fn name(&self) -> &str { "get_active_llm" }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_active_llm".to_string(),
            description: "Return the runtime-active LLM provider and model powering \
                this agent turn. Use this when the user asks about your model / LLM / \
                provider and you want to VERIFY (not INFER). The system prompt's \
                ## Runtime section carries the same information — this tool is the \
                on-demand verifier when a user explicitly asks you to 'go check'.".to_string(),
            input_schema: serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
        }
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput> {
        Ok(ToolOutput::success(format!(
            "Active LLM: provider=`{}`, model=`{}` (runtime source: live LlmProvider instance).",
            ctx.provider_name, ctx.model_name,
        )))
    }
}
```

- Registered in `default_tools()` so every agent (including delegates and team members) has access — self-identity honesty is engine-level, not skill-level.
- Read-only classification: added to `is_read_tool()` in `crates/mika-agent/src/tools/classification.rs` (mandatory per `test_every_builtin_tool_has_explicit_classification`).
- Return format keeps `provider=` / `model=` verbatim from `ToolContext` — same source the Runtime prompt section quotes, so the two channels cannot drift.

### 3. Self-Identity discipline — prompt-level rule

**File:** `crates/mika-agent/src/prompt.rs`, appended to the Instructions section (search for existing `## Instructions` block in `build_system_prompt`; if none present in current structure, add one below `## Runtime`).

**Section text:**

```markdown
## Self-Identity Discipline

When a user asks about YOU — which model you are, which provider powers you,
your configuration, your capabilities — the ground truth is the `## Runtime`
section above, populated from your live LLM instance. Follow these rules:

1. **Quote, don't infer.** For "which model / LLM are you?" quote the Runtime
   section (or call `get_active_llm`). Do NOT reason from commented-out config
   lines, "probably", "vu le contexte", or default fallbacks.

2. **Verb discipline.** "I will VERIFY" implies reading the source of truth
   (Runtime section, get_active_llm, get_config, read_agent_file). "I will
   GUESS / INFER" implies reasoning without a read. Never say VERIFY and then
   deliver an INFERENCE.

3. **Fallback honestly.** If ground truth is genuinely unavailable
   (Runtime section absent AND get_active_llm errors), say « I cannot
   reliably determine my model » and point at where the configuration
   lives (e.g. `~/.mika/config.toml`). Never fabricate a confident answer.

4. **Consistency across a single turn.** You may not say "I don't know
   with certainty" and then in the next paragraph assert a model with
   confidence. Uncertainty at t=0 and confidence at t=1 within the same
   response is confabulation.

This applies self-referentially: the anti-fabrication virtue you extend to
user-facing tasks (phone numbers, image contents, file paths) MUST also
apply to facts about yourself. Contrast reference: mika#1784 (you correctly
refused to fabricate what an unseen image contained — apply the same
integrity to self-identity questions).
```

The section anchors on the Runtime data (rule 1 quotes it, rule 3 falls back when it's missing). This is what makes the doctrine mechanical rather than aspirational — the answer is *right there* in the prompt.

### 4. Anti-confabulation tests — pin the discipline shape

**File:** `crates/mika-agent/src/prompt.rs` `#[cfg(test)] mod tests` (extend existing).

Three new tests:

```rust
#[test]
fn runtime_section_carries_ground_truth_from_context() {
    let ctx = PromptContext { /* … runtime_provider: "zai", runtime_model: "glm-5.2" … */ };
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Runtime"));
    assert!(prompt.contains("provider `zai`"));
    assert!(prompt.contains("model `glm-5.2`"));
    // The rule text is present (structural, not paraphrase-tolerant).
    assert!(prompt.contains("ground truth for questions about your own LLM/model"));
    assert!(prompt.contains("Do NOT infer your model from commented-out config lines"));
}

#[test]
fn self_identity_discipline_section_present_and_ordered_after_runtime() {
    let ctx = PromptContext { /* runtime_provider: "anthropic", runtime_model: "claude-sonnet-4-6" */ };
    let prompt = build_system_prompt(&ctx);
    let runtime_pos = prompt.find("## Runtime").expect("Runtime section present");
    let discipline_pos = prompt.find("## Self-Identity Discipline").expect("discipline section present");
    assert!(runtime_pos < discipline_pos, "Runtime must precede discipline (rules quote Runtime data)");
    // All four rules present verbatim.
    assert!(prompt.contains("Quote, don't infer"));
    assert!(prompt.contains("Verb discipline"));
    assert!(prompt.contains("Fallback honestly"));
    assert!(prompt.contains("Consistency across a single turn"));
}

#[test]
fn compact_prompt_also_carries_runtime_line() {
    let ctx = PromptContext { /* runtime_provider: "mikamodel", runtime_model: "wizzard-v1" */ };
    let prompt = build_compact_system_prompt(&ctx);
    assert!(prompt.contains("## Runtime"));
    assert!(prompt.contains("mikamodel"));
    assert!(prompt.contains("wizzard-v1"));
}
```

Plus one test on the new tool (mirroring `get_config`'s test shape):

```rust
// crates/mika-agent/src/tools/get_active_llm.rs #[cfg(test)] mod tests
#[tokio::test]
async fn returns_provider_and_model_from_context() {
    let harness = TestHarness::new();
    // TestHarness ctx sets provider_name / model_name — verify a couple of fixture values.
    let ctx = harness.ctx_with_llm("zai", "glm-5.2");
    let out = GetActiveLlmTool.execute(json!({}), &ctx).await.unwrap();
    assert!(!out.is_error);
    assert!(out.content.contains("provider=`zai`"));
    assert!(out.content.contains("model=`glm-5.2`"));
}
```

And a classification test (auto-covered by the existing `test_every_builtin_tool_has_explicit_classification` — need only add `"get_active_llm"` to `is_read_tool()`'s match arm).

## Acceptance Criteria (verbatim from ticket)

1. **Mika a un mécanisme pour lire son modèle actif au runtime (via config ou introspection LLM).**
   - Satisfied by `get_active_llm` tool (on-demand read) AND `## Runtime` prompt section (per-turn injection). Both read from the same source (`ToolContext::{provider_name, model_name}` / `llm.provider_name() + llm.model_name()`). Source of truth is `Settings::active_llm_config()` resolved once at LLM-instance construction and carried through.

2. **Test : user demande « quel LLM ? » → Mika répond avec ground truth (le modèle réel du config résolu, pas une devinette).**
   - Satisfied by `runtime_section_carries_ground_truth_from_context` (proves the runtime data is in the prompt) + the tool test (proves on-demand introspection works). The behavioural test — LLM-driven end-to-end — is documented as an eval fixture: `crates/mika-agent/tests/evals/self_identity_ground_truth.yaml` (Yes-shape: user asks "which LLM are you?"; expected: response contains the actual provider + model from the runtime context; assertion: substring match on both provider and model tokens as configured in the eval harness). Deferred to Phase 2 if the eval harness scaffold isn't already Rust-native (per `feedback_no_provider_prompts` — plan lands the prompt + tool + unit tests now; eval-suite tie-in follows the eval convention already in the crate).

3. **Fallback honnête : si Mika ne peut techniquement pas lire son modèle (env limitations) → réponse explicite « je ne peux pas le déterminer de façon fiable, mais voici où c'est configuré », pas une devinette confiante.**
   - Satisfied by Self-Identity Discipline rule 3 (prompt-level directive). Rule 3 has structural teeth via the Runtime section being ALWAYS present in production (populated from the live LLM instance which cannot exist without a resolved provider). The rule fires as a fallback only for genuinely broken states (get_active_llm tool errors + Runtime section missing) — expected shape is a mechanism the agent has, not a scenario it should hit.

4. **Cross-test avec mika#1784 shape : préserver l'honnêteté observée (pas de fabrication self-image ni sur les tâches user).**
   - No changes to image-handling or user-task behavior. The prompt changes are additive (new sections), the tool is additive (new registered tool), the discipline is self-referential (mentions mika#1784 explicitly in the section text as the contrast anchor). Existing prompt tests remain green (verified in test suite as no-diff on non-runtime sections).

5. **Al re-teste : Mika répond « GLM (z.ai) » ou « je ne peux pas déterminer, mais config est ici » — jamais « très probablement Claude Sonnet 4 ».**
   - Acceptance is behavioural. Verification path: after merge + deploy, retest with Al (or the operator running Vincent's local mika instance) with the same "quel LLM utilises-tu ? va voir" sequence. Expected: Mika quotes the Runtime section (zai / glm-5.2). If the response drifts, the failure is a prompt-adherence issue and reopens the ticket. Documented as a manual acceptance step in the PR body; the unit tests above prove the mechanism is in place.

## Definition of Done

- [ ] `PromptContext` has `runtime_provider` + `runtime_model` fields; `build_system_prompt` writes the `## Runtime` section between `## Identity` and `## Current Time`.
- [ ] `build_compact_system_prompt` writes the compact Runtime line.
- [ ] `SilentPromptContext` extended identically; `build_silent_prompt` writes the same Runtime section (heartbeat/reflection self-identity coverage).
- [ ] Both `PromptContext` construction sites in `crates/mika-agent/src/agent_loop/mod.rs` (line 3007, line 4536) populate `runtime_provider` / `runtime_model` from `llm.provider_name()` / `llm.model_name()`. Silent prompt construction site (line 3932) same.
- [ ] `crates/mika-agent/src/tools/get_active_llm.rs` exists implementing `Tool`; registered in `default_tools()`; added to `is_read_tool()` in `classification.rs`.
- [ ] `## Self-Identity Discipline` section present in `build_system_prompt` output after `## Runtime`.
- [ ] Four new unit tests in `prompt.rs` + one in `get_active_llm.rs` pass.
- [ ] `cargo test -p mika-agent --lib prompt` clean.
- [ ] `cargo test -p mika-agent --lib tools::get_active_llm` clean.
- [ ] `cargo test -p mika-agent --lib tools::classification` clean (structural test `test_every_builtin_tool_has_explicit_classification`).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo fmt --all --check` clean.
- [ ] PR body documents manual acceptance step: retest with the "quel LLM ?" prompt on a fresh chat; expected substring match on `zai` and `glm-5.2` (or whatever the runtime resolves to in the deploy env).

## Injection verification (per `feedback_verify_pipeline_passes_without_the_fix`)

For each new anti-confabulation guard, verify the test fails without the fix, then restore:

1. **Runtime section carries ground truth** — temporarily replace `runtime_provider` with a hardcoded `"UNKNOWN"` in `write_runtime_section`; verify `runtime_section_carries_ground_truth_from_context` fails on the provider assertion; restore.
2. **Discipline ordering** — temporarily reorder so discipline writes before runtime; verify `self_identity_discipline_section_present_and_ordered_after_runtime` fails on the ordering assertion; restore.
3. **Tool returns context values** — temporarily hardcode return string as `"provider=\`anthropic\`, model=\`claude-sonnet-4\`"`; verify the tool test fails for the zai/glm-5.2 fixture; restore.

Document the three inversions + restorations in `todos/1815-injection-verification.md` per the convention already established for milestone plans (see mika#1945 pattern).

## Out of scope

- **Deep-provider introspection** (calling the actual API to echo back the model). The LLM trait's `model_name()` returns the *configured* model, not an API round-trip. If the user configures `zai_model = "glm-5.2"` but z.ai is silently serving a different variant, that's an upstream-provider drift issue, not a self-identity confabulation issue. Adding an API-echo tier is a separate follow-up if warranted (would need `LlmProvider::ping_active_model()` or equivalent).
- **Prompt-injection defense on the Runtime section.** The values come from configuration, not user input, so injection risk is negligible. Existing sanitize patterns (`sanitize_label`) not needed here.
- **Reflecting `provider_fields` to the agent** for "which OTHER providers are configured?" questions. The ticket is scoped to self-identity ("which model are YOU?"), not config surveys. Follow-up work if surface pressure emerges.
- **Behavioural eval integration.** The eval fixture is mentioned as a follow-up path in AC2 — the plan lands the mechanism (prompt + tool + unit tests) now. Eval integration ties into the crate's Rust-native eval scaffold whose current state is beyond this plan's scope.

## Risks and mitigations

- **Prompt-adherence drift** — the agent might see the Runtime section and still say "Claude Sonnet 4" if the underlying model is a known-fabricator. Mitigation: the Self-Identity Discipline section (rule 1 "Quote, don't infer") is directive-shaped; combined with the tool for explicit "va voir" scenarios, the mechanism is triple-redundant (prompt data + prompt rule + on-demand tool). If drift persists on a specific model, that's a swap-gate concern per `feedback_model_quirk_catch_belongs_at_swap_gate_not_engine` — not an engine-code fix.
- **Compact-prompt size budget** — the compact prompt is optimised for MikaModel's 512-byte capacity. The Runtime line adds ~50 bytes. Verified: current compact output is well under 512 bytes; new line stays in budget.
- **Silent-mode prompt drift** — heartbeat/reflection prompts also get the section. This is desired (self-identity honesty should be uniform), but adds bytes to already-large silent prompts. No mitigation needed unless we observe token-budget pressure.

## Related solutions

- `docs/solutions/architecture-patterns/memory-aware-introspection-tool-pattern.md` — the pattern this plan follows for the `get_active_llm` tool (small, no-arg, read-only, registered in `default_tools`).
- `docs/solutions/best-practices/opaque-tool-errors-invite-llm-fabrication-2026-07-26.md` — same failure family: "opaque source of truth invites LLM to fabricate a plausible answer". This plan closes the analogous gap for self-identity.
- `docs/solutions/692-self-knowledge-kg-upgrade.md` — historical self-knowledge work; this ticket extends the pattern to runtime LLM identity specifically.

## Compounding potential

After merge, capture in `docs/solutions/best-practices/`:

- **Self-identity-honesty pattern** (~50-line note): the tri-layer approach (prompt data + prompt rule + on-demand tool) as the canonical shape for any "what is my X?" question the agent might be asked. Analog: extend to "which repo am I working on?", "which agent am I?" etc.
- **Verb-discipline in prompts**: the "VERIFY vs INFER" split as a general prompt-authoring pattern for any place we want to force a read over a guess.
