# Plan — mika#742 — Weekly calibration CI job + drift-detection PR automation

**Issue:** senara-solutions/mika#742
**Branch:** `feat/742/calibration-ci-job`
**Milestone:** Evaluation (#16)
**Blocked by:** `#338` at plan commit **`fa54d950`** (calibration artifact format, `eval-diff` CLI, four-provider matrix configuration, `#[ignore]`+env gating)
**Status:** Groomed draft — pending Vincent review

## Context

`#338` D7 deliberately deferred the "committed baseline" story — shipping a maintenance loop makes a committed `tests/fixtures/eval-baseline.json` *trustworthy*; shipping a baseline without the maintenance loop is theater (friend-review verdict at `#338` D7). `#742` is that loop.

This ticket ships the discipline that makes SHA-pinned baselines defensible long-term: a weekly scheduled run that regenerates calibration, diffs against the committed baseline, and opens a PR when the diff is non-empty. Without this, the baseline rots the moment a provider changes tolerance or a model gets re-fine-tuned and no one notices.

#742 is the last ticket in milestone #16 by dep chain (blocked on `#338`'s machinery) but not on the critical path — `#339` / `#740` / `#741` can ship before or alongside `#742`. It was filed p3 because scenarios don't regression-gate on the baseline yet; the maintenance loop's value grows as downstream scenarios start citing baseline as a comparison point.

## Scope boundary

**In scope:**
- `.github/workflows/eval-calibration.yml` — scheduled workflow (weekly cron + `workflow_dispatch`)
- First-run bootstrap via `eval-diff bootstrap` subcommand
- Drift-PR automation via `gh pr create` in-workflow
- `tests/fixtures/eval-baseline.json` as the committed reference (created by bootstrap run, updated by drift PRs)
- Runbook in `crates/mika-agent/CLAUDE.md` eval section

**Out of scope:**
- Dashboard integration — explicit non-goal per `#742` body
- Langfuse export — same
- Alerting beyond PR creation (Slack, email, etc.)
- Historical baseline archiving — single "latest" committed file; git history is the archive
- Scenario authoring or modification — consumed from `#339` / `#740` / `#741`
- Threshold tuning for drift-severity — all drift opens a PR; severity is a human-review concern

## Decisions

### D1 — Schedule: weekly Monday 08:00 UTC, plus manual `workflow_dispatch`

**Problem:** How often should calibration run, and when?

**Decision:** `cron: '0 8 * * 1'` — Mondays at 08:00 UTC (09:00 Paris, Vincent's TZ). Plus `workflow_dispatch` for on-demand runs (e.g., before a model upgrade or after a prompt change).

**Rationale:** Weekly matches the maintenance cadence named in the issue body. Monday morning puts the drift-PR in Vincent's review queue at the start of the week when calibration-shift has highest signal value (after a weekend of provider-side changes). 08:00 UTC is off-peak for Anthropic/OpenAI infrastructure. `workflow_dispatch` is the structural analog of "re-run calibration before shipping" — explicit trigger for intentional events.

**Rejected alternatives:**
- Daily cron. Higher cost (~7× per week), little signal gain — providers don't change tolerance daily.
- Trigger on every `main` push. Scenarios change rarely; most pushes wouldn't affect calibration. Signal-to-noise too low; also makes PR creation noisy.
- No cron; manual-only. Defeats the purpose — the point of the maintenance loop is catching drift the operator isn't watching for.

### D2 — Provider keys via repo secrets; match `#338` four-provider matrix

**Problem:** Which provider keys does the workflow need, and how are they managed?

**Decision:** Four repo secrets matching `#338` D2's provider matrix:
- `ANTHROPIC_API_KEY`
- `OPENAI_API_KEY` (also used for embedding calls — `text-embedding-3-small`)
- `OPENROUTER_API_KEY` (for Kimi routing — `moonshotai/kimi-k2.5`)
- `GROQ_API_KEY`

Workflow injects each as the provider-specific `MIKA_*_API_KEY` env var per `mika-common/src/llm/mod.rs` provider config. Keys stored in the `senara-solutions/mika` repo Secrets (Settings → Secrets and variables → Actions). Rotation is manual: update the secret in the UI, next scheduled run picks it up.

**Key-missing behavior:** If any secret is unset, the workflow skips that provider's scenarios with a log warning rather than failing. Matches `#338` D1 parser behavior (hard-fail on unknown provider name; soft-skip on missing credentials for a known provider).

**Rationale:** Secrets are the standard GitHub pattern for CI credentials; no point inventing another mechanism. Soft-skip on missing key lets the workflow remain useful even if one provider's key lapses — drift detection on the other three still runs.

**Rejected alternatives:**
- OIDC token exchange with a secrets vault. Overkill for a p3 ticket; repo secrets suffice.
- Single `MIKA_EVAL_KEYS_JSON` blob. Couples four rotations into one; no operational win.

### D3 — Baseline file: `tests/fixtures/eval-baseline.json`, committed, sole source of truth

**Problem:** Where does the committed baseline live and what format does it use?

**Decision:** Single committed file at `tests/fixtures/eval-baseline.json`. Format matches `#338` D7's calibration artifact schema (`timestamp`, `providers`, per-scenario outcomes including judge model version per `#339` D4). One file; git history is the archive. Drift PRs overwrite the file with the new artifact content.

**Rationale:** One file means drift detection is a single `git diff` on a known path. Format reuse means `eval-diff` (from `#338` D7) compares two files with the same parser on both sides. Git history is a free archive — any prior baseline is `git show <SHA>:tests/fixtures/eval-baseline.json`.

**Rejected alternatives:**
- Multiple dated files under `baselines/`. Same rot problem as #338 D7 rejected — files pile up without a clear single-source-of-truth.
- Baseline in DB. Test-time state in a DB adds a DB dependency to a CI workflow for no value.
- Baseline in workflow artifacts only (not committed). Defeats the purpose — no grep-able reference for downstream scenarios.

### D4 — Drift detection: `eval-diff` semantic diff, zero-tolerance on outcome change

**Problem:** What counts as drift? Any JSON byte change, or semantically-meaningful change only?

**Decision:** `eval-diff` CLI (shipped by `#338` D7) does semantic comparison:
- **Outcome-level changes** (e.g., `matched_exact` → `matched_llm`, `resolved` → `skipped_no_llm`) → **drift, always opens PR**
- **Confidence changes <0.05** → not drift (floating-point / LLM sampling noise)
- **Confidence changes ≥0.05** → drift, opens PR
- **Judge model version change** → drift, opens PR flagged as `calibration-reset` (per `#339` D4 deprecation protocol)
- **Timestamp-only changes** → not drift

Zero-tolerance on outcome changes means any semantic flip triggers human review. This is the point of drift detection.

**Rationale:** Semantic diff is the value-add over `git diff` on the raw JSON (which would fire on every timestamp). Zero-tolerance on outcome shifts is the signal the whole pipeline exists to surface.

**Rejected alternatives:**
- Raw `git diff` on the JSON. Too noisy; timestamps alone would fire weekly.
- Thresholded drift (open PR only if ≥N scenarios changed). Masks individual-scenario regressions; "drift" at a scenario is already worth a human look.
- Whitelist of "expected" drift. Builds an allow-list maintenance problem; rejected in favor of per-PR review.

### D5 — Drift PR: grep-able body, assigned to repo owner, labeled

**Problem:** How does the drift PR communicate what changed so a reviewer can approve without re-running calibration locally?

**Decision:** PR title format: `eval-calibration: drift detected on YYYY-MM-DD (N outcome changes)`.

PR body format (fenced for grep-ability, same pattern as `#339` D5 baseline format):

````markdown
## Eval Calibration Drift

Weekly calibration run detected drift against committed baseline.

**Summary:** N outcome changes across M providers.

```json
{
  "previous_baseline_captured_at": "...",
  "new_calibration_captured_at": "...",
  "changes": [
    {
      "provider": "anthropic",
      "model": "claude-sonnet-4-6",
      "scenario": "stage_2_llm_disambiguation_case_variants",
      "previous_outcome": "matched_llm",
      "new_outcome": "skipped_no_llm",
      "confidence_delta": null,
      "classification": "outcome_change"
    }
  ]
}
```

## Review checklist
- [ ] Outcome changes are legitimate (provider/model actually behaves differently), not a workflow bug
- [ ] No calibration-reset flag triggered unexpectedly
- [ ] Scenarios affected are reasonable targets for the provider/model change
- [ ] Merge updates baseline; decline closes this PR without merging
````

Assignee: repo owner (or a configurable maintainer list). Labels: `eval-calibration`, `drift-detected`. If `classification: calibration-reset` fires, additionally add label `calibration-reset` so it sorts visibly.

**Rationale:** Fenced JSON block is parse-able by future tooling without committing to build it. Review checklist makes the approval criterion explicit — reviewer isn't guessing what "approve this drift" means. Labels make drift PRs filterable in repo search.

**Rejected alternatives:**
- Free-prose PR body. Locks out automation; rejected per `#339` D5 precedent.
- Auto-merge on drift. Rejected — drift is the point of human review; auto-merging defeats the signal.

### D6 — First-run bootstrap: `eval-diff bootstrap` subcommand, explicit PR

**Problem:** On first workflow run, `tests/fixtures/eval-baseline.json` doesn't exist. What happens?

**Decision:** `eval-diff bootstrap` subcommand in the `eval-diff` CLI (small extension to `#338` D7's CLI). Behavior:
1. Runs full calibration (same as `MIKA_EVAL_CALIBRATE=1` path)
2. Writes output to `tests/fixtures/eval-baseline.json`
3. Opens a PR titled `eval-calibration: initial baseline bootstrap` with label `calibration-bootstrap`

Workflow invokes `eval-diff bootstrap` when it detects the baseline file is missing; otherwise invokes `eval-diff` against the existing file. First-run PR is reviewed and merged as the "initial baseline" — from then on, subsequent runs diff against it.

**Rationale:** Explicit bootstrap path means the first committed baseline is a reviewed artifact, not an auto-commit. Single CLI surface (`eval-diff` with subcommands) keeps the tooling discoverable.

**Rejected alternatives:**
- Auto-commit the first baseline without a PR. Rejected — every baseline should be a reviewed artifact, including the first.
- Manual bootstrap-only (no workflow path). Creates a chicken-and-egg problem where a human has to know to run it; workflow detecting missing file is more robust.

### D7 — Failure modes: drift PR on drift only; test failures escalate separately

**Problem:** The workflow can fail for reasons other than drift — provider outage, test infrastructure bug, invalid fixture state. Don't want PR noise on transient failures.

**Decision:** Three failure classes, three handlers:

1. **Actual drift detected** (`eval-diff` exit code 1) → open drift PR per D5.
2. **Calibration test failure** (cargo-test exit code nonzero before `eval-diff` runs) → workflow fails loudly; **no PR opened**. Repo owner sees the failed workflow in the Actions tab.
3. **Transient provider failures** (rate limit, timeout, 5xx) → `eval-diff` classifies as transient via retry-with-backoff (3 attempts, exponential). If all retries fail, workflow fails with class 2 behavior (no PR), log tags `transient_provider_failure` for operator clarity.

No drift PR for transient issues — transient-failure PRs are spam. Real-drift PRs are the signal.

**Rationale:** PR noise destroys the signal. A drift PR every week is useful; a drift PR every week PLUS three failure-PRs a month is noise that gets ignored. Failure-handling in the workflow layer keeps the PR surface clean.

**Rejected alternatives:**
- Open PR on every failure class. Noise problem above.
- Silent fail on any error. Defeats observability; Vincent wouldn't know calibration stopped working.

### D8 — Cost control: workflow timeout + scenario-count cap, no in-workflow enforcement

**Problem:** Full matrix run against four providers × milestone scenario surface could theoretically run unbounded if something goes wrong (infinite retry, provider stuck in timeout).

**Decision:** `timeout-minutes: 30` on the workflow job (GitHub Actions level). `eval-diff` + harness already respect `#338` D1's env gate and `#[ignore]` intent gate; no in-workflow token counting.

Upper bound check: `#339` + `#740` + `#741` full-matrix rollup from `#741`'s milestone cost envelope is ~$2.57/run. 30-min timeout at worst-case sonnet-4-6 throughput caps wall-clock at well under that cost even if a run hangs. No need for in-test budget enforcement.

**Rationale:** Workflow timeout is the structural backstop per `#338` D8; repeating the same structure here keeps the enforcement layer consistent across milestone #16. Consistent with "structural > in-process enforcement" throughout the milestone.

**Rejected alternatives:**
- In-workflow per-provider token counter. Duplicates structure already covered by workflow timeout.
- No timeout. Irresponsible; a stuck provider could burn hours of runner time.

## Acceptance Criteria

- [ ] `.github/workflows/eval-calibration.yml` exists with `cron: '0 8 * * 1'` + `workflow_dispatch`.
- [ ] Workflow uses four repo secrets (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GROQ_API_KEY`) per D2. Missing-key behavior: soft-skip with log warning.
- [ ] `eval-diff bootstrap` subcommand added (small extension to `#338` D7 CLI; may require amendment to `#338` if surface locks change). First-run opens PR with label `calibration-bootstrap`.
- [ ] Drift-detection classifier per D4: outcome-change = drift; confidence <0.05 = noise; confidence ≥0.05 = drift; judge version change = drift+calibration-reset.
- [ ] Drift PR format per D5: grep-able fenced JSON block, review checklist, labels `eval-calibration` + `drift-detected` (+ `calibration-reset` when applicable).
- [ ] Failure handling per D7: drift-only PRs; test failures and transient errors fail the workflow without PR.
- [ ] `timeout-minutes: 30` on workflow job per D8.
- [ ] Runbook section added to `crates/mika-agent/CLAUDE.md` eval section: workflow purpose, expected cadence, how to manually trigger, how to interpret a drift PR, how to roll back a merged baseline update.
- [ ] `tests/fixtures/eval-baseline.json` exists on main after the first-run bootstrap PR merges.
- [ ] Bootstrap + drift + transient-failure paths tested via workflow dry-run (push to branch, trigger via `workflow_dispatch`) before merging.

## Dependencies

- Blocked by `#338` at `fa54d950` — `eval-diff` CLI and calibration artifact format.
- Secret-provisioning prerequisite: repo owner adds the four secrets to `senara-solutions/mika` Settings → Secrets. Not code, but required before first workflow run succeeds.

## Downstream

- None within milestone #16 (terminal ticket for the calibration maintenance loop).
- Future: a "regression-gating" ticket may cite #742 — once `tests/fixtures/eval-baseline.json` exists and is trustworthy, scenarios can assert against it for gating, not just drift detection. Out of scope here.

## Cross-cutting notes

- **#742 is the trust infrastructure for SHA-pinned committed baselines.** `#338` D7 explicitly deferred here; this plan is the maintenance loop that makes the committed baseline defensible.
- **No scenarios authored here.** Consumes `#339` / `#740` / `#741` scenario surfaces; produces only the workflow + CLI subcommand + runbook.
- **SHA-pinned to `#338` at `fa54d950`** per CONVENTIONS.md. If `#338` amends its D7 CLI shape during implementation, the amendment bumps `#338`'s SHA and this plan's pin needs refresh + fit review.
- **Amendment protocol:** per `docs/plans/CONVENTIONS.md` ("Amendment protocol for SHA-pinned plans"). If `eval-diff` CLI surface changes during `#338` implementation, this plan's D3/D4/D5/D6 may need updating.

## Cost envelope (design-time)

Per-run cost at annual steady-state (52 weekly runs + occasional `workflow_dispatch`):

- Full-matrix calibration: ~$2.57/run per milestone rollup from `#741` (`#339` class-average + `#740` + `#741`)
- Annual lower bound (52 scheduled runs): **~$134/yr**
- Manual triggers add marginal cost (~$2.57 each); typical usage is before-major-change, so ~6-12/yr → ~$15-30/yr

**Total annualized: ~$150-165/yr.** Fully bounded by `#338` D8's workflow timeout; no runaway scenario. This cost is "insurance" against silent baseline rot — a single caught regression in a year (avoided production incident, avoided operator debugging hours) is worth the annual spend many times over.

Numbers rot as provider pricing changes; workflow-timeout + scenario-count caps are the structural enforcement per `#338` D8. This rollup is design intent, not runtime guarantee.

## Review log

Initial groom after Vincent flagged inconsistency — `#742` had been filed during `#338`'s first friend review but never received the same Socratic grooming as the other five plans in the milestone. This plan closes that gap.
