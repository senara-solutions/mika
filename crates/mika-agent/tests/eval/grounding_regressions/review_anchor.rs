//! Scenario: Review-anchor guard — attestation required on non-terminal dispositions (mika#2037)
//!
//! Context: mika#901's F-list guard fires on TERMINAL dispositions (ITERATE/ESCALATE) and
//! exempts the rest. That left `Disposition: READY` and `Verdict: GROOMED` — the two
//! dispositions that advance the grooming chain — as the only ones owing no evidence.
//! On 2026-08-29 mika-arch returned a 302-byte acknowledgement carrying `Disposition: READY`
//! on a 10 492-byte brief with four numbered questions, none of them addressed. The keyword
//! forged the attestation; `/mika-groom-ticket` Phase 3 step 10 committed the plan as
//! architect-validated.
//!
//! The guard requires anchor lines quoting the brief verbatim at distinct positions, and is
//! fail-CLOSED: when the corrective re-prompt does not produce them, the disposition is
//! never accepted. Since mika#2338 the refusal is also fail-VISIBLE: the disposition is
//! rewritten into a terminal `ESCALATE` of its own family, preceded by an engine finding
//! line naming the cause (`anchors_found`, `anchors_valid`, `miss_reason`), so the groom
//! reports why instead of an opaque "verdict missing". The withheld marker remains the
//! fallback for a skill that declares no `ESCALATE` of that family.
//!
//! "Verbatim" is the brief's words, not its bytes (mika#2338): a quote of a markdown brief
//! with the `**` and backticks dropped — what the mika#2335 architect actually returned —
//! is accepted.
//!
//! ## Hard Assertions
//! - Guard fires on READY without attestation, and on `Verdict: GROOMED`.
//! - Guard does NOT fire on a terminal disposition (that half belongs to mika#901).
//! - Guard does NOT fire when the skill does not declare the contract.
//! - A real anchored review passes on the first turn, with no re-prompt.
//! - After the re-prompt fails at 2 of 3, the response ends on `ESCALATE` of the withdrawn
//!   line's family, carries the engine finding line, and no READY/GROOMED survives.
//! - The corrective re-prompt names the anchor that failed — its label, the start of what it
//!   said, and the ones that passed — without relaxing the fail-closed outcome (mika#2794).
//! - With no `ESCALATE` of that family declared, the withheld marker is present (fallback).
//! - A rendered quote of a markdown brief is accepted.
//! - The anchor guard's retry budget is independent of the F-list guard's.
//!
//! ## Tags
//! - `grounding:review-anchor-required` — post-fix success tag
//! - `grounding:unanchored-ready` — pre-fix failure tag
//!
//! Reference: mika#2037, mika#901 (the exempted half), mika#1957 (n=2 of the same class),
//! mika#2338 (fail-visible escalation and word-level comparison)

use std::collections::HashMap;
use std::path::PathBuf;

use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Output, SkillInfo, SkillManifest, Triggers};
use mika_common::llm::mock::*;
use mika_common::llm::{LlmContent, LlmRequest, LlmRole};

use super::*;

const GROOM_SUFFIX_LINES: &[&str] = &[
    "Disposition: READY",
    "Disposition: ITERATE",
    "Disposition: ESCALATE",
];
const SECOND_REVIEW_SUFFIX_LINES: &[&str] = &["Verdict: GROOMED", "Verdict: ESCALATE"];
/// What mika-arch actually sees: the three arch skills are `always_on`, so the engine
/// collects the union of every declared line on every turn (mika#2338).
const ARCH_SUFFIX_UNION: &[&str] = &[
    "Disposition: READY",
    "Disposition: ITERATE",
    "Disposition: ESCALATE",
    "Verdict: GROOMED",
    "Verdict: ESCALATE",
];
/// The engine finding line's fixed marker (mika#2338). Kept as a literal for the same reason
/// as `WITHHELD_MARKER`: `dispatch-lib`'s tier 0b recognizes this exact shape.
const ENGINE_FINDING_MARKER: &str = "(BLOCKING) [mika-engine] review-anchor:";
const ANCHOR_PREFIXES: &[&str] = &["A1:", "A2:", "A3:", "A4:", "A5:"];
const FINDING_PREFIXES: &[&str] = &["F1:", "F2:", "F3:"];

/// The withheld-disposition marker. Kept as a literal here on purpose: if the engine's
/// constant is renamed, this test fails rather than silently passing against a marker
/// `dispatch-lib`'s tier 0 no longer recognizes.
const WITHHELD_MARKER: &str = "Disposition-Withheld: REVIEW-ANCHOR-MISSING";

