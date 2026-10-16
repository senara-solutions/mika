//! Un tour de revue ne finit pas sans avoir **soumis** la revue (mika#2727).
//!
//! # Le défaut que ce module ferme
//!
//! `skills/bundled/qa-review/skill.toml` déclare
//! `required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]`. La
//! garde `required_tools` de `agent_loop::run_loop` vérifie **qu'un outil de ce
//! nom a été appelé**, jamais ce qu'il a fait. Un `run_gh` en lecture —
//! `pr list --head …`, `pr view` — satisfait donc le terme `run_gh` alors que la
//! raison d'être de ce terme est l'**écriture** du verdict sur la PR.
//!
//! Le 2026-10-15, deux tours de revue de #2704 sont partis sur deux têtes
//! différentes (`4a216bf7` à 15:11:52Z, `3f2c3fa4` à 19:11:36Z), ont rendu leur
//! verdict **en texte seulement**, et ont fini en `EndTurn` sans qu'aucune revue
//! n'atteigne GitHub. Les deux ont exigé une relance humaine. Le budget de
//! re-prompt de `required_tools` avait brûlé au pas 0, sur l'absence d'outils ;
//! quand les outils sont arrivés, elle n'avait plus rien à dire sur leur contenu.
//!
//! # Pourquoi un module, frère de `qa_build_callback`
//!
//! La garde mika#2355 ([`crate::qa_build_callback`]) couvre l'**autre bout du
//! même tour** : le callback de `build_mika`. Sa population est
//! `is_build_callback(msg) ∧ qa-review chargé`. Les deux tours incriminés ne sont
//! pas des callbacks — ce sont les tours d'**ouverture** de revue, partis d'un
//! événement webhook `[GitHub] PR …`, et ils n'ont jamais appelé `build_mika`,
//! donc aucun callback n'a jamais suivi. mika#2355 est restée hors population,
//! correctement.
//!
//! Ce module porte donc la même grammaire — une population, un label, une
//! consigne, un prédicat de satisfaction, un complémentaire — sur l'autre
//! population. Les deux préfixes de message sont disjoints
//! (`[callback: long_running:build_mika]` contre `[GitHub] PR `), donc les deux
//! gardes ne peuvent pas être dues sur un même tour : propriété épinglée par
//! [`tests::mika2727_les_deux_gardes_ne_sont_jamais_dues_sur_le_meme_message`].
//!
//! # Ce qu'on ne lit PAS : la carte de session `pr_reviews_posted` (#821)
//!
//! Le ticket nomme le signal (« celui que la session pose déjà »), et c'est
//! [`crate::qa_build_callback::pr_review_posted_in_turn`] — le prédicat **de
//! tour**. Ce n'est pas la carte de dédup de session : sa clé
//! (`format_pr_dedup_key`) ne porte **aucune tête**, donc une revue rendue sur une
//! tête antérieure y supposerait la garde satisfaite sur une tête nouvelle —
//! exactement l'incident mesuré, deux fois, puisque #2704 portait déjà une revue
//! mika-qa sur une tête précédente. Armer la garde là-dessus l'armerait sur la
//! condition qui l'empêche de voir le défaut (motif mika#2272 : *une condition
//! d'armement insatisfiable se lit comme une flotte en bonne santé*).

use std::collections::HashSet;

use crate::qa_build_callback::{self, BUILD_MIKA_TOOL, QA_REVIEW_SKILL};
use crate::tool_execution::ToolCallSummary;

/// Les quatre actions de PR que la gateway route vers mika-qa.
///
/// Source de vérité : `mika-gateway/src/github.rs` (et mika#1822 pour
/// `ready_for_review`). `closed` est **délibérément absent** : il part à
/// mika-dev, et une garde qui l'inclurait re-prompterait mika-dev pour poster une
/// revue qu'il ne doit à personne.
pub const REVIEW_TRIGGERING_ACTIONS: &[&str] = &[
    "opened",
    "synchronize",
    "review_requested",
    "ready_for_review",
];

/// Ce message est-il l'un des événements webhook de PR qui ouvrent une revue ?
///
/// Lit la grammaire de fil par son **lecteur unique déjà en place**,
/// [`crate::server::webhook_queue_v2::parse_pr_action_event`]. Aucun second
/// extracteur n'est écrit ici : une grammaire de fil dupliquée est la façon dont
/// deux lecteurs divergent en silence (doctrine mika#2158).
pub fn is_review_triggering_event(msg: &str) -> bool {
    crate::server::webhook_queue_v2::parse_pr_action_event(msg).is_some_and(
        |(action, _repo, _pr)| {
            REVIEW_TRIGGERING_ACTIONS
                .iter()
                .any(|a| a.eq_ignore_ascii_case(action))
        },
    )
}

