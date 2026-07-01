---
module: agent-core
date: 2026-07-01
problem_type: architecture
component: well_known_agents
severity: medium
tags:
  - orchestrator-role
  - skill-allowlist
  - permission-policy
  - calibration
  - baseline-honesty
  - grounding-discipline
issue: 1641
---

# Orchestrator role transfer: the real tool-surface lever + baseline honesty

## Context

mika#1641 transfers the platform-orchestrator seat from Claude Code to Mika (the
executive-assistant agent). The groomed plan's AC1 deliverable pointed at two
surfaces: (a) the skill allowlist, and (b) `skills/bundled/permission-policy/*`
classifier expansion "coupled with mika#1639". Implementing AC1 surfaced two
non-obvious facts that any future orchestrator/permission work must know.

## Learning 1 — the interactive `mika ask` path has no permission classifier

There are **two** permission surfaces in the platform, and they are easy to
conflate:

- **claude-pilot (headless) dispatch** is gated by the deterministic classifier in
  `claude-pilot-py/tier1.py` (TIER 1/1.5/2/3). This is the `canUseTool` callback
  path for autonomous pilot sessions.
- **`mika ask --agent <name>` (interactive)** has **no** permission classifier. An
  agent's tool reach on this path is gated purely by (1) its `[skills].allowlist`
  and (2) each tool's own validation (e.g. `run_gh`'s three-tier subcommand /
  scope / `gh api`-matrix checks).

AC1's verifiable criterion — `mika ask --agent mika "list open PRs"` returns real
data without an operator confirmation prompt — is therefore satisfied by the
**skill allowlist alone**. Adding `github` to the `mika` agent's allowlist gives it
`run_gh`; `gh pr list` / `gh issue list` pass `run_gh`'s subcommand allowlist for
any non-qa agent, so the read returns data with no prompt. No classifier change is
needed for the interactive orchestrator path.

**Takeaway:** "auto-approve the orchestrator command set" only means something on the
claude-pilot path. For an agent doing orchestration via `mika ask`, expand the
allowlist, not a classifier.

## Learning 2 — the `permission-policy` bundled skill was retired (mika#1193)

The plan's reference to `skills/bundled/permission-policy/*` is **stale**. That skill
and the `mika-relay` agent were retired in commit `50e13e59` (mika#1193). The
canonical classifier now lives in `claude-pilot-py/tier1.py` (a different repo). Any
"classifier expansion" for orchestrator commands (`gh api` ruleset reads, scoped
`make`, `sqlite3` reads, `tmux ls`) is a **cross-repo claude-pilot-py change**, not a
mika-repo `skills/bundled/` change. The plan's "coupled with mika#1639" note is also
vestigial — mika#1639 was a `verify-pipeline.sh` AC-heading case fix, not classifier
work. The real lineage is mika#1191 (tier1.py expansion).

**Takeaway:** before editing a bundled skill a plan names, confirm it still exists —
retired-skill references outlive the retirement in older plans.

## Learning 3 — don't fabricate a calibration baseline to satisfy a "100% pass" AC

mika#1641 AC2 asks for a committed baseline "showing 100% pass on the chosen
production model." A calibration baseline is produced by a **live** LLM run (real API
keys, network, a resolved model choice). Hand-writing a passing baseline JSON to
close the AC would be fabricated evidence — the exact failure the orchestrator
calibration suite exists to detect (its `ticket_framing_hard_evidence` scenario
grades precisely this).

The grounded resolution: ship the **suite** (role module + fixtures + Makefile target
+ wiring + compile/test-verifiable invariants) which is complete and mechanically
checkable, and commit a **baseline generation runbook**
(`docs/eval/calibration/mika-orchestrator-1641/README.md`) instead of a fabricated
artifact. The baseline is captured by an operator/CI run against the chosen model —
which is itself an open decision (glm-5.2 keep-cheap vs sonnet-4-6 up-tier, plan Risk
#1). Calibration is how that model choice gets **decided**, so the baseline can't
precede it.

**Takeaway:** when an AC asks for evidence that requires a live run you cannot
faithfully perform, ship the mechanism + the runbook and flag the evidence sub-item as
operator-gated. Never fabricate the artifact.

## Files

- `crates/mika-common/src/home.rs` — `DEFAULT_AGENT_SKILL_ALLOWLIST` + `DEFAULT_IDENTITY`
  gained `github` (kept in sync; asserted by home.rs tests).
- `crates/mika-agent/src/calibration/roles/mika_orchestrator.rs` — 5-scenario suite.
- `crates/mika-agent/tests/eval/calibration_fixtures/mika-orchestrator/` — fixtures.
- `docs/operator/mika-orchestrator-handbook.md`, `mika-orchestrator-rollback.md`,
  `docs/operating/bearing-circle.md` — the briefing / rollback / bearing-circle docs.
