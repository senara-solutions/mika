//! mika#2242 — **un dé-groomage est un fait estampillé par son producteur.**
//!
//! # Le défaut mesuré (2026-09-08)
//!
//! Fermer une PR **umbrella** qui déclare `Closes #<sous-ticket>` dé-groome ses
//! sous-tickets : le plan et le marqueur `GROOMED` vivent sur la branche de
//! l'umbrella, jamais sur le corps de l'issue du sous-ticket. #2131 a conservé
//! `ready` sans aucun callout, et chaque dispatch a re-routé vers `groom`. Le
//! travail revu — un plan plus deux passes architecte — vivait sur une branche
//! que **rien ne désignait**, et l'opérateur a dû le retrouver à la main.
//!
//! # Ce que ces tests nomment, et ce qu'ils ne nomment PAS
//!
//! *Le routage ne change pas.* Un ticket dé-groomé continue de partir en
//! `groom`, sous le même `dispatch_class` et la même `ReadyLabelGate` (T6/AC5).
//! Ce qui est ajouté est une **attribution** : un enregistrement durable écrit à
//! l'instant où le lien umbrella → sous-ticket est lisible, et une ligne qui le
//! relit au routage.
//!
//! # Pourquoi les contrôles négatifs sont les tests porteurs
//!
//! Sans eux, un marqueur écrit **inconditionnellement** (T2) et un lecteur
//! firant sur **tout** ticket non groomé (T4b) passeraient chaque assertion
//! positive — en produisant exactement le bruit qui noierait le signal que ce
//! ticket existe pour lever. Un filet qui porte le trafic nominal a en plus
//! effacé la mesure qui permettrait de le voir (mika#2334).
//!
//! # Rouge-avant
//!
//! Recette d'injection : retirer le bloc `if !merged { record_… }` de
//! `handle_pr_closed` — T1, T3 et T6 rougissent. Retirer l'appel à
//! `note_degroomed_ticket` de `try_handle_ready_label_dispatch_inner` — T3
//! rougit seul, ce qui distingue « le producteur n'écrit pas » de « le lecteur
//! ne relit pas ».

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::server::ready_label_handler::try_handle_ready_label_dispatch_with_fetcher;
use mika_agent::server::upstream_close_handler::{
    CLOSING_PR_CLOSED_UNMERGED_TOOL, degroom_marker_key, try_handle_upstream_close,
};
use mika_agent::server::verdict_handler::VerdictAction;
use mika_agent::skills::SkillRegistry;
use mika_agent::task_state::tasks::{NewTask, UPSTREAM_PR_CLOSED_UNMERGED};

const AGENT_ID: &str = "mika";
const SESSION: &str = "degroom-2242-session";
const TRACE: &str = "degroom-2242-trace";

const OWNER_REPO: &str = "senara-solutions/mika";
const SUB_ISSUE: u64 = 2131;
const UMBRELLA_PR: u64 = 2226;
const UMBRELLA_BRANCH: &str = "fix/umbrella-auto-pull-exclusion-observability";

const READY_LABEL_DEGROOMED_TOOL: &str = "ready_label_degroomed";

/// La fermeture mesurée : PR umbrella #2226, **sans merge**, déclarant
/// `Closes #2131`, sur la branche qui porte le plan groomé.
fn unmerged_close_event() -> String {
    format!(
        "[GitHub] PR closed: {OWNER_REPO}#{UMBRELLA_PR} — fix(umbrella): observabilité \
         des exclusions (branch: {UMBRELLA_BRANCH})\n\
         https://github.com/{OWNER_REPO}/pull/{UMBRELLA_PR}\n\
         Merged: false\n\n\
         Décomposition ratifiée par Prime : pas d'umbrella.\n\nCloses #{SUB_ISSUE}"
    )
}

/// Le **contrôle négatif** de T2 : strictement le même texte, `Merged: true`.
fn merged_close_event() -> String {
    unmerged_close_event().replace("Merged: false", "Merged: true")
}

/// Un corps d'issue portant les trois callouts canoniques : `is_groomed` vrai,
/// donc le lecteur doit rester muet quoi que porte le registre.
const GROOMED_BODY: &str = "> - **Branch:** `fix/2131/x`\n\
     > - **Plan:** `docs/plans/2026-09-07-002-fix-2131-x-plan.md`\n\
     > - **Grooming history:** second-pass (GROOMED)\n\nCorps.";

