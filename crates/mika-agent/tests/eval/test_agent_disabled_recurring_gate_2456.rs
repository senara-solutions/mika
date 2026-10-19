//! `enabled = false` éteint les tâches récurrentes de l'agent — mika#2456.
//!
//! **Ce que le ticket a mesuré.** L'agent well-known `mika-test`, porteur de
//! `enabled = false` et censé dormant, portait deux récurrentes **actives**
//! (`heartbeat`, `curator_review`) qui tiraient des tours LLM automatiques :
//! ~5,18 $/22 h sur le tenant canari. La porte `enabled` ne couvrait que le
//! chemin interactif. `curator_review` n'était gardé par *rien* — il revenait à
//! chaque boot complet.
//!
//! **Ce que ces sondes tiennent.** La porte vit en tête de
//! `ensure_recurring_task_with_policy`, avant `retry_on_busy`, et `home_dir`
//! est dans la signature des deux fonctions publiques — donc un site
//! d'enregistrement qui l'esquiverait ne compile pas. Ce que le compilateur ne
//! peut pas dire est *ce que la porte fait*, et c'est l'objet de ce fichier :
//!
//! - **V1** — agent désactivé : rien ne s'enregistre, et une ligne
//!   `recurring_active` préexistante du même label est annulée.
//! - **V2** — **contrôle négatif**, un seul octet d'écart (`enabled = true`) :
//!   la récurrente s'enregistre normalement. C'est la paire V1/V2 qui sépare
//!   « la porte mord » de « rien ne s'enregistre jamais » — sans V2, un bug qui
//!   casserait tout enregistrement rendrait V1 vert.
//! - **V3** — **anti-résurrection**, et son revers. Une ligne annulée par la
//!   porte ne revient pas au boot suivant tant que l'agent est désactivé ; le
//!   vecteur est `revert_config_cancel_recurring_task` (mika#2271), qui lève le
//!   veto mika#1742 sur les lignes `cancelled`, et c'est précisément ce que la
//!   porte ne doit pas atteindre — d'où son refus **avant** d'entrer dans
//!   `register_recurring_task_once`. Le revers compte autant : réactiver
//!   l'agent ramène la récurrente, donc l'annulation de la porte n'est pas un
//!   veto permanent déguisé.
//! - **V7** — `identity.toml` absent, et `identity.toml` illisible : résolution
//!   `true`, provenance `default`. La porte est **soustractive** ; seul un
//!   `false` écrit éteint quelque chose. L'inverse ferait d'une lecture
//!   transitoirement ratée un arrêt muet du substrat.
//! - **V9** — `mika tasks rearm` sur un agent désactivé rend
//!   `RearmError::AgentDisabled` — **pas** `NotRegistered` — et n'écrit ni
//!   ligne d'audit ni marqueur de ré-armement.
//!
//! **Hors périmètre de ce vol, et c'est dit plutôt que tu.** Ces sondes portent
//! sur l'**enregistrement**. Le filet au **tir** (une ligne déjà posée, que la
//! porte n'a pas pu annuler, ne doit pas faire feu sur un agent désactivé) est
//! le vol 2 ; aucun test ici ne le couvre, et son absence n'est pas une
//! omission silencieuse.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask, RECURRING_OPERATOR_REARM_PATH};
use mika_agent::task_engine::{RearmError, ensure_recurring_task, rearm_recurring_task};

const AGENT_ID: &str = "mika-test";
const LABEL: &str = "curator_review";
const CRON: &str = "0 0 3 * * *";
const CONFIG: &str = r#"{"trigger":"curator_review"}"#;

/// Une base portant l'agent du ticket.
///
/// `register_agent` n'est pas décoratif : `tasks.agent_id` porte une contrainte
/// de clé étrangère et `PRAGMA foreign_keys = ON`, donc sans la ligne d'agent
/// `ensure_recurring_task` échoue en silence — il rend `()` — et **toutes** ces
/// sondes passeraient au vert pour la mauvaise raison, la porte comprise.
async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    let db = AsyncDatabase::new_with_agent(db, AGENT_ID);
    db.register_agent(AGENT_ID, AGENT_ID, "/nonexistent")
        .await
        .expect("inscrire l'agent du ticket");
    db
}

