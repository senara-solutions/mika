//! Verdicts, dispositions et budget de raisonnement.
//!
//! Une disposition non attestée escalade avec sa cause (mika#2338), la
//! relance corrective nomme l'ancre fautive (mika#2794), et l'épuisement du
//! budget de raisonnement se distingue des autres `stop_reason` (mika#2296).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// ===========================================================================
// mika#2338 — unattested disposition escalates with its cause
// ===========================================================================

/// What mika-arch sees in production: the three arch skills are `always_on`, so the
/// union of all five declared lines is present on every turn.
fn arch_suffix_union() -> Vec<String> {
    [
        "Disposition: READY",
        "Disposition: ITERATE",
        "Disposition: ESCALATE",
        "Verdict: GROOMED",
        "Verdict: ESCALATE",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn finding_prefixes() -> Vec<String> {
    (1..=3).map(|n| format!("F{n}:")).collect()
}

fn escalate_2338(text: &str, suffixes: &[String], prefixes: &[String]) -> Option<String> {
    escalate_unattested_disposition(
        text,
        suffixes,
        prefixes,
        3,
        2,
        review_anchor::AnchorMissReason::QuoteNotInBrief,
    )
}

fn last_non_empty_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
}

#[test]
fn mika2338_first_pass_2_of_3_becomes_a_disposition_escalate_naming_the_cause() {
    let text = "A1: \"une citation\" — ok.\nA2: \"une autre\" — ok.\nA3: paraphrase.\n\nLe plan est sain.\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes())
        .expect("the union declares Disposition: ESCALATE");
    assert_eq!(last_non_empty_line(&out), "Disposition: ESCALATE");
    assert!(
        out.contains("F1: (BLOCKING) [mika-engine] review-anchor: attestation withheld"),
        "finding line must open with the prefix and the constant marker: {out}"
    );
    assert!(out.contains("anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief"));
    assert!(
        !out.contains("Disposition: READY"),
        "no READY may survive: {out}"
    );
    assert!(
        out.contains("Le plan est sain."),
        "the model's body is kept"
    );
}

#[test]
fn mika2338_second_pass_groomed_becomes_a_verdict_escalate_not_a_disposition() {
    let text = "A1: \"une citation\" — ok.\nA2: \"une autre\" — ok.\nA3: paraphrase.\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
    assert!(!out.contains("Verdict: GROOMED"));
    assert!(
        !out.contains("Disposition: ESCALATE"),
        "the family follows the withdrawn line, not the order of the list: {out}"
    );
}

#[test]
fn mika2338_undeclared_family_falls_back_to_none() {
    let only_dispositions: Vec<String> = [
        "Disposition: READY",
        "Disposition: ITERATE",
        "Disposition: ESCALATE",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let text = "A1: x\n\nVerdict: GROOMED\n";
    assert_eq!(
        escalate_2338(text, &only_dispositions, &finding_prefixes()),
        None,
        "Verdict: ESCALATE is not declared — the caller withholds instead"
    );

    let only_ready = vec!["Disposition: READY".to_string()];
    let text = "A1: x\n\nDisposition: READY\n";
    assert_eq!(escalate_2338(text, &only_ready, &finding_prefixes()), None);
}

#[test]
fn mika2338_no_finding_prefixes_declared_defaults_to_f1() {
    let text = "A1: x\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &[]).unwrap();
    assert!(
        out.contains("\nF1: (BLOCKING) [mika-engine] review-anchor:"),
        "{out}"
    );
}

#[test]
fn mika2338_a_disposition_cited_early_on_its_own_line_is_rewritten_too() {
    let text = "Le contrat dit :\nDisposition: READY\nquand le plan est sain.\n\nA1: x\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert_eq!(out.matches("Disposition: ESCALATE").count(), 3, "{out}");
}

/// The case the withheld marker used to cover at tier 0 and that this path must cover
/// itself: `dispatch-lib` reads the FIRST `Disposition:` anywhere in the text.
#[test]
fn mika2338_an_inline_mention_of_the_disposition_is_rewritten() {
    let text = "Je confirme que ma réponse se termine par `Disposition: READY` comme demandé.\n\nA1: x\n\nDisposition: READY\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert!(
        out.contains("se termine par `Disposition: ESCALATE`"),
        "{out}"
    );

    let text = "Mon verdict est bien `Verdict: GROOMED`.\n\nA1: x\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Verdict: GROOMED"), "{out}");
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
}

/// A READY quoted inline during a second-review turn is rewritten into ITS family's
/// ESCALATE, so `_parse_disposition`'s tier 1a cannot read READY out of it either.
#[test]
fn mika2338_a_foreign_family_mention_is_rewritten_into_its_own_escalate() {
    let text = "La première passe disait Disposition: READY.\n\nA1: x\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &arch_suffix_union(), &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert!(out.contains("disait Disposition: ESCALATE."), "{out}");
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
}

