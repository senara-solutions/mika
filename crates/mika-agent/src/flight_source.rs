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
//! de lecture. Le périmètre, c'est `issue_urls` — les sous-tickets du milestone
//! que le manager surveille déjà.
//!
//! # Une panne rend le vide, jamais une erreur
//!
//! [`FlightSource::finished_implement_flights`] ne rend pas de `Result`, et
//! c'est structurel plutôt que conventionnel : un cycle du manager ne doit pas
//! mourir parce qu'une lecture d'évaluation a échoué. Une base injoignable rend
//! `vec![]` — donc un rapport **sans** la section, ce qui se lit comme « aucun
//! vol terminé » — et un WARN nommé, qui est la trace que la différence a
//! existé.

use async_trait::async_trait;
use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::milestone_manager::evaluator::{FinishedFlight, FlightSource};

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
    async fn finished_implement_flights(
        &self,
        since: Option<&str>,
        issue_urls: &[String],
    ) -> Vec<FinishedFlight> {
        let rows = self
            .db
            .list_finished_implement_flights_in_container(
                since.map(str::to_string),
                issue_urls.to_vec(),
            )
            .await;
        match rows {
            Ok(tasks) => tasks.into_iter().map(task_to_finished_flight).collect(),
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "flight_source_read_failed",
                    issue_count = issue_urls.len(),
                    since = since.unwrap_or(""),
                    error = %e,
                    "could not read finished flights — the cycle reports no verdict rather than dying"
                );
                Vec::new()
            }
        }
    }
}

/// `Task` → [`FinishedFlight`], c'est-à-dire la projection sur les sept données
/// que l'évaluateur sait lire.
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
fn task_to_finished_flight(t: crate::task_state::tasks::Task) -> FinishedFlight {
    FinishedFlight {
        task_id: t.id,
        agent_id: t.agent_id,
        status: t.status,
        result: t.result,
        metadata: t.metadata,
        reference_url: t.reference_url,
        completed_at: t.completed_at.or(Some(t.updated_at)),
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
        let got = source
            .finished_implement_flights(
                None,
                &["https://github.com/senara-solutions/mika/issues/2693".to_string()],
            )
            .await;
        assert!(got.is_empty(), "une base vide n'a aucun vol terminé");
    }

    /// Le contrôle négatif du détecteur, côté source : **aucune URL** ne peut
    /// pas rendre « tous les vols ». Sans cette borne, un milestone sans
    /// sous-ticket ferait évaluer l'intégralité de `tasks`.
    #[tokio::test]
    async fn mika2698_aucune_url_ne_rend_jamais_tous_les_vols() {
        let source = DbFlightSource::new(seeded_db());
        let got = source.finished_implement_flights(None, &[]).await;
        assert!(got.is_empty());
    }

    /// Le `COALESCE` de `completed_at` est dans la projection, pas seulement
    /// dans le SQL : un vol dont seul `updated_at` est estampillé garde un
    /// horodatage lisible.
    #[test]
    fn mika2698_un_vol_sans_completed_at_retombe_sur_updated_at() {
        let mut t = task_fixture();
        t.completed_at = None;
        t.updated_at = "2026-10-14T08:00:00Z".into();
        let f = task_to_finished_flight(t);
        assert_eq!(f.completed_at.as_deref(), Some("2026-10-14T08:00:00Z"));

        let mut t = task_fixture();
        t.completed_at = Some("2026-10-14T09:00:00Z".into());
        t.updated_at = "2026-10-14T08:00:00Z".into();
        let f = task_to_finished_flight(t);
        assert_eq!(
            f.completed_at.as_deref(),
            Some("2026-10-14T09:00:00Z"),
            "quand `completed_at` existe, il gagne"
        );
    }

    /// La projection porte les sept données que l'évaluateur lit, et rien de
    /// plus : si une ligne de `tasks` gagne un champ, ce test ne bouge pas.
    #[test]
    fn mika2792_la_projection_porte_les_sept_donnees_lues() {
        let f = task_to_finished_flight(task_fixture());
        assert_eq!(f.task_id, "task-1");
        assert_eq!(f.status, "completed");
        assert!(f.result.unwrap().contains("Outcome: PR_OPENED"));
        assert!(f.metadata.unwrap().contains("claude_pilot"));
        assert_eq!(
            f.reference_url.as_deref(),
            Some("https://github.com/senara-solutions/mika/issues/2693")
        );
    }

    /// La septième donnée est l'agent **de la ligne**, pas celui du handle : la
    /// projection la lit dans `tasks.agent_id`, et c'est elle qui rend l'agent
    /// interrogeable dans la ligne de scan du cycle (mika#2792).
    #[test]
    fn mika2792_la_projection_porte_lagent_du_vol() {
        let mut t = task_fixture();
        t.agent_id = "mika-dev".into();
        assert_eq!(task_to_finished_flight(t).agent_id, "mika-dev");
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
        let got = source
            .finished_implement_flights(None, &[url.to_string()])
            .await;

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