/// Un corps sans aucun callout : le routage tombe en `groom`.
const UNGROOMED_BODY: &str = "Observation mesurée.\n\nPas de callout ici.";

fn ready_label_event() -> String {
    format!(
        "[GitHub] Issue labeled ready on {OWNER_REPO}#{SUB_ISSUE} — observabilité\n\
         https://github.com/{OWNER_REPO}/issues/{SUB_ISSUE}"
    )
}

fn issue_url() -> String {
    format!("https://github.com/{OWNER_REPO}/issues/{SUB_ISSUE}")
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Les `(tool_name, target_key, after_value, reasoning)` écrits pendant le tour.
async fn audit_rows(db: &AsyncDatabase) -> Vec<(String, String, Option<String>, Option<String>)> {
    db.get_audit_events(SESSION)
        .await
        .expect("read audit events")
        .into_iter()
        .map(|e| (e.tool_name, e.target_key, e.after_value, e.reasoning))
        .collect()
}

async fn rows_named(
    db: &AsyncDatabase,
    tool: &str,
) -> Vec<(String, Option<String>, Option<String>)> {
    audit_rows(db)
        .await
        .into_iter()
        .filter(|(t, ..)| t == tool)
        .map(|(_, key, after, reasoning)| (key, after, reasoning))
        .collect()
}

/// Joue le handler de fermeture amont sur `text`.
async fn run_close(db: &AsyncDatabase, text: &str) {
    let action = try_handle_upstream_close(text, db, SESSION, TRACE).await;
    // Contrat inchangé : side-effect-only, toujours `Passthrough` (R2).
    assert!(
        matches!(action, VerdictAction::Passthrough { enrichment: None }),
        "le handler de fermeture reste side-effect-only"
    );
}

/// Joue le handler de ready-label sur un corps d'issue donné.
async fn run_ready_label(db: &AsyncDatabase, body: &'static str) -> VerdictAction {
    let sender: Arc<dyn MessageSender> = Arc::new(NoopSender);
    let skills = SkillRegistry::empty();
    // mika#2049 — un home réel sans stamp « relais à terre », pour que la porte
    // 2d lise « le relais sert » et que l'étape 5 soit atteinte.
    let home = tempfile::tempdir().expect("temp home");
    try_handle_ready_label_dispatch_with_fetcher(
        &ready_label_event(),
        db,
        Some("fake-token"),
        Some(&sender),
        SESSION,
        TRACE,
        &skills,
        home.path(),
        move |_owner_repo, _number, _token| async move { Ok((body.to_string(), Vec::new())) },
    )
    .await
}

// ───────────────────────── AC1 / AC2 — le producteur ─────────────────────────

/// **AC1.** Une fermeture sans merge déclarant `Closes #N` laisse une ligne
/// durable pour `#N`, nommant la PR **et sa branche de tête**.
///
/// La branche est la moitié qui sauve le travail revu : sans elle la ligne dit
/// « dé-groomé » et laisse l'archéologie que #2131 a coûtée.
#[tokio::test]
async fn mika2242_t1_an_unmerged_close_stamps_the_marker() {
    let db = test_db();
    run_close(&db, &unmerged_close_event()).await;

    let rows = rows_named(&db, CLOSING_PR_CLOSED_UNMERGED_TOOL).await;
    assert_eq!(rows.len(), 1, "une ligne, une seule : {rows:?}");

    let (key, after, reasoning) = &rows[0];
    assert_eq!(*key, degroom_marker_key(OWNER_REPO, SUB_ISSUE));
    assert_eq!(
        after.as_deref(),
        Some("pr#2226"),
        "`after_value` est le format de fil du `GROUP BY` opérateur"
    );
    let reasoning = reasoning.as_deref().expect("reasoning présent");
    assert!(
        reasoning.contains(&format!("head_branch={UMBRELLA_BRANCH}")),
        "la branche portant le plan doit être nommée : {reasoning}"
    );
    assert!(
        reasoning.contains(&format!("pr_url=https://github.com/{OWNER_REPO}/pull/2226")),
        "l'URL de la PR doit être nommée : {reasoning}"
    );
    assert!(reasoning.contains("merged=false"), "{reasoning}");
}

/// **AC2 — le contrôle négatif porteur.** Le même texte avec `Merged: true`
/// n'écrit **rien**.
///
/// Sans lui, un marqueur écrit inconditionnellement passerait T1 tout en
/// comptant chaque merge de la boucle — c'est-à-dire en noyant le signal.
#[tokio::test]
async fn mika2242_t2_a_merged_close_writes_nothing() {
    let db = test_db();
    run_close(&db, &merged_close_event()).await;

    let rows = rows_named(&db, CLOSING_PR_CLOSED_UNMERGED_TOOL).await;
    assert!(
        rows.is_empty(),
        "une PR mergée ferme son issue : il n'y a pas de dé-groomage à attribuer — {rows:?}"
    );
}

/// Plusieurs `Closes #N` sur une même fermeture : une ligne par sous-ticket.
/// Un plan d'umbrella couvre N sous-tickets, et c'est la forme visée.
#[tokio::test]
async fn mika2242_t1b_every_closing_ref_gets_its_own_marker() {
    let db = test_db();
    let text = unmerged_close_event().replace(
        &format!("Closes #{SUB_ISSUE}"),
        &format!("Closes #{SUB_ISSUE}, closes #1403 and Resolves #1651"),
    );
    run_close(&db, &text).await;

    // L'assertion porte sur la POPULATION, jamais sur l'ordre de lecture :
    // `audit_events.created_at` est à la seconde, donc trois lignes écrites
    // dans la même seconde se relisent dans un ordre arbitraire. L'ordre
    // d'extraction, lui, est une propriété de `parse_closing_issue_refs` et son
    // test unitaire l'atteste déjà — l'affirmer ici serait l'affirmer d'une
    // surface qui ne peut pas la porter.
    let mut keys: Vec<String> = rows_named(&db, CLOSING_PR_CLOSED_UNMERGED_TOOL)
        .await
        .into_iter()
        .map(|(key, ..)| key)
        .collect();
    keys.sort();
    let mut expected = vec![
        degroom_marker_key(OWNER_REPO, 2131),
        degroom_marker_key(OWNER_REPO, 1403),
        degroom_marker_key(OWNER_REPO, 1651),
    ];
    expected.sort();
    assert_eq!(keys, expected, "une ligne par ref fermante");
}

// ─────────────────────── AC3 / AC4 — le lecteur au routage ───────────────────

/// **AC3.** Marqueur posé, puis `labeled(ready)` sur un corps sans callouts :
/// une ligne `ready_label_degroomed` nommant la PR et la branche.
#[tokio::test]
async fn mika2242_t3_the_reader_names_the_cause() {
    let db = test_db();
    run_close(&db, &unmerged_close_event()).await;
    run_ready_label(&db, UNGROOMED_BODY).await;

    let rows = rows_named(&db, READY_LABEL_DEGROOMED_TOOL).await;
    assert_eq!(rows.len(), 1, "une attribution, une seule : {rows:?}");

    let (key, after, reasoning) = &rows[0];
    assert_eq!(
        *key,
        degroom_marker_key(OWNER_REPO, SUB_ISSUE),
        "les deux moitiés partagent la clé, sinon elles ne se rencontrent jamais"
    );
    assert_eq!(
        after.as_deref(),
        Some("pr#2226"),
        "le lecteur RÉ-ÉMET la valeur du producteur, il ne la recalcule pas"
    );
    let reasoning = reasoning.as_deref().expect("reasoning présent");
    assert!(
        reasoning.contains(&format!("head_branch={UMBRELLA_BRANCH}")),
        "la branche qui porte le plan antérieur : {reasoning}"
    );
    assert!(
        reasoning.contains("missing_markers="),
        "ce qui manque au corps est dit : {reasoning}"
    );
}

/// **AC4a — contrôle négatif du chemin nominal.** Un ticket groomé ne produit
/// aucune de ces lignes, marqueur présent ou non.
///
/// Le chemin `implement` doit rester strictement muet : c'est la moitié qui
/// garantit que le lecteur est placé sous `!is_groomed` et non en aval.
#[tokio::test]
async fn mika2242_t4a_a_groomed_ticket_is_silent_even_with_a_marker() {
    let db = test_db();
    run_close(&db, &unmerged_close_event()).await;
    run_ready_label(&db, GROOMED_BODY).await;

    assert!(
        rows_named(&db, READY_LABEL_DEGROOMED_TOOL).await.is_empty(),
        "le chemin nominal ne doit produire aucune attribution"
    );
}

/// **AC4b — contrôle négatif du premier grooming.** Un ticket simplement non
/// groomé, sans marqueur, ne produit rien.
///
/// Sans lui, un lecteur firant sur *tout* ticket non groomé passerait T3 en
/// comptant chaque premier grooming de la boucle — le trafic nominal.
#[tokio::test]
async fn mika2242_t4b_a_first_grooming_is_silent() {
    let db = test_db();
    run_ready_label(&db, UNGROOMED_BODY).await;

    assert!(
        rows_named(&db, READY_LABEL_DEGROOMED_TOOL).await.is_empty(),
        "un premier grooming nominal n'est pas un dé-groomage"
    );
}

// ─────────────────────────── AC5 — le routage ne bouge pas ───────────────────

/// **AC5.** Même `dispatch_class`, même `ReadyLabelGate`, avec et sans marqueur.
///
/// La porte est lue sur la ligne `ready_label_outcome` (mika#2323), qui est la
/// surface d'attribution du routage : si le bloc mika#2242 déplaçait une
/// décision, elle changerait ici.
#[tokio::test]
async fn mika2242_t6_routing_is_byte_identical_with_and_without_the_marker() {
    async fn observe(with_marker: bool) -> (Option<String>, Option<String>) {
        let db = test_db();
        if with_marker {
            run_close(&db, &unmerged_close_event()).await;
        }
        run_ready_label(&db, UNGROOMED_BODY).await;

        let gate = rows_named(&db, "ready_label_outcome")
            .await
            .into_iter()
            .next()
            .and_then(|(_, after, _)| after);

        let task = db
            .find_active_task_by_ref_url(&issue_url())
            .await
            .expect("lookup de la row de suivi")
            .expect("l'étape 7 pré-crée la row");
        (gate, task.dispatch_class)
    }

    let without = observe(false).await;
    let with = observe(true).await;

    assert_eq!(
        with.1.as_deref(),
        Some("groom"),
        "un ticket dé-groomé part TOUJOURS en groom — re-groomer est un travail correct"
    );
    assert_eq!(
        with, without,
        "l'attribution n'est pas une décision : porte et classe de dispatch sont inchangées"
    );
}

// ──────────────────────── AC6 — le nettoyage est inchangé ────────────────────

/// **AC6.** Les rows de suivi transitionnées, leurs statuts, leurs résultats et
/// leur ligne `tracking_row_upstream_closed` sont inchangés.
///
/// Le marqueur est écrit `fire-and-forget` **avant** le nettoyage, et son écriture
/// ne peut pas le faire échouer (R2). Ce test observe la moitié assertable de
/// cette propriété : sur la fermeture qui écrit un marqueur, la row transitionne
/// exactement comme avant.
#[tokio::test]
async fn mika2242_t7_the_existing_row_cleanup_is_untouched() {
    let db = test_db();

    let task_id = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: format!("ready-label: {OWNER_REPO}#{SUB_ISSUE}"),
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
            created_by_session: Some(SESSION.to_string()),
            created_trace_id: Some(TRACE.to_string()),
            reference_url: Some(issue_url()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("groom".to_string()),
        })
        .await
        .expect("create tracking row");
    db.mark_parent_dispatched(&task_id)
        .await
        .expect("in_progress");

    run_close(&db, &unmerged_close_event()).await;

    let task = db
        .get_task(&task_id)
        .await
        .expect("relecture")
        .expect("la row existe");
    assert_eq!(task.status, "cancelled", "une fermeture sans merge annule");
    assert_eq!(task.result.as_deref(), Some(UPSTREAM_PR_CLOSED_UNMERGED));

    let cleanup = rows_named(&db, "tracking_row_upstream_closed").await;
    assert_eq!(cleanup.len(), 1, "une ligne de nettoyage : {cleanup:?}");
    let (key, after, reasoning) = &cleanup[0];
    assert_eq!(*key, format!("task:{task_id}"));
    assert_eq!(after.as_deref(), Some(UPSTREAM_PR_CLOSED_UNMERGED));
    let reasoning = reasoning.as_deref().expect("reasoning présent");
    assert!(
        reasoning.contains("event_type=pull_request.closed"),
        "{reasoning}"
    );
    assert!(
        reasoning.contains(&format!("reference_url={}", issue_url())),
        "{reasoning}"
    );
}

