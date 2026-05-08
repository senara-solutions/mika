---
ticket: mika#1029
type: investigate
module: docs
tags: [docs-currency, knowledge-base-curation, compound-discipline, kg, cross-repo]
parent: mika_priority_stack_2026-05 Tier 1 #1
sibling: mika#1027
---

# Plan: Compound Knowledge Currency Audit (Tier 1 #1)

## Problem

`mika_priority_stack_2026-05` Tier 1 #1: *"Compounded knowledge currency — decide compound-refresh or not."* The question is whether `docs/solutions/**/*.md` entries decay as the corpus grows, and if so what curation policy is appropriate. Currently judgment-class — the operator needs decision-grade data before policy commitment.

This investigation produces a finding doc + script, declares an outcome path (A/B/C), and (if Path B) files follow-up implementation ticket(s) for the chosen curation policy. Same shape as mika#1027 (KG sync verification) which closed via outcome-path declaration.

## Reconnaissance already done (treat as starting context, verify in /ce:work)

Pre-investigation file walks (operator, 2026-05-08T09:50Z):

**Cross-repo entry counts:**

| Repo | `docs/solutions/` entries |
|---|---|
| `mika` | 410 |
| `mika-platform` | 61 |
| `mika-cloud` | 29 |
| `mika-skills` | 51 |
| **Total** | **551** |

**mika filename conventions (three coexisting patterns):**

| Convention | Count | Example |
|---|---|---|
| `YYYY-MM-DD-` prefix | 11 | `2026-05-08-summarizer-factual-assertion-reform.md` |
| `NNN-` ticket prefix | 23 | `692-self-knowledge-kg-upgrade.md` |
| Plain hyphenated (no prefix) | 376 | `dispatch-relay-minimax-fixes.md` |

The bulk (376/410 in mika) are plain-named — date-prefix convention is recent. The investigation must derive entry dates from **multiple sources**: filename prefix, frontmatter (if present), and `git log` first/last-commit timestamps as fallback.

