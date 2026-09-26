# Calibration Report: mika-dev

**Model:** openrouter/z-ai/glm-5.2
**Date:** 2026-06-29
**Ticket:** mika#1633

## Summary

| Metric | Value |
|--------|-------|
| Pass rate | 100% (5/5) |
| Confidence | single-shot |

## Context

Model swap from `anthropic/claude-sonnet-4-6` to `openrouter/z-ai/glm-5.2` for
cost reduction (~50-100x). Calibration run documented in mika#1633 issue body.

## Per-Scenario Results

| Scenario | Result |
|----------|--------|
| refusal_regression | PASS |
| contract_dev_groom | PASS |
| golden_path_dispatch | PASS |
| required_tools_gate | PASS |
| plan_callout_recognition | PASS |
