//! mika#2653 — un tour webhook PR n'annule pas une tâche dont un pilote est vif.
//!
//! **Le défaut rejoué.** Trace `8ec6364c-be71-11f1-908b-e931f18d2c16`,
//! 2026-10-02 : un tour mika-dev ouvert par un **webhook de revue QA sur la PR
//! #2644** a appelé `cancel_task` sur le pilote **Fix-CI en vol**
//! (`8a3b2082`) — **71 tours jetés**. Second cas mesuré de la famille mika#2649,
//! le même jour, et le plus coûteux des deux.
//!
//! **Pourquoi le chemin de production et pas le prédicat.** Frère de
//! `test_fallthrough_run_gh_refused_2573.rs`, et il suit le raisonnement que ce
//! fichier écrit déjà : les tests unitaires de `classify_cancel_on_live_pilot`
//! (`tools::cancel_task::tests::mika2653`) restent **verts** si le champ
//! `ToolContext.is_webhook_pr_event_turn` n'est jamais calculé ou jamais lu, et
//! c'est très exactement le mode de panne — le booléen est posé à un seul site
//! de production, et les vingt autres le mettent à `false`. Seul un vrai tour,
//! dont la garde tourne à sa position de production dans `CancelTaskTool`,
//! ferme ça.
//!
//! **Le contrôle négatif est porteur, pas décoratif.** V6a seul serait satisfait
//! par une garde qui refuse `cancel_task` sur **tout** tour — c'est-à-dire par
//! un correctif qui retire à l'opérateur son geste d'annulation, lequel n'a pas
//! de contournement. V6b (le même appel depuis un tour de conversation) est
//! l'axe qui sépare « la garde décide » de « la garde bloque l'annulation », et
//! V6c (tour webhook PR, aucun pilote) est le **contrôle positif** : sans lui,
//! zéro refus serait indistinguable d'une garde qui ne tourne pas.
//!
//! **Linux seulement** pour les deux cas qui demandent un pilote réellement
//! vif : la vivacité est établie par `is_same_process_alive`, qui lit
//! `/proc/<pid>/stat`. Même motif que `live_pilot.rs`'s `self_pid_and_start` —
//! le processus de test est son propre pilote, donc vivant par construction et
//! avec un `process_start_time` qui correspond.

use mika_agent::db::NewTask;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// L'identifiant stable du refus, tel que le LLM le reçoit.
const REFUSAL: &str = "cancel_refused_live_pilot";

/// Le `tool_name` sous lequel la garde audite chaque décision.
const AUDIT_TOOL: &str = "cancel_task_pilot_guard";

/// Le déclencheur mesuré le 2026-10-02 : une revue QA sur la PR #2644.
const PR_REVIEW_MSG: &str = "[GitHub] PR review (changes_requested) on senara-solutions/mika#2644 (fix CI) by @mika-platform-qa";

/// Hors population : un opérateur qui demande l'annulation en conversation.
const OPERATOR_MSG: &str = "annule la tâche de dispatch, elle a l'air perdue";

/// Un pilote **réellement vif**, et c'est un processus enfant — jamais le
/// nôtre — nettoyé par RAII.
///
/// # Pourquoi pas `std::process::id()`
///
/// `live_pilot.rs` l'emploie pour ses propres unités, et ça y est sans danger :
/// aucune n'atteint le chemin de kill. Ici le contrôle négatif V6b **traverse**
/// `cancel_task_and_kill` par conception, qui envoie un SIGTERM au **groupe de
/// processus** du pgid enregistré. Avec notre propre pid, ce groupe est celui du
/// binaire de test : le contrôle négatif tuerait la campagne qui l'exécute, de
/// façon dépendante de l'environnement.
///
/// # Pourquoi `process_group(0)` et pourquoi `Drop`
///
/// `process_group(0)` fait de l'enfant son **propre** chef de groupe, donc le
/// pgid enregistré en base en est un véritable et le `kill(-pid)` de
/// `kill_process_gracefully` l'atteint sans retomber sur son repli monoprocessus
/// — motif du frère `test_ready_label_live_pilot_noop_2279.rs`.
///
/// Le `Drop` est ce que `std::mem::forget` + un appel de nettoyage en dernière
/// ligne n'offrent pas : **deux** des quatre cas de ce fichier sont des refus,
/// donc rien d'autre ne signale l'enfant, et une assertion qui panique y
/// abandonnerait un `sleep 120` pour deux minutes. Il `waitpid` aussi, faute de
/// quoi chaque enfant tué reste un zombie pour la vie du binaire de test.
#[cfg(target_os = "linux")]
struct LivePilotChild {
    child: std::process::Child,
    pid: i64,
    start_time: u64,
}

