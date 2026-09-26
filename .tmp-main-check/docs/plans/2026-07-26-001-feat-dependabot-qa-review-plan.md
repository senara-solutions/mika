---
issue: mika#1729
title: Extend mika-qa autonomous review to Dependabot PRs with distinct-from-CI breakage-check
type: feat
date: 2026-07-26
---

# Plan — Extend mika-qa autonomous review to Dependabot PRs (mika#1729)

## Context

Prime AMEND (2026-07-06) ratified **Path 1** (topology config + reviewer capability),
**not** Path 2 (`dependabot.yml` auto-merge). The autonomous chain for Dependabot PRs is:

```
Dependabot → mika-qa approve → mika-dev merge
```

on the three already-autonomous repos: `mika`, `mika-cloud`, `mika-platform`.

**Prime's teeth (load-bearing acceptance property):** mika-qa's Dependabot review must
*do something CI does not* — check changelog/advisory for breaking changes. If qa's
dep-review collapses to "CI green → approve", the trust boundary is laundered, not
answered. That is exactly why Path 2 was rejected.

### Grounding — what the codebase actually looks like today

This plan is written against verified reads of the current tree (not the ticket's
assumptions). Four reconciliations materially shape scope:

1. **Gateway routing is already author-agnostic.** `route_event`
   (`crates/mika-gateway/src/github.rs:312-333`) routes
   `pull_request.opened | synchronize | ready_for_review` to `mika-qa`
   **unconditionally** — no author filter. Only `review_requested` carries an author
   gate (`is_suppressed_review_request`, must equal `QA_REVIEWER_LOGIN =
   "mika-platform-qa"`). All three target repos are in `INTERNAL_REPOS`
   (`github.rs:253`), so Dependabot PRs on them already reach mika-qa via the org-level
   GitHub App webhook. **AC2/AC3 are largely satisfied by existing infra.**

2. **The ticket's "per-repo `.github/workflows/` triggers" framing is incorrect.**
   mika-qa review is **not** triggered by per-repo GitHub Actions workflows. It is
   triggered by the org-level GitHub App webhook → gateway → `route_event` → mika-qa
   dispatch. There is no per-repo workflow file to edit for *triggering* the review.
   The only per-repo config that Dependabot PRs need is `.github/dependabot.yml` to
   **enable** Dependabot (AC1), not to trigger review (AC3).

3. **AC1 is *not* "already true".** There is no `.github/dependabot.yml` on `mika`
   (verified: `gh api .../contents/.github/dependabot.yml` → 404; zero historical
   Dependabot PRs on `mika` and `mika-cloud`). Enabling Dependabot is a **prerequisite**,
   not a given.

4. **The engine-side deterministic merge won't fire for Dependabot PRs.**
   `verdict_handler::find_task_for_verdict` requires an active in-progress mika task for
   the PR (`crates/mika-agent/src/server/verdict_handler.rs:302`). Dependabot PRs are not
   dispatched by mika-dev, so no task exists → the handler returns
   `Passthrough` and the merge decision falls to the **LLM-level** `self-dev-webhook-qa`
   skill. AC4 therefore hinges on that LLM path accepting a qa-approved, task-less,
   `dependabot[bot]`-authored PR for merge.

### mika-qa's review is *already* CI-independent — but that is not enough

`skills/bundled/qa-review/system_prompt.md:45` explicitly forbids mika-qa from fetching
or reasoning about CI status. So "distinct-from-CI" is trivially true in the literal
sense. **That does not satisfy Prime's teeth.** For a Dependabot PR the review has no
meaningful content today: no plan file, no acceptance criteria, and the diff is a bare
version-string bump in `Cargo.toml`/`Cargo.lock` (or `package.json`/lockfile). The
qa-review pipeline (Step 2 pipeline compliance, Step 2.5 plan-AC verification, Step 3
diff analysis, build verification) produces a hollow verdict on such a PR. The
**substantive** distinct-from-CI signal Prime demands — a changelog/advisory
breaking-change check — **does not exist yet** (verified: no `advisory` / `CVE` /
`securityVulnerabilities` references anywhere in `skills/` or `docs/skills/`).

**This is the load-bearing deliverable of the ticket (AC5), and its failure is the
explicit AC6 escalation trigger.**

## Requirements

- **R1 (AC1):** Dependabot enabled to open PRs on `mika`, `mika-cloud`, `mika-platform`
  via `.github/dependabot.yml`. On `mika` and `mika-cloud`: Cargo + GitHub Actions
  ecosystems. On `mika-platform`: GitHub Actions (+ the sub-repos own their Cargo/npm
  ecosystems). Documented as a load-bearing precondition.
