//! L'implémentation base de données de
//! [`crate::milestone_manager::GroomInFlightSource`] (mika#2958, vol 2c).
//!
//! # Pourquoi ce fichier vit **hors** de `milestone_manager/`
//!
//! Même idiome, et même raison, que [`crate::flight_source`] — dont le
//! module-doc porte l'arbitrage complet : `no_dispatch_test.rs` grep **chaque
//! `.rs` de l'arbre du module** à la recherche des jetons d'autorité d'écriture,
//! et y faire entrer du SQL et un handle de base ferait passer sous ce grep du
//! code qui n'a rien à y faire, en rendant le prochain élargissement du grep
//! inoffensif par accident. Le module reste ce qu'il dit être : lecture,
//! jugement, rapport.
//!
//! # Ce que la source lit, et ce qu'elle ne lit pas
//!
//! Elle lit `tasks` — rien d'autre. Pas de `gh`, pas d'HTTP : « un groom est en
//! cours » est un fait du **moteur**, et le moteur le tient en base. Lire l'état
//! du ticket sur GitHub répondrait à une autre question — *a-t-il déjà un
//! plan ?* — qui est celle de `GroomCandidate::deja_groome` et qui a déjà son
//! lecteur (mika#2201).
//!
//! # Ce qu'elle interroge : le conteneur, pas l'agent du handle (mika#2792)
//!
//! Le [`AsyncDatabase`] passé au constructeur ne sert que de **connexion**. Son
//! `agent_id` n'entre dans aucune requête d'ici : la lecture passe par
//! [`crate::db::Database::list_groom_dispatches_in_container`], dont le nom
//! porte l'invariant. Le manager est une tâche de **processus** sans agent, les
//! dispatches de groom sont des tâches de `mika-dev`, et filtrer sur l'agent du
//! handle rendrait une lecture constamment vide — le défaut de mika#2792, qui a
//! coûté quarante-trois vols non jugés en quatre jours à la source voisine, avec
//! un vide parfaitement lisible comme « aucun groom en cours ».
//!
//! Qui construit la source choisit donc **une connexion**, jamais un périmètre
//! de lecture.
//!
//! # Trois décisions vivent ici, et une seule fois chacune
//!
//! **1. Le filtre terminal.** La requête rend le statut brut (voir son
//! doc-comment) ; c'est ici qu'un statut terminal est écarté. En SQL, il aurait
//! rendu « un groom est mort ici » et « aucun groom n'a jamais eu lieu ici » en
//! octets identiques (classe mika#2205), et aurait mis le même filtre à deux
//! endroits.
//!
//! **2. La retraduction URL → numéro.** Par la table que la source construit
//! **avant** d'interroger, depuis les [`GroomIssue`] qu'on lui a données, et
//! jamais par un `parse` sur la queue de l'URL rendue : un décodeur d'URL de
//! plus est un décodeur de plus à faire mentir le jour où une variante de chemin
//! apparaît. Une URL rendue que la table ne connaît pas est **ignorée** plutôt
//! que devinée.
//!
//! **3. La déduplication.** Deux lignes peuvent décrire le même ticket — c'est
//! le `UNION ALL` de la requête, et la parente annulée portant un enfant vivant
//! en est la forme réelle (mika#2279). Le `BTreeSet` les ramène à un numéro.
//!
//! # Une panne rend `Unreadable`, jamais une erreur — ni un vide
//!
//! [`GroomInFlightSource::grooms_in_flight`] ne rend pas de `Result` : un cycle
//! du manager ne doit pas mourir parce qu'une lecture a échoué. Mais il ne rend
//! pas non plus un ensemble vide, car le consommateur **envoie** et ne doit pas
//! envoyer sur la foi d'un vide qu'il n'a pas mesuré. La direction de ce
//! fail-safe — et pourquoi elle est l'inverse de celle de [`crate::live_pilot`] —
//! est écrite au doc-comment de [`GroomInFlightReading`], avec son coût.
//!
//! La trace est le WARN nommé `groom_in_flight_read_failed`, un nom pour cette
//! population seule : `flight_source_read_failed` couvre une autre lecture, et
//! deux populations sous un seul nom ne sont pas soustractibles (mika#2205).

