//! Offline mechanism analyzer — RT-005 physics pilot, brick 5/5 (mika#1891).
//!
//! Reads the per-turn `turn_usage` log stream emitted by mika#1889 and answers
//! **one** pre-registered question about the 2x2 batch:
//!
//! > Does the sign of the reliability effect on planning tokens flip between
//! > the high-confidence and the low-confidence arm?
//!
//! Nothing here runs any part of the protocol. There is no network, no
//! subprocess, no filesystem access, no clock — [`analyze`] takes bytes the
//! caller already holds and returns a value. Per the [`crate::research`]
//! doctrine this is disposable apparatus: not a tool, not wired into the agent
//! loop, deletable once RT-005 has reported.
//!
//! # The pre-registered estimand
//!
//! **Planning tokens of a run = the sum of `output_tokens` over that run's
//! turns where `tool_use_in_turn == false`.**
//!
//! One definition, fixed before the data exists, with no runtime switch and no
//! alternate kept "for comparison" — a selectable boundary is a garden fork.
//! Each of its three parts is load-bearing:
//!
//! - **`output_tokens`, never `input_tokens`.** Output tokens are what the
//!   agent *produced* — deliberation it authored. Input tokens are context
//!   handed to it, dominated by system prompt and history, and would mostly
//!   measure prompt size. This also *dissolves* the cross-provider asymmetry
//!   recorded in
//!   `docs/solutions/best-practices/cross-provider-input-tokens-cache-inclusion-asymmetry-2026-08-20.md`:
//!   the Anthropic-vs-OpenAI-compat disagreement is entirely about what
//!   `input_tokens` includes, and this estimand never reads that field. No
//!   provider normalization is needed and none is implemented — reintroducing
//!   an `input_tokens` term would silently reintroduce that miscount.
//! - **`tool_use_in_turn == false`.** The one mechanical discriminator
//!   mika#1889 D3 emitted for exactly this purpose. A turn that calls no tool
//!   is deliberation without action — the clean non-verification component the
//!   primary outcome was defined as. A turn that calls tools is the *act*
//!   (consulting `peer_b`, checking its answer) and is excluded.
//! - **Continuation turns count.** The `step == u32::MAX` continuation turn
//!   (mika#1889 R4) is a text-only summarization turn, planning-class by
//!   construction, and always carries `tool_use_in_turn == false` — so the rule
//!   above already includes it. No special case; a test pins the behaviour.
//!
//! Error and timeout turns carry zero tokens by construction, so they
//! contribute nothing. Declining to add an exclusion rule for them keeps the
//! definition free of one more researcher degree of freedom.
//!
//! # Covariates are description, not outcome
//!
//! [`Covariates`] counts turns, handshakes and recalculations. These are
//! **unregistered mechanical proxies**, chosen because they are computable from
//! the raw log, and they carry **no authority over the estimand**. They are
//! partly definitional — a turn is a turn because the loop says so — which is
//! precisely why RT-005 refused them as outcomes.
//!
//! The separation is enforced by the compiler, not by convention (Prime hard
//! guardrail 2). [`estimand::PlanningTokens`] has a private field, lives in its
//! own module, and its only constructor applies the definition above.
//! [`Covariates`] exposes no path to it and no arithmetic bridges the two. There
//! is no expression a contributor can write that slides a covariate into the
//! primary estimand — doing it requires editing
//! [`estimand::PlanningTokens::from_turns`], which is a visible act, not an
//! accident.
//!
//! # Existence, not magnitude
//!
//! The reliability knob is synthetic and the confidence knob is an injected
//! prior. Any effect size read off this pilot describes the apparatus, not the
//! world. [`Report::render`] therefore leads with that disclaimer, states a
//! [`Verdict`] built from signs, and prints the raw contrast only under an
//! explicit reproducibility-diagnostics label.
//!
//! # Input contract (consumed by mika#1890, brick 3/5)
//!
//! Two inputs:
//!
//! 1. The `turn_usage` JSON-lines stream, exactly as `mika_common::logging`
//!    writes it under `MIKA_LOG_FORMAT=json` (`fmt::layer().json()
//!    .flatten_event(true)` — one object per line, tracing fields at the root).
//!    Capturing the batch under `pretty` yields no JSON and nothing to parse.
//! 2. A **run manifest** mapping each run's `session_id` to its reliability arm.
//!    Reliability leaves no trace in the log — `peer_b`'s knob is invisible to
//!    the agent loop — so brick 3/5 must record it.
//!
//! Confidence is deliberately **not** in the manifest: it is recovered from
//! `agent_id` (mika#1888 made `--agent` the confidence selector). Each factor is
//! read from its own authority, so a manifest that disagrees with the log cannot
//! silently mis-assign a run.

