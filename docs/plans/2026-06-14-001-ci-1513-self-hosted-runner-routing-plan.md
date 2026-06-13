---
ticket: mika#1513
branch: ci/1513/self-hosted-runner
status: active
date: 2026-06-14
origin: https://github.com/senara-solutions/mika/issues/1513
execution: code
---

# Plan: route heavy CI jobs to gentux self-hosted runner + cancel superseded runs (mika#1513)

## Problem frame

mika#1513 documented the `##[error]The runner has received a shutdown signal` failure pattern: GitHub Actions evicts the `Check` job mid-`cargo test` under runner-pool saturation when the 7-job Docker matrix × open PRs exceeds the senara org's Team-plan concurrent-job cap. Earlier framing pointed at billing/quota; root cause is the concurrent-job pool limit (60 standard-runner jobs across all senara repos) per samidarko-claude's runner specs.

Self-hosted runner `gentux-runner-1` is now online (verified via `gh api /repos/senara-solutions/mika/actions/runners`) with labels `self-hosted, Linux, X64, gentux, docker`. Routing the heavy jobs there bypasses the org pool entirely.

## Scope boundaries

**In scope:**
- Add workflow-top `concurrency` block with `cancel-in-progress: true` to kill superseded runs on the same ref
- Route 3 heaviest jobs to the self-hosted runner: `check`, `docker-build-filter`, `docker-build`
- Keep lighter jobs on `ubuntu-22.04` (parity + load distribution)

**Out of scope:**
- Promoting the runner to org-level scope (needs `admin:org` from Vincent — pending; tracked in samidarko-claude memory)
- OpenRC service for runner auto-restart (Stage 3, root-claude's domain)
- Fork-PR security hardening (separate review; mika has no external contributors today)
- Migrating other workflows (`pr-body-validation.yml`, `release-pr.yml`, `release.yml`, `publish-ui.yml`) — out of scope, can do as follow-up if pool pressure persists

## Implementation Units

### U1 — Add concurrency cancel-in-progress block

**Goal:** Superseded runs on the same ref get cancelled, freeing pool slots immediately.

**File:** `.github/workflows/ci.yml`

**Approach:** Insert at workflow top after `permissions:`:

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

**Verification:** Pushing two commits in quick succession to the same PR should cancel the first run before completion.

### U2 — Route Check job to gentux self-hosted

**Goal:** The heaviest job (Rust test suite, ~3463 tests) runs on dedicated hardware, off the org pool.

**File:** `.github/workflows/ci.yml` — `check:` job's `runs-on:`

**Approach:**

```yaml
check:
  name: Check
  runs-on: [self-hosted, Linux, X64, gentux]
```

**Verification:** Post-merge CI run shows `runner_name: gentux-runner-1` on the Check job.

### U3 — Route Docker Build Filter to gentux self-hosted

**Goal:** The filter job that gates Docker Build runs alongside its consumer.

**File:** `.github/workflows/ci.yml` — `docker-build-filter:` job's `runs-on:`

**Approach:**

```yaml
docker-build-filter:
  name: Docker Build Filter
  runs-on: [self-hosted, Linux, X64, gentux]
```

**Verification:** Post-merge CI run shows `runner_name: gentux-runner-1` on the Docker Build Filter job.

### U4 — Route Docker Build matrix to gentux self-hosted (with docker label)

**Goal:** The 7-job Docker matrix runs on gentux where `docker` is installed (28.2.2).

**File:** `.github/workflows/ci.yml` — `docker-build:` job's `runs-on:`

**Approach:**

```yaml
docker-build:
  name: Docker Build
  runs-on: [self-hosted, Linux, X64, gentux, docker]
```

The `docker` label ensures the matrix only runs on runners with Docker installed (defensive — currently only gentux-runner-1 has it, but if/when a second runner registers without Docker, this prevents misrouting).

**Verification:** Post-merge CI run shows `runner_name: gentux-runner-1` on all 7 Docker Build matrix jobs.

## Acceptance Criteria

- AC1: Workflow-top `concurrency` block with `group: ${{ github.workflow }}-${{ github.ref }}` and `cancel-in-progress: true` is present in `.github/workflows/ci.yml`.
- AC2: `check` job's `runs-on` is `[self-hosted, Linux, X64, gentux]`.
- AC3: `docker-build-filter` job's `runs-on` is `[self-hosted, Linux, X64, gentux]`.
- AC4: `docker-build` job's `runs-on` is `[self-hosted, Linux, X64, gentux, docker]`.
- AC5: All other jobs (`dashboard`, `docs-site`, `docs-sync`, `byte-slice-lint`, `loop-select-lint`, `pipeline-artifacts`, `security`) remain on `ubuntu-22.04`.
- AC6: YAML is structurally valid (parses as a workflow).
- AC7: Post-merge: a fresh CI run on `main` (or trivial follow-up PR) shows the 3 targeted jobs land on `gentux-runner-1`; Check completes without `received a shutdown signal`.

## Risk shape

- **Single point of failure**: gentux-runner-1 is the only self-hosted runner. If it goes offline, the routed jobs queue indefinitely. Mitigation: lighter jobs stay on hosted runners (PR still produces signal); samidarko-claude's tmux supervision is interim; root-claude's OpenRC service (Stage 3) is the durable answer.
- **Cache divergence**: `Swatinem/rust-cache@v2` caches per-runner; self-hosted runner builds a cache from scratch on first run. Expected one-time cost; subsequent runs hit warm cache.
- **Docker daemon trust**: Self-hosted runner's Docker daemon is shared across PRs. Existing PRs trust the same daemon already (it's the local host). No new exposure.
- **Concurrency cancellation on main**: `cancel-in-progress: true` means a push to main while a prior main CI is in flight cancels the prior run. Trade-off: latest commit always wins; intermediate validation is lost. Acceptable — main pushes are merges from PRs whose CI already validated.

## References

- mika#1513 — substrate ticket with hard evidence (job log #81188435116)
- mika#1484 — sibling concern (unverified Docker merges); same root cause; this fix partially addresses it
- samidarko-claude inbox: `2026-06-14-001817-from-samidarko-re-self-hosted-runner-online-modify-ci-yml.md` — runner specs + routing recommendation
- Stage 1 follow-up (parked): promote runner to org-scope when Vincent grants `admin:org`
- Stage 3 follow-up (parked, root-claude): OpenRC service for runner auto-restart
