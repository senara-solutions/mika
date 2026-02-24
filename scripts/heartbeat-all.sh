#!/usr/bin/env bash
set -euo pipefail

# Trigger heartbeat on all active customer containers.
# Intended to be called by a single cluster-level CronJob or cron entry.
# The agent's own pre-filter handles active hours, rate limits, and suppression.

usage() {
    cat <<USAGE
Usage: heartbeat-all.sh [--dry-run]

Options:
  --dry-run     Show which customers would be triggered without executing
  --help        Show this help

Required environment variables:
  DATABASE_URL          Postgres connection string for gateway DB
  MIKA_INTERNAL_TOKEN   Shared 64-char hex auth token
USAGE
    exit "${1:-1}"
}

DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)  DRY_RUN=true; shift ;;
        --help)     usage 0 ;;
        -*)         echo "Error: Unknown option: $1" >&2; usage ;;
        *)          echo "Error: Unexpected argument: $1" >&2; usage ;;
    esac
done

: "${DATABASE_URL:?DATABASE_URL is required}"
: "${MIKA_INTERNAL_TOKEN:?MIKA_INTERNAL_TOKEN is required}"

NAMESPACE="mika-agents"

# Query active customers (status='active' means paired and routing)
CUSTOMER_IDS=$(psql "${DATABASE_URL}" -t -A -c \
    "SELECT id FROM customers WHERE status = 'active' ORDER BY id;" 2>/dev/null)

if [[ -z "$CUSTOMER_IDS" ]]; then
    echo "No active customers found."
    exit 0
fi

COUNT=$(echo "$CUSTOMER_IDS" | wc -l | tr -d ' ')
echo "Triggering heartbeat for ${COUNT} active customer(s)..."

SUCCEEDED=0
FAILED=0

while IFS= read -r cid; do
    URL="http://mika-${cid}.${NAMESPACE}.svc.cluster.local:8080/heartbeat"

    if [[ "$DRY_RUN" == "true" ]]; then
        echo "  [DRY RUN] Would POST ${URL}"
        continue
    fi

    HTTP_CODE=$(curl -s -X POST \
        -H "Authorization: Bearer ${MIKA_INTERNAL_TOKEN}" \
        -H "Content-Type: application/json" \
        -o /dev/null -w '%{http_code}' \
        --connect-timeout 5 --max-time 10 \
        "${URL}" 2>/dev/null) || HTTP_CODE="000"
    if [[ "$HTTP_CODE" =~ ^2 ]]; then
        SUCCEEDED=$((SUCCEEDED + 1))
    else
        echo "  Warning: heartbeat failed for ${cid} (HTTP ${HTTP_CODE})" >&2
        FAILED=$((FAILED + 1))
    fi
done <<< "$CUSTOMER_IDS"

if [[ "$DRY_RUN" == "true" ]]; then
    echo "Done (dry run). ${COUNT} customer(s) would be triggered."
else
    echo "Done. Succeeded: ${SUCCEEDED}, Failed: ${FAILED}"
fi
