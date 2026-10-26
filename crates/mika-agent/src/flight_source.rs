//! L'implémentation base de données de
//! [`crate::milestone_manager::evaluator::FlightSource`] (mika#2698, vol 1).
//!
//! # Pourquoi ce fichier vit **hors** de `milestone_manager/`
//!
//! Même idiome, et même raison, que [`crate::auth_boundary_ledger`] : le trait
//! est déclaré dans le module, son implémentation SQL est dehors.
//! `no_dispatch_test.rs` grep **chaque `.rs` de l'arbre du module** à la
//! recherche des jetons d'autorité d'écriture ; y faire entrer du SQL et un
//! handle de base ferait passer sous ce grep du code qui n'a rien à y faire, et
//! rendrait le prochain élargissement du grep inoffensif par accident. Le
//! module reste ce qu'il dit être : lecture, jugement, rapport.
//!
//! # Ce que la source lit, et ce qu'elle ne lit pas
//!
//! Elle lit `tasks` — rien d'autre. Pas de `gh`, pas de HTTP, pas de
//! transcript : le ticket demande « aucune nouvelle dépendance réseau », et
//! l'état de la PR arrive déjà par `SubIssue::pr_state`, hydraté par le
//! `Reader` au même cycle.
//!
//! # Ce qu'elle interroge : le conteneur, pas l'agent du handle (mika#2792)
//!
//! Le [`AsyncDatabase`] passé au constructeur ne sert que de **connexion**. Son
//! `agent_id` n'entre dans aucune requête d'ici : la lecture passe par
//! [`crate::db::Database::list_finished_implement_flights_in_container`], dont
//! le nom porte l'invariant. C'était le défaut de mika#2792 — le serveur
//! construit la source sur le handle de l'agent par défaut du processus (sur
//! gentux `mika-test`), les vols d'implement sont des tâches de `mika-dev`, et
//! la requête les filtrait sur l'agent emprunté. Quarante-trois vols en quatre
//! jours, zéro ligne rendue, aucun verdict, et un vide parfaitement lisible
//! comme « aucun vol terminé ».
//!
//! Qui construit la source choisit donc **une connexion**, jamais un périmètre
//! de lecture.
//!
//! # Le périmètre, et son lecteur unique (mika#2921)
//!
//! Le périmètre est **la liste des dépôts sous contrôle**, et plus les
//! sous-tickets du milestone surveillé. Un manager qui ne juge qu'un milestone
//! n'est pas un manager, c'est un rapporteur du milestone ; le rapport, lui,
//! reste par milestone.
//!
//! Ce fichier est le **seul lecteur** de
//! [`crate::webhook_dispatch::DISPATCHABLE_REPOS`] sur le chemin de
//! l'évaluateur, et c'est la forme du point 1 tranché par le plan : réutiliser
//! la liste que ce dépôt a déjà arbitrée sur la même question
//! (`server/ci_failure_handler.rs`), plutôt qu'en retaper une troisième. Les
//! quatre dépôts sont `mika`, `mika-cloud`, `mika-skills`, `mika-platform`.
//!
//! Le coût est nommé plutôt que tu : `claude-pilot` et `wizzard` — que le
//! CLAUDE.md du méta-dépôt compte parmi les dépôts contrôlés et que
//! `INTERNAL_REPOS` (privé, dans `mika-gateway`) porte — **sortent** du
//! périmètre jugé. L'alternative aurait été de promouvoir une liste adjacente à
//! la sécurité dans `mika-common`, c'est-à-dire un changement d'autorité
//! déguisé en dé-duplication. « Jamais au-delà » désigne la liste la plus
//! serrée, et le geste de réparation est **une ligne dans une seule liste**.
//! Aucun nom de dépôt n'est écrit ailleurs sur ce chemin : la garde
//! `milestone_manager/no_repo_list_test.rs` le refuse.
//!
//! # Une panne rend le vide, jamais une erreur
//!
//! [`FlightSource::finished_implement_flights`] ne rend pas de `Result`, et
//! c'est structurel plutôt que conventionnel : un cycle du manager ne doit pas
//! mourir parce qu'une lecture d'évaluation a échoué. Une base injoignable rend
//! `vec![]` — donc un rapport **sans** la section, ce qui se lit comme « aucun
//! vol terminé » — et un WARN nommé, qui est la trace que la différence a
//! existé.
//!
//! # Un vol est deux lignes de `tasks` (mika#2886)
//!
//! La ligne que l'URL du ticket trouve est la **parente** `ready-label: …#N` :
//! elle porte le ticket, l'agent et les tours, et son `result` est vide. La
//! ligne `Outcome: PR_OPENED — <url>` que l'évaluateur lit vit sur l'**enfant**
//! `long_running:run_claude_pilot`, dont le `reference_url` est NULL et dont le
//! `parent_task_id` pointe la parente — c'est la topologie que
//! [`crate::skills::executor::build_callback_task`] produit, et
//! `try_extract_callback_metadata` ne remonte dans la parente que des champs
//! **nommés** de métadonnées, jamais le texte du `result`.
//!
//! La projection assemble donc les deux : tous les champs viennent de la
//! parente, sauf `result`, qui vient de l'enfant quand il y en a un. Avant ce
//! correctif, la source rendait le `result` vide de la parente, et le puits du
//! manager a produit **531 verdicts identiques** — tous
//! `mort — cause non classée (no_outcome_line) — reprise à la main` — entre le
//! 2026-10-19 et le 2026-10-23, dont celui du vol de #2875, qui avait ouvert la
//! PR #2881, mergée.

