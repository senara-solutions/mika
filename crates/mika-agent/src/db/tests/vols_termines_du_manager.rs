//! Ce que « les vols d'implémentation terminés depuis le cycle précédent »
//! veut dire en SQL (mika#2698 vol 1, élargi par mika#2921).
//!
//! Le lecteur unique est
//! [`crate::db::Database::list_finished_implement_flights_in_container`], et
//! cinq de ses clauses sont des décisions, pas des détails :
//!
//! - **l'absence de clause `agent_id`** (mika#2792) : le manager n'a pas
//!   d'agent, et filtrer sur celui du handle emprunté pour la connexion est
//!   exactement ce qui a rendu l'évaluateur inerte pendant quatre jours. Le
//!   scope est le conteneur, qui n'a qu'une base ;
//! - **un motif `LIKE` par dépôt** (mika#2921), `https://github.com/<dépôt>/issues/%`
//!   avec `ESCAPE '\'`. Le périmètre est le **dépôt**, plus le ticket : un vol
//!   est jugé quel que soit son milestone, pourvu que son ticket vive sur un
//!   dépôt sous contrôle. Le `/issues/` littéral avant le joker est ce qui
//!   empêche `mika` de capturer `mika-cloud`, et l'échappement est ce qui
//!   empêche un `_` d'élargir le périmètre en silence ;
//! - **`COALESCE(completed_at, updated_at)`** comme borne temporelle, parce
//!   qu'un vol mort par `failed` peut n'avoir que son `updated_at` : avec
//!   `completed_at > ?` seul, `NULL > '…'` rend `NULL`, et ce sont exactement
//!   les vols morts — ceux de l'AC3 — qui disparaîtraient en silence ;
//! - **le filtre de statut terminal en Rust**, pas en SQL, parce que la règle
//!   « un statut inconnu n'est pas terminal » de `is_terminal_task_status` n'a
//!   pas d'expression dans ce dialecte, et qu'une troisième copie de la liste
//!   des statuts divergerait ;
//! - **une liste de dépôts vide rend `vec![]`** avant toute requête : c'est le
//!   contrôle négatif du détecteur. « Aucun dépôt sous contrôle » ne doit pas
//!   faire évaluer la table entière.
//!
//! Depuis mika#2886 la lecture rend **deux lignes de `tasks` par vol** : la
//! parente, et le `result` du dernier enfant pilote terminé. Les deux tests
//! `mika2886_*` en bas de ce fichier pinnent les deux arbitrages de cette
//! seconde requête (dernier enfant, population des labels) ; la forme réelle
//! que la production produit, elle, est semée de bout en bout jusqu'au verdict
//! par l'AC1 de mika#2886 — `crate::flight_source`,
//! `mika2886_la_forme_reelle_rend_un_verdict_de_pr_ouverte`.

use super::*;

const REPO: &str = "senara-solutions/mika";

/// Un second dépôt sous contrôle, dont le nom **commence par celui du premier**
/// — c'est tout l'intérêt du `/issues/` littéral dans le motif.
const REPO_VOISIN: &str = "senara-solutions/mika-cloud";

/// Un dépôt **hors** de la liste. C'est la forme de claude-pilot#232, le vol
/// mesuré le 2026-10-26 qui sort du périmètre jugé (AC2 de mika#2921).
const REPO_DEHORS: &str = "senara-solutions/claude-pilot";

/// Le ticket de référence, et un second sur le même dépôt — deux milestones
/// différents en production, une seule et même population ici.
const TICKET: u32 = 2693;
const AUTRE_TICKET: u32 = 26930;

fn issue_url(n: u32) -> String {
    issue_url_sur(REPO, n)
}

fn issue_url_sur(repo: &str, n: u32) -> String {
    format!("https://github.com/{repo}/issues/{n}")
}

/// Un vol d'implémentation portant `reference_url` : la topologie d'un dispatch
/// `implement`, c'est-à-dire [`groom_parent`] avec sa classe corrigée.
fn vol(agent_id: &str, reference_url: &str) -> NewTask {
    NewTask {
        dispatch_class: Some("implement".to_string()),
        ..groom_parent(agent_id, reference_url)
    }
}

/// Seede un vol **terminé par une PR** sur `reference_url`, et rend son id.
fn seed_vol_termine(db: &Database, agent_id: &str, reference_url: &str) -> String {
    let id = db
        .create_task(&vol(agent_id, reference_url))
        .expect("le vol doit être créé");
    db.update_task_completed(&id, agent_id, Some("Outcome: PR_OPENED\nTurns: 83\n"))
        .expect("la complétion doit réussir");
    id
}

/// Lit les vols du dépôt de référence. **Plus d'agent à passer** (mika#2792) :
/// c'est le conteneur qui est interrogé, et l'absence de ce paramètre est la
/// forme même du correctif.
fn lire(db: &Database, since: Option<&str>) -> Vec<String> {
    lire_sur(db, since, &[REPO.to_string()])
}

/// Idem, sur une liste de dépôts choisie par l'appelant.
fn lire_sur(db: &Database, since: Option<&str>, repos: &[String]) -> Vec<String> {
    db.list_finished_implement_flights_in_container(since, repos)
        .expect("la lecture doit répondre")
        .into_iter()
        .map(|r| r.flight.id)
        .collect()
}

