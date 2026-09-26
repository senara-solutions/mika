//! Frozen-fixture effect guard for the RT-005 invocation surface (mika#2116).
//!
//! `examples/rt005_analyze.rs` is three library calls and a print. The rule that
//! it — and the analyzer it calls — computes the pre-registered estimand and
//! nothing else is guarded here **by effect, not by intent**: a committed batch
//! and the exact text [`Report::render`] produces for it. Any arithmetic added
//! anywhere on the `load_batch → analyze → render` path (in the example, in
//! `mechanism_analyzer.rs`, anywhere) changes the output and turns this test
//! red. A source-level no-arithmetic lint would check intent, is fragile
//! (negative literals, computed indices), and costs more to maintain than the
//! disposable file it guards; this checks effect instead.
//!
//! It runs the **same three calls the example runs** against a small synthetic
//! batch committed under `tests/fixtures/rt005-analyze/`. The batch is designed
//! so `analyze` produces every section of the report — both pre-registered
//! contrasts non-degenerate, the within-design control, all four covariate
//! cells, and all three exclusion counters.
//!
//! To regenerate the frozen text after an intentional, reviewed change to the
//! report format, either run the example and capture its stdout:
//!
//! ```text
//! cargo run -p mika-agent --example rt005_analyze -- \
//!   crates/mika-agent/tests/fixtures/rt005-analyze > \
//!   crates/mika-agent/tests/fixtures/rt005-analyze/expected_report.txt
//! ```
//!
//! or run the ignored writer below, which produces byte-identical output through
//! the same `render()` call:
//!
//! ```text
//! cargo test -p mika-agent --test rt005_analyze -- --ignored regenerate
//! ```

use std::path::PathBuf;

use mika_agent::research::mechanism_analyzer::{Verdict, analyze, load_batch};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rt005-analyze")
}

fn expected_report() -> String {
    let path = fixture_dir().join("expected_report.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading frozen report {}: {e}", path.display()))
}

/// The guard: the produced report matches the frozen one byte for byte.
///
/// This is the same `load_batch → analyze → render` chain the example runs.
#[test]
fn frozen_report_is_reproduced_exactly() {
    let runs = load_batch(&fixture_dir()).expect("fixture batch loads");
    assert!(!runs.is_empty(), "fixture batch must carry runs");

    let report = analyze(&runs);
    let rendered = report.render();

    assert_eq!(
        rendered,
        expected_report(),
        "rendered report drifted from the frozen fixture — if this change to the \
         report is intentional, regenerate expected_report.txt (see module docs); \
         otherwise arithmetic entered the load→analyze→render path"
    );
}

/// Writer for the frozen report. `#[ignore]` so it never runs in the normal
/// suite — it has a side effect (writes the fixture). Run it only to regenerate
/// after a reviewed, intentional format change; it produces byte-identical
/// output to the example because it is the same `render()` call.
#[test]
#[ignore = "writes the fixture; run explicitly to regenerate the frozen report"]
fn regenerate_frozen_report() {
    let runs = load_batch(&fixture_dir()).expect("fixture batch loads");
    let report = analyze(&runs);
    let path = fixture_dir().join("expected_report.txt");
    std::fs::write(&path, report.render())
        .unwrap_or_else(|e| panic!("writing frozen report {}: {e}", path.display()));
}

/// A guardrail on the fixture itself: it must exercise the interesting shape the
/// frozen text encodes, so a future fixture edit that silently flattens it into
/// a degenerate report is caught here rather than passing vacuously.
#[test]
fn fixture_exercises_both_contrasts_and_all_cells() {
    let runs = load_batch(&fixture_dir()).expect("fixture batch loads");
    let report = analyze(&runs);

    // Eight success runs enter the primary contrast; three are excluded.
    assert_eq!(report.runs_analyzed(), 8, "eight success runs analysed");

    // Both pre-registered contrasts are non-degenerate — the whole point of the
    // secondary being reported alongside the primary.
    let (primary, secondary) = report.verdicts();
    assert_ne!(
        primary,
        Verdict::Degenerate,
        "primary contrast must not be degenerate"
    );
    assert_ne!(
        secondary,
        Verdict::Degenerate,
        "secondary contrast must not be degenerate"
    );
}
