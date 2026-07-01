---
type: fix
issue: 1616
title: pr_merge_with_gate PAT gap on mika-cloud PRs — extend credential scope + fallback
status: draft
---

# Plan — mika#1616 pr_merge_with_gate PAT gap on mika-cloud

## Ticket

mika#1616 — mika-dev's `pr_merge_with_gate` refuses to merge mika-cloud PRs with "PAT gap blocks pr_merge_with_gate". Autonomous-loop completion for mika-cloud tickets currently requires operator/orchestrator hand-merge. Two-path fix proposed in body — this plan commits to path (1) primary + path (2) as defense-in-depth fallback.

## Problem

`pr_merge_with_gate` is the mika-dev tool that gates PR merges on qa verdict + CI + no-conflicts. It works on senara-solutions/mika PRs (evidence: PR#1606, #1610, #1611 all merged autonomously). It fails on senara-solutions/mika-cloud PRs (evidence: mika-cloud PRs #135, #136, both required operator direct-merge via `gh pr merge --squash`).

Root cause hypothesis (per body): mika-dev's GitHub credential is one of {App installation token, PAT, OAuth token}. mika-cloud is not in the credential's allowlist OR the credential's scope doesn't permit private-repo merges (mika-cloud is private).

## Scope

**In scope (v1 ships):**

1. **Path (1) — extend credential scope.** Identify mika-dev's GitHub credential type (installation token / PAT / OAuth) via `crates/mika-common/src/config.rs` + related identity provisioning. Add senara-solutions/mika-cloud + senara-solutions/mika-skills + senara-solutions/claude-pilot-py to the credential's repo allowlist. Operator step: physically install the App on those repos (if App-based) OR add the PAT scope (if PAT-based).
2. **Path (2) — defense-in-depth fallback.** Modify `pr_merge_with_gate` implementation to attempt GraphQL `enablePullRequestAutoMerge` mutation via the standard installation token when the primary path fails with "PAT gap". If BOTH fail, emit a clear diagnostic naming which credential path was attempted.
3. Add integration test simulating mika-cloud PR merge attempt: mocked GH API returns "PAT gap" on primary, succeeds on GraphQL fallback → asserts merge succeeds via fallback.

**Out of scope:**

- Other repos' merge paths (mika works fine).
- Manual `gh pr merge` workarounds (used tonight, must not be the structural answer per body).
- Credential re-architecture (single-App-vs-multi-PAT design).

## Committed positions

1. **Path (1) primary, Path (2) fallback.** Body recommends Path (1) alone. This plan adds Path (2) as defense-in-depth so future scope-drift (a fourth repo added, credential re-provisioning) doesn't silently re-break the autonomous merge path.
2. **App-based credential preferred** if diagnosis reveals PAT. Body's Root cause hypothesis lists 3 options; preferred is GitHub App because per-repo install + per-repo revocation is cleaner than PAT scope management. The primary-vs-fallback design accommodates either.
3. **Extend allowlist to 4 repos, not just mika-cloud.** Same class of problem for mika-skills + claude-pilot-py (both potential mika-dev autonomous-merge targets). Extend proactively; test only on mika-cloud (the reported case).

## Acceptance criteria

- **AC1** — Credential diagnosis complete: `crates/mika-common/src/config.rs` + `crates/mika-agent/src/tools/pr_merge_with_gate.rs` review identifies whether mika-dev uses App/PAT/OAuth for `senara-solutions/*` merges. Diagnosis documented in the plan or a follow-up compound doc.
- **AC2** — Credential scope extended: allowlist covers senara-solutions/mika + mika-cloud + mika-skills + claude-pilot-py. If App-based: install verified on all 4. If PAT-based: token has `repo` scope for private-repo access. Operator step documented.
- **AC3** — Fallback path implemented: `pr_merge_with_gate` catches "PAT gap" error class and attempts GraphQL `enablePullRequestAutoMerge` mutation via installation token. If both fail, emits diagnostic naming which paths were attempted.
  - **F2 trigger criteria (architect required):** If error class is named (e.g., `PATScopeError`), catch by name. If error is generic 403/404 with message body, catch by status code + body pattern containing "Resource not accessible" or equivalent. If error is completely indistinguishable from other failures, ESCALATE to architect — do not implement blind fallback (would mask unrelated failures).
  - **F3 scope boundary (architect required):** If existing client abstraction requires refactor beyond credential-provider injection, BOUND to separate ticket — Path (2) implementation is limited to adding secondary credential path with the existing abstraction. Wider refactor is a followup, not scope creep here.
- **AC4** — Integration test: mocked GH API scenario asserts merge succeeds via fallback. `cargo test -p mika-agent` clean.
  - **F4 test-scaffolding boundary (architect required):** If existing GH mocking is unavailable in `crates/mika-agent/tests/eval/`, scope limited to unit-testable credential-provider logic; integration test uses recorded fixture (per `docs/adr/recorded-fixture-pattern.md` if it exists) or is deferred to manual verification, documented in the PR body. Building a full GH mocking harness is out of scope for this ticket.
- **AC5** — Verification against mika-cloud: after ship + deploy, next mika-cloud PR that lands (or a synthetic test PR) merges via autonomous path without operator intervention. Verified by dispatch log inspection.

## Deliverables (mapped to ACs)

| AC | Deliverable | File(s) |
|---|---|---|
| AC1 | Credential diagnosis | `crates/mika-common/src/config.rs`, `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — inspection + documented findings |
| AC2 | Scope extension | `~/.mika/.env` (PAT if applicable) OR GitHub App install page. Operator step. Documented in `mika-platform/docs/operator/mika-dev-credential-setup.md` (NEW). |
| AC3 | Fallback code | `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — new error-class match + GraphQL fallback + diagnostic messages |
| AC4 | Integration test | `crates/mika-agent/tests/eval/pr_merge_with_gate_test.rs` (or existing eval location) — mocked API scenario |
| AC5 | End-to-end verification | Post-deploy dispatch log verification. Attach evidence to PR body. |

## Implementation steps

**Phase 1 — Diagnosis (blocking).** Read `crates/mika-common/src/config.rs` for credential mechanism. Read `crates/mika-agent/src/tools/pr_merge_with_gate.rs` for the current gate logic + which error string is emitted. If credential is GitHub App: verify installation status on all 4 target repos via `gh api /installation/repositories`. If PAT: verify scope via `gh auth status`. Document findings.

**Phase 2 — Scope extension (operator step + code).** If App-based: install on missing repos (operator UI action). If PAT-based: rotate token with correct scope + update env. Update relevant docs.

**Phase 3 — Fallback implementation.** Modify `pr_merge_with_gate` to catch "PAT gap" (or the actual error class returned by primary path) and attempt fallback. Fallback uses the standard installation token via existing GH client abstraction.

**Phase 4 — Test.** Add integration test with mocked GH client. Assert primary-fail + fallback-succeed = merge succeeds. Assert primary-fail + fallback-fail = diagnostic error.

**Phase 5 — Verify.** Post-deploy, watch for next mika-cloud PR merge or synthesize a test. Attach dispatch-log evidence to PR body.

## Risks

1. **App installation is UI operation.** AC2 requires operator to click through GitHub App install for missing repos. Cannot be automated by orchestrator-CC. Plan surfaces this as an operator step.
2. **Fallback path uses different credential.** If installation token has different rate-limit budget or per-request semantics than PAT, fallback may hit new failure modes. Mitigation: fallback emits diagnostic on failure with both paths' error text.
3. **Test harness for GH client.** May not have mock scaffolding yet. AC4 may require building the mock; documented as implementer discretion within scope.
4. **Regression on mika PRs.** The primary-path changes could regress the working mika PR merges. AC4 test coverage must include a mika-repo scenario to catch this.
5. **Cross-repo trust surface.** Extending mika-dev's credential to mika-cloud/mika-skills/claude-pilot-py expands its blast radius. Named as a design consideration; operator judgment on whether App scoping (per-repo permissions) is preferable to PAT (global scope).

## References

- mika-cloud#135, mika-cloud#136 — the operator-hand-merge evidence
- mika PRs #1606, #1610, #1611 — the working mika-repo path
- mika#1607 — customer_id thread-through, adjacent context
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — the tool
- `crates/mika-common/src/config.rs` — credential wiring
- `mika/CLAUDE.md` § Optional (GitHub App — preferred over PAT) — credential doc
