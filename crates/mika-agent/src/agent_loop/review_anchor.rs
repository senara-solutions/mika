//! Review-anchor verification — the non-terminal half of the disposition guard family.
//!
//! mika#901 requires an F-list when the architect returns a *terminal* disposition
//! (`ITERATE` / `ESCALATE`). The non-terminal half (`READY` / `Verdict: GROOMED`) was
//! deliberately exempt: a plan with no objection needs no findings. That exemption is the
//! hole mika#2037 measured — the one disposition that advances the chain was the only one
//! requiring no evidence, so a 302-byte acknowledgement carrying the keyword forged an
//! architect attestation on a 10 KB brief it had not read.
//!
//! This module decides whether a response carries an attestation only a real review can
//! produce: **anchor lines that quote the brief verbatim, at distinct positions**.
//!
//! Why verbatim quotation and not the two alternatives considered:
//!
//! - *Answering the brief's numbered questions* depends on the brief being numbered. Free-text
//!   briefs are not, so the guard would be inert exactly where it is most needed.
//! - *A per-criterion verdict* is trivially satisfiable by a grid of "OK" with no content.
//!
//! Verbatim quotation is the only candidate the engine can check against a source it already
//! holds. Dispersion — non-overlapping regions — is what gives it bite: one quote is crossable
//! by copying the brief's first line; three quotes spread across a multi-kilobyte document are
//! not a by-product of an acknowledgement.
//!
//! "Verbatim" means the brief's *words*, not its bytes (mika#2338). Briefs are markdown and a
//! model quotes what it read, not the markup around it; the comparison therefore runs on a
//! canonical form of both sides — see [`normalize_for_anchor_match`] for exactly what folds
//! and why each fold is grounded in a measured case. What never folds: the words themselves.
//! A paraphrase, or an exact quote of a different document, is refused as before.
//!
//! The matcher is deliberately regex-free. mika#864 established the precedent for this family
//! ("regex is a footgun — silent failure to fire when pattern is malformed"), and a guard that
//! silently stops firing is the defect class being closed here, not an acceptable cost.
//!
//! ## The verdict says *which* line failed (mika#2794)
//!
//! [`AnchorVerdict::Missing`] carries `per_anchor: Vec<AnchorOutcome>` — one entry per anchor
//! line found in the body, in body order, each with its prefix, the start of what it said and
//! whether it was refused. **Nothing in the decision reads it**: the counters and the
//! [`AnchorMissReason`] still decide alone, and the two early returns carry an empty list. It
//! exists for the corrective re-prompt, which [`format_anchor_diagnosis`] builds from it.
//!
//! The measurement that asked for it. After mika#2786 (frein à trois retenues, tolérance
//! `**A1:**`), an offline replay of this matcher over the real 2026-10-18 briefs
//! (`messages`) and responses (`llm_calls`) of three mika-arch grooming sessions gave
//! `49ffa230` 3 anchors found / 2 valid before *and* after the re-prompt, `a7f1ccad` the same
//! 3 / 2 on each of three responses, `f167ff53` 3 / 1 then 3 / 0: **one anchor of three was
//! refused, and in two of the three sessions it was the first** — a quote of the opening
//! sentence of the ticket body read with `gh_read`, not of the turn's brief message. The
//! re-prompt named the `miss_reason` and not the line, and `a7f1ccad` returned the same faulty
//! anchor three times. Naming the class was not enough; naming the line is what `per_anchor`
//! makes possible.
//!
//! What this did **not** change, deliberately: the guard stays fail-closed, the comparison
//! stays verbatim against the brief, and `verify_review_anchors` returns the same decision on
//! the same input as before mika#2794. Whether a model obeys a re-prompt that names the line
//! is not something an offline test can measure — the before/after is read from the logs, per
//! the protocol in `docs/agent-internals/agent-loop.md`.

/// Why an attestation was rejected. Carried into the corrective re-prompt so the model is told
/// which condition it missed rather than the whole contract again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnchorMissReason {
    /// No line in the message body starts with a declared anchor prefix.
    NoAnchorLine,
    /// Anchor lines exist, but none carries a quote long enough to check.
    QuoteTooShort,
    /// Anchor lines carry long-enough text, but it does not appear in the brief.
    QuoteNotInBrief,
    /// Enough anchors quote the brief, but they land on the same region of it.
    OverlappingRegions,
    /// Anchors are individually valid but fewer than the declared minimum.
    TooFewAnchors,
}

impl AnchorMissReason {
    /// One-line description used in the corrective re-prompt.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::NoAnchorLine => {
                "the response body carries no anchor line at all — no line starts with a \
                 declared anchor prefix"
            }
            Self::QuoteTooShort => {
                "the anchor lines are too short to carry a checkable quote of the reviewed brief"
            }
            Self::QuoteNotInBrief => {
                "the anchor lines do not quote the reviewed brief verbatim — paraphrase does \
                 not satisfy the contract, the quoted span must appear in the brief exactly"
            }
            Self::OverlappingRegions => {
                "the anchor lines all quote the same region of the brief — each anchor must \
                 quote a distinct part of it"
            }
            Self::TooFewAnchors => "fewer valid anchor lines than the contract requires",
        }
    }
}

/// How many characters of an anchor line the verdict carries back to the caller.
///
/// Counted in **chars**, never bytes. The briefs this guard runs on are French markdown and a
/// multi-byte codepoint inside the first 72 characters is the ordinary case, where `&line[..72]`
/// panics — see `scripts/check-byte-slices.sh`. Long enough that the model recognizes which of
/// its own three lines is named, short enough that naming three of them stays a paragraph.
const ANCHOR_EXCERPT_CHARS: usize = 72;

/// Opening marker line of the plan a groom retry carries (mika#2996).
///
/// The UNPARSED retries of `_iterate_groom_loop` (mika#1823 on the first pass, mika#2641 on
/// the second) used to send a ~380-character corrective prompt and then accept the
/// disposition returned on that turn. Below `review_anchor_min_brief_chars` the guard does
/// not arm, so a `READY` / `Verdict: GROOMED` with no anchor advanced the chain — measured on
/// 2026-10-31, trace `15ea28bd`, a review of *another* plan accepted as `READY-after-retry`.
///
/// The retry now carries the plan, between this line and [`RETRY_PLAN_END`].
/// `skills/bundled/_shared/dispatch-lib.sh` writes both lines **literally**; the copy there
/// is held to this one by `tests/eval/grounding_regressions/review_anchor_unparsed_retry_2996.rs`,
/// which runs the shell function and looks for these constants in its output.
pub const RETRY_PLAN_BEGIN: &str = "----- BEGIN PLAN UNDER REVIEW (mika#2996) -----";

/// Closing marker line of the plan a groom retry carries. See [`RETRY_PLAN_BEGIN`].
pub const RETRY_PLAN_END: &str = "----- END PLAN UNDER REVIEW (mika#2996) -----";

/// The part of a turn's brief that review anchors are verified against.
///
/// A brief carrying the retry envelope is reduced to the plan between the two markers: the
/// corrective instruction around it is the engine's own text, not the reviewed document, and
/// three 40-character quotes of it would otherwise satisfy the contract while saying nothing
/// about the plan. Every other brief — a first pass, whose message *is* the plan — is
/// returned whole.
///
/// The first opening marker and the last closing one bound the region, so a plan that
/// happens to quote a marker line stays inside it. A brief with only one of the two markers
/// is returned whole: the fallback keeps the guard armed on the full text rather than
/// disarming it on an empty region, which would reopen the hole this exists to close.
pub fn anchor_reference(brief: &str) -> &str {
    brief
        .split_once(RETRY_PLAN_BEGIN)
        .and_then(|(_, rest)| rest.rsplit_once(RETRY_PLAN_END))
        .map(|(plan, _)| plan)
        .unwrap_or(brief)
}

/// La région du brief qui n'est PAS le plan revu : ce qui s'adresse à l'agent.
///
/// Exact complément de [`anchor_reference`], sur les mêmes deux constantes — posé à côté
/// d'elle parce qu'une seconde lecture des marqueurs ailleurs dans l'arbre prendrait du
/// retard sans que rien ne rougisse.
///
/// Rend DEUX tranches — la tête (avant [`RETRY_PLAN_BEGIN`]) et la queue (après
/// [`RETRY_PLAN_END`]) — parce que les deux sont adressables à l'agent et qu'un `&str`
/// unique ne peut pas porter deux régions disjointes. Aujourd'hui
/// `_append_retry_plan_envelope` n'écrit rien après la ligne de fermeture, donc la queue est
/// vide ; la rendre quand même est ce qui fait que le jour où la relance ajoutera un
/// post-scriptum, il sera lu au lieu d'être muet.
///
/// - deux marqueurs → `(tête, queue)`
/// - **zéro** marqueur → `("", "")`. Sur un tour de revue, un brief sans enveloppe EST le
///   plan — [`anchor_reference`] le dit déjà : une première passe, dont le message *est* le
///   plan, est rendue entière — donc rien n'y est adressé à l'agent.
/// - **un seul** marqueur → `(brief, "")`, le brief entier. Fail-closed, même posture
///   qu'[`anchor_reference`] : une enveloppe mutilée est un défaut du shell, et la direction
///   conservatrice est que la garde appelante s'arme — elle ne coûte qu'un re-prompt.
///
/// Référence : mika#3002, mika#2996.
pub fn outside_reviewed_plan(brief: &str) -> (&str, &str) {
    match brief.split_once(RETRY_PLAN_BEGIN) {
        // Enveloppe complète : la tête porte l'instruction, la queue le post-scriptum.
        Some((head, rest)) => match rest.rsplit_once(RETRY_PLAN_END) {
            Some((_, tail)) => (head, tail),
            // Ouvrant seul.
            None => (brief, ""),
        },
        // Fermant seul : l'autre moitié d'enveloppe mutilée.
        None if brief.contains(RETRY_PLAN_END) => (brief, ""),
        // Aucun marqueur : le brief est le plan.
        None => ("", ""),
    }
}

/// One anchor line's fate, in body order.
///
/// Four things a future editor needs, because each undoes itself if guessed:
///
/// - **`Satisfied` carries none of this.** A response that passed needs no diagnosis; the list
///   exists to tell a refused response *which* of its lines failed.
/// - **It does not derive from the counters.** `anchors_found` and `anchors_valid` are two
///   integers; which line produced which is not recoverable from them, and that is exactly the
///   information mika#2794 measured missing — `a7f1ccad` returned the same faulty anchor three
///   times because the corrective re-prompt named the reason and not the line.
/// - **The two early returns yield an empty list** (`NoAnchorLine`, and an empty normalized
///   brief): there is no per-line fate to report. The re-prompt falls back word for word on its
///   pre-mika#2794 form there.
/// - **`refused: None` is the only positive statement.** An accepted line is named so the
///   re-prompt can tell the model to keep it; everything else carries why it was blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnchorOutcome {
    /// The declared prefix that identified the line (`A1:`) — never the `**` the model may have
    /// wrapped it in (mika#2786 strips those before the prefix is looked for).
    pub(crate) label: String,
    /// Start of the line's normalized content, truncated to [`ANCHOR_EXCERPT_CHARS`]. A `…` is
    /// appended only when the line was longer, so the excerpt never claims an elision that did
    /// not happen.
    pub(crate) excerpt: String,
    /// `None` when the line claimed a region of the brief; else what blocked it.
    pub(crate) refused: Option<AnchorMissReason>,
}

/// Outcome of verifying a response against the review-anchor contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorVerdict {
    /// The response carries a valid attestation.
    Satisfied,
    /// The response does not. `anchors_found` counts prefixed lines; `anchors_valid` counts
    /// those that survived every check; `per_anchor` says which line is which.
    Missing {
        anchors_found: usize,
        anchors_valid: usize,
        reason: AnchorMissReason,
        per_anchor: Vec<AnchorOutcome>,
    },
}

impl AnchorVerdict {
    /// Test-only convenience — production code matches on `Missing` to read the reason.
    #[cfg(test)]
    pub(crate) fn is_satisfied(&self) -> bool {
        matches!(self, Self::Satisfied)
    }
}

/// Total budget of the turn's tool-read ledger, in **normalized characters**.
///
/// Counted in chars, never bytes (`scripts/check-byte-slices.sh`): the ticket bodies and plans
/// this engine reads are French, and a 200 000-*byte* prefix of one lands inside a codepoint.
///
/// The figure is a ceiling on cost, not a target. The measured shape — the body of mika#2995,
/// read with `gh_read issue_view` on 2026-10-31 — is a few kilobytes, and a whole grooming
/// turn's reads sit far below this. What the ceiling bounds is the other tail: a `read_file` on
/// a multi-megabyte log, where an unbounded ledger would pay a normalization pass over the lot.
/// The budget is the price of ONE pass per tool result, paid at push and linear in its length;
/// the search it serves is a 72-character needle ([`ANCHOR_EXCERPT_CHARS`]) in a bounded
/// haystack. That asymmetry is what retires mika#2794's verbatim-budget objection.
const TOOL_READ_LEDGER_CHARS: usize = 200_000;

/// Hard floor, in normalized characters, on the span an attribution is allowed to rest on.
///
/// The claim the indicative mood makes — *this line comes from that tool* — is the same kind of
/// claim the matcher makes about the brief, so it owes the same evidence: a span long enough
/// that a coincidence is not the likelier explanation. 40 is `review_anchor_min_quote_chars` as
/// every shipped arch manifest declares it, so in practice the attribution and the matcher run
/// the same window; the floor exists for the skill that lowers its own threshold. Lowering the
/// bar there would buy nothing and risk naming a tool on a coincidence — and a false
/// attribution is worse than none, because the sentence that carries it is indicative.
const ATTRIBUTION_MIN_QUOTE_CHARS: usize = 40;

/// Normalized text of what the turn's tools returned, for anchor attribution (mika#3003).
///
/// Filled forward, at the single site where a tool result is appended to the turn — never
/// by walking `request.messages` back at the guard site. The guard receives a value; it
/// traverses nothing. This is the shape mika#2794 refused when the lookup would have been
/// a second reader of the conversation.
///
/// Three properties, each because the opposite guesses badly:
///
/// - **Filled forward, at the single append site.** No access to `request.messages`, so no
///   second reader of the conversation. The single writer is held by a source scan in this
///   module's `mika3003` tests — a second push site would double-count one tool output, or
///   worse push it *unnormalized*, leaving a ledger no search ever hits and a guard that falls
///   back on the conditional sentence in silence (the class of mika#2158: every behavioural
///   assertion stays green).
/// - **Normalized and clipped at push, never at read.** [`TOOL_READ_LEDGER_CHARS`] bounds the
///   whole ledger, and the normalization is the matcher's own
///   [`normalize_for_anchor_match`] — a needle normalized one way and a haystack another is a
///   comparison that silently never matches.
/// - **The scope is the TURN, not the step.** Declared before the step loop, so it accumulates
///   every step of the turn — "read during the turn" is exactly what the attribution claims, and
///   it is precisely what the measurement needs: on 2026-10-31 the architect read the ticket
///   body at step 1 and quoted it at step 4. A per-step ledger would have attributed nothing.
#[derive(Debug, Default)]
pub(crate) struct ToolReadLedger {
    /// `(tool name, normalized output)`, in call order. Normalized with the SAME
    /// [`normalize_for_anchor_match`] the matcher uses.
    entries: Vec<(String, String)>,
    /// Set when a push was clipped by [`TOOL_READ_LEDGER_CHARS`]. A miss in a truncated
    /// ledger is NOT an absence (mika#2331) — see [`AnchorAttribution::Unknown`].
    truncated: bool,
}

