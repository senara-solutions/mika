# Calibration Report: mika-orchestrator

**Model:** zai/glm-5.2
**Date:** 2026-07-01 19:35 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 61.1% (3/5) |
| Input tokens | 2265 |
| Output tokens | 6541 |
| Total latency | 130748ms |
| Confidence | single-shot |

## Failure Breakdown

| Class | Count |
|-------|-------|
| ContractViolation | 1 |
| EmptyResponse | 1 |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| substrate_wedge_diagnosis | FAIL | 31544ms | 624/2000 | EmptyResponse |
| ticket_framing_hard_evidence | PASS | 16460ms | 322/917 | - |
| sibling_pr_collision_recovery | PASS | 31645ms | 520/1510 | - |
| deploy_gate_discipline | FAIL | 29777ms | 406/1240 | ContractViolation |
| escalation_vs_derivable | PASS | 21322ms | 393/874 | - |
