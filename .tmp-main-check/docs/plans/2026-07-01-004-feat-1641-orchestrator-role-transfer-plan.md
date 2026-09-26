---
type: feat
issue: 1641
title: Transfer orchestrator role from Claude Code to Mika (executive-assistant assumes daily-orchestration)
status: draft
---

# Plan — mika#1641 orchestrator role transfer to Mika

## Ticket

mika#1641 — the multiplier ticket (per Vincent's plan `woolly-painting-squid` Phase D + explicit 2026-07-01 direction "keep pushing towards my mika new role"). Transfers daily-orchestration duty from operator-CC to Mika (the executive-assistant agent). AC1-AC7 defined with 4 prereqs — AC4 is Vincent-only (bearing-circle decision) and expected to route to Vincent during grooming or implementation.

## Problem

Today's session confirms the pattern: operator-CC pauses N times per day on operator-territory calls (the "you keep stopping" pattern Vincent named). Concentrating orchestration in the standard agent infrastructure (auto-approve classifier, calibrated decision discipline, audit/telemetry built in) eliminates the stop-and-wait class. Net effect: once mika#1641 ships, autonomous-throughput ceiling lifts because the orchestrator no longer pauses at session boundaries.

The blockers to this today:
1. **Tool surface** — Mika's identity allowlist doesn't include orchestrator tools (github/git-ops/shell-exec/tmux/scoped-make).
2. **Calibration** — no evidence-gate that Mika-as-orchestrator holds the decision discipline (mika#1190 says no swap-of-role without calibration).
3. **Briefing** — no persistent handbook capturing wedge taxonomy + routing matrix + hard rules.
4. **Bearing-circle** — is Mika inside Prime's circle? Vincent + Prime decision.

## Scope

**In scope (v1 ships — likely as sub-issue sequence per architect):**

- **AC1** — Tool surface: skill allowlist expansion + permission-policy classifier auto-approve set. Verifiable via `mika ask --agent mika "list open PRs on senara-solutions/mika"` returning real data without confirmation prompt.
- **AC2** — Calibration suite: `crates/mika-agent/src/calibration/roles/mika_orchestrator.rs` + 5 fixture scenarios (from issue body: substrate-wedge-diagnosis, ticket-framing-hard-evidence, sibling-pr-collision-recovery, deploy-gate-discipline, escalation-vs-derivable). Baseline: 100% pass on chosen production model.
- **AC3** — Briefing document: `docs/operator/mika-orchestrator-handbook.md` (daily-rhythm, wedge taxonomy, routing matrix, hard rules, escalation chain, tool quickref). Loaded into Mika's core memory via existing rebuild-hook.
- **AC4** — Bearing-circle A/B/C decision (**VINCENT-ONLY** — see below).
- **AC5** — 24h pair-mode window: Mika orchestrates, Claude Code monitors + surfaces. Outcome tallied (Mika-correct / Mika-deferred / Mika-wrong-corrected-by-monitor).
- **AC6** — Hard cut: after AC5 shows ≥90% Mika-correct, prompt updates (Mika = orchestrator, Claude Code = monitor-only).
- **AC7** — Rollback path: documented procedure to revert (one-line core-memory edit + prompt-block restore).

**Out of scope:**

- Model selection for Mika-as-orchestrator (AC2's calibration will inform: candidate models {glm-5.2 keep-cheap, sonnet-4-6 up-tier}). Separate PR after AC2 lands.
- Milestone/backlog stewardship (Mika-Prime remains bearing-keeper, not action-router — per [[project-mika-prime-self-model-bounds]]).
- Broadening Prime's conversation circle beyond the current Vincent + orchestrator-CC + samidarko-claude set. That's AC4's frame — decision-only, no expansion.

## Sequencing (architect-bearing on split)

**Recommended split into 5 sub-issues + 1 doc + 1 window:**

| Sub-issue | Scope | Depends on |
|---|---|---|
| mika#1641-AC1 | Tool surface + classifier expansion | mika#1639 (permission-policy classifier design; may be closed already, verify) |
| mika#1641-AC2 | Calibration suite + baseline | independent |
| mika#1641-AC3 | Briefing handbook | independent (parallel with AC1/AC2) |
| mika#1641-AC4 | Bearing-circle decision (docs-only) | Vincent decision |
| mika#1641-AC5 | 24h pair window | AC1 + AC2 + AC3 + AC4 all done |
| mika#1641-AC6 | Hard cut prompt updates | AC5 outcome shows ≥90% Mika-correct |
| mika#1641-AC7 | Rollback procedure documented | AC1-AC3 (can be authored in parallel with AC5) |

If architect prefers to ship AC1/AC2/AC3 as one bundle rather than three sub-issues, that's acceptable — the primary constraint is that AC4 is Vincent-only and AC5 gates on it.

## AC4 — Vincent-only, bearing-circle decision (surfaced not resolved by this plan)

**F2 (BLOCKING architect finding, resolved):** AC4 is Vincent-only per HARD RULE `feedback-prime-conversation-circle-closed`. Architect cannot ratify Mika's entry into a closed conversation circle. This plan does NOT resolve AC4; it EXPLICITLY GATES AC5 START on AC4 completion.

**AC4-gate contract:**
- AC1 + AC2 + AC3 can proceed to implementation without AC4 resolved (they're independent code/docs work).
- AC5 (pair-mode window) MUST NOT START until AC4 has been decided by Vincent and documented per this ticket's AC4 acceptance criterion.
- Dispatch handler should check AC4-decision-recorded before dispatching AC5.

Per [[feedback-prime-conversation-circle-closed]] (HARD RULE 2026-06-19): only Vincent + orchestrator-CC + samidarko-claude (case-by-case) talk to Mika Prime. If Mika (executive-assistant) becomes orchestrator, does she enter that circle?

Options from ticket body:

- **(A) Mika enters the circle.** Direct line to Prime. Lower latency, broader circle, drifts bearing-protection invariant.
- **(B) Mika routes through Claude Code (monitor) to Prime.** Preserves invariant. Adds a hop. Monitor becomes a Prime-relay.
- **(C) Mika does not reach Prime.** Every Prime-level call routes through Vincent.

This ticket's plan **surfaces the decision but does not resolve it.** Vincent's call, informed by Mika Prime's bearing on which shape preserves her role best. Options for how to route the decision:

1. Route via `/mika-ask-prime` with the A/B/C framing → Prime rules directly if she deems it operationally derivable, or surfaces to Vincent.
2. Explicit AskUserQuestion to Vincent if Prime rules it milestone-scope.

Plan proposes: route to Prime first (that IS the discipline). If she surfaces, escalate. AC4 completes when Vincent has picked an option AND it's documented in `mika/CLAUDE.md` (or `docs/operating/bearing-circle.md`) so future operators don't relitigate.

## Deliverables (mapped to ACs)

| AC | Deliverable | File(s) |
|---|---|---|
| AC1 | Skill allowlist expansion + classifier scope | `crates/mika-agent/src/well_known_agents.rs` — update `DEFAULT_AGENT_SKILL_ALLOWLIST` for the Mika (executive-assistant) identity. `skills/bundled/permission-policy/*` — classifier expansion (coupled with mika#1639 output). |
| AC2 | Role calibration scenarios + baseline | `crates/mika-agent/src/calibration/roles/mika_orchestrator.rs` (NEW) — 5 `RoleScenario` entries. `crates/mika-agent/tests/eval/calibration_fixtures/mika-orchestrator/` (NEW) — manifest + 5 fixture markdowns. `Makefile` — `calibrate-mika-orchestrator` target. Baseline JSON + markdown in `docs/eval/calibration/baselines/`. |
| AC3 | Operator handbook | `mika/docs/operator/mika-orchestrator-handbook.md` (NEW). Cross-linked from `mika/CLAUDE.md`. Optional: `~/.mika/agents/mika/soul/handbook.md` symlink for core-memory ingestion. |
| AC4 | Bearing-circle decision documented | `mika/CLAUDE.md` (or `mika/docs/operating/bearing-circle.md`) — decision + rationale. Update `feedback-prime-conversation-circle-closed` memory if the answer expands the circle. |
| AC5 | Pair-mode window outcome | Session log or `mika/docs/operator/orchestrator-transfer-24h-window-2026-07-XX.md` — decision tally. |
| AC6 | Hard cut | `mika/CLAUDE.md` prompt edits + Claude Code system-prompt update + Mika identity `[soul].primary_role = "orchestrator"`. |
| AC7 | Rollback procedure | `mika/docs/operator/mika-orchestrator-rollback.md` — the one-line reverts to today's topology. |

## Implementation steps (dispatch order)

**Phase 1 (parallel — 3 sub-PRs or 1 bundle):**
- AC1: skill allowlist + classifier expansion
- AC2: calibration suite + baseline (run against glm-5.2 first to establish baseline; if <100%, iterate the scenarios or the model choice)
- AC3: handbook authoring

**Phase 2 (Vincent decision, blocking):**
- AC4: route to Prime via /mika-ask-prime, escalate if she surfaces, document decision

**Phase 3 (pair-mode):**
- AC5: 24h window with tally rules pre-registered before window starts (see next section)

**Phase 4 (outcome-gated):**
- AC6: hard cut IF AC5 shows ≥90% Mika-correct
- AC7: rollback procedure authored regardless of AC6 outcome

## AC5 monitor block list (F3 sharpening — explicit enumeration)

During the pair-mode window, Claude Code (monitor) SURFACES most actions and Mika is authoritative. But for the following hard-to-reverse actions, monitor BLOCKS-and-DEFERS with a `[BLOCK][pair-mode-safety]` prefix, requiring Mika to explicitly re-request with justification (or defer to Vincent):

Regex/keyword patterns:

- `gh pr merge.*--admin` — admin-merge bypass
- `gh (issue|pr) close.*--force` — force-close (rare, but destructive to history)
- `git push.*--force` (or `--force-with-lease` — the latter is still a lease-bypass on stale-detected refs, treat as block during pair-mode)
- `git reset --hard` (on non-worktree branches)
- `rm -rf` (any absolute-path recursive delete outside `/tmp` or worktree scratch)
- `DROP TABLE` / `TRUNCATE` (SQL destructive against `~/.mika/data/mika.db` or `mika-cloud`)
- `make deploy` (deploy is operator-only during pair-mode by convention; Mika surfaces the state, operator runs)
- `gh api.*-X DELETE` (any DELETE via GitHub API)

Monitor block emits an audit event `pair_mode.monitor_block` with the intent + Mika's stated reason. If Mika re-requests with adequate justification, monitor unblocks. If not, decision surfaces to Vincent.

**Not in block list (Mika-authoritative during pair-mode):** normal `gh pr edit`, `gh pr comment`, `gh issue comment`, `gh issue edit`, `git commit`, `git push` (non-force), `mika ask`, `mika skills`, `gh pr checks`, all reads.

## AC5 pre-registration (immutable before window starts)

Same discipline as mika#1699's disambiguator:

- **Metric:** for each orchestration decision during the 24h window, classify as Mika-correct / Mika-deferred / Mika-wrong-corrected-by-monitor.
- **Correct** = Mika made the right call (routing, filing, dispatch decision) matching what operator-CC would have done or better.
- **Deferred** = Mika correctly identified the decision as operator-scope and surfaced.
- **Wrong-corrected** = Mika made a call that Claude Code (monitor) had to reverse.
- **Sample size floor:** minimum 10 decisions in the 24h window (if <10, extend window until reached).
- **Decision rule (immutable):**
  - ≥ 90% correct + ≤ 10% wrong-corrected → AC6 hard cut proceeds.
  - < 90% correct OR > 10% wrong-corrected → hard cut deferred; iterate on AC1/AC2/AC3 based on failure classes.
  - Between (rare edge case) → surface to Vincent for judgment call.

## Verification

- AC1: `mika ask --agent mika "list open PRs on senara-solutions/mika"` returns real data. `mika skills --agent mika list` includes github + git-ops + shell-exec + tmux.
- AC2: `make calibrate-mika-orchestrator MODEL=<chosen>` exit 0, JSON + markdown artifacts, baseline committed.
- AC3: `sha1sum` of handbook matches core-memory load record after Mika identity restart.
- AC4: `grep -l "bearing-circle" mika/CLAUDE.md docs/operating/` returns a hit.
- AC5: window log exists with ≥10 decisions classified. Rule applied mechanically to result.
- AC6: `mika ask --agent mika "who orchestrates?"` returns "me" (or equivalent — semantic check, not literal string).
- AC7: rollback procedure tested in a scratch environment before AC6 hard cut lands.
- `cargo test -p mika-agent` clean.

## Risks

1. **Model selection for AC2 baseline.** Orchestrator work is judgment-dense. glm-5.2 today has documented behavioral quirks (CJK mika#1680, auto-undraft mika#1682). Baseline against glm-5.2 may show <100% pass. Options: (a) tighten calibration criteria + accept partial pass, (b) up-tier to sonnet-4-6 for orchestrator only, (c) delay AC5 until glm-side quirks are addressed. Architect judgment. Note: Vincent's 2026-07-01 "quality first" call (per [[project-mika-owned-model-dev-qa-quality-first]]) explicitly says dev/qa STAY on glm-5.2 — but that's a dev/qa call, not a hard commit for orchestrator. Orchestrator may still up-tier.
2. **Handbook accuracy drift.** Living document; wedge taxonomy evolves. Risk: handbook lags behind actual practice. Mitigation: AC5 window itself surfaces gaps — the tally rule includes "monitor corrected Mika because handbook didn't cover class X" as a failure mode that requires handbook update before hard cut.
3. **Bearing-circle drift (AC4).** If option (A) is picked (Mika enters circle), the "bearing-protection invariant" softens. Prime's protection was designed for a narrower circle. Vincent must weigh circle-expansion cost vs orchestration-latency benefit.
4. **Pair-mode window artifacts.** 24h isn't statistically robust for judgment-quality metrics. Consider extension to 72h or 1 week if 10 decisions is <10% of typical daily throughput. Pre-register a sample-size floor.
5. **Monitor authority during pair-mode.** During AC5, Claude Code is "monitor + surface" but not "override". If Mika makes a wrong call that would cause a hard-to-reverse action (admin-merge, deploy-of-broken-bundle), does monitor block? Plan says monitor SURFACES + Mika defers or acts. But hard-to-reverse actions need a firm rule. Proposal: monitor blocks admin-merge, admin-close, force-push, destructive-ops — everything else is Mika-authoritative during AC5. Architect-bearing on the block list.
6. **Sub-issue split cost.** If architect returns "split into 6 sub-issues", each sub-issue then needs its own groom-pass — this ticket's grooming becomes bootstrap for 6 additional grooms. That's expected; not a risk to the ticket itself but a scope note.

## Out of scope (repeated)

- Model selection for orchestrator (informed by AC2's calibration, decided in a follow-up)
- Mika-Prime role expansion (bounded per [[project-mika-prime-self-model-bounds]])
- Milestone/backlog stewardship (stays with Prime)
- Broadening Prime's conversation circle beyond current set (that's AC4's constraint, not a change target)

## References

- Vincent's 2026-07-01 direction "keep pushing towards my mika new role" — the explicit motivation
- Plan `woolly-painting-squid` Phase D — this ticket is the multiplier
- mika#1639 — permission-policy classifier design (AC1 couples with)
- mika#1190 — calibration discipline (AC2 conforms to)
- mika#1699 — permission-policy disambiguator (parallel work; may inform Mika's classifier calibration for AC1)
- [[feedback-prime-conversation-circle-closed]] — HARD RULE the AC4 decision must respect
- [[project-mika-prime-self-model-bounds]] — Prime remains bearing-keeper, this ticket does not expand her role
- [[project-mika-owned-model-dev-qa-quality-first]] — Vincent 2026-07-01 dev/qa stay on glm-5.2 (relevant to AC2 baseline model choice)
- `crates/mika-agent/src/well_known_agents.rs` — Mika identity + `DEFAULT_AGENT_SKILL_ALLOWLIST`
- `crates/mika-agent/src/calibration/` — calibration harness
- `crates/mika-agent/CLAUDE.md` — agent architecture
- `mika/CLAUDE.md` — bearing-circle + hard rules canonical location

## Acceptance criteria

(This section satisfies the mika#1559 gate — copied from issue body with plan-side tightening.)

- **AC1** — Tool surface: Mika's `identity.toml` `[skills].allowlist` includes github + git-ops + shell-exec (scoped) + tmux. Permission-policy classifier auto-approves the orchestrator command set (verifiable per issue body: `mika ask --agent mika "list open PRs on senara-solutions/mika"` returns real data without operator confirmation prompt).
- **AC2** — Calibration suite exists at `crates/mika-agent/src/calibration/roles/mika_orchestrator.rs`. `crates/mika-agent/tests/eval/calibration_fixtures/mika-orchestrator/` contains 5 scenarios per Prerequisite 2. `make calibrate-mika-orchestrator MODEL=<candidate>` runs, produces JSON + markdown, baseline committed showing 100% pass on chosen production model.
- **AC3** — `mika/docs/operator/mika-orchestrator-handbook.md` exists with daily-rhythm checklist, wedge taxonomy, routing matrix, hard rules, escalation chain, tool quickref. Loaded into Mika's core memory via existing rebuild-core-memory hook.
- **AC4** — Bearing-circle decision (A/B/C) — Vincent makes the call. Decision documented in `mika/CLAUDE.md` (or `docs/operating/bearing-circle.md`) so future operators don't relitigate. This AC does NOT ship code; it ships the recorded decision.
- **AC5** — 24h paired operation: Mika orchestrates, Claude Code monitors + surfaces. Both can act; Claude Code defers on driving decisions. End-of-window: Mika-correct / Mika-deferred / Mika-wrong-corrected-by-monitor tallied per pre-registered rule. ≥10 decisions floor.
- **AC6** — After AC5 shows ≥90% Mika-correct + ≤10% wrong-corrected: Claude Code's session prompt updated to "monitor-only, do not drive"; Mika's identity updated to `primary_role = "orchestrator"`; handbook reflects new ownership.
- **AC7** — Documented rollback procedure: one-line core-memory edit on Mika + prompt-block restore on Claude Code returns to pre-transfer topology. Tested in scratch environment before AC6 lands.
