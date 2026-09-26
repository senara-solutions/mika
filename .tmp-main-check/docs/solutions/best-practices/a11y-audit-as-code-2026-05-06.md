---
module: dashboard
tags: [accessibility, audit, testing, ci, axe-core]
problem_type: process
category: best-practices
date: 2026-05-06
---

# Accessibility audit as code — capturing audit findings durably

## Problem

Accessibility audits decay rapidly when stored in conversation, Slack, or PR descriptions. Findings get lost, dispositions are forgotten, and the same issues are rediscovered repeatedly. Without CI enforcement, regressions silently accumulate.

## Solution

Structure the audit as durable code artifacts:

1. **`docs/audits/` for the finding catalog.** Time-stamped markdown with methodology, findings table, and disposition column. Each finding gets one of three dispositions: `fix-here`, `file-follow-up`, or `accept-with-rationale`. The audit doc is the single source of truth for what was checked and what was found.

2. **Per-component test assertions for fix-here findings.** Every primitive gets an `axe(container)` assertion in its test file. The assertion is the durable proof that the fix landed and stays. `jest-axe` wraps axe-core and integrates with vitest/jest via `toHaveNoViolations()`.

3. **Follow-up issues for file-follow-up findings.** Each finding that can't be fixed in the audit PR (wrong scope, design-system territory, page-level fix that conflicts with in-flight work) gets a GitHub issue with severity, repro, and proposed fix.

4. **CI gate for regression prevention.** Add the test suite to CI so future PRs that break a11y fail the build. This is the highest-leverage artifact — it prevents the audit from decaying.

## Key decisions

- **Scope-bound fixes to primitives only.** Fixing page-level a11y issues in an audit PR creates merge conflicts with every in-flight page redesign. The audit documents page-level findings; page-level fixes happen in their respective tickets.
- **Mechanical triage rules.** Source path determines disposition: `packages/ui/src/components/` → fix-here; `dashboard/src/` → file-follow-up; design-system territory → file-follow-up. No judgment calls needed.
- **axe-core in jsdom, not Playwright.** jsdom axe catches structural/semantic issues (missing ARIA, role violations, label gaps) but cannot check color contrast or layout. This is the right tradeoff for a component library — structural a11y is the library's responsibility; visual a11y depends on the consuming app's theme and layout.

## Canonical example

PR: mika#668. Audit doc: `docs/audits/2026-05-06-dashboard-a11y-audit.md`. The audit doc structure (methodology → automated findings → keyboard walkthrough → contrast matrix → reduced motion → finding catalog with dispositions) is the canonical template for future audits (security, performance, etc.).

## References

- `packages/ui/CLAUDE.md` § Accessibility Standards — review-fail criteria for new primitives
- `docs/audits/README.md` — naming convention and disposition taxonomy
