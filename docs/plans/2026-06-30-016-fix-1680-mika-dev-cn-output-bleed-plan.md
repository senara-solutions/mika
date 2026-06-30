---
issue: 1680
type: fix
date: 2026-06-30
---

# Plan — fix(mika-dev): Chinese-language output bleed in webhook dispatch summaries (mika#1680)

## Problem

mika-dev (running zai/glm-5.2 since 2026-06-30) is emitting fluent Chinese (Mandarin) text in webhook dispatch summaries and status responses. Hard evidence captured during grooming (2026-06-30 14:53Z):

- **10 distinct mika-dev assistant responses in the last 2 hours** contain CJK characters (`messages` table query: `content GLOB '*[一-龥]*'`).
- One sample at 14:48:48Z (576 chars): `引擎已确认：issue_comment.created webhook 不被授权进行 dispatch (mika#933)。mika#1665 已完成 grooming —— 计划位于 fix/1665/... 分支，已应用 ready 标签...` (translation: "Engine confirmed: webhook is not authorized for dispatch. mika#1665 grooming completed — plan located in fix/1665/... branch, ready label applied.")

The operator (Vincent, English-speaking) cannot read the dispatch summary. TUI box-glyphs are the surface symptom (terminal font lacking CJK coverage), but the underlying issue is **the model itself responding in Chinese**, not just emitting unrenderable decoration.

## Architectural lineage

