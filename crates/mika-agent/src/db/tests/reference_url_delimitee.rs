//! `reference_url` est comparée à un **numéro délimité**, jamais à un préfixe
//! (mika#2638).
//!
//! # Le défaut que ces tests épinglent
//!
//! Un préfixe ne délimite pas un numéro : la sonde de `…/issues/216` matchait
//! aussi les tâches de `…/issues/2160` à `…/issues/2169`, et `…/issues/21600`.
//! Deux sites SQL portaient le défaut et posent deux questions différentes sur
//! la même table — « une tâche self_dev active référence-t-elle ce ticket ? »
//! (`find_active_self_dev_task_for_issue`, lu par `auto_pull`) et « un pilote
//! est-il vif pour ce ticket ? » (`find_dispatch_children_for_issue_url`, le
//! lecteur unique de `live_pilot`, mika#2279).
//!
//! # Pourquoi « vu rouge » est non négociable ici
//!
//! Sans le rouge d'abord, un test qui passe sur l'arbre réparé ne distingue pas
//! « le prédicat délimite » de « la fixture ne pose pas la question ». Chaque
//! test de délimitation ci-dessous a été observé rouge sur l'arbre **avant** le
//! passage à `reference_url IN (?2, ?3)`, et la ligne qui le dit est à côté de
//! son assertion.

use super::*;

/// Le ticket court dont la sonde était trop large.
const COURT: u32 = 216;

/// Son voisin numérique : `216` est un préfixe de `2161`, et c'est tout le
/// défaut.
const VOISIN: u32 = 2161;

const REPO: &str = "senara-solutions/mika";

fn issue_url(repo: &str, n: u32) -> String {
    format!("https://github.com/{repo}/issues/{n}")
}

/// Seede un parent self_dev actif portant `reference_url`, et rend son id.
fn seed_parent(db: &Database, reference_url: &str) -> String {
    db.create_task(&groom_parent("mika", reference_url))
        .expect("le parent de suivi doit être créé")
}

/// Seede la paire parent + enfant callback porteur d'un pgid — la topologie
/// d'un dispatch (mika#2279 : l'URL est sur le parent, le pgid sur l'enfant,
/// rien ne porte les deux).
fn seed_dispatch(db: &Database, reference_url: &str) -> String {
    let parent = seed_parent(db, reference_url);
    let child = db
        .create_task(&groom_callback("mika", &parent, "implement"))
        .expect("l'enfant callback doit être créé");
    db.set_task_process_id(&child, Some(4_242))
        .expect("le pgid doit être enregistré");
    parent
}

/// **D1 / AC3, site 1 — la sonde en vol de #216 ne voit pas la tâche de #2161.**
///
/// **Vu rouge** avant le correctif : le `LIKE '…/issues/216%'` matchait la ligne
/// de `#2161`, donc `auto_pull` nommait la tâche d'un autre ticket comme celle
/// qui coince son bassin — le défaut que le message (b) de mika#2161 a rendu
/// visible.
#[test]
fn mika2638_la_sonde_en_vol_de_216_ignore_la_tache_de_2161() {
    let db = db();
    seed_parent(&db, &issue_url(REPO, VOISIN));

    assert!(
        db.find_active_self_dev_task_for_issue("mika", &issue_url(REPO, COURT))
            .expect("la sonde doit répondre")
            .is_none(),
        "la sonde de #216 a trouvé la tâche de #2161 : un préfixe ne délimite pas \
         un numéro (mika#2638)"
    );
}

/// **D1 / AC3, site 1 — contrôle positif.**
///
/// La moitié qui empêche de lire « le prédicat délimite » là où il n'y aurait
/// plus rien à trouver : la sonde de #216 trouve bien **sa** tâche.
#[test]
fn mika2638_la_sonde_en_vol_de_216_trouve_bien_la_tache_de_216() {
    let db = db();
    let parent = seed_parent(&db, &issue_url(REPO, COURT));

    let found = db
        .find_active_self_dev_task_for_issue("mika", &issue_url(REPO, COURT))
        .expect("la sonde doit répondre")
        .expect("la tâche de #216 doit être trouvée");
    assert_eq!(found.task_id, parent);
}

/// **D1 / AC3, site 2 — la sonde de pilote vif de #216 ne voit pas le dispatch
/// de #2161.**
///
/// **Vu rouge** avant le correctif. Le rayon de souffle de ce site est plus
/// large que celui du premier : la porte 2c du `ready_label_handler` refusait un
/// `labeled ready` **au motif du pilote d'un autre ticket**, c'est-à-dire gelait
/// un ticket sain.
#[test]
fn mika2638_la_sonde_de_pilote_de_216_ignore_le_dispatch_de_2161() {
    let db = db();
    seed_dispatch(&db, &issue_url(REPO, VOISIN));

    assert!(
        db.find_dispatch_children_for_issue_url("mika", &issue_url(REPO, COURT))
            .expect("la sonde doit répondre")
            .is_empty(),
        "la sonde de pilote de #216 a trouvé le dispatch de #2161 (mika#2638)"
    );
}

/// **D1 / AC3, site 2 — contrôle positif.**
#[test]
fn mika2638_la_sonde_de_pilote_de_216_trouve_bien_le_dispatch_de_216() {
    let db = db();
    let parent = seed_dispatch(&db, &issue_url(REPO, COURT));

    let found = db
        .find_dispatch_children_for_issue_url("mika", &issue_url(REPO, COURT))
        .expect("la sonde doit répondre");
    assert_eq!(found.len(), 1, "le dispatch de #216 doit être trouvé");
    assert_eq!(found[0].parent_task_id, parent);
}