/// A brief above the arming threshold, standing in for the 10 492-byte one of mika#2037.
/// Anchor lines in the tests below quote from it verbatim.
///
/// It must exceed `review_anchor_min_brief_chars` (2000) or the guard correctly declines to
/// arm — see `test_review_anchor_no_op_on_short_ad_hoc_question` for the other side of that
/// boundary.
const BRIEF: &str = "groom-ticket: Review the plan for the milestone-manager token renewal.\n\
     \n\
     ## Summary\n\
     The plan re-resolves the manager cycle token before every cycle instead of freezing it \
     at spawn time. A GitHub App installation token has roughly a one-hour lifetime, so the \
     manager cycles 401 until the process restarts.\n\
     \n\
     ## Design\n\
     The cycle token is re-resolved through a TokenResolver trait, never frozen at spawn. \
     SettingsTokenResolver is the production implementation: PAT first per ADR-008, GitHub \
     App installation token as the fallback. A change in the resolved value emits \
     manager_token_refreshed at INFO, carrying presence booleans only, never token material. \
     AuthFailureTracker counts the duration of an unbroken 401 run rather than a count of \
     cycles, because poll_interval is operator-configurable and N cycles has no stable \
     temporal meaning. Past a thirty-minute threshold the tracker escalates and re-announces \
     at most hourly.\n\
     \n\
     ## Error handling\n\
     A failed re-resolution keeps the previous token rather than overwriting it with None: \
     reader.rs only sets GH_TOKEN when the value is Some, so overwriting would silently drop \
     the cycle onto the host ambient credentials. The refresh is bounded by a fifteen-second \
     timeout because it sits outside the select! on the cancellation token, and GitHubApp \
     holds its cache write-lock across an un-timed HTTP call. Any successful cycle clears the \
     failure window; non-401 failures neither advance nor clear it, since a network blip is \
     not proof of recovery, nor of auth failure.\n\
     \n\
     ## Test plan\n\
     Unit coverage for a resolver returning a changed value, an unchanged value, and an \
     error; the tracker advancing, clearing, and re-announcing on schedule. Integration \
     coverage for a cycle running against a resolver whose token rotates mid-run.\n\
     \n\
     ## Questions\n\
     Question 1: where does the correction belong, in the spawn path or in the cycle body?\n\
     Question 2: is the trace exhaustive, should every refresh be journaled at INFO level?\n\
     Question 3: I have not fixed N for the repeated authentication failure threshold.\n\
     Question 4: is putting the 403 response class out of scope safe for this milestone?\n";

/// Three anchor lines, each quoting a different region of the brief verbatim.
const ANCHORED_REVIEW: &str = "A1: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct placement, the cycle body owns it.\n\
     A2: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes, one INFO per resolved change is enough.\n\
     A3: \"I have not fixed N for the repeated authentication failure threshold\" — express it as a duration, not a cycle count.\n\
     \n\
     Disposition: READY";

/// The measured mika#2037 response shape: an acknowledgement plus the keyword.
const UNANCHORED_STUB: &str = "Preference stored — the per-cycle re-resolution pattern and the N=3 threshold.\n\n\
     Disposition: READY";

fn make_anchor_skill(
    name: &str,
    keywords: &[&str],
    suffix_lines: &[&str],
    anchor_prefixes: &[&str],
    finding_prefixes: &[&str],
) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} test skill"),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            },
            llm: Default::default(),
            constraints: Default::default(),
            output: Output {
                required_suffix_lines: suffix_lines.iter().map(|s| s.to_string()).collect(),
                required_finding_list_prefixes: finding_prefixes
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                required_review_anchor_prefixes: anchor_prefixes
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                review_anchor_min_count: 3,
                review_anchor_min_quote_chars: 40,
                review_anchor_min_brief_chars: 2000,
                required_tool_arg_suffixes: vec![],
            },
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

fn groom_skill() -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )])
}

/// Test 1: the founding incident — READY with no attestation triggers the corrective
/// re-prompt, and the anchored second turn is accepted.
#[tokio::test]
async fn test_review_anchor_caught_on_unanchored_ready() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(UNANCHORED_STUB),
            text_response(ANCHORED_REVIEW),
        ])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected the review-anchor guard to fire and re-prompt, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "A1:");
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

/// Test 2: fail-closed AND fail-visible (mika#2338). When the corrective re-prompt still
/// yields only two verifiable anchors out of three — the measured mika#2335 shape — the
/// disposition does not survive: the response ends on `Disposition: ESCALATE`, carries the
/// engine finding line with the counters and the miss reason, and keeps the model's prose.
#[tokio::test]
async fn test_review_anchor_escalates_with_cause_after_failed_retry() -> anyhow::Result<()> {
    // Two verbatim anchors, one paraphrase: 2 of 3, QuoteNotInBrief.
    let two_of_three = "A1: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct.\n\
         A2: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes.\n\
         A3: the threshold for repeated auth failures is left open, express it as a duration.\n\
         \n\
         Still nothing to add. The plan is fine as written.\n\
         \n\
         Disposition: READY";
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        ARCH_SUFFIX_UNION,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(two_of_three),
            text_response(two_of_three),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(&trace, &["Disposition: READY"]);
    grounding_assertions::assert_response_contains(&trace, "Disposition: ESCALATE");
    grounding_assertions::assert_response_contains(&trace, ENGINE_FINDING_MARKER);
    grounding_assertions::assert_response_contains(&trace, "anchors_valid=2");
    grounding_assertions::assert_response_contains(&trace, "QuoteNotInBrief");
    // The body survives — the guard removes the unearned attestation, not the content.
    grounding_assertions::assert_response_contains(&trace, "Still nothing to add");
    // The marker is the fallback, not this path.
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// The text of every `User` message of one captured request.
fn user_texts(req: &LlmRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| matches!(m.role, LlmRole::User))
        .filter_map(|m| match &m.content {
            LlmContent::Text(t) => Some(t.clone()),
            LlmContent::Blocks(_) => None,
        })
        .collect()
}

