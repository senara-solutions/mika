# Audits

Time-stamped audit reports for cross-cutting concerns (accessibility, security, performance). Each audit has a methodology section and a finding catalog with dispositions. Findings flagged as `file-follow-up` link to GitHub issues.

## Naming convention

`YYYY-MM-DD-<domain>-<scope>-audit.md` — e.g., `2026-05-06-dashboard-a11y-audit.md`.

## Disposition taxonomy

- **fix-here** — fixed in the same PR that produced the audit.
- **file-follow-up** — filed as a GitHub issue with severity, repro, and proposed fix.
- **accept-with-rationale** — intentional; one-line rationale documented inline.
