---
title: "KG resolver type-allowlist contract — what `count_pending` actually counts (and what it doesn't)"
date: 2026-05-06
category: best-practices
module: mika-agent/kg/entity_resolver
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Reading `kg_resolver_tick.complete` log events for `pending_before` / `pending_after` interpretation
  - Reading `mika kg status` and seeing a non-zero "pending" column
  - Writing custom DB queries against `kg_subject_entities` to estimate resolver backlog
  - Auditing whether the periodic resolver tick (#906) is making progress
tags:
  - kg
  - audit
  - resolver
  - observability
  - type-allowlist
  - subject-graph
  - mika-arch
---

# KG resolver type-allowlist contract — what `count_pending` actually counts

## Context

A 2026-05-06 audit observed `kg_resolver_tick.complete` reporting `pending_before: 0` and `per_corpus_attempted: "{}"` on every tick post-restart while `mika kg status` reported ~28,000 "pending" entities per mika-family agent. The audit author (me) interpreted this as a regression of #927's per-corpus fairness, filed mika#997, and authored a "regression detection" diagnostic doc. Reading `count_pending()` during grooming exposed the error: the two counters measure different things, and the resolver was working correctly.

This doc replaces the original (which framed the contract gap as a regression) with the actual contract — so the next operator reading `mika kg status` or the resolver tick log doesn't repeat the misread.

The shape of the lesson, before the details: **`count_pending()` deliberately scopes to 5 of the 8 entity types the subject extractor produces. `mika kg status` "pending" doesn't apply that scope. The two numbers are not supposed to agree.**

## Guidance

### The type taxonomy

The KG subject extractor (`crates/mika-agent/src/kg/subject_extractor.rs:43-90`) produces eight entity types. Two roles:

| Type | Role | Has `kg_entities` projection? | Resolver processes? |
|---|---|---|---|
| `skill` | Domain | ✅ via `domain_builder` | ✅ |
| `tool` | Domain | ✅ via `domain_builder` | ✅ |
| `agent` | Domain | ✅ via `domain_builder` | ✅ |
| `problem_type` | Domain | ✅ via `domain_builder` | ✅ |
| `concept` | Domain | ✅ via `domain_builder` | ✅ |
| `pattern` | Subject-graph-only | ❌ | ❌ |
| `failure_mode` | Subject-graph-only | ❌ | ❌ |
| `solution_path` | Subject-graph-only | ❌ | ❌ |

The resolver's job is to map subject-graph entities of the first 5 types onto canonical `kg_entities` rows (the deterministic domain projection of skills/tools/agents/etc.). Entities of the last 3 types live in the subject graph as standalone nodes, linked into the broader graph via `kg_subject_relationships`. They have no canonical domain entity to resolve against, so the resolver intentionally skips them.

### What `count_pending()` counts

The 5-type filter is hard-coded in `crates/mika-agent/src/kg/entity_resolver.rs:891-906`:

```sql
SELECT COUNT(*)
FROM kg_subject_entities e
LEFT JOIN kg_resolutions_log r
    ON r.subject_entity_id = e.id AND r.agent_id = ?
WHERE e.docs_root_hash IN (?, ?, ...)
  AND e.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')   -- <-- the allowlist
  AND (
    r.id IS NULL
    OR r.source_extraction_trace_id != (
        SELECT cs.extraction_trace_id
        FROM kg_chunk_subjects cs
        WHERE cs.subject_entity_id = e.id
        ORDER BY cs.created_at DESC LIMIT 1
    )
  )
```

This is the count fed into `kg_resolver_tick.complete`'s `pending_before` field via `tick_body()` (`crates/mika-agent/src/kg/resolver_tick.rs:95`). It answers: *"How many domain-resolvable subject entities still need a resolution log entry against the most recent extraction trace?"*

Healthy state for the post-restart resolver is **`pending_before: 0`** for every agent — that means every entity the resolver is supposed to process has been processed.

### What `mika kg status` "pending" counts

`mika kg status` computes `pending` by counting `kg_subject_entities` rows lacking a corresponding `kg_subject_resolutions` row (the table that holds final resolution outcomes), without applying the 5-type filter. So its "pending" count includes:

- Entities of resolvable types that haven't been seen yet by the resolver (legitimate backlog)
- Entities of subject-graph-only types that the resolver will never touch (not actionable)

A high "pending" number in `mika kg status` does NOT imply the resolver has work to do. It implies the subject graph holds entities outside the resolver's scope, which is the steady-state design.

This conflation is filed as `senara-solutions/mika#999` ("`mika kg status` pending count conflates subject-graph-only types with actionable resolver backlog"). Until that ships, anyone reading the CLI must interpret the column with the type-allowlist contract in mind.

### The diagnostic that does work

To estimate the **actionable** resolver backlog, mirror `count_pending()`'s scope. The 2026-05-06 audit data:

```sql
SELECT akc.agent_id, COUNT(*) AS actionable_pending
FROM kg_subject_entities se
JOIN agent_kg_corpora akc ON akc.docs_root_hash = se.docs_root_hash
WHERE se.type IN ('skill', 'tool', 'agent', 'problem_type', 'concept')
  AND NOT EXISTS (
    SELECT 1 FROM kg_resolutions_log rl
    WHERE rl.agent_id = akc.agent_id
      AND rl.subject_entity_id = se.id
  )
GROUP BY akc.agent_id;
```

Returns 0 rows on 2026-05-06 = no actionable work. Compare against `kg_resolver_tick.complete` `pending_before` — they should agree (both are scoped to the same 5 types).

A custom query without the type filter is the audit-author trap. Don't write one.

## Why This Matters

1. **Misreading the resolver's scope produces a false-regression filing pipeline.** The 2026-05-06 audit produced mika#997 (closed-invalid), an incorrect Audit 5 doc (this file's predecessor), and incorrect refresh edits to two sibling docs. Roughly an hour of cleanup. The misread was not subtle — `count_pending()`'s SQL is one function, 30 lines, with an explicit type allowlist.
2. **The CLI counter and the tick counter are designed for different audiences.** `mika kg status` shows the full subject graph including non-resolvable types because those entities are real graph nodes used by traversal queries. The tick counter is scoped to actionable work. The two are correct on their own terms; they're not redundant counters of the same thing.
3. **Adding a 6th resolvable type is a multi-place change.** If a future entity type needs domain-graph resolution, the change has to land in at least three places: the type allowlist in `count_pending()`, the equivalent allowlist in `get_pending_entities()` (line 928), and the 5-type filter wherever else it's mirrored (`count_pending_for_corpus`, `get_pending_entities_for_corpus`). The type allowlist is a contract, not an enum derived from one source. Find every occurrence of `('skill', 'tool', 'agent', 'problem_type', 'concept')` before shipping a new resolver type.
4. **Audit ticket-filing without code-read is the prohibited pattern.** Per `docs/solutions/best-practices/check-code-when-asked-about-code` family of disciplines: code claims need PRAGMA + grep + file:line, not "the function must be doing X." The /mika-groom-ticket pipeline catches this because grooming forces a code-read on the suspect file before writing the plan. If mika#997 had been dispatched directly to mika-dev (skipping grooming), the implementer would have wasted a session before catching the contract.

