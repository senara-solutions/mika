//! La revue ne dépend plus d'un événement unique et perdable — mika#2334.
//!
//! Le 2026-09-15, deux PRs du drain sont restées ouvertes sans revue et
//! l'orchestrateur a dû poser `mika-platform-qa` à la main. Le ticket en
//! déduisait qu'un pas trailing `gh pr edit --add-reviewer` avait été sauté
//! parce que le pilote mourait avant de l'atteindre. Deux mesures déplacent le
//! diagnostic : ce pas n'existait nulle part dans le dépôt, et la revue est
//! déclenchée par le webhook `pull_request.opened`, pas par le pilote. Le défaut
//! réel est que cet événement est **unique, non rejouable, et que rien ne
//! relisait une PR ouverte sans revue**.
//!
//! Ce fichier épingle les deux moitiés du correctif :
//!
//! - **La décision** — l'incident lui-même rejoué contre la fonction pure, dans
//!   sa forme mesurée : #2333 non-draft (retenue) et #2332 draft de rescue
//!   (écartée, elle relève de `wip_rescue`). La terminaison du pilote n'est pas
//!   une entrée de la fonction : c'est la forme structurelle de « indépendant de
//!   la survie du pilote » (AC1).
//! - **Le câblage** — trois faits de *callsite*, qu'aucune assertion
//!   comportementale ne voit : le scan est routé dans le dispatcher, il est
//!   enregistré comme tâche récurrente derrière son kill-switch (AC7), et le
//!   relecteur qu'il pose est exactement celui que le filtre du gateway laisse
//!   passer (AC9). Un correctif dont la décision est juste et que personne
//!   n'appelle serait vert sur toute la moitié haute.
//!
//! Précédent de forme : `test_merge_identity_2248.rs`, dont le défaut d'origine
//! était lui aussi un callsite plutôt qu'un calcul.

use mika_agent::qa_review_reconcile::{
    GhAuthor, GhReview, GhReviewRequest, PrSnapshot, ReconcileConfig, select_prs_needing_review,
};
use mika_common::forge_identity::{DISPATCHER_FORGE_LOGIN, REVIEWER_FORGE_LOGIN};

/// Sources épinglées à la compilation : les assertions structurelles ne peuvent
/// pas passer contre une copie périmée sur le disque.
const DISPATCHER_SRC: &str = include_str!("../../src/task_engine/dispatcher.rs");
const SERVER_SRC: &str = include_str!("../../src/server/mod.rs");
const GATEWAY_SRC: &str = include_str!("../../../mika-gateway/src/github.rs");

/// `2026-09-15T20:00:00Z` — l'heure à laquelle l'orchestrateur a constaté les
/// deux PRs sans relecteur, soit un peu plus de deux heures après la mort du
/// pilote de #2293 (17:39:29Z).
fn incident_now() -> chrono::DateTime<chrono::Utc> {
    mika_agent::timestamp::parse("2026-09-15T20:00:00Z").unwrap()
}

fn loop_pr(number: u64, created_at: &str, is_draft: bool) -> PrSnapshot {
    PrSnapshot {
        number,
        author: Some(GhAuthor {
            login: DISPATCHER_FORGE_LOGIN.to_string(),
        }),
        is_draft,
        created_at: created_at.to_string(),
        // mika#2347 — le SHA de tête keye désormais le ledger. Il n'est pas une
        // entrée de la décision de *population* ; il ne doit simplement pas être
        // vide, un SHA illisible sortant la PR comme toute autre information
        // manquante.
        head_ref_oid: format!("{number:040x}"),
        review_requests: vec![],
        reviews: vec![],
    }
}

/// AC1 — le test négatif du ticket, sur ses données.
///
/// #2333 a été poussée puis son pilote est mort en `error_during_execution:
/// after_deny` dans `/ce-code-review`. #2332 a été poussée puis son pilote a
/// fini par la voie recovery dirty-worktree (mika#1282), qui ouvre un brouillon.
/// Les deux terminaisons sont différentes et **aucune n'est une entrée de la
/// décision** : ce qui sépare les deux PRs, c'est `isDraft`, pas la façon dont
/// leur pilote est mort.
#[test]
fn mika2334_lincident_fondateur_est_rattrape() {
    let pr_2333 = loop_pr(2333, "2026-09-15T17:30:00Z", false);
    let pr_2332 = loop_pr(2332, "2026-09-15T17:20:00Z", true);

    let picked: Vec<u64> = select_prs_needing_review(
        &[pr_2332, pr_2333],
        incident_now(),
        &ReconcileConfig::default(),
    )
    .into_iter()
    .map(|p| p.number)
    .collect();

    assert_eq!(
        picked,
        vec![2333],
        "la PR non-draft de l'incident doit être rattrapée ; le brouillon de \
         rescue relève de wip_rescue, qui a sa propre voie"
    );
}

