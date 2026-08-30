---
title: A `printf|echo | grep -q` pipeline is a SIGPIPE trap under pipefail — sweep it, then build the incapacity
date: 2026-08-30
category: best-practices
module: scripts, skills/bundled/_shared
problem_type: best_practice
component: tooling
severity: high
applies_when:
  - "Writing or reviewing a bash assertion of the shape `producer | grep -q PATTERN` under `set -o pipefail`"
  - "Chasing a bash test failure that reports a value as absent when it is present, and does not reproduce on re-run"
  - "Deciding the scope of a structural CI guard over shell source"
related_components:
  - testing_framework
  - development_workflow
tags:
  - sigpipe
  - pipefail
  - grep
  - here-string
  - structural-guard
  - anti-vacuity
  - deny-by-default
  - ci-gate
---

# A `printf|echo | grep -q` pipeline is a SIGPIPE trap under pipefail

## The class

Under `set -o pipefail`, this shape is a probabilistic false failure:

```bash
if printf '%s' "$haystack" | grep -qF "$needle"; then
```

`grep -q` exits at the **first** match and closes the pipe. `printf`, still
writing the rest of the haystack, takes SIGPIPE and exits 141. `pipefail`
promotes 141 to the pipeline's status — so the assertion reports "absent" for a
value that is **present** (a false failure), or masks a real one. The race is
between grep exiting and the producer finishing, so the probability grows with
haystack size and with how early the needle sits: first observed on a 32 KB
haystack whose needle was at line 78 of 489, then not reproduced across four
runs. A non-reproducible red is the most expensive test failure to chase, and
this class was already a merge gate.

Full incident: `docs/solutions/test-failures/bash-assert-sigpipe-and-host-coupling-before-ci-gate-2026-08-29.md` (mika#1772).

## The remedy

A here-string. No pipeline, no SIGPIPE:

```bash
if grep -qF -- "$needle" <<<"$haystack"; then
```

The `--` matters independently: a needle beginning with `-` would otherwise be
read as an option. Preserve the original grep flags (`-qE`, `-qiE`, `-qx`, BRE
alternations) — only the pipeline is at fault, not the match semantics.

## Why prose was not enough — the argument for a guard

`n=2` in days. The incident above documented the trap; the pattern was then
**reintroduced** in mika#2039's canary probe while that document sat unchanged
in the tree. A writeup does not prevent its own recurrence. mika#2055 swept the
57 remaining occurrences across 8 pipefail-exposed files and shipped the
incapacity: `scripts/verify-no-sigpipe-grep.sh`, wired into CI and `make test`.

## Two scope decisions the guard got right

1. **Do not gate on a literal in-file `set … pipefail`.** A shell *library*
   like `skills/bundled/_shared/dispatch-lib.sh` does not arm `pipefail`
   itself — it inherits it by being `source`d into pipefail contexts (its test
   suite, the dispatch handlers). A guard gated on the in-file `set` line would
   miss exactly the file most exposed. You cannot decide pipefail-ness
   statically, so deny the fragile shape across **every bash file** under the
   roots; the here-string remedy is equivalent and safe even outside pipefail.

2. **Exempt pure POSIX `#!/bin/sh`.** `pipefail` is not POSIX and here-strings
   (`<<<`) are undefined there (shellcheck SC3011). In a POSIX sh script with
   no `pipefail`, `producer | grep -q` is neither dangerous nor fixable with the
   here-string — applying the remedy would introduce a regression. The guard
   skips this class (`skills/bundled/address-pr-comments/handlers/run.sh`).

## Anti-vacuity is not optional

A deny-by-default guard is worth shipping only if it can fail. The companion
harness (`scripts/test-verify-no-sigpipe-grep.sh`) proves it: the guard is
clean on the swept tree, goes **red on a deliberately-reintroduced bad
pattern** (the negative control), accepts the here-string, and honours a
per-line `# sigpipe-safe: #<ticket>` escape only with a real ticket citation —
a bare marker does not suppress. Measured decisively: the guard fails on the
pre-sweep tree (53 hits) and passes once swept. See the fail-open siblings in
`docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md`.
