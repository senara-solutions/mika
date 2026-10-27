//! Le seul geste d'écriture du manager de milestone (mika#2698, vol 2).
//!
//! Ce fichier est l'**unique** exception nommée de l'allowlist de
//! `no_dispatch_test.rs`, pour l'unique jeton `gh issue comment`. Tout le reste
//! du module reste en lecture seule, et le garde continue de rougir sur tout
//! autre jeton d'écriture — y compris dans ce fichier-ci (propriété tenue par
//! `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier`).
//!
//! ## Trois propriétés, et pourquoi elles sont écrites comme ça
//!
//! **1. L'interrupteur est fermé par défaut, et son lecteur est unique.**
//! [`write_is_armed`] est le seul endroit du dépôt qui lit
//! `MIKA_MANAGER_EVALUATE_WRITE`. L'absence de la variable est l'état normal,
//! pas un cas d'erreur : un second lecteur, écrit un peu autrement, serait un
//! second interrupteur (doctrine mika#2201).
//!
//! **2. `armed` est un paramètre de [`post_verdicts`], pas une lecture
//! d'environnement.** La fonction est alors déterministe, testable en
//! parallèle sans `unsafe { set_var }`, et il reste exactement un lecteur de la
//! variable. La branche OFF est donc vérifiable *par un test de comportement*
//! — « zéro appel » — et pas seulement par l'absence d'un chemin.
//!
//! **3. Le geste est déclaré à plat, et les arguments en sont dérivés.**
//! Le vol 1 a documenté que la forme éclatée `.args(["issue", "comment"])`
//! **échappe** au grep plat du garde. Un posteur écrit naïvement n'aurait donc
//! besoin d'aucune exception, et l'entrée nommée que le ticket exige serait une
//! fiction : le garde dirait « rien à signaler » sur un fichier qui poste. En
//! dérivant le vecteur d'arguments du jeton lui-même, ce qui s'exécute est
//! exactement ce que le garde voit.
//!
//! ## Ce que ce fichier ne fait pas
//!
//! Il ne lit aucun commentaire existant sur la forge. Le ticket autorise **un**
//! geste, et c'est une écriture ; l'idempotence vient donc du checkpoint
//! (`MilestoneCheckpoint::posted_flight_task_ids`), pas d'une relecture
//! réseau. Le marqueur HTML en tête de corps n'est là que pour rendre un
//! doublon *reconnaissable* par un humain si ce checkpoint est perdu.

use super::evaluator::FlightVerdict;
use super::reader::GhRunner;
use super::reporter::render_flight_verdict;

/// L'interrupteur du ticket. **OFF par défaut.**
pub const ENV_EVALUATE_WRITE: &str = "MIKA_MANAGER_EVALUATE_WRITE";

/// Le geste autorisé, **à plat** — dans la forme même que
/// `no_dispatch_test.rs` grep. Voir le docstring du module, propriété 3 : ce
/// n'est pas une coquetterie, c'est ce qui rend l'exception honnête.
const GESTE: &str = "gh issue comment";

/// Le préfixe du marqueur d'idempotence porté par chaque corps.
pub const MARKER_PREFIX: &str = "<!-- mika-manager:verdict:";

/// Le seul lecteur de [`ENV_EVALUATE_WRITE`] du dépôt.
pub fn write_is_armed() -> bool {
    armed_from_value(std::env::var(ENV_EVALUATE_WRITE).ok().as_deref())
}

/// La table de vérité de l'interrupteur, séparée de l'environnement pour être
/// testable sans `unsafe`.
///
/// `None` — la variable absente — est **false**, et c'est le sens de « OFF par
/// défaut ». Toute valeur autre que `1`/`true` est également false : un
/// interrupteur d'écriture ne s'arme pas sur une faute de frappe.
fn armed_from_value(raw: Option<&str>) -> bool {
    match raw {
        Some(v) => {
            let v = v.trim();
            v.eq_ignore_ascii_case("1") || v.eq_ignore_ascii_case("true")
        }
        None => false,
    }
}

/// Le marqueur d'idempotence d'un vol donné.
pub fn verdict_marker(task_id: &str) -> String {
    format!("{MARKER_PREFIX}{task_id} -->")
}

/// Le corps du commentaire d'un vol.
///
/// La ligne de verdict vient de [`render_flight_verdict`], celle que le rapport
/// rend déjà : le commentaire sur le ticket et le rapport du manager disent
/// ainsi la **même phrase par construction**, et non deux phrases qu'il
/// faudrait maintenir d'accord.
pub fn render_verdict_comment(v: &FlightVerdict) -> String {
    format!(
        "{marker}\n\
         **Verdict du manager de milestone** — vol `{task_id}`\n\
         \n\
         {ligne}\n\
         \n\
         Verdict automatique (mika#2698). Ce commentaire est le **seul** geste \
         d'écriture du manager, derrière `{env}` ; le reste du module est en \
         lecture seule. Un désaccord se lit, il ne se moyenne pas — contredis-le \
         ici si tu le juges faux.",
        marker = verdict_marker(&v.task_id),
        task_id = v.task_id,
        ligne = render_flight_verdict(v),
        env = ENV_EVALUATE_WRITE,
    )
}

