#!/usr/bin/env bash
# investigate-kg-1027.sh — Post-axis-deploy KG sync verification (mika#1027)
#
# Investigation script: reads KG state from SQLite + structured server logs.
# Six layers: L1 chunking, L2 extraction, L3 relationships, L4 resolution,
# L5 resolver tick health, L6 domain graph health.
#
# Usage:
#   bash scripts/investigate-kg-1027.sh [LOG_FILE]
#
# Environment:
#   MIKA_SPIRIT_LOG_FILE — server log path (default: /var/log/mika/server.log)
#   MIKA_DB              — SQLite DB path  (default: ~/.mika/data/mika.db)
#
# This script is read-only and idempotent — safe to re-run.
# Per-ticket one-shot (mika#1027); methodology extraction is a separate follow-up.

set -euo pipefail

# --- Configuration ---
DB="${MIKA_DB:-$HOME/.mika/data/mika.db}"
LOG_FILE="${1:-${MIKA_SPIRIT_LOG_FILE:-/var/log/mika/server.log}}"
AGENT_ID="mika-arch"  # sole KG consumer per mika#800
TICK_CUTOFF="2026-05-07T16:00:00Z"
DOMAIN_CUTOFF="2026-05-08T08:00:00Z"
MIN_SCHEMA_VERSION=27

# Target docs (LIKE patterns for source_doc_path matching)
DOC1_PATTERN="%silent-mode-summary-budget-cap%"
DOC1_LABEL="silent-mode-summary-budget-cap-2026-05-08.md"
DOC2_PATTERN="%summarizer-factual-assertion%"
DOC2_LABEL="summarizer-factual-assertion-reform-2026-05-08.md"
DOC3_PATTERN="%verify-on-write-citation%"
DOC3_LABEL="verify-on-write-citation-discipline-2026-05-07.md"

WHERE_CLAUSE="(source_doc_path LIKE '$DOC1_PATTERN' OR source_doc_path LIKE '$DOC2_PATTERN' OR source_doc_path LIKE '$DOC3_PATTERN')"

# --- Pre-flight checks ---
echo "# KG Post-Axis-Deploy Sync Verification — mika#1027"
echo ""
echo "**Timestamp:** $(date -u +'%Y-%m-%dT%H:%M:%SZ')"
echo "**DB:** $DB"
echo "**Log:** $LOG_FILE"
echo "**Agent:** $AGENT_ID"
echo ""

if [ ! -f "$DB" ]; then
  echo "ERROR: Database not found at $DB"
  exit 1
fi

SCHEMA_VERSION=$(sqlite3 "$DB" "SELECT MAX(version) FROM schema_version;")
echo "**Schema version:** $SCHEMA_VERSION"
if [ "$SCHEMA_VERSION" -lt "$MIN_SCHEMA_VERSION" ]; then
  echo "ERROR: Schema version $SCHEMA_VERSION < $MIN_SCHEMA_VERSION — pre-v27 schema; queries incompatible"
  exit 1
fi
echo ""

LOG_ACCESSIBLE=true
if [ ! -f "$LOG_FILE" ]; then
  echo "WARN: Log file not found at $LOG_FILE — L5/L6 log probes will be skipped"
  LOG_ACCESSIBLE=false
elif [ ! -r "$LOG_FILE" ]; then
  echo "WARN: Log file not readable at $LOG_FILE — L5/L6 log probes will be skipped"
  echo "Recovery: re-run with sudo or update log file permissions"
  LOG_ACCESSIBLE=false
fi
echo ""

# --- Helper: per-doc query runner ---
run_per_doc_query() {
  local label="$1"
  local query="$2"
  echo "### $label"
  echo ""
  echo '```'
  sqlite3 -header -column "$DB" "$query" 2>&1 || echo "(query returned no rows or error)"
  echo '```'
  echo ""
}

# ============================================================
# LAYER 1 — Chunking (kg_chunks)
# ============================================================
echo "## Layer 1 — Chunking (\`kg_chunks\`)"
echo ""