/// Test 2a: the corrective re-prompt names the anchor that failed (mika#2794).
///
/// Same replay as test 2 — two identical responses at 2 of 3 — read from the other end: not
/// what the turn returns, but what the engine *said* to the model between the two. mika#2794
/// measured that `reason.describe()` alone does not land: session `a7f1ccad` returned the same
/// faulty anchor three times in a row under a re-prompt that named the class and not the line.
///
/// The three properties the fix owes, end to end through `run_agent`:
///
/// - the refused anchor is named, **with the start of what it said** — a label alone does not
///   survive a model that renumbered its own lines;
/// - the two that passed are named as accepted, so the correction is local rather than a
///   rewrite of all three;
/// - the fail-closed behaviour is **unchanged** — an uncorrected second response still ends on
///   `Disposition: ESCALATE` with the engine finding line. The re-prompt is an attempt at
///   obedience, never a relaxation of the guard (mika#2037, interdit reconduit par mika#2794).
///
/// The fixture is the **measured shape**, not a convenient one: `A1` quotes the opening
/// sentence of a ticket body the architect had read with `gh_read` — the mika#2750 reaper
/// sentence of session `49ffa230`, verbatim — while `A2` and `A3` quote `BRIEF` at disjoint
/// regions. 3 anchors found, 2 valid, `QuoteNotInBrief`, and the refused one is the first.
///
/// **Depuis mika#2037 vol mika#3003, ce test est aussi le contrôle négatif d'AC2** : les mêmes
/// ancres, sans l'appel d'outil. Le registre de lecture du tour est vide, donc le moteur ne
/// peut rien établir et le re-prompt reste celui d'avant ce vol, au conditionnel, mot pour
/// mot. Son pendant positif est
/// [`test_review_anchor_reprompt_names_the_tool_that_produced_the_quote`], dans ce fichier.
#[tokio::test]
async fn test_review_anchor_reprompt_names_the_refused_anchor() -> anyhow::Result<()> {
    let two_of_three = "A1: \"The reaper introduced by mika#2750 fails the parent before the relaunch\" — noted.\n\
         A2: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct.\n\
         A3: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes.\n\
         \n\
         Still nothing to add. The plan is fine as written.\n\
         \n\
         Disposition: READY";
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        ARCH_SUFFIX_UNION,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(two_of_three),
            text_response(two_of_three),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    let requests = harness.mock().captured_requests();
    assert!(
        requests.len() >= 2,
        "the guard must have re-prompted, got {} request(s)",
        requests.len()
    );
    let reprompt = user_texts(&requests[1])
        .into_iter()
        .find(|t| t.contains("What blocked it"))
        .expect("the second request carries the review-anchor corrective re-prompt");

    // The refused anchor, by label and by its opening words.
    assert!(
        reprompt.contains("Anchors refused:"),
        "the re-prompt must open the refused list, got:\n{reprompt}"
    );
    // The label, then the start of what that line said. The doubled quote is real and worth
    // seeing: the outer pair is the engine's, the inner one the model's own quotation marks.
    assert!(
        reprompt.contains("A1: \"\"The reaper introduced by mika#2750 fails the parent"),
        "the re-prompt must name A1 and quote its opening, got:\n{reprompt}"
    );
    // The accepted ones, by name — this is what keeps the correction local.
    assert!(
        reprompt.contains("Anchors accepted: A2, A3"),
        "the re-prompt must name the two anchors that passed, got:\n{reprompt}"
    );
    // A2 and A3 are named as accepted and never quoted back as failures.
    for named in ["A2: \"", "A3: \""] {
        assert!(
            !reprompt.contains(named),
            "{named} must not appear in the refused list, got:\n{reprompt}"
        );
    }
    // The origin sentence: the quote that is absent from the brief usually comes from a
    // document read with a tool — the measured cause of mika#2794.
    assert!(
        reprompt.contains("not from the brief message of this turn"),
        "the re-prompt must say where a wrong quote usually comes from, got:\n{reprompt}"
    );
    // Stated as a likelihood on this turn — and since mika#3003 that is a *measured* fact
    // about this turn, not a permanent property of the engine: no tool ran here, so the
    // read ledger is empty, nothing can be attributed, and the mood stays conditional.
    assert!(
        reprompt.contains("usually comes from"),
        "the origin sentence must stay hedged, got:\n{reprompt}"
    );
    // mika#3003 — **le contrôle négatif d'AC2, dans le même fichier que le cas positif.**
    // Aucun outil n'a tourné sur ce tour, donc le moteur ne détient aucun texte où chercher
    // l'extrait : la phrase à l'indicatif doit être absente. Sans cette moitié, un moteur
    // qui attribuerait à tort — à un outil qu'il n'a pas appelé, ou par défaut — passerait
    // les sept assertions ci-dessus sans en rougir aucune.
    for indicative in [
        "quotes, word for word, what the",
        "This is measured, not guessed",
    ] {
        assert!(
            !reprompt.contains(indicative),
            "mika#3003 — with no tool read this turn, nothing may be asserted in the \
             indicative; found {indicative:?} in:\n{reprompt}"
        );
    }

    // Fail-closed, unchanged: the second response is no better, so the disposition still falls.
    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(&trace, &["Disposition: READY"]);
    grounding_assertions::assert_response_contains(&trace, "Disposition: ESCALATE");
    grounding_assertions::assert_response_contains(&trace, ENGINE_FINDING_MARKER);

    Ok(())
}

