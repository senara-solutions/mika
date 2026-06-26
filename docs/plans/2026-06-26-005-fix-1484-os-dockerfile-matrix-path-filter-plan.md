---
title: "fix(ci): path-filter the os/Dockerfile docker-build matrix to os PRs only"
date: 2026-06-26
type: fix
issue: senara-solutions/mika#1484
branch: ci/1484/os-path-filter-docker-build-matrix-to-os
status: planned
depth: standard
---

# fix(ci): path-filter the os/Dockerfile docker-build matrix to os PRs only

## Summary

The 5 expensive `os/Dockerfile` Gentoo stage3 build jobs currently fire on **every code PR** because they share a single path-filter gate with the fast `Dockerfile.agent`/`Dockerfile.gateway` builds — and that gate triggers on `crates/**` and `Cargo.*`. Split the gate so the 5 os targets run **only** when `os/**` or any Dockerfile changes, while the agent/gateway image builds keep their existing code-change trigger. This is a workflow-YAML-only change in `.github/workflows/ci.yml`; no engine, gate, or branch-protection changes.

---

## Problem frame

**WHY (hard evidence, verified 2026-06-26):**

- `.github/workflows/ci.yml:177-241` defines a single `docker-build-filter` job (`dorny/paths-filter@v3`) that emits one boolean output `docker-relevant`. Its filter paths (lines 188-197) are: `Dockerfile.agent`, `Dockerfile.gateway`, `os/Dockerfile`, `os/Dockerfile.dockerignore`, `Cargo.toml`, `Cargo.lock`, `crates/**`, `.github/workflows/ci.yml`.
- That one boolean gates the **entire** `docker-build` matrix (line 203 `if`), all 7 targets: `mika-agent`, `mika-gateway`, and the 5 `os/Dockerfile` targets `mika-os`, `mika-runtime-server`, `mika-runtime-gateway`, `mika-runtime-cli`, `mika-runtime-all` (verified as build stages in `os/Dockerfile:34,222,243,262,272`). The 5 os targets are Gentoo stage3 builds taking hours each.
- **Consequence:** any `crates/**` or `Cargo.*` change trips all 5 slow os builds even though `os/Dockerfile` is untouched. Verified on two live PRs — PR #1578 (touched `crates/mika-agent/src/*`) and PR #1570 (touched `crates/mika-agent/src/bundled_skills.rs`) both matched `crates/**` and sat with the 5 os jobs pending 2h+ with zero merge-gate value.

The original mika#1484 body raised a second concern (os builds could merge *unverified* due to a merge race). That concern is **closed separately** by mika#1577 (PR #1578, the not-behind-base assertion, merged 2026-06-26) per the ticket's 2026-06-26 retraction comment. This plan addresses only the remaining scope: correctly scoping the matrix via path filter.

---

## Scope boundaries

**In scope:**
- Split `docker-build-filter` to emit two outputs and split the `docker-build` matrix into two jobs (code-image build + os-image build), each gated on its own filter.

