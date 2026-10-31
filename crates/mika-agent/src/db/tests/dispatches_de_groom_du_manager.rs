//! Ce que « un groom est en cours sur ce ticket » veut dire en SQL
//! (mika#2958, vol 2c).
//!
//! Le lecteur unique est
//! [`crate::db::Database::list_groom_dispatches_in_container`], et cinq de ses
//! clauses sont des décisions, pas des détails :
//!
//! - **l'absence de clause `agent_id`** (mika#2792) : le manager est une tâche
//!   de processus sans agent, et filtrer sur celui du handle emprunté pour la
//!   connexion rend une lecture constamment vide. C'est le défaut qui a rendu
//!   l'évaluateur de vols inerte pendant quatre jours, et il a déjà coûté un
//!   correctif à `list_finished_implement_flights_in_container`. Le test
//!   [`mika2958_une_parente_dun_autre_agent_est_vue`] est ce qui rougit le jour
//!   où quelqu'un réintroduit le filtre ;
//! - **un `IN` sur un ensemble fermé de variantes** (mika#2638), rendu par
//!   `issue_url_variants`, et jamais un `LIKE 'url%'` : le préfixe faisait
//!   apparier `/issues/216` avec `/issues/2161`.
//!   [`mika2958_un_prefixe_nappariee_pas`] est le contrôle négatif de cette
//!   clause ;
//! - **`COALESCE(dispatch_class, 'implement') = 'groom'`** (schéma v34) : une
//!   ligne antérieure à v34 porte `NULL`, qui vaut `'implement'`, donc jamais
//!   `'groom'`. Un dispatch d'implémentation sur la même URL n'est pas un groom
//!   en cours, et le relancer n'est pas ce que ce vol cherche à éviter ;
//! - **deux formes de ligne en `UNION ALL`** (mika#2279) : la parente porte
//!   l'URL et jamais le `process_id`, l'enfant porte le pgid et jamais l'URL.
//!   Sans la seconde branche, un enfant de groom vivant sous une parente
//!   **annulée** lirait comme « aucun groom » — la collision même que le motif
//!   d'AC2 nomme ;
//! - **le statut sort brut**, et le filtre terminal est en Rust, chez
//!   `crate::groom_in_flight`. Les tests d'ici lisent donc le statut tel quel :
//!   c'est le contrat de la couche, pas une commodité.
//!
//! Et une sixième, qui est un contrôle négatif de détecteur : **une liste
//! d'URLs vide rend `vec![]`** avant toute requête. « Aucun candidat » ne doit
//! pas faire lire la table entière.

use super::*;

const AGENT: &str = "mika-dev";

/// Un second agent du même conteneur. Le manager ne lui appartient pas, et
/// aucune des deux lectures ne doit en dépendre.
const AUTRE_AGENT: &str = "mika-qa";

/// Le ticket de référence, et le ticket dont le numéro **commence par** le
/// sien : c'est tout l'intérêt de l'ensemble fermé de variantes.
const TICKET: u32 = 216;
const TICKET_PREFIXE: u32 = 2161;

fn issue_url(n: u32) -> String {
    format!("https://github.com/senara-solutions/mika/issues/{n}")
}

/// L'URL telle que la porte la parente d'un dispatch de **groom** : la variante
/// `?phase=groom` que `issue_url_variants` reconnaît sans l'énumérer au site
/// d'appel.
fn issue_url_phase_groom(n: u32) -> String {
    format!("{}?phase=groom", issue_url(n))
}

/// Une base où les **deux** agents du conteneur existent — `tasks.agent_id`
/// porte une contrainte de clé étrangère, et c'est tout ce que cet
/// enregistrement sert. Qui possède la tâche n'entre dans aucune des lectures
/// de ce fichier : c'est précisément la propriété testée.
fn db_du_conteneur() -> Database {
    let base = db();
    for agent in [AGENT, AUTRE_AGENT] {
        base.register_agent(agent, agent, &format!("/tmp/{agent}"))
            .expect("l'agent doit être enregistré");
    }
    base
}

/// Seede une parente de groom sur `reference_url`, dans le statut demandé, et
/// rend son id. `groom_parent` porte déjà la topologie réelle
/// (`trigger_type='manual'`, `action_type='none'`, `dispatch_class='groom'`).
fn seed_parente(db: &Database, agent_id: &str, reference_url: &str, status: &str) -> String {
    let id = db
        .create_task(&groom_parent(agent_id, reference_url))
        .expect("la parente doit être créée");
    db.update_task_status(&id, status)
        .expect("le statut doit être posé");
    id
}

/// Lit les dispatches de groom portant `urls`, et rend les couples
/// `(url appariée, statut brut)` triés — la couche ne promet pas d'ordre.
fn lire(db: &Database, urls: &[String]) -> Vec<(String, String)> {
    let mut lignes: Vec<(String, String)> = db
        .list_groom_dispatches_in_container(urls)
        .expect("la lecture doit répondre")
        .into_iter()
        .map(|r| (r.issue_url, r.status))
        .collect();
    lignes.sort();
    lignes
}

