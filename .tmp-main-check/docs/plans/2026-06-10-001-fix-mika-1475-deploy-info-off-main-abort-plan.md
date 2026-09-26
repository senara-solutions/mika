---
status: active
issue: mika#1475
parent: mika-platform#163
companion_ticket: mika-platform#163
created: 2026-06-10
groomed_session_id: 78787485-3f72-4c68-a79c-8cb08c878c93
execution: code
---

# Plan — mika#1475: deploy-info off-main ABORT (defense-in-depth for mika-platform#163)

## Origin

Filed under `senara-solutions/mika-platform#163` as parent (observation + meta-repo contract scope). Implementation lives in this repo because `mika/Makefile` is the bypass surface. Two PRs land together:

- This repo (mika) — `mika/Makefile` enforcement edit + shell test
- mika-platform — `mika-platform/CLAUDE.md § Local Dev Environment` documents the two-layer contract

## Problem frame

`cd mika && make deploy` structurally bypasses `mika-platform-deploy-preflight`. The bypass is a real path used by humans and scripts. The 2026-06-10 incident produced a deployed binary missing mika#1469's fix and a cascade of mika#1471 misfires. The autonomous loop's `merge → deploy → fix-is-live` invariant breaks every time the bypass is taken.

## Decision rationale

### Defense-in-depth, not centralization reversal

mika-platform#139 explicitly rejected sub-repo enforcement as "wrong layer" — its Alternatives Considered item 2 said *"Add the check inside `make -C mika deploy` instead of meta-repo deploy. Wrong layer — the meta-repo's `deploy` target is the canonical entry point per `mika-platform/CLAUDE.md`, and the same drift can happen in `claude-pilot-py/`, `mika-skills/`, `mika-cloud/`. Centralize at the dispatcher."*

#139's framing was sound under its assumption: meta-repo is the only deploy path. The mika#1471 incident proved the assumption was incomplete. Sub-repo `make deploy` is a real bypass used in practice.

This plan does NOT contradict #139's centralization principle. The meta-repo gate stays the canonical strict check (all four sub-repos, on-main + up-to-date). This plan adds a single-purpose safety net at the bypass point. The sub-repo guard is intentionally **weaker** than meta-repo's so the layers don't compete.

### Scope: off-main only, not behind-origin

`FORCE_DEPLOY_FROM_BRANCH=1` is established convention for off-main only. Reusing it for behind-origin would change its semantics (surprising for operators reading the var name literally). Renaming or adding new env vars pollutes a working convention for no incident-class gain (the 2026-06-10 incident WAS off-main, not behind-on-main).

Behind-origin-on-main stays as WARN at sub-repo. Meta-repo preflight remains the strict gate for that case.

## Implementation Units

### U1 — Add off-main ABORT to `mika/Makefile`'s `deploy-info` target

**Goal:** Block `cd mika && make deploy` from a non-main branch unless `FORCE_DEPLOY_FROM_BRANCH=1` is set.

**Files:**
- Modify: `mika/Makefile` — `deploy-info` target (currently lines 56-66)

**Approach:**

Replace the current `deploy-info` target with a stricter check. The new target:

1. Resolves the current branch via `git rev-parse --abbrev-ref HEAD`.
2. If branch != `main` and `FORCE_DEPLOY_FROM_BRANCH` != `1`: print red ABORT message naming the current branch + fix instructions, `exit 1` before any further step runs.
3. If branch != `main` and `FORCE_DEPLOY_FROM_BRANCH` == `1`: print yellow WARN banner naming the override, continue.
4. Otherwise (on main): keep existing freshness check unchanged — print "Building from:" line, fetch origin, print WARNING if behind-origin (no abort), continue.

The ABORT message should mirror `mika-platform-deploy-preflight`'s tone for consistency: red bold, name the override env var, name the canonical meta-repo path as the primary fix.

**Patterns to follow:**
- `scripts/mika-platform-deploy-preflight` lines 117-134 (the off-main + FORCE escape pattern this mirrors)
- Existing `mika/Makefile:56-66` `deploy-info` style (color escapes, `@if` conditional, multi-line shell)

**Execution note:** None — straightforward Makefile edit.

**Test scenarios:**

