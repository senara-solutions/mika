//! mika#2220 — end-to-end capture on the TUI-agent surface.
//!
//! Its own test binary, and a single `#[test]`, because [`logging::init_pretty`]
//! calls `.init()`: the process-global default subscriber can be installed once.
//! Two things are pinned here that no in-crate test can reach, because both are
//! properties of the *installed* subscriber rather than of the filter value:
//!
//! 1. **The falsified hypothesis (AC1).** mika#2220 was groomed on the theory
//!    that `init_pretty` + `LogOutput::FileOnly` + `default_level = "warn"` drops
//!    `mika::llm_debug` DEBUG events — that the base level suppresses the specific
//!    directive. It does not. This test measured it: the body lands in the JSON
//!    file. The real defect was upstream, in how the flag's *value* was read
//!    (`mika-common::logging::LogLlmBodiesSetting`), not in the filter. This test
//!    stays so that theory cannot be re-litigated, and so a future filter change
//!    that genuinely breaks the surface fails here.
//!
//! 2. **The announce line (AC2).** When capture is armed, the process says so and
//!    names the file it writes to. That is the half an operator needs: the flag is
//!    process-global while an agent's turns are not all served by one process, so
//!    "which file" is the question an empty log leaves unanswered.
//!
//! `default_level` is `"warn"` on purpose — the CLI's own default (`resolve_log_level`
//! in `mika-cli/src/main.rs`), and the exact configuration the ticket reported as
//! inert.

use mika_common::logging::{self, LogOutput, NoopLayer};

#[test]
fn mika2220_tui_agent_captures_llm_bodies_and_names_its_sink() {
    let dir = tempfile::tempdir().expect("tempdir");

    let guard = logging::init_pretty(
        "warn",
        Some(dir.path()),
        LogOutput::FileOnly,
        None::<NoopLayer>,
        true,
    );

    // The gate the providers actually use (`claude.rs`, `openai.rs`, `ollama.rs`).
    assert!(
        tracing::enabled!(target: "mika::llm_debug", tracing::Level::DEBUG),
        "the emission gate must be open, or the providers never build a body to log"
    );

    tracing::debug!(target: "mika::llm_debug", body = "REQUEST_BODY_MARKER", "llm request body");
    tracing::debug!(target: "mika::llm_debug", body = "RESPONSE_BODY_MARKER", "llm response body");

    // Dropping the guard flushes and stops the non-blocking file writer.
    drop(guard);

    let logged = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(|entry| std::fs::read_to_string(entry.ok()?.path()).ok())
        .collect::<String>();

    assert!(
        logged.contains("REQUEST_BODY_MARKER") && logged.contains("RESPONSE_BODY_MARKER"),
        "both bodies must reach the daily JSON file under FileOnly + default_level=warn \
         (mika#2220 AC1: the groomed hypothesis said they would not); got:\n{logged}"
    );

    assert!(
        logged.contains("llm_body_capture"),
        "an armed capture must announce itself, or an operator reading an empty log \
         cannot tell 'the flag is off here' from 'the turn ran elsewhere' (mika#2220 AC2); \
         got:\n{logged}"
    );
    assert!(
        logged.contains("mika.log."),
        "the announce line must name the sink it writes to, not merely that capture is on; \
         got:\n{logged}"
    );
}