#[cfg(target_os = "linux")]
impl LivePilotChild {
    fn spawn() -> Self {
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new("sleep")
            .arg("120")
            .process_group(0)
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id();
        let start_time = mika_agent::task_engine::process_liveness::read_process_start_time(pid)
            .expect("read child start time");
        Self {
            child,
            pid: i64::from(pid),
            start_time,
        }
    }

    /// Le couple `(pgid, process_start_time)` tel que l'exécuteur l'enregistre.
    fn pgid_and_start(&self) -> (i64, u64) {
        (self.pid, self.start_time)
    }

    fn is_alive(&self) -> bool {
        mika_agent::task_engine::process_liveness::is_same_process_alive(
            u32::try_from(self.pid).expect("pid fits in u32"),
            self.start_time,
        )
    }
}

#[cfg(target_os = "linux")]
impl Drop for LivePilotChild {
    fn drop(&mut self) {
        // Le groupe d'abord (l'enfant en est le chef), puis le processus seul —
        // même ordre et même raison que `kill_process_gracefully`.
        for target in [format!("-{}", self.pid), self.pid.to_string()] {
            let _ = std::process::Command::new("kill")
                .arg("-KILL")
                .arg(target)
                .output();
        }
        // Et on le moissonne : sans ça, un enfant tué reste un zombie jusqu'à la
        // fin du binaire de test.
        let _ = self.child.wait();
    }
}

/// Une ligne callback de dispatch, telle que l'exécuteur l'écrit : c'est elle
/// qui porte le pgid (la topologie à deux lignes de `live_pilot`).
async fn seed_dispatch_callback(harness: &EvalHarness, pid: Option<(i64, Option<u64>)>) -> String {
    let id = harness
        .db
        .create_task(NewTask {
            agent_id: harness.db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "long_running:run_claude_pilot".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: None,
            created_trace_id: None,
            reference_url: None,
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: None,
            dispatch_class: Some("implement".to_string()),
        })
        .await
        .expect("create dispatch callback row");

    if let Some((p, start_time)) = pid {
        harness
            .db
            .set_task_process_id(&id, Some(p))
            .await
            .expect("record pgid");
        if let Some(st) = start_time {
            harness
                .db
                .set_task_metadata_field(&id, "process_start_time", &st.to_string())
                .await
                .expect("record start time");
        }
    }
    id
}

/// Construire le harness, semer la ligne de dispatch, puis lancer le tour.
///
/// L'ordre est contraint : le harness possède la base, donc la ligne ne peut
/// être semée qu'après lui — et son identifiant n'est connu qu'ensuite, donc les
/// réponses mockées sont injectées en dernier (`clear_and_set`).
async fn run_on_seeded(
    message: &str,
    pid: Option<(i64, Option<u64>)>,
) -> (EvalHarness, String, String) {
    let harness = EvalHarness::builder()
        .responses(Vec::new())
        .build()
        .await
        .unwrap();
    let task_id = seed_dispatch_callback(&harness, pid).await;

    harness.mock().clear_and_set(vec![
        tool_call_response("cancel_task", json!({"id": task_id})),
        // Assez de réponses texte pour absorber le re-prompt de
        // `webhook_zero_tools` : sur le tour webhook PR, l'appel `cancel_task`
        // est refusé, donc le tour n'a aucun appel d'outil *réussi* et le garde
        // d'intention re-prompte une fois. C'est le comportement nominal du
        // chemin, pas un défaut de la fixture.
        text_response("Acknowledged."),
        text_response("Acknowledged."),
        text_response("Acknowledged."),
        text_response("Acknowledged."),
    ]);

    let trace = harness.run(message).await.unwrap();
    let calls = trace.calls_for_tool("cancel_task");
    assert!(
        !calls.is_empty(),
        "la fixture doit avoir atteint `cancel_task` — sans appel il n'y a rien à \
         affirmer"
    );
    let output = calls[0].output.clone().unwrap_or_default();
    (harness, task_id, output)
}

