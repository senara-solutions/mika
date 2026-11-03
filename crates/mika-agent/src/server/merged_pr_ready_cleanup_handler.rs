//! Le merge d'une PR de la boucle rend le consentement du ticket qu'elle sert
//! (mika#3022).
//!
//! Mesuré du 2026-10-30 au 2026-11-03 : cinq retraits de `ready` à la main, sous
//! le compte `samidarko`, sur des tickets gardés ouverts après le merge d'une de
//! leurs PR (#3002/#3008, #2194/#3014, #2194/#3017, #2996/#3018, #2995/#3020).
//! Entre le merge et le retrait, le ticket reste exposé au feeder — la chaîne
//! mesurée sur #3002 est une tâche `ready-label` à 14:00:04Z suivie d'un tour de
//! mika-dev qui ferme le ticket à 14:46:22Z, pour un consentement que le merge
//! avait déjà consommé.
//!
//! ## Pourquoi un module et pas une branche de `draft_pr_opened_handler`
//!
//! Le voisin de mika#1849 retire `ready` sur `PR opened` **draft**, et choisit
//! ses tickets par `closingIssuesReferences`. Les deux prémisses tombent ici :
//! son absence d'effet hors draft est **épinglée** (`plan_cleanup_non_draft_is_noop`),
//! et sur les cinq PR mesurées `closingIssuesReferences` est vide — la branche
//! est le seul porteur de `<N>`. Deux prédicats de population dans un seul
//! handler, c'est la forme de mika#2158 : ils divergent sans que rien ne rougisse.
//!
//! ## Invariants
//!
//! - **Injecteur à effet de bord seul** : rend `VerdictAction::Passthrough
//!   { enrichment: None }` sur **tous** les chemins, jamais `Handled` ni
//!   `Dispatched` (patron de `milestone_context_handler` et de
//!   `draft_pr_opened_handler`). Le câblage le tient par trois `unreachable!()`.
//! - **Aucun parseur nouveau.** Cinq lecteurs canoniques sont *appelés* :
//!   [`PR_CLOSED_PREFIX`], [`MERGED_TRUE_LINE`], [`extract_head_branch`]
//!   (`upstream_close_handler`), [`parse_pr_action_event`] (`webhook_queue_v2`,
//!   seul lecteur de `PR_ACTION_RE`) et [`issue_number_from_branch`]
//!   (`worktree_reaper`). Doctrine du lecteur canonique unique,
//!   `canonical_tokens::mika2649_aucun_second_parseur_de_grammaire_devenement`.
//! - **Retirer, c'est rendre un consentement, jamais s'en donner un** (voie B,
//!   mika#2995). Ce module ne porte aucune route d'ajout : le seul drapeau `gh`
//!   qu'il atteint est `--remove-label`, par `remove_ready_label`, et il n'est
//!   dans aucune des deux listes argv de [`crate::ready_label`].
//! - **Lire avant de retirer.** Le retrait n'a lieu que si l'étiquette est lue
//!   présente : un instrument qui dit « retiré » sur un ticket qui ne portait
//!   rien est un instrument qui mente (mika#2331) — absent n'est pas zéro.
//! - **Fail-open sur toute erreur `gh`** (token absent, `issue view`,
//!   `issue edit`) : une ligne WARN nommée, et le tour continue. Un retrait
//!   manqué coûte un geste à l'opérateur ; un panic coûterait le tour.
//!
//! Voir mika#3022.

use tracing::{debug, info, warn};

use super::draft_pr_opened_handler::{
    fetch_issue_labels, issue_has_ready_label, remove_ready_label,
};
use super::upstream_close_handler::{MERGED_TRUE_LINE, PR_CLOSED_PREFIX, extract_head_branch};
use super::verdict_handler::VerdictAction;
use super::webhook_queue_v2::parse_pr_action_event;
use crate::async_db::AsyncDatabase;
use crate::worktree_reaper::issue_number_from_branch;

/// L'action que ce handler sert, telle que [`parse_pr_action_event`] la rend.
///
/// Redondante avec [`PR_CLOSED_PREFIX`] **par construction** et non par
/// négligence : le préfixe décide que c'est notre événement, le lecteur de
/// grammaire décide quel dépôt et quelle PR il nomme. Le jour où les deux ne
/// s'accordent plus sur l'action, c'est que l'un des deux gabarits a bougé, et
/// `RepoUnrecognized` est la bonne réponse — pas un geste sur une population
/// devinée.
const CLOSED_ACTION: &str = "closed";