**KG corpora (per mika-arch's [kg].docs_roots):** all four `docs/solutions/` directories are ingested by mika-arch as separate corpora keyed by `docs_root_hash`. The KG-resolution-drift check (AC#5) runs against each.

**Operator question this answers:** does the corpus need a curation policy (sweep, supersession-marker, age-tag, etc.), or is write-only-grow sustainable indefinitely?

## Design

### Investigation script approach

Single Bash script at `mika/scripts/investigate-docs-currency-1029.sh` (per architect-NF2 pattern from #1027 — per-ticket one-shot, not marketed as canonical reusable; methodology canonization is a separate decision after this audit). Bash + `find` + `grep` + `sqlite3` + `jq` + `git log`. Read-only, idempotent. Sources `MIKA_SERVER_LOG_FILE` from env or argv.

### What the script checks (six axes)

For each of the four corpora (mika, mika-platform, mika-cloud, mika-skills):

#### Axis 1 — Inventory + categorization

```bash
# A1.1: total entries per repo + per-category
for repo in mika mika-platform mika-cloud mika-skills; do
  ROOT="/data/workspace/mika-platform/$repo/docs/solutions"
  [[ -d "$ROOT" ]] || continue
  echo "## $repo"
  find "$ROOT" -mindepth 2 -name '*.md' -type f \
    | sed -E "s|^$ROOT/||; s|/[^/]+$||" \
    | sort | uniq -c | sort -rn
done
```

```bash
# A1.2: per-problem-type frontmatter tag (top 20)
for repo in mika mika-platform mika-cloud mika-skills; do
  find "/data/workspace/mika-platform/$repo/docs/solutions" -name '*.md' -type f 2>/dev/null \
    | xargs -I{} sh -c 'awk "/^problem_type:/{print \$2; exit}" "{}"' 2>/dev/null
done | sort | uniq -c | sort -rn | head -20
```

**Output:** per-repo entry count, per-category breakdown, top 20 `problem_type` tags. Markdown table.

#### Axis 2 — Age distribution (filename + frontmatter + git fallback)

```bash
# A2: derive entry date via 3-source fallback
for f in "$(find <corpus> -name '*.md')"; do
  # Source 1: filename YYYY-MM-DD prefix
  DATE=$(basename "$f" | grep -oE '^20[0-9]{2}-[01][0-9]-[0-3][0-9]')
  # Source 2: frontmatter `date:` field
  [[ -z "$DATE" ]] && DATE=$(awk '/^date:/{print $2; exit}' "$f" | grep -oE '20[0-9]{2}-[01][0-9]-[0-3][0-9]')
  # Source 3: git log first-commit date (introduction)
  [[ -z "$DATE" ]] && DATE=$(git -C "<repo>" log --diff-filter=A --format=%aI -- "$f" | head -1 | cut -dT -f1)
  echo "$DATE"
done | cut -dT -f1 | grep -oE '^20[0-9]{2}-[01][0-9]' | sort | uniq -c
```

**Output:** entries-per-quarter histogram per repo. Identify entries ≥ 6 months old (PRE-2025-11-01 as of today's run).

#### Axis 3 — Supersession analysis

```bash
# A3: explicit supersession markers
PATTERNS='supersedes|superseded by|replaced by|deprecated by|see instead|no longer applies|use .* instead|retired in favor of'
grep -r -i -E "$PATTERNS" /data/workspace/mika-platform/{mika,mika-platform,mika-cloud,mika-skills}/docs/solutions/ \
  --include='*.md' -l 2>/dev/null
```

For each marker hit, the script extracts the source doc + target doc reference and builds a **supersession chain** (graph of `superseded_by` edges). Output: list of chains + orphan markers (target doc not found).

**PASS shape:** every marker has a valid target; chains terminate at a current entry.
**FAIL shape:** orphan markers (target doc missing → broken supersession chain).

#### Axis 4 — Contradiction analysis

```sql
-- A4: same problem_type tag, different best-practices recommendations
-- (Heuristic; hand-validated sample.)
```

Approach: for each `problem_type` tag with ≥ 2 entries, sample 5 pairs and run a heuristic mini-LLM check (or operator-reviewed pairs list — groom decides which) to flag conflicting guidance. **Sample size cap:** 25 pairs total (5 per problem-type, top 5 problem-types by frequency). The 25-pair budget keeps the LLM cost bounded.

**Output:** flagged contradiction pairs + verdict (real / false-positive). Real contradictions named in finding doc.

#### Axis 5 — KG-resolution drift (per-corpus, mika-arch)

Per-corpus query against `kg_resolutions_log` for entries of varying ages:

```sql
-- A5: no_match rate by entry-age quartile, mika-arch agent only
WITH entries AS (
  SELECT
    c.source_doc_path,
    c.docs_root_hash,
    DATE(MIN(c.created_at)) AS first_chunked,
    cs.subject_entity_id
  FROM kg_chunks c
  JOIN kg_chunk_subjects cs
    ON cs.chunk_id = c.id
   AND cs.docs_root_hash = c.source_doc_hash
  GROUP BY c.source_doc_path, cs.subject_entity_id
),
resolved AS (
  SELECT
    e.source_doc_path,
    e.first_chunked,
    rl.outcome
  FROM entries e
  LEFT JOIN kg_resolutions_log rl
    ON rl.subject_entity_id = e.subject_entity_id
   AND rl.agent_id = 'mika-arch'
)
SELECT
  CASE
    WHEN first_chunked < '2025-11-01' THEN '01-old (>=6mo)'
    WHEN first_chunked < '2026-02-01' THEN '02-mid (3-6mo)'
    WHEN first_chunked < '2026-05-01' THEN '03-recent (1-3mo)'
    ELSE '04-fresh (<1mo)'
  END AS age_quartile,
  COUNT(*) AS total,
  SUM(CASE WHEN outcome = 'no_match' THEN 1 ELSE 0 END) AS no_match,
  ROUND(100.0 * SUM(CASE WHEN outcome = 'no_match' THEN 1 ELSE 0 END) / COUNT(*), 1) AS no_match_pct
FROM resolved
GROUP BY age_quartile
ORDER BY age_quartile;
```

**Hypothesis to validate:** older entries should have higher `no_match` rate IF the domain graph has shifted under them (skills renamed, tools removed, etc.).

**PASS shape (Outcome A):** `no_match` rate is roughly flat across age quartiles, OR rises by ≤ 10 percentage points from fresh to old. Means: domain graph stable; old docs still resolve.
**FAIL shape (Outcome B):** `no_match` rate rises by ≥ 25 percentage points fresh→old. Means: real drift; older docs are losing KG resolution. Curation needed.
**MIXED (Outcome C):** rise in 10–25 percentage points range. Operator judgment call.

#### Axis 6 — mika-arch consumption signal

```sql
-- A6: from mika-arch's last 30 days of query_knowledge_graph calls,
-- what fraction surface entries >= 6 months old?
SELECT
  CASE
    WHEN c.created_at < '2025-11-01' THEN '01-old (>=6mo)'
    ELSE '02-recent (<6mo)'
  END AS doc_age,
  COUNT(DISTINCT tc.id) AS distinct_calls
FROM tool_calls tc
JOIN llm_calls lc ON lc.id = tc.llm_call_id  -- linkage TBD; check schema
JOIN kg_chunks c ON ...  -- linkage TBD
WHERE tc.agent_id = 'mika-arch'
  AND tc.tool_name = 'query_knowledge_graph'
  AND tc.created_at >= datetime('now', '-30 days')
GROUP BY doc_age;
```

**Caveat:** this query requires schema introspection at /ce:work time — the linkage from `query_knowledge_graph` results back to specific `kg_chunks` may not be directly traceable in the current schema. If unavailable, this axis becomes a **proxy via `query_tool_history`** — search for KG queries returning content from old docs by sampling their output text. If neither path is feasible, axis is reported as `unavailable; needs schema enrichment` and noted in the finding.

**Hypothesis:** if old docs are rarely surfaced in real queries, sunset is low-risk. If old docs ARE surfaced often, they're load-bearing — curation must preserve them carefully.

#### Axis 7 — Curation-policy options

After Axes 1–6, synthesize 3–5 candidate policies. Examples (groom may revise):

1. **No curation** (status quo) — write-only-grow, no maintenance.
2. **Quarterly retention sweep** — operator-driven; flag docs > 12 months old with low query frequency for review.
3. **Supersession-marker convention** — mandate `> Superseded-By: <path>` callout when authoring a doc that replaces another; CI lint + KG metadata.
4. **Auto-sunset on KG resolution decay** — if a doc's subject entities have `no_match` rate > 80% AND last-queried > 6 months, auto-tag with `currency: stale` frontmatter; operator triages.
5. **Age-tagged frontmatter** — `currency: fresh|aging|stale` reviewed quarterly; KG ingestion weights by currency.
6. **Tag-based supersession (keyword-only)** — non-structural; lightweight.

**Trade-off table per policy:**

| Policy | Effort | False-sunset risk | Author overhead | Automation |
|---|---|---|---|---|
| 1 (none) | 0 | 0 | 0 | n/a |
| 2 (sweep) | High (manual review) | Low (operator-gated) | Low | None |
| 3 (markers) | Medium (CI + convention) | None | Medium (per-PR) | Partial |
| 4 (auto-sunset) | High (engine work) | Medium (heuristic-driven) | None | Full |
| 5 (age-tags) | Medium (review cadence) | Low | Medium | Partial |
| 6 (keyword) | Low | Low | Low | None |

The audit recommends a policy based on Axes 2–6 evidence; recommends does not commit (operator decides).

### Outcome path declaration

Finding doc TL;DR ends with one of:

- **Outcome A — Corpus is current:** age distribution healthy (no large pre-2025-11 cohort), no orphan supersession markers, no real contradictions, KG-resolution drift < 10pp fresh→old, mika-arch consumption signal shows old docs rarely surface. Recommendation: keep status quo (Policy 1). Close ticket; no follow-up.
- **Outcome B — Curation needed:** at least one signal is FAIL — significant aging cohort + drift OR real contradictions OR orphan markers. Audit recommends a specific policy from §"Axis 7 — options" with rationale; file follow-up implementation ticket for that policy.
- **Outcome C — Signals mixed:** drift in 10-25pp range or insufficient consumption-signal data (axis 6 unavailable). Audit names the ambiguity, surfaces options to operator with no auto-recommendation; operator chooses.

## Implementation Steps

### Step 1: Write the investigation script

**File:** `mika/scripts/investigate-docs-currency-1029.sh` (per architect-NF2 pattern from mika#1027 — per-ticket one-shot, not canonical).

Bash script that:
1. Sources `MIKA_SERVER_LOG_FILE` (env or argv).
2. Resolves the four corpus paths from `/data/workspace/mika-platform/<repo>/docs/solutions/`.
3. Runs Axes 1–6 in order, producing structured stdout (markdown tables per axis).
4. Writes findings to a buffer; finding-doc author (Step 3) cites verbatim where load-bearing.
5. Pre-flight schema-version assertion (`SELECT MAX(version) FROM schema_version` ≥ 27) before SQL runs.

Idempotent (read-only). The Axis 4 contradiction LLM check (sample of 25 pairs) is the only paid step; if `MIKA_KG_INGESTION_MODEL` (or equivalent) is unset, Axis 4 produces an "unavailable; LLM model unset" report and continues.

### Step 2: Run the script + capture output

```bash
bash mika/scripts/investigate-docs-currency-1029.sh > /tmp/docs-1029-output.md
```

### Step 3: Author the finding doc

**File:** `mika/docs/audits/2026-05-08-002-investigate-compound-knowledge-currency.md` (per architect-NF1 from mika#1027 — `audits/` unconditionally).

Frontmatter: `module: docs`, `tags: [docs-currency, knowledge-base-curation, compound-discipline, kg, cross-repo]`, `problem_type: docs-currency-audit`.

Structure:
- TL;DR with verdict per axis + outcome path declaration (A/B/C).
- Reconnaissance § (the 551-entry, three-convention starting state).
- Axes 1–6 results: verbatim tables.
- Axis 7: candidate policies with trade-off table; recommendation with rationale (or "no recommendation; operator decides" for Outcome C).
- Outcome path declaration; follow-up tickets named inline (Outcome B).

### Step 4: If Outcome B, file follow-up implementation ticket(s)

The ticket implements the chosen policy. Linked back to mika#1029 + the finding doc.

### Step 5: Close mika#1029

Same as mika#1027: investigation IS done regardless of outcome. If Outcome B: close after follow-up ticket(s) filed. If Outcome C: close after escalation delivered.

## Test Strategy

Investigation-only — no production code, no unit tests. Script self-validates via PASS/FAIL signals per axis. Re-runnable (idempotent).

The LLM-driven Axis 4 (contradiction analysis) is the only stochastic step; the script must record the model + temperature used for reproducibility, and the finding doc must list the 25 pair-verdicts inline so future audits can compare.

## Acceptance Criteria

- **AC#1**: Finding doc produced at `mika/docs/audits/2026-05-08-002-investigate-compound-knowledge-currency.md` with cross-repo scope.
- **AC#2**: Inventory tables per axis: per-repo + per-category + per-problem-type counts.
- **AC#3**: Age distribution per repo with three-source date derivation (filename prefix + frontmatter + git log fallback). Per-quarter histogram.
- **AC#4**: Supersession analysis: every marker chain identified; orphan markers flagged.
- **AC#5**: Contradiction analysis: 25 pair-sample verdicts (real vs false-positive) listed inline. Real contradictions named in finding.
- **AC#6**: KG-resolution drift summary: `no_match` rate per age-quartile (4 quartiles), per-corpus.
- **AC#7**: mika-arch consumption signal — fraction of last-30-days `query_knowledge_graph` calls surfacing docs ≥ 6 months old. If schema linkage unavailable, axis reported as `unavailable` with note + ticket reference for schema enrichment.
- **AC#8**: Curation-policy options (3–5) with trade-off table; recommendation OR no-recommendation declaration.
- **AC#9**: Outcome path declared (A/B/C); follow-up ticket(s) filed if Path B.
- **AC#10**: Investigation script committed at `mika/scripts/investigate-docs-currency-1029.sh`. Idempotent. Pre-flight schema-version assertion. Per-ticket one-shot per mika#1027 NF2.

## Risks & Open Questions

- **R1 (low):** Axis 6 schema linkage may not be directly traceable from `tool_calls` (query_knowledge_graph) back to specific `kg_chunks` rows. Mitigation: report axis as `unavailable; needs schema enrichment` if linkage absent. Don't gate the audit on this signal.
- **R2 (low):** Axis 4 LLM cost. 25 pair-checks at ~500 tokens each ≈ 12.5K tokens. At cheap-tier rates ($0.0001/call), ~$0.0025 — negligible. At sonnet rates ($3/M input + $15/M output), ~$0.05–$0.10. Acceptable. The script asserts a budget cap of 25 pairs and doesn't auto-expand.
- **R3 (low):** False-positive contradictions. The LLM heuristic flagging may be over-aggressive. Hand-validation of every flagged pair is required (named in AC#5). The script outputs the LLM verdict + a hand-validation column to fill in.
- **R4 (low):** Three filename conventions risk under-counting. The age-distribution axis uses filename → frontmatter → git-log fallback chain (Axis 2); plain-named entries fall through to git log. If git log is unavailable for a file (e.g., uncommitted), report as `undated`. Don't infer from neighboring files.
- **R5 (low):** Cross-repo scope. The investigation reads four repos. The script must NOT assume repo-local CWD — paths resolved from `/data/workspace/mika-platform/<repo>/`. Hard-coded for the operator's workstation; document the path as a script-input variable for future portability (out of scope).
- **R6 (low):** Currency vs Quality conflation. Old docs may still be high-quality; new docs may be wrong. The audit measures **currency proxies** (age + KG drift + consumption), NOT quality. Don't recommend sunset based on age alone — the policy options must specify how to combine signals.

## Sources

- `mika_priority_stack_2026-05` Tier 1 #1
- mika#1027 / PR #1028 — pattern + investigation methodology (audit class, outcome-path declaration, time-skew guards, jq selectors)
- `feedback_compound_infra_fixes.md` — compound discipline (infra fixes evaporate faster than product fixes; suggests currency may matter MORE for older infra entries)
- `mika/docs/solutions/best-practices/kg-post-deploy-verification-methodology-2026-05-08.md` — methodology pattern shipped via mika#1028 (same shape as this investigation)
- `crates/mika-agent/CLAUDE.md` § Knowledge Graph (subject extraction, resolution, query)
- mika#800 (KG topology — mika-arch sole consumer)
- mika#927 (per-corpus fairness)
- mika#876 (extraction parse tolerance — affects whether old entries got fully extracted)
