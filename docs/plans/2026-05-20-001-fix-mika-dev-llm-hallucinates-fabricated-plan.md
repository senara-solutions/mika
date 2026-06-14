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

## Phase 0 Pin (verbatim slices at base SHA `fe610fd4`)

All citations in this plan resolve against the worktree at `fe610fd4` (the
plan-commit itself). The full files are short enough that the plan attaches
them by path; the implementer reads these paths verbatim before editing.

### Pin A — `mika/skills/bundled/self-dev-callback/system_prompt.md` (full file, 138 lines, 14226 bytes)

Annotated removal targets vs keep-verbatim blocks for F2:

| Lines | Block | Action |
|-------|-------|--------|
| 1 | `### Callback Entry Point (post background task)` header | Keep — load-bearing structural marker. |
| 3 | "Engine contract (mika#991)" callout (one paragraph, ~480 chars) | **F2 removal target #1** — replace with one-line pointer: "Engine guard `callback_milestone_advance` (#991) plus `PostCallbackAdvance` backstop. See `crates/mika-agent/CLAUDE.md` § Agent Loop / Post-Conditions." |
| 5–9 | "When you receive a callback result… CALLBACK TYPE DETECTION" block | Keep — load-bearing routing logic, no engine equivalent. |
| 11 | "CRITICAL: DO NOT end your turn after receiving a callback" callout | Keep — re-asserts engine guard #870 but the verbatim emphasis is observed to matter for sonnet adherence; do not trim until F1 telemetry confirms safety. |
| 13 | "SCOPE RULE: Post-callback turns handle ONLY the task that triggered the callback" | Keep — operator-load-bearing instruction; no engine equivalent. |
| 15–20 | "**Permitted post-callback actions (exhaustive list — mika#991):**" + 4 numbered items (~720 chars) | **F2 removal target #2** — replace with single sentence: "Permitted post-callback actions are described prosaically in the success/failure/recover_unpushed_work handlers below." |
| 22–26 | "**Forbidden actions (mika#991):**" + 4 bullets (~520 chars) | **F2 removal target #3** — replace with one-line pointer: "Engine rejects confirmation-only EndTurn on milestone callbacks via guard 6/6b; see `crates/mika-agent/CLAUDE.md` § Agent Loop / Post-Conditions." |
| 28–35 | "MILESTONE/PROJECT CONTEXT CHECK (mandatory)" + sub-bullets | Keep — verifies the runtime check the LLM must perform; no engine equivalent inside the LLM-readable surface. |
| 37 | "**Auto-skip recognition (MANDATORY — run before all other classification):**" + body (~440 chars) | Keep verbatim — mika#988 recovery logic. |
| 41–75 | "**Pipeline result classification (MANDATORY)**" + grounding check + decision tree + recover_unpushed_work handler (~4200 chars) | Keep verbatim — no engine-side equivalent. |
| 80–86 | "**On pipeline failure**" handler (4 numbered steps, ~1000 chars) | Keep verbatim. |
| 87–92 | "**On success**" handler (5 numbered steps, ~900 chars) | Keep verbatim. |
| 94 | "**On failure (non-zero exit, FAILED, or not structured JSON):**" handler | Keep verbatim. |
| 96–98 | "**Step 4 — Wait for the completion callback**" paragraph (~620 chars) | **F2 removal target #4** — delete entirely. The `permission-policy` skill keyword-activates on `[claude-pilot]` markers; this paragraph describes behavior that fires during a different turn (not the callback turn this prompt covers). |
| 100–106 | "**Completion callback result**" block | Keep verbatim. |
| 108–138 | Step 4.5 diagnose-and-recover, classification table, recovery actions, escalation template, Step 5 webhook QA note | Keep verbatim — failure-mode lookup table is load-bearing; sonnet uses it as a switch statement. |

Expected delta: ~2160 bytes removed (480 + 720 + 520 + 440) → 14226 − 2160 ≈ 12066 bytes. To reach the 10000-byte target, F2 must also compress ~2000 additional bytes from within the keep blocks. **The implementer audits the keep-verbatim blocks under F1 telemetry on a representative dispatch run before deciding additional trim sites.** If F1 telemetry shows the assembled system prompt at ≤90% of budget after the four removal targets, the additional 2000-byte trim is deferred to a follow-up; AC1 relaxes to ≤12500 bytes (≤76% of budget, validator Ok).

