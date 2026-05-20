---
ticket: mika#1217
type: fix
priority: p0-critical
component: agent-core
date: 2026-05-20
---

# Plan — mika#1217: mika-dev LLM hallucinates prompt-injection rejections

## Context

p0-critical. mika-dev's LLM fabricates security-defense responses on dispatch turns
— confabulating `[mika-engine]` authority blocks, `[output].required_suffix_lines`
contracts, and a non-existent `feedback_prompt_enforcement_fragile.md` memory file.
Zero tool calls. Task transitions to `blocked` with fabricated state ("HEAD unchanged
— nothing committed" while `git log` shows the post-flight-recovery commit at
`2f60757d`). Canonical autonomous loop is down; overnight throughput 0 PRs.

Two evidence sessions on 2026-05-19:
- **344824d0** (~20:59Z): rejected `mika ask --agent mika-dev "groom mika issue#1205"`.
- **95086d5d** (~21:42Z): same fabrication after operator-typed `retry`;
  task `57fcaaf9` blocked as `PIPELINE_INCOMPLETE` despite recovery commit.

mika-arch (session `22011146`) on the same machine, same morning, did not exhibit
the failure — the symptom is specific to mika-dev.

## Hypothesis (operator's framing, to be verified before acting)

mika-dev startup emits five skill-validation warnings:

| # | Warning | Validator step |
|---|---------|----------------|
| 1 | `self-dev-callback: system_prompt.md (14226 bytes) is above 75% of limit (16384 bytes)` | `skills/index.rs` §6 line 1054 |
| 2 | `self-dev: required_tools references 'run_claude_pilot' which is not in this skill's tools.json` | `skills/index.rs` §5b line 827 |
| 3 | `qa-review: required_tools references 'run_gh'` (same site) | §5b |
| 4 | `dev-handsoff: required_tools references 'write_agent_file'` (same site) | §5b |
| 5 | `skill-review: no trigger keywords + not always_on` (operator paraphrase; no exact warn matches) | none — see Verification F0.2 |

Operator hypothesis: self-dev-callback at 87% of system_prompt budget, combined
with mika-dev base prompt + KG corpus + memory injection + dispatch turn context,
saturates sonnet-4-6. Under saturation, sonnet falls back to plausible-but-fabricated
security responses. The `[mika-engine] authority` shape is the model confabulating
prompt-injection-defense structure it does not actually have.

May compound with `project_skill_override_scope_gap` (deployed v0.8.0): if the
self-dev sonnet skill-override does not fire on autonomous-loop **callback** turns
(distinct from the conversation-mode webhook turns proven on 2026-05-08), mika-dev
runs on kimi-k2.5 base — even worse grounding than sonnet-with-saturation.

The hypothesis is plausible but **not yet proven**. The plan begins with a
verification phase so that the fixes target measured causes, not assumed ones.

## Verification phase (must precede F2 trim)

### F0.1 — Reconstruct the failing turn's actual context

For sessions `344824d0` and `95086d5d`:

```sql
-- Which model actually ran the failing assistant turn?
SELECT id, session_id, model, provider, prompt_variant, total_tokens, created_at
FROM llm_calls
WHERE session_id IN ('344824d0-...', '95086d5d-...')
ORDER BY created_at DESC;

-- Which skills were keyword/AlwaysOn-matched?
-- (extract from prompt_variant JSON — keys correspond to active skills)

-- Confirm tool_calls = 0 on the fabricated turn
SELECT id, session_id, tool_name, created_at
FROM tool_calls
WHERE session_id IN ('344824d0-...', '95086d5d-...');
```

Expected reads:
1. Was `self-dev-callback` in the active-skill set for the failing turn? (Conversation-mode
   user message "groom mika issue#1205" contains no `callback`/`PIPELINE FAILURE`/
   `claude-pilot` keyword. If self-dev-callback is NOT active on this turn, its prompt
   size does not enter the system-prompt for that turn — the hypothesis collapses for
   session 344824d0 specifically.)
2. What was the actual model (`anthropic/claude-sonnet-4-6` vs `openrouter/moonshotai/kimi-k2.5`)?
   If it ran on kimi, the failure is the override-scope gap, not budget saturation.
