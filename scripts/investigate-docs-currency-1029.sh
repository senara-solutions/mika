#!/usr/bin/env bash
# investigate-docs-currency-1029.sh — Compound knowledge currency audit (mika#1029)
#
# Investigation script: reads docs/solutions/ corpora across four repos + KG state
# from SQLite + structured server logs. Six axes: A1 inventory, A2 age distribution,
# A3 supersession, A4 topic-cluster overlap, A5 KG-resolution drift, A6 consumption signal.
#
# Usage:
#   bash scripts/investigate-docs-currency-1029.sh
#
# Environment:
#   MIKA_PLATFORM_ROOT   — workspace root holding the four sub-repos
#                          (default: /data/workspace/mika-platform)
#   MIKA_SERVER_LOG_FILE — server log path (default: /var/log/mika/server.log)
#   MIKA_DB              — SQLite DB path  (default: ~/.mika/data/mika.db)
#
# This script is read-only and idempotent — safe to re-run.
# Per-ticket one-shot (mika#1029); not marketed as canonical reusable.
# Zero LLM cost in the primary path.

set -euo pipefail

# --- Configuration (per architect NF3) ---
PLATFORM_ROOT="${MIKA_PLATFORM_ROOT:-/data/workspace/mika-platform}"
DB="${MIKA_DB:-$HOME/.mika/data/mika.db}"
LOG_FILE="${MIKA_SERVER_LOG_FILE:-/var/log/mika/server.log}"
AGENT_ID="mika-arch"  # sole KG consumer per mika#800
MIN_SCHEMA_VERSION=27
TODAY=$(date -u +'%Y-%m-%d')
SIX_MONTHS_AGO=$(date -u -d '6 months ago' +'%Y-%m-%d' 2>/dev/null || date -u -v-6m +'%Y-%m-%d' 2>/dev/null || echo "2025-11-01")

# Corpus paths — note: mika-platform's docs/solutions/ is at the meta-repo root,
# NOT at $PLATFORM_ROOT/mika-platform/docs/solutions/
declare -A CORPUS_PATHS=(
  [mika]="$PLATFORM_ROOT/mika/docs/solutions"
  [mika-platform]="$PLATFORM_ROOT/docs/solutions"
  [mika-cloud]="$PLATFORM_ROOT/mika-cloud/docs/solutions"
  [mika-skills]="$PLATFORM_ROOT/mika-skills/docs/solutions"
)

# Git roots for git log queries
declare -A GIT_ROOTS=(
  [mika]="$PLATFORM_ROOT/mika"
  [mika-platform]="$PLATFORM_ROOT"
  [mika-cloud]="$PLATFORM_ROOT/mika-cloud"
  [mika-skills]="$PLATFORM_ROOT/mika-skills"
)

# --- Pre-flight checks ---
echo "# Compound Knowledge Currency Audit — mika#1029"
echo ""
echo "**Timestamp:** $(date -u +'%Y-%m-%dT%H:%M:%SZ')"
echo "**Platform root:** $PLATFORM_ROOT"
echo "**DB:** $DB"
echo "**Log:** $LOG_FILE"
echo "**Agent:** $AGENT_ID"
echo "**Six-months-ago cutoff:** $SIX_MONTHS_AGO"
echo ""

# Verify corpora exist
for repo in mika mika-platform mika-cloud mika-skills; do
  if [ -d "${CORPUS_PATHS[$repo]}" ]; then
    echo "- **$repo corpus:** ${CORPUS_PATHS[$repo]} ✓"
  else
    echo "- **$repo corpus:** ${CORPUS_PATHS[$repo]} — WARN: not found, skipping"
  fi
done
echo ""

# DB check
if [ ! -f "$DB" ]; then
  echo "ERROR: Database not found at $DB"
  exit 1
fi

SCHEMA_VERSION=$(sqlite3 "$DB" "SELECT MAX(version) FROM schema_version;")
echo "**Schema version:** $SCHEMA_VERSION"
if [ "$SCHEMA_VERSION" -lt "$MIN_SCHEMA_VERSION" ]; then
  echo "ERROR: Schema version $SCHEMA_VERSION < $MIN_SCHEMA_VERSION — queries incompatible"
  exit 1
fi
echo ""