/// Le contrôle négatif du test ci-dessus : une fonction qui retiendrait tout le
/// passerait. Une fois le relecteur posé, la PR sort de la population — sur la
/// demande **et** sur la revue, parce que GitHub retire la demande dès que la
/// revue est soumise.
#[test]
fn mika2334_une_pr_deja_servie_ne_revient_jamais() {
    let mut demandee = loop_pr(2333, "2026-09-15T17:30:00Z", false);
    demandee.review_requests = vec![GhReviewRequest {
        login: Some(REVIEWER_FORGE_LOGIN.to_string()),
    }];

    let mut revue = loop_pr(2333, "2026-09-15T17:30:00Z", false);
    revue.reviews = vec![GhReview {
        author: Some(GhAuthor {
            login: REVIEWER_FORGE_LOGIN.to_string(),
        }),
    }];

    for (label, pr) in [("demandée", demandee), ("revue", revue)] {
        assert!(
            select_prs_needing_review(&[pr], incident_now(), &ReconcileConfig::default())
                .is_empty(),
            "une PR déjà {label} ne doit jamais être re-servie — c'est la revue \
             en double que toute cette conception existe pour éviter"
        );
    }
}

/// AC7 (moitié routage) — le scan est appelé. Sans cette ligne, la tâche
/// récurrente se déclencherait toutes les 15 minutes dans le vide.
#[test]
fn mika2334_le_scan_est_route_dans_le_dispatcher() {
    assert!(
        DISPATCHER_SRC
            .contains(r#""qa_review_reconcile" => Ok(self.dispatch_qa_review_reconcile("#),
        "le dispatcher doit router l'action_type qa_review_reconcile"
    );
    assert!(
        DISPATCHER_SRC.contains("qa_review_reconcile::reconcile_qa_review_requests("),
        "le dispatch doit appeler le scan, pas seulement exister"
    );
}

/// AC7 (moitié enregistrement + kill-switch) — la tâche récurrente existe, et
/// `MIKA_QA_REVIEW_RECONCILE=0` l'annule, dans la forme de ses deux voisins.
#[test]
fn mika2334_la_tache_recurrente_est_enregistree_et_desarmable() {
    assert!(
        SERVER_SRC.contains(r#""qa_review_reconcile","#),
        "la tâche récurrente qa_review_reconcile doit être enregistrée"
    );
    assert!(
        SERVER_SRC.contains(r#"std::env::var("MIKA_QA_REVIEW_RECONCILE")"#),
        "le kill-switch MIKA_QA_REVIEW_RECONCILE doit être lu"
    );
    assert!(
        SERVER_SRC.contains(r#"cancel_recurring_task_by_label("qa_review_reconcile")"#),
        "désarmer doit annuler la tâche, pas seulement sauter l'enregistrement — \
         sinon une tâche posée par un démarrage antérieur survit au kill-switch"
    );
}

/// AC9 — le signal est branché de bout en bout.
///
/// `is_suppressed_review_request` (mika#1655) ne laisse passer un
/// `review_requested` que lorsque le relecteur demandé est exactement le login
/// QA. Ce scan pose donc le seul login qui réveille mika-qa — et il l'importe
/// de la même constante que le gateway, plutôt que de le réécrire. Si cette
/// égalité cassait, le rattrapage poserait un relecteur que le gateway
/// supprimerait : une PR « rattrapée » que personne ne vient revoir.
#[test]
fn mika2334_le_relecteur_pose_est_celui_que_le_gateway_route() {
    assert!(
        GATEWAY_SRC.contains("mika_common::forge_identity::REVIEWER_FORGE_LOGIN"),
        "le gateway doit importer la constante partagée, pas en redéfinir une copie"
    );
    assert!(
        GATEWAY_SRC.contains("requested_reviewer != Some(QA_REVIEWER_LOGIN)"),
        "le filtre mika#1655 doit toujours être celui contre lequel ce scan est calibré"
    );
    assert_ne!(
        REVIEWER_FORGE_LOGIN, DISPATCHER_FORGE_LOGIN,
        "relecteur et auteur doivent rester deux identités distinctes"
    );
}