- mika#1633 — glm-5.2 swap on mika-dev (introduced CN-trained model).
- mika#1657 — Z.AI native provider (the routing).
- mika#1670 / PR #1673 — mika-qa swap to glm-5.2 (same model class — will exhibit same regression).
- mika#862 (asserted-unavailability guard) + mika#1331 (assert-grounded guard) — sibling engine-guard pattern (regex/keyword detection + single-retry re-prompt) that this fix mirrors.
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` — counter-evidence that prompt-only fixes don't bind across model classes; defense-in-depth must be structural.

## Fix shape (two-layer — prompt-level primary + engine-level defense-in-depth)

### Layer A (primary, cheap) — English-only instruction in identity + skill prompt

Add explicit instruction:

```
Always respond in English. Do not use Chinese, Japanese, Korean, or any non-Latin script in any response — including dispatch summaries, status messages, and webhook handler responses. Technical terms and code should be ASCII only.
```

Two insertion sites:
1. **`MIKA_DEV_IDENTITY` const** in `crates/mika-agent/src/well_known_agents.rs` — persona-level reinforcement, applies to all mika-dev turns.
2. **`skills/bundled/self-dev-callback/system_prompt.md`** — webhook-handler skill where most CN-bleed lands per the evidence. Skill-prompt-level reinforcement composes with identity.

Per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, prompt-only fixes don't fully bind on swapped models. Layer A reduces fire-rate; Layer B catches the residue.

### Layer B (defense-in-depth, structural) — non-ASCII detector + re-prompt guard

Add a new EndTurn post-condition guard (or extend an existing one) that detects non-Latin script in assistant text and re-prompts once.

**Detection:** scan response_text for codepoints in:
- `U+4E00..U+9FFF` (CJK Unified Ideographs) — primary target (Chinese, Japanese kanji).
- `U+3000..U+303F` (CJK Symbols and Punctuation) — `、。「」` etc.
- `U+3040..U+309F` (Hiragana), `U+30A0..U+30FF` (Katakana) — Japanese.
- `U+AC00..U+D7AF` (Hangul Syllables) — Korean.

Trigger threshold: ≥ 5 codepoints from these ranges in a single response (avoids false-positives on incidental single-character technical content).

**Behavior:**
- First occurrence in a turn → re-prompt with the Layer A instruction inlined ("Your previous response contained non-English script. Respond in English only — restate the full message in English"). Single retry tracked via `intent_guard_retries` with label `"non_latin_script"`.
- Second occurrence → accept the EndTurn but emit a structured `warn!` log event `non_latin_script_persistent` for operator-visibility. Don't lock the agent into an infinite re-prompt loop.

**Scope:** Fire on assistant text in conversation mode AND silent/webhook mode (per body evidence — most CN-bleed is on webhook turns). Guard sits in the EndTurn chain alongside existing post-conditions (`crates/mika-agent/src/agent_loop/post_conditions.rs` or equivalent).

**Pattern:** mirrors `asserted_unavailability` (mika#862) and `assert_grounded` (mika#1331) inline guard shape — small detector function + single-retry tracking + structured log event.

### Layer C (architect F2 BLOCKING — IN SCOPE this PR) — mika-qa prompt-layer fixes

mika-qa swap to zai/glm-5.2 (mika#1670 → PR#1673) merged today; mika-qa is now exposed to the same CJK-bleed regression. Per architect F2 ("known configuration drift shall not be deferred when marginal cost of inclusion is low"), Layer C ships atomically:

1. **`MIKA_QA_IDENTITY` const** in `crates/mika-agent/src/well_known_agents.rs` — add English-only instruction (same text as Layer A.1).
2. **`skills/bundled/qa-review/system_prompt.md`** — add English-only instruction (same text as Layer A.2).

Layer B's engine guard (described below) fires for any agent — mika-qa included — but Layer A+C together give both prompt-level and engine-level coverage on both swapped agents.

## Implementation outline

1. **Layer A.1 — identity edit:** add English-only instruction to `MIKA_DEV_IDENTITY` const in `crates/mika-agent/src/well_known_agents.rs`. Position: in the persona-rules section (find existing rules block and add as a new bullet/paragraph).

2. **Layer A.2 — skill-prompt edit:** add same instruction to `skills/bundled/self-dev-callback/system_prompt.md`. Position: near the top of the system prompt, in the "response style" section if one exists, else as a new top-level rule.

3. **Layer B.1 — detector helper:** new function `detect_non_latin_script(text: &str) -> bool` in `crates/mika-agent/src/agent_loop/mod.rs` (or wherever post-condition helpers live). Scans codepoints; returns true if ≥ 5 codepoints are in the named CJK/Hangul ranges. Pure function, unit-testable.

4. **Layer B.2 — guard wiring:** insert call-site in `run_loop`'s EndTurn chain **AFTER `persistence-eval` (#648)** — as a response-quality gate, not a fabrication-class guard (architect F1 BLOCKING — CJK bleed is wrong-language regression, not grounding failure). Ordering: `assert-grounded` → `persistence-eval` → `lang-quality-guard` → persistence-return. Single-retry tracked via `intent_guard_retries` HashSet with label `"non_latin_script"`. **Correction message MUST quote the offending CJK substring (first 100 chars)** so GLM-5.2 sees what it emitted (architect F4 — improves self-correction efficacy per `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).

5. **Tests (AC5):** unit tests in `agent_loop`'s `#[cfg(test)] mod tests`:
   - Pure-ASCII text → detector returns false.
   - 5+ CJK codepoints → detector returns true.
   - 4 CJK codepoints → detector returns false (threshold boundary).
   - Mixed ASCII + Japanese kanji → detector returns true.

6. **Smoke verification (AC4):** post-deploy, query `messages` table for new CJK content over 24h window. Document the query in PR body. Expected: zero (or near-zero) matches after Layer A+B land.

## Acceptance criteria

- **AC1** — Hard evidence captured in issue body (DONE — 10 instances in last 2 hours, sample message 引擎已确认...).

- **AC2 — Layer A + C (prompt-layer, both swapped agents):** English-only instruction added to all four sites: `MIKA_DEV_IDENTITY` + `MIKA_QA_IDENTITY` (architect F2 BLOCKING — mika-qa swap merged today, same model class, deferring leaves vulnerability window) AND `skills/bundled/self-dev-callback/system_prompt.md` + `skills/bundled/qa-review/system_prompt.md`. PR diff visible.

