---
title: "feat(skills/mika-arch): add milestone-grooming sibling skill + operator companion"
type: feat
status: active
date: 2026-04-29
ticket: senara-solutions/mika#879
branch: feat/879/skills-mika-arch-add-milestone-grooming
---

# Add milestone-grooming capability to mika-arch via sibling skill `mika-arch-groom-milestone`

## Process Note (load-bearing)

This plan modifies mika-arch's own bundled skill surface. Per `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md`, mika-arch cannot review changes to her own skills via the standard two-pass dogfood — the very behavior under test is the reviewer. The second-pass review must route to an external reviewer (Vincent or Claude Chat).

This is **carve-out instance #3** for 2026 (verified 2026-04-29 via grep across `docs/plans/` and `docs/solutions/`):
- Instance #1: mika#788
- Instance #2: mika#872 (the promotion-protocol spec plan, 2026-04-28-003)
- **Instance #3: this PR**

Per the carve-out doc § "When to revisit", instance #3 promotes codification-prep **to ship**. Codification-prep is in-scope for this PR (Unit 8) — assemble three-instance evidence and draft codification language for `docs/architecture/review-guide.md`. The follow-up ticket pattern from earlier instances no longer applies.

Pre-commit verifications applied per second-pass external review (carve-out external reviewer, 2026-04-29):
- D3 schema-vs-engine compatibility verified by reading `crates/mika-agent/src/tools/resolve_issue_order.rs:127-200` — engine consumes `{repo, issues: [int]}` JSON and reads `blockedBy` from GitHub directly; our schema does NOT directly feed the engine. D3 reframed accordingly.
- Carve-out instance count verified by `grep -rl recursive-self-review-carve-out docs/plans/ docs/solutions/` — three instances confirmed.
- `Scope:` header parser-bypass verified by reading all six callsites (`mika-groom-ticket.md:67-69, 88-89`, `mika-ask-arch.md:31-33`, `dev-groom/system_prompt.md:41-43, 56-57`, `MIKA_ARCH_SOUL:600-602`) — every callsite is LLM-based pattern matching, NOT regex. Additive `Scope:` line is unambiguous as long as literal `Disposition: <KEYWORD>` final-line discipline is preserved.

## Overview

mika-arch's two existing skills (`mika-arch-groom-ticket`, `mika-arch-second-review`) handle a single-issue input contract only. This plan adds a sibling skill `mika-arch-groom-milestone` that accepts a milestone reference (`<repo> milestone#N`), enumerates the milestone's sub-issues, delegates to the per-ticket flow for each, and produces a milestone-level **sequencing record** capturing sub-issue dependencies + ordering. An operator-side companion command `/mika-groom-milestone.md` orchestrates the flow.

Shape B (sibling skill) was chosen over Shape A (extending `mika-arch-groom-ticket`) on three converging compound-doc citations summarized in § Key Technical Decisions.

## Problem Frame

PR #872 (merged 2026-04-28) made mika-arch's grooming quality dependent on KG-corpus reachability for policy docs. Audit on 2026-04-29 surfaced multiple KG defects (filed as mika#874–877) collected under milestone #19. When dispatched to groom milestone#19 wholesale via `/mika-ask-arch`, mika-arch correctly returned `Disposition: ESCALATE — milestone-orchestration shape mismatch` because milestone shape is not a contract her current skill recognizes (verified 2026-04-29, session_id `93990770-1264-4831-bc57-e82b23fd27c6`).