use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::milestone_manager::{GroomInFlightReading, GroomInFlightSource, GroomIssue};
use crate::task_state::tasks::{is_terminal_task_status, issue_url_variants};

/// La source de production : les grooms en vol lus dans `tasks`.
pub struct DbGroomInFlightSource {
    db: AsyncDatabase,
}

impl DbGroomInFlightSource {
    pub fn new(db: AsyncDatabase) -> Self {
        Self { db }
    }
}

#[async_trait]
impl GroomInFlightSource for DbGroomInFlightSource {
    async fn grooms_in_flight(&self, issues: &[GroomIssue]) -> GroomInFlightReading {
        if issues.is_empty() {
            // Personne à regarder : la réponse est **mesurée**, et elle est
            // vide. Rendre `Unreadable` ferait sauter une relève qui n'a aucun
            // candidat — sans effet aujourd'hui, et faux le jour où le motif
            // remonte au rapport.
            return GroomInFlightReading::Read(BTreeSet::new());
        }

        // La table de retraduction, construite depuis ce qu'on nous a donné et
        // couvrant **les deux variantes** : la requête apparie sur l'ensemble
        // fermé, donc elle peut rendre l'une ou l'autre.
        let par_url: HashMap<String, u64> = issues
            .iter()
            .flat_map(|issue| {
                issue_url_variants(&issue.url)
                    .into_iter()
                    .map(move |variante| (variante, issue.number))
            })
            .collect();

        let urls: Vec<String> = issues.iter().map(|i| i.url.clone()).collect();
        let demandees = urls.len();

        match self.db.list_groom_dispatches_in_container(urls).await {
            Ok(lignes) => GroomInFlightReading::Read(
                lignes
                    .into_iter()
                    .filter(|ligne| !is_terminal_task_status(&ligne.status))
                    .filter_map(|ligne| par_url.get(&ligne.issue_url).copied())
                    .collect(),
            ),
            Err(e) => {
                warn!(
                    target: "mika::milestone_manager",
                    event = "groom_in_flight_read_failed",
                    requested = demandees,
                    error = %e,
                    "could not read grooms in flight — every candidate is skipped rather than sent blind"
                );
                GroomInFlightReading::Unreadable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::tests::{groom_callback, groom_parent};
    use crate::task_state::tasks::NewTask;

    /// L'agent chez qui vivent les dispatches de groom.
    const AGENT: &str = "mika-dev";

    /// L'agent du **handle**, et il est délibérément différent : c'est celui que
    /// le serveur prête à la source (sur gentux `mika-test`). Les deux coïncidant
    /// toujours, la question de mika#2792 ne se pose jamais.
    const AGENT_DU_HANDLE: &str = "mika-test";

    const TICKET: u64 = 216;
    const TICKET_PREFIXE: u64 = 2161;

    fn issue_url(n: u64) -> String {
        format!("https://github.com/senara-solutions/mika/issues/{n}")
    }

    fn issue(n: u64) -> GroomIssue {
        GroomIssue {
            number: n,
            url: issue_url(n),
        }
    }

    /// Une base de conteneur où l'agent dispatcheur existe — `tasks.agent_id`
    /// porte une clé étrangère, et c'est tout le rôle de cet enregistrement.
    fn db_du_conteneur() -> Database {
        let db = Database::open_in_memory().expect("la base doit s'ouvrir");
        db.register_agent(AGENT, AGENT, "/tmp/mika-dev")
            .expect("l'agent dispatcheur doit être enregistré");
        db
    }

    /// La source telle que la production la bâtit : sur le handle d'un **autre**
    /// agent que celui qui porte les tâches.
    fn source(db: Database) -> DbGroomInFlightSource {
        DbGroomInFlightSource::new(AsyncDatabase::new_with_agent(db, AGENT_DU_HANDLE))
    }

    /// Sème une parente de dispatch sur `reference_url`, de la classe et dans le
    /// statut demandés, et rend son id.
    fn seme_parente(db: &Database, reference_url: &str, classe: &str, status: &str) -> String {
        let tache = NewTask {
            dispatch_class: Some(classe.to_string()),
            ..groom_parent(AGENT, reference_url)
        };
        let id = db.create_task(&tache).expect("la parente doit être créée");
        db.update_task_status(&id, status)
            .expect("le statut doit être posé");
        id
    }

    /// Les numéros d'une lecture **faite**. Rougit sur `Unreadable` : une base en
    /// mémoire répond, donc l'illisible y serait un défaut, pas un cas.
    fn lus(lecture: &GroomInFlightReading) -> Vec<u64> {
        match lecture {
            GroomInFlightReading::Read(numeros) => numeros.iter().copied().collect(),
            GroomInFlightReading::Unreadable => {
                panic!("la lecture devait aboutir : une base en mémoire répond")
            }
        }
    }

    /// Un groom non terminal rend son **numéro**, et le prédicat du consommateur
    /// dit oui.
    ///
    /// C'est aussi **le test qui aurait rougi sur mika#2792** : la tâche vit chez
    /// `mika-dev`, la source est bâtie sur le handle de `mika-test`, et le vol
    /// doit être trouvé quand même.
    #[tokio::test]
    async fn mika2958_un_groom_vivant_rend_son_numero_quel_que_soit_lagent_du_handle() {
        let db = db_du_conteneur();
        seme_parente(&db, &issue_url(TICKET), "groom", "in_progress");

        let lecture = source(db).grooms_in_flight(&[issue(TICKET)]).await;
        assert_eq!(
            lus(&lecture),
            vec![TICKET],
            "le scope est le conteneur, pas l'agent du handle : {lecture:?}"
        );
        assert!(
            lecture.carries_groom(TICKET),
            "le prédicat du consommateur doit dire oui : {lecture:?}"
        );
    }

    /// La variante `?phase=groom` apparie, et rend le numéro du ticket **nu**
    /// qu'on a demandé : c'est la table de retraduction qui couvre les deux
    /// formes, et jamais un `parse` sur la queue de l'URL rendue.
    #[tokio::test]
    async fn mika2958_la_variante_phase_groom_rend_le_bon_numero() {
        let db = db_du_conteneur();
        let variante = format!("{}?phase=groom", issue_url(TICKET));
        seme_parente(&db, &variante, "groom", "pending");

        assert_eq!(
            lus(&source(db).grooms_in_flight(&[issue(TICKET)]).await),
            vec![TICKET]
        );
    }

    /// **Le filtre terminal vit ici, et une seule fois.** Un groom `completed`
    /// n'est pas en vol — et son contrôle positif est dans le même test : même
    /// semis, le seul statut en différence, sans quoi « rien en vol » serait
    /// aussi rendu par une requête qui n'apparie plus rien (mika#2205).
    #[tokio::test]
    async fn mika2958_un_groom_terminal_nest_pas_en_vol_et_son_controle_positif() {
        let db = db_du_conteneur();
        let id = seme_parente(&db, &issue_url(TICKET), "groom", "completed");
        let handle = AsyncDatabase::new_with_agent(db, AGENT_DU_HANDLE);
        let source = DbGroomInFlightSource::new(handle.clone());

        assert!(
            lus(&source.grooms_in_flight(&[issue(TICKET)]).await).is_empty(),
            "un statut terminal est écarté en Rust, pas en SQL"
        );

        handle
            .update_task_status(&id, "in_progress")
            .await
            .expect("le statut doit être posé");
        assert_eq!(
            lus(&source.grooms_in_flight(&[issue(TICKET)]).await),
            vec![TICKET],
            "contrôle positif : c'est le statut qui retient, pas l'appariement"
        );
    }

    /// Une parente **annulée** portant un enfant de groom vivant est en vol
    /// (mika#2279), et le ticket ne sort **qu'une fois** : les deux branches du
    /// `UNION ALL` parlent, le `BTreeSet` les réunit.
    ///
    /// Sans la branche (b), la seule ligne serait `cancelled`, donc terminale,
    /// donc « aucun groom en cours » sur un ticket qu'un pilote est en train de
    /// grommer — la collision que nomme AC2.
    #[tokio::test]
    async fn mika2958_une_parente_annulee_portant_un_enfant_vivant_sort_une_seule_fois() {
        let db = db_du_conteneur();
        let parente = seme_parente(&db, &issue_url(TICKET), "groom", "cancelled");
        db.create_task(&groom_callback(AGENT, &parente, "groom"))
            .expect("l'enfant de groom doit être créé");

        assert_eq!(
            lus(&source(db).grooms_in_flight(&[issue(TICKET)]).await),
            vec![TICKET],
            "l'enfant vivant porte le vol, et la déduplication le ramène à un numéro"
        );
    }

    /// Un dispatch d'**implémentation** n'est pas un groom en cours, et
    /// `/issues/2161` n'est pas `/issues/216` (mika#2638).
    #[tokio::test]
    async fn mika2958_ni_limplementation_ni_un_prefixe_ne_sont_un_groom() {
        let db = db_du_conteneur();
        seme_parente(&db, &issue_url(TICKET), "implement", "in_progress");
        seme_parente(&db, &issue_url(TICKET_PREFIXE), "groom", "in_progress");

        assert!(
            lus(&source(db).grooms_in_flight(&[issue(TICKET)]).await).is_empty(),
            "ni la classe ni le préfixe ne doivent apparier"
        );
    }

    /// Toute la relève est lue d'une **seule** demande, et chaque ticket en vol
    /// ressort avec son propre numéro.
    #[tokio::test]
    async fn mika2958_plusieurs_tickets_sont_lus_dune_seule_demande() {
        let db = db_du_conteneur();
        seme_parente(&db, &issue_url(216), "groom", "in_progress");
        seme_parente(&db, &issue_url(2958), "groom", "pending");
        seme_parente(&db, &issue_url(2700), "groom", "completed");

        let lecture = source(db)
            .grooms_in_flight(&[issue(216), issue(2958), issue(2700)])
            .await;
        assert_eq!(
            lus(&lecture),
            vec![216, 2958],
            "les deux vivants, et pas le terminal : {lecture:?}"
        );
    }

    /// **Aucun candidat ⇒ `Read(∅)`, jamais `Unreadable`.** « Personne à
    /// regarder » n'est pas « je n'ai pas pu regarder » : l'illisible ici ferait
    /// sauter une relève vide, ce qui est sans effet aujourd'hui et faux le jour
    /// où le motif remonte au rapport.
    #[tokio::test]
    async fn mika2958_une_demande_vide_est_une_lecture_faite() {
        let db = db_du_conteneur();
        seme_parente(&db, &issue_url(TICKET), "groom", "in_progress");

        let lecture = source(db).grooms_in_flight(&[]).await;
        assert_eq!(lecture, GroomInFlightReading::Read(BTreeSet::new()));
        assert!(
            !lecture.carries_groom(TICKET),
            "une lecture faite ne porte personne : {lecture:?}"
        );
    }

    /// **La direction du fail-safe**, sur une base réellement morte : le fil de
    /// la base est arrêté, la lecture échoue, et la source rend `Unreadable` —
    /// donc `carries_groom` dit oui pour tout le monde — au lieu d'un vide qui se
    /// lirait comme une mesure.
    ///
    /// Le contrôle positif est dans le même test, et il passe **avant** : la
    /// même source, le même semis, le fil encore vivant, et une lecture faite.
    /// Sans lui, `Unreadable` serait aussi rendu par une source qui n'interroge
    /// jamais rien. Le `shutdown` est la seule différence entre les deux
    /// moitiés.
    #[tokio::test]
    async fn mika2958_une_base_morte_rend_illisible_et_non_un_vide() {
        let db = db_du_conteneur();
        seme_parente(&db, &issue_url(TICKET), "groom", "in_progress");

        let handle = AsyncDatabase::new_with_agent(db, AGENT_DU_HANDLE);
        let source = DbGroomInFlightSource::new(handle.clone());

        // Contrôle positif d'abord, sur le fil vivant : sans lui, `Unreadable`
        // serait aussi rendu par une source qui n'interroge jamais rien.
        assert_eq!(
            lus(&source.grooms_in_flight(&[issue(TICKET)]).await),
            vec![TICKET],
            "prémisse : le même semis, un fil vivant, une lecture faite"
        );

        handle.shutdown();
        let lecture = source.grooms_in_flight(&[issue(TICKET)]).await;
        assert_eq!(
            lecture,
            GroomInFlightReading::Unreadable,
            "une panne de lecture n'est pas un ensemble vide"
        );
        assert!(
            lecture.carries_groom(TICKET),
            "le doute fait sauter le candidat : ce consommateur envoie"
        );
    }
}
