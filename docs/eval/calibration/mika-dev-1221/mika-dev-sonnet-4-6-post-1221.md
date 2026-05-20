# Calibration Report: mika-dev

**Model:** anthropic/claude-sonnet-4-6
**Date:** 2026-05-20 14:41 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 0.0% (0/5) |
| Input tokens | 0 |
| Output tokens | 0 |
| Total latency | 1478ms |
| Confidence | single-shot |

## Failure Breakdown

| Class | Count |
|-------|-------|
| TransportError | 5 |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| refusal_regression | FAIL | 608ms | 0/0 | TransportError |
| contract_dev_groom | FAIL | 217ms | 0/0 | TransportError |
| golden_path_dispatch | FAIL | 219ms | 0/0 | TransportError |
| required_tools_gate | FAIL | 218ms | 0/0 | TransportError |
| plan_callout_recognition | FAIL | 216ms | 0/0 | TransportError |