**Out of scope (do NOT touch):**
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs`, `crates/mika-agent/src/server/ci_success_handler.rs`, `crates/mika-agent/src/server/verdict_handler.rs` — the merge-gate engine. Unchanged; covered by mika#1577 (merged).
- Branch-protection / required-status-check configuration, GitHub rulesets, merge-queue. Per the ticket's bypass-actor evidence, the autonomous merge actor bypasses rulesets anyway; the in-code gate is the real authority and is explicitly out of scope here.
- The Gentoo `os/Dockerfile` build logic itself, and the agent/gateway Dockerfiles.

### Deferred to follow-up work

None. The change is self-contained.

---

## Key technical decisions

### KTD1. Split the matrix into two jobs rather than one matrix with a dynamic include list

GitHub Actions cannot conditionally drop individual matrix entries based on a filter without computing the matrix JSON dynamically in a step — which is harder to read and review. Splitting into two statically-defined jobs (`docker-build` for the 2 code images, `docker-build-os` for the 5 os targets) is the idiomatic solution: each job carries its own `if:` gate keyed to its own filter output, and each matrix entry produces a distinct, stable check name. This is readable and reviewable in a single YAML diff.

### KTD2. Keep agent/gateway image builds on the code-change trigger

`Dockerfile.agent` and `Dockerfile.gateway` are fast multi-stage Rust-image builds (per `mika/CLAUDE.md`, the docker-build job exists "to catch structural bugs before merge"). A `crates/**`/`Cargo.*` change genuinely *can* break those image builds (new build dep, changed binary layout), so their pre-merge value is real and worth preserving. Only the 5 hours-long Gentoo os builds are the problem. Therefore the code filter (`Dockerfile.agent`, `Dockerfile.gateway`, `Cargo.toml`, `Cargo.lock`, `crates/**`, `.github/workflows/ci.yml`) stays as-is and gates the agent/gateway job; the new os filter gates the os job. Note: AC1 names "os/Dockerfile matrix jobs" specifically — the agent/gateway builds are not the matrix the ticket scopes down, so retaining their code trigger honors AC1.

### KTD3. OS filter is `os/**` plus `**/Dockerfile*`

The corrected ACs specify the os jobs trigger on `os/**` or `**/Dockerfile*`. `dorny/paths-filter` evaluates globs with **picomatch**: `os/**` matches `os/Dockerfile`, `os/Dockerfile.dockerignore`, and everything under `os/`; `**/Dockerfile*` matches root `Dockerfile.agent`/`Dockerfile.gateway` and any nested `Dockerfile*` (picomatch's globstar matches zero or more leading path segments, so a root-level `Dockerfile.agent` matches `**/Dockerfile*`). The `**/Dockerfile*` arm is a conservative safety net — when *any* Dockerfile changes, rebuilding the full os matrix is cheap insurance against a structural break, and Dockerfile edits are rare. `.github/workflows/ci.yml` is intentionally **excluded** from the os filter so routine ci.yml edits (frequent in this repo) do not re-trigger hours of Gentoo builds; os build-definition changes live in `os/Dockerfile` and are caught by `os/**`.

---

## High-level technical design

Before — one filter gates all 7 targets:

```
docker-build-filter ──► docker-relevant (bool)
   paths: Dockerfile.agent, Dockerfile.gateway, os/Dockerfile,
          os/Dockerfile.dockerignore, Cargo.toml, Cargo.lock,
          crates/**, .github/workflows/ci.yml
        │
        └─ if docker-relevant ─► docker-build matrix [7 targets]
              mika-agent, mika-gateway,
              mika-os, mika-runtime-server, mika-runtime-gateway,
              mika-runtime-cli, mika-runtime-all   ← 5 os builds fire on any crates/** change
```

After — two filters, two jobs:

```
docker-build-filter ──► images-relevant (bool)   ──► docker-build matrix [2 targets]
   images paths: Dockerfile.agent, Dockerfile.gateway,           mika-agent, mika-gateway
                 Cargo.toml, Cargo.lock, crates/**,
                 .github/workflows/ci.yml

                    └─► os-relevant (bool)        ──► docker-build-os matrix [5 targets]
   os paths: os/**, **/Dockerfile*                                mika-os, mika-runtime-server,
                                                                  mika-runtime-gateway,
                                                                  mika-runtime-cli, mika-runtime-all
