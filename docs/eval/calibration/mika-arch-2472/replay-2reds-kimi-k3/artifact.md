# Calibration Report: mika-arch

**Model:** openrouter/moonshotai/kimi-k3
**Date:** 2026-09-22 15:42 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 0.0% (0/2) |
| Input tokens | 8268 |
| Output tokens | 15121 |
| Total latency | 149492ms |
| Confidence | single-shot |

## Failure Breakdown

| Class | Count |
|-------|-------|
| ContractViolation | 2 |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| groomed_no_tbds_passes | FAIL | 68364ms | 4172/8192 | ContractViolation |
| groomed_with_tbd_rejected | FAIL | 81128ms | 4096/6929 | ContractViolation |
