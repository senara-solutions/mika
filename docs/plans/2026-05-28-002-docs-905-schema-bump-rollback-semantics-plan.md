# Plan: Extract schema-bump rollback semantics best-practices doc (#905)

## Context

mika#874 second-pass surfaced finding F8: every SQLite CHECK-constraint expansion that adds a new enum value carries rollback-class risk the migration-correctness story doesn't cover. If v(N) ships a new enum value, writes rows using it, then rolls back to v(N-1), the old binary's enum-string parser must either tolerate unknown values or operators must drain new writes before downgrade.

The pattern should be extracted as a best-practices doc so future schema bumps catch this without re-discovering it. The worked example is v29→v30's `matched_llm_db_fallback` and v34→v35's `no_candidate_of_type` additions to `kg_resolutions_log.outcome`.

## Phase 0 — Pin (verbatim source citations)

The #905 / #874 issue bodies and the v1 grooming-audit trail use the framing "v28→v29 + matched_llm_db_fallback". This was the original target; the migration ultimately shipped at **v29→v30** (and `no_candidate_of_type` at **v34→v35**). Plan version numbers come from the **migration source code**, not the older issue-body framing:

- `crates/mika-agent/src/db.rs:3782-3785` — `/// v29→v30: Expand kg_resolutions_log.outcome CHECK constraint to include 'matched_llm_db_fallback' (#874)`. DDL at `db.rs:3809`. Audit message `db.rs:3845` reads `"v29→v30: expanded kg_resolutions_log outcome CHECK constraint (#874)"`.
- `crates/mika-agent/src/db.rs:3980, 4044` — `/// v34→v35: ... 'no_candidate_of_type' (#1154)`. DDL at `db.rs:4007`. Audit message `db.rs:4044` reads `"v34→v35: expanded kg_resolutions_log outcome CHECK to include 'no_candidate_of_type' (#1154)"`.
- `CLAUDE.md` Database section reproduces both bumps with the same version numbers (v29→v30 #874; v34→v35 #1154).

If a future architect/reviewer cites "v28→v29" from issue body, **the source code is authoritative** — the issue body framing is stale relative to the migration that actually shipped.

**Stub-file note (architect F2):** A prior groom pass cited `docs/solutions/best-practices/schema-bump-rollback-class-stub-2026-04-30.md` as a stub that needed folding/deletion. Verification on `origin/main` at plan-write time: `git show origin/main:docs/solutions/best-practices/schema-bump-rollback-class-stub-2026-04-30.md` returns `fatal: path does not exist in 'origin/main'`. The stub was added once (commit `6ec2d3ee`) and removed before this work started; it no longer exists on the trunk. No fold/delete change is required by this plan.

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

Add a new bullet under **§ 4: KISS → What to flag**, immediately after the existing schema-migration bullet whose verbatim text begins:

> *"A new schema migration when an existing field would do. D2 in the mika-arch v1 plan proposed migration with a per-agent skill-allowlist table; it was rejected in favor of `Identity.skills.allowlist` with in-memory synthesis..."*

(Currently at `docs/architecture/review-guide.md:94`. The verbatim opening clause is the unambiguous anchor — line number may drift but the leading sentence won't.)

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