/// Which tool's output a refused anchor's excerpt was found in, verbatim, this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnchorAttribution {
    /// Found: the excerpt appears in this tool's normalized output.
    Tool(String),
    /// Not found, and the ledger was WHOLE — the engine may say so.
    AbsentFromReads,
    /// Not found, but the ledger was CLIPPED — the engine may NOT say so.
    ///
    /// A failed search in a clipped ledger and a failed search in a whole one return the same
    /// `false`; conflating them would have the engine assert "this line comes from no tool"
    /// about an output it had itself cut short. On this arm the re-prompt falls back **word for
    /// word** on mika#2794's conditional sentence: the knowledge is missing, so the assertion
    /// is not made. That is the repo's grounding rule applied to the engine itself.
    Unknown,
}

impl ToolReadLedger {
    /// Record what one tool returned this turn, normalized and clipped to the remaining budget.
    ///
    /// **The single writer under `src/`.** Held by
    /// `review_anchor::mika3003::mika3003_le_registre_de_lecture_a_un_seul_ecrivain`.
    pub(crate) fn record_tool_read(&mut self, tool_name: &str, output: &str) {
        let used: usize = self
            .entries
            .iter()
            .map(|(_, text)| text.chars().count())
            .sum();
        let remaining = TOOL_READ_LEDGER_CHARS.saturating_sub(used);
        if remaining == 0 {
            self.truncated = true;
            return;
        }
        // Clip the RAW text first, then normalize. Normalization only ever shrinks (markers
        // dropped, runs of whitespace collapsed), so a raw prefix of `remaining` chars can
        // never normalize past the budget — and the transient allocation stays bounded by it
        // too, which clipping after normalizing would not achieve.
        let normalized = if output.chars().count() > remaining {
            self.truncated = true;
            let clipped: String = output.chars().take(remaining).collect();
            normalize_for_anchor_match(&clipped)
        } else {
            normalize_for_anchor_match(output)
        };
        self.entries.push((tool_name.to_string(), normalized));
    }

    /// Where a refused anchor's text came from, when a verbatim search can establish it.
    ///
    /// `None` on any outcome that is not a [`AnchorMissReason::QuoteNotInBrief`] refusal: an
    /// `OverlappingRegions` or a `QuoteTooShort` does not ask "where does this text come
    /// from?" — the text **is** in the brief, it is its geometry or its length that failed.
    /// Saying "you quoted another document" there would send exactly the wrong fix. This is
    /// the gate mika#2794 already wrote at the formatting site, reused as written.
    ///
    /// `Tool` on the first entry the search hits, in call order; `Unknown` when nothing matched
    /// and the ledger was clipped; `AbsentFromReads` when nothing matched and it was whole — an
    /// empty ledger (no tool called this turn) is whole, not unknown.
    ///
    /// **The search is the matcher's own** [`find_brief_quote_range`], at the same threshold,
    /// and not a `contains` of the excerpt. An anchor line reads
    /// `A1: "…citation…" — my comment`, so the excerpt carries the model's quotation marks and
    /// the start of its own prose; no document ever contains that string, and a `contains`
    /// would have answered `AbsentFromReads` on the very shape mika#3003 was measured on. What
    /// the attribution asks is the question the matcher asks of the brief — *does a checkable
    /// span of this line appear verbatim in that text* — put to the tool's output instead.
    pub(crate) fn attribute(
        &self,
        outcome: &AnchorOutcome,
        min_quote_chars: usize,
    ) -> Option<AnchorAttribution> {
        if outcome.refused != Some(AnchorMissReason::QuoteNotInBrief) {
            return None;
        }
        // The trailing `…` is a rendering marker [`anchor_excerpt`] appended, not text the
        // model wrote: searching with it attached would miss every truncated anchor, which is
        // every anchor longer than 72 characters — the ordinary case.
        //
        // Re-normalized rather than assumed normalized: `anchor_excerpt` already works on a
        // normalized line, and the function is idempotent, so this costs one pass and removes
        // a dependency on a caller's invariant (the asymmetry of mika#2794's objection).
        let needle = normalize_for_anchor_match(outcome.excerpt.trim_end_matches('…'));
        // The excerpt is capped at `ANCHOR_EXCERPT_CHARS`, so a contract demanding a longer
        // quote cannot be re-proven on it; the window is clamped to that cap. The floor is the
        // part that matters: below `ATTRIBUTION_MIN_QUOTE_CHARS` a hit is noise, and naming a
        // tool on noise is a fabrication the indicative mood would carry as a measurement.
        // Both bounds are constants, and the floor is below the cap, so `clamp` cannot panic.
        let window = min_quote_chars.clamp(ATTRIBUTION_MIN_QUOTE_CHARS, ANCHOR_EXCERPT_CHARS);
        if needle.chars().count() >= window {
            for (tool, text) in &self.entries {
                if matches!(
                    find_brief_quote_range(&needle, text, window, &[]),
                    QuoteSearch::Found(_)
                ) {
                    return Some(AnchorAttribution::Tool(tool.clone()));
                }
            }
        }
        Some(if self.truncated {
            AnchorAttribution::Unknown
        } else {
            AnchorAttribution::AbsentFromReads
        })
    }
}

/// What the denominator line of mika#3003 AC3 reports, accumulated across the guard's branches.
///
/// The numerator `guard.review_anchor_withheld` existed before this flight; the ticket's own
/// words are "the denominator, that is the number of non-terminal passages per day, is not
/// measured". So `guard.review_anchor_armed` is emitted **once per evaluation** of the armed
/// guard — `Satisfied` included — after both branches have run. A turn that re-prompts produces
/// two lines: the re-prompt's (`withheld: false`) and the following passage's. That is
/// deliberate: the rate the ticket asks for is "withholds over non-terminal passages", and a
/// passage is an evaluation. The pre-flight denominator can only be rebuilt offline, from
/// `llm_calls.response_text` (column v31) — see `docs/agent-internals/agent-loop.md`.
///
/// This is **not** the probe mika#2794 refused: that one wanted to replay the verbatim search
/// over every document of the turn to guess an origin. This line only counts what the guard has
/// already decided.
///
/// `found` / `valid` stay `None` on the `Satisfied` branch, hence **absent** from the line:
/// `AnchorVerdict::Satisfied` is a unit variant a dozen `assert_eq!`s depend on, and adding two
/// fields to it for one log line would change those tests' intent. This is not the absence
/// mika#2331 condemns — the line always carries `withheld` and `brief_chars`, and the
/// denominator is counted in **lines**, not in anchors.
#[derive(Debug, Default)]
pub(crate) struct ArmedPassage {
    found: Option<usize>,
    valid: Option<usize>,
    withheld: bool,
    /// Tool names, in encounter order and deduplicated. Empty string when nothing was
    /// attributed, never an absent field — the line must separate "no source found" from
    /// "pre-mika#3003 binary" (mika#2331, mika#2340).
    sources: String,
}

impl ArmedPassage {
    /// Attribute every anchor of a `Missing` verdict, and capture what the line will report.
    ///
    /// Returns the attributions **indexed in parallel with `per_anchor`**, for
    /// [`format_anchor_diagnosis`]. Computed once, before the guard's two branches: the
    /// re-prompt renders it as prose, the two structured lines render it as a field, and a
    /// single read of the ledger feeds both.
    ///
    /// Three anchors taken from the same ticket body describe ONE source, so repeating it would
    /// read as a count where there is a set — hence the dedup on the way into [`Self::sources`].
    pub(crate) fn attribute(
        &mut self,
        ledger: &ToolReadLedger,
        per_anchor: &[AnchorOutcome],
        min_quote_chars: usize,
        anchors_found: usize,
        anchors_valid: usize,
    ) -> Vec<Option<AnchorAttribution>> {
        let attributions: Vec<Option<AnchorAttribution>> = per_anchor
            .iter()
            .map(|outcome| ledger.attribute(outcome, min_quote_chars))
            .collect();
        let mut sources: Vec<&str> = Vec::new();
        for attribution in attributions.iter().flatten() {
            if let AnchorAttribution::Tool(tool) = attribution
                && !sources.contains(&tool.as_str())
            {
                sources.push(tool.as_str());
            }
        }
        self.sources = sources.join(",");
        self.found = Some(anchors_found);
        self.valid = Some(anchors_valid);
        attributions
    }

    /// The turn's tools a refused quote was traced to, `,`-joined. Empty, never absent.
    ///
    /// Also carried by `guard.review_anchor` and `guard.review_anchor_withheld`, so the
    /// numerator says where the quotes came from too.
    pub(crate) fn sources(&self) -> &str {
        &self.sources
    }

    /// The disposition was withheld on this passage.
    pub(crate) fn mark_withheld(&mut self) {
        self.withheld = true;
    }

    /// Emit the denominator line, once, for every exit of the armed guard.
    pub(crate) fn emit(
        &self,
        trace_id: &str,
        agent_id: &str,
        session_id: &str,
        step: usize,
        label: &str,
        brief_chars: usize,
    ) {
        tracing::info!(
            target: "mika::otel",
            trace_id = %trace_id,
            agent_id = %agent_id,
            session_id,
            step,
            label,
            anchors_found = self.found,
            anchors_valid = self.valid,
            brief_chars,
            withheld = self.withheld,
            attributed_sources = %self.sources,
            event = "guard.review_anchor_armed",
            "Review-anchor guard armed on a non-terminal disposition \
             (#3003 AC3 — the denominator `review_anchor_withheld` is read against)"
        );
    }
}

/// How many refused anchor lines the corrective re-prompt names.
///
/// The measured shape refuses one of three (mika#2794); three is already the contract's whole
/// anchor budget, and a re-prompt that enumerates ten lines is a wall the model skims.
const MAX_DIAGNOSED_ANCHORS: usize = 3;

/// Name the anchor lines that failed, for the corrective re-prompt (mika#2794).
///
/// The guard already told the model *what* condition it missed — `reason.describe()` — and
/// mika#2794 measured that this is not enough: across three replayed 2026-10-18 sessions the
/// refused anchor was one of three and, in two of them, always the first, and session
/// `a7f1ccad` returned that same faulty anchor three times in a row. The reason alone names a
/// class; this paragraph names the line.
///
/// Three parts, and each one is there for a reason the others do not cover:
///
/// - **The accepted anchors, by name.** A model told only "one anchor does not quote the brief"
///   rewrites all three. Naming the two that passed is what makes the correction local.
/// - **The refused anchors, one per line**, with the start of what they said and why it was
///   blocked, bounded by [`MAX_DIAGNOSED_ANCHORS`].
/// - **Where a wrong quote comes from**, gated on `QuoteNotInBrief`, and since mika#3003 said in
///   one of two moods depending on what the engine can establish. `attributions[i]` carries
///   [`ToolReadLedger::attribute`]'s verdict for `per_anchor[i]`: on `Tool(name)` — the refused
///   excerpt was found **verbatim** in what that tool returned this turn — the sentence is
///   indicative and names the line and the tool. On `AbsentFromReads`, on `Unknown`, and on an
///   absent entry it stays the pre-mika#3003 likelihood, word for word: the engine then holds no
///   fact to assert, and a conditional sentence is what the grounding rule allows. mika#2794
///   phrased it conditionally because the engine had no access to the turn's tool outputs; this
///   flight gives it that access, and the mood follows the knowledge rather than the hypothesis.
///   The gate matters as much as the wording: saying "you quoted another document" to a response
///   whose anchors all landed on the *same region of the right brief* would send exactly the
///   wrong fix.
///
/// `attributions` is indexed **in parallel with `per_anchor`**, never by rank among the refused
/// lines: `attribute` returns `None` for every refusal that is not `QuoteNotInBrief`, so a turn
/// refusing one line for its quote and another for overlap yields one attribution, and a rank
/// would have lost which line it belongs to. A shorter slice degrades to the conditional mood
/// rather than mis-pairing — the caller holds both vectors, but the pairing is load-bearing
/// enough to be defended here too.
///
/// `None` when the list is empty — the two early returns of [`verify_review_anchors`] report no
/// per-line fate, and the re-prompt then falls back word for word on the form it had before
/// mika#2794.
pub(crate) fn format_anchor_diagnosis(
    per_anchor: &[AnchorOutcome],
    attributions: &[Option<AnchorAttribution>],
) -> Option<String> {
    if per_anchor.is_empty() {
        return None;
    }

    let label_of = |outcome: &AnchorOutcome| outcome.label.trim_end_matches(':').to_string();
    let accepted: Vec<String> = per_anchor
        .iter()
        .filter(|outcome| outcome.refused.is_none())
        .map(label_of)
        .collect();
    let refused: Vec<&AnchorOutcome> = per_anchor
        .iter()
        .filter(|outcome| outcome.refused.is_some())
        .collect();

    let mut parts: Vec<String> = Vec::new();
    if !accepted.is_empty() {
        parts.push(format!(
            "Anchors accepted: {} — keep them exactly as written; do not rewrite them.",
            accepted.join(", ")
        ));
    }
    if !refused.is_empty() {
        let mut lines = vec!["Anchors refused:".to_string()];
        for outcome in refused.iter().take(MAX_DIAGNOSED_ANCHORS) {
            let reason = outcome
                .refused
                .map(AnchorMissReason::describe)
                .unwrap_or_default();
            lines.push(format!(
                "{} \"{}\" — refused: {reason}",
                outcome.label, outcome.excerpt
            ));
        }
        parts.push(lines.join("\n"));
    }
    // mika#3003 — l'indicatif, une ligne par ancre dont le registre de lecture du
    // tour a retrouvé l'extrait mot pour mot dans la sortie d'un outil.
    for (label, tool) in per_anchor
        .iter()
        .enumerate()
        .filter_map(|(index, outcome)| match attributions.get(index) {
            Some(Some(AnchorAttribution::Tool(tool))) => Some((label_of(outcome), tool.as_str())),
            _ => None,
        })
    {
        parts.push(format!(
            "{label} quotes, word for word, what the `{tool}` tool returned during this turn — \
             not the brief. This is measured, not guessed: the engine searched the text that \
             tool returned and found that line in it. Quote the brief instead: the message that \
             asked you for this review, which is the document under review."
        ));
    }
    // Le conditionnel reste, mot pour mot, pour toute ancre refusée sur
    // `QuoteNotInBrief` que le registre n'a PAS attribuée — `AbsentFromReads`,
    // `Unknown`, ou pas d'entrée du tout. Un registre tronqué qui n'a pas trouvé
    // l'extrait n'établit pas une absence (mika#3003), et le moteur n'a alors
    // rien à affirmer.
    if per_anchor.iter().enumerate().any(|(index, outcome)| {
        outcome.refused == Some(AnchorMissReason::QuoteNotInBrief)
            && !matches!(
                attributions.get(index),
                Some(Some(AnchorAttribution::Tool(_)))
            )
    }) {
        parts.push(
            "A quote that is absent from the brief usually comes from a document you read with \
             a tool — a ticket body, a file, a page — and not from the brief message of this \
             turn. Quote the brief itself: the message that asked you for this review."
                .to_string(),
        );
    }

    Some(parts.join("\n\n"))
}