use async_trait::async_trait;
use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::db::tasks::FinishedFlightRow;
use crate::milestone_manager::evaluator::{FinishedFlight, FlightSource};
use crate::webhook_dispatch::DISPATCHABLE_REPOS;

/// Les dépôts sous contrôle, **lus ici et nulle part ailleurs** sur le chemin de
/// l'évaluateur (mika#2921). Voir le module pour l'arbitrage et son coût.
///
/// Rend des `String` parce que c'est ce que le paramètre de liaison SQLite
/// demande ; la liste reste un `&[&str]` à sa source.
pub(crate) fn controlled_repos() -> Vec<String> {
    DISPATCHABLE_REPOS.iter().map(|r| r.to_string()).collect()
}

/// `https://github.com/<owner>/<repo>/issues/<n>` → `owner/repo` (mika#2921).
///
/// Tolère ce qui suit le numéro — `?phase=groom`, un `#fragment`, un segment de
/// plus — parce que la coupure est faite **avant** : le dépôt vit à gauche de
/// `/issues/`. Rend `None` sur tout ce qui n'est pas une URL d'issue GitHub, et
/// `None` est « ce vol ne nomme pas de dépôt », jamais une chaîne vide (motif
/// mika#2331).
pub(crate) fn repo_from_issue_url(url: &str) -> Option<String> {
    let (gauche, _) = url.split_once("/issues/")?;
    let reste = gauche
        .strip_prefix("https://github.com/")
        .or_else(|| gauche.strip_prefix("http://github.com/"))?;
    let (owner, repo) = reste.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// La source de production : les vols terminés lus dans `tasks`.
pub struct DbFlightSource {
    db: AsyncDatabase,
}

impl DbFlightSource {
    pub fn new(db: AsyncDatabase) -> Self {
        Self { db }
    }
}

#[async_trait]
impl FlightSource for DbFlightSource {
    async fn finished_implement_flights(&self, since: Option<&str>) -> Vec<FinishedFlight> {
        let repos = controlled_repos();
        let repo_count = repos.len();
        let rows = self
            .db
            .list_finished_implement_flights_in_container(since.map(str::to_string), repos)
            .await;
        match rows {
            Ok(lignes) => lignes.into_iter().map(task_to_finished_flight).collect(),
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "flight_source_read_failed",
                    repo_count,
                    since = since.unwrap_or(""),
                    error = %e,
                    "could not read finished flights — the cycle reports no verdict rather than dying"
                );
                Vec::new()
            }
        }
    }
}

