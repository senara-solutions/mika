//! Le seul geste d'écriture du manager de milestone (mika#2698, Phase 2 (b),
//! **vol 2**) : poster le verdict d'un vol en commentaire sur son ticket.
//!
//! # Pourquoi ce fichier existe, et pourquoi il est seul
//!
//! Tout le reste du module est LECTURE SEULE, et `no_dispatch_test.rs` en fait
//! une propriété structurelle : un grep de jetons sur chaque `.rs` du
//! sous-arbre. `gh issue comment` est l'un de ces jetons interdits. Ce fichier
//! est l'**exception nommée** — la première et, à ce jour, la seule entrée de
//! `EXEMPTED_OCCURRENCES`, qui nomme ce fichier, ce jeton, ce ticket, et rien
//! d'autre. `gh issue edit`, `gh pr comment`, `gh pr merge` restent interdits
//! **ici aussi** : l'exception porte sur une paire (fichier, jeton), jamais sur
//! un répertoire.
//!
//! **Le geste est donc écrit en clair, à dessein.** [`WRITE_GESTURE`] porte la
//! chaîne `"gh issue comment"` et les sous-commandes en sont dérivées par
//! découpage. Écrire `&["issue", "comment"]` passerait le grep sans exception :
//! la garde ne verrait rien, l'allowlist resterait vide, et le fichier aurait
//! obtenu son autorisation d'écrire en se rendant invisible. C'est exactement
//! la forme que le plan de ce ticket refuse. Ici, la garde voit, l'allowlist
//! nomme, et un second geste coûterait une seconde entrée — visible en revue.
//!
//! # Mode ombre par défaut
//!
//! [`ENV_EVALUATE_WRITE`] est OFF par défaut et son absence est un refus, pas
//! un défaut à deviner : [`comments_to_post`] rend `vec![]` et aucun chemin
//! n'atteint [`post_verdict_comments`]. Le verdict continue d'aller dans le
//! rapport, qui est sa destination permanente — l'interrupteur ajoute une
//! destination, il n'en remplace aucune.
//!
//! # Idempotence (AC2)
//!
//! Un vol évalué deux fois ne reçoit qu'un commentaire. La mémoire est
//! [`super::cadence::MilestoneCheckpoint::commented_flights`] : les `task_id`
//! déjà commentés, persistés avec le checkpoint par le même
//! `save_checkpoint`. Un `task_id` est la bonne clé — il est stable sur un
//! rejeu du cycle, là où le numéro de ticket ne l'est pas (deux vols sur le
//! même ticket sont deux verdicts).
//!
//! La fenêtre reste bornée des deux côtés : `since` (le checkpoint) borne les
//! vols que la source rend, et [`COMMENTED_HISTORY_CAP`] borne la liste
//! retenue, parce qu'un checkpoint qui croît sans fin est un incident à
//! retardement.
//!
//! **Ce que l'idempotence ne couvre pas, et c'est dit ici** : un commentaire
//! posté puis un `save_checkpoint` qui échoue re-poste au cycle suivant. Il n'y
//! a pas de transaction entre GitHub et un fichier JSON local, et l'ordre
//! choisi est le moins mauvais — il préfère un doublon rare à un verdict perdu.

use super::evaluator::FlightVerdict;
use super::reader::GhRunner;
use super::reporter::render_flight_verdict;

/// L'interrupteur. OFF par défaut, et déclaré ici parce que c'est ce fichier
/// qui possède le canal qu'il ouvre — même règle que `ENV_OFFLINE_SINK_DIR`
/// dans `sink_dir.rs` et `ENV_LIVENESS_URL` dans `liveness.rs`.
pub const ENV_EVALUATE_WRITE: &str = "MIKA_MANAGER_EVALUATE_WRITE";

/// Le geste autorisé, **en clair** : voir le docstring du module.
const WRITE_GESTURE: &str = "gh issue comment";

/// Combien de vols commentés le checkpoint retient.
///
/// Généreusement au-dessus de ce qu'une fenêtre `since` peut rendre (un
/// milestone n'a pas 200 vols terminés entre deux cycles de 5 minutes), donc la
/// troncature n'arrive jamais en service nominal. Elle existe pour le cas où le
/// checkpoint survit des mois : le fichier plafonne au lieu de croître.
const COMMENTED_HISTORY_CAP: usize = 200;

/// L'interrupteur est-il ouvert ?
///
/// Lu à chaque composition de cycle plutôt qu'une fois au démarrage : le seuil
/// d'ouverture du ticket est un geste opérateur, et un interrupteur qui exige un
/// redémarrage pour se **refermer** est un mauvais interrupteur.
pub fn evaluate_write_enabled() -> bool {
    write_enabled_from(std::env::var(ENV_EVALUATE_WRITE).ok().as_deref())
}

