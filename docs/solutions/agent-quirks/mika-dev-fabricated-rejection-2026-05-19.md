---
module: mika-agent
tags: [fabrication, saturation, system-prompt, callback-turn, self-dev-callback, prompt-budget]
problem_type: agent-fabrication
category: agent-quirks
issue: 1217
---

# mika-dev LLM hallucinates prompt-injection rejections under system-prompt saturation

## Problem

p0-critical. On 2026-05-19, mika-dev's LLM fabricated security-defense responses on dispatch and callback turns — confabulating `[mika-engine]` authority blocks, `[output].required_suffix_lines` contracts, and a non-existent `feedback_prompt_enforcement_fragile.md` memory file. Zero tool calls. Tasks transitioned to `blocked` with fabricated state ("HEAD unchanged — nothing committed" while `git log` showed the post-flight-recovery commit at `2f60757d`). The canonical autonomous loop went down; overnight throughput dropped to 0 PRs.

Two evidence sessions confirmed the failure:

- **`callback-54a6ef5b-6846-4378-8d62-17299662140c`** (mika-dev, 2026-05-19 ~20:59Z) — the `mika ask --agent mika-dev "groom mika issue#1205"` callback turn. Three `llm_calls` rows: ToolUse/ToolUse/EndTurn. Model: `anthropic/claude-sonnet-4-6`. The final EndTurn response_text was the operator-fabricated framing "Groom blocked. Task `57fcaaf9` marked blocked. Awaiting your call on retry vs manual recovery." The middle ToolUse contained the `[Tool Call: send_message(...)]` with the verbatim fabrication: "mika#1205 groom failed — PIPELINE_INCOMPLETE.\n\nPlan file was written to worktree but `/ce:plan` was never invoked (session log shows no planning skill call). HEAD unchanged — nothing committed..."

- Operator-reported second session at ~21:42Z — same fabrication shape after operator-typed `retry`; task `57fcaaf9` blocked as `PIPELINE_INCOMPLETE` despite the post-flight-recovery commit visible in `git log`.

mika-arch on the same machine, same morning, did not exhibit the failure — the symptom is specific to mika-dev.

## Root cause

System-prompt saturation on callback turns.

The `self-dev-callback` skill is `always_on=true` and loaded on every mika-dev silent turn. Its system prompt was 14226 bytes — **87% of the 16384-byte skill prompt budget**. Combined with mika-dev's base prompt + KG corpus + memory injection + callback context, the assembled system prompt approached the model's capacity threshold. Under saturation, sonnet-4-6 fell back to plausible-but-fabricated security responses. The `[mika-engine] authority` shape was the model confabulating prompt-injection-defense structure it did not actually have.

