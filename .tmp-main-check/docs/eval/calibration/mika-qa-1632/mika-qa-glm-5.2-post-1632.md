# Calibration Report: mika-qa

**Model:** zai/glm-5.2
**Date:** 2026-06-30 11:15 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 100.0% (5/5) |
| Input tokens | 3488 |
| Output tokens | 4539 |
| Total latency | 84110ms |
| Confidence | single-shot |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| verdict_format_precision | PASS | 9492ms | 594/427 | - |
| per_ac_enumeration | PASS | 14221ms | 657/781 | - |
| absence_claim_grounding | PASS | 23487ms | 500/1276 | - |
| wip_rescue_skip | PASS | 16597ms | 525/903 | - |
| no_fabricated_fix | PASS | 20313ms | 1212/1152 | - |
