---
ticket: mika#1154
title: KG subject extractor produces subjects with no domain-graph counterpart
type: feat
labels: [enhancement, p1-important, agent-core]
created: 2026-05-16
plan_seq: 002
status: drafted
related:
  - mika#1152 (parent investigation — closed by sonnet-baseline experiment)
  - mika#1076 (corpora backfill — umbrella-paused)
  - mika#1077 (no_match instrumentation — umbrella-paused)
  - mika#1091 (gpt-5-nano MaxTokens — umbrella-paused)
  - docs/solutions/kg-investigations/2026-05-16-resolver-sonnet-baseline.md
  - docs/brainstorms/2026-05-11-kg-query-tool-deprecation-question.md
---

# Plan: KG subject-extractor roster grounding (mika#1154)

## Recap (one paragraph)

The resolver-sonnet baseline (mika#1152) found 0/87 sampled `no_match` outcomes
have a domain-entity match by `entity_key` or by name. The resolver isn't
failing to disambiguate — the LLM extractor is producing subjects (`agent:vincent`,
`agent:ci`, `agent:tower_http`, `tool:tailwind`) that have **no canonical
counterpart in the domain graph** and never could (operator names, abstract
roles, Rust crates, CSS frameworks are not platform agents/tools). This is the
structural cause of the chronic 99% miss-rate. Decision A (2026-05-12) removed
`query_knowledge_graph` from mika-arch, so there is currently **no active
consumer of resolved entities** — this ticket's urgency is anticipatory, not
firefighting.

## Code grounding (verified, file:line)

- `crates/mika-agent/src/kg/subject_extractor.rs:38–47` — approved entity types
  enum (hardcoded const).
- `crates/mika-agent/src/kg/subject_extractor.rs:171–323` — validation; **no
  roster check** today. Filters structurally invalid items only; accepts any
  `<type>:<name>` pair where type is approved.
- `crates/mika-agent/src/kg/subject_extractor.rs:800+` — prompt assembly; the
  extractor is **completely roster-blind**. No reference to live domain
  entities anywhere in the LLM prompt.
- `crates/mika-agent/src/kg/domain_builder.rs:12–21` — **sole-writer
  contract**: this module is the only writer of `skill:*`, `tool:*`, `agent:*`,
  `problem_type:*`, `concept:*` entity keys. Startup-only, deterministic, from
  schema (skill manifests, tool registry, MCP, hardcoded seeds for
  problem_type + concept).
- `crates/mika-agent/src/kg/entity_resolver.rs:56–65` — outcome string
  constants. `NO_MATCH` is a single bucket today.
- `crates/mika-agent/src/kg/entity_resolver.rs:617–622` — `no_match` write
  site: LLM's `matched: None` parses to `ResolutionResult::NoMatch` →
  `apply_result()` writes `kg_resolutions_log` with `outcome = 'no_match'` +
  model name + extraction trace ID. **No domain-of-same-type pre-check
  exists.**

## Shape selection rationale

The ticket enumerates three shapes. Code reading collapses the choice.

### Shape 2 (widen roster via observation) — REJECTED

- **Why rejected**: Violates the sole-writer contract documented at
  `domain_builder.rs:12–21`. The domain graph is intentionally schema-driven:
  skills come from manifests, tools from `ToolRegistry`/`McpManager`, agents
  from configs, problem-types and concepts from hardcoded seeds. Switching to
  observation-driven emission would (a) accept extractor output as canonical
  ground truth (i.e., let the LLM define what counts as a Mika tool), (b)
  pollute the catalog with low-signal entries (`tool:tailwind` becomes
  canonical, then resolves to itself, then "matches" — the metric goes green
  while the underlying error gets worse), and (c) remove the only invariant
  that distinguishes domain truth from extraction output.
- **The four cited examples confirm**: `agent:tower_http` is a Rust crate; if
  observation-driven emission was enabled, the domain graph would gain
  `agent:tower_http` as a canonical entity. That's the failure mode this
  contract exists to prevent.

### Shape 1 (constrain extractor to closed-set roster) — PRIMARY FIX

- **Why this is the structural fix**: It's the only shape that addresses the
  type-misclassification revealed by the four examples. Telling the LLM "here
  is the complete list of agents/tools/problem_types/concepts that exist;
  refuse to emit anything outside this set, OR emit with a `discovered: true`
  flag and a specific reason" makes hallucinated subjects impossible by
  construction.
- **Cost**: prompt-size growth. Ballpark estimate (needs validation, see open
  questions): current skill registry ≈ 30 skills, tool registry ≈ 60 builtins
  + dozens of MCP tools, 11 agent configs, 5 problem-types, 20 concepts. That
  is ~130 lines if rendered as a flat list. Per-extraction call this is
  recoverable; we already pay for the per-chunk doc text which dominates.
- **Risk**: extractor becomes brittle to roster turnover (a new skill not yet
  in the registry at extraction time gets refused). Mitigated by treating the
  roster as a soft prior (allow `discovered: true` with reason) instead of a
  hard filter.

### Shape 3 (split `no_match` outcome) — DIAGNOSTIC COMPLEMENT

- **Why this is also valuable**: Even if Shape 1 lands, we want the
  diagnostic split — it's how we measure Shape 1's effectiveness over time.
  Splitting `no_match` into `no_candidate_of_type` (extractor produced a
  phantom of type T but no canonical entity of type T matches by name) vs
  `candidates_exist_no_match` (the resolver saw candidates and rejected all)
  is cheap (one outcome string → two, plus a pre-check in `apply_result()` or
  one level up) and turns the headline metric into something actionable.
- **Without Shape 3, Shape 1's success is unmeasurable**: today every miss is
  bucketed identically. After Shape 1 we'd expect the curve to shift from
  "mostly phantoms" to "mostly real disambiguation failures." Shape 3 is the
  measurement.

### Recommendation

**Hybrid Shape 1 + Shape 3, sequenced**:
1. **First**: Shape 3 (the diagnostic). Cheap, observation-only, no consumer
   risk. Establishes the baseline metric Shape 1 will move.
2. **Then**: Shape 1 (the roster constraint). Larger surface, prompt-size
   experiment, validated against the Shape 3 baseline.

**Sequencing rationale**: Shape 3 gives us the measurement instrument before
we run the experiment. If we ship Shape 1 alone, we can only judge it by
"miss-rate went down" — but the miss-rate has many causes, and we'd be
unable to isolate the roster-grounding effect from e.g. a model swap or a
prompt edit. Shape 3 first, Shape 1 second, lets us see the curve.

**Anticipated counter-argument**: "There's no consumer right now (Decision A
removed mika-arch's `query_knowledge_graph`), why fix it at all?" The
extractor runs regardless of consumers — every ingested doc still produces
subject rows. The miss-rate continues to grow technical debt in the
`kg_subject_entities` table. Shape 3 is cheap insurance; Shape 1 is the
unblock when a consumer returns. This ticket files the work; we don't
necessarily ship Shape 1 this week.

## Decomposition

This is a **two-ticket plan**, not a single ticket. mika#1154 is the
umbrella scope question; the implementation work breaks into:

### Work unit A — Shape 3 (this ticket, mika#1154)

**Scope**: Split `no_match` into two outcomes in `entity_resolver.rs`. Adds
one DB query in the resolver path (or reuses the existing Stage 1 lookup
result), no schema change.

**Files**:
- `crates/mika-agent/src/kg/entity_resolver.rs:56–65` — add
  `NO_CANDIDATE_OF_TYPE` constant alongside existing outcomes.
- `crates/mika-agent/src/kg/entity_resolver.rs:617–622` (and the surrounding
  fn) — before returning `NoMatch`, look up `kg_entities` count for
  `(type = subject.type)` filtered by name (case-insensitive). If zero
  same-type candidates exist by name, return `NoCandidateOfType`.
  Otherwise keep `NoMatch`.
- Migration: **none**. `outcome` is `TEXT` in `kg_resolutions_log`; adding a
  new string value doesn't require schema change.
- Backfill: **none** in initial PR. Historical rows stay `no_match`. Filed
  follow-up if dashboards demand reclassification.

**ACs**:
1. New outcome `no_candidate_of_type` appears in `kg_resolutions_log` rows
   produced after deploy.
2. Existing `no_match` semantics narrow to "candidates of correct type
   exist but resolver rejected" — verifiable by manual `SELECT` on a known
   subject with same-type candidate.
3. Unit test in `entity_resolver.rs`'s test block covering both branches
   (phantom subject → `no_candidate_of_type`, real subject rejected by
   LLM → `no_match`).
4. No regression in existing 30+ resolver tests.

**Out of scope**:
- Surfacing the new outcome in any dashboard or audit tool. (Filed separately.)
- Backfilling historical rows.
- Any prompt change.

### Work unit B — Shape 1 (NEW FOLLOW-UP TICKET, to be filed at handoff)

**Scope**: Inject the live domain-entity roster (`agent:*`, `tool:*`,
`problem_type:*`, `concept:*` only — not `skill:*`, see open question Q1)
into `build_extraction_prompt()`. Allow `discovered: true` flag in the
extractor's JSON schema for subjects whose name is not in the roster
(refusal + reason), validated at `validate_extraction_output()`.

**Files**:
- `crates/mika-agent/src/kg/subject_extractor.rs:800+` — modify prompt
  assembly to fetch `SELECT entity_key, name FROM kg_entities WHERE
  type IN (...)` and render as a list section.
- `crates/mika-agent/src/kg/subject_extractor.rs:171–323` — validation
  layer; add `discovered: bool` field handling.
- Update prompt instructions to "only emit subjects whose name is in the
  roster below; for subjects not in the roster, set `discovered: true`
  and include a `discovery_reason` field."

**ACs** (deferred — full ACs in the follow-up ticket).

**Why split**: Shape 3 is observational and committable on its own. Shape
1 changes the LLM prompt shape and needs its own architect review on the
"discovered" carveout design. Bundling them obscures which change moved
the metric.

## Acceptance criteria (for this ticket — mika#1154 = Shape 3 only)

1. `entity_resolver.rs` defines `NO_CANDIDATE_OF_TYPE` outcome string.
2. Resolver returns the new outcome when a subject's type+name has zero
   matches in `kg_entities`.
3. Existing `no_match` is preserved for the candidates-exist-but-rejected
   case.
4. New unit test covers both branches.
5. No DB migration (outcome column is TEXT).
6. Follow-up ticket filed for Shape 1 (roster injection in extractor
   prompt), linked from this ticket's close comment.

## Open questions for architect

**Q1. Should `skill:*` be in the Shape 1 roster?** Skills are extracted
by-name from prose ("the self-dev skill", "the mika-handsoff skill"). Skill
names are versioned/renamed; an extractor seeing a stale doc reference to
a renamed skill should arguably produce a `discovered: true` entity, not
refuse. But that increases the roster carveout surface. Defer to architect.

**Q2. Shape 3 outcome ordering — `no_candidate_of_type` resolved before or
after `skipped_discovered_type`?** The latter (line ~60) fires when the
subject's type isn't in the domain at all (`solution_path`, `failure_mode`,
`pattern`). `no_candidate_of_type` fires when type IS in domain but no
same-type name match exists. Both should be emitted; what's their relative
priority if both could apply? Probably `skipped_discovered_type` wins (type
not in domain → don't even check candidates). Confirm.

**Q3. Should Shape 3 add an index?** The pre-check
`SELECT COUNT(*) FROM kg_entities WHERE type = ? AND lower(name) = lower(?)`
needs an index to avoid full-scan per resolution. Existing index review:
TBD — needs a quick scan of `kg_entities` indices. If the index is
missing, do we add it as part of this PR or file a separate ticket?

**Q4. Is the sequencing right?** Shape 3 first → measure → Shape 1 second.
Or do we ship Shape 1 immediately on the theory that the diagnostic split
is rendered moot once phantoms stop existing? Counterargument: Shape 3
also surfaces non-phantom-driven rejections that Shape 1 doesn't fix
(genuine name disambiguation failures). Architect's call.

## Risks

- **Shape 2 framing in the ticket misleads grooming**: The ticket presents
  three shapes as parallel choices. Code grounding shows Shape 2 is a
  sole-writer-contract violation. The plan above rejects it explicitly so
  the architect doesn't have to re-derive the argument.
- **Anticipatory work with no current consumer**: Decision A removed
  mika-arch's `query_knowledge_graph` 2026-05-12. We're investing in a
  surface no agent currently uses. Justified by (a) extraction continues
  regardless, (b) re-enabling the consumer requires this fix anyway,
  (c) Shape 3 is cheap. If architect disagrees, this becomes a "park
  pending consumer return" outcome.
- **Roster injection prompt-size growth (Shape 1)**: Defer to follow-up
  ticket; not this PR's risk.

## Out of scope

- Shape 1 implementation (filed as follow-up at grooming close).
- Shape 2 (rejected; rationale documented above).
- Backfilling historical `no_match` rows to `no_candidate_of_type`.
- Any change to `query_knowledge_graph` re-enable on mika-arch (Decision A
  stands).
- Subject-extractor LLM model swap (separate experiment).

## Compounding hooks

Two patterns surface that should compound if this plan ships:
1. **Sole-writer contracts are load-bearing**: domain_builder.rs's
   sole-writer comment is the only thing preventing Shape 2 from looking
   reasonable. Add to `docs/solutions/best-practices/` if not already
   covered.
2. **Diagnostic-before-fix sequencing**: Shape 3 before Shape 1 is an
   instance of "measure first" — file under
   `docs/solutions/cross-repo-patterns/` if there isn't already a peer.
