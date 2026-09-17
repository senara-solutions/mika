//! Le discriminant du callback de build QA, écrit **une seule fois** (mika#2355).
//!
//! # Le défaut que ce module ferme
//!
//! Le 2026-09-17, trois revues mika-qa (`7309c48c`/#2352, `52780caa`/#2353,
//! `1ea9f92c`/#2350) ont rendu « Build succeeded » et **rien d'autre** : zéro
//! `VERDICT:`, zéro commentaire sur la PR. Le tour de revue lance `build_mika`
//! en asynchrone et termine son tour pour attendre le callback — ça, c'est
//! voulu. Ce qui ne l'était pas : sur le tour de reprise, le moteur prescrit
//! **le contrat terminal d'un autre flux**.
//!
//! `build_callback_trigger_context` injectait « This turn MUST end with both
//! `update_task_status` and `send_message` », et la garde `callback_terminal_action`
//! (#870) ne relâchait le tour que sur ces deux outils — un contrat écrit pour
//! le dispatch d'un pilote self_dev, à une époque où son commentaire pouvait
//! encore affirmer *« only one callback flow exists today »*. `build_mika`
//! (`long_running: true`) en est le second, et rien dans le source ne l'a dit.
//! Un tour qui répondait « Build succeeded » par `send_message` satisfaisait donc
//! le moteur, et **aucune garde n'exigeait `run_gh pr review`**.
//!
//! # Pourquoi un module, et pas un littéral à chaque site
//!
//! Quatre lecteurs posent la même question : le framing (`agent_loop`), la garde
//! négative (`callback_trigger_active`), la garde positive (`qa_verdict_required`)
//! et le filet (`task_engine::dispatcher`). Une grammaire de fil recopiée entre
//! quatre lecteurs est exactement la classe que mika#2158 a dû refermer une fois
//! — deux regex de grooming qui répondaient différemment à la même question
//! pendant des mois sans que rien ne casse. Le cinquième lecteur est du
//! **prompt** (`qa-review-build-callback/system_prompt.md`) et ne peut pas
//! partager une constante Rust : `tests::mika2355_the_scope_header_quotes_the_engine_marker`
//! épingle que la chaîne qu'il cite est bien celle que le moteur émet.

use crate::agent_loop::ToolCallSummary;

/// Nom de l'outil long-running exposé par le skill `build-mika`.
///
/// Source de vérité : `skills/bundled/build-mika/tools.json`.
pub const BUILD_MIKA_TOOL: &str = "build_mika";

/// Le skill dont la présence au tour rend un verdict **dû**.
pub const QA_REVIEW_SKILL: &str = "qa-review";

/// Le marqueur de tour qu'émet `run_silent_agent` pour un callback de build.
///
/// Forme composée de deux moitiés qui vivent ailleurs et qu'on ne peut pas
/// importer : `format!("[callback: {label}]")` dans `run_silent_agent`, et
/// `format!("long_running:{tool_name}")` dans
/// `skills::executor::build_callback_task`. Le test
/// [`tests::mika2355_the_marker_is_the_shape_the_engine_actually_emits`]
/// reconstruit les deux `format!` et compare — un changement de l'une ou
/// l'autre grammaire rougit ici plutôt que de désarmer les gardes en silence.
pub const BUILD_CALLBACK_MESSAGE_MARKER: &str = "[callback: long_running:build_mika]";

/// Le message de ce tour est-il un callback de build ?
///
/// `starts_with` et non `contains` : `run_silent_agent` peut suffixer le
/// marqueur `[milestone-parent: …]`, jamais préfixer quoi que ce soit.
pub fn is_build_callback(msg: &str) -> bool {
    msg.starts_with(BUILD_CALLBACK_MESSAGE_MARKER)
}

/// Un verdict est-il **dû** sur ce tour ?
///
/// Conjonction, et les deux moitiés comptent. Le label porte le nom de l'outil,
/// jamais celui de l'agent : `mika-dev` porte `build-mika` dans son allowlist
/// (`well_known_agents.rs`) et lance des builds qui ne doivent aucun verdict à
/// personne. Une garde armée sur le seul label re-prompterait mika-dev pour
/// poster une revue de PR — elle échangerait le loop-breaker QA contre un
/// loop-breaker dev.
pub fn qa_verdict_required(msg: &str, skill_names: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    is_build_callback(msg)
        && skill_names
            .into_iter()
            .any(|n| n.as_ref().eq_ignore_ascii_case(QA_REVIEW_SKILL))
}

/// Un `run_gh pr review` a-t-il **réussi** dans ce tour ?
///
/// Le prédicat de satisfaction de la garde positive, et le même que consulte le
/// filet B3 avant de poster quoi que ce soit. Miroir exact de
/// `agent_loop::has_successful_pr_review` (early-accept #695/#821), délibérément
/// réécrit ici plutôt qu'appelé : cette fonction-là est privée à `agent_loop` et
/// la rendre publique pour le dispatcher exporterait un détail de la chaîne de
/// post-conditions. `tests::mika2355_the_satisfied_predicate_matches_the_early_accept_one`
/// épingle que les deux répondent pareil sur les formes qui comptent.
pub fn pr_review_posted_in_turn(summaries: &[ToolCallSummary]) -> bool {
    summaries.iter().any(|s| {
        s.name == "run_gh"
            && s.success
            && s.input_summary.contains("\"pr\"")
            && s.input_summary.contains("\"review\"")
    })
}

/// Label de la garde positive, pour `intent_guard_retries`.
pub const QA_VERDICT_REQUIRED_LABEL: &str = "qa_build_callback_verdict";

