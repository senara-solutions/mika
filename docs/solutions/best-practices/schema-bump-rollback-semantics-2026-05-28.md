---
module: database
tags: [schema, migration, rollback, check-constraint, enum, sqlite]
problem_type: best-practice
category: best-practices
date: 2026-05-28
---

# Schema-Bump Rollback Semantics

## The pattern

Schema CHECK-constraint expansion is asymmetric. The DDL (`ALTER TABLE` rebuild in SQLite) accepts new enum values immediately, and rows written with those values persist even if the binary is rolled back. A downgrade to the prior binary leaves rows in the database carrying values the old code has never seen. Every Rust consumer of the enum-string column — `match` arms, const lists, parsers, formatters, stats accumulators — must be audited for behavior on unknown values. Panic-on-unknown is a deployment hazard (the old binary crashes on startup or query). Silent-skip is a data-loss risk (rows vanish from aggregations without warning). Neither is acceptable as a default; the PR author must choose and document the forward-compatibility strategy for each consumer.

## Audit checklist

Before merging any PR that adds a value to a SQLite CHECK-constraint enumeration, run these five `rg` commands (substituting the actual column name and known values):

```bash
# 1. All Rust references to the column
rg '<column_name>' --type rust

# 2. All match arms / const lists for the enum's known values
rg 'matched_exact|matched_llm|no_match' --type rust  # substitute actual values

# 3. The DDL definition
rg 'CHECK.*<column_name>' crates/mika-agent/src/db/

# 4. Pattern-match consumers
rg '<column_name>.*=>' --type rust

# 5. String-to-enum parsers
rg 'from_str|parse.*<column_name>' --type rust
```

For each consumer found, apply the decision matrix:

| Consumer type | Forward-compat action | Example |
|---|---|---|
| `match` arm (exhaustive) | Add `_ => warn + skip` or `_ => error + default` | `ResolutionOutcomeStats` accumulator |
| Stats counter / accumulator | Add a catch-all bucket or `other` field | `kg_schema.rs` `ResolutionOutcomeStats` |
| String parser / `FromStr` | Return `Err`, not panic | Hypothetical `Outcome::from_str()` |
| Log formatter | Pass through unknown values verbatim | Tracing span attributes |
| API serializer (JSON) | Pass through — downstream clients must tolerate unknown strings | Dashboard DTOs |

## Worked example: mika#874 v29→v30 (`matched_llm_db_fallback`)

The v29→v30 migration (mika#874) adds `matched_llm_db_fallback` to the `kg_resolutions_log.outcome` CHECK constraint. The migration is a full table rebuild (SQLite has no `ALTER TABLE ... ALTER CONSTRAINT`). The DDL is at `crates/mika-agent/src/db.rs:3809`; the audit message at `db.rs:3845` reads `"v29→v30: expanded kg_resolutions_log outcome CHECK constraint (#874)"`.

**Consumers identified:**

- **`kg_schema.rs::ResolutionOutcomeStats`** — the stats struct has an explicit `matched_llm_db_fallback: u64` field that accumulates counts via a `match` on the outcome string. Rolling back to the v29 binary leaves rows with `outcome = 'matched_llm_db_fallback'` in the database. The v29 stats accumulator's `match` arm does not know this value:
  - If the match is exhaustive without a wildcard, it **panics** on the unknown string.
  - If it uses `_ => {}`, the count is **silently dropped** from stats output.
- The same pattern recurs at v34→v35 (mika#1154) with `no_candidate_of_type`.

**Key insight:** The migration DDL being correct (transaction-wrapped, preserves data, FK relationships, and indexes) is a **necessary but distinct** concern from the Rust consumers handling rollback gracefully. Both must be verified independently.

## Trigger rule

Any PR that adds a value to a SQLite CHECK-constraint enumeration **MUST** include a `## Rollback Semantics` section in the plan or PR description covering:

1. **Which consumers were audited** — with `rg` evidence (command + output summary)
2. **What behavior each consumer exhibits on the unknown value** — panic, skip, error, pass-through
3. **Whether forward-compat handling was added** or an operator runbook entry is needed for safe downgrade

## Anti-pattern: "mirroring the migration shape"

Copying a prior migration's DDL pattern (e.g., reusing v26→v27's table-rebuild template for v29→v30) is necessary but not sufficient. Migration correctness (transaction shape, FK rewiring, index recreation) and rollback semantics (consumer behavior on unknown enum values) are **distinct concerns**. A migration can be DDL-correct and still leave the old binary in a panic-on-unknown state.

The two concerns require separate verification:

| Concern | What to verify | Where the evidence lives |
|---|---|---|
| Migration correctness | Transaction wrapping, data preservation, FK/index recreation | The migration function in `db.rs` |
| Rollback semantics | Every Rust consumer tolerates unknown enum values | The `rg` audit in the `## Rollback Semantics` section |

Ref: mika#874 finding F8, mika#905.
