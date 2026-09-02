//! Offline mechanism analyzer — RT-005 physics pilot, brick 5/5 (mika#1891).
//!
//! Reads a batch directory produced by mika#1890 (brick 3/5) and reports the
//! **two pre-registered contrasts** on one metric: planning tokens.
//!
//! Nothing here runs any part of the protocol. No network, no subprocess, no
//! clock. The only filesystem access is [`load_batch`], which reads files the
//! orchestrator already wrote; the analysis itself ([`analyze`]) is a pure
//! function of values the caller holds.
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
//!   above already includes it. No special case; a test pins the behaviour
//!   against a real production log line.
//!
//! Error and timeout turns carry zero tokens by construction, so they
//! contribute nothing. Declining to add an exclusion rule for them keeps the
//! definition free of one more researcher degree of freedom.
//!
//! # Two contrasts, both pre-registered, both always reported
//!
//! `peer_b` perturbs `n * 2 / 6` items — 3 of 10. A run's prompt carries only
//! the item and peer_b's answer, so on the other 7 items the prompt is
//! byte-identical across the reliability arms. Of the 20 runs in a degraded
//! cell, 6 carry a wrong peer answer and 14 are input-identical to their fiable
//! counterparts: **the design is balanced on the label, not on the
//! manipulation.**
//!
//! `research/rt005-physics-pilot/orchestration/PREREGISTRATION.md` (operator
//! decision, 2026-08-29, before any data existed) fixes both contrasts:
//!
//! - **Primary — the labelled arm.** Every successful run, by its assigned
//!   `reliability`. This protocol's intention-to-treat estimand. It is not
//!   changed after the fact.
//! - **Secondary, pre-specified — the realised perturbation.** The same
//!   estimand restricted to the items `peer_b` actually perturbed.
//!
//! > Reporting only one is a protocol violation, whichever one it is.
//!
//! That rule is enforced structurally, not by convention: [`Report`] exposes no
//! way to obtain one contrast without the other. [`Report::render`] emits both,
//! and [`Report::verdicts`] returns them as a pair. There is no `primary()`
//! accessor to call in isolation.
//!
//! The runs on *unperturbed* items are the pre-registration's within-design
//! controls. Their inputs are identical across arms, so their contrast should be
//! noise. It is rendered as a **diagnostic of the secondary contrast**, never as
//! a third claim — a non-null control contrast means the batch carries
//! variation the manipulation does not explain.
//!
//! Note that this remains **one metric and one interaction form**. The two
//! contrasts differ in which runs enter, not in what is measured or how — that
//! is what keeps this a pre-registered pair rather than a family of tests.
//!
//! # Covariates are description, not outcome
//!
//! [`Covariates`] counts turns, handshakes and recalculations. These are
//! **unregistered mechanical proxies**, chosen because they are computable from
//! the raw log, and they carry **no authority over either contrast**. They are
//! partly definitional — a turn is a turn because the loop says so — which is
//! precisely why RT-005 refused them as outcomes.
//!
//! The separation is enforced by the compiler, not by convention (Prime hard
//! guardrail 2). [`estimand::PlanningTokens`] has a private field, lives in its
//! own module, and its only constructor applies the definition above.
//! [`Covariates`] exposes no path to it and no arithmetic bridges the two. There
//! is no expression a contributor can write that slides a covariate into an
//! estimand — doing it requires editing
//! [`estimand::PlanningTokens::from_turns`], which is a visible act, not an
//! accident.
//!
//! # Existence, not magnitude
//!
//! The reliability knob is synthetic and the confidence knob is an injected
//! prior. Any effect size read off this pilot describes the apparatus, not the
//! world. [`Report::render`] therefore leads with that disclaimer, states
//! [`Verdict`]s built from signs, and prints raw contrasts only under an
//! explicit reproducibility-diagnostics label.
//!
//! # Input contract (produced by mika#1890, brick 3/5)
//!
//! A batch directory:
//!
//! ```text
//! runs/<run_id>.json              one run record — status, arms, item, perturbed
//! logs/<run_id>.turn_usage.jsonl  that run's raw turn_usage lines, verbatim
//! ```
//!
//! **The measurement channel is mika-spirit's log, not a per-agent CLI log.**
//! Since mika#1727 `mika ask` is an A2A client: spirit owns the execution
//! session and emits `turn_usage` under its own session id, so the per-agent
//! `~/.mika/agents/<name>/logs/` files carry no `turn_usage` from an `ask` at
//! all. `mika/CLAUDE.md` Signal O still describes the pre-#1727 topology. The
//! orchestrator slices spirit's log per run; this module never goes looking for
//! the lines itself.
//!
//! Only runs with `status == "success"` are observations. `contaminated` means
//! the slice carried more than one spirit session for that agent, so its lines
//! are not attributable to the run; `failed` and `in_flight` never produced a
//! usable capture. All three are counted and reported as excluded, never
//! analysed.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// The injected-confidence arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    /// Injected prior 0.95 on peer_b's reliability (`mika-dev-confidence-high`).
    High,
    /// Injected prior 0.55 (`mika-dev-confidence-low`).
    Low,
}

