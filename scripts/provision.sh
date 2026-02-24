#!/usr/bin/env bash
set -euo pipefail

# --- Usage ---
usage() {
    cat <<USAGE
Usage: provision.sh <customer_name> [plan] [--timezone TZ]

Arguments:
  customer_name   Human-readable customer name (required)
  plan            Customer plan: standard (default) or premium

Options:
  --timezone TZ   IANA timezone (default: UTC)
  --output json   Output structured JSON instead of human-readable text
  --dry-run       Show what would be done without executing
  --help          Show this help

Exit codes:
  0   Success
  1   Invalid arguments
  2   K8s secret creation failed
  3   Helm install failed
  4   Postgres registration failed
  10  Provisioning and rollback both failed

Required environment variables:
  DATABASE_URL            Postgres connection string for gateway DB
  MIKA_ANTHROPIC_API_KEY  Anthropic API key for the customer
  MIKA_INTERNAL_TOKEN     Shared 64-char hex auth token
  TELEGRAM_BOT_USERNAME   Telegram bot username (for deep link)
  MIKA_IMAGE_REPO         ECR image repository

Optional environment variables:
  MIKA_IMAGE_TAG          Image tag (default: latest)
  MIKA_GATEWAY_URL        Gateway URL (default: http://mika-gateway.mika-system.svc.cluster.local:8080)
USAGE
    exit "${1:-1}"
}

# --- Parse arguments ---
CUSTOMER_NAME=""
PLAN="standard"
TIMEZONE="UTC"
DRY_RUN=false
OUTPUT_FORMAT="text"

while [[ $# -gt 0 ]]; do
    case $1 in
        --timezone) TIMEZONE="$2"; shift 2 ;;
        --output)   OUTPUT_FORMAT="$2"; shift 2 ;;
        --dry-run)  DRY_RUN=true; shift ;;
        --help)     usage 0 ;;
        -*)         echo "Error: Unknown option: $1" >&2; usage ;;
        *)
            if [[ -z "$CUSTOMER_NAME" ]]; then
                CUSTOMER_NAME="$1"
            elif [[ "$1" =~ ^(standard|premium)$ ]]; then
                PLAN="$1"
            else
                echo "Error: Unexpected argument: $1" >&2; usage
            fi
            shift ;;
    esac
done

if [[ "$OUTPUT_FORMAT" != "text" && "$OUTPUT_FORMAT" != "json" ]]; then
    echo "Error: --output must be 'text' or 'json', got '${OUTPUT_FORMAT}'" >&2
    exit 1
fi

[[ -z "$CUSTOMER_NAME" ]] && { echo "Error: customer_name is required" >&2; usage; }

# --- Validate required env vars ---
: "${DATABASE_URL:?DATABASE_URL is required}"
: "${MIKA_ANTHROPIC_API_KEY:?MIKA_ANTHROPIC_API_KEY is required}"
: "${MIKA_INTERNAL_TOKEN:?MIKA_INTERNAL_TOKEN is required}"
: "${TELEGRAM_BOT_USERNAME:?TELEGRAM_BOT_USERNAME is required}"
: "${MIKA_IMAGE_REPO:?MIKA_IMAGE_REPO is required}"