/// Le corps du ticket mika#3003, servi par un outil pendant le tour.
///
/// C'est `gh_read issue_view` qui l'avait rendu le 2026-10-31 ; ici c'est `read_agent_file`,
/// qui lit un fichier du home de l'agent — déterministe, sans réseau. **Le nom de l'outil est
/// volontairement un autre que celui de l'incident** : ce vol n'a pas de cas spécial pour
/// `gh_read`, il interroge le registre de lecture du tour, donc n'importe quel outil ayant
/// rendu le texte prouve le mécanisme. Un test qui n'aurait passé que sur `gh_read` aurait
/// laissé passer une implémentation par liste d'outils, que le plan écarte nommément.
///
/// Le corps est en français et [`BRIEF`] en anglais : aucune fenêtre de 40 caractères ne peut
/// passer de l'un à l'autre par accident, donc l'ancre fautive est refusée pour la bonne
/// raison (`QuoteNotInBrief`) et pas par collision de fixture.
const TICKET_BODY_3003: &str = "# mika#3003 — mika-arch cite le corps du ticket\n\n\
     La garde d'ancrage de mika#2037 retient une disposition de mika-arch quand ses citations \
     ne viennent pas du brief, c'est-à-dire du plan revu. Le 2026-10-31, l'architecte a cité \
     le corps du ticket, qu'il avait lu avec un outil, au lieu du plan.\n\n\
     La garde a eu raison : elle n'est pas en cause. Le défaut est que l'architecte ne sait \
     pas, à la relance, que sa citation vient du ticket et non du plan.\n";

/// Test 2e: **mika#3003 AC2, moitié positive** — le rejeu de la forme mesurée le 2026-10-31.
///
/// Même géométrie que le test 2a, à une chose près et c'est tout le vol : un pas d'outil
/// précède la revue. L'architecte lit un document, puis son `A1` en cite la première phrase
/// mot pour mot tandis que `A2` et `A3` citent le brief à deux régions disjointes. 3 ancres
/// trouvées, 2 valides, `QuoteNotInBrief` — exactement la trace `f2a64bd7`.
///
/// Ce que ce test attest et que 2a ne peut pas voir :
///
/// - le re-prompt passe à l'**indicatif** et **nomme l'outil** qui a rendu le texte ;
/// - il dit que c'est mesuré, parce que ça l'est : le registre a retrouvé l'extrait dans la
///   sortie de cet outil, à la fenêtre du matcher et au seuil du contrat ;
/// - la phrase conditionnelle de mika#2794 **disparaît** pour cette ancre — deux réponses à
///   la même question seraient pires qu'une seule imprécise ;
/// - l'extrait de la ligne fautive est toujours cité, guillemets doublés compris (mika#2794) ;
/// - et le fail-closed de mika#2037 avec le fail-visible de mika#2338 ne bougent pas d'un
///   octet : une seconde réponse non corrigée finit sur `Disposition: ESCALATE`, portant la
///   ligne de constat du moteur et `anchors_valid=2`. **Un re-prompt plus précis est une
///   tentative d'obéissance, jamais une relaxation de la garde** (interdit du brief, doctrine
///   mika#1190 pour le risque nommé).
#[tokio::test]
async fn test_review_anchor_reprompt_names_the_tool_that_produced_the_quote() -> anyhow::Result<()>
{
    // `A1` cite la première phrase du corps du ticket ; `A2` et `A3` citent le brief.
    let two_of_three = "A1: \"La garde d'ancrage de mika#2037 retient une disposition de mika-arch quand ses citations ne viennent pas du brief\" — noté.\n\
         A2: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct.\n\
         A3: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes.\n\
         \n\
         Still nothing to add. The plan is fine as written.\n\
         \n\
         Disposition: READY";
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        ARCH_SUFFIX_UNION,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);
    let harness = EvalHarness::builder()
        .responses(vec![
            // Le pas qui alimente le registre de lecture du tour.
            tool_call_response(
                "read_agent_file",
                serde_json::json!({"path": "ticket-3003.md"}),
            ),
            text_response(two_of_three),
            text_response(two_of_three),
        ])
        .skills(skills)
        .build()
        .await?;

    // Le document que l'outil rendra, dans le home de l'agent — `read_agent_file` sans
    // paramètre `agent` lit sous cette racine exactement.
    std::fs::write(
        harness.home_dir.path().join("ticket-3003.md"),
        TICKET_BODY_3003,
    )?;

    let trace = harness.run(BRIEF).await?;

    let requests = harness.mock().captured_requests();
    // Le re-prompt est localisé **par son contenu**, jamais par un index : un pas d'outil
    // s'intercale ici, donc la relance n'est pas la deuxième requête capturée mais la
    // troisième. Chercher par contenu est ce qui garde ce test vrai si un futur vol ajoute
    // ou retire un pas avant la revue.
    let reprompt = requests
        .iter()
        .flat_map(user_texts)
        .find(|t| t.contains("What blocked it"))
        .expect("one request must carry the review-anchor corrective re-prompt");

    // L'indicatif, et le nom de l'outil qui a réellement rendu le texte.
    assert!(
        reprompt.contains(
            "A1 quotes, word for word, what the `read_agent_file` tool returned during this turn"
        ),
        "mika#3003 AC1 — the re-prompt must name the tool that produced the quote, got:\n{reprompt}"
    );
    assert!(
        reprompt.contains("This is measured, not guessed"),
        "mika#3003 — the sentence must say that the origin was established, not guessed, \
         got:\n{reprompt}"
    );
    // Et la conditionnelle de mika#2794 s'efface : l'origine est établie, donc elle n'est
    // plus une hypothèse. Deux phrases répondant à la même question en deux humeurs
    // opposées, c'est la forme que le modèle skimme.
    assert!(
        !reprompt.contains("usually comes from"),
        "mika#3003 — the hedged sentence must give way once the origin is established, \
         got:\n{reprompt}"
    );

    // mika#2794 n'a rien perdu : la ligne fautive est toujours nommée et citée, guillemets
    // doublés compris — l'extérieur est au moteur, l'intérieur au modèle.
    assert!(
        reprompt.contains("Anchors refused:"),
        "the re-prompt must still open the refused list, got:\n{reprompt}"
    );
    assert!(
        reprompt.contains("A1: \"\"La garde d'ancrage de mika#2037 retient une disposition"),
        "the re-prompt must still quote the faulty line's opening, got:\n{reprompt}"
    );
    assert!(
        reprompt.contains("Anchors accepted: A2, A3"),
        "the re-prompt must still name the two anchors that passed, got:\n{reprompt}"
    );

    // Fail-closed et fail-visible, inchangés.
    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(&trace, &["Disposition: READY"]);
    grounding_assertions::assert_response_contains(&trace, "Disposition: ESCALATE");
    grounding_assertions::assert_response_contains(&trace, ENGINE_FINDING_MARKER);
    grounding_assertions::assert_response_contains(&trace, "anchors_valid=2");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 2b: the second-review mirror. On the same union of declared lines, a withdrawn