/// Poste un commentaire de verdict par vol non encore posté, et rend les
/// `task_id` **effectivement** postés.
///
/// Les trois raisons de ne rien poster, toutes silencieuses pour l'appelant et
/// visibles dans les logs :
///
/// - `armed == false` — l'AC1. Aucun appel au runner n'est émis, pas même pour
///   un vol neuf ;
/// - le vol est déjà dans `already_posted` — l'AC2, l'idempotence sur le rejeu ;
/// - le verdict n'a pas de numéro de ticket : il n'y a aucun endroit où poster,
///   et inventer un numéro serait pire que se taire.
///
/// Un appel `gh` qui échoue **n'est pas** rendu : le vol reste donc non posté et
/// sera retenté au cycle suivant. C'est la seule lecture honnête — marquer
/// comme posté ce qui ne l'a pas été perdrait le verdict pour toujours.
pub async fn post_verdicts(
    armed: bool,
    runner: &dyn GhRunner,
    repo: &str,
    verdicts: &[FlightVerdict],
    already_posted: &[String],
) -> Vec<String> {
    if !armed {
        return Vec::new();
    }
    let mut posted = Vec::new();
    for v in verdicts {
        if already_posted.iter().any(|id| id == &v.task_id) {
            continue;
        }
        let Some(number) = v.issue_number else {
            tracing::warn!(
                target: "mika::milestone_manager",
                event = "manager_verdict_not_postable",
                task_id = %v.task_id,
                reason = "issue_number_absent",
            );
            continue;
        };
        let numero = number.to_string();
        let corps = render_verdict_comment(v);
        let mut args = geste_args();
        args.push(&numero);
        args.push("--repo");
        args.push(repo);
        args.push("--body");
        args.push(&corps);
        match runner.run(&args).await {
            Ok(_) => {
                tracing::info!(
                    target: "mika::milestone_manager",
                    event = "manager_verdict_posted",
                    task_id = %v.task_id,
                    issue = number,
                    repo = %repo,
                );
                posted.push(v.task_id.clone());
            }
            Err(e) => {
                tracing::warn!(
                    target: "mika::milestone_manager",
                    event = "manager_verdict_post_failed",
                    task_id = %v.task_id,
                    issue = number,
                    error = %e,
                );
            }
        }
    }
    posted
}