# Log check
LOG_ACCESSIBLE=true
if [ ! -f "$LOG_FILE" ]; then
  echo "WARN: Log file not found at $LOG_FILE — Axis 6 log probes may be limited"
  LOG_ACCESSIBLE=false
elif [ ! -r "$LOG_FILE" ]; then
  echo "WARN: Log file not readable at $LOG_FILE"
  LOG_ACCESSIBLE=false
fi
echo ""

# ============================================================
# AXIS 1 — Inventory + Categorization
# ============================================================
echo "## Axis 1 — Inventory + Categorization"
echo ""

TOTAL_ALL=0
echo "### A1.1: Entry count per repo + per category"
echo ""

for repo in mika mika-platform mika-cloud mika-skills; do
  ROOT="${CORPUS_PATHS[$repo]}"
  [ -d "$ROOT" ] || continue

  COUNT=$(find "$ROOT" -name '*.md' -type f 2>/dev/null | wc -l)
  TOTAL_ALL=$((TOTAL_ALL + COUNT))
  echo "#### $repo ($COUNT entries)"
  echo ""
  echo "| Category | Count |"
  echo "|----------|-------|"

  # Per-category breakdown (subdirectories)
  find "$ROOT" -mindepth 2 -name '*.md' -type f 2>/dev/null \
    | sed -E "s|^$ROOT/||; s|/[^/]+$||" \
    | sort | uniq -c | sort -rn \
    | while read -r cnt cat; do
        echo "| $cat | $cnt |"
      done

  # Root-level files (no subdirectory)
  ROOT_COUNT=$(find "$ROOT" -maxdepth 1 -name '*.md' -type f ! -name 'README.md' 2>/dev/null | wc -l)
  if [ "$ROOT_COUNT" -gt 0 ]; then
    echo "| (root level) | $ROOT_COUNT |"
  fi
  echo ""
done

echo "**Total across all repos:** $TOTAL_ALL"
echo ""

# A1.2: Per-problem-type frontmatter tag (top 20)
echo "### A1.2: Top 20 problem_type tags (all repos)"
echo ""
echo "| problem_type | Count |"
echo "|-------------|-------|"

for repo in mika mika-platform mika-cloud mika-skills; do
  ROOT="${CORPUS_PATHS[$repo]}"
  [ -d "$ROOT" ] || continue
  find "$ROOT" -name '*.md' -type f 2>/dev/null
done | xargs grep -h '^problem_type:' 2>/dev/null \
  | sed 's/^problem_type:[[:space:]]*//' \
  | sort | uniq -c | sort -rn | head -20 \
  | while read -r cnt ptype; do
      echo "| $ptype | $cnt |"
    done
echo ""

# ============================================================
# AXIS 2 — Age Distribution
# ============================================================
echo "## Axis 2 — Age Distribution"
echo ""

A2_RAW=$(mktemp /tmp/a2-raw-XXXXXX.tsv)
trap "rm -f $A2_RAW" EXIT

echo "### A2: Date derivation (filename → frontmatter → git log fallback)"
echo ""
echo "Using three-source fallback chain per plan. Recording introduction_date + last_modified."
echo ""