/// `Verdict: GROOMED` becomes `Verdict: ESCALATE` — its own family, not the first ESCALATE
/// in the list.
#[tokio::test]
async fn test_review_anchor_escalates_second_pass_in_its_own_family() -> anyhow::Result<()> {
    let two_of_three = "A1: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct.\n\
         A2: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes.\n\
         A3: the threshold for repeated auth failures is left open.\n\
         \n\
         Verdict: GROOMED";
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        // The shipped second-review skill name: a verdict producer, so the #1133
        // dev-groom fabrication guard stays out of the way as it does in production.
        "mika-arch-second-review",
        &["second-review"],
        ARCH_SUFFIX_UNION,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);
    let brief = format!("second-review: {BRIEF}");
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(two_of_three),
            text_response(two_of_three),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(&brief).await?;

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(
        &trace,
        &["Verdict: GROOMED", "Disposition: ESCALATE"],
    );
    grounding_assertions::assert_response_contains(&trace, "Verdict: ESCALATE");
    grounding_assertions::assert_response_contains(&trace, ENGINE_FINDING_MARKER);

    Ok(())
}

/// Test 2c: the fallback. A skill that declares no `ESCALATE` of the withdrawn family cannot
/// be escalated in that family, so the withheld marker still replaces the disposition
/// (unreachable with the shipped arch manifests, kept as the defensive floor).
#[tokio::test]
async fn test_review_anchor_withholds_when_no_escalate_of_the_family_is_declared()
-> anyhow::Result<()> {
    // `Verdict: GROOMED` is declared (the guard must fire on it) but no `Verdict: ESCALATE`
    // is: the withdrawn line has no ESCALATE of its own family to become.
    let mut lines: Vec<&str> = GROOM_SUFFIX_LINES.to_vec();
    lines.push("Verdict: GROOMED");
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        // The shipped second-review skill name: a verdict producer, so the #1133
        // dev-groom fabrication guard stays out of the way as it does in production.
        "mika-arch-second-review",
        &["second-review"],
        &lines,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);
    let brief = format!("second-review: {BRIEF}");
    let stub = "All prior findings resolved.\n\nVerdict: GROOMED";
    let harness = EvalHarness::builder()
        .responses(vec![text_response(stub), text_response(stub)])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(&brief).await?;

    assert_has_output(&trace);
    grounding_assertions::assert_response_forbids(&trace, &["Verdict: GROOMED", "ESCALATE"]);
    grounding_assertions::assert_response_contains(&trace, WITHHELD_MARKER);

    Ok(())
}

/// Test 2d: the mika#2335 case seen from the harness. The brief is markdown; the architect
/// quotes it with the markup dropped. That is a genuine review and must be accepted on the
/// first turn — the guard exists to refuse forgeries, not renderings.
#[tokio::test]
async fn test_review_anchor_accepts_a_rendered_quote_of_a_markdown_brief() -> anyhow::Result<()> {
    let markdown_brief = BRIEF
        .replace(
            "The cycle token is re-resolved through a TokenResolver trait",
            "**The cycle token is re-resolved** through a `TokenResolver` trait",
        )
        .replace(
            "is the trace exhaustive, should every refresh be journaled at INFO level",
            "is the trace **exhaustive**, should every refresh be journaled at `INFO` level",
        )
        .replace(
            "I have not fixed N for the repeated authentication failure threshold",
            "I have not fixed `N` for the **repeated authentication failure** threshold",
        );
    let rendered_review = "A1: \"The cycle token is re-resolved through a TokenResolver trait, never frozen at spawn\" — correct.\n\
         A2: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes.\n\
         A3: \"I have not fixed N for the repeated authentication failure threshold\" — a duration.\n\
         \n\
         Disposition: READY";
    let harness = EvalHarness::builder()
        .responses(vec![text_response(rendered_review)])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(&markdown_brief).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "a rendered quote of a markdown brief is a real review; no re-prompt"
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");
    grounding_assertions::assert_response_forbids(
        &trace,
        &[ENGINE_FINDING_MARKER, WITHHELD_MARKER],
    );

    Ok(())
}