- **R2 (AC2):** `dependabot[bot]` recognized as a trusted author-class by the mika-qa
  review chain on these three repos. Given gateway routing is already author-agnostic,
  this reduces to: (a) confirm/document the existing unconditional routing as the
  author-class acceptance for review, and (b) ensure no *downstream* author gate
  (skill prompt, merge path) rejects `dependabot[bot]`.
- **R3 (AC3):** mika-qa autonomous review fires on Dependabot PRs the same as on agent
  PRs. Correct the ticket's per-repo-workflow framing; the mechanism is gateway routing.
  Add a regression test asserting `route_event("pull_request", "opened", …) == "mika-qa"`
  is author-independent (already true — pin it against regression).
- **R4 (AC4):** mika-dev autonomous merge fires post-qa-approve on Dependabot PRs. Close
  the task-lookup gap: a qa-approved, task-less `dependabot[bot]` PR must reach
  `pr_merge_with_gate` through the `self-dev-webhook-qa` LLM path (or a targeted
  engine-side extension), with an explicit trusted-author check so *only* `dependabot[bot]`
  (and existing trusted authors) get this task-less merge treatment.
- **R5 (AC5) — load-bearing:** mika-qa's review of a Dependabot PR includes a
  **distinct-from-CI signal** — a changelog/advisory breaking-change check for the bumped
  version range. The signal must be *present and named* in the verdict (not implicit).
- **R6 (AC6):** If R5 cannot demonstrate a distinct-from-CI signal at implementation time,
  **halt and escalate to Prime** — the original dep-runtime-breakage concern was right and
  per-repo trust needs re-argument.

### Non-functional / constraints

- **NF1 — no Path 2.** Do not add Dependabot auto-merge config (`dependabot.yml`
  auto-merge, or a GitHub Actions auto-merge workflow). Merge must route through mika-qa's
  review, per Prime's ruling.
- **NF2 — least author surface.** The task-less merge path (R4) must be gated to
  `dependabot[bot]` explicitly. Do not weaken `find_task_for_verdict` or
  `pr_merge_with_gate` for *all* task-less PRs — that would open an unreviewed-merge hole.
- **NF3 — engine-coupled edits ship atomically.** `qa-review` and `self-dev-webhook-qa`
  are bundled engine-coupled skills; any verdict sub-type or handler-contract change must
  land in lockstep with the Rust `verdict_handler` dispatch table and `run_gh` scope gate.
- **NF4 — fail-closed on advisory-check failure.** If the advisory/changelog fetch fails
  (network, rate-limit, unparseable), the dep-review signal degrades to `hold[review]`
  ("could not verify breaking-change status"), never to `pass`. Mirrors the existing
  "tool failure → max verdict hold[review]" data-integrity rule.

## Approach / Design

### Decision A — AC5 mechanism: changelog + GitHub Advisory Database check inside qa-review

Of the three AC5 shapes offered by the ticket, this plan selects a **hybrid inside the
qa-review skill**, not a separate `mika-skills` skill:

