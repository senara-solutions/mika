//! Le réconciliateur ne peut plus redemander une revue indéfiniment — mika#2347.
//!
//! # Le défaut
//!
//! Depuis que `qa_review_reconcile` est effectif (mika#2341), les revues QA
//! atteignent la limite de leur enveloppe et postent des `hold[review]` « aucune
//! conclusion » (le filet mika#2276 M2) au lieu de verdicts. La mesure faite dans
//! le code, elle, est plus simple et ne dépend d'aucune horloge : **rien ne
//! bornait le nombre de re-poses**. Les seuls termes d'idempotence de mika#2334
//! étaient *l'absence de demande* et *l'absence de revue* pour le relecteur —
//! deux états **GitHub qui n'existent qu'après aboutissement de la revue**. Une
//! PR dont le tour de revue meurt sans rien poster retombait dans la population
//! au tick suivant, identique à elle-même : avec le cron `0 */15 * * * *` et un
//! cap de 3, jusqu'à **96 re-demandes par jour et par PR** pendant sept jours.
//! La ligne d'audit `qa_review_reconciled` était écrite et **jamais relue**.
//!
//! # Ce que ce fichier épingle
//!
//! - **AC1/AC2/AC3/AC6/AC7** — le ledger décisionnel, sur la vraie base et par le
//!   vrai chemin d'écriture (`log_reconciled`), jamais contre une clé réécrite à
//!   la main : une assertion qui recopie le format de clé passerait encore le
//!   jour où la production en change.
//! - **AC4 (sérialisation)** — deux assertions sur le **moteur**, pas sur le
//!   réconciliateur. « Au plus une revue QA active par agent » est déjà un
//!   invariant : le scan ne déclenche aucun tour, il pose un relecteur et
//!   l'événement `review_requested` retombe dans la file bornée mika#1870,
//!   drainée par **un seul worker par agent** qui prend `agent_lock`. Écrire un
//!   verrou de concurrence dans `qa_review_reconcile` serait un placebo — le scan
//!   ne détient aucun tour. Ce fichier **épingle** l'invariant, il ne
//!   l'implémente pas.
//! - **AC5/AC8** — le cap arithmétique et le fait que l'enveloppe agent ne bouge
//!   pas.
//!
//! # Ce que ce correctif ne prouve PAS
//!
//! Les preuves horaires du ticket ne sont pas compatibles avec le réconciliateur
//! comme source unique du churn : deux `hold[review]` sur #2344 à onze minutes
//! d'écart ne peuvent pas venir d'un scan dont l'intervalle est de quinze
//! minutes, et le premier `hold` **est une revue postée**, ce qui sort la PR de
//! la population dès le tick suivant. `pull_request.synchronize`, le fan-out
//! `check_suite` et un rejeu de file restent en lice, et sont hors périmètre.
//! Ce correctif réduit le nombre de revues *demandées* ; il ne rend aucun tour
//! plus rapide.

use chrono::{Duration, Utc};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::qa_review_reconcile::{
    PrRef, ReconcileConfig, ReviewLedgerVerdict, log_reconciled, reconciled_audit_key,
    review_ledger_verdict,
};

/// Sources épinglées à la compilation — une assertion structurelle ne doit pas
/// pouvoir passer contre une copie périmée sur le disque.
const HANDLERS_SRC: &str = include_str!("../../src/server/handlers.rs");
const STATE_SRC: &str = include_str!("../../src/server/state.rs");
const SERVER_SRC: &str = include_str!("../../src/server/mod.rs");
const RECONCILE_SRC: &str = include_str!("../../src/qa_review_reconcile.rs");

const REPO: &str = "senara-solutions/mika";
const SESSION: &str = "session-2347";
const TRACE: &str = "trace-2347";

fn db() -> AsyncDatabase {
    AsyncDatabase::new(Database::open_in_memory().expect("base en mémoire"))
}

fn pr(number: u64, head_sha: &str) -> PrRef {
    PrRef {
        number,
        age_secs: 7200,
        head_sha: head_sha.to_string(),
    }
}