Covered by U2's shell test. Manual verification before merge:
- Happy path: `make deploy-info` on main → prints "Building from: main @ <sha>", continues.
- ABORT path: checkout a non-main branch, `make deploy-info` → exit != 0, ABORT in stderr, no further targets run.
- FORCE escape: `FORCE_DEPLOY_FROM_BRANCH=1 make deploy-info` on a non-main branch → WARN banner, continues.
- Behind-origin on main: contrived scenario (reset local main to a parent commit, then run `make deploy-info`) → WARN, continues. No ABORT.

**Verification:** AC1, AC2, AC3, AC4 (see Acceptance Criteria below).

### U2 — Shell test asserting off-main ABORT behavior

**Goal:** Lock in the off-main ABORT structurally so a future Makefile rewrite can't silently regress it.

**Files:**
- Create: `mika/tests/scripts/deploy_info_off_main.sh` (or place under existing test infra if mika has a Makefile-test convention — execution discovers)

**Approach:**

Bash test script that:
1. Creates a `mktemp -d` synthetic git repo with an initial commit on `main`.
2. Copies (or symlinks) the relevant `deploy-info` rule into a minimal `Makefile` in the temp repo. Alternative: invoke `make -C $REAL_MIKA deploy-info` after `cd $TEMP_REPO_ON_BRANCH` if it's simpler — execution decides.
3. Checks out a non-main branch in the temp repo.
4. Runs `make deploy-info` in the temp repo without `FORCE_DEPLOY_FROM_BRANCH=1`.
5. Asserts exit code != 0.
6. Asserts stderr contains "ABORT" and the branch name.
7. Repeats with `FORCE_DEPLOY_FROM_BRANCH=1` → asserts exit 0 + WARN in output.
8. Cleanup `rm -rf $TEMP_REPO`.

**Patterns to follow:**
- Look in `mika/tests/` for existing shell-test conventions. If a runner exists (e.g., `bats`, plain bash with `set -e`), use it. Otherwise plain bash with explicit `if [ $? -ne 0 ] then fail` shape.
- `scripts/mika-platform-deploy-preflight` test pattern if mika-platform has one — mirror the structural test approach.

**Execution note:** Structural test (AST-shape, not real git ops on the actual repo). Test must NOT mutate the real working repo or live `~/.local/bin/mika-spirit`.

**Test scenarios:**
- Off-main without FORCE: exit != 0, ABORT message present
- Off-main with FORCE: exit 0, WARN message present
- On main: exit 0 (sanity — must not regress the happy path)

**Verification:** AC5.

### U3 — (Companion, separate PR on mika-platform repo) `mika-platform/CLAUDE.md` documents the two-layer contract

**Goal:** Document the new contract in workspace docs so future readers don't re-litigate #139.

**Files:**
- Modify: `mika-platform/CLAUDE.md` § Local Dev Environment (the section that currently describes `scripts/mika-platform-deploy-preflight`)

**Approach:**

Add a paragraph after the existing preflight-promise paragraph:

> **Two-layer contract.** The meta-repo `make deploy` runs the preflight gate as a workspace-level invariant (all sub-repos on main + up-to-date). For defense-in-depth against the `cd <sub-repo> && make deploy` bypass, each sub-repo's `make deploy` (currently mika; others as they ship) also enforces an off-main ABORT at the sub-repo layer — narrower than the meta-repo gate (off-main only, not behind-origin), single-purpose: block the structural bypass class. `FORCE_DEPLOY_FROM_BRANCH=1` semantics are identical at both layers.

This PR is OUT OF SCOPE for the mika impl PR. File it as a separate mika-platform PR after the mika PR merges (so the doc edit references the merged behavior, not promised behavior). Track separately.

**Verification:** AC4 (doc updated).

## Acceptance Criteria

1. **AC1** — `cd mika && make deploy` on a non-main branch (without `FORCE_DEPLOY_FROM_BRANCH=1`) ABORTs in `deploy-info` before any cargo invocation. Exit != 0. (U1)
2. **AC2** — `FORCE_DEPLOY_FROM_BRANCH=1 make deploy` on a non-main branch: WARN banner, proceeds. Exit 0. (U1)
3. **AC3** — `cd mika && make deploy` on main with HEAD behind origin/main: WARN (existing behavior preserved). Exit 0. (U1) — behind-origin enforcement stays at meta-repo gate.
4. **AC4** — `cd mika-platform && make deploy` continues to work unchanged. Meta-repo preflight runs first; if it passes (all on main + up-to-date), mika sub-repo's stricter `deploy-info` also passes. (U1 + verified by no-regression on the meta-repo gate.)
5. **AC5** — Shell test for AC1 + AC2 paths committed; running it (via whatever test harness mika uses) produces a pass. (U2)
6. **AC6** — Companion `mika-platform/CLAUDE.md` documents the two-layer contract. (U3 — separate PR.)