F0.1 attribution paragraph (per mika#1217 plan): **the fabricated turn ran on model `anthropic/claude-sonnet-4-6`, with the `self-dev-callback` skill active (AlwaysOn match, `from_db_override = true`). The skill_override carve-out (mika#1011) DID fire — saturation, not override-scope-gap, is the root cause.** This places the incident in **Branch (a)** of the mika#1217 plan's F0 decision fork.

The `feedback_prompt_enforcement_fragile.md` memory warning was prescient: prompt-level enforcement is brittle under LLM rationalization. The 87%-of-budget callback skill was a structural fragility — defensive scaffolding piled on top of engine-side guards (#870, #991, #988, #1011) that already enforce the same rules at the post-condition layer.

## Solution

Four-phase fix shipped together as mika#1217:

**F1 — Context-budget observability.** New `system_prompt_bytes INTEGER` column on `llm_calls` (schema v37→v38, additive nullable, mirrors v30→v31). New `system_prompt_assembled` INFO log event emitted per turn with `total_bytes`, `total_chars`, `agent_id`, `session_id`, `trace_id`, `mode`, `trigger`, `active_skill_count`, `per_skill_bytes`. Emission sites: `run_agent_inner` (conversation), `run_silent_inner` (silent — Callback, DeferredDispatch, Heartbeat, etc.), `run_team_agent_inner` (team). Helper: `emit_system_prompt_assembled` in `crates/mika-agent/src/agent.rs`.

**F2 — `self-dev-callback` prompt trim from 14226 → 8578 bytes (52% of budget; validator emits `Ok`, not `Warn`).** Every line removed in the trim points to its engine-side equivalent:

| Removed prompt block | Engine-side rule |
|----------------------|------------------|
| "Engine contract (mika#991)" callout (3 lines, ~480 chars) | `callback_milestone_advance` guard (post-condition 6b) — `crates/mika-agent/src/agent.rs` + `CLAUDE.md § Agent Loop / Post-Conditions` |
| "Permitted post-callback actions" numbered list (5 lines, ~720 chars) | Same actions described prosaically in the success/failure/recover_unpushed_work handlers below in the same prompt |
| "Forbidden actions" bullets (5 lines, ~520 chars) | Engine rejects confirmation-only EndTurn on milestone callbacks via `callback_terminal_action` (post-condition 6) + `callback_milestone_advance` (post-condition 6b) — structurally guaranteed |
| "Step 4 — Wait for the completion callback" paragraph (5 lines, ~440 chars) | `permission-policy` skill keyword-activates on `[claude-pilot]` markers; this paragraph described behavior that fires on a different turn than the callback turn this prompt covers |

Plus compression within keep blocks (CALLBACK TYPE DETECTION, MILESTONE/PROJECT CONTEXT CHECK, decision tree prose) without removing any load-bearing routing logic.

**F3 — Override-scope contract test for SilentTrigger::Callback and SilentTrigger::DeferredDispatch.** New unit test `test_resolve_skill_llm_override_silent_callback_and_deferred_dispatch_carve_out` verifies that AlwaysOn + `from_db_override = true` qualifies for override resolution under both trigger shapes. **Open gap:** `run_silent_inner` does not currently invoke `resolve_skill_llm_override` on the silent path — the carve-out shape is correct at the function level, but the call-site wiring is the residual gap. This was not the cause of the canonical failure (Branch (a) — saturation, not override-scope-gap), so the residual gap ships as a follow-up ticket rather than a hot-fix.

**F4 — Suppress false-positive validator warnings for builtin tool references in `[constraints] required_tools`.** New `BUILTIN_TOOL_NAMES: &[&str]` const in `crates/mika-agent/src/tools/mod.rs` listing every engine-registered tool (default_tools + management_tools_if_needed + skills::builtin_handlers::KNOWN_BUILTINS). Skill validator §5b in `skills/index.rs` short-circuits to `Ok` when the required tool is a known builtin, instead of emitting a hedged warning. Parity test `test_builtin_tool_names_parity` enforces the const stays in sync with the real registries — drift fails CI.

## Verification

- **AC1** — `wc -c skills/bundled/self-dev-callback/system_prompt.md` → `8578` bytes (52% of 16384 budget). Validator emits `Ok` for prompt size at startup.
- **AC2** — Every `llm_calls` row written after deploy has a non-null `system_prompt_bytes` value. Every turn emits exactly one `system_prompt_assembled` INFO event.
- **AC3 / AC3b** — `mika ask --agent mika-dev "groom mika issue#<test>"` after deploy completes with `tool_calls > 0` on the dispatch turn; the claude-pilot completion callback turn also shows `tool_calls > 0` (inverting the zero-tool-call fabrication signature on the same surface as session `callback-54a6ef5b`).
- **AC4** — `test_resolve_skill_llm_override_silent_callback_and_deferred_dispatch_carve_out` passes. The residual silent-path-invocation gap is documented in this doc and tracked as a follow-up.
- **AC5** — `cargo run --bin mika-server` startup log no longer emits `required_tools references` Warn for `self-dev` (run_claude_pilot via dependency-prefix), `qa-review` (run_gh via BUILTIN_TOOL_NAMES), or `dev-handsoff` (write_agent_file via BUILTIN_TOOL_NAMES).
- **AC6** — This document.

## Lessons

- **System-prompt saturation is a recurring failure mode**, especially on long-lived AlwaysOn skills that accumulate defensive scaffolding over time. The 75%-of-budget validator warn predates this incident but lacked the per-turn observability to correlate the warning to actual fabrication. F1's `system_prompt_assembled` log event closes that gap.

- **Defensive prompt scaffolding decays.** Every "do NOT do X" rule added to a skill prompt is a structural fragility under LLM rationalization (per `feedback_prompt_enforcement_fragile.md`). When the engine grows a guard that enforces the same rule, the prompt-side scaffolding should be replaced by a one-line pointer to the engine rule — not retained as defense-in-depth. The F2 trim is the canonical move.

- **F0.1 attribution before F2/F3 sequencing.** The plan's mandatory F0 decision fork (Branch a/b/c) prevented the implementer from facing an unguided "ship trim as fix vs ship trim as hygiene" choice. Without that fork, the same code change could have been mis-shipped under the wrong root-cause narrative.

- **Validator noise hides legitimate misconfig.** Three false-positive `required_tools` warnings (self-dev, qa-review, dev-handsoff) made every startup log noisy enough that a real misconfig would have been hidden. F4's `BUILTIN_TOOL_NAMES` + parity test re-establishes the signal/noise contract.

## References

- mika#1217 — this fix.
- mika#716 — earlier mika-dev LLM fabrication baseline (this is recurrence + escalation).
- mika#988 — auto-skip recognition (referenced in self-dev-callback prompt).
- mika#991 — callback intent-precondition guard (post-condition 6b).
- mika#1011 — `LlmOverride.from_db_override` carve-out for AlwaysOn skills.
- mika#1207 — recursive guard self-audit class (resolved 2026-05-19); shape-adjacent.
- memory: `feedback_mika_dev_llm_fabricates_tool_errors.md`
- memory: `project_skill_override_scope_gap.md`
- memory: `feedback_prompt_enforcement_fragile.md`
