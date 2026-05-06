---
ticket: mika#800
type: fix
title: Resolve shared-corpus extractor race via single-consumer KG topology
date: 2026-05-07
seq: 001
---

# Plan: Resolve shared-corpus extractor race (mika#800)

## Why

mika#800 documents a real cost-waste defect: per-agent extractor loops on shared corpora redundantly call the LLM for the same doc, and only the first writer's result survives the `kg_extractions` `INSERT OR IGNORE` dedup. Observed 14% over-count on the odds-engine 24-doc/3-agent case; for mika-arch sharing a single corpus across many agents the waste compounds linearly.

The ticket body proposes two coordination-layer fixes (claim-then-extract, single shared extractor) and after a 2026-05-06 read-path audit a third option (single-consumer topology) was added. This plan evaluates all three on equal footing and commits to a recommendation; the architect passes resolve whether the recommendation is sound or whether the original coordination path is the right move.

## Phase 0 — Pin verified state (source-anchored)

All facts verified against `~/.mika/data/mika.db` and `/var/log/mika/server.log` on 2026-05-07; code paths verified at branch HEAD `21d824b7`.

### Read-path utilization (last 7 days)

```
SELECT name, COUNT(*) FROM tool_calls WHERE created_at > datetime('now','-7 days') GROUP BY name ORDER BY COUNT(*) DESC;
```

- `query_knowledge_graph` — 53 calls. **100% from mika-arch.** Triggered by 3 bundled skills only:
  - `mika/skills/bundled/mika-arch-groom-ticket/system_prompt.md:25,62`
  - `mika/skills/bundled/mika-arch-groom-milestone/system_prompt.md:25,83`
  - `mika/skills/bundled/mika-arch-second-review/system_prompt.md:24,63`
- `search_memory` — 260 calls. Distributed across all agents (mika-arch 99, mika 53, mika-dev 39, others). FTS5+vec hybrid over `memory_facts`, not the KG.
- Other bundled skills (`self-dev`, `dev-pilot`, `qa-review`, `qa-review-build-callback`, `self-dev-webhook-{ci,qa}`, `permission-policy`, etc.): zero references to `query_knowledge_graph` in their `system_prompt.md` files.
- Community skills (`/data/workspace/mika-platform/mika-skills/`): zero KG references.

### Current corpus topology (`agent_kg_corpora`)

| docs_root_hash | docs_root | Agents (4 mika-family + 3 odds-engine = 7 enabled) |
|---|---|---|
| `34b8cf03c80614f9` | mika docs | mika, mika-arch, mika-dev, mika-qa |
| `98509090f0a833d2` | mika-skills docs | mika-arch (sole) |
| `ac0e96dc51b85b80` | mika-platform docs | mika-arch (sole) |
| `d7107cd14e544043` | mika-cloud docs | mika-arch (sole) |
| `62386bb31b9664e9` | odds-engine docs | odds-engine-{ceo,cto,quant} |

Five disabled agents: chase-hughes, elon-musk, mika-relay, mika-test, steve-jobs.

### Current resolution state on mika-docs corpus (shared by 4 agents)

Per `mika kg status` 2026-05-07:

| Agent | Subjects | Resolved | Pending (unfiltered) |
|---|---|---|---|
| mika | 30,301 | 2,367 | 27,934 |
| mika-arch | 30,301 | 2,236 | 28,065 |
| mika-dev | 30,301 | 2,172 | 28,129 |
| mika-qa | 30,301 | 2,249 | 28,052 |

Actionable backlog (5-type filter per `entity_resolver.rs:891-906`): **0** for every agent. The unfiltered/filtered gap is fully accounted for by subject-graph-only types (`pattern`+`failure_mode`+`solution_path`) — see `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md` for the contract.

The ~200-entity divergence in `resolved` counts across siblings on the same corpus reflects per-agent Stage-2 LLM judgment non-determinism. Per-agent resolution log is by design (`docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md`).

### mika-arch's secondary corpora (where shared-corpus race does not apply)

Single-consumer corpora — mika-arch alone holds them. No race to resolve here.

| Corpus | Chunks | Subjects | Resolved |
|---|---|---|---|
| mika-skills docs | 345 | 201 | 65 |
| mika-platform docs | 464 | 23 | 8 |
| mika-cloud docs | 199 | 10 | 0 |

Low-yield concern is mika#962 (separate ticket, not gated by mika#800).

### Current race exposure

mika#800's race fires on:
1. Server startup re-extraction (per-agent `tokio::spawn` per corpus)
2. Compound-hook reingest (`IngestionOrchestrator` triggers extraction in N agents simultaneously)

