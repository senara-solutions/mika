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
use tracing::{info, warn};

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
            Ok(lignes) => {
                let (ajournees, termines): (Vec<_>, Vec<_>) =
                    lignes.into_iter().partition(|l| l.pending_implement);
                sonde_ajournements(&ajournees);
                termines.into_iter().map(task_to_finished_flight).collect()
            }
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

/// Dit, à **chaque** lecture, combien de parentes ont été ajournées parce qu'un
/// pilote d'implémentation vole encore sous elles (mika#2951).
///
/// **Émise y compris à zéro**, et c'est tout son intérêt. Sans la ligne, « aucun
/// vol parce qu'il n'y en avait pas » et « aucun vol parce que les trois en
/// cours ont été ajournés » rendent les mêmes octets — deux populations sous un
/// seul nom ne sont pas soustractibles (mika#2205). C'est exactement le silence
/// qui a laissé le registre figer trois verdicts prématurés sans que personne
/// le voie.
///
/// `task_ids` est **absent** quand `deferred == 0`, jamais `""` : rien n'a été
/// ajourné, ce n'est pas « une liste vide d'ajournements » (motif mika#2331). Et
/// l'**absence de la ligne** signifie un binaire antérieur à ce correctif, pas
/// un zéro (motif mika#2340) : l'instrument n'est vivant qu'après `make deploy`.
fn sonde_ajournements(ajournees: &[FinishedFlightRow]) {
    let task_ids = if ajournees.is_empty() {
        None
    } else {
        Some(
            ajournees
                .iter()
                .map(|l| l.flight.id.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    info!(
        target: "mika::milestone_manager",
        event = "flight_deferred_pending_implement",
        deferred = ajournees.len(),
        task_ids = task_ids.as_deref(),
        scope = "container",
        "parentes terminales dont le pilote d'implémentation vole encore — non jugées ce cycle"
    );
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
        // mika#2951 — déjà consommé par `finished_implement_flights`, qui
        // n'appelle cette projection que sur les vols non ajournés. Le champ est
        // nommé ici pour que l'ajout d'un troisième ne passe pas en silence.
        pending_implement: _,
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

    /// L'instant du cycle qui date la lecture CI (mika#2968). Les tests d'ici
    /// ne câblent aucun lecteur de PR, donc **aucune** CI n'est lue et cette
    /// valeur n'entre dans aucune de leurs assertions — elle existe pour que
    /// l'appel reste celui de la production.
    const AU_CYCLE_DE: &str = "2026-10-29T17:46:12Z";

    /// Aucun lecteur de PR : ce qui est mesuré ici est l'**appariement des
    /// vols** (quelle tâche porte le ticket, quelle ligne `Outcome:` atteint
    /// l'évaluateur), pas la lecture `gh`. `Unwired` le dit sans l'inventer —
    /// les verdicts rendus portent une CI **et** une revue non lues, cause
    /// `no_runner`.
    ///
    /// **Conséquence depuis mika#2968 AC6 : la recommandation d'un `PrOpened`
    /// est ici `ManualResumption`, pas `MergeCandidate`.** Non parce que le
    /// vol serait suspect, mais parce que AC6 est écrit en positif — *ne rendre
    /// merge-candidat que si la tête porte une approbation du relecteur sur ce
    /// commit* — donc une revue non lue n'est pas une preuve d'autorisation.
    /// C'est l'inverse de AC1, négatif, où une CI non lue ne retirait rien.
    /// Les tests de ce module épinglent l'**issue** du vol ; leur
    /// recommandation n'est qu'un corollaire de l'absence de lecteur.
    const SANS_LECTEUR: crate::milestone_manager::evaluator::FlightPrReads =
        crate::milestone_manager::evaluator::FlightPrReads::Unwired;

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
            pending_implement: false,
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
    ///
    /// Le geste lui-même a été **promu** en
    /// [`crate::db::tests::seme_la_forme_reelle`] par mika#2921 : le corpus des
    /// 13 vols mesurés en a besoin depuis `db::tests::vols_termines_du_manager`,
    /// un module frère de celui-ci — donc invisible d'ici, et réciproquement
    /// (règle de visibilité mika#2321). Ce qui reste ici est ce que mika#2886
    /// fixe : *ce* ticket, *cet* agent, et une parente `completed`.
    fn seme_la_forme_reelle(db: &Database, resultat_de_lenfant: &str) -> String {
        crate::db::tests::seme_la_forme_reelle(
            db,
            "mika-dev",
            &crate::db::tests::VolReel {
                url_du_ticket: TICKET_2886,
                statut: "completed",
                issue_du_pilote: resultat_de_lenfant,
            },
        )
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
    /// Le test est hermétique : `sub_issues` est vide et il n'y a pas de
    /// lecteur, donc ni la CI ni la revue de la tête ne sont lues. Aucun accès
    /// à GitHub. La recommandation rendue est `ManualResumption` — voir
    /// [`SANS_LECTEUR`] pour pourquoi ; ce que mika#2886 épingle ici est
    /// l'**issue** `PrOpened`, et c'est l'assertion qui précède.
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

        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(
            &vols,
            &[],
            &SANS_LECTEUR,
            AU_CYCLE_DE,
        );
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
            crate::milestone_manager::evaluator::FlightRecommendation::ManualResumption,
            "sans lecteur, la revue de la tête n'est pas lue, et AC6 (mika#2968) \
             ne rend merge-candidat que sur une approbation lue sur ce commit"
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
        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(
            &vols,
            &[],
            &SANS_LECTEUR,
            AU_CYCLE_DE,
        );

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

    /// Le ticket de la première ligne de la mesure de mika#2951 : parente
    /// `9bed44f8`, groom terminé à 12:28:08Z, pilote d'implémentation terminé à
    /// 13:20:48Z sur la PR #2947 — et registre figé sur
    /// « mort PLAN_GROOMED » dès 12:31:59Z.
    const TICKET_2951: &str = "https://github.com/senara-solutions/mika/issues/2944";

    /// Ce que le pilote de **groom** écrit : la ligne que mika#2949 classe en
    /// `not_a_flight`.
    const ISSUE_DU_GROOM: &str = "Outcome: PLAN_GROOMED\nTurns: 44\n";

    /// Ce que le pilote d'**implémentation** écrit : la PR réellement ouverte,
    /// mergée depuis par Vincent.
    const ISSUE_DE_LIMPLEMENTATION: &str =
        "Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2947\nTurns: 197\n";

    /// Sème un vol de groom relancé sous `mika-dev` et rend ses ids.
    fn seme_un_groom_relance(
        db: &Database,
        implementation: crate::db::tests::Implementation<'_>,
        regroom_posterieur: Option<&str>,
    ) -> crate::db::tests::VolRelanceSeme {
        crate::db::tests::seme_un_groom_puis_un_implement(
            db,
            "mika-dev",
            &crate::db::tests::VolDeGroomRelance {
                url_du_ticket: TICKET_2951,
                issue_du_groom: ISSUE_DU_GROOM,
                implementation,
                regroom_posterieur,
            },
        )
    }

    /// **AC1 — le test qui rougit contre `origin/main`.** Une parente dont
    /// l'enfant d'implémentation vole encore n'est pas un vol terminé.
    ///
    /// Contre le code d'avant le correctif, la source rend **un** vol : la
    /// parente est `completed` (mika#1614 réutilise sa row), son seul enfant
    /// terminal est le pilote de **groom**, et l'évaluateur en tire
    /// `not_a_flight` — verdict que `flight_verdict_ledger` fige pour toujours,
    /// alors que la PR n'est même pas encore ouverte. C'est la ligne
    /// « 12:31:59Z : mort PLAN_GROOMED » de la mesure.
    ///
    /// Après le correctif, la lecture **ajourne** : zéro vol, donc zéro verdict,
    /// donc rien à figer. Le vol sera jugé au cycle suivant la fin de l'enfant —
    /// c'est le test [`mika2951_une_fois_limplement_terminal_cest_son_issue_qui_est_jugee`].
    #[tokio::test]
    async fn mika2951_une_parente_dont_limplement_vole_encore_nest_pas_un_vol_termine() {
        let db = Database::open_in_memory().unwrap();
        let seme = seme_un_groom_relance(&db, crate::db::tests::Implementation::EnVol, None);

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;

        assert!(
            vols.is_empty(),
            "la parente {} est terminale, mais son enfant d'implémentation vole \
             encore : le vol n'est pas terminé et ne doit pas être rendu — reçu {:?}",
            seme.parente,
            vols.iter().map(|v| &v.result).collect::<Vec<_>>()
        );
    }

    /// **AC1 — la précédence est celle de la classe, jamais celle de la
    /// récence.** Un re-groom terminé *après* l'implémentation ne reprend pas la
    /// main.
    ///
    /// C'est le test qui refuse le correctif facile. « Prendre l'enfant le plus
    /// récent » laisserait le premier test vert et celui-ci rouge, parce que
    /// « le plus récent » est exactement la moitié du défaut : la lecture d'avant
    /// le correctif prend déjà le plus récent *des enfants terminaux*, et c'est
    /// le groom. Seul un prédicat sur le **label** de l'enfant
    /// (`is_implement_pilot_label`) tient les deux.
    #[tokio::test]
    async fn mika2951_limplement_prime_le_groom_meme_quand_le_groom_est_plus_recent() {
        let db = Database::open_in_memory().unwrap();
        let seme = seme_un_groom_relance(
            &db,
            crate::db::tests::Implementation::Terminee(ISSUE_DE_LIMPLEMENTATION),
            Some("Outcome: PLAN_GROOMED\nTurns: 12\n"),
        );
        assert!(
            seme.regroom.is_some(),
            "le re-groom postérieur est la condition de la question"
        );

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;

        assert_eq!(vols.len(), 1, "un vol, la parente");
        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(
            &vols,
            &[],
            &SANS_LECTEUR,
            AU_CYCLE_DE,
        );
        assert_eq!(
            verdicts[0].issue,
            crate::milestone_manager::evaluator::FlightIssue::PrOpened {
                number: Some(2947),
                url: Some("https://github.com/senara-solutions/mika/pull/2947".to_string()),
            },
            "l'issue jugée est celle de l'implémentation, pas celle du groom le \
             plus récent"
        );
    }

    /// **AC1 — une fois l'enfant d'implémentation terminal, c'est son issue qui
    /// est jugée.** La forme exacte de la première ligne de la mesure, à
    /// l'instant où le manager a enfin le droit de juger : PR ouverte, #2947.
    ///
    /// Hermétique comme son aîné de mika#2886 : `sub_issues` est vide et il n'y
    /// a pas de lecteur, donc rien de la tête n'est lu et rien ne touche
    /// GitHub. Recommandation `ManualResumption` depuis mika#2968 AC6 — voir
    /// [`SANS_LECTEUR`] ; ce que ce test épingle est l'issue.
    #[tokio::test]
    async fn mika2951_une_fois_limplement_terminal_cest_son_issue_qui_est_jugee() {
        let db = Database::open_in_memory().unwrap();
        let seme = seme_un_groom_relance(
            &db,
            crate::db::tests::Implementation::Terminee(ISSUE_DE_LIMPLEMENTATION),
            None,
        );

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;

        assert_eq!(vols.len(), 1, "un vol, la parente");
        assert_eq!(
            vols[0].task_id, seme.parente,
            "le vol rendu est la parente — c'est elle qui porte le ticket"
        );

        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(
            &vols,
            &[],
            &SANS_LECTEUR,
            AU_CYCLE_DE,
        );
        assert_eq!(verdicts.len(), 1);
        assert_eq!(
            verdicts[0].issue,
            crate::milestone_manager::evaluator::FlightIssue::PrOpened {
                number: Some(2947),
                url: Some("https://github.com/senara-solutions/mika/pull/2947".to_string()),
            },
            "l'issue du pilote d'implémentation doit atteindre l'évaluateur"
        );
        assert_eq!(
            verdicts[0].recommendation,
            crate::milestone_manager::evaluator::FlightRecommendation::ManualResumption,
            "corollaire de l'absence de lecteur, pas de l'issue : voir `SANS_LECTEUR`"
        );
    }

    /// **AC2 — contrôle négatif n° 1 : un groom pur reste un non-vol.**
    ///
    /// La parente a basculé en `implement` (étape 5c du dispatch) mais aucun
    /// enfant d'implémentation n'est né : le frein `auto_pull_stop`, un jeton
    /// GitHub absent, un `validate_dispatch_readiness` qui refuse, ou un handler
    /// manquant. Le seul enfant est le groom, et mika#2949 le classe
    /// `not_a_flight` — ce que ce test exige **avant comme après** le correctif.
    ///
    /// C'est ce qui sépare « ajourner une parente dont l'implémentation vole »
    /// de « ne plus jamais rendre une parente de groom » : sans ce contrôle, un
    /// correctif qui ajournerait les deux resterait vert.
    #[tokio::test]
    async fn mika2951_un_groom_pur_reste_un_non_vol() {
        let db = Database::open_in_memory().unwrap();
        seme_un_groom_relance(&db, crate::db::tests::Implementation::Absente, None);

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;

        assert_eq!(
            vols.len(),
            1,
            "un groom sans implémentation relancée est un vol terminé — il n'a \
             rien à attendre"
        );
        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(
            &vols,
            &[],
            &SANS_LECTEUR,
            AU_CYCLE_DE,
        );
        assert_eq!(
            verdicts[0].issue,
            crate::milestone_manager::evaluator::FlightIssue::NotAFlight {
                reason: crate::milestone_manager::evaluator::NotFlightReason::GroomCompleted,
            },
            "classé comme mika#2949 le classe"
        );
        assert_eq!(
            verdicts[0].recommendation,
            crate::milestone_manager::evaluator::FlightRecommendation::Nothing
        );
    }

    /// **AC2 — contrôle négatif n° 2 : un vol `ready-label` classique est jugé
    /// comme aujourd'hui.**
    ///
    /// Une parente, **un seul** enfant pilote d'implémentation, aucun groom : la
    /// topologie de mika#2886, inchangée par mika#2951. Vert avant comme après,
    /// par le même chemin de production que
    /// `mika2886_la_forme_reelle_rend_un_verdict_de_pr_ouverte` — dont il se
    /// distingue par ce qu'il interroge : non pas « l'enfant est-il lu ? » mais
    /// « le nouveau prédicat laisse-t-il passer le cas nominal ? ».
    #[tokio::test]
    async fn mika2951_un_vol_ready_label_a_un_seul_enfant_est_juge_comme_avant() {
        let db = Database::open_in_memory().unwrap();
        let parente_id = seme_la_forme_reelle(&db, ISSUE_DE_LIMPLEMENTATION);

        let source = DbFlightSource::new(AsyncDatabase::new_with_agent(db, "mika-test"));
        let vols = source.finished_implement_flights(None).await;

        assert_eq!(vols.len(), 1, "un vol, la parente");
        assert_eq!(vols[0].task_id, parente_id);
        let verdicts = crate::milestone_manager::evaluator::evaluate_flights(
            &vols,
            &[],
            &SANS_LECTEUR,
            AU_CYCLE_DE,
        );
        assert_eq!(
            verdicts[0].issue,
            crate::milestone_manager::evaluator::FlightIssue::PrOpened {
                number: Some(2947),
                url: Some("https://github.com/senara-solutions/mika/pull/2947".to_string()),
            },
            "le vol `ready-label` à un seul enfant est jugé comme avant"
        );
        assert_eq!(
            verdicts[0].recommendation,
            crate::milestone_manager::evaluator::FlightRecommendation::ManualResumption,
            "corollaire de l'absence de lecteur, pas du prédicat de mika#2951 : \
             voir `SANS_LECTEUR`"
        );
    }

    // ═════════════════════════════════════════════════════════════════════
    // mika#2951 — Fire-Disposition : `pending_implement` a un lecteur unique.
    // ═════════════════════════════════════════════════════════════════════

    /// Les deux formes sous lesquelles le champ est touché dans du code Rust.
    ///
    /// Deux, pas une, et ni l'une ni l'autre n'est le nom nu : `pending_implement`
    /// seul attraperait la chaîne `"flight_deferred_pending_implement"` — le nom
    /// de la sonde, qui n'est pas une lecture du champ — et rendrait l'attendu
    /// faux pour une raison qui n'a rien à voir avec ce que le scan refuse.
    const AIGUILLES_DAJOURNEMENT: &[&str] = &[".pending_implement", "pending_implement:"];

    /// Les **fichiers** de production qui touchent le champ, triés.
    ///
    /// Le grain est le fichier et non la fonction, pour deux raisons qui vont
    /// dans le même sens :
    ///
    /// - `pending_implement` est un contrat **entre** deux fichiers — l'un
    ///   l'écrit, l'autre le décide — donc « qui le touche » se pose à ce
    ///   grain-là, et c'est à ce grain que la régression redoutée se voit.
    /// - [`crate::source_scan::fn_bodies`] est **plat** : `recence` est une
    ///   fonction imbriquée dans `list_finished_implement_flights_in_container`,
    ///   donc tout ce qui suit sa déclaration est attribué à `recence` et le nom
    ///   de la fonction englobante n'apparaît jamais. Mesuré, pas supposé.
    ///   Au même grain, un `pub pending_implement: bool` à portée de module
    ///   serait invisible — la borne que le doc-comment de
    ///   [`crate::source_scan::strip_comment_lines`] nomme.
    ///
    /// Les commentaires sont retirés par [`crate::source_scan::strip_comment_lines`] :
    /// trois doc-comments de ce ticket nomment le champ pour expliquer la
    /// décision (celui de `list_finished_implement_flights_in_container`, celui
    /// du champ lui-même, celui du test D5 dans `task_engine/engine.rs`) et une
    /// mention n'est pas une lecture — motif mika#2050 signal S.
    ///
    /// Le corpus est **tout** `src/`, pas un module : la régression redoutée est
    /// précisément un lecteur qui naît ailleurs.
    fn fichiers_touchant_lajournement(corpus: &[(String, String)]) -> Vec<String> {
        let mut trouves: Vec<String> = corpus
            .iter()
            .filter(|(_, source)| {
                let code = crate::source_scan::strip_comment_lines(source);
                AIGUILLES_DAJOURNEMENT.iter().any(|a| code.contains(a))
            })
            .map(|(rel, _)| rel.clone())
            .collect();
        trouves.sort();
        trouves.dedup();
        trouves
    }

    /// **`pending_implement` est rapporté par la base et décidé ici, nulle part
    /// ailleurs.**
    ///
    /// La couche base *rapporte* (mika#2205 : deux populations sous un nom ne
    /// sont pas soustractibles, donc elle ne filtre pas), et `flight_source`
    /// *décide* — il écarte les parentes ajournées et émet la sonde. Un second
    /// site qui lirait le champ pour en tirer sa propre conclusion est la classe
    /// mika#2158 : correct le jour où il est écrit, muet le jour où la règle
    /// d'ajournement change, et aucune assertion de comportement ne rougirait
    /// puisque les deux sites rendraient les mêmes octets jusque-là.
    ///
    /// **Deux fichiers attendus, et chacun pour sa raison** — un attendu à un
    /// seul nom serait faux et donnerait envie d'élargir l'aiguille :
    ///
    /// - `db/tasks.rs` **rapporte** : il déclare le champ et l'écrit
    ///   (`pending_implement: etat.implement_en_vol`). Il ne répond pas à la
    ///   question « faut-il ajourner ? », il la rend posable — la distinction
    ///   que mika#2305 a dû écrire pour son résolveur de portée
    ///   (« it *produces* the answer »).
    /// - `flight_source.rs` **décide** : le `partition` de
    ///   `finished_implement_flights`, seul site qui en tire une conséquence,
    ///   et la sonde qui publie la quantité. Son `task_to_finished_flight`
    ///   porte en plus un `pending_implement: _` qui ne lit rien — un
    ///   destructuring exhaustif dont le commentaire dit que le champ a déjà
    ///   été consommé en amont.
    ///
    /// Quand ce test rougit : **router le site par `finished_implement_flights`**,
    /// ne pas ajouter de ligne à l'allowlist (doctrine mika#2201).
    #[test]
    fn mika2951_lajournement_a_un_lecteur_unique() {
        /// Aucune exception. Livrée vide, et son vide est épinglé juste en
        /// dessous : une allowlist qui se remplit sans bruit est le mécanisme
        /// que mika#2309 condamne.
        const ALLOWLIST: &[&str] = &[];
        assert!(
            ALLOWLIST.is_empty(),
            "une exception a été ajoutée à ce scan. Ce n'est pas la \
             réparation : router le site par `finished_implement_flights` \
             l'est (mika#2201). Si une exception est vraiment la bonne \
             réponse, ouvrir un ticket qui porte sa raison et le citer ici."
        );

        let corpus = crate::source_scan::production_files();
        assert!(
            corpus.len() >= 100,
            "le corpus de production est anormalement petit ({} fichiers) — \
             l'énumération ne mord plus, et le scan ne regarde presque rien",
            corpus.len()
        );

        let trouves = fichiers_touchant_lajournement(&corpus);

        // Anti-vacuité : un prédicat qui ne trouve plus rien passerait toute
        // égalité d'ensembles qu'on lui demanderait si l'attendu venait à se
        // vider par un refactor.
        assert!(
            !trouves.is_empty(),
            "le scan ne trouve plus aucune mention — les aiguilles ont cessé \
             de mordre (champ renommé, dépouillement cassé), et le filet est \
             décoratif"
        );

        assert_eq!(
            trouves,
            vec!["db/tasks.rs".to_string(), "flight_source.rs".to_string()],
            "l'ensemble des fichiers de production qui touchent \
             `pending_implement` a changé. Lire le doc-comment de ce test \
             avant de toucher à l'attendu."
        );
    }

    /// Le contrôle de bonne foi du scan ci-dessus : sur un corpus
    /// **synthétique** portant un second lecteur, le prédicat doit rougir.
    ///
    /// Sans lui, un scan dont les aiguilles ont cessé de mordre — champ
    /// renommé, dépouillement cassé — resterait vert pour toujours, et son
    /// vert se lirait comme « un seul lecteur » au lieu de « je ne vois plus
    /// rien ». L'anti-vacuité du test précédent couvre le corpus réel ;
    /// celui-ci couvre le prédicat, sur une entrée dont on connaît la réponse.
    ///
    /// Les deux fichiers innocents ne sont pas décoratifs : ils portent les
    /// deux formes qui nomment le champ **sans le toucher** — la prose d'un
    /// doc-comment, et le nom de la sonde, qui contient le nom du champ comme
    /// sous-chaîne. Un scan ancré sur le nom nu les compterait tous les deux.
    #[test]
    fn mika2951_le_scan_rougit_sur_un_second_lecteur() {
        let prose = r#"
/// Cette prose nomme .pending_implement et pending_implement: sans les lire.
fn seulement_de_la_prose() {
    let _ = 1;
}
"#;
        let sonde = r#"
fn seulement_la_sonde() {
    tracing::info!(event = "flight_deferred_pending_implement", deferred = 1);
}
"#;
        let second_lecteur = r#"
fn un_second_lecteur(lignes: Vec<FinishedFlightRow>) -> usize {
    lignes.iter().filter(|l| l.pending_implement).count()
}
"#;
        let corpus = vec![
            ("innocent_prose.rs".to_string(), prose.to_string()),
            ("innocent_sonde.rs".to_string(), sonde.to_string()),
            ("coupable.rs".to_string(), second_lecteur.to_string()),
        ];
        assert_eq!(
            fichiers_touchant_lajournement(&corpus),
            vec!["coupable.rs".to_string()],
            "le prédicat doit voir le second lecteur, et lui seul — une prose \
             qui nomme le champ n'est pas une lecture, et le nom de la sonde \
             contient le nom du champ sans le toucher"
        );
    }
}