/// Normalize a span of text to the form the quote comparison runs on.
///
/// Applied to both the brief and each anchor line before comparison, so it is a symmetric
/// canonical form, never a one-way concession. The contract is "quotes the brief's *words*
/// verbatim", not its bytes, and three things separate the two in practice:
///
/// - **Whitespace.** A reviewer quoting a plan re-flows the text they lift — the line break
///   that sat mid-sentence in a 10 KB brief does not survive into a one-line citation.
/// - **Inline markdown markers** (`*` for emphasis, `` ` `` for code spans). Briefs are
///   markdown written by a planning agent — 237 of the 245 briefs mika-arch received between
///   2026-09-01 and 2026-09-16 carry `**`, 243 carry backticks — and a model reading one the
///   way a human does quotes the *rendered* text. mika#2338 measured the cost: an anchor that
///   quoted the brief word for word was refused because `**Le mécanisme de kill, lui, est
///   correct** : \`kill_process_gracefully\`` had become `Le mécanisme de kill, lui, est
///   correct : kill_process_gracefully`, and no 40-character window survived the two markers.
///   Underscores are deliberately left alone: the repo's identifiers are full of them, and no
///   measured case put them in question.
/// - **Apostrophe shape.** `’` and `‘` fold onto `'`. Rare (2 of 199 responses in the same
///   window) but one character wide, and a false rejection costs a whole grooming run.
///
/// Markers are stripped *before* whitespace collapses, so `** :` does not leave a double space
/// behind, and the quote-length threshold is measured on the result — markup cannot buy length.
///
/// Without this, a genuine quote fails on the reviewer's rendering, which is a false rejection
/// of exactly the response the guard must let through.
/// `pub(crate)` since mika#3003: [`ToolReadLedger`] normalizes its haystacks with this very
/// function rather than a second copy of it. A ledger normalized one way and a needle another
/// is a search that never matches and a guard that silently stops attributing.
pub(crate) fn normalize_for_anchor_match(text: &str) -> String {
    let unmarked: String = text
        .chars()
        .filter(|c| !matches!(c, '*' | '`'))
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' => '\'',
            other => other,
        })
        .collect();
    unmarked.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The head of an already-normalized anchor line, for [`AnchorOutcome::excerpt`].
///
/// Char-indexed, and the `…` is appended only when something was actually dropped: an excerpt
/// that advertises an elision it did not make would have the model hunt for a longer line.
fn anchor_excerpt(normalized_line: &str) -> String {
    let mut excerpt: String = normalized_line.chars().take(ANCHOR_EXCERPT_CHARS).collect();
    if normalized_line.chars().count() > ANCHOR_EXCERPT_CHARS {
        excerpt.push('…');
    }
    excerpt
}

/// The message body, up to (exclusive of) the disposition-line landmark.
///
/// Same scan window as the mika#901 F-list guard, deliberately: the two guards must not
/// disagree about what "the body" is. An anchor placed after the disposition line does not
/// count — otherwise the contract could be satisfied below the line the operator reads.
fn body_lines<'a>(text: &'a str, required_suffix_lines: &[String]) -> Vec<&'a str> {
    let lines: Vec<&str> = text.lines().collect();
    // The LAST matching line, not the first. The effective disposition is the one at the end
    // of the message — that is the line the #864 guard and `has_declared_disposition` read.
    // Truncating at the first occurrence would drop the whole body of a response that quotes
    // the contract early (the shipped prompt now shows a worked example containing
    // `Disposition: READY`), and report `NoAnchorLine` for a fully anchored review.
    let suffix_line_idx = lines.iter().rposition(|line| {
        let trimmed = line.trim();
        required_suffix_lines
            .iter()
            .any(|req| trimmed == req.as_str())
    });
    let scan_end = suffix_line_idx.unwrap_or(lines.len());
    lines[..scan_end].to_vec()
}

/// Find a span of at least `min_quote_chars` characters of `line` that occurs verbatim in
/// `brief`, returning its byte range within `brief`.
///
/// Slides a fixed `min_quote_chars`-character window over the line rather than searching for the
/// longest common substring: the contract asks whether a checkable quote exists, not how long
/// the longest one is, and the fixed window keeps the cost linear in the line length.
///
/// Character-indexed throughout (`Vec<char>`, never byte slicing of `&str`) — a multi-byte quote
/// is the ordinary case in this repo's French briefs, and byte slicing panics on it. See
/// `scripts/check-byte-slices.sh`.
fn find_brief_quote_range(
    line: &str,
    brief: &str,
    min_quote_chars: usize,
    claimed: &[std::ops::Range<usize>],
) -> QuoteSearch {
    if min_quote_chars == 0 {
        return QuoteSearch::NoQuote;
    }
    let chars: Vec<char> = line.chars().collect();
    if chars.len() < min_quote_chars {
        return QuoteSearch::TooShort;
    }
    let mut saw_quote_in_claimed_region = false;
    for start in 0..=(chars.len() - min_quote_chars) {
        let window: String = chars[start..start + min_quote_chars].iter().collect();
        if window.trim().is_empty() {
            continue;
        }
        // Every occurrence of this window, not just the first: an early anchor whose first
        // match happens to land in a region a later anchor genuinely quotes would otherwise
        // steal it, and this guard is fail-closed — a false rejection costs a real review.
        // `match_indices` walks occurrences without hand-rolled slicing. Advancing a byte
        // cursor by `pos + 1` would land inside a multi-byte character and panic — measured,
        // on the first accented brief in the test suite. See `scripts/check-byte-slices.sh`.
        for (pos, _) in brief.match_indices(&window) {
            let range = pos..pos + window.len();
            if overlaps_any(&range, claimed) {
                saw_quote_in_claimed_region = true;
                continue;
            }
            return QuoteSearch::Found(range);
        }
    }
    if saw_quote_in_claimed_region {
        QuoteSearch::OnlyClaimedRegions
    } else {
        QuoteSearch::NoQuote
    }
}

/// Whether `range` overlaps any already-claimed region of the brief.
fn overlaps_any(range: &std::ops::Range<usize>, claimed: &[std::ops::Range<usize>]) -> bool {
    claimed
        .iter()
        .any(|c| range.start < c.end && c.start < range.end)
}

/// Outcome of looking for one anchor's quote in the brief.
enum QuoteSearch {
    /// A usable quote at this byte range of the brief.
    Found(std::ops::Range<usize>),
    /// The anchor line is shorter than the quote threshold.
    TooShort,
    /// The line quotes the brief, but only in regions an earlier anchor already claimed.
    OnlyClaimedRegions,
    /// No span of this line occurs in the brief at all.
    NoQuote,
}

