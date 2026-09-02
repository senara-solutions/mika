//! `rt005_analyze` — RT-005 physics pilot, invocation surface (mika#2116).
//!
//! Loads a mika#1890 batch directory, runs the mika#1891 analyzer over it, and
//! prints the pre-registered report to stdout. Three library calls and a print:
//!
//! ```text
//! load_batch(dir) -> analyze(&runs) -> Report::render()
//! ```
//!
//! It exists so that anyone holding an RT-005 batch directory can reproduce the
//! pre-registered report with one command, without reconstructing a harness. On
//! 2026-08-31 the 80-run batch finished, the report was needed, and no command
//! existed to produce it — it was produced by a throwaway harness outside the
//! repo. That the throwaway worked first try proves the library is sound; what
//! was missing is access, not computation.
//!
//! # It computes nothing — and must not start
//!
//! This runner adds arithmetic to nothing. It does not sum, average, filter, or
//! reformat any value the report carries. The estimand — planning tokens summed
//! over tool-free turns — is defined in exactly one place,
//! [`mika_agent::research::mechanism_analyzer`], and any second definition here
//! is the fork RT-005 exists to forbid. A number computed in this file would be
//! a place that could silently disagree with the analyzer about what RT-005
//! measures.
//!
//! That rule is guarded by effect, not by intent: a frozen fixture
//! (`crates/mika-agent/tests/fixtures/rt005-analyze/`) pins the exact text
//! `render()` produces for a committed batch. Any arithmetic added anywhere on
//! this path changes the output and turns `tests/rt005_analyze.rs` red. This
//! docstring states the rule; the fixture enforces it.
//!
//! # Why an example and not a CLI subcommand
//!
//! Cargo auto-discovers `examples/*.rs`, so this file adds no `Cargo.toml`
//! entry, no runtime surface, no API surface, and no agent-loop wiring — and it
//! deletes with the rest of the RT-005 scaffold. `research/` describes itself as
//! disposable experiment apparatus; a subcommand would give a disposable
//! apparatus the appearance of a product feature. It does become a standing
//! `--all-targets` build and lint target for as long as the scaffold lives;
//! that is the accepted price of not reimplementing the analysis under pressure.
//! Precedent and form to match: `examples/rt005_batch_plan.rs` (brick 3/5).
//!
//! Usage: `cargo run -p mika-agent --example rt005_analyze -- <batch-dir>`

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use mika_agent::research::mechanism_analyzer::{analyze, load_batch};

/// The one positional argument: the batch directory to analyse.
///
/// Hand-rolled on purpose. One positional path needs no parser crate, and
/// adding one would be the first step of the CLI surface this ticket declines
/// to build.
fn parse_batch_dir(args: &[String]) -> Result<PathBuf> {
    match args {
        [dir] => Ok(PathBuf::from(dir)),
        [] => bail!("usage: rt005_analyze <batch-dir> (missing batch directory)"),
        _ => bail!(
            "usage: rt005_analyze <batch-dir> (expected one positional path, got {})",
            args.len()
        ),
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let batch_dir = parse_batch_dir(&args)?;

    let runs = load_batch(&batch_dir)
        .with_context(|| format!("loading RT-005 batch from {}", batch_dir.display()))?;

    // A loud stop, not an empty report. `analyze` over zero runs renders every
    // heading with `n/a` cells — indistinguishable from a report over broken
    // data. Refusing here is input validation, not computation: it reads no
    // value the report carries.
    if runs.is_empty() {
        bail!(
            "no runs loaded from {}: not an RT-005 batch directory, or its runs/ is empty",
            batch_dir.display()
        );
    }

    let report = analyze(&runs);
    // `render()` already terminates every line; `print!` keeps stdout
    // byte-identical to the frozen fixture.
    print!("{}", report.render());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_positional_path_parses() {
        let dir = parse_batch_dir(&["/tmp/rt005-batch".to_string()]).expect("parses");
        assert_eq!(dir, PathBuf::from("/tmp/rt005-batch"));
    }

    #[test]
    fn missing_path_is_an_error() {
        assert!(parse_batch_dir(&[]).is_err());
    }

    #[test]
    fn extra_arguments_are_rejected() {
        // No second knob exists by design — this runner takes exactly one path.
        assert!(parse_batch_dir(&["a".to_string(), "b".to_string()]).is_err());
    }
}
