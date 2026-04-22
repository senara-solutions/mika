---
title: "Socratic multi-ticket milestone planning in Claude Code"
module: development_workflow
date: 2026-04-21
last_updated: 2026-04-22
problem_type: best_practice
component: development_workflow
severity: low
applies_when: >
  Planning a sequence of 3+ related tickets in a single GitHub milestone
  where ticket bodies settle scope but not architecture, a human peer
  (Vincent or equivalent experienced reviewer) is available for in-session
  questions, and tickets have not yet been dispatched to the autonomous
  implementer. Especially fits milestones touching a shared substrate
  (schema, registry, core trait, prompt set) where decisions in later
  tickets may need to fold back into earlier still-open plans.
tags:
  - planning
  - workflow
  - ce-plan
  - milestone
  - feature-branch
  - conventions-doc
  - socratic-method
  - knowledge-graph
  - ask-user-question
related_components:
  - documentation
  - development_workflow
---

## Context

Mika's GitHub milestones routinely bundle 5–10 implementation tickets that share schema, conventions, and cross-cutting design decisions. Naively dispatching each ticket to `mika-dev` one at a time produces churn: ticket N surfaces a schema question that belongs in ticket N−2's still-open plan, conventions get restated in every plan document, and architectural decisions are buried inside implementation PRs where reviewers can't see them as a set.

The gap this workflow closes sits between "file an issue with acceptance criteria" and "dispatch to `mika-dev`." Ticket bodies are written under time pressure and typically settle scope, not design — 2–4 material design questions per ticket are usually unresolved when dispatch would otherwise happen. Those questions need to be pulled out of the implementer's path, answered by an experienced architect (Vincent), folded into committed plan documents on feature branches the implementer will later reuse, and allowed to ripple backward into earlier tickets' plans when they expose schema amendments.

Milestone mika#14 (Knowledge Graph, 7 tickets) made the pattern explicit. Planning the first three tickets — mika#686 (SQLite schema), mika#687 (domain graph builder), mika#689 (lexical ingestion) — in a single Claude Code session on 2026-04-20 → 2026-04-21 produced 23 D-numbered decisions across the three plans, two schema amendments back into #686 driven by #689 planning, and a shared conventions doc that all downstream plans cite by section rather than restate. The pattern is reusable for any future milestone of similar shape.

## Guidance

### 1. Research the ticket before asking anything

Pull the issue body verbatim, then grep the codebase for adjacent patterns. Asking architectural questions without knowing what already exists wastes the reviewer's time.

```bash
gh issue view 689 --repo senara-solutions/mika --json body --jq .body
```

Grep for registries, schema files, existing helpers, and naming conventions. Example from this session:

```bash
grep -rn "SkillRegistry\|ToolRegistry" crates/mika-agent/src/skills/mod.rs
grep -n "search_content\|index_content" crates/mika-agent/src/db.rs
```

Read any cross-cutting conventions doc from earlier in the milestone first — it already contains decisions you should not relitigate.

### 2. Identify 2–4 material unresolved questions

Filter out anything the ticket body prescribes. Focus on the open design space: schema shape, edge semantics, where state lives vs. where structure lives, retry taxonomy, layer boundaries. If a "question" has only one sensible answer given stated constraints, don't ask it — answer it in the plan and cite the constraint.

### 3. Ask via `AskUserQuestion` with structured options

Three options per question. "(Recommended)" marker on the first option only when you have a real recommendation. Trade-offs named explicitly in each option's description, not hidden in follow-ups. Batch 2–3 tightly-scoped questions per call when they share context; split into separate calls when the answer to Q1 changes Q2's option set.

### 4. Expect architectural answers, not yes/no

A ticket-sized set of questions typically returns 500–2000 words per question from an engaged reviewer, with pushbacks on option framings, new principles surfaced, and occasionally cross-cutting questions that affect earlier tickets. Budget time for absorbing answers, not just transcribing them. The answer's quality usually exceeds the question's framing — expect to be corrected on the shape of the question itself.

### 5. Amend earlier tickets when answers surface schema gaps

If an answer exposes a schema or cross-cutting question that belongs in an earlier (still-pre-implementation) plan, **amend that plan on its still-open feature branch**. Do not defer to a "v+1 migration" ticket. The earlier ticket hasn't shipped yet; the plan can absorb the change now for free. Fold the amendment in as a new D-numbered decision so traceability is preserved.

### 6. Write the plan at Standard depth

Use D-numbered decisions (D1, D2, ..., Dn) so future reviewers can cite them by number. Cite the shared conventions doc by section (e.g., "per C2.2 retry taxonomy") rather than restating content. Break implementation into 5–8 units, each with: goal, requirements, dependencies, files, approach, test scenarios, verification.

### 7. Commit on the feature branch, **never main**