for repo in mika mika-platform mika-cloud mika-skills; do
  ROOT="${CORPUS_PATHS[$repo]}"
  GIT_ROOT="${GIT_ROOTS[$repo]}"
  [ -d "$ROOT" ] || continue

  # Build a git log cache for this repo (batch, no --follow for speed)
  # Maps relative path → "intro_date last_mod_date"
  GIT_CACHE=$(mktemp /tmp/a2-gitcache-XXXXXX.tsv)
  if [ -d "$GIT_ROOT/.git" ] || [ -f "$GIT_ROOT/.git" ]; then
    # Get last-modified dates in one pass via git log
    git -C "$GIT_ROOT" log --name-only --format="DATE:%aI" -- "docs/solutions/" 2>/dev/null \
      | awk -F'DATE:' '
        /^DATE:/ { date = substr($2, 1, 10); next }
        /\.md$/ && date != "" {
          if (!(($0) in last_mod)) last_mod[$0] = date
          intro[$0] = date  # keeps getting overwritten; last value = earliest commit
        }
        END {
          for (f in intro) print f "\t" intro[f] "\t" last_mod[f]
        }
      ' > "$GIT_CACHE"
  fi

  while IFS= read -r f; do
    BASENAME=$(basename "$f")

    # Source 1: filename YYYY-MM-DD prefix
    DATE=$(echo "$BASENAME" | grep -oE '^20[0-9]{2}-[01][0-9]-[0-3][0-9]' || true)

    # Source 2: frontmatter date: field
    if [ -z "$DATE" ]; then
      DATE=$(awk '/^date:/{print $2; exit}' "$f" 2>/dev/null | grep -oE '20[0-9]{2}-[01][0-9]-[0-3][0-9]' || true)
    fi

    # Source 3: git log introduction date (from cache, no per-file git call)
    REL_PATH=$(realpath --relative-to="$GIT_ROOT" "$f" 2>/dev/null || echo "")
    if [ -z "$DATE" ] && [ -n "$REL_PATH" ]; then
      DATE=$(awk -F'\t' -v p="$REL_PATH" '$1 == p {print $2; exit}' "$GIT_CACHE" || true)
    fi

    # Last modified date from cache
    LAST_MOD=""
    if [ -n "$REL_PATH" ]; then
      LAST_MOD=$(awk -F'\t' -v p="$REL_PATH" '$1 == p {print $3; exit}' "$GIT_CACHE" || true)
    fi

    [ -z "$DATE" ] && DATE="undated"
    [ -z "$LAST_MOD" ] && LAST_MOD="unknown"

    echo "$repo	$DATE	$LAST_MOD	$f" >> "$A2_RAW"
  done < <(find "$ROOT" -name '*.md' -type f ! -name 'README.md' 2>/dev/null)

  rm -f "$GIT_CACHE"
done

# Per-repo quarterly histogram (using introduction_date)
echo "### A2.1: Per-repo quarterly age distribution"
echo ""

for repo in mika mika-platform mika-cloud mika-skills; do
  REPO_COUNT=$(grep -c "^$repo	" "$A2_RAW" 2>/dev/null || echo 0)
  [ "$REPO_COUNT" -eq 0 ] && continue

  echo "#### $repo ($REPO_COUNT entries)"
  echo ""
  echo "| Quarter | Count |"
  echo "|---------|-------|"

  { grep "^$repo	" "$A2_RAW" || true; } \
    | awk -F'\t' '{print $2}' \
    | { grep -oE '^20[0-9]{2}-[01][0-9]' || true; } \
    | sed 's/-0[123]$/Q1/; s/-0[456]$/Q2/; s/-0[789]$/Q3/; s/-1[012]$/Q4/' \
    | sort | uniq -c | sort -k2 \
    | while read -r cnt quarter; do
        echo "| $quarter | $cnt |"
      done

  # Count undated
  UNDATED=$(grep -c "^${repo}	undated	" "$A2_RAW" 2>/dev/null || true)
  UNDATED=${UNDATED:-0}
  # Sanitize: strip any non-numeric chars (grep -c can include filenames on error)
  UNDATED=$(echo "$UNDATED" | tr -cd '0-9')
  UNDATED=${UNDATED:-0}
  if [ "$UNDATED" -gt 0 ]; then
    echo "| undated | $UNDATED |"
  fi
  echo ""
done

# Entries >= 6 months old
echo "### A2.2: Entries ≥ 6 months old (introduction_date < $SIX_MONTHS_AGO)"
echo ""

OLD_COUNT=$(awk -F'\t' -v cutoff="$SIX_MONTHS_AGO" '$2 != "undated" && $2 < cutoff' "$A2_RAW" | wc -l)
TOTAL_DATED=$(awk -F'\t' '$2 != "undated"' "$A2_RAW" | wc -l)
echo "**Old entries (≥6mo):** $OLD_COUNT / $TOTAL_DATED dated entries"
echo ""