use std::collections::HashMap;

use serde::Deserialize;

/// The injected-confidence arm, recovered from the run's `agent_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    /// `mika-dev-confidence-high` — injected prior 0.95 on peer_b's reliability.
    High,
    /// `mika-dev-confidence-low` — injected prior 0.55.
    Low,
}

impl Confidence {
    /// Recover the arm from an agent id. Anything else is not an RT-005 run and
    /// its events are dropped rather than guessed into a cell.
    pub fn from_agent_id(agent_id: &str) -> Option<Self> {
        match agent_id {
            "mika-dev-confidence-high" => Some(Self::High),
            "mika-dev-confidence-low" => Some(Self::Low),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::High => "high (0.95)",
            Self::Low => "low  (0.55)",
        }
    }

    /// Bare arm name, for the covariate table's row labels.
    fn short_label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Low => "low",
        }
    }
}

/// The real-reliability arm of `peer_b`. Supplied by the run manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reliability {
    /// `peer_b` answers every item correctly.
    Fiable,
    /// `peer_b` answers a seeded subset with another item's answer.
    Degradee,
}

impl Reliability {
    fn label(self) -> &'static str {
        match self {
            Self::Fiable => "fiable",
            Self::Degradee => "degradee",
        }
    }
}

/// One cell of the 2x2 design.
type Cell = (Confidence, Reliability);

const CELLS: [Cell; 4] = [
    (Confidence::High, Reliability::Fiable),
    (Confidence::High, Reliability::Degradee),
    (Confidence::Low, Reliability::Fiable),
    (Confidence::Low, Reliability::Degradee),
];

/// The subset of a mika#1889 `turn_usage` event this analyzer reads.
///
/// Deserialized from the flattened JSON event; every other field the emitter
/// writes (`trace_id`, `mode`, `provider`, `model`, `stop_reason`,
/// `input_tokens`, cache counters, `latency_ms`, `status`) is ignored here.
/// `input_tokens` is absent by design, not by oversight — see the module docs.
#[derive(Debug, Clone, Deserialize)]
pub struct TurnUsage {
    /// Identifies the confidence arm (mika#1888).
    pub agent_id: String,
    /// One RT-005 run.
    pub session_id: String,
    /// 0-indexed turn; `u32::MAX` marks a continuation turn (mika#1889 R4).
    /// Read for traceability only — the continuation turn is included in the
    /// estimand through the `tool_use_in_turn` rule, not through this field.
    #[serde(default)]
    pub step: u32,
    /// Tokens the model produced this turn. The estimand's only token input.
    #[serde(default)]
    pub output_tokens: u64,
    /// Whether the model requested tool calls this turn (mika#1889 D3).
    #[serde(default)]
    pub tool_use_in_turn: bool,
}

/// Parse a `turn_usage` JSON-lines stream, skipping everything else.
///
/// An operator's capture interleaves `system_prompt_assembled` events, other
/// targets, and plain-text noise; failing on those would make the analyzer
/// unusable against real log files, so non-JSON lines and non-`turn_usage`
/// events are skipped silently.
pub fn parse_turn_usage_lines(log: &str) -> Vec<TurnUsage> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("event").and_then(serde_json::Value::as_str) == Some("turn_usage"))
        .filter_map(|v| serde_json::from_value::<TurnUsage>(v).ok())
        .collect()
}