// ────────────────────── AC7 / AC8 — les deux inerties, dites ─────────────────

/// **AC8.** Une fermeture sans merge à corps **tronqué** et sans ref parsée émet
/// la ligne d'inertie ; la même sans troncature ne l'émet pas.
///
/// C'est l'aveu de l'angle mort R-e : `format_event_text` coupe le corps à
/// 2 000 caractères et `parse_closing_issue_refs` lit CE corps, donc les
/// `Closes #N` d'une umbrella peuvent tomber au-delà. Un détecteur
/// silencieusement inerte se lit exactement comme un détecteur sain (mika#2205),
/// d'où une ligne nommée plutôt qu'un silence.
#[tokio::test]
async fn mika2242_t8_the_truncation_blind_spot_is_said() {
    let (_guard, events) = capture();

    let cut = format!(
        "[GitHub] PR closed: {OWNER_REPO}#{UMBRELLA_PR} — fix (branch: {UMBRELLA_BRANCH})\n\
         https://github.com/{OWNER_REPO}/pull/{UMBRELLA_PR}\n\
         Merged: false\n\nCorps long…\n\n[truncated]"
    );
    let short = format!(
        "[GitHub] PR closed: {OWNER_REPO}#{UMBRELLA_PR} — fix (branch: {UMBRELLA_BRANCH})\n\
         https://github.com/{OWNER_REPO}/pull/{UMBRELLA_PR}\n\
         Merged: false\n\nCorps court, aucune ref."
    );

    let db = test_db();
    run_close(&db, &cut).await;
    let after_cut = named(&events, "closing_pr_body_truncated_no_refs").len();
    assert_eq!(
        after_cut, 1,
        "un corps coupé sans ref est une inertie à dire"
    );

    // Même buffer de capture : le contrôle négatif est qu'AUCUNE ligne nouvelle
    // ne s'ajoute. Un corps non tronqué sans ref est une PR qui ne ferme rien —
    // le cas nominal, et le dire noierait le signal (mika#2131 AC7).
    let db = test_db();
    run_close(&db, &short).await;
    let after_short = named(&events, "closing_pr_body_truncated_no_refs").len();
    assert_eq!(
        after_short, after_cut,
        "un corps NON tronqué sans ref ne doit rien émettre"
    );
}