/// Seede un enfant de dispatch **terminé** sous `parente_id`, portant `label`
/// et `result`, et rend son id. Le gabarit est [`groom_callback`], qui porte
/// déjà la topologie réelle d'un enfant de pilote : `trigger_type: "callback"`,
/// `parent_task_id` renseigné, `depth: 1`, et surtout `reference_url: None` —
/// c'est cette absence qui rend l'enfant invisible à la requête 1.
fn seed_enfant(
    db: &Database,
    agent_id: &str,
    parente_id: &str,
    label: &str,
    result: &str,
) -> String {
    let id = db
        .create_task(&NewTask {
            label: label.to_string(),
            ..groom_callback(agent_id, parente_id, "implement")
        })
        .expect("l'enfant doit être créé");
    db.update_task_completed(&id, agent_id, Some(result))
        .expect("la complétion de l'enfant doit réussir");
    id
}

/// Les lignes elles-mêmes, et non leurs seuls ids : les tests de mika#2886
/// portent sur `pilot_result`.
fn lire_lignes(db: &Database) -> Vec<crate::db::tasks::FinishedFlightRow> {
    db.list_finished_implement_flights_in_container(None, &[REPO.to_string()])
        .expect("la lecture doit répondre")
}

/// Enregistre un agent pour que ses lignes de `tasks` puissent exister :
/// `tasks.agent_id` est une FK vers `agents(id)`.
fn enregistrer(db: &Database, agent_id: &str) {
    db.register_agent(agent_id, agent_id, &format!("/tmp/{agent_id}"))
        .expect("l'agent doit être enregistré");
}

/// Le suffixe `?phase=groom` est couvert par le `%` terminal du motif.
/// mika#2698 l'obtenait en énumérant un ensemble clos de variantes ; mika#2921
/// l'obtient par construction, et ce test est ce qui dit que le remplacement
/// n'a rien perdu.
#[test]
fn mika2698_la_variante_phase_groom_est_appariee() {
    let db = db();
    let id = seed_vol_termine(&db, "mika", &format!("{}?phase=groom", issue_url(TICKET)));
    assert_eq!(lire(&db, None), vec![id]);
}

/// **AC1 — tout vol du dépôt, quel que soit son ticket.** Deux vols sur deux
/// tickets du même dépôt sortent tous les deux. Avant mika#2921 la lecture
/// visait une liste de tickets (les sous-issues du milestone cible), donc le
/// second aurait été invisible : c'est exactement le silence que ce ticket
/// répare.
#[test]
fn mika2921_deux_tickets_du_meme_depot_sortent_tous_les_deux() {
    let db = db();
    let un = seed_vol_termine(&db, "mika", &issue_url(TICKET));
    let deux = seed_vol_termine(&db, "mika", &issue_url(AUTRE_TICKET));

    let vu = lire(&db, None);
    assert_eq!(
        vu.len(),
        2,
        "le périmètre est le dépôt, pas le ticket — obtenu {vu:?}"
    );
    assert!(vu.contains(&un) && vu.contains(&deux));
}

/// **Un dépôt n'est pas un préfixe d'un autre.** `senara-solutions/mika` ne
/// capture pas `senara-solutions/mika-cloud` : le motif exige `/issues/` juste
/// après le nom du dépôt. Avec son contrôle positif, sans quoi le test ne
/// distinguerait pas « le motif délimite » de « la fixture n'enregistre rien ».
#[test]
fn mika2921_un_depot_nest_pas_le_prefixe_dun_autre() {
    let db = db();
    let voisin = seed_vol_termine(&db, "mika", &issue_url_sur(REPO_VOISIN, TICKET));
    assert!(
        lire(&db, None).is_empty(),
        "un vol de {REPO_VOISIN} a été lu comme un vol de {REPO}"
    );

    // Contrôle positif : la fixture est bien enregistrée, sous son dépôt.
    assert_eq!(
        lire_sur(&db, None, &[REPO_VOISIN.to_string()]),
        vec![voisin]
    );
}

/// **L'échappement du `_`, et pourquoi il porte.** Sans `ESCAPE '\'`, le `_` de
/// `senara-solutions/mika_cloud` serait un joker d'un caractère et le motif
/// reconnaîtrait `mika-cloud` — un élargissement silencieux du périmètre, soit
/// précisément ce que « jamais au-delà » interdit. Le dépôt interrogé n'existe
/// pas sur GitHub : c'est la **forme** du prédicat qui est mesurée.
#[test]
fn mika2921_un_souligne_dans_un_nom_de_depot_nest_pas_un_joker() {
    let db = db();
    let reel = seed_vol_termine(&db, "mika", &issue_url_sur(REPO_VOISIN, TICKET));

    assert!(
        lire_sur(&db, None, &["senara-solutions/mika_cloud".to_string()]).is_empty(),
        "le `_` a joué le joker : {REPO_VOISIN} est entré dans le périmètre"
    );

    // Contrôle positif : la même fixture, sous son nom exact, est bien lue.
    assert_eq!(lire_sur(&db, None, &[REPO_VOISIN.to_string()]), vec![reel]);
}

/// **Plusieurs dépôts sous contrôle sont lus d'un seul coup** — les motifs sont
/// joints par `OR`, et l'ordre rendu reste chronologique.
#[test]
fn mika2921_la_liste_de_depots_est_lue_dun_seul_coup() {
    let db = db();
    let ici = seed_vol_termine(&db, "mika", &issue_url(TICKET));
    let la = seed_vol_termine(&db, "mika", &issue_url_sur(REPO_VOISIN, TICKET));

    let vu = lire_sur(&db, None, &[REPO.to_string(), REPO_VOISIN.to_string()]);
    assert_eq!(vu, vec![ici, la], "les deux dépôts sont dans le périmètre");
}