/// [`FinishedFlightRow`] → [`FinishedFlight`], c'est-à-dire la projection sur
/// les huit données que l'évaluateur sait lire.
///
/// **`repo` est la huitième, et elle n'existe que depuis mika#2921** : le
/// rapport couvre désormais plusieurs dépôts, donc `#2900` tout seul est
/// ambigu — `mika#2900` et `mika-cloud#196` sont deux vols que le rapport doit
/// pouvoir nommer sans les confondre. Elle est dérivée de la ligne lue
/// ([`repo_from_issue_url`]), jamais de la liste interrogée : un vol qui ne
/// nomme pas de dépôt rend `None`, et le rendu le dit.
///
/// **`result` est la seule donnée qui ne vient pas de la parente** (mika#2886) :
/// `pilot_result.or(flight.result)` — l'enfant d'abord, la parente en
/// repli. Les six autres restent celles du vol, et c'est ce qui laisse
/// `PilotFields::parse` trouver les tours là où elle les a toujours trouvés, en
/// métadonnées de la parente.
///
/// Le repli n'est pas défensif : il couvre le vol qui n'a aucun enfant pilote —
/// un dispatch mort avant de produire un enfant, et la forme que sèment les
/// tests de mika#2698/#2792. Un enfant présent mais sans `result` ne l'atteint
/// pas : la lecture ne le retient pas (absent ≠ vide, motif mika#2331).
///
/// `completed_at` reprend le **même `COALESCE`** que la borne qui a sélectionné
/// la ligne
/// ([`crate::db::Database::list_finished_implement_flights_in_container`]) : un
/// vol mort par `failed` peut n'avoir que son `updated_at`, et l'horodatage
/// affiché doit être celui qui a décidé que ce vol était « depuis le cycle
/// précédent », pas un autre.
///
/// `agent_id` est la septième donnée, et elle n'est portée que depuis mika#2792.
/// La requête ne filtrant plus sur un agent, c'est la **ligne lue** qui dit
/// désormais chez qui le vol vivait ; sans cette projection, « zéro vol » et
/// « la bonne base, mais interrogée sur le mauvais agent » rendraient à nouveau
/// les mêmes octets — la mesure est ce qui a manqué quatre jours.
fn task_to_finished_flight(row: FinishedFlightRow) -> FinishedFlight {
    let FinishedFlightRow {
        flight: t,
        pilot_result,
    } = row;
    let repo = t.reference_url.as_deref().and_then(repo_from_issue_url);
    FinishedFlight {
        task_id: t.id,
        agent_id: t.agent_id,
        status: t.status,
        result: pilot_result.or(t.result),
        metadata: t.metadata,
        reference_url: t.reference_url,
        completed_at: t.completed_at.or(Some(t.updated_at)),
        repo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn seeded_db() -> AsyncDatabase {
        AsyncDatabase::new_with_agent(Database::open_in_memory().unwrap(), "mika")
    }

    /// Une base sans la ligne `agents` fait échouer la lecture ? Non : la
    /// lecture est un `SELECT`, elle réussit et rend zéro ligne. Ce que ce test
    /// pin, c'est la **forme** du contrat — pas d'erreur remontée, une liste.
    ///
    /// Le vide reste donc la lecture prévue d'une base sans vol. Ce qu'il ne
    /// peut plus être, depuis mika#2792, c'est le vide d'une base **peuplée**
    /// interrogée sur le mauvais agent : ce vide-là est mesuré par
    /// `mika2792_la_source_trouve_un_vol_dun_agent_different_du_handle`, et les
    /// deux tests ensemble séparent les deux populations que ce fichier rendait
    /// identiques.
    #[tokio::test]
    async fn mika2698_une_base_vide_rend_aucun_vol_sans_erreur() {
        let source = DbFlightSource::new(seeded_db());
        let got = source.finished_implement_flights(None).await;
        assert!(got.is_empty(), "une base vide n'a aucun vol terminé");
    }

    /// Le périmètre interrogé n'est **jamais vide** (mika#2921). La borne
    /// « liste vide ⇒ `vec![]` » vit en base
    /// (`mika2921_aucun_depot_ne_rend_jamais_tous_les_vols`) ; ce qu'elle ne peut
    /// pas dire, c'est que la production ne l'atteint pas. Depuis que
    /// `issue_urls` a disparu du trait, la source n'a plus de paramètre où un
    /// appelant pourrait passer `&[]` : la liste vient de
    /// [`controlled_repos`], et ce test atteste qu'elle porte bien les quatre
    /// dépôts sous contrôle.
    ///
    /// Un `DISPATCHABLE_REPOS` vidé rendrait l'évaluateur muet **sans aucune
    /// erreur** — un silence que rien d'autre ne nomme.
    #[test]
    fn mika2921_le_perimetre_interroge_nest_jamais_vide() {
        let repos = controlled_repos();
        assert!(
            !repos.is_empty(),
            "un périmètre vide rendrait l'évaluateur muet en silence"
        );
        for attendu in [
            "senara-solutions/mika",
            "senara-solutions/mika-cloud",
            "senara-solutions/mika-skills",
            "senara-solutions/mika-platform",
        ] {
            assert!(
                repos.iter().any(|r| r == attendu),
                "{attendu} doit être dans le périmètre jugé — reçu {repos:?}"
            );
        }
    }

    /// `repo_from_issue_url` : la dérivation du dépôt depuis l'URL **de la
    /// ligne lue** (mika#2921), suffixe de grooming compris, et `None` partout
    /// où il n'y a pas de dépôt à nommer — jamais une chaîne vide (motif
    /// mika#2331).
    #[test]
    fn mika2921_le_depot_se_lit_dans_lurl_du_ticket() {
        assert_eq!(
            repo_from_issue_url("https://github.com/senara-solutions/mika/issues/2900").as_deref(),
            Some("senara-solutions/mika")
        );
        assert_eq!(
            repo_from_issue_url("https://github.com/senara-solutions/mika-cloud/issues/196")
                .as_deref(),
            Some("senara-solutions/mika-cloud"),
            "le voisin de préfixe est un dépôt distinct, et il se nomme"
        );
        assert_eq!(
            repo_from_issue_url("https://github.com/senara-solutions/mika/issues/2693?phase=groom")
                .as_deref(),
            Some("senara-solutions/mika"),
            "le suffixe de grooming est après le numéro — il ne touche pas le dépôt"
        );
        for pas_une_url in [
            "",
            "https://github.com/senara-solutions/mika",
            "https://github.com/senara-solutions/mika/pull/2881",
            "https://example.com/senara-solutions/mika/issues/1",
            "https://github.com/mika/issues/1",
        ] {
            assert_eq!(
                repo_from_issue_url(pas_une_url),
                None,
                "{pas_une_url} ne nomme aucun dépôt d'issue"
            );
        }
    }

    /// Le `COALESCE` de `completed_at` est dans la projection, pas seulement
    /// dans le SQL : un vol dont seul `updated_at` est estampillé garde un
    /// horodatage lisible.
    #[test]
    fn mika2698_un_vol_sans_completed_at_retombe_sur_updated_at() {
        let mut t = task_fixture();
        t.completed_at = None;
        t.updated_at = "2026-10-14T08:00:00Z".into();
        let f = task_to_finished_flight(sans_enfant(t));
        assert_eq!(f.completed_at.as_deref(), Some("2026-10-14T08:00:00Z"));

        let mut t = task_fixture();
        t.completed_at = Some("2026-10-14T09:00:00Z".into());
        t.updated_at = "2026-10-14T08:00:00Z".into();
        let f = task_to_finished_flight(sans_enfant(t));
        assert_eq!(
            f.completed_at.as_deref(),
            Some("2026-10-14T09:00:00Z"),
            "quand `completed_at` existe, il gagne"
        );
    }

    /// La projection porte les huit données que l'évaluateur lit, et rien de
    /// plus : si une ligne de `tasks` gagne un champ, ce test ne bouge pas.
    ///
    /// Le vol n'a **aucun enfant pilote**, donc ce test porte aussi le repli de
    /// mika#2886 : `pilot_result` absent ⇒ c'est le `result` de la parente qui
    /// est rendu.
    ///
    /// La huitième est `repo` (mika#2921) : dérivée de l'URL **de la ligne**, et
    /// c'est elle qui permet au rapport de distinguer `mika#2900` de
    /// `mika-cloud#196`.
    #[test]
    fn mika2792_la_projection_porte_les_huit_donnees_lues() {
        let f = task_to_finished_flight(sans_enfant(task_fixture()));
        assert_eq!(f.task_id, "task-1");
        assert_eq!(f.status, "completed");
        assert!(f.result.unwrap().contains("Outcome: PR_OPENED"));
        assert!(f.metadata.unwrap().contains("claude_pilot"));
        assert_eq!(
            f.reference_url.as_deref(),
            Some("https://github.com/senara-solutions/mika/issues/2693")
        );
        assert_eq!(f.repo.as_deref(), Some("senara-solutions/mika"));
    }

    /// Un vol sans `reference_url` ne nomme **aucun** dépôt, et `repo` le dit
    /// par `None` — jamais par une chaîne vide (motif mika#2331, mika#2921).
    #[test]
    fn mika2921_un_vol_sans_url_ne_nomme_aucun_depot() {
        let mut t = task_fixture();
        t.reference_url = None;
        assert_eq!(task_to_finished_flight(sans_enfant(t)).repo, None);
    }

    /// La septième donnée est l'agent **de la ligne**, pas celui du handle : la
    /// projection la lit dans `tasks.agent_id`, et c'est elle qui rend l'agent
    /// interrogeable dans la ligne de scan du cycle (mika#2792).
    #[test]
    fn mika2792_la_projection_porte_lagent_du_vol() {
        let mut t = task_fixture();
        t.agent_id = "mika-dev".into();
        assert_eq!(task_to_finished_flight(sans_enfant(t)).agent_id, "mika-dev");
    }

    /// Un vol sans enfant pilote — la forme que lisent les tests de projection
    /// de mika#2698 et mika#2792, et le repli de mika#2886.
    fn sans_enfant(flight: crate::task_state::tasks::Task) -> FinishedFlightRow {
        FinishedFlightRow {
            flight,
            pilot_result: None,
        }
    }

    /// **Le test qui aurait rougi sur mika#2792.** Le chemin de production
    /// complet — `DbFlightSource` sur un handle d'agent `mika-test`, un vol
    /// semé sous `mika-dev` — et l'exigence que le vol soit trouvé.
    ///
    /// La suite de mika#2698 ne pouvait pas voir le défaut : elle semait sous
    /// l'`agent_id` du handle qu'elle passait, donc les deux coïncidaient
    /// toujours. Ici ils diffèrent, et c'est la condition de la question.
    #[tokio::test]
    async fn mika2792_la_source_trouve_un_vol_dun_agent_different_du_handle() {
        let url = "https://github.com/senara-solutions/mika/issues/2693";
        let db = Database::open_in_memory().unwrap();

        // `tasks.agent_id` est une FK vers `agents(id)` : l'agent dispatcheur
        // doit exister avant que son vol puisse l'être.
        db.register_agent("mika-dev", "mika-dev", "/tmp/mika-dev")
            .expect("l'agent dispatcheur doit être enregistré");
        let vol = crate::task_state::tasks::NewTask {
            dispatch_class: Some("implement".to_string()),
            ..crate::db::tests::groom_parent("mika-dev", url)
        };
        let id = db.create_task(&vol).expect("le vol doit être créé");
        db.update_task_completed(&id, "mika-dev", Some("Outcome: PR_OPENED\nTurns: 83\n"))
            .expect("la complétion doit réussir");

        // Le handle est celui de l'agent par défaut du processus — sur gentux
        // `mika-test`, et c'est lui que `server/mod.rs` prête à la source.
        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let got = source.finished_implement_flights(None).await;

        assert_eq!(
            got.len(),
            1,
            "le vol de `mika-dev` doit être trouvé par une source bâtie sur le \
             handle de `mika-test` — c'est le défaut de mika#2792"
        );
        assert_eq!(got[0].task_id, id);
        assert_eq!(
            got[0].agent_id, "mika-dev",
            "et le vol doit dire chez qui il vivait"
        );
    }

    /// L'URL du ticket de l'exemple relevé à la source (mika#2886) : le vol de
    /// #2875, classé « mort, reprise à la main » par le puits alors qu'il avait
    /// ouvert #2881.
    const TICKET_2886: &str = "https://github.com/senara-solutions/mika/issues/2875";

    /// Sème **la forme que la production produit**, et elle seule (mika#2886).
    ///
    /// Deux lignes, pas une. La **parente** `ready-label: …#N` porte l'URL du
    /// ticket et un `result` **absent** ; l'**enfant**
    /// `long_running:run_claude_pilot` porte l'issue du pilote, son
    /// `reference_url` est NULL et son `parent_task_id` pointe la parente.
    /// C'est le relevé de #2875 : parente `752be6cf` (`result` vide), enfant
    /// `d2dbf5e8` (`Outcome: PR_OPENED — …/pull/2881`).
    ///
    /// L'enfant passe par le constructeur de **production**
    /// [`crate::skills::executor::build_callback_task`] plutôt que par un
    /// `NewTask` littéral : c'est lui qui décide le label, le `reference_url`
    /// NULL et la `dispatch_class`, et un fixture qui les recopierait pourrait
    /// rester vert après qu'il a changé d'avis.
    ///
    /// Rend l'identifiant de la parente — la ligne que la source doit rendre.
    fn seme_la_forme_reelle(db: &Database, resultat_de_lenfant: &str) -> String {
        db.register_agent("mika-dev", "mika-dev", "/tmp/mika-dev")
            .expect("l'agent dispatcheur doit être enregistré");

        let parente = crate::task_state::tasks::NewTask {
            dispatch_class: Some("implement".to_string()),
            ..crate::db::tests::groom_parent("mika-dev", TICKET_2886)
        };
        let parente_id = db
            .create_task(&parente)
            .expect("la parente doit être créée");
        // `None` : la parente se termine sans `result`. C'est l'absence, pas une
        // chaîne vide (motif mika#2331), et c'est ce que la production écrit.
        db.update_task_completed(&parente_id, "mika-dev", None)
            .expect("la complétion de la parente doit réussir");

        let enfant = crate::skills::executor::build_callback_task(
            "mika-dev".to_string(),
            Some(parente_id.clone()),
            "run_claude_pilot",
            &serde_json::json!({"skill": "dev-pilot"}),
            3_600,
            "session-2886",
            "trace-2886",
            None,
        );
        let enfant_id = db.create_task(&enfant).expect("l'enfant doit être créé");
        db.update_task_completed(&enfant_id, "mika-dev", Some(resultat_de_lenfant))
            .expect("la complétion de l'enfant doit réussir");

        parente_id
    }

    /// **AC1 — le test qui rougit contre le code actuel (AC3).** La forme réelle
    /// sur le chemin complet : base → source → `evaluate_flights`, et
    /// l'exigence du verdict *PR ouverte, merge-candidat*.
    ///
    /// Les 531 verdicts du puits mesurés le 2026-10-23 disaient tous
    /// `mort — cause non classée (no_outcome_line) — reprise à la main`, parce
    /// que la source ne rendait que le `result` de la parente — vide. L'issue du
    /// pilote vit sur l'enfant.
    ///
    /// Le test est hermétique : `sub_issues` est vide, donc `pr_state` est
    /// `None`, et `recommend` rend `MergeCandidate` sur `PrOpened` + `None`
    /// (*« va regarder »*). Aucun accès à GitHub.
    #[tokio::test]
    async fn mika2886_la_forme_reelle_rend_un_verdict_de_pr_ouverte() {
        let db = Database::open_in_memory().unwrap();
        let parente_id = seme_la_forme_reelle(
            &db,
            "Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2881\nTurns: 121\n",
        );

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;

        assert_eq!(
            vols.len(),
            1,
            "un vol, celui de la parente : l'enfant n'a pas de `reference_url` et \
             n'est pas un vol de plus"
        );
        assert_eq!(
            vols[0].task_id, parente_id,
            "le vol rendu est la parente — c'est elle qui porte le ticket"
        );

        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(&vols, &[]);
        assert_eq!(verdicts.len(), 1);
        assert!(
            matches!(
                verdicts[0].issue,
                crate::milestone_manager::evaluator::FlightIssue::PrOpened { .. }
            ),
            "la ligne `Outcome:` de l'enfant doit atteindre l'évaluateur — reçu {:?}",
            verdicts[0].issue
        );
        assert_eq!(
            verdicts[0].recommendation,
            crate::milestone_manager::evaluator::FlightRecommendation::MergeCandidate,
            "une PR ouverte dont l'état n'est pas connu est un merge-candidat"
        );
    }

    /// **AC2 — le contrôle négatif, et il est auto-nettoyant.** Un enfant qui
    /// n'écrit aucune ligne `Outcome:` rend *toujours* `no_outcome_line` : le
    /// correctif de mika#2886 fait lire le bon `result`, il n'invente pas
    /// d'issue quand il n'y en a pas.
    ///
    /// Vert **avant comme après** le correctif, et c'est voulu : un contrôle
    /// négatif qui aurait rougi avant ne contrôlerait rien — il mesurerait le
    /// même défaut que l'AC1, par le même chemin.
    #[tokio::test]
    async fn mika2886_un_enfant_sans_ligne_outcome_reste_une_cause_non_classee() {
        let db = Database::open_in_memory().unwrap();
        seme_la_forme_reelle(
            &db,
            "Le pilote s'est arrêté sans rien conclure.\nTurns: 47\n",
        );

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;
        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(&vols, &[]);

        assert_eq!(verdicts.len(), 1);
        assert_eq!(
            verdicts[0].issue,
            crate::milestone_manager::evaluator::FlightIssue::Dead {
                cause: crate::milestone_manager::evaluator::DeathCause::Unknown(
                    "no_outcome_line".to_string()
                )
            },
            "sans ligne `Outcome:`, la cause reste non classée"
        );
        assert_eq!(
            verdicts[0].recommendation,
            crate::milestone_manager::evaluator::FlightRecommendation::ManualResumption
        );
    }

    fn task_fixture() -> crate::task_state::tasks::Task {
        crate::task_state::tasks::Task {
            id: "task-1".into(),
            agent_id: "mika".into(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "self_dev #2693".into(),
            trigger_type: "callback".into(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "skill".into(),
            action_config: "{}".into(),
            status: "completed".into(),
            process_id: None,
            input_context: None,
            result: Some("Outcome: PR_OPENED\nTurns: 83\nPR: https://github.com/senara-solutions/mika/pull/2696\n".into()),
            created_by_session: None,
            created_trace_id: None,
            execution_trace_id: None,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T06:00:00Z".into(),
            fired_at: None,
            completed_at: Some("2026-10-13T06:00:00Z".into()),
            reference_url: Some("https://github.com/senara-solutions/mika/issues/2693".into()),
            source: None,
            metadata: Some(r#"{"claude_pilot":{"turns":83}}"#.into()),
            r#type: "issue".into(),
            dispatch_class: Some("implement".into()),
            dispatcher_source: None,
        }
    }
}