/// Test 3: a genuine anchored review passes on the first turn. This is the anti-vacuity
/// direction that "reject everything" would fail — without it the guard would block all
/// grooming.
#[tokio::test]
async fn test_review_anchor_no_op_on_anchored_ready() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(ANCHORED_REVIEW)])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "A real anchored review must pass with no re-prompt, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 4: the terminal half stays mika#901's. An ITERATE with an F-list and no anchors must
/// not be asked for anchors — the two guards partition the disposition space, they do not
/// overlap.
#[tokio::test]
async fn test_review_anchor_no_op_on_terminal_disposition() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "F1: (BLOCKING) The threshold is expressed in cycles, not duration.\n\
                Concern: poll_interval is operator-configurable.\n\
                Change required: express the threshold as a duration.\n\
                Citation: review-guide.md\n\n\
             Disposition: ITERATE",
        )])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "Terminal dispositions belong to the mika#901 guard, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: ITERATE");

    Ok(())
}

/// Test 5: `Verdict: GROOMED` is the second non-terminal disposition and is covered too.
/// The second-review pass commits the plan just as the first does.
#[tokio::test]
async fn test_review_anchor_caught_on_unanchored_groomed() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-second-review",
        &["second-review"],
        SECOND_REVIEW_SUFFIX_LINES,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);

    let brief = format!("second-review: {BRIEF}");
    let anchored = ANCHORED_REVIEW.replace("Disposition: READY", "Verdict: GROOMED");

    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("All prior findings resolved.\n\nVerdict: GROOMED"),
            text_response(&anchored),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(&brief).await?;

    assert!(
        trace.llm_call_count > 1,
        "Expected the guard to fire on an unanchored GROOMED, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "A1:");

    Ok(())
}

/// Test 6: opt-in. A skill that declares no anchor contract is untouched, whatever it emits.
#[tokio::test]
async fn test_review_anchor_no_op_when_undeclared() -> anyhow::Result<()> {
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        GROOM_SUFFIX_LINES,
        &[], // no anchor contract declared
        FINDING_PREFIXES,
    )]);

    let harness = EvalHarness::builder()
        .responses(vec![text_response(UNANCHORED_STUB)])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "An undeclared contract must never fire the guard, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

/// Test 7: retry budgets are independent. An F-list re-prompt must not consume the anchor
/// guard's single corrective attempt — otherwise a turn that trips mika#901 first would then
/// get its unanchored READY accepted on the very next response.
#[tokio::test]
async fn test_review_anchor_retry_budget_independent_of_finding_list() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Turn 1: terminal disposition, no F-list → mika#901 guard fires.
            text_response("Concerns remain.\n\nDisposition: ITERATE"),
            // Turn 2: switches to READY with no anchors → mika#2037 guard must still have
            // its own re-prompt available.
            text_response(UNANCHORED_STUB),
            // Turn 3: the anchored review.
            text_response(ANCHORED_REVIEW),
        ])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 3,
        "Expected both guards to fire once each (3 LLM calls), got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "A1:");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 8: an ad-hoc question to mika-arch is not a plan review. All three arch skills are
/// `always_on`, so skill-match alone arms the contract on every turn of that agent — and the
/// #864 guard forces a disposition on every turn. Without a brief-length carve-out,
/// `/mika-ask-arch` could never return READY again: a short question cannot supply three
/// non-overlapping 40-character quotes, so the guard would re-prompt and then withhold.
#[tokio::test]
async fn test_review_anchor_no_op_on_short_ad_hoc_question() -> anyhow::Result<()> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Oui — le placement dans le corps du cycle est correct.\n\nDisposition: READY",
        )])
        .skills(groom_skill())
        .build()
        .await?;

    // The shape of a `/mika-ask-arch` question: keyword-matched, far below the arming length.
    let trace = harness
        .run("groom-ticket: le correctif doit-il vivre dans le spawn ou dans le cycle ?")
        .await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "a short question must not arm the guard, got {} LLM call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");
    grounding_assertions::assert_response_forbids(&trace, &[WITHHELD_MARKER]);

    Ok(())
}