The race only matters on corpora with N>1 enabled agents. Today: only the mika-docs corpus (4 agents) and the odds-engine corpus (3 agents). mika-arch's other 3 corpora are sole-occupancy and have no race.

### Current `[kg]` config (the surface Option C edits)

```bash
$ grep -A3 '\[kg\]' ~/.mika/agents/{mika,mika-dev,mika-qa,mika-arch}/identity.toml
```

All four show `enabled = true` plus a `docs_root` line. Disabling on three of them is three single-line edits.

### Provisioning idempotency

`crates/mika-agent/src/well_known_agents.rs` provisions well-known agent identities at startup when `MIKA_DEV_MODE=true`. Per `crates/mika-agent/CLAUDE.md` § "Operating Mode": **"Idempotent — existing agents are never overwritten."** Direct identity.toml edits persist across restarts without `MIKA_DISABLE_AGENT_PROVISIONING=true`.

## Phase 1 — Option evaluation

The ticket body lists three options. Evaluating each against three axes — blast radius, reversibility, and how cleanly it disposes of the stated cost-waste problem.

### Option A — claim-then-extract

Schema migration (v32) adds `status TEXT NOT NULL DEFAULT 'complete' CHECK (status IN ('claimed', 'complete'))` to `kg_extractions`. Code changes in `subject_extractor.rs` insert claim row before the LLM call, update on completion, recover stale claims on startup.

- **Blast radius:** schema migration + extractor refactor + stale-claim recovery logic + tests. Estimated 200–400 lines of code, schema v32, two new SQL paths (claim insert, claim→complete update).
- **Reversibility:** schema migrations are sticky once shipped. Reverting requires a v33 to back it out. The code path becomes load-bearing for every shared-corpus extraction forever.
- **Disposes of mika#800:** yes, fully and durably. Adds a coordination primitive that scales to any future N-agent shared corpus.
- **Side effects:** stale-claim recovery introduces a new failure surface (orphaned claims on crash mid-extraction). `audit_events` and `llm_calls.agent_id` attribution stays per-agent (whichever agent won the claim).

### Option B — single shared extractor per corpus

Server maintains `HashMap<docs_root_hash, ExtractorHandle>`. Per-agent `tokio::spawn` model in `server/mod.rs` is reshaped so dispatch goes through the shared handle. Only one tokio task processes pending docs per corpus.

- **Blast radius:** structural change to `server/mod.rs` startup, new shared-handle abstraction in `subject_extractor.rs`, lifecycle coupling between agent spawn and handle lifetime. Estimated 400–800 lines plus careful concurrency review.
- **Reversibility:** larger refactor. Reverting requires undoing the shared-handle abstraction.
- **Disposes of mika#800:** yes, structurally cleaner than A — the race cannot exist because there is only one extractor per corpus.
- **Side effects:** per-agent metric attribution (`audit_events`, `llm_calls.agent_id`) becomes "first triggering agent wins" — semantically different from today's per-agent attribution. Affects observability dashboards and any per-agent cost reporting that consumes `llm_calls`.

### Option C — single-consumer topology

`[kg].enabled = false` in `~/.mika/agents/{mika,mika-dev,mika-qa}/identity.toml`. mika-arch keeps full KG. Restart mika-server.

- **Blast radius:** three single-line edits in three identity files. Zero code, zero schema. One restart.
- **Reversibility:** one config line per agent + one restart cycle. Shared-corpus markers stay alive while mika-arch holds them; re-enabling on a sibling rebuilds only that agent's resolution log, not the extraction layer.
- **Disposes of mika#800:** yes, by collapsing the input to the race rather than coordinating its participants. The mika-docs corpus stops being shared (only mika-arch consumes it). The odds-engine corpus keeps three consumers — but those agents do query the KG (read-path audit excluded them but they're a separate question from this fix; their three-agent topology is a product decision in scope of `senara-solutions/odds-engine`, not mika#800).
- **Side effects:** forecloses (until re-enabled) read-path expansion to mika-dev/mika-qa skills. Three orphaned per-agent resolution logs (~2,200 rows each) stay on disk; not FK violations (CASCADE on `agents` and `kg_subject_entities`), not deleted in scope of this PR.

### Recommendation

**Option C** for these specific reasons; the recommendation is contingent on architect validation of the assumptions in §Open Questions:

1. **The KG is mika-arch's tool by usage and by design.** The 53/7d call rate with 100% concentration on one agent isn't an adoption gap — it's the use case. milestone#14 retrospective and `crates/mika-agent/CLAUDE.md` already frame the KG as the architect-grooming substrate. mika-dev/mika-qa have a working retrieval path through `search_memory` that matches their decision shape (commitments/preferences/recent context), and zero current skill prompts ask them to query the graph.

