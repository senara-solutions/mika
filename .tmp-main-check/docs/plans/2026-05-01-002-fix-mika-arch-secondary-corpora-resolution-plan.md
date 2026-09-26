---
title: "fix(kg): mika-arch secondary corpora resolution + CLI status surface"
type: fix
status: active
date: 2026-05-01
---

# fix(kg): mika-arch secondary corpora resolution + CLI status surface

## Problem Frame

mika-arch is registered with 4 corpora in `agent_kg_corpora` (mika, mika-skills, mika-platform, mika-cloud), but only the primary (mika) corpus has meaningful resolution coverage. The 3 secondary corpora are chunked but produce ~0 resolved entities, making graph traversals into cross-repo docs (e.g., `mika-platform/docs/solutions/cross-repo-patterns/`) effectively broken.

Verified state from issue body (2026-04-29 against `~/.mika/data/mika.db`, post-restart, post-merge of #872):

| Corpus | Chunks | Subjects extracted | Resolved |
|---|---:|---:|---:|
| `mika/docs/solutions` (primary) | 2,845 | 30,164 | 8,082 |
| `mika-skills/docs/solutions` | 345 | 164 | 4 |
| `mika-platform/docs/solutions` | 374 | 63 | 0 |
| `mika-cloud/docs/solutions` | 199 | 65 | 0 |

Two distinct surfaces are broken:

1. **Subject extraction asymmetry** — secondaries yielded 63–164 subjects vs. 30,164 on the primary. This is a 200×–500× disparity, far larger than the chunk-count ratio (~7×–14×). Likely downstream of mika#876 (subject_extractor returning 0 entities/relationships on malformed-JSON batches) hitting the secondaries hardest.
2. **Resolution to ~zero on secondaries** — even the small subject pool isn't resolving. This is downstream of mika#874 (Stage-2 resolver candidate-list rejection) AND mika#875 (Stage-1 exact-match returns 0 — already CLOSED) — together they form a resolver-stack pile-up that left secondaries at 0 while the primary built up resolutions over time.
3. **CLI display masks the problem** — `mika kg status --agent mika-arch` reports only the primary corpus, surfacing `KG state summary (1 agents — 1 unique corpora + 0 disabled)`. Operator can't see that 3 corpora are stalled until they query the DB directly.

The issue body's acceptance is explicit: this ticket lands AFTER mika#874 and mika#876 ship, then runs the verify-and-fix cycle on mika-arch's corpora.

## Plan Contract

**Forward groom** (not retroactive). /ce:work dispatch will run normally once milestone#19 launches the workflow on this issue. Implementation Units below carry `[ ]` open markers; mika-dev's claude-pilot session will execute them per /ce:work.

**Cross-ticket sequencing inside milestone#19:**

```
mika#876 (subject_extractor parse-tolerance) ─┐
                                              ├─► mika#877 (this ticket — re-extract + re-resolve + CLI display)
mika#874 (Stage-2 resolver candidate-list)  ─┘
```

mika#877 is the verification + cleanup ticket. It cannot land until both upstream resolver/extractor fixes (#874, #876) are on main, because Unit 3 (re-extract + re-resolve mika-arch corpora) requires the resolver and extractor to be functionally correct first. mika#875 (Stage-1 exact-match) shipped earlier and is already on main — closed in milestone#19.

The issue body should carry an explicit `blockedBy: mika#874, mika#876` GitHub edge once both are GROOMED, so the milestone-workflow's `resolve_issue_order` tool sequences correctly.

## Requirements Trace

- **R1.** Verify the operator's stated cause hypothesis empirically post-#874+#876 deploy: re-run the issue body's diagnostic SQL and confirm secondary-corpus subject extraction recovers (driven by #876's parse-tolerance fix) and resolution coverage recovers (driven by #874's Stage-2 candidate-list fix + #875's already-shipped Stage-1 fix). The hypothesis is named verbatim in the issue body: *"Likely #876 (extraction returning 0 entities) hit these batches hardest"* and *"Resolution then produced 0–4 matches on the secondaries (downstream of #874 + #875 pile-up)"*. R1's deliverable is the verification, not a multi-candidate investigation.
- **R2.** Trigger backfill cycle on mika-arch's secondary corpora after #874+#876 deploy so resolution coverage reaches operating parity. Preferred path: rely on mika#906's resolver tick + mika#757's extraction idempotency for natural drain. Fallback: SQL invalidation of `kg_extractions.source_doc_hash` to force re-fire.
- **R3a (resolver-throughput verification).** `resolved/attempted > 50%` for `primary` (mika) and `mika-skills` corpora — the corpora where domain-graph coverage is adequate, so resolver throughput is the bottleneck.
- **R3b (recovery-from-baseline verification).** `attempted > 0` for `mika-platform` and `mika-cloud` corpora — proves #874+#876 fixes lifted these from baseline 0/0 (pre-fix) to non-zero attempts. Absolute resolved/attempted threshold for these two corpora is bounded by domain-graph coverage gaps and is deferred per **§ Known Limitations** below.

  > **Plan amendment 2026-05-01 (post-PR-#926 mika-qa block[ac] response):** R3 was originally written as a single `resolved/total > 50% for all 4 corpora` threshold. Post-deploy verification surfaced that two of the four corpora (mika-platform, mika-cloud) are limited by domain-graph coverage (cross-repo workflow concepts; Helm/K8s infrastructure concepts), not by resolver throughput. The original R3 conflated *resolver-throughput recovery* (universally achievable post-#874+#876) with *absolute coverage threshold* (corpus-dependent on domain graph). R3 has been decomposed into R3a (throughput) and R3b (recovery-from-baseline) to surface the two distinct invariants. The throughput-and-coverage threshold lift for mika-platform + mika-cloud is the absolute-rate work captured in the follow-up tickets cited below.
- **R4.** `mika kg status --agent mika-arch` surfaces all 4 corpora rows, not just the primary. CLI display fix in `crates/mika-cli/` or `crates/mika-common/src/cli/` (per issue body's "Files to inspect").
- **R5.** Spot-check: graph traversal from a known entity in `mika-platform/docs/solutions/cross-repo-patterns/` returns related entities — proves cross-corpus reachability is restored.

## Scope Boundaries

- This ticket does NOT fix #874 or #876. Those are independent milestone#19 sub-issues with their own plans.
- This ticket does NOT address mika#800 (per-agent extractor loop race on shared corpus). mika#800's body characterizes it as *"no data loss, purely a cost optimization"* — the race produces duplicate compute, not missing data. Therefore mika#800 cannot be the cause of secondary-corpus zero-resolution; category error to enumerate it as an alternative cause. Out of scope.
- No multi-corpus iteration bug hunt. The issue body's data table empirically disconfirms a per-agent-first-corpus iteration bug: mika-skills shows 4 resolved (non-zero), so the resolver IS visiting that secondary corpus. If iteration skipped secondaries entirely, mika-skills would be 0. The deficit shape (low non-zero across secondaries) is consistent with the upstream extraction/resolution defects, not an iteration bug.
- No new corpora added; no changes to which agents use which corpora.
- No changes to chunking logic — the chunk counts in the actual-behavior table are healthy (chunking ran on all 4 corpora successfully).

### Deferred to Separate Tasks

- **Backfill orchestration tooling** if a recurring multi-restart resolver-tick (mika#906 already landed) isn't sufficient to drain the secondary-corpus backlog within the SLA implied by R3a. (R3b is a recovery-from-baseline check, not a throughput-bound check — backfill orchestration doesn't apply.)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/kg/subject_extractor.rs` — per-agent extraction loop. The `extract_pending(budget: u32)` entry point caps per-batch LLM calls. Need to verify it iterates over ALL `agent_kg_corpora.docs_root_hash` rows for the agent, not just the first.
- `crates/mika-agent/src/kg/entity_resolver.rs` — per-agent resolver. `resolve_pending(budget: u32)` similarly needs to iterate all corpora.
- `crates/mika-agent/src/kg/resolver_tick.rs` (mika#906, deployed today) — periodic 30-min resolver tick. If the secondary corpora are simply waiting for the tick to drain a backlog, that's a wait-not-fix scenario; R1's verification confirms whether tick coverage includes secondaries via `kg_resolver_tick.complete` log events keyed by `docs_root_hash`.
- `crates/mika-cli/src/...` (or `crates/mika-common/src/cli/`) — `mika kg status` formatter. Issue body cites `KG state summary (1 agents — 1 unique corpora + 0 disabled)` as the broken output line. Need to inspect the query that builds this summary — likely SELECTs distinct `docs_root_hash` per agent but the JOIN drops secondaries silently.

### Institutional Learnings

- mika#798 (CLOSED) shipped the multi-corpus aggregation primitive (`agent_kg_corpora` table, `[kg].docs_roots` plural in identity.toml). #877 picks up the runtime/CLI surface around that primitive that wasn't covered.
- mika#874 (p0-critical, milestone#19 sibling, awaiting Vincent's verdict relay) — Stage-2 resolver candidate-list check rejects valid LLM matches. Direct upstream cause of secondary-corpus 0-resolution rates.
- mika#875 (CLOSED) — Stage-1 exact-match returned 0 across all batches. Already shipped; secondary corpora may have backlog from when this was broken.
- mika#876 (GROOMED, milestone#19 sibling) — subject_extractor returns 0 entities/relationships on malformed-JSON batches. Upstream cause of secondary-corpus subject-count deficit.
- mika#906 (deployed today) — periodic resolver tick (30-min intervals) decouples drain rate from restart cadence. After R2's re-extract triggers, the tick should drain the secondary-corpus pending pool over ~17–18 hours per the post-restart safety check § Signal E.
- `docs/architecture/kg-id-convention.md` and `docs/architecture/kg-implementation-conventions.md` — sole-writer contracts for KG tables. Stay within `kg::subject_extractor` and `kg::entity_resolver` write surfaces; don't add a new writer.
- `docs/solutions/database-issues/kg-v27-stuck-migration-recovery-2026-04-24.md` — historical context on the multi-corpus migration; not directly applicable but informs the operational shape of corpus-level state.

### External References

- None applicable — fix is purely internal to mika-agent's KG subsystem and the mika-cli status command.

## Key Technical Decisions

- **Backfill via natural drain mechanisms, not a one-shot migration.** mika#906's resolver tick (30-min, 500-budget per agent) is the canonical drain mechanism. Trigger re-extraction in mika-arch's secondary corpora by inserting marker rows or invalidating the extraction-idempotency hash, then let the tick drain. Avoid a custom one-shot script — the tick is the durable path. If the tick can't reach steady state within the post-restart SLA (Signal E), THAT is a tick-coverage bug worth filing separately, not a reason to bypass the tick here.
- **CLI display fix is a self-contained sub-unit.** Independent of the runtime fix; can ship as a separate commit on the same branch. Doesn't need to wait for R2's drain to complete — surfacing the secondary corpora rows (even at low coverage) is itself an operator-readiness improvement.
- **R1 is verification, not investigation.** Issue body's stated cause hypothesis is anchored verbatim ("Likely #876 hit these batches hardest" + "downstream of #874+#875 pile-up"). R1's deliverable is empirical confirmation post-#874+#876 deploy that resolved/total > 50% per secondary corpus, not a multi-cause investigation. Per architect persisted preference `verification_vs_investigation_unit_framing`.

## Open Questions

### Resolved During Planning

- **Is mika#875 still open?** No — CLOSED, already shipped on main. Acceptance criterion's reference to "after #874, #875, #876 land" simplifies to "after #874, #876 land."
- **Does this ticket need to wait for both #874 and #876?** Yes for R1's verification post-deploy and R2's backfill cycle. The CLI display fix (R4) is independent of all upstream gates and can ship as a separate commit on the branch.

### Deferred to Implementation

- **Will the resolver tick drain naturally, or does a forced trigger help?** Implementation may add a `mika kg backfill --agent mika-arch --corpus <name>` CLI subcommand if the tick alone is insufficient to hit R3a's > 50% threshold within Signal E's window. Decision deferred to implementation; the plan accepts either outcome (preferred natural drain, fallback SQL invalidation). (R3b is recovery-from-baseline only and doesn't depend on backfill mechanics.)

## Implementation Units

- [ ] **Unit 1: Verify operator's stated cause hypothesis post-#874+#876 deploy**

  **Goal:** Re-run the issue body's diagnostic SQL after #874 and #876 ship. Confirm the predicted recovery: secondary-corpus subject extraction climbs (driven by #876's parse-tolerance fix) and resolution coverage climbs (driven by #874's Stage-2 fix + #875's already-shipped Stage-1 fix). Document the post-deploy snapshot for the closing comment.

  **Requirements:** R1

  **Dependencies:** mika#874 and mika#876 must have merged before this unit runs. No code changes.

  **Files:**
  - Read: `~/.mika/data/mika.db` (run the issue body's diagnostic SQL)
  - Read: `~/.mika/agents/mika-arch/logs/mika.log.YYYY-MM-DD` (post-deploy `subject_extraction_*` and `kg_resolver_tick.*` events keyed by `docs_root_hash`)
  - Modify (output): post-deploy snapshot table inline in the closing comment + this plan's verification section

  **Approach:**
  - After #874+#876 deploy, wait one full resolver-tick cycle (~30 min) so the natural drain mechanism runs.
  - Re-execute the issue body's Steps to Reproduce SQL. Compare the post-deploy resolved/total ratios to the 2026-04-29 baseline.
  - Grep `kg_resolver_tick.complete` log events post-deploy: confirm `pending_after` trends toward 0 across hourly windows for mika-arch's 4 corpora (matches mika#906 post-restart safety check Signal E).
  - Pin the operator's stated cause hypothesis verbatim in the verification commit message: *"Recovery driven by #876's parse-tolerance fix (extraction) + #874's Stage-2 candidate-list fix (resolution). #875's Stage-1 fix already in place."*

  **Patterns to follow:**
  - `mika#906` post-restart safety check Signals A–E shape — same diagnostic discipline (cite signal, expected trend, observed value).

  **Test scenarios:**
  - Test expectation: none — empirical verification unit, no behavioral change in code. Output is the documented snapshot.

  **Verification:**
  - **Pass threshold:** R3a — `resolved/attempted` exceeds 50% on coverage-adequate corpora (primary + mika-skills). R3b — non-zero `attempted` on coverage-limited corpora (mika-platform, mika-cloud), proving #874+#876 lifted them from baseline 0/0. If the resolver tick has run for ~17–18 hours post-deploy and R3a's ratio is still below 50% on a coverage-adequate corpus, R1's verification fails and Unit 2's fallback path (SQL invalidation) becomes the operating mechanism. The architect's second-pass review reads the post-deploy SQL output against these explicit thresholds.
  - Unit 2's backfill design adapts based on whether natural drain is sufficient or fallback SQL invalidation is needed.

- [ ] **Unit 2: Trigger backfill cycle on mika-arch secondary corpora**

  **Goal:** Drive mika-arch's secondary corpora to (a) R3a SLA (>50% resolved/attempted on coverage-adequate corpora — primary + mika-skills) and (b) R3b SLA (non-zero attempted on coverage-limited corpora — mika-platform + mika-cloud) by triggering re-extraction + re-resolution after #874+#876 deploy.

  **Requirements:** R2, R3a, R3b, R5

  **Dependencies:** Unit 1's verification snapshot. **Also depends on mika#874 and mika#876 having merged** — re-extracting before those ship would reproduce the same deficits.

  **Files:**
  - Modify (potentially): `crates/mika-agent/src/kg/subject_extractor.rs` — if a forced-re-extract entry point is needed (e.g., invalidating idempotency markers per `docs_root_hash`).
  - Operational: SQL invalidation of `kg_extractions.source_doc_hash` for the secondary corpora's docs (sets pending state).

  **Approach:**
  - Preferred: rely on the resolver tick (mika#906) and extraction idempotency to naturally re-fire after #876 deploys with the parse fix. If the source doc hashes change (because doc content changed, even subtly), extraction re-runs.
  - Fallback: invalidate `kg_extractions.source_doc_hash` for secondary corpora — sets pending state, next tick batch processes them. Operational change, no code change. Document the SQL in `docs/runtime-structure.md` as a recovery pattern.
  - If neither natural-drain nor SQL-invalidation hits the R3a SLA on coverage-adequate corpora within Signal E's 17–18 hour steady state, the throughput gap is the *attempt-rate* problem (per-corpus fairness in `get_pending_entities()`) — see mika#927 in §Known Limitations.
  - Validate empirically: re-run the issue body's Steps to Reproduce SQL post-backfill; capture the output table for the closing comment.

  **Patterns to follow:**
  - mika#906's resolver tick + mika#757's extraction idempotency are the canonical drain mechanisms.

  **Test scenarios:**
  - Integration: against a fixture DB seeded with mika-arch + 4 corpora where 3 secondaries have pending extraction state, run the tick once and assert all 4 corpora are visited and at least one entity is extracted per corpus.
  - Operational: run the SQL invalidation against a real `~/.mika/data/mika.db` (post-#874/#876 deploy), wait one tick cycle, re-query — secondary subject counts should increase.
  - Acceptance check: the actual-behavior table from the issue body, re-captured. R3a — resolved/attempted > 50% on primary + mika-skills (coverage-adequate corpora). R3b — non-zero attempted on mika-platform + mika-cloud. Coverage-limited absolute thresholds are §Known Limitations (deferred to mika#928). Capture the post-backfill table in the closing comment.

  **Verification:**
  - The actual-behavior SQL re-run shows resolved/total > 50% on all 4 corpora.
  - mika-arch can graph-traverse from `mika-platform/docs/solutions/cross-repo-patterns/` and reach related entities (R5 spot-check).

- [ ] **Unit 3: CLI status display surfaces all corpora rows**

  **Goal:** `mika kg status --agent mika-arch` lists every registered corpus with its own row. Currently displays only the primary (per the issue body's "1 agents — 1 unique corpora + 0 disabled" line).

  **Requirements:** R4

  **Dependencies:** None — independent of all other units. Can ship as a separate commit on this branch even before Unit 2's drain finishes.

  **Files:**
  - Modify: `crates/mika-cli/` or `crates/mika-common/src/cli/` (the formatter that emits the `KG state summary` line — issue body cites this surface).
  - Test: inline `#[cfg(test)] mod tests` exercising the formatter against a fixture with 4 corpora.

  **Approach:**
  - Locate the `kg status` query and formatter. Audit the SQL: it likely SELECTs distinct corpora per agent but the JOIN or aggregation drops secondaries (the chunk counts ARE non-zero in the issue body's table — extraction is happening, the display just doesn't show it).
  - Adjust the query/formatter to emit one row per `agent_kg_corpora` row, including `docs_root_path`, chunks, subjects, resolved counts.
  - Match the existing display style (table, fixed-width columns, summary line at top).

  **Patterns to follow:**
  - Existing `mika kg status` output format. Match column widths and labels.
  - SQL pattern from the issue body's Steps to Reproduce — that's already a verified-correct query for surfacing per-corpus state.

  **Test scenarios:**
  - Happy path: agent with 4 corpora, each with non-zero chunks; assert formatter emits 4 rows.
  - Edge case: agent with 1 corpus; assert formatter emits 1 row (no regression).
  - Edge case: agent with 0 corpora (well-known agent disabled?); assert graceful empty-state message.
  - Edge case: corpus with NULL `docs_root_path` (shouldn't happen in normal operation, but defensive); assert renders without panicking.

  **Verification:**
  - `cargo test -p mika-cli` (or wherever the formatter lives) passes.
  - Manual smoke: `cargo run --bin mika -- kg status --agent mika-arch` shows 4 rows on a real DB.

## System-Wide Impact

- **Interaction graph:** Re-extraction in Unit 2 fans out across mika-arch's 4 corpora; the existing per-agent locking pattern is preserved. CLI display change in Unit 3 is read-only and additive.
- **Error propagation:** No new error paths. Per C2.3 log-and-skip, transient extraction errors on secondaries don't abort the loop.
- **State lifecycle risks:** Re-extraction overwrites `kg_subject_entities` rows for the secondary corpora; preserve the sole-writer contract per `docs/architecture/kg-implementation-conventions.md`. UPSERT semantics already handle re-runs idempotently.
- **API surface parity:** None — internal subsystem.
- **Integration coverage:** Unit 3's empirical re-verification (the SQL re-run) is the cross-layer integration check.
- **Unchanged invariants:** Sole-writer contracts on KG tables, mika#906's tick budget, mika#757's extraction idempotency keying, the `docs_root_hash` PK on shared-corpus tables.

## Known Limitations

mika-platform and mika-cloud corpora's `resolved/attempted` ratios are bounded by **domain-graph coverage** for cross-repo workflow concepts and Helm/K8s infrastructure concepts respectively. Absolute throughput threshold for these two corpora is deferred to the follow-up tickets below.

- **mika#927** — `fix(kg/resolver): per-corpus fairness in get_pending_entities() — primary corpus backlog starves secondaries`. p1-important. Addresses the *attempt-rate* gap: secondaries are starved of resolver budget while primary's 17,538-entity backlog drains. Round-robin allocation across corpora within a single `resolve_pending` call.

- **mika#928** — `feat(kg/domain-graph): expand domain graph to cover cross-repo workflow + Helm/K8s infrastructure concepts (mika-platform + mika-cloud corpora)`. p2-normal. Addresses the *match-rate* gap: subjects extracted from these corpora have no domain-entity to resolve to. Targets >= 70% mika-platform and >= 60% mika-cloud after expansion.

- **mika-skills#159** — `fix(qa-review): per-AC enumeration + per-corpus reporting in qa verdicts`. p1-important. Adjacent qa-skill defect surfaced by mika-qa's review of PR #926 (claimed "all 4 corpora below threshold" when 2/4 were above; missed R5 spot-check entirely). Per-AC enumeration discipline in the qa skill prompt to prevent the same misread shape on future multi-element AC tables.

These three follow-ups complete the structural recovery story; this ticket (mika#877) closes on R3a + R3b verification + R4 (CLI display fix) + R5 (graph traversal spot-check).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Unit 1's verification shows secondaries still don't recover after #874/#876 deploy | Unit 2's fallback path (SQL invalidation + tick) handles this for the throughput-bound case (R3a). If R3a still fails on a coverage-adequate corpus, the gap is per-corpus fairness — tracked in mika#927 (§Known Limitations). For coverage-limited corpora (R3b), the absolute threshold is bounded by domain graph and tracked in mika#928. |
| #874 doesn't ship before this ticket dispatches (Vincent's verdict relay still pending) | Unit 1 + Unit 4 can run independently. Unit 3's verification waits. The plan structure allows partial progress. |
| Re-extraction triggers excessive LLM cost on backfill | mika#906's tick budget (default 500 calls per agent per cycle) bounds the per-cycle exposure; full drain is multi-cycle by design. |
| Unit 4's display fix surfaces an even worse picture (e.g., a 5th corpus that wasn't expected) | Acceptable outcome — the display surfacing reality is the goal. If the picture is unexpected, file follow-up tickets. |
| mika#800 race condition is the actual operative cause for the secondary deficit | Unit 1 explicitly tests for this. If confirmed, scope shifts to mika#800; this ticket reduces to Unit 4 only + a coordination note. |

## Documentation / Operational Notes

- After Unit 2's empirical re-verification, capture the post-backfill SQL output in a compound doc at `docs/solutions/kg/multi-corpus-backfill-recovery-after-resolver-extractor-fixes-2026-05-XX.md`. The compound doc should explain the operational shape: when secondary-corpus deficit appears, what the SQL diagnostic looks like, and which mechanisms (#874, #876, #906 tick, manual SQL invalidation) restore coverage. **Authored during /ce:work, not during the groom** — per architect persisted preference `compound_doc_timing_forward_vs_retroactive_groom`, forward grooms author compound docs after empirical operational shape is known (drain latency, multi-cycle measurements). Retroactive grooms differ; this is forward.
- Update `docs/runtime-structure.md` if the backfill SQL becomes part of the operator runbook.
- mika-arch's `~/.mika/agents/mika-arch/identity.toml` already has the 4 `[kg].docs_roots` entries per CLAUDE.md; no identity changes needed.

## Sources & References

- Issue: `mika issue#877`
- Milestone: `KG flawlessness — extraction + resolution defects` (#19)
- Milestone siblings:
  - `mika issue#874` (p0-critical, Stage-2 resolver candidate-list rejection — awaiting Vincent's verdict relay)
  - `mika issue#876` (p1-important, subject_extractor parse-tolerance — already GROOMED on `fix/876/...`)
  - `mika issue#875` (p0-critical, Stage-1 exact-match returns 0 — CLOSED)
- Related closed: `mika issue#798` (multi-corpus aggregation primitive — `agent_kg_corpora` table, identity.toml `[kg].docs_roots` plural)
- Related open: `mika issue#800` (per-agent extractor loop race on shared corpus — adjacent, pressure-test in Unit 1)
- Recent infra: `mika issue#906` (resolver tick, deployed 2026-05-01), `mika issue#757` (extraction idempotency)
- Architecture: `docs/architecture/kg-implementation-conventions.md`, `docs/architecture/kg-id-convention.md`
- Files to inspect (per issue body): `crates/mika-agent/src/kg/subject_extractor.rs`, `crates/mika-agent/src/kg/entity_resolver.rs`, `crates/mika-cli/` or `crates/mika-common/src/cli/` formatter