/// Test 9: a short corrective prompt, the shape the mika#1823 UNPARSED-recovery re-ask had
/// before mika#2996. Below the arming threshold the guard stays a no-op — that boundary is
/// still the engine's, and still right for a brief that cannot contain the proof.
///
/// **This is no longer what the retry sends.** Since mika#2996 `dispatch-lib.sh` carries the
/// plan in the retry message, so the guard arms on that turn and checks anchors against the
/// plan: a short re-ask let an unanchored READY advance the chain (trace `15ea28bd`). The
/// retry path itself is tested end to end in `review_anchor_unparsed_retry_2996.rs`.
#[tokio::test]
async fn test_review_anchor_no_op_on_unparsed_recovery_prompt() -> anyhow::Result<()> {
    let corrective_prompt = "Your previous plan-review response is missing the required \
        `Disposition:` line.\n\nPlease re-emit your findings and end with exactly ONE of these \
        three lines as the last non-empty line of your response:\n\n    Disposition: READY\n    \
        Disposition: ITERATE\n    Disposition: ESCALATE\n\nThe routing engine parses this line \
        as the verdict — its absence blocks the pipeline (see mika#1823). groom-ticket";

    assert!(
        corrective_prompt.chars().count() < 2000,
        "the corrective prompt must stay below the arming threshold for this test to mean \
         anything (got {})",
        corrective_prompt.chars().count()
    );

    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "F1: le seuil est exprimé en cycles.\n\nDisposition: READY",
        )])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(corrective_prompt).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "the recovery re-ask must not arm the guard against its own corrective prompt, got {}",
        trace.llm_call_count
    );
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: READY");

    Ok(())
}

// ---------------------------------------------------------------------------
// mika#3003 U2 bis — le DÉNOMINATEUR d'AC3, lu sur le journal du tour réel.
//
// AC3 demande « le compte de `review_anchor_withheld` **avec son dénominateur**
// (passages non terminaux) ». Ce dénominateur n'existait pas : les deux lignes
// préexistantes ne sont émises que sur les branches fautives, donc leur rapport
// mesure « retenues / refus », jamais « retenues / passages ». La ligne
// `guard.review_anchor_armed` est ce dénominateur, et elle n'a de valeur que si
// elle est émise sur **toutes** les branches armées, la saine comprise — une
// ligne absente de la branche saine ferait de son compte un second numérateur.
//
// Ces deux sondes ne sont pas dans `src/` parce qu'elles ne peuvent pas y être :
// la ligne est émise au fond de `run_loop`, qu'un test unitaire du module de la
// garde n'atteint pas. Elles lisent donc le journal d'un tour `run_agent` entier.
// ---------------------------------------------------------------------------

/// Un événement capturé : son message, puis ses champs structurés.
type Captured = std::sync::Arc<std::sync::Mutex<Vec<(String, HashMap<String, String>)>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let message = fields.remove("message").unwrap_or_default();
        if let Ok(mut events) = self.events.lock() {
            events.push((message, fields));
        }
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// `tracing_subscriber::registry()` installe un dispatcher mono-thread si rien ne
/// retient l'autre forme ; sans ce garde, un `set_default` concurrent d'un test
/// voisin perd les événements.
fn keep_registry_multi_dispatcher() {
    static KEEPER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEPER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
}

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    keep_registry_multi_dispatcher();
    let events: Captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: std::sync::Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

/// Les champs des événements portant exactement ce nom, dans l'ordre d'émission.
fn lines_named(events: &Captured, name: &str) -> Vec<HashMap<String, String>> {
    events
        .lock()
        .expect("captured events mutex")
        .iter()
        .filter(|(_, fields)| fields.get("event").map(String::as_str) == Some(name))
        .map(|(_, fields)| fields.clone())
        .collect()
}

/// U2 bis, moitié SAINE : la garde s'arme, elle est satisfaite, et la ligne du
/// dénominateur est émise quand même.
///
/// Ce que cette sonde attache, et que la moitié fautive ne peut pas voir :
///
/// - `guard.review_anchor_armed` est émise sur la branche `Satisfied`. C'est
///   toute la valeur du dénominateur : un passage non terminal qui *réussit* est
///   un passage, et une ligne qui ne sortirait que sur un refus donnerait un
///   rapport de 1 à chaque fenêtre de mesure.
/// - `anchors_found` et `anchors_valid` sont **absents**, pas à zéro. Sur la
///   branche saine la garde ne compte rien : `AnchorVerdict::Satisfied` ne porte
///   aucun compteur, et un `0` rendu là mentirait sur une réponse à trois ancres
///   valides (motif mika#2331). Ils sont émis en `Option<usize>` pour cela.
/// - `withheld` vaut `false` et `attributed_sources` est présent et vide : aucun
///   outil n'a tourné, donc rien n'est attribuable, et c'est une chaîne vide
///   plutôt qu'un champ absent (classe mika#2340 — un champ absent dirait
///   « binaire antérieur », pas « rien attribué »).
/// - Les deux lignes préexistantes sont absentes : elles mesurent le refus, pas
///   le passage. Sans cette assertion, une ligne `armed` émise au mauvais endroit
///   passerait les quatre précédentes.
#[tokio::test]
async fn test_review_anchor_armed_line_on_the_satisfied_branch_3003() -> anyhow::Result<()> {
    let (_guard, events) = capture();

    let harness = EvalHarness::builder()
        .responses(vec![text_response(ANCHORED_REVIEW)])
        .skills(groom_skill())
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;

    assert_eq!(
        trace.llm_call_count, 1,
        "the anchored review must be accepted on the first pass, got {} call(s)",
        trace.llm_call_count
    );
    assert_has_output(&trace);

    let armed = lines_named(&events, "guard.review_anchor_armed");
    assert_eq!(
        armed.len(),
        1,
        "mika#3003 AC3 — exactly one armed line per guard evaluation, got {}: {armed:?}",
        armed.len()
    );
    let line = &armed[0];

    assert_eq!(
        line.get("withheld").map(String::as_str),
        Some("false"),
        "the satisfied branch withholds nothing: {line:?}"
    );
    assert_eq!(
        line.get("attributed_sources").map(String::as_str),
        Some(""),
        "mika#3003 — `attributed_sources` is present and EMPTY when nothing was attributed, \
         never absent: {line:?}"
    );
    assert!(
        !line.contains_key("anchors_found") && !line.contains_key("anchors_valid"),
        "mika#3003 — the satisfied branch counts nothing, so the two counters must render \
         ABSENT rather than zero (mika#2331): {line:?}"
    );
    assert!(
        line.get("brief_chars")
            .and_then(|v| v.parse::<usize>().ok())
            .is_some_and(|n| n > 0),
        "the armed line must carry the brief it armed on: {line:?}"
    );

    assert!(
        lines_named(&events, "guard.review_anchor").is_empty()
            && lines_named(&events, "guard.review_anchor_withheld").is_empty(),
        "the two pre-existing lines measure a refusal; a satisfied pass emits neither"
    );

    Ok(())
}

