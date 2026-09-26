# Forge-Gate Perimeter

Classifies a PR's touched-file set to decide whether the autonomous merge path can fire, or whether the operator must merge by hand. Introduced by mika#1829 after PR mika#1827 auto-merged its own retry-loop fix — a self-substrate change that should have been human-gated but wasn't.

## Design (Prime bearing, GO Vincent 2026-07-25)

### 1. Gradué

Not blanket-substrate. Auto-repair loops are the win — killing them to defend against self-modification is over-correction. Two zones:

- **DECISION-CORE** — permission-policy, verdict-form (parser + prompt contract), gate logic, dispatch authority, perimeter itself. Editing any of these is a governance act. Human gate mandatory.
- **HARNESS-MÉCANIQUE** — retry, timeout, logging, telemetry, tests, docs, dashboard. The loop can fix these under mika-qa verdict without operator intervention. PR mika#1827's retry fix belongs here — mechanical.

### 2. Perimeter defines itself (non-negotiable a)

`crates/mika-agent/src/perimeter/**` and `docs/gate/perimeter.md` (this file) are themselves DECISION-CORE. They are not on the MECHANICAL allowlist — so editing them auto-gates. The loop cannot silently de-gate itself by editing the allowlist.

This is a structural invariant: covered by `perimeter_module_files_are_decision_core` and `perimeter_doc_is_decision_core` in `crates/mika-agent/src/perimeter/tests.rs`. If a refactor moves the module under a MECHANICAL prefix, those tests fail.

### 3. Behavioral, not declarative (non-negotiable b)

The classifier reads the *actual* diff (files touched, via `gh pr view --json files`). Labels are hints, never authority. A "mechanical" label on a PR that touches `crates/mika-agent/src/server/verdict.rs` is still gated because the diff is what the classifier sees. Re-classifying by adding a false label buys nothing.

### 4. Fail-closed

Any file that does not match a MECHANICAL rule is DECISION-CORE. Empty diff → DECISION-CORE (we cannot verify a zero-file diff's intent). New / unclassified path → DECISION-CORE. In doubt, gate.

## What's MECHANICAL

Canonical list: `crates/mika-agent/src/perimeter/rules.rs::MECHANICAL_PREFIXES` + `MECHANICAL_EXACT` + `MECHANICAL_CONTAINS`.

Broadly:

- Narrative docs — `docs/logs/**`, `docs/plans/**`, `docs/solutions/**`, `docs/eval/calibration/**`
- Integration + inline test files — `crates/*/tests/**`, `**/tests/**` substring
- Telemetry / tracing plumbing — `crates/mika-agent/src/telemetry/**`
- Frontend — `dashboard/**`, `packages/ui/**`, `docs-site/**`
- Release / packaging artifacts — `CHANGELOG.md`, `cliff.toml`, release-please files, top-level `README.md`

## What's DECISION-CORE (grep-anchored)

By fail-closed default. Named callouts (not exhaustive):

- `crates/mika-agent/src/perimeter/**` — the perimeter itself (self-reference)
- `crates/mika-agent/src/server/verdict.rs` — verdict parser
- `crates/mika-agent/src/server/verdict_handler.rs` — verdict → dispatch authority
- `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — gate logic
- `crates/mika-agent/src/skills/executor.rs` — dispatch-readiness gate
- `skills/bundled/permission-policy/**` — permission-policy contract
- `skills/bundled/_shared/dispatch-lib.sh` — dispatch plumbing (retry + authority entangled; whole file gates)
- `skills/bundled/qa-review/**` — verdict-form (mika-qa's contract)
- `.github/labels.yml` — label taxonomy governance
- `docs/gate/perimeter.md` — this doc (self-reference at doc layer)
- `docs/architecture/**`, `docs/adr/**`, `docs/design/**` — authority docs

Everything not on the MECHANICAL allowlist gates. There is no explicit DECISION-CORE list to maintain — that's the fail-closed guarantee.

## Integration sites

Two call sites both fetch `gh pr view --json files` then call `perimeter::classify_pr_files`:

1. **`server::verdict_handler::handle_pass_verdict`** — before `run_gh_merge` on a `VERDICT: pass` review. If DECISION-CORE, transitions to `hold[review]` semantics (notify operator, task stays `in_progress`). Records a `verdict_handler_human_gate_required` audit event.

2. **`tools::pr_merge_with_gate`** — as an additional preflight after DRAFT / CONFLICTING / behind-main checks. Returns `BlockReason::HumanGateRequired { decision_core_files }`. Blocks direct tool invocations (agent-authored `pr_merge_with_gate` calls) with the same policy.

Both sites fail-open on `gh` API failure — a fetch error logs a warning and DOES NOT auto-merge. It falls through to the operator gate. This is deliberate: an unreliable classifier must never auto-approve. See `perimeter::fetch::FetchError`.

## Growth

MECHANICAL allowlist additions require a Vincent-gated PR (this doc + the rules file are DECISION-CORE). Justify in the PR body why the zone is provably harness-only. **Err toward not-adding** — a false MECHANICAL breach costs more than a false DECISION-CORE hold.

## Founding evidence

- PR mika#1827 auto-merged by `mika-platform-dev` bot 2026-07-25 06:42:59Z, 2min54s after mika-qa APPROVED. Prose "Do not merge autonomously" in PR body was ignored — non-machine-readable. See samidarko-CC spool report `2026-07-25-064943-...URGENT-1827-breach-confirmed-1829-filed-draft-guard-active.md`.
- mika#1829 filed as the structural fix. This module + doc are the shipped answer.
