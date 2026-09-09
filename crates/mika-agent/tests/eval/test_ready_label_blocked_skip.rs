//! Négatif (c) — mika#2263 : **un ticket que l'opérateur tient (`blocked`) ne
//! produit aucune task de dispatch, quel que soit le nombre d'événements
//! `ready` qui arrivent.**
//!
//! Classe mesurée le 2026-09-09 : #1781 portait `blocked` et son label `ready`
//! avait été retiré — pourtant `ready_label_handler` l'a re-dispatché DEUX fois
//! de plus (pgid 478551, 492118, row daba9416) sur des events `ready`
//! stale/redélivrés. `blocked` excluait le feeder `auto_pull`
//! (`is_feeder_excluded`) et RIEN dans le chemin du handler. Le label ne
//! contenait donc pas le dispatch : il contenait la moitié des chemins qui y
//! mènent.
//!
//! L'invariant que ces tests nomment : *aucune task n'est créée pour un ticket
//! porteur d'un label operator-held, et le refus ne peut pas être re-converti
//! en dispatch par la garde d'intention.*
//!
//! Le second membre n'est pas décoratif. `VerdictAction::Passthrough` laisse
//! `req.text` sur le marqueur ready — exactement ce que déclenche l'INTENT_GUARD
//! `webhook_ready_label_dispatch`, qui re-somme alors le LLM de dispatcher le
//! ticket que la porte vient de refuser. Les deux portes voisines (allowlist de
//! dépôt #2046, siège de dispatch #2084) refusent en `Handled` pour cette
//! raison précise ; celle-ci fait de même, et le test le vérifie plutôt que de
//! le supposer.
//!
//! # Rouge-avant (porte #2264)
//!
//! Sur le commit précédent (joint d'injection posé, porte absente),
//! [`a_blocked_ticket_creates_no_task`] échoue : le handler pré-crée sa row et
//! le compte des tasks vaut 1. Recette d'injection : retirer l'étape 4c de
//! `try_handle_ready_label_dispatch_with_fetcher` — rouge de nouveau.

use std::sync::Arc;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::server::ready_label_handler::try_handle_ready_label_dispatch_with_fetcher;
use mika_agent::server::verdict_handler::VerdictAction;
use mika_agent::skills::SkillRegistry;

const AGENT_ID: &str = "mika";
const SESSION: &str = "eval-session";
const TRACE: &str = "eval-trace";

/// Un ticket groomé : le marqueur de grooming est présent, donc rien d'autre
/// que la porte testée ne peut refuser le dispatch.
const GROOMED_BODY: &str = "> - **Plan:** docs/plans/2026-09-09-fix-1781.md\n\nCorps du ticket.";

const READY_EVENT: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#1781 — fix: quelque chose\nhttps://github.com/senara-solutions/mika/issues/1781";

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

/// Toutes les tasks vivantes ou mortes de l'agent — le compte que la porte doit
/// laisser à zéro.
async fn task_count(db: &AsyncDatabase) -> usize {
    db.get_tasks_by_status(vec![
        "pending".to_string(),
        "in_progress".to_string(),
        "blocked".to_string(),
        "completed".to_string(),
        "failed".to_string(),
        "cancelled".to_string(),
        "expired".to_string(),
        "delivered".to_string(),
    ])
    .await
    .expect("list tasks")
    .len()
}

async fn run_handler(db: &AsyncDatabase, labels: Vec<String>) -> VerdictAction {
    let sender: Arc<dyn MessageSender> = Arc::new(NoopSender);
    let skills = SkillRegistry::empty();
    try_handle_ready_label_dispatch_with_fetcher(
        READY_EVENT,
        db,
        Some("fake-token"),
        Some(&sender),
        SESSION,
        TRACE,
        &skills,
        move |_owner_repo, _number, _token| async move { Ok((GROOMED_BODY.to_string(), labels)) },
    )
    .await
}

/// INVARIANT : un ticket `blocked` ne crée aucune task, et le refus n'est pas
/// re-convertible en dispatch par la garde d'intention.
#[tokio::test]
async fn a_blocked_ticket_creates_no_task() {
    let db = test_db();
    assert_eq!(task_count(&db).await, 0, "contrôle positif : base vide");

    let action = run_handler(&db, vec!["ready".to_string(), "blocked".to_string()]).await;

    assert_eq!(
        task_count(&db).await,
        0,
        "INVARIANT VIOLÉ : un ticket `blocked` a produit une task de dispatch — \
         c'est le re-dispatch de #1781 (mika#2263)"
    );
    match action {
        VerdictAction::Handled { pre_digest } => {
            assert!(
                pre_digest.starts_with("<ready_label_handler>"),
                "le refus doit ouvrir sur la balise du handler, sinon l'INTENT_GUARD \
                 webhook_ready_label_dispatch réclame le dispatch qu'on vient de refuser"
            );
            assert!(
                pre_digest.contains("blocked"),
                "le refus doit nommer le label qui tient le ticket"
            );
            assert!(
                pre_digest.contains("1781"),
                "le refus doit nommer l'issue refusée"
            );
        }
        other => panic!(
            "un refus operator-held doit être Handled (Passthrough laisse le marqueur \
             ready dans req.text et la garde re-somme le LLM de dispatcher) — obtenu {other:?}"
        ),
    }
}

/// Le même verrou pour les deux autres labels du même trio operator-held, ceux
/// que `auto_pull::is_feeder_excluded` refuse déjà côté feeder. Sans eux, la
/// porte fermerait un chemin sur trois et la dérive entre les deux surfaces
/// resterait ouverte.
#[tokio::test]
async fn operator_review_and_operator_gated_create_no_task_either() {
    for label in ["operator-review", "operator-gated"] {
        let db = test_db();
        let action = run_handler(&db, vec!["ready".to_string(), label.to_string()]).await;
        assert_eq!(
            task_count(&db).await,
            0,
            "INVARIANT VIOLÉ : `{label}` tient le ticket côté feeder mais pas côté handler"
        );
        assert!(
            matches!(action, VerdictAction::Handled { .. }),
            "`{label}` doit refuser en Handled"
        );
    }
}

/// Contrôle négatif : sans label operator-held, la porte ne mord pas. Le
/// handler poursuit son chemin et pré-crée sa row — c'est ce que ce test
/// mesure, et sans lui un refus inconditionnel passerait les deux tests
/// ci-dessus.
#[tokio::test]
async fn an_unheld_ticket_still_reaches_the_pre_create() {
    let db = test_db();
    let action = run_handler(&db, vec!["ready".to_string()]).await;

    assert_eq!(
        task_count(&db).await,
        1,
        "contrôle négatif : un ticket non tenu doit encore atteindre le pré-create"
    );
    assert!(
        !matches!(action, VerdictAction::Passthrough { .. }),
        "un ticket non tenu n'est pas un passthrough"
    );
}