/// **La clause porteuse.** Un vol mort par `failed` sans `completed_at` est vu,
/// parce que la borne est `COALESCE(completed_at, updated_at)`. Avec
/// `completed_at` seul, cette ligne rendrait `NULL` à la comparaison et
/// sortirait du résultat — c'est-à-dire que les morts au plafond, le cas de
/// l'AC3, seraient invisibles.
#[test]
fn mika2698_un_vol_mort_sans_completed_at_est_vu_par_le_coalesce() {
    let db = db();
    let id = db
        .create_task(&vol("mika", &issue_url(TICKET)))
        .expect("le vol doit être créé");
    // `update_task_status` est un écrivain de production, et il n'écrit *pas*
    // `completed_at` : c'est par ce chemin que la colonne reste `NULL` sur un
    // vol pourtant terminé.
    db.update_task_status(&id, "failed")
        .expect("le passage en failed doit réussir");

    let vu = db
        .list_finished_implement_flights_in_container(None, &[REPO.to_string()])
        .expect("la lecture doit répondre");
    assert_eq!(vu.len(), 1, "le vol mort sans `completed_at` doit être vu");
    assert_eq!(vu[0].flight.id, id);
    assert!(
        vu[0].flight.completed_at.is_none(),
        "la fixture doit bien porter `completed_at = NULL`, sinon ce test ne \
         pose pas sa question"
    );

    // Et la borne temporelle le voit aussi : c'est `updated_at` qui décide.
    let bound = crate::timestamp::now_minus(chrono::Duration::hours(1));
    assert_eq!(
        lire(&db, Some(&bound)),
        vec![id.clone()],
        "borné à une heure, le vol mort reste dans la fenêtre"
    );
}

/// La borne `since` exclut un vol plus ancien : « depuis le cycle précédent »
/// est une borne, pas une décoration.
#[test]
fn mika2698_la_borne_since_exclut_un_vol_plus_ancien() {
    let db = db();
    let ancien = seed_vol_termine(&db, "mika", &issue_url(TICKET));
    db.backdate_task_completed_at(&ancien, 7_200)
        .expect("le recul doit réussir");
    db.backdate_task_updated_at(&ancien, 7_200)
        .expect("le recul doit réussir");
    let recent = seed_vol_termine(&db, "mika", &issue_url(TICKET));

    assert_eq!(
        lire(&db, None).len(),
        2,
        "sans borne, les deux vols sont lus — « depuis toujours », pas un silence"
    );

    let bound = crate::timestamp::now_minus(chrono::Duration::hours(1));
    assert_eq!(
        lire(&db, Some(&bound)),
        vec![recent],
        "borné à une heure, le vol de deux heures est dehors"
    );
}

/// Un vol encore en vol n'est pas un vol terminé. Le filtre tourne en Rust
/// (`is_terminal_task_status`), donc ce test est aussi ce qui dit que la
/// projection l'applique bien.
#[test]
fn mika2698_un_vol_non_terminal_est_exclu() {
    for statut in ["pending", "in_progress", "blocked"] {
        let db = db();
        let id = db
            .create_task(&vol("mika", &issue_url(TICKET)))
            .expect("le vol doit être créé");
        db.update_task_status(&id, statut)
            .expect("le changement de statut doit réussir");
        assert!(
            lire(&db, None).is_empty(),
            "statut {statut:?} : un vol non terminal a été lu comme terminé"
        );
    }
}

/// Un vol de **grooming** n'est pas un vol d'implémentation ; une classe
/// absente l'est, par le `COALESCE(dispatch_class, 'implement')` qui porte tout
/// l'historique d'avant la v34.
#[test]
fn mika2698_le_grooming_est_exclu_et_la_classe_absente_vaut_implement() {
    let db = db();
    let groom = db
        .create_task(&groom_parent("mika", &issue_url(TICKET)))
        .expect("le groom doit être créé");
    db.update_task_completed(&groom, "mika", Some("Verdict: GROOMED"))
        .expect("la complétion doit réussir");
    assert!(
        lire(&db, None).is_empty(),
        "un vol de grooming n'est pas un vol d'implémentation"
    );

    let sans_classe = db
        .create_task(&NewTask {
            dispatch_class: None,
            ..groom_parent("mika", &issue_url(TICKET))
        })
        .expect("le vol sans classe doit être créé");
    db.update_task_completed(&sans_classe, "mika", Some("Outcome: PR_OPENED"))
        .expect("la complétion doit réussir");
    assert_eq!(
        lire(&db, None),
        vec![sans_classe],
        "une classe absente vaut `implement` (lignes d'avant la v34)"
    );
}