/// Le re-prompt de la garde positive.
///
/// Nomme l'outil **et** la ligne attendue : une correction qui dit seulement
/// « vous n'avez pas fini » laisse le modèle deviner par quoi finir, et il
/// devine le contrat qu'on vient précisément de lui retirer.
pub const QA_VERDICT_REQUIRED_CORRECTION: &str =
    "[mika-engine] This is a build callback for a QA review, and the review is not \
     complete: no successful `run_gh` call with `pr review` appears in this turn's \
     tool history. A qa-review turn concludes by POSTING the review to GitHub — \
     the posted review is the source of truth, and verdict text in your response \
     is only a mirror. Follow the qa-review-build-callback workflow (re-read the \
     plan, execute the ACs, compose PLAN-AC VERIFICATION and DIFF ANALYSIS) and \
     call `run_gh` with `pr review` carrying a trailing `VERDICT:` line \
     (`pass` / `hold[review]` / `block[ac]` / `block[ci]` / `block[security]` / \
     `block[pipeline]`). Do not call `update_task_status` or `send_message` \
     instead — they do not deliver a verdict to the PR.";

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Le marqueur est la forme que le moteur émet réellement — reconstruite
    /// par les deux `format!` de production, jamais recopiée à la main.
    #[test]
    fn mika2355_the_marker_is_the_shape_the_engine_actually_emits() {
        let label = format!("long_running:{BUILD_MIKA_TOOL}");
        let emitted = format!("[callback: {label}]");
        assert_eq!(
            emitted, BUILD_CALLBACK_MESSAGE_MARKER,
            "le marqueur a divergé d'une des deux grammaires qui le composent"
        );
    }

    /// Le suffixe milestone ne doit pas désarmer le discriminant.
    #[test]
    fn mika2355_a_milestone_suffix_does_not_hide_the_marker() {
        assert!(is_build_callback(
            "[callback: long_running:build_mika] [milestone-parent: abc]"
        ));
    }

    /// Les cinq autres outils `long_running` ne sont pas des callbacks de build.
    #[test]
    fn mika2355_the_other_long_running_tools_are_not_build_callbacks() {
        for tool in [
            "run_claude_pilot",
            "run_claude_pilot_groom",
            "deploy_mika",
            "address_pr_comments",
            "resolve_pr_conflicts",
        ] {
            assert!(
                !is_build_callback(&format!("[callback: long_running:{tool}]")),
                "{tool} ne doit pas être lu comme un callback de build"
            );
        }
    }

    /// AC4b — la conjonction, et le contrôle négatif qui est la moitié qui compte.
    #[test]
    fn mika2355_a_verdict_is_due_only_where_qa_review_is_loaded() {
        let msg = BUILD_CALLBACK_MESSAGE_MARKER;
        assert!(qa_verdict_required(msg, ["build-mika", "qa-review"]));
        assert!(
            qa_verdict_required(msg, ["QA-Review"]),
            "la comparaison de nom de skill est insensible à la casse"
        );
        // Le cas mika-dev : build_mika est dans son allowlist, il ne doit rien.
        assert!(!qa_verdict_required(msg, ["build-mika", "self-dev"]));
        assert!(!qa_verdict_required(msg, Vec::<String>::new()));
        // Et un autre flux de callback avec qa-review chargé ne doit rien non plus.
        assert!(!qa_verdict_required(
            "[callback: long_running:run_claude_pilot]",
            ["qa-review"]
        ));
    }

    /// Le prédicat de satisfaction répond comme l'early-accept #695/#821.
    #[test]
    fn mika2355_the_satisfied_predicate_matches_the_early_accept_one() {
        let posted = vec![summary(
            "run_gh",
            r#"{"args":["pr","review","2355","--comment"]}"#,
            true,
        )];
        assert!(pr_review_posted_in_turn(&posted));

        // Un échec n'est pas une revue postée.
        let failed = vec![summary(
            "run_gh",
            r#"{"args":["pr","review","2355"]}"#,
            false,
        )];
        assert!(!pr_review_posted_in_turn(&failed));

        // Un autre sous-commande `gh` non plus.
        let view = vec![summary("run_gh", r#"{"args":["pr","view","2355"]}"#, true)];
        assert!(!pr_review_posted_in_turn(&view));

        // Ni le contrat self_dev qu'on vient de retirer à ce flux.
        let self_dev = vec![
            summary("update_task_status", "{}", true),
            summary("send_message", "{}", true),
        ];
        assert!(!pr_review_posted_in_turn(&self_dev));
    }

    /// AC1b — l'en-tête de portée du prompt cite **la** chaîne que le moteur
    /// émet. Le prompt ne peut pas importer la constante ; ce test est le seul
    /// lien entre les deux, et sans lui l'en-tête peut citer un marqueur périmé
    /// tout en restant parfaitement lisible.
    #[test]
    fn mika2355_the_scope_header_quotes_the_engine_marker() {
        let prompt = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../skills/bundled/qa-review-build-callback/system_prompt.md"
        ));
        assert!(
            prompt.contains(BUILD_CALLBACK_MESSAGE_MARKER),
            "l'en-tête de portée ne cite pas le marqueur du moteur — hors callback \
             de build, ce fichier autorise textuellement à sauter la revue de diff"
        );
        // Et il doit le citer AVANT la première instruction de reprise, sinon la
        // condition de portée arrive après ce qu'elle conditionne.
        let marker_at = prompt.find(BUILD_CALLBACK_MESSAGE_MARKER).unwrap();
        let resume_at = prompt
            .find("Steps 1–3d were completed")
            .expect("la phrase de reprise doit exister");
        assert!(
            marker_at < resume_at,
            "la condition de portée doit précéder l'instruction qu'elle conditionne"
        );
    }
}
