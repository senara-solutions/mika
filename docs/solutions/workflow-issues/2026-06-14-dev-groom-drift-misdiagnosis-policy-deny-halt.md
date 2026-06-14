---
module: skills/bundled/_shared/dispatch-lib, claude-pilot-py
tags: [autonomous-loop, dev-groom, claude-pilot, policy-deny, interrupt-true, dispatch-lib, drift-detection]
problem_type: investigation
category: workflow-issues
date: 2026-06-14
ticket: mika#1318 (related)
applies_when:
  - Investigating dev-groom sessions that report "Session drifted into executor mode"
  - Authoring post-freeze fixes for dispatch-lib's drift detection
  - Widening claude-pilot-py tier1 policy allow-list
resolution_type: investigation_finding
---

# dev-groom "drift" misdiagnosis — policy-deny-induced early halt

## TL;DR

Today's three "drift-into-executor-mode" failures (#96 groom, #624 groom, #625 groom) are **NOT actually LLM drift**. They are pilot sessions halted mid-flight by `interrupt=True` from claude-pilot-py's policy classifier on legitimate research bash commands. dispatch-lib's post-flight message conflates this failure mode (Class C) with mika#1033's genuine LLM drift (Class A) and mika#1097's zero-artifact exit (Class B).

The substrate gaps are in **claude-pilot-py tier1**, not in dev-groom's prompt.

## Founding incidents — 2026-06-14

| Ticket | Session | Cost | Halted on |
|---|---|---|---|
| mika#96 (groom) | `ab665c9c-...` | $0.62, 15t | `grep -r "pub fn delete_word\|pub fn delete_line_by_head\|WordBack\|WordForward" /home/samidarko/.cargo/registry/src/*/tui-textarea-*/src/` |
| mika#624 (groom rescue session) | `1f701234-...` | $0.56, 13t | `gh auth status 2>&1 \| head -10` |
| mika#625 (groom) | `8ff405b7-...` | $0.56, 15t | `gh release view --repo senara-solutions/mika --json tagName,assets 2>/dev/null \| jq '{tag: .tagName, assets: [.assets[].name]}' \| head -30` |

All three sessions exit with `interrupt=True` halt mid-research. dispatch-lib's post-flight then runs `_iterate_groom_loop` (which fails because the architect canvass can't complete on incomplete input) AND the structural plan-file check (which fails because no plan was produced), producing the conflated message:

> PIPELINE FAILURE: architect convergence did not complete (_iterate_groom_loop returned non-zero). Plan exists on branch but architect verdict is missing.
> PIPELINE FAILURE: dev-groom produced no valid plan file ... no /ce:plan invocation detected in session log. Session drifted into executor mode.

The first sentence accurately describes the immediate symptom; the second is a misdiagnosis — the pilot did not "drift into executor mode," it never got to execute anything because policy denied its first research move.

## Three distinct failure classes — disambiguation

Today's failures muddle the diagnostic vocabulary. The doctrine should treat these as three separate classes:

### Class A — LLM drift from planner to executor (mika#1033)

- **Symptom**: pilot makes tool calls, but they are workflow commands (cargo build, gh pr create, edit) instead of `/ce:plan` invocation.
- **Cause**: ticket body's imperative verbs prime the LLM to mode-switch.
- **Existing guard**: ROLE CONSTRAINT block at the head of `dev-groom/system_prompt.md`; structural post-flight check for plan-file >500 bytes.
- **Detection signal**: `tool_calls` table shows non-`/ce:plan` invocations during the session window.

### Class B — Zero-artifact exit (mika#1097)

- **Symptom**: session exits `Success` with **zero** tool calls (no `[tool:request]` lines), zero content blocks, ~12 turns of empty SDK turns.
- **Cause**: prompt block position confusing the model into inhibiting tool use entirely.
- **Existing guard**: `--trace` diagnostic instrumentation; structural plan-file check.
- **Detection signal**: zero rows in `tool_calls` for the session_id.

### Class C — Policy-deny-induced early halt (today's pattern, undiagnosed)