/// The sealed primary estimand.
///
/// Everything in this module exists to make one mistake impossible: putting a
/// descriptive covariate into the primary outcome. [`PlanningTokens`] has a
/// private field and one constructor, and [`CellMean::of`] accepts nothing else,
/// so the mistake does not compile rather than being caught in review.
mod estimand {
    use super::TurnUsage;

    /// Planning tokens for one run. Constructible **only** via
    /// [`PlanningTokens::from_turns`], which is where the pre-registered
    /// definition lives.
    ///
    /// No `From<u64>`, no `From<Covariates>`, no arithmetic with anything else:
    /// the private field is the barrier.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PlanningTokens(u64);

    impl PlanningTokens {
        /// **The pre-registered definition** (module docs): sum `output_tokens`
        /// over turns that called no tool. The single point of truth — changing
        /// the estimand means changing this function.
        pub fn from_turns(turns: &[&TurnUsage]) -> Self {
            Self(
                turns
                    .iter()
                    .filter(|t| !t.tool_use_in_turn)
                    .map(|t| t.output_tokens)
                    .sum(),
            )
        }
    }

    /// Mean planning tokens over the runs in one cell; `None` when the cell is
    /// empty. Accepts `PlanningTokens` and nothing else.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct CellMean(Option<f64>);

    impl CellMean {
        pub fn of(runs: &[PlanningTokens]) -> Self {
            if runs.is_empty() {
                return Self(None);
            }
            let total: f64 = runs.iter().map(|p| p.0 as f64).sum();
            Self(Some(total / runs.len() as f64))
        }

        pub fn value(self) -> Option<f64> {
            self.0
        }
    }
}

use estimand::{CellMean, PlanningTokens};

/// What the analysis claims. Signs only — never a magnitude (guardrail 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The two simple effects have strictly opposite signs: the interaction
    /// exists in the only sense this pilot can support.
    SignFlip,
    /// Both arms move the same way; no flip.
    SameDirection,
    /// A cell is empty, or a simple effect is exactly zero — there is no
    /// direction to compare, so no claim is made.
    Degenerate,
}

/// The single primary result. One interaction, one metric (guardrail 1).
#[derive(Debug, Clone, Copy)]
pub struct Interaction {
    high: CellMean,
    high_degraded: CellMean,
    low: CellMean,
    low_degraded: CellMean,
    /// `mean(fiable) - mean(degradee)` within the high-confidence arm.
    simple_effect_high: Option<f64>,
    /// The same contrast within the low-confidence arm.
    simple_effect_low: Option<f64>,
    verdict: Verdict,
}

impl Interaction {
    fn compute(means: &HashMap<Cell, CellMean>) -> Self {
        let get = |c: Cell| means.get(&c).copied().unwrap_or_default();
        let (high, high_degraded) = (
            get((Confidence::High, Reliability::Fiable)),
            get((Confidence::High, Reliability::Degradee)),
        );
        let (low, low_degraded) = (
            get((Confidence::Low, Reliability::Fiable)),
            get((Confidence::Low, Reliability::Degradee)),
        );
        let simple = |a: CellMean, b: CellMean| Some(a.value()? - b.value()?);
        let simple_effect_high = simple(high, high_degraded);
        let simple_effect_low = simple(low, low_degraded);

        let verdict = match (simple_effect_high, simple_effect_low) {
            (Some(h), Some(l)) if h != 0.0 && l != 0.0 => {
                if h.is_sign_positive() == l.is_sign_positive() {
                    Verdict::SameDirection
                } else {
                    Verdict::SignFlip
                }
            }
            _ => Verdict::Degenerate,
        };

        Self {
            high,
            high_degraded,
            low,
            low_degraded,
            simple_effect_high,
            simple_effect_low,
            verdict,
        }
    }

    /// The claim (guardrail 3).
    pub fn verdict(self) -> Verdict {
        self.verdict
    }

    /// `simple_effect_high - simple_effect_low`.
    ///
    /// **Not an effect size.** Exposed so a reader can reproduce the arithmetic
    /// behind [`Interaction::verdict`]; the knob is synthetic and its
    /// calibration, not the phenomenon, sets this number's scale.
    pub fn raw_contrast(self) -> Option<f64> {
        Some(self.simple_effect_high? - self.simple_effect_low?)
    }
}