/// Une pose écrite par **le chemin de production**, pas par une clé recopiée.
async fn seed_pose(db: &AsyncDatabase, pr: &PrRef) {
    log_reconciled(db, SESSION, REPO, pr, TRACE).await;
}

async fn verdict_at(
    db: &AsyncDatabase,
    pr: &PrRef,
    cfg: &ReconcileConfig,
    now: chrono::DateTime<Utc>,
) -> ReviewLedgerVerdict {
    review_ledger_verdict(db, REPO, pr, cfg, now, TRACE).await
}

async fn verdict(db: &AsyncDatabase, pr: &PrRef, cfg: &ReconcileConfig) -> ReviewLedgerVerdict {
    verdict_at(db, pr, cfg, Utc::now()).await
}

// ---------------------------------------------------------------------------
// AC1 / AC2 — idempotence et cooldown par (PR, head SHA)
// ---------------------------------------------------------------------------

/// AC1 — une PR dont le ledger porte une pose pour le SHA courant, à l'intérieur
/// du cooldown, n'est pas re-posée. C'est l'assertion que rien ne faisait avant :
/// le ledger existait et ne décidait rien.
#[tokio::test]
async fn mika2347_une_pose_recente_bloque_la_suivante() {
    let db = db();
    let cfg = ReconcileConfig::default();
    let pr = pr(2343, "aaaa1111");

    assert_eq!(
        verdict(&db, &pr, &cfg).await,
        ReviewLedgerVerdict::Postable,
        "un ledger vide doit laisser poser"
    );

    seed_pose(&db, &pr).await;

    assert_eq!(
        verdict(&db, &pr, &cfg).await,
        ReviewLedgerVerdict::Cooldown,
        "une pose à l'intérieur du cooldown doit sortir la PR du tick"
    );
}

/// AC2 (moitié haute) — le cooldown expire. Sans cette assertion, un verdict qui
/// rendrait toujours `Cooldown` passerait le test ci-dessus, et le rattrapage
/// serait mort au premier tick.
#[tokio::test]
async fn mika2347_le_cooldown_expire() {
    let db = db();
    let cfg = ReconcileConfig::default();
    let pr = pr(2343, "aaaa1111");
    seed_pose(&db, &pr).await;

    let apres = Utc::now() + Duration::seconds(cfg.cooldown_secs + 60);
    assert_eq!(
        verdict_at(&db, &pr, &cfg, apres).await,
        ReviewLedgerVerdict::Postable,
        "passé le cooldown, une seconde chance reste due (le budget n'est pas épuisé)"
    );
}

/// AC2 (moitié basse) — **un nouveau SHA est posable immédiatement.** C'est
/// l'idempotence « par (PR, SHA) » que le ticket demande, obtenue par la forme de
/// la clé plutôt que par un champ de plus : la clé change, le compte repart à
/// zéro. Une nouvelle poussée mérite une nouvelle revue.
#[tokio::test]
async fn mika2347_un_nouveau_sha_rouvre_le_budget() {
    let db = db();
    let cfg = ReconcileConfig::default();
    let ancien = pr(2343, "aaaa1111");

    // Budget entièrement consommé sur l'ancien SHA.
    for _ in 0..cfg.max_attempts {
        seed_pose(&db, &ancien).await;
    }
    assert!(
        matches!(
            verdict(&db, &ancien, &cfg).await,
            ReviewLedgerVerdict::Abandoned { .. }
        ),
        "l'ancien SHA doit être abandonné"
    );

    let nouveau = pr(2343, "bbbb2222");
    assert_eq!(
        verdict(&db, &nouveau, &cfg).await,
        ReviewLedgerVerdict::Postable,
        "le même numéro de PR sur un nouveau SHA doit être immédiatement posable"
    );
}

// ---------------------------------------------------------------------------
// AC3 — pas de replay en boucle
// ---------------------------------------------------------------------------