/// Une parente de groom non terminale sur l'URL : une ligne, et le statut sort
/// **brut**.
#[test]
fn mika2958_une_parente_de_groom_vivante_est_vue_avec_son_statut_brut() {
    let db = db_du_conteneur();
    seed_parente(&db, AGENT, &issue_url(TICKET), "in_progress");

    assert_eq!(
        lire(&db, &[issue_url(TICKET)]),
        vec![(issue_url(TICKET), "in_progress".to_string())],
        "la parente de groom doit être rendue, et son statut non interprété"
    );
}

/// La même parente, portant la variante `?phase=groom` : appariée par
/// l'ensemble fermé, sur une demande qui ne nomme que l'URL nue.
#[test]
fn mika2958_la_variante_phase_groom_est_appariee() {
    let db = db_du_conteneur();
    seed_parente(&db, AGENT, &issue_url_phase_groom(TICKET), "pending");

    assert_eq!(
        lire(&db, &[issue_url(TICKET)]),
        vec![(issue_url_phase_groom(TICKET), "pending".to_string())],
        "la variante `?phase=groom` appartient à l'ensemble fermé de la demande"
    );
}

/// **Contrôle négatif de préfixe** (mika#2638) : `/issues/2161` n'apparie pas
/// une demande sur `/issues/216`. C'est l'assertion qui rougit le jour où
/// quelqu'un remplace l'`IN` par un `LIKE 'url%'`.
#[test]
fn mika2958_un_prefixe_nappariee_pas() {
    let db = db_du_conteneur();
    seed_parente(&db, AGENT, &issue_url(TICKET_PREFIXE), "in_progress");

    assert!(
        lire(&db, &[issue_url(TICKET)]).is_empty(),
        "un numéro dont le ticket demandé est un préfixe n'est pas ce ticket"
    );
}

/// Une parente **annulée** portant un enfant de groom encore vivant : la
/// branche (b) rend l'enfant (mika#2279).
///
/// Les **deux** lignes sortent, et c'est le `UNION ALL` qui est voulu : la
/// parente par la branche (a) avec son `cancelled`, l'enfant par la branche (b)
/// avec son statut vivant. Aucune des deux n'est écartée ici — cette couche
/// rapporte. C'est le filtre terminal de `crate::groom_in_flight` qui ne retient
/// que l'enfant, et c'est sa déduplication en `BTreeSet` qui ramène le ticket à
/// un numéro. Sans la branche (b), la seule ligne rendue serait `cancelled`,
/// donc terminale, donc « aucun groom en cours » sur un ticket qu'un pilote est
/// en train de grommer.
#[test]
fn mika2958_une_parente_annulee_portant_un_enfant_vivant_est_vue() {
    let db = db_du_conteneur();
    let parente = seed_parente(&db, AGENT, &issue_url(TICKET), "cancelled");
    db.create_task(&groom_callback(AGENT, &parente, "groom"))
        .expect("l'enfant de groom doit être créé");

    let vu = lire(&db, &[issue_url(TICKET)]);
    assert_eq!(
        vu,
        vec![
            (issue_url(TICKET), "cancelled".to_string()),
            (issue_url(TICKET), "pending".to_string()),
        ],
        "les deux branches doivent parler, et la couche ne doit rien trancher"
    );
    assert!(
        vu.iter()
            .any(|(_, statut)| !crate::task_state::tasks::is_terminal_task_status(statut)),
        "la lecture doit porter au moins un statut non terminal : c'est ce que \
         le consommateur lira comme « un groom est en cours »"
    );
}

/// Une parente de classe `implement` sur la même URL : **aucune** ligne. Un vol
/// d'implémentation n'est pas un groom en cours.
#[test]
fn mika2958_une_parente_dimplementation_nest_pas_un_groom() {
    let db = db_du_conteneur();
    let id = db
        .create_task(&NewTask {
            dispatch_class: Some("implement".to_string()),
            ..groom_parent(AGENT, &issue_url(TICKET))
        })
        .expect("la parente doit être créée");
    db.update_task_status(&id, "in_progress").unwrap();

    assert!(
        lire(&db, &[issue_url(TICKET)]).is_empty(),
        "la classe est lue par COALESCE(dispatch_class, 'implement')"
    );
}

/// **L'assertion de container-scope** : une parente de groom portée par un
/// *autre* `agent_id` est vue. Elle rougit le jour où quelqu'un réintroduit un
/// filtre `agent_id` dans la requête (mika#2792).
#[test]
fn mika2958_une_parente_dun_autre_agent_est_vue() {
    let db = db_du_conteneur();
    seed_parente(&db, AUTRE_AGENT, &issue_url(TICKET), "in_progress");

    assert_eq!(
        lire(&db, &[issue_url(TICKET)]),
        vec![(issue_url(TICKET), "in_progress".to_string())],
        "le scope est le conteneur, qui n'a qu'une base — pas l'agent du handle"
    );
}

/// `issue_urls` vide ⇒ `Ok(vec![])`, et la table n'est pas lue : le contrôle
/// négatif du détecteur.
#[test]
fn mika2958_une_demande_vide_ne_lit_rien() {
    let db = db_du_conteneur();
    seed_parente(&db, AGENT, &issue_url(TICKET), "in_progress");

    assert!(
        db.list_groom_dispatches_in_container(&[])
            .expect("la lecture doit répondre")
            .is_empty(),
        "aucun candidat ne doit jamais faire lire la table entière"
    );
}