2. **Topology fix dominates code fix on cost.** Option C ships in three lines and one restart. Options A and B ship a schema migration / structural refactor whose carrying cost (maintenance, observability impact, future migration overhead) recurs forever. With no live mika-dev/mika-qa demand for KG retrieval, the coordination primitive would be paid for compute that no one reads.

3. **Reversibility asymmetry favors C.** Disabling and re-enabling is config + restart. Shipping and ripping out a coordination primitive is two schema migrations or two refactors. If a future mika-dev or mika-qa flow surfaces that needs KG retrieval, re-enable for that flow specifically (one config edit, one restart cycle, re-extraction of that agent's resolution log only — extraction layer is preserved by mika-arch's continued ownership).

4. **The mika#800 problem statement is preserved.** The ticket says: per-agent extractor loops race on shared corpus, wasting LLM compute. C resolves that — collapses the race to a single consumer per corpus on the mika-docs corpus, eliminating the documented cost-waste pattern. The odds-engine corpus is out of scope.

### Open questions for architect evaluation (these gate the C recommendation)

The recommendation flips to A or B if any of these resolve in the direction marked.

1. **Latent mika-dev / mika-qa value.** Is there a near-term flow on the milestone backlog that wants doc-grounded retrieval (vs. memory_facts retrieval)? If yes — particularly for mika-dev's PR / dispatch / lineage workflows — the coordination primitive (A or B) is the right move because read-path parity is already on the roadmap, and Option C foreclosing that path would produce config-flip churn the moment the flow ships. Architect may have line-of-sight to milestone work this plan does not.

2. **77% no_match interpretation.** Resolution outcomes since 2026-05-01: 874 no_match / 234 matched_llm / 17 matched_exact. Is 77% no_match the expected long-tail of mentions with no domain projection (in which case Option C is fine) or a domain-graph coverage gap that would worsen if mika-dev/mika-qa demand-side pressure is removed (in which case the right move is to expand the domain graph, separate from this ticket, with KG kept enabled meanwhile via Option A/B)?

3. **Per-agent resolution divergence load-bearing?** ~200-entity spread across siblings on the same corpus. Today nothing reads multiple agents' resolution logs in aggregate. If a future cross-agent grooming flow does, Option C's collapse to single-agent resolution may matter. Mika-arch should know whether such a flow exists.

4. **odds-engine corpus.** mika#800 references the odds-engine 14% over-count. Option C does not touch odds-engine — the three odds-engine agents stay enabled. Do they actually read the KG? If yes, the odds-engine corpus's race remains and is in scope for a separate ticket on `senara-solutions/odds-engine`. If no, this is a parallel topology fix in that repo. Either way, the mika#800 resolution path is mika-platform/mika-side; odds-engine is downstream.

## Phase 2 — Implementation (assumes Option C selected)

If architect ratifies C, the implementation is:

### File edits

```
~/.mika/agents/mika/identity.toml          [kg].enabled = true → false
~/.mika/agents/mika-dev/identity.toml      [kg].enabled = true → false
~/.mika/agents/mika-qa/identity.toml       [kg].enabled = true → false
```

`docs_root` lines stay as-is (harmless when disabled, keeps diff minimal, trivially reversible). mika-arch's identity is unchanged.

### Server restart

```bash
sudo rc-service mika-server restart
```

OpenRC `supervise-daemon` per `/etc/init.d/mika-server` (chdir `/data/workspace/mika-platform/mika`, user `samidarko`, log `/var/log/mika/server.log`).

No `MIKA_KG_BATCH_BUDGET` adjustment needed — disabled agents skip extraction/resolution at startup; mika-arch's pending backlog is already 0.

### No purge in scope

Resolution logs for the three disabled agents become orphaned but stay on disk. `mika kg purge --agent <name>` would clear `kg_subject_resolutions` and `kg_resolutions_log` for the agent but is not required for the fix; cleanup is a follow-up if disk cost matters (~6,800 total rows across three agents, negligible).

### Documentation updates

Reconcile any "all four mika-family agents have KG enabled" claim with the new state. Likely sites (verify via `grep -rn 'mika-dev\|mika-qa' /data/workspace/mika-platform/mika/docs/ /data/workspace/mika-platform/mika/crates/mika-agent/CLAUDE.md` plus the workspace memory file):

- `mika/CLAUDE.md` — KG section, agent topology table.
- `crates/mika-agent/CLAUDE.md` — Knowledge Graph subsection, per-agent KG scoping.
- `mika-platform/memory/project_kg_v27_deploy_2026-04-25.md` — workspace memory, "Agent KG topology (as of 2026-04-25)" block.

