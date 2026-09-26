---
module: docs
tags: [docs-currency, knowledge-base-curation, compound-discipline, kg, cross-repo]
problem_type: docs-currency-audit
ticket: mika#1029
date: 2026-05-08
---

# Compound Knowledge Currency Audit — Finding Doc

**Ticket:** mika#1029 (investigate(docs): compound knowledge currency audit — Tier 1 #1 of `mika_priority_stack_2026-05`)
**Investigation date:** 2026-05-08
**Cross-repo scope:** mika, mika-platform, mika-cloud, mika-skills (four `docs/solutions/` corpora)
**Script:** `scripts/investigate-docs-currency-1029.sh` (idempotent, zero LLM cost)

## TL;DR

**Outcome A — Corpus is current. No curation policy needed at this time.**

The entire 551-entry corpus was authored in 2026 (Q1: 200, Q2: 351). Zero entries are older than 6 months. Age-based currency signals (drift, staleness, supersession decay) are structurally inapplicable to a corpus this young. The KG-resolution no_match rate is ~36% across all age quartiles with no measurable drift between fresh and recent entries.

| Signal | Level | Rationale |
|--------|-------|-----------|
| **Aging** | Low | 0/551 entries ≥6 months old. Entire corpus is 2026Q1–Q2. |
| **Drift** | Low (insufficient data) | No old quartile exists to measure drift against. Fresh vs. recent no_match rates are flat (~36-38%). |
| **Load-bearing** | Unavailable | Axis 6 consumption signal unavailable — `tool_calls.output` for `query_knowledge_graph` does not contain source doc paths. |

Per the signal-triple → policy mapping table (Axis 7 in plan): Aging=Low, Drift=Low → **Policy 1 (no curation)** regardless of Load-bearing signal.

**Recommendation:** Policy 1 (no curation — status quo). **Runner-up:** Policy 6 (keyword supersession) — lightweight, zero-overhead, and ready to adopt if the corpus ages past 12 months without natural refresh.

**Revisit trigger:** Re-run this audit when the corpus reaches 12 months of age (earliest: 2027-Q1) or when entry count exceeds 1,000. The investigation script is committed and idempotent.

---

## Reconnaissance

**Cross-repo entry counts (verified by script):**

| Repo | `docs/solutions/` entries |
|------|--------------------------|
| mika | 410 |
| mika-platform | 61 |
| mika-cloud | 29 |
| mika-skills | 51 |
| **Total** | **551** |

**Filename conventions (mika repo, 410 entries):**

| Convention | Count | Example |
|-----------|-------|---------|
| `YYYY-MM-DD-` prefix | 11 | `2026-05-08-summarizer-factual-assertion-reform.md` |
| `NNN-` ticket prefix | 23 | `692-self-knowledge-kg-upgrade.md` |
| Plain hyphenated | 376 | `dispatch-relay-minimax-fixes.md` |

The bulk (376/410) use plain names; date-prefix convention is recent. The investigation script derives dates via three-source fallback: filename prefix > frontmatter `date:` > `git log` introduction date.

---

## Axis 1 — Inventory + Categorization

### Per-repo category breakdown

**mika (410 entries):** Dominated by `architecture-patterns` (100), `best-practices` (82), `logic-errors` (41), `integration-issues` (39). 24 categories total.

**mika-platform (61 entries):** `best-practices` (15), `integration-issues` (11), `workflow-patterns` (8), `dev-loop` (6), `cross-repo-patterns` (6).

**mika-cloud (29 entries):** `infrastructure` (6), `api-patterns` (6), `ui-patterns` (4).

**mika-skills (51 entries):** `prompt-engineering` (19), `integration-issues` (13), `architecture-patterns` (6).

### Top problem_type tags (all repos)

| problem_type | Count |
|-------------|-------|
| best_practice | 108 |
| logic_error | 23 |
| workflow_issue | 21 |
| runtime_error | 8 |
| integration_issue | 8 |

**Observation:** `best_practice` dominates (108/551 = 20%). There is tag inconsistency: `logic_error` vs `logic-error` (23 + 4 = 27), `workflow_issue` vs `workflow-issues` vs `workflow_pattern` vs `workflow-pattern` (21 + 1 + 1 + 1 = 24). This is a taxonomy hygiene issue, not a currency issue — mentioned for completeness.

