# Plan: Extract schema-bump rollback semantics best-practices doc (#905)

## Context

mika#874 second-pass surfaced finding F8: every SQLite CHECK-constraint expansion that adds a new enum value carries rollback-class risk the migration-correctness story doesn't cover. If v(N) ships a new enum value, writes rows using it, then rolls back to v(N-1), the old binary's enum-string parser must either tolerate unknown values or operators must drain new writes before downgrade.

The pattern should be extracted as a best-practices doc so future schema bumps catch this without re-discovering it. The worked example is v29→v30's `matched_llm_db_fallback` and v34→v35's `no_candidate_of_type` additions to `kg_resolutions_log.outcome`.

## Changes

### Change 1: Create the best-practices doc

**File:** `docs/solutions/best-practices/schema-bump-rollback-semantics-2026-05-28.md` (new)

YAML frontmatter:
```yaml
---
module: database
tags: [schema, migration, rollback, check-constraint, enum, sqlite]
problem_type: best-practice
category: best-practices
date: 2026-05-28
---
```

Content structure (per ticket acceptance criteria):

1. **The pattern in one paragraph.** Schema CHECK expansion is asymmetric — the table accepts new values immediately via DDL, but downgrading to a prior binary leaves rows with the new value in place. The old binary's enum-string consumers (const lists, `match` arms, parsers, formatters, stats accumulators) must be audited for behavior on unknown values. Panic-on-unknown is a deployment hazard; silent-skip is a data-loss risk. The doc states this tradeoff explicitly.

2. **Reproducible audit checklist.** Five `rg` commands to surface all consumers of a given CHECK enum column:
   - `rg '<column_name>' --type rust` — find all Rust code referencing the column
   - `rg 'matched_exact|matched_llm|no_match' --type rust` — find all match arms / const lists for the enum's known values (substitute actual values)
   - `rg 'CHECK.*<column_name>' crates/mika-agent/src/db/` — find the DDL definition
   - `rg '<column_name>.*=>' --type rust` — find pattern-match consumers
   - `rg 'from_str|parse.*<column_name>' --type rust` — find string-to-enum parsers

   Decision matrix for each consumer:
   | Consumer type | Forward-compat action | Example |
   |---|---|---|
   | `match` arm (exhaustive) | Add `_ => warn + skip` or `_ => error + default` | `ResolutionOutcomeStats` accumulator |
   | Stats counter / accumulator | Add a catch-all bucket or `other` field | `kg_schema.rs` `ResolutionOutcomeStats` |
   | String parser / `FromStr` | Return `Err` not panic | Hypothetical `Outcome::from_str()` |
   | Log formatter | Pass through unknown values verbatim | Tracing span attributes |
   | API serializer (JSON) | Pass through — downstream clients must tolerate unknown strings | Dashboard DTOs |

   Naming convention for the runbook section in PR descriptions: `## Rollback Semantics` as a required H2 section.

3. **Worked example.** mika#874's v29→v30 migration adds `matched_llm_db_fallback` to `kg_resolutions_log.outcome` CHECK constraint. The migration is a table rebuild (SQLite has no `ALTER TABLE ... ALTER CONSTRAINT`). Consumers:
   - `kg_schema.rs::ResolutionOutcomeStats` — the stats struct has an explicit `matched_llm_db_fallback: u64` field. Rolling back to v29 binary leaves rows with `matched_llm_db_fallback` in the DB; the v29 stats accumulator's `match` arm doesn't know this value. If the match is exhaustive without a wildcard, it panics. If it uses `_ => {}`, the count is silently dropped from stats.
   - The migration DDL (table rebuild) is correct — it preserves data, FK relationships, and indexes. But the DDL being correct is a distinct concern from the Rust consumers handling rollback gracefully.

4. **Trigger rule.** Any PR that adds a value to a SQLite CHECK constraint enumeration MUST include a `## Rollback Semantics` section in the plan or PR description covering:
   - Which consumers were audited (with `rg` evidence)
   - What behavior each consumer exhibits on the unknown value
   - Whether forward-compat handling was added or an operator runbook entry is needed

5. **Anti-pattern callout.** "Mirroring the migration shape from a prior precedent" (e.g., copying v26→v27's table-rebuild pattern for v29→v30) is necessary but not sufficient. Migration correctness (transaction shape, FK rewiring, index recreation) and rollback semantics (consumer behavior on unknown enum values) are distinct concerns. A migration can be DDL-correct and still leave the old binary in a panic-on-unknown state.

### Change 2: Add reference to review-guide.md

**File:** `docs/architecture/review-guide.md`

Add a new bullet under **§ 4: KISS → What to flag** (after the existing schema-migration bullet at ~line 94):

```markdown
- **A CHECK-constraint enum expansion without rollback-semantics audit.** Any PR that adds a value to a SQLite CHECK enumeration must include a `## Rollback Semantics` section auditing all Rust consumers of the enum string. See `docs/solutions/best-practices/schema-bump-rollback-semantics-2026-05-28.md` for the checklist and worked example. Mirroring a prior migration's DDL shape is necessary but not sufficient — migration correctness and rollback semantics are distinct concerns (ref: mika#874 F8, mika#905).
```

## Acceptance Criteria

- `docs/solutions/best-practices/schema-bump-rollback-semantics-2026-05-28.md` exists with the five-section structure above
- YAML frontmatter includes `module`, `tags`, `problem_type: best-practice`, `category`
- Reference added to `docs/architecture/review-guide.md` § 4 as a flaggable pattern
- Worked example references mika#874 v29→v30 concretely with file paths and field names
- Anti-pattern callout distinguishes migration correctness from rollback semantics

## Out of Scope

- Refactoring `outcome` from bare string to typed Rust enum (separate SOLID ticket per mika#874 plan)
- Auditing existing enum-string consumers across the codebase preemptively (scoped to future schema-bump PRs per this doc's trigger rule)
- Changes to migration code or runtime behavior — this is a docs-only ticket