/// **D2 — délimitation complète, en table.**
///
/// Les trois voisins numériques (`2160`, `21600`, `2161`) sont exclus, la
/// variante `?phase=groom` reste couverte, et les deux suffixes **non
/// déclarés** (`/` et `#issuecomment-1`) sont hors de l'ensemble clos — ce
/// dernier point est le rétrécissement nommé du plan § 5, épinglé ici plutôt
/// que découvert en production.
///
/// **Vu rouge** avant le correctif sur les trois voisins.
#[test]
fn mika2638_la_table_de_delimitation_du_site_en_vol() {
    // (suffixe ou numéro seedé, la sonde de #216 doit-elle le voir ?)
    let cas: &[(&str, bool)] = &[
        ("", true),
        ("?phase=groom", true),
        // Les deux suffixes hors de l'ensemble clos des variantes : une telle
        // ligne ne dédoublonne PAS contre la ligne canonique dans
        // `idx_tasks_manual_active_ref_url` (elle y entre sous sa propre clé),
        // donc un défaut en amont et plus grave que celui-ci (plan § 5 ; la
        // formulation exacte est sur `issue_url_variants`).
        ("/", false),
        ("#issuecomment-1", false),
    ];

    for (suffixe, attendu) in cas {
        let db = db();
        seed_parent(&db, &format!("{}{suffixe}", issue_url(REPO, COURT)));
        let vu = db
            .find_active_self_dev_task_for_issue("mika", &issue_url(REPO, COURT))
            .expect("la sonde doit répondre")
            .is_some();
        assert_eq!(
            vu, *attendu,
            "suffixe {suffixe:?} : la sonde de #216 rend {vu}, attendu {attendu}"
        );
    }

    for voisin in [2160u32, 21600, 2161] {
        let db = db();
        seed_parent(&db, &issue_url(REPO, voisin));
        assert!(
            db.find_active_self_dev_task_for_issue("mika", &issue_url(REPO, COURT))
                .expect("la sonde doit répondre")
                .is_none(),
            "la sonde de #216 a trouvé la tâche de #{voisin} (mika#2638)"
        );
    }
}

/// **D2 — le joker `_` de `LIKE`, le second élargissement que le ticket ne
/// nommait pas.**
///
/// `_` matche un caractère quelconque en `LIKE`, et GitHub autorise `_` dans un
/// nom de dépôt : une sonde pour `…/my_repo/issues/42` matchait
/// `…/myXrepo/issues/42`. Population **vide aujourd'hui** (`senara-solutions/mika`
/// ne porte pas de `_`), réelle demain — et fermée gratuitement par l'égalité.
///
/// **Vu rouge** avant le correctif.
#[test]
fn mika2638_le_joker_underscore_de_like_est_ferme() {
    let db = db();
    let parent = seed_parent(&db, &issue_url("acme/myXrepo", 42));

    assert!(
        db.find_active_self_dev_task_for_issue("mika", &issue_url("acme/my_repo", 42))
            .expect("la sonde doit répondre")
            .is_none(),
        "`_` a été lu comme un joker `LIKE` : la sonde de `my_repo` a trouvé la \
         tâche de `myXrepo` (mika#2638, R3)"
    );

    // CONTRÔLE POSITIF sur la fixture elle-même, relevé en revue : sans lui ce
    // test ne distingue pas « le joker est fermé » de « cette fixture
    // n'enregistre rien », et il passerait sur un arbre où le seed est cassé.
    let found = db
        .find_active_self_dev_task_for_issue("mika", &issue_url("acme/myXrepo", 42))
        .expect("la sonde doit répondre")
        .expect("la fixture doit être enregistrée sous son propre dépôt");
    assert_eq!(found.task_id, parent);
}

/// **V3 — non-régression mika#1934 : le site déjà correct reste correct.**
///
/// `find_active_tracking_rows_by_reference_url_and_variants` est réécrit à
/// travers le helper **sans changement de comportement** : sa sortie est octet
/// pour octet la même. Le but est que les trois sites ne puissent plus diverger,
/// pas de modifier celui qui était juste.
#[test]
fn mika2638_le_site_deja_correct_est_inchange() {
    let base = issue_url(REPO, COURT);

    // La variante `?phase=groom` et l'URL exacte sont toutes deux vues…
    for suffixe in ["", "?phase=groom"] {
        let db = db();
        let parent = seed_parent(&db, &format!("{base}{suffixe}"));
        db.update_manual_task_status(&parent, "mika", "in_progress")
            .expect("le passage en in_progress doit réussir");
        let rows = db
            .find_active_tracking_rows_by_reference_url_and_variants("mika", &base)
            .expect("la sonde doit répondre");
        assert_eq!(
            rows.len(),
            1,
            "suffixe {suffixe:?} : la variante doit rester couverte (mika#1934)"
        );
    }

    // …et le voisin numérique ne l'est pas.
    let db = db();
    let parent = seed_parent(&db, &issue_url(REPO, VOISIN));
    db.update_manual_task_status(&parent, "mika", "in_progress")
        .expect("le passage en in_progress doit réussir");
    assert!(
        db.find_active_tracking_rows_by_reference_url_and_variants("mika", &base)
            .expect("la sonde doit répondre")
            .is_empty(),
        "le site 3 était déjà délimité et doit le rester"
    );
}