/// **L'assertion renversée, et pourquoi** (mika#2792).
///
/// Ce test s'appelait `mika2698_la_lecture_est_cantonnee_a_lagent` et exigeait
/// l'inverse : qu'un vol de `mika-qa` soit **invisible** depuis un handle
/// `mika`. C'est ce cantonnement qui a rendu l'évaluateur de vols inerte —
/// quatre jours, 43 vols, aucun verdict — parce que le manager n'a pas d'agent
/// et interrogeait celui qu'il avait emprunté pour obtenir une connexion.
///
/// Le test n'a pas été supprimé : il est **retourné**, pour qu'un lecteur de
/// l'historique trouve la trace du renversement plutôt qu'un trou. Ce qui porte
/// désormais la délimitation est le **dépôt**, et c'est
/// [`mika2921_un_vol_hors_depot_controle_nest_pas_trouve_meme_sous_un_autre_agent`]
/// qui l'exige.
#[test]
fn mika2792_la_lecture_nest_plus_cantonnee_a_lagent() {
    let db = db();
    enregistrer(&db, "mika-qa");
    enregistrer(&db, "mika-dev");

    let qa = seed_vol_termine(&db, "mika-qa", &issue_url(TICKET));
    let dev = seed_vol_termine(&db, "mika-dev", &issue_url(TICKET));

    let vu = lire(&db, None);
    assert_eq!(
        vu.len(),
        2,
        "deux vols sous deux agents distincts : les deux sont rendus, obtenu {vu:?}"
    );
    assert!(vu.contains(&qa) && vu.contains(&dev));
}

/// **Le contrôle positif que la suite de mika#2698 n'avait pas.** Un vol semé
/// sous un `agent_id` qui n'est pas celui du handle par défaut doit être trouvé.
/// C'est la forme exacte de la production : les vols d'implement sont des tâches
/// de `mika-dev`, le handle emprunté par la cadence est celui de l'agent par
/// défaut du processus.
#[test]
fn mika2792_un_vol_seme_sous_un_autre_agent_est_trouve() {
    let db = db();
    enregistrer(&db, "mika-dev");
    let id = seed_vol_termine(&db, "mika-dev", &issue_url(TICKET));
    assert_eq!(
        lire(&db, None),
        vec![id],
        "le vol de `mika-dev` doit être trouvé — c'est là que vivent les vols"
    );
}

/// **AC2 — le contrôle négatif, au niveau du SQL.** Élargir du milestone au
/// dépôt n'a pas levé la délimitation : un vol sur un dépôt hors de la liste
/// reste invisible, y compris sous un autre agent. C'est la forme de
/// claude-pilot#232.
#[test]
fn mika2921_un_vol_hors_depot_controle_nest_pas_trouve_meme_sous_un_autre_agent() {
    let db = db();
    enregistrer(&db, "mika-dev");
    let dehors = seed_vol_termine(&db, "mika-dev", &issue_url_sur(REPO_DEHORS, 232));
    let dedans = seed_vol_termine(&db, "mika-dev", &issue_url(TICKET));

    assert_eq!(
        lire(&db, None),
        vec![dedans],
        "seul le vol du dépôt interrogé sort, même sous un autre agent"
    );

    // Contrôle positif sur la fixture écartée : elle existe bien, et c'est son
    // dépôt — non son agent — qui l'a tenue dehors.
    assert_eq!(
        lire_sur(&db, None, &[REPO_DEHORS.to_string()]),
        vec![dehors]
    );
}

/// **Le contrôle négatif du détecteur** (V5 de mika#2921). Aucun dépôt ne peut
/// pas vouloir dire « tous les vols » : sans cette borne, une liste de dépôts
/// vide — un `DISPATCHABLE_REPOS` vidé, une projection en échec — ferait juger
/// la table entière, claude-pilot compris. Et un prédicat vide n'est pas du SQL
/// valide.
#[test]
fn mika2921_aucun_depot_ne_rend_jamais_tous_les_vols() {
    let db = db();
    seed_vol_termine(&db, "mika", &issue_url(TICKET));
    seed_vol_termine(&db, "mika", &issue_url_sur(REPO_DEHORS, 232));

    let vu = db
        .list_finished_implement_flights_in_container(None, &[])
        .expect("la lecture doit répondre, pas échouer sur un prédicat vide");
    assert!(vu.is_empty(), "aucun dépôt ⇒ aucun vol, jamais tous");
}

/// L'ordre est chronologique croissant sur la borne qui a sélectionné la ligne
/// — donc un rapport qui lit les vols dans l'ordre où ils ont fini.
#[test]
fn mika2698_les_vols_sortent_dans_lordre_chronologique() {
    let db = db();
    let vieux = seed_vol_termine(&db, "mika", &issue_url(TICKET));
    db.backdate_task_completed_at(&vieux, 3_600)
        .expect("le recul doit réussir");
    db.backdate_task_updated_at(&vieux, 3_600)
        .expect("le recul doit réussir");
    let neuf = seed_vol_termine(&db, "mika", &issue_url(TICKET));

    assert_eq!(lire(&db, None), vec![vieux, neuf]);
}

