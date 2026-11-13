//! Le geste d'écriture du bassin `ready` (mika#2699, vol 4).
//!
//! Ce fichier est la **deuxième** exception nommée de l'allowlist de
//! `no_dispatch_test.rs`, pour l'unique jeton `gh issue edit`. Tout le reste du
//! module reste en lecture seule, et le garde continue de rougir sur tout autre
//! jeton d'écriture — y compris ici : `gh issue comment` lui est refusé dans ce
//! fichier-ci, comme `gh issue edit` l'est dans [`super::verdict_poster`]
//! (propriété tenue par
//! `mika2698_une_exception_ne_couvre_quun_seul_jeton_du_fichier`).
//!
//! ## Cinq propriétés, et pourquoi elles sont écrites comme ça
//!
//! **1. L'interrupteur est fermé par défaut, et son lecteur est unique.**
//! [`pool_write_is_armed`] est le seul endroit du dépôt qui lit
//! `MIKA_MANAGER_POOL_WRITE`. L'absence de la variable est l'état normal, pas un
//! cas d'erreur : un second lecteur, écrit un peu autrement, serait un second
//! interrupteur (doctrine mika#2201).
//!
//! **2. `armed` est un paramètre de [`admit_pool_tickets`], pas une lecture
//! d'environnement.** La fonction est alors déterministe, testable en parallèle
//! sans `unsafe { set_var }`, et il reste exactement un lecteur de la variable.
//! La branche OFF est donc vérifiable *par un test de comportement* — « zéro
//! appel » — et pas seulement par l'absence d'un chemin. Son **contrôle
//! négatif** (`armed == true` ⇒ exactement un appel) est ce qui empêche « zéro
//! appel » d'être aussi satisfait par un poseur mort (classe mika#2205).
//!
//! **3. Le geste est déclaré à plat, et les arguments en sont dérivés.**
//! Le vol 1 de mika#2698 a documenté que la forme éclatée
//! `.args(["issue", "edit"])` **échappe** au grep plat du garde. Un poseur écrit
//! naïvement n'aurait donc besoin d'aucune exception, et l'entrée nommée que le
//! ticket exige serait une fiction : le garde dirait « rien à signaler » sur un
//! fichier qui écrit. En dérivant le vecteur d'arguments du jeton lui-même, ce
//! qui s'exécute est exactement ce que le garde voit.
//!
//! **4. Les deux mutations sont un seul appel, et le milestone se rattache par
//! son TITRE.** L'AC3 demande « une seule mutation par décision » : poser
//! `ready` et rattacher au milestone cible sont deux effets d'un unique
//! `gh issue edit`, jamais deux appels qu'un échec intermédiaire pourrait
//! laisser à moitié faits. `--milestone` ne connaît pas les numéros de
//! milestone, seulement les titres — c'est donc [`MilestoneState::title`] que ce
//! fichier consomme, pas [`MilestoneRef::number`].
//!
//! [`MilestoneState::title`]: super::types::MilestoneState::title
//! [`MilestoneRef::number`]: super::types::MilestoneRef::number
//!
//! **5. Jamais un retrait.** Le ticket l'écrit en toutes lettres — « jamais de
//! retrait de `ready` pendant un vol » — et c'est une propriété de l'argv, donc
//! elle s'assertit sur l'argv : aucun jeton `--remove…` n'est composé, ce que
//! `mika2699_interrupteur_on_un_seul_appel_groupe_et_cest_le_bon_geste`
//! vérifie sur la commande réellement émise.
//!
//! ## Ce que ce fichier ne fait pas
//!
//! **Il ne tient pas de checkpoint, et n'en a pas besoin.** L'idempotence
//! inter-cycle du vol 2 de mika#2698 passait par
//! `MilestoneCheckpoint::posted_flight_task_ids` parce qu'un commentaire posé
//! est invisible au cycle suivant. Un **label** posé, lui, est lu par le filtre
//! de candidats de [`super::pool`] : un ticket qui porte `ready` n'est plus un
//! candidat, donc le second cycle ne le propose plus. L'idempotence est
//! structurelle ; un champ de checkpoint serait un second juge du même fait.
//!
//! **Il ne consulte pas le park de [`crate::ready_label`]** — et c'est une
//! précondition d'**ouverture**, pas un détail d'implémentation. mika#2315 fait
//! de `ready_label.rs` l'applicateur unique du label *gouverné*, celui qui
//! interroge le park avant d'écrire, pour qu'un STOP opérateur tienne. Ce geste
//! ne peut pas passer par [`crate::ready_label::apply_ready`] sans renoncer à la
//! propriété 4 : `apply_ready` pose le label et rien d'autre, donc le rattachement
//! du milestone redeviendrait une seconde mutation. Ce que l'interrupteur OFF par
//! défaut garantit, c'est qu'aucune écriture non parkée n'a lieu au vol 4 ; ce
//! que l'ouverture demandera en plus, c'est de trancher cette interaction. Elle
//! est nommée dans le corps de la PR du vol sous
//! `Fire-Disposition: cinquième applicateur découvert`, comme le test 13 de
//! `ready_label.rs` l'exige de qui la rencontre.