/// Verify a response against a skill's review-anchor contract.
///
/// Returns `Satisfied` only when at least `min_count` anchor lines each quote at least
/// `min_quote_chars` characters of the brief verbatim, at mutually non-overlapping regions of it.
/// Every other outcome is a `Missing` carrying the reason — there is no third, permissive answer:
/// a response the engine cannot validate as a review is an absence of verdict, not an approval.
pub(crate) fn verify_review_anchors(
    text: &str,
    brief: &str,
    prefixes: &[String],
    required_suffix_lines: &[String],
    min_count: usize,
    min_quote_chars: usize,
) -> AnchorVerdict {
    let normalized_brief = normalize_for_anchor_match(brief);

    // Keep the anchor's *content*, not its label. The declared prefix (`A1:`) is the marker
    // that identifies the line, never part of the quote — counting it toward
    // `min_quote_chars` would let a longer prefix buy a shorter quote.
    //
    // Leading `*` are skipped before the prefix is looked for (mika#2786): a reviewer who
    // writes the label in bold (`**A1:** "…"`) has still written an anchor line. Measured on
    // mika#2762's third withheld pass (2026-10-18, session `8c9da828`): every label was bold,
    // so the guard saw `NoAnchorLine`; replayed with the label recognized, the response
    // carried three verbatim quotes at three disjoint regions of the brief. This tolerance is
    // on *finding* the line only. The `**` left after the prefix goes through
    // [`normalize_for_anchor_match`] like any other emphasis marker, so the quote is held to
    // exactly the same verbatim test as an unbolded one.
    //
    // The prefix is kept *alongside* the content, not discarded: it is the name the corrective
    // re-prompt calls the line by (mika#2794). It still does not enter the quote.
    let anchor_lines: Vec<(&str, &str)> = body_lines(text, required_suffix_lines)
        .into_iter()
        .filter_map(|line| {
            let trimmed = line.trim_start().trim_start_matches('*');
            prefixes.iter().find_map(|p| {
                trimmed
                    .strip_prefix(p.as_str())
                    .map(|rest| (p.as_str(), rest))
            })
        })
        .collect();

    let anchors_found = anchor_lines.len();
    if anchors_found == 0 {
        return AnchorVerdict::Missing {
            anchors_found: 0,
            anchors_valid: 0,
            reason: AnchorMissReason::NoAnchorLine,
            per_anchor: Vec::new(),
        };
    }

    // An empty brief can never ground a quote. Fail closed rather than vacuously pass —
    // a guard that approves when it has nothing to check against is the failure it prevents.
    if normalized_brief.is_empty() {
        return AnchorVerdict::Missing {
            anchors_found,
            anchors_valid: 0,
            reason: AnchorMissReason::QuoteNotInBrief,
            per_anchor: Vec::new(),
        };
    }

    let mut too_short = 0usize;
    let mut not_in_brief = 0usize;
    let mut overlapping = 0usize;
    let mut claimed: Vec<std::ops::Range<usize>> = Vec::new();
    let mut seen_lines: Vec<String> = Vec::new();
    let mut per_anchor: Vec<AnchorOutcome> = Vec::with_capacity(anchors_found);

    for &(label, line) in &anchor_lines {
        let normalized_line = normalize_for_anchor_match(line);
        let excerpt = anchor_excerpt(&normalized_line);
        // A repeated anchor line is one read of the brief, however many times it is pasted.
        // Region disjointness alone does not catch this: a single line longer than
        // `2 * min_quote_chars` yields non-overlapping windows, so copying one long sentence
        // three times would otherwise satisfy a contract that exists to require three reads.
        if seen_lines.contains(&normalized_line) {
            overlapping += 1;
            per_anchor.push(AnchorOutcome {
                label: label.to_string(),
                excerpt,
                refused: Some(AnchorMissReason::OverlappingRegions),
            });
            continue;
        }
        seen_lines.push(normalized_line.clone());
        let refused = match find_brief_quote_range(
            &normalized_line,
            &normalized_brief,
            min_quote_chars,
            &claimed,
        ) {
            QuoteSearch::Found(range) => {
                claimed.push(range);
                None
            }
            // The line quotes the brief only where an earlier anchor already did. Three
            // anchors must mean three reads of the brief, not one line copied three times.
            QuoteSearch::OnlyClaimedRegions => {
                overlapping += 1;
                Some(AnchorMissReason::OverlappingRegions)
            }
            QuoteSearch::TooShort => {
                too_short += 1;
                Some(AnchorMissReason::QuoteTooShort)
            }
            QuoteSearch::NoQuote => {
                not_in_brief += 1;
                Some(AnchorMissReason::QuoteNotInBrief)
            }
        };
        per_anchor.push(AnchorOutcome {
            label: label.to_string(),
            excerpt,
            refused,
        });
    }

    let anchors_valid = claimed.len();
    if anchors_valid >= min_count {
        return AnchorVerdict::Satisfied;
    }

    // Report the reason that actually blocked the response. Ordered by how far the anchors got:
    // naming "too short" when the real problem is overlap would send the model the wrong fix.
    let reason = if overlapping > 0 {
        AnchorMissReason::OverlappingRegions
    } else if not_in_brief > 0 {
        AnchorMissReason::QuoteNotInBrief
    } else if too_short > 0 {
        AnchorMissReason::QuoteTooShort
    } else {
        AnchorMissReason::TooFewAnchors
    };

    AnchorVerdict::Missing {
        anchors_found,
        anchors_valid,
        reason,
        per_anchor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes() -> Vec<String> {
        (1..=10).map(|n| format!("A{n}:")).collect()
    }

    fn suffix_lines() -> Vec<String> {
        vec![
            "Disposition: READY".to_string(),
            "Disposition: ITERATE".to_string(),
            "Disposition: ESCALATE".to_string(),
            "Verdict: GROOMED".to_string(),
            "Verdict: ESCALATE".to_string(),
        ]
    }

    /// A brief with three clearly distinct regions, standing in for a real grooming brief.
    fn brief() -> String {
        "Le plan propose de re-résoudre le token avant chaque cycle du manager de milestones.\n\
         La question 2 porte sur l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ?\n\
         Je n'ai pas fixé N pour le seuil de détection des échecs d'authentification répétés.\n\
         La mise hors périmètre du code 403 doit être justifiée ou retirée du plan.\n"
            .to_string()
    }

    fn verify(text: &str) -> AnchorVerdict {
        verify_review_anchors(text, &brief(), &prefixes(), &suffix_lines(), 3, 40)
    }

    /// The per-line fates projected onto what a caller reads them for: which label, and whether
    /// it was refused. Expectations stay off the excerpt, which has its own tests — hand-counting
    /// 72 characters of French in every assertion would measure the arithmetic, not the matcher.
    fn fates(per_anchor: &[AnchorOutcome]) -> Vec<(&str, Option<AnchorMissReason>)> {
        per_anchor
            .iter()
            .map(|outcome| (outcome.label.as_str(), outcome.refused))
            .collect()
    }

    /// The measured response from mika#2037, verbatim.
    const STUB_2037: &str = "Préférence stockée — le pattern de re-résolution par cycle et le seuil N=3 pour mika#2013.\n\nDisposition: READY\n";

    #[test]
    fn stub_from_2037_is_rejected() {
        let verdict = verify(STUB_2037);
        assert_eq!(
            verdict,
            AnchorVerdict::Missing {
                anchors_found: 0,
                anchors_valid: 0,
                reason: AnchorMissReason::NoAnchorLine,
                // No line to report a fate for. The re-prompt falls back on its generic form.
                per_anchor: Vec::new(),
            },
            "the founding incident must not pass the guard built to catch it"
        );
    }

    #[test]
    fn three_distinct_quotes_are_satisfied() {
        let text = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui, un INFO suffit.\n\
                    A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — je propose 30 minutes.\n\
                    \n\
                    Disposition: READY\n";
        assert_eq!(verify(text), AnchorVerdict::Satisfied);
    }

    /// mika#2786 — the shape of mika#2762's third withheld pass: every label in bold. Until
    /// this fix the guard saw no anchor line at all.
    #[test]
    fn mika2786_bold_labels_are_anchor_lines() {
        let text = "**A1:** le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    **A2:** sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    **A3:** « Je n'ai pas fixé N pour le seuil de détection des échecs » — 30 minutes.\n\
                    \n\
                    Disposition: READY\n";
        assert_eq!(verify(text), AnchorVerdict::Satisfied);
    }

    /// mika#2786 — the negative control of the tolerance: it finds the line, it does not
    /// relax the quote. A bold anchor that paraphrases the brief, or quotes another document,
    /// is refused exactly as an unbolded one is.
    #[test]
    fn mika2786_a_bold_label_does_not_relax_the_verbatim_test() {
        let text = "**A1:** le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    **A2:** sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    **A3:** Le reaper groom introduit par mika#2750 (PR mika#2754) fait échouer le parent.\n\
                    \n\
                    Disposition: READY\n";
        match verify(text) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
                per_anchor,
            } => {
                assert_eq!((anchors_found, anchors_valid), (3, 2));
                assert_eq!(reason, AnchorMissReason::QuoteNotInBrief);
                assert_eq!(
                    fates(&per_anchor),
                    vec![
                        ("A1:", None),
                        ("A2:", None),
                        ("A3:", Some(AnchorMissReason::QuoteNotInBrief)),
                    ],
                    "the label must be the declared prefix, never the `**` around it"
                );
            }
            other => panic!("expected not-in-brief rejection, got {other:?}"),
        }
    }

    #[test]
    fn three_quotes_of_the_same_region_are_rejected() {
        let line = "re-résoudre le token avant chaque cycle du manager de milestones";
        let text = format!("A1: {line}\nA2: {line}\nA3: {line}\n\nDisposition: READY\n");
        match verify(&text) {
            AnchorVerdict::Missing {
                anchors_valid,
                reason,
                ..
            } => {
                assert_eq!(anchors_valid, 1, "only the first claim on a region counts");
                assert_eq!(reason, AnchorMissReason::OverlappingRegions);
            }
            other => panic!("expected overlap rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_quote_absent_from_the_brief_is_rejected() {
        let text = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    A3: cette phrase n'apparaît nulle part dans le brief soumis à la revue architecte.\n\
                    \n\
                    Disposition: READY\n";
        match verify(text) {
            AnchorVerdict::Missing { reason, .. } => {
                assert_eq!(reason, AnchorMissReason::QuoteNotInBrief)
            }
            other => panic!("expected not-in-brief rejection, got {other:?}"),
        }
    }

    #[test]
    fn quote_length_boundary_is_inclusive_at_the_threshold() {
        // A span of the brief cut so neither end lands on whitespace — otherwise normalization
        // would trim it and the test would measure trimming rather than the threshold.
        let source = "Je n'ai pas fixé N pour le seuil de détection des échecs";
        let exact: String = source.chars().take(40).collect();
        let short: String = source.chars().take(39).collect();
        assert_eq!(exact.chars().count(), 40);
        assert!(!exact.ends_with(' ') && !short.ends_with(' '));

        let at_threshold = format!("A1: {exact}\n\nDisposition: READY\n");
        let below_threshold = format!("A1: {short}\n\nDisposition: READY\n");

        // min_count 1 isolates the length check from the count check.
        let at =
            verify_review_anchors(&at_threshold, &brief(), &prefixes(), &suffix_lines(), 1, 40);
        assert_eq!(at, AnchorVerdict::Satisfied, "40 chars must pass at n=40");

        let below = verify_review_anchors(
            &below_threshold,
            &brief(),
            &prefixes(),
            &suffix_lines(),
            1,
            40,
        );
        assert!(
            !below.is_satisfied(),
            "39 chars must fail at n=40, got {below:?}"
        );
    }

    #[test]
    fn two_valid_anchors_do_not_meet_a_minimum_of_three() {
        let text = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    \n\
                    Disposition: READY\n";
        match verify(text) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
                per_anchor,
            } => {
                assert_eq!(anchors_found, 2);
                assert_eq!(anchors_valid, 2);
                assert_eq!(reason, AnchorMissReason::TooFewAnchors);
                assert_eq!(
                    fates(&per_anchor),
                    vec![("A1:", None), ("A2:", None)],
                    "both lines were accepted — the contract failed on the count, not on them"
                );
            }
            other => panic!("expected too-few rejection, got {other:?}"),
        }
    }

    #[test]
    fn anchors_after_the_disposition_line_do_not_count() {
        let text = "Rien à signaler.\n\
                    \n\
                    Disposition: READY\n\
                    A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager ».\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? ».\n\
                    A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — noté.\n";
        match verify(text) {
            AnchorVerdict::Missing { reason, .. } => {
                assert_eq!(reason, AnchorMissReason::NoAnchorLine)
            }
            other => panic!("expected body-scope rejection, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_brief_never_passes_vacuously() {
        let text = "A1: une ligne d'ancrage suffisamment longue pour franchir le seuil de citation.\n\
                    A2: une deuxième ligne d'ancrage suffisamment longue pour franchir ce seuil.\n\
                    A3: une troisième ligne d'ancrage suffisamment longue pour franchir ce seuil.\n\
                    \n\
                    Disposition: READY\n";
        let verdict = verify_review_anchors(text, "", &prefixes(), &suffix_lines(), 3, 40);
        assert!(!verdict.is_satisfied(), "empty brief must fail closed");
    }

    /// mika#2794 — the fate list does not derive from the counters, so the counters must derive
    /// from it. Checked on every branch the verification loop can take: a drift between the two
    /// would have the corrective re-prompt name a line the verdict never refused.
    #[test]
    fn mika2794_the_fate_list_agrees_with_the_counters_on_every_loop_branch() {
        let two_valid = "A1: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                         A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n";
        let same_region = "re-résoudre le token avant chaque cycle du manager de milestones";
        let cases = vec![
            ("too few", format!("{two_valid}\nDisposition: READY\n")),
            (
                "not in brief",
                format!(
                    "{two_valid}A3: cette phrase n'apparaît nulle part dans le brief soumis à la revue.\n\nDisposition: READY\n"
                ),
            ),
            (
                "overlapping",
                format!(
                    "A1: {same_region}\nA2: {same_region}\nA3: {same_region}\n\nDisposition: READY\n"
                ),
            ),
            ("too short", "A1: court\n\nDisposition: READY\n".to_string()),
        ];
        for (name, text) in cases {
            match verify(&text) {
                AnchorVerdict::Missing {
                    anchors_found,
                    anchors_valid,
                    per_anchor,
                    ..
                } => {
                    assert_eq!(
                        per_anchor.len(),
                        anchors_found,
                        "{name}: one fate per prefixed line, in body order"
                    );
                    assert_eq!(
                        per_anchor.iter().filter(|o| o.refused.is_none()).count(),
                        anchors_valid,
                        "{name}: an accepted line is exactly one that claimed a region"
                    );
                }
                other => panic!("{name}: expected a refusal, got {other:?}"),
            }
        }
    }

    /// mika#2794 — the two early returns carry no per-line fate, by construction: no quote was
    /// attempted, so no line has one. The diagnosis paragraph is then absent and the corrective
    /// re-prompt is word for word its pre-mika#2794 self.
    #[test]
    fn mika2794_the_early_returns_carry_no_per_line_fate() {
        match verify(STUB_2037) {
            AnchorVerdict::Missing { per_anchor, .. } => {
                assert!(per_anchor.is_empty(), "no anchor line, no fate to report")
            }
            other => panic!("expected the founding refusal, got {other:?}"),
        }

        let text = "A1: une ligne d'ancrage suffisamment longue pour franchir le seuil de citation.\n\
                    A2: une deuxième ligne d'ancrage suffisamment longue pour franchir ce seuil.\n\
                    A3: une troisième ligne d'ancrage suffisamment longue pour franchir ce seuil.\n\
                    \n\
                    Disposition: READY\n";
        match verify_review_anchors(text, "", &prefixes(), &suffix_lines(), 3, 40) {
            AnchorVerdict::Missing {
                anchors_found,
                per_anchor,
                ..
            } => {
                assert_eq!(
                    anchors_found, 3,
                    "the lines were found before the brief was read"
                );
                assert!(
                    per_anchor.is_empty(),
                    "an empty brief grounds nothing — no line can be said to have failed on its own"
                );
            }
            other => panic!("expected the empty-brief refusal, got {other:?}"),
        }
    }

    /// mika#2794 — the excerpt is cut in **characters**, and advertises an elision only when it
    /// made one. `é` is the whole point: a French brief puts a multi-byte codepoint well inside
    /// the first 72 characters, where byte slicing panics (`scripts/check-byte-slices.sh`).
    #[test]
    fn mika2794_the_excerpt_is_cut_in_chars_and_marks_only_a_real_elision() {
        let excerpt_of =
            |content: &str| match verify(&format!("A1: {content}\n\nDisposition: READY\n")) {
                AnchorVerdict::Missing { per_anchor, .. } => {
                    per_anchor.first().expect("one anchor line").excerpt.clone()
                }
                other => panic!("expected a refusal, got {other:?}"),
            };

        let over = excerpt_of(&"é".repeat(ANCHOR_EXCERPT_CHARS + 10));
        assert_eq!(over.chars().count(), ANCHOR_EXCERPT_CHARS + 1);
        assert!(over.ends_with('…'), "a real elision is marked: {over:?}");

        let exactly = excerpt_of(&"é".repeat(ANCHOR_EXCERPT_CHARS));
        assert_eq!(exactly.chars().count(), ANCHOR_EXCERPT_CHARS);
        assert!(
            !exactly.ends_with('…'),
            "nothing was dropped at the ceiling, so nothing is marked: {exactly:?}"
        );
    }

    /// mika#2794, the measured shape (sessions `49ffa230` and `a7f1ccad`, 2026-10-18): `A1`
    /// quotes the opening sentence of the ticket body read by `gh_read`, `A2` and `A3` quote the
    /// turn's brief verbatim at disjoint regions. Three anchors found, two valid — and the
    /// verdict now says it is the **first** one that failed, which is the single piece of
    /// information the corrective re-prompt had no way to name. `a7f1ccad` returned the same
    /// faulty anchor three times.
    #[test]
    fn mika2794_the_measured_shape_names_the_first_anchor() {
        let text = "A1: Le reaper groom introduit par mika#2750 (PR mika#2754) fait échouer le parent avant la relance.\n\
                    A2: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                    A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — je propose 30 minutes.\n\
                    \n\
                    Disposition: READY\n";
        match verify(text) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
                per_anchor,
            } => {
                assert_eq!(
                    (anchors_found, anchors_valid),
                    (3, 2),
                    "the shape replayed offline on the real 2026-10-18 passes"
                );
                assert_eq!(reason, AnchorMissReason::QuoteNotInBrief);
                assert_eq!(
                    fates(&per_anchor),
                    vec![
                        ("A1:", Some(AnchorMissReason::QuoteNotInBrief)),
                        ("A2:", None),
                        ("A3:", None),
                    ]
                );
                assert!(
                    per_anchor[0]
                        .excerpt
                        .starts_with("Le reaper groom introduit par mika#2750"),
                    "the excerpt must open on the faulty line itself, got {:?}",
                    per_anchor[0].excerpt
                );
            }
            other => panic!("expected the measured refusal, got {other:?}"),
        }
    }

    #[test]
    fn line_wrapping_in_the_brief_does_not_break_a_genuine_quote() {
        // The brief holds the sentence across a line break; the reviewer quotes it on one line.
        let wrapped_brief =
            "Le plan propose de re-résoudre\nle token avant chaque cycle du manager de milestones.";
        let text = "A1: le plan propose de re-résoudre le token avant chaque cycle du manager\n\n\
                    Disposition: READY\n";
        let verdict =
            verify_review_anchors(text, wrapped_brief, &prefixes(), &suffix_lines(), 1, 40);
        assert_eq!(
            verdict,
            AnchorVerdict::Satisfied,
            "a reviewer re-flows what they quote; the guard must not reject that"
        );
    }

    #[test]
    fn multibyte_quotes_do_not_panic() {
        // Accented French and emoji at the window boundary — byte slicing would panic here.
        let unicode_brief = "Décision — la référence « éàüñ 🔒 » doit être conservée telle quelle dans le rapport final.";
        let text = "A1: la référence « éàüñ 🔒 » doit être conservée telle quelle dans le rapport\n\n\
                    Disposition: READY\n";
        let verdict =
            verify_review_anchors(text, unicode_brief, &prefixes(), &suffix_lines(), 1, 40);
        assert!(verdict.is_satisfied(), "got {verdict:?}");
    }

    /// The guard runs on every non-terminal EndTurn of a verdict producer, so its cost has to
    /// be measured rather than assumed. Worst realistic shape: a 10 KB brief (the size of the
    /// one in mika#2037), the maximum ten anchor lines, and none of them quoting it — the path
    /// that scans every window of every line against the whole brief before giving up.
    ///
    /// The bound is deliberately loose. This is a guard against an algorithmic regression
    /// (someone reaching for a longest-common-substring scan), not a latency SLO, and a tight
    /// bound would be flaky on a loaded CI runner.
    #[test]
    fn worst_case_cost_is_bounded() {
        let brief: String = std::iter::repeat_n(
            "Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer au spawn. ",
            120,
        )
        .collect();
        assert!(
            brief.len() > 10_000,
            "brief should be ~10 KB, got {}",
            brief.len()
        );

        let mut text = String::new();
        for n in 1..=10 {
            text.push_str(&format!(
                "A{n}: cette ligne d'ancrage ne cite rien du brief et doit donc être parcourue \
                 en entier avant d'être rejetée, fenêtre par fenêtre.\n"
            ));
        }
        text.push_str("\nDisposition: READY\n");

        let start = std::time::Instant::now();
        let verdict = verify_review_anchors(&text, &brief, &prefixes(), &suffix_lines(), 3, 40);
        let elapsed = start.elapsed();

        assert!(!verdict.is_satisfied());
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "worst-case verification took {elapsed:?} — an algorithmic regression, not a slow \
             machine, is the thing this bound catches"
        );
        println!("review-anchor worst case (10 KB brief, 10 non-quoting anchors): {elapsed:?}");
    }

    /// Region disjointness alone is not enough. A brief sentence longer than twice the quote
    /// threshold yields non-overlapping windows, so pasting ONE long line three times would
    /// otherwise clear a contract that exists to require three reads of the document.
    #[test]
    fn one_long_line_pasted_three_times_is_rejected() {
        // 96 characters lifted from the brief — well over 2 x 40, so its windows can land on
        // disjoint regions.
        let long = "Le plan propose de re-résoudre le token avant chaque cycle du manager de milestones. La question 2";
        assert!(long.chars().count() > 80);
        let text = format!("A1: {long}\nA2: {long}\nA3: {long}\n\nDisposition: READY\n");
        match verify(&text) {
            AnchorVerdict::Missing {
                anchors_valid,
                reason,
                ..
            } => {
                assert_eq!(anchors_valid, 1, "one read of the brief is one anchor");
                assert_eq!(reason, AnchorMissReason::OverlappingRegions);
            }
            other => panic!("expected rejection of the paste-three-times shape, got {other:?}"),
        }
    }

    /// The complement of the rule above: an early anchor must not be able to steal the region
    /// a later one genuinely quotes. Before the fix, the first matching window was claimed
    /// unconditionally, so a legitimate three-anchor review could be rejected.
    #[test]
    fn an_early_anchor_does_not_steal_a_later_anchors_region() {
        // A1 quotes a span that also occurs inside the region A2 needs; A1 has another
        // usable window, so both must resolve.
        let text = "A1: La question 2 porte sur l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? Le plan propose de re-résoudre le token avant chaque cycle.\n\
                    A2: Le plan propose de re-résoudre le token avant chaque cycle du manager de milestones.\n\
                    A3: Je n'ai pas fixé N pour le seuil de détection des échecs d'authentification.\n\
                    \n\
                    Disposition: READY\n";
        assert_eq!(
            verify(text),
            AnchorVerdict::Satisfied,
            "three genuine anchors must not be rejected because of claim ordering"
        );
    }

    /// The body window ends at the LAST disposition line, not the first. The shipped prompt
    /// now shows a worked READY example, so a response that quotes the contract before
    /// answering is an ordinary shape — truncating at the first occurrence would drop every
    /// anchor of a fully anchored review.
    #[test]
    fn a_disposition_quoted_early_does_not_truncate_the_body() {
        let text = "Le contrat demande de terminer par `Disposition: READY`. Voici la revue.\n\
                    Disposition: READY\n\
                    A1: \"Le plan propose de re-résoudre le token avant chaque cycle du manager\" — correct.\n\
                    A2: \"l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement\" — oui.\n\
                    A3: \"Je n'ai pas fixé N pour le seuil de détection des échecs\" — en durée.\n\
                    \n\
                    Disposition: READY\n";
        assert_eq!(
            verify(text),
            AnchorVerdict::Satisfied,
            "an early echo of the disposition must not hide the body from the guard"
        );
    }

    /// The measured brief excerpt of mika#2338 (message 87416, the mika#2335 grooming brief):
    /// three regions, each carrying inline markdown — bold and code spans — as every brief
    /// written by a planning agent does (237 of 245 briefs since 2026-09-01 carry `**`, 243
    /// carry backticks).
    const BRIEF_2338_MARKDOWN: &str = "\
**Le mécanisme de kill, lui, est correct** : `kill_process_gracefully` (`process_kill.rs:82`)\n\
signale bien le groupe (`kill -TERM -<pid>`, l.53-58), possible parce que le spawn fait du\n\
fils un chef de groupe (`.process_group(0)`, `executor.rs:3201`).\n\
\n\
**Le chaînon manquant existe déjà et est testé.** `find_dispatch_children_with_pid(parent_task_id)`\n\
(`db.rs:7843-7910`, wrapper `async_db.rs:1023`) fait exactement la traversée parent → enfant-avec-pgid.\n\
\n\
- **AC1** — Une supersession qui annule un parent portant un dispatch **vivant** signale le\n\
  process du fils (par groupe) et le marque terminal.\n";

    /// The measured architect response of mika#2338 (`llm_calls` 8abf1b1b, session 975d44d0,
    /// 2026-09-16T07:32:11Z): the model quoted the *rendered* text — bold markers and code
    /// spans dropped — so A1 had no 40-character window in the brief while A2 and A3 did.
    /// The engine logged `anchors_found=3 anchors_valid=2 miss_reason=QuoteNotInBrief`.
    const RESPONSE_2338_REAL: &str = "\
A1: \"Le mécanisme de kill, lui, est correct : kill_process_gracefully (process_kill.rs:82)\n\
signale bien le groupe (kill -TERM -<pid>, l.53-58)\" — le plan ne réécrit pas ce qui marche.\n\
\n\
A2: \"Le chaînon manquant existe déjà et est testé. find_dispatch_children_with_pid(parent_task_id)\n\
(db.rs:7843-7910, wrapper async_db.rs:1023) fait exactement la traversée\" — réutilisation directe.\n\
\n\
A3: \"AC1 — Une supersession qui annule un parent portant un dispatch vivant signale le\n\
process du fils (par groupe) et le marque terminal.\" — critère testable.\n\
\n\
Disposition: READY\n";

    fn verify_2338(text: &str) -> AnchorVerdict {
        verify_review_anchors(
            text,
            BRIEF_2338_MARKDOWN,
            &prefixes(),
            &suffix_lines(),
            3,
            40,
        )
    }

    /// The founding case of mika#2338, verbatim: a genuine review of the brief, refused on
    /// markup alone. Must pass — the contract is the words of the brief, not its bytes.
    #[test]
    fn mika2338_rendered_quote_of_a_markdown_brief_is_satisfied() {
        assert_eq!(
            verify_2338(RESPONSE_2338_REAL),
            AnchorVerdict::Satisfied,
            "the mika#2335 architect quoted the brief word for word; only `**` and backticks \
             differed"
        );
    }

    /// Symmetry: a brief written plain and an anchor that *adds* markup must match too, so the
    /// normalization is not a one-way concession.
    #[test]
    fn mika2338_markup_in_the_anchor_against_a_plain_brief_is_satisfied() {
        let text = "A1: le plan propose bien de **re-résoudre le token avant chaque cycle du manager** — correct.\n\
                    A2: sur `l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ?` — oui.\n\
                    A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — je propose 30 minutes.\n\
                    \n\
                    Disposition: READY\n";
        assert_eq!(verify(text), AnchorVerdict::Satisfied);
    }

    /// Typographic apostrophes (2 of 199 architect responses since 2026-09-01) fold onto the
    /// ASCII one, in both directions.
    #[test]
    fn mika2338_apostrophe_variants_fold_together() {
        let curly_anchor = "A1: sur « l\u{2019}exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                            A2: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                            A3: « Je n\u{2019}ai pas fixé N pour le seuil de détection des échecs » — 30 minutes.\n\
                            \n\
                            Disposition: READY\n";
        assert_eq!(verify(curly_anchor), AnchorVerdict::Satisfied);

        let curly_brief = brief().replace('\'', "\u{2019}");
        let ascii_anchor = "A1: sur « l'exhaustivité du tracé : faut-il journaliser chaque rafraîchissement ? » — oui.\n\
                            A2: le plan propose bien de « re-résoudre le token avant chaque cycle du manager » — correct.\n\
                            A3: « Je n'ai pas fixé N pour le seuil de détection des échecs » — 30 minutes.\n\
                            \n\
                            Disposition: READY\n";
        assert_eq!(
            verify_review_anchors(
                ascii_anchor,
                &curly_brief,
                &prefixes(),
                &suffix_lines(),
                3,
                40
            ),
            AnchorVerdict::Satisfied
        );
    }

    /// Markup cannot buy length: the quote threshold is measured after normalization, so a
    /// 44-character anchor whose markers account for 6 of them is a 38-character quote.
    #[test]
    fn mika2338_markup_does_not_count_toward_the_quote_length() {
        // 38 real characters of the brief, dressed up to 44 with `**` and backticks.
        let span: String = "Je n'ai pas fixé N pour le seuil de dét"
            .chars()
            .take(38)
            .collect();
        assert_eq!(span.chars().count(), 38);
        let dressed = format!("**{span}**``");
        assert_eq!(dressed.chars().count(), 44);
        let text = format!("A1: {dressed}\n\nDisposition: READY\n");
        let verdict = verify_review_anchors(&text, &brief(), &prefixes(), &suffix_lines(), 1, 40);
        assert!(
            !verdict.is_satisfied(),
            "38 real characters must not pass n=40 because of 6 markup characters, got {verdict:?}"
        );
    }

    /// Normalization is not a paraphrase licence: the same brief, the same words changed.
    #[test]
    fn mika2338_paraphrase_of_a_markdown_brief_is_still_rejected() {
        let text = "A1: \"Le mécanisme de kill est correct : kill_process_gracefully signale le groupe\" — ok.\n\
                    A2: \"Le chaînon manquant existe déjà et est testé. find_dispatch_children_with_pid(parent_task_id)\" — ok.\n\
                    A3: \"AC1 — Une supersession qui annule un parent portant un dispatch vivant signale le\" — ok.\n\
                    \n\
                    Disposition: READY\n";
        match verify_2338(text) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
                per_anchor,
            } => {
                assert_eq!((anchors_found, anchors_valid), (3, 2));
                assert_eq!(reason, AnchorMissReason::QuoteNotInBrief);
                assert_eq!(
                    fates(&per_anchor),
                    vec![
                        ("A1:", Some(AnchorMissReason::QuoteNotInBrief)),
                        ("A2:", None),
                        ("A3:", None),
                    ],
                    "the dropped `, lui,` is on the FIRST line — the information mika#2794 adds"
                );
            }
            other => panic!("expected not-in-brief rejection, got {other:?}"),
        }
    }

    /// The other measured class of mika#2338 (session 09b565bb): three exact quotes of a
    /// *different* ticket's brief, read from the agent-wide history window. Normalization
    /// must not let a wrong-brief attestation through — that refusal was the guard working.
    #[test]
    fn mika2338_exact_quotes_of_another_brief_are_still_rejected() {
        let text = "A1: \"La résolution retenue est l'option 2 : reconstruction explicite, parce que `load_for_agent` a déjà fusionné les sources\"\n\
                    A2: \"**Geste d'opérateur requis, et non automatisable :** si M2 se confirme, une variable `MIKA_LLM_HTTP_TIMEOUT_SECS`\"\n\
                    A3: \"**Ce que la garde ne fait pas :** elle ne corrige aucune valeur et n'en invente aucune. Un couple invalide\"\n\
                    \n\
                    Verdict: GROOMED\n";
        match verify_2338(text) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
                per_anchor,
            } => {
                assert_eq!((anchors_found, anchors_valid), (3, 0));
                assert_eq!(reason, AnchorMissReason::QuoteNotInBrief);
                assert_eq!(
                    fates(&per_anchor),
                    vec![
                        ("A1:", Some(AnchorMissReason::QuoteNotInBrief)),
                        ("A2:", Some(AnchorMissReason::QuoteNotInBrief)),
                        ("A3:", Some(AnchorMissReason::QuoteNotInBrief)),
                    ],
                    "a wrong-brief attestation refuses every line, and says so line by line"
                );
            }
            other => panic!("expected not-in-brief rejection, got {other:?}"),
        }
    }

    #[test]
    fn whitespace_only_windows_are_not_quotes() {
        // A line of padding must not satisfy the contract by matching a run of spaces.
        let padded_brief =
            "début                                                                    fin";
        let text = "A1:                                                                          \n\n\
                    Disposition: READY\n";
        let verdict =
            verify_review_anchors(text, padded_brief, &prefixes(), &suffix_lines(), 1, 40);
        assert!(!verdict.is_satisfied(), "got {verdict:?}");
    }
}