/// **AC7.** Un registre illisible ne change ni le routage ni le nettoyage.
///
/// La voie lisible est celle qu'observe T6 ; ici on nomme l'invariant de
/// structure : le lecteur est fail-open sur **toutes** ses lectures et ne peut
/// coûter qu'une explication, jamais une décision ni une attribution fausse.
#[tokio::test]
async fn mika2242_t9_the_reader_is_fail_open_on_every_read() {
    // Un marqueur dont le `reasoning` est inexploitable : la décision du lecteur
    // ne tient qu'à la PRÉSENCE de la ligne, la branche est un enrichissement.
    let db = test_db();
    db.log_audit_event(
        SESSION,
        CLOSING_PR_CLOSED_UNMERGED_TOOL,
        &degroom_marker_key(OWNER_REPO, SUB_ISSUE),
        None,
        Some("pr#2226"),
        None,
        Some(TRACE),
    )
    .await
    .expect("poser un marqueur dégradé");

    run_ready_label(&db, UNGROOMED_BODY).await;

    let rows = rows_named(&db, READY_LABEL_DEGROOMED_TOOL).await;
    assert_eq!(rows.len(), 1, "le fait survit à un reasoning illisible");
    let (_, after, reasoning) = &rows[0];
    assert_eq!(after.as_deref(), Some("pr#2226"));
    assert!(
        reasoning
            .as_deref()
            .expect("reasoning")
            .contains("head_branch=<unknown>"),
        "la branche inconnue est DITE, elle n'est pas inventée : {reasoning:?}"
    );

    // Et le routage est celui de toujours.
    let task = db
        .find_active_task_by_ref_url(&issue_url())
        .await
        .expect("lookup")
        .expect("row pré-créée");
    assert_eq!(task.dispatch_class.as_deref(), Some("groom"));
}

// ───────────────────────────── capture de tracing ────────────────────────────
// Même forme que `tests/manager_delivery_observability_2267.rs`.

#[derive(Debug, Clone)]
struct CapturedEvent {
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn name(&self) -> &str {
        self.fields.get("event").map(String::as_str).unwrap_or("")
    }
}

struct CapturingLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent { fields });
        }
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

fn capture() -> (
    tracing::subscriber::DefaultGuard,
    Arc<Mutex<Vec<CapturedEvent>>>,
) {
    use tracing_subscriber::layer::SubscriberExt;
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

fn named(events: &Arc<Mutex<Vec<CapturedEvent>>>, name: &str) -> Vec<CapturedEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.name() == name)
        .cloned()
        .collect()
}