L1_QUERY="
SELECT
  source_doc_path,
  docs_root_hash,
  COUNT(*)        AS chunks,
  source_doc_hash,
  MAX(created_at) AS last_chunked
FROM kg_chunks
WHERE $WHERE_CLAUSE
GROUP BY source_doc_path;
"
run_per_doc_query "L1: Chunk presence per doc" "$L1_QUERY"

# Count per doc for verdict
L1_DOC1=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_chunks WHERE source_doc_path LIKE '$DOC1_PATTERN';")
L1_DOC2=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_chunks WHERE source_doc_path LIKE '$DOC2_PATTERN';")
L1_DOC3=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_chunks WHERE source_doc_path LIKE '$DOC3_PATTERN';")

for doc_label in "$DOC1_LABEL:$L1_DOC1" "$DOC2_LABEL:$L1_DOC2" "$DOC3_LABEL:$L1_DOC3"; do
  label="${doc_label%%:*}"
  count="${doc_label##*:}"
  if [ "$count" -gt 0 ]; then
    echo "- **$label:** PASS ($count chunks)"
  else
    echo "- **$label:** FAIL (0 chunks)"
  fi
done
echo ""

# ============================================================
# LAYER 2 — Subject Extraction (kg_chunk_subjects × kg_subject_entities)
# ============================================================
echo "## Layer 2 — Subject Extraction (\`kg_chunk_subjects\` × \`kg_subject_entities\`)"
echo ""

L2_QUERY="
SELECT
  c.source_doc_path,
  COUNT(DISTINCT cs.subject_entity_id) AS entities,
  COUNT(*)                              AS provenance_rows
FROM kg_chunks c
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.docs_root_hash
WHERE $WHERE_CLAUSE
GROUP BY c.source_doc_path;
"
run_per_doc_query "L2: Entity count per doc" "$L2_QUERY"

# Check extraction status (the actual bottleneck if L2 is empty)
echo "### L2 pre-check: extraction status per doc (\`kg_extractions\`)"
echo ""
echo '```'
sqlite3 -header -column "$DB" "
SELECT
  source_doc_path,
  docs_root_hash,
  entities_extracted,
  relationships_extracted,
  extraction_model,
  created_at
FROM kg_extractions
WHERE $WHERE_CLAUSE;
" 2>&1 || echo "(no extraction records — docs are pending extraction)"
echo '```'
echo ""

# Check overall extraction backlog
EXTRACTION_PENDING=$(sqlite3 "$DB" "
SELECT COUNT(DISTINCT c.source_doc_path)
FROM kg_chunks c
WHERE NOT EXISTS (
  SELECT 1 FROM kg_extractions e
  WHERE e.docs_root_hash = c.docs_root_hash
    AND e.source_doc_path = c.source_doc_path
);
")
echo "**Extraction backlog (all corpora):** $EXTRACTION_PENDING docs pending extraction"
echo ""

# Also show sample entities per doc for context
echo "### L2 sample: entity names per doc (top 10 each)"
echo ""
for doc_pattern in "$DOC1_PATTERN" "$DOC2_PATTERN" "$DOC3_PATTERN"; do
  echo '```'
  sqlite3 -header -column "$DB" "
    SELECT DISTINCT se.name, se.type
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    JOIN kg_subject_entities se ON se.id = cs.subject_entity_id AND se.docs_root_hash = cs.docs_root_hash
    WHERE c.source_doc_path LIKE '$doc_pattern'
    LIMIT 10;
  " 2>&1 || echo "(no entities)"
  echo '```'
  echo ""
done

# Counts for verdict
L2_DOC1=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT cs.subject_entity_id) FROM kg_chunks c JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$DOC1_PATTERN';")
L2_DOC2=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT cs.subject_entity_id) FROM kg_chunks c JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$DOC2_PATTERN';")
L2_DOC3=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT cs.subject_entity_id) FROM kg_chunks c JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$DOC3_PATTERN';")