/// The two-directional measurement matrix for the thresholds (mika#2037 U7).
///
/// The guard has to satisfy two conditions that pull against each other: a real review must
/// ALWAYS pass, and a stub like the one measured in mika#2037 must be refused. The second
/// alone is satisfied by "refuse everything", which would block grooming entirely — so both
/// columns are measured, and the thresholds are whatever separates them, not whatever seemed
/// reasonable in prose.
///
/// Provenance of the accept column matters: a case written to pass the guard proves nothing
/// (`a-stub-built-from-the-doc-cannot-falsify-the-doc`). `ready_example_from_the_shipped_prompt`
/// is read from `mika-arch-groom-ticket/system_prompt.md` on disk — it is the response the
/// repo actually instructs the architect to produce, so a future prompt edit that drifts from
/// the guard fails here rather than in production.
#[cfg(test)]
mod matrix {
    use super::*;
    use std::path::Path;

    /// The brief these cases are reviewed against — a stand-in for the 10 492-byte brief of
    /// the founding incident, carrying its four numbered questions.
    const BRIEF: &str = "Plan de renouvellement du token du manager de milestones.\n\
        Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer au spawn.\n\
        Question 1 : où placer le correctif, dans le chemin de spawn ou dans le corps du cycle ?\n\
        Question 2 : le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement ?\n\
        Question 3 : je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés.\n\
        Question 4 : la mise hors périmètre du code 403 est-elle sûre pour ce jalon ?\n";