- **Changelog / release-notes surface.** Dependabot PR bodies already embed the
  dependency's release notes, changelog excerpt, and a compatibility score. mika-qa reads
  and *reasons about* these (present in the `qa_pr_view` body) for breaking-change entries
  in the version delta — but treating Dependabot's own body as the sole signal is
  laundering (we'd be trusting the thing under review). So it is one input, not the gate.
- **Independent advisory query.** mika-qa issues an independent GitHub Advisory Database
  query for the bumped package over the version delta via `gh api` (GraphQL
  `securityVulnerabilities(ecosystem:, package:)` or REST `GET /advisories?ecosystem=…`).
  This is the *active* check CI does not perform and Dependabot's body cannot be trusted to
  self-report. **This is the substantive distinct-from-CI signal.**

Rationale for in-skill over a new `mika-skills` skill: mika-qa already carries `run_gh`
and `run_shell`; the dependency-review reasoning is verdict-shaped and belongs with the
reviewer. A standalone skill adds a dispatch/install surface for one call path. (If a
future non-qa consumer needs dep-review, extract then — YAGNI now.)

### Decision B — a named verdict section + a distinct sub-type

- Add a mandatory **`DEP-REVIEW:`** section to the qa-review verdict for
  `dependabot[bot]`-authored PRs, carrying: package, version delta, advisory-query result
  (clean / N advisories with severities), and changelog breaking-change findings. This is
  the "present and named" signal (R5).
- Introduce a new gating sub-type **`block[dependency]`** (parallel to `block[ac]` /
  `block[ci]`) for confirmed breaking-change / open-advisory cases. Wire it into the
  `verdict_handler` dispatch table and `self-dev-webhook-qa` routing:
  `block[dependency]` → notify operator, mark task-less PR held, **no auto-merge**
  (mirrors `block[security]` semantics — human-gated, no auto-dispatch).
- Clean advisory + no breaking-change changelog entry → `pass` is permitted, with the
  `DEP-REVIEW:` section stating the clean result and the advisory-query citation.

### Decision C — task-less merge path for `dependabot[bot]` (R4/AC4)

`verdict_handler` passes through when no task is found; the `self-dev-webhook-qa` LLM turn
owns the merge. Extend `self-dev-webhook-qa` with an explicit branch:
- On `VERDICT: pass` where the PR has no correlated mika task **and** author is
  `dependabot[bot]` (verified via `qa_pr_view` author field) → call `pr_merge_with_gate`
  directly. `pr_merge_with_gate` still enforces CI-required-checks, behind-main, and the
  forge-gate perimeter (#1829) — those gates are author-independent and must continue to
  bind (a Dependabot PR touching a DECISION-CORE zone is correctly blocked).
- Any other task-less author → unchanged (no autonomous merge; operator-gated). This
  satisfies NF2's least-author-surface constraint.

### Decision D — `gh api` allow-matrix + qa-review gh-scope extension

The advisory query needs `gh api` access to the advisory endpoint, which is deny-by-default:
- Add a `GhApiAllowEntry` (GET, advisory path regex, rule name) to `GH_API_ALLOW_MATRIX`
  (`crates/mika-agent/src/tools/…run_gh` gating).
- Extend `validate_qa_review_gh_scope` (mika#1196) to permit the advisory `gh api` call
  within the qa-review skill scope.
- If GraphQL is chosen over REST, ensure the `graphql` subcommand path is covered by the
  scope gate.

## Implementation phases

**Phase 1 — Enable Dependabot (AC1 / R1).**
- Add `.github/dependabot.yml` to `mika`, `mika-cloud`, `mika-platform` (inline
  cross-repo commits per ticket scope; mika-cloud/mika-platform follow the label-sync
  precedent, no separate tickets). Ecosystems per R1. Weekly schedule, grouped minor/patch
  where sensible to bound PR volume.
- Document the enablement as a load-bearing precondition in the relevant CLAUDE.md /
  cross-repo doc.

**Phase 2 — Confirm & pin routing (AC2/AC3 / R2, R3).**
- Add/confirm a `route_event` regression test asserting author-independence of
  `pull_request.opened|synchronize|ready_for_review` → mika-qa.
- Document the correction: review-trigger is gateway routing, not per-repo workflows.
  Update `crates/mika-gateway/CLAUDE.md` routing notes to name Dependabot PRs explicitly as
  a covered author-class.

**Phase 3 — Merge path for task-less Dependabot PRs (AC4 / R4).**
- Extend `self-dev-webhook-qa/system_prompt.md` with the `dependabot[bot]` + task-less +
  `VERDICT: pass` → `pr_merge_with_gate` branch (Decision C), with an explicit author check
  and the NF2 least-surface guard.
- Confirm `pr_merge_with_gate`'s existing gates (CI-required, behind-main, forge-gate)
  bind for Dependabot PRs; add a regression/eval fixture for the task-less-pass shape.

**Phase 4 — Distinct-from-CI dep-review capability (AC5 / R5) — load-bearing.**
- Add a Dependabot-PR detection + dep-review step to `qa-review/system_prompt.md`
  (author == `dependabot[bot]`; extract package + version delta from the structured
  Dependabot title/body).
- Implement the changelog reasoning + independent advisory query (Decision A), emit the
  mandatory `DEP-REVIEW:` section (Decision B), and map findings to `pass` /
  `block[dependency]` / `hold[review]` per NF4.
- Wire `block[dependency]` into `verdict_handler` dispatch + `self-dev-webhook-qa` routing
  (Decision B), and add the `gh api` allow-matrix entry + qa-review scope extension
  (Decision D).

**Phase 5 — Tests, calibration, docs.**
- Grounding-regression / eval fixture: a Dependabot PR whose changelog contains a breaking
  change must NOT yield `pass` (proves the signal is real, not laundered).
- Add a `mika-qa` calibration scenario (`dep_review_distinct_from_ci`) asserting the
  `DEP-REVIEW:` section is present and the advisory citation is grounded. Run
  `make calibrate-mika-qa` if any mika-qa model surface is touched (mika#1190); no model
  swap in this ticket, so calibration is additive-scenario only.
- Update `crates/mika-agent/CLAUDE.md` (verdict sub-types, `gh api` matrix), qa-review docs,
  and the cross-repo pattern doc.

**Phase 6 — AC6 gate.**
- If, at Phase 4, the advisory query + changelog reasoning cannot be shown to produce a
  signal distinct from "CI green" (e.g., advisory API unusable for the ecosystems, or the
  only obtainable signal is CI-equivalent), **halt Phase 4, do not ship a laundered
  approve path, and escalate to Prime** with the concrete blocker. Per AC6, this re-opens
  the per-repo trust argument rather than papering over it.

## Verification Contract

- `cargo build` / `cargo clippy` / `cargo fmt --check` clean.
- `cargo test -p mika-gateway` — routing author-independence test passes.
- `cargo test -p mika-agent` — verdict_handler `block[dependency]` dispatch test +
  task-less-pass merge-path test pass.
- `cargo test -p mika-agent --test eval` — Dependabot breaking-change fixture yields a
  non-`pass` verdict; clean fixture yields `pass` with a grounded `DEP-REVIEW:` citation.
- `make verify-bundled-skills` — qa-review / self-dev-webhook-qa manifests remain coherent
  (required_tools, allowlists).
- Manual: a real Dependabot PR on `mika` receives a mika-qa review carrying a `DEP-REVIEW:`
  section with an advisory-query citation, and merges (or blocks) via the mika-dev path
  without operator intervention on the happy path.

## Definition of Done

- `.github/dependabot.yml` present on all three repos; Dependabot opens PRs (AC1).
- mika-qa autonomously reviews Dependabot PRs on all three repos via gateway routing,
  with the per-repo-workflow framing corrected in docs (AC2, AC3).
- A qa-approved Dependabot PR merges via the mika-dev path, gated to `dependabot[bot]` and
  still bound by CI/behind-main/forge-gate checks (AC4).
- Every mika-qa Dependabot review emits a named `DEP-REVIEW:` section backed by an
  independent advisory query + changelog reasoning; a breaking-change/open-advisory case is
  blocked, not approved (AC5).
- If AC5 cannot be demonstrated, the work halts with a written escalation to Prime (AC6).
- Tests, calibration scenario, and docs updated; `make verify-bundled-skills` green.

## Acceptance criteria

- **AC1** — Dependabot bot allowed to open PRs on `mika`, `mika-cloud`, `mika-platform`
  (already true; verify + document as a load-bearing precondition).
- **AC2** — `dependabot[bot]` mapped to an author-class the mika-qa review chain accepts
  (extend mika-qa's classifier / config to recognize the bot as a trusted author on these
  three repos).
- **AC3** — mika-qa autonomous review fires on Dependabot PRs same as on agent PRs
  (per-repo workflow triggers include `pull_request` events from `dependabot[bot]`).
- **AC4** — mika-dev autonomous merge fires post-qa-approve on Dependabot PRs (extend
  mika-dev's merger classifier / merge-gate to accept qa-approved Dependabot PRs).
- **AC5** — **Breakage-check acceptance property**: qa's review of a Dependabot PR must
  include a distinct-from-CI signal. Proposed shapes (choose one at implementation time —
  the AC is that the signal is present and named, not the exact mechanism):
  - Parse `CHANGELOG.md` / `CHANGES` for the bumped version range and surface any
    breaking-change entries.
  - Query the GitHub Advisory Database for CVEs in the version delta.
  - A `mika-skills` skill (e.g., `dep-review`) that wraps the parse + query into a single
    capability the qa agent can invoke.
- **AC6** — If qa's dep-review cannot demonstrate a distinct-from-CI signal at
  implementation time, **escalate back to Prime** — because then the original
  dep-runtime-breakage concern was right and per-repo trust needs re-argument.

### Note on AC framing (grooming reconciliation)

- **AC1** is written as "already true" but is **not**: no `.github/dependabot.yml` exists
  on `mika` and there are zero historical Dependabot PRs. Treat AC1 as a real
  enable-and-verify task (Phase 1), not a documentation-only confirmation.
- **AC3**'s "per-repo workflow triggers" clause reflects an incorrect mental model. The
  review trigger is the org-level GitHub App webhook → gateway `route_event`, which is
  already author-agnostic on `opened|synchronize|ready_for_review`. AC3 is satisfied by
  existing routing (Phase 2 pins it); there are no per-repo workflow trigger files to edit.
- **AC5** is the load-bearing property; Phase 4 chooses the changelog + independent
  advisory-query hybrid. AC6 is a live halt condition, not a formality.
