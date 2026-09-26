---
title: "Harden write_skill_variant: structural fixes for hallucinated writes, wrong paths, and silent truncation"
category: architecture-patterns
date: 2026-04-07
tags: [skills, builtin-handler, write-skill-variant, review-skill, payload-cap, generated-variants, srp]
---

# Harden write_skill_variant: structural fixes for hallucinated writes, wrong paths, and silent truncation

## Problem

After landing the `review_skill` builtin (see [adding-skill-review-builtin-handler](./adding-skill-review-builtin-handler.md)) the autonomous skill-review loop kept failing in four observable modes:

| Failure | Observed |
|---|---|
| Variant not written / pure hallucination | Agent reported "variant generated" but no file existed |
| Wrong target model (minimax writes for gemini) | A `qa-review/google/gemini-2.5-flash/system_prompt.md` was authored by minimax impersonating Gemini |
| Silent truncation (half-size variant written) | Variants landed at ~50% the source size with no warning |
| Variant generated but not written | The agent ended its turn after `review_skill` without ever calling the write step |

## Root Cause / Design Challenge

The original two-step flow was held together by the model:

1. `review_skill` returned a `variant_path` field plus `provider`/`model` fields.
2. The agent was expected to call `write_agent_file` with that path.

Three structural weaknesses converged:

- **No required-tools gate** — nothing in the engine enforced that the write actually happened. If the model decided it had "done enough" after `review_skill`, the variant never made it to disk.
- **Path comes from agent context** — `variant_path` was an output, but the agent treated it as something it could reconstruct or substitute. When the model fabricated provider/model strings, the wrong path was used.
- **Generic 10K control-field cap on a file body** — `write_agent_file`'s `content` parameter shared the `MAX_INPUT_LEN = 10_000` control-field cap. Anything bigger than ~10 KB was silently truncated by the LLM before it ever reached the tool — the truncation was invisible to both ends.

The deeper design lesson: **prompt-level enforcement of load-bearing constraints is fragile** (per `feedback_prompt_enforcement_fragile`). When a behaviour is required for correctness, it has to be made structurally impossible to violate.

## Solution

Three layered fixes, ordered by dependency and blast radius. Layers A, B, C ship in `mika`; D and E (skill-review prompt + bogus variant cleanup) ship in `mika-skills` as a follow-up.

### Layer A — Payload field cap (independent, must-fix)

`crates/mika-agent/src/tools/mod.rs`

- Introduced `MAX_PAYLOAD_BYTES = 200 * 1024` (200 KB) alongside the existing `MAX_INPUT_LEN = 10_000`.
- `write_agent_file` and `write_workspace` now apply `MAX_PAYLOAD_BYTES` to their `content` field. Control fields (paths, names, queries) keep the 10 K cap.
- Documented in `CLAUDE.md` Tools section.

Deliberate simplification: the original brainstorm proposed a per-tool `payload_fields() -> &[&str]` hook to declare payload fields. The codebase has no central enforcement layer (each tool checks its own fields inline), so the hook would have had no consumer. Adding it would have been speculative abstraction. Per `feedback_keep_simple`, the right amount of complexity is what the task requires — two inline edits and a shared constant.

### Layer B — `write_skill_variant` builtin (structural fix)

`crates/mika-agent/src/skills/builtin_handlers.rs`

A new builtin with **no path input**. The full input schema is `{ skill_name, content, force? }`. The target path is computed entirely from `ctx.provider_name` / `ctx.model_name`, with a hard-coded `generated/` segment:

```text
skills/<skill_name>/generated/<provider>/<sanitized_model>/system_prompt.md
```

This makes path fabrication structurally impossible: the agent cannot supply a wrong path because there is no path parameter. All four runtime safety properties are co-located in one function:

1. `skill_name` validation (length, no traversal, no symlink, must exist)
2. Canonical provider/model resolution via the existing `resolve_canonical_provider_model()` (handles OpenRouter aggregator namespacing)
3. Truncation guard: reads the source `system_prompt.md` size from disk and rejects content smaller than `MIN_VARIANT_RATIO = 0.5` of the source
4. Refuses overwrite unless `force = true`

**`review_skill` cleanup** in the same file:
- Dropped the `variant_path` response field — the agent no longer sees a writable target path at all.
- Renamed `provider` → `runtime_provider` and `model` → `runtime_model`. The `runtime_` prefix signals these are observations, not inputs.
- Added `next_action` field instructing the agent to call `write_skill_variant`.
- The existing-variant lookup now reads from `generated/<provider>/<model>/`, matching the new write path.
- `MAX_PROMPT_IN_RESPONSE = 8_000` cap on `root_prompt` is preserved — it's safe now because `write_skill_variant` reads the source fresh from disk for the truncation check, so response truncation can no longer affect write integrity.

