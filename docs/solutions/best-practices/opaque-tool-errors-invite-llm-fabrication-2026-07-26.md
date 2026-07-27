---
module: tools
tags: [pr-merge-with-gate, error-classification, credential-scope, github-app, grounding, fabrication, structured-errors, gate-errored]
problem_type: opaque-error-invites-fabrication
category: best-practices
---

# Opaque tool errors invite LLM fabrication — classify credential-scope failures explicitly

## Problem (mika#1616)

mika-dev refused to merge mika-cloud PRs #135/#136 (both qa-approved, CI green,
`mergeable=MERGEABLE`), reporting: *"Awaiting Vincent's instruction on PR #136
merge (PAT gap blocks pr_merge_with_gate)."* Every mika-cloud closure then
required an operator/orchestrator hand-merge via `gh pr merge`.

Two facts surfaced during diagnosis that reframed the whole ticket:

1. **There is no "PAT gap" string anywhere in the codebase.** The phrase was the
   LLM *paraphrasing an opaque `gate_errored` result into a plausible-sounding
   cause*. The session output shows mika-dev took "no action" — it never even
   called the tool on that turn. "PAT gap" was a fabricated guess.
2. **`pr_merge_with_gate` uses a single credential** — `ctx.github_token` (the
   GitHub App installation token, PAT fallback), shelled to `gh` as `GH_TOKEN`.
   There is no per-repo allowlist in code and no "primary path vs
   installation-token fallback." The originally-proposed fix (a GraphQL
   `enablePullRequestAutoMerge` fallback "via the installation token") was a
   **no-op**: `gh pr merge --auto` already runs exactly that mutation with
   exactly that token.

The real root cause is infra: the GitHub App is not installed on the private
`senara-solutions/mika-cloud` repo, so a genuine `gh pr merge` returns HTTP 403
"Resource not accessible by integration." That is an operator step, not a code
fix.

## Lesson

**When a tool returns an opaque failure (bare exit code + raw stderr) for a
condition that has a specific, actionable cause, the LLM will invent a cause.**
An error the model cannot map to a known class becomes a confabulation surface —
here, an HTTP 403 with no classification became "PAT gap," which then routed the
work to a human instead of surfacing the true remediation.

The structural fix is not a behavioral fallback — it is **diagnostic
classification**: give the specific failure its own structured variant with an
actionable message, so the model reports a real cause and the operator gets a
concrete next step.

## Fix

Add a first-class `GateErrorKind::CredentialScope { repo }` variant. A shared
`classify_credential_scope_error(err, repo)` detects the 403 / forbidden /
"resource not accessible" / "must have admin rights" shapes (mirroring the
existing `classify_gh_error()` heuristic in `builtin_handlers.rs`) and returns a
`gate_errored` result whose `detail` names the repo **and** the remediation
(install the App with Contents + Pull requests write, or widen the PAT scope),
while preserving the underlying `gh` error for debugging.

Wire it at **every** `gh` failure site, not just the merge call — a
credential-scope 403 can surface first at the preflight `gh pr view` (private
repo the token can't read) or at `gh pr checks`, as well as at `gh pr merge`.
Order matters in the immediate-merge branch: the credential-scope check must run
before the generic draft/conflict/review classification so a 403 isn't
misattributed.

Detection is a substring heuristic on `err.to_lowercase()`. Keep it a pure,
separately-unit-tested function: assert positive classification on each 403
shape, and — equally important — assert **no false positives** on unrelated
failures ("no checks reported", "draft state", "merge conflict", "connection
refused"), so the new variant never masks a real, differently-caused failure.

## Guardrails for the class

- **A tool's error taxonomy is part of its contract.** When a failure mode has a
  distinct operator remediation, it deserves a distinct structured variant —
  not a fold into `unknown` / `gh_cli_failure`.
- **Diagnose before implementing a groomed plan.** This plan shipped groomed
  (READY → GROOMED) with a hypothesis-based fix that turned out to be a no-op.
  The F2/F3/F4 "architect required" escape hatches in the plan existed precisely
  for this; reading the code first is what let the escape hatch fire.
- **Credential/config gaps masquerade as code bugs.** "Works on repo A, fails on
  repo B" with a shared code path is an install/scope-difference signal. The
  code deliverable here is *observability of the config gap*, not a code
  workaround for it.

## References

- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — `GateErrorKind::CredentialScope`, `classify_credential_scope_error()`
- `crates/mika-agent/src/skills/builtin_handlers.rs` — `classify_gh_error()` (the 403/forbidden heuristic reused here)
- mika-cloud#135, mika-cloud#136 — the operator-hand-merge evidence
- `docs/solutions/best-practices/pr-merge-with-gate-supervisor-metadata-2026-05-20.md` — adjacent pr_merge_with_gate hardening
