# Plan: perf(permission-policy): mika#768 — superseded by mika#1193

**Ticket:** mika issue#768
**Type:** bug/perf
**Status:** SUPERSEDED — close with no code changes

## Summary

mika#768 proposed adding an output-format directive to `skills/bundled/permission-policy/system_prompt.md` to suppress haiku's reasoning paragraphs after JSON responses, saving ~$0.28 and ~10 minutes per pilot run.

## Why this is superseded

mika#1193 ("retire mika-relay agent + permission-policy skill") was merged as commit `50e13e59` on 2026-05-30. That PR:

1. Deleted the entire `skills/bundled/permission-policy/` directory
2. Removed mika-relay from the well-known agent provisioning
3. Replaced LLM-based permission classification with a deterministic policy file

The target file (`skills/bundled/permission-policy/system_prompt.md`) no longer exists. The cost/latency problem described in mika#768 is eliminated at the root — there is no LLM call to emit reasoning anymore.

## Action

Close mika#768 as superseded by mika#1193. No code changes needed.

## Previous plan

A prior plan (`2026-05-28-002-perf-768-permission-policy-haiku-reasoning-plan.md`) was drafted before the retirement landed. That plan is now obsolete.