---

## Axis 2 — Age Distribution

### Per-repo quarterly histogram

| Repo | 2026Q1 | 2026Q2 | Undated |
|------|--------|--------|---------|
| mika | 142 | 268 | 0 |
| mika-platform | 13 | 48 | 0 |
| mika-cloud | 19 | 10 | 0 |
| mika-skills | 26 | 25 | 0 |
| **Total** | **200** | **351** | **0** |

### Age verdict

**Zero entries ≥6 months old.** The entire corpus was authored between 2026-01 and 2026-05. The oldest entries (2026-Q1) are approximately 3 months old as of the investigation date.

- **Old entries (≥6mo):** 0 / 551 dated entries
- **Old + untouched:** 0 (both introduction and last_modified are old)
- **Old + maintained:** 0 (old intro, recent last_modified)

**Aging signal: LOW.** There is no aging cohort to curate.

---

## Axis 3 — Supersession Analysis

**Files with supersession-pattern markers:** 71

However, manual inspection reveals that the vast majority of these 71 files contain **coding advice patterns** ("use X instead of Y") rather than true document-supersession markers. The `grep` pattern (`supersedes|replaced by|...`) has a high false-positive rate against solution docs that naturally contain phrases like "use `chars().count()` instead of byte slicing."

### True supersession markers

Of the 71 files, only a handful contain genuine document-to-document supersession references:

| Type | Source | Target | Status |
|------|--------|--------|--------|
| Chain | `webhook-zero-tools-guard-fabrication-prevention.md` | `2026-04-12-tighten-webhook-qa-pass-entry-point.md` | Target found (mika) |
| Chain | `merge-two-step-llm-tool-contracts.md` | `harden-write-skill-variant-no-path-input.md` | Target found (mika) |
| Orphan | `kg-milestone-14-autonomous-execution-retrospective.md` | `feedback_pipeline_scaling.md` | Target NOT found |
| Orphan | `required-tools-gate-evasion-patterns.md` | plan doc (not in solutions/) | Expected — references a plan, not a solution |
| Orphan | `socratic-multi-ticket-milestone-planning.md` | `feedback_full_pipeline_always.md` | Target NOT found |
| Orphan | `minimax-m2.7-calibration.md` | `branch-protection-required-checks.md` | Target NOT found (runbook, not solution) |

**Supersession verdict:** 2 valid chains, 4 orphan markers. Of the orphans, 2 reference non-solution targets (a plan doc and a runbook) — expected false positives. 2 reference solution docs that may have been renamed or removed (`feedback_pipeline_scaling.md`, `feedback_full_pipeline_always.md`) — these appear to be memory/feedback docs that don't live in `docs/solutions/`.

**Assessment:** Supersession is not a systemic problem. The orphans are isolated and traceable to naming-convention differences between solution docs and memory/feedback files. No chain is broken within the `docs/solutions/` corpus.

---

## Axis 4 — Topic-Cluster Overlap (KG-based)

**Methodology:** KG topic-clustering via `kg_subject_resolutions` (zero LLM cost). Per architect F1 — replaces the original 25-pair LLM contradiction scheme.

**KG subject resolutions (mika-arch):** 2,317 resolved entities. Resolution is sufficient for meaningful clustering (threshold: 100).

**Total doc pairs sharing ≥3 domain entities:** 2,566

### Top 10 highest-overlap pairs — hand classification