/// Pourquoi aucun geste n'a lieu. **Une raison, jamais un booléen** : AC3 exige
/// qu'une ligne de log dise *lequel* des cas s'est présenté, et un `bool` rend
/// les quatre indistinguables au site d'émission.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SkipReason {
    /// Le texte n'est pas un `PR closed` du gateway. **Muet** : ce handler voit
    /// tout le trafic webhook, et une ligne par message étranger noierait les
    /// six autres.
    NotOurEvent,
    /// `PR closed` sans `Merged: true` — une PR fermée sans merge ne rend aucun
    /// consentement. C'est le contrôle négatif d'AC1.
    NotMerged,
    /// L'en-tête ne se lit pas comme `<repo>#<pr>`, ou l'action lue n'est pas
    /// `closed`.
    RepoUnrecognized,
    /// La branche est absente de l'en-tête, ou hors de la forme
    /// `<type>/<N>/…` — donc le ticket serait **deviné** plutôt que porté.
    BranchOffForm,
}

/// La décision du seam : ne rien faire, et pourquoi — ou retirer, et sur quoi.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MergedPrCleanup {
    Skip(SkipReason),
    Clean { repo: String, issue: u64 },
}

/// Le ticket dont le merge rend le consentement, **s'il y en a un**.
///
/// Fonction **pure** : elle ne touche ni le réseau, ni la base, ni l'horloge.
/// C'est ce qui rend AC1 mesurable dans un seul appel, contrôle négatif compris,
/// sans forge ni token.
///
/// Les quatre portes, dans cet ordre, et chacune rend sa propre raison :
///
/// 1. le texte commence par [`PR_CLOSED_PREFIX`] — sinon [`SkipReason::NotOurEvent`] ;
/// 2. il porte [`MERGED_TRUE_LINE`] — sinon [`SkipReason::NotMerged`] ;
/// 3. [`parse_pr_action_event`] lit un dépôt et l'action [`CLOSED_ACTION`] —
///    sinon [`SkipReason::RepoUnrecognized`] ;
/// 4. [`extract_head_branch`] puis [`issue_number_from_branch`] rendent un
///    numéro — sinon [`SkipReason::BranchOffForm`].
///
/// L'ordre est porteur : la porte 2 est franchie avant qu'on cherche un dépôt,
/// pour qu'un `Merged: false` sur un en-tête bancal soit compté comme ce qu'il
/// est — une PR non mergée — et non comme un gabarit cassé.
pub(crate) fn plan_ready_cleanup_on_merge(text: &str) -> MergedPrCleanup {
    if !text.starts_with(PR_CLOSED_PREFIX) {
        return MergedPrCleanup::Skip(SkipReason::NotOurEvent);
    }
    if !text.contains(MERGED_TRUE_LINE) {
        return MergedPrCleanup::Skip(SkipReason::NotMerged);
    }
    let Some((action, repo, _pr)) = parse_pr_action_event(text) else {
        return MergedPrCleanup::Skip(SkipReason::RepoUnrecognized);
    };
    if action != CLOSED_ACTION {
        return MergedPrCleanup::Skip(SkipReason::RepoUnrecognized);
    }
    let Some(branch) = extract_head_branch(text) else {
        return MergedPrCleanup::Skip(SkipReason::BranchOffForm);
    };
    let Some(issue) = issue_number_from_branch(&branch) else {
        return MergedPrCleanup::Skip(SkipReason::BranchOffForm);
    };
    MergedPrCleanup::Clean {
        repo: repo.to_string(),
        issue,
    }
}