/// Un foyer d'agent dont l'`identity.toml` porte exactement `body`.
fn agent_home(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("foyer d'agent temporaire");
    std::fs::write(dir.path().join("identity.toml"), body).expect("écrire identity.toml");
    dir
}

/// La graine de V1 et de V2, à un octet près. Les deux foyers sont produits par
/// la **même** fonction pour que l'écart mesuré soit celui de la clé et non
/// celui de deux montages qui auraient divergé.
fn home_with_enabled(enabled: bool) -> tempfile::TempDir {
    agent_home(&format!(
        "name = \"mika-test\"\nemoji = \"🧪\"\nenabled = {enabled}\n"
    ))
}

/// Tous les statuts portés par ce label, y compris les terminaux : c'est ce qui
/// distingue « rien n'a été créé » de « créé puis annulé », deux issues que la
/// seule absence de ligne active confondrait.
async fn statuses_for(db: &AsyncDatabase, label: &str) -> Vec<String> {
    db.get_tasks_by_status(vec![
        "recurring_active".to_string(),
        "pending".to_string(),
        "in_progress".to_string(),
        "cancelled".to_string(),
        "failed".to_string(),
        "expired".to_string(),
    ])
    .await
    .expect("lire les tâches")
    .into_iter()
    .filter(|t| t.label == label)
    .map(|t| t.status)
    .collect()
}

/// Tue la récurrente de ce label en `failed`, comme le ferait le faucheur.
///
/// `Database::conn` est `pub(crate)`, donc la mort ne s'écrit pas en SQL depuis
/// ici — elle passe par `update_task_status`, qui pose `updated_at = now` et
/// laisse donc la mort **dans** la fenêtre de grâce mika#1742 : le veto est
/// armé, ce qui est exactement l'état où `mika tasks rearm` a une raison
/// d'exister.
async fn kill_recurring(db: &AsyncDatabase, label: &str) {
    let tasks = db
        .get_tasks_by_status(vec!["recurring_active".to_string()])
        .await
        .expect("lire les tâches");
    let target = tasks
        .into_iter()
        .find(|t| t.label == label)
        .expect("la récurrente à tuer doit exister");
    db.update_task_status(&target.id, "failed")
        .await
        .expect("tuer la ligne");
}

/// Y a-t-il, sur une ligne quelconque de ce label, le marqueur posé par
/// `mark_recurring_operator_rearm` ?
async fn has_rearm_marker(db: &AsyncDatabase, label: &str) -> bool {
    let needle = RECURRING_OPERATOR_REARM_PATH.trim_start_matches("$.");
    db.get_tasks_by_status(vec![
        "recurring_active".to_string(),
        "cancelled".to_string(),
        "failed".to_string(),
        "expired".to_string(),
    ])
    .await
    .expect("lire les tâches")
    .into_iter()
    .filter(|t| t.label == label)
    .any(|t| {
        t.metadata
            .as_deref()
            .is_some_and(|m| m.contains(needle) && m.contains('1'))
    })
}

// ───────────────────── V1 — la porte mord, et nettoie ─────────────────────

/// **V1.** Un agent portant `enabled = false` n'enregistre rien, et la ligne
/// `recurring_active` qu'un boot antérieur avait laissée est annulée.
///
/// Les deux moitiés comptent. Refuser l'enregistrement sans annuler la ligne
/// existante laisserait tirer exactement ce que le ticket a mesuré : un agent
/// dormant qui brûle des tours LLM parce que la ligne est *déjà* là.
#[tokio::test]
async fn mika2456_un_agent_desactive_nenregistre_rien_et_sa_ligne_est_annulee() {
    let db = test_db().await;

    // Boot 1 — l'agent est encore activé : la récurrente s'inscrit. C'est
    // l'état de `mika-test` au moment de la mesure du ticket.
    let enabled = home_with_enabled(true);
    ensure_recurring_task(&db, enabled.path(), LABEL, CRON, CONFIG).await;
    assert_eq!(
        statuses_for(&db, LABEL).await,
        vec!["recurring_active".to_string()],
        "précondition : la ligne active doit exister avant que la porte agisse"
    );

    // Boot 2 — l'opérateur a posé `enabled = false`.
    let disabled = home_with_enabled(false);
    ensure_recurring_task(&db, disabled.path(), LABEL, CRON, CONFIG).await;

    let statuses = statuses_for(&db, LABEL).await;
    assert_eq!(
        statuses,
        vec!["cancelled".to_string()],
        "la porte doit annuler la ligne préexistante et n'en créer aucune — \
         statuts observés : {statuses:?}"
    );
    assert!(
        db.get_recurring_task_cron(LABEL)
            .await
            .expect("lire le cron")
            .is_none(),
        "aucune récurrente ne doit rester armée sur un agent désactivé"
    );
}

