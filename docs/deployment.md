# Mika Deployment Guide

Operator-focused documentation for deploying Mika in hosted mode on Kubernetes.

---

## 1. Architecture Overview

Mika uses a hub-and-spoke architecture with per-customer container isolation.

```
Telegram
   |
   v
[Ingress / Load Balancer]
   |
   v
[mika-gateway]  (mika-system namespace)
   |   Stateless, Postgres-backed
   |   Routes by telegram_chat_id
   |
   +---> [mika-{uuid-1}]  (mika-agents namespace)
   |        SQLite on PVC
   |        Axum HTTP server
   |
   +---> [mika-{uuid-2}]  (mika-agents namespace)
   |        SQLite on PVC
   |        Axum HTTP server
   |
   +---> [mika-{uuid-N}] ...
```

**Two namespaces:**

| Namespace | Contents | Purpose |
|-----------|----------|---------|
| `mika-system` | Gateway deployment, service, ingress, secrets | Shared Telegram webhook router |
| `mika-agents` | Per-customer deployments, services, PVCs, secrets | Isolated agent containers |

**Message flow (inbound):**

1. Telegram sends a webhook POST to the gateway at `/webhook/telegram`.
2. The gateway validates the `X-Telegram-Bot-Api-Secret-Token` header (constant-time comparison).
3. The gateway looks up the customer by `telegram_chat_id` in Postgres.
4. The gateway forwards the message to the customer container at `http://mika-{uuid}.mika-agents.svc.cluster.local:8080/message` with Bearer token authentication.
5. The container processes the message through the agent loop (context retrieval, prompt assembly, Claude API call, tool execution).

**Message flow (outbound):**

1. The customer container POSTs to the gateway at `/send` with Bearer token authentication.
2. The gateway relays the message to Telegram via the Bot API.

**Container URL pattern:** Container URLs are computed deterministically from the customer UUID, eliminating any user-controlled URL input (SSRF prevention):

```
http://mika-{customer_uuid}.mika-agents.svc.cluster.local:8080
```

---

## 2. Prerequisites

Before deploying Mika, ensure you have:

| Requirement | Details |
|-------------|---------|
| Kubernetes cluster | 1.27+ (tested on EKS) |
| Postgres database | 14+ (gateway customer registry) |
| Anthropic API key | For Claude API access |
| Telegram Bot | Created via BotFather (see Section 4) |
| Container registry | ECR, GHCR, or similar |
| `kubectl` | Configured with cluster access |
| `helm` | v3.12+ |
| `psql` | For provisioning scripts (Postgres client) |
| `openssl` | For token generation |
| `uuidgen` | For customer ID generation |
| `curl` | For heartbeat script |

**Postgres schema:** The gateway expects a `customers` table. Run the gateway migrations before first deployment. The gateway uses `sqlx` and expects the schema to include at minimum:

- `id` (UUID, primary key)
- `name` (text)
- `plan` (text)
- `timezone` (text)
- `status` (text: `provisioned`, `active`, `suspended`)
- `pairing_token` (text, nullable)
- `pairing_expires_at` (timestamptz, nullable)
- `telegram_chat_id` (bigint, nullable, unique)
- `paired_at` (timestamptz, nullable)
- `last_update_id` (bigint, default 0)

---

## 3. Building Docker Images