    /// Case provenance, so the matrix says which evidence is real and which is constructed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Origin {
        /// Measured in the field, or read from a shipped file in this repo.
        Real,
        /// Written for this matrix to probe a specific evasion shape.
        Constructed,
    }

    struct Case {
        name: &'static str,
        origin: Origin,
        text: String,
    }

    fn prefixes() -> Vec<String> {
        (1..=10).map(|n| format!("A{n}:")).collect()
    }

    fn suffix_lines() -> Vec<String> {
        vec![
            "Disposition: READY".to_string(),
            "Disposition: ITERATE".to_string(),
            "Disposition: ESCALATE".to_string(),
            "Verdict: GROOMED".to_string(),
            "Verdict: ESCALATE".to_string(),
        ]
    }

    /// Lift the worked READY example out of the shipped prompt, and re-anchor its quotes onto
    /// BRIEF. The prompt's own example quotes a different brief; what is under test is its
    /// SHAPE — three prefixed lines each carrying a long verbatim span — not its content.
    fn ready_example_shape_from_shipped_prompt() -> Option<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../skills/bundled/mika-arch-groom-ticket/system_prompt.md");
        let prompt = std::fs::read_to_string(path).ok()?;
        let example = prompt.split("#### Disposition: READY example").nth(1)?;
        let block = example.split("```").nth(1)?;
        // Count the anchors the shipped example carries, then build a response of that shape
        // whose quotes come from BRIEF.
        let anchor_count = (1..=10)
            .filter(|n| block.contains(&format!("A{n}:")))
            .count();
        if anchor_count == 0 {
            return None;
        }
        let quotes = [
            "Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer",
            "le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement",
            "je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés",
            "la mise hors périmètre du code 403 est-elle sûre pour ce jalon",
            "où placer le correctif, dans le chemin de spawn ou dans le corps du cycle",
        ];
        let mut out = String::new();
        for (i, q) in quotes
            .iter()
            .take(anchor_count.min(quotes.len()))
            .enumerate()
        {
            out.push_str(&format!("A{}: \"{q}\" — traité.\n", i + 1));
        }
        out.push_str("\nDisposition: READY\n");
        Some(out)
    }

    fn must_reject() -> Vec<Case> {
        let mut cases = vec![
            Case {
                name: "stub_2037_verbatim",
                origin: Origin::Real,
                text: "Préférence stockée — le pattern de re-résolution par cycle et le seuil \
                       N=3 pour mika#2013.\n\nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "single_long_quote_only",
                origin: Origin::Constructed,
                text: "A1: Le plan re-résout le token du cycle avant chaque cycle au lieu de le \
                       figer au spawn.\n\nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "three_anchors_same_region",
                origin: Origin::Constructed,
                text: {
                    let line = "Le plan re-résout le token du cycle avant chaque cycle au lieu \
                                de le figer au spawn";
                    format!("A1: {line}\nA2: {line}\nA3: {line}\n\nDisposition: READY\n")
                },
            },
            Case {
                name: "three_anchors_paraphrase_only",
                origin: Origin::Constructed,
                text: "A1: le token est renouvelé à chaque tour de boucle plutôt qu'une seule fois.\n\
                       A2: la journalisation des rafraîchissements paraît suffisante en l'état.\n\
                       A3: le seuil d'échec devrait être exprimé autrement qu'en nombre de tours.\n\
                       \nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "anchors_below_the_disposition_line",
                origin: Origin::Constructed,
                text: "Rien à signaler.\n\nDisposition: READY\n\
                       A1: Le plan re-résout le token du cycle avant chaque cycle au lieu de le figer.\n\
                       A2: le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement ?\n\
                       A3: je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés.\n"
                    .to_string(),
            },
        ];
        // Padding is not a quote: a wall of spaces must not buy an anchor.
        cases.push(Case {
            name: "whitespace_padding_anchors",
            origin: Origin::Constructed,
            text: format!(
                "A1:{pad}\nA2:{pad}\nA3:{pad}\n\nDisposition: READY\n",
                pad = " ".repeat(80)
            ),
        });
        cases
    }

    fn must_accept() -> Vec<Case> {
        let mut cases = vec![
            Case {
                name: "three_dispersed_quotes",
                origin: Origin::Constructed,
                text: "A1: \"Le plan re-résout le token du cycle avant chaque cycle au lieu de le \
                       figer\" — le corps du cycle est le bon endroit.\n\
                       A2: \"le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement\" \
                       — un INFO par changement suffit.\n\
                       A3: \"je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés\" \
                       — exprime-le en durée, pas en nombre de cycles.\n\
                       \nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "four_quotes_one_per_question",
                origin: Origin::Constructed,
                text: "A1: \"où placer le correctif, dans le chemin de spawn ou dans le corps du \
                       cycle\" — dans le corps du cycle.\n\
                       A2: \"le tracé est-il exhaustif, faut-il journaliser chaque rafraîchissement\" \
                       — oui, au niveau INFO.\n\
                       A3: \"je n'ai pas fixé N pour le seuil d'échecs d'authentification répétés\" \
                       — 30 minutes.\n\
                       A4: \"la mise hors périmètre du code 403 est-elle sûre pour ce jalon\" — oui, \
                       403 est une forme de permission, pas d'expiration.\n\
                       \nDisposition: READY\n"
                    .to_string(),
            },
            Case {
                name: "quotes_reflowed_across_prose",
                origin: Origin::Constructed,
                text: "Revue complète ci-dessous.\n\n\
                       A1: sur le placement — \"Le plan re-résout le token du cycle avant chaque \
                       cycle au lieu de le figer au spawn.\" C'est correct.\n\
                       Le reste du raisonnement tient.\n\n\
                       A2: sur le tracé — \"Question 2 : le tracé est-il exhaustif, faut-il \
                       journaliser chaque rafraîchissement ?\" Oui.\n\n\
                       A3: sur le seuil — \"Question 3 : je n'ai pas fixé N pour le seuil d'échecs \
                       d'authentification répétés.\" À exprimer en durée.\n\n\
                       Disposition: READY\n"
                    .to_string(),
            },
        ];
        if let Some(text) = ready_example_shape_from_shipped_prompt() {
            cases.push(Case {
                name: "ready_example_from_the_shipped_prompt",
                origin: Origin::Real,
                text,
            });
        }
        cases
    }

    fn separates(min_count: usize, min_quote_chars: usize) -> Result<(), String> {
        for case in must_reject() {
            let v = verify_review_anchors(
                &case.text,
                BRIEF,
                &prefixes(),
                &suffix_lines(),
                min_count,
                min_quote_chars,
            );
            if v.is_satisfied() {
                return Err(format!(
                    "reject-case '{}' ({:?}) passed at ({min_count}, {min_quote_chars})",
                    case.name, case.origin
                ));
            }
        }
        for case in must_accept() {
            let v = verify_review_anchors(
                &case.text,
                BRIEF,
                &prefixes(),
                &suffix_lines(),
                min_count,
                min_quote_chars,
            );
            if !v.is_satisfied() {
                return Err(format!(
                    "accept-case '{}' ({:?}) failed at ({min_count}, {min_quote_chars}): {v:?}",
                    case.name, case.origin
                ));
            }
        }
        Ok(())
    }

    /// The shipped thresholds separate both columns. This is the assertion the manifests'
    /// declared values rest on.
    #[test]
    fn shipped_thresholds_separate_both_columns() {
        if let Err(e) = separates(3, 40) {
            panic!("the shipped thresholds (3, 40) do not separate the matrix: {e}");
        }
    }

    /// Sweep the neighbourhood and report which pairs separate. This is what turns a chosen
    /// number into a measured one: if (3, 40) were the only separating pair it would be a
    /// knife edge, and if none separated, KTD1 would be invalid.
    #[test]
    fn threshold_sweep_shows_a_separating_region_not_a_knife_edge() {
        let mut separating = Vec::new();
        for min_count in 1..=5usize {
            for min_quote_chars in [16usize, 24, 32, 40, 56, 72] {
                if separates(min_count, min_quote_chars).is_ok() {
                    separating.push((min_count, min_quote_chars));
                }
            }
        }
        // Printed so `--nocapture` shows the measured region, not just a pass/fail.
        println!("review-anchor separating (min_count, min_quote_chars): {separating:?}");
        assert!(
            !separating.is_empty(),
            "no threshold pair separates the two columns — the anchor design (KTD1) would be \
             invalid and must be routed to the operator, not patched with a different number"
        );
        assert!(
            separating.contains(&(3, 40)),
            "the shipped pair is not in the separating region: {separating:?}"
        );
        // A single separating pair would mean the thresholds sit on a knife edge and any
        // wording change in a real review would tip a genuine READY into refusal.
        assert!(
            separating.len() >= 3,
            "only {} separating pair(s) — too narrow to be robust: {separating:?}",
            separating.len()
        );
    }

    /// `min_count = 1` must NOT separate: one anchor is crossable by copying a single line of
    /// the brief, which is the obvious evasion once anchors are required at all. This is the
    /// measurement behind KTD2's choice of 3 over 1.
    #[test]
    fn a_single_anchor_does_not_separate() {
        assert!(
            separates(1, 40).is_err(),
            "min_count = 1 separated the columns, which contradicts the reason KTD2 chose 3 — \
             re-derive the threshold rather than keeping the written one"
        );
    }

    /// Inversion: the non-overlap constraint carries weight. Three anchors quoting the same
    /// region are found but only one survives — without R3 the count would reach 3 and the
    /// case would pass.
    #[test]
    fn non_overlap_constraint_is_load_bearing() {
        let case = must_reject()
            .into_iter()
            .find(|c| c.name == "three_anchors_same_region")
            .expect("the overlap case must stay in the matrix");
        match verify_review_anchors(&case.text, BRIEF, &prefixes(), &suffix_lines(), 3, 40) {
            AnchorVerdict::Missing {
                anchors_found,
                anchors_valid,
                reason,
                ..
            } => {
                assert_eq!(anchors_found, 3, "three prefixed lines were emitted");
                assert_eq!(
                    anchors_valid, 1,
                    "only one region may be claimed — without the non-overlap rule this would \
                     be 3 and the case would pass"
                );
                assert_eq!(reason, AnchorMissReason::OverlappingRegions);
            }
            other => panic!("expected overlap rejection, got {other:?}"),
        }
    }

    /// The accept column must contain at least one case whose provenance is real, or the
    /// matrix only proves the guard agrees with itself.
    #[test]
    fn the_accept_column_carries_real_provenance() {
        let real = must_accept()
            .into_iter()
            .filter(|c| c.origin == Origin::Real)
            .count();
        assert!(
            real >= 1,
            "no real-provenance accept case — the shipped prompt's READY example could not be \
             read, so the matrix measures only constructed inputs"
        );
    }
}

/// mika#2996 — the region of a retry brief that anchors are checked against.
#[cfg(test)]
mod mika2996 {
    use super::*;

    const PLAN: &str = "Le plan re-résout le token du cycle avant chaque cycle au lieu de le \
        figer au spawn, et garde le jeton précédent quand la re-résolution échoue.\n";

    const CORRECTIVE: &str = "Your previous plan-review response is missing the required \
        `Disposition:` line.\n\nPlease re-emit your findings and end with exactly ONE of these \
        three lines as the last non-empty line of your response:\n\n    Disposition: READY\n    \
        Disposition: ITERATE\n    Disposition: ESCALATE\n\nThe routing engine parses this line \
        as the verdict — its absence blocks the pipeline (see mika#1823).\n";

    fn enveloped(plan: &str) -> String {
        format!("{CORRECTIVE}\n{RETRY_PLAN_BEGIN}\n{plan}{RETRY_PLAN_END}\n")
    }

    #[test]
    fn mika2996_an_enveloped_brief_reduces_to_the_plan() {
        let brief = enveloped(PLAN);
        let reference = anchor_reference(&brief);
        assert_eq!(reference.trim(), PLAN.trim());
        assert!(!reference.contains("missing the required"));
    }

    /// Negative control of the reduction: a first pass has no envelope, and its whole
    /// message is the plan.
    #[test]
    fn mika2996_a_brief_without_the_envelope_is_returned_whole() {
        assert_eq!(anchor_reference(PLAN), PLAN);
        assert_eq!(anchor_reference(CORRECTIVE), CORRECTIVE);
    }

    /// Half an envelope does not disarm the guard by producing an empty region.
    #[test]
    fn mika2996_half_an_envelope_keeps_the_whole_brief() {
        let opened_only = format!("{CORRECTIVE}\n{RETRY_PLAN_BEGIN}\n{PLAN}");
        assert_eq!(anchor_reference(&opened_only), opened_only);
        let closed_only = format!("{CORRECTIVE}\n{PLAN}{RETRY_PLAN_END}\n");
        assert_eq!(anchor_reference(&closed_only), closed_only);
    }

    /// A plan that quotes the closing marker keeps it inside the region: the LAST closing
    /// marker bounds it.
    #[test]
    fn mika2996_a_plan_quoting_the_marker_stays_whole() {
        let plan = format!("Avant.\n{RETRY_PLAN_END}\nAprès, toujours dans le plan.\n");
        let brief = enveloped(&plan);
        assert_eq!(anchor_reference(&brief).trim(), plan.trim());
    }

    /// Three anchors that quote only the corrective instruction — three distinct 40-character
    /// regions of the message — satisfy the contract against the whole message, and are
    /// refused against the plan. This pair is why the envelope exists.
    #[test]
    fn mika2996_anchors_quoting_the_corrective_text_are_refused_against_the_plan() {
        let response = "A1: \"Your previous plan-review response is missing the required\" — vu.\n\
             A2: \"end with exactly ONE of these three lines as the last non-empty line\" — vu.\n\
             A3: \"The routing engine parses this line as the verdict — its absence blocks\" — vu.\n\
             \n\
             Disposition: READY";
        let prefixes: Vec<String> = (1..=10).map(|n| format!("A{n}:")).collect();
        let suffix = vec!["Disposition: READY".to_string()];
        let brief = enveloped(PLAN);

        let against_whole = verify_review_anchors(response, &brief, &prefixes, &suffix, 3, 40);
        assert!(
            against_whole.is_satisfied(),
            "control: the three quotes must be valid against the whole message, or this test \
             proves nothing about the envelope — got {against_whole:?}"
        );

        let against_plan = verify_review_anchors(
            response,
            anchor_reference(&brief),
            &prefixes,
            &suffix,
            3,
            40,
        );
        assert!(
            !against_plan.is_satisfied(),
            "anchors quoting the engine's own corrective text must not attest a review of the plan"
        );
    }
}

/// mika#3002 — la région complémentaire : ce qui, dans un brief de revue, s'adresse à
/// l'agent plutôt qu'au relecteur.
#[cfg(test)]
mod mika3002 {
    use super::*;

    /// Un plan de groom ordinaire : il parle de reprise parce que c'est son sujet, et il
    /// cite un jalon. Les deux conjoints de `detect_resume_intent` sont là, et aucune
    /// intention de reprise n'est exprimée — c'est la forme mesurée le 2026-10-31
    /// (trace `3a066c59`).
    const PLAN: &str = "## 4. Ordre de vol\n\nLe premier vol livre le détecteur ; il faudra \
        reprendre le vol suivant une fois la sonde relevée.\n\nLié : milestone #33, dont ce \
        ticket est l'enfant.\n";

    /// Ce que la relance de mika#1823 écrit avant l'enveloppe : l'engin parlant à l'agent.
    const CORRECTIVE: &str = "Your previous plan-review response is missing the required \
        `Disposition:` line.\n";

    /// La forme que le shell écrit : chaque marqueur seul sur sa ligne, le plan entre les
    /// deux, et ce que la relance ajoute avant et après.
    fn enveloped(head: &str, plan: &str, tail: &str) -> String {
        format!("{head}{RETRY_PLAN_BEGIN}\n{plan}{RETRY_PLAN_END}\n{tail}")
    }

    #[test]
    fn mika3002_deux_marqueurs_rendent_la_tete_et_la_queue() {
        let brief = enveloped(CORRECTIVE, PLAN, "Post-scriptum : rien à ajouter.\n");
        let (head, tail) = outside_reviewed_plan(&brief);
        assert_eq!(head, CORRECTIVE);
        assert_eq!(tail, "\nPost-scriptum : rien à ajouter.\n");
        // La raison d'être de la fonction : les mots du plan ne sont dans aucune des deux.
        for region in [head, tail] {
            assert!(
                !region.contains("reprendre") && !region.contains("milestone #33"),
                "le plan revu a fui dans la région adressée à l'agent : {region:?}"
            );
        }
    }

    /// Aujourd'hui `_append_retry_plan_envelope` n'écrit rien après la fermeture : la queue
    /// ne porte que la fin de la ligne-marqueur, et la tête porte tout le reste.
    #[test]
    fn mika3002_sans_post_scriptum_la_queue_est_vide() {
        let brief = enveloped(CORRECTIVE, PLAN, "");
        assert_eq!(outside_reviewed_plan(&brief), (CORRECTIVE, "\n"));
    }

    /// Zéro marqueur — une première passe, dont le message EST le plan. Rien n'y est
    /// adressé à l'agent, donc les deux régions sont vides. C'est exactement le cas qui
    /// désarme la garde de reprise sur l'incident `3a066c59`.
    #[test]
    fn mika3002_zero_marqueur_ne_rend_aucune_region() {
        assert_eq!(outside_reviewed_plan(PLAN), ("", ""));
        assert_eq!(outside_reviewed_plan(""), ("", ""));
    }

    /// Un seul marqueur — enveloppe mutilée, donc le brief entier, dans les deux sens.
    /// Fail-closed : la garde appelante reste armée plutôt que de se taire sur un défaut
    /// du shell.
    #[test]
    fn mika3002_un_seul_marqueur_rend_le_brief_entier() {
        let opened_only = format!("{CORRECTIVE}{RETRY_PLAN_BEGIN}\n{PLAN}");
        assert_eq!(
            outside_reviewed_plan(&opened_only),
            (opened_only.as_str(), "")
        );
        let closed_only = format!("{CORRECTIVE}{PLAN}{RETRY_PLAN_END}\n");
        assert_eq!(
            outside_reviewed_plan(&closed_only),
            (closed_only.as_str(), "")
        );
    }

    /// Les deux fonctions découpent le même brief sur les mêmes marqueurs : ce que l'une
    /// rend, l'autre ne le rend pas. Si une seule des deux change de convention de
    /// marqueur, cette paire rougit.
    #[test]
    fn mika3002_les_deux_regions_sont_complementaires() {
        let brief = enveloped(CORRECTIVE, PLAN, "Queue.\n");
        let plan = anchor_reference(&brief);
        let (head, tail) = outside_reviewed_plan(&brief);
        assert_eq!(plan.trim(), PLAN.trim());
        assert!(!plan.contains(CORRECTIVE));
        assert!(!plan.contains("Queue."));
        assert_eq!(
            head.len() + RETRY_PLAN_BEGIN.len() + plan.len() + RETRY_PLAN_END.len() + tail.len(),
            brief.len(),
            "les deux découpes doivent partitionner le brief, marqueurs exclus"
        );
    }
}

/// mika#3003 — le registre de lecture du tour et l'attribution d'une ancre refusée.
#[cfg(test)]
mod mika3003 {
    use super::*;

    /// La forme mesurée le 2026-10-31 : le corps du ticket que l'architecte a lu avec
    /// `gh_read issue_view`, et dont il a cité la première phrase au lieu de citer le plan.
    const TICKET_BODY: &str = "La garde d'ancrage de mika#2037 retient une disposition de \
        mika-arch quand ses citations ne viennent pas du brief, c'est-à-dire du plan revu.\n\n\
        Le 2026-10-31, l'architecte a cité le **corps du ticket**, qu'il avait lu avec \
        `gh_read issue_view`, au lieu du plan.\n";

    fn refused(excerpt: &str, reason: AnchorMissReason) -> AnchorOutcome {
        AnchorOutcome {
            label: "A1:".to_string(),
            excerpt: excerpt.to_string(),
            refused: Some(reason),
        }
    }

