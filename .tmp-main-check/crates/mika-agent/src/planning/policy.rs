//! Agent-loop policy constants — step budgets, timeouts, byte/char caps,
//! staleness thresholds. Per Foundation §6, `planning/` owns the rules
//! governing what the agent loop is allowed to do (budgets/timeouts/caps).
//!
//! Consumed by `crate::agent::RunMode::max_steps`,
//! `crate::agent::SilentTrigger::max_steps`, and other loop sites.
//! The impl methods that read these constants stay near their enums in
//! `agent.rs` (agent_loop/ #1452 domain); only the constants relocate.

pub const MAX_TOOL_STEPS: usize = 20;

pub const MAX_CALLBACK_TOOL_STEPS: usize = 20;

pub const MAX_TEAM_TOOL_STEPS: usize = 20;

pub const TOOL_TIMEOUT_SECS: u64 = 30;

/// Resolve the per-agent turn envelope, in seconds, from the provider that will
/// serve the turn (mika#2189 D1).
///
/// Before mika#2189 this was `AGENT_TOTAL_TIMEOUT_SECS`, a bare `300` with no
/// env var and no per-agent setting — while the per-call plafond it must
/// contain (`MIKA_LLM_HTTP_TIMEOUT_SECS`, mika#1660) *did* have one. Measured
/// consequence: 27 % of mika-arch passes burned ≥ 200 s of pure LLM time inside
/// a 300 s envelope that also had to hold the tool calls, and the only
/// available remedy — raising the plafond — would have let one call swallow the
/// envelope of a pass that averages 3.1 calls.
///
/// The envelope is read off the **provider** rather than from a `Settings`
/// threaded through `AgentParams` because the provider is the object that
/// already holds the plafond: taking both from one place is what stops them
/// drifting apart again. See `LlmProvider::timeout_budget`.
///
/// # The structural gate this replaces
///
/// The old constant had three readers (`agent_loop` × 2, plus the team sibling
/// below). A grep for `AGENT_TOTAL_TIMEOUT_SECS` outside comments must now find
/// nothing — pinned by `policy::tests::no_bare_agent_timeout_constant_remains`,
/// because a gate that covers three callers out of four is a gate that lies.
pub fn agent_total_timeout_secs(llm: &dyn mika_common::llm::LlmProvider) -> u64 {
    llm.timeout_budget().agent_total_timeout_secs()
}

/// Maximum bytes for callback results injected into the system prompt via
/// `format_callback_framing()`. Results exceeding this are truncated to prevent
/// oversized prompts from consuming the agent timeout during serialization.
/// Full results remain available in task logs.
pub const CALLBACK_RESULT_MAX_BYTES: usize = 10_240;

/// Per-agent timeout for team sub-agents.
///
/// Since team agents run in parallel, the constraint is fitting within the
/// global team run budget (max of agent times, not sum) — so this tracks the
/// same envelope as [`agent_total_timeout_secs`] rather than being a second
/// number that happens to read 300.
///
/// The doc comment on the constant this replaces said "matches
/// AGENT_TOTAL_TIMEOUT_SECS", which was a promise the type system did not keep:
/// the first change to either value would have silently broken it. mika#2189
/// makes the match structural.
pub fn team_agent_timeout_secs(llm: &dyn mika_common::llm::LlmProvider) -> u64 {
    agent_total_timeout_secs(llm)
}

/// Wrapper timeout for the `delegate_task` tool, in seconds.
///
/// **The one static reader of the envelope, and it is static for a reason that
/// is worth writing down rather than working around.** `Tool::timeout_secs`
/// takes `&self` and nothing else — no provider, no context — so this site
/// physically cannot follow a per-agent envelope the way
/// [`agent_total_timeout_secs`] does.
///
/// The limitation, stated rather than discovered: an agent configured with an
/// envelope **above** the default and delegating through this tool would be cut
/// at the default, not at its own envelope. That is not reachable today (the
/// only agent mika#2189 D4 retunes is mika-arch, a read-only architect with no
/// `delegate_task` in its allowlist), which is why this ships as a documented
/// bound rather than as a `Tool` trait change bundled into a timeout ticket.
pub const DELEGATE_TASK_TOOL_TIMEOUT_SECS: u64 = mika_common::llm::DEFAULT_AGENT_TOTAL_TIMEOUT_SECS;

/// Timeout for the continuation API call after max tool steps are exceeded.
/// Longer than TOOL_TIMEOUT_SECS because this is a full generation call, not a tool.
pub const CONTINUATION_TIMEOUT_SECS: u64 = 60;

/// Maximum total base64 image bytes across all tool results in a single agent step.
/// Prevents memory spikes when multiple tools return images in one step.
/// 5 images at 5 MB each ≈ 33 MB base64 — this caps at ~20 MB to stay within
/// container memory limits (256 MB target).
pub const MAX_IMAGE_BYTES_PER_STEP: usize = 20 * 1024 * 1024;

/// Maximum age (in minutes) for a failed callback to be delivered to the agent.
/// Failed callbacks older than this are silently marked as delivered to prevent
/// flooding the conversation with stale failures (e.g., after an upgrade).
pub const STALE_FAILED_CALLBACK_MINUTES: i64 = 5;