```

---

## Implementation units

### U1. Split `docker-build-filter` into two filter outputs

**Goal:** Replace the single `docker-relevant` output with two outputs — `images-relevant` (code/agent/gateway paths) and `os-relevant` (`os/**`, `**/Dockerfile*`).

**Requirements:** AC1, AC2.

**Dependencies:** none.

**Files:**
- `.github/workflows/ci.yml` (modify the `docker-build-filter` job, lines ~177-197)

**Approach:**
- Keep the job name `Docker Build Filter`, `runs-on: ubuntu-22.04`, and `if: github.event_name == 'pull_request'`.
- Change `outputs:` to expose both `images-relevant: ${{ steps.filter.outputs.images-relevant }}` and `os-relevant: ${{ steps.filter.outputs.os-relevant }}`.
- In the `dorny/paths-filter` `filters:` block, define two named filters:
  - `images-relevant`: `Dockerfile.agent`, `Dockerfile.gateway`, `Cargo.toml`, `Cargo.lock`, `crates/**`, `.github/workflows/ci.yml`
  - `os-relevant`: `os/**`, `**/Dockerfile*`
- Keep the pinned `dorny/paths-filter@v3` (matches the existing pin style in this repo).

**Patterns to follow:** existing `docker-build-filter` block in `.github/workflows/ci.yml`; multi-output filter is the standard dorny/paths-filter usage.

**Test scenarios:**
- Covers AC1. YAML parses and the workflow is structurally valid (`actionlint` / GitHub workflow parse, or `python -c yaml.safe_load`).
- Covers AC1. Glob semantics hold: `crates/foo.rs` and `skills/bar.toml` match `images-relevant` paths but NOT `os-relevant`; `os/Dockerfile` and `os/config/x` match `os-relevant`; `Dockerfile.agent` matches both `images-relevant` (listed) and `os-relevant` (`**/Dockerfile*`). Verify with a picomatch check if a node runtime is available, else by documented glob reasoning.

**Verification:** the filter job exposes two boolean outputs; a `crates/**`-only diff yields `images-relevant=true, os-relevant=false`.

### U2. Split the `docker-build` matrix into code-image and os-image jobs

**Goal:** The existing 7-entry `docker-build` matrix becomes two jobs — `docker-build` (mika-agent, mika-gateway) gated on `images-relevant`, and `docker-build-os` (the 5 os targets) gated on `os-relevant`.

**Requirements:** AC1, AC2.

**Dependencies:** U1.

**Files:**
- `.github/workflows/ci.yml` (modify the `docker-build` job, lines ~199-241; add a new `docker-build-os` job)

**Approach:**
- `docker-build`: keep `needs: docker-build-filter`, change `if:` to `github.event_name == 'pull_request' && needs.docker-build-filter.outputs.images-relevant == 'true'`, and reduce its matrix `include:` to the two code targets (`Dockerfile.agent`/`mika-agent`, `Dockerfile.gateway`/`mika-gateway`). Keep the build steps verbatim.
- `docker-build-os`: new job, same `runs-on`, same `needs: docker-build-filter`, `if: github.event_name == 'pull_request' && needs.docker-build-filter.outputs.os-relevant == 'true'`, matrix `include:` = the 5 os targets (each `dockerfile: os/Dockerfile` with its `target:` and `tag:`). Reuse the identical build-step body (Buildx + `docker/build-push-action` with `cache-from`/`cache-to` scopes per tag) so cache scopes and behavior are unchanged.
- Do not rename the agent/gateway check names; the os check names change from `Docker Build (mika-os)` etc. to `Docker Build OS (mika-os)` etc. This is safe — `main` has no required-status-check branch protection (per ticket evidence), and the in-code gate reads checks dynamically, not by hardcoded name (AC3 keeps the gate untouched).

**Patterns to follow:** the existing `docker-build` matrix entries and build steps in `.github/workflows/ci.yml` — copy the step body unchanged into the new job.

**Test scenarios:**
- Covers AC1. On a non-os PR (e.g. `crates/**` only), `docker-build-os` is skipped (does not appear / no os check runs); `docker-build` runs the 2 code targets.
- Covers AC1 / AC4. On an `os/**`-touching PR, `docker-build-os` runs all 5 os targets.
- Covers AC2. On an `os/**`-touching PR, the 5 os checks appear as PR checks that must conclude green before the merge gate is satisfied (no engine change — the existing `gh pr checks` read picks them up because they are present and running).
- YAML parse / workflow lint passes; both jobs reference valid `needs` outputs.

**Verification:** two distinct docker-build jobs exist; a `crates/**`-only PR runs only `docker-build` (2 jobs); an `os/Dockerfile` PR runs `docker-build-os` (5 jobs). This PR itself (touches `ci.yml` + a plan doc, not `os/**`) is live negative evidence — its os matrix must be skipped.

---

## Acceptance criteria

- AC1: os/Dockerfile matrix jobs only run on PRs touching os/** or **/Dockerfile*. Non-os PRs do NOT trigger the matrix.
- AC2: PRs that DO touch os/** still cannot merge until per-role docker-build jobs conclude green (existing required-status mechanism, no engine changes).
- AC3: pr_merge_with_gate, ci_success_handler, verdict_handler unchanged — covered by #1577 merged today.
- AC4: regression evidence — a deliberately-broken os/Dockerfile change is blocked on a touching PR; a non-touching PR does NOT trigger the matrix at all.

---

## Verification strategy

1. **Static** — `actionlint .github/workflows/ci.yml` (or YAML parse) confirms the split workflow is valid.
2. **Glob** — verify the two filters' picomatch semantics (node/picomatch one-shot if available, else documented reasoning per KTD3): `crates/**` ∈ images-only; `os/**` and `**/Dockerfile*` ∈ os; neither matches unrelated paths like `skills/**` or `docs/**`.
3. **Live negative (AC4)** — this PR touches `.github/workflows/ci.yml` + the plan doc only (no `os/**`), so on the PR the `docker-build-os` job must be **skipped** while `docker-build` runs (ci.yml is in the images filter). Confirm via `gh pr checks` on the PR.
4. **Live positive (AC1/AC2/AC4)** — documented in the PR: an `os/Dockerfile`-touching PR triggers all 5 os jobs (glob reasoning + the matrix `if` on `os-relevant`). A throwaway broken-os/Dockerfile PR is the operator-side acceptance test; not committed to this PR to avoid polluting it with a deliberately-broken Dockerfile.

---

## Risks & dependencies

- **Check-name change for os jobs.** The 5 os checks rename (`Docker Build (mika-os)` → `Docker Build OS (mika-os)`). Risk is only material if a required-status-check rule pins the old names — verified absent (no branch protection on `main`; in-code gate reads checks dynamically). Low risk.
- **dorny/paths-filter multi-output behavior.** Standard, well-documented usage; two named filters under one `filters:` block emit independent outputs. No new dependency.
- **No engine/gate coupling.** AC3 holds by construction — no Rust files are touched.
