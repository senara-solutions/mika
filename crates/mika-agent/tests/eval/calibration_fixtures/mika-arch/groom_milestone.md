# Groom Milestone: Observability Stack (Phase 1)

## Milestone: mika#98 — Production Observability

### Goal

Ship baseline observability so operators can monitor agent health, detect regressions,
and debug production issues without SSH access.

### Sub-Issues

#### #101 — Add health endpoint
Basic `GET /health` returning version, uptime, and agent count. Used by K8s
liveness/readiness probes. No auth required. Estimated: 1 story point.

#### #102 — Add Prometheus metrics
Expose `/metrics` endpoint with: request count, latency histogram (p50/p95/p99),
active sessions gauge, LLM call count by provider, error rate by status code.
Uses `metrics` crate with prometheus exporter. Estimated: 3 story points.

#### #103 — Add alerting rules
Define Prometheus alerting rules for: error rate > 5% (5 min window),
p99 latency > 10s, health endpoint down for > 30s, LLM provider errors > 10/min.
Ship as a `alerts.yaml` file for Helm chart integration. Estimated: 2 story points.

### Sequencing Constraints

- #101 must land first (other issues depend on the health check pattern)
- #102 and #103 can be parallelized after #101
- All three must land before the milestone is closed

### Cross-Cutting Concerns

- Feature-gate metrics behind `--features metrics` to avoid binary size increase for non-K8s deploys
- All endpoints must respect the existing `AppState` pattern
