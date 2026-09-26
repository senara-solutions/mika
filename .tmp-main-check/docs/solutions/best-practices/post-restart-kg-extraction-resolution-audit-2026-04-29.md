---
title: "Post-restart KG audit — extraction-and-resolution health (sibling to entity-orphan audit)"
date: 2026-04-29
last_updated: 2026-05-06
category: best-practices
module: mika-agent/kg, mika-arch
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - After every `make deploy` of `mika/` when the change touches `crates/mika-agent/src/kg/`
  - After every Mika server restart (extraction batches re-run on startup)
  - When mika-arch's grooming quality degrades or hallucination rate spikes (KG reachability is the upstream signal)
  - When PR #872's reflection-pass surface depends on policy-doc graph traversal (which itself depends on extraction + resolution succeeding)
tags:
  - kg
  - audit
  - post-deploy
  - post-restart
  - extraction
  - resolution
  - mika-arch
  - observability
---

# Post-restart KG audit — extraction-and-resolution health

## Context

`mika/CLAUDE.md` § "Post-restart safety check #757" defines four post-restart KG signals (A: extraction not re-running; B: `kg_budget_exhausted` zero; C: resolver backlog drains; D: cost prediction). The sibling audit at `docs/solutions/best-practices/post-deploy-kg-entity-audit-2026-04-28.md` covers a fifth check: orphan entities after skill renames or removals.

Together those audits cover **whether KG infrastructure is starting up cleanly**. They do NOT cover the case where infrastructure starts up cleanly but the **content side of the KG (subjects, relationships, resolutions) is silently incomplete or broken**.

On 2026-04-29, post-merge of mika#872 (which makes mika-arch's grooming depend on KG-corpus reachability for policy docs), an audit found:

- mika-arch's primary corpus (`mika/docs/solutions`): 30,456 subjects, 1,459 resolved (4.8%), **28,997 pending**.
- 3 of 4 mika-arch secondary corpora (`mika-skills`, `mika-platform`, `mika-cloud`): chunked but with ~0 resolutions (`mika-skills` 4 resolutions out of 164 subjects; `mika-platform` 0 out of 63; `mika-cloud` 0 out of 65).
- Recent extraction batches across all four agents producing `entities=0, relationships=0, docs_failed=0` (failure path is log-and-skip, not error).

