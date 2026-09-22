# Calibration Report: mika-arch

**Model:** openrouter/moonshotai/kimi-k3
**Date:** 2026-09-22 14:36 UTC

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 81.0% (8/10) |
| Input tokens | 19725 |
| Output tokens | 46396 |
| Total latency | 1258474ms |
| Confidence | single-shot |

## Failure Breakdown

| Class | Count |
|-------|-------|
| ContractViolation | 2 |

## Per-Scenario Results

| Scenario | Result | Latency | Tokens (in/out) | Failure Class |
|----------|--------|---------|-----------------|---------------|
| groom_ticket_basic | PASS | 76872ms | 454/2308 | - |
| groom_milestone | PASS | 196247ms | 507/6210 | - |
| citation_discipline | PASS | 49332ms | 391/930 | - |
| disposition_keyword_discipline | PASS | 120813ms | 455/2156 | - |
| required_finding_list | PASS | 68508ms | 385/1984 | - |
| review_anchor_attestation | PASS | 120901ms | 925/2346 | - |
| groomed_no_tbds_passes | FAIL | 294549ms | 4169/8192 | ContractViolation |
| groomed_with_tbd_rejected | FAIL | 49577ms | 4096/8192 | ContractViolation |
| groomed_with_placeholder_path_rejected | PASS | 54068ms | 4108/7856 | - |
| fire_disposition_gate | PASS | 227607ms | 4235/6222 | - |