If Option A or B is selected instead, the implementation plan in this section is replaced by the schema migration / refactor; this plan would need to be re-grooming or replaced.

## Phase 3 — Verification

Post-restart checks. All read-only.

1. **Topology check.**
   ```bash
   mika kg list-agents
   ```
   Expected: `enabled=false` on mika, mika-dev, mika-qa; `enabled=true` only on mika-arch (mika-family) and odds-engine-{ceo,cto,quant}. The mika-docs corpus row should list 1 agent (mika-arch), not 4.

2. **Status parity.**
   ```bash
   mika kg status --agent mika           # expect: no enabled corpora
   mika kg status --agent mika-arch       # expect: 4 corpora unchanged from pre-PR
   ```
   No purge happened, so mika-arch's chunks/subjects/resolved counts must be identical to pre-PR.

3. **Startup log.**
   ```bash
   grep kg_shared_docs_root /var/log/mika/server.log | tail
   ```
   After restart, mika-arch alone holds the mika-docs corpus. No 4-way share log line.

4. **No new extraction work for disabled agents.**
   ```bash
   grep '"event":"subject_extraction_start"' /var/log/mika/server.log \
     | grep -E '"agent_id":"(mika|mika-dev|mika-qa)"'
   ```
   Expected: zero new events post-restart for the three agents.

5. **No new resolver tick events for disabled agents.**
   ```bash
   grep '"event":"kg_resolver_tick.complete"' /var/log/mika/server.log \
     | tail -20 \
     | jq -r '.agent_id' \
     | sort -u
   ```
   Expected after one tick cycle (~30 min): only `mika-arch`, `odds-engine-ceo`, `odds-engine-cto`, `odds-engine-quant`. No `mika`, `mika-dev`, `mika-qa`.

6. **Validate clean.**
   ```bash
   mika kg validate
   ```
   Expected: 8/8 OK. Orphaned per-agent resolution logs are not FK violations — `kg_resolutions_log.agent_id` and `kg_resolutions_log.subject_entity_id` both reference still-alive parents (CASCADE on `agents` and `kg_subject_entities`).

7. **mika#800 closure.** PR description references `Closes #800`. Manual cross-check after merge: comment on mika#800 noting "resolved by single-agent KG topology — see PR #<n>."

## Phase 4 — Rollback

If verification fails or a regression surfaces:

```bash
# Re-enable on the affected agent(s)
sed -i 's/^enabled = false$/enabled = true/' ~/.mika/agents/<agent>/identity.toml
sudo rc-service mika-server restart
```

The agent's per-agent resolution log was preserved (no purge), so on restart it picks up where it left off. Extraction layer was preserved by mika-arch's continued ownership of the markers, so re-extraction on the re-enabled agent rebuilds only the per-agent resolution log — no LLM cost on the extraction side.

## Out of scope (explicitly)

- **mika#962** (extractor low-yield on mika-arch's secondary corpora — mika-cloud 10/199, mika-platform 23/464). Separate ticket. May benefit mechanically from siblings being detached on the mika-docs corpus (no shared-corpus race remaining), but this plan is not gated on or coupled to mika#962.
- **mika#999** (CLI `pending` counter conflation). Documentation/CLI clarity. Separate ticket.
- **Read-path expansion to dev/qa skills.** Explicitly deferred per peer review. The trigger condition for re-enable is documented but not in code: if a concrete mika-dev or mika-qa flow surfaces that wants doc-grounded retrieval, re-enable KG for that flow specifically rather than re-enabling broadly.
- **odds-engine corpus race.** Parallel concern in a different repo. Not in scope for mika#800's resolution.

## Related

- `crates/mika-agent/src/kg/subject_extractor.rs` — extractor entry path; Options A/B mutate this, Option C does not.
- `crates/mika-agent/src/kg/entity_resolver.rs:891-906` — type-allowlist contract for `count_pending`, relevant to the verification queries.
- `crates/mika-agent/CLAUDE.md` § Knowledge Graph — per-agent KG scoping (#778), idempotency (`mika kg list-agents` semantics).
- `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md` — counter-contract reference for verification step 6 if it ever surfaces a "pending" mismatch.
- `docs/solutions/best-practices/post-restart-kg-extraction-resolution-audit-2026-04-29.md` — Audits 1–4 + Signals A–F applied as the post-deploy check.
- mika#800 itself — ticket body now lists Options A/B/C; this plan recommends C, the architect evaluation gates the recommendation.
- mika#778, mika#795 — per-agent `[kg]` config introduction. Option C is the steady-state of the configuration surface those tickets shipped.
