//! Forge-gate perimeter classifier (mika#1829).
//!
//! Classifies a PR's touched-file set into either [`Classification::Mechanical`]
//! (auto-merge OK under verdict) or [`Classification::DecisionCore`] (human
//! gate required). The autonomous merge path
//! ([`crate::server::verdict_handler`] and [`crate::tools::pr_merge_with_gate`])
//! consults this classifier before firing `run_gh_merge` — any PR touching a
//! DECISION-CORE zone is held for the operator.
//!
//! ## Design (Prime bearing, GO Vincent 2026-07-25)
//!
//! 1. **Gradué.** Not blanket-substrate. Auto-repair loops are the win — we do
//!    not kill them. Only *decision-core* zones (permission-policy, verdict
//!    parser + handler, gate logic, dispatch authority, and the perimeter
//!    itself) require a human gate. *Harness-mécanique* zones (retry, timeout,
//!    logging, telemetry, tests, docs, dashboard) auto-merge under verdict.
//! 2. **Perimeter defines itself (non-negotiable a).** This module and its
//!    [`rules`] table are DECISION-CORE by construction — they are NOT on the
//!    MECHANICAL allowlist ([`is_mechanical`] returns `false` for
//!    `crates/mika-agent/src/perimeter/...`), so editing them auto-gates. The
//!    loop cannot silently de-gate itself by editing the allowlist. The doc
//!    at `docs/gate/perimeter.md` is likewise DECISION-CORE.
//! 3. **Behavioral, not declarative (non-negotiable b).** The classifier reads
//!    the *actual* diff (touched files). Labels are hints, never authority.
//!    A "mechanical" label on a PR that touches [`crate::server::verdict`] is
//!    still gated because the diff is what the classifier sees.
//! 4. **Fail-closed.** Any file that does not match a MECHANICAL rule is
//!    treated as DECISION-CORE. Adding zones to MECHANICAL requires editing
//!    [`rules`], which itself is DECISION-CORE.
//!
//! ## Integration sites
//!
//! - [`crate::server::verdict_handler::handle_pass_verdict`] — before
//!   `run_gh_merge` on a `VERDICT: pass` review. Skips merge dispatch if
//!   [`classify_pr_files`] returns [`Classification::DecisionCore`], falling
//!   through to `hold[review]` semantics.
//! - [`crate::tools::pr_merge_with_gate`] — as an additional preflight after
//!   DRAFT / CONFLICTING / behind-main checks. Blocks direct tool invocations
//!   (agent-authored `pr_merge_with_gate` calls) with the same policy.
//!
//! Both sites use [`fetch_pr_files`] to read the actual touched-file set from
//! GitHub before classifying — no reliance on labels, commit trailers, or PR
//! body prose.

pub mod fetch;
pub mod rules;

#[cfg(test)]
mod tests;

/// Classification of a single file path OR of a PR's overall file set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The path/PR touches only auto-merge-safe (harness, docs, tests, etc.)
    /// zones. Auto-merge under verdict is allowed.
    Mechanical,
    /// The path/PR touches at least one DECISION-CORE zone (permission-policy,
    /// verdict / gate logic, dispatch authority, perimeter itself, and any
    /// path not on the MECHANICAL allowlist by fail-closed default). The auto-
    /// merge path must NOT fire; hand over to the operator.
    DecisionCore,
}

/// Per-PR classification result with the concrete file lists that drove the
/// verdict. Attached to log lines, notifications, and (later) PR comments so
/// operators can see exactly which files tripped the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrClassification {
    pub verdict: Classification,
    /// Files matching a MECHANICAL rule.
    pub mechanical_files: Vec<String>,
    /// Files NOT matching any MECHANICAL rule (fail-closed → DECISION-CORE).
    pub decision_core_files: Vec<String>,
}

impl PrClassification {
    /// Compact, human-readable summary suitable for log lines and hold-review
    /// notifications. Names up to 5 decision-core files by path.
    pub fn summary(&self) -> String {
        match self.verdict {
            Classification::Mechanical => format!(
                "MECHANICAL ({} file{})",
                self.mechanical_files.len(),
                if self.mechanical_files.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            Classification::DecisionCore => {
                let named: Vec<&str> = self
                    .decision_core_files
                    .iter()
                    .take(5)
                    .map(String::as_str)
                    .collect();
                let more = self.decision_core_files.len().saturating_sub(5);
                let tail = if more > 0 {
                    format!(", +{more} more")
                } else {
                    String::new()
                };
                format!(
                    "DECISION-CORE ({} file{}: {}{})",
                    self.decision_core_files.len(),
                    if self.decision_core_files.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    named.join(", "),
                    tail
                )
            }
        }
    }
}

/// Classify a single file path via the MECHANICAL allowlist.
///
/// Fail-closed: any path that does not match a rule in [`rules`] is treated
/// as [`Classification::DecisionCore`].
pub fn classify_path(path: &str) -> Classification {
    if rules::is_mechanical(path) {
        Classification::Mechanical
    } else {
        Classification::DecisionCore
    }
}

/// Classify a PR's file set.
///
/// Semantics:
/// - Empty file list → [`Classification::DecisionCore`] (fail-closed: we
///   cannot know what a zero-file diff is doing; hand it to the operator).
/// - Any file matches DECISION-CORE (i.e., fails the MECHANICAL rule) → the
///   whole PR is DECISION-CORE. One decision-core file taints the batch.
/// - All files MECHANICAL → PR is MECHANICAL, auto-merge OK under verdict.
pub fn classify_pr_files(files: &[String]) -> PrClassification {
    if files.is_empty() {
        return PrClassification {
            verdict: Classification::DecisionCore,
            mechanical_files: Vec::new(),
            decision_core_files: Vec::new(),
        };
    }
    let mut mechanical_files = Vec::new();
    let mut decision_core_files = Vec::new();
    for path in files {
        match classify_path(path) {
            Classification::Mechanical => mechanical_files.push(path.clone()),
            Classification::DecisionCore => decision_core_files.push(path.clone()),
        }
    }
    let verdict = if decision_core_files.is_empty() {
        Classification::Mechanical
    } else {
        Classification::DecisionCore
    };
    PrClassification {
        verdict,
        mechanical_files,
        decision_core_files,
    }
}