| Pair | Shared | Classification | Evidence |
|------|--------|---------------|----------|
| `bundle-engine-coupled-skills.md` ↔ `removing-bundled-skill.md` | 10 | **Adjacent topic** | Both document the bundled skills architecture from different angles (adding vs removing). Complementary, not contradictory. |
| `run-gh-github-token-injection.md` ↔ `exec-handler-gh-token-injection.md` | 8 | **Adjacent topic** | Same security hardening domain (GH_TOKEN injection), different subsystems (run_gh vs exec handler). Complementary. |
| `ci-gate-tool.md` ↔ `exec-handler-gh-token-injection.md` | 7 | **Adjacent topic** | CI gate touches the same tools/agents as the exec handler. Both reference qa-review, build_mika, mika-dev. No conflict. |
| `tasks-type-column.md` ↔ `rename-work-item-tools.md` | 7 | **Supersession candidate** | `rename-work-item-tools` documents the rename of tools that `tasks-type-column` references by old names. The earlier doc's tool references are outdated but the architectural content is still valid. |
| `removing-bundled-skill.md` ↔ `custom-skill-silent-loading-failure.md` | 7 | **Adjacent topic** | Skill system from two perspectives. No conflict. |
| `socratic-multi-ticket-milestone-planning.md` ↔ `exec-handler-gh-token-injection.md` | 7 | **Adjacent topic** | Both reference the same agent ecosystem. No content conflict. |
| `bundle-engine-coupled-skills.md` ↔ `custom-skill-silent-loading-failure.md` | 6 | **Adjacent topic** | Related skill subsystem documentation. No conflict. |
| `adding-skill-review-builtin-handler.md` ↔ `removing-bundled-skill.md` | 6 | **Adjacent topic** | Lifecycle operations on the same subsystem. |
| `ci-gate-tool.md` ↔ `run-gh-github-token-injection.md` | 6 | **Adjacent topic** | Overlapping tool references. |
| `git-credential-helper.md` ↔ `gh-token-identity-collision.md` | 6 | **Adjacent topic** | Same GH token management domain. |

**Contradiction verdict:** **Zero genuine contradictions found.** The top 10 pairs are all adjacent-topic (legitimately related docs) or supersession-candidate (tool rename drift). The one supersession candidate (`tasks-type-column` ↔ `rename-work-item-tools`) is minor — the earlier doc uses old tool names but the architectural guidance is still correct.

---

## Axis 5 — KG-Resolution Drift

**Methodology:** Resolution-rate drift — `no_match` rate per age quartile. Per architect NF1.

### no_match rate by age quartile (all corpora)

| Age quartile | Total entities | no_match | no_match % |
|-------------|---------------|----------|-----------|
| 03-recent (1-3mo) | 19,943 | 6,911 | 34.7% |
| 04-fresh (<1mo) | 164 | 63 | 38.4% |
| 01-old (≥6mo) | — | — | N/A |
| 02-mid (3-6mo) | — | — | N/A |

**No old or mid quartile data exists.** The entire corpus was chunked in the last 3 months.

### Drift computation

- **Fresh (<1mo) no_match rate:** 38.4%
- **Old (≥6mo) no_match rate:** N/A (no old quartile)
- **Drift:** Cannot compute (insufficient data)

**Observation on baseline no_match rate:** The ~36% no_match rate is not a currency signal — it's a structural baseline of the domain graph's coverage. It means ~36% of subject entities extracted from docs don't map to current domain graph entities. This is expected: docs reference concepts (e.g., specific bug names, PR numbers, third-party libraries) that are not modeled in the domain graph (which contains skills, tools, agents, problem_types, concepts). A future audit should compare this baseline to see if it changes.

**A5 verdict:** PASS by default — no drift measurable. Baseline recorded for future comparison.

---

## Axis 6 — mika-arch Consumption Signal

**Total KG queries (30 days):** 92
**With doc-path hints in output:** 0

**Verdict: UNAVAILABLE.** `tool_calls.output` for `query_knowledge_graph` calls does not contain `source_doc_path` or `docs/solutions/` strings. The KG query tool returns results in a format that does not preserve chunk provenance at the tool-output level.

This is an expected schema limitation, not a bug. The KG query path is: `query_knowledge_graph` → search `kg_chunks` FTS/vec → return excerpts → LLM consumes. The source doc path is available at the DB layer but not surfaced in the serialized tool output.

**Impact on this audit:** Axis 6 cannot determine whether old docs are load-bearing in mika-arch's KG queries. However, since Axis 2 shows zero old docs exist, this signal gap has no practical impact on the outcome determination.

---

## Axis 7 — Curation Policy Options

### Signal triple

| Signal | Level |
|--------|-------|
| Aging | **Low** (0/551 ≥6mo) |
| Drift | **Low** (no drift measurable; flat baseline) |
| Load-bearing | **Unavailable** (A6 schema limitation) |

