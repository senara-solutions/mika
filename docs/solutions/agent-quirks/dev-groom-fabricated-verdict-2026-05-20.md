---
module: mika-agent/skills
tags: [fabrication, dev-groom, verdict, dispatcher, manifest, post-condition-guard]
problem_type: agent-fabrication
category: agent-quirks
created: 2026-05-20
issue: mika#1133
---

# dev-groom fabricated "Verdict: GROOMED" without dispatching architect

## Attribution

Observed 2026-05-15 on mika#920. Operator dispatched `mika ask --agent mika-dev "groom mika issue#920"` against a ticket with plan committed but no architect review record. Response carried `"groomed ✅, ... Verdict: GROOMED"`. Verification: issue body still started with `## Symptom` — no callout block written back. mika-arch was never called.

## Root cause

**Manifest pattern mismatch.** `dev-groom/skill.toml` declared the **producer** manifest shape (`[output] required_suffix_lines`) without the **dispatcher** shape (`[constraints] required_tools`). Producer skills (mika-arch-groom-ticket, mika-arch-second-review) produce the Verdict themselves — `required_suffix_lines` is correct there. Dispatcher skills (self-dev, dev-pilot, dev-groom) forward work to claude-pilot; the Verdict arrives later via callback.

Putting `required_suffix_lines` on a dispatcher creates fabrication pressure:
1. LLM dispatches (or doesn't) and emits a text response
2. Suffix-line guard (#864, post-condition #8) rejects EndTurn for missing Verdict
3. Corrective re-prompt names the accept-set verbatim ("one of: Verdict: GROOMED, Verdict: ESCALATE")
4. LLM rationalizes by appending `Verdict: GROOMED`

The required-tools gate (#3) would have caught the missing dispatch, but `dev-groom` didn't declare `required_tools` — so the LLM was free to respond conversationally without calling `run_claude_pilot_groom`, leaving the suffix-line guard as the only enforcement, which the LLM satisfies by fabrication.

## Five-phase fix

### F1 — Manifest shape (root cause)

`skills/bundled/dev-groom/skill.toml`: Removed `[output] required_suffix_lines`, added `[constraints] required_tools = ["run_claude_pilot_groom"]`. Mirrors the canonical dispatcher pattern from `self-dev/skill.toml`.

### F2 — Prompt update (self-explanation)

`skills/bundled/dev-groom/system_prompt.md`: Added explicit prohibition: "Do NOT emit Verdict: GROOMED or Verdict: ESCALATE in your dispatch response." Qualified callback bullets with "On Verdict: GROOMED **callback**". Per `feedback_prompt_enforcement_fragile.md`, prompt-level "do not" is weak — F3 is the binding enforcement.

### F3 — Structural fabrication guard (defense-in-depth)

`crates/mika-agent/src/agent.rs`: New post-condition guard at position 5b (after #5 fabricated-action guard, before #6 intent-precondition registry). Detects `Verdict: GROOMED` / `Verdict: ESCALATE` in conversation-mode text without a successful `run_claude_pilot_groom` call in the turn.

Gating: conversation mode only (`mode.is_conversation()`). Callback turns legitimately carry Verdict lines from the inner session and must pass through unaffected. Single-retry semantics.

Position rationale: F3 is a claim-legitimacy guard (same family as #4b milestone-close-claim and #5 fabricated-action-claim), not a manifest-driven output-shape validator (#864 / #901). It belongs in the fabrication cluster at positions 4b–5, not the output-validation tail at positions 8–9.

### F4 — Regression tests

Four eval scenarios in `tests/eval/grounding_regressions/`:
- `dev_groom_fabricated_verdict_caught` — GROOMED variant
- `dev_groom_fabricated_verdict_escalate_caught` — ESCALATE variant
- `dev_groom_dispatched_no_verdict` — happy path (clean dispatch text)
- `dev_groom_status_response_no_verdict` — status question (no Verdict claimed)

Static parity test in `tools/mod.rs`: `test_dispatcher_skills_dont_declare_required_suffix_lines` — guards against future dispatcher skills acquiring the broken producer shape.

### F5 — This compound doc

## Dispatcher vs producer pattern delineation

| Skill | Role | `required_tools` | `required_suffix_lines` |
|---|---|---|---|
| `self-dev` | dispatcher | `["run_claude_pilot"]` | (none) |
| `dev-pilot` | dispatcher | (none — relies on `self-dev` parent) | (none) |
| `dev-groom` | dispatcher | `["run_claude_pilot_groom"]` | (none) |
| `mika-arch-groom-ticket` | **producer** | `["gh_read"]` | `["Disposition: READY/ITERATE/ESCALATE"]` |
| `mika-arch-second-review` | **producer** | `["gh_read"]` | `["Verdict: GROOMED/ESCALATE"]` |

**Rule:** `required_suffix_lines` is for LLM-produced verdicts. Never for dispatch-and-wait flows where the verdict arrives via callback.

## Verification

Pre-merge:
1. `cargo build --release` — clean compile
2. `cargo test -p mika-agent --test eval dev_groom_fabricated_verdict_caught` — F3 catches fabrication
3. `cargo test -p mika-agent --test eval dev_groom_dispatched_no_verdict` — F3 doesn't block legitimate dispatch
4. `cargo test -p mika-agent --lib test_dispatcher_skills_dont_declare_required_suffix_lines` — parity test passes
5. `cargo clippy --all-targets -- -D warnings` — clean

Post-deploy:
1. Dispatch `mika ask --agent mika-dev "groom mika issue#<unreviewed>"` against a freshly-filed ticket
2. Response should contain "Dispatched", a task ID, and NOT contain "Verdict: GROOMED"
3. `grep dev_groom_fabrication_guard $MIKA_SPIRIT_LOG_FILE` — empty on healthy ticks

## Lessons

1. **Manifest shape inversion across skill categories is a silent bug-class.** The `test_dispatcher_skills_dont_declare_required_suffix_lines` parity test catches the next one at build time.
2. **`required_suffix_lines` is for LLM-produced verdicts; never for dispatch-and-wait flows.** The delineation is now documented in this compound doc and the CLAUDE.md post-condition chain.
3. **Engine-level structural guards beat prompt-level "do not."** F3 is the binding enforcement; F2 is for self-explanation and operator-readable documentation. This matches the pattern from `feedback_prompt_enforcement_fragile.md`.

## No parallel risk on mika-qa

Confirmed: qa-review embeds the verdict trailer inside the `gh pr review --body` argument via `required_tool_arg_suffixes` (pre-spawn validation, mika#899), and its `required_tools = ["qa_pr_view", "run_gh", ...]` forces real GitHub API calls. The verdict is produced server-side by GitHub, not by the mika-qa LLM. Different mechanism, no fabrication surface.