/// Les arguments du geste, **dérivés** de [`GESTE`].
///
/// Le premier mot est le programme, que [`GhRunner`] fournit lui-même ; les
/// suivants sont le sous-commande. Un `GESTE` réécrit en une autre commande
/// changerait donc ce qui s'exécute *et* ce que le garde voit, ensemble.
fn geste_args() -> Vec<&'static str> {
    let mut mots = GESTE.split_whitespace();
    let programme = mots.next();
    debug_assert_eq!(
        programme,
        Some("gh"),
        "GESTE doit commencer par le programme que GhRunner lance"
    );
    mots.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::milestone_manager::evaluator::{
        FlightIssue, FlightRecommendation, PrStateKnowledge,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct RunnerEnregistreur {
        appels: Mutex<Vec<Vec<String>>>,
    }

    impl RunnerEnregistreur {
        fn appels(&self) -> Vec<Vec<String>> {
            self.appels.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl GhRunner for RunnerEnregistreur {
        async fn run(&self, args: &[&str]) -> anyhow::Result<String> {
            self.appels
                .lock()
                .unwrap()
                .push(args.iter().map(|a| (*a).to_string()).collect());
            Ok(String::new())
        }
    }

    struct RunnerEnPanne;

    #[async_trait::async_trait]
    impl GhRunner for RunnerEnPanne {
        async fn run(&self, _args: &[&str]) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("gh: HTTP 502"))
        }
    }

    fn verdict(task_id: &str, issue_number: Option<u64>) -> FlightVerdict {
        FlightVerdict {
            task_id: task_id.into(),
            repo: Some("senara-solutions/mika".into()),
            issue_number,
            issue: FlightIssue::PrOpened {
                number: Some(2704),
                url: None,
            },
            turns_used: Some(201),
            hit_ceiling: true,
            turn_ceiling: None,
            last_commit_turn: None,
            pr_state: PrStateKnowledge::Read(None),
            recommendation: FlightRecommendation::MergeCandidate,
            completed_at: Some("2026-10-14T22:00:00Z".into()),
        }
    }

    /// OFF par défaut, et toute valeur qui n'est pas un oui franc reste OFF.
    #[test]
    fn mika2698_la_table_de_verite_de_linterrupteur() {
        assert!(!armed_from_value(None), "l'absence est OFF");
        assert!(!armed_from_value(Some("")), "le vide est OFF");
        assert!(!armed_from_value(Some("0")));
        assert!(!armed_from_value(Some("false")));
        assert!(
            !armed_from_value(Some("oui")),
            "une faute de frappe est OFF"
        );
        assert!(armed_from_value(Some("1")));
        assert!(armed_from_value(Some("true")));
        assert!(armed_from_value(Some("TRUE")));
        assert!(armed_from_value(Some(" 1 ")), "les espaces sont tolérés");
    }

    /// Le lecteur unique rend faux dans un processus qui ne pose pas la
    /// variable — c'est-à-dire partout, jusqu'à un geste opérateur.
    #[test]
    fn mika2698_le_lecteur_unique_est_off_sans_la_variable() {
        if std::env::var(ENV_EVALUATE_WRITE).is_ok() {
            // L'environnement du test porte la variable : le lecteur n'est pas
            // ce qui est en cause, et se taire vaut mieux que rougir à tort.
            return;
        }
        assert!(!write_is_armed());
    }

    /// **AC1.** Interrupteur fermé : aucun appel, pas même pour un vol neuf.
    #[tokio::test]
    async fn mika2698_interrupteur_off_aucun_appel_decriture() {
        let runner = RunnerEnregistreur::default();
        let posted = post_verdicts(
            false,
            &runner,
            "senara-solutions/mika",
            &[verdict("t-1", Some(2698))],
            &[],
        )
        .await;
        assert!(posted.is_empty(), "rien ne doit être marqué comme posté");
        assert!(
            runner.appels().is_empty(),
            "AC1 : zéro appel gh avec l'interrupteur OFF, vu : {:?}",
            runner.appels()
        );
    }

    /// Le **contrôle négatif** de l'AC1 : sans lui, « zéro appel » serait aussi
    /// satisfait par un posteur qui ne poste jamais (classe mika#2205).
    #[tokio::test]
    async fn mika2698_interrupteur_on_un_seul_appel_et_cest_le_bon_geste() {
        let runner = RunnerEnregistreur::default();
        let posted = post_verdicts(
            true,
            &runner,
            "senara-solutions/mika",
            &[verdict("t-1", Some(2698))],
            &[],
        )
        .await;
        assert_eq!(posted, vec!["t-1".to_string()]);
        let appels = runner.appels();
        assert_eq!(appels.len(), 1, "un seul commentaire par vol : {appels:?}");
        let args = &appels[0];
        // Le geste dérivé du jeton : la sous-commande, puis le ticket, puis le
        // dépôt, puis le corps.
        assert_eq!(args[0], "issue");
        assert_eq!(args[1], "comment");
        assert_eq!(args[2], "2698");
        assert_eq!(args[3], "--repo");
        assert_eq!(args[4], "senara-solutions/mika");
        assert_eq!(args[5], "--body");
        assert!(
            args[6].starts_with(&verdict_marker("t-1")),
            "le corps doit porter le marqueur du vol : {}",
            args[6]
        );
    }

    /// **AC2.** Un vol déjà posté ne l'est pas deux fois — le rejeu du cycle.
    #[tokio::test]
    async fn mika2698_un_vol_deja_poste_nest_pas_repose() {
        let runner = RunnerEnregistreur::default();
        let posted = post_verdicts(
            true,
            &runner,
            "senara-solutions/mika",
            &[verdict("t-1", Some(2698)), verdict("t-2", Some(2699))],
            &["t-1".to_string()],
        )
        .await;
        assert_eq!(posted, vec!["t-2".to_string()]);
        assert_eq!(runner.appels().len(), 1, "seul le vol neuf est posté");
    }

    /// Un verdict sans numéro de ticket n'a aucun endroit où aller.
    #[tokio::test]
    async fn mika2698_un_vol_sans_numero_de_ticket_nest_pas_poste() {
        let runner = RunnerEnregistreur::default();
        let posted = post_verdicts(
            true,
            &runner,
            "senara-solutions/mika",
            &[verdict("t-1", None)],
            &[],
        )
        .await;
        assert!(posted.is_empty());
        assert!(runner.appels().is_empty());
    }

    /// Un `gh` en panne ne marque rien comme posté : le vol est retenté.
    #[tokio::test]
    async fn mika2698_un_echec_gh_ne_marque_pas_le_vol_comme_poste() {
        let posted = post_verdicts(
            true,
            &RunnerEnPanne,
            "senara-solutions/mika",
            &[verdict("t-1", Some(2698))],
            &[],
        )
        .await;
        assert!(
            posted.is_empty(),
            "un échec réseau ne doit pas consommer le verdict"
        );
    }

    /// Le commentaire et le rapport disent la même phrase, par construction.
    #[test]
    fn mika2698_le_corps_porte_la_ligne_de_verdict_du_rapport() {
        let v = verdict("t-1", Some(2698));
        let corps = render_verdict_comment(&v);
        assert!(
            corps.contains(&render_flight_verdict(&v)),
            "corps : {corps}"
        );
        assert!(corps.contains(ENV_EVALUATE_WRITE), "corps : {corps}");
    }
}
