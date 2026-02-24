#!/usr/bin/env bash
set -euo pipefail

# --- Usage ---
usage() {
    cat <<USAGE
Usage: deprovision.sh <customer_id> [--force] [--dry-run]

Arguments:
  customer_id   UUID of the customer to deprovision (required)

Options:
  --force       Skip confirmation prompt
  --dry-run     Show what would be done without executing
  --help        Show this help

Required environment variables:
  DATABASE_URL    Postgres connection string for gateway DB
USAGE
    exit 1
}

# --- Parse arguments ---
CUSTOMER_ID=""
FORCE=false
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --force)    FORCE=true; shift ;;
        --dry-run)  DRY_RUN=true; shift ;;
        --help)     usage ;;
        -*)         echo "Error: Unknown option: $1" >&2; usage ;;
        *)
            if [[ -z "$CUSTOMER_ID" ]]; then
                CUSTOMER_ID="$1"
            else
                echo "Error: Unexpected argument: $1" >&2; usage
            fi
            shift ;;
    esac
done

[[ -z "$CUSTOMER_ID" ]] && { echo "Error: customer_id is required" >&2; usage; }

# Validate UUID format
if ! [[ "$CUSTOMER_ID" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]; then
    echo "Error: customer_id must be a valid lowercase UUID, got '${CUSTOMER_ID}'" >&2
    exit 1
fi

: "${DATABASE_URL:?DATABASE_URL is required}"

NAMESPACE="mika-agents"

# --- Look up customer info (parameterized) ---
CUSTOMER_NAME=$(psql "${DATABASE_URL}" -t -A \
    -v customer_id="${CUSTOMER_ID}" \
    -c "SELECT name FROM customers WHERE id = :'customer_id'::uuid;" 2>/dev/null || echo "")
if [[ -z "$CUSTOMER_NAME" ]]; then
    echo "Warning: Customer ${CUSTOMER_ID} not found in Postgres (may already be partially deprovisioned)"
fi

echo "=== Mika Customer Deprovisioning ==="
echo "Customer ID: ${CUSTOMER_ID}"
echo "Name:        ${CUSTOMER_NAME:-unknown}"
echo "Namespace:   ${NAMESPACE}"
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] Would execute:"
    echo "  1. Mark suspended in Postgres"
    echo "  2. Helm uninstall mika-${CUSTOMER_ID}"
    echo "  3. Delete PVC mika-${CUSTOMER_ID}-data"
    echo "  4. Delete K8s secret mika-${CUSTOMER_ID}-secrets"
    echo "  5. Remove Postgres row"
    exit 0
fi

# --- Confirmation ---
if [[ "$FORCE" != "true" ]]; then
    # Abort if not running in a terminal (prevents accidental pipe execution)
    if [[ ! -t 0 ]]; then
        echo "Error: Confirmation required but stdin is not a terminal. Use --force for non-interactive mode." >&2
        exit 1
    fi
    echo "WARNING: This will permanently destroy all data for this customer."
    echo "  - SQLite database (conversation history, memory, reminders)"
    echo "  - K8s secret"
    echo "  - Postgres registration"
    echo ""
    read -rp "Type the customer ID to confirm: " CONFIRM
    if [[ "$CONFIRM" != "$CUSTOMER_ID" ]]; then
        echo "Aborted. ID did not match."
        exit 1
    fi
fi

# --- Step 1: Mark suspended (stops routing immediately) ---
echo "Step 1/5: Marking suspended in Postgres..."
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    -c "UPDATE customers SET status = 'suspended' WHERE id = :'customer_id'::uuid;" \
    2>/dev/null || echo "  (no row to update)"

# --- Step 2: Helm uninstall ---
echo "Step 2/5: Uninstalling Helm release..."
helm uninstall "mika-${CUSTOMER_ID}" --namespace "${NAMESPACE}" 2>/dev/null || echo "  (no release found)"

# --- Step 3: Delete PVC ---
echo "Step 3/5: Deleting PVC..."
kubectl delete pvc "mika-${CUSTOMER_ID}-data" -n "${NAMESPACE}" 2>/dev/null || echo "  (no PVC found)"

# --- Step 4: Delete K8s secret ---
echo "Step 4/5: Deleting K8s secret..."
kubectl delete secret "mika-${CUSTOMER_ID}-secrets" -n "${NAMESPACE}" 2>/dev/null || echo "  (no secret found)"

# --- Step 5: Remove from Postgres ---
echo "Step 5/5: Removing from Postgres..."
psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    -c "DELETE FROM customers WHERE id = :'customer_id'::uuid;" \
    2>/dev/null || echo "  (no row to delete)"

echo ""
echo "=== Deprovisioning Complete ==="
echo "Customer ${CUSTOMER_ID} has been fully deprovisioned."