impl Confidence {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "high" => Some(Self::High),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    /// The agent id brick 2/5 realises this arm as. Used to cross-check the run
    /// record against the log it points at.
    fn agent_id(self) -> &'static str {
        match self {
            Self::High => "mika-dev-confidence-high",
            Self::Low => "mika-dev-confidence-low",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::High => "high (0.95)",
            Self::Low => "low  (0.55)",
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Low => "low",
        }
    }
}

/// The assigned reliability arm of `peer_b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reliability {
    /// `peer_b` answers every item correctly.
    Fiable,
    /// `peer_b` answers a seeded subset with another item's answer.
    Degradee,
}

impl Reliability {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "fiable" => Some(Self::Fiable),
            "degradee" => Some(Self::Degradee),
            _ => None,
        }
    }

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

/// The status brick 3/5 writes for a run. Only `success` is an observation.
const STATUS_SUCCESS: &str = "success";

/// A run record as mika#1890 writes it to `runs/<run_id>.json`.
///
/// Only the fields the analysis needs are deserialized; the record also carries
/// the prompt, peer_b's answer, the agent's raw output, stderr, the design
/// fingerprint and the disclaimer, none of which this module reads.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    /// `success` / `contaminated` / `failed` / `in_flight`.
    pub status: String,
    /// `high` / `low`.
    pub confidence: String,
    /// `fiable` / `degradee` — the **assigned** arm (primary contrast).
    pub reliability: String,
    pub item_id: String,
    /// Whether `peer_b` actually perturbed this item (secondary contrast).
    /// Always `false` in the fiable arm.
    pub perturbed: bool,
    /// The spirit session the captured lines must all belong to. Absent on
    /// records written before the paid call returned.
    #[serde(default)]
    pub spirit_session_id: Option<String>,
}

/// One run: its record plus the raw `turn_usage` lines sliced for it.
#[derive(Debug, Clone)]
pub struct Run {
    pub record: RunRecord,
    /// Verbatim contents of `logs/<run_id>.turn_usage.jsonl`.
    pub turn_usage: String,
}

/// The subset of a mika#1889 `turn_usage` event this analyzer reads.
///
/// Deserialized from the flattened JSON event; every other field the emitter
/// writes (`trace_id`, `mode`, `provider`, `model`, `stop_reason`,
/// `input_tokens`, cache counters, `latency_ms`, `status`, and the `span` /
/// `spans` objects) is ignored here. `input_tokens` is absent by design, not by
/// oversight — see the module docs.
#[derive(Debug, Clone, Deserialize)]
pub struct TurnUsage {
    /// Identifies the confidence arm (mika#1888). Cross-checked against the run
    /// record so a mis-sliced capture cannot be analysed as if it were the run.
    pub agent_id: String,
    /// The spirit-side session id.
    pub session_id: String,
    /// 0-indexed turn; `u32::MAX` marks a continuation turn (mika#1889 R4).
    /// Ordering key for the covariates — see `analyze`.
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
/// A capture may interleave other `mika::otel` events and non-JSON noise;
/// failing on those would make the analyzer unusable against real log slices,
/// so non-JSON lines and non-`turn_usage` events are skipped silently.
pub fn parse_turn_usage_lines(log: &str) -> Vec<TurnUsage> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("event").and_then(serde_json::Value::as_str) == Some("turn_usage"))
        .filter_map(|v| serde_json::from_value::<TurnUsage>(v).ok())
        .collect()
}

/// Read a mika#1890 batch directory into [`Run`]s.
///
/// The only filesystem access in this module, and read-only: it opens files the
/// orchestrator already wrote. Records that do not parse, and records whose
/// companion capture is missing, are skipped — a half-written batch must not
/// stop the analysis of the runs that did complete.
pub fn load_batch(batch_dir: &Path) -> Result<Vec<Run>> {
    let runs_dir = batch_dir.join("runs");
    let entries = std::fs::read_dir(&runs_dir)
        .with_context(|| format!("reading run records from {}", runs_dir.display()))?;

    let mut runs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<RunRecord>(&text) else {
            continue;
        };
        let log_path = batch_dir
            .join("logs")
            .join(format!("{}.turn_usage.jsonl", record.run_id));
        let turn_usage = std::fs::read_to_string(&log_path).unwrap_or_default();
        runs.push(Run { record, turn_usage });
    }
    runs.sort_by(|a, b| a.record.run_id.cmp(&b.record.run_id));
    Ok(runs)
}

/// The sealed primary estimand.
///
/// Everything in this module exists to make one mistake impossible: putting a
/// descriptive covariate into an estimand. [`PlanningTokens`] has a private
/// field and one constructor, and [`CellMean::of`] accepts nothing else, so the
/// mistake does not compile rather than being caught in review.
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

/// What a contrast claims. Signs only — never a magnitude (guardrail 3).
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

/// One interaction contrast. One metric, one interaction form (guardrail 1) —
/// what differs between the primary and the secondary is which runs enter.
#[derive(Debug, Clone, Copy)]
struct Interaction {
    high: CellMean,
    high_degraded: CellMean,
    low: CellMean,
    low_degraded: CellMean,
    /// `mean(fiable) - mean(degradee)` within the high-confidence arm.
    simple_effect_high: Option<f64>,
    /// The same contrast within the low-confidence arm.
    simple_effect_low: Option<f64>,
    verdict: Verdict,
    runs: usize,
}