# --- Validate inputs ---
# Customer name: alphanumeric, spaces, hyphens, periods, apostrophes. 1-100 chars.
# Prevents issues with Helm --set YAML parsing and Postgres text.
if [[ ! "$CUSTOMER_NAME" =~ ^[a-zA-Z0-9\ \'\.\-]{1,100}$ ]]; then
    echo "Error: customer_name must be 1-100 chars, alphanumeric/spaces/hyphens/periods/apostrophes only" >&2
    exit 1
fi

if [[ "$PLAN" != "standard" && "$PLAN" != "premium" ]]; then
    echo "Error: plan must be 'standard' or 'premium', got '$PLAN'" >&2
    exit 1
fi

# Validate MIKA_INTERNAL_TOKEN is 64 hex chars
if ! [[ "$MIKA_INTERNAL_TOKEN" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "Error: MIKA_INTERNAL_TOKEN must be exactly 64 hex characters" >&2
    exit 1
fi

# --- Generate IDs ---
CUSTOMER_ID=$(uuidgen | tr '[:upper:]' '[:lower:]')
PAIRING_TOKEN=$(openssl rand -hex 32)  # 64-char hex, matches gateway validation
NAMESPACE="mika-agents"
GATEWAY_URL="${MIKA_GATEWAY_URL:-http://mika-gateway.mika-system.svc.cluster.local:8080}"

echo "=== Mika Customer Provisioning ==="
echo "Customer:    ${CUSTOMER_NAME}"
echo "Customer ID: ${CUSTOMER_ID}"
echo "Plan:        ${PLAN}"
echo "Timezone:    ${TIMEZONE}"
echo "Namespace:   ${NAMESPACE}"
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] Would create:"
    echo "  1. K8s secret: mika-${CUSTOMER_ID}-secrets in ${NAMESPACE}"
    echo "  2. Helm release: mika-${CUSTOMER_ID} in ${NAMESPACE}"
    echo "  3. Postgres row: customers(id=${CUSTOMER_ID})"
    echo "  4. Deep link: https://t.me/${TELEGRAM_BOT_USERNAME}?start=${PAIRING_TOKEN}"
    exit 0
fi

# --- Rollback on failure ---
STEP_COMPLETED=0
PROVISION_EXIT_CODE=1
cleanup() {
    local rollback_failed=false
    echo "" >&2
    echo "!!! Provisioning failed at step ${STEP_COMPLETED}. Rolling back..." >&2
    if [[ $STEP_COMPLETED -ge 3 ]]; then
        echo "  Rolling back Postgres row..." >&2
        if ! psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
            -v customer_id="${CUSTOMER_ID}" \
            <<'ROLLBACK_SQL' 2>/dev/null; then
DELETE FROM customers WHERE id = :'customer_id'::uuid;
ROLLBACK_SQL
            rollback_failed=true
        fi
    fi
    if [[ $STEP_COMPLETED -ge 2 ]]; then
        echo "  Rolling back Helm release..." >&2
        if ! helm uninstall "mika-${CUSTOMER_ID}" --namespace "${NAMESPACE}" 2>/dev/null; then
            rollback_failed=true
        fi
    fi
    if [[ $STEP_COMPLETED -ge 1 ]]; then
        echo "  Rolling back K8s secret..." >&2
        if ! kubectl delete secret "mika-${CUSTOMER_ID}-secrets" -n "${NAMESPACE}" 2>/dev/null; then
            rollback_failed=true
        fi
    fi
    if [[ "$rollback_failed" == "true" ]]; then
        echo "Rollback encountered errors." >&2
        if [[ "$OUTPUT_FORMAT" == "json" ]]; then
            printf '{"customer_id":"%s","namespace":"%s","status":"rollback_failed"}\n' \
                "${CUSTOMER_ID}" "${NAMESPACE}"
        fi
        exit 10
    fi
    echo "Rollback complete." >&2
    if [[ "$OUTPUT_FORMAT" == "json" ]]; then
        printf '{"customer_id":"%s","namespace":"%s","status":"failed"}\n' \
            "${CUSTOMER_ID}" "${NAMESPACE}"
    fi
    exit "${PROVISION_EXIT_CODE}"
}
trap cleanup ERR

# --- Ensure namespace exists ---
kubectl get namespace "${NAMESPACE}" >/dev/null 2>&1 || kubectl create namespace "${NAMESPACE}"

# --- Step 1: Create K8s secret ---
PROVISION_EXIT_CODE=2
echo "Step 1/4: Creating K8s secret..."
kubectl create secret generic "mika-${CUSTOMER_ID}-secrets" \
    --namespace "${NAMESPACE}" \
    --from-literal=anthropic-api-key="${MIKA_ANTHROPIC_API_KEY}" \
    --from-literal=internal-token="${MIKA_INTERNAL_TOKEN}" \
    --dry-run=client -o yaml | kubectl apply -f -
STEP_COMPLETED=1
echo "  Created: mika-${CUSTOMER_ID}-secrets"

# --- Step 2: Helm install ---
PROVISION_EXIT_CODE=3
echo "Step 2/4: Installing Helm release..."
helm install "mika-${CUSTOMER_ID}" ./helm/mika-customer \
    --namespace "${NAMESPACE}" --create-namespace \
    --set customer.id="${CUSTOMER_ID}" \
    --set-string "customer.name=${CUSTOMER_NAME}" \
    --set customer.plan="${PLAN}" \
    --set customer.timezone="${TIMEZONE}" \
    --set image.repository="${MIKA_IMAGE_REPO}" \
    --set image.tag="${MIKA_IMAGE_TAG:-latest}" \
    --set gateway.url="${GATEWAY_URL}" \
    --wait --timeout 120s
STEP_COMPLETED=2
echo "  Installed: mika-${CUSTOMER_ID}"

# --- Step 3: Register in Postgres (parameterized via psql \set) ---
PROVISION_EXIT_CODE=4
echo "Step 3/4: Registering in Postgres..."
psql "${DATABASE_URL}" \
    -v ON_ERROR_STOP=1 \
    -v customer_id="${CUSTOMER_ID}" \
    -v customer_name="${CUSTOMER_NAME}" \
    -v plan="${PLAN}" \
    -v timezone="${TIMEZONE}" \
    -v pairing_token="${PAIRING_TOKEN}" \
    <<'SQL'
INSERT INTO customers (id, name, plan, timezone, status, pairing_token, pairing_expires_at)
VALUES (
    :'customer_id'::uuid,
    :'customer_name',
    :'plan',
    :'timezone',
    'provisioned',
    :'pairing_token',
    now() + interval '72 hours'
)
ON CONFLICT (id) DO NOTHING;
SQL
STEP_COMPLETED=3
echo "  Registered: ${CUSTOMER_ID}"

# --- Step 4: Output deep link ---
echo "Step 4/4: Generating deep link..."
DEEP_LINK="https://t.me/${TELEGRAM_BOT_USERNAME}?start=${PAIRING_TOKEN}"
STEP_COMPLETED=4

if [[ "$OUTPUT_FORMAT" == "json" ]]; then
    printf '{"customer_id":"%s","namespace":"%s","status":"success"}\n' \
        "${CUSTOMER_ID}" "${NAMESPACE}"
else
    echo ""
    echo "=== Provisioning Complete ==="
    echo "Customer ID:   ${CUSTOMER_ID}"
    echo "Deep link:     ${DEEP_LINK}"
    echo "Pairing token: ${PAIRING_TOKEN}"
    echo "Token expires: 72 hours from now"
    echo ""
    echo "Send the deep link to the customer to start onboarding."
fi