for doc_info in "$DOC1_PATTERN|$DOC1_LABEL|$L2_DOC1" "$DOC2_PATTERN|$DOC2_LABEL|$L2_DOC2" "$DOC3_PATTERN|$DOC3_LABEL|$L2_DOC3"; do
  pattern=$(echo "$doc_info" | cut -d'|' -f1)
  label=$(echo "$doc_info" | cut -d'|' -f2)
  count=$(echo "$doc_info" | cut -d'|' -f3)
  # Check if extraction has run for this doc
  extracted=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_extractions WHERE source_doc_path LIKE '$pattern';")
  if [ "$count" -gt 0 ]; then
    echo "- **$label:** PASS ($count distinct entities)"
  elif [ "$extracted" -eq 0 ]; then
    echo "- **$label:** PENDING (extraction not yet run — doc is in extraction queue)"
  else
    echo "- **$label:** FAIL (0 entities despite extraction completed — subject extractor drop)"
  fi
done
echo ""

# ============================================================
# LAYER 3 — Subject Relationships (kg_chunk_subject_relationships)
# ============================================================
echo "## Layer 3 — Subject Relationships (\`kg_chunk_subject_relationships\`)"
echo ""

L3_QUERY="
SELECT
  c.source_doc_path,
  COUNT(DISTINCT csr.subject_relationship_id) AS triples,
  COUNT(*)                                     AS provenance_rows
FROM kg_chunks c
JOIN kg_chunk_subject_relationships csr
  ON csr.chunk_id = c.id
 AND csr.docs_root_hash = c.docs_root_hash
WHERE $WHERE_CLAUSE
GROUP BY c.source_doc_path;
"
run_per_doc_query "L3: Fact-triple count per doc" "$L3_QUERY"

# Sample triples
echo "### L3 sample: relationship triples per doc (top 5 each)"
echo ""
for doc_pattern in "$DOC1_PATTERN" "$DOC2_PATTERN" "$DOC3_PATTERN"; do
  echo '```'
  sqlite3 -header -column "$DB" "
    SELECT DISTINCT se_from.name AS subject, sr.type AS predicate, se_to.name AS object
    FROM kg_chunks c
    JOIN kg_chunk_subject_relationships csr ON csr.chunk_id = c.id AND csr.docs_root_hash = c.docs_root_hash
    JOIN kg_subject_relationships sr ON sr.id = csr.subject_relationship_id AND sr.docs_root_hash = csr.docs_root_hash
    JOIN kg_subject_entities se_from ON se_from.id = sr.from_entity_id AND se_from.docs_root_hash = sr.docs_root_hash
    JOIN kg_subject_entities se_to ON se_to.id = sr.to_entity_id AND se_to.docs_root_hash = sr.docs_root_hash
    WHERE c.source_doc_path LIKE '$doc_pattern'
    LIMIT 5;
  " 2>&1 || echo "(no triples)"
  echo '```'
  echo ""
done

# Counts for verdict
L3_DOC1=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT csr.subject_relationship_id) FROM kg_chunks c JOIN kg_chunk_subject_relationships csr ON csr.chunk_id = c.id AND csr.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$DOC1_PATTERN';")
L3_DOC2=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT csr.subject_relationship_id) FROM kg_chunks c JOIN kg_chunk_subject_relationships csr ON csr.chunk_id = c.id AND csr.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$DOC2_PATTERN';")
L3_DOC3=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT csr.subject_relationship_id) FROM kg_chunks c JOIN kg_chunk_subject_relationships csr ON csr.chunk_id = c.id AND csr.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$DOC3_PATTERN';")

for doc_info in "$DOC1_PATTERN|$DOC1_LABEL|$L3_DOC1" "$DOC2_PATTERN|$DOC2_LABEL|$L3_DOC2" "$DOC3_PATTERN|$DOC3_LABEL|$L3_DOC3"; do
  pattern=$(echo "$doc_info" | cut -d'|' -f1)
  label=$(echo "$doc_info" | cut -d'|' -f2)
  count=$(echo "$doc_info" | cut -d'|' -f3)
  extracted=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_extractions WHERE source_doc_path LIKE '$pattern';")
  if [ "$count" -gt 0 ]; then
    echo "- **$label:** PASS ($count distinct triples)"
  elif [ "$extracted" -eq 0 ]; then
    echo "- **$label:** PENDING (extraction not yet run)"
  else
    echo "- **$label:** FAIL (0 triples despite extraction completed — relationship extractor drop)"
  fi