## When to Apply

Read this doc:

- **Before writing a custom DB query** that estimates resolver backlog. The query MUST mirror `count_pending()`'s 5-type filter, otherwise the result is meaningless.
- **Before filing a ticket** claiming the resolver is silently broken because `mika kg status` "pending" disagrees with `kg_resolver_tick.complete` `pending_before`. They're supposed to disagree by ~18k.
- **Before adding a 6th entity type** to `subject_extractor.rs`. Verify whether it should be resolvable (update all type-filter sites in `entity_resolver.rs`) or subject-graph-only (no resolver changes).
- **Before refactoring `count_pending()` or its callers** in `entity_resolver.rs` / `resolver_tick.rs`. The 5-type scope is load-bearing — removing the filter would re-introduce the very bug pattern this doc warns against (resolver attempting to canonicalize types with no domain projection).

## Examples

**2026-05-06 misread (the originating event for this doc).** Audit observed `pending_before: 0` on every tick + ~28k "pending" in `mika kg status`. Custom DB query without type filter returned 17,590 unresolved entities per agent. Filed mika#997 as a regression. Re-running the query *with* the 5-type filter returned 0 for every agent. Type distribution showed 18,077 entities of subject-graph-only types (`pattern: 6,762`, `failure_mode: 6,133`, `solution_path: 5,182`) accounting for the entire gap. mika#997 closed as invalid; the legitimate CLI-clarity concern split out as mika#999.

**Counter-example: a real regression would look different.** If a 6th resolvable type were added to `subject_extractor.rs` without updating `count_pending()`'s allowlist, the symptom would be:

- `kg_subject_entities` rows of the new type accumulating
- The mirroring query (5-type filter) showing 0 unresolved
- A NEW query (filter to the 6 types including the new one) showing thousands unresolved
- `kg_resolver_tick.complete` `pending_before: 0` despite the new-type backlog

That's the actionable diagnostic — if you can demonstrate a NEW type is being extracted but excluded from the resolver allowlist, file the regression. Don't file based on the unfiltered count alone.

## Related

- Sibling audit: `docs/solutions/best-practices/post-restart-kg-extraction-resolution-audit-2026-04-29.md` — Audits 1–4. This doc is **not** Audit 5 in the original sense (a sixth signal); it's a contract reference for interpreting Audits 1, 3 and Signals A–F correctly.
- Resolver tick design: `docs/solutions/kg/906-resolver-tick-periodic-drain-2026-04-30.md` — describes the tick whose `pending_before` field this doc explains.
- Per-corpus fairness: `docs/solutions/best-practices/kg-multi-corpus-per-agent-query-fan-out-2026-04-25.md` — #927's design, which I incorrectly flagged as regressed in the now-corrected mika#997.
- Resolver primitives: `docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md` — `kg_resolutions_log` table contract that `count_pending()` joins against.
- Closed-invalid ticket: `senara-solutions/mika#997` — preserved as a record of the misread for future audit-author reference.
- Live ticket: `senara-solutions/mika#999` — the legitimate CLI-clarity follow-up extracted from this audit.
- Code source of truth: `crates/mika-agent/src/kg/entity_resolver.rs:891-906` (the type allowlist), `crates/mika-agent/src/kg/subject_extractor.rs:43-90` (the full extraction type set), `crates/mika-agent/src/kg/resolver_tick.rs:78-158` (`tick_body` calling `count_pending`).