- **AC3 — Layer B (engine, response-quality gate):** `detect_non_latin_script()` helper + EndTurn guard call-site placed AFTER `persistence-eval` (#648) — response-quality class, NOT fabrication class (architect F1 BLOCKING). Triggers on ≥ 5 codepoints from CJK Unified Ideographs / CJK Symbols / Hiragana / Katakana / Hangul ranges. Threshold rationale (architect F3): "5 catches observed emission density (sample = 576 chars CJK, well above threshold); raise to 10 if false positives observed on foreign-issue-body quotes" — documented in code comment. Single retry via `intent_guard_retries` ("non_latin_script" label). **Retry message MUST quote the offending CJK substring (first 100 chars)** per architect F4. Second occurrence in same turn → accept + `warn!` log event `non_latin_script_persistent`.

- **AC4 — Smoke verification:** post-deploy, the `messages` table CJK-content query returns ZERO matches for mika-dev over 24h. Documented in PR body with the query + result.

- **AC5 — Unit tests:** detector helper unit tests covering pure-ASCII (false), 5+ CJK (true), 4 CJK boundary (false), mixed-script (true). Located in the helper's module's `#[cfg(test)] mod tests`.

## Out of scope

- **Reverting glm-5.2 swap.** Calibration-discipline-gated (mika#1190). Cost-reduction story requires the swap; this fix is the structural compensation for the model class's CJK-bleed tendency.
- **mika-qa identity/skill-prompt edits** (Layer C). Defer to follow-up — Layer B's engine guard covers mika-qa structurally; if mika-qa needs persona-level reinforcement, file follow-up after observation.
- **Font/terminal recommendations.** The font issue is symptom, not cause. Documenting CJK-capable fonts for the operator only matters if Layer A+B fail to bind — in which case operators still see broken Chinese (not English). Out of scope.
- **General Unicode normalization** (stripping all non-Latin to `?`, etc.). Too aggressive — emoji, mathematical symbols, accented Latin characters are legitimate. Scope is specifically CJK/Hangul-bleed from model-output mode-confusion.

## Files involved

- `crates/mika-agent/src/well_known_agents.rs::MIKA_DEV_IDENTITY` — Layer A.1 instruction
- `skills/bundled/self-dev-callback/system_prompt.md` — Layer A.2 instruction
- `crates/mika-agent/src/agent_loop/mod.rs` (or `post_conditions.rs` if extracted) — Layer B.1 detector helper + Layer B.2 guard wiring + tests

## Verification

- **Static:** PR diff shows three edits — identity const, skill prompt, agent_loop helper+guard+tests.
- **Unit (AC5):** `cargo test -p mika-agent agent_loop::tests::non_latin_script_*` — four cases covered.
- **Integration:** existing mika-dev calibration scenarios stay green (no regression on Latin-only test fixtures).
- **Empirical (AC4):** post-deploy, run `sqlite3 ~/.mika/data/mika.db "SELECT COUNT(*) FROM messages WHERE created_at > '<deploy-time>' AND content GLOB '*[一-龥]*';"` → 0.

## References

- mika#1633 — glm-5.2 swap on mika-dev
- mika#1657 — Z.AI native provider
- mika#1670 / PR #1673 — mika-qa swap to glm-5.2 (sibling regression vector)
- mika#862 — asserted-unavailability guard (engine-guard pattern this fix mirrors)
- mika#1331 — assert-grounded guard (sibling fabrication-class detector)
- `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate.md` — why Layer A alone is insufficient
- Operator screenshot 2026-06-30 ~14:30Z (in operator-CC conversation `bba3bcac`)
- Hard evidence DB query: `sqlite3 ~/.mika/data/mika.db "SELECT created_at, length(content), substr(content, 1, 100) FROM messages WHERE created_at > '2026-06-30T13:00:00Z' AND role='assistant' AND content GLOB '*[一-龥]*'"` (10 rows returned)
- Sample CJK message (14:48:48Z): `引擎已确认：issue_comment.created webhook 不被授权进行 dispatch (mika#933)。`