Running only Signals A/B/C/D + the entity-orphan audit would have **passed clean** while the KG was effectively broken: extraction was running (A), budget was occasionally exhausted but not on every batch (B), resolver had drained some backlog (C), no skill renames so no orphans. The defects (mika#874–877, collected under milestone#19) are observable only when you query content tables and scan extraction-and-resolution log events directly.

## Status update — 2026-05-06

Audited again 11 days post the v27 deploy (server up since 2026-05-05T20:03). What's changed since the originating event above:

- **Audit 4 defect resolved.** mika#798 ("multi-corpus aggregation for mika-arch") closed 2026-04-25. `mika kg status --agent mika-arch` now lists all 4 corpora; the CLI parity check (Audit 4) currently passes.
- **Audit 3 budget-exhaustion clear.** Zero `kg_budget_exhausted` events since the 2026-05-05 restart, vs. the dense cluster of `aborted_budget=true` warnings the originating audit observed. Audit 3's red flag for that defect still applies as a procedure; the live failure mode is no longer this.
- **Audit 1 numbers improving but slow.** mika-arch primary corpus resolution rate moved 4.8% → 7.4% across 7 days under the #906 periodic resolver tick. The two-restart-cycle drain expectation in Audit 1 still holds.
- **Counter contract documented.** A 2026-05-06 audit initially misread `kg_resolver_tick.complete` `pending_before: 0` as a regression (filed and closed as mika#997). The actual contract: `count_pending()` is scoped to the 5 domain-resolvable subject types (`skill`, `tool`, `agent`, `problem_type`, `concept`) — see `crates/mika-agent/src/kg/entity_resolver.rs:891-906`. `mika kg status` "pending" includes the 3 subject-graph-only types (`pattern`, `failure_mode`, `solution_path`) that the resolver intentionally never touches. The two counters are designed to disagree by the size of the subject-graph-only inventory. Reference: `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md`. The CLI-clarity follow-up is mika#999.

The procedure (Audits 1–4, including the "passes clean while broken" framing) remains the canonical post-restart reference. Treat the originating-event examples below as historical — what they document is the bug class, not the current incident.

## Guidance

After deploys touching `crates/mika-agent/src/kg/` or after server restarts when KG quality is in question, run this audit alongside Signals A/B/C/D and the entity-orphan check. It composes — it does not replace either.

### Audit 1 — Per-corpus health probe

For each KG-enabled agent, surface chunk / subject / resolution counts per corpus.

**Run:**
```bash
sqlite3 ~/.mika/data/mika.db "
SELECT akc.docs_root_path,
       (SELECT COUNT(*) FROM kg_chunks
          WHERE docs_root_hash=akc.docs_root_hash) AS chunks,
       (SELECT COUNT(*) FROM kg_subject_entities
          WHERE docs_root_hash=akc.docs_root_hash) AS subjects,
       (SELECT COUNT(*) FROM kg_subject_resolutions
          WHERE subject_entity_id IN
            (SELECT id FROM kg_subject_entities
               WHERE docs_root_hash=akc.docs_root_hash)) AS resolved
FROM agent_kg_corpora akc
WHERE akc.agent_id='<agent>';
"
```

**Expected (healthy):**
- `chunks > 0` on every corpus row (chunking finished).
- `subjects / chunks` ratio is roughly consistent across corpora of similar doc count + density (typically ~5–15× chunks).
- `resolved / subjects > 0.5` on every corpus that has been live for more than one resolution-batch cycle.

**Red flags:**
- `chunks > 0, subjects = 0` → extraction never ran or always returned 0 entities for that corpus (Audit 2 confirms).
- `subjects > 0, resolved = 0` → resolver never made a successful match against this corpus (Audit 3 confirms; mika#877 family).
- `resolved / subjects < 0.1` after several batch cycles → resolver throwing out valid matches or budget-capping early (Audit 3 / mika#874 + #875 family).

### Audit 2 — Extraction-batch health

Scan `/var/log/mika/server.log` for `event:subject_extraction_complete` end-of-batch summaries.

**Run:**
```bash
tail -2000 /var/log/mika/server.log \
  | grep -oE '"event":"subject_extraction_complete"[^}]*' \
  | head -20
```

Or via DB for per-model breakdown:
```bash
sqlite3 ~/.mika/data/mika.db "
SELECT extraction_model,
       COUNT(*) AS total,
       SUM(CASE WHEN entities_extracted>0 THEN 1 ELSE 0 END) AS with_entities,
       SUM(CASE WHEN entities_extracted=0 AND relationships_extracted=0 THEN 1 ELSE 0 END) AS zero_zero
FROM kg_extractions
WHERE created_at > '2026-04-25'
GROUP BY extraction_model;
"
```

**Expected (healthy):**
- Most batches show `entities > 0, relationships > 0` per `subject_extraction_complete` event.
- `with_entities / total > 0.8` per extraction model in the per-model breakdown.

**Red flags:**
- Repeated batch summaries with `entities=0, relationships=0, docs_failed=0` — extraction LLM returning malformed JSON (mika#876 family). The `docs_failed=0` is the silent-failure tell: the extractor logs-and-skips, doesn't raise.
- Log-line chains `extraction_parse_failed_retry` → `extraction_semantic_exhausted` repeating across many `trace_id`s.
- Per-model `with_entities / total < 0.5` indicates a model-specific JSON-format regression (claude-haiku-4-5 and openai/gpt-5-nano both seen failing on 2026-04-29).

### Audit 3 — Resolution-batch health

Scan for `event:resolution_pending_complete` end-of-batch summaries.

**Run:**
```bash
tail -2000 /var/log/mika/server.log \
  | grep -oE '"event":"resolution_pending_complete"[^}]*' \
  | head -10
```

**Expected (healthy):**
- `matched_exact > 0` on every batch where the corpus has entity_keys already present in `kg_entities` (e.g., `skill:*` rows seeded by domain_builder).
- `aborted_budget=false` on most batches; `kg_budget_exhausted` events should be rare.
- `matched_llm + matched_exact + skipped_*` accounts for nearly all of `total`; `no_match` should be a small minority.

**Red flags:**
- `matched_exact: 0` across multiple consecutive batches — Stage-1 cheap path is dead (mika#875 family). Forces every resolution through expensive LLM disambiguation; budget exhausts; backlog grows.
- `resolution_matched_key_not_in_candidates` warnings where `entity_key == matched_key` (literal field equality in the warning JSON) — resolver discards its own correct LLM matches (mika#874 family).
- `aborted_budget=true` on every batch — budget cap hit consistently; backlog drain rate < extraction rate.

### Audit 4 — Multi-corpus visibility (CLI parity check)

`mika kg status --agent <agent>` only shows the primary corpus per agent. If an agent has multiple corpora registered (`agent_kg_corpora` rows > 1), the secondary corpora are invisible to the CLI summary.

**Run:**
```bash
sqlite3 ~/.mika/data/mika.db \
  "SELECT COUNT(*) FROM agent_kg_corpora WHERE agent_id='<agent>';"
mika kg status --agent <agent> | grep -c "docs_root"
```

**Expected (healthy):**
- The two counts match.

**Red flags:**
- Mismatch (e.g., 4 in `agent_kg_corpora`, 1 in CLI output for mika-arch). The secondary corpora exist but are silently uninspected (mika#877 family).

## Why This Matters

PR #872's reflection-pass design (instruction C3.3 — "runtime policy-doc consultation read path") makes mika-arch's grooming quality dependent on KG-corpus reachability for policy docs. Three failure modes degrade grooming silently:

1. **Extraction silently produces 0 entities for a doc.** The doc is chunked (visible to FTS / vector search) but has no graph entities — graph traversal can't reach it. Mika-arch's grooming will use FTS, which works, but cannot follow entity relationships across docs. This is the mika#876 family.

2. **Resolution rejects valid matches as `no_match`.** Subjects extracted from the doc never get resolved to canonical entities — graph traversal sees the subject as un-promoted noise. mika-arch's grooming sees the subject in the corpus but cannot link it to other docs. This is the mika#874 family.

3. **Stage-1 exact-match returns 0; budget exhausts on every batch.** All resolution funnels through expensive LLM disambiguation; the 500-call cap hits early; the rest stay pending. Backlog grows faster than drain rate. mika#875 family. Compounds with mika#874 to make backlog drain near-zero.

4. **Multi-corpus secondary corpora are invisible.** Even if extraction and resolution work for the primary corpus, secondary corpora (e.g., mika-arch's view of mika-skills, mika-platform, mika-cloud) may be stuck at 0 resolutions. Cross-repo policy docs become unreachable. mika#877 family.

The orphan-only audit (`post-deploy-kg-entity-audit-2026-04-28.md`) would have passed clean for all four. Signals A/B/C/D would have passed clean for #874 and #877 (extraction was running, budget wasn't exhausted on every batch, resolver had partial drain). Today's defects are observable only by querying content tables and reading the structured-event JSON in extraction/resolution log lines.

## When to Apply

Run this audit:

- **After every `make deploy`** of the `mika/` repo when the change touches `crates/mika-agent/src/kg/` (extractor, resolver, domain_builder, schema migrations).
- **After every Mika server restart** that triggers extraction batches on KG-enabled agents (`MIKA_DEV_MODE=true` startup re-runs extraction).
- **When mika-arch's grooming quality degrades** — hallucination rate spikes, citations become vague, the architect ghosts known compound docs. KG reachability is the upstream signal; if reachability is broken the symptom shows up at the agent level.
- **Before dispatching milestone-shaped work to mika-arch** — milestone grooming (per mika#879) requires the architect to read across all sub-issues' policy docs. If the secondary corpora are at 0 resolutions, the architect's milestone-level synthesis cannot ground its dependency analysis.

Skip when:

- The change is purely in `mika-skills/` or `mika-cloud/` and does not touch `mika/crates/mika-agent/src/kg/`.
- Server has been up for more than 24 hours without incident and no KG-related changes have shipped.

## Examples

**2026-04-29 audit (the originating event for this doc).** Post-restart of Mika server after merging mika#872. Audits 1–4 surfaced milestone#19's four sub-issues:

- Audit 1 found mika-arch primary at 4.8% resolved, secondary corpora at ≤4 resolutions each.
- Audit 2 found `entities=0, relationships=0` on most recent extraction batches across all four KG-enabled agents (mika, mika-arch, mika-dev, mika-qa); both `claude-haiku-4-5-20251001` and `openai/gpt-5-nano` failed.
- Audit 3 found `matched_exact=0` on every recent resolution batch; `resolution_matched_key_not_in_candidates` warnings with `entity_key == matched_key` literally identical.
- Audit 4 found mika-arch has 4 corpora rows in `agent_kg_corpora`; `mika kg status --agent mika-arch` only listed 1.

Each audit's findings became one ticket: mika#874 (resolver candidate-list bug), mika#875 (Stage-1 exact-match dead), mika#876 (extraction LLM JSON failures), mika#877 (secondary corpora invisible). All four assigned to milestone#19 ("KG flawlessness — extraction + resolution defects").

**Counter-example: passing all four audits cleanly.** A healthy mika-arch corpus would show:
```
docs_root_path                                   chunks  subjects  resolved
/data/workspace/mika-platform/mika/docs/solutions  342    1500       1100
```
…on Audit 1; `entities > 0, relationships > 0` on every Audit 2 batch summary; `matched_exact > 0` and `aborted_budget=false` on Audit 3; matching counts on Audit 4. With those four green, the orphan audit + Signals A/B/C/D cover the remaining post-restart concerns.

## Related

- **Counter contract reference (added 2026-05-06):** `docs/solutions/best-practices/kg-resolver-tick-visibility-audit-2026-05-06.md` — explains why `count_pending()` (and therefore `kg_resolver_tick.complete` `pending_before`) disagrees with `mika kg status` "pending" by the size of the subject-graph-only inventory. Read before writing custom backlog-estimation queries against `kg_subject_entities`.
- Sibling audit: `docs/solutions/best-practices/post-deploy-kg-entity-audit-2026-04-28.md` — orphan detection after skill renames or removals. Same SQLite + post-deploy slot; different table family + bug class.
- Discipline source: `docs/solutions/best-practices/verification-claims-with-expected-output-shape-2026-04-28.md` — every signal in this doc lists `Run:` + `Expected:` (healthy) + red flags per its requirement.
- Canonical signals: `mika/CLAUDE.md` § "Post-restart safety check (#757)" — Signals A/B/C/D this audit composes alongside.
- Resolver primitives: `docs/solutions/best-practices/kg-entity-resolution-two-stage-pipeline.md` — `kg_subject_resolutions` / `kg_resolutions_log` table contracts and outcome enum (`matched_exact|matched_llm|no_match|skipped_*|error`) used in Audit 3.
- Extractor primitive: `docs/solutions/best-practices/kg-subject-extraction-constrained-ner-2026-04-22.md` — what successful extraction produces; baseline for what Audit 2 should observe.
- Prior resolver/extractor bug class: `docs/solutions/runtime-errors/utf8-byte-slicing-panic-kg-resolver-extractor-2026-04-23.md` — different defect, same surface.
- Milestone-level KG retrospective precedent: `docs/solutions/workflow-issues/kg-milestone-14-autonomous-execution-retrospective-2026-04-22.md`.
- Originating defects: senara-solutions/mika#874, #875, #876, #877 (collected under milestone senara-solutions/mika#19 — "KG flawlessness — extraction + resolution defects", filed 2026-04-29).
- KG v27 deploy: per memory `project_kg_v27_deploy_2026-04-25.md` (deployed 2026-04-25; per-agent corpora introduced; this audit is the first comprehensive post-deploy verification of those corpora).
