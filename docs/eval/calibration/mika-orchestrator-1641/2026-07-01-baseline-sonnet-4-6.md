# Calibration Report: mika-orchestrator

**Model:** anthropic/claude-sonnet-4-6
**Date:** 2026-07-01 19:36 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 61.1% (3/5) |
| Input tokens | 2387 |
| Output tokens | 4175 |
| Total latency | 95801ms |
| Confidence | single-shot |

## Failure Breakdown

| Class | Count |
|-------|-------|
| ContractViolation | 2 |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| substrate_wedge_diagnosis | FAIL | 21795ms | 659/997 | ContractViolation |
| ticket_framing_hard_evidence | PASS | 16924ms | 333/679 | - |
| sibling_pr_collision_recovery | PASS | 23324ms | 566/1044 | - |
| deploy_gate_discipline | FAIL | 22761ms | 427/946 | ContractViolation |
| escalation_vs_derivable | PASS | 10997ms | 402/509 | - |