The agent image is built from this repo. The gateway image is built from the private [mika-cloud](https://github.com/senara-solutions/mika-cloud) repo. Both use multi-stage builds with a Rust builder stage and a minimal Debian runtime stage.

### Agent Image (~95MB)

```bash
docker build -f Dockerfile.agent -t mika-agent:dev .
```

Build details:
- **Builder:** `rust:1.85-slim` with gcc, libc-dev, pkg-config, libssl-dev (needed for rusqlite bundled SQLite + OpenSSL)
- **Runtime:** `debian:bookworm-slim` with ca-certificates, wget, file, jq, and gh (GitHub CLI v2.65.0 with SHA256 checksum verification)
- **Binary:** `mika-server` (Axum HTTP server)
- **User:** `mika` (UID 1000), non-root
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/health` (10s interval, 5s start period)
- **Config:** Default config copied to `/app/config/default.toml`
- **Data dir:** `/home/mika/.mika` (PVC mount point)

### Gateway Image (~90MB)

The gateway Dockerfile lives in the private [mika-cloud](https://github.com/senara-solutions/mika-cloud) repo.

```bash
# From the mika-cloud repo:
docker build -t mika-gateway:dev .
```

Build details:
- **Builder:** `rust:1.85-slim` with pkg-config, libssl-dev (no gcc needed -- gateway skips rusqlite)
- **Runtime:** `debian:bookworm-slim` with ca-certificates and wget
- **Binary:** `mika-gateway`
- **User:** `mika` (UID 1000), non-root, no home directory (stateless)
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/readyz` (10s interval, 10s start period)

### Pushing to Registry

```bash
# Tag and push agent image
docker tag mika-agent:dev 123456789.dkr.ecr.us-east-1.amazonaws.com/mika-agent:v0.1.0
docker push 123456789.dkr.ecr.us-east-1.amazonaws.com/mika-agent:v0.1.0

# Tag and push gateway image
docker tag mika-gateway:dev 123456789.dkr.ecr.us-east-1.amazonaws.com/mika-gateway:v0.1.0
docker push 123456789.dkr.ecr.us-east-1.amazonaws.com/mika-gateway:v0.1.0
```

### Dependency Caching

Both Dockerfiles use a dependency caching strategy:

1. Copy only `Cargo.toml`, `Cargo.lock`, and crate manifests.
2. Create dummy source files and build dependencies.
3. Remove workspace crate artifacts but keep dependency cache.
4. Copy real source code and rebuild (only workspace crates recompile).

This means rebuilds after source changes are fast because dependency compilation is cached in the Docker layer.

---

## 4. Setting Up Telegram Bot

### Step 1: Create the Bot

1. Open Telegram and search for `@BotFather`.
2. Send `/newbot`.
3. Choose a display name (e.g., "Mika Assistant").
4. Choose a username ending in `bot` (e.g., `mika_prod_bot`).
5. BotFather replies with your **bot token** (format: `123456:ABC-DEF...`). Save this securely.

### Step 2: Record the Bot Username

Save the bot username (without the `@` prefix). You will need this for:
- The `TELEGRAM_BOT_USERNAME` environment variable in `provision.sh`
- Generating customer deep links (`https://t.me/<bot_username>?start=<token>`)

### Step 3: Generate the Webhook Secret

Generate a 64-character hex token for validating inbound Telegram webhooks (see
[Token Generation](#token-generation) for the command):

```bash
export MIKA_TELEGRAM_WEBHOOK_SECRET=<generated-token>
```

Save this value. It will be used in `setup-gateway.sh` and when registering the webhook with Telegram.

### Step 4: Register the Webhook

After the gateway is deployed and reachable at a public URL (see Section 5), register the webhook:

```bash
curl -X POST "https://api.telegram.org/bot${MIKA_TELEGRAM_BOT_TOKEN}/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://mika.example.com/webhook/telegram",
    "secret_token": "'${MIKA_TELEGRAM_WEBHOOK_SECRET}'",
    "allowed_updates": ["message"]
  }'
```

Verify the webhook is set:

```bash
curl "https://api.telegram.org/bot${MIKA_TELEGRAM_BOT_TOKEN}/getWebhookInfo"
```

---

## 5. Deploying the Gateway

### Step 1: Generate the Internal Token

The internal token authenticates communication between the gateway and customer
containers. Generate a 64-character hex token as described in
[Token Generation](#token-generation):

```bash
export MIKA_INTERNAL_TOKEN=<generated-token>
```

Save this token securely. Every customer container and the gateway must share the same internal token.

### Step 2: Create Gateway Secrets

Use the `setup-gateway.sh` script to create the Kubernetes secret:

```bash
export DATABASE_URL="postgres://mika:password@postgres-host:5432/mika_gateway"
export MIKA_TELEGRAM_BOT_TOKEN="123456:ABC-DEF..."
export MIKA_TELEGRAM_WEBHOOK_SECRET="<64-char-hex>"
export MIKA_INTERNAL_TOKEN="<64-char-hex>"

./scripts/setup-gateway.sh
```

This creates the `mika-gateway-secrets` secret in the `mika-system` namespace with four keys:

| Secret Key | Source Variable | Description |
|------------|-----------------|-------------|
| `database-url` | `DATABASE_URL` | Postgres connection string |
| `telegram-bot-token` | `MIKA_TELEGRAM_BOT_TOKEN` | Telegram Bot API token |
| `telegram-webhook-secret` | `MIKA_TELEGRAM_WEBHOOK_SECRET` | Webhook validation secret |
| `internal-token` | `MIKA_INTERNAL_TOKEN` | Gateway-to-container auth token |

The script validates that `MIKA_INTERNAL_TOKEN` is exactly 64 hex characters before creating the secret.

**Options:**
- `--namespace NS` -- Override the namespace (default: `mika-system`)
- `--dry-run` -- Show what would be created without executing

**Dry run example:**

```bash
./scripts/setup-gateway.sh --dry-run
```

### Step 3: Install the Gateway Helm Chart

```bash
helm install mika-gateway ./helm/mika-gateway \
  --namespace mika-system --create-namespace \
  --set image.repository="123456789.dkr.ecr.us-east-1.amazonaws.com/mika-gateway" \
  --set image.tag="v0.1.0" \
  --set telegram.webhookUrl="https://mika.example.com/webhook/telegram" \
  --set ingress.enabled=true \
  --set ingress.className="nginx" \
  --set ingress.host="mika.example.com"
```

The gateway chart creates:
- A Deployment (default: 1 replica, RollingUpdate strategy with `maxUnavailable: 0`)
- A ClusterIP Service on port 8080
- An Ingress (if `ingress.enabled=true`), routing `/webhook/telegram` to the service
- Probes: liveness on `/livez`, readiness on `/readyz` (checks Postgres connectivity)

### Step 4: Verify

```bash
# Check pod status
kubectl get pods -n mika-system

# Check readiness
kubectl exec -n mika-system deploy/mika-gateway -- wget -qO- http://localhost:8080/readyz

# Check logs
kubectl logs -n mika-system deploy/mika-gateway
```

### Gateway Endpoints

The gateway exposes `/webhook/telegram` (inbound from Telegram), `/send`
(outbound relay from containers), and health probe endpoints (`/health`,
`/readyz`, `/livez`). See the
[Gateway Endpoints](architecture.md#endpoints) table in the architecture
document for full details including auth requirements.

### Gateway Environment Variables

The gateway requires `MIKA_DATABASE_URL`, `MIKA_TELEGRAM_BOT_TOKEN`,
`MIKA_TELEGRAM_WEBHOOK_SECRET`, `MIKA_TELEGRAM_WEBHOOK_URL`, and
`MIKA_INTERNAL_TOKEN`. Both token variables are validated at startup to be
exactly 64 hex characters.

See [Gateway Environment Variables](configuration.md#gateway) for the complete
reference table.

---

## 6. Provisioning a Customer

The `provision.sh` script automates the full customer provisioning lifecycle in four steps.

### Usage

```bash
./scripts/provision.sh <customer_name> [plan] [--timezone TZ] [--output json] [--dry-run]
```

### Required Environment Variables

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | Postgres connection string for gateway DB |
| `MIKA_ANTHROPIC_API_KEY` | Anthropic API key for this customer |
| `MIKA_INTERNAL_TOKEN` | Shared 64-char hex auth token |
| `TELEGRAM_BOT_USERNAME` | Telegram bot username (for deep link generation) |
| `MIKA_IMAGE_REPO` | Container image repository |

### Optional Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MIKA_IMAGE_TAG` | `latest` | Image tag |
| `MIKA_GATEWAY_URL` | `http://mika-gateway.mika-system.svc.cluster.local:8080` | Gateway URL |

### Provisioning Steps

The script performs four steps, with automatic rollback on failure:

**Step 1: Create K8s Secret** (`mika-{uuid}-secrets` in `mika-agents`)

Contains:
- `anthropic-api-key` -- Anthropic API key
- `internal-token` -- Shared bearer token

**Step 2: Helm Install** (`mika-{uuid}` release in `mika-agents`)

Creates:
- Deployment (1 replica, `Recreate` strategy for RWO PVC compatibility)
- ClusterIP Service on port 8080
- PersistentVolumeClaim (`1Gi`, `gp3` storage class)

The deployment runs with `--wait --timeout 120s`, so it blocks until the pod is ready.

**Step 3: Register in Postgres**

Inserts a row into the `customers` table with:
- `status = 'provisioned'`
- A 64-char hex pairing token (generated via `openssl rand -hex 32`)
- `pairing_expires_at = now() + 72 hours`

**Step 4: Output Deep Link**

Generates and prints the Telegram deep link for customer pairing:

```
https://t.me/<bot_username>?start=<pairing_token>
```

### Example: Full Provisioning

```bash
export DATABASE_URL="postgres://mika:password@postgres-host:5432/mika_gateway"
export MIKA_ANTHROPIC_API_KEY="sk-ant-..."
export MIKA_INTERNAL_TOKEN="$(openssl rand -hex 32)"
export TELEGRAM_BOT_USERNAME="mika_prod_bot"
export MIKA_IMAGE_REPO="123456789.dkr.ecr.us-east-1.amazonaws.com/mika-agent"

./scripts/provision.sh "Jane Doe" premium --timezone "America/New_York"
```

Output:

```
=== Mika Customer Provisioning ===
Customer:    Jane Doe
Customer ID: a1b2c3d4-e5f6-7890-abcd-ef1234567890
Plan:        premium
Timezone:    America/New_York
Namespace:   mika-agents

Step 1/4: Creating K8s secret...
  Created: mika-a1b2c3d4-e5f6-7890-abcd-ef1234567890-secrets
Step 2/4: Installing Helm release...
  Installed: mika-a1b2c3d4-e5f6-7890-abcd-ef1234567890
Step 3/4: Registering in Postgres...
  Registered: a1b2c3d4-e5f6-7890-abcd-ef1234567890
Step 4/4: Generating deep link...

=== Provisioning Complete ===
Customer ID:   a1b2c3d4-e5f6-7890-abcd-ef1234567890
Deep link:     https://t.me/mika_prod_bot?start=<64-char-hex>
Pairing token: <64-char-hex>
Token expires: 72 hours from now

Send the deep link to the customer to start onboarding.
```

### Example: Dry Run

```bash
./scripts/provision.sh "Jane Doe" --dry-run
```

### Example: JSON Output

```bash
./scripts/provision.sh "Jane Doe" --output json
```

Returns on success:

```json
{"customer_id":"a1b2c3d4-...","namespace":"mika-agents","status":"success"}
```

### Rollback on Failure

If any step fails, the script automatically rolls back all completed steps in reverse order:
- Step 3 failure: deletes Postgres row, then Helm release, then K8s secret
- Step 2 failure: deletes K8s secret

Exit codes indicate which step failed:

| Exit Code | Meaning |
|-----------|---------|
| 0 | Success |
| 1 | Invalid arguments |
| 2 | K8s secret creation failed |
| 3 | Helm install failed |
| 4 | Postgres registration failed |
| 10 | Provisioning and rollback both failed (manual cleanup needed) |

### Input Validation

- **Customer name:** 1-100 characters, alphanumeric, spaces, hyphens, periods, apostrophes only
- **Plan:** Must be `standard` or `premium`
- **Internal token:** Must be exactly 64 hex characters

---

## 7. Customer Pairing Flow

After provisioning, the customer must pair their Telegram account to activate message routing.

### Flow

1. The operator sends the deep link to the customer (e.g., via email).
2. The customer opens the link: `https://t.me/<bot_username>?start=<pairing_token>`
3. Telegram opens the bot chat and sends `/start <pairing_token>` automatically.
4. The gateway receives the webhook and calls `handle_pairing`:
   - Validates the token format (64 hex characters).
   - Performs an atomic Postgres UPDATE that checks:
     - `pairing_token` matches
     - `telegram_chat_id IS NULL` (not already paired)
     - `status = 'provisioned'`
     - `pairing_expires_at > now()` (token not expired)
   - On success: sets `telegram_chat_id`, `status = 'active'`, clears the pairing token.
5. The gateway forwards a synthetic `"Hello!"` message to the customer container at `/message`.
6. The agent runs its onboarding flow and responds via `/send` back through the gateway to Telegram.

### Pairing Failure Cases

| Scenario | User Sees |
|----------|-----------|
| Token expired (>72 hours) | "Invalid or expired invite link." |
| Token already used | "Invalid or expired invite link." |
| Malformed token | "Invalid or expired invite link." |
| Telegram account already linked to another customer | "This Telegram account is already linked to another account." |
| Database error | "I'm having trouble right now. Please try again in a moment." |

Error messages are intentionally vague to avoid leaking information about token state.

### Bare /start (No Token)

If a user sends `/start` without a token, the gateway replies:

> "Welcome! If you have an invite link, please use it to get started. If you're already set up, just type a message."

### Re-pairing

To re-pair a customer (e.g., new Telegram account), you must:

1. Manually update the Postgres row to clear `telegram_chat_id`, set `status = 'provisioned'`, and set a new pairing token with expiry.
2. Send the customer a new deep link.

---

## 8. Heartbeat CronJob

The heartbeat system enables proactive agent behavior (e.g., morning briefings, reminder delivery) without an inbound message.

### How It Works

1. A cluster-level CronJob runs `heartbeat-all.sh` on a schedule.
2. The script queries Postgres for all customers with `status = 'active'`.
3. For each active customer, it POSTs to `http://mika-{uuid}.mika-agents.svc.cluster.local:8080/heartbeat` with Bearer token authentication.
4. The agent container has built-in pre-filters that decide whether to act:
   - **Active hours:** Only runs during 8:00-21:00 in the customer's local timezone.
   - **Rate limit:** Maximum 1 heartbeat per hour, 3 per day.
   - **Suppression:** Skips if the user sent a message within the last 2 hours.
5. If the agent decides to act, it runs a silent agent loop where output must be sent explicitly via the `send_message` tool.

### Setup

```bash
export DATABASE_URL="postgres://mika:password@postgres-host:5432/mika_gateway"
export MIKA_INTERNAL_TOKEN="<64-char-hex>"

# Test with dry run first
./scripts/heartbeat-all.sh --dry-run

# Run for real
./scripts/heartbeat-all.sh
```

### Recommended CronJob Schedule

Since the agent containers handle their own active-hours filtering, the CronJob can fire frequently. Recommended: every 30 minutes.

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: mika-heartbeat
  namespace: mika-system
spec:
  schedule: "*/30 * * * *"
  concurrencyPolicy: Forbid
  successfulJobsHistoryLimit: 3
  failedJobsHistoryLimit: 3
  jobTemplate:
    spec:
      backoffLimit: 1
      activeDeadlineSeconds: 300
      template:
        spec:
          restartPolicy: Never
          containers:
            - name: heartbeat
              image: bitnami/postgresql:14
              command: ["/bin/bash", "/scripts/heartbeat-all.sh"]
              env:
                - name: DATABASE_URL
                  valueFrom:
                    secretKeyRef:
                      name: mika-gateway-secrets
                      key: database-url
                - name: MIKA_INTERNAL_TOKEN
                  valueFrom:
                    secretKeyRef:
                      name: mika-gateway-secrets
                      key: internal-token
              volumeMounts:
                - name: scripts
                  mountPath: /scripts
          volumes:
            - name: scripts
              configMap:
                name: mika-heartbeat-scripts
                defaultMode: 0755
```

Create the ConfigMap from the script:

```bash
kubectl create configmap mika-heartbeat-scripts \
  --from-file=heartbeat-all.sh=./scripts/heartbeat-all.sh \
  -n mika-system
```

### Heartbeat Output

```
Triggering heartbeat for 5 active customer(s)...
Done. Succeeded: 4, Failed: 1
```

Failed heartbeats are logged with the customer ID and HTTP status code. Common non-error status codes:
- **200:** Heartbeat accepted, agent will process.
- **204:** Heartbeat skipped by pre-filter (outside active hours, rate limited, etc.).
- **429:** Agent is busy processing another request.

---

## 9. Deprovisioning

The `deprovision.sh` script permanently removes a customer and all their data.

### Usage

```bash
./scripts/deprovision.sh <customer_id> [--force] [--output json] [--dry-run]
```

### Required Environment Variables

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | Postgres connection string for gateway DB |

### Deprovisioning Steps

The script performs five steps in order:

| Step | Action | Effect |
|------|--------|--------|
| 1 | Mark `suspended` in Postgres | Stops message routing immediately |
| 2 | `helm uninstall mika-{uuid}` | Removes Deployment and Service |
| 3 | Delete PVC `mika-{uuid}-data` | Destroys SQLite database (conversations, memory, reminders) |
| 4 | Delete K8s secret `mika-{uuid}-secrets` | Removes API key and token |
| 5 | DELETE from Postgres `customers` table | Removes registration |

Step 1 runs first so that routing stops immediately, even before the container is removed. This prevents messages from arriving at a container that is being torn down.

Each step is idempotent -- if the resource does not exist, the step logs a warning and continues.

### Example: Interactive Deprovisioning

```bash
export DATABASE_URL="postgres://mika:password@postgres-host:5432/mika_gateway"

./scripts/deprovision.sh a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Output:

```
=== Mika Customer Deprovisioning ===
Customer ID: a1b2c3d4-e5f6-7890-abcd-ef1234567890
Name:        Jane Doe
Namespace:   mika-agents

WARNING: This will permanently destroy all data for this customer.
  - SQLite database (conversation history, memory, reminders)
  - K8s secret
  - Postgres registration

Type the customer ID to confirm: a1b2c3d4-e5f6-7890-abcd-ef1234567890
Step 1/5: Marking suspended in Postgres...
Step 2/5: Uninstalling Helm release...
Step 3/5: Deleting PVC...
Step 4/5: Deleting K8s secret...
Step 5/5: Removing from Postgres...

=== Deprovisioning Complete ===
Customer a1b2c3d4-e5f6-7890-abcd-ef1234567890 has been fully deprovisioned.
```

### Example: Non-Interactive (CI/Automation)

```bash
./scripts/deprovision.sh a1b2c3d4-e5f6-7890-abcd-ef1234567890 --force --output json
```

Returns:

```json
{"customer_id":"a1b2c3d4-e5f6-7890-abcd-ef1234567890","status":"success"}
```

### Example: Dry Run

```bash
./scripts/deprovision.sh a1b2c3d4-e5f6-7890-abcd-ef1234567890 --dry-run
```

Output:

```
[DRY RUN] Would execute:
  1. Mark suspended in Postgres
  2. Helm uninstall mika-a1b2c3d4-e5f6-7890-abcd-ef1234567890
  3. Delete PVC mika-a1b2c3d4-e5f6-7890-abcd-ef1234567890-data
  4. Delete K8s secret mika-a1b2c3d4-e5f6-7890-abcd-ef1234567890-secrets
  5. Remove Postgres row
```

### Safety

- **Interactive by default:** Requires typing the full customer UUID to confirm.
- **Pipe protection:** Aborts if stdin is not a terminal (prevents accidental execution in scripts). Use `--force` for automation.
- **UUID validation:** Customer ID must be a valid lowercase UUID format.

---

## 10. Helm Values Reference

### mika-customer (Per-Customer Chart)

File: `helm/mika-customer/values.yaml`

| Key | Default | Description |
|-----|---------|-------------|
| `customer.id` | `""` | Customer UUID (set by `provision.sh`) |
| `customer.name` | `""` | Human-readable customer name |
| `customer.plan` | `"standard"` | Plan tier: `standard` or `premium` |
| `customer.timezone` | `"UTC"` | IANA timezone (e.g., `Europe/Berlin`) |
| `image.repository` | `""` | Container image repository (e.g., ECR URL) |
| `image.tag` | `"latest"` | Image tag |
| `image.pullPolicy` | `IfNotPresent` | Image pull policy |
| `imagePullSecrets` | `[]` | Image pull secret names |
| `resources.requests.memory` | `"64Mi"` | Memory request |
| `resources.requests.cpu` | `"50m"` | CPU request |
| `resources.limits.memory` | `"256Mi"` | Memory limit |
| `persistence.size` | `"1Gi"` | PVC size |
| `persistence.storageClass` | `"gp3"` | Storage class |
| `gateway.url` | `"http://mika-gateway.mika-system.svc.cluster.local:8080"` | Gateway URL for outbound messages |
| `logLevel` | `"info"` | Log level |
| `claudeModel` | `""` | Claude model override (empty = default sonnet-4-6) |

**Notes:**
- No CPU limit is set. The agent is I/O-bound (Claude API calls), and CPU limits cause CFS throttling.
- The Deployment uses `Recreate` strategy because the PVC is RWO (ReadWriteOnce) and cannot be mounted by two pods simultaneously.
- The container mounts `/home/mika/.mika` from the PVC and `/tmp` from a 10Mi emptyDir.

### mika-gateway (Shared Gateway Chart)

File: `helm/mika-gateway/values.yaml`

| Key | Default | Description |
|-----|---------|-------------|
| `image.repository` | `""` | Container image repository |
| `image.tag` | `"latest"` | Image tag |
| `image.pullPolicy` | `IfNotPresent` | Image pull policy |
| `imagePullSecrets` | `[]` | Image pull secret names |
| `replicas` | `1` | Number of replicas |
| `resources.requests.memory` | `"32Mi"` | Memory request |
| `resources.requests.cpu` | `"50m"` | CPU request |
| `resources.limits.memory` | `"128Mi"` | Memory limit |
| `telegram.webhookUrl` | `""` | Public webhook URL (e.g., `https://mika.example.com/webhook/telegram`) |
| `logLevel` | `"info"` | Log level |
| `ingress.enabled` | `false` | Enable Ingress resource |
| `ingress.className` | `""` | Ingress class (e.g., `nginx`) |
| `ingress.host` | `""` | Ingress hostname |
| `ingress.annotations` | `{}` | Ingress annotations |
| `ingress.tls` | `[]` | TLS configuration |

**Notes:**
- No CPU limit is set. The gateway is I/O-bound (Telegram API + Postgres).
- The gateway Deployment uses `RollingUpdate` with `maxUnavailable: 0` and `maxSurge: 1` for zero-downtime deployments.
- The Ingress only routes `/webhook/telegram` (Exact path match). The `/send` endpoint is internal-only (ClusterIP).

---

## 11. Security

### Token Generation

All tokens are 64-character hex strings (32 bytes of randomness), generated with:

```bash
openssl rand -hex 32
```

This applies to:
- `MIKA_INTERNAL_TOKEN` -- shared bearer token for gateway-to-container and container-to-gateway auth
- `MIKA_TELEGRAM_WEBHOOK_SECRET` -- validates inbound Telegram webhooks
- Pairing tokens -- per-customer, single-use, 72-hour expiry

### Constant-Time Comparison

All token comparisons use the `subtle` crate's `ConstantTimeEq` trait to prevent timing attacks. Both the webhook secret header and Bearer token middleware use the `constant_time_eq` helper (see gateway `routes.rs`). Token length is validated at startup (must be exactly 64 hex chars) to eliminate length-based timing leaks from `ct_eq`.

### Non-Root Containers

Both images run as user `mika` (UID 1000). The Helm charts enforce this with pod security context: `runAsUser/runAsGroup: 1000`, `runAsNonRoot: true`, `fsGroup: 1000`, and `seccompProfile: RuntimeDefault`. Container-level security context sets `allowPrivilegeEscalation: false`, drops all capabilities, and enables `readOnlyRootFilesystem: true`. See the Helm chart templates in `helm/mika-customer/` and `helm/mika-gateway/` for the exact YAML.

### Read-Only Root Filesystem

Both containers use `readOnlyRootFilesystem: true`. Writable paths:
- **Agent:** `/home/mika/.mika` (PVC for SQLite), `/tmp` (10Mi emptyDir)
- **Gateway:** `/tmp` (10Mi emptyDir)

### Service Account Token

Both deployments set `automountServiceAccountToken: false` to prevent pods from accessing the Kubernetes API.

### Encrypted Volumes

Data at rest is encrypted at the cloud provider level. SQLite databases are stored as plaintext on Kubernetes PVCs backed by encrypted storage (e.g., AWS EBS gp3 with default encryption). Mika does not implement application-level encryption.

### SSRF Prevention

Container URLs are computed deterministically from the customer UUID via the `container_url()` function in the gateway. No user-controlled input influences the URL. The gateway never forwards to arbitrary destinations.

### Request Body Limits

- `/webhook/telegram` -- 64 KB limit (Telegram updates are small)
- `/send` -- 256 KB limit (outbound messages)

### Webhook Concurrency

The gateway uses a semaphore to limit concurrent webhook processing. When at capacity, it returns 503 (Service Unavailable) and Telegram retries automatically.

### Secret Redaction

Both `GatewaySettings` and `AppState` implement custom `Debug` traits that redact all secret values as `[REDACTED]`.

---

## 12. Troubleshooting

### 429 Agent Busy

**Symptom:** Container returns HTTP 429 to the gateway. User sees "I'm having trouble right now."

**Cause:** The agent container serializes agent loop execution with a `tokio::sync::Mutex`. If one request is being processed, subsequent requests are rejected with 429 (non-blocking `try_lock`).

**Resolution:** This is expected behavior during long Claude API calls. The gateway resets the Telegram dedup counter so Telegram can retry. If the user sends another message, it will be processed after the current one completes. If this happens persistently, check for stuck agent loops via container logs.

```bash
kubectl logs -n mika-agents deploy/mika-<uuid> --tail=100
```

### Pairing Token Expired

**Symptom:** Customer clicks the deep link and sees "Invalid or expired invite link."

**Cause:** Pairing tokens expire after 72 hours.

**Resolution:** Generate a new pairing token manually:

```bash
NEW_TOKEN=$(openssl rand -hex 32)
psql "${DATABASE_URL}" -c "
  UPDATE customers
  SET pairing_token = '${NEW_TOKEN}',
      pairing_expires_at = now() + interval '72 hours',
      status = 'provisioned'
  WHERE id = '<customer-uuid>';
"
echo "New deep link: https://t.me/<bot_username>?start=${NEW_TOKEN}"
```

### Bot Blocked by User

**Symptom:** Gateway logs `bot blocked by user` with HTTP 410 (Gone) from `/send`.

**Cause:** The customer blocked the Telegram bot or deleted their Telegram account.

**Resolution:** The gateway returns 410 to the container. The container stores the message in `failed_sends` for later delivery. No operator action is required unless the customer reports the issue. They need to unblock the bot in Telegram.

### Failed Sends Accumulating

**Symptom:** The container's `failed_sends` table grows.

**Cause:** The gateway was unreachable or Telegram API was down when the container tried to send.

**Resolution:** Failed sends are automatically flushed (up to 5 at a time) before each inbound message is processed. If the gateway is restored, the next inbound message will trigger delivery of pending messages. Check gateway health:

```bash
kubectl exec -n mika-system deploy/mika-gateway -- wget -qO- http://localhost:8080/readyz
```

### Container Crash Loop

**Symptom:** Customer pod is in `CrashLoopBackOff`.

**Cause:** Common causes include:
- Missing or invalid `MIKA_ANTHROPIC_API_KEY`
- Corrupted SQLite database
- Missing PVC mount

**Resolution:**

```bash
# Check pod events
kubectl describe pod -n mika-agents -l mika.io/customer-id=<uuid>

# Check container logs (previous instance)
kubectl logs -n mika-agents deploy/mika-<uuid> --previous

# Verify secret exists
kubectl get secret mika-<uuid>-secrets -n mika-agents

# Verify PVC is bound
kubectl get pvc mika-<uuid>-data -n mika-agents
```

If the SQLite database is corrupted, the simplest recovery is to delete the PVC and let the container bootstrap a fresh database on restart. This loses all conversation history and memory.

```bash
# Scale down first
kubectl scale deploy mika-<uuid> -n mika-agents --replicas=0

# Delete PVC
kubectl delete pvc mika-<uuid>-data -n mika-agents

# Scale back up (Helm will recreate the PVC on next reconcile, or re-run provision)
kubectl scale deploy mika-<uuid> -n mika-agents --replicas=1
```

### Gateway Cannot Reach Postgres

**Symptom:** `/readyz` returns 503. Gateway logs show database connection errors.

**Resolution:**

```bash
# Check gateway logs
kubectl logs -n mika-system deploy/mika-gateway --tail=50

# Verify the secret contains a valid DATABASE_URL
kubectl get secret mika-gateway-secrets -n mika-system -o jsonpath='{.data.database-url}' | base64 -d

# Test Postgres connectivity from within the cluster
kubectl run pg-test --rm -it --image=postgres:14 --restart=Never -- \
  psql "postgres://mika:password@postgres-host:5432/mika_gateway" -c "SELECT 1;"
```

### Webhook Not Receiving Updates

**Symptom:** Messages sent to the bot in Telegram are not arriving at the gateway.

**Resolution:**

1. Verify the webhook is registered:

```bash
curl "https://api.telegram.org/bot${MIKA_TELEGRAM_BOT_TOKEN}/getWebhookInfo"
```

2. Check for `pending_update_count` in the response. If high, the webhook URL may be unreachable.

3. Verify the Ingress is routing correctly:

```bash
kubectl get ingress -n mika-system
curl -v https://mika.example.com/webhook/telegram
```

4. Check that the webhook secret matches between Telegram registration and the `mika-gateway-secrets` K8s secret.

### Message Deduplication

**Symptom:** Duplicate messages being processed.

**Cause:** The gateway uses Telegram's `update_id` for deduplication via an atomic CAS (compare-and-swap) UPDATE in Postgres. If the container is unreachable, the dedup counter is rolled back so Telegram's retry succeeds.

**Resolution:** This is normally self-healing. If duplicates persist, check for multiple gateway replicas racing on the same update. The semaphore limits concurrent processing, but with multiple replicas, each has its own semaphore. The Postgres CAS ensures only one wins.

### Unsupported Media Types

**Symptom:** User sends a sticker, voice, or video message and gets a reply about unsupported media.

**Cause:** The gateway supports text and image messages. Other media types (stickers, voice, video, etc.) trigger a friendly error reply:

> "I can read text and image messages. This media type isn't supported yet."

This is expected behavior, not a bug.