/// Le prédicat, séparé de la lecture d'environnement pour être testable sans
/// `set_var` (qui est `unsafe` en edition 2024 et racé entre tests parallèles).
///
/// Tout ce qui n'est pas explicitement vrai est faux — y compris `"0"`,
/// `"off"`, la chaîne vide et une valeur mal orthographiée. Un interrupteur
/// d'écriture qui s'ouvrirait sur `MIKA_MANAGER_EVALUATE_WRITE=flase` serait
/// pire qu'absent.
pub(crate) fn write_enabled_from(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true"))
}

/// Les sous-commandes du geste, sans le programme — [`GhRunner`] lance `gh`
/// lui-même.
///
/// Dérivées de [`WRITE_GESTURE`] au lieu d'être réécrites : une seule chaîne
/// déclare le geste, donc le jeton que la garde voit et les arguments que le
/// processus reçoit ne peuvent pas divergir.
fn write_gesture_argv() -> Vec<&'static str> {
    let mut words = WRITE_GESTURE.split_whitespace();
    debug_assert_eq!(
        words.next(),
        Some("gh"),
        "le geste doit commencer par le programme que GhRunner lance"
    );
    words.collect()
}

/// Un commentaire décidé mais pas encore posté.
///
/// L'étape intermédiaire est ce qui rend l'AC1 testable sans processus : la
/// décision (interrupteur, idempotence, ticket connu) est une fonction pure, et
/// le geste n'est qu'une boucle sur ce qu'elle rend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingComment {
    /// `tasks.id` du vol — la clé d'idempotence.
    pub task_id: String,
    pub issue_number: u64,
    pub body: String,
}

/// Quels commentaires ce cycle doit poster.
///
/// Rend `vec![]` dès que `write_enabled` est faux : c'est l'AC1 sous sa forme
/// fonctionnelle, en complément de la garantie structurelle du grep. Un verdict
/// sans numéro de ticket est également écarté — le manager ne devine pas sur
/// quel ticket écrire.
pub fn comments_to_post(
    verdicts: &[FlightVerdict],
    already_commented: &[String],
    write_enabled: bool,
) -> Vec<PendingComment> {
    if !write_enabled {
        return Vec::new();
    }
    let mut seen: Vec<&str> = already_commented.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for v in verdicts {
        let Some(issue_number) = v.issue_number else {
            continue;
        };
        // Le doublon intra-cycle compte autant que l'inter-cycle : une source
        // qui rendrait deux fois le même vol ne doit pas produire deux
        // commentaires.
        if seen.contains(&v.task_id.as_str()) {
            continue;
        }
        seen.push(v.task_id.as_str());
        out.push(PendingComment {
            task_id: v.task_id.clone(),
            issue_number,
            body: verdict_comment_body(v),
        });
    }
    out
}

/// Le corps du commentaire.
///
/// La ligne de verdict vient de [`render_flight_verdict`], la **même** fonction
/// que le rapport : le commentaire et le rapport ne peuvent donc pas dire deux
/// choses différentes du même vol. Le marqueur HTML en tête est une seconde
/// prise d'idempotence, lisible par un humain qui cherche « ce vol a-t-il déjà
/// été commenté » sans ouvrir le checkpoint.
pub fn verdict_comment_body(v: &FlightVerdict) -> String {
    let completed = v.completed_at.as_deref().unwrap_or("date non lue");
    format!(
        "<!-- mika-manager:verdict:{task} -->\n\
         **Verdict du manager de milestone** (mika#2698)\n\n\
         {line}\n\n\
         Vol `{task}`, terminé le {completed}. Lecture automatique de `tasks.result` \
         et de l'état du sous-ticket ; ce commentaire est le seul geste posé.\n",
        task = v.task_id,
        line = render_flight_verdict(v),
        completed = completed,
    )
}