## Scope Boundaries

**In scope (this mika PR):**
- `mika/Makefile` `deploy-info` target edit
- Shell test for the off-main ABORT path
- The mika impl ticket (mika#1475) closes when this PR merges. Parent mika-platform#163 closes when both this PR AND the companion CLAUDE.md PR land.

**Out of scope:**
- claude-pilot-py + mika-skills sub-repo Makefiles. Same bypass class likely exists. **Investigate during U1 execution** — if their `make deploy` targets have the same structural pattern (deploy-info-style WARN-only), the same Makefile guard can land in this PR (still narrow, ~6 more lines each). If their deploy paths differ structurally, file siblings.
- Post-deploy SHA-stamp verification (parent ticket body §3 — bake `--version` to include git SHA, fail post-deploy smoke if doesn't match origin/main HEAD). Stronger structural fix but bigger scope. Deferred.
- Exported `FORCE_DEPLOY_FROM_BRANCH` leakage audit. n=1 (not yet observed in practice). Deferred per `feedback_hard_evidence_before_filing`.
- Renaming the `FORCE_DEPLOY_FROM_BRANCH` env var. Established convention preserved.

## Risks + named trades

1. **Operator muscle memory hits the ABORT.** Operators (Vincent, future contributors) typing `cd mika && make deploy` on a feature branch will hit the new ABORT. **Trade accepted:** the ABORT message names the override (`FORCE_DEPLOY_FROM_BRANCH=1`) inline; one-time friction in exchange for closing the bypass class. The error message must be informative — exit-quietly is not acceptable.

2. **U2's shell test environment.** Mika may not have a Makefile-test runner. If the test infra is thin, the test may live as a manual-run script under `mika/tests/scripts/` rather than wired into CI. **Trade accepted:** structural lock-in via a tracked test file is sufficient even if not auto-run; CI integration can land as follow-up if mika gains a Makefile-test runner.

3. **claude-pilot-py + mika-skills sibling bypass.** If their `make deploy` has identical structural bypass shape, leaving them unfixed leaks the contract. **Mitigation:** U1 execution audit checks; same-PR fix if shape matches, sibling ticket if not.

## Non-goals

- This plan does NOT modify `scripts/mika-platform-deploy-preflight`. Meta-repo gate stays as-is.
- This plan does NOT change `FORCE_DEPLOY_FROM_BRANCH` semantics or naming.
- This plan does NOT introduce a behind-origin ABORT at sub-repo level. That stays at meta-repo.

## Verification (end-to-end)

After merging the mika PR:
1. Manually verify on a clean checkout: `cd mika && git checkout -b test/abort-verify` → `make deploy-info` → assert exit != 0 with ABORT message.
2. Manually verify FORCE escape: `FORCE_DEPLOY_FROM_BRANCH=1 make deploy-info` on the same branch → assert exit 0 with WARN.
3. Cleanup: `git checkout main && git branch -D test/abort-verify`.
4. Run the shell test (U2): `bash mika/tests/scripts/deploy_info_off_main.sh` → assert pass.
5. Verify meta-repo path still works: `cd mika-platform && bash scripts/mika-platform-deploy-preflight` → expected outputs unchanged.

After the companion mika-platform PR (U3) merges:
6. Read mika-platform/CLAUDE.md § Local Dev Environment → assert the two-layer contract paragraph is present and accurate.

## Grooming history

- First-pass (ITERATE) — 3 sharpenings: F1 cross-repo placement ambiguity, F2 #139 "wrong layer" reversal needs framing, F3 FORCE escape doesn't semantically cover behind-origin.
- Revised brief addressed all three: cross-repo split explicit, defense-in-depth reframe added, dropped behind-origin AC at sub-repo level.
- Second-pass (GROOMED) — session `78787485-3f72-4c68-a79c-8cb08c878c93`.