/// Les `after_value` de chaque ligne de la garde, pour la session du harness.
async fn guard_rows(harness: &EvalHarness) -> Vec<Option<String>> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events must be readable")
        .into_iter()
        .filter(|e| e.tool_name == AUDIT_TOOL)
        .map(|e| e.after_value)
        .collect()
}

// -------------------------------------------------------------------------
// V6a (AC1) — le défaut mesuré, rejoué par le chemin de production.
// -------------------------------------------------------------------------

#[tokio::test]
#[cfg(target_os = "linux")]
async fn mika2653_un_tour_webhook_pr_nannule_pas_un_pilote_vif() {
    let pilot = LivePilotChild::spawn();
    let (pid, st) = pilot.pgid_and_start();
    let (harness, task_id, output) = run_on_seeded(PR_REVIEW_MSG, Some((pid, Some(st)))).await;

    assert!(
        output.contains(REFUSAL),
        "annuler un pilote vif depuis un tour ouvert par une revue de PR jette tout \
         ce que ce pilote a produit (71 tours, le 2026-10-02) — ça doit être \
         refusé ; obtenu : {output}"
    );
    assert!(
        output.contains("mika#2653"),
        "le refus doit nommer sa doctrine ; obtenu : {output}"
    );

    // AC1 — aucun statut écrit : la ligne reste non terminale.
    let task = harness
        .db
        .get_task(&task_id)
        .await
        .expect("task readable")
        .expect("task exists");
    assert_eq!(
        task.status, "pending",
        "INVARIANT VIOLÉ : la garde a laissé écrire un statut — elle tourne AVANT \
         `cancel_task_and_kill`, donc avant toute écriture et avant tout signal"
    );

    // AC1 — aucun signal envoyé : le pilote tourne toujours. En apparence
    // trivial, et c'est pourtant le seul contrôle qui dise que le travail
    // survit, puisque c'est le travail que le défaut détruisait.
    assert!(
        pilot.is_alive(),
        "le pilote a été signalé : la garde a laissé passer le kill"
    );

    // AC7 — la décision est comptable.
    assert_eq!(
        guard_rows(&harness).await,
        vec![Some("blocked_live_pilot".to_string())],
        "exactement une ligne d'audit, portant la valeur du format de fil sur \
         laquelle l'opérateur groupe"
    );
}

// -------------------------------------------------------------------------
// V6b (AC3) — LE contrôle négatif : hors tour webhook PR, rien ne change.
//
// Sans lui, « la garde décide » est indistinguable de « la garde bloque toute
// annulation », et les trois chemins opérateur pourraient être cassés avec tous
// les autres tests au vert.
// -------------------------------------------------------------------------