3. Read the saved assistant `response_text` (`llm_calls.response_text`, schema v31) to
   confirm the fabricated `[mika-engine]` shape verbatim.

Output of F0.1: a one-paragraph attribution — "the fabricated turn ran on **model X**,
with **N skills** matched (list), with assembled system prompt of **Y bytes**" —
written to `docs/solutions/agent-core/mika-dev-fabricated-rejection-2026-05-19.md`
as part of the implementation PR.

### F0.2 — Confirm warning #5 source

Warning text "no trigger keywords + not always_on" does **not** appear verbatim in
`crates/mika-agent/src/skills/index.rs` (the source of the other four). Either it is
operator paraphrase of an existing diagnostic, or it surfaces from a different
validator pass. Run `mika-server` startup with `RUST_LOG=debug` against a fresh
agent and grep `skill_skipped\|skill_warning\|skill-review` from the log. If the
exact wording is paraphrase, leave warning #5 out of F4's scope.

## Fix sequence

Fixes are sequenced **observability → trim → override-scope verify → validator cleanup**.
Observability first because every later fix depends on being able to measure its effect.

### F1 — Add context-budget observability (load-bearing prerequisite)

**Why first:** Without measuring the assembled system-prompt size per turn, F2's trim
has no signal — operator only sees fabrication or non-fabrication, not the budget
state that caused either. F1 also enables future regression detection independent of
this incident.

**What:** Emit a structured `system_prompt_assembled` log event on every turn entry,
with:
- `total_bytes` (the assembled system prompt that goes into the API call)
- `total_chars` (utf-8 char count)
- `agent_id`, `session_id`, `trace_id`, `mode` (conversation/silent/team), `trigger`
  (for silent: heartbeat/callback/deferred-dispatch/etc.)
- `active_skill_count`
- `per_skill_bytes`: `{<skill_name>: <bytes>}` (only loaded skills — skipped/disabled excluded)

**Where:** `crates/mika-agent/src/prompt.rs`, immediately after the `build_system_prompt`
call returns. Log at INFO. Add a per-skill bytes computation by reading
`active_skill_paths[i].prompt_bytes_len` (`SkillPathInfo`); if `prompt_bytes_len` is not
yet a field, add it as a `usize` computed at injection time.

**Persisted in `llm_calls`:** add a `system_prompt_bytes INTEGER` column (schema v38)
populated at `save_llm_call()` from a new `system_prompt_bytes: Option<i64>` param.
Migration: additive `ALTER TABLE ... ADD COLUMN` with `column_exists` guard (matches
v30→v31 shape at the same site, see `mika-agent/CLAUDE.md` § "v30->v31").

**Out of scope for F1:** dashboard UI for the new column. Surfacing in the timeline /
LLM-call detail page can ship as a follow-up (file `mika-platform#TBD` from
implementation).

**Acceptance signal:** running `mika ask --agent mika-dev "groom mika issue#1205"` on a
dev binary emits the `system_prompt_assembled` event with non-zero `total_bytes` and a
`per_skill_bytes` map containing every active skill.

### F2 — Trim self-dev-callback system_prompt.md from 14226 bytes → ≤10000 bytes

**Why:** Even without proving F1 saturation, 87% of budget is unhealthy headroom for
a callback skill that gets layered on top of mika-dev's already-substantial base
prompt + KG context + memory injection. Trim shrinks the proximate risk surface.

**Audit method:** the current `system_prompt.md` (139 lines) has four redundancy
classes documented across `mika-agent/CLAUDE.md` § "Agent Loop / Post-Conditions" as
structurally-enforced engine rules. The prompt repeats them as defensive scaffolding:

