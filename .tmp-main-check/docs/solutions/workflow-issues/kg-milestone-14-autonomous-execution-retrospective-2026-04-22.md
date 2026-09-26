---
title: "KG Milestone #14 Retrospective — Socratic Planning + Autonomous Execution Loop"
date: 2026-04-22
category: workflow-issues
module: mika
problem_type: workflow_issue
component: development_workflow
severity: high
applies_when:
  - Planning a multi-ticket milestone with peer-Claude Socratic review
  - Dispatching milestone execution through the mika-dev autonomous loop
  - Building cross-ticket plans that share a schema or contract
  - Deciding between per-ticket vs milestone-end deploy
  - Re-dispatching a ticket after CI failure or callback stall
tags:
  - knowledge-graph
  - milestone-retrospective
  - autonomous-loop
  - socratic-planning
  - mika-dev
  - claude-pilot
  - self-dev
  - dispatch-pipeline
related_components:
  - mika-agent
  - mika-dev
  - claude-pilot
  - self-dev-skill
  - mika-kg
  - mika-cli
---

# KG Milestone #14 Retrospective — Socratic Planning + Autonomous Execution Loop

## Context

Milestone #14 (Knowledge Graph) replaced Mika's prose-based agent self-awareness with a SQLite-backed graph spanning three layers: a domain graph (skills, tools, agents — imported deterministically from manifests), a lexical graph (solution and compound docs chunked and embedded), and a subject graph (problem types and solution paths, LLM-extracted per doc with three-phase reconciliation). The design was informed by the DeepLearning.ai Agentic KG curriculum and positioned as the "inner" layer of Vincent's product vision — core code plus KG — addressing the root cause of mika-dev fabrication bugs: prose state drifts, and LLMs rationalize around stale context rather than query it.

