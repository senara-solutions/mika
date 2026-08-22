# mika#1815 — Injection verification

Per `feedback_verify_pipeline_passes_without_the_fix`, each anti-confabulation
guard added by this PR was temporarily inverted, the corresponding test was
run against the inverted code to prove it fails, then the fix was restored.
All three inversions caught the fault correctly.

## Guard 1 — Runtime section carries ground truth

**Inversion:** In `crates/mika-agent/src/prompt.rs::write_runtime_section`,
replace the `{provider}` interpolation with a hardcoded string `UNKNOWN`:

```rust
// before
"You are currently running on provider `{provider}` model `{model}`."
// inverted
"You are currently running on provider `UNKNOWN` model `{model}`."
```

**Expected failure:** `prompt::tests::mika1815_runtime_section_carries_ground_truth_from_context`
fails on the `provider `zai`` substring assertion (the section now says
`UNKNOWN` regardless of ctx input).

**Observed:**
```
failures:
    prompt::tests::mika1815_runtime_section_carries_ground_truth_from_context

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; ...
```

**Restored.** Post-restore run: PASS.

## Guard 2 — Discipline ordered AFTER Runtime

**Inversion:** In `crates/mika-agent/src/prompt.rs::build_system_prompt`,
swap the two `write_*` calls so discipline is written before runtime:

```rust
// before
write_runtime_section(&mut prompt, ctx.runtime_provider, ctx.runtime_model);
write_self_identity_discipline_section(&mut prompt);
// inverted
write_self_identity_discipline_section(&mut prompt);
write_runtime_section(&mut prompt, ctx.runtime_provider, ctx.runtime_model);
```

**Expected failure:** `prompt::tests::mika1815_self_identity_discipline_section_present_and_ordered_after_runtime`
fails on the `runtime_pos < discipline_pos` assertion (order flipped).

**Observed:**
```
failures:
    prompt::tests::mika1815_self_identity_discipline_section_present_and_ordered_after_runtime

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; ...
```

**Restored.** Post-restore run: PASS.

## Guard 3 — Tool returns ground truth from ToolContext

**Inversion:** In `crates/mika-agent/src/tools/get_active_llm.rs::execute`,
replace the ctx-driven `format!` with a hardcoded Anthropic response:

```rust
// before
Ok(ToolOutput::success(format!(
    "Active LLM: provider=`{}`, model=`{}` (runtime source: live LlmProvider instance).",
    ctx.provider_name, ctx.model_name,
)))
// inverted
Ok(ToolOutput::success(
    "Active LLM: provider=`anthropic`, model=`claude-sonnet-4` (runtime source: live LlmProvider instance).".to_string(),
))
```

**Expected failure:** `tools::get_active_llm::tests::returns_ground_truth_for_zai_glm`
fails because the tool now returns `anthropic`/`claude-sonnet-4` regardless
of the ToolContext's `provider_name`/`model_name`. The default-context test
`returns_provider_and_model_from_context` also fails because the harness
default is `claude-sonnet-4-6`, not the hardcoded `claude-sonnet-4`.

**Observed:**
```
failures:
    tools::get_active_llm::tests::returns_ground_truth_for_zai_glm
    tools::get_active_llm::tests::returns_provider_and_model_from_context

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; ...
```

**Restored.** Post-restore run: PASS.

## Post-verification full-suite run

`cargo test -p mika-agent --lib -- prompt::tests::mika1815 tools::get_active_llm`
→ 7 passed, 0 failed.

All three inversions were caught by the intended test; each fix is
verifiably load-bearing.