/// AC3 — au-delà de `max_attempts` poses pour un même (PR, SHA), la PR est
/// abandonnée pour ce SHA.
///
/// **C'est ce qui distingue ce correctif d'un simple ralentissement** : un
/// cooldown seul rejoue pour toujours, juste moins vite. Précédent direct du
/// dépôt : `MIKA_AUTO_PULL_MAX_REDRIVES` (mika#2020), né du constat que mika#1901
/// avait reçu le label `ready` seize fois en dix-neuf heures.
#[tokio::test]
async fn mika2347_le_budget_sepuise_et_la_pr_est_abandonnee() {
    let db = db();
    let cfg = ReconcileConfig::default();
    let pr = pr(2344, "cccc3333");

    for n in 1..=cfg.max_attempts {
        seed_pose(&db, &pr).await;
        // Chaque pose est hors cooldown pour isoler le budget de la cadence.
        let plus_tard = Utc::now() + Duration::seconds(cfg.cooldown_secs * (n + 1));
        let v = verdict_at(&db, &pr, &cfg, plus_tard).await;
        if n < cfg.max_attempts {
            assert_eq!(
                v,
                ReviewLedgerVerdict::Postable,
                "pose {n}/{} : le budget n'est pas encore épuisé",
                cfg.max_attempts
            );
        } else {
            assert_eq!(
                v,
                ReviewLedgerVerdict::Abandoned {
                    attempts: cfg.max_attempts
                },
                "au plafond, la PR doit être abandonnée pour ce SHA"
            );
        }
    }
}

/// Le budget ne se recharge pas avec le temps : c'est un abandon, pas un
/// ralentissement. Contrôle négatif du test de cooldown ci-dessus, qui passerait
/// encore si le budget était lu dans la fenêtre du cooldown.
#[tokio::test]
async fn mika2347_un_abandon_ne_se_recharge_pas_en_attendant() {
    let db = db();
    let cfg = ReconcileConfig::default();
    let pr = pr(2345, "dddd4444");
    for _ in 0..cfg.max_attempts {
        seed_pose(&db, &pr).await;
    }

    let bien_plus_tard = Utc::now() + Duration::seconds(cfg.cooldown_secs * 24);
    assert!(
        matches!(
            verdict_at(&db, &pr, &cfg, bien_plus_tard).await,
            ReviewLedgerVerdict::Abandoned { .. }
        ),
        "attendre ne doit pas rendre son budget à un SHA abandonné"
    );
}

// ---------------------------------------------------------------------------
// Transition des lignes héritées (mika#2334 → mika#2347)
// ---------------------------------------------------------------------------

/// Les poses écrites avant ce correctif portent `pr:{repo}#{n}` sans SHA. Sans
/// cette deuxième lecture, elles seraient invisibles au nouveau cooldown, ce qui
/// autoriserait une re-pose immédiate sur des PRs déjà rattrapées — exactement le
/// doublon que tout ce conditionnement existe pour éviter.
#[tokio::test]
async fn mika2347_une_pose_heritee_sans_sha_compte_encore() {
    let db = db();
    let cfg = ReconcileConfig::default();
    let pr = pr(2346, "eeee5555");

    db.log_audit_event(
        SESSION,
        "qa_review_reconciled",
        &format!("pr:{REPO}#{}", pr.number),
        None,
        Some("7200"),
        Some("ligne héritée mika#2334, sans SHA"),
        Some(TRACE),
    )
    .await
    .expect("écriture de la ligne héritée");

    assert_eq!(
        verdict(&db, &pr, &cfg).await,
        ReviewLedgerVerdict::Cooldown,
        "une pose héritée doit tenir la PR pendant son cooldown"
    );
}

