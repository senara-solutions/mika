# Calibration Report: mika-qa

**Model:** openrouter/z-ai/glm-5.2
**Date:** 2026-09-09 14:36 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 0.0% (0/1) |
| Input tokens | 32762 |
| Output tokens | 14954 |
| Total latency | 178780ms |
| Confidence | single-shot |

## Failure Breakdown

| Class | Count |
|-------|-------|
| ContractViolation | 1 |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| negative_test_invariant_gate | FAIL | 178780ms | 32762/14954 | ContractViolation |