/// Descriptive covariates for one run. **Not** the outcome (guardrail 2).
///
/// Unregistered proxies, defined for mechanical computability:
/// - `turns` — events observed for the run.
/// - `handshakes` — turns that called tools.
/// - `recalculations` — tool-free turns directly following a tool-calling turn,
///   i.e. re-deliberation after a tool result.
///
/// No method here returns [`PlanningTokens`], and none ever should.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Covariates {
    pub turns: u32,
    pub handshakes: u32,
    pub recalculations: u32,
}

impl Covariates {
    fn from_turns(turns: &[&TurnUsage]) -> Self {
        let handshakes = turns.iter().filter(|t| t.tool_use_in_turn).count() as u32;
        let recalculations = turns
            .windows(2)
            .filter(|w| w[0].tool_use_in_turn && !w[1].tool_use_in_turn)
            .count() as u32;
        Self {
            turns: turns.len() as u32,
            handshakes,
            recalculations,
        }
    }
}

/// Per-cell covariate means, reported in their own section and nowhere else.
#[derive(Debug, Clone, Default)]
pub struct CovariateSummary {
    cells: HashMap<Cell, (f64, f64, f64, usize)>,
}

impl CovariateSummary {
    fn insert(&mut self, cell: Cell, runs: &[Covariates]) {
        if runs.is_empty() {
            return;
        }
        let n = runs.len();
        let mean =
            |f: fn(&Covariates) -> u32| runs.iter().map(|c| f(c) as f64).sum::<f64>() / n as f64;
        self.cells.insert(
            cell,
            (
                mean(|c| c.turns),
                mean(|c| c.handshakes),
                mean(|c| c.recalculations),
                n,
            ),
        );
    }
}

/// The analysis result: one primary estimand plus separately held covariates.
#[derive(Debug, Clone)]
pub struct Report {
    interaction: Interaction,
    covariates: CovariateSummary,
    runs_analyzed: usize,
}

/// Analyze a `turn_usage` log against a run manifest.
///
/// Runs whose `session_id` is absent from `manifest`, and events whose
/// `agent_id` is not an RT-005 confidence agent, are dropped — never guessed
/// into a cell.
pub fn analyze(log: &str, manifest: &HashMap<String, Reliability>) -> Report {
    let events = parse_turn_usage_lines(log);

    // Group by run, preserving log order within a run (`recalculations` reads
    // adjacency).
    let mut by_run: HashMap<&str, Vec<&TurnUsage>> = HashMap::new();
    for e in &events {
        by_run.entry(&e.session_id).or_default().push(e);
    }

    let mut planning: HashMap<Cell, Vec<PlanningTokens>> = HashMap::new();
    let mut covariates: HashMap<Cell, Vec<Covariates>> = HashMap::new();
    let mut runs_analyzed = 0;

    for (session, turns) in &by_run {
        let Some(confidence) = turns
            .first()
            .and_then(|t| Confidence::from_agent_id(&t.agent_id))
        else {
            continue;
        };
        let Some(&reliability) = manifest.get(*session) else {
            continue;
        };
        let cell = (confidence, reliability);
        planning
            .entry(cell)
            .or_default()
            .push(PlanningTokens::from_turns(turns));
        covariates
            .entry(cell)
            .or_default()
            .push(Covariates::from_turns(turns));
        runs_analyzed += 1;
    }

    let means: HashMap<Cell, CellMean> = planning
        .iter()
        .map(|(cell, runs)| (*cell, CellMean::of(runs)))
        .collect();
    let mut summary = CovariateSummary::default();
    for (cell, runs) in &covariates {
        summary.insert(*cell, runs);
    }

    Report {
        interaction: Interaction::compute(&means),
        covariates: summary,
        runs_analyzed,
    }
}

fn sign(v: Option<f64>) -> &'static str {
    match v {
        Some(x) if x > 0.0 => "+",
        Some(x) if x < 0.0 => "-",
        Some(_) => "0",
        None => "?",
    }
}

fn opt(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |x| format!("{x:.1}"))
}

impl Report {
    /// The single primary result.
    pub fn interaction(&self) -> Interaction {
        self.interaction
    }