/// Poste les commentaires et rend les `task_id` effectivement postés.
///
/// Les `task_id` rendus, et eux seuls, entrent dans le checkpoint : un échec de
/// `gh` laisse donc le vol non commenté, et le cycle suivant réessaiera. C'est
/// le bon sens de l'erreur — un verdict manquant se rattrape, un commentaire en
/// double ne se retire pas.
///
/// Best-effort, jamais fatal : la cadence est au service du rapport, et un
/// `gh` en panne ne doit pas tuer le cycle qui vient de l'écrire.
pub async fn post_verdict_comments<R: GhRunner + ?Sized>(
    runner: &R,
    repo: &str,
    pending: &[PendingComment],
) -> Vec<String> {
    let mut posted = Vec::new();
    for c in pending {
        let number = c.issue_number.to_string();
        let mut args = write_gesture_argv();
        args.extend_from_slice(&[&number, "--repo", repo, "--body", &c.body]);
        match runner.run(&args).await {
            Ok(_) => {
                tracing::info!(
                    target: "mika::milestone_manager",
                    event = "manager_verdict_commented",
                    repo = %repo,
                    issue = c.issue_number,
                    task_id = %c.task_id,
                );
                posted.push(c.task_id.clone());
            }
            Err(e) => {
                tracing::warn!(
                    target: "mika::milestone_manager",
                    event = "manager_verdict_comment_failed",
                    repo = %repo,
                    issue = c.issue_number,
                    task_id = %c.task_id,
                    error = %e,
                    "le verdict n'a pas été posté — il le sera au prochain cycle"
                );
            }
        }
    }
    posted
}