#[tokio::test]
#[cfg(target_os = "linux")]
async fn mika2653_un_tour_de_conversation_annule_comme_avant() {
    // Ce contrôle traverse `cancel_task_and_kill` par conception — c'est très
    // exactement ce qu'il atteste — donc le pilote est un enfant dans son
    // propre groupe, et son kill est l'issue nominale du scénario.
    let pilot = LivePilotChild::spawn();
    let (pid, st) = pilot.pgid_and_start();
    let (harness, task_id, output) = run_on_seeded(OPERATOR_MSG, Some((pid, Some(st)))).await;

    assert!(
        !output.contains(REFUSAL),
        "INVARIANT VIOLÉ (AC3) : la garde a mordu hors de sa population — retirer à \
         l'opérateur son geste d'annulation est pire que le défaut qu'on referme, \
         puisque ce geste n'a pas de contournement ; obtenu : {output}"
    );
    assert!(
        output.contains("cancelled"),
        "l'annulation doit aboutir exactement comme avant ; obtenu : {output}"
    );

    let task = harness
        .db
        .get_task(&task_id)
        .await
        .expect("task readable")
        .expect("task exists");
    assert_eq!(task.status, "cancelled");

    assert!(
        guard_rows(&harness).await.is_empty(),
        "hors population : aucune ligne d'audit — ce serait l'essentiel du trafic, \
         soit très exactement le churn que la doctrine mika#2131 borne"
    );
}

// -------------------------------------------------------------------------
// V6c (AC7) — LE CONTRÔLE POSITIF : tour webhook PR, aucun pilote, ça passe.
//
// Sans lui, zéro refus aurait trois causes indistinguables : aucun refus
// (sain), aucune annulation depuis un tour webhook (sain), binaire antérieur au
// correctif (classe mika#2340). *Une garde que personne n'a exercée se lit
// exactement comme une garde qui marche* (mika#2205).
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2653_un_tour_webhook_pr_sans_pilote_annule_et_le_dit() {
    let (harness, task_id, output) = run_on_seeded(PR_REVIEW_MSG, None).await;

    assert!(
        !output.contains(REFUSAL),
        "aucun pilote ne porte cette tâche : l'annulation doit passer ; obtenu : {output}"
    );
    assert!(output.contains("cancelled"), "obtenu : {output}");

    let task = harness
        .db
        .get_task(&task_id)
        .await
        .expect("task readable")
        .expect("task exists");
    assert_eq!(task.status, "cancelled");

    assert_eq!(
        guard_rows(&harness).await,
        vec![Some("allowed_no_pilot".to_string())],
        "la garde a tourné et a laissé passer : c'est le contrôle positif, et c'est \
         ce qui rend lisible un zéro sur les trois valeurs de refus"
    );
}

// -------------------------------------------------------------------------
// V6d (AC2) — le fail-closed sur l'illisible, sur le chemin de production.
//
// Un pgid présent dont le `process_start_time` n'est pas lisible : la vivacité
// n'est pas prouvable, donc l'annulation est refusée. C'est l'arbitrage inverse
// de celui de `live_pilot` pour ses deux appelants, et c'est l'asymétrie de coût
// qui le décide (un appel d'outil refusé contre 71 tours jetés).
// -------------------------------------------------------------------------

#[tokio::test]
#[cfg(target_os = "linux")]
async fn mika2653_un_signal_illisible_refuse_sur_un_tour_webhook_pr() {
    let pilot = LivePilotChild::spawn();
    let (pid, _) = pilot.pgid_and_start();
    let (harness, task_id, output) = run_on_seeded(PR_REVIEW_MSG, Some((pid, None))).await;

    assert!(
        output.contains("cancel_refused_pilot_unreadable"),
        "sans `process_start_time` la paire qui identifie une *instance* est \
         incomplète : un signal qu'on ne peut pas lire n'est jamais un terme \
         satisfait ; obtenu : {output}"
    );

    let task = harness
        .db
        .get_task(&task_id)
        .await
        .expect("task readable")
        .expect("task exists");
    assert_eq!(task.status, "pending");

    assert_eq!(
        guard_rows(&harness).await,
        vec![Some("blocked_start_time_unreadable".to_string())],
        "la cause est comptée SÉPARÉMENT de la cause base : les deux remèdes sont \
         opposés — « l'instance n'est pas prouvable » contre « la base ne répond \
         pas »"
    );
}
