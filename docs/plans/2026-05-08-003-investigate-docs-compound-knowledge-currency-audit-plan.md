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
# NOTE per architect NF2: TWO git-log dates exist; we record both.
#   - introduction_date: first-commit (--diff-filter=A) — when the doc was added
#   - last_modified:     most-recent commit — when the doc was last touched
# For age-distribution histograms, use introduction_date (the canonical "creation" date).
# last_modified is recorded separately for the policy-options analysis (Axis 7) where
# "old + recently modified" tells a different story than "old + untouched".
# Rename caveat: `git log --follow` tracks renames; without it, a renamed file's
# introduction_date is the rename, not the original creation. The script uses --follow.
for f in "$(find <corpus> -name '*.md')"; do
  # Source 1: filename YYYY-MM-DD prefix
  DATE=$(basename "$f" | grep -oE '^20[0-9]{2}-[01][0-9]-[0-3][0-9]')
  # Source 2: frontmatter `date:` field
  [[ -z "$DATE" ]] && DATE=$(awk '/^date:/{print $2; exit}' "$f" | grep -oE '20[0-9]{2}-[01][0-9]-[0-3][0-9]')
  # Source 3: git log INTRODUCTION date (first-commit, with rename tracking)
  [[ -z "$DATE" ]] && DATE=$(git -C "<repo>" log --follow --diff-filter=A --format=%aI -- "$f" | tail -1 | cut -dT -f1)
  # Always record last_modified for Axis 7 inputs
  LAST_MOD=$(git -C "<repo>" log --follow --format=%aI -- "$f" | head -1 | cut -dT -f1)
  echo "$DATE $LAST_MOD $f"
done | tee /tmp/a2-raw.tsv \
     | awk '{print $1}' | grep -oE '^20[0-9]{2}-[01][0-9]' | sort | uniq -c
```

**Output:** entries-per-quarter histogram per repo (using introduction_date). Per-doc raw output preserved at `/tmp/a2-raw.tsv` with `<introduction_date> <last_modified> <path>` columns for Axis 7 inputs.

**Per architect NF2 — date semantics:**
- `introduction_date` is the canonical "when did this enter the corpus" date.
- `last_modified` is the canonical "when was this last touched" date.
- A doc with `introduction_date = 2025-04-01, last_modified = 2026-04-01` is **old-but-maintained**; a doc with both at `2025-04-01` is **old-and-untouched**. The two are policy-relevant in Axis 7 (an auto-sunset policy that ignores `last_modified` would falsely flag actively-maintained-old docs).

Identify entries ≥ 6 months old by introduction_date (pre-2025-11-01 as of today's run); cross-reference last_modified for "untouched-old" subset.

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

#### Axis 4 — Topic-cluster overlap (replaces LLM contradiction analysis per architect F1)

**Methodology change rationale:** the original Axis 4 (25-pair LLM contradiction verdict) was unsound at corpus scale — 25 pairs from 551 entries is a 0.018% sample, and "contradiction" is too fuzzy for verdict-class LLM classification. **Replaced with KG topic-cluster overlap analysis** (architect-proposed Option A): use existing `kg_subject_resolutions` to find docs that share the same domain entities, surface high-overlap pairs as topic-related (a topic cluster), and let the human-readable finding-doc author identify which clusters contain genuine contradictions. **Zero new LLM cost; uses already-extracted KG data.**

```sql
-- A4: topic clusters from kg_subject_resolutions (mika-arch agent only, all four corpora)
-- For each pair of docs that share at least N domain entities, surface the pair.
-- N = 3 (heuristic; tuned by Axis 7 thresholds — operator can re-run with different N).
WITH doc_entities AS (
  SELECT
    c.source_doc_path,
    c.docs_root_hash,
    rl.domain_entity_id
  FROM kg_chunks c
  JOIN kg_chunk_subjects cs
    ON cs.chunk_id = c.id
   AND cs.docs_root_hash = c.source_doc_hash
  JOIN kg_resolutions_log rl
    ON rl.subject_entity_id = cs.subject_entity_id
   AND rl.agent_id = 'mika-arch'
   AND rl.outcome IN ('exact_match', 'matched_llm', 'matched_llm_db_fallback')
  WHERE rl.domain_entity_id IS NOT NULL
  GROUP BY c.source_doc_path, c.docs_root_hash, rl.domain_entity_id
)
SELECT
  a.source_doc_path AS doc_a,
  b.source_doc_path AS doc_b,
  a.docs_root_hash  AS corpus_a,
  b.docs_root_hash  AS corpus_b,
  COUNT(*) AS shared_entities
