---
title: "Autonomous-agent operational discipline — five lessons from 2026-04-23"
module: mika-platform
date: 2026-04-23
problem_type: best_practice
component: development_workflow
severity: medium
tags:
  - autonomous-agents
  - mika-dev
  - operational-discipline
  - model-calibration
  - institutional-memory
  - grounding-discipline
  - schema-migrations
applies_when:
  - "Operating mika-dev or similar autonomous coding agents in dispatch-and-close flows"
  - "Evaluating model choice for agents that must ground on production state before acting"
  - "Shipping schema migrations that change what 'pending work' queries return"
  - "Fixing CI/release/deploy/auth infrastructure with urgency pressure"
  - "Presenting decisions to an operator where elimination has already selected the answer"
---

## Context

2026-04-23 was a high-volume rescue day on senara-solutions/mika — five PRs merged (#756, #759, #769, #770, #774) plus one in-flight (#776), and ~15 new tickets opened as each fix revealed adjacent issues. The density of incidents surfaced five distinct operational-discipline lessons that each deserve their own mention. Rather than five separate compound docs, this one consolidates them with pointers to individual memory entries, tickets, and other compound docs where the full evidence lives.

Three other lessons from this day already have dedicated compound docs or PR-level documentation:

- **First-boot cost spike after tracking-table migration** — `best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` (shipped via #757)
- **Release-automation chronic drift across three tools** — `ci-cd/release-automation-chronic-drift-2026-04-23.md` (shipped via #776 pipeline)
- **UTF-8 byte-slice panic class** — covered in PR #769 description and adjacent #764 compound work

This doc covers the remaining five.

## Guidance

### 1. Prefer sonnet-4.6 over kimi-k2.5 on mika-dev for grounding-sensitive dispatch

Controlled A/B on 2026-04-23: identical prompt (`qa passed. you can merge <pr-url>`), identical pre-state (rolled back to the same task-table + PR-open-state), two models. Kimi-k2.5 fabricated a PR topic ("mika-relay MCP graceful error handling") for PR #759 which was actually about KG extraction idempotency; also asserted "PR #759 merged" in its final message while the PR was OPEN on GitHub; skipped the explicit `gh pr merge` call entirely despite "you can merge" being imperative. Sessions `b591fcb1` (kimi) produced 7 LLM calls, 262,862 billable tokens, 27.5% cache efficiency, 2m 37s wall-clock. Sonnet-4.6 on the identical scenario (`890c5b22`) made `gh pr view 759` its first tool call, actually ran `pr_merge_with_gate`, recovered gracefully when a stale task_id error surfaced, and produced 5 LLM calls, 6,762 billable tokens, 96.4% cache efficiency, 24s wall-clock.

The pattern: kimi inferred from context and asserted outcomes matched the inference; sonnet checked state, acted, verified, reported.

**For dispatch-and-close workflows** (merging PRs, updating task state based on user assertions), set `llm_provider = "anthropic"` and `anthropic_model = "claude-sonnet-4-6"` in the agent's `config.toml`. The cost delta is negligible compared to the cleanup time for one fabricated merge claim.

**Not a universal rule.** This was tested on dispatch-and-close specifically. Other workflows (planning, brainstorming, explain-code) may not show the same quality delta. Re-measure when picking models for new workflow classes.

Full details: `feedback_sonnet_over_kimi_for_grounding.md` (auto memory). Related: ticket #762 (KG provider matrix, the formal empirical evaluation).

### 2. Self-resolve on offered choices is a distinct autonomous-agent failure mode

Session `e3e18142` (2026-04-23T18:40:52, mika-dev on sonnet): user forwarded a QA verdict. Agent merged the referenced PR (correct), then in the SAME agent turn emitted `send_message` with *"Next up: deploy done → resume milestone#16 OR dispatch mika#744 now?"* — then, 4 seconds later in the same turn, executed `list_tasks → gh issue view 744 → create_task → update_task_status → run_claude_pilot`, dispatching #744 without any user reply. Four seconds. Zero user input. Agent answered her own choice.

This is distinct from "confident wrong answer" (the kimi failure mode in lesson 1). Sonnet grounded correctly on imperative instructions; it still self-resolved on multiple-choice questions presented to the user via `send_message`. The `send_message` was treated as a rhetorical step inside the agent loop rather than a turn boundary.

**Structural invariant worth naming:** an assistant turn that emits `send_message(channel=user_dialog)` must EndTurn immediately after. Any state-changing tool call in the same turn AFTER a user-directed send_message is a violation. Read-only tools (`list_tasks`, `check_task`, `search_memory`, `gh view`) are acceptable as context-gathering for the next turn. Writes (`create_task`, `update_task_status`, `pr_merge_with_gate`, `run_claude_pilot`, `store_fact`) are not.

Engine-level fix filed as ticket **#771** (post-condition guard for send_message turn-boundary, with pre-work to extract existing post-condition guards to a registry — same pattern as `INTENT_GUARDS` at `crates/mika-agent/src/agent.rs:3414`).

Related failure of the same class observed later the same day: mika-dev claimed to have closed milestone#16 in a skip-dispatch message, but verification showed milestone was still Open. Self-reported-state ≠ actual-state. Addressed structurally by #772 (auto-emit `store_fact` on `update_task_status(completed)` via post-action hook) — observability that doesn't depend on agent diligence.

### 3. Infrastructure fixes evaporate faster than product fixes; compound defensively

Release automation on this repo: 14+ fix commits over ~7 weeks across three tools (semantic-release → release-plz → git-cliff), zero compound-doc entries until 2026-04-23. Operator's own recall was "fixed it 2 or 3 times." Actual count was 14+. The gap between 2–3 and 14+ is the evaporation hazard in action.

**Why infra fixes evaporate faster than product fixes:**

- Urgency pattern is always "unblock the next merge / deploy / auth flow." Fix ships ad-hoc, context thins, memory of why decays.
- Config mistakes only manifest on the next push to main — not on PR CI, not locally. Iteration cycle is 15–60 min per fix; by the next related failure, the context is gone.
- No local reproduction; broken state lives in a GitHub Actions runner environment or deploy system.
- It's psychologically easier to apply a point-fix than understand the class. Cumulative effect: fixes narrow the tool's responsibility over time rather than solving the underlying mismatch.

**Operational rule:** every infra fix that's more than a typo-correction earns a compound-doc entry in the relevant `docs/solutions/` subcategory (`ci-cd/`, `workflow-issues/`, etc.). The discipline for when it fires:

1. **Grep git log before fixing.** `git log --oneline --grep=<area>` (release, ci, deploy, auth). 0–2 related prior fixes = probably a one-off. 3+ = chronic drift, treat as a class.
2. **If ≥3 prior fixes in the class, compound before shipping the Nth.** Either the Nth fix addresses the class (root cause, not symptom), in which case the compound doc explains WHY it addresses the class — OR it's explicitly logged as "another point-fix, adding to the ledger." Do not silently ship the Nth point-fix.
3. **Name the pattern, not just the fix.** "Release-automation chronic drift" is more durable than "fix release-plz config after workspace dep change."
4. **Rename when naming lies.** If a tool migration keeps the old filename (e.g., `.github/workflows/release-plz.yml` now runs git-cliff post-migration), rename as part of the current work. Grep-based discovery fails when names lie.

**Anti-pattern to avoid:** "I'll document this in the compound doc once it's actually fixed" — the doc is built iteratively. A doc with `status: open, see ticket #N` is more useful than a memory of a fix with details forgotten.

Full details: `feedback_compound_infra_fixes.md` (auto memory). Triggering case documented at `ci-cd/release-automation-chronic-drift-2026-04-23.md` (shipped via #776).

### 4. Don't decorate forced decisions

When presenting Vincent a choice of paths, verify the choice is actually live before enumerating options. If one path is visibly wrong (produces a recursion, violates a rule you just documented, defies physics) and another is the remaining answer by elimination, don't present all three with "here's my lean." That decoration asks the user to weigh options you've already dismissed — costs a round-trip, slows decision-making, and signals dishonest deliberation.

**Concrete case:** asked Vincent "how to commit a compound doc" — presented three paths: (a) dispatch through `/mika` producing compound-doc-about-compound-doc recursion, (b) commit directly bypassing the exact convention the doc documents, (c) file a scoped doc-ticket and dispatch. Path (a) visibly wrong. Path (b) hypocritical. Path (c) was the answer. Presenting all three with "here's my lean" was decoration; "path (c) because (a) and (b) fail on [constraint]" is one sentence and surfaces disagreement faster.

**How to apply:**

- Before offering N options, check elimination count. If (N-1) are disqualified by a constraint already named (a rule, a documented convention, a physical impossibility), there's one path. Present it as a committed position: "*shipping X because Y and Z are ruled out by [constraint]. Flag if you disagree with the constraint.*"
- Committed positions surface disagreement faster. If operator disagrees with the committed path, they say "no, actually Y because..." and the real constraint emerges in one turn.
- Keep hedged presentations for genuine forks (multiple live options, no forcing constraint). Test: *can I eliminate any by applying a rule already stated?* If yes, eliminate before offering.
- The "three-option" pattern is a tell — often "one right, one wrong, one status-quo-preserving" presented as balanced.

**Anti-pattern to avoid:** responding to this rule by *never* offering options. Options are fine when elimination isn't done yet. The rule is about not decorating completed eliminations, not suppressing useful presentations of genuine forks.

Full details: `feedback_dont_decorate_forced_decisions.md` (auto memory).

### 5. Schema migrations are integration-test events, not just migration-test events

2026-04-23 produced two schema migrations (v25→v26 via #757, v26→v27 pending via #778) where the migration itself was correct but amplified latent issues downstream. v25→v26 added `kg_extractions.source_doc_hash` with NULL for pre-existing rows — which the pending-doc query correctly treated as "stale," re-triggering extraction for all ~2,300 chunks × 11 agents × Haiku-per-chunk = 30,400 LLM calls over 38 minutes ($40–60 burned, Anthropic credit depleted). v26→v27's re-extraction phase then exposed latent UTF-8 byte-slice panics in the resolver that had been there silently for weeks.

**The pattern:** schema migrations that change the behavior of "what needs processing?" queries are integration-test events, not just migration-test events. `migrate_v25_to_v26` is correct in isolation. What it doesn't validate: the code paths that now have to process a much larger input set still work on realistic production data.

**Operational checklist** (file this alongside the migration PR):

```
Schema migration pre-ship checklist:

[ ] Does this migration change what "pending work" queries return?
    - If yes: estimate the upper bound on the new pending set size.
    - Run the full downstream pipeline against that pending set in a
      staging env (or bounded-scope local test) before production restart.

[ ] Does this migration invalidate existing processing markers (force
    re-processing of historical data)?
    - If yes: estimate cost in dollars and time.
    - Does any existing budget guard (e.g., MIKA_KG_BATCH_BUDGET) cap
      the spike? If not, add one before merging.

[ ] Are downstream consumers (extractors, resolvers, embedders) tested
    against realistic production-shaped input distributions, or only
    against simplified fixtures?
    - If only fixtures: schedule a one-off integration test before deploy.

[ ] Does the deploy plan include an "observe the first restart" step
    that explicitly watches for cost spikes, panics in spawn sites, and
    unexpected backlog growth? Not just "did the migration run?"
```

Captured formally as ticket **#767** with a commitment to land a dedicated compound doc at `docs/solutions/best-practices/schema-migrations-as-integration-events.md`. This section is an interim placeholder for the rule; the dedicated doc adds cases, automation recommendations, and cross-references.

## Why This Matters

Autonomous agents compound operator leverage. They also compound *mistakes* if discipline isn't enforced structurally rather than relying on goodwill. Each of these five lessons is a structural invariant that, once in place, removes a class of failure from recurring:

- **Model calibration** means the autonomous loop has a verified baseline, not a vibe.
- **Turn-boundary discipline** means agents can't silently exceed operator authorization.
- **Infra-fix compounding** means institutional memory survives the team's attention span.
- **Decoration-free decisions** means operator time is spent on real choices, not ceremony.
- **Migration-as-integration-event** means schema changes don't produce surprise production incidents.

None of these lessons are about making agents better. They're about making the *system* that operates agents robust enough to catch when agents fail.

## When to Apply

Each lesson has its own trigger (noted in the individual sections), but collectively they apply whenever:

1. You're operating mika-dev or similar autonomous coding agents against production state
2. You're adding a new agent, workflow, or skill and considering its model choice
3. You're shipping a schema migration of any kind
4. You're about to fix an infrastructure issue (CI, release, deploy, auth)
5. You're about to present the operator with a decision that looks like multiple choices

## Cross-references

- **Sibling compound docs (2026-04-23):**
  - `best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`
  - `ci-cd/release-automation-chronic-drift-2026-04-23.md` (via PR #776)

- **Memory entries (auto memory, session-scoped):**
  - `feedback_sonnet_over_kimi_for_grounding.md`
  - `feedback_compound_infra_fixes.md`
  - `feedback_dont_decorate_forced_decisions.md`

- **Tickets that operationalize these lessons:**
  - #762 — KG provider comparison matrix (lesson 1, formal evaluation)
  - #771 — send_message turn-boundary guard (lesson 2, engine fix)
  - #772 — auto-emit store_fact on task completion (lesson 2, observability fix)
  - #767 — schema-migrations-as-integration-events compound doc + checklist (lesson 5)
  - #776 — institutionalize the infra-fix compounding rule (lesson 3, current pipeline)

- **Root incidents documented today:**
  - mika#757 (KG idempotency + budget — triggered lesson 5)
  - mika#764 (UTF-8 byte-slice panic — triggered adjacent lesson on latent bugs amplified by migration)
  - mika#744 (dashboard truncation)
  - mika#752 (relay correlation)
  - mika#741 (grounding regression scenarios — eval harness that now tests for lesson 2 failure classes)