### Pin B — `crates/mika-agent/src/prompt.rs:478-486` (build_system_prompt entry)

```rust
/// Build the system prompt from context.
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    write_soul_section(&mut prompt, ctx.soul_content);
    write_identity_section(&mut prompt, ctx.identity);
    write_time_section(&mut prompt, ctx.current_utc, ctx.timezone.as_deref());
    write_channel_section(&mut prompt, ctx.channel_type, ctx.telegram_configured);
```

F1 emission boundary candidate A: `prompt.rs` immediately after this function returns (caller-side, before any post-assembly transformation). F1 emission boundary candidate B: `crates/mika-agent/src/agent.rs:2239` (`let mut system = prompt::build_system_prompt(&prompt_ctx);` in `run_agent`) and `agent.rs:3879` (same call in `run_silent_agent`), measured AFTER summary gate appends:

```rust
let mut system = prompt::build_system_prompt(&prompt_ctx);

// Axis 4 + Axis 3 summary gate (mika#1019, mika#1021).
// Conversation mode: silent_trigger is None — Axis 3 cap does not fire.
if let Some(content) = load_gated_summary(db, &ctx.identity.context.summary, None).await? {
    system.push_str("\n## Conversation Summary\n");
    system.push_str("<context type=\"summary\" trust=\"data\">\n");
    system.push_str(&content);
    system.push_str("\n</context>\n");
}
```

**Decision (resolved):** emit at the call site after summary-gate append (candidate B) — this measures the bytes that actually go on the wire. Two emission sites: `agent.rs:2239+15` (conversation) and `agent.rs:3879+15` (silent). Both wrap into a shared `emit_system_prompt_assembled(&system, &ctx, …)` helper to avoid drift.

### Pin C — `crates/mika-agent/src/agent.rs:4180-4202` (override scope carve-out)

```rust
// Two qualification paths (#463, mika#1011):
// 1. Keyword-matched skills: always qualify (original #463 behavior).
// 2. AlwaysOn skills with DB-sourced LLM overrides (from_db_override = true):
//    qualify because DB overrides represent explicit operator intent via
//    `mika skills llm set`, not developer-time skill.toml [llm] hijacks.
let mut overrides: Vec<(&str, Option<&str>)> = Vec::new();
let mut override_skills: Vec<&str> = Vec::new();

for ms in matched {
    let qualifies = match ms.reason {
        MatchReason::Keyword => true,
        MatchReason::AlwaysOn => ms.entry.manifest.llm.from_db_override,
        MatchReason::Dependency => false,
    };
```

F3 reads this carve-out and confirms `from_db_override = true` is the only gate on AlwaysOn skill overrides firing. The function `resolve_skill_llm_override` (containing this block) is called from `run_silent_agent` at `agent.rs:3879+N` — F3 traces back from each `SilentTrigger` variant to confirm the call site is reached.

### Pin D — `crates/mika-agent/src/skills/index.rs:818-834` (F4 required_tools validator)

```rust
// 5b. Validate [constraints] required_tools against known tool names
for required in &manifest.constraints.required_tools {
    if !skill_tool_names.contains(required) {
        // Check if the tool name plausibly comes from a declared dependency
        let likely_from_dep = manifest.skill.dependencies.iter().any(|dep| {
            let prefix = dep.replace('-', "_");
            required.starts_with(&prefix)
        });
        if !likely_from_dep {
            diags.push(SkillDiagnostic::warn(format!(
                "[constraints] required_tools references '{}' which is not in this skill's \
                 tools.json — this is OK if it's a builtin, MCP, or dependency tool",
                required
            )));
        }
    }
}
```

F4 inserts a `known_builtin_tools.contains(required)` check before the `!likely_from_dep` branch and short-circuits to `SkillDiagnostic::ok(...)`.

