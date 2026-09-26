---
title: "KG multi-corpus per-agent: query fan-out must use IN-lists, not first-element extraction"
date: 2026-04-25
category: best-practices
module: kg
problem_type: best_practice
component: database
severity: high
applies_when:
  - Adding multi-value support to a query path that previously accepted a single value
  - Extending a per-entity config field from Option<T> to Vec<T>
  - Wiring IN-list SQL predicates through async closure boundaries
tags:
  - knowledge-graph
  - multi-corpus
  - query-fan-out
  - in-list
  - docs-root-hash
  - sqlite
  - async-closure
---

# KG multi-corpus per-agent: query fan-out must use IN-lists, not first-element extraction

## Context

Issue #798 extended the KG system from a single `docs_root` per agent to multiple docs roots, enabling agents like `mika-arch` to reason across all six platform repos simultaneously. The ingestion, extraction, and resolution paths were correctly updated to iterate per-corpus. However, the query path (`kg/query.rs`) was left with a `.first()` extraction that silently reduced the multi-hash `Vec<String>` to a single hash — defeating the entire purpose of multi-corpus support at query time.

The gap was caught during code review (5 of 6 independent reviewers flagged it as P0). The root cause was an incremental migration strategy that deferred the query-path IN-list wiring to a "next step" — but that step was within the same ticket's scope (plan requirement R9).

## Guidance

When extending a single-value query parameter to accept multiple values:

1. **Change the internal function signatures end-to-end in the same PR.** Don't leave intermediate `.first()` extractions as "incremental migration" placeholders within the same feature scope. The ingestion and query paths must reach parity atomically.

2. **Use `&[String]` (slice) as the internal API, not `Option<&str>`.** Empty slice means "no filter" (equivalent to the old `None`). Non-empty slice means `WHERE column IN (?, ?, ...)`. This naturally handles both the single-value back-compat case (one-element slice) and the multi-value case.

3. **Follow the established IN-list pattern** from `entity_resolver.rs`:

```rust
// Build dynamic placeholders
let hash_placeholders: Vec<&str> = hashes.iter().map(|_| "?").collect();
let sql = format!(
    "SELECT ... FROM table WHERE docs_root_hash IN ({})",
    hash_placeholders.join(",")
);

// Build boxed params for params_from_iter
let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
for h in &hashes {
    params.push(Box::new(h.clone()));
}
// ... append other params ...

let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| { ... })?;
```

4. **Clone the hash vec before moving into `with_db` closures.** The `AsyncDatabase::with_db` takes a `move` closure that runs on the DB thread. The hash data must be owned inside the closure:

```rust
let hashes_owned = docs_root_hashes.to_vec(); // clone for move
db.with_db(move |db| {
    // hashes_owned is now owned by the closure
    let placeholders: Vec<&str> = hashes_owned.iter().map(|_| "?").collect();
    // ...
}).await?
```

5. **Guard against empty IN-lists.** SQLite rejects `WHERE x IN ()` (empty parentheses). When the hash slice is empty, either skip the WHERE clause entirely or short-circuit the function.

## Why This Matters

A `.first()` extraction on a multi-value input is a silent data loss bug — the query returns results but only from one corpus, with no error or warning. For the `mika-arch` use case (6 repos), this means 5/6 of the agent's knowledge base is invisible at query time. The ingestion cost is paid for all 6 corpora but only 1 is queryable.

This class of bug is particularly insidious because:
- All existing single-corpus tests pass (one-element vec behaves identically to the old singular path)
- The code compiles and runs without errors
- Results look plausible (they come from the first corpus, which may be the most important one)
- Only a multi-corpus integration test or production observation would catch it

## When to Apply

- Extending any `Option<T>` config or query parameter to `Vec<T>` where the downstream SQL uses equality (`= ?`) predicates
- Any time a "first-element extraction" appears as a migration placeholder within the same feature's scope
- When the entity_resolver already has the IN-list pattern and the query module needs to match

## Examples

**Before (broken — silently drops corpora 2-6):**

```rust
let docs_root_hash = if !input.docs_root_hashes.is_empty() {
    input.docs_root_hashes.first().map(|s| s.as_str())  // BUG: drops all but first
} else {
    input.docs_root_hash.as_deref()
};
// All downstream functions take Option<&str>
let entities = find_by_entity_key(db, key, docs_root_hash).await?;
```

**After (correct — full IN-list fan-out):**

```rust
let effective_hashes: Vec<String> = if !input.docs_root_hashes.is_empty() {
    input.docs_root_hashes.clone()
} else if let Some(ref h) = input.docs_root_hash {
    vec![h.clone()]
} else {
    vec![]  // empty = no filter
};
// All downstream functions take &[String]
let entities = find_by_entity_key(db, key, &effective_hashes).await?;
```

**Function signature change:**

```rust
// Before
async fn find_subject_entities_by_name(
    db: &AsyncDatabase, question: &str, docs_root_hash: &str,
) -> anyhow::Result<Vec<EntryEntity>>

// After
async fn find_subject_entities_by_name(
    db: &AsyncDatabase, question: &str, docs_root_hashes: &[String],
) -> anyhow::Result<Vec<EntryEntity>>
```

## Related

- `docs/solutions/best-practices/kg-per-agent-docs-root-config-isolation-2026-04-24.md` — predecessor: single-root per-agent config (#778)
- `docs/solutions/database-issues/kg-schema-v27-shared-corpus-docs-root-hash-2026-04-24.md` — v27 shared-corpus PK that enables multi-corpus
- `docs/solutions/best-practices/kg-lexical-ingestion-composed-write-2026-04-22.md` — per-corpus write contract
- `docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md` — entity resolver IN-list pattern (the correct reference implementation)
- Issue #798 — multi-corpus aggregation for mika-arch
- Plan: `docs/plans/2026-04-25-001-feat-kg-multi-corpus-per-agent-plan.md`