/// **V1 (second volet)** — sur une base vierge, la porte ne crée rien du tout.
/// Distinct du volet ci-dessus : celui-là mesure le nettoyage, celui-ci mesure
/// qu'un premier boot désactivé ne pose aucune ligne, pas même `cancelled`.
#[tokio::test]
async fn mika2456_un_premier_boot_desactive_ne_cree_aucune_ligne() {
    let db = test_db().await;
    let disabled = home_with_enabled(false);

    ensure_recurring_task(&db, disabled.path(), LABEL, CRON, CONFIG).await;

    let statuses = statuses_for(&db, LABEL).await;
    assert!(
        statuses.is_empty(),
        "un agent désactivé ne doit laisser aucune trace de tâche — \
         statuts observés : {statuses:?}"
    );
}

// ──────────────────── V2 — le contrôle négatif de V1 ─────────────────────

/// **V2.** Même graine que V1, un seul octet d'écart : la récurrente
/// s'enregistre normalement.
///
/// Sans cette sonde, un défaut qui casserait *tout* enregistrement rendrait V1
/// vert et la porte paraîtrait fonctionner. C'est la paire qui mesure, pas V1
/// seul.
#[tokio::test]
async fn mika2456_controle_negatif_un_agent_active_senregistre_normalement() {
    let db = test_db().await;
    let enabled = home_with_enabled(true);

    ensure_recurring_task(&db, enabled.path(), LABEL, CRON, CONFIG).await;

    assert_eq!(
        statuses_for(&db, LABEL).await,
        vec!["recurring_active".to_string()],
        "`enabled = true` ne doit rien gater"
    );
    assert_eq!(
        db.get_recurring_task_cron(LABEL)
            .await
            .expect("lire le cron")
            .as_deref(),
        Some(CRON),
        "le cron enregistré doit être celui demandé"
    );
}

// ─────────────── V3 — anti-résurrection, et sa réversibilité ──────────────

/// **V3.** La ligne annulée par la porte ne revient pas au boot suivant tant
/// que l'agent est désactivé — puis revient dès qu'il est réactivé.
///
/// Le vecteur guetté est `revert_config_cancel_recurring_task` (mika#2271) :
/// il lève le veto mika#1742 sur les lignes `cancelled` pour que le cycle
/// knob-off → knob-on réalimente la boucle. Appliqué à la porte, il
/// ressusciterait précisément ce qu'elle vient d'éteindre. Il n'est jamais
/// atteint parce que le refus est *avant* `register_recurring_task_once`.
///
/// Le revers est dans la même sonde, et pas par économie : « la porte éteint »
/// et « la porte brûle le label » ne se distinguent qu'en réactivant. Un
/// opérateur qui remet `enabled = true` doit retrouver ses récurrentes sans
/// éditer la base.
#[tokio::test]
async fn mika2456_la_porte_ne_ressuscite_pas_et_reste_reversible() {
    let db = test_db().await;
    let enabled = home_with_enabled(true);
    let disabled = home_with_enabled(false);

    // Boot 1 (activé) puis boot 2 (désactivé) : l'état de départ de V3.
    ensure_recurring_task(&db, enabled.path(), LABEL, CRON, CONFIG).await;
    ensure_recurring_task(&db, disabled.path(), LABEL, CRON, CONFIG).await;
    assert_eq!(
        statuses_for(&db, LABEL).await,
        vec!["cancelled".to_string()]
    );

    // Boots 3 et 4, toujours désactivé : rien ne ressuscite. Deux passes
    // plutôt qu'une, parce qu'une résurrection par `revert_` + ré-inscription
    // demande justement deux tours pour se manifester.
    for boot in 3..=4 {
        ensure_recurring_task(&db, disabled.path(), LABEL, CRON, CONFIG).await;
        let statuses = statuses_for(&db, LABEL).await;
        assert_eq!(
            statuses,
            vec!["cancelled".to_string()],
            "boot {boot} : la ligne annulée par la porte ne doit pas revenir — \
             statuts observés : {statuses:?}"
        );
    }

    // Boot 5 — l'opérateur réactive l'agent : la récurrente revient.
    ensure_recurring_task(&db, enabled.path(), LABEL, CRON, CONFIG).await;
    let statuses = statuses_for(&db, LABEL).await;
    assert!(
        statuses.iter().any(|s| s == "recurring_active"),
        "réactiver l'agent doit ramener la récurrente : l'annulation de la porte \
         est un réglage, pas un veto permanent — statuts observés : {statuses:?}"
    );
}