Without milestone-grooming capability, the operator must run `/mika-groom-ticket` four times by hand for the four KG defects, manually carrying the sequencing constraint (#874+#875 → #876 → #877) in their head. This breaks two invariants the platform already enforces for single tickets:

1. **Plan-on-branch is the contract** (`docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md`) — sequencing constraints are conversation-volatile unless committed.
2. **Architect produces sequencing artifacts** — per the milestone-resolver pattern (`docs/solutions/architecture-patterns/milestone-resolver-topo-sort-deploy-hooks-2026-04-21.md`), the engine already has Kahn's-algorithm support; what's missing is the architect-authored DAG input.

This is a one-time prerequisite for milestone#19, but compounds: every future milestone (KG-driven or not) needs milestone-aware grooming.

## Requirements Trace

- **R1.** mika-arch accepts `<repo> milestone#N` input shape and produces (a) one plan-on-branch per sub-issue and (b) one milestone-level sequencing record.
- **R2.** Sequencing record names dependencies and ordering for sub-issues in machine-readable form (parseable by `mika-dev` or future automation).
- **R3.** No regression in per-ticket grooming — `mika-arch-groom-ticket` and `/mika-groom-ticket` behavior unchanged.
- **R4.** No regression in disposition/verdict parsers — closed token alphabet (READY/ITERATE/ESCALATE/GROOMED) preserved across all callsites.
- **R5.** Milestone groom is idempotent — re-dispatching the same milestone reuses existing plan-on-branch artifacts when present.
- **R6.** Operator-side companion command exists: `/mika-groom-milestone <repo> milestone#N`.
- **R7.** New skill is added to mika-arch's identity allowlist at all four sites in `crates/mika-agent/src/well_known_agents.rs` per `docs/solutions/best-practices/rename-bundled-skill-touchpoints-and-gates-2026-04-28.md`.

## Scope Boundaries

**Out of scope:**
- Changing the per-block 500-token core_memory cap (the cap is the discipline mechanism PR #872 enforces; raising it defeats #872 per `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`).
- Engine-side hard enforcement of milestone topology (Kahn's-algorithm machinery already exists per `architecture-patterns/milestone-resolver-topo-sort-deploy-hooks-2026-04-21.md`; this plan emits the input the engine consumes, it does not reimplement ordering).
- Auto-dispatch from milestone groom to mika-dev — the existing `/mika` dispatch path is unchanged; the operator chooses when to dispatch.
- KG corpus reachability fixes (those are #874/#875/#876/#877, the four sub-issues this skill enables grooming for, not blockers for #879 itself).
- Refactor of `mika-arch-second-review` beyond minimal addition of milestone-aware verdict scope.

### Deferred to Separate Tasks

- **dev-side milestone sibling** (`dev-groom-milestone` for autonomous milestone runs via dev-pilot). Out of scope: dev-groom keeps its current per-ticket contract (R3 protects this). The first concrete autonomous-milestone use case is the trigger to file this; until then, YAGNI applies.

## Context & Research

### Relevant Code and Patterns

- `skills/bundled/mika-arch-groom-ticket/{system_prompt.md,skill.toml,tools.json}` — first-pass per-ticket review; mirror its three-file structure for the new sibling.
- `skills/bundled/mika-arch-second-review/system_prompt.md` — second-pass reviewer with output-format compatibility discipline at §4.
- `skills/bundled/dev-groom/system_prompt.md` — closest precedent for an orchestrator-style sibling skill (added 2026-04-27 via `docs/plans/2026-04-27-011-feat-add-dev-groom-bundled-skill-plan.md`).
- `crates/mika-agent/src/well_known_agents.rs` — mika-arch identity allowlist at lines 87-88, 114-115, 152-153, 311 (4 sites — `rename-bundled-skill-touchpoints` checklist) and `MIKA_ARCH_SOUL` constant at lines 579-616. Per-skill LLM overrides at lines 193-198.
- `.claude/commands/mika-groom-ticket.md` (in mika-platform meta-repo) — operator-side dispatcher for per-ticket groom; mirror its phase structure for the new milestone command.
- `.claude/commands/mika-ask-arch.md` — JSON-envelope wrapper, hard contract on `.metadata.session_id`. New milestone command consumes this unchanged.
- `scripts/derive-branch-name`, `scripts/derive-worktree-path` — canonical slug + path derivation. Per `docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md`, all callers MUST invoke these scripts; never re-derive.

### Institutional Learnings

- `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md` — N=9 recurrence pattern; adding "detect input shape and branch" prompt rules is the cheap-but-wrong default. Decisive against Shape A.
- `docs/solutions/best-practices/operator-only-bundled-skill-structural-enforcement-2026-04-28.md` — sibling-skill template already in production for dev-groom. Direct precedent for Shape B.
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — N=2 evidence the architect ghosts its own catalogue under load when prompt rules require shape detection. Compounds the case against Shape A.
- `docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md` §4 — output-format compatibility check is mandatory; second-pass MUST grep parser callsites before approving any output-shape change.
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — Kimi paraphrased "Proceed" instead of literal "Disposition: READY". Literal final-line discipline mandated; this plan preserves it.
- `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` — when modifying mika-arch's own skills, second-pass routes to external reviewer. Triggers carve-out instance #2.
- `docs/solutions/best-practices/dispatcher-cross-file-invariant-2026-04-28.md` — branch slug IMMUTABLE once worktree created; semantic mismatch goes in plan filename + frontmatter `type:`, never `git branch -m`. Carries through unchanged for milestone scope.
- `docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md` — all callers use `scripts/derive-branch-name` + `derive-worktree-path`. New milestone command must call these.
- `docs/solutions/best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md` — D-numbered decisions, shared conventions doc cited not restated, plan-amendment-fold-back. Direct shape-of-output for what milestone grooming should yield.
- `docs/solutions/architecture-patterns/milestone-resolver-topo-sort-deploy-hooks-2026-04-21.md` — engine-side `resolve_issue_order` already implements Kahn's algorithm. Sequencing-record schema must be parseable by this tool.
- `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md` — schema amendments fold back upstream into earlier still-open plans rather than forking; merge-all-then-deploy-once at milestone close. Live precedent.
- `mika/docs/solutions/602-milestone-project-workflow-implementation.md` — counter-precedent (self-dev folded milestone shape into existing skill) but self-dev was already an orchestrator. Weighed and rejected as a Shape A signal.
- `docs/solutions/best-practices/rename-bundled-skill-touchpoints-and-gates-2026-04-28.md` — propagation checklist for adding a new bundled skill (4 allowlist sites, identity.toml template, tests).

### External References

None — internal skill spec work. External research skipped at Phase 1.2 (codebase has solid local patterns).

## Key Technical Decisions

### D1: Shape B (sibling skill) over Shape A (extend existing)

**Decision:** Add a new sibling skill `mika-arch-groom-milestone` rather than extending `mika-arch-groom-ticket`.

**Rationale (re-stated per second-pass external review in output-contract terms, not orchestrator-vs-reviewer terms):**

1. **One skill = one output contract.** `mika-arch-groom-ticket` emits a per-ticket plan-on-branch + `Disposition: <KEYWORD>` trailer. Milestone grooming emits per-sub-issue plans + a milestone-level sequencing record + a milestone-level disposition. These are **two distinct output contracts**, not two input shapes feeding one output. Per `plan-on-branch-load-bearing-contract-2026-04-26.md` §4, prompts under load decay at output-shape boundaries (not input-shape boundaries) — one skill emitting two output contracts is the failure mode. This is the load-bearing argument.
2. **Prompt-rule cheapness lens** (`prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`, N=9 recurrence): the cheap default for accommodating two output contracts in one skill is "branch on input shape and switch output shape" — exactly the pattern this doc warns against.
3. **Required-tools-gate evasion evidence** (`required-tools-gate-evasion-patterns-2026-04-28.md`, N=2): mika-arch under load reads its catalogue while violating it. The boundary between two output contracts is where this drift concentrates, not the input-detection rule itself.
4. **Operator-only sibling-skill template already in production** (`operator-only-bundled-skill-structural-enforcement-2026-04-28.md`): `dev-groom` is the production precedent. Shape B mirrors that pattern.
5. **mika#602 counter-precedent does not transfer.** self-dev folded milestone patterns in *because* it was already producing milestone-shaped output (orchestration plans). It generalized its existing single output contract; it did NOT add a second contract. So mika#602 is a "skill whose existing output already covered the new shape" precedent, not a "skill that grew a second output contract" precedent. Shape A would do the latter; that's what makes it the wrong analogy.

### D2: Disposition/Verdict vocabulary unchanged; introduce additive `Scope:` header

**Decision:** Keep the closed alphabet (`Disposition: READY|ITERATE|ESCALATE`, `Verdict: GROOMED|ESCALATE`). Add a single new header line `Scope: milestone` (or `Scope: ticket`, defaulted) before the disposition trailer to disambiguate context.

**Rationale:** New tokens like `MILESTONE-READY` would break six parser callsites (`mika-groom-ticket.md:67-69, 88-89`, `mika-ask-arch.md:31, 33`, `dev-groom/system_prompt.md:41-43, 56-57`, two MIKA_ARCH_SOUL list entries). Additive `Scope:` is consumed by new operator command only; existing parsers ignore unknown lines. Preserves R4. Per `mika-arch-first-dogfood-2026-04-25.md`, literal-final-line discipline is preserved (Disposition is still the literal final line).

### D3: Sequencing record location, schema, and engine-tool relationship

**Decision:** Each milestone groom produces one record at `<repo>/docs/plans/<YYYY-MM-DD>-<NNN>-milestone-<N>-sequencing.md` on a milestone-coordination branch (`feat/milestone-<N>/coordination`). Required sections:

```
---
title: "Milestone <N> sequencing record"
type: milestone-sequencing
milestone: senara-solutions/<repo>#<N>
date: YYYY-MM-DD
status: active
---

## Sub-issues
- #<n>: <title> (priority: <p0/p1/p2/p3>, plan: docs/plans/<file>, branch: <slug>)

## Dependencies
- #<a> + #<b> → #<c>: <one-line reason>

## Recommended GitHub `blockedBy` edits
- #<c> blockedBy #<a>: <reason — file via gh issue edit or GraphQL>
- #<c> blockedBy #<b>: <reason>

## Order
1. <unit or parallel-set>
2. <unit or parallel-set>

## Cross-cutting concerns
- <concern>: <which sub-issues touch it, mitigation>

## Open milestone-level questions
- <question>: <resolution path or escalation>
```

**Rationale (corrected per second-pass external review):** The engine-side `resolve_issue_order` tool (`crates/mika-agent/src/tools/resolve_issue_order.rs:127-200`) consumes `{repo, issues: [int]}` JSON via the LLM tool-call interface and reads `blockedBy` relationships from **GitHub itself** via GraphQL — it does NOT consume our markdown sequencing record directly. The earlier framing ("schema feeds resolve_issue_order") was incorrect.

The correct relationship:
- **Sequencing record is the architect's authored artifact** documenting the DAG she identifies, plus cross-cutting concerns the engine tool doesn't capture.
- **`## Recommended GitHub blockedBy edits` is the bridge** between the sequencing record and the engine tool. The architect's groom recommends edits the operator (or future automation) applies to GitHub. Once applied, `resolve_issue_order` consumes them.
- **Both artifacts are needed.** The sequencing record carries human/LLM-readable rationale + cross-cutting concerns. GitHub `blockedBy` edges carry the machine-readable graph the engine traverses.

Living on a coordination branch (not the main branch) keeps the record discoverable but not blocking; merges into main when the milestone closes per the kg-milestone-14 retrospective's "merge-all-then-deploy-once" pattern.

### D4: New operator command `/mika-groom-milestone.md` (sibling, not extension)

**Decision:** Add a new file `.claude/commands/mika-groom-milestone.md` in the meta-repo (and propagate to each sub-repo's `.claude/commands/` per existing dispatch convention). Do not extend `/mika-groom-ticket.md`.

**Rationale:** Mirrors D1 at the operator surface. `/mika-groom-ticket.md`'s phase structure (Phase 1: parse + branch, Phase 2: worktree + plan, Phase 3: first pass, Phase 4: revisions + second pass, Phase 5: finalize) is reused as the per-sub-issue inner loop, but the outer milestone phases (enumerate sub-issues, assemble sequencing record, milestone-level disposition) are distinct concerns. Per `dispatcher-cross-file-invariant-2026-04-28.md`, when one agent authors both files together this is fine; the invariant is that they don't drift, which is enforced by the plan listing both as touchpoints.

### D5: Recursive self-review carve-out for this PR — instance #3, codification ships

**Decision:** Second-pass review routes to external reviewer (Claude Chat or Vincent direct) per `recursive-self-review-carve-out-2026-04-26.md`. Codification-prep ships in this PR (Unit 8) — instance #3 has been reached.

**Rationale:** This PR ships changes to mika-arch's own bundled skills. Asking mika-arch to second-review a plan that adds a new skill to her own surface is the recursive-self-review failure mode the carve-out prevents.

Instance count verified by grep across `docs/plans/` and `docs/solutions/` on 2026-04-29:
- Instance #1: mika#788
- Instance #2: mika#872 (`docs/plans/2026-04-28-003-feat-promotion-protocol-prompts-and-reflection-spec-plan.md`)
- **Instance #3: this PR**

Per the carve-out doc § "When to revisit", instance #3 promotes codification-prep **to ship**. Codification-prep is therefore in-scope (Unit 8): assemble three-instance evidence, draft codification language for `docs/architecture/review-guide.md`, ship as a section update in this PR.

### D6: Per-skill LLM override mirrors `mika-arch-groom-ticket`

**Decision:** New skill uses Claude Opus 4.7 (same override as `mika-arch-groom-ticket` at `well_known_agents.rs:193-198`).

**Rationale:** Milestone grooming aggregates the same kind of architectural reasoning the per-ticket flow does, just over more inputs. Consistent model choice avoids cross-pass drift between per-sub-issue grooms and the milestone-level synthesis.

### D7: MIKA_ARCH_SOUL receives one-line update only

**Decision:** Add a single line under `## Behaviors` referencing milestone scope (the canonical disposition vocabulary at lines 600-602 stays unchanged because no new tokens). No restructuring.

**Rationale:** Per `core-memory-as-citation-not-accumulator-2026-04-28.md` (the policy #872 enforces), soul edits should be minimal-citation, not narrative accretion. The skill prompt itself is the durable artifact for milestone-grooming behavior; the soul cites that behavior exists.

### D8: Failure-aggregation rule for milestone-level disposition

**Decision:** Per-sub-issue dispositions aggregate to the milestone-level disposition by **highest-severity-wins** ordering: `ESCALATE > ITERATE > READY`. Partial-state propagation rules:

- **All sub-issues `READY`** → milestone disposition `READY`. Operator may dispatch immediately.
- **At least one `ITERATE`, none `ESCALATE`** → milestone disposition `ITERATE`. The READY sub-issues stay groomed (their plans are committed); the operator iterates on the ITERATE sub-issues. Re-running the milestone groom reuses READY plans (R5 idempotence) and re-grooms the ITERATE ones.
- **At least one `ESCALATE`** → milestone disposition `ESCALATE`. Halt. Operator decides whether to drop the escalated sub-issue from milestone scope, escalate to Vincent, or rework.

**Rationale:** The sub-issues in a milestone are not all-or-nothing. A milestone with 3 READY + 1 ITERATE is more groomed than one with 4 ITERATE. Allowing the READY plans to stay committed preserves the cheapest-action progress while concentrating iteration on the actual blocker. ESCALATE remains a hard halt because escalated sub-issues require human judgment that doesn't compose.

The aggregation rule is **enforced by the operator command** (`/mika-groom-milestone.md`), not by the skill prompt. The skill emits per-sub-issue dispositions individually; the command aggregates and emits the milestone-level `Disposition: <KEYWORD>` final line. This keeps the skill's output contract single-shape (per D1) and the aggregation deterministic (per D2 — closed alphabet preserved).

## Open Questions

### Resolved During Planning

- **Q: New disposition tokens or reuse?** → D2: reuse with additive `Scope:` header.
- **Q: Extend `mika-arch-groom-ticket` or sibling skill?** → D1: sibling.
- **Q: Where does the sequencing record live and what's its schema?** → D3.
- **Q: Does `mika-arch-second-review` need extension?** → No (D5 carve-out): for this PR specifically, second-pass routes external. The skill itself stays per-ticket-shaped; future milestone-second-review is deferred until the recurrence demands it (instance #3 of carve-out triggers codification-prep, which may include a milestone-second-review skill).

### Deferred to Implementation

- **Q: Exact text of the milestone-skill system_prompt.md sections.** Mirror `mika-arch-groom-ticket`'s structure (Operating Discipline, Process, Output, Constraints) but the Process section will be longer (~80–100 lines) covering enumeration, per-sub-issue dispatch, sequencing-record assembly. Final wording resolves during /ce:work.
- **Q: How does the operator command detect that mika-arch's first-pass succeeded for *all* sub-issues vs partial?** Likely: aggregate over per-sub-issue dispositions — if any returns ITERATE/ESCALATE, the milestone-level disposition reflects the highest-severity outcome. Final aggregation rule resolves during /ce:work after seeing the actual prompt loop behavior.
- **Q: tools.json content for the new skill.** Likely identical to `mika-arch-groom-ticket/tools.json` (read-only tool kit per the architect's constraints). Verify during implementation.

## Output Structure

```
mika/
├── skills/bundled/
│   └── mika-arch-groom-milestone/         (NEW)
│       ├── skill.toml
│       ├── system_prompt.md
│       └── tools.json
├── crates/mika-agent/src/
│   └── well_known_agents.rs               (MODIFY: 4 allowlist sites + LLM override)
└── docs/plans/
    └── templates/
        └── milestone-sequencing-record-template.md  (NEW)

mika-platform/.claude/commands/
└── mika-groom-milestone.md                (NEW)

mika/.claude/commands/
└── mika-groom-milestone.md                (NEW — propagated copy per existing convention)
```

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

```
Operator: /mika-groom-milestone mika milestone#19
   │
   ├── Phase 1: Parse milestone ref → fetch milestone + sub-issues via gh
   │
   ├── Phase 2: Set up milestone-coordination branch + worktree
   │       (uses scripts/derive-branch-name with --explicit "feat/milestone-19/coordination")
   │
   ├── Phase 3: Per-sub-issue inner loop
   │       For each sub-issue (in declared order, parallelizable when independent):
   │         3a. invoke /mika-groom-ticket internally (reuse all 5 phases)
   │         3b. capture disposition + plan path
   │
   ├── Phase 4: First-pass milestone synthesis via mika-arch-groom-milestone skill
   │       Send peer-review brief covering all sub-issue plans + cross-cutting concerns
   │       Skill returns:
   │         <annotated commentary on cross-ticket assumptions>
   │         Scope: milestone
   │         Disposition: READY|ITERATE|ESCALATE
   │
   ├── Phase 5: Second-pass external review (carve-out per D5)
   │       Output:
   │         Scope: milestone
   │         Verdict: GROOMED|ESCALATE
   │
   └── Phase 6: Finalize
         Commit sequencing record on coordination branch
         Push branches
         Attach callouts to milestone parent issue body:
           > - **Coordination branch:** `feat/milestone-19/coordination`
           > - **Sequencing record:** `mika/docs/plans/<file>` @ `<sha>`
           > - **Sub-issues:** #874 (groomed), #875 (groomed), #876 (groomed), #877 (groomed)
           > - **Grooming history:** /ce:plan per-sub-issue → mika-arch first-pass → external second-pass GROOMED
```

## Implementation Units

- [ ] **Unit 1: Create `mika-arch-groom-milestone` bundled skill scaffold**

**Goal:** Add the three required files for a new bundled skill at `mika/skills/bundled/mika-arch-groom-milestone/`.

**Requirements:** R1, R6.

**Dependencies:** None.

**Files:**
- Create: `skills/bundled/mika-arch-groom-milestone/skill.toml` (mirror `skills/bundled/mika-arch-groom-ticket/skill.toml`; new name, new description)
- Create: `skills/bundled/mika-arch-groom-milestone/system_prompt.md` (~80–120 lines; sections per § High-Level Technical Design Phase 4)
- Create: `skills/bundled/mika-arch-groom-milestone/tools.json` (mirror per-ticket tools.json — same read-only kit; verify exact contents at implementation)
- Test: `crates/mika-agent/tests/eval/skills/mika_arch_groom_milestone.rs` (NEW — eval harness test)

**Approach:**
- Mirror `mika-arch-groom-ticket` structure: H2 title, `### Operating Discipline` (citation-or-silence, unchanged), `### Process` (longer; covers enumeration, per-sub-issue dispatch, sequencing-record assembly), `### Output` (literal `Scope: milestone` + per-sub-issue dispositions + per-sub-issue scope footer per D2 and D8 — milestone-level aggregation happens operator-side, not in the skill prompt), `### Constraints` (read-only, same tool kit, citation required).
- The skill prompt does NOT spawn child Claude sessions. Per-sub-issue grooming runs in the operator command's outer loop; the skill receives the bundled brief (all sub-issue plans + cross-cutting context) as input and reasons over it.
- **Cross-cutting concern analysis is REQUIRED, not optional** (per second-pass external review). The skill prompt's `### Process` step on synthesis must explicitly require the architect to: (a) enumerate cross-sub-issue coupling — entities/files/contracts touched by ≥2 sub-issue plans; (b) flag undeclared assumptions where one sub-issue assumes a state another sub-issue produces; (c) propose `blockedBy` edges (D3 Recommended GitHub edits section) the per-sub-issue grooms missed. Without this requirement, the milestone groom degenerates into "concatenate the per-ticket grooms and stamp READY."

**Patterns to follow:**
- `skills/bundled/mika-arch-groom-ticket/system_prompt.md` (input contract declaration, output trailer)
- `skills/bundled/dev-groom/system_prompt.md` (orchestrator-style sibling precedent)

**Test scenarios:**
- Happy path: input = brief listing 3 sub-issues with plan paths + dependencies → skill emits annotated commentary + `Scope: milestone` + `Disposition: READY`.
- Edge case: single-sub-issue milestone (n=1) → skill still emits milestone-shaped output, not per-ticket output.
- Edge case: sub-issue with conflicting acceptance criteria across siblings → skill flags as cross-cutting concern, returns `Disposition: ITERATE`.
- Error path: brief missing required sections (no plan paths) → skill returns `Disposition: ESCALATE` with citation to the schema.
- Integration: the literal `Disposition: <KEYWORD>` final line is parseable by the operator command's regex (verified via Unit 6).

**Verification:**
- `cargo build -p mika-agent` succeeds with the new skill present (build.rs discovers it).
- A representative agent eval run against a synthetic 3-sub-issue brief produces the trailer-line shape.

---

- [ ] **Unit 2: Register new skill in mika-arch identity allowlist + LLM override**

**Goal:** Make the new skill visible to mika-arch at runtime.

**Requirements:** R7.

**Dependencies:** Unit 1 (skill must exist before it can be allowlisted).

**Files:**
- Modify: `crates/mika-agent/src/well_known_agents.rs` (4 allowlist sites at lines 87-88, 114-115, 152-153, 311; LLM override at lines 193-198 — add `mika-arch-groom-milestone` → `claude-opus-4-7` per D6)

**Approach:**
- Add `mika-arch-groom-milestone` to all four allowlist sites (per `rename-bundled-skill-touchpoints-and-gates-2026-04-28.md` checklist; missing any one site causes `apply_identity_allowlist` to evict the skill).
- Add per-skill LLM override row mirroring the existing `mika-arch-groom-ticket` row.
- Update `MIKA_ARCH_SOUL` (lines 579-616) per D7: one-line addition under `## Behaviors` referencing milestone scope. Disposition vocabulary list at lines 600-602 stays unchanged.

**Patterns to follow:**
- `crates/mika-agent/src/well_known_agents.rs` existing rows for `mika-arch-groom-ticket` and `mika-arch-second-review` (template).

**Test scenarios:**
- Happy path: starting mika server with `MIKA_DEV_MODE=true` provisions mika-arch with the new skill visible (`mika ask --agent mika-arch "list your skills"` → output includes `mika-arch-groom-milestone`).
- Edge case: existing mika-arch skill list still includes the prior two skills (no regression in R3).
- Integration: per-skill LLM override applied — running the new skill uses Opus 4.7, not the agent's base model.

**Verification:**
- `mika kg validate` runs clean post-deploy.
- `sqlite3 ~/.mika/data/mika.db "SELECT skill_name, provider, model FROM skill_overrides WHERE agent_id='mika-arch';"` shows the new row with `claude-opus-4-7`.

---

- [ ] **Unit 3: Create operator command `/mika-groom-milestone`**

**Goal:** Operator-side dispatcher that orchestrates the milestone-groom flow.

**Requirements:** R6, R5.

**Dependencies:** Units 1 + 2 (skill must exist and be allowlisted before the command can invoke it).

**Files:**
- Create: `mika-platform/.claude/commands/mika-groom-milestone.md` (canonical location)
- Create: `mika/.claude/commands/mika-groom-milestone.md` (propagated copy per existing dispatch convention; identical content)

**Approach:**
- Mirror `.claude/commands/mika-groom-ticket.md`'s phase structure (lines 22–121).
- Phase 1: Parse `<repo> milestone#<N>` ref. Fetch milestone metadata + sub-issues via `gh api` (`/repos/<owner>/<repo>/milestones/<N>` and `gh issue list --milestone <N>`).
- Phase 2: Use `scripts/derive-branch-name --explicit "feat/milestone-<N>/coordination"` for the milestone-coordination branch (immutable per `dispatcher-cross-file-invariant-2026-04-28.md`); then `scripts/derive-worktree-path --branch ... --repo <repo>` for the path. **Coordination-worktree idempotence (per second-pass external review)** — handle three sub-cases explicitly when the worktree path already exists:
  - **(a) Clean reuse** — worktree exists, branch ref matches expected slug, no uncommitted state → reuse as-is. Phase 3+ writes additively.
  - **(b) Divergent state with prior sequencing record** — worktree exists, branch ref matches, prior sequencing record present (possibly partial from an aborted run) → reuse the worktree, treat the prior record as a draft input. Phase 4 amends rather than overwrites; the architect's groom session sees the prior content and reconciles.
  - **(c) Dispatcher-cross-file invariant violation** — worktree exists but branch ref does NOT match `feat/milestone-<N>/coordination`, OR worktree path slug doesn't match `sanitize(branch_ref)` per `dispatcher-cross-file-invariant-2026-04-28.md` → halt with error. Do NOT auto-rename or auto-recreate. Surface to operator with the mismatch detail; operator decides whether to remove the divergent worktree manually (`git worktree remove --force`) and re-run, or escalate.
- Phase 3: Per-sub-issue inner loop. For each sub-issue, internally invoke the per-ticket flow (read `.claude/commands/mika-groom-ticket.md` and follow it for that sub-issue, NOT via the Skill tool — direct execution). Capture each sub-issue's disposition + plan path.
- Phase 4: Compose milestone-level brief. Send to `mika-arch-groom-milestone` via `/mika-ask-arch`. Capture session_id + parse `Scope: milestone` + `Disposition: <KEYWORD>` final line.
- Phase 5: External second-pass (D5 carve-out). For this implementation: emit a clear pause-and-ask prompt to the operator with the consolidated brief; do NOT route to mika-arch. Capture verdict from operator response.
- Phase 6: Commit sequencing record (Unit 4 schema), push all branches, attach callouts to milestone parent issue body.

**Patterns to follow:**
- `.claude/commands/mika-groom-ticket.md` (full phase structure, scripts invocation, callout attachment).
- `.claude/commands/mika-ask-arch.md` (JSON envelope handling).

**Test scenarios:**
- Happy path: `/mika-groom-milestone mika milestone#19` enumerates 4 sub-issues (#874–877), dispatches each per-ticket groom, produces 4 plan-on-branches + 1 sequencing record, attaches callouts to milestone#19 body.
- Edge case: re-dispatching the same milestone (R5 idempotence) — existing plan-on-branches are reused; per-ticket flow's existing reuse logic handles this.
- Edge case: milestone with 0 open sub-issues → command returns clear "no open sub-issues" message and exits without error.
- Error path: malformed milestone ref (`mika milestone-19` instead of `milestone#19`) → command halts with clear message.
- Integration: each per-sub-issue branch derivation calls `scripts/derive-branch-name` (verified by Unit 6 grep).

**Verification:**
- Manual smoke test against milestone#19: groom produces all expected artifacts, sequencing record matches the dependency graph in milestone#19's body.
- Re-run idempotence: second invocation reuses plan-on-branches without regenerating.

---

- [ ] **Unit 4: Define sequencing record template and schema**

**Goal:** Reusable schema for milestone-level sequencing records.

**Requirements:** R2.

**Dependencies:** None.

**Files:**
- Create: `docs/plans/templates/milestone-sequencing-record-template.md` (template per D3 schema)

**Approach:**
- Frontmatter: `title`, `type: milestone-sequencing`, `milestone`, `date`, `status`.
- Required H2 sections (per D3): `## Sub-issues`, `## Dependencies`, `## Recommended GitHub blockedBy edits`, `## Order`, `## Cross-cutting concerns`, `## Open milestone-level questions`.
- Each section's expected line shape documented inline so the architect's prompt can produce LLM-readable + human-readable content.
- The `## Recommended GitHub blockedBy edits` section is the **bridge to the engine tool** (per D3): list each `blockedBy` edge the architect identifies, paired with a rationale + the `gh issue edit` (or GraphQL) command shape to apply it. The operator (or future automation) applies these edits to GitHub; once applied, `crates/mika-agent/src/tools/resolve_issue_order.rs` consumes them via GraphQL on next invocation.

**Patterns to follow:**
- `docs/solutions/best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md` D-numbered decisions structure.
- Existing plan frontmatter precedents (`docs/plans/2026-04-28-002-chore-extract-mika-arch-foundational-refs-plan.md`).

**Test scenarios:**
- Test expectation: none — pure template/schema documentation, no behavior under test. Unit 1's eval test verifies the architect produces output that conforms to this schema.

**Verification:**
- Template loads (markdown lint passes).
- Unit 1's eval test asserts the architect's output for a 3-sub-issue brief matches the schema.

---

- [ ] **Unit 5: Output-format compatibility report (already pre-verified)**

**Goal:** Document the pre-commit output-format-compatibility verdict per second-pass external review and the discipline in `plan-on-branch-load-bearing-contract-2026-04-26.md` §4.

**Requirements:** R4.

**Dependencies:** Units 1 + 3 (need the skill prompt and operator command to align their emitted shape with the verdict).

**Files:**
- Create: `docs/plans/2026-04-29-002-mika-arch-milestone-grooming-compatibility-report.md` (committed alongside the plan; PR description references it)

**Approach (verdict already established 2026-04-29 during plan grooming):**
- All six callsites for `Disposition:` and `Verdict:` keyword recognition are **LLM-based pattern matching, NOT regex parsers**. There are no anchored `^Disposition:` expressions or line-consuming regex; recognition happens via prompt-level instructions to look for the keywords.
- Therefore an additive `Scope: <milestone|ticket>` header line is unambiguous as long as the literal `Disposition: <KEYWORD>` final-line discipline is preserved (`mika-arch-first-dogfood-2026-04-25.md`).
- The report enumerates the six callsites (`.claude/commands/mika-groom-ticket.md:67-69, 88-89`; `.claude/commands/mika-ask-arch.md:31-33`; `skills/bundled/dev-groom/system_prompt.md:41-43, 56-57`; `well_known_agents.rs:600-602` — `MIKA_ARCH_SOUL`) with verdict per callsite.

**Patterns to follow:**
- `docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md` §4.
- `docs/solutions/best-practices/verification-claims-with-expected-output-shape-2026-04-28.md` (specify command + expected shape).

**Test scenarios:**
- Test expectation: report itself is the verification. Each callsite enumerated with verdict (LLM-based recognition unaffected by additive `Scope:` line).

**Verification:**
- Report committed to the branch alongside the plan.
- PR description references the report.

---

- [ ] **Unit 6: Sub-issue groom flow tests**

**Goal:** Confidence that the operator-side milestone command actually reuses the per-ticket flow correctly.

**Requirements:** R3, R5.

**Dependencies:** Units 1, 2, 3.

**Files:**
- Create: `mika/scripts/test-mika-groom-milestone.sh` (smoke test fixture; calls the command against a synthetic milestone)
- OR: integration test in `crates/mika-agent/tests/` if a Rust-side surface exists; otherwise shell smoke test only.

**Approach:**
- Smoke test: against a real test milestone (or milestone#19 itself in dry-run mode), verify:
  - Per-sub-issue branches use canonical derivation (`scripts/derive-branch-name`).
  - Per-sub-issue plan files land at `<repo>/docs/plans/<expected-name>-plan.md`.
  - Sequencing record lands at the expected location.
  - Issue body callouts attached to milestone parent.

**Patterns to follow:**
- `scripts/verify-pipeline.sh` (existing smoke-test pattern).

**Test scenarios:**
- Happy path: test milestone with 2 sub-issues → 2 plan-on-branches + 1 sequencing record + callouts.
- Edge case: re-run produces same artifacts (idempotence).
- Error path: invalid milestone number → clean exit.

**Verification:**
- Test passes locally before push.
- CI runs the smoke test (if CI surface exists for this; otherwise documented as manual pre-merge gate).

---

- [ ] **Unit 7: Codification of recursive-self-review carve-out into review-guide.md (instance #3 trigger)**

**Goal:** Per the carve-out doc § "When to revisit", instance #3 promotes codification-prep to ship. Assemble three-instance evidence and update `docs/architecture/review-guide.md` with the codified rule.

**Requirements:** Carve-out doc instance-tracking discipline.

**Dependencies:** None (independent of Units 1–7; can land in parallel).

**Files:**
- Modify: `docs/architecture/review-guide.md` (add new section: when modifying mika-arch's own bundled skill surface, second-pass routes external — three-instance evidence base)
- Modify: `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md` (update § "When to revisit" — the rule is now codified; mark the section as historical evidence rather than active trigger)

**Approach:**
- Three-instance evidence to cite in `review-guide.md`:
  - Instance #1: mika#788 (referenced in carve-out doc as the originating case)
  - Instance #2: mika#872, plan `docs/plans/2026-04-28-003-feat-promotion-protocol-prompts-and-reflection-spec-plan.md` § Process Note
  - Instance #3: this PR, plan `docs/plans/2026-04-29-001-feat-mika-arch-milestone-grooming-plan.md` § Process Note
- Codification language (draft, refine during /ce:work): "When a plan modifies mika-arch's own bundled skill surface (`skills/bundled/mika-arch-*`) or mika-arch's identity allowlist in `well_known_agents.rs`, the second-pass review MUST route to an external reviewer (Vincent or another Claude instance via Claude Chat). First-pass may stay with mika-arch only when the change is purely additive — no skill being deprecated, no behavioral contract under reduction. ESCALATE on any other shape."
- The carve-out doc itself stays as the evidence record; review-guide.md becomes the active rule.

**Patterns to follow:**
- `docs/architecture/review-guide.md` existing sections (citation-or-silence, what-NOT-to-flag, etc.) — same rule-with-citation structure.

**Test scenarios:**
- Test expectation: none — documentation codification. Verified by reading.
- Smoke check: a future PR modifying `skills/bundled/mika-arch-*` references review-guide.md's new section in its Process Note rather than re-deriving the rule.

**Verification:**
- `review-guide.md` has the new section.
- Carve-out doc updated to reference review-guide.md as the active rule.
- PR description explicitly notes carve-out instance #3 → codification shipped.

---

- [ ] **Unit 8: Documentation updates**

**Goal:** Update workflow documentation so operators know the new command exists.

**Requirements:** R6.

**Dependencies:** Units 1–7.

**Files:**
- Modify: `mika-platform/CLAUDE.md` (add a § Mandatory `/mika` Pipeline subsection or sibling on milestone grooming, OR a new § Milestone grooming section).
- Modify: `mika-platform/.claude/commands/mika-groom-ticket.md` (add a § Related cross-link to `/mika-groom-milestone`).
- Modify: `mika/CLAUDE.md` (parallel update if it documents the per-ticket grooming flow).

**Approach:**
- Add a short prose section: "When to use `/mika-groom-milestone` vs `/mika-groom-ticket`" — single ticket = ticket; ≥2 tickets sharing a GitHub milestone = milestone.
- Cross-link the sequencing record schema (Unit 4 template path) so operators can read it.

**Patterns to follow:**
- Existing CLAUDE.md sections on `/mika` pipeline.

**Test scenarios:**
- Test expectation: none — documentation. Reviewer reads and confirms clarity.

**Verification:**
- Markdown lint passes.
- Reviewer accepts.

## System-Wide Impact

- **Interaction graph:** New skill `mika-arch-groom-milestone` is invoked only via the new operator command. The existing per-ticket flow is unchanged (`mika-arch-groom-ticket` keeps its single-issue contract).
- **Error propagation:** Per-sub-issue groom failures bubble up to the milestone command, which aggregates into a milestone-level disposition. Failures do not cascade — a single sub-issue ESCALATE pauses the whole flow until operator decision.
- **State lifecycle risks:** Each sub-issue's worktree is independent (canonical derivation guarantees disjoint paths). Milestone-coordination worktree is a fifth disjoint location. No shared mutable state.
- **API surface parity:** None — milestone shape is additive, ticket shape unchanged.
- **Integration coverage:** Unit 6's smoke test covers the cross-layer `command → skill → architect → callout-attachment` flow that mocks alone cannot prove.
- **Unchanged invariants:**
  - `mika-arch-groom-ticket` and `mika-arch-second-review` skills, prompts, and contracts.
  - Per-ticket `/mika-groom-ticket` command (Phase 1–6 unchanged; this plan adds a sibling that *internally* invokes the same logic, it does not modify the existing command).
  - Disposition/Verdict closed token alphabet (`READY|ITERATE|ESCALATE` and `GROOMED|ESCALATE`) — no new tokens (D2).
  - `MIKA_ARCH_SOUL`'s disposition vocabulary list at lines 600-602.
  - Branch slug + worktree path immutability invariant (`dispatcher-cross-file-invariant-2026-04-28.md`).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Architect ghosts the milestone-shape contract under load (precedent: `required-tools-gate-evasion-patterns-2026-04-28.md` N=2) | Sibling-skill structure (D1) means the milestone-shape contract IS the entire skill — no input-shape branching to ghost. Eval test (Unit 1) asserts trailer shape on synthetic input. |
| Operator command's per-sub-issue inner loop drifts from `/mika-groom-ticket`'s phase structure | Unit 3 explicitly mirrors the existing command structure; Unit 6's smoke test verifies parity. |
| Architect omits `## Recommended GitHub blockedBy edits` section, leaving DAG visible only in markdown — engine tool can't traverse it (D3) | Skill prompt (Unit 1) makes the section REQUIRED, not optional; eval test (Unit 1 test scenarios) asserts presence and non-empty content for any milestone with declared dependencies. |
| Operator forgets to apply recommended `blockedBy` edges to GitHub before invoking `resolve_issue_order` (D3) | Operator command (Unit 3) Phase 6 callout summary lists the `gh issue edit` commands explicitly; sequencing record commit message references "blockedBy edits pending" until applied. Future enhancement: auto-apply (out of scope for this PR — operator confirmation gate is enough). |
| Carve-out instance #2 (D5) creates an external-review bottleneck on this PR | Vincent or Claude Chat handles second-pass; brief is small (this plan + 4 sub-issue references). Acceptable single-PR cost. |
| `Scope: milestone` header line breaks an undiscovered parser | Unit 5 grep across all three repos before merge. If anchored `^Disposition:` regex parsers exist that consume the trailing context, additive header still passes (header is on a different line). |

## Documentation / Operational Notes

- Post-merge: file the codification-prep ticket flagged in § Scope Boundaries (carve-out instance #2 follow-up).
- Post-deploy: re-dispatch milestone#19 via the new command. This dogfoods the new skill against the original motivating milestone. If successful, milestone#19's sub-issues drain; if not, the dogfood result becomes the codification-prep evidence.
- No deploy hook needed — new skill is auto-discovered by `build.rs` on next mika-server restart.

## Sources & References

- **Origin issue:** [senara-solutions/mika#879](https://github.com/senara-solutions/mika/issues/879)
- **Related milestone:** [senara-solutions/mika#19](https://github.com/senara-solutions/mika/milestone/19)
- **Sibling sub-issues:** mika#874, mika#875, mika#876, mika#877
- **Closest plan precedent:** `docs/plans/2026-04-27-011-feat-add-dev-groom-bundled-skill-plan.md`
- **Decisive compound docs:**
  - `docs/solutions/best-practices/prompt-rule-cheapness-bias-toward-wrong-layer-2026-04-28.md`
  - `docs/solutions/best-practices/operator-only-bundled-skill-structural-enforcement-2026-04-28.md`
  - `docs/solutions/best-practices/plan-on-branch-load-bearing-contract-2026-04-26.md`
  - `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md`
  - `docs/solutions/best-practices/recursive-self-review-carve-out-2026-04-26.md`
  - `docs/solutions/best-practices/dispatcher-cross-file-invariant-2026-04-28.md`
  - `docs/solutions/cross-repo-patterns/centralized-derivation-load-bearing-invariant-2026-04-28.md`
  - `docs/solutions/architecture-patterns/milestone-resolver-topo-sort-deploy-hooks-2026-04-21.md`
  - `docs/solutions/best-practices/socratic-multi-ticket-milestone-planning-2026-04-21.md`
  - `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`
- **Codebase pointers:**
  - `skills/bundled/mika-arch-groom-ticket/system_prompt.md`
  - `skills/bundled/mika-arch-second-review/system_prompt.md`
  - `skills/bundled/dev-groom/system_prompt.md`
  - `crates/mika-agent/src/well_known_agents.rs:87-88, 114-115, 152-153, 193-198, 311, 579-616`
  - `mika-platform/.claude/commands/mika-groom-ticket.md`
  - `mika-platform/.claude/commands/mika-ask-arch.md`
  - `scripts/derive-branch-name`, `scripts/derive-worktree-path`
