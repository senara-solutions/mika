# Plan — fix(perimeter): add root `.gitignore` to MECHANICAL_EXACT (mika#1864)

## Problem

PRs that ONLY modify root scaffolding files (`.gitignore`, `.gitattributes`,
`.editorconfig`) are classified as **DECISION-CORE** by the forge-gate
perimeter classifier and blocked from auto-merge — they require Vincent
hand-merge despite being pure config/scaffolding with zero decision-core
semantics.

### Root cause

`crates/mika-agent/src/perimeter/rules.rs` — root `.gitignore` matches none of
the MECHANICAL allowlist buckets:

- Not in `MECHANICAL_PREFIXES` (`docs/`, `.github/release-please/`,
  `src/claude_pilot/agent.py`, …)
- Not in `MECHANICAL_EXACT` (`pyproject.toml`, `uv.lock`, `LICENSE`,
  `CHANGELOG.md`, …)
- Not in `MECHANICAL_CONTAINS` (`/tests/`) or `MECHANICAL_SUFFIXES` (empty)

Per the fail-closed default in `classify_path` (`mod.rs:123`), any unrecognized
path → `Classification::DecisionCore`. Over-block is the deliberate safe
direction (mika#1855), but a repo-root `.gitignore` is a well-known scaffolding
file with no safety surface.

### Hard evidence

mika#1862 (2026-07-28) — a `.gitignore`-only PR (untracked 3 `.iterate/`
scratch files), APPROVED by mika-qa with all CI green, was held for human merge:

```
audit_events:
  tool_name = ci_success_handler_human_gate_required
  target_key = pr:senara-solutions/mika#1862
  reasoning = trigger=check_suite_success gate=forge-gate
              reviewer=mika-platform-qa decision_core_files=.gitignore
```

## Approach

Add the three root scaffolding files to the **exact-path** allowlist
(`MECHANICAL_EXACT`) — NOT to `MECHANICAL_PREFIXES`. Exact-match is the correct
bucket because it is a closed-world, exact-path check: it matches only the
repo-root file, never a nested `crates/foo/.gitignore` that lives inside a
decision-core path.

### Why exact-match, not prefix

A prefix or suffix rule (`.gitignore` as a suffix, or `*/​.gitignore`) would
classify `crates/mika-agent/src/perimeter/.gitignore` as MECHANICAL. That would
let a decision-core PR sneak an auto-merge by touching a nested `.gitignore`
alongside decision-core code. `MECHANICAL_EXACT` already implements the
root-only discipline via `MECHANICAL_EXACT.contains(&path)` in `is_mechanical`
(`rules.rs:158`) — the path string must equal `.gitignore` exactly, which only
the repo-root file does (touched-file paths from GitHub are repo-relative, so a
nested file is `crates/foo/.gitignore`, never bare `.gitignore`).

### Taint invariant is preserved by construction

`classify_pr_files` (`mod.rs:139`) already tallies decision-core files across
the whole PR file set and returns `DecisionCore` if the count is ≥ 1
(`mod.rs:155-159`). A mixed PR (`.gitignore` + a decision-core `.rs`) still
taints to DECISION-CORE — this fix does not touch that logic, so the invariant
holds without change. AC3's `gitignore_plus_decision_core_taints` test locks it.

## Requirements

### R1 — Extend `MECHANICAL_EXACT`

In `crates/mika-agent/src/perimeter/rules.rs`, add three entries to the
`MECHANICAL_EXACT` const array (`rules.rs:123`):

- `.gitignore`
- `.gitattributes`
- `.editorconfig`

