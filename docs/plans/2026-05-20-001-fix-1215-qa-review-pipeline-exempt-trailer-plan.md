---
title: "Teach qa-review to honor Pipeline-Exempt commit trailer (asymmetry with verify-pipeline.sh)"
date: 2026-05-20
type: fix
issue: senara-solutions/mika#1215
module: qa-review
problem_type: inconsistency
component: skill-prompt
status: groomed
---

## Context

`scripts/verify-pipeline.sh` (CI: "Pipeline Artifacts" check) and `skills/bundled/qa-review/system_prompt.md` (mika-qa skill) both gate the same intent — "PR with source changes must ship a plan/solution doc" — but with **divergent permissiveness**.

| Exemption mechanism | `verify-pipeline.sh` (CI) | `qa-review` skill |
|---|---|---|
| `documentation` label on linked issue (docs-only) | ✅ honored | ❌ missing |
| `pipeline-exempt` PR label (docs-only) | ✅ honored | ✅ honored (mika#1065, 2026-05-10) |
| `Pipeline-Exempt: docs-only — <reason>` trailer | ✅ honored | ❌ missing |
| `Pipeline-Exempt: code-only — <reason>` trailer | ✅ honored | ❌ missing — **this PR's reproduction case** |

The asymmetry surfaced on mika PR #1185 (TUI lifecycle hardening cohort, 2026-05-17 → 2026-05-19). It is a code-only PR with `Pipeline-Exempt: code-only — TUI lifecycle hardening cohort salvaged from #1181…` trailer on commit `5e2b8feb`. CI passed. mika-qa posted `block[pipeline]` ("No plan document in docs/plans/ and no plan callout in issue #1150 body — grooming required before this PR can be reviewed."). Operator unblocked by retroactively grooming mika#1150 and writing a throwaway plan doc.

The trailer mechanism (mika#860) exists *because* some code-only changes legitimately don't need a plan/solution doc. When qa-review enforces a stricter gate than CI, the trailer's intent is negated and the operator pays a tax in throwaway plan docs.

## Scope

**In scope:**
1. Teach `skills/bundled/qa-review/system_prompt.md` Step 2 to honor the `Pipeline-Exempt:` commit trailer, mirroring `verify-pipeline.sh`'s permissiveness for both `docs-only` and `code-only` variants.
2. Use the dual-form pattern (with-reason / bare-warns) to match `verify-pipeline.sh` exactly (lines 158–172).
3. Update Step 2.5 (plan-AC verification) entry conditions so the trailer skips it.
4. Update verdict templates with a new bypass example (matching the existing `pipeline-exempt` label example).
5. Update Step 5 pre-termination self-check invariant 3 to accept the trailer-bypass state.
6. Compound doc at `docs/solutions/prompt-engineering/qa-review-pipeline-exempt-trailer-parity-2026-05-20.md` recording the asymmetry resolution and the deferred items (see Deferred below).

**Out of scope (deliberate):**
- `documentation` label-on-linked-issue inheritance in qa-review. This is a separate exemption path (linked-issue semantic, not PR-level commit). It's a real asymmetry but wasn't the empirical break on PR #1185. The ticket body explicitly mentions only the trailer and `Status: retroactive`. If Vincent wants the full asymmetry closed, file a separate ticket — keeping this PR scoped to the trailer aligns with the ticket-body wording and `feedback_implementation_scope_bundling.md`.
- `Status: retroactive` frontmatter marker. The ticket body says: *"If a third instance appears, the right move is to teach qa-review's pipeline check to accept the `Status: retroactive` frontmatter marker rather than to build `mika-plan-audit`."* The retroactive plan-doc pattern has only been used **twice** so far. P3-deferred to third occurrence per the ticket. Track in the ticket body, do not implement.
- Mocking PR test coverage for the new prompt path. qa-review is a prompt-only skill; the structural correctness of a prompt edit can't be unit-tested. Verification is empirical (manual trigger on a synthesized PR; see AC1 below).
- Changes to `verify-pipeline.sh`. It is already the canonical reference implementation; this PR brings qa-review into parity with it, not the other way.

## Implementation

### Where the trailer check lives

The qa-review skill already uses `run_shell("git -C $MIKA_PLATFORM_DIR/<repo>/ ...")` in Step 2.5.1 (reading a plan file via `git show`). Re-use the same primitive to read commit message bodies from `base..head`:

```
run_shell("git -C $MIKA_PLATFORM_DIR/mika/ fetch origin <headRefName> 2>/dev/null && \
          git -C $MIKA_PLATFORM_DIR/mika/ log origin/<baseRefName>..origin/<headRefName> --format=%B | \
          grep -E '^Pipeline-Exempt: (docs-only|code-only)([[:space:]]+.+)?$' | head -1")
```

`baseRefName` and `headRefName` are already in the `qa_pr_view` response (used by Step 3e.2). No new tool or scope change is needed; this stays within the existing `run_shell` capability.

**Why not extend `qa_pr_view` to return commits.** `qa_pr_view`'s charter is "CI-stripped PR metadata" — `gh pr view --json commits` returns per-commit `statusCheckRollup` fields, which is precisely what `qa_pr_view` was designed to exclude (per `qa_pr_view.sh:6` "CI fields … excluded so the reviewing LLM never sees CI data"). Adding a `commits` field would either (a) leak CI status, contradicting the tool's design intent, or (b) require a hand-stripped subset projection inside `qa_pr_view.sh`, expanding its scope beyond metadata. `run_shell + git log` mirrors verify-pipeline.sh's own implementation (lines 151, 158–172) and avoids the tool-scope expansion.

**Why not `gh pr view --json commits` via `run_gh`.** `pr view` is not in qa-review's `run_gh` scope (`pr review`, `pr diff`, `pr list`, `issue view` only, per `validate_qa_review_gh_scope` mika#1196). Adding `pr view` opens up the CI-data surface that the scope explicitly forbids. `run_shell + git` is the right tool.

### Step 2 changes (skill prompt)

Insert a new bypass check at the top of Step 2, **after** the existing `pipeline-exempt` label bypass, **before** checks 1–3. Order matters: label → trailer → strict checks. This mirrors `verify-pipeline.sh`'s priority order (label-on-issue → PR-label → trailer).

Pseudocode for the new prompt section:

```
**Pipeline-Exempt trailer bypass** — After the `pipeline-exempt` label check above,
also scan commit messages in `base..head` for the `Pipeline-Exempt:` trailer.
This mirrors `scripts/verify-pipeline.sh`'s permissiveness (see mika#860, mika#1215).

Run:
  run_shell("git -C $MIKA_PLATFORM_DIR/mika/ fetch origin <headRefName> 2>/dev/null && \
            git -C $MIKA_PLATFORM_DIR/mika/ log origin/<baseRefName>..origin/<headRefName> --format=%B | \
            grep -E '^Pipeline-Exempt: (docs-only|code-only)([[:space:]]+.+)?\$' | head -1")

If the result matches `Pipeline-Exempt: docs-only`:
  1. Confirm the PR is docs-only via the same source-change check as the label bypass:
     run_gh("pr diff <PR_URL> --name-only | grep -v '^docs/' | grep -v '^\\.github/' | grep -v '^\\.claude/' | head -1")
  2. If result is empty (no source files): skip checks 1–3 and Step 2.5. Note:
     "Pipeline-exempt: docs-only trailer honored, skipping pipeline checks and plan-AC verification." Jump to Step 3.
  3. If result is non-empty (source files present): note "Pipeline-Exempt: docs-only trailer
     present but PR contains source changes — ignoring trailer." Continue with checks 1–3 normally.

If the result matches `Pipeline-Exempt: code-only`:
  1. Confirm the PR is code-only (no docs/plans/ doc): re-use check 1 of the existing logic,
     inverted — i.e., absence of `^docs/plans/.*\\.md$` in the diff.
  2. If the diff contains no `docs/plans/*.md` and source changes are present:
     skip checks 1–3 and Step 2.5. Note "Pipeline-exempt: code-only trailer honored,
     skipping pipeline checks and plan-AC verification." Jump to Step 3.
  3. If the diff contains a `docs/plans/*.md` doc OR no source changes: note "Pipeline-Exempt:
     code-only trailer present but PR shape does not match (has plan doc or no source) —
     ignoring trailer." Continue with checks 1–3 normally.

Bare-form warning: if the trailer is bare (e.g. `Pipeline-Exempt: code-only` with no
` — <reason>` suffix), append to the note: "(bare trailer; prefer 'Pipeline-Exempt: code-only — <reason>'
for audit trail)". Bypass still honored — bare form remains backward-compatible per
`verify-pipeline.sh` lines 161 and 169.
```

### Step 2.5 entry condition

Currently Step 2.5 always runs unless `pipeline-exempt` label honored. Update the gate to also skip on trailer bypass. The "Pre-termination self-check" invariant 3 already accepts:

> *"or, if the `pipeline-exempt` bypass was honored in Step 2, you have emitted `PLAN-AC VERIFICATION: skipped (pipeline-exempt)`"*

Extend it to:

> *"or, if a Step 2 trailer or label bypass was honored, you have emitted `PLAN-AC VERIFICATION: skipped (pipeline-exempt — <docs-only|code-only> trailer)` or `PLAN-AC VERIFICATION: skipped (pipeline-exempt label)`"*

This unifies the verdict-body shape across all three bypass paths (label, docs-only trailer, code-only trailer).

### Verdict template

Add one new example to the verdict-template section (after the existing `pipeline-exempt` label example, line 461–477 of `system_prompt.md`):

```
When `Pipeline-Exempt: code-only — <reason>` trailer is honored (code-only PR):

VERDICT: pass
REASON: Code-only PR; Pipeline-Exempt trailer honored — diff review clean.

DIFF ANALYSIS:
Files reviewed: <n>
Key changes: <bullets>

PLAN-AC VERIFICATION: skipped (pipeline-exempt — code-only trailer)

BUILD VERIFICATION: skipped (pipeline-exempt — code-only trailer)

VERDICT: pass
REASON: Code-only PR; Pipeline-Exempt trailer honored — diff review clean.
```

`docs-only` trailer reuses the existing `pipeline-exempt` label template's structure with `— docs-only trailer` substituted for `— label`.

### Defensive ordering

`pipeline-exempt` PR label is checked before the trailer (label > trailer priority). This matches `verify-pipeline.sh` lines 188–198: label paths checked first, trailer last. The first match wins; this prevents double-honoring on PRs that have both and keeps the logged reason unambiguous ("which mechanism allowed this PR through").

### Compound doc

Write `docs/solutions/prompt-engineering/qa-review-pipeline-exempt-trailer-parity-2026-05-20.md` with the standard frontmatter (`module: qa-review`, `tags: [qa-review, pipeline-exempt, trailer, asymmetry]`, related_issues including #1215, #1185, #1150, #860, #1064, #1065). Body: the asymmetry table from this plan's Context section, the reproduction (PR #1185), the resolution shape, and the deferred items (`Status: retroactive` and `documentation`-label-inheritance).

## Acceptance criteria

- [ ] **Structural — qa-review prompt Step 2 trailer bypass present.** `grep -F 'Pipeline-Exempt trailer bypass' skills/bundled/qa-review/system_prompt.md` returns at least one match within the Step 2 section (before "Step 2.5").
- [ ] **Structural — qa-review prompt Step 2 runs `git log` to detect the trailer.** `grep -E 'git[[:space:]]+log.+Pipeline-Exempt' skills/bundled/qa-review/system_prompt.md` returns at least one match.
- [ ] **Structural — Pre-termination self-check invariant 3 accepts trailer-bypass states.** `grep -F 'pipeline-exempt — docs-only trailer' skills/bundled/qa-review/system_prompt.md` AND `grep -F 'pipeline-exempt — code-only trailer' skills/bundled/qa-review/system_prompt.md` each return at least one match (or the equivalent shape inside the invariant-3 paragraph).
- [ ] **Structural — verdict template includes code-only trailer example.** `grep -F 'Code-only PR; Pipeline-Exempt trailer honored' skills/bundled/qa-review/system_prompt.md` returns at least one match.
- [ ] **Structural — compound doc shipped.** `test -f docs/solutions/prompt-engineering/qa-review-pipeline-exempt-trailer-parity-2026-05-20.md` returns 0; frontmatter includes `module: qa-review` and `tags:` array with `pipeline-exempt`, `trailer`, `asymmetry`.
- [ ] **Behavioral (manual, post-merge) — empirical verification on a synthesized PR.** Open a code-only test PR with a `Pipeline-Exempt: code-only — verification of mika#1215` trailer; trigger mika-qa via direct invocation or `synchronize` event; confirm verdict is `pass` (or `hold[review]` for unrelated security findings) and NOT `block[pipeline]`. Record the test PR number in the issue close comment. This AC is deferred to post-merge because the new prompt instructions ship with the release; an in-PR mock isn't possible (skill prompts are LLM-interpreted text, not unit-testable code).
- [ ] **CI-deferred — no test regressions.** `cargo test`, `cargo clippy`, `cargo fmt --check` pass. The skill prompt is a `.md` file outside the test surface; the only risk is if a test fixture greps the prompt text and breaks on the new content. Inspect test failures (if any) for prompt-text-coupling.

## Test plan

1. **Local — `cargo test -p mika-agent`** to confirm no skill-discovery or prompt-loading regression (the prompt is `include_str!`'d at build time).
2. **Local — `cargo clippy --all-targets -- -D warnings`** for the standard lint floor.
3. **Manual — re-run `bash scripts/verify-pipeline-test.sh`** to confirm the CI-side trailer behavior is unchanged (this PR does not modify `verify-pipeline.sh`; this is a parity-confirmation test, not a new test).
4. **Post-merge — synthesized PR.** Create a `chore/test-pipeline-exempt-trailer` branch with a one-line code-only change (e.g., a comment in a trivial Rust file), commit with `Pipeline-Exempt: code-only — verification of mika#1215`. Open PR. Trigger mika-qa. Confirm verdict.

## Risks

1. **LLM prompt-adherence drift.** mika-qa runs on a frontier model; new instructions can be misinterpreted on edge-case PR shapes (e.g., trailer with whitespace variations, trailer in PR description not commit, etc.). Mitigation: mirror `verify-pipeline.sh`'s exact regex and dual-form pattern verbatim in the prompt; cite mika#860 in the prompt section so the model has prior-art anchoring.
2. **`run_shell` execution variance.** The `git log origin/<base>..origin/<head>` call requires the worktree to have fetched `origin/<head>`. If qa-review runs on a PR before the local repo has fetched the branch, the `fetch` subcommand in the same shell call handles it (mirrors how Step 3e.2 already fetches the head). Failure case: network/permissions; verdict downgrades to `hold[review]` per Data Integrity Rules.
3. **Trailer-on-PR-description-only (not commit).** The trailer mechanism is commit-message-based per mika#860's design. If an operator puts `Pipeline-Exempt:` in the PR description body instead, neither this fix nor `verify-pipeline.sh` honors it. This is by-design (commit messages are immutable; descriptions are not). Document in the compound doc.
4. **False positives if a commit message quotes the trailer.** Defensive: the regex anchors `^Pipeline-Exempt:` to start-of-line. A quoted trailer like `> Pipeline-Exempt: docs-only` won't match. Same defense as `verify-pipeline.sh` lines 158/161/166/169.
5. **Scope-creep pressure during architect review.** The architect may ask "why not close the full asymmetry?" — the `documentation` label-on-linked-issue path is a known gap. Position: this PR addresses the empirical break (PR #1185 trailer); the label-on-linked-issue path is a separate ticket if/when it bites. Surface as a known divergence rather than expand scope mid-grooming (`feedback_implementation_scope_bundling.md`).

## Deferred / follow-ups

- **`Status: retroactive` frontmatter marker** — track on mika#1215 issue body as P3 trigger ("on third retroactive-plan-doc instance"). Two instances so far (mika#825 thread; today's #1185).
- **`documentation` label-on-linked-issue inheritance in qa-review** — separate ticket if it bites. The current asymmetry is observable but no PR has been empirically broken by *this specific* path yet.

## Related

- Ticket: senara-solutions/mika#1215
- Reproduction: senara-solutions/mika#1185 (PR), senara-solutions/mika#1150 (issue retro-groomed)
- Trailer origin: senara-solutions/mika#860 (introduced `Pipeline-Exempt:` trailer in `verify-pipeline.sh`)
- Prior qa-review label gate: senara-solutions/mika#1064 (PR #1065, 2026-05-10) — `pipeline-exempt` label honored for docs-only PRs
- CI label parity: senara-solutions/mika#1067 (PR shipped 2026-05-11) — `verify-pipeline.sh` extended to honor `pipeline-exempt` label
- Compound: `docs/solutions/prompt-engineering/qa-review-docs-only-pipeline-exempt-gate-2026-05-10.md` (prior pattern, fabrication failure mode)
- Compound: `docs/solutions/ci-cd/ci-verify-pipeline-label-exemption-2026-05-11.md` (CI side of the same asymmetry-resolution shape)
- File: `skills/bundled/qa-review/system_prompt.md` (the change surface)
- File: `scripts/verify-pipeline.sh` (the reference implementation; do not modify)
- File: `scripts/verify-pipeline-test.sh` (the CI test fixture; do not modify)