### Pin E — `crates/mika-agent/src/skills/index.rs:1053-1064` (F2 validator signal site)

```rust
} else if effective_limit > 0 && size > effective_limit * 3 / 4 {
    diags.push(SkillDiagnostic::warn(format!(
        "system_prompt.md ({} bytes) is above 75% of limit ({} bytes)",
        size, effective_limit
    )));
} else {
    diags.push(SkillDiagnostic::ok(format!(
        "system_prompt.md size OK ({} bytes, limit {} bytes)",
        size, effective_limit
    )));
}
```

After F2 trim, validator should emit the `Ok` branch for `self-dev-callback`. AC1 verifies by reading the startup log.

### Pin F — migration pattern, `crates/mika-agent/src/db.rs:2890-2893` (v34->v35 shape mirror)

```rust
if !self.column_exists("llm_calls", "step")? {
    let sql =
        "ALTER TABLE llm_calls ADD COLUMN step INTEGER NOT NULL DEFAULT 0;";
```

F1's `system_prompt_bytes INTEGER` column follows this exact shape (additive, nullable variant — no `NOT NULL DEFAULT` since pre-migration rows have no measurable value). Implementation site: add a new `migrate_v37_to_v38()` (or whatever the next migration index is at implementation time) following the same `column_exists` guard pattern.

### Pin G — `crates/mika-agent/src/async_db.rs:1965` (save_llm_call dispatch)

`async_db.rs:1965: pub async fn save_llm_call(` — F1 adds a new `system_prompt_bytes: Option<i64>` parameter here and threads it to the sync `Database::save_llm_call` at `db.rs:6160`. All call sites updated (eval harness, agent.rs:649/966/992, etc).

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
validator pass. Run `mika-spirit` startup with `RUST_LOG=debug` against a fresh
agent and grep `skill_skipped\|skill_warning\|skill-review` from the log. If the
exact wording is paraphrase, leave warning #5 out of F4's scope.

### F0 decision fork (mandatory — drives whether F2 ships as fix or hygiene)

After F0.1 attribution lands, the implementer chooses one of three branches.
The branch is recorded in the PR description and in the solutions doc (AC6).

**Branch (a) — F0 confirms `self-dev-callback` was active AND model was `sonnet-4-6`:**

Root cause is **system-prompt saturation**. F2 ships as **the load-bearing fix**;
AC3 (callback-turn fabrication-inverse) is the primary acceptance signal; AC3b
(see Acceptance Criteria) is the verifying callback-mode reproduction. F3 stays
as informational verification (override fired; saturation was the cause).

**Branch (b) — F0 confirms `self-dev-callback` was NOT active on the failing turn:**

The saturation hypothesis collapses for the canonical evidence sessions; the
14226-byte prompt was not in the assembled system prompt for those turns. F2
**still ships, but as hygiene** (87% of budget is unhealthy independent of this
incident; trim reduces a real risk surface for the callback turns that *do*
load it). AC3 re-baselines: the post-deploy `mika ask "groom mika issue#<test>"`
is no longer the canonical inverse-of-fabrication signal — it becomes a
regression smoke. A **new ticket is filed** at PR-open time with `p0-critical`,
referencing F0.1's attribution doc, scoping the **actual root cause** (the
fabrication mechanism that fires on conversation-mode turns without
self-dev-callback in the active set). The new ticket inherits the canonical
loop blocker; mika#1217 ships as the hygiene + observability + validator PR.

**Branch (c) — F0 confirms the model was `kimi-k2.5` (not `sonnet-4-6`):**

Root cause is the **override-scope gap on autonomous-loop callback turns**. F3
**promotes from "verify" to "fix"**: the implementation extends the
`from_db_override` carve-out (or its caller chain in `run_silent_agent`) so
that `SilentTrigger::Callback` and `SilentTrigger::DeferredDispatch` resolve
the DB override correctly. F2's trim ships as defense-in-depth (the
combination of override-firing + trimmed prompt is the durable fix). AC4 is
upgraded from "test exercises override + may file follow-up" to "test
demonstrates override fires on Callback/DeferredDispatch; the fix is in this
PR." Calibration suite must pass on both sonnet-4-6 AND kimi-k2.5 base models
because the trim affects both.

