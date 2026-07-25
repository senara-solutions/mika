//! MECHANICAL allowlist for the forge-gate perimeter (mika#1829).
//!
//! Every rule in this file marks a zone as MECHANICAL — auto-merge OK under
//! mika-qa verdict. All other paths default to DECISION-CORE (fail-closed).
//!
//! **This file is DECISION-CORE by construction.** No rule below matches
//! `crates/mika-agent/src/perimeter/**`, so editing this list forces a
//! human gate. That is the second non-negotiable of the forge-gate design
//! ("perimeter defines itself" — see [`super`] doc-comment).
//!
//! ## What qualifies as MECHANICAL
//!
//! Add here ONLY zones that are pure harness — provably never DECISION-CORE:
//!
//! - retry, timeout, logging, tracing/telemetry plumbing
//! - dashboard / frontend (no engine-level authority)
//! - tests (they cannot change production behavior — they *check* it)
//! - narrative docs (logs, plans, solution write-ups); NOT authority docs
//!   (`docs/architecture/**`, `docs/adr/**`, `docs/design/**`,
//!   `docs/gate/**` — those are gated)
//!
//! ## What is explicitly DECISION-CORE (grep-anchored)
//!
//! For clarity and audit-trail — none of these paths appears in the rules
//! below, so they fail-closed to DECISION-CORE via [`super::classify_path`]:
//!
//! - `crates/mika-agent/src/perimeter/**` — the perimeter itself
//! - `crates/mika-agent/src/server/verdict.rs` — verdict parser
//! - `crates/mika-agent/src/server/verdict_handler.rs` — verdict-form / dispatch-authority
//! - `crates/mika-agent/src/tools/pr_merge_with_gate.rs` — gate-logic
//! - `crates/mika-agent/src/skills/executor.rs` — dispatch-authority
//! - `skills/bundled/permission-policy/**` — permission-policy
//! - `skills/bundled/_shared/dispatch-lib.sh` — dispatch shared plumbing (whole file gates; retry & authority are entangled)
//! - `skills/bundled/qa-review/**` — verdict-form (mika-qa's contract)
//! - `.github/labels.yml` — label taxonomy (governance)
//! - `docs/gate/perimeter.md` — the perimeter doc (self-reference at doc layer)
//! - `docs/architecture/**`, `docs/adr/**`, `docs/design/**` — authority docs
//!
//! ## Growth
//!
//! Adding a new MECHANICAL prefix requires a Vincent-gated PR (this file is
//! DECISION-CORE). Justify in the PR body why the zone is provably
//! harness-only. Err on the side of not-adding — a false MECHANICAL breach
//! costs more than a false DECISION-CORE hold.

/// Prefixes that grant MECHANICAL. A path is MECHANICAL if it starts with any
/// of these strings.
///
/// Kept narrow. Prefer adding to this list only after real evidence that a
/// zone is pure harness (never authority).
const MECHANICAL_PREFIXES: &[&str] = &[
    // --- Narrative docs (logs, plans, solutions, calibration write-ups) ---
    "docs/logs/",
    "docs/plans/",
    "docs/solutions/",
    "docs/eval/calibration/",
    // --- Test scaffolding (integration test suites) ---
    // Tests can only *check* production behavior; they cannot change it.
    // The engine ignores test code at runtime.
    "crates/mika-agent/tests/",
    "crates/mika-common/tests/",
    "crates/mika-gateway/tests/",
    "crates/mika-a2a/tests/",
    "crates/mika-cli/tests/",
    // --- Fixtures — read by tests only ---
    "crates/mika-agent/tests/fixtures/",
    "tests/fixtures/",
    // --- Telemetry / tracing plumbing (log emission, span attributes) ---
    // These emit observability data; they do NOT decide verdicts, gates,
    // dispatch, or permissions.
    "crates/mika-agent/src/telemetry/",
    // --- Dashboard / frontend (no engine authority) ---
    "dashboard/",
    "packages/ui/",
    "docs-site/",
    // --- Release / packaging tooling (versioning artifacts) ---
    // These control how versions are stamped and released, not what the
    // engine does at runtime.
    ".github/release-please/",
];

/// Exact file paths that grant MECHANICAL.
const MECHANICAL_EXACT: &[&str] = &[
    "CHANGELOG.md",
    "cliff.toml",
    "release-please-manifest.json",
    ".release-please-manifest.json",
    "release-please-config.json",
    // The workspace-level README — cosmetic for humans, not consumed by
    // the engine.
    "README.md",
];

/// Path substrings that grant MECHANICAL when found anywhere in the path.
///
/// Kept minimal — substring matching is coarser than prefix matching and
/// carries higher false-positive risk. Justify each entry.
const MECHANICAL_CONTAINS: &[&str] = &[
    // Any test module directory anywhere under a crate, e.g.
    // `crates/mika-common/src/foo/tests/mod.rs`. Test modules cannot
    // change production behavior at runtime.
    "/tests/",
];

/// Path suffixes that grant MECHANICAL. Not currently used; kept for
/// symmetry with prefix/exact/contains and to make the growth pattern
/// obvious.
const MECHANICAL_SUFFIXES: &[&str] = &[];

/// Return `true` if `path` is on the MECHANICAL allowlist. Fail-closed:
/// paths not matching any rule are considered DECISION-CORE.
pub fn is_mechanical(path: &str) -> bool {
    if MECHANICAL_EXACT.contains(&path) {
        return true;
    }
    if MECHANICAL_PREFIXES
        .iter()
        .any(|&prefix| path.starts_with(prefix))
    {
        return true;
    }
    if MECHANICAL_CONTAINS
        .iter()
        .any(|&substr| path.contains(substr))
    {
        return true;
    }
    if MECHANICAL_SUFFIXES
        .iter()
        .any(|&suffix| path.ends_with(suffix))
    {
        return true;
    }
    false
}