# Old-and-untouched subset (both introduction_date and last_modified are old)
OLD_UNTOUCHED=$(awk -F'\t' -v cutoff="$SIX_MONTHS_AGO" '
  $2 != "undated" && $2 < cutoff && $3 != "unknown" && $3 < cutoff
' "$A2_RAW" | wc -l)
OLD_MAINTAINED=$(awk -F'\t' -v cutoff="$SIX_MONTHS_AGO" '
  $2 != "undated" && $2 < cutoff && $3 != "unknown" && $3 >= cutoff
' "$A2_RAW" | wc -l)
echo "- **Old + untouched:** $OLD_UNTOUCHED (both dates old)"
echo "- **Old + maintained:** $OLD_MAINTAINED (old intro, recent last_modified)"
echo ""

# ============================================================
# AXIS 3 — Supersession Analysis
# ============================================================
echo "## Axis 3 — Supersession Analysis"
echo ""

PATTERNS='supersedes|superseded by|replaced by|deprecated by|see instead|no longer applies|use .* instead|retired in favor of'

echo "### A3: Supersession markers found"
echo ""

MARKER_FILES=$(mktemp /tmp/a3-markers-XXXXXX.txt)

for repo in mika mika-platform mika-cloud mika-skills; do
  ROOT="${CORPUS_PATHS[$repo]}"
  [ -d "$ROOT" ] || continue

  { grep -r -i -E "$PATTERNS" "$ROOT" --include='*.md' -l 2>/dev/null || true; } \
    | while read -r match_file; do
        echo "$repo	$match_file"
      done
done > "$MARKER_FILES"

MARKER_COUNT=$(wc -l < "$MARKER_FILES")
echo "**Files with supersession markers:** $MARKER_COUNT"
echo ""

if [ "$MARKER_COUNT" -gt 0 ]; then
  echo "| Repo | File | Marker line(s) |"
  echo "|------|------|----------------|"

  while IFS=$'\t' read -r repo match_file; do
    BASENAME=$(basename "$match_file")
    MARKER_LINES=$(grep -i -E "$PATTERNS" "$match_file" 2>/dev/null | head -3 | sed 's/|/\\|/g; s/$/; /' | tr -d '\n')
    echo "| $repo | $BASENAME | ${MARKER_LINES%%; } |"
  done < "$MARKER_FILES"
  echo ""

  # Build supersession chains
  echo "### A3.1: Supersession chain analysis"
  echo ""

  CHAIN_COUNT=0
  ORPHAN_COUNT=0
  while IFS=$'\t' read -r repo match_file; do
    # Extract target references — use process substitution to avoid subshell
    # (piped `while` loses variable mutations; process substitution keeps them)
    while read -r line; do
      # Try to extract a filename or path reference from the marker line
      TARGET=$(echo "$line" | grep -oE '[a-zA-Z0-9_-]+\.md' | head -1 || true)
      if [ -n "$TARGET" ]; then
        # Search for target across all corpora
        FOUND=false
        for search_repo in mika mika-platform mika-cloud mika-skills; do
          SEARCH_ROOT="${CORPUS_PATHS[$search_repo]}"
          [ -d "$SEARCH_ROOT" ] || continue
          if [ "$(find "$SEARCH_ROOT" -name "$TARGET" -type f 2>/dev/null | head -1)" != "" ]; then
            FOUND=true
            break
          fi
        done
        SOURCE=$(basename "$match_file")
        if [ "$FOUND" = true ]; then
          echo "- **Chain:** \`$SOURCE\` → \`$TARGET\` (target found in $search_repo) ✓"
          CHAIN_COUNT=$((CHAIN_COUNT + 1))
        else
          echo "- **Orphan:** \`$SOURCE\` → \`$TARGET\` (target NOT found) ✗"
          ORPHAN_COUNT=$((ORPHAN_COUNT + 1))
        fi
      fi
    done < <(grep -i -E "$PATTERNS" "$match_file" 2>/dev/null || true)
  done < "$MARKER_FILES"

  echo ""
  echo "**Chains with valid targets:** $CHAIN_COUNT"
  echo "**Orphan markers (target missing):** $ORPHAN_COUNT"
else
  echo "No explicit supersession markers found in any corpus."
fi
echo ""

rm -f "$MARKER_FILES"

# ============================================================
# AXIS 4 — Topic-Cluster Overlap (KG-based, per architect F1)
# ============================================================
echo "## Axis 4 — Topic-Cluster Overlap (KG-based)"
echo ""
echo "Methodology: KG topic-clustering via kg_subject_resolutions (zero LLM cost)."
echo "Per architect F1: replaces 25-pair LLM contradiction scheme."
echo ""

# Pre-flight: check resolution coverage
RESOLUTION_COUNT=$(sqlite3 "$DB" "
  SELECT COUNT(*)
  FROM kg_subject_resolutions
  WHERE agent_id = '$AGENT_ID';
")
echo "**KG subject resolutions (mika-arch):** $RESOLUTION_COUNT"
echo ""

if [ "$RESOLUTION_COUNT" -lt 100 ]; then
  echo "WARN: < 100 resolutions cross-corpus — primary KG clustering path unreliable."
  echo "Falling back to problem_type frontmatter overlap (no LLM cost)."
  echo ""

  # Fallback: find docs sharing the same problem_type
  echo "### A4 fallback: problem_type overlap"
  echo ""
  echo "| problem_type | Doc count | Sample docs |"
  echo "|-------------|-----------|-------------|"

  for repo in mika mika-platform mika-cloud mika-skills; do
    ROOT="${CORPUS_PATHS[$repo]}"
    [ -d "$ROOT" ] || continue
    find "$ROOT" -name '*.md' -type f 2>/dev/null
  done | while read -r f; do
    PTYPE=$(awk '/^problem_type:/{print $2; exit}' "$f" 2>/dev/null || true)
    if [ -n "$PTYPE" ]; then
      echo "$PTYPE	$(basename "$f")"
    fi
  done | sort | awk -F'\t' '
    {
      ptype[$1]++
      if (samples[$1] == "") samples[$1] = $2
      else if (split(samples[$1], _, ",") < 3) samples[$1] = samples[$1] "," $2
    }
    END {
      for (p in ptype) if (ptype[p] >= 2)
        printf "| %s | %d | %s |\n", p, ptype[p], samples[p]
    }
  ' | sort -t'|' -k3 -rn
  echo ""

else
  # Primary path: KG topic clustering via kg_subject_resolutions
  echo "### A4.1: Topic clusters (docs sharing ≥3 domain entities)"
  echo ""
  echo '```'
  sqlite3 -header -column "$DB" "
    WITH doc_entities AS (
      SELECT
        c.source_doc_path,
        c.docs_root_hash,
        sr.domain_entity_id
      FROM kg_chunks c
      JOIN kg_chunk_subjects cs
        ON cs.chunk_id = c.id
       AND cs.docs_root_hash = c.docs_root_hash
      JOIN kg_subject_resolutions sr
        ON sr.subject_entity_id = cs.subject_entity_id
       AND sr.agent_id = '$AGENT_ID'
      GROUP BY c.source_doc_path, c.docs_root_hash, sr.domain_entity_id
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
     AND a.source_doc_path < b.source_doc_path
    GROUP BY a.source_doc_path, b.source_doc_path
    HAVING shared_entities >= 3
    ORDER BY shared_entities DESC
    LIMIT 50;
  " 2>&1 || echo "(query returned no rows or error)"
  echo '```'
  echo ""

  # Count total clusters for summary
  CLUSTER_COUNT=$(sqlite3 "$DB" "
    WITH doc_entities AS (
      SELECT
        c.source_doc_path,
        c.docs_root_hash,
        sr.domain_entity_id
      FROM kg_chunks c
      JOIN kg_chunk_subjects cs
        ON cs.chunk_id = c.id
       AND cs.docs_root_hash = c.docs_root_hash
      JOIN kg_subject_resolutions sr
        ON sr.subject_entity_id = cs.subject_entity_id
       AND sr.agent_id = '$AGENT_ID'
      GROUP BY c.source_doc_path, c.docs_root_hash, sr.domain_entity_id
    )
    SELECT COUNT(*) FROM (
      SELECT
        a.source_doc_path,
        b.source_doc_path,
        COUNT(*) AS shared_entities
      FROM doc_entities a
      JOIN doc_entities b
        ON a.domain_entity_id = b.domain_entity_id
       AND a.source_doc_path < b.source_doc_path
      GROUP BY a.source_doc_path, b.source_doc_path
      HAVING shared_entities >= 3
    );
  ")
  echo "**Total doc pairs sharing ≥3 domain entities:** $CLUSTER_COUNT"
  echo ""

  # Show shared entities for top 10 pairs
  echo "### A4.2: Shared entity details for top 10 pairs"
  echo ""

  sqlite3 "$DB" "
    WITH doc_entities AS (
      SELECT
        c.source_doc_path,
        c.docs_root_hash,
        sr.domain_entity_id
      FROM kg_chunks c
      JOIN kg_chunk_subjects cs
        ON cs.chunk_id = c.id
       AND cs.docs_root_hash = c.docs_root_hash
      JOIN kg_subject_resolutions sr
        ON sr.subject_entity_id = cs.subject_entity_id
       AND sr.agent_id = '$AGENT_ID'
      GROUP BY c.source_doc_path, c.docs_root_hash, sr.domain_entity_id
    ),
    pairs AS (
      SELECT
        a.source_doc_path AS doc_a,
        b.source_doc_path AS doc_b,
        a.domain_entity_id,
        COUNT(*) OVER (PARTITION BY a.source_doc_path, b.source_doc_path) AS shared_count
      FROM doc_entities a
      JOIN doc_entities b
        ON a.domain_entity_id = b.domain_entity_id
       AND a.source_doc_path < b.source_doc_path
    )
    SELECT
      p.doc_a,
      p.doc_b,
      p.shared_count,
      e.name AS shared_entity,
      e.type AS entity_type
    FROM pairs p
    JOIN kg_entities e ON e.id = p.domain_entity_id
    WHERE p.shared_count >= 3
    ORDER BY p.shared_count DESC, p.doc_a, p.doc_b
    LIMIT 60;
  " 2>/dev/null | while IFS='|' read -r doc_a doc_b count entity etype; do
    echo "- **$(basename "$doc_a")** ↔ **$(basename "$doc_b")** ($count shared): $entity ($etype)"
  done
  echo ""
fi

# ============================================================
# AXIS 5 — KG-Resolution Drift (per-corpus, mika-arch)
# ============================================================
echo "## Axis 5 — KG-Resolution Drift"
echo ""
echo "Methodology: resolution-rate drift (no_match rate per age-quartile)."
echo "Per architect NF1: resolution-rate, not temporal."
echo ""

echo "### A5: no_match rate by entry-age quartile"
echo ""
echo '```'
sqlite3 -header -column "$DB" "
  WITH entries AS (
    SELECT
      c.source_doc_path,
      c.docs_root_hash,
      DATE(MIN(c.created_at)) AS first_chunked,
      cs.subject_entity_id
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs
      ON cs.chunk_id = c.id
     AND cs.docs_root_hash = c.docs_root_hash
    GROUP BY c.source_doc_path, cs.subject_entity_id
  ),
  resolved AS (
    SELECT
      e.source_doc_path,
      e.first_chunked,
      e.docs_root_hash,
      COALESCE(rl.outcome, 'unresolved') AS outcome
    FROM entries e
    LEFT JOIN kg_resolutions_log rl
      ON rl.subject_entity_id = e.subject_entity_id
     AND rl.agent_id = '$AGENT_ID'
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
" 2>&1 || echo "(query returned no rows or error)"
echo '```'
echo ""

# Per-corpus breakdown
echo "### A5.1: no_match rate per corpus per quartile"
echo ""
echo '```'
sqlite3 -header -column "$DB" "
  WITH entries AS (
    SELECT
      c.source_doc_path,
      c.docs_root_hash,
      c.docs_root,
      DATE(MIN(c.created_at)) AS first_chunked,
      cs.subject_entity_id
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs
      ON cs.chunk_id = c.id
     AND cs.docs_root_hash = c.docs_root_hash
    GROUP BY c.source_doc_path, cs.subject_entity_id
  ),
  resolved AS (
    SELECT
      e.source_doc_path,
      e.first_chunked,
      e.docs_root,
      COALESCE(rl.outcome, 'unresolved') AS outcome
    FROM entries e
    LEFT JOIN kg_resolutions_log rl
      ON rl.subject_entity_id = e.subject_entity_id
     AND rl.agent_id = '$AGENT_ID'
  )
  SELECT
    docs_root,
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
  GROUP BY docs_root, age_quartile
  ORDER BY docs_root, age_quartile;
" 2>&1 || echo "(query returned no rows or error)"
echo '```'
echo ""

# Drift summary (fresh vs old no_match rate difference)
FRESH_RATE=$(sqlite3 "$DB" "
  WITH entries AS (
    SELECT c.source_doc_path, DATE(MIN(c.created_at)) AS first_chunked, cs.subject_entity_id
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    GROUP BY c.source_doc_path, cs.subject_entity_id
  ),
  resolved AS (
    SELECT e.first_chunked, COALESCE(rl.outcome, 'unresolved') AS outcome
    FROM entries e
    LEFT JOIN kg_resolutions_log rl ON rl.subject_entity_id = e.subject_entity_id AND rl.agent_id = '$AGENT_ID'
  )
  SELECT ROUND(100.0 * SUM(CASE WHEN outcome = 'no_match' THEN 1 ELSE 0 END) / NULLIF(COUNT(*), 0), 1)
  FROM resolved
  WHERE first_chunked >= '2026-05-01';
" 2>/dev/null || echo "null")

OLD_RATE=$(sqlite3 "$DB" "
  WITH entries AS (
    SELECT c.source_doc_path, DATE(MIN(c.created_at)) AS first_chunked, cs.subject_entity_id
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    GROUP BY c.source_doc_path, cs.subject_entity_id
  ),
  resolved AS (
    SELECT e.first_chunked, COALESCE(rl.outcome, 'unresolved') AS outcome
    FROM entries e
    LEFT JOIN kg_resolutions_log rl ON rl.subject_entity_id = e.subject_entity_id AND rl.agent_id = '$AGENT_ID'
  )
  SELECT ROUND(100.0 * SUM(CASE WHEN outcome = 'no_match' THEN 1 ELSE 0 END) / NULLIF(COUNT(*), 0), 1)
  FROM resolved
  WHERE first_chunked < '2025-11-01';
" 2>/dev/null || echo "null")

echo "### A5 Drift Summary"
echo ""
echo "- **Fresh (<1mo) no_match rate:** ${FRESH_RATE:-N/A}%"
echo "- **Old (≥6mo) no_match rate:** ${OLD_RATE:-N/A}%"

if [ "$FRESH_RATE" != "null" ] && [ "$OLD_RATE" != "null" ] && [ -n "$FRESH_RATE" ] && [ -n "$OLD_RATE" ]; then
  # Calculate drift (old - fresh) using awk for floating point
  DRIFT=$(awk "BEGIN { printf \"%.1f\", $OLD_RATE - $FRESH_RATE }")
  echo "- **Drift (old→fresh):** ${DRIFT}pp"

  # Verdict
  DRIFT_ABS=$(awk "BEGIN { d = $OLD_RATE - $FRESH_RATE; printf \"%.1f\", (d < 0 ? -d : d) }")
  if awk "BEGIN { exit !($DRIFT_ABS < 10) }"; then
    echo "- **A5 verdict:** PASS (drift < 10pp)"
  elif awk "BEGIN { exit !($DRIFT_ABS >= 25) }"; then
    echo "- **A5 verdict:** FAIL (drift ≥ 25pp)"
  else
    echo "- **A5 verdict:** MIXED (drift 10–25pp)"
  fi
else
  echo "- **Drift:** cannot compute (insufficient data in one or both quartiles)"
  echo "- **A5 verdict:** INSUFFICIENT DATA"
fi
echo ""

# ============================================================
# AXIS 6 — mika-arch Consumption Signal
# ============================================================
echo "## Axis 6 — mika-arch Consumption Signal"
echo ""

# A6.0: Pre-flight check
echo "### A6.0: Pre-flight — path visibility in tool_calls.output"
echo ""

PREFLIGHT=$(sqlite3 "$DB" "
  SELECT
    SUM(CASE WHEN output LIKE '%source_doc_path%' OR output LIKE '%docs/solutions/%' THEN 1 ELSE 0 END) AS with_path_hint,
    COUNT(*) AS total
  FROM tool_calls
  WHERE agent_id = '$AGENT_ID'
    AND tool_name = 'query_knowledge_graph'
    AND created_at >= datetime('now', '-30 days');
")
PREFLIGHT_WITH_PATH=$(echo "$PREFLIGHT" | cut -d'|' -f1)
PREFLIGHT_TOTAL=$(echo "$PREFLIGHT" | cut -d'|' -f2)

echo "- **Total KG queries (30d):** $PREFLIGHT_TOTAL"
echo "- **With path hints in output:** $PREFLIGHT_WITH_PATH"
echo ""

if [ "$PREFLIGHT_TOTAL" -eq 0 ]; then
  echo "**A6 verdict:** INSUFFICIENT DATA — mika-arch hasn't queried KG in 30 days."
  echo ""
elif [ "$PREFLIGHT_WITH_PATH" -eq 0 ] || awk "BEGIN { exit !($PREFLIGHT_WITH_PATH / $PREFLIGHT_TOTAL < 0.5) }" 2>/dev/null; then
  echo "**A6 primary path unreliable** (< 50% of outputs contain doc paths)."
  echo "Reporting as: unavailable; needs schema enrichment to trace KG queries → chunk provenance."
  echo ""
  echo "**A6 verdict:** UNAVAILABLE"
  echo ""
else
  # A6 primary path: extract doc paths from tool_calls.output
  echo "### A6.1: Doc paths surfaced in KG queries (last 30 days)"
  echo ""

  A6_RAW=$(mktemp /tmp/a6-raw-XXXXXX.txt)

  sqlite3 "$DB" "
    SELECT output
    FROM tool_calls
    WHERE agent_id = '$AGENT_ID'
      AND tool_name = 'query_knowledge_graph'
      AND created_at >= datetime('now', '-30 days')
      AND (output LIKE '%source_doc_path%' OR output LIKE '%docs/solutions/%');
  " 2>/dev/null | { grep -oE 'docs/solutions/[^"]+\.md' || true; } | sort | uniq -c | sort -rn > "$A6_RAW"

  TOTAL_PATHS=$(awk '{sum += $1} END {print sum+0}' "$A6_RAW")
  echo "**Total doc-path mentions in KG query outputs:** $TOTAL_PATHS"
  echo ""

  if [ "$TOTAL_PATHS" -gt 0 ]; then
    echo "| Count | Doc path | Intro date | Age class |"
    echo "|-------|----------|-----------|-----------|"

    OLD_PATH_MENTIONS=0
    TOTAL_PATH_MENTIONS=0
    while read -r count path; do
      TOTAL_PATH_MENTIONS=$((TOTAL_PATH_MENTIONS + count))
      # Look up introduction date from A2 raw data
      INTRO_DATE=$({ grep "$path" "$A2_RAW" 2>/dev/null || true; } | head -1 | awk -F'\t' '{print $2}')
      [ -z "$INTRO_DATE" ] && INTRO_DATE="undated"
      if [ "$INTRO_DATE" != "undated" ] && [ "$INTRO_DATE" \< "$SIX_MONTHS_AGO" ]; then
        AGE_CLASS="≥6mo"
        OLD_PATH_MENTIONS=$((OLD_PATH_MENTIONS + count))
      elif [ "$INTRO_DATE" = "undated" ]; then
        AGE_CLASS="undated"
      else
        AGE_CLASS="<6mo"
      fi
      echo "| $count | $(basename "$path") | $INTRO_DATE | $AGE_CLASS |"
    done < "$A6_RAW"
    echo ""

    OLD_FRACTION=$(awk "BEGIN { printf \"%.1f\", 100.0 * $OLD_PATH_MENTIONS / $TOTAL_PATH_MENTIONS }")
    echo "**Old-doc fraction (≥6mo):** $OLD_PATH_MENTIONS / $TOTAL_PATH_MENTIONS mentions = ${OLD_FRACTION}%"
    echo ""

    if awk "BEGIN { exit !($OLD_FRACTION < 30) }"; then
      echo "**A6 verdict:** PASS (old-doc fraction < 30%)"
    elif awk "BEGIN { exit !($OLD_FRACTION > 60) }"; then
      echo "**A6 verdict:** FAIL (old-doc fraction > 60%)"
    else
      echo "**A6 verdict:** MIXED (old-doc fraction 30-60%)"
    fi
  else
    echo "**A6 verdict:** INSUFFICIENT DATA — path extraction returned 0 paths despite output containing path hints."
  fi

  rm -f "$A6_RAW"
fi
echo ""

# ============================================================
# SUMMARY
# ============================================================
echo "## Summary"
echo ""
echo "| Axis | Description | Status |"
echo "|------|-------------|--------|"
echo "| A1 | Inventory | $TOTAL_ALL entries across repos |"
echo "| A2 | Age distribution | $OLD_COUNT / $TOTAL_DATED ≥6mo |"
echo "| A3 | Supersession markers | $MARKER_COUNT files with markers |"
echo "| A4 | Topic clustering | ${CLUSTER_COUNT:-fallback} pairs |"
echo "| A5 | KG-resolution drift | Fresh=${FRESH_RATE:-N/A}% Old=${OLD_RATE:-N/A}% |"
echo "| A6 | Consumption signal | $PREFLIGHT_TOTAL queries (30d) |"
echo ""
echo "---"
echo ""
echo "Investigation complete. See finding doc for Axis 7 synthesis + outcome path declaration."