impl Interaction {
    fn of(cells: &HashMap<Cell, Vec<PlanningTokens>>) -> Self {
        let runs = cells.values().map(Vec::len).sum();
        let mean = |c: Cell| {
            cells
                .get(&c)
                .map_or_else(CellMean::default, |r| CellMean::of(r))
        };
        let (high, high_degraded) = (
            mean((Confidence::High, Reliability::Fiable)),
            mean((Confidence::High, Reliability::Degradee)),
        );
        let (low, low_degraded) = (
            mean((Confidence::Low, Reliability::Fiable)),
            mean((Confidence::Low, Reliability::Degradee)),
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
            runs,
        }
    }

    /// `simple_effect_high - simple_effect_low`.
    ///
    /// **Not an effect size.** Reachable so a reader can reproduce the
    /// arithmetic behind the verdict; the knob is synthetic and its
    /// calibration, not the phenomenon, sets this number's scale.
    fn raw_contrast(self) -> Option<f64> {
        Some(self.simple_effect_high? - self.simple_effect_low?)
    }
}

/// Descriptive covariates for one run. **Not** an outcome (guardrail 2).
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
struct CovariateSummary {
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

/// Why a run was not analysed.
#[derive(Debug, Clone, Copy, Default)]
struct Excluded {
    /// `status` was not `success` — includes `contaminated`.
    non_success: usize,
    /// The record's arms did not parse, or the capture was empty.
    unusable: usize,
    /// The captured lines did not all belong to this run's agent and session.
    unattributable: usize,
}

/// The analysis result. Carries **both** pre-registered contrasts; there is no
/// way to obtain one without the other.
#[derive(Debug, Clone)]
pub struct Report {
    primary: Interaction,
    secondary: Interaction,
    control: Interaction,
    covariates: CovariateSummary,
    excluded: Excluded,
    perturbed_items: usize,
}

/// Analyze a batch.
///
/// Runs that are not `status == "success"` are excluded and counted, never
/// analysed. A run whose captured lines do not all carry its own agent id and
/// spirit session id is not attributable and is likewise excluded — the
/// orchestrator already marks that case `contaminated`, and re-checking here is
/// cheap defence-in-depth against a mis-sliced capture.
pub fn analyze(runs: &[Run]) -> Report {
    // The realised perturbation, read off the records rather than re-derived:
    // `perturbed` is a design fact peer_b reported, not a computed statistic.
    let perturbed_items: HashSet<&str> = runs
        .iter()
        .filter(|r| r.record.perturbed)
        .map(|r| r.record.item_id.as_str())
        .collect();

    let mut excluded = Excluded::default();
    let mut all: HashMap<Cell, Vec<PlanningTokens>> = HashMap::new();
    let mut manipulated: HashMap<Cell, Vec<PlanningTokens>> = HashMap::new();
    let mut controls: HashMap<Cell, Vec<PlanningTokens>> = HashMap::new();
    let mut covariates: HashMap<Cell, Vec<Covariates>> = HashMap::new();

    for run in runs {
        if run.record.status != STATUS_SUCCESS {
            excluded.non_success += 1;
            continue;
        }
        let (Some(confidence), Some(reliability)) = (
            Confidence::parse(&run.record.confidence),
            Reliability::parse(&run.record.reliability),
        ) else {
            excluded.unusable += 1;
            continue;
        };

        let mut turns: Vec<&TurnUsage> = Vec::new();
        let parsed = parse_turn_usage_lines(&run.turn_usage);
        turns.extend(parsed.iter());
        if turns.is_empty() {
            excluded.unusable += 1;
            continue;
        }

        // Attribution: every captured line must be this arm's agent, and — when
        // the record names the spirit session — that same session. Anything
        // else means the slice caught someone else's turns.
        let expected_agent = confidence.agent_id();
        let session_ok = |t: &TurnUsage| match &run.record.spirit_session_id {
            Some(id) if !id.is_empty() => &t.session_id == id,
            _ => true,
        };
        if turns
            .iter()
            .any(|t| t.agent_id != expected_agent || !session_ok(t))
        {
            excluded.unattributable += 1;
            continue;
        }

        // Order by `step`, the emitter's own turn index, rather than by position
        // in the file: nothing guarantees a multi-threaded writer appends a
        // session's lines in turn order, and `recalculations` reads adjacency.
        // The `u32::MAX` continuation sentinel sorts last, which is where the
        // continuation turn belongs. The estimand is a sum and does not depend
        // on order — only the covariate does.
        turns.sort_by_key(|t| t.step);

        let cell = (confidence, reliability);
        let planning = PlanningTokens::from_turns(&turns);
        all.entry(cell).or_default().push(planning);
        if perturbed_items.contains(run.record.item_id.as_str()) {
            manipulated.entry(cell).or_default().push(planning);
        } else {
            controls.entry(cell).or_default().push(planning);
        }
        covariates
            .entry(cell)
            .or_default()
            .push(Covariates::from_turns(&turns));
    }

    let mut summary = CovariateSummary::default();
    for (cell, runs) in &covariates {
        summary.insert(*cell, runs);
    }

    Report {
        primary: Interaction::of(&all),
        secondary: Interaction::of(&manipulated),
        control: Interaction::of(&controls),
        covariates: summary,
        excluded,
        perturbed_items: perturbed_items.len(),
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

fn verdict_line(v: Verdict) -> &'static str {
    match v {
        Verdict::SignFlip => "SIGN FLIP — the reliability effect reverses between arms",
        Verdict::SameDirection => "SAME DIRECTION — no sign flip",
        Verdict::Degenerate => "DEGENERATE — a cell is empty or an effect is exactly zero",
    }
}

impl Report {
    /// Both verdicts, as a pair: primary first, pre-specified secondary second.
    ///
    /// Returned together on purpose. The pre-registration's reporting rule —
    /// "reporting only one is a protocol violation, whichever one it is" — is
    /// enforced by there being no accessor that yields one alone.
    pub fn verdicts(&self) -> (Verdict, Verdict) {
        (self.primary.verdict, self.secondary.verdict)
    }

    /// Number of runs entering the primary contrast.
    pub fn runs_analyzed(&self) -> usize {
        self.primary.runs
    }

    fn render_contrast(&self, out: &mut String, i: &Interaction) {
        out.push_str(&format!("  verdict: {}\n", verdict_line(i.verdict)));
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
        out.push_str(&format!("  runs entering this contrast: {}\n\n", i.runs));
        out.push_str("  reproducibility diagnostics — NOT an effect size:\n");
        out.push_str(&format!(
            "    mean planning tokens: high/fiable={} high/degradee={} low/fiable={} low/degradee={}\n",
            opt(i.high.value()),
            opt(i.high_degraded.value()),
            opt(i.low.value()),
            opt(i.low_degraded.value())
        ));
        out.push_str(&format!("    raw contrast: {}\n\n", opt(i.raw_contrast())));
    }

    /// Render the report. The bounded-external-validity disclaimer is the first
    /// content, before any number (guardrail 3); both pre-registered contrasts
    /// are emitted, primary first (pre-registration reporting rule); and the
    /// covariates live under their own heading, below both (guardrail 2).
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(
            "!! EXISTENCE CLAIM ONLY — THIS REPORT DOES NOT ESTIMATE A MAGNITUDE !!\n\
             The reliability knob is synthetic (seeded substitution at a fixed rate in\n\
             research::peer_b, on a fixture of deliberately easy items) and the confidence\n\
             knob is an injected prior, not a belief formed from evidence. External validity\n\
             is bounded by construction. The only defensible claim is whether the reliability\n\
             effect on planning tokens CHANGES SIGN between the confidence arms — never how\n\
             large that change is.\n\n\
             RT-005 mechanism analyzer — brick 5/5 (mika#1891)\n\
             Metric, pre-registered: output_tokens summed over tool-free turns. One metric\n\
             for both contrasts below; they differ only in which runs enter.\n\n\
             ## PRIMARY contrast — the labelled arm (intention-to-treat)\n\n\
             Every successful run, by its assigned reliability. This is the estimand the 2x2\n\
             randomisation licenses, and it is not changed after the fact.\n\n",
        );
        let primary = self.primary;
        self.render_contrast(&mut out, &primary);

        out.push_str(
            "## SECONDARY contrast, pre-specified — the realised perturbation\n\n\
             peer_b perturbs 3 of 10 items, and a run's prompt carries only its item and\n\
             peer_b's answer — so the arms are byte-identical on the other 7. The design is\n\
             balanced on the label, not on the manipulation. This contrast restricts the same\n\
             estimand to the items actually perturbed. Both contrasts are reported because\n\
             PREREGISTRATION.md (operator, 2026-08-29, before data) fixed both: reporting only\n\
             one is a protocol violation, whichever one it is.\n\n",
        );
        out.push_str(&format!(
            "  perturbed items in this batch: {}\n",
            self.perturbed_items
        ));
        let secondary = self.secondary;
        self.render_contrast(&mut out, &secondary);

        out.push_str(&format!(
            "  within-design control (unperturbed items — inputs identical across arms,\n  \
             so this contrast should be noise; a non-null one means variation the\n  \
             manipulation does not explain). NOT a third claim:\n    \
             control verdict: {}\n    control raw contrast: {} over {} runs\n\n",
            verdict_line(self.control.verdict),
            opt(self.control.raw_contrast()),
            self.control.runs
        ));

        out.push_str(&format!(
            "## Runs excluded — not observations\n\n  \
             status not success (incl. contaminated): {}\n  \
             unusable record or empty capture:        {}\n  \
             capture not attributable to the run:     {}\n\n",
            self.excluded.non_success, self.excluded.unusable, self.excluded.unattributable
        ));

        out.push_str(
            "## Descriptive covariates — NOT part of either contrast above\n\n\
             Unregistered mechanical proxies, reported for description only. They carry no\n\
             authority over either estimand and cannot enter one: PlanningTokens is a sealed\n\
             type whose only constructor applies the pre-registered definition.\n\n",
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

    // ---------------------------------------------------------------------
    // Real production log lines, copied verbatim from /var/log/mika/server.log
    // on 2026-08-30. Not hand-authored: a fixture written from a reading of the
    // emitter can only confirm that reading. These prove the parser reads what
    // mika-spirit actually writes — including the `span`/`spans` objects and
    // the `message` field that the hand-written shape omitted.
    // ---------------------------------------------------------------------

    /// EndTurn, no tool call — a planning turn under the pre-registered rule.
    const REAL_ENDTURN: &str = r#"{"timestamp":"2026-08-21T07:39:00.578957Z","level":"INFO","message":"turn usage","event":"turn_usage","agent_id":"mika-dev","session_id":"b7446903-62ad-4d68-af7f-27dcf7e66b3b","trace_id":"1169ad60-9d32-11f1-98c7-5b1881cbd461","mode":"agent","provider":"openrouter","model":"z-ai/glm-5.2","step":0,"stop_reason":"EndTurn","input_tokens":47538,"output_tokens":26,"cache_read_tokens":256,"cache_write_tokens":0,"latency_ms":5462,"tool_use_in_turn":false,"status":"success","target":"mika::otel","span":{"agent":"mika-dev","channel":"github","mode":"conversation","trace_id":"1169ad60-9d32-11f1-98c7-5b1881cbd461","name":"agent_turn"},"spans":[{"agent":"mika-dev","channel":"github","mode":"conversation","trace_id":"1169ad60-9d32-11f1-98c7-5b1881cbd461","name":"agent_turn"}]}"#;

    /// ToolUse — an acting turn, excluded from the estimand.
    const REAL_TOOLUSE: &str = r#"{"timestamp":"2026-08-21T08:00:05.992598Z","level":"INFO","message":"turn usage","event":"turn_usage","agent_id":"chase-hughes","session_id":"heartbeat-f0b6fe56-7936-4c00-8301-c5f9f21eb127","trace_id":"e7a8fe5e039745f1ad223a2852517519","mode":"silent agent","provider":"openrouter","model":"z-ai/glm-5.2","step":0,"stop_reason":"ToolUse","input_tokens":13056,"output_tokens":335,"cache_read_tokens":0,"cache_write_tokens":0,"latency_ms":5651,"tool_use_in_turn":true,"status":"success","target":"mika::otel","span":{"agent":"chase-hughes","mode":"silent","trigger":"heartbeat","name":"agent_turn"},"spans":[{"agent":"chase-hughes","mode":"silent","trigger":"heartbeat","name":"agent_turn"}]}"#;

    /// The `u32::MAX` continuation sentinel, as production actually emits it.
    const REAL_CONTINUATION: &str = r#"{"timestamp":"2026-08-21T20:47:37.753221Z","level":"INFO","message":"turn usage","event":"turn_usage","agent_id":"mika-qa","session_id":"babdb2f5-ac49-457b-8dfd-abb18521b0f8","trace_id":"1bd95740-9da1-11f1-9685-d69404aaf21f","mode":"agent","provider":"openrouter","model":"z-ai/glm-5.2","step":4294967295,"stop_reason":"EndTurn","input_tokens":51662,"output_tokens":1377,"cache_read_tokens":0,"cache_write_tokens":0,"latency_ms":18769,"tool_use_in_turn":false,"status":"success","target":"mika::otel","span":{"agent":"mika-qa","channel":"github","mode":"conversation","trace_id":"1bd95740-9da1-11f1-9685-d69404aaf21f","name":"agent_turn"},"spans":[{"agent":"mika-qa","channel":"github","mode":"conversation","trace_id":"1bd95740-9da1-11f1-9685-d69404aaf21f","name":"agent_turn"}]}"#;

    const SESSION: &str = "s-spirit-0001";

    /// One `turn_usage` line in the shape mika-spirit writes, with the fields
    /// this analyzer keys on set for the test. Field order and the surrounding
    /// keys mirror `REAL_ENDTURN` above.
    fn event(agent: &str, step: u32, output: u64, tool_use: bool) -> String {
        format!(
            r#"{{"timestamp":"2026-08-30T00:00:00.000000Z","level":"INFO","message":"turn usage","event":"turn_usage","agent_id":"{agent}","session_id":"{SESSION}","trace_id":"t","mode":"agent","provider":"anthropic","model":"claude","step":{step},"stop_reason":"EndTurn","input_tokens":9999,"output_tokens":{output},"cache_read_tokens":0,"cache_write_tokens":0,"latency_ms":10,"tool_use_in_turn":{tool_use},"status":"success","target":"mika::otel","span":{{"name":"agent_turn"}},"spans":[{{"name":"agent_turn"}}]}}"#
        )
    }

    /// A successful run record + its capture, as brick 3/5 writes them.
    fn run(
        item: &str,
        confidence: Confidence,
        reliability: Reliability,
        perturbed: bool,
        turn_usage: String,
    ) -> Run {
        Run {
            record: RunRecord {
                run_id: format!(
                    "{}.{}.{item}.r1",
                    confidence.short_label(),
                    reliability.label()
                ),
                status: STATUS_SUCCESS.to_string(),
                confidence: confidence.short_label().to_string(),
                reliability: reliability.label().to_string(),
                item_id: item.to_string(),
                perturbed,
                spirit_session_id: Some(SESSION.to_string()),
            },
            turn_usage,
        }
    }

    /// A single-turn run producing `output` planning tokens.
    fn simple_run(
        item: &str,
        confidence: Confidence,
        reliability: Reliability,
        perturbed: bool,
        output: u64,
    ) -> Run {
        let log = event(confidence.agent_id(), 0, output, false);
        run(item, confidence, reliability, perturbed, log)
    }

    #[test]
    fn parses_real_production_log_lines() {
        // The load-bearing test: these three lines were emitted by mika-spirit,
        // not composed here. If the emitter's shape ever drifts, this fails.
        let log = [REAL_ENDTURN, REAL_TOOLUSE, REAL_CONTINUATION].join("\n");
        let parsed = parse_turn_usage_lines(&log);
        assert_eq!(parsed.len(), 3);

        assert_eq!(parsed[0].agent_id, "mika-dev");
        assert_eq!(parsed[0].output_tokens, 26);
        assert!(!parsed[0].tool_use_in_turn);
        assert_eq!(parsed[0].step, 0);

        assert!(parsed[1].tool_use_in_turn);
        assert_eq!(parsed[1].output_tokens, 335);

        assert_eq!(parsed[2].step, u32::MAX, "continuation sentinel");
        assert_eq!(parsed[2].output_tokens, 1377);
    }

    #[test]
    fn the_estimand_reads_real_lines_the_way_the_rule_says() {
        // Same three real lines, treated as one run: the two tool-free turns
        // count (26 + 1377), the ToolUse turn does not.
        let turns =
            parse_turn_usage_lines(&[REAL_ENDTURN, REAL_TOOLUSE, REAL_CONTINUATION].join("\n"));
        let refs: Vec<&TurnUsage> = turns.iter().collect();
        let report = analyze(&[Run {
            record: RunRecord {
                run_id: "r".into(),
                status: STATUS_SUCCESS.into(),
                confidence: "high".into(),
                reliability: "fiable".into(),
                item_id: "i1".into(),
                perturbed: false,
                spirit_session_id: None,
            },
            turn_usage: String::new(),
        }]);
        // The run above has an empty capture, so it is excluded, not analysed.
        assert_eq!(report.runs_analyzed(), 0);
        // The rule itself, applied to the real turns:
        assert_eq!(PlanningTokens::from_turns(&refs), {
            let only_toolfree: Vec<&TurnUsage> =
                turns.iter().filter(|t| !t.tool_use_in_turn).collect();
            PlanningTokens::from_turns(&only_toolfree)
        });
    }

    #[test]
    fn skips_non_turn_usage_events_and_non_json_noise() {
        let log = [
            r#"{"timestamp":"x","level":"INFO","target":"mika::otel","event":"system_prompt_assembled","total_bytes":42}"#,
            "12:00:00  INFO not json at all",
            REAL_ENDTURN,
            "",
        ]
        .join("\n");
        assert_eq!(parse_turn_usage_lines(&log).len(), 1);
    }

    #[test]
    fn continuation_turn_counts_toward_planning_tokens() {
        // mika#1889 R4: step == u32::MAX summarizes, and that is planning-class.
        let log = event(Confidence::High.agent_id(), u32::MAX, 250, false);
        let r = run("i1", Confidence::High, Reliability::Fiable, false, log);
        let report = analyze(&[r]);
        assert_eq!(report.runs_analyzed(), 1);
        assert_eq!(report.primary.high.value(), Some(250.0));
    }

    #[test]
    fn tool_calling_turn_is_excluded_and_counted_as_a_handshake() {
        let log = [
            event(Confidence::High.agent_id(), 0, 100, false),
            event(Confidence::High.agent_id(), 1, 500, true),
        ]
        .join("\n");
        let report = analyze(&[run("i1", Confidence::High, Reliability::Fiable, false, log)]);
        assert_eq!(report.primary.high.value(), Some(100.0));
        let (turns, handshakes, ..) =
            report.covariates.cells[&(Confidence::High, Reliability::Fiable)];
        assert_eq!((turns, handshakes), (2.0, 1.0));
    }

    #[test]
    fn input_tokens_never_reach_the_estimand() {
        // Pins the provider-asymmetry immunity: the Anthropic/OpenAI-compat
        // disagreement is entirely about input_tokens, which no estimand reads.
        let base = event(Confidence::High.agent_id(), 0, 100, false);
        let inflated = base.replace(r#""input_tokens":9999"#, r#""input_tokens":999999"#);
        assert_ne!(base, inflated);
        let a = analyze(&[run(
            "i1",
            Confidence::High,
            Reliability::Fiable,
            false,
            base,
        )]);
        let b = analyze(&[run(
            "i1",
            Confidence::High,
            Reliability::Fiable,
            false,
            inflated,
        )]);
        assert_eq!(a.primary.high.value(), b.primary.high.value());
    }

    /// A four-cell batch on one unperturbed item, with the given cell totals.
    fn four_cells(hf: u64, hd: u64, lf: u64, ld: u64) -> Vec<Run> {
        vec![
            simple_run("i1", Confidence::High, Reliability::Fiable, false, hf),
            simple_run("i1", Confidence::High, Reliability::Degradee, false, hd),
            simple_run("i1", Confidence::Low, Reliability::Fiable, false, lf),
            simple_run("i1", Confidence::Low, Reliability::Degradee, false, ld),
        ]
    }

    #[test]
    fn opposite_signed_simple_effects_are_a_sign_flip() {
        // high: 100 - 300 = -200 ; low: 400 - 100 = +300
        let report = analyze(&four_cells(100, 300, 400, 100));
        assert_eq!(report.verdicts().0, Verdict::SignFlip);
        assert_eq!(report.runs_analyzed(), 4);
    }

    #[test]
    fn same_signed_simple_effects_are_not_a_flip() {
        let report = analyze(&four_cells(300, 100, 400, 100));
        assert_eq!(report.verdicts().0, Verdict::SameDirection);
    }

    #[test]
    fn an_empty_cell_is_degenerate_not_a_claim() {
        let runs = vec![
            simple_run("i1", Confidence::High, Reliability::Fiable, false, 100),
            simple_run("i1", Confidence::Low, Reliability::Fiable, false, 400),
            simple_run("i1", Confidence::Low, Reliability::Degradee, false, 100),
        ];
        let report = analyze(&runs);
        assert_eq!(report.verdicts().0, Verdict::Degenerate);
        assert_eq!(report.primary.raw_contrast(), None);
    }

    #[test]
    fn a_zero_simple_effect_is_degenerate_not_a_direction() {
        assert_eq!(
            analyze(&four_cells(200, 200, 400, 100)).verdicts().0,
            Verdict::Degenerate
        );
    }

    #[test]
    fn contaminated_and_failed_runs_are_excluded_never_analysed() {
        let mut runs = four_cells(100, 300, 400, 100);
        let mut bad = simple_run("i2", Confidence::High, Reliability::Fiable, false, 9999);
        bad.record.status = "contaminated".to_string();
        let mut worse = simple_run("i3", Confidence::High, Reliability::Fiable, false, 9999);
        worse.record.status = "failed".to_string();
        runs.push(bad);
        runs.push(worse);

        let report = analyze(&runs);
        assert_eq!(report.runs_analyzed(), 4, "only the successful runs");
        assert_eq!(report.excluded.non_success, 2);
        assert_eq!(
            report.primary.high.value(),
            Some(100.0),
            "the contaminated run's 9999 tokens must not enter the mean"
        );
        assert!(
            report
                .render()
                .contains("status not success (incl. contaminated): 2")
        );
    }

    #[test]
    fn a_capture_from_another_agent_is_not_attributable() {
        // The record says high-confidence; the captured lines are another
        // agent's. The slice caught someone else's turns.
        let log = event("mika-dev", 0, 500, false);
        let r = run("i1", Confidence::High, Reliability::Fiable, false, log);
        let report = analyze(&[r]);
        assert_eq!(report.runs_analyzed(), 0);
        assert_eq!(report.excluded.unattributable, 1);
    }

    #[test]
    fn a_capture_from_another_spirit_session_is_not_attributable() {
        let log = event(Confidence::High.agent_id(), 0, 500, false)
            .replace(SESSION, "some-other-spirit-session");
        let r = run("i1", Confidence::High, Reliability::Fiable, false, log);
        let report = analyze(&[r]);
        assert_eq!(report.runs_analyzed(), 0);
        assert_eq!(report.excluded.unattributable, 1);
    }

    #[test]
    fn covariates_read_turn_order_from_step_not_from_line_order() {
        let a = Confidence::High.agent_id();
        let log = [
            event(a, 4, 10, false),
            event(a, 1, 10, true),
            event(a, 3, 10, true),
            event(a, 0, 10, false),
            event(a, 2, 10, false),
        ]
        .join("\n");
        let report = analyze(&[run("i1", Confidence::High, Reliability::Fiable, false, log)]);
        let (turns, handshakes, recalcs, _) =
            report.covariates.cells[&(Confidence::High, Reliability::Fiable)];
        assert_eq!((turns, handshakes, recalcs), (5.0, 2.0, 2.0));
    }

    // ---------------------------------------------------------------------
    // The two pre-registered contrasts (PREREGISTRATION.md, operator 2026-08-29)
    // ---------------------------------------------------------------------

    /// A batch shaped like the real dilution: one perturbed item and one
    /// unperturbed item per cell. On the perturbed item the arms diverge; on the
    /// unperturbed item the inputs are identical, so the arms agree.
    fn diluted_batch() -> Vec<Run> {
        vec![
            // Perturbed item — the manipulation actually happened here.
            simple_run("p1", Confidence::High, Reliability::Fiable, false, 100),
            simple_run("p1", Confidence::High, Reliability::Degradee, true, 400),
            simple_run("p1", Confidence::Low, Reliability::Fiable, false, 400),
            simple_run("p1", Confidence::Low, Reliability::Degradee, true, 100),
            // Unperturbed item — inputs identical across arms, so no effect.
            simple_run("u1", Confidence::High, Reliability::Fiable, false, 200),
            simple_run("u1", Confidence::High, Reliability::Degradee, false, 200),
            simple_run("u1", Confidence::Low, Reliability::Fiable, false, 200),
            simple_run("u1", Confidence::Low, Reliability::Degradee, false, 200),
        ]
    }

    #[test]
    fn the_secondary_contrast_restricts_to_the_perturbed_items() {
        let report = analyze(&diluted_batch());
        assert_eq!(report.runs_analyzed(), 8, "primary takes every run");
        assert_eq!(
            report.secondary.runs, 4,
            "secondary takes the perturbed item"
        );
        assert_eq!(report.control.runs, 4, "controls are the rest");
        assert_eq!(report.perturbed_items, 1);

        // On the perturbed item the effect reverses; the dilution washes it out
        // of the primary, which is exactly why both must be reported.
        assert_eq!(report.verdicts().1, Verdict::SignFlip);
        assert_eq!(report.control.verdict, Verdict::Degenerate);
    }

    #[test]
    fn dilution_can_hide_in_the_primary_what_the_secondary_shows() {
        let report = analyze(&diluted_batch());
        let (primary, secondary) = report.verdicts();
        // Primary: high 150-300 = -150, low 300-150 = +150 → still a flip here,
        // but attenuated. The point under test is that the two contrasts are
        // computed over different run sets and can disagree.
        assert_ne!(
            report.primary.raw_contrast(),
            report.secondary.raw_contrast(),
            "the dilution must actually change the arithmetic"
        );
        assert_eq!((primary, secondary), (Verdict::SignFlip, Verdict::SignFlip));
    }

    #[test]
    fn both_contrasts_are_always_rendered_and_named() {
        let rendered = analyze(&diluted_batch()).render();
        let primary_at = rendered
            .find("## PRIMARY contrast")
            .expect("primary section");
        let secondary_at = rendered
            .find("## SECONDARY contrast, pre-specified")
            .expect("secondary section");
        assert!(
            primary_at < secondary_at,
            "the primary is reported first — it is the intention-to-treat estimand"
        );
        assert!(rendered.contains("reporting only\none is a protocol violation"));
        assert!(rendered.contains("within-design control"));
    }

    #[test]
    fn there_is_no_way_to_read_one_verdict_without_the_other() {
        // Structural half of the pre-registration's reporting rule: `verdicts`
        // is the only accessor and it returns the pair. This test exists to
        // fail loudly if someone adds a `primary()` that yields one alone.
        let report = analyze(&diluted_batch());
        let (_primary, _secondary) = report.verdicts();
    }

    #[test]
    fn the_disclaimer_is_the_first_content_of_the_report() {
        let rendered = analyze(&four_cells(100, 300, 400, 100)).render();
        let first = rendered.lines().next().unwrap();
        assert!(
            first.contains("EXISTENCE CLAIM ONLY")
                && first.contains("DOES NOT ESTIMATE A MAGNITUDE"),
            "guardrail 3: disclaimer leads the report, got {first:?}"
        );
        let disclaimer_end = rendered.find("## PRIMARY contrast").unwrap();
        let verdict_pos = rendered.find("verdict:").unwrap();
        assert!(disclaimer_end < verdict_pos);
    }

    #[test]
    fn covariates_never_appear_in_either_contrast_section() {
        let rendered = analyze(&four_cells(100, 300, 400, 100)).render();
        let split = rendered.find("## Descriptive covariates").unwrap();
        let (contrasts, covariate_section) = rendered.split_at(split);

        assert!(
            !contrasts.contains("handshake"),
            "guardrail 2: covariate leaked into a contrast section"
        );
        assert!(!contrasts.contains("recalculation"));
        assert!(covariate_section.contains("handshakes"));
        assert!(covariate_section.contains("recalculations"));
    }

    #[test]
    fn load_batch_reads_records_and_their_captures() {
        let dir = std::env::temp_dir().join(format!("rt005-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("runs")).unwrap();
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(
            dir.join("runs/high.fiable.i1.r1.json"),
            r#"{"disclaimer":"…","status":"success","attempt":1,"session_id":"rt005-b-high.fiable.i1.r1-a1","spirit_session_id":"s-spirit-0001","design_fingerprint":"fp","mode":"live","output":"…","stderr":"","turn_usage_log":"logs/high.fiable.i1.r1.turn_usage.jsonl","run_id":"high.fiable.i1.r1","confidence":"high","reliability":"fiable","item_id":"i1","replicate":1,"agent":"mika-dev-confidence-high","peer_b_answer":"x","perturbed":false,"prompt":"…"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("logs/high.fiable.i1.r1.turn_usage.jsonl"),
            event(Confidence::High.agent_id(), 0, 42, false),
        )
        .unwrap();

        let runs = load_batch(&dir).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].record.run_id, "high.fiable.i1.r1");
        assert!(!runs[0].record.perturbed);
        assert_eq!(analyze(&runs).primary.high.value(), Some(42.0));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