done
echo ""

# ============================================================
# LAYER 4 — Resolution (kg_resolutions_log, agent-scoped)
# ============================================================
echo "## Layer 4 — Resolution (\`kg_resolutions_log\`, agent=$AGENT_ID)"
echo ""

L4_QUERY="
SELECT
  c.source_doc_path,
  rl.outcome,
  COUNT(*) AS rows
FROM kg_chunks c
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.docs_root_hash
JOIN kg_resolutions_log rl
  ON rl.subject_entity_id = cs.subject_entity_id
 AND rl.agent_id = '$AGENT_ID'
WHERE $WHERE_CLAUSE
GROUP BY c.source_doc_path, rl.outcome;
"
run_per_doc_query "L4: Resolution outcomes per doc" "$L4_QUERY"

# Cross-check: unresolved entities
echo "### L4 cross-check: unresolved entities (no resolution log row)"
echo ""
L4_XCHECK="
SELECT
  c.source_doc_path,
  COUNT(DISTINCT cs.subject_entity_id) AS unresolved_pending
FROM kg_chunks c
JOIN kg_chunk_subjects cs
  ON cs.chunk_id = c.id
 AND cs.docs_root_hash = c.docs_root_hash
WHERE NOT EXISTS (
  SELECT 1 FROM kg_resolutions_log rl
  WHERE rl.subject_entity_id = cs.subject_entity_id
    AND rl.agent_id = '$AGENT_ID'
)
  AND $WHERE_CLAUSE
GROUP BY c.source_doc_path;
"
echo '```'
sqlite3 -header -column "$DB" "$L4_XCHECK" 2>&1 || echo "(no unresolved entities — all resolved)"
echo '```'
echo ""