/// La lecture de la clé héritée est **exacte, jamais un `LIKE`** : `#234` ne doit
/// pas matcher `#2343`. Une recherche par préfixe confondrait des PRs voisines et
/// tiendrait des PRs qui n'ont jamais été servies.
#[tokio::test]
async fn mika2347_la_cle_heritee_ne_matche_pas_par_prefixe() {
    let db = db();
    let cfg = ReconcileConfig::default();

    db.log_audit_event(
        SESSION,
        "qa_review_reconciled",
        &format!("pr:{REPO}#234"),
        None,
        Some("7200"),
        Some("ligne héritée sur une PR voisine"),
        Some(TRACE),
    )
    .await
    .expect("écriture");

    assert_eq!(
        verdict(&db, &pr(2343, "ffff6666"), &cfg).await,
        ReviewLedgerVerdict::Postable,
        "la PR #2343 ne doit pas être tenue par une ligne de la PR #234"
    );
}

// ---------------------------------------------------------------------------
// AC6 — fail-closed sur ledger illisible
// ---------------------------------------------------------------------------

/// AC6 — une erreur de lecture d'`audit_events` empêche la pose.
///
/// C'est l'inverse du choix de `ci_success_handler` (fail-open) et le même que
/// celui de `wip_rescue` (mika#2199), pour la raison qui y est écrite : un faux
/// négatif fait attendre une PR qui, avant mika#2334, attendait indéfiniment ;
/// un faux positif rejoue une revue, c'est-à-dire produit le défaut que ce ticket
/// existe pour fermer.
#[tokio::test]
async fn mika2347_un_ledger_illisible_refuse_la_pose() {
    let db = db();
    db.with_db(|d| {
        d.execute_sql("DROP TABLE audit_events", &[])?;
        Ok(())
    })
    .await
    .expect("suppression de la table d'audit");

    assert_eq!(
        verdict(&db, &pr(2343, "aaaa1111"), &ReconcileConfig::default()).await,
        ReviewLedgerVerdict::Unreadable,
        "un ledger illisible doit refuser la pose, pas l'autoriser par défaut"
    );
}

// ---------------------------------------------------------------------------
// AC4 — sérialisation : un invariant du moteur, épinglé
// ---------------------------------------------------------------------------

/// AC4 (moitié comportementale) — deux `review_requested` pour le même agent
/// produisent **deux entrées de file distinctes**, consommées l'une après
/// l'autre. Ni fusionnées (ce qui perdrait une revue) ni concurrentes (ce qui les
/// ralentirait mutuellement, la lecture du ticket).
///
/// `classify_event` range `review_requested` dans `Other`, dont la
/// `coalescing_key` rend `None` : « préserver l'ordre, ne jamais fusionner ».
#[tokio::test]
async fn mika2347_deux_demandes_de_revue_font_deux_entrees_sequentielles() {
    use mika_agent::server::webhook_queue_v2::{
        EnqueueResult, WebhookQueue, classify_event, coalescing_key,
    };

    use mika_agent::server::types::MessageRequest;

    let texte =
        |n: u64| format!("[GitHub] PR review_requested: {REPO}#{n}\nreviewer: mika-platform-qa");

    for n in [2343u64, 2344] {
        let kind = classify_event(&texte(n));
        assert!(
            coalescing_key(&kind).is_none(),
            "un review_requested ne doit jamais coalescer : chacun doit réveiller une revue"
        );
    }

    let queue = WebhookQueue::new(64, std::time::Duration::from_millis(100));
    for n in [2343u64, 2344] {
        let req = MessageRequest {
            text: texte(n),
            chat_id: None,
            channel: "github".to_string(),
            request_id: format!("req-{n}"),
            agent: "mika-qa".to_string(),
            images: None,
        };
        assert!(
            matches!(queue.enqueue(req).await, EnqueueResult::Enqueued { .. }),
            "chaque demande doit prendre sa propre place dans la file"
        );
    }

    let premier = queue.dequeue().await.expect("première entrée");
    let second = queue.dequeue().await.expect("seconde entrée");
    assert!(
        premier.request.text.contains("#2343") && second.request.text.contains("#2344"),
        "les deux entrées doivent être drainées l'une après l'autre, dans l'ordre"
    );
}

