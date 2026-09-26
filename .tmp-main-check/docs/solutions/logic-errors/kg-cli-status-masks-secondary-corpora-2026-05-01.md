---
title: "mika kg status masks secondary corpora for multi-corpus agents"
date: 2026-05-01
category: logic-errors
module: mika-cli
problem_type: logic_error
component: tooling
symptoms:
  - "`mika kg status --agent mika-arch` reports '1 unique corpora' despite 4 registered in agent_kg_corpora"
  - "Secondary corpora with ~0 resolutions invisible to operator without direct DB queries"
  - "Per-agent detail table shows single aggregated row for multi-corpus agents"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags:
  - kg
  - cli
  - multi-corpus
  - mika-arch
  - agent-kg-corpora
  - observability
---

# mika kg status masks secondary corpora for multi-corpus agents

## Problem

`mika kg status --agent mika-arch` displayed only the primary corpus despite mika-arch having 4 registered corpora in `agent_kg_corpora`. This masked that 3 secondary corpora had ~0 resolutions, preventing operators from diagnosing KG coverage gaps without running raw SQL against the database.

## Symptoms

- `mika kg status --agent mika-arch` output: `KG state summary (1 agents -- 1 unique corpora + 0 disabled)` — should show 4 corpora
- Per-agent detail table showed a single row with aggregated (summed) counts across all corpora, hiding per-corpus resolution deficits
- The summary line counted `agent_states.len()` as `total_agents`, which with per-corpus expansion would over-count agents

## What Didn't Work

- The original code was designed for single-corpus agents. When multi-corpus support shipped (#798), `build_agent_kg_state` was updated to aggregate counts across all corpora but still used `corpora.first()` for the display `docs_root` and `docs_root_hash`, masking the secondaries entirely.
- The corpus grouping logic used only the first corpus hash per agent to build the `CorpusGroup` map, so secondaries never appeared in the summary section.
- Resolution counts used `kg_count_rows("kg_subject_resolutions", "agent_id", agent_name)` which counts ALL resolutions for the agent regardless of corpus — no per-corpus breakdown was possible.

## Solution

Three coordinated changes:

1. **New DB method** — `Database::kg_count_resolved_for_corpus(agent_id, docs_root_hash)` joins `kg_subject_resolutions` through `kg_subject_entities` to scope resolution counts to a single corpus:

```rust
pub fn kg_count_resolved_for_corpus(&self, agent_id: &str, docs_root_hash: &str) -> Result<u64> {
    let count = self.conn.query_row(
        "SELECT COUNT(*) FROM kg_subject_resolutions sr \
         JOIN kg_subject_entities se ON sr.subject_entity_id = se.id \
         WHERE sr.agent_id = ?1 AND se.docs_root_hash = ?2",
        params![agent_id, docs_root_hash],
        |r| r.get::<_, i64>(0),
    )?;
    Ok(count as u64)
}
```

2. **Per-corpus state generation** — `build_agent_kg_state` renamed to `build_agent_kg_states`, now returns `Vec<AgentKgState>` with one entry per corpus (via `.iter().map()`), each with its own `chunks`, `subjects`, `resolved`, `pending`, and `last_extraction`.

3. **Display deduplication** — The text formatter tracks `prev_agent` and shows the agent name/enabled flag only on the first row. Summary line uses `HashSet<String>` for unique agent counting instead of `agent_states.len()` (which now counts corpus-rows, not agents).

## Why This Works

The root cause was a single-corpus assumption baked into `AgentKgState` — it held one `docs_root` and one `docs_root_hash` (always from `corpora.first()`). The fix breaks this 1:1 mapping by emitting one state entry per corpus, which naturally flows through the existing display pipeline. The per-corpus resolution count requires a JOIN because `kg_subject_resolutions` is keyed by `agent_id` while corpus scoping lives on `kg_subject_entities.docs_root_hash`.

## Prevention

- When adding multi-value support to a data model (e.g., single `docs_root` to `docs_roots` array), audit all display surfaces that consume the model — not just the data aggregation path.
- The diagnostic SQL from the issue body provides a ready-made per-corpus health check that should be equivalent to what the CLI shows:

```sql
SELECT akc.docs_root_path,
       (SELECT COUNT(*) FROM kg_chunks WHERE docs_root_hash=akc.docs_root_hash) AS chunks,
       (SELECT COUNT(*) FROM kg_subject_entities WHERE docs_root_hash=akc.docs_root_hash) AS subjects,
       (SELECT COUNT(*) FROM kg_subject_resolutions sr
          JOIN kg_subject_entities se ON sr.subject_entity_id = se.id
          WHERE sr.agent_id=akc.agent_id AND se.docs_root_hash=akc.docs_root_hash) AS resolved
FROM agent_kg_corpora akc
WHERE akc.agent_id='mika-arch';
```

## Related Issues

- mika#877 — This issue
- mika#798 — Multi-corpus aggregation primitive (`agent_kg_corpora` table)
- mika#874 — Stage-2 resolver candidate-list fix (upstream dependency)
- mika#876 — Subject extractor parse-tolerance fix (upstream dependency)
- mika#906 — Resolver tick (30-min periodic drain)
- Milestone #19 — KG flawlessness