    /// Number of runs that were assigned to a cell.
    pub fn runs_analyzed(&self) -> usize {
        self.runs_analyzed
    }

    /// Render the report. The bounded-external-validity disclaimer is the first
    /// content, before any number (guardrail 3), and the covariates live under
    /// their own heading, below the estimand (guardrail 2).
    pub fn render(&self) -> String {
        let i = &self.interaction;
        let verdict = match i.verdict {
            Verdict::SignFlip => "SIGN FLIP — the reliability effect reverses between arms",
            Verdict::SameDirection => "SAME DIRECTION — no sign flip",
            Verdict::Degenerate => "DEGENERATE — a cell is empty or an effect is exactly zero",
        };

        let mut out = String::new();
        out.push_str(
            "!! EXISTENCE CLAIM ONLY — THIS REPORT DOES NOT ESTIMATE A MAGNITUDE !!\n\
             The reliability knob is synthetic (seeded substitution at a fixed rate in\n\
             research::peer_b) and the confidence knob is an injected prior, not a belief\n\
             formed from evidence. External validity is bounded to this apparatus. The only\n\
             defensible claim is whether the reliability effect on planning tokens CHANGES\n\
             SIGN between the confidence arms — never how large that change is.\n\n\
             RT-005 mechanism analyzer — brick 5/5 (mika#1891)\n\n\
             ## Primary estimand — confidence × reliability interaction on planning tokens\n\n\
             Pre-registered metric: output_tokens summed over tool-free turns. Sole primary\n\
             outcome; no other metric enters this section.\n\n",
        );
        out.push_str(&format!("  verdict: {verdict}\n"));
        out.push_str(&format!(
            "  sign of reliability effect @ confidence={}: {}\n",
            Confidence::High.label(),
            sign(i.simple_effect_high)
        ));
        out.push_str(&format!(
            "  sign of reliability effect @ confidence={}: {}\n",
            Confidence::Low.label(),
            sign(i.simple_effect_low)
        ));
        out.push_str(&format!("  runs analyzed: {}\n\n", self.runs_analyzed));
        out.push_str("  reproducibility diagnostics — NOT an effect size:\n");
        out.push_str(&format!(
            "    mean planning tokens: high/fiable={} high/degradee={} low/fiable={} low/degradee={}\n",
            opt(i.high.value()),
            opt(i.high_degraded.value()),
            opt(i.low.value()),
            opt(i.low_degraded.value())
        ));
        out.push_str(&format!("    raw contrast: {}\n\n", opt(i.raw_contrast())));

        out.push_str(
            "## Descriptive covariates — NOT part of the estimand above\n\n\
             Unregistered mechanical proxies, reported for description only. They carry no\n\
             authority over the primary estimand and cannot enter it: PlanningTokens is a\n\
             sealed type whose only constructor applies the pre-registered definition.\n\n",
        );
        out.push_str("  cell                turns  handshakes  recalculations   n\n");
        for cell in CELLS {
            let name = format!("{}/{}", cell.0.short_label(), cell.1.label());
            match self.covariates.cells.get(&cell) {
                Some((t, h, r, n)) => out.push_str(&format!(
                    "  {name:<19} {t:>5.1} {h:>11.1} {r:>15.1} {n:>3}\n"
                )),
                None => out.push_str(&format!(
                    "  {name:<19} {:>5} {:>11} {:>15} {:>3}\n",
                    "n/a", "n/a", "n/a", 0
                )),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One event in the exact shape `mika_common::logging` writes under
    /// `MIKA_LOG_FORMAT=json` (`flatten_event(true)`): tracing fields at the
    /// root next to `timestamp`/`level`/`target`.
    fn event(session: &str, agent: &str, step: u32, output: u64, tool_use: bool) -> String {
        format!(
            r#"{{"timestamp":"2026-08-29T12:00:00.000000Z","level":"INFO","target":"mika::otel","event":"turn_usage","agent_id":"{agent}","session_id":"{session}","trace_id":"t","mode":"conversation","provider":"anthropic","model":"claude","step":{step},"stop_reason":"EndTurn","input_tokens":9999,"output_tokens":{output},"cache_read_tokens":0,"cache_write_tokens":0,"latency_ms":10,"tool_use_in_turn":{tool_use},"status":"success","message":"turn usage"}}"#
        )
    }

    const HIGH: &str = "mika-dev-confidence-high";
    const LOW: &str = "mika-dev-confidence-low";

    fn manifest(entries: &[(&str, Reliability)]) -> HashMap<String, Reliability> {
        entries
            .iter()
            .map(|(s, r)| ((*s).to_string(), *r))
            .collect()
    }

    #[test]
    fn parses_the_shipped_log_format_and_skips_everything_else() {
        let log = [
            r#"{"timestamp":"x","level":"INFO","target":"mika::otel","event":"system_prompt_assembled","total_bytes":42}"#.to_string(),
            "12:00:00  INFO not json at all".to_string(),
            event("s1", HIGH, 0, 100, false),
            String::new(),
        ]
        .join("\n");

        let parsed = parse_turn_usage_lines(&log);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].session_id, "s1");
        assert_eq!(parsed[0].output_tokens, 100);
        assert!(!parsed[0].tool_use_in_turn);
    }

    #[test]
    fn continuation_turn_counts_toward_planning_tokens() {
        // mika#1889 R4: step == u32::MAX summarizes, and that is planning-class.
        let log = event("s1", HIGH, u32::MAX, 250, false);
        let turns = parse_turn_usage_lines(&log);
        assert_eq!(turns[0].step, u32::MAX);

        let report = analyze(&log, &manifest(&[("s1", Reliability::Fiable)]));
        assert_eq!(report.runs_analyzed(), 1);
        assert_eq!(
            report.interaction().high.value(),
            Some(250.0),
            "continuation turn must not be dropped"
        );
    }

    #[test]
    fn tool_calling_turn_is_excluded_from_the_estimand_and_counted_as_a_handshake() {
        let log = [
            event("s1", HIGH, 0, 100, false),
            event("s1", HIGH, 1, 500, true),
        ]
        .join("\n");

        let report = analyze(&log, &manifest(&[("s1", Reliability::Fiable)]));
        assert_eq!(report.interaction().high.value(), Some(100.0));
        let (turns, handshakes, ..) =
            report.covariates.cells[&(Confidence::High, Reliability::Fiable)];
        assert_eq!(turns, 2.0);
        assert_eq!(handshakes, 1.0);
    }

    #[test]
    fn input_tokens_never_reach_the_estimand() {
        // Pins the provider-asymmetry immunity: the Anthropic/OpenAI-compat
        // disagreement is entirely about input_tokens, which the estimand
        // never reads.
        let baseline = event("s1", HIGH, 0, 100, false);
        let inflated = baseline.replace(r#""input_tokens":9999"#, r#""input_tokens":999999"#);
        assert_ne!(baseline, inflated);

        let m = manifest(&[("s1", Reliability::Fiable)]);
        assert_eq!(
            analyze(&baseline, &m).interaction().high.value(),
            analyze(&inflated, &m).interaction().high.value()
        );
    }

    /// Build a four-cell log, one run per cell, with the given planning totals.
    fn four_cells(hf: u64, hd: u64, lf: u64, ld: u64) -> (String, HashMap<String, Reliability>) {
        let log = [
            event("hf", HIGH, 0, hf, false),
            event("hd", HIGH, 0, hd, false),
            event("lf", LOW, 0, lf, false),
            event("ld", LOW, 0, ld, false),
        ]
        .join("\n");
        let m = manifest(&[
            ("hf", Reliability::Fiable),
            ("hd", Reliability::Degradee),
            ("lf", Reliability::Fiable),
            ("ld", Reliability::Degradee),
        ]);
        (log, m)
    }

    #[test]
    fn opposite_signed_simple_effects_are_a_sign_flip() {
        // high: 100 - 300 = -200 ; low: 400 - 100 = +300
        let (log, m) = four_cells(100, 300, 400, 100);
        let report = analyze(&log, &m);
        assert_eq!(report.interaction().verdict(), Verdict::SignFlip);
        assert_eq!(report.runs_analyzed(), 4);
    }

    #[test]
    fn same_signed_simple_effects_are_not_a_flip() {
        // high: 300 - 100 = +200 ; low: 400 - 100 = +300
        let (log, m) = four_cells(300, 100, 400, 100);
        assert_eq!(
            analyze(&log, &m).interaction().verdict(),
            Verdict::SameDirection
        );
    }

    #[test]
    fn an_empty_cell_is_degenerate_not_a_claim() {
        let log = [
            event("hf", HIGH, 0, 100, false),
            event("lf", LOW, 0, 400, false),
            event("ld", LOW, 0, 100, false),
        ]
        .join("\n");
        let m = manifest(&[
            ("hf", Reliability::Fiable),
            ("lf", Reliability::Fiable),
            ("ld", Reliability::Degradee),
        ]);
        let report = analyze(&log, &m);
        assert_eq!(report.interaction().verdict(), Verdict::Degenerate);
        assert_eq!(report.interaction().raw_contrast(), None);
    }

    #[test]
    fn a_zero_simple_effect_is_degenerate_not_a_direction() {
        let (log, m) = four_cells(200, 200, 400, 100);
        assert_eq!(
            analyze(&log, &m).interaction().verdict(),
            Verdict::Degenerate
        );
    }

    #[test]
    fn runs_missing_from_the_manifest_are_dropped_not_guessed() {
        let log = [
            event("known", HIGH, 0, 100, false),
            event("unknown", HIGH, 0, 900, false),
        ]
        .join("\n");
        let report = analyze(&log, &manifest(&[("known", Reliability::Fiable)]));
        assert_eq!(report.runs_analyzed(), 1);
        assert_eq!(report.interaction().high.value(), Some(100.0));
    }

    #[test]
    fn non_rt005_agents_are_dropped() {
        let log = event("s1", "mika-dev", 0, 100, false);
        let report = analyze(&log, &manifest(&[("s1", Reliability::Fiable)]));
        assert_eq!(report.runs_analyzed(), 0);
    }

    #[test]
    fn recalculations_count_tool_free_turns_following_a_tool_call() {
        let log = [
            event("s1", HIGH, 0, 10, false), // plan
            event("s1", HIGH, 1, 10, true),  // act
            event("s1", HIGH, 2, 10, false), // re-deliberate  <- 1
            event("s1", HIGH, 3, 10, true),  // act
            event("s1", HIGH, 4, 10, false), // re-deliberate  <- 2
        ]
        .join("\n");
        let report = analyze(&log, &manifest(&[("s1", Reliability::Fiable)]));
        let (turns, handshakes, recalcs, _) =
            report.covariates.cells[&(Confidence::High, Reliability::Fiable)];
        assert_eq!((turns, handshakes, recalcs), (5.0, 2.0, 2.0));
    }

    #[test]
    fn the_disclaimer_is_the_first_content_of_the_report() {
        let (log, m) = four_cells(100, 300, 400, 100);
        let rendered = analyze(&log, &m).render();
        let first = rendered.lines().next().unwrap();
        assert!(
            first.contains("EXISTENCE CLAIM ONLY")
                && first.contains("DOES NOT ESTIMATE A MAGNITUDE"),
            "guardrail 3: disclaimer leads the report, got {first:?}"
        );
        // ...and before any number the report prints.
        let disclaimer_end = rendered.find("## Primary estimand").unwrap();
        let verdict_pos = rendered.find("verdict:").unwrap();
        assert!(disclaimer_end < verdict_pos);
    }

    #[test]
    fn covariates_never_appear_in_the_primary_section() {
        let (log, m) = four_cells(100, 300, 400, 100);
        let rendered = analyze(&log, &m).render();
        let split = rendered.find("## Descriptive covariates").unwrap();
        let (primary, covariate_section) = rendered.split_at(split);

        assert!(
            !primary.contains("handshake"),
            "guardrail 2: covariate leaked into the estimand section"
        );
        assert!(!primary.contains("recalculation"));
        assert!(covariate_section.contains("handshakes"));
        assert!(covariate_section.contains("recalculations"));
    }
}