FROM doc_entities a
JOIN doc_entities b
  ON a.domain_entity_id = b.domain_entity_id
 AND a.source_doc_path < b.source_doc_path  -- triangular (no self-pairs, no dupes)
GROUP BY a.source_doc_path, b.source_doc_path
HAVING shared_entities >= 3
ORDER BY shared_entities DESC
LIMIT 50;
```

| Output column | Meaning |
|---|---|
| `doc_a`, `doc_b` | the two docs in the pair |
| `corpus_a`, `corpus_b` | which corpus each lives in (cross-corpus pairs surface as different hashes) |
| `shared_entities` | count of distinct domain entities both docs reference |

**PASS shape:** topic clusters surfaced; finding-doc author manually inspects the top 10 highest-overlap pairs for genuine contradictions vs. legitimate topic adjacency. Most pairs will be **adjacent-topic** (e.g., two docs about the same skill = legitimately related, not contradictory). A subset may be **stale-vs-current** (an old doc and a new doc on the same topic — a candidate for supersession). A smaller subset may be **genuine contradiction** (two docs giving conflicting guidance).

**FAIL shape:** zero pairs returned (would indicate KG resolutions are absent — already covered by Axis 5 KG-resolution-drift; Axis 4 wouldn't independently fail).

**Hand-validation in finding doc:** for the top 10 clusters, the finding doc author categorizes each pair as `adjacent` / `supersession-candidate` / `genuine-contradiction`. No LLM verdict; human judgment with cited evidence (the shared entity list).

**Per architect F1 Option B fallback:** if the KG resolutions table is sparse (low coverage, e.g., < 100 resolutions cross-corpus), fall back to a 10-pair LLM spot-check that ONLY flags "this pair shares topic, look at it" with NO verdict classification. The 10-pair spot-check is informational, not decisional. Cost: ~$0.01 sonnet, ~$0.001 cheap-tier — negligible.

**Sample size note:** the LIMIT 50 keeps the script output bounded. The top 10 are hand-validated. Zero LLM cost in the primary path.

#### Axis 5 — KG-resolution drift (per-corpus, mika-arch)

**Drift definition (per architect NF1):** "drift" here means **resolution-rate drift** — the fraction of subject entities returning `outcome = 'no_match'` against the domain graph, computed per age-quartile. NOT temporal drift (days/weeks since last update). Resolution-rate drift is the right metric because it directly measures "is this doc still pointing at things that exist in the current domain graph?" — a structural staleness signal that age alone doesn't capture (a 3-year-old doc that still resolves cleanly is fresh-by-resolution; a 1-week-old doc whose entities all `no_match` is stale-by-resolution).

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

#### Axis 6 — mika-arch consumption signal (verbatim, per architect F2)

The schema linkage from `query_knowledge_graph` tool output back to specific `kg_chunks.id` is NOT directly traceable in the current schema (verified pre-grooming: `tool_calls.output` is an opaque JSON blob; chunk IDs may appear in it but require parsing). Two approaches, in priority order:

**A6 primary — substring scan on `tool_calls.output`:**

```sql
-- A6.1: count query_knowledge_graph calls in the last 30 days, mika-arch only
SELECT COUNT(*) AS total_kg_queries
FROM tool_calls
WHERE agent_id = 'mika-arch'
  AND tool_name = 'query_knowledge_graph'
  AND created_at >= datetime('now', '-30 days');
```

```sql
-- A6.2: dump the OUTPUT (tool_calls.output) for those calls; parse offline.
SELECT
  id,
  created_at,
  substr(output, 1, 50000) AS output_text
FROM tool_calls
WHERE agent_id = 'mika-arch'
  AND tool_name = 'query_knowledge_graph'
  AND created_at >= datetime('now', '-30 days')
ORDER BY created_at DESC;
```

The script then post-processes the dumped output: for each tool-call's `output_text`, regex-extract `source_doc_path` strings (the KG query tool returns chunk excerpts with their source paths). For each surfaced path, look up the doc's introduction date (Axis 2 derivation), classify as ≥6mo / <6mo, count.

```bash
# A6.3: post-process — extract source_doc_path mentions, classify by age
sqlite3 ~/.mika/data/mika.db "<A6.2 query>" \
  | grep -oE 'docs/solutions/[^"]+\.md' \
  | sort | uniq -c | sort -rn \
  | while read -r count path; do
      # Use Axis 2's date-derivation chain on $path
      DATE=$(...)
      echo "$count $DATE $path"
    done
