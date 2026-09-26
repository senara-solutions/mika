---
module: qa-review
tags: [qa-review, pipeline-exempt, trailer, asymmetry, prompt-engineering, ci-parity]
problem_type: inconsistency
category: prompt-engineering
related_issues: [1215, 1185, 1150, 860, 1064, 1065, 1067]
date: 2026-05-20
---

# qa-review ↔ verify-pipeline.sh parity — Pipeline-Exempt trailer

## Problem

`scripts/verify-pipeline.sh` (CI: "Pipeline Artifacts" check) and `skills/bundled/qa-review/system_prompt.md` (mika-qa skill) both gate the same intent — "PR with source changes must ship a plan/solution doc" — but with divergent permissiveness.

| Exemption mechanism | `verify-pipeline.sh` (CI) | `qa-review` skill (pre-fix) | `qa-review` skill (post-#1215) |
|---|---|---|---|
| `documentation` label on linked issue (docs-only) | ✅ honored | ❌ missing | ❌ deferred (out of scope #1215) |
| `pipeline-exempt` PR label (docs-only) | ✅ honored | ✅ honored (mika#1065, 2026-05-10) | ✅ honored |
| `Pipeline-Exempt: docs-only — <reason>` trailer | ✅ honored | ❌ missing | ✅ honored |
| `Pipeline-Exempt: code-only — <reason>` trailer | ✅ honored | ❌ missing — break case | ✅ honored |

The trailer mechanism (mika#860) exists *because* some code-only changes legitimately don't need a plan/solution doc (e.g., cohort salvage from a previously-closed work item, infra hardening cohorts with shared rationale upstream). When qa-review enforces a stricter gate than CI, the trailer's intent is negated and the operator pays a tax in throwaway plan docs.

## Reproduction

mika PR #1185 (TUI lifecycle hardening cohort, 2026-05-17 → 2026-05-19): code-only PR with `Pipeline-Exempt: code-only — TUI lifecycle hardening cohort salvaged from #1181…` trailer on commit `5e2b8feb`. CI's "Pipeline Artifacts" check passed (trailer honored). mika-qa posted `block[pipeline]`: "No plan document in docs/plans/ and no plan callout in issue #1150 body — grooming required before this PR can be reviewed." Operator unblocked by retroactively grooming mika#1150 and writing a throwaway plan doc.

## Fix shape (mika#1215)

Teach qa-review Step 2 to scan commit messages in `base..head` for `Pipeline-Exempt:` trailers, mirroring `verify-pipeline.sh` lines 150–172 verbatim:

- Anchor regex to start-of-line (`^Pipeline-Exempt: (docs-only|code-only)([[:space:]]+.+)?$`) — same as CI side. Defends against quoted trailers like `> Pipeline-Exempt: …`.
- Dual-form matching: with-reason (preferred) vs bare (backwards compat, warns). Bare form remains honored to match CI.
- Order: label bypass first, then trailer bypass, then strict checks (1–3). First match wins; logged reason is unambiguous.
- Distinct skip literals in the verdict body so the operator can tell which mechanism allowed the bypass: `skipped (pipeline-exempt label)`, `skipped (pipeline-exempt — docs-only trailer)`, `skipped (pipeline-exempt — code-only trailer)`.

The trailer read uses `run_shell + git log` instead of extending `qa_pr_view` or `run_gh pr view`. Rationale:

- `qa_pr_view`'s charter is "CI-stripped PR metadata" — adding a `commits` field would either leak CI status (via `statusCheckRollup`) or require a hand-stripped subset projection that expands scope beyond metadata.
- `pr view` is not in qa-review's `run_gh` scope (mika#1196). Adding it opens the CI-data surface the scope explicitly forbids.
- `run_shell + git log origin/<base>..origin/<head>` mirrors `verify-pipeline.sh`'s own implementation. Same primitive, same regex, same precedence order — by construction the two gates can't diverge silently.

## Why mirror, not consolidate

The CI side is shell/bash; the qa-review side is an LLM-interpreted prompt. They cannot share code. The parity invariant is enforced by **mirroring the regex and precedence-order verbatim** in the prompt, and citing the CI script (`verify-pipeline.sh` lines 158–172) inline. Any future change to CI's trailer semantics requires a matching prompt edit — a manual coupling, but visible.

## Deferred items

These were intentionally kept out of mika#1215 scope to honor the ticket-body wording and `feedback_implementation_scope_bundling.md`:

- **`Status: retroactive` frontmatter marker** — the ticket-body trigger condition was "if a third instance appears, teach qa-review to accept the `Status: retroactive` frontmatter marker." Two retroactive-plan-doc instances so far (mika#825 thread; mika#1185). P3-deferred to third occurrence per the ticket.
- **`documentation` label-on-linked-issue inheritance in qa-review** — separate exemption path (linked-issue semantic, not PR-level commit). Real asymmetry but wasn't the empirical break on PR #1185. If a PR is empirically broken by this specific path, file a separate ticket.

## Trailer mechanics reference

Commit trailers are commit-message-based per mika#860's design. Putting `Pipeline-Exempt:` in the PR **description** body (not a commit message) is not honored — neither CI nor qa-review reads PR descriptions for the trailer. This is by-design: commit messages are immutable, PR descriptions are not.

To apply the trailer, add a Git trailer line to any commit in the PR's `base..head` range:

```
Pipeline-Exempt: code-only — <reason>
```

Standard Git trailer format (last paragraph of the commit body, key-value with colon-space separator). With-reason form (after `—`, em-dash by convention) is preferred for the audit trail; bare form is backwards-compatible but logs a warning.

## Related

- Ticket: senara-solutions/mika#1215
- Reproduction PR: senara-solutions/mika#1185 (issue retro-groomed: senara-solutions/mika#1150)
- Trailer origin: senara-solutions/mika#860 (introduced `Pipeline-Exempt:` trailer in `verify-pipeline.sh`)
- Prior qa-review label gate: senara-solutions/mika#1064 (PR #1065, 2026-05-10) — `pipeline-exempt` label honored for docs-only PRs in qa-review
- CI label parity: senara-solutions/mika#1067 (PR shipped 2026-05-11) — `verify-pipeline.sh` extended to honor `pipeline-exempt` label
- Prior compound: `docs/solutions/prompt-engineering/qa-review-docs-only-pipeline-exempt-gate-2026-05-10.md` (label-side parity, same shape)
- Prior compound: `docs/solutions/ci-cd/ci-verify-pipeline-label-exemption-2026-05-11.md` (CI side of the same asymmetry-resolution pattern)
- Source: `skills/bundled/qa-review/system_prompt.md` (the change surface)
- Reference: `scripts/verify-pipeline.sh` (the canonical implementation; not modified)
