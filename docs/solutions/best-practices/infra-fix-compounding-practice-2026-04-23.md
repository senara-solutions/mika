---
module: development_workflow
date: 2026-04-23
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Shipping a fix to CI, release automation, deploy scripts, or auth/token plumbing
  - Investigating an infrastructure failure that feels familiar
  - Reviewing a PR that touches workflow files or infrastructure config
  - Starting work on any infra-scoped ticket
tags:
  - infra-fix
  - compounding
  - institutional-memory
  - chronic-drift
  - ci-cd
  - release-automation
related_components:
  - tooling
  - documentation
---

## Context

Infrastructure fixes evaporate faster than product fixes. The urgency is always "unblock the next merge / deploy / auth flow," so fixes ship ad-hoc, context thins, and when the issue recurs — infra failures cluster — the next investigator starts from scratch.

This practice was named after a concrete case: release automation on this repo produced 14+ fixes over ~7 weeks across three tools (semantic-release → release-plz → git-cliff), with zero compound-doc entries until the pattern was identified. The developer's own recall was "2 or 3 times." The gap between 2–3 and 14+ is the evaporation hazard.

The case study is documented separately in [`release-automation-chronic-drift-2026-04-23.md`](../ci-cd/release-automation-chronic-drift-2026-04-23.md). This doc is about the **generalized rule** that emerged from that case.

## Guidance

**Core rule: look back before shipping an infra fix.**

Before shipping any infrastructure fix — CI, release automation, deploy scripts, auth/token plumbing, observability, secrets management, networking — grep git log for prior related fixes first.

```bash
git log --oneline --grep=<area>
# e.g., git log --oneline --grep=release
#       git log --oneline --grep=ci
#       git log --oneline --grep=deploy
```

Count related prior fixes:
- **0–2 hits** → probably a one-off. Fix and move on.
- **3+ hits** → chronic drift. Treat as a class, not an instance.

**When 3+ hits: compound before shipping.** Either the fix addresses the underlying class (root cause, not symptom) — in which case the compound doc explains WHY it addresses the class — or the fix is explicitly logged as "Nth point-fix in this class, adding to the ledger" in an existing or new compound doc. Do not silently ship the Nth point-fix without that explicit framing.

**Rename when naming lies.** If looking back surfaces a naming hazard (e.g., a workflow file named `release-plz.yml` that now runs git-cliff after a migration), fix the name as part of the current work. Future grep-based discovery fails when names lie.

**Commit messages are evidence, not explanation.** `git log --grep` surfaces WHAT was fixed; it rarely surfaces WHY the fix was right or what alternatives were tried. The compound doc fills that gap.

## Why This Matters

The anti-pattern this practice prevents: silently shipping the Nth point-fix in a chronic-drift class, producing 14+ fix commits with zero institutional memory. Each fix is muscle-memory at the time and evaporates within a week.

The compounding effect: the first time a class is identified costs research. Document it, and every subsequent occurrence in that class takes minutes instead of hours. The doc grows iteratively; the memory decays exponentially.

This rule applies far beyond release automation — it generalizes to any infrastructure domain where:
1. Failures manifest in environments you can't reproduce locally (CI runners, production deploy pipelines, cloud auth flows)
2. The fix cycle is slow (one attempt per merge/deploy cycle, 15–60 min between iterations)
3. The psychological path of least resistance is a point-fix rather than understanding the class

## When to Apply

- **CI/CD pipeline fixes** — workflow files, build scripts, release automation
- **Deploy script fixes** — provisioning, Helm charts, Docker builds
- **Auth/token plumbing** — OAuth flows, API key rotation, App installation tokens
- **Observability fixes** — logging, tracing, metrics pipelines
- **Secrets management** — env var propagation, scrubbing, injection patterns
- **Networking/routing** — gateway config, webhook delivery, DNS

## Examples

**Before (anti-pattern):**
```
commit abc1234  fix: pin Rust toolchain in release workflow
commit def5678  fix: add git identity for release tags
commit ghi9012  fix: exclude release/* from pipeline checks
commit jkl3456  fix: disable cargo package verification
...
# 14+ fixes, zero compound docs, each investigator starts from scratch
```

**After (with this practice):**
```
# Developer about to ship fix #4 for release automation
$ git log --oneline --grep=release | wc -l
7

# 7 hits → chronic drift. Before shipping:
# 1. Read/create compound doc for the class
# 2. Classify the fix (Class A/B/C/D in the release automation case)
# 3. Ship the fix WITH the compound doc update
```

## Cross-references

- Case study: [`release-automation-chronic-drift-2026-04-23.md`](../ci-cd/release-automation-chronic-drift-2026-04-23.md) — the triggering case with Class A/B/C/D taxonomy
- MEMORY: `feedback_compound_infra_fixes.md` — the operational rule in session memory (auto memory [claude])
- Ticket: mika#776 — institutionalization ticket