The milestone shipped seven tickets (#686–#692) through seven PRs (#722, #726, #728, #729, #730, #731, #733), all merged on 2026-04-22. Schema v25 landed in the first ticket with ten tables and sixteen D-numbered decisions; six subsequent tickets layered on the domain builder, lexical ingestion with async embeddings, subject extraction, two-stage entity resolution (exact match plus LLM disambiguation), the query tool, and a self-knowledge skill upgrade. Approximately sixty D-decisions accumulated across the seven plan docs.

The work emerged from a Socratic planning pattern that itself became a subject of this retrospective. Vincent and Claude iteratively designed each ticket as a sequence of D-numbered decisions capturing tradeoffs, rationale, and downstream implications. A second Claude session ran peer review on each plan, pushing back on decisions and producing amendments (D11–D15 on plan #686 were folded back from #690's planning session). Vincent framed the pre-launch state as "the most tuned or prepared milestone I ever did," and the outcome — a clean sweep of seven merges with no rolled-back work on the core designs — validated the investment. The pattern itself is documented forward-looking in [`best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md`](../best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md); this doc is the hindsight companion.

## Guidance

**Socratic planning with peer-Claude review.** Architectural milestones benefit from a two-session design loop: one session drafts the plan, a second Claude session reviews and pushes back. The peer review produced real amendments on milestone #14 rather than rubber-stamping, catching schema gaps before they hit implementation.

**D-numbered decisions encode the tradeoff trail.** Each plan doc should enumerate decisions as D1, D2, … with tradeoffs, rationale, and implications captured inline. Approximately sixty D-decisions across the KG milestone gave every downstream ticket a citable rationale and let schema amendments reference the original decision they modified.

**Plan doc lives on the feature branch, not main.** Claude-pilot's worktree is created from `origin/main`, so a plan committed only to main won't load when mika-dev dispatches against a feature branch. Commit the plan to `feat/<issue>/<slug>` and add a `> **Branch:** <code>&lt;name&gt;</code>` callout to the issue body so the dispatcher can resolve the branch. (auto memory [claude] — `feedback_secondary_pr_plan_doc.md`)

**Branch-callout parsing belongs in the handler, not just in discipline.** Documented conventions in issue bodies are not sufficient if the tooling doesn't enforce them. The #686 plan was followed by accident (`/mika` happened to derive a slug that matched the plan branch); on #687 the derived name diverged, mika-dev implemented from the issue body alone, ignoring 60 D-numbered decisions (session history). The fix added explicit parsing of `> **Branch:**` in both `.claude/commands/mika.md` and `skills/bundled/claude-pilot/handlers/run.sh`, cherry-picked to every remaining plan branch.

**Schema amendments fold back upstream.** When a downstream ticket surfaces a constraint that requires schema changes, amend the upstream schema ticket's plan doc rather than forking a parallel schema or filing a v26 migration. D11–D15 on plan #686 originated in plan #690's discovery and were reconciled into the schema doc — the single source of truth stayed coherent, and downstream plans cite the upstream D-numbers they depend on.

**Dispatch merge-all-then-deploy-once.** Compile-time dependencies between tickets are validated by CI; runtime integration only matters at milestone close. With seven tickets executing serially and a shared schema migration, deploying after each PR would have caused migration version conflicts (session history). Merge the seven PRs as they land and deploy once at the end.

**Full `/mika` pipeline always — no lightweight path.** Even a "trivial" docs-sync CI fix's `/ce:review` caught real bugs in `entity_resolver.rs` (duplicated match arms, LIKE wildcard bug, case-insensitivity). A 55-minute pipeline shipping correct code beats a 30-second shortcut shipping silent data corruption. (auto memory [claude] — `feedback_full_pipeline_always.md`, `feedback_never_skip_ce_review.md`)

**Dispatch the issue, never the PR number.** Re-dispatching a CI failure as `/mika mika#722` (the PR number) caused `/mika` to treat it as a fresh issue, create a new worktree, implement the schema from scratch, and merge a wrong version — requiring a force-push revert (session history). Always re-dispatch by issue number, and let the branch callout route the worktree to the existing plan.

**Verify merge state before advancing.** `gh pr merge --auto` returns success for "auto-merge enabled," not "merged." On #727 mika-dev advanced the dispatch queue treating enablement as completion. Confirm via `gh pr view --json state` before moving to the next ticket. (Filed as `mika#727`.)

**Investigate silent callback failures before filing bugs.** On #688 the callback arrived but the session produced zero messages — mika-dev's LLM (kimi-k2.5) responded to a relay permission event with prose narrative instead of JSON, which the relay consumed silently (session history). A bogus `claude-pilot-py#8` was filed assuming `--task-complete` was missing; handler inspection showed the code was correct. Reproduce, inspect the handler, then file. The underlying cause motivates `mika#721` (dedicated mika-relay agent).

**Track LLM provider strictness differences.** The duplicate `task_id` in the JSON schema `required` array (`mika/crates/mika-agent/src/skills/index.rs`, latent since commit 440a9d59 on Mar 11) was tolerated by kimi and rejected by Anthropic and DeepSeek. Provider switches surface latent contract bugs; treat a provider change as a regression risk worth validating.

**Deterministic tiebreakers for topo-sort dispatch.** When `blockedByIssues` returned empty (wrong GraphQL field), the dispatcher silently fell back to issue-number order — putting #688 before its dependencies #689/#690/#691 (session history). The dependency-correct order was #686→#687→#689→#690→#691→#688→#692. Dependency-respecting sorts need an explicit, stable tiebreaker that matches the planner's intent; empty results from the dependency source should fail loud, not fall back silently.

**Retrospective = mechanical + judgment.** Mechanical sections (Planned / Metrics / PRs / Dates) are facts recoverable from git and the issue tracker and should be automated. Judgment sections — Systemic Issues, Learnings, Implications — are human-authored. The retrospective itself is retrospective training data: it gets chunked, embedded, and becomes queryable via the self-knowledge skill.

## Why This Matters

The retrospective is not a paper artifact — it is training data for the system being retrospected. Milestone #14's output is the KG, and the KG's lexical layer ingests `docs/solutions/**/*.md`. The retrospective on the KG buildout gets chunked, embedded, and becomes queryable via the self-knowledge skill, so future milestones retrieve these lessons at dispatch time. This is the institutional memory story working as designed: the system learns how it was built.

The KG itself exists because prose-based state drifts. Mika-dev's fabrication bugs — hallucinating GraphQL fields like `blockedByIssues` (fixed in PR #720), trying `remove_line` on a non-existent `current_priorities` entry, treating auto-merge-enabled as merged — all trace to LLMs rationalizing from stale or under-structured context. Replacing prose with a query-backed graph removes the surface where rationalization can take hold. The Socratic planning pattern, D-numbered decisions, and peer review are the same principle applied one level up: make the design trail structured enough that amendments cite the decisions they modify, so downstream Claude sessions can't rationalize around forgotten tradeoffs.

Vincent's "actually I just care about quality" pre-empts any future argument for a lightweight pipeline. The 55-minute full-pipeline run that caught the `entity_resolver.rs` bugs is the proof point; the decision is cemented and supersedes earlier exploration of a fast path (auto memory [claude] — `feedback_pipeline_scaling.md` is superseded by `feedback_full_pipeline_always.md`).

## When to Apply

- Milestone-scale work with architectural density — multiple tickets that share a schema, protocol, or cross-cutting concern.
- Any dispatch to mika-dev: plan doc on the feature branch plus `> **Branch:**` callout is mandatory, not optional.
- Schema or contract design where downstream tickets are likely to surface constraints — reserve the amendment-fold-back pattern upfront.
- Cross-ticket dependency chains where CI can validate compile-time coupling; defer deploy to milestone close.
- Every code change regardless of perceived triviality — the full `/mika` pipeline is non-negotiable.
- Provider or model switches on agents whose prompts exercise JSON schema contracts — validate latent tolerance bugs before production traffic.
- Re-dispatch after CI failure — always pass the issue number, never the PR number, so the existing worktree and plan are reused.

## Examples

**Branch callout pattern.** Issue bodies carry a dispatch hint the handler parses:

```
> **Branch:** `feat/686/kg-sqlite-schema`
> **Plan:** docs/plans/2026-04-21-003-feat-kg-sqlite-schema-plan.md
```

The handler (`skills/bundled/claude-pilot/handlers/run.sh`) and `.claude/commands/mika.md` both grep for the callout and check out that branch before invoking claude-pilot. Absent the callout, the worktree starts from `origin/main`, the plan is invisible, and mika-dev reimplements from scratch — the exact failure mode that produced PR #723 (wrong reimplementation), forced a force-push revert of main, and required a salvage session to reopen #722 with the `toggle_skill` fix cherry-picked from #723 (session history). The fix landed in `mika/` and is still pending cross-repo propagation to `mika-cloud/` and `mika-skills/`.

**`task_id` dedup fix.** In `mika/crates/mika-agent/src/skills/index.rs`'s `inject_task_id_field`:

```rust
// Before — latent since 440a9d59 (Mar 11). Kimi tolerated; Anthropic/DeepSeek rejected.
required.push(task_id_val);

// After
if !required.contains(&task_id_val) {
    required.push(task_id_val);
}
```

The bug only surfaced on provider switch. The lesson is not the one-line fix; a provider migration is a contract-validation event and should be treated like a dependency upgrade.

**Schema amendment fold-back (D11–D15).** Plan #686 defined schema v25 with sixteen initial D-decisions. While planning #690 (subject extraction), the three-phase reconciliation design surfaced constraints the initial schema didn't satisfy — entity identity stability across extraction runs, tombstoning for retracted subjects, reconciliation provenance. Rather than carrying the amendments in plan #690 or forking a v25.1 schema, decisions D11–D15 were written back into plan #686 as amendments with cross-references to #690's discovery context. When PR #722 implemented schema v25, the plan it implemented was the amended one, and plan #690's decisions cite the upstream D-numbers they depend on.

**GraphQL field hallucination fix.** `blockedByIssues` — used by the pre-dispatch blocked-by guard (#713, merged as PR #715) — does not exist on GitHub's GraphQL `Issue` type. The mutation side (`addBlockedBy`) and the query side (`blockedBy`) use different field names; the mutation worked when setting relationships, so the query field was assumed by analogy and never verified (session history). Every topo-sort since #713 fell back silently to issue-number order. Verified via `gh api graphql` schema introspection and replaced in six call sites of `mika/crates/mika-agent/src/github_graphql.rs`. Merged as PR #720.

## Outstanding Infrastructure

Follow-up tickets filed during the milestone, unresolved at close:

- **`mika#721`** — Dedicated `mika-relay` agent (haiku) for permission decisions. The silent callback failure on #688 was attributed to kimi-k2.5 conflating relay events with conversational turns; a dedicated relay agent with only permission-policy loaded would not have this context-confusion problem.
- **`mika#727`** — Self-dev treats `auto-merge enabled` as merged.
- **`mika#732`** — `current_priorities` core memory prompt fix (structural fix for the drift symptom that required manual sqlite repair during this milestone).
- **Callback → dispatch gap** (no ticket) — every ticket transition required a manual `mika ask --agent mika-dev "continue"` (session history). M6–M8 self-dev prompt steps (deploy / close milestone / write retrospective) are not wired.
- **`/mika` branch-callout fix cross-repo** — landed in `mika/`; still pending for `mika-cloud/` and `mika-skills/`.

## Related

**Milestone artifacts (per-ticket compound docs):**

- [`database-issues/kg-schema-three-layer-sqlite-design.md`](../database-issues/kg-schema-three-layer-sqlite-design.md) — #686 schema v25, 10 tables
- [`best-practices/kg-domain-graph-startup-projection-2026-04-22.md`](../best-practices/kg-domain-graph-startup-projection-2026-04-22.md) — #687 domain builder
- [`best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md`](../best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md) — #689 lexical ingestion
- [`best-practices/kg-subject-extraction-constrained-ner-2026-04-22.md`](../best-practices/kg-subject-extraction-constrained-ner-2026-04-22.md) — #690 subject extraction
- [`best-practices/kg-entity-resolution-two-stage-pipeline.md`](../best-practices/kg-entity-resolution-two-stage-pipeline.md) — #691 entity resolution
- [`../688-kg-query-tool-graph-traversal.md`](../688-kg-query-tool-graph-traversal.md) — #688 query tool
- [`../692-self-knowledge-kg-upgrade.md`](../692-self-knowledge-kg-upgrade.md) — #692 self-knowledge upgrade

**Planning pattern and adjacent workflow docs:**

- [`best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md`](../best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md) — Forward-looking pattern guide authored mid-milestone; this retrospective is its hindsight companion.
- [`architecture-patterns/milestone-resolver-topo-sort-deploy-hooks-2026-04-21.md`](../architecture-patterns/milestone-resolver-topo-sort-deploy-hooks-2026-04-21.md) — Deploy-once-at-milestone-end mechanism.
- [`architecture-patterns/blocked-by-dispatch-guard-graphql-validation-2026-04-21.md`](../architecture-patterns/blocked-by-dispatch-guard-graphql-validation-2026-04-21.md) — `blockedBy` GraphQL field fix (PR #720).

**GitHub issues:**

- `mika#686`–`#692` — milestone tickets (all closed)
- `mika#720` — GraphQL `blockedBy` field fix (merged)
- `mika#721` — dedicated mika-relay agent (open)
- `mika#727` — self-dev treats auto-merge enabled as merged (open)
- `mika#732` — current_priorities prompt fix (open)
- `mika-platform#48` — audit commands update for KG awareness