| Prompt section | Lines | Engine-side coverage | Action |
|----------------|-------|---------------------|--------|
| Top "Engine contract" callout (mika#991) | 3 | Intent-precondition registry entry `callback_milestone_advance` (#702 + #991) — both a guard and a `PostCallbackAdvance` structural backstop | Compress to one-line pointer ("Engine guard fires on milestone-context callbacks; see CLAUDE.md § Post-Conditions 6/6b.") |
| "Permitted post-callback actions" (numbered 1–4) | 5 | Identical actions described prosaically below in success/failure/recover_unpushed_work | Replace numbered list with single sentence; let downstream sections own the detail |
| "Forbidden actions" list (4 bullets) | 5 | Engine rejects via guard 6/6b structurally | Replace with one line: "Engine rejects confirmation-only EndTurn on milestone callbacks; structurally guaranteed." |
| Step 4 "you have nothing to do during dispatch" | 5 | The permission-policy skill keyword-activates on `[claude-pilot]`; this paragraph never fires during the callback turn it describes | Delete; the paragraph belongs in self-dev's prompt, not the callback handler's |

**Other compression opportunities:**
- Auto-skip recognition block (4 lines, mika#988) — keep, but compress to one sentence
  with a JSON-shape pointer.
- Recover-unpushed-work handler (4 numbered steps + decision tree) — keep; this is the
  load-bearing recovery logic with no engine-side equivalent.
- Pipeline-result classification + grounding check — keep verbatim; complex
  conditional logic where prompt-side specificity is genuinely needed.
- Failure mode table — keep verbatim; the LLM uses this as a lookup table.

**Target outcome:** ≤10000 bytes (≤61% of 16384 budget). After trim, validator should
emit `OK` for prompt size, not WARN.

**Method discipline:** every line removed must point to its engine-side equivalent in
the PR description (file:line citations). No "cargo cult trim" — each delete is
justified by a corresponding code path that already enforces the same rule.

### F3 — Verify skill_override scope on autonomous-loop **callback** turns

**Why:** `project_skill_override_scope_gap.md` says the override-scope fix (mika#1011)
deployed v0.7.0 → v0.8.0 with end-to-end webhook-path proof on 2026-05-08. The proof
covers `SilentTrigger::Webhook`-derived **ready-label dispatch** turns (conversation
mode). It does NOT explicitly cover `SilentTrigger::Callback` turns (claude-pilot
completion handling, error_max_turns retry, PIPELINE FAILURE classification).

**What:** Code-read pass in `crates/mika-agent/src/agent.rs` (`run_silent_agent` and
`resolve_skill_llm_override` callers). For each SilentTrigger variant — `Heartbeat`,
`Reflection`, `Callback`, `SkillRun`, `Reminder`, `PostCallbackAdvance`,
`DeferredDispatch` — confirm:
1. Does `apply_overrides()` run for that variant? (It runs once at agent init, but skill
   sets per variant differ; override carve-out runs on resolved-skill set.)
2. Does `resolve_skill_llm_override()` get called from the silent path for that variant?
3. For `Callback` specifically: when the matched skill is `self-dev-callback` (AlwaysOn +
   keyword-match on `claude-pilot callback` / `callback` / `error_max_turns`), does the
   `LlmOverride.from_db_override = true` carve-out grant override-eligibility? Verify
   against the `agent.rs:3773-3774` gate cited in the override-scope memory.

**Output:** add a unit test in `crates/mika-agent/src/skills/mod.rs` (or wherever
`resolve_skill_llm_override` lives) that exercises each `SilentTrigger` variant
against a fixture agent with a DB skill override. Test must demonstrate the override
fires on `Callback` and `DeferredDispatch` turns. If it does not, F3 promotes a
**follow-up ticket** — fixing it is out of scope for this p0 hot-fix; the carve-out
extension is its own design call.

**Bound the work:** if F0.1 attribution shows the failing turn ran on `sonnet-4-6`,
F3 is informational (the override DID fire — saturation is the cause). If it shows
`kimi-k2.5`, F3 becomes blocking and the trim alone won't fix the fabrication.

### F4 — Suppress false-positive validator warnings for builtin tools

**Why:** Warnings 2–4 are noise — the referenced tools ARE in the registry (just as
builtins, not in skill-local `tools.json`). Operator currently has no way to
distinguish "the skill's tools.json forgot to declare a real local handler" from
"required_tools cites a builtin which is fine." This makes legitimate skill misconfig
hide in the noise.

**What:** `crates/mika-agent/src/skills/index.rs` §5b (lines 818–834). Currently
emits a Warn for every required_tool not in skill-local tools.json, with the
hedge "this is OK if it's a builtin, MCP, or dependency tool." Tighten:

1. Pass a `known_builtin_tools: &HashSet<String>` set into `validate_skill()` —
   computed once at call site from `tools::default_tools().tool_names()` (or equivalent
   accessor; add one if missing — must not allocate a fresh registry, since instantiation
   has side-effects; instead use a static const list of builtin tool names mirrored
   from `default_tools()`, with a compile-time check or unit test that enforces parity).
2. In §5b, before emitting the Warn, check `if known_builtin_tools.contains(required)`
   and short-circuit to an `Ok` diagnostic ("required tool '{required}' is a registered
   builtin") instead.
3. Leave the dependency-prefix heuristic (`likely_from_dep`) unchanged — it covers a
   different case (MCP tools, skill-dependency tools).
4. MCP tools cannot be enumerated at validator time (they connect at startup). For
   MCP tools, the Warn stays — operator sees it once per skill, accepts it as a known
   limitation. Document in the validator comment.

**Parity test:** add `tests/skills_validator_builtin_parity.rs` that asserts every name
in `BUILTIN_TOOL_NAMES` resolves to a tool registered by `default_tools()`. Prevents
drift between the validator's known-builtin set and the actual registry.

**Out of scope for F4:** Warning #5 (skill-review). Verification F0.2 determines its
source first; if it's operator paraphrase of an existing diagnostic, it's documented;
if it's a real diagnostic with a real cause, it stays as-is (skill-review's empty
keywords are intentional per `skills/bundled/skill-review/skill.toml` comment).

## Acceptance criteria

- **AC1.** `mika/skills/bundled/self-dev-callback/system_prompt.md` is ≤ 10000 bytes
  (≤ 61% of 16384). Validator emits `Ok` for prompt size at startup. Every line
  removed in F2 is justified in the PR description with a `file:line` pointer to the
  engine-side equivalent rule.

- **AC2.** Every `llm_calls` row written after deploy has a non-null
  `system_prompt_bytes` value. Every turn emits exactly one `system_prompt_assembled`
  INFO log event with `total_bytes` and `per_skill_bytes`. Sample query:
  `SELECT model, system_prompt_bytes FROM llm_calls WHERE created_at > <deploy_ts> LIMIT 10`
  returns 10 non-null rows.

- **AC3.** `mika ask --agent mika-dev "groom mika issue#<test>"` after deploy
  completes with `tool_calls > 0` (proves the LLM made at least one tool call,
  inverting the zero-tool-call fabrication signature). The dispatch produces
  either a successful `dev-groom` flow or a structured failure with a real tool-error
  payload — not a confabulated `[mika-engine]` security-defense response.

- **AC4.** A unit test in the skill_override module exercises `SilentTrigger::Callback`
  and `SilentTrigger::DeferredDispatch` against a fixture DB override. Test passes
  (override fires) OR a follow-up ticket is filed explicitly scoping the override-scope
  gap for those variants.

- **AC5.** Validator no longer emits Warn for builtin tool references in `required_tools`
  for `self-dev`, `qa-review`, `dev-handsoff`. Manual verification: start `mika-server`
  against a fresh dev-mode home, grep startup log for `required_tools references`. Match
  count drops from current N=3 to 0 for these three skills.

- **AC6.** `docs/solutions/agent-core/mika-dev-fabricated-rejection-2026-05-19.md`
  exists, written from F0.1's reconstructed attribution, naming the actual root cause
  (saturation vs. override-scope-gap vs. other) and citing both evidence sessions.

## Risks & tradeoffs

- **R1 — Trim alone may not fix the fabrication.** Operator hypothesis is plausible
  but unproven until F1 ships and a recurrence (or non-recurrence) is measured.
  Mitigation: F1 ships first; F2 ships informed by F1 telemetry on a representative
  dispatch run.

- **R2 — `feedback_prompt_enforcement_fragile.md` warning.** Memory file flags that
  prompt-level enforcement is brittle under LLM rationalization. F2's prompt-level
  defensive consolidation might re-introduce a similar fragility class. Mitigation:
  every removal in F2 is replaced by a one-line **pointer** to the engine-side rule,
  not by a re-worded prompt-level rule. The prompt's role shrinks to navigation, not
  enforcement.

- **R3 — F1 schema migration risk.** Additive `ALTER TABLE` is shape-identical to
  v30→v31 (additive nullable). Standard guarded migration; no data movement. Worst
  case: column added but never populated (NULL); query plans treat as unknown.

- **R4 — F4 parity drift.** If `BUILTIN_TOOL_NAMES` const drifts out of sync with
  `default_tools()`, the validator silently mis-handles new builtins. Mitigation:
  parity test in F4 step 4 fails at CI if the lists diverge.

- **R5 — F3 may surface a deeper gap.** If the override does NOT fire on `Callback`
  turns, mika-dev runs on kimi-k2.5 base for the entire dispatch-callback chain, and
  the trim has reduced impact. Mitigation: F3 promotes a follow-up ticket; this PR
  ships the trim + observability + validator cleanup regardless. The follow-up
  ticket gets `p1`/`agent-core` and links back to mika#1217.

- **R6 — F2 prompt regression.** Trimming a callback prompt that the autonomous loop
  depends on could create a behavioral regression elsewhere (e.g., milestone advance
  decisions). Mitigation: run mika-agent's calibration suite on `mika-dev` role
  (per CLAUDE.md § "Model calibration" — `make calibrate-mika-dev MODEL=anthropic/claude-sonnet-4-6`)
  before merge. Calibration fixtures (5 scenarios anchored on #1168/#1166/#1173) should
  pass at parity or better after trim.

## Sequencing

Single PR, four landed-together commits:

1. `feat(observability): add system_prompt_bytes to llm_calls + INFO log event (F1)`
2. `chore(skills): trim self-dev-callback prompt 14226→<X> bytes (F2)`
3. `test(skills): verify llm_override fires on Callback/DeferredDispatch SilentTriggers (F3)`
4. `fix(skills/validator): suppress false-positive warn for builtin required_tools (F4)`

PR body includes:
- F0.1 attribution paragraph as the leading "Root cause" section.
- Line-by-line F2 trim justification table with engine-side citations.
- Calibration report from `make calibrate-mika-dev`.
- Manual reproduction notes for AC3 (the post-deploy `mika ask` smoke).

## Out of scope

Per ticket body:
- **Model switch.** sonnet-4-6 is the correct grounded choice for mika-dev. The fix
  is input-side (prompt budget), not model-side.
- **Removing self-dev-callback.** The skill is load-bearing for claude-pilot
  completion handling, milestone-advance dispatch, and pipeline-failure recovery
  (per mika#991 + #870 + #1058). Removal would be a wholesale architectural change,
  not a hot-fix.

Promoted to follow-up tickets (filed by implementation, not pre-grooming):
- Dashboard UI for `llm_calls.system_prompt_bytes` (F1 follow-up).
- Override-scope extension to `Callback`/`DeferredDispatch` if F3 shows the carve-out
  doesn't fire there.
- Warning #5 (skill-review empty-keywords) if F0.2 finds it's a real diagnostic
  rather than operator paraphrase.

## Related

- mika#716 — earlier mika-dev LLM fabrication baseline (this is recurrence + escalation).
- mika#1207 — recursive guard self-audit class (resolved 2026-05-19); shape-adjacent.
- mika#988 — auto-skip recognition (referenced in self-dev-callback prompt).
- mika#991 — callback intent-precondition guard (referenced in self-dev-callback prompt).
- mika#1011 — `LlmOverride.from_db_override` carve-out for AlwaysOn skills (override
  scope gap closure).
- memory: `feedback_mika_dev_llm_fabricates_tool_errors.md`
- memory: `project_skill_override_scope_gap.md`
- memory: `feedback_prompt_enforcement_fragile.md`
- code: `crates/mika-agent/src/skills/index.rs:818-834` (F4 site).
- code: `crates/mika-agent/src/skills/index.rs:1054-1058` (F2 validator signal site).
- code: `crates/mika-agent/src/prompt.rs:479` (F1 instrumentation site).
- code: `crates/mika-agent/src/tools/mod.rs:663-703` (F4 builtin-tool-set source).