**Why this fork lives in the plan, not in implementer judgment:** without it,
the implementer completes F0 and faces an unguided choice between "ship the
trim as the fix" and "ship the trim as hygiene + file a follow-up" — and the
choice has materially different ticket-shape consequences (one PR vs PR +
p0-critical follow-up). The plan binds the decision so architect-validated
sequencing survives F0's surprise.

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

- **AC3 (conversation-mode regression smoke).** `mika ask --agent mika-dev
  "groom mika issue#<test>"` after deploy completes with `tool_calls > 0`
  (proves the LLM made at least one tool call, inverting the zero-tool-call
  fabrication signature on the same surface as session `344824d0`). The
  dispatch produces either a successful `dev-groom` flow or a structured
  failure with a real tool-error payload — not a confabulated `[mika-engine]`
  security-defense response.

- **AC3b (callback-mode failure-path verification — load-bearing per F0).**
  Both evidence sessions involve dispatch/callback context responses. AC3
  alone tests conversation-mode; if F0 confirms the canonical failure fires
  on callback turns specifically (Branch (a) or (c)), AC3 may pass while the
  fix is incomplete. **Required verification:** dispatch a test ticket via
  `mika ask --agent mika-dev "implement mika issue#<test>"` and wait for the
  claude-pilot completion callback to fire. The callback turn's row in
  `llm_calls` must show `tool_calls > 0`. If F0 lands in Branch (b) (canonical
  failure is not callback-turn-specific), AC3b downgrades to a sanity-check
  smoke (still run, but not load-bearing). The decision is recorded in the PR
  description alongside the F0 attribution.

- **AC4.** A unit test in the skill_override module exercises `SilentTrigger::Callback`
  and `SilentTrigger::DeferredDispatch` against a fixture DB override. Test passes
  (override fires) OR a follow-up ticket is filed explicitly scoping the override-scope
  gap for those variants.

- **AC5.** Validator no longer emits Warn for builtin tool references in `required_tools`
  for `self-dev`, `qa-review`, `dev-handsoff`. Manual verification: start `mika-spirit`
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

## Rollback

The four-commit PR mixes a one-way ratchet (schema migration) with three
cleanly-revertable changes. Operators reverting the PR need to know which is
which:

- **F1 schema migration (`ALTER TABLE llm_calls ADD COLUMN system_prompt_bytes
  INTEGER`) is permanent.** Additive nullable, mirrors v34->v35 shape, no
  data movement. The column is independently valuable for observability
  regardless of whether the rest of the fix lands; reverting the PR does NOT
  remove the column. Post-revert behavior: zero writes populate the new
  column (the `save_llm_call` call site no longer passes the new param),
  query plans treat values as NULL — harmless. Removing the column requires
  a new forward migration; not done as part of a revert.

- **F1 INFO log event (`system_prompt_assembled`)** reverts cleanly with the
  PR. The shared helper `emit_system_prompt_assembled` and its two call
  sites in `agent.rs` come out atomically.

- **F2 prompt trim (`skills/bundled/self-dev-callback/system_prompt.md`)**
  reverts cleanly — the file returns to its pre-fix shape at base SHA
  `fe610fd4`. No coupled engine changes; the prompt is a build-time-discovered
  asset.

- **F3 unit test (skill-override on Callback/DeferredDispatch)** reverts
  cleanly — pure test addition, no production code impact even if F3 ships
  in Branch (c) mode (the override-scope fix itself reverts cleanly too;
  it's a small change to the `qualifies` match arm or carve-out condition).

- **F4 validator change (`skills/index.rs` §5b + `BUILTIN_TOOL_NAMES` const
  + parity test)** reverts cleanly — the validator returns to its pre-fix
  warning behavior; the parity test goes with it.

**Operator-facing summary in PR description:** "Reverting this PR backs out
F2/F3/F4 cleanly; the `system_prompt_bytes` column in `llm_calls` is added
by F1 and persists across revert (additive, nullable, harmless — file a new
migration if removal is needed)."

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