```

| Output | Meaning |
|---|---|
| `total_kg_queries` | denominator |
| `count` per path | how many distinct KG queries surfaced this doc |
| `DATE` per path | derived via Axis 2 fallback chain |

**Aggregate:** fraction of distinct surfaced paths that are ≥6 months old.

**A6 fallback (if `tool_calls.output` is too short to contain useful chunk paths, e.g., truncated by 50KB cap or path not present):** use `query_tool_history` from agent perspective — sample 20 mika-arch sessions, eyeball whether KG content surfaced is from old docs. Report as `unavailable; needs schema enrichment` if even sampling fails. **Pre-flight check** in the script:

```sql
-- A6.0: verify A6 primary path is feasible (paths visible in output)
SELECT
  SUM(CASE WHEN output LIKE '%source_doc_path%' OR output LIKE '%docs/solutions/%' THEN 1 ELSE 0 END) AS with_path_hint,
  COUNT(*) AS total
FROM tool_calls
WHERE agent_id = 'mika-arch'
  AND tool_name = 'query_knowledge_graph'
  AND created_at >= datetime('now', '-30 days');
```

If `with_path_hint / total < 0.5`, primary path is unreliable → use fallback. If `total = 0`, mika-arch hasn't queried KG in 30 days → axis reported as `insufficient data` (separate from `unavailable`).

**PASS shape:** old-doc fraction is < 30% (most queries surface recent docs; old docs rarely load-bearing → curation/sunset is lower-risk).
**FAIL shape:** old-doc fraction is > 60% (old docs heavily load-bearing → curation must be conservative).
**MIXED:** 30-60% (old docs partially load-bearing; nuanced policy needed).

#### Axis 7 — Curation-policy options (synthesis from Axes 1–6 outputs)

Axis 7 is synthesis, not a query — but its inputs are the verbatim outputs of Axes 1–6. The script does not produce Axis 7 output directly; the **finding-doc author** does, with the structured Axes-1–6 outputs in hand.

**Recipe for the finding-doc author:**

1. **Read Axes 1–6 outputs.** Specifically: total entry count (A1), per-quarter age distribution (A2), supersession chains (A3), topic-clusters with hand-validation verdicts (A4), KG-resolution-rate drift per quartile (A5), mika-arch old-doc consumption fraction (A6).
2. **Compute three signal levels:** `Aging` (Axis 2: how old is the corpus?), `Drift` (Axis 5: are old docs losing KG resolution?), `Load-bearing` (Axis 6: do old docs surface in real queries?). Each scored as Low/Medium/High based on Axes-1–6 PASS/FAIL/MIXED verdicts.
3. **Map signal triple to recommendation:** the table below names the pre-decided mapping. Operator can override; the audit recommends.

| Aging | Drift | Load-bearing | Recommended policy | Rationale |
|---|---|---|---|---|
| Low | Low | any | **Policy 1 (no curation)** | Corpus is current; no decay to fix. |
| Medium | Low | Low | **Policy 1 (no curation)** | Old docs exist but don't matter. |
| Medium | Low | Medium/High | **Policy 6 (keyword supersession)** | Old docs are load-bearing; lightweight tagging gives authors a way to mark obsolescence without forcing a heavy process. |
| Medium | Medium | any | **Policy 5 (age-tagged frontmatter)** | Drift suggests review needed; age-tags are lighter than auto-sunset. |
| Medium/High | High | any | **Policy 4 (auto-sunset on KG decay)** | Drift is the dominant signal; auto-flagging by KG resolution rate scales without operator overhead. |
| High | any | High | **Policy 2 (quarterly sweep)** | Load-bearing old corpus needs operator-gated review to avoid false-sunsetting. |
| High | any | Low | **Policy 3 (supersession-marker convention)** | Author-driven supersession; safer than auto-sunset for high-aging-low-load corpus. |

**Candidate policies in scope (groom-validated cardinality of 6):**

1. **No curation** (status quo) — write-only-grow.
2. **Quarterly retention sweep** — operator flags docs >12mo old with low query frequency for review.
3. **Supersession-marker convention** — `> Superseded-By: <path>` callout mandate; CI lint + KG metadata.
4. **Auto-sunset on KG resolution decay** — `no_match` > 80% AND last-queried > 6mo → auto-tag `currency: stale`.
5. **Age-tagged frontmatter** — `currency: fresh|aging|stale` reviewed quarterly; KG ingestion weights by currency.
6. **Keyword supersession** — non-structural; lightweight tag-based "see X instead" hints.

**Trade-off table:**

| Policy | Effort | False-sunset risk | Author overhead | Automation |
|---|---|---|---|---|
| 1 (none) | 0 | 0 | 0 | n/a |
| 2 (sweep) | High | Low | Low | None |
| 3 (markers) | Medium | None | Medium | Partial |
| 4 (auto-sunset) | High | Medium | None | Full |
| 5 (age-tags) | Medium | Low | Medium | Partial |
| 6 (keyword) | Low | Low | Low | None |

**Output (per architect Q4):** finding doc presents **single recommendation with named runner-up** (not a ranked list of all 6). Runner-up is the policy in the row above or below in the signal-triple table — i.e., the closest alternative if Aging/Drift/Load-bearing categorization is borderline. Operator can override the recommendation; the audit's job is to make the recommendation actionable.

### Outcome path declaration

Finding doc TL;DR ends with one of:

- **Outcome A — Corpus is current:** age distribution healthy (no large pre-2025-11 cohort), no orphan supersession markers, no real contradictions, KG-resolution drift < 10pp fresh→old, mika-arch consumption signal shows old docs rarely surface. Recommendation: keep status quo (Policy 1). Close ticket; no follow-up.
- **Outcome B — Curation needed:** at least one signal is FAIL — significant aging cohort + drift OR real contradictions OR orphan markers. Audit recommends a specific policy from §"Axis 7 — options" with rationale; file follow-up implementation ticket for that policy.
- **Outcome C — Signals mixed:** drift in 10-25pp range or insufficient consumption-signal data (axis 6 unavailable). Audit names the ambiguity, surfaces options to operator with no auto-recommendation; operator chooses.

## Implementation Steps

### Step 1: Write the investigation script

**File:** `mika/scripts/investigate-docs-currency-1029.sh` (per architect-NF2 pattern from mika#1027 — per-ticket one-shot, not canonical).

Bash script that:
1. **Resolves environment variables** (per architect NF3):
   - `MIKA_PLATFORM_ROOT=${MIKA_PLATFORM_ROOT:-/data/workspace/mika-platform}` — workspace root holding the four sub-repos
   - `MIKA_SERVER_LOG_FILE=${MIKA_SERVER_LOG_FILE:-/var/log/mika/server.log}` — server log
   - `MIKA_DB=${MIKA_DB:-$HOME/.mika/data/mika.db}` — agent DB
   The script names these explicitly at the top with comments and uses them throughout — no hardcoded paths beyond the env defaults. Future portability without script edits.
2. Resolves the four corpus paths from `$MIKA_PLATFORM_ROOT/<repo>/docs/solutions/` for `<repo>` in `mika`, `mika-platform`, `mika-cloud`, `mika-skills`. Skips silently with a WARN if a corpus path doesn't exist.
3. Runs Axes 1–6 in order, producing structured stdout (markdown tables per axis).
4. Writes findings to a buffer; finding-doc author (Step 3) cites verbatim where load-bearing.
5. Pre-flight schema-version assertion (`SELECT MAX(version) FROM schema_version` ≥ 27) before SQL runs.

Idempotent (read-only). **Zero LLM cost in the primary path** (Axis 4 replaced by KG topic-clustering per architect F1; LLM fallback only fires if KG resolutions are sparse). If the script's primary path runs cleanly, no API keys are required.

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
- **AC#5**: Topic-cluster overlap analysis (per architect F1 — replaces the 25-pair LLM contradiction scheme): top 10 highest-overlap pairs identified via KG `kg_subject_resolutions` join (zero LLM cost in primary path). Each pair hand-classified in finding doc as `adjacent` / `supersession-candidate` / `genuine-contradiction`. Genuine contradictions named explicitly. LLM 10-pair fallback fires only if `kg_resolutions_log` rows are sparse (< 100 cross-corpus); if invoked, used for "look at this pair" flagging only, NO verdict claim.
- **AC#6**: KG **resolution-rate drift** (per architect NF1 — explicitly resolution-rate, not temporal): `no_match` rate per age-quartile (4 quartiles), per-corpus. Thresholds: PASS < 10pp fresh→old, FAIL ≥ 25pp, MIXED 10–25pp.
- **AC#7**: mika-arch consumption signal — fraction of last-30-days `query_knowledge_graph` calls surfacing docs ≥ 6 months old. If schema linkage unavailable, axis reported as `unavailable` with note + ticket reference for schema enrichment.
- **AC#8**: Curation-policy options (6 candidates per groom) with trade-off table; **single recommendation with named runner-up** per architect Q4 ratification. Recommendation derived via the signal-triple → policy mapping table (Axis 7). Operator override is supported.
- **AC#9**: Outcome path declared (A/B/C); follow-up ticket(s) filed if Path B.
- **AC#10**: Investigation script committed at `mika/scripts/investigate-docs-currency-1029.sh`. Idempotent. Pre-flight schema-version assertion. Per-ticket one-shot per mika#1027 NF2. Sources `MIKA_PLATFORM_ROOT` / `MIKA_SERVER_LOG_FILE` / `MIKA_DB` env vars with documented defaults (per architect NF3). Zero LLM cost in primary path.

## Risks & Open Questions

- **R1 (low):** Axis 6 schema linkage may not be directly traceable from `tool_calls` (query_knowledge_graph) back to specific `kg_chunks` rows. Mitigation: report axis as `unavailable; needs schema enrichment` if linkage absent. Don't gate the audit on this signal.
- **R2 (low):** Axis 4 LLM cost. 25 pair-checks at ~500 tokens each ≈ 12.5K tokens. At cheap-tier rates ($0.0001/call), ~$0.0025 — negligible. At sonnet rates ($3/M input + $15/M output), ~$0.05–$0.10. Acceptable. The script asserts a budget cap of 25 pairs and doesn't auto-expand.
- **R3 (low):** False-positive contradictions. The LLM heuristic flagging may be over-aggressive. Hand-validation of every flagged pair is required (named in AC#5). The script outputs the LLM verdict + a hand-validation column to fill in.
- **R4 (low):** Three filename conventions risk under-counting. The age-distribution axis uses filename → frontmatter → git-log fallback chain (Axis 2); plain-named entries fall through to git log. If git log is unavailable for a file (e.g., uncommitted), report as `undated`. Don't infer from neighboring files.
- **R5 (low):** Cross-repo scope. The investigation reads four repos. The script must NOT assume repo-local CWD — paths resolved from `/data/workspace/mika-platform/<repo>/`. Hard-coded for the operator's workstation; document the path as a script-input variable for future portability (out of scope).
- **R6 (low):** Currency vs Quality conflation. Old docs may still be high-quality; new docs may be wrong. The audit measures **currency proxies** (age + KG drift + consumption), NOT quality. Don't recommend sunset based on age alone — the policy options must specify how to combine signals.

**Resolved by architect first-pass:**
- F1 (BLOCKING) Axis 4 methodology: 25-pair LLM verdicts replaced with KG topic-clustering (Option A). LLM 10-pair fallback as conditional only if KG resolutions sparse.
- F2 (BLOCKING) verbatim queries: all seven axes now have verbatim SQL/bash with PASS/FAIL criteria.
- NF1 drift: explicitly defined as **resolution-rate drift** (not temporal); thresholds 10pp/25pp ratified.
- NF2 git-log dates: introduction_date (with `--follow` rename tracking) AND last_modified both recorded. Distinction named in plan text.
- NF3 env vars: `MIKA_PLATFORM_ROOT` / `MIKA_SERVER_LOG_FILE` / `MIKA_DB` with documented defaults; no hardcoded paths beyond defaults.
- Q3 (one-shot vs canonical): one-shot ratified.
- Q4 (single vs ranked): single recommendation with named runner-up ratified; signal-triple → policy mapping table added in Axis 7.

No open questions remain.

## Sources

- `mika_priority_stack_2026-05` Tier 1 #1
- mika#1027 / PR #1028 — pattern + investigation methodology (audit class, outcome-path declaration, time-skew guards, jq selectors)
- `feedback_compound_infra_fixes.md` — compound discipline (infra fixes evaporate faster than product fixes; suggests currency may matter MORE for older infra entries)
- `mika/docs/solutions/best-practices/kg-post-deploy-verification-methodology-2026-05-08.md` — methodology pattern shipped via mika#1028 (same shape as this investigation)
- `crates/mika-agent/CLAUDE.md` § Knowledge Graph (subject extraction, resolution, query)
- mika#800 (KG topology — mika-arch sole consumer)
- mika#927 (per-corpus fairness)
- mika#876 (extraction parse tolerance — affects whether old entries got fully extracted)