    /// Le `review_anchor_min_quote_chars` que les trois manifestes arch déclarent.
    const MIN_QUOTE: usize = 40;

    /// `attribute` au seuil de production.
    fn attribue(ledger: &ToolReadLedger, outcome: &AnchorOutcome) -> Option<AnchorAttribution> {
        ledger.attribute(outcome, MIN_QUOTE)
    }

    fn tool(name: &str) -> Option<AnchorAttribution> {
        Some(AnchorAttribution::Tool(name.to_string()))
    }

    /// Une sortie d'outil sous le plafond est retenue entière, et le registre reste complet.
    #[test]
    fn une_poussee_sous_le_plafond_ne_tronque_pas() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);

        assert!(
            !ledger.truncated,
            "rien n'a été clippé, donc rien à signaler"
        );
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].0, "gh_read");
        assert!(
            ledger.entries[0]
                .1
                .contains("l'architecte a cité le corps du ticket"),
            "le texte normalisé doit être retrouvable : {:?}",
            ledger.entries[0].1
        );
    }

    /// Le plafond est en caractères, et il pose `truncated`. Le corps est français : le même
    /// clippage en octets couperait au milieu d'un codepoint (`scripts/check-byte-slices.sh`).
    #[test]
    fn une_poussee_au_dessus_du_plafond_clippe_en_caracteres_et_le_dit() {
        let overflowing = "é ".repeat(TOOL_READ_LEDGER_CHARS);
        assert!(overflowing.chars().count() > TOOL_READ_LEDGER_CHARS);

        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("read_file", &overflowing);

        assert!(
            ledger.truncated,
            "un clippage doit être signalé (mika#2331)"
        );
        assert!(
            ledger.entries[0].1.chars().count() <= TOOL_READ_LEDGER_CHARS,
            "le registre dépasse son plafond : {} caractères",
            ledger.entries[0].1.chars().count()
        );
        assert!(
            ledger.entries[0].1.starts_with("é é"),
            "le clippage doit garder le début, et ne pas couper un codepoint"
        );
    }

    /// Le plafond est celui du registre **entier**, pas celui d'une poussée : trois sorties
    /// d'un demi-plafond chacune ne font pas un registre d'un plafond et demi.
    #[test]
    fn le_plafond_porte_sur_le_registre_entier() {
        let half = "a ".repeat(TOOL_READ_LEDGER_CHARS / 2);
        let mut ledger = ToolReadLedger::default();
        for _ in 0..3 {
            ledger.record_tool_read("read_file", &half);
        }

        let total: usize = ledger
            .entries
            .iter()
            .map(|(_, text)| text.chars().count())
            .sum();
        assert!(
            total <= TOOL_READ_LEDGER_CHARS,
            "le registre entier dépasse son plafond : {total} caractères"
        );
        assert!(
            ledger.truncated,
            "le troisième appel n'a pas tenu : le registre doit le dire"
        );
    }

    /// Le registre normalise avec **la fonction du matcher**, pas une seconde copie : un
    /// extrait dont seuls les espaces et les marqueurs markdown diffèrent est retrouvé.
    #[test]
    fn le_registre_normalise_comme_le_matcher() {
        let mut ledger = ToolReadLedger::default();
        // Sauts de ligne et `**` dans la source, aucun des deux dans l'extrait de l'ancre.
        ledger.record_tool_read(
            "gh_read",
            "l'architecte a cité le\n   **corps du ticket**, qu'il avait lu\n",
        );

        let outcome = refused(
            "l'architecte a cité le corps du ticket, qu'il avait lu",
            AnchorMissReason::QuoteNotInBrief,
        );
        assert_eq!(attribue(&ledger, &outcome), tool("gh_read"));
    }

    /// Premier arm : l'extrait est dans la sortie d'un outil du tour, et l'attribution nomme
    /// cet outil-là. C'est le cas du 2026-10-31 et la première phrase d'AC2.
    #[test]
    fn extrait_present_dans_une_sortie_doutil_rend_tool() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("store_fact", "Préférence enregistrée.");
        ledger.record_tool_read("gh_read", TICKET_BODY);

        let outcome = refused(
            "La garde d'ancrage de mika#2037 retient une disposition de mika-arch",
            AnchorMissReason::QuoteNotInBrief,
        );
        assert_eq!(
            attribue(&ledger, &outcome),
            tool("gh_read"),
            "l'attribution doit nommer l'outil dont la sortie porte l'extrait"
        );
    }

    /// **Le test que le défaut de conception de ce vol a gagné.** `anchor_excerpt` coupe la
    /// ligne d'ancre APRÈS son préfixe `A1:`, donc l'extrait porte les guillemets du modèle
    /// et le début de sa propre prose — forme attestée par l'assertion de l'eval 2a
    /// (`A1: ""The reaper introduced by mika#2750 fails the parent`). **Aucun document ne
    /// contient cette chaîne**, donc un `contains` aurait répondu `AbsentFromReads` sur la
    /// forme même sur laquelle mika#3003 a été mesuré. La recherche passe par la fenêtre
    /// glissante du matcher, et c'est ce que ce test épingle.
    #[test]
    fn un_extrait_de_la_forme_reelle_du_2026_10_31_sattribue() {
        let anchor_line = normalize_for_anchor_match(
            "\"La garde d'ancrage de mika#2037 retient une disposition de mika-arch quand ses \
             citations ne viennent pas du brief\" — le plan porte bien cette garantie.",
        );
        let excerpt = anchor_excerpt(&anchor_line);
        assert!(
            excerpt.starts_with('"'),
            "le fixture doit porter les guillemets du modèle : {excerpt:?}"
        );

        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);

        let needle = excerpt.trim_end_matches('…');
        assert!(
            !normalize_for_anchor_match(TICKET_BODY).contains(needle),
            "la prémisse du test : le corps du ticket ne contient PAS l'extrait tel quel"
        );
        assert_eq!(
            attribue(
                &ledger,
                &refused(&excerpt, AnchorMissReason::QuoteNotInBrief)
            ),
            tool("gh_read"),
            "la fenêtre glissante doit retrouver la citation dans le bruit du modèle"
        );
    }

    /// Deuxième arm : rien trouvé, registre complet — y compris vide, quand le tour n'a
    /// appelé aucun outil. C'est la seconde phrase d'AC2 (le re-prompt d'avant ce vol).
    #[test]
    fn extrait_absent_dun_registre_complet_rend_absent_from_reads() {
        // 59 caractères : au-dessus du plancher, donc l'échec vient bien de la recherche
        // et non de la garde de longueur — les deux rendent `AbsentFromReads`.
        const INEDIT: &str = "une phrase que personne au monde n'a lue pendant ce tour-ci";
        assert!(INEDIT.chars().count() > ATTRIBUTION_MIN_QUOTE_CHARS);

        let mut ledger = ToolReadLedger::default();
        assert_eq!(
            attribue(&ledger, &refused(INEDIT, AnchorMissReason::QuoteNotInBrief)),
            Some(AnchorAttribution::AbsentFromReads),
            "un registre vide est complet, pas inconnu"
        );

        ledger.record_tool_read("gh_read", TICKET_BODY);
        assert_eq!(
            attribue(&ledger, &refused(INEDIT, AnchorMissReason::QuoteNotInBrief)),
            Some(AnchorAttribution::AbsentFromReads)
        );
    }

    /// Troisième arm : rien trouvé, mais le registre avait été clippé. L'absence n'est pas
    /// établie, donc elle n'est pas affirmée — c'est la décision de conception de D1.
    #[test]
    fn extrait_absent_dun_registre_tronque_rend_unknown() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("read_file", &"a ".repeat(TOOL_READ_LEDGER_CHARS));

        assert_eq!(
            attribue(
                &ledger,
                &refused(
                    "une phrase qui pourrait bien être dans la part coupée du registre",
                    AnchorMissReason::QuoteNotInBrief
                )
            ),
            Some(AnchorAttribution::Unknown),
            "un échec de recherche dans un registre tronqué n'est pas une absence"
        );
    }

    /// Le `…` que `anchor_excerpt` appose est un marqueur de rendu, pas du texte lu : il est
    /// retiré avant la recherche, sinon aucune ancre de plus de 72 caractères ne s'attribue.
    #[test]
    fn le_marqueur_de_troncature_est_retire_avant_la_recherche() {
        let long_line = normalize_for_anchor_match(TICKET_BODY);
        let excerpt = anchor_excerpt(&long_line);
        assert!(
            excerpt.ends_with('…'),
            "le fixture doit bien produire un extrait tronqué : {excerpt:?}"
        );

        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);
        assert_eq!(
            attribue(
                &ledger,
                &refused(&excerpt, AnchorMissReason::QuoteNotInBrief)
            ),
            tool("gh_read"),
            "un extrait tronqué doit s'attribuer comme un court"
        );
    }

    /// L'attribution ne tourne que sur `QuoteNotInBrief`. Les autres refus ne posent pas la
    /// question de l'origine : le texte EST dans le brief, c'est sa géométrie ou sa longueur
    /// qui échoue, et une phrase d'origine y enverrait le mauvais correctif.
    #[test]
    fn seul_un_refus_quote_not_in_brief_est_attribue() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);
        let excerpt = "La garde d'ancrage de mika#2037 retient une disposition de mika-arch";

        for reason in [
            AnchorMissReason::OverlappingRegions,
            AnchorMissReason::QuoteTooShort,
            AnchorMissReason::NoAnchorLine,
            AnchorMissReason::TooFewAnchors,
        ] {
            assert_eq!(
                attribue(&ledger, &refused(excerpt, reason)),
                None,
                "{reason:?} ne demande pas d'où vient le texte"
            );
        }

        let accepted = AnchorOutcome {
            label: "A1:".to_string(),
            excerpt: excerpt.to_string(),
            refused: None,
        };
        assert_eq!(
            attribue(&ledger, &accepted),
            None,
            "une ancre acceptée n'est jamais attribuée"
        );
    }

    /// Un extrait vide est dans tous les foins. Atteignable à `min_quote_chars == 0`, où une
    /// ligne d'ancre vide est refusée en `QuoteNotInBrief` : l'attribuer au premier outil du
    /// tour serait une fabrication. Le plancher de longueur suffit à l'écarter — pas besoin
    /// d'un `is_empty` séparé, qui serait un second prédicat pour la même propriété.
    #[test]
    fn un_extrait_vide_ne_sattribue_a_aucun_outil() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);
        assert_eq!(
            attribue(&ledger, &refused("", AnchorMissReason::QuoteNotInBrief)),
            Some(AnchorAttribution::AbsentFromReads)
        );
        assert_eq!(
            attribue(&ledger, &refused("   ", AnchorMissReason::QuoteNotInBrief)),
            Some(AnchorAttribution::AbsentFromReads)
        );
    }

    /// **Le plancher de l'attribution, par les deux bouts.** La phrase que ce vol ajoute au
    /// re-prompt est à l'indicatif : elle affirme. Une fausse attribution est donc pire
    /// qu'aucune, et un span de 12 caractères retrouvé dans une sortie d'outil est une
    /// coïncidence, pas une provenance. En bas, `ATTRIBUTION_MIN_QUOTE_CHARS` tient le
    /// plancher même si un manifeste déclarait un `min_quote_chars` laxiste ; en haut, la
    /// fenêtre est bornée par `ANCHOR_EXCERPT_CHARS`, sans quoi un contrat exigeant plus que
    /// la longueur de l'extrait rendrait toute attribution inatteignable.
    #[test]
    fn le_plancher_de_lattribution_ne_depend_pas_du_contrat() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);

        // Présent mot pour mot dans le corps, mais trop court pour affirmer quoi que ce soit.
        let court = "mika#2037 retient";
        assert!(court.chars().count() < ATTRIBUTION_MIN_QUOTE_CHARS);
        assert!(normalize_for_anchor_match(TICKET_BODY).contains(court));
        assert_eq!(
            ledger.attribute(&refused(court, AnchorMissReason::QuoteNotInBrief), 10),
            Some(AnchorAttribution::AbsentFromReads),
            "un contrat laxiste ne doit pas abaisser le plancher de l'affirmation"
        );

        // Le même extrait au-dessus du plancher s'attribue, au même contrat laxiste : ce qui
        // a changé est la longueur, pas le seuil déclaré.
        let long = "La garde d'ancrage de mika#2037 retient une disposition";
        assert!(long.chars().count() > ATTRIBUTION_MIN_QUOTE_CHARS);
        assert_eq!(
            ledger.attribute(&refused(long, AnchorMissReason::QuoteNotInBrief), 10),
            tool("gh_read")
        );

        // Borne haute : un contrat de 400 caractères n'empêche pas d'attribuer un extrait
        // plein, parce qu'`ANCHOR_EXCERPT_CHARS` est tout ce qu'`anchor_excerpt` peut rendre
        // et qu'une fenêtre de 400 ne tiendrait dans aucun extrait.
        let plein = "La garde d'ancrage de mika#2037 retient une disposition de mika-arch quand";
        assert!(plein.chars().count() >= ANCHOR_EXCERPT_CHARS);
        assert_eq!(
            ledger.attribute(&refused(plein, AnchorMissReason::QuoteNotInBrief), 400),
            tool("gh_read"),
            "la fenêtre doit être clippée à la longueur maximale d'un extrait"
        );
    }

    // -- le re-prompt : l'humeur de la phrase d'origine ----------------------

    /// Un `AnchorOutcome` à l'étiquette choisie. [`refused`] fixe `A1:`, et les tests du
    /// formateur ont besoin de trois lignes distinctes dans un même `per_anchor`.
    fn outcome(label: &str, excerpt: &str, refused: Option<AnchorMissReason>) -> AnchorOutcome {
        AnchorOutcome {
            label: label.to_string(),
            excerpt: excerpt.to_string(),
            refused,
        }
    }

    /// La forme mesurée du 2026-10-31, rejouée au niveau du formateur : `A1` cite le corps du
    /// ticket lu avec `gh_read`, `A2` et `A3` citent le plan. Le registre a retrouvé l'extrait
    /// de `A1` mot pour mot dans la sortie de `gh_read`, donc la phrase d'origine passe à
    /// l'indicatif, nomme la ligne **et** l'outil, et le conditionnel disparaît : affirmer et
    /// supposer la même chose dans le même re-prompt est ce qui le rend ignorable.
    #[test]
    fn mika3003_une_ancre_attribuee_passe_le_paragraphe_a_lindicatif() {
        let per_anchor = vec![
            outcome(
                "A1:",
                "La garde d'ancrage de mika#2037 retient une disposition de mika-arch…",
                Some(AnchorMissReason::QuoteNotInBrief),
            ),
            outcome("A2:", "sur l'exhaustivité du tracé", None),
            outcome("A3:", "Je n'ai pas fixé N pour le seuil", None),
        ];
        let attributions = vec![tool("gh_read"), None, None];
        let diagnosis = format_anchor_diagnosis(&per_anchor, &attributions)
            .expect("a non-empty list diagnoses");

        // Les deux premiers paragraphes sont ceux de mika#2794, inchangés.
        assert!(
            diagnosis.contains("Anchors accepted: A2, A3") && diagnosis.contains("A1: \""),
            "{diagnosis}"
        );
        // AC1 : la ligne, l'outil, et le fait que c'est mesuré.
        assert!(
            diagnosis.contains("A1 quotes, word for word, what the `gh_read` tool returned"),
            "l'indicatif doit nommer la ligne et l'outil : {diagnosis}"
        );
        assert!(
            diagnosis.contains("This is measured, not guessed"),
            "le moteur dit d'où lui vient le droit d'affirmer : {diagnosis}"
        );
        assert!(
            !diagnosis.contains("usually comes from"),
            "le conditionnel ne cohabite pas avec l'indicatif sur la même ancre : {diagnosis}"
        );
    }

    /// Premier contrôle négatif : l'extrait est absent d'un registre **complet**. Le moteur a
    /// cherché et n'a pas trouvé, mais « absent de ce que j'ai lu » n'est pas « absent de tout
    /// document » — il n'a donc rien à affirmer, et le paragraphe doit être caractère pour
    /// caractère celui d'avant ce vol. La comparaison porte sur le diagnostic entier contre
    /// l'appel à liste vide, parce qu'une égalité de substrings laisserait passer un ajout.
    #[test]
    fn mika3003_une_absence_etablie_garde_le_conditionnel_mot_pour_mot() {
        let per_anchor = vec![
            outcome(
                "A1:",
                "une phrase que personne n'a lue ce tour-ci",
                Some(AnchorMissReason::QuoteNotInBrief),
            ),
            outcome("A2:", "sur l'exhaustivité du tracé", None),
        ];
        let avant = format_anchor_diagnosis(&per_anchor, &[]).expect("non-empty");

        for attribution in [
            AnchorAttribution::AbsentFromReads,
            AnchorAttribution::Unknown,
        ] {
            let apres = format_anchor_diagnosis(&per_anchor, &[Some(attribution.clone())])
                .expect("non-empty");
            assert_eq!(
                apres, avant,
                "{attribution:?} n'établit aucune origine : le re-prompt doit être intact"
            );
            assert!(apres.contains("usually comes from"), "{apres}");
            assert!(!apres.contains("word for word, what the"), "{apres}");
        }
    }

    /// Les deux humeurs cohabitent quand les ancres ne sont pas du même cas : `A1` est
    /// attribuée, `A2` ne l'est pas. L'indicatif ne porte que sur `A1`, et le conditionnel
    /// reste pour `A2` — c'est le seul re-prompt où les deux phrases sont vraies ensemble.
    #[test]
    fn mika3003_les_deux_humeurs_cohabitent_sur_un_tour_mixte() {
        let per_anchor = vec![
            outcome(
                "A1:",
                "La garde d'ancrage de mika#2037 retient une disposition…",
                Some(AnchorMissReason::QuoteNotInBrief),
            ),
            outcome(
                "A2:",
                "une phrase que personne n'a lue ce tour-ci",
                Some(AnchorMissReason::QuoteNotInBrief),
            ),
        ];
        let attributions = vec![tool("gh_read"), Some(AnchorAttribution::AbsentFromReads)];
        let diagnosis = format_anchor_diagnosis(&per_anchor, &attributions).expect("non-empty");

        assert!(
            diagnosis.contains("A1 quotes, word for word, what the `gh_read` tool returned"),
            "{diagnosis}"
        );
        assert!(
            !diagnosis.contains("A2 quotes, word for word"),
            "A2 n'a pas été retrouvée : rien ne l'affirme : {diagnosis}"
        );
        assert!(
            diagnosis.contains("usually comes from"),
            "A2 garde le conditionnel : {diagnosis}"
        );
    }

    /// **L'appariement, qui est la déviation de ce vol par rapport au plan.** Le plan écrivait
    /// « dans l'ordre des refusées » ; `attribute` rend `None` pour tout refus qui n'est pas
    /// `QuoteNotInBrief`, donc un rang parmi les refusées perd de quelle ligne l'attribution
    /// parle. Ici `A1` est refusée sur un chevauchement et `A2` sur sa citation : un index par
    /// rang parmi les refusées attribuerait `gh_read` à `A1`, et le moteur affirmerait, à
    /// l'indicatif, une origine pour une ancre qui cite bel et bien le plan.
    #[test]
    fn mika3003_lattribution_sapparie_sur_per_anchor_et_pas_sur_les_refusees() {
        let per_anchor = vec![
            outcome(
                "A1:",
                "re-résoudre le token avant chaque cycle",
                Some(AnchorMissReason::OverlappingRegions),
            ),
            outcome(
                "A2:",
                "La garde d'ancrage de mika#2037 retient une disposition…",
                Some(AnchorMissReason::QuoteNotInBrief),
            ),
        ];
        let attributions = vec![None, tool("gh_read")];
        let diagnosis = format_anchor_diagnosis(&per_anchor, &attributions).expect("non-empty");

        assert!(
            diagnosis.contains("A2 quotes, word for word, what the `gh_read` tool returned"),
            "{diagnosis}"
        );
        assert!(
            !diagnosis.contains("A1 quotes, word for word"),
            "A1 a été refusée sur sa géométrie, pas sur son origine : {diagnosis}"
        );
    }

    /// Dernier contrôle négatif, et le plus bête : une liste d'attributions vide — ce que rend
    /// un appel qui n'a rien attribué — laisse le diagnostic identique à la forme mika#2794.
    /// Les quatre tests de ce vol-là l'appellent ainsi, et c'est ce qui les garde honnêtes.
    #[test]
    fn mika3003_une_liste_dattributions_vide_est_la_forme_pre_vol() {
        let per_anchor = vec![outcome(
            "A1:",
            "une ligne qui ne cite pas le brief",
            Some(AnchorMissReason::QuoteNotInBrief),
        )];
        let avant = format_anchor_diagnosis(&per_anchor, &[]).expect("non-empty");
        assert!(avant.contains("usually comes from"), "{avant}");
        assert!(!avant.contains("word for word, what the"), "{avant}");
        assert_eq!(format_anchor_diagnosis(&[], &[]), None);
    }

    // -- le registre de lecture a UN seul écrivain --------------------------

    /// Sites de production exemptés de
    /// [`mika3003_le_registre_de_lecture_a_un_seul_ecrivain`].
    ///
    /// **Née vide, et la résolution quand le scan tire est de RETIRER le second
    /// écrivain, jamais d'ajouter une entrée ici.** Même contrat que
    /// `CONTEXT_HISTORY_READERS_ALLOWED` du module parent : une liste née vide
    /// est un emplacement pour le prochain oubli, donc sa vacuité est elle-même
    /// assertée, et une entrée doit nommer le ticket de suite qui la retirera.
    const TOOL_READ_LEDGER_WRITERS_ALLOWED: &[&str] = &[];

    /// Lignes de production qui alimentent le registre de lecture du tour, en
    /// `(ligne 1-indexée, texte trimé)`.
    ///
    /// L'aiguille est la **forme appelée** (`.record_tool_read(`), pas le nom nu :
    /// la déclaration de la méthode, son doc-comment et le nom de ce test
    /// portent l'identifiant sans alimenter quoi que ce soit — même raison que
    /// le chemin pointé d'`identity_history_reader_sites` (module parent), dont
    /// ce prédicat reprend aussi les trois exclusions (ligne de commentaire,
    /// fichier de test par son chemin, régions de test masquées ailleurs).
    fn tool_read_ledger_writer_sites(path: &std::path::Path, src: &str) -> Vec<(usize, String)> {
        if crate::source_scan::is_test_source_path(path) {
            return Vec::new();
        }
        mika_common::source_guard::mask_test_regions(src)
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim().to_string()))
            .filter(|(_, line)| {
                line.contains(".record_tool_read(")
                    && !line.starts_with("//")
                    && !line.starts_with('*')
            })
            .collect()
    }

    /// **D4** — le registre a exactement un site d'alimentation en production.
    ///
    /// La phrase que ce vol ajoute au re-prompt est à l'**indicatif** : elle
    /// affirme qu'un outil a rendu ce texte pendant ce tour. Ce droit tient
    /// entièrement à ce que le registre ne contienne que des sorties d'outils.
    /// Un second écrivain — un texte écrit par le moteur, un message du modèle,
    /// un fragment de brief — ferait attribuer à un outil une phrase qu'aucun
    /// outil n'a rendue, et **rien ne redeviendrait rouge** : l'attribution
    /// resterait plausible, le re-prompt resterait bien formé, et l'architecte
    /// se verrait accusé d'avoir cité un document qu'il n'a pas lu. C'est la
    /// classe de mika#2158 — deux réponses individuellement crédibles à une
    /// question qui n'en a qu'une.
    #[test]
    fn mika3003_le_registre_de_lecture_a_un_seul_ecrivain() {
        assert!(
            TOOL_READ_LEDGER_WRITERS_ALLOWED
                .iter()
                .all(|entry| entry.contains("mika#")),
            "mika#3003 — toute entrée de TOOL_READ_LEDGER_WRITERS_ALLOWED doit nommer le \
             ticket de suite qui la retirera. Une exemption que personne ne porte est la \
             façon dont une garde cesse de garder. Liste actuelle : \
             {TOOL_READ_LEDGER_WRITERS_ALLOWED:?}"
        );

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let src_root = scanner.src_root().to_path_buf();
        let mut sites: Vec<String> = Vec::new();

        scanner.for_each(|path, production| {
            let rel = path
                .strip_prefix(&src_root)
                .unwrap_or(path)
                .display()
                .to_string();
            if TOOL_READ_LEDGER_WRITERS_ALLOWED
                .iter()
                .any(|allowed| rel.contains(allowed))
            {
                return;
            }
            for (line, text) in tool_read_ledger_writer_sites(path, production) {
                sites.push(format!("{rel}:{line}: {text}"));
            }
        });

        assert_eq!(
            sites.len(),
            1,
            "mika#3003 — un seul site de production doit alimenter le registre de lecture : \
             celui de `tool_execution::dispatch::process_tool_calls` qui tient la sortie \
             qu'un outil vient de rendre. Trouvés {} :\n{}\n\nSi tu en as ajouté un second, \
             retire-le : le registre est ce qui donne au moteur le droit d'affirmer, à \
             l'indicatif, qu'un outil a rendu ce texte.",
            sites.len(),
            sites.join("\n")
        );
        assert!(
            sites[0].starts_with("tool_execution/dispatch.rs:")
                || sites[0].starts_with("tool_execution\\dispatch.rs:"),
            "mika#3003 — l'écrivain unique doit être le site de dispatch, là où la sortie \
             de l'outil est encore disponible :\n{}",
            sites[0]
        );
    }

    /// **D4b — contrôle de bonne foi pour D4.**
    ///
    /// D4 pourrait être vert parce qu'il ne regarde rien, ce qui est exactement
    /// la panne que D4 existe pour rendre visible. Le prédicat est donc montré
    /// rouge sur un écrivain ajouté ailleurs, et muet sur les quatre formes qui
    /// nomment la méthode sans l'appeler.
    #[test]
    fn mika3003_le_scan_de_lecrivain_rougit_sur_un_second_site() {
        let prod = std::path::Path::new("src/ailleurs.rs");

        let offending = "fn ailleurs(l: &mut ToolReadLedger, brief: &str) {\n    \
             l.record_tool_read(\"brief\", brief);\n}\n";
        assert_eq!(
            tool_read_ledger_writer_sites(prod, offending).len(),
            1,
            "le scan doit voir un écrivain ajouté hors du site de dispatch"
        );

        // 1. La déclaration de la méthode elle-même n'est pas un appel.
        let declaration = "    pub(crate) fn record_tool_read(&mut self, n: &str, o: &str) {\n";
        assert!(
            tool_read_ledger_writer_sites(prod, declaration).is_empty(),
            "déclarer la méthode n'alimente pas le registre"
        );

        // 2. Le nom en prose — la forme qui accuse les doc-comments expliquant
        // pourquoi l'écrivain est unique (classe mika#2050 Signal S).
        let prose = "\
// Seul site appelant `record_tool_read` sous `src/`.
/// Tenu par `mika3003_le_registre_de_lecture_a_un_seul_ecrivain`.
 * `ledger.record_tool_read(name, out)` est l'unique alimentation.
";
        assert!(
            tool_read_ledger_writer_sites(prod, prose).is_empty(),
            "une prose nommant la méthode n'alimente rien — et la troisième ligne porte \
             bien la forme appelée, derrière un marqueur de commentaire de bloc"
        );

        // 3. Un appel dans une région de test masquée.
        let in_tests = "fn prod() {}\n#[cfg(test)]\nmod tests {\n    \
             ledger.record_tool_read(\"gh_read\", TICKET_BODY);\n}\n";
        assert!(
            tool_read_ledger_writer_sites(prod, in_tests).is_empty(),
            "une région de test est masquée avant le scan"
        );

        // 4. Un fichier dont le CHEMIN est du code de test n'a pas de moitié de
        // production, quel que soit son contenu (mika#2321).
        let test_path = std::path::Path::new("src/db/tests/harnais.rs");
        assert!(
            tool_read_ledger_writer_sites(test_path, offending).is_empty(),
            "mika#2321 — un fichier sous `tests/` est du code de test"
        );

        // …mais un commentaire en fin de ligne n'exempte pas le code qui précède.
        let trailing = "    ledger.record_tool_read(name, &out.content); // site unique\n";
        assert_eq!(
            tool_read_ledger_writer_sites(prod, trailing).len(),
            1,
            "seule une ligne qui COMMENCE par un marqueur de commentaire est de la prose"
        );
    }

    /// Le dénominateur de l'AC3 n'est pas un binaire : il porte les comptes de la branche
    /// `Missing`, l'ensemble dédupliqué des outils, et la retenue quand elle a eu lieu.
    /// Trois ancres prises dans le même corps de ticket décrivent UNE source.
    #[test]
    fn mika3003_le_passage_arme_dedoublonne_ses_sources_et_capte_les_comptes() {
        let mut ledger = ToolReadLedger::default();
        ledger.record_tool_read("gh_read", TICKET_BODY);

        let excerpt = "La garde d'ancrage de mika#2037 retient une disposition de mika-arch";
        let per_anchor = vec![
            outcome("A1:", excerpt, Some(AnchorMissReason::QuoteNotInBrief)),
            outcome("A2:", excerpt, Some(AnchorMissReason::QuoteNotInBrief)),
            outcome("A3:", "sur l'exhaustivité du tracé", None),
        ];

        let mut armed = ArmedPassage::default();
        let attributions = armed.attribute(&ledger, &per_anchor, MIN_QUOTE, 3, 1);

        assert_eq!(
            attributions,
            vec![tool("gh_read"), tool("gh_read"), None],
            "l'attribution est parallèle à `per_anchor`, index pour index"
        );
        assert_eq!(
            armed.sources(),
            "gh_read",
            "deux ancres du même corps de ticket décrivent une source, pas deux"
        );
        assert_eq!((armed.found, armed.valid), (Some(3), Some(1)));
        assert!(!armed.withheld, "rien n'a encore été retenu");

        armed.mark_withheld();
        assert!(armed.withheld);
    }

    /// La forme par défaut — celle de la branche `Satisfied`, qui n'appelle jamais
    /// `attribute` : les comptes restent absents de la ligne, et `attributed_sources` est une
    /// chaîne vide, jamais un champ absent (mika#2331, mika#2340).
    #[test]
    fn mika3003_le_passage_arme_par_defaut_est_vide_et_pas_absent() {
        let armed = ArmedPassage::default();
        assert_eq!(armed.sources(), "");
        assert_eq!((armed.found, armed.valid), (None, None));
        assert!(!armed.withheld);
    }
}
