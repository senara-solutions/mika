---
title: Enabling an automated PR author without checking the gates it must pass blocks every PR it opens
tags:
  - mika-platform
  - workflow
  - ci
  - dependabot
  - verify-pipeline
  - missing-surface
module: .github/workflows/ci.yml
problem_type: workflow_issue
category: workflow-issues
severity: high
created: 2026-08-27
---

# Enabling an automated PR author without checking the gates it must pass blocks every PR it opens

## Symptom

`.github/dependabot.yml` was activated on `mika` (PR#1995 / mika#1729) at 2026-08-26 13:03Z.
Dependabot immediately opened 11 PRs. **All 11 failed CI**, every one of them on the same job:

```
[pipeline-exempt: none] REJECT: code-only PR: source changes present but no plan/solution doc
        Add 'Pipeline-Exempt: code-only — <reason>' trailer to a commit
Verification FAILED: 1 missing artifact(s).
```

The dependency-update flow shipped that day could not merge a single PR.

## Root cause

`scripts/verify-pipeline.sh` requires every source-touching PR to carry a plan or solution
doc. Its only escape hatch is the `Pipeline-Exempt:` commit trailer (mika#860), which must
be added **by hand to a commit**. Dependabot writes neither — it opens a bump PR with a
templated body and no repo docs, by design.

The `pipeline-artifacts` job was already branch-aware (`release/`, `release-please--`) but
had no entry for `dependabot/**`. So the gate did exactly what it was written to do; nobody
had asked whether the new author could satisfy it.

## Fix

One line, staying in the job's existing idiom (mika#2010):

```yaml
      !startsWith(github.head_ref, 'dependabot/')
```

The gate is not weakened: any head_ref outside those prefixes still REJECTs a code-only PR
with no plan. Real CI failures on bump PRs (e.g. #2008, `opentelemetry_sdk` 0.31→0.32
breaking `SpanExporter`) are still caught — by `Check`/Clippy, which is their job.

## The class, for next time

**Turning on an automated PR author is shipping half a flow.** The other half is the path
that lets its output through the gates. The producer and the gate are built by different
people at different times, and neither side's tests fail — the producer produces, the gate
gates, and the work piles up in between.

Checklist before enabling any automated author (Dependabot, Renovate, a bot, a scheduled
job, a pilot loop):

1. List every required check the PRs will face.
2. For each, ask: **can this author physically produce what the check demands?** Not "is it
   reasonable to expect" — can it, mechanically, with no human in the loop.
3. Where the answer is no, add the exemption **in the same PR that enables the author**.
4. Verify empirically on the first PR it opens, rather than assuming.

Related: mika#1996 (a missing surface is not a chance to recover later), mika#1997 (same
gap waiting on mika-cloud and mika-platform, where dependabot.yml is not yet enabled).