/// The cross-family fallback inside the rewrite loop: a declared non-terminal line whose
/// own family has no declared ESCALATE is rewritten into the WITHDRAWN line's ESCALATE,
/// so no declared verdict keyword survives. Unreachable with the shipped always_on arch
/// manifests (their union always carries both families); live code for a future
/// single-family verdict producer.
#[test]
fn mika2338_a_line_whose_family_has_no_escalate_is_rewritten_into_the_withdrawn_family() {
    // Verdict family complete; Disposition family declares READY but no ESCALATE.
    let partial: Vec<String> = [
        "Disposition: READY",
        "Verdict: GROOMED",
        "Verdict: ESCALATE",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let text = "La première passe disait Disposition: READY.\n\nA1: x\n\nVerdict: GROOMED\n";
    let out = escalate_2338(text, &partial, &finding_prefixes()).unwrap();
    assert!(!out.contains("Disposition: READY"), "{out}");
    assert!(
        out.contains("disait Verdict: ESCALATE."),
        "the READY mention falls back to the withdrawn line's family: {out}"
    );
    assert!(
        !out.contains("Disposition: ESCALATE"),
        "undeclared, must not be emitted: {out}"
    );
    assert_eq!(last_non_empty_line(&out), "Verdict: ESCALATE");
}

#[test]
fn mika2338_the_finding_line_never_carries_a_declared_line_in_clear() {
    for reason in [
        review_anchor::AnchorMissReason::NoAnchorLine,
        review_anchor::AnchorMissReason::QuoteTooShort,
        review_anchor::AnchorMissReason::QuoteNotInBrief,
        review_anchor::AnchorMissReason::OverlappingRegions,
        review_anchor::AnchorMissReason::TooFewAnchors,
    ] {
        let out = escalate_unattested_disposition(
            "A1: x\n\nDisposition: READY\n",
            &arch_suffix_union(),
            &finding_prefixes(),
            1,
            0,
            reason,
        )
        .unwrap();
        let finding = out
            .lines()
            .find(|l| l.starts_with("F1: "))
            .expect("finding line present");
        assert!(
            !finding.contains("READY") && !finding.contains("GROOMED"),
            "{finding}"
        );
        for req in arch_suffix_union() {
            assert!(!finding.contains(&req), "{finding} carries {req}");
        }
    }
}

// ===========================================================================
// mika#2794 — the corrective re-prompt names the faulty anchor
// ===========================================================================

fn outcome(
    label: &str,
    excerpt: &str,
    refused: Option<review_anchor::AnchorMissReason>,
) -> review_anchor::AnchorOutcome {
    review_anchor::AnchorOutcome {
        label: label.to_string(),
        excerpt: excerpt.to_string(),
        refused,
    }
}

/// The measured shape of mika#2794: `A1` quotes the ticket body, `A2` and `A3` quote the
/// brief. The diagnosis must name all three — the refused one so it is rewritten, the two
/// accepted so they are **not** — and say where a wrong quote usually comes from.
#[test]
fn mika2794_the_diagnosis_names_the_refused_anchor_and_the_accepted_ones() {
    let per_anchor = vec![
        outcome(
            "A1:",
            "Le reaper groom introduit par mika#2750 (PR mika#2754)…",
            Some(review_anchor::AnchorMissReason::QuoteNotInBrief),
        ),
        outcome("A2:", "sur l'exhaustivité du tracé", None),
        outcome("A3:", "Je n'ai pas fixé N pour le seuil", None),
    ];
    // mika#3003 — aucune attribution : ce test mesure la forme pré-mika#3003
    // du diagnostic, et c'est précisément ce qu'une liste vide doit rendre.
    let diagnosis = review_anchor::format_anchor_diagnosis(&per_anchor, &[])
        .expect("a non-empty list diagnoses");

    assert!(
        diagnosis.contains("Anchors accepted: A2, A3"),
        "the accepted anchors are named so the correction stays local: {diagnosis}"
    );
    assert!(
        diagnosis.contains("do not rewrite them"),
        "naming them is useless without the instruction to keep them: {diagnosis}"
    );
    assert!(
        diagnosis
            .contains("A1: \"Le reaper groom introduit par mika#2750 (PR mika#2754)…\" — refused:"),
        "the refused anchor is named by label AND by what it said: {diagnosis}"
    );
    assert!(
        !diagnosis.contains("A2: \"") && !diagnosis.contains("A3: \""),
        "an accepted anchor is listed, never quoted back as a failure: {diagnosis}"
    );
    assert!(
        diagnosis.contains("a document you read with a tool"),
        "the measured cause — a tool-read document, not this turn's brief: {diagnosis}"
    );
}

/// Negative control of the origin sentence, and the reason it is gated. Anchors that all
/// quote the **same region of the right brief** have not quoted another document; telling
/// the model it did would send it looking for a brief it already quoted.
#[test]
fn mika2794_an_overlap_refusal_does_not_blame_another_document() {
    let per_anchor = vec![
        outcome("A1:", "re-résoudre le token avant chaque cycle", None),
        outcome(
            "A2:",
            "re-résoudre le token avant chaque cycle",
            Some(review_anchor::AnchorMissReason::OverlappingRegions),
        ),
    ];
    let diagnosis = review_anchor::format_anchor_diagnosis(&per_anchor, &[])
        .expect("a non-empty list diagnoses");

    assert!(diagnosis.contains("Anchors accepted: A1"), "{diagnosis}");
    assert!(diagnosis.contains("A2: \"re-résoudre"), "{diagnosis}");
    assert!(
        !diagnosis.contains("a document you read with a tool"),
        "the origin sentence is gated on QuoteNotInBrief: {diagnosis}"
    );
}

/// The other negative control: the matcher's two early returns report no per-line fate, and
/// the corrective re-prompt must then be character for character its pre-mika#2794 self.
/// `None` is the whole mechanism — the insertion point is a `{}` filled with `""`.
#[test]
fn mika2794_an_empty_fate_list_leaves_the_reprompt_untouched() {
    assert_eq!(review_anchor::format_anchor_diagnosis(&[], &[]), None);
}

/// The paragraph is bounded. Five refused anchors name three — a wall of ten lines is a
/// wall the model skims, and the measured shape refuses one of three.
#[test]
fn mika2794_the_diagnosis_names_at_most_three_refused_anchors() {
    let per_anchor: Vec<review_anchor::AnchorOutcome> = (1..=5)
        .map(|n| {
            outcome(
                &format!("A{n}:"),
                "une ligne qui ne cite pas le brief",
                Some(review_anchor::AnchorMissReason::QuoteNotInBrief),
            )
        })
        .collect();
    let diagnosis = review_anchor::format_anchor_diagnosis(&per_anchor, &[])
        .expect("a non-empty list diagnoses");

    let named = (1..=5)
        .filter(|n| diagnosis.contains(&format!("A{n}: \"")))
        .collect::<Vec<_>>();
    assert_eq!(
        named,
        vec![1, 2, 3],
        "the first MAX_DIAGNOSED_ANCHORS in body order, and only those: {diagnosis}"
    );
    assert!(
        !diagnosis.contains("Anchors accepted:"),
        "nothing was accepted, so nothing is claimed to have been: {diagnosis}"
    );
}

/// The instrument's two fields (mika#2794 D3). Empty is an empty string, never an absent
/// field: absence means a binary older than this change, not an absence of refusal.
#[test]
fn mika2794_the_label_fields_are_empty_strings_not_absent() {
    let (accepted, refused) = anchor_label_fields(&[
        outcome(
            "A1:",
            "x",
            Some(review_anchor::AnchorMissReason::QuoteNotInBrief),
        ),
        outcome("A2:", "y", None),
        outcome("A3:", "z", None),
    ]);
    assert_eq!(refused, "A1");
    assert_eq!(accepted, "A2,A3");

    let (accepted, refused) = anchor_label_fields(&[]);
    assert_eq!((accepted.as_str(), refused.as_str()), ("", ""));
}

// ===========================================================================
// mika#2296 — reasoning-budget-exhaustion predicate (M2 / T3)
// ===========================================================================

/// The founding signature: MaxTokens, a full output budget billed, nothing
/// visible. This is the arch pass `8b623724` — 123 s, 8192 output tokens,
/// empty content, no `Disposition:` for dispatch-lib to parse.
#[test]
fn mika2296_maxtokens_with_empty_text_and_billed_output_is_budget_exhaustion() {
    assert!(is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "",
        8192
    ));
    // Whitespace-only is the same failure wearing a newline.
    assert!(is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "\n  \n",
        8192
    ));
}

/// A turn that hit the ceiling *while writing* is truncated output, not a
/// starved verdict — the operator's remedy differs, so the signal must not
/// conflate them.
#[test]
fn mika2296_maxtokens_with_visible_text_is_not_budget_exhaustion() {
    assert!(!is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "Disposition: READY",
        8192
    ));
}

/// Without the `output_tokens > 0` term, a provider that returned nothing at
/// all would read as a budget problem and send the operator to raise a
/// ceiling that was never reached.
#[test]
fn mika2296_zero_output_tokens_is_an_empty_response_not_a_starved_budget() {
    assert!(!is_reasoning_budget_exhausted(
        LlmStopReason::MaxTokens,
        "",
        0
    ));
}

/// The other stop reasons keep their own meanings; an empty `EndTurn` is a
/// different defect with a different remedy.
#[test]
fn mika2296_other_stop_reasons_never_fire_the_budget_signal() {
    for stop_reason in [
        LlmStopReason::EndTurn,
        LlmStopReason::ToolUse,
        LlmStopReason::ContentFilter,
    ] {
        assert!(
            !is_reasoning_budget_exhausted(stop_reason, "", 8192),
            "{stop_reason:?} must not be reported as reasoning-budget exhaustion"
        );
    }
}