/// U2 bis, moitié FAUTIVE : deux passages, deux lignes, et le champ neuf présent
/// et vide sur les deux lignes préexistantes.
///
/// Même rejeu que le test 2 — deux réponses identiques à 2 ancres sur 3 — lu sur
/// le journal. Ce qu'il attache :
///
/// - **Deux** lignes `armed` pour un tour qui relance une fois : une par
///   *évaluation*. Le dénominateur d'AC3 est « passages non terminaux », et une
///   relance est un passage ; compter les tours au lieu des passages sous-estimerait
///   le dénominateur d'un facteur qui dépend du taux de relance.
/// - `withheld` passe de `false` à `true` entre les deux. Le numérateur d'AC3 se
///   lit donc aussi sur cette ligne seule, sans jointure avec l'ERROR.
/// - `anchors_found=3` / `anchors_valid=2` sont **présents** ici — l'exact
///   complément de l'absence attachée par la sonde saine. Les deux assertions
///   ensemble disent que les compteurs suivent la branche, et non la ligne.
/// - `attributed_sources` est présent et **vide** sur `guard.review_anchor`
///   (WARN) et sur `guard.review_anchor_withheld` (ERROR). Aucun outil n'a
///   tourné, donc le registre est vide : c'est le contrôle négatif du champ, et
///   sans lui un champ qui ne serait émis *que* lorsqu'une attribution existe
///   laisserait le lecteur d'AC3 incapable de distinguer « rien attribué » de
///   « binaire antérieur au vol ».
#[tokio::test]
async fn test_review_anchor_armed_line_on_the_missing_branch_3003() -> anyhow::Result<()> {
    let (_guard, events) = capture();

    let two_of_three = "A1: \"re-resolves the manager cycle token before every cycle instead of freezing it\" — correct.\n\
         A2: \"is the trace exhaustive, should every refresh be journaled at INFO level\" — yes.\n\
         A3: the threshold for repeated auth failures is left open, express it as a duration.\n\
         \n\
         Still nothing to add. The plan is fine as written.\n\
         \n\
         Disposition: READY";
    let skills = SkillRegistry::from_test_entries(vec![make_anchor_skill(
        "arch-groom-ticket",
        &["groom-ticket"],
        ARCH_SUFFIX_UNION,
        ANCHOR_PREFIXES,
        FINDING_PREFIXES,
    )]);
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(two_of_three),
            text_response(two_of_three),
        ])
        .skills(skills)
        .build()
        .await?;

    let trace = harness.run(BRIEF).await?;
    assert_has_output(&trace);
    grounding_assertions::assert_response_contains(&trace, "Disposition: ESCALATE");

    let armed = lines_named(&events, "guard.review_anchor_armed");
    assert_eq!(
        armed.len(),
        2,
        "mika#3003 AC3 — one armed line per PASS, so a re-prompting turn emits two, got {}: \
         {armed:?}",
        armed.len()
    );
    assert_eq!(
        armed[0].get("withheld").map(String::as_str),
        Some("false"),
        "the first pass re-prompts, it does not withhold: {:?}",
        armed[0]
    );
    assert_eq!(
        armed[1].get("withheld").map(String::as_str),
        Some("true"),
        "the second pass withholds — this is AC3's numerator on the armed line itself: {:?}",
        armed[1]
    );
    for line in &armed {
        assert_eq!(
            line.get("anchors_found").map(String::as_str),
            Some("3"),
            "the missing branch carries its counters: {line:?}"
        );
        assert_eq!(
            line.get("anchors_valid").map(String::as_str),
            Some("2"),
            "the missing branch carries its counters: {line:?}"
        );
        assert_eq!(
            line.get("attributed_sources").map(String::as_str),
            Some(""),
            "no tool ran this turn, so the ledger is empty — present and empty, never absent: \
             {line:?}"
        );
    }

    for name in ["guard.review_anchor", "guard.review_anchor_withheld"] {
        let lines = lines_named(&events, name);
        assert_eq!(
            lines.len(),
            1,
            "{name} must be emitted exactly once on this replay, got {}: {lines:?}",
            lines.len()
        );
        assert_eq!(
            lines[0].get("attributed_sources").map(String::as_str),
            Some(""),
            "mika#3003 — the new field is present and EMPTY on {name} when nothing was \
             attributed (mika#2340: an absent field would mean an older binary): {:?}",
            lines[0]
        );
    }

    Ok(())
}