// ──────────── V7 — l'absence et l'illisible valent `true` ────────────────

/// **V7.** `identity.toml` absent : résolution `(true, "default")`, et la
/// récurrente s'enregistre. C'est l'état de toute la flotte — R-2 et R-6
/// disent qu'aucun agent du dépôt ne porte la clé, donc ce cas *est* le cas
/// nominal, et une régression ici éteindrait le substrat entier.
#[tokio::test]
async fn mika2456_une_identity_absente_resout_true_par_defaut() {
    let dir = tempfile::tempdir().expect("foyer d'agent temporaire");
    assert!(
        !dir.path().join("identity.toml").exists(),
        "précondition : le foyer ne porte pas d'identity.toml"
    );

    let identity = mika_agent::prompt::load_identity_async(dir.path()).await;
    assert_eq!(
        identity.recurring_tasks_enabled(),
        (true, "default"),
        "une clé absente vaut `true`, et le dit : la provenance `default` est \
         ce qui distingue un défaut d'un `true` écrit à la main (mika#2331)"
    );

    let db = test_db().await;
    ensure_recurring_task(&db, dir.path(), LABEL, CRON, CONFIG).await;
    assert_eq!(
        statuses_for(&db, LABEL).await,
        vec!["recurring_active".to_string()],
        "la flotte sans la clé doit continuer d'enregistrer ses récurrentes"
    );
}

/// **V7 (second volet)** — `identity.toml` illisible (TOML invalide) : encore
/// `true`.
///
/// Délibéré, et c'est le point délicat du ticket. Le repli fail-closed de
/// mika#2027 protège la *permission* — skills évincés, outils mutants refusés.
/// Il ne doit pas éteindre les récurrentes : sinon une lecture transitoirement
/// ratée arrête le substrat, et l'arrête en silence. Une absence n'échoue pas
/// fermé (mika#2023) ; seul un `false` écrit éteint.
#[tokio::test]
async fn mika2456_une_identity_illisible_resout_true_par_defaut() {
    let dir = agent_home("ceci n'est pas du TOML { ] [\n");

    let identity = mika_agent::prompt::load_identity_async(dir.path()).await;
    assert_eq!(
        identity.recurring_tasks_enabled(),
        (true, "default"),
        "un identity.toml illisible ne doit pas éteindre les récurrentes"
    );

    let db = test_db().await;
    ensure_recurring_task(&db, dir.path(), LABEL, CRON, CONFIG).await;
    assert_eq!(
        statuses_for(&db, LABEL).await,
        vec!["recurring_active".to_string()],
        "la porte est soustractive : elle n'éteint que sur un `false` écrit"
    );
}

// ──────────────── V9 — `mika tasks rearm` refuse, et nommément ────────────