/// **Le dernier enfant pilote gagne** (mika#2886). Un vol qui a été relancé
/// porte plusieurs enfants `long_running:run_claude_pilot`, chacun avec sa
/// propre issue ; celle qui vaut est la dernière. Le tri est fait en Rust sur
/// `COALESCE(completed_at, updated_at)` puis sur l'`id`, sur l'ensemble déjà
/// filtré — un `ORDER BY … LIMIT 1` en SQL aurait choisi avant les filtres de
/// population et de contenu, donc parfois un enfant muet.
///
/// Les trois lignes de `tasks` ne rendent **qu'un** vol : un enfant ne porte
/// pas de `reference_url`, donc la requête 1 ne le voit pas. Si ce compte
/// passait à trois, la jointure aurait fait des enfants des vols.
#[test]
fn mika2886_le_dernier_enfant_pilote_gagne() {
    let db = db();
    let parente = seed_vol_termine(&db, "mika", &issue_url(TICKET));

    let premier = seed_enfant(
        &db,
        "mika",
        &parente,
        "long_running:run_claude_pilot",
        "Outcome: PIPELINE_INCOMPLETE\nTurns: 47\n",
    );
    db.backdate_task_completed_at(&premier, 3_600)
        .expect("le recul doit réussir");
    db.backdate_task_updated_at(&premier, 3_600)
        .expect("le recul doit réussir");

    seed_enfant(
        &db,
        "mika",
        &parente,
        "long_running:run_claude_pilot",
        "Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2881\nTurns: 121\n",
    );

    let lignes = lire_lignes(&db);
    assert_eq!(
        lignes.len(),
        1,
        "trois lignes de `tasks`, un seul vol : un enfant n'est pas un vol"
    );
    assert_eq!(lignes[0].flight.id, parente);
    assert_eq!(
        lignes[0].pilot_result.as_deref(),
        Some(
            "Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2881\nTurns: 121\n"
        ),
        "c'est l'issue du dernier enfant qui vaut, non celle de la relance \
         précédente"
    );
}

/// **Un enfant hors population n'est pas lu** (mika#2886). Une parente de vol
/// porte aussi des enfants qui ne sont pas des dispatches de pilote —
/// `long_running:build_mika` en est un, et son `result` n'est pas une issue de
/// vol. Le filtre est [`crate::task_engine::engine::is_pilot_dispatch_label`],
/// son lecteur unique : une clause `IN` recopiant la liste des labels en SQL
/// serait une seconde source de vérité, et c'est par là que `build_mika` se
/// glisserait au premier élargissement de la liste.
///
/// Le contrôle positif est ce qui sépare « le label délimite » de « la
/// jointure ne trouve jamais rien » : le même enfant, sous le label du pilote,
/// doit être lu.
#[test]
fn mika2886_un_enfant_hors_population_nest_pas_lu() {
    const ISSUE: &str = "Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2881\n";

    let hors = db();
    let parente = seed_vol_termine(&hors, "mika", &issue_url(TICKET));
    seed_enfant(&hors, "mika", &parente, "long_running:build_mika", ISSUE);

    let lignes = lire_lignes(&hors);
    assert_eq!(lignes.len(), 1);
    assert!(
        lignes[0].pilot_result.is_none(),
        "`long_running:build_mika` n'est pas un dispatch de pilote — reçu {:?}",
        lignes[0].pilot_result
    );

    // Contrôle positif, sur une base neuve : même parente, même `result`, seul
    // le label change.
    let temoin = db();
    let parente = seed_vol_termine(&temoin, "mika", &issue_url(TICKET));
    seed_enfant(
        &temoin,
        "mika",
        &parente,
        "long_running:run_claude_pilot",
        ISSUE,
    );
    assert_eq!(
        lire_lignes(&temoin)[0].pilot_result.as_deref(),
        Some(ISSUE),
        "sous le label du pilote, le même enfant doit être lu — sinon ce test \
         ne mesure pas le label"
    );
}

// ---- mika#2921 : le corpus des 13 vols mesurés, de la base au rapport ------

/// Le milestone cible de la production le 2026-10-26
/// (`MIKA_MANAGER_TARGET_MILESTONE = senara-solutions/mika#33`), et ses quatre
/// sous-tickets. **Aucun** des 13 vols mesurés n'en est un : c'est le relevé
/// qui a ouvert mika#2921, et c'est ce qui rendait la section « Vols terminés »
/// vide en production.
const MILESTONE_CIBLE: u64 = 33;
const SOUS_TICKETS_DU_MILESTONE_CIBLE: [u64; 4] = [2921, 2922, 2923, 2924];

