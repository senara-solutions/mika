---
problem_type: workflow-pattern
module: mika-platform
tags: [grooming, operator-claude, dev-groom, mika-arch, dashboard-launch]
date: 2026-05-02
---

# Operator-Claude grooming marathon — closing the dev-groom canary chain (2026-05-02)

## Context

8 tickets groomed end-to-end in a single operator-Claude session, ranging from the original dev-groom canary blocker (mika-platform#76, mika#938, mika#939) through the dashboard-launch sprint (mika-platform#77, mika#929, mika#931) to the milestone#19 closure (mika#927, mika#928). Plus a citation-fabrication follow-up (mika#952). Pattern emerged: when dev-groom autonomous path is unreliable, operator-Claude can replicate the full /mika-groom-ticket flow at scale within a single session, dispatching via `ready` label after each grooming.

## What worked

1. **Reuse worktrees + FF-merge before re-dispatch.** Stale worktrees from killed canaries can be ff-merged onto origin/main; staged-but-uncommitted plans can be moved to /tmp as backup and the dispatch re-fired clean. Saves the cost of full worktree re-derivation.
2. **Skill-disable bypass for mika-arch fabrication.** When `mika-arch-groom-ticket` skill envelope produces persistence-meta hallucination ("No new facts warrant persistence"), `mika skills --agent mika-arch disable mika-arch-groom-ticket` then bare `mika ask --agent mika-arch` delivers real verdicts. Re-enable after grooming.
3. **Plan-doc backup-and-restart pattern.** Killed canaries leave staged plan files in worktrees that block subsequent rebases. Pattern: `cp <plan> /tmp/<backup>`, `git restore --staged <plan>`, `mv <plan> /tmp/`, redispatch. Plan content preserved as starter for next iteration.
4. **Cross-ticket pattern compounding within one session.** F2-style filing-gate pattern (PR-description checkbox + reviewer-blocks-merge) applied across mika#938, mika#939, mika#952. Each plan referenced prior instances; mika-arch acknowledged "N=2 pattern" by mika#939's pass-2.

## What broke (catalog these failure modes)

1. **mika-arch citation-fabrication** (mika#952 filed). Two confirmed instances: mika#931 pass-1 cited a non-existent prior architect session; mika#928 pass-1 fabricated "verbatim" concept lists. Distinct from persistence-meta (mika#947). Self-corrects when challenged in pass-2.
2. **mika-arch persistence-meta** (mika#947 filed). Skill envelope's system prompt triggers memory-tool conditioning. "No new facts warrant persistence from this turn" instead of review.
3. **Opus 4.7 transient-error chain** (mika#939, fixed via PR #941). 3 retry attempts × 2-min timeout = 8 min wall time before agent deadline; mika-spirit emits "I'm sorry, that took too long" fallback. Mitigated by deadline-aware retry abort.
4. **Pipeline-truncation on /mika dispatch** (mika#940, severity downgraded to N=1). claude-pilot exits cleanly after compound doc Write without continuing to gh pr create. Mika#938 hit it; mika#939 didn't (118-turn full pipeline). Not universal pattern.
5. **dev-groom canary chain — 4 distinct deny-cause layers**. (1) relay LLM Bash-classifier (mika#935/PR#937 fixed). (2) Phase 1 step 4 interactive prose (mika-platform#76/PR#78 fixed). (3) backtick-in-message rejection (mika#938/PR#945 fixed). (4) env-var-prefix-cmd-substitution rejection (caught in canary v8 v2; recovers via inside-quotes form). Each layer required its own ticket and fix.

## Calibrated patterns

- **Opus on review work hallucinates** — Vincent confirmed via `feedback_qa_provider_perf.md`. Sonnet 4.6 + bare-ask is the reliable architect invocation pattern.
- **Issue-as-versioned-contract discipline** — when plan reframes a spec item, issue body should be updated post-merge. Plan-vs-issue drift surfaced in mika#928 (acceptance threshold) and mika#931 (R6 ceiling) — architect ratified the divergence at pass-2.
- **Cross-session parametric-memory bleed** — mika-arch's pass-1 on mika#931 conflated session `02cb26ed` (different mika#931 brief) with current brief. Mika#952 KTD-2 addresses via session-id chain anchoring instruction.

## Future-pointer (N=2+ compound triggers)

- mika#947 + mika#952 both ship → author compound doc on mika-arch failure-mode catalog (persistence-meta + citation-fabrication + Opus deadline + LLM routing per-skill).
- mika#940 second instance → upgrade severity, ship structural fix.
- "Operator-Claude grooming marathon" pattern repeats → factor into a CLAUDE.md note about when to bypass dev-groom.

## References

- mika#935/PR#937 (layer 1 relay-deny)
- mika-platform#76/PR#78 (layer 2 spec language)
- mika#938/PR#945 (layer 3 quote-aware pre-classifier)
- mika#939/PR#941 (mika-arch routing + retry abort)
- mika#940 (pipeline truncation, N=1)
- mika#947 (persistence-meta)
- mika#952/mika#953 (citation-fabrication + telemetry follow-up)
- mika#927/mika#928 (KG milestone#19 closure tickets, both groomed today)