/// Le geste : retirer `ready` du ticket que le merge vient de servir.
///
/// Rend `VerdictAction::Passthrough { enrichment: None }` sur **tous** les
/// chemins — injecteur à effet de bord seul, comme `try_handle_draft_pr_opened`
/// et `try_handle_pr_closed_milestone_context`. Le câblage dans `handlers.rs`
/// tient cette propriété par trois `unreachable!()`.
///
/// Appelé **avant** `webhook_prefilter::skip_turn`, donc avant le tour LLM
/// (AC1) ; l'ordre est mesuré par le détecteur de source
/// `tests/eval/test_ready_cleanup_on_merge_3022.rs`, pas supposé.
///
/// Chaque sortie sans geste est **nommée** (AC3), et aucune ne coûte le tour :
///
/// | sortie | ligne |
/// |---|---|
/// | pas notre événement | aucune, délibérément muet |
/// | PR fermée sans merge | `debug! merged_pr_ready_cleanup_not_merged` |
/// | dépôt non reconnu | `warn! merged_pr_ready_cleanup_repo_unrecognized` |
/// | branche hors forme | `warn! merged_pr_ready_cleanup_branch_off_form` |
/// | token absent | `warn! merged_pr_ready_cleanup_no_token` |
/// | `gh` en erreur | `warn! merged_pr_ready_cleanup_gh_failed`, champ `step` |
/// | étiquette déjà absente | `debug! merged_pr_ready_cleanup_label_absent` |
/// | retrait fait | `info! ready_label_cleaned_up_on_merge` + audit |
///
/// Les lignes portent `repo` et `issue`, jamais le numéro de PR : le seam rend
/// le **ticket**, et la PR se rejoint par le `trace_id` que l'audit transporte.
/// Y remettre la PR demanderait de rappeler [`parse_pr_action_event`] ici, donc
/// de re-décider hors du seam ce que le seam a déjà décidé.
pub(crate) async fn try_handle_merged_pr_ready_cleanup(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    // 1. Le seam décide, et dit pourquoi quand il ne rend aucun ticket.
    let (repo, issue) = match plan_ready_cleanup_on_merge(text) {
        MergedPrCleanup::Clean { repo, issue } => (repo, issue),
        // Muet par décision : ce handler voit tout le trafic webhook, et une
        // ligne par message étranger noierait les sept autres.
        MergedPrCleanup::Skip(SkipReason::NotOurEvent) => {
            return VerdictAction::Passthrough { enrichment: None };
        }
        MergedPrCleanup::Skip(SkipReason::NotMerged) => {
            debug!(
                event = "merged_pr_ready_cleanup_not_merged",
                "merged_pr_ready_cleanup_handler: PR closed sans Merged: true — aucun \
                 consentement n'a été consommé, le label reste"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        MergedPrCleanup::Skip(SkipReason::RepoUnrecognized) => {
            warn!(
                event = "merged_pr_ready_cleanup_repo_unrecognized",
                "merged_pr_ready_cleanup_handler: en-tête illisible comme <repo>#<pr> closed \
                 — passthrough sans geste"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
        MergedPrCleanup::Skip(SkipReason::BranchOffForm) => {
            warn!(
                event = "merged_pr_ready_cleanup_branch_off_form",
                "merged_pr_ready_cleanup_handler: branche absente ou hors de la forme \
                 <type>/<N>/… — le ticket serait deviné, passthrough sans geste"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };

    // 2. Un token est nécessaire — son absence est un fail-open nommé.
    let Some(token) = github_token else {
        warn!(
            event = "merged_pr_ready_cleanup_no_token",
            repo = %repo,
            issue = issue,
            "merged_pr_ready_cleanup_handler: aucun token GitHub — retrait impossible"
        );
        return VerdictAction::Passthrough { enrichment: None };
    };

    // 3. Lire avant de retirer (mika#2331) : un instrument qui dit « retiré »
    //    sur un ticket qui ne portait rien est un instrument qui mente, et
    //    l'absence de l'étiquette n'est pas un retrait de plus.
    let labels = match fetch_issue_labels(issue, &repo, token).await {
        Ok(l) => l,
        Err(e) => {
            warn!(
                event = "merged_pr_ready_cleanup_gh_failed",
                step = "issue_view",
                repo = %repo,
                issue = issue,
                error = %e,
                "merged_pr_ready_cleanup_handler: gh issue view a échoué — passthrough"
            );
            return VerdictAction::Passthrough { enrichment: None };
        }
    };
    if !issue_has_ready_label(&labels) {
        debug!(
            event = "merged_pr_ready_cleanup_label_absent",
            repo = %repo,
            issue = issue,
            "merged_pr_ready_cleanup_handler: le ticket ne portait pas ready — rien à rendre"
        );
        return VerdictAction::Passthrough { enrichment: None };
    }

    // 4. Le retrait, et lui seul : `--remove-label`, jamais `--add-label`
    //    (voie B, mika#2995).
    match remove_ready_label(issue, &repo, token).await {
        Ok(()) => {
            info!(
                event = "ready_label_cleaned_up_on_merge",
                repo = %repo,
                issue = issue,
                "merged_pr_ready_cleanup_handler: ready retiré du ticket servi par le merge"
            );

            // Trace opérateur, tirée-et-oubliée : l'échec de l'audit ne défait
            // pas un retrait déjà posé sur GitHub.
            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    "merged_pr_ready_label_cleaned",
                    &format!("issue:{repo}#{issue}"),
                    None,
                    Some("ready_removed"),
                    Some(&format!("repo={repo} issue={issue}")),
                    Some(trace_id),
                )
                .await
            {
                warn!(
                    event = "merged_pr_ready_cleanup_audit_failed",
                    repo = %repo,
                    issue = issue,
                    error = %e,
                    "merged_pr_ready_cleanup_handler: écriture de l'audit échouée (non fatal)"
                );
            }
        }
        Err(e) => {
            warn!(
                event = "merged_pr_ready_cleanup_gh_failed",
                step = "remove_label",
                repo = %repo,
                issue = issue,
                error = %e,
                "merged_pr_ready_cleanup_handler: gh issue edit --remove-label a échoué — \
                 passthrough"
            );
        }
    }

    VerdictAction::Passthrough { enrichment: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gabarit réel du gateway pour `pull_request.closed`, **recopié verbatim**
    /// de `webhook_prefilter.rs:1989` (`mod tests` privé, donc non importable —
    /// élargir sa visibilité ajouterait de la surface que ce vol n'autorise pas).
    /// Deuxième ligne : l'URL de la PR ; troisième : `Merged: <bool>`.
    const PR_CLOSED: &str = "[GitHub] PR closed: senara-solutions/mika#2679 — fix(webhooks): x (branch: fix/2679/x)\nhttps://github.com/senara-solutions/mika/pull/2679\nMerged: true";

    /// Même gabarit, `html_url` absent : le gateway interpole `""`, donc la
    /// deuxième ligne est vide. Recopié de `webhook_prefilter.rs:1993`. Sa
    /// branche est `x` — hors forme, et c'est à ce titre qu'elle sert ici.
    const PR_CLOSED_SANS_URL: &str =
        "[GitHub] PR closed: senara-solutions/mika#2679 — t (branch: x)\n\nMerged: true";

    /// La fixture de production, son booléen de merge retourné. Le **seul**
    /// octet qui change entre le cas et son contrôle négatif.
    fn non_merge(text: &str) -> String {
        text.replace("Merged: true", "Merged: false")
    }

    /// AC1, les deux moitiés **dans le même test** — c'est ce que le critère
    /// demande, et ce n'est pas une économie de lignes : deux tests séparés
    /// restent verts le jour où le seam rend `Clean` sur tout, et le contrôle
    /// négatif est la seule assertion qui le voit.
    #[test]
    fn mika3022_un_merge_en_forme_rend_le_ticket_et_le_meme_evenement_non_merge_ne_rend_rien() {
        assert_eq!(
            plan_ready_cleanup_on_merge(PR_CLOSED),
            MergedPrCleanup::Clean {
                repo: "senara-solutions/mika".to_string(),
                issue: 2679,
            },
            "un `PR closed` mergé dont la branche porte `<type>/<N>/…` doit rendre #N — \
             c'est le geste que cinq mains humaines ont posé entre le 10-30 et le 11-03"
        );
        assert_eq!(
            plan_ready_cleanup_on_merge(&non_merge(PR_CLOSED)),
            MergedPrCleanup::Skip(SkipReason::NotMerged),
            "le MÊME événement en `Merged: false` ne doit rendre aucun consentement : une PR \
             fermée sans merge laisse le ticket dispatchable, et le retirer serait une \
             décision que personne n'a prise"
        );
    }

    /// Une branche hors forme ne laisse jamais deviner un numéro (B3.2 de
    /// `issue_number_from_branch`) : un ticket fabriqué enverrait le retrait sur
    /// le consentement de quelqu'un d'autre.
    #[test]
    fn mika3022_une_branche_hors_forme_ne_rend_aucun_ticket() {
        let cas = [
            ("main", "une branche par défaut ne sert aucun ticket"),
            ("feat/slug", "pas de deuxième segment numérique"),
            (
                "/2482/x",
                "premier segment vide : pas une branche de dispatch",
            ),
            ("fix/x/2482", "le numéro n'est pas en deuxième position"),
        ];
        for (branche, pourquoi) in cas {
            let text = format!(
                "[GitHub] PR closed: senara-solutions/mika#2679 — t (branch: {branche})\nhttps://github.com/senara-solutions/mika/pull/2679\nMerged: true"
            );
            assert_eq!(
                plan_ready_cleanup_on_merge(&text),
                MergedPrCleanup::Skip(SkipReason::BranchOffForm),
                "`{branche}` doit passer sans geste ({pourquoi})"
            );
        }
    }

    /// L'en-tête sans `(branch: …)` du tout, et le gabarit sans URL dont la
    /// branche est `x` : deux formes de production, une seule raison.
    #[test]
    fn mika3022_un_entete_sans_branche_lisible_passe_sans_geste() {
        let sans_forme = "[GitHub] PR closed: senara-solutions/mika#2679 — t\nhttps://github.com/senara-solutions/mika/pull/2679\nMerged: true";
        assert_eq!(
            plan_ready_cleanup_on_merge(sans_forme),
            MergedPrCleanup::Skip(SkipReason::BranchOffForm),
            "sans `(branch: …)` dans l'en-tête, la branche est absente et non vide"
        );
        assert_eq!(
            plan_ready_cleanup_on_merge(PR_CLOSED_SANS_URL),
            MergedPrCleanup::Skip(SkipReason::BranchOffForm),
            "le gabarit sans `html_url` porte la branche `x` : lisible, mais hors forme"
        );
    }

    /// Un en-tête dont le jeton `<repo>#<pr>` ne se lit pas est un dépôt non
    /// reconnu, pas une branche hors forme — AC3 demande que la ligne dise
    /// lequel, donc les deux raisons doivent être séparables ici.
    #[test]
    fn mika3022_un_entete_illisible_est_un_depot_non_reconnu() {
        let text =
            "[GitHub] PR closed: pas-un-jeton-de-depot — t (branch: fix/3022/x)\nMerged: true";
        assert_eq!(
            plan_ready_cleanup_on_merge(text),
            MergedPrCleanup::Skip(SkipReason::RepoUnrecognized),
            "l'en-tête ne porte pas `<repo>#<pr>` : le dépôt serait deviné"
        );
    }

    /// Un titre qui porte la forme `(branch: …)` ne déplace pas la vraie
    /// branche : [`extract_head_branch`] lit la **dernière** occurrence de la
    /// première ligne, et mika#2242 l'épingle à son site. Ce test dit que le
    /// seam en hérite au lieu de se recoudre une sémantique.
    #[test]
    fn mika3022_un_titre_portant_la_forme_ne_deplace_pas_la_branche() {
        let text = "[GitHub] PR closed: senara-solutions/mika#2679 — fix: parle de (branch: main) (branch: fix/3022/x)\nhttps://github.com/senara-solutions/mika/pull/2679\nMerged: true";
        assert_eq!(
            plan_ready_cleanup_on_merge(text),
            MergedPrCleanup::Clean {
                repo: "senara-solutions/mika".to_string(),
                issue: 3022,
            },
            "la branche est la dernière de la première ligne, pas celle que le titre cite"
        );
    }

    /// Le trafic étranger passe, et **sans raison nommée** : c'est la seule des
    /// quatre portes dont la fréquence est celle de tout le webhook.
    #[test]
    fn mika3022_le_trafic_etranger_nest_pas_notre_evenement() {
        for text in [
            "[GitHub] Check suite success on senara-solutions/mika (branch: main)",
            "[GitHub] PR opened: senara-solutions/mika#2679 — t (branch: fix/2679/x)",
            "",
        ] {
            assert_eq!(
                plan_ready_cleanup_on_merge(text),
                MergedPrCleanup::Skip(SkipReason::NotOurEvent),
                "`{text}` n'est pas un `PR closed` du gateway"
            );
        }
    }
}