/// Les 13 vols `implement` terminés entre le 2026-10-23T15:00Z et le relevé du
/// 2026-10-26, sous leur forme réelle : `(dépôt, ticket, statut de la parente,
/// issue du pilote)`, dans l'ordre où ils ont fini.
///
/// Les issues sont celles que l'enfant `long_running:run_claude_pilot` porte en
/// base, réduites aux lignes que l'évaluateur lit (`Turns:`, `PR:`,
/// `Outcome:`, `Halt class:`, la bannière `PIPELINE FAILURE:`) et aux numéros
/// de PR réels. Les répétitions sont réelles aussi : mika#2900, #2879 et #2917
/// ont chacun volé deux fois.
///
/// **Deux écarts de forme, nommés.** (1) Le premier vol de mika#2879 est
/// `cancelled` en base (conflit de rebase, deux lignes) ; le ticket le compte
/// parmi les douze `completed`, et [`seme_la_forme_reelle`] ne sème que les deux
/// chemins d'écriture terminaux qu'il connaît, donc il est semé `completed`
/// avec son issue réelle — elle ne porte aucune ligne `Outcome:`, et c'est ce
/// qui décide son verdict, pas son statut. (2) claude-pilot#232 n'a **aucun**
/// enfant en base (`failed`, `phantom_aged_out`) ; le helper lui en sème un.
/// Ni l'un ni l'autre ne touche ce que ce test mesure : le **dépôt** est le
/// seul axe du périmètre.
const CORPUS_DES_13_VOLS: [(&str, u32, &str, &str); 13] = [
    (
        "senara-solutions/mika",
        2900,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 112\nPR: https://github.com/senara-solutions/mika/pull/2903\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2903\n",
    ),
    (
        "senara-solutions/mika",
        2879,
        "completed",
        "STATUS=REBASE_CONFLICT\nBranch feat/2879/suivi-mika-2872-vol-2-rattacher-la-seule is 6 commits behind origin/main.\n",
    ),
    (
        "senara-solutions/mika",
        2879,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 88\nPR: https://github.com/senara-solutions/mika/pull/2904\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2904\n",
    ),
    (
        "senara-solutions/mika",
        2900,
        "completed",
        "aucun dispatch produit ; parent d4b93658-ca04-4800-8bed-59a04ee3fe0c (completed) terminal — re-armement impossible\n",
    ),
    (
        "senara-solutions/mika",
        2897,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 77\nPR: https://github.com/senara-solutions/mika/pull/2907\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2907\n",
    ),
    (
        "senara-solutions/mika",
        2908,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 47\nPR: https://github.com/senara-solutions/mika/pull/2910\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2910\n",
    ),
    (
        "senara-solutions/mika",
        2909,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 60\nPR: https://github.com/senara-solutions/mika/pull/2911\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2911\n",
    ),
    (
        "senara-solutions/mika-cloud",
        193,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 177\nPR: https://github.com/senara-solutions/mika-cloud/pull/277\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika-cloud/pull/277\n",
    ),
    (
        "senara-solutions/mika-cloud",
        198,
        "completed",
        "PIPELINE FAILURE: the claude-pilot session was terminated before it finished, but it left work behind.\nTurns: 151\nHalt class: budget_exhausted — SDK turn limit reached; work was produced and the recovery chain carries it\nOutcome: UNKNOWN — inspect worktree manually.\nPR: https://github.com/senara-solutions/mika-cloud/pull/281\n",
    ),
    (
        "senara-solutions/mika-cloud",
        196,
        "completed",
        "PIPELINE FAILURE: the claude-pilot session was terminated before it finished, but it left work behind.\nTurns: 151\nHalt class: budget_exhausted — SDK turn limit reached; work was produced and the recovery chain carries it\nOutcome: UNKNOWN — inspect worktree manually.\nPR: https://github.com/senara-solutions/mika-cloud/pull/284\n",
    ),
    (
        "senara-solutions/claude-pilot",
        232,
        "failed",
        "phantom_aged_out\n",
    ),
    (
        "senara-solutions/mika",
        2917,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 102\nPR: https://github.com/senara-solutions/mika/pull/2918\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2918\n",
    ),
    (
        "senara-solutions/mika",
        2917,
        "completed",
        "claude-pilot completed (status: success).\nTurns: 138\nPR: https://github.com/senara-solutions/mika/pull/2919\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2919\n",
    ),
];

/// Sème chaque vol de `corpus` dans la topologie à deux lignes de mika#2886, sous
/// `mika-dev` — l'agent qui possède les vols en production — et rend la base
/// prête à être prêtée à la source.
fn base_semee(corpus: &[(&str, u32, &str, &str)]) -> Database {
    let db = db();
    for (repo, ticket, statut, issue_du_pilote) in corpus {
        seme_la_forme_reelle(
            &db,
            "mika-dev",
            &VolReel {
                url_du_ticket: &issue_url_sur(repo, *ticket),
                statut,
                issue_du_pilote,
            },
        );
    }
    db
}

/// Un sous-ticket du milestone cible, tel que le `Reader` l'hydrate : seuls
/// `number` et `pr_state` comptent pour l'évaluateur.
fn sous_ticket(number: u64, pr_state: Option<&str>) -> crate::milestone_manager::types::SubIssue {
    use crate::milestone_manager::types::{CiState, IssueState, SubIssue};
    SubIssue {
        number,
        title: format!("sous-ticket {number}"),
        state: IssueState::Open,
        priority_rank: None,
        plan_present: true,
        branch_present: false,
        pr_number: None,
        pr_url: None,
        pr_link_source: None,
        merged_prs: None,
        pr_state: pr_state.map(str::to_string),
        ci_state: CiState::None,
        blockers: vec![],
        updated_at: "2026-10-26T00:00:00Z".into(),
        labels: vec![],
    }
}

/// Le rapport complet d'un cycle dont l'évaluateur a rendu `verdicts`, sur le
/// milestone cible de production.
fn rapport(
    sub_issues: Vec<crate::milestone_manager::types::SubIssue>,
    verdicts: Vec<crate::milestone_manager::evaluator::FlightVerdict>,
) -> String {
    use crate::milestone_manager::types::{
        Assessment, IssueState, MilestoneRef, MilestoneState, ProgressCounts, Recommendation,
        Severity,
    };
    let state = MilestoneState {
        milestone_ref: MilestoneRef {
            repo: REPO.into(),
            number: MILESTONE_CIBLE,
        },
        title: "mika-manager tient sa partie".into(),
        description: "".into(),
        state: IssueState::Open,
        created_at: "".into(),
        due_on: None,
        last_activity_at: None,
        sub_issues,
        progress: ProgressCounts::default(),
        recent_activity: vec![],
        executor_healthy: None,
        ready_pool: vec![],
        outside_candidates: vec![],
        outside_read_truncated: None,
        plan_sizes: Default::default(),
    };
    let assessment = Assessment {
        severity: Severity::Healthy,
        recommendation: Recommendation {
            next_sub_issue: None,
            rationale: "corpus mika#2921".into(),
        },
        alerts: vec![],
        cross_cutting: vec![],
        contention_events: vec![],
        pool: None,
        flight_verdicts: verdicts,
        relief: None,
    };
    crate::milestone_manager::reporter::Reporter::new().report(&state, &assessment)
}