use super::pool::Admission;
use super::reader::GhRunner;
use crate::ready_label::READY_LABEL;

/// L'interrupteur du point 3 du ticket. **OFF par défaut.**
pub const ENV_POOL_WRITE: &str = "MIKA_MANAGER_POOL_WRITE";

/// Le geste autorisé, **à plat** — dans la forme même que
/// `no_dispatch_test.rs` grep. Voir le docstring du module, propriété 3 : ce
/// n'est pas une coquetterie, c'est ce qui rend l'exception honnête.
const GESTE: &str = "gh issue edit";

/// Le seul lecteur de [`ENV_POOL_WRITE`] du dépôt.
pub fn pool_write_is_armed() -> bool {
    armed_from_value(std::env::var(ENV_POOL_WRITE).ok().as_deref())
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

/// Admet au bassin les tickets d'un verdict, **un appel groupé par ticket**, et
/// rend les numéros **effectivement** posés.
///
/// Les deux raisons de ne rien faire du tout, toutes deux dites par une sonde :
///
/// - `armed == false` — l'AC1. Aucun appel au runner n'est émis, pas même pour
///   une admission neuve (`manager_pool_not_admitted`, `reason = "disarmed"`) ;
/// - le milestone cible n'a pas de titre. `--milestone` résout par titre : sans
///   titre il n'y a rien à rattacher, et poser `ready` seul livrerait la moitié
///   d'une décision groupée (`reason = "empty_milestone_title"`). Se taire est
///   la seule lecture honnête.
///
/// Un appel `gh` qui échoue **n'est pas** rendu : le ticket reste donc candidat
/// et sera re-proposé au cycle suivant. Marquer comme admis ce qui ne l'a pas
/// été ferait disparaître le ticket du bassin *et* de la file des candidats.
pub async fn admit_pool_tickets(
    armed: bool,
    runner: &dyn GhRunner,
    repo: &str,
    milestone_title: &str,
    admissions: &[Admission],
) -> Vec<u64> {
    if !armed {
        tracing::info!(
            target: "mika::milestone_manager",
            event = "manager_pool_not_admitted",
            reason = "disarmed",
            candidates = admissions.len(),
        );
        return Vec::new();
    }
    let titre = milestone_title.trim();
    if titre.is_empty() {
        tracing::warn!(
            target: "mika::milestone_manager",
            event = "manager_pool_not_admitted",
            reason = "empty_milestone_title",
            candidates = admissions.len(),
        );
        return Vec::new();
    }
    let mut poses = Vec::new();
    for admission in admissions {
        let numero = admission.number.to_string();
        let mut args = geste_args();
        args.push(&numero);
        args.push("--repo");
        args.push(repo);
        args.push("--add-label");
        args.push(READY_LABEL);
        args.push("--milestone");
        args.push(titre);
        match runner.run(&args).await {
            Ok(_) => {
                tracing::info!(
                    target: "mika::milestone_manager",
                    event = "manager_pool_admitted",
                    issue = admission.number,
                    repo = %repo,
                    milestone = %titre,
                );
                poses.push(admission.number);
            }
            Err(e) => {
                tracing::warn!(
                    target: "mika::milestone_manager",
                    event = "manager_pool_admit_failed",
                    issue = admission.number,
                    repo = %repo,
                    error = %e,
                );
            }
        }
    }
    poses
}

/// Les arguments du geste, **dérivés** de [`GESTE`].
///
/// Le premier mot est le programme, que [`GhRunner`] fournit lui-même ; les
/// suivants sont la sous-commande. Un `GESTE` réécrit en une autre commande
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
    use crate::milestone_manager::pool::{CandidateOrigin, PlanSizeReading};
    use std::sync::Mutex;

    /// Le harnais est **redéclaré ici** plutôt que partagé avec le `mod tests`
    /// de `verdict_poster.rs` : un `mod tests` n'est pas importable depuis un
    /// module frère sans le rendre visible en production, et une trentaine de
    /// lignes dupliquées coûte moins qu'un chemin de test exporté.
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

    fn admission(number: u64) -> Admission {
        Admission {
            number,
            origin: CandidateOrigin::Milestone,
            size: PlanSizeReading::Measured(120),
        }
    }

    /// OFF par défaut, et toute valeur qui n'est pas un oui franc reste OFF.
    ///
    /// La dernière assertion est celle du **lecteur unique** : dans un processus
    /// qui ne pose pas la variable — c'est-à-dire partout, jusqu'à un geste
    /// opérateur — il rend faux. Elle est conditionnelle parce qu'un
    /// environnement de test qui porterait la variable ne met pas le lecteur en
    /// cause, et se taire vaut mieux que rougir à tort.
    #[test]
    fn mika2699_la_table_de_verite_de_linterrupteur() {
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
        if std::env::var(ENV_POOL_WRITE).is_err() {
            assert!(!pool_write_is_armed(), "le lecteur unique est OFF");
        }
    }

    /// **AC1.** Interrupteur fermé : aucun appel, pas même pour une admission
    /// neuve.
    #[tokio::test]
    async fn mika2699_interrupteur_off_aucun_appel_decriture() {
        let runner = RunnerEnregistreur::default();
        let poses = admit_pool_tickets(
            false,
            &runner,
            "senara-solutions/mika",
            "Phase 2 — le manager tient le bassin",
            &[admission(2699), admission(2687)],
        )
        .await;
        assert!(poses.is_empty(), "rien ne doit être marqué comme posé");
        assert!(
            runner.appels().is_empty(),
            "AC1 : zéro appel gh avec l'interrupteur OFF, vu : {:?}",
            runner.appels()
        );
    }

    /// Le **contrôle négatif** de l'AC1 et la première moitié de l'AC3 : armé,
    /// une décision vaut **un** appel, et c'est le geste groupé — `ready` posé
    /// et le milestone rattaché ensemble, aucun retrait nulle part.
    #[tokio::test]
    async fn mika2699_interrupteur_on_un_seul_appel_groupe_et_cest_le_bon_geste() {
        let runner = RunnerEnregistreur::default();
        let poses = admit_pool_tickets(
            true,
            &runner,
            "senara-solutions/mika",
            "Phase 2 — le manager tient le bassin",
            &[admission(2699)],
        )
        .await;
        assert_eq!(poses, vec![2699]);
        let appels = runner.appels();
        assert_eq!(
            appels.len(),
            1,
            "AC3 : une seule mutation par décision : {appels:?}"
        );
        let args = &appels[0];
        // Le geste dérivé du jeton, puis le ticket, puis le dépôt, puis les deux
        // effets groupés — dans cet ordre, parce que c'est la commande qui part.
        assert_eq!(args[0], "issue");
        assert_eq!(args[1], "edit");
        assert_eq!(args[2], "2699");
        assert_eq!(args[3], "--repo");
        assert_eq!(args[4], "senara-solutions/mika");
        assert_eq!(args[5], "--add-label");
        assert_eq!(args[6], "ready");
        assert_eq!(args[7], "--milestone");
        assert_eq!(args[8], "Phase 2 — le manager tient le bassin");
        assert_eq!(args.len(), 9, "rien d'autre ne part : {args:?}");
        assert!(
            args.iter().all(|a| !a.starts_with("--remove")),
            "jamais un retrait de `ready` : {args:?}"
        );
    }

    /// Ni un `gh` en panne ni un milestone sans titre ne comptent comme posés.
    ///
    /// Les deux ensemble parce qu'ils disent la même chose par deux chemins :
    /// ce qui est rendu est ce qui a **eu lieu**. Le titre vide ne pose rien du
    /// tout — `--milestone` résout par titre, et poser `ready` seul livrerait la
    /// moitié d'une décision groupée.
    #[tokio::test]
    async fn mika2699_un_echec_gh_et_un_titre_vide_ne_comptent_pas_comme_poses() {
        let poses = admit_pool_tickets(
            true,
            &RunnerEnPanne,
            "senara-solutions/mika",
            "Phase 2 — le manager tient le bassin",
            &[admission(2699)],
        )
        .await;
        assert!(
            poses.is_empty(),
            "un échec réseau ne doit pas consommer l'admission"
        );

        let runner = RunnerEnregistreur::default();
        let poses = admit_pool_tickets(
            true,
            &runner,
            "senara-solutions/mika",
            "   ",
            &[admission(2699)],
        )
        .await;
        assert!(poses.is_empty());
        assert!(
            runner.appels().is_empty(),
            "sans titre de milestone, aucun appel ne part : {:?}",
            runner.appels()
        );
    }
}
