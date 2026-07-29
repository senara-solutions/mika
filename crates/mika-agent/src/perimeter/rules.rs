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
//! - repo-root scaffolding config (`.gitignore`/`.gitattributes`/`.editorconfig`)
//!   via exact-match (root-only); nested copies stay DECISION-CORE (mika#1864)
//! - repo-root dependency manifests (`Cargo.toml`/`Cargo.lock` for mika,
//!   `pyproject.toml`/`uv.lock` for claude-pilot) via exact-match (root-only) —
//!   Dependabot-managed, diff-visible, never engine-runtime authority (mika#1729);
//!   nested member-crate manifests stay DECISION-CORE
//!
//! ## What is explicitly DECISION-CORE (grep-anchored)
//!
//! For clarity and audit-trail — none of these paths appears in the rules
//! below, so they fail-closed to DECISION-CORE via [`super::classify_path`]:
//!
//! ### mika (senara-solutions/mika)
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
//! ### claude-pilot (senara-solutions/claude-pilot) — mika#1860, precondition for cpp#79
//!
//! - `src/claude_pilot/tier1.py` — tier-1 auto-approval classifier
//! - `src/claude_pilot/policy.py` — tier-2 policy YAML evaluator (decision authority)
//! - `src/claude_pilot/permissions.py` — can_use_tool orchestration + chain-safe
//! - `src/claude_pilot/per_spawn.py` — per-spawn permission-policy evaluator (mika#1708)
//! - `src/claude_pilot/policies/permissions.yaml` — policy rules themselves
//! - `src/claude_pilot/policies/**` (any `.yaml` under this prefix) — any future policy file
//! - `src/claude_pilot/policies/__init__.py` — policy loader (touches loader = touches policy)
//!
//! Non-security cpp python files (agent.py, cli.py, guardrails.py, inbox_writer.py,
//! ipython/, logger.py, transport.py, types.py, ui.py) are enumerated as MECHANICAL
//! below; anything not listed defaults to DECISION-CORE via fail-closed.
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
    // --- claude-pilot (senara-solutions/claude-pilot) non-security python ---
    //
    // Precondition for cpp#79 (autonomous onboarding). Each path below is
    // provably non-security-critical: no allowlist, no policy decision, no
    // permission surface. DECISION-CORE cpp files (tier1.py, policy.py,
    // permissions.py, per_spawn.py, policies/*.yaml, policies/__init__.py)
    // are NOT in this list — they fail-closed to DECISION-CORE via the
    // classify_path fall-through, correct semantics.
    //
    // The MECHANICAL classification only activates ONCE cpp webhooks flow
    // through mika-agent (post-cpp#79). Until then this list is precondition
    // work only. mika#1860.
    "src/claude_pilot/agent.py", // ClaudeSDKClient wrapper — delegates to permissions.py
    "src/claude_pilot/cli.py",   // argparse + signal handling — no policy
    "src/claude_pilot/guardrails.py", // stall/empty/idle detection — no permission surface
    "src/claude_pilot/inbox_writer.py", // gateway POST side-channel — no permission decision
    "src/claude_pilot/ipython/", // optional REPL magics — opt-in `[ipython]` extra
    "src/claude_pilot/logger.py", // stderr + file sink — no auth surface
    "src/claude_pilot/transport.py", // asyncio subprocess wrapper — no allowlist
    "src/claude_pilot/types.py", // Pydantic models — no runtime decision
    "src/claude_pilot/ui.py",    // ANSI color renderer — no logic
    // cpp tests at repo root (mika tests live under `crates/*/tests/` and are
    // already covered by the MECHANICAL_CONTAINS `/tests/` rule below).
    "tests/",
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
    // --- claude-pilot (senara-solutions/claude-pilot) root artifacts ---
    // Precondition for cpp#79. Python packaging + license — no security surface.
    "pyproject.toml", // cpp deps (Dependabot-managed; visible in diff)
    "uv.lock",        // cpp dep lock (regenerated from pyproject.toml)
    "LICENSE",        // license file (both repos have one; universally safe)
    // --- mika (senara-solutions/mika) root dependency manifests (mika#1729) ---
    // Rust equivalents of cpp's pyproject.toml / uv.lock above: Dependabot-managed,
    // fully visible in the diff, and consumed by cargo — not by the engine at
    // runtime. Exact-match (root-only) BY DESIGN so the autonomous Dependabot
    // review chain (Dependabot → mika-qa approve → mika-dev merge, mika#1729 AC4)
    // clears the forge-gate for a workspace-root dependency bump. A NEW dependency
    // (vs a version bump) is still surfaced by qa-review's diff pass + CI build,
    // and the AC5 DEP-REVIEW advisory/changelog check runs regardless. NESTED
    // member-crate manifests (e.g. `crates/mika-agent/Cargo.toml`) deliberately
    // stay DECISION-CORE: a prefix/suffix rule here would open an auto-merge
    // bypass for decision-core PRs. Grouped `[workspace.dependencies]` bumps —
    // the common Dependabot shape on this workspace — touch only the two paths
    // below plus (optionally) member manifests, which then route to the operator.
    "Cargo.toml", // workspace root manifest ([workspace.dependencies] lives here)
    "Cargo.lock", // workspace lockfile (regenerated from Cargo.toml)
    // --- Repo-root scaffolding config (mika#1864) ---
    // Pure config/scaffolding with zero decision-core semantics. EXACT-match
    // (root-only) BY DESIGN: `is_mechanical` checks `MECHANICAL_EXACT.contains(&path)`,
    // so only the bare repo-root path matches. A nested copy arrives as
    // `crates/foo/.gitignore` and deliberately stays DECISION-CORE — a prefix
    // or suffix rule here would open an auto-merge bypass for decision-core PRs.
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
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