/// Une revue soumise est-elle **due** sur ce tour ?
///
/// Conjonction, et les deux moitiés comptent — même raison, mot pour mot, que
/// [`crate::qa_build_callback::qa_verdict_required`]. Le premier terme seul
/// (« un webhook de PR est arrivé ») re-prompterait mika-dev, qui reçoit
/// `pull_request.closed`, `check_suite.completed` et
/// `pull_request_review.submitted`. Le second seul (« `qa-review` est chargé »)
/// est quasi toujours vrai : ce skill est `always_on = true`, donc présent sur
/// **tous** les tours mika-qa, y compris une question d'opérateur en conversation.
pub fn review_submission_required(
    msg: &str,
    skill_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> bool {
    is_review_triggering_event(msg)
        && skill_names
            .into_iter()
            .any(|n| n.as_ref().eq_ignore_ascii_case(QA_REVIEW_SKILL))
}

/// Label de la garde, pour `intent_guard_retries` (budget de re-prompt unique).
pub const REVIEW_SUBMISSION_LABEL: &str = "qa_review_submission";

/// Un `build_mika` a-t-il été **lancé** avec succès dans ce tour ?
///
/// # La seule exception nommée de la garde, et pourquoi elle est là
///
/// Le flux nominal d'une revue **finit son tour sans revue soumise**, et c'est
/// voulu : le tour lance `build_mika` en asynchrone et rend la main pour attendre
/// le callback — l'en-tête de [`crate::qa_build_callback`] le dit dans ces termes,
/// et c'est là que la garde mika#2355 prend le relais. Une garde qui ignorerait
/// cette réserve re-prompterait **tous** les tours de revue sains.
///
/// L'exception est donc par **valeur nommée** ([`BUILD_MIKA_TOOL`], l'outil du
/// skill `build-mika`), jamais par allowlist de tours ou d'agents, et elle est
/// grep-visible depuis le nom de la garde qui la relaie. Son test auto-nettoyant
/// est la suite mika#2355 elle-même : si ce relais cesse d'exister,
/// `test_qa_build_callback_verdict_2355` rougit avant que cette réserve ne
/// devienne un trou.
///
/// Les deux incidents de mika#2727 tombent bien dans la population restante : le
/// ticket établit qu'aucun des deux tours n'a appelé `build_mika` — c'est même
/// l'un des outils que le re-prompt du pas 0 réclamait.
pub fn build_dispatched_in_turn(summaries: &[ToolCallSummary]) -> bool {
    summaries
        .iter()
        .any(|s| s.name == BUILD_MIKA_TOOL && s.success)
}

/// La revue a-t-elle été soumise, ou un build a-t-il été lancé (réserve D3) ?
///
/// Disjonction. Le premier terme est
/// [`crate::qa_build_callback::pr_review_posted_in_turn`] — réutilisé et non
/// réécrit : c'est le signal que le ticket nomme, déjà épinglé comme équivalent
/// au prédicat d'early-accept `has_successful_pr_review` (#695/#821) par
/// `qa_build_callback::tests::mika2355_the_satisfied_predicate_matches_the_early_accept_one`.
/// Un second prédicat écrit ici serait un second lecteur du même fait.
pub fn review_submission_satisfied(summaries: &[ToolCallSummary]) -> bool {
    qa_build_callback::pr_review_posted_in_turn(summaries) || build_dispatched_in_turn(summaries)
}

/// Ce tour devait une revue, son budget de re-prompt est épuisé, et rien n'a été
/// soumis ni lancé.
///
/// **Complémentaire exact** de la garde : même conjonction, le terme de budget
/// inversé. Là où la garde re-prompte (budget non consommé), ceci dit que le
/// re-prompt a échoué **aussi** — la population de l'AC2 du ticket
/// (`qa_turn_ended_without_review`), dont l'émetteur part en vol 2 parce que le
/// SHA de tête n'est sur aucun fil (cf. `docs/plans/2026-10-16-001-…`, D6).
///
/// Livré ici, et pas avec son consommateur, pour la raison que porte déjà
/// [`crate::qa_build_callback::verdict_unmet_after_retry`] : le site « budget
/// déjà consommé » **n'existe pas comme branche** — la condition
/// `!intent_guard_retries.contains(…)` est *dans* le `if` du re-prompt, et budget
/// épuisé on tombe à travers, sans `else`. Le prédicat se pose donc sur les deux
/// chemins de sortie EndTurn, et deux copies d'une conjonction à trois termes
/// divergent (leçon mika#2158).
pub fn submission_unmet_after_retry(
    due: bool,
    retries: &HashSet<&'static str>,
    summaries: &[ToolCallSummary],
) -> bool {
    due && retries.contains(REVIEW_SUBMISSION_LABEL) && !review_submission_satisfied(summaries)
}

/// Consigne du re-prompt : nomme l'outil, la ligne attendue, ce qui ne vaut pas
/// soumission, et la réserve D3.
///
/// Suit la leçon du doc-comment de
/// [`crate::qa_build_callback::QA_VERDICT_REQUIRED_CORRECTION`] : la revue postée
/// est la source de vérité, le texte de la réponse n'en est qu'un miroir. Dire
/// seulement « poste la revue » laisserait le modèle libre de relire son propre
/// texte comme une soumission — c'est littéralement ce que les deux tours
/// incriminés ont fait.
pub const REVIEW_SUBMISSION_CORRECTION: &str = "[mika-engine] This is a qa-review turn opened by a pull-request webhook, and \
     the review has not been submitted: no successful `run_gh` call with `pr review` \
     appears in this turn's tool history. A qa-review turn concludes by POSTING the \
     review to GitHub — the posted review is the source of truth, and verdict text \
     in your response is only a mirror. Call `run_gh` with `pr review` carrying a \
     trailing `VERDICT:` line (`pass` / `hold[review]` / `block[ac]` / `block[ci]` / \
     `block[security]` / `block[pipeline]`). A read-only `run_gh` (`pr list`, \
     `pr view`) does not submit anything, and neither does `update_task_status` nor \
     `send_message` — they do not deliver a verdict to the PR. If the build has not \
     run yet, dispatching `build_mika` is a valid way to end this turn: the build \
     callback resumes the review and posts the verdict then.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Un événement webhook de PR tel que la gateway l'écrit
    /// (`mika-gateway/src/github.rs`), reconstruit ici plutôt que recopié à
    /// moitié : la garde se lit sur la **première ligne**, et une fixture sans
    /// seconde ligne ne prouverait pas qu'elle la tolère.
    fn pr_event(action: &str) -> String {
        format!(
            "[GitHub] PR {action}: senara-solutions/mika#2704 — fix(x): y (branch: fix/2727/x)\n\
             https://github.com/senara-solutions/mika/pull/2704"
        )
    }

    fn summary(name: &str, input: &str, success: bool) -> ToolCallSummary {
        ToolCallSummary {
            step: 0,
            name: name.to_string(),
            input_summary: input.to_string(),
            output_summary: String::new(),
            success,
            non_zero_exit: false,
        }
    }

    const QA_SKILLS: [&str; 2] = ["qa-review", "build-mika"];

    #[test]
    fn mika2727_une_revue_est_due_sur_les_quatre_actions_routees_vers_mika_qa() {
        assert_eq!(
            REVIEW_TRIGGERING_ACTIONS.len(),
            4,
            "anti-vacuité : la boucle ci-dessous doit regarder quatre actions"
        );
        for action in REVIEW_TRIGGERING_ACTIONS {
            assert!(
                review_submission_required(&pr_event(action), QA_SKILLS),
                "action routée vers mika-qa : {action}"
            );
        }
    }

    #[test]
    fn mika2727_closed_va_a_mika_dev_et_ne_doit_aucune_revue() {
        assert!(!review_submission_required(&pr_event("closed"), QA_SKILLS));
        // Contrôle négatif sur une action fabriquée : la garde lit une liste
        // nommée, pas « toute action de PR ».
        assert!(!review_submission_required(
            &pr_event("zorglubbed"),
            QA_SKILLS
        ));
    }

    #[test]
    fn mika2727_sans_qa_review_charge_rien_nest_du() {
        // La topologie mika-dev : il reçoit des webhooks de PR et n'a pas
        // `qa-review`. C'est la moitié qui compte.
        assert!(!review_submission_required(
            &pr_event("opened"),
            ["self-dev", "dev-pilot", "build-mika"]
        ));
    }

    #[test]
    fn mika2727_un_message_qui_nest_pas_un_evenement_de_pr_nest_pas_une_population() {
        for msg in [
            "Peux-tu relire la PR #2704 ?",
            "[GitHub] issue opened: senara-solutions/mika#2727",
            "",
        ] {
            assert!(
                !is_review_triggering_event(msg),
                "hors population : {msg:?}"
            );
        }
    }

    #[test]
    fn mika2727_une_revue_postee_satisfait_la_garde() {
        let posted = [summary(
            "run_gh",
            r#"{"command":["pr","review","2704","--approve","--body","VERDICT: pass"]}"#,
            true,
        )];
        assert!(review_submission_satisfied(&posted));
    }

    #[test]
    fn mika2727_un_run_gh_en_lecture_ne_satisfait_rien() {
        // La forme exacte des deux incidents : `qa_pr_view` puis `pr list`.
        let read_only = [
            summary("qa_pr_view", r#"{"pr":2704}"#, true),
            summary(
                "run_gh",
                r#"{"command":["pr","list","--head","fix/2727/x","--json","number"]}"#,
                true,
            ),
        ];
        assert!(!review_submission_satisfied(&read_only));
    }

    #[test]
    fn mika2727_une_revue_qui_a_echoue_ne_satisfait_rien() {
        let failed = [summary(
            "run_gh",
            r#"{"command":["pr","review","2704","--approve"]}"#,
            false,
        )];
        assert!(!review_submission_satisfied(&failed));
    }

    #[test]
    fn mika2727_reserve_d3_un_build_lance_termine_legitimement_le_tour() {
        let dispatched = [summary(BUILD_MIKA_TOOL, r#"{"pr":2704}"#, true)];
        assert!(build_dispatched_in_turn(&dispatched));
        assert!(review_submission_satisfied(&dispatched));
        // Et un `build_mika` qui a échoué n'ouvre pas la réserve : aucun callback
        // ne suivra, donc mika#2355 ne prendra pas le relais.
        let failed = [summary(BUILD_MIKA_TOOL, r#"{"pr":2704}"#, false)];
        assert!(!review_submission_satisfied(&failed));
    }

    #[test]
    fn mika2727_les_deux_gardes_ne_sont_jamais_dues_sur_le_meme_message() {
        let webhook = pr_event("synchronize");
        let callback = format!(
            "{} Build succeeded.",
            qa_build_callback::BUILD_CALLBACK_MESSAGE_MARKER
        );

        assert!(review_submission_required(&webhook, QA_SKILLS));
        assert!(!qa_build_callback::qa_verdict_required(&webhook, QA_SKILLS));

        assert!(qa_build_callback::qa_verdict_required(&callback, QA_SKILLS));
        assert!(!review_submission_required(&callback, QA_SKILLS));

        assert_ne!(
            REVIEW_SUBMISSION_LABEL,
            qa_build_callback::QA_VERDICT_REQUIRED_LABEL,
            "deux labels distincts : deux budgets de re-prompt, jamais un partagé"
        );
    }

    #[test]
    fn mika2727_le_complementaire_sarme_quand_le_re_prompt_a_echoue_aussi() {
        let nothing: [ToolCallSummary; 0] = [];
        let mut retries: HashSet<&'static str> = HashSet::new();

        // Budget intact : c'est la garde qui parle, pas le complémentaire.
        assert!(!submission_unmet_after_retry(true, &retries, &nothing));

        retries.insert(REVIEW_SUBMISSION_LABEL);
        assert!(submission_unmet_after_retry(true, &retries, &nothing));

        // Hors population, ou revue finalement postée : rien.
        assert!(!submission_unmet_after_retry(false, &retries, &nothing));
        let posted = [summary(
            "run_gh",
            r#"{"command":["pr","review","2704","--approve"]}"#,
            true,
        )];
        assert!(!submission_unmet_after_retry(true, &retries, &posted));
    }

    #[test]
    fn mika2727_la_consigne_nomme_loutil_la_ligne_et_la_reserve() {
        for needle in [
            "`run_gh`",
            "`pr review`",
            "`VERDICT:`",
            "`pr list`",
            "update_task_status",
            "send_message",
            "`build_mika`",
            "source of truth",
        ] {
            assert!(
                REVIEW_SUBMISSION_CORRECTION.contains(needle),
                "la consigne doit nommer {needle}"
            );
        }
        for value in [
            "pass",
            "hold[review]",
            "block[ac]",
            "block[ci]",
            "block[security]",
            "block[pipeline]",
        ] {
            assert!(
                REVIEW_SUBMISSION_CORRECTION.contains(value),
                "la consigne doit nommer la valeur de verdict {value}"
            );
        }
    }
}