/// Les lignes de la section « Vols terminés » d'un rapport — et rien d'autre.
fn lignes_des_vols(rapport: &str) -> Vec<String> {
    let debut = rapport
        .find("### Vols terminés")
        .unwrap_or_else(|| panic!("la section « Vols terminés » doit être rendue :\n{rapport}"));
    let section = &rapport[debut..];
    let fin = section
        .find("### Silent-progress")
        .expect("la section des alertes suit celle des vols");
    section[..fin]
        .lines()
        .filter(|l| l.starts_with("- "))
        .map(str::to_string)
        .collect()
}

/// **AC1 et AC2 de mika#2921, sur une vraie base et dans le même test.**
///
/// Le corpus des 13 vols mesurés passe par le chemin de production entier —
/// base, [`crate::flight_source::DbFlightSource`] (donc le périmètre lu dans
/// `DISPATCHABLE_REPOS`, et lui seul), `evaluate_flights`, puis le `Reporter` —
/// sans bouchon de `FlightSource`.
///
/// - **AC1, la boucle visible.** Les 12 vols sur mika et mika-cloud reçoivent
///   chacun un verdict, alors qu'**aucun** n'est un sous-ticket du milestone
///   cible : c'est le relevé du 2026-10-26, contre lequel l'ancien périmètre
///   rendait zéro verdict.
/// - **AC2, jamais au-delà.** claude-pilot#232 n'est ni jugé ni rapporté. Son
///   contrôle positif est dans le même test : la ligne existe en base, et c'est
///   son **dépôt** — absent de `DISPATCHABLE_REPOS` — qui la tient dehors, pas
///   une fixture qui n'aurait rien semé.
///
/// Le compte des issues est épinglé aussi (8 PR ouvertes, 2 morts au plafond,
/// 2 sans ligne `Outcome:`) : c'est la forme qu'AC6 attend du premier cycle
/// après déploiement — des verdicts **classés** sur des vols qui ont ouvert une
/// PR, pas une colonne de `no_outcome_line`.
#[tokio::test]
async fn mika2921_le_corpus_des_13_vols_rend_12_verdicts_et_aucun_pour_claude_pilot() {
    use crate::milestone_manager::evaluator::{
        DeathCause, FlightIssue, FlightSource, PrStateKnowledge, evaluate_flights,
    };

    let base = crate::async_db::AsyncDatabase::new_with_agent(
        base_semee(&CORPUS_DES_13_VOLS),
        "mika-test",
    );
    let source = crate::flight_source::DbFlightSource::new(base.clone());
    let sub_issues: Vec<_> = SOUS_TICKETS_DU_MILESTONE_CIBLE
        .iter()
        .map(|n| sous_ticket(*n, None))
        .collect();

    let vols = source.finished_implement_flights(None).await;
    let verdicts = evaluate_flights(&vols, &sub_issues);

    // AC1 — un verdict par vol d'un dépôt sous contrôle, et pas un de plus.
    assert_eq!(
        verdicts.len(),
        12,
        "13 vols semés, 12 sur un dépôt sous contrôle : obtenu {:?}",
        verdicts
            .iter()
            .map(|v| (v.repo.clone(), v.issue_number))
            .collect::<Vec<_>>()
    );
    let mut juges: Vec<(String, u64)> = verdicts
        .iter()
        .map(|v| {
            (
                v.repo
                    .clone()
                    .expect("chaque vol du corpus nomme son dépôt"),
                v.issue_number
                    .expect("chaque vol du corpus nomme son ticket"),
            )
        })
        .collect();
    juges.sort();
    let mut attendus: Vec<(String, u64)> = CORPUS_DES_13_VOLS
        .iter()
        .filter(|(repo, ..)| *repo != REPO_DEHORS)
        .map(|(repo, ticket, ..)| (repo.to_string(), u64::from(*ticket)))
        .collect();
    attendus.sort();
    assert_eq!(
        juges, attendus,
        "chaque vol de mika et de mika-cloud est jugé, répétitions comprises"
    );

    // Aucun n'est dans le milestone cible : tous disent qu'ils n'ont rien lu.
    assert!(
        verdicts
            .iter()
            .all(|v| v.pr_state == PrStateKnowledge::NotRead),
        "aucun vol du corpus n'est un sous-ticket du milestone {MILESTONE_CIBLE} — \
         un état « lu » serait inventé"
    );

    // La forme qu'AC6 attend : des verdicts classés.
    let ouvertes = verdicts
        .iter()
        .filter(|v| {
            matches!(
                v.issue,
                FlightIssue::PrOpened {
                    number: Some(_),
                    ..
                }
            )
        })
        .count();
    let au_plafond = verdicts
        .iter()
        .filter(|v| {
            v.issue
                == FlightIssue::Dead {
                    cause: DeathCause::CeilingReached,
                }
        })
        .count();
    let sans_outcome = verdicts
        .iter()
        .filter(|v| {
            v.issue
                == FlightIssue::Dead {
                    cause: DeathCause::Unknown("no_outcome_line".into()),
                }
        })
        .count();
    assert_eq!(
        (ouvertes, au_plafond, sans_outcome),
        (8, 2, 2),
        "8 PR ouvertes, 2 morts au plafond, 2 sans ligne `Outcome:`"
    );

    // AC2 — ni le verdict ni le rapport ne nomment le vol hors périmètre.
    assert!(
        verdicts
            .iter()
            .all(|v| v.repo.as_deref() != Some(REPO_DEHORS)),
        "claude-pilot#232 a été jugé : le périmètre fuit"
    );
    let lignes = lignes_des_vols(&rapport(sub_issues, verdicts));
    assert_eq!(
        lignes.len(),
        12,
        "une ligne de rapport par verdict : {lignes:#?}"
    );
    for ligne in &lignes {
        assert!(
            !ligne.contains("claude-pilot") && !ligne.contains("#232 "),
            "le rapport nomme le vol hors périmètre : {ligne}"
        );
        assert!(
            ligne.contains(" — hors milestone cible — ") && ligne.contains("état de PR non lu"),
            "un vol hors cible le dit, et dit qu'il n'a rien lu : {ligne}"
        );
    }

    // Contrôle positif d'AC2 : la ligne de claude-pilot#232 existe, et c'est son
    // dépôt — non une fixture vide — qui l'a tenue dehors.
    let dehors = base
        .list_finished_implement_flights_in_container(None, vec![REPO_DEHORS.to_string()])
        .await
        .expect("la lecture doit répondre");
    assert_eq!(
        dehors.len(),
        1,
        "claude-pilot#232 doit être en base, sous son propre dépôt"
    );
    assert_eq!(
        dehors[0].flight.reference_url.as_deref(),
        Some(issue_url_sur(REPO_DEHORS, 232).as_str())
    );
}