/// AC4 (moitié structurelle) — **un seul consommateur du verrou par agent.**
///
/// Un test comportemental ne verrait pas la régression qui compte ici : un second
/// consommateur ne rendrait aucune décision fausse, il lèverait l'invariant en
/// silence, et toutes les assertions de file resteraient vertes pendant que deux
/// tours de revue se ralentiraient mutuellement.
///
/// Les deux faits : `run_agent_for_message` **reçoit** le garde `agent_lock` en
/// paramètre plutôt que de le prendre lui-même (donc l'acquisition a un site
/// unique, celui du worker de drain), et ce worker n'est spawné que depuis les
/// deux chemins de construction d'un `AgentState` — le boot et la résolution
/// paresseuse de mika#1399.
#[test]
fn mika2347_un_seul_consommateur_du_verrou_par_agent() {
    assert!(
        HANDLERS_SRC.contains("lock: tokio::sync::OwnedMutexGuard<()>,"),
        "run_agent_for_message doit recevoir le garde, pas le prendre : \
         sinon l'acquisition a plusieurs sites et l'invariant n'est plus lisible"
    );

    let sites = HANDLERS_SRC
        .matches("pub(super) fn spawn_webhook_drain_worker")
        .count();
    assert_eq!(
        sites, 1,
        "le worker de drain doit avoir une seule définition"
    );

    let spawns = STATE_SRC.matches("spawn_webhook_drain_worker(").count()
        + SERVER_SRC.matches("spawn_webhook_drain_worker(").count();
    assert_eq!(
        spawns, 2,
        "exactement deux sites de spawn — le boot et la résolution paresseuse \
         (mika#1399), un par agent ; un troisième doublerait le consommateur du \
         verrou et ferait tourner deux tours en parallèle sur le même agent"
    );
}

// ---------------------------------------------------------------------------
// AC5 / AC8 — le cap descend, l'enveloppe ne bouge pas
// ---------------------------------------------------------------------------

/// AC5 — le réconciliateur ne sur-demande pas. Trois poses par tick et quatre
/// ticks par heure font 12 demandes/heure ; à 600 s par revue et une exécution
/// sérialisée, la capacité de mika-qa plafonne à 6 revues/heure. Le cap à 1 fait
/// passer la demande de rattrapage à 4/heure, sous la capacité.
#[test]
fn mika2347_le_cap_par_defaut_tient_sous_la_capacite() {
    let cfg = ReconcileConfig::default();
    assert_eq!(cfg.max_per_tick, 1);

    const TICKS_PAR_HEURE: usize = 4; // cron `0 */15 * * * *`
    const CAPACITE_REVUES_PAR_HEURE: usize = 6; // 3600 s / enveloppe 600 s
    assert!(
        cfg.max_per_tick * TICKS_PAR_HEURE < CAPACITE_REVUES_PAR_HEURE,
        "le rattrapage doit demander strictement moins que la capacité, \
         puisque le trafic nominal s'ajoute par-dessus"
    );
}

/// AC8 — l'enveloppe ne bouge pas. Bearing Prime explicite : le correctif est
/// côté reconcile. Ce module ne touche à aucun budget de temps.
#[test]
fn mika2347_lenveloppe_agent_nest_pas_touchee() {
    for interdit in [
        "MIKA_AGENT_TOTAL_TIMEOUT_SECS",
        "agent_total_timeout_secs",
        "MIKA_LLM_HTTP_TIMEOUT_SECS",
    ] {
        assert!(
            !RECONCILE_SRC.contains(interdit),
            "qa_review_reconcile ne doit pas connaître {interdit} : \
             le bearing interdit d'élargir l'enveloppe, et un correctif qui la \
             touche répond à un autre ticket"
        );
    }
}

/// La clé d'audit garde son préfixe : la requête opérateur
/// `WHERE tool_name = 'qa_review_reconciled'` est inchangée, et un `LIKE
/// 'pr:{repo}#{n}%'` continue de rendre l'historique d'une PR à travers ses SHAs.
#[test]
fn mika2347_la_cle_daudit_reste_prefixee_par_la_pr() {
    assert_eq!(
        reconciled_audit_key(REPO, 2343, "deadbeef"),
        format!("pr:{REPO}#2343@deadbeef")
    );
}
