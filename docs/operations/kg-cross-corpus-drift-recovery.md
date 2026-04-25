---
title: KG cross-corpus resolution drift — recovery procedure
type: runbook
component: knowledge-graph
related_issues: [778, 795, 796, 798, 802]
created: 2026-04-25
---

# Cross-corpus resolution drift recovery

## Symptom

`mika kg status` shows `[DRIFT]` next to one or more agents. KG queries from those agents return wrong-corpus answers (e.g., an `odds-engine-*` agent surfaces mika-platform engineering content).

Diagnostic query — confirms the failure mode:

```sql
SELECT docs_root_hash, COUNT(DISTINCT agent_id) AS agents, COUNT(*) AS resolutions
FROM kg_subject_resolutions ksr
JOIN kg_subject_entities kse ON kse.id = ksr.subject_entity_id
GROUP BY docs_root_hash;
```

If an agent's `kg_subject_resolutions` rows reference `kg_subject_entities` under a `docs_root_hash` that does NOT match the agent's currently-configured `[kg].docs_root` in `~/.mika/agents/<name>/identity.toml`, the agent is drifted.

## Root cause

In-flight resolution tasks under config A (e.g., the global pre-#778 docs_root) persist `kg_subject_resolutions` rows seconds before the supervisor SIGTERMs the OLD binary. The NEW binary inherits those rows and treats them as "already resolved" because the resolver's pending-detection scopes by `(agent_id, subject_entity_id)` without joining on `docs_root_hash`. Tracked in #802.

## Recovery

For each drifted agent:

```bash
mika kg purge --agent <name> --yes
```

`mika kg purge` without `--include-orphaned-corpus` deletes ONLY `kg_subject_resolutions` and `kg_resolutions_log` rows for that agent. The shared-corpus tables (`kg_chunks`, `kg_subject_entities`, `kg_subject_relationships`, `kg_extractions`) are preserved — these contain the expensive LLM extraction work that should NOT be redone.

After purging all drifted agents:

```bash
mika kg validate                # confirm 8/8 OK
sudo rc-service mika-server restart
```

The next restart's `resolution_pending_start` event fires for each enabled agent. With zero `kg_resolutions_log` entries, the resolver re-resolves all `kg_subject_entities` under the agent's now-correct `docs_root_hash` from scratch.

## Cost expectation

Per restart, `MIKA_KG_BATCH_BUDGET=500` caps Stage-2 LLM calls per agent. Stage-1 exact-match misses record `no_match` directly without falling through to Stage-2 (verified empirically 2026-04-25 — 818 entities resolved as `no_match` per agent without LLM cost). Net cost is bounded by:
- Number of entities whose type is in `KG_DOMAIN_ENTITY_TYPES` (skill/tool/agent/problem_type) AND whose `entity_key` matches a domain entity at low extraction confidence — these escalate to Stage-2.
- Discovered types (solution_path/failure_mode/pattern) skip resolution entirely (zero cost).

Realistic recovery cost: <$5 per drift event for typical corpus sizes.

## Verification

After restart, re-run the diagnostic query. Healthy state:

```
docs_root_hash      | agents | resolutions
34b8cf03c80614f9    |      3 |       1,396
62386bb31b9664e9    |      3 |         818
```

Each agent's hash matches their `identity.toml` `[kg].docs_root` resolution.

## When this can recur

Any deploy that changes per-agent KG config — `[kg].enabled`, `[kg].docs_root` — exposes the SIGTERM race. Will be eliminated structurally when #802 lands (CancellationToken plumbing for resolver/extractor background tasks). Until then, treat as a known post-deploy check after any per-agent KG config change.

## Historical context

First observed 2026-04-25 during deploy of #795/#796. 3 odds-engine agents drifted from mika hash to odds-engine hash; 5 disabled agents had pre-existing per-agent rows from before #778 landed. Recovery executed via 8 sequential `mika kg purge --agent <name> --yes` invocations + restart. Validate stayed clean throughout. Total cost ≈ $5 in re-resolution.
