---
title: KG single-consumer topology eliminates shared-corpus extractor race
date: 2026-05-07
category: best-practices
module: kg
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Multiple agents share a docs_root corpus via matching docs_root_hash
  - Per-agent SubjectExtractor loops race on INSERT OR IGNORE dedup
  - Agents on the shared corpus have zero query_knowledge_graph usage
tags:
  - kg
  - shared-corpus
  - extractor-race
  - topology
  - well-known-agents
  - identity-toml
---

# KG single-consumer topology eliminates shared-corpus extractor race

## Context

In the v27 shared-corpus model (#786), multiple agents pointing at the same `docs_root` each run their own `SubjectExtractor::extract_pending(budget)` loop independently. The persistence layer deduplicates correctly via `kg_extractions`'s `INSERT OR IGNORE` — but the LLM call happens BEFORE the INSERT. Whichever agent persists the marker first wins; the others' compute (the actual LLM call to extract entities/relationships from the doc) is discarded.

Observed on a 24-doc/3-agent corpus: 28 `extraction_complete` events instead of the expected 24. The 4 over-count were duplicate races where two agents both ran the LLM extraction on the same doc seconds apart. This waste scales linearly with agent count per corpus.

A read-path audit (2026-05-06, `tool_calls` last 7 days) showed `query_knowledge_graph` was called 53 times, **100% from mika-arch** via three bundled grooming skills. mika-dev, mika-qa, and the base mika agent had zero KG query usage — their retrieval goes through `search_memory` (FTS5+vec over `memory_facts`).

## Guidance

When multiple agents share a corpus but only one reads the KG, disable KG for the non-consumers via `[kg].enabled = false` in their `identity.toml`. This collapses the shared-corpus race to a single consumer per corpus, eliminating wasted LLM compute without any code or schema changes.

For well-known agents provisioned via `well_known_agents.rs`, set `identity_source: Some(IdentitySource::Static(...))` with `[kg]\nenabled = false` in the identity template. This ensures new deployments get the correct default. Existing deployments need manual `identity.toml` edits (provisioning is idempotent — existing agents are never overwritten).

**Implementation for mika#800:**

```rust
// well_known_agents.rs — mika-dev identity
const MIKA_DEV_IDENTITY: &str = "\
name = \"Dev\"\n\
emoji = \"🛠\"\n\
\n\
[kg]\n\
enabled = false\n";
```

**Operational steps for existing deployments:**

```bash
# 1. Purge per-agent resolution logs (mika#802 SIGTERM race defense)
mika kg purge --agent mika-dev --yes
mika kg purge --agent mika-qa --yes

# 2. Edit identity.toml
sed -i 's/^enabled = true$/enabled = false/' ~/.mika/agents/mika-dev/identity.toml
sed -i 's/^enabled = true$/enabled = false/' ~/.mika/agents/mika-qa/identity.toml

# 3. Restart
sudo rc-service mika-server restart
```

## Why This Matters

Three options were evaluated for mika#800:

| Option | Approach | Blast radius | Reversibility |
|--------|----------|-------------|---------------|
| A | claim-then-extract coordination | Schema v32 + 200-400 LOC | Two schema migrations to revert |
| B | Single shared extractor per corpus | 400-800 LOC structural refactor | Large revert |
| **C** | Single-consumer topology | 3 config edits + restart | One config edit + restart |

Option C dominates when the KG is consumed by a single agent. The coordination primitives in A/B would be paid for compute that no one reads. If a future mika-dev or mika-qa flow needs KG-backed retrieval, re-enable with one `identity.toml` edit + restart — the extraction layer is preserved by the remaining consumer's ownership of the corpus markers.

## When to Apply

- **Use this pattern** when a shared-corpus race is observed AND the non-primary agents have zero `query_knowledge_graph` usage
- **Do NOT use this pattern** when multiple agents actively query the KG — use Option A (claim-then-extract) or Option B (single shared extractor) instead
- **Verify first** with a read-path audit: `SELECT agent_id, COUNT(*) FROM tool_calls WHERE name = 'query_knowledge_graph' AND created_at > datetime('now', '-7 days') GROUP BY agent_id`

## Examples

**Before (4 agents on mika-docs corpus):**
```
kg_shared_docs_root: agents=["mika","mika-arch","mika-dev","mika-qa"], count=4
→ 4× extraction loops racing on each doc
→ 14% wasted LLM calls on a 24-doc corpus
```

**After (1 agent on mika-docs corpus):**
```
kg_shared_docs_root: agents=["mika-arch"], count=1
→ 1 extraction loop, 0 races
→ 0% waste
```

**Verification commands:**
```bash
# Topology check
mika kg list-agents  # expect: enabled=false on mika, mika-dev, mika-qa

# No extraction work for disabled agents post-restart
grep '"event":"subject_extraction_start"' /var/log/mika/server.log \
  | grep -E '"agent_id":"(mika|mika-dev|mika-qa)"'
# expect: zero new events
```

## Related

- mika#800 — originating issue (per-agent extractor loops race on shared corpus)
- mika#802 — SIGTERM race on config changes (pre-restart purge workaround adopted)
- mika#786, #787 — v27 shared-corpus model that introduced the dedup-on-write path
- mika#778 — per-agent `[kg]` config introduction (`enabled` flag)
- `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md` — counter-contract reference
- `docs/solutions/best-practices/post-restart-kg-extraction-resolution-audit-2026-04-29.md` — post-deploy signals A–F