/// **V9.** `mika tasks rearm` sur un agent désactivé rend
/// `RearmError::AgentDisabled`, et n'écrit ni ligne d'audit ni marqueur.
///
/// **Pourquoi pas `NotRegistered`.** Sans ce bras, la séquence serait : tracer
/// l'acte, poser le marqueur, ré-enregistrer, voir la porte annuler, puis
/// rendre `NotRegistered` — dont le message envoie lire un WARN mika#1742 qui
/// n'existe pas. L'opérateur chercherait un défaut là où il y a un réglage, et
/// l'`audit_events` dirait « ré-armé » pour une récurrente qui ne tirera
/// jamais. Le refus est donc au même rang que `NotRoutable` : avant la trace et
/// avant le marqueur.
#[tokio::test]
async fn mika2456_rearm_refuse_un_agent_desactive_sans_tracer_ni_marquer() {
    let db = test_db().await;
    let enabled = home_with_enabled(true);
    let disabled = home_with_enabled(false);

    // Une ligne morte à ré-armer : inscrite agent activé, puis tuée `failed`
    // dans la fenêtre de grâce — l'état qui arme le veto mika#1742 et qui donne
    // à `mika tasks rearm` sa raison d'exister.
    ensure_recurring_task(&db, enabled.path(), LABEL, CRON, CONFIG).await;
    kill_recurring(&db, LABEL).await;

    let err = rearm_recurring_task(&db, disabled.path(), LABEL)
        .await
        .expect_err("un agent désactivé doit être refusé");

    match &err {
        RearmError::AgentDisabled { agent_id, label } => {
            assert_eq!(agent_id, AGENT_ID);
            assert_eq!(label, LABEL);
        }
        other => panic!(
            "attendu AgentDisabled — `NotRegistered` enverrait l'opérateur \
             chercher un WARN mika#1742 inexistant ; obtenu {other:?}"
        ),
    }
    let rendered = err.to_string();
    assert!(
        rendered.contains("enabled = false") && rendered.contains("identity.toml"),
        "le refus doit nommer la clé et le fichier à éditer : {rendered}"
    );

    assert_eq!(
        db.count_audit_events_by_tool_name("recurring_operator_rearm")
            .await
            .expect("compter les événements d'audit"),
        0,
        "un refus ne trace pas d'acte"
    );
    assert!(
        !has_rearm_marker(&db, LABEL).await,
        "un refus ne pose pas le marqueur de ré-armement"
    );
    assert_eq!(
        statuses_for(&db, LABEL).await,
        vec!["failed".to_string()],
        "la ligne morte reste morte, et son veto reste armé"
    );

    // Contrôle négatif du refus : le même geste, agent activé, aboutit. Sans
    // lui, un `rearm` cassé pour une autre raison rendrait V9 vert.
    let outcome = rearm_recurring_task(&db, enabled.path(), LABEL)
        .await
        .expect("agent activé : le ré-armement doit aboutir");
    assert_eq!(outcome.label, LABEL);
    assert_eq!(outcome.trigger, "curator_review");
}

/// La porte n'escalade pas hors des récurrentes : son prédicat ne regarde que
/// `trigger_type = 'recurring'`, et la surface interactive — `mika ask --agent
/// mika-test` — n'est pas touchée (§3.5).
///
/// Ce que cette sonde peut tenir depuis ici est l'invariance des lignes
/// **non récurrentes** : une tâche `pending` ordinaire survit à un boot
/// d'agent désactivé, là où son homonyme récurrent est annulé.
#[tokio::test]
async fn mika2456_la_porte_ne_touche_que_les_recurrentes() {
    let db = test_db().await;
    let disabled = home_with_enabled(false);

    let one_shot = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: LABEL.to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        })
        .await
        .expect("créer une tâche ponctuelle");

    ensure_recurring_task(&db, disabled.path(), LABEL, CRON, CONFIG).await;

    let task = db
        .get_task(&one_shot)
        .await
        .expect("relire la tâche")
        .expect("la tâche ponctuelle doit exister");
    assert_eq!(
        task.status, "pending",
        "la porte ne doit pas annuler une tâche non récurrente qui partage le \
         label — son prédicat est `trigger_type = 'recurring'`"
    );
}