# Per-doc resolution counts + no_match rate check
for doc_info in "$DOC1_PATTERN|$DOC1_LABEL" "$DOC2_PATTERN|$DOC2_LABEL" "$DOC3_PATTERN|$DOC3_LABEL"; do
  pattern="${doc_info%%|*}"
  label="${doc_info##*|}"

  total_resolved=$(sqlite3 "$DB" "
    SELECT COUNT(DISTINCT cs.subject_entity_id)
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    JOIN kg_resolutions_log rl ON rl.subject_entity_id = cs.subject_entity_id AND rl.agent_id = '$AGENT_ID'
    WHERE c.source_doc_path LIKE '$pattern';
  ")
  no_match_count=$(sqlite3 "$DB" "
    SELECT COUNT(DISTINCT cs.subject_entity_id)
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    JOIN kg_resolutions_log rl ON rl.subject_entity_id = cs.subject_entity_id AND rl.agent_id = '$AGENT_ID'
    WHERE c.source_doc_path LIKE '$pattern'
      AND rl.outcome = 'no_match';
  ")
  unresolved=$(sqlite3 "$DB" "
    SELECT COUNT(DISTINCT cs.subject_entity_id)
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    WHERE NOT EXISTS (
      SELECT 1 FROM kg_resolutions_log rl
      WHERE rl.subject_entity_id = cs.subject_entity_id
        AND rl.agent_id = '$AGENT_ID'
    )
    AND c.source_doc_path LIKE '$pattern';
  ")
  total_entities=$(sqlite3 "$DB" "
    SELECT COUNT(DISTINCT cs.subject_entity_id)
    FROM kg_chunks c
    JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash
    WHERE c.source_doc_path LIKE '$pattern';
  ")

  # Check last_chunked vs last resolver tick for PENDING detection
  last_chunked=$(sqlite3 "$DB" "
    SELECT MAX(created_at) FROM kg_chunks WHERE source_doc_path LIKE '$pattern';
  ")

  # Check extraction status
  extracted=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_extractions WHERE source_doc_path LIKE '$pattern';")

  # Determine verdict
  if [ "$total_entities" -eq 0 ] && [ "$extracted" -eq 0 ]; then
    verdict="PENDING (extraction not yet run — awaiting extraction batch)"
  elif [ "$total_entities" -eq 0 ]; then
    verdict="FAIL (no entities at L2 despite extraction completed)"
  elif [ "$unresolved" -gt 0 ]; then
    verdict="PENDING ($unresolved/$total_entities entities unresolved — awaiting resolver tick)"
  elif [ "$total_resolved" -gt 0 ]; then
    if [ "$total_resolved" -gt 0 ] && [ "$no_match_count" -gt 0 ]; then
      no_match_pct=$(( no_match_count * 100 / total_resolved ))
      if [ "$no_match_pct" -gt 80 ]; then
        verdict="WARN (no_match rate ${no_match_pct}% > 80% threshold — check domain graph sparsity)"
      else
        verdict="PASS ($total_resolved resolved, $no_match_count no_match [${no_match_pct}%])"
      fi
    else
      verdict="PASS ($total_resolved resolved, 0 no_match)"
    fi
  else
    verdict="FAIL (0 resolutions despite entities present)"
  fi
  echo "- **$label:** $verdict (last_chunked: $last_chunked)"
done
echo ""

# ============================================================
# LAYER 5 — Resolver Tick Health (jq on structured server log)
# ============================================================
echo "## Layer 5 — Resolver Tick Health"
echo ""

if [ "$LOG_ACCESSIBLE" = false ]; then
  echo "SKIPPED: log file not accessible (see WARN above)"
  echo ""
else
  # Pre-filter with grep to handle mixed-format log files, then jq for structured parsing
  # (NF3: jq selectors against .event field — grep is only for pre-filtering non-JSON lines)
  TICK_LINES=$(grep -a "kg_resolver_tick" "$LOG_FILE" | grep "$AGENT_ID" || true)

  echo "### L5: Resolver tick events from $TICK_CUTOFF onward ($AGENT_ID)"
  echo ""
  echo '```'
  echo "$TICK_LINES" | jq -c '
    select(.timestamp > "'"$TICK_CUTOFF"'")
    | {
        ts: .timestamp,
        event: .event,
        agent_id: .agent_id,
        pending_after: .pending_after,
        llm_calls: .llm_calls,
        aborted_budget: .aborted_budget,
        per_corpus_attempted: .per_corpus_attempted
      }
  ' 2>/dev/null || echo "(no matching events)"
  echo '```'
  echo ""

  # Summary metrics
  TOTAL_TICKS=$(echo "$TICK_LINES" | jq -c '
    select(.event == "kg_resolver_tick.complete")
    | select(.timestamp > "'"$TICK_CUTOFF"'")
  ' 2>/dev/null | wc -l | tr -d '[:space:]')
  TOTAL_TICKS="${TOTAL_TICKS:-0}"

  ABORTED_TICKS=$(echo "$TICK_LINES" | jq -c '
    select(.event == "kg_resolver_tick.complete")
    | select(.timestamp > "'"$TICK_CUTOFF"'")
    | select(.aborted_budget == true)
  ' 2>/dev/null | wc -l | tr -d '[:space:]')
  ABORTED_TICKS="${ABORTED_TICKS:-0}"

  LATEST_PENDING=$(echo "$TICK_LINES" | jq -c '
    select(.event == "kg_resolver_tick.complete")
    | select(.timestamp > "'"$TICK_CUTOFF"'")
    | .pending_after
  ' 2>/dev/null | tail -1)
  LATEST_PENDING="${LATEST_PENDING:-unknown}"

  TICK_ERRORS=$(echo "$TICK_LINES" | jq -c '
    select(.event == "kg_resolver_tick.error")
    | select(.timestamp > "'"$TICK_CUTOFF"'")
  ' 2>/dev/null | wc -l | tr -d '[:space:]')
  TICK_ERRORS="${TICK_ERRORS:-0}"

  echo "### L5 Summary"
  echo ""
  echo "- **Total ticks since $TICK_CUTOFF:** $TOTAL_TICKS"
  echo "- **Ticks with aborted_budget=true:** $ABORTED_TICKS"
  echo "- **Latest pending_after:** $LATEST_PENDING"
  echo "- **Tick errors:** $TICK_ERRORS"

  if [ "$ABORTED_TICKS" -gt 0 ]; then
    echo "- **L5 verdict:** WARN — $ABORTED_TICKS tick(s) aborted by budget"
  elif [ "$TICK_ERRORS" -gt 0 ]; then
    echo "- **L5 verdict:** FAIL — $TICK_ERRORS tick error(s) detected"
  elif [ "$TOTAL_TICKS" -eq 0 ]; then
    echo "- **L5 verdict:** WARN — no resolver ticks found (server may not have been running)"
  else
    echo "- **L5 verdict:** PASS"
  fi
  echo ""

  # Per-corpus fairness check
  echo "### L5 Per-corpus fairness (mika#927)"
  echo ""
  echo '```'
  echo "$TICK_LINES" | jq -c '
    select(.event == "kg_resolver_tick.complete")
    | select(.timestamp > "'"$TICK_CUTOFF"'")
    | {ts: .timestamp, per_corpus_attempted: .per_corpus_attempted}
  ' 2>/dev/null || echo "(no per-corpus data)"
  echo '```'
  echo ""
fi

# ============================================================
# LAYER 6 — Domain Graph Health
# ============================================================
echo "## Layer 6 — Domain Graph Health"
echo ""

# L6a: domain rebuild events (from log)
if [ "$LOG_ACCESSIBLE" = true ]; then
  echo "### L6a: Domain rebuild events post-deploy (since $DOMAIN_CUTOFF)"
  echo ""
  echo '```'
  REBUILD_LINES=$(grep -a "domain_rebuild" "$LOG_FILE" || true)
  echo "$REBUILD_LINES" | jq -c '
    select(.timestamp > "'"$DOMAIN_CUTOFF"'")
    | {ts: .timestamp, event: .event, trace_id: .trace_id, added: .added, updated: .updated, removed: .removed}
  ' 2>/dev/null || echo "(no domain rebuild events found)"
  echo '```'
  echo ""
else
  echo "### L6a: SKIPPED (log not accessible)"
  echo ""
fi

# L6b: entity-count snapshot (from DB)
echo "### L6b: Domain graph entity snapshot"
echo ""
echo '```'
sqlite3 -header -column "$DB" "
SELECT COUNT(*) AS total_entities FROM kg_entities;
"
echo ""
sqlite3 -header -column "$DB" "
SELECT COUNT(*) AS total_relationships FROM kg_relationships;
"
echo ""
sqlite3 -header -column "$DB" "
SELECT type, COUNT(*) AS n
FROM kg_entities
GROUP BY type
ORDER BY n DESC;
"
echo '```'
echo ""

TOTAL_ENTITIES=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_entities;")
if [ "$TOTAL_ENTITIES" -eq 0 ]; then
  echo "- **L6b verdict:** FAIL — zero domain entities (catastrophic rebuild failure)"
else
  echo "- **L6b verdict:** PASS ($TOTAL_ENTITIES domain entities)"
fi
echo ""

# ============================================================
# PER-DOC ROLLUP MATRIX
# ============================================================
echo "## Per-Doc Rollup Matrix"
echo ""
echo "| Doc | L1 chunks | L2 entities | L3 triples | L4 resolutions | Verdict |"
echo "|-----|-----------|-------------|------------|----------------|---------|"

for doc_info in "$DOC1_PATTERN|$DOC1_LABEL" "$DOC2_PATTERN|$DOC2_LABEL" "$DOC3_PATTERN|$DOC3_LABEL"; do
  pattern="${doc_info%%|*}"
  label="${doc_info##*|}"

  l1=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_chunks WHERE source_doc_path LIKE '$pattern';")
  l2=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT cs.subject_entity_id) FROM kg_chunks c JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$pattern';")
  l3=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT csr.subject_relationship_id) FROM kg_chunks c JOIN kg_chunk_subject_relationships csr ON csr.chunk_id = c.id AND csr.docs_root_hash = c.docs_root_hash WHERE c.source_doc_path LIKE '$pattern';")
  l4_resolved=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT cs.subject_entity_id) FROM kg_chunks c JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash JOIN kg_resolutions_log rl ON rl.subject_entity_id = cs.subject_entity_id AND rl.agent_id = '$AGENT_ID' WHERE c.source_doc_path LIKE '$pattern';")
  l4_pending=$(sqlite3 "$DB" "SELECT COUNT(DISTINCT cs.subject_entity_id) FROM kg_chunks c JOIN kg_chunk_subjects cs ON cs.chunk_id = c.id AND cs.docs_root_hash = c.docs_root_hash WHERE NOT EXISTS (SELECT 1 FROM kg_resolutions_log rl WHERE rl.subject_entity_id = cs.subject_entity_id AND rl.agent_id = '$AGENT_ID') AND c.source_doc_path LIKE '$pattern';")

  # Check extraction status
  rollup_extracted=$(sqlite3 "$DB" "SELECT COUNT(*) FROM kg_extractions WHERE source_doc_path LIKE '$pattern';")

  # Determine per-doc verdict (per plan NF4: PENDING when extraction hasn't run yet)
  if [ "$l1" -eq 0 ]; then
    verdict="absent-bug-flagged"
  elif [ "$rollup_extracted" -eq 0 ]; then
    verdict="present (PENDING: extraction not yet run)"
  elif [ "$l2" -eq 0 ] || [ "$l3" -eq 0 ]; then
    verdict="partial-bug-flagged"
  elif [ "$l4_pending" -gt 0 ]; then
    verdict="present (PENDING: $l4_pending unresolved)"
  elif [ "$l4_resolved" -gt 0 ]; then
    verdict="present"
  else
    verdict="partial-bug-flagged (L4 empty)"
  fi

  l4_display="$l4_resolved resolved"
  if [ "$l4_pending" -gt 0 ]; then
    l4_display="$l4_display, $l4_pending pending"
  fi

  echo "| \`$label\` | $l1 | $l2 | $l3 | $l4_display | **$verdict** |"
done
echo ""

# ============================================================
# EXTRACTION PROGRESS (supplementary — explains PENDING verdicts)
# ============================================================
echo "## Extraction Progress"
echo ""

if [ "$LOG_ACCESSIBLE" = true ]; then
  echo "### Subject extraction events (since $TICK_CUTOFF)"
  echo ""
  echo '```'
  EXTRACT_LINES=$(grep -a "subject_extraction\|kg_budget_exhausted" "$LOG_FILE" || true)
  echo "$EXTRACT_LINES" | jq -c '
    select(.timestamp > "'"$TICK_CUTOFF"'")
    | {ts: .timestamp, event: .event, agent_id: .agent_id, pending_docs: .pending_docs, completed: .completed, remaining: .remaining, budget: .budget}
  ' 2>/dev/null | tail -20 || echo "(no extraction events)"
  echo '```'
  echo ""
fi

echo "### Extraction backlog per corpus"
echo ""
echo '```'
sqlite3 -header -column "$DB" "
SELECT
  c.docs_root_hash,
  c.docs_root,
  COUNT(DISTINCT c.source_doc_path) AS pending_docs
FROM kg_chunks c
WHERE NOT EXISTS (
  SELECT 1 FROM kg_extractions e
  WHERE e.docs_root_hash = c.docs_root_hash
    AND e.source_doc_path = c.source_doc_path
)
GROUP BY c.docs_root_hash;
"
echo '```'
echo ""

echo "---"
echo ""
echo "Investigation complete. See finding doc for outcome path declaration."
