#!/usr/bin/env bash
set -euo pipefail

# --- Usage ---
usage() {
    cat <<USAGE
Usage: setup-gateway.sh [--namespace NS] [--dry-run]

Creates the mika-gateway-secrets K8s secret with the required keys
for the mika-gateway deployment.

Options:
  --namespace NS  K8s namespace (default: mika-system)
  --dry-run       Show what would be done without executing
  --help          Show this help

Required environment variables:
  DATABASE_URL              Postgres connection string for gateway DB
  MIKA_TELEGRAM_BOT_TOKEN   Telegram bot token
  MIKA_TELEGRAM_WEBHOOK_SECRET  Telegram webhook secret
  MIKA_INTERNAL_TOKEN       Shared 64-char hex auth token
USAGE
    exit "${1:-1}"
}

# --- Parse arguments ---
NAMESPACE="mika-system"
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --namespace) NAMESPACE="$2"; shift 2 ;;
        --dry-run)   DRY_RUN=true; shift ;;
        --help)      usage 0 ;;
        -*)          echo "Error: Unknown option: $1" >&2; usage ;;
        *)           echo "Error: Unexpected argument: $1" >&2; usage ;;
    esac
done

# --- Validate required env vars ---
: "${DATABASE_URL:?DATABASE_URL is required}"
: "${MIKA_TELEGRAM_BOT_TOKEN:?MIKA_TELEGRAM_BOT_TOKEN is required}"
: "${MIKA_TELEGRAM_WEBHOOK_SECRET:?MIKA_TELEGRAM_WEBHOOK_SECRET is required}"
: "${MIKA_INTERNAL_TOKEN:?MIKA_INTERNAL_TOKEN is required}"

# Validate MIKA_INTERNAL_TOKEN is 64 hex chars
if ! [[ "$MIKA_INTERNAL_TOKEN" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "Error: MIKA_INTERNAL_TOKEN must be exactly 64 hex characters" >&2
    exit 1
fi

echo "=== Mika Gateway Secret Setup ==="
echo "Namespace: ${NAMESPACE}"
echo "Secret:    mika-gateway-secrets"
echo ""

if [[ "$DRY_RUN" == "true" ]]; then
    echo "[DRY RUN] Would create:"
    echo "  K8s secret: mika-gateway-secrets in ${NAMESPACE}"
    echo "  Keys: database-url, telegram-bot-token, telegram-webhook-secret, internal-token"
    exit 0
fi

# --- Ensure namespace exists ---
kubectl get namespace "${NAMESPACE}" >/dev/null 2>&1 || kubectl create namespace "${NAMESPACE}"

# --- Create secret ---
echo "Creating K8s secret..."
kubectl create secret generic mika-gateway-secrets \
    --namespace "${NAMESPACE}" \
    --from-literal=database-url="${DATABASE_URL}" \
    --from-literal=telegram-bot-token="${MIKA_TELEGRAM_BOT_TOKEN}" \
    --from-literal=telegram-webhook-secret="${MIKA_TELEGRAM_WEBHOOK_SECRET}" \
    --from-literal=internal-token="${MIKA_INTERNAL_TOKEN}" \
    --dry-run=client -o yaml | kubectl apply -f -

echo "  Created: mika-gateway-secrets"
echo ""
echo "=== Gateway Secret Setup Complete ==="
