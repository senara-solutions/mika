---
title: "fix: Harden review_skill / write_skill_variant"
type: fix
status: active
date: 2026-04-07
issue: senara-solutions/mika#469
---

# Harden `review_skill` / `write_skill_variant`

Closes #469.

## Context

The skill-review autonomous loop has been failing in four observable modes:

| Failure | Root cause |
|---|---|
| Variant not written / pure hallucination | Two-step flow held together by the model — nothing enforces the write happens |
| Wrong target model (minimax writes for gemini) | Agent constructs the path from context it fabricated, not from `ctx` |
| Silent truncation (half-size variant written) | Generic 10K char cap applied to file body payloads |
| "Variant generated but not written" | Same as failure #1 — no required-tools gate on the write step |

This plan ships Layers A, B, C in **mika** as a single PR. Layers D and E (skill-review prompt update + bogus-variant cleanup) ship as a follow-up PR in **mika-skills** (tracked by senara-solutions/mika-skills#99) — they depend on the new `write_skill_variant` builtin existing in the runtime, so they merge after this PR.

The plan is pre-validated by the user. We adopt it verbatim rather than re-deriving.

## Layer A — Fix the 10K cap on file payloads (must-fix, independent)

**File:** `crates/mika-agent/src/tools/mod.rs`

The current 10K char cap is documented in `CLAUDE.md` as a generic input guard. It is appropriate for control fields (path, name, query). It is wrong for fields that carry file bodies.

- Introduce a per-tool `payload_fields() -> &[&str]` hook on the `Tool` trait, default empty, analogous to `timeout_secs()`. Tools declare which fields carry file bodies.
- `write_agent_file` and `write_workspace` declare `content` as a payload field.
- Payload fields bypass the 10K control-field check and apply a separate `MAX_PAYLOAD_BYTES = 204_800` (200 KB) cap.
- Control fields keep the existing 10K cap.
- Update the `Conventions → Tools` section of `mika/CLAUDE.md`: *"`content` fields are payload fields; cap is 200 KB. All other fields: 10K."*

## Layer B — Replace the two-step write with `write_skill_variant`

**File:** `crates/mika-agent/src/skills/builtin_handlers.rs`

A dedicated builtin is preferred over a `commit` mode parameter on `review_skill` because:
- A mode parameter on one builtin violates SRP — one function doing read and write.
- `write_skill_variant` with **no path input** makes path fabrication structurally impossible: the agent cannot supply a wrong path because there is no path parameter.

### `write_skill_variant` spec

**Inputs:** `{ skill_name: string, content: string, force?: bool }`
**No path input.** Path is computed entirely from `ctx.provider_name` / `ctx.model_name`.

Implementation:
1. Validate `skill_name` (same suite as `review_skill`: no traversal, no symlink, skill must exist).
2. Call `resolve_canonical_provider_model(ctx.provider_name, ctx.model_name)` — strips `openrouter/` prefix, splits namespace.
3. Call `sanitize_model_dir_name()` on the model segment.
4. Build path: `skills_dir/<skill_name>/generated/<provider>/<sanitized_model>/system_prompt.md`. The `generated/` segment is hard-coded — no input can move the write outside it.
5. **Truncation guard:** read `skills_dir/<skill_name>/system_prompt.md` size from disk. If `content.len() < MIN_VARIANT_RATIO * source_size` (`MIN_VARIANT_RATIO = 0.5`), reject with: *"variant is N% the size of source — looks like truncation; re-emit with full content."*
6. Refuse overwrite unless `force == true`.
7. Create parent dirs, write the file.
8. Return `{ written_path, provider, model, content_bytes, source_bytes }` so the agent can verify.

Register in `KNOWN_BUILTINS` and add to the dispatch `match`.

### `review_skill` cleanup

- Drop any field the agent could use as a writable target path.
- Rename `provider_name` → `runtime_provider`, `model_name` → `runtime_model` in the response so the agent cannot mistake them for inputs.
- Add `"next_action": "Call write_skill_variant with skill_name and content"` to the response payload.
- Keep `root_prompt` capped at the existing `MAX_PROMPT_IN_RESPONSE = 8_000`. Response truncation no longer affects write integrity because `write_skill_variant` reads the source fresh from disk for the truncation check.

## Layer C — Registry: load `generated/` variants

**File:** `crates/mika-agent/src/skills/registry.rs`

Extend `SkillEntry` scan to also load `generated/<provider>/<model>/system_prompt.md` into a parallel `generated_model_prompts` map alongside the existing hand-authored `model_prompts` map.

Update `resolve_prompt(provider, model)` fallback chain:
1. Hand-authored `<provider>/<model>/` → wins always
2. Hand-authored `<provider>/` (provider-level) — no longer supported per current code; keep semantics as-is
3. **Generated `generated/<provider>/<model>/`** ← new
4. Root `system_prompt.md`

`mika skills info` / `mika skills list`: mark generated variants distinctly from hand-authored, e.g. `[variants: 2 hand, 1 generated]`.

`effective_timeout()`: unchanged — generated variants do not carry `skill.toml` overrides.

## Files modified

- `crates/mika-agent/src/tools/mod.rs` — `payload_fields()` hook, `MAX_PAYLOAD_BYTES`, validation split
- `crates/mika-agent/src/tools/write_agent_file.rs` (or wherever defined) — declare `content` as payload field
- `crates/mika-agent/src/tools/write_workspace.rs` — declare `content` as payload field
- `crates/mika-agent/src/skills/builtin_handlers.rs` — new `write_skill_variant`, `review_skill` cleanup
- `crates/mika-agent/src/skills/registry.rs` — `generated_model_prompts` map, fallback chain, list/info markings
- `mika/CLAUDE.md` — Tools convention update for payload field cap

## Tests

All in `#[cfg(test)] mod tests` of the touched files:

| Test | Asserts |
|---|---|
| `test_write_skill_variant_uses_runtime_model` | Path derives from `ctx`, not from any input |
| `test_write_skill_variant_writes_under_generated` | Resolved path always contains `/generated/` segment |
| `test_write_skill_variant_canonicalises_openrouter` | `openrouter/minimax/minimax-m2.7` → `generated/minimax/minimax-m2.7/` |
| `test_write_skill_variant_path_traversal_rejected` | Same suite as `review_skill` |
| `test_write_skill_variant_refuses_linked_skill` | Symlink check |
| `test_write_skill_variant_no_overwrite_without_force` | Rejects second write; accepts with `force=true` |
| `test_write_skill_variant_truncation_rejected` | Content < 50% source size → hard reject |
| `test_write_agent_file_large_payload` | 50 KB body succeeds; 210 KB body rejected |
| `test_resolve_prompt_handauthored_beats_generated` | Both present → hand-authored wins |
| `test_resolve_prompt_falls_back_to_generated` | Only generated present → generated used |

## Acceptance Criteria

- [ ] `payload_fields()` hook lands; `write_agent_file` / `write_workspace` declare `content`; control fields still capped at 10K
- [ ] `write_skill_variant` builtin registered, takes no path input, writes under `generated/`
- [ ] `review_skill` no longer exposes a writable target path; response includes `next_action`
- [ ] Registry loads `generated/<provider>/<model>/`; fallback chain prefers hand-authored
- [ ] All 10 unit tests pass under `cargo test -p mika-agent`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo fmt --check` clean
- [ ] `mika/CLAUDE.md` Tools section updated to document the payload field cap

## Out of scope (separate PR — mika-skills#99)

- **Layer D** — `mika-skills/skill-review/system_prompt.md` switches from `write_agent_file` to `write_skill_variant` and adds `[constraints] required_tools = ["review_skill", "write_skill_variant"]`.
- **Layer E** — Delete `mika-skills/qa-review/google/gemini-2.5-flash/system_prompt.md` (authored by minimax impersonating Gemini).

These ship after this PR merges and is deployed.

## Verification

```
cargo build --workspace
cargo test -p mika-agent
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

End-to-end (after both PRs merge and `make deploy`): trigger a skill-review run on a non-default model and confirm a variant lands under `<skill>/generated/<provider>/<model>/system_prompt.md`, that the registry picks it up on the next run, and that the size matches the source's order of magnitude (no truncation).

## Sources

- senara-solutions/mika#469 — implementation issue (full plan body)
- senara-solutions/mika-skills#99 — companion follow-up issue
- `feedback_prompt_enforcement_fragile` — institutional learning informing why we use a structural fix (no path input) instead of more prompt rules