### Policy mapping

Per the plan's signal-triple → policy mapping table:

- **Aging=Low, Drift=Low, Load-bearing=any → Policy 1 (no curation)**

The corpus is too young for any curation policy to have a target. All six candidate policies are solutions to a problem that does not yet exist.

### Candidate policies (for future reference)

| # | Policy | Effort | False-sunset risk | Author overhead | Automation | When to adopt |
|---|--------|--------|------------------|-----------------|------------|---------------|
| 1 | **No curation (status quo)** | 0 | 0 | 0 | n/a | Now (recommended) |
| 2 | Quarterly retention sweep | High | Low | Low | None | If corpus reaches 1000+ entries with significant ≥12mo cohort |
| 3 | Supersession-marker convention | Medium | None | Medium | Partial | If orphan marker count grows significantly |
| 4 | Auto-sunset on KG resolution decay | High | Medium | None | Full | If A5 drift reaches ≥25pp in future audit |
| 5 | Age-tagged frontmatter | Medium | Low | Medium | Partial | If operator wants graduated currency signals |
| 6 | **Keyword supersession** | Low | Low | Low | None | Lightweight first step if aging appears before formal policy |

### Recommendation

**Policy 1 (no curation)** — the corpus is 3 months old at most. Curation is premature.

**Runner-up: Policy 6 (keyword supersession)** — if the corpus ages without natural refresh, this is the lowest-overhead first step. It requires only a convention (`> Superseded-By: <path>` callout) and optional CI lint. Zero ongoing operator cost.

---

## Outcome Path Declaration

### Outcome A — Corpus is current

- **Age distribution:** All 551 entries are 2026Q1 or 2026Q2. Zero entries ≥6 months old.
- **Supersession:** 2 valid chains (both healthy), 4 orphan markers (2 expected false positives, 2 reference non-solution files). No broken chains within the corpus.
- **Topic overlap:** 2,566 pairs with ≥3 shared entities. Zero genuine contradictions in top 10 hand-validated pairs.
- **KG-resolution drift:** Not measurable (no old quartile). Baseline no_match rate ~36% recorded.
- **Consumption signal:** Unavailable (no practical impact — no old docs exist to consume).

**Decision:** Keep status quo (Policy 1). Close ticket. No follow-up implementation ticket required.

**Revisit triggers:**
1. Corpus age reaches 12 months (2027-Q1)
2. Entry count exceeds 1,000
3. KG-resolution drift audit shows ≥25pp rise in no_match rate for old quartile
4. Operator observes contradictory guidance in practice

---

## Incidental findings (non-blocking)

1. **Frontmatter tag inconsistency:** `logic_error` vs `logic-error`, `workflow_issue` vs `workflow-issues`. Low priority — doesn't affect KG or retrieval, but a taxonomy cleanup pass would improve `problem_type` aggregation. Not in scope for this audit.

2. **High baseline no_match rate (~36%):** Expected — the domain graph models skills/tools/agents/problem_types/concepts, but docs reference many entities outside this taxonomy (PR numbers, commit SHAs, third-party libraries). Not a staleness signal; it's a coverage boundary.

3. **Axis 6 schema gap:** `query_knowledge_graph` tool output doesn't preserve chunk provenance. Future schema enrichment (adding `source_doc_path` to the serialized KG query response) would enable consumption-signal analysis. Low priority while the corpus is young.

4. **Supersession orphans reference memory/feedback files:** `feedback_pipeline_scaling.md` and `feedback_full_pipeline_always.md` live outside `docs/solutions/` (likely in `.claude/` memory or project feedback). The supersession pattern works within `docs/solutions/` but doesn't cross the boundary to other doc locations. Not a problem unless cross-location supersession becomes common.

---

## Sources

- `mika_priority_stack_2026-05` Tier 1 #1
- mika#1029 (this ticket)
- mika#1027 / PR #1028 — pattern (audit class, outcome-path declaration)
- `scripts/investigate-docs-currency-1029.sh` — investigation script (committed)
- `docs/plans/2026-05-08-003-investigate-docs-compound-knowledge-currency-audit-plan.md` — groomed plan