/// La liste des vols commentés pour le prochain checkpoint, bornée.
///
/// Les plus récents sont gardés : la troncature coupe la queue, c'est-à-dire
/// les vols les plus anciens — ceux que la fenêtre `since` ne rendra de toute
/// façon plus.
pub fn remember_commented(previous: &[String], newly_posted: &[String]) -> Vec<String> {
    let mut out: Vec<String> = newly_posted.to_vec();
    for id in previous {
        if out.len() >= COMMENTED_HISTORY_CAP {
            break;
        }
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    // `out` est un `Vec<String>` : la borne compte des éléments, jamais des
    // octets, donc aucune frontière de caractère n'est en jeu. La boucle
    // ci-dessus borne déjà le report du précédent ; cette ligne borne le cas où
    // `newly_posted` dépasse le plafond à lui seul.
    out.truncate(COMMENTED_HISTORY_CAP); // safe-byte-slice: Vec<String>, un compte d'éléments
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::evaluator::{FlightIssue, FlightRecommendation};
    use std::sync::Mutex;

    /// Un `GhRunner` qui n'exécute rien et retient ce qu'on lui a demandé.
    #[derive(Default)]
    struct RecordingRunner {
        calls: Mutex<Vec<Vec<String>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl GhRunner for RecordingRunner {
        async fn run(&self, args: &[&str]) -> anyhow::Result<String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            if self.fail {
                anyhow::bail!("gh refused");
            }
            Ok(String::new())
        }
    }

    impl RecordingRunner {
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn verdict(task_id: &str, issue: Option<u64>) -> FlightVerdict {
        FlightVerdict {
            task_id: task_id.to_string(),
            issue_number: issue,
            issue: FlightIssue::PrOpened {
                number: Some(2704),
                url: Some("https://github.com/senara-solutions/mika/pull/2704".to_string()),
            },
            turns_used: Some(83),
            hit_ceiling: false,
            turn_ceiling: Some(150),
            last_commit_turn: None,
            recommendation: FlightRecommendation::MergeCandidate,
            completed_at: Some("2026-10-15T08:00:00Z".to_string()),
        }
    }

    // ---- AC1, forme fonctionnelle : l'interrupteur OFF ne décide rien ------

    #[test]
    fn mika2698_linterrupteur_ferme_ne_produit_aucun_commentaire() {
        let out = comments_to_post(&[verdict("t1", Some(2693))], &[], false);
        assert!(
            out.is_empty(),
            "OFF par défaut : aucun geste à poser, donc rien à poster"
        );
    }

    #[tokio::test]
    async fn mika2698_rien_a_poster_ne_touche_pas_le_processus() {
        // Le contrôle négatif du test ci-dessus : la même liste vide passée au
        // geste ne lance rien. Sans cette assertion, « OFF ⇒ vec![] » serait
        // vert même si le geste s'exécutait par un autre chemin.
        let runner = RecordingRunner::default();
        let posted = post_verdict_comments(&runner, "senara-solutions/mika", &[]).await;
        assert!(posted.is_empty());
        assert!(runner.calls().is_empty(), "aucun appel gh avec OFF");
    }

    #[test]
    fn mika2698_seules_les_valeurs_vraies_ouvrent_linterrupteur() {
        for v in ["1", "true", "TRUE", "True"] {
            assert!(write_enabled_from(Some(v)), "{v} doit ouvrir");
        }
        for v in ["0", "false", "off", "", "yes", "flase"] {
            assert!(!write_enabled_from(Some(v)), "{v} ne doit pas ouvrir");
        }
        assert!(!write_enabled_from(None), "absente ⇒ fermée");
    }

    // ---- AC2 : un seul commentaire par vol, idempotent --------------------

    #[tokio::test]
    async fn mika2698_linterrupteur_ouvert_poste_exactement_un_commentaire_par_vol() {
        let runner = RecordingRunner::default();
        let pending = comments_to_post(
            &[verdict("t1", Some(2693)), verdict("t2", Some(2655))],
            &[],
            true,
        );
        assert_eq!(pending.len(), 2);
        let posted = post_verdict_comments(&runner, "senara-solutions/mika", &pending).await;
        assert_eq!(posted, vec!["t1".to_string(), "t2".to_string()]);

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "un appel par vol, pas deux, pas zéro");
        assert_eq!(
            &calls[0][..3],
            &[
                "issue".to_string(),
                "comment".to_string(),
                "2693".to_string()
            ],
            "le geste est `gh issue comment <ticket>` et rien d'autre"
        );
        assert!(
            calls[0].contains(&"--repo".to_string())
                && calls[0].contains(&"senara-solutions/mika".to_string()),
            "le dépôt est explicite : le démon ne tourne pas dans un checkout"
        );
    }

    #[test]
    fn mika2698_un_vol_deja_commente_ne_lest_pas_deux_fois() {
        let already = vec!["t1".to_string()];
        let out = comments_to_post(
            &[verdict("t1", Some(2693)), verdict("t2", Some(2655))],
            &already,
            true,
        );
        assert_eq!(out.len(), 1, "le rejeu du cycle ne reposte pas t1");
        assert_eq!(out[0].task_id, "t2");
    }

    #[test]
    fn mika2698_un_vol_rendu_deux_fois_dans_le_meme_cycle_ne_compte_quune_fois() {
        let out = comments_to_post(
            &[verdict("t1", Some(2693)), verdict("t1", Some(2693))],
            &[],
            true,
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn mika2698_un_verdict_sans_ticket_nest_pas_commente() {
        // `reference_url` absente ou illisible ⇒ aucun ticket connu. Le manager
        // ne choisit pas une cible par défaut.
        let out = comments_to_post(&[verdict("t1", None)], &[], true);
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn mika2698_un_echec_gh_laisse_le_vol_a_reessayer() {
        let runner = RecordingRunner {
            fail: true,
            ..Default::default()
        };
        let pending = comments_to_post(&[verdict("t1", Some(2693))], &[], true);
        let posted = post_verdict_comments(&runner, "senara-solutions/mika", &pending).await;
        assert!(
            posted.is_empty(),
            "un échec ne doit pas entrer dans le checkpoint, sinon le verdict est perdu"
        );
        assert_eq!(runner.calls().len(), 1, "l'appel a bien été tenté");
    }

    // ---- La mémoire du checkpoint ----------------------------------------

    #[test]
    fn mika2698_la_memoire_des_vols_commentes_est_bornee() {
        let previous: Vec<String> = (0..COMMENTED_HISTORY_CAP + 50)
            .map(|i| format!("old-{i}"))
            .collect();
        let out = remember_commented(&previous, &["fresh".to_string()]);
        assert_eq!(out.len(), COMMENTED_HISTORY_CAP);
        assert_eq!(out[0], "fresh", "le plus récent survit à la troncature");
        assert!(
            !out.contains(&format!("old-{}", COMMENTED_HISTORY_CAP + 40)),
            "la queue la plus ancienne est celle qui tombe"
        );
    }

    #[test]
    fn mika2698_la_memoire_ne_duplique_pas_un_vol() {
        let out = remember_commented(&["t1".to_string()], &["t1".to_string()]);
        assert_eq!(out, vec!["t1".to_string()]);
    }

    // ---- Le geste, et le fait qu'il reste seul ---------------------------

    #[test]
    fn mika2698_le_geste_est_un_seul_et_se_derive_de_sa_declaration() {
        // Si quelqu'un élargit `WRITE_GESTURE` (`gh issue edit`, deux gestes
        // dans la chaîne…), ceci rougit AVANT que l'allowlist ne soit
        // interrogée. La garde structurelle et ce test se tiennent par les deux
        // bouts : elle refuse un jeton non exempté, lui refuse un geste élargi.
        assert_eq!(WRITE_GESTURE, "gh issue comment");
        assert_eq!(write_gesture_argv(), vec!["issue", "comment"]);
    }

    #[test]
    fn mika2698_le_corps_du_commentaire_porte_le_verdict_du_rapport() {
        let v = verdict("task-pr", Some(2693));
        let body = verdict_comment_body(&v);
        assert!(
            body.contains(&render_flight_verdict(&v)),
            "le commentaire et le rapport rendent le MÊME verdict — obtenu :\n{body}"
        );
        assert!(
            body.contains("<!-- mika-manager:verdict:task-pr -->"),
            "le marqueur nomme le vol, pas le ticket"
        );
        assert!(body.contains("83/150 tours"), "les tours sur le plafond");
    }
}
