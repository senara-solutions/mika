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
[mika-gateway]  (mika-system namespace, from mika-cloud repo)
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

The gateway lives in the private [mika-cloud](https://github.com/senara-solutions/mika-cloud) repo. See that repo for gateway deployment, Helm charts, provisioning/deprovisioning scripts, and Telegram setup.

---

## 2. Prerequisites

| Requirement | Details |
|-------------|---------|
| Kubernetes cluster | 1.27+ (tested on EKS) |
| Anthropic API key | For Claude API access |
| Container registry | ECR, GHCR, or similar |
| `kubectl` | Configured with cluster access |
| `helm` | v3.12+ |

---

## 3. Building the Agent Docker Image (~95MB)

```bash
docker build -f Dockerfile.agent -t mika-agent:dev .
```

Build details:
- **Builder:** `rust:1.85-slim` with gcc, libc-dev, pkg-config, libssl-dev
- **Runtime:** `debian:bookworm-slim` with ca-certificates, wget, file, jq, and gh (GitHub CLI v2.65.0 with SHA256 checksum verification)
- **Binary:** `mika-server` (Axum HTTP server)
- **User:** `mika` (UID 1000), non-root
- **Port:** 8080
- **Healthcheck:** `wget -q --spider http://localhost:8080/health` (10s interval, 5s start period)
- **Config:** Default config copied to `/app/config/default.toml`
- **Data dir:** `/home/mika/.mika` (PVC mount point)

### Pushing to Registry

```bash
docker tag mika-agent:dev 123456789.dkr.ecr.us-east-1.amazonaws.com/mika-agent:v0.1.0
docker push 123456789.dkr.ecr.us-east-1.amazonaws.com/mika-agent:v0.1.0
```

### Dependency Caching

The Dockerfile uses a dependency caching strategy:

1. Copy only `Cargo.toml`, `Cargo.lock`, and crate manifests.
2. Create dummy source files and build dependencies.
3. Remove workspace crate artifacts but keep dependency cache.
4. Copy real source code and rebuild (only workspace crates recompile).

Rebuilds after source changes are fast because dependency compilation is cached in the Docker layer.

---

## 4. Helm Values Reference (mika-customer)

File: `helm/mika-customer/values.yaml`

| Key | Default | Description |
|-----|---------|-------------|
| `customer.id` | `""` | Customer UUID (set by provisioning script) |
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

---

## 5. Security

### Token Generation

All tokens are 64-character hex strings (32 bytes of randomness), generated with:

```bash
openssl rand -hex 32
```

### Constant-Time Comparison

Token comparisons use the `subtle` crate's `ConstantTimeEq` trait to prevent timing attacks. Token length is validated at startup (must be exactly 64 hex chars).

### Non-Root Containers

The agent image runs as user `mika` (UID 1000). The Helm chart enforces this with pod security context: `runAsUser/runAsGroup: 1000`, `runAsNonRoot: true`, `fsGroup: 1000`, and `seccompProfile: RuntimeDefault`. Container-level security context sets `allowPrivilegeEscalation: false`, drops all capabilities, and enables `readOnlyRootFilesystem: true`.

### Read-Only Root Filesystem

The agent container uses `readOnlyRootFilesystem: true`. Writable paths:
- `/home/mika/.mika` (PVC for SQLite)
- `/tmp` (10Mi emptyDir)

### Service Account Token

Deployments set `automountServiceAccountToken: false` to prevent pods from accessing the Kubernetes API.

### Encrypted Volumes

SQLite databases are stored as plaintext on Kubernetes PVCs backed by encrypted storage (e.g., AWS EBS gp3 with default encryption). Mika does not implement application-level encryption.

---

## 6. Troubleshooting

### 429 Agent Busy

**Symptom:** Container returns HTTP 429. User sees "I'm having trouble right now."

**Cause:** The agent container serializes agent loop execution with a `tokio::sync::Mutex`. If one request is being processed, subsequent requests are rejected with 429 (non-blocking `try_lock`).

**Resolution:** Expected behavior during long Claude API calls. If persistent, check container logs:

```bash
kubectl logs -n mika-agents deploy/mika-<uuid> --tail=100
```

### Container Crash Loop

**Symptom:** Customer pod is in `CrashLoopBackOff`.

**Common causes:**
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

If the SQLite database is corrupted, delete the PVC and let the container bootstrap a fresh database. This loses all conversation history and memory.

```bash
kubectl scale deploy mika-<uuid> -n mika-agents --replicas=0
kubectl delete pvc mika-<uuid>-data -n mika-agents
kubectl scale deploy mika-<uuid> -n mika-agents --replicas=1
```

### Failed Sends Accumulating

**Symptom:** The container's `failed_sends` table grows.

**Cause:** The routing endpoint was unreachable when the container tried to send.

**Resolution:** Failed sends are automatically flushed (up to 5 at a time) before each inbound message. If the routing endpoint is restored, the next inbound message triggers delivery of pending messages.

---

## 7. Related Repositories

- **[mika-cloud](https://github.com/senara-solutions/mika-cloud)** (private) — Gateway deployment, Helm chart, provisioning/deprovisioning scripts, Telegram bot setup, heartbeat CronJob, customer pairing flow