- **Symptom**: pilot makes `[tool:request]` calls, claude-pilot logs `[policy:deny]` on a research bash command, session terminates via `interrupt=True` mid-grooming.
- **Cause**: research bash command falls outside tier1 allow-list AND outside tier2 policy allow-list, triggering default-deny + halt (cpp#20 joint 2).
- **Existing guard**: **none for diagnosis**. The structural check sees no plan, reports it as drift; the operator-facing message wrongly implicates the LLM.
- **Detection signal**: `[policy:deny]` line in `/var/log/claude-pilot/<task_id>.{log,stderr}` before the rescue write.

## Substrate gaps surfaced by today's incidents

Three concrete claude-pilot-py tier1 gaps were exercised:

### Gap 1 — Quote-blind compound splitter (FIXED in cpp#31, merged 2026-06-14 14:17Z)

`_split_compound_command` was a single quote-blind regex that matched `|` inside `"..."`. Pre-fix, grep with regex alternation (`grep "a\|b\|c"`) was shredded into nonsense segments, every segment failed safe-list, and chain-safety vetoed the policy allow rule. The fix is a hand-written quote-aware tokenizer mirroring `contains_unquoted_metacharacter` POSIX semantics. 177/177 tier1 tests pass.

mika#96 groom's `grep -r "pub fn ..."` was this class.

### Gap 2 — `bash-jq` policy regex misses pipe-to-jq

Current regex: `^(for\s.*do\s+.*\s)?jq\s|;\s*jq\s` — matches `^jq ` and `; jq ` only.

The pipe-to-jq idiom `cmd | jq '...'` is **not** matched. This is the dominant usage pattern in research bash (gh + jq, cat + jq, curl + jq). With the quote-aware splitter, the segment after `|` is bare `jq '...'` which:
- Is not tier1-safe (jq isn't in SAFE_SHELL_COMMANDS)
- Doesn't match the bash-jq policy regex (anchored to `^` or after `;`)
- Falls through to default-deny

mika#625 groom's `gh release view ... | jq '...' | head` was this class.

**Proposed fix**: extend the bash-jq pattern to cover pipe-to-jq:

```yaml
- id: bash-jq
  tool: Bash
  pattern: "^(for\\s.*do\\s+.*\\s)?jq\\s|[;|]\\s*jq\\s"
  decision: allow
```

(Single char-class addition: `[;|]` instead of `;`. Quote-aware splitter still gates compound safety on each segment.)

### Gap 3 — `SAFE_GH_SUBCOMMANDS` missing common research verbs

Current allow-list:
- `pr`: create, view, list, checkout, diff, checks
- `issue`: view, list, edit, comment
- `run`: view, list
- `repo`: view
- `release`: view, list
- `workflow`: view, list

Missing common-and-safe verbs the pilot reaches for during research:
- `gh auth status` (mika#624 halt) — read-only, surfaces credential state, no side effects
- `gh auth token` — read-only, but emits secret to stdout — should NOT be in allow-list (leak risk)

**Proposed fix**: extend `auth` to allow `status` only:

```python
"auth": frozenset({"status"}),
```

Other candidates to investigate: `gh cache list`, `gh secret list` (NO — leaks names), `gh variable list`, `gh gist list`, `gh extension list`, `gh search code/issues/prs`.

## Proposed dispatch-lib disambiguation (post-freeze)

dispatch-lib's drift detection should distinguish Class A/B from Class C by grepping the session stderr for `[policy:deny]` before declaring drift.

Conceptual change to `skills/bundled/_shared/dispatch-lib.sh`:

```bash
# After the existing plan-file + /ce:plan check, before declaring drift:
SESSION_STDERR="/var/log/claude-pilot/${LOG_ID}.stderr"
POLICY_DENY=""
if [ -r "$SESSION_STDERR" ]; then
    POLICY_DENY=$(grep -oE '\[policy:deny\] [A-Za-z]+: [^[]+\[[a-z-]+\]' "$SESSION_STDERR" | head -1)
fi

if [ -n "$POLICY_DENY" ]; then
    RESULT="PIPELINE FAILURE: session halted by policy deny on tool — ${POLICY_DENY}.

Likely a tier1/policy allow-list gap in claude-pilot-py. Investigate the deny rule and either (a) widen the policy to include the legitimate research command, or (b) rewrite the dispatch context so the pilot avoids the denied command shape. This is NOT LLM drift — the pilot was prevented from doing its work.

${RESULT}"
elif [ -z "$VALID_PLAN" ] && [ "$CE_PLAN_INVOKED" != "1" ]; then
    # Existing Class A drift message
    ...
fi
```

This is a strictly additive log-message change with no behavioral change to the grooming pipeline. Safe to ship without freeze-pivot risk because it does not modify the dev-groom skill prompt or the architect-convergence loop logic.

## Post-freeze fix sequencing (2026-06-18 onward)

| Fix | Layer | Risk | Why this order |
|---|---|---|---|
| 1. bash-jq pattern widening | cpp/policies/permissions.yaml | LOW (regex addition only) | Unblocks most common pipe-to-jq research idioms |
| 2. `gh auth status` allow | cpp/tier1.py SAFE_GH_SUBCOMMANDS | LOW | Tiny additive change, restricted to `status` verb |
| 3. dispatch-lib drift disambiguation | mika/skills/bundled/_shared/dispatch-lib.sh | LOW (log-message only) | Operator clarity improvement; no pipeline behavior change |
| 4. Audit other `gh` subcommands | cpp/tier1.py | MEDIUM (more verbs, more attack surface) | Defer until #1-3 measured |

## Out of scope for this investigation

- mika#1410 (recoverable denials for the LLM) — separate substrate problem; this investigation does not change that ticket's scope.
- dev-groom skill prompt changes — the freeze-pivot subject; not implicated in today's failures.
- The architect-convergence loop in `_iterate_groom_loop` — failed downstream of the policy deny; not the root cause.

## Evidence trail

- mika#96 groom callback task: `99e104e2-613e-46e4-99f4-e2c66f8c3d9c` (mika.db)
- mika#624 groom rescue session: `1f701234-98ce-4da0-99d9-37c31fd9f4e3` (recovery task `4614fbe9-...`)
- mika#625 groom callback task: `545f9918-3bf7-4aad-bb25-0b985f76bf16` (mika.db)
- cpp#31 (quote-aware splitter fix): merged `94ddf4d` 2026-06-14 14:17Z
- Prior art: `dev-groom-zero-artifact-exit-2026-05-13.md`, `dev-groom-drift-detection-structural-validation-2026-05-11.md`

## Hand-off

This document is operator/architect-canvassable post-freeze (2026-06-18). A canvass should:
1. Verify the bash-jq + SAFE_GH_SUBCOMMANDS gaps against current cpp main (the fixes may already have shipped in another PR).
2. Decide whether the dispatch-lib disambiguation belongs in mika#1097's lineage (zero-artifact lineage) or as a standalone ticket.
3. Sequence the three proposed fixes against the rest of the 06-18 mission slate.