/// Maximum total characters for serialized tool call metadata.
pub const TOOL_METADATA_MAX: usize = 4000;

/// Maximum characters for tool input summary in metadata.
pub const INPUT_SUMMARY_MAX: usize = 200;

/// Maximum characters for tool output summary in metadata.
pub const OUTPUT_SUMMARY_MAX: usize = 300;

/// Maximum characters of conversation/memory digest injected into the reflection prompt.
/// ~12,500 tokens at 4 chars/token -- keeps total prompt well within Claude's context.
pub const MAX_REFLECTION_DIGEST_CHARS: usize = 50_000;

#[cfg(test)]
mod tests {
    /// Fails if any code still reads a bare per-agent-envelope constant instead
    /// of [`super::agent_total_timeout_secs`] (mika#2189 D1, verification V1).
    ///
    /// # Why a structural gate and not a unit test
    ///
    /// The old `AGENT_TOTAL_TIMEOUT_SECS` had **four** readers: two in
    /// `agent_loop` (the conversation and silent deadlines), plus
    /// `TEAM_AGENT_TIMEOUT_SECS`, which was a second literal `300` whose doc
    /// comment merely *promised* it matched — a promise the type system did not
    /// keep, and which the first change to either value would have broken
    /// silently. A unit test on the accessor proves the accessor works; it
    /// cannot prove nobody bypassed it. Per
    /// `feedback_structural_gate_audit_grep_all_callsites`, a gate covering
    /// three callers out of four is a gate that lies.
    ///
    /// # What it detects, and what it deliberately does not
    ///
    /// A line that *uses* one of the retired constant names outside a comment.
    /// Doc prose naming them (this file is full of it, explaining why they are
    /// gone) is exempt — otherwise the gate would forbid documenting itself.
    /// The one legitimate remaining literal reader,
    /// [`super::DELEGATE_TASK_TOOL_TIMEOUT_SECS`], is a different name on
    /// purpose: `Tool::timeout_secs` takes `&self` and cannot reach a provider,
    /// so that limitation is stated at its definition rather than hidden behind
    /// a name that suggests it follows the envelope.
    ///
    /// Per the plan's Fire-Disposition table, a failure here is
    /// **halt-and-escalate**: a new bare reader means a turn somewhere is
    /// deadlined against 300 s while its provider was configured for something
    /// else, which is the exact drift mika#2189 exists to end. Do not silence
    /// this test — remove the reader.
    #[test]
    fn no_bare_agent_timeout_constant_remains() {
        // The retired names, spelled in halves.
        //
        // Not obfuscation — necessity. A gate that scans every `.rs` under
        // `src/` scans this file too, so writing either name whole here would
        // make the gate its own first offender. `concat!` reconstitutes them at
        // compile time while no source line ever carries a full token.
        //
        // The team sibling is included because it was the *fourth* reader: a
        // second bare `300` whose doc comment claimed to match the first. A
        // grep for the obvious name alone misses it, which is precisely the
        // partial-coverage failure this gate exists to refuse.
        const RETIRED: &[&str] = &[
            concat!("AGENT_TOTAL_", "TIMEOUT_SECS"),
            concat!("TEAM_AGENT_", "TIMEOUT_SECS"),
        ];

        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

        let mut offenders = Vec::new();
        let mut stack = vec![src_root.clone()];
        let mut scanned = 0usize;

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("the gate must be able to read {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("readable directory entry").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("the gate must be able to read {}: {e}", path.display())
                });
                scanned += 1;
                for (n, line) in content.lines().enumerate() {
                    let trimmed = line.trim_start();
                    // Comments are how this ticket explains itself. Scanning
                    // them would make the explanation the violation.
                    if trimmed.starts_with("//") || trimmed.starts_with("///") {
                        continue;
                    }
                    // `DEFAULT_AGENT_TOTAL_TIMEOUT_SECS` (mika-common) contains
                    // the retired name as a substring but is the *new* home of
                    // the default — match on the retired name only when it is
                    // not preceded by an identifier character.
                    for name in RETIRED {
                        let mut from = 0usize;
                        while let Some(rel) = line[from..].find(name) {
                            let at = from + rel;
                            let preceded_by_ident = line[..at]
                                .chars()
                                .next_back()
                                .is_some_and(|c| c.is_alphanumeric() || c == '_');
                            if !preceded_by_ident {
                                offenders.push(format!(
                                    "{}:{}: {}",
                                    path.strip_prefix(&src_root).unwrap_or(&path).display(),
                                    n + 1,
                                    line.trim()
                                ));
                                break;
                            }
                            from = at + name.len();
                        }
                    }
                }
            }
        }

        assert!(scanned > 0, "the gate scanned no files — broken path");
        assert!(
            offenders.is_empty(),
            "a bare per-agent-envelope constant is still read in {} place(s). \
             Read the envelope off the provider via \
             `planning::policy::agent_total_timeout_secs(llm)` instead — see mika#2189 D1:\n{}",
            offenders.len(),
            offenders.join("\n")
        );
    }
}
