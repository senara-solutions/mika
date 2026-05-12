---
module: ci
tags: [docs-sync, ci, crate-local-docs, pipeline-gap]
problem_type: ci-failure
category: workflow-issues
date: 2026-05-12
issue: 1082
---

# Doc-sync CI gap when impl updates canonical docs but not crate-local copies

## Problem

PR #1068 (mika#1066 TUI fix) was APPROVED by mika-qa but blocked on the `Docs Sync` CI check for 24+ hours. The implementation updated `docs/slash-commands.md` (canonical) but did not sync to `crates/mika-agent/docs/slash-commands.md` (crate-local copy). Multiple re-dispatches via `/mika mika#1066` returned "DONE" because claude-pilot scoped the impl as complete — the doc-sync gap was downstream of implementation.

## Root Cause

The `/mika` pipeline's `/mika-doc-audit` step checks for documentation **content** accuracy but does not verify crate-local doc copies are in sync with canonical docs. The `Docs Sync` CI job (`ci.yml` § `docs-sync`) is the only guard, and it catches the drift only after push — too late for the pilot to fix it in the same session.

## Fix Applied

Created issue #1082 and pushed a one-line sync commit directly to PR #1068's branch:
```
bash scripts/sync-agent-docs.sh
git add crates/mika-agent/docs/slash-commands.md
git commit && git push
```

## Lesson

When a `/mika` pipeline modifies any file under `docs/` (the canonical source), `scripts/sync-agent-docs.sh` should be run before the final commit to ensure crate-local copies stay in sync. The doc-audit step does not cover this — it audits content accuracy, not file-copy parity.

## Prevention

Consider adding a `sync-agent-docs.sh` call to the `/mika` pipeline after `/mika-doc-audit` or as part of the pre-commit lefthook checks. Until then, any impl that touches `docs/*.md` files should manually run the sync script.