### Layer C — Registry: load `generated/` variants

`crates/mika-agent/src/skills/index.rs`

- `SkillEntry` gains a parallel `generated_model_prompts: HashMap<String, String>` map alongside the existing hand-authored `model_prompts` map.
- New `scan_generated_variants()` walks `<skill_dir>/generated/<provider>/<model>/system_prompt.md` at scan time, gating providers via `ProviderKind::parse` (same gate as the hand-authored scan).
- `resolve_prompt(provider, model)` fallback chain becomes: hand-authored model variant → generated variant → root prompt. **Hand-authored entries always win** so generated content can never silently shadow human curation.

### Tests

10 new unit tests covering each safety property:

| Test | Asserts |
|---|---|
| `test_write_skill_variant_uses_runtime_model` | Path derives from `ctx`, not from any input |
| `test_write_skill_variant_writes_under_generated` | Resolved path always contains `/generated/` segment |
| `test_write_skill_variant_canonicalises_openrouter` | `openrouter/minimax/minimax-m2.7` → `generated/minimax/minimax-m2.7/` |
| `test_write_skill_variant_path_traversal_rejected` | Rejects `..`, `/`, `\`, null byte |
| `test_write_skill_variant_refuses_linked_skill` | Symlink check |
| `test_write_skill_variant_no_overwrite_without_force` | Rejects second write; accepts with `force=true` |
| `test_write_skill_variant_truncation_rejected` | Content < 50% source size → hard reject |
| `test_write_agent_file_large_payload` | 50 KB body succeeds; 210 KB rejected |
| `test_resolve_prompt_handauthored_beats_generated` | Both present → hand-authored wins |
| `test_resolve_prompt_falls_back_to_generated` | Only generated present → used |

Full mika-agent suite: 1365 passed, 0 failed. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.

## Lessons Learned

### Make load-bearing constraints structurally impossible to violate

The pattern that worked here is the same one in `validate_and_resolve_path()`: the function signature itself prevents the misuse. Removing the `path` input from `write_skill_variant` is more powerful than any prompt instruction telling the agent "use the right path". The agent's only way to be wrong is to be wrong about which skill it's variant-generating for — and that's already validated against the filesystem.

When an autonomous loop is failing, look for "the model is supposed to" in the contract. Each one of those is a place where a structural fix is possible.

### Control fields and payload fields need distinct caps

A single `MAX_INPUT_LEN` for everything is a footgun the moment a tool starts accepting file bodies. The fix is two constants and per-call discipline at the few sites that handle payloads — not a generic abstraction. Reach for the abstraction only when there's a third payload-bearing tool.

### "Hand-authored beats generated" must be enforced at resolution time, not at write time

Generated variants live alongside hand-authored ones. Writing to `generated/` is the right boundary, but resolution still has to prefer hand-authored content explicitly. Two parallel maps + a fallback chain is simpler than encoding precedence into the file layout.

### `runtime_` prefix on response fields signals "observation, not input"

When a tool response carries fields the agent might mistake for inputs, naming them with `runtime_` prefix (`runtime_provider`, `runtime_model`) reduces the confusion at zero cost. This is a small thing that compounds — the agent stops trying to "set" the runtime model.

## Files Modified

- `crates/mika-agent/src/tools/mod.rs` — `MAX_PAYLOAD_BYTES` constant
- `crates/mika-agent/src/tools/write_agent_file.rs` — switched to payload cap
- `crates/mika-agent/src/tools/write_workspace.rs` — switched to payload cap
- `crates/mika-agent/src/skills/builtin_handlers.rs` — new `write_skill_variant`, `review_skill` cleanup, dispatch wiring
- `crates/mika-agent/src/skills/index.rs` — `generated_model_prompts` map, `scan_generated_variants`, fallback chain
- `crates/mika-agent/src/skills/{matcher.rs,mod.rs}`, `crates/mika-agent/src/agent.rs` — test fixture initialisation for new field
- `CLAUDE.md` — Tools convention update

## Related

- senara-solutions/mika#469 — implementation issue
- senara-solutions/mika#470 — this PR
- senara-solutions/mika-skills#99 — companion follow-up: switch skill-review prompt to call `write_skill_variant`, add `[constraints] required_tools` gate, delete the bogus `qa-review/google/gemini-2.5-flash` variant
- [adding-skill-review-builtin-handler.md](./adding-skill-review-builtin-handler.md) — the original `review_skill` design
- `feedback_prompt_enforcement_fragile` — the institutional learning that drove the structural-fix approach
- `feedback_keep_simple` — drove the decision to skip the speculative `payload_fields()` hook