Add a grouping comment (matching the file's existing comment style) explaining
these are repo-root scaffolding with zero decision-core semantics, and calling
out explicitly that this is exact-match (root-only) by design — nested
`.gitignore` deliberately stays DECISION-CORE (references mika#1864).

### R2 — Update the module doc-comment inventory

The `rules.rs` header doc-comment enumerates what qualifies as MECHANICAL
(`rules.rs:11-21`). Add a one-line bullet noting root scaffolding config files
(`.gitignore`/`.gitattributes`/`.editorconfig`) are MECHANICAL via exact-match,
so the audit-trail inventory stays accurate. Keep it terse.

### R3 — Tests

In `crates/mika-agent/src/perimeter/tests.rs`, add three tests following the
existing `classify_path` / `classify_pr_files` assertion patterns (see
`release_tooling_is_mechanical` and `one_decision_core_file_taints_the_batch`):

- **`root_gitignore_is_mechanical`** — `classify_path(".gitignore")`,
  `classify_path(".gitattributes")`, `classify_path(".editorconfig")` each
  return `Classification::Mechanical`; and a single-file PR touching only
  `.gitignore` (`classify_pr_files(&[".gitignore".into()])`) yields
  `verdict == Mechanical`.
- **`nested_gitignore_still_decision_core`** —
  `classify_path("crates/foo/.gitignore")` and
  `classify_path("crates/mika-agent/src/perimeter/.gitignore")` return
  `Classification::DecisionCore` (not accidentally allowed by any prefix/suffix
  rule).
- **`gitignore_plus_decision_core_taints`** —
  `classify_pr_files(&[".gitignore".into(), "crates/mika-agent/src/foo.rs".into()])`
  yields `verdict == DecisionCore`, and the decision-core file is named in the
  batch's `decision_core_files` (mirrors `one_decision_core_file_taints_the_batch`).

## Verification contract

- `cargo test -p mika-agent perimeter` — all three new tests pass, all existing
  perimeter tests still green (esp. `perimeter_module_files_are_decision_core`,
  which asserts the classifier file itself stays DECISION-CORE).
- `cargo clippy -p mika-agent` — clean.
- `cargo fmt --check` — clean.
- Manual grep-audit: confirm `.gitignore` appears ONLY in `MECHANICAL_EXACT`
  (never in `MECHANICAL_PREFIXES` or `MECHANICAL_SUFFIXES`), so the root-only
  guarantee is structural.

## Files touched

| File | Change |
|------|--------|
| `crates/mika-agent/src/perimeter/rules.rs` | Add 3 entries to `MECHANICAL_EXACT` + grouping comment + header-doc inventory line (R1, R2) |
| `crates/mika-agent/src/perimeter/tests.rs` | Add 3 tests (R3) |
| `docs/plans/2026-07-28-001-fix-1864-…-plan.md` | This plan |

No schema change, no new dependency, no runtime behavior change beyond the
classification verdict. The change is additive to a const allowlist — the
narrowest possible surface.

## Scope guard / non-goals

- **Do NOT** add nested `.gitignore` (e.g. `crates/foo/.gitignore`) — they live
  inside decision-core paths; MECHANICAL classification would open a bypass.
  Root-only, exact-match (ticket § "Note on scope").
- **Do NOT** convert to a prefix or suffix rule — that is the exact failure mode
  the exact-match bucket exists to prevent.
- **Do NOT** touch the fail-closed default (`classify_path` fall-through) or the
  taint logic (`classify_pr_files`) — mika#1855 doctrine is unchanged; this is a
  closed-world exact-path addition, no invariant expansion.
- This PR is itself DECISION-CORE (it edits `perimeter/rules.rs`) → Vincent
  hand-merge required. It is throughput-positive after landing (expands what can
  auto-merge).

## Definition of Done

- `MECHANICAL_EXACT` contains `.gitignore`, `.gitattributes`, `.editorconfig`
  with an explanatory root-only comment.
- Header doc-comment inventory updated (R2).
- Three new tests added and passing; full perimeter test suite green.
- `cargo clippy` + `cargo fmt --check` clean.
- PR body documents this is DECISION-CORE (Vincent hand-merge) and
  throughput-positive.

## Acceptance criteria

Transcribed from mika#1864 § AC.

### AC1 — Add root scaffolding files to MECHANICAL_EXACT
`crates/mika-agent/src/perimeter/rules.rs`: extend `MECHANICAL_EXACT` with
`.gitignore`, `.gitattributes`, `.editorconfig`.

### AC2 — Tests
`crates/mika-agent/src/perimeter/tests.rs` — add:
- `root_gitignore_is_mechanical` — a PR touching only `.gitignore` classifies as
  MECHANICAL.
- `nested_gitignore_still_decision_core` — a PR touching
  `crates/foo/.gitignore` classifies as decision_core (not accidentally allowed
  by prefix).
- `gitignore_plus_decision_core_taints` — a mixed PR (`.gitignore` +
  `crates/mika-agent/src/foo.rs`) taints to decision_core (existing invariant).

### AC3 — Post-deploy verify
Re-open + close a synthetic PR touching only root `.gitignore`. Verify
`audit_events` shows a MECHANICAL-verdict tool (e.g. `verdict_handler_auto_merge`
or similar) with `gate=forge-gate mechanical_files=.gitignore` — i.e. the gate
auto-merges instead of holding for human review. (Operator/acceptance-testing
step, post-merge + deploy; not part of the code diff.)

## References

- Founding incident: mika#1862 (2026-07-28) — `.gitignore` untrack B P0 fix
- Related: mika#1861 (perimeter cpp extension) — same classifier
- Doctrine: mika#1855 fail-closed default (unchanged — MECHANICAL_EXACT is an
  exact-path closed-world addition, no invariant expansion)