Branch name must match the autonomous implementer's convention so `mika-dev`'s pipeline reuses the worktree and finds the plan on it:

```
feat|fix|chore/<issue-number>/<short-kebab-slug>
```

Commit message includes the `Pipeline-Exempt: docs-only` trailer — this trailer is a pre-existing escape hatch in `scripts/verify-pipeline.sh` that lets docs-only commits bypass the plan-doc-missing block that `mika-qa` enforces on non-docs PRs. Committing a plan on main breaks the implementer's branch-reuse assumption and bypasses the PR review the plan itself deserves.

### 8. Push and update the issue body with a dispatch callout

Prepend a block-quote callout to the issue body naming branch, plan path, conventions doc, and the `branch:<name>` dispatch prefix for `/mika`. This makes the dispatch command copy-pasteable and ensures anyone reading the issue sees the plan is ready.

```bash
gh issue edit 689 --repo senara-solutions/mika --body-file /tmp/issue-689-body.md
```

### 9. Land the shared conventions doc once, cite it forever

Multi-ticket milestones produce cross-cutting decisions that would otherwise repeat in every plan. The first downstream plan should land a companion `docs/architecture/<milestone>-implementation-conventions.md` committed on its own feature branch. Downstream plans cite sections by number (C1.3, C2.2) instead of restating.

This is a **net-new architectural hygiene pattern** — prior sessions had no precedent for a cross-ticket conventions doc under `docs/architecture/`. (session history)

### 10. Review cycle before moving to the next ticket

Wait for the reviewer to respond to the committed plan. Expect 2–3 targeted tightenings per plan. Edit and re-commit before starting the next ticket — never batch review across tickets, because later answers depend on the earlier plan's final shape.

### 11. Distinguish durable from ephemeral artifacts

- **Durable** (commit on a branch): plans, conventions docs, updated issue bodies, compound docs.
- **Ephemeral** (inline prose in-session, no commit): session-transition handoffs, scratch notes, option-comparison tables that informed a decision already recorded in the plan.

If you catch yourself about to `git checkout -b` for a handoff artifact, stop and return inline prose instead. (session history — memory file `feedback_secondary_pr_plan_doc.md` was rewritten mid-session to make this distinction explicit.)

## Why This Matters

**Schema mistakes in ticket N are 10× cheaper to fix in ticket N−2's plan than in a v+1 migration.** Migrations require data backfill, two deploys, and usually a feature flag. Plan amendments on an unmerged branch require an edit and a re-commit. The Socratic front-loading turns expensive migrations into free edits.

**Conventions docs prevent decision drift across tickets.** Without one, each plan re-litigates retry taxonomy, error types, and naming conventions — and drifts. Ticket A's plan says "retry on 429" while ticket B's says "retry on 429 or 503" because the authors read different stale examples. Landing the convention once and citing it forces alignment.

**Feature-branch plans unlock worktree reuse by the autonomous implementer.** `mika-dev`'s pipeline creates a worktree at `.claude/worktrees/<branch>/mika/` and expects the plan to already be on that branch. A plan on main means the implementer either rebases (slow, error-prone) or works without the plan (worse output). (auto memory [claude] — `feedback_secondary_pr_plan_doc.md`.)

**Block-quote dispatch callouts in issue bodies reduce dispatch friction.** The callout makes `mika ask --agent mika-dev "implement mika issue#686 branch:feat/686/kg-sqlite-schema"` copy-pasteable. Without the callout, dispatch requires remembering the branch name and convention.

**Planning without Socratic iteration produces thin plans.** In a prior single-turn `/ce:plan` use for mika#628 (session `4cad7617`, 2026-04-17), the skill ran in a short-circuit path — "this is a lightweight config change, no research needed" — and jumped straight to `/ce:work`. That path is correct for trivial tickets but would have produced a surface-level plan if applied to KG-scale architectural work. (session history)

## Post-Execution Notes (2026-04-22)

The pattern was carried through all 7 tickets of milestone #14 and produced roughly 60 D-numbered decisions across the milestone's plan docs — not 23 as captured mid-milestone. Schema amendments D9–D10 (folded back from #689) were joined by D11–D15 (folded back from #690), confirming the fold-back mechanic scales past two tickets. Execution produced a clean sweep: seven PRs merged, no rolled-back designs. Details in [`../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md).

**What the planner-side discipline did not prevent.** This doc covers the planner-side contract — plan on a branch, callout in the issue body, amendments fold upstream. The retrospective surfaced three classes of failure that sit on the *dispatcher* side and were invisible from here:

- **Branch callout was not parsed by `/mika` or the claude-pilot handler.** #686 followed its plan by coincidence (derived slug matched the plan branch); #687 diverged and `mika-dev` reimplemented from the issue body alone, ignoring the D-decisions on that branch. Fix in `.claude/commands/mika.md` + `skills/bundled/claude-pilot/handlers/run.sh` landed mid-milestone and was cherry-picked to the remaining plan branches.
- **Re-dispatch by PR number, not issue number**, produced a wrong reimplementation (PR #723) and forced a force-push revert of main. Always re-dispatch the issue.
- **`blockedByIssues` GraphQL field does not exist** — the blocked-by dispatch guard (#713/#715) silently fell back to issue-number order, putting #688 before its dependencies. Fixed as `blockedBy` in PR #720.

The planner-side pattern remains correct, but a milestone that passes every planner-side check can still dispatch wrong if the handler, dispatcher guards, and merge verification are buggy. Open follow-ups: `mika#721` (dedicated relay agent), `mika#727` (auto-merge vs merged), `mika#732` (`current_priorities` drift), and the unticketed callback→dispatch gap.

## When to Apply

Apply this workflow when **all** of the following hold:

- Milestone has 3+ tickets with shared schema, conventions, or cross-cutting design decisions.
- Ticket bodies settle scope but not architecture.
- An experienced architect is available for Socratic answers in-session.
- Tickets have not yet been dispatched to the autonomous implementer.

Apply partially (research + D-numbered plan + feature-branch commit, skipping the conventions doc if it doesn't earn its keep) when:

- A single ticket has 2+ material unresolved questions.
- The ticket touches a shared substrate other tickets will build on, even if those other tickets aren't yet written.

**Skip** this workflow when:

- The ticket is a pure bug fix with a known root cause.
- The ticket is a config tweak or one-line change — skip this workflow, but note the `/mika` pipeline itself still runs in full. This doc only covers when to skip the **Socratic planning step**, not the pipeline. See `feedback_full_pipeline_always.md` (supersedes `feedback_pipeline_scaling.md`).
- The ticket's acceptance criteria fully prescribe the design.
- No architect is available — dispatch with notes on open questions and let `mika-dev` escalate via `canUseTool`.

## Examples

### Case study: Milestone #14 (Knowledge Graph), 2026-04-20 → 2026-04-21

**Upstream:** Milestone mika#14 and its 7 tickets (#686–#692) were created in a prior session on 2026-04-20 from a digest of three DeepLearning.ai courses (agentic KG construction, KG-RAG, KG APIs). Memory file `project_knowledge_graph.md` captures the milestone-level intent; the Socratic planning session extended it from "what should we build" to "how exactly will we build it." (session history — session `a5ef66c9`)

**Session scope:** planned 3 of 7 tickets interactively.

| Ticket | Topic | D-decisions | Plan branch | Plan path |
|--------|-------|-------------|-------------|-----------|
| mika issue#686 | SQLite schema | 10 (D9, D10 added retroactively from #689) | `feat/686/kg-sqlite-schema` | `docs/plans/2026-04-21-003-feat-kg-sqlite-schema-plan.md` |
| mika issue#687 | Domain graph builder | 5 | `feat/687/domain-graph-builder` | `docs/plans/2026-04-21-004-feat-domain-graph-builder-plan.md` |
| mika issue#689 | Lexical ingestion | 8 | `feat/689/lexical-ingestion` | `docs/plans/2026-04-21-005-feat-lexical-ingestion-plan.md` |

**Shared conventions doc:** `docs/architecture/kg-implementation-conventions.md`, committed on `feat/687/domain-graph-builder`, cited by #687 and #689 plans by section number (C1.1 async-embedding contract, C2 non-interactive LLM call taxonomy, C3 observability granularity).

### Cross-ticket amendment example (the high-value mechanic)

While answering #689's lexical-ingestion questions, Vincent's reasoning surfaced two schema gaps in #686:

1. **`kg_chunks.entity_id` had no writer.** #689 wouldn't infer domain entities at the lexical layer (that's subject-graph territory for #690/#691), and chunk→domain linkage goes canonically through `kg_subject_entities` + `kg_subject_resolutions`. A column with no writer would be silently NULL forever. **Added as D9 to #686's plan**: drop the column.
2. **`kg_chunks` needed `source_doc_hash TEXT NOT NULL`.** #689's ingestion idempotency depends on detecting content changes via a stored hash of normalized doc content. **Added as D10 to #686's plan**: add the column.

Neither amendment was a new ticket. Both folded into `feat/686/kg-sqlite-schema` as a single commit (`6d038256`) before #686 was dispatched. Cost: one commit. Cost had #686 been merged: a v25→v26 migration, two schema migrations in the same milestone, coordinated re-review of #687's plan against the updated schema.

### Meta-principles that emerged

- **"Domain graph contains structure, not state."** If an edge's truth can differ across agents or change without a manifest change, it's state (belongs in `skill_overrides` or equivalent), not structure. Rejected `Agent -[HAS_SKILL]-> Skill` edges on these grounds in #687.
- **"Layers compose through the resolution pipeline, not direct cross-layer columns."** Rejected a direct `kg_chunks.entity_id` FK as a drift-prone shortcut. Multi-hop JOINs through subject entities + resolutions are the canonical query shape.
- **"Schema questions surface through implementation tickets."** Fold them into the earlier plan on its feature branch; never defer to v+1 migrations.
- **"Accept imperfection now, instrument for future data, optimize with real data."** Per-agent duplication of shared docs in #689 was accepted with observability hooks (ingestion duration + chunk counts) to drive future optimization decisions, rather than building a shared-chunks layer speculatively.

### Counter-example 1: the handoff branch that shouldn't have existed

Mid-session, I proposed creating a handoff doc on a new branch (`chore/kg-planning-handoff`) to transfer context to a future session. Vincent pushed back: session-transition artifacts are ephemeral; they don't need branches or commits; inline prose in the next session's prompt is sufficient. Deleted the branch, produced the handoff as inline text.

**Lesson encoded:** durable (plans, conventions, issue bodies) versus ephemeral (handoffs, scratch). Only the former gets branches.

### Counter-example 2: the "commit plan to main" attempt

First attempt at landing #686's plan proposed committing to `main` directly. Vincent caught it: "we've done this many times but you still don't seem to remember." A memory entry existed (`feedback_secondary_pr_plan_doc.md`) but was framed for cross-repo secondary PRs, not single-repo planning. The memory was rewritten mid-session with clearer framing and a counter-example from the KG work to ensure the lesson carries across sessions. (auto memory [claude])

**Lesson encoded:** plans live on feature branches matching the autonomous-implementer convention. Memory entries must be framed generally enough to trigger in adjacent contexts, not just the original one.

### Command cookbook

```bash
# 1. Research
gh issue view 689 --repo senara-solutions/mika --json body --jq .body
grep -rn "SkillRegistry" crates/mika-agent/src/skills/

# 2. Create feature branch (in the target repo, not meta-repo)
git -C mika/ switch -c feat/689/lexical-ingestion

# 3. Write plan, commit (docs-only)
git -C mika/ add docs/plans/2026-04-21-005-feat-lexical-ingestion-plan.md
git -C mika/ commit -m "$(cat <<'EOF'
docs: plan for lexical graph ingestion (#689)

8 D-numbered decisions, cites C1.1, C3.2 from KG conventions.

Pipeline-Exempt: docs-only
EOF
)"

# 4. Push
git -C mika/ push -u origin feat/689/lexical-ingestion

# 5. Update issue body with dispatch callout
gh issue edit 689 --repo senara-solutions/mika --body-file /tmp/issue-689-body.md

# 6. Dispatch when the reviewer signs off on the plan
mika ask --agent mika-dev "implement mika issue#689 branch:feat/689/lexical-ingestion"
```

Typed task references used throughout (per `feedback_task_reference_format.md`): `mika issue#686`, `mika milestone#14`. Never bare `#686` or `mika#686`.

## Related Documentation

- [`../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`](../workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md) — hindsight companion to this forward-looking doc; the 7-ticket execution, what broke on the dispatcher side, and the tickets filed as follow-ups.
- [`docs/architecture/kg-implementation-conventions.md`](../../architecture/kg-implementation-conventions.md) — the concrete conventions doc produced by this session's workflow; the shape to emulate for future milestones.
- `../../plans/2026-04-21-003-feat-kg-sqlite-schema-plan.md`, `../../plans/2026-04-21-004-feat-domain-graph-builder-plan.md`, `../../plans/2026-04-21-005-feat-lexical-ingestion-plan.md` — the three plans this workflow produced; cite them as shape references when planning future KG tickets #690/#691/#688/#692.
- `docs/solutions/architecture-patterns/2026-04-17-internalize-ce-pipeline-into-mika-skills.md` (meta-repo) — future direction for internalizing `/ce:plan` into a Mika-native skill; the Socratic pattern documented here is what such a skill should preserve.
- `docs/solutions/cross-repo-patterns/cross-repo-feature-coordination-from-meta-repo.md` (meta-repo) — sibling workflow for cross-repo phase coordination; different axis (many repos, one feature) from this doc (one repo, many tickets).
- `docs/solutions/integration-issues/meta-repo-branch-ticket-mismatch.md` (meta-repo) — the load-bearing rule that branches and tickets belong on target repos; `mika issue#686`'s branch `feat/686/kg-sqlite-schema` lives on `senara-solutions/mika`, not on the meta-repo.
- Memory files: `feedback_secondary_pr_plan_doc.md`, `feedback_prompt_enforcement_fragile.md`, `project_heartbeat_milestone_phantom.md`, `feedback_task_reference_format.md`.