/// **AC5 de mika#2921 — l'état de PR hors milestone, par le chemin réel.**
///
/// Un vol sur mika **hors** du milestone cible rend
/// [`PrStateKnowledge::NotRead`], et sa ligne de rapport dit
/// `état de PR non lu`. Le contrôle positif est dans le même test : un vol
/// **dans** le milestone cible, apparié à un sous-ticket dont le `Reader` a lu
/// `pr_state = "open"`, rend `Read(Some("open"))` et une ligne qui ne dit pas
/// « non lu ». Les deux arms côte à côte séparent « le type est correct » de
/// « le rendu est branché » ; les tests du `Reporter` tiennent le rendu sur des
/// verdicts littéraux, celui-ci le tient sur des verdicts sortis de la base.
#[tokio::test]
async fn mika2921_hors_cible_letat_de_pr_est_non_lu_et_dans_la_cible_il_est_lu() {
    use crate::milestone_manager::evaluator::{FlightSource, PrStateKnowledge, evaluate_flights};

    const PR_OUVERTE: &str = "claude-pilot completed (status: success).\nTurns: 90\nPR: https://github.com/senara-solutions/mika/pull/2925\n\nOutcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/2925\n";
    let corpus = [
        (REPO, 2921, "completed", PR_OUVERTE),
        (REPO, 2900, "completed", PR_OUVERTE),
    ];
    let source = crate::flight_source::DbFlightSource::new(
        crate::async_db::AsyncDatabase::new_with_agent(base_semee(&corpus), "mika-test"),
    );
    let sub_issues = vec![sous_ticket(2921, Some("open"))];

    let vols = source.finished_implement_flights(None).await;
    let verdicts = evaluate_flights(&vols, &sub_issues);
    assert_eq!(verdicts.len(), 2);
    let verdict_de = |n: u64| {
        verdicts
            .iter()
            .find(|v| v.issue_number == Some(n))
            .unwrap_or_else(|| panic!("le vol de mika#{n} doit être jugé"))
    };

    // Hors cible : rien n'a été lu, et le verdict le porte.
    let hors = verdict_de(2900);
    assert_eq!(hors.pr_state, PrStateKnowledge::NotRead);
    let ligne_hors = crate::milestone_manager::reporter::render_flight_verdict(hors);
    assert!(
        ligne_hors.contains(" — hors milestone cible — ")
            && ligne_hors.contains(" — état de PR non lu — "),
        "obtenu : {ligne_hors}"
    );

    // Contrôle positif : dans la cible, l'état lu est rendu, et jamais « non lu ».
    let dans = verdict_de(2921);
    assert_eq!(dans.pr_state, PrStateKnowledge::Read(Some("open".into())));
    let ligne_dans = crate::milestone_manager::reporter::render_flight_verdict(dans);
    assert!(
        ligne_dans.starts_with("- mika#2921 — milestone cible — ")
            && ligne_dans.contains(" — PR open — "),
        "obtenu : {ligne_dans}"
    );
    assert!(
        !ligne_dans.contains("non lu"),
        "un état lu ne se rend jamais comme non lu : {ligne_dans}"
    );

    // Et le rapport complet porte les deux, dans la même section.
    let lignes = lignes_des_vols(&rapport(sub_issues, verdicts.clone()));
    assert_eq!(lignes.len(), 2, "{lignes:#?}");
    assert!(lignes.contains(&ligne_hors) && lignes.contains(&ligne_dans));
}
