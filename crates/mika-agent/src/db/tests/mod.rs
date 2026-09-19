//! Tests de la couche DB (`crate::db`), sortis de `db.rs` par mika#2321.
//!
//! ## Pourquoi un répertoire et pas un `db/tests.rs`
//!
//! Trois raisons, chacune mesurée (plan mika#2321 D2) :
//!
//! 1. **Classification du périmètre.** `perimeter::rules` classe `Mechanical`
//!    tout chemin portant un segment `/tests/`, et `DecisionCore` un
//!    `some_module/tests.rs` isolé — par choix fail-closed explicite. Or une PR
//!    DECISION-CORE n'est pas dé-draftée automatiquement (mika#2286) : le
//!    fichier unique ferait de *chaque* PR touchant un test de `db` un draft en
//!    attente d'un geste humain de vérification.
//! 2. **`harnais_porte` y vivait déjà** (mika#2310), avec un `#[path]` dont la
//!    seule raison d'être était que `mod tests` était inline. Le répertoire en
//!    fait un frère naturel et retire l'attribut.
//! 3. **Croissance.** Un `tests.rs` unique naîtrait à 463 Ko — la moitié du
//!    plafond de `scripts/check-secrets.sh`, dans la région de `db.rs` qui
//!    croît le plus vite (une région append-only : un test est rarement
//!    supprimé). Un répertoire répartit la croissance par thème.
//!
//! ## Ce qui reste ici, et ce qui descend
//!
//! **Ici :** les helpers, fixtures, constantes et types partagés — c'est-à-dire
//! tout ce qui n'est pas un `#[test]`. Un sous-module est un *descendant* de ce
//! module, donc il voit ses items privés ; regrouper les helpers à la racine
//! évite d'avoir à arbitrer, helper par helper, quel thème le « possède ».
//!
//! Six d'entre eux ne peuvent pas bouger : `db()`, `GROOM_ISSUE_URL`,
//! `GROOM_CALLBACK_PLAN_GROOMED`, `groom_parent`,
//! `groom_callback` et `completed_groom_pair` sont importés par
//! `skills::executor` sous le chemin `crate::db::tests::*`. **Ce chemin doit
//! survivre octet pour octet** — c'est un contrat inter-modules, pas un détail
//! d'organisation (plan mika#2321 E4). Une retouche de `skills/executor.rs`
//! dans un diff qui touche ce module est le signe que la contrainte a été
//! contournée plutôt que respectée.
//!
//! **En dessous :** les `#[test]`, répartis par thème en suivant les marqueurs
//! de section que `db.rs` portait déjà.
//!
//! ## Visibilité
//!
//! Ce module reste **enfant de `db`**, donc tous les items privés de `db` lui
//! restent visibles exactement comme quand il était inline — le découpage du
//! volet A ne demande aucun élargissement de visibilité. La règle qui rend cela
//! vrai est générale (plan mika#2321 D3) : *un module de test voyage avec le
//! code qu'il teste*. C'est pourquoi les tests de migration ne sont pas ici mais
//! dans `db/migrations.rs`, sous leur propre `#[cfg(test)] mod tests` : ils
//! appellent une trentaine de `migrate_vN_to_vM` privées, et deux frères ne se
//! voient pas.

use super::*;

mod dispatch_stamp_and_slots;
mod reapers;
mod recurring_tasks;
mod secrets_and_agent_reset;
mod settle_dispatch_parents;
mod skill_overrides_and_task_types;
mod task_messages_and_groom;
mod tool_calls_and_messages;

pub(crate) fn db() -> Database {
    Database::open_in_memory().unwrap()
}

fn db_with_session() -> (Database, String) {
    let db = db();
    let session_id = "test-session".to_string();
    db.create_session(&session_id, "mika", "cli").unwrap();
    (db, session_id)
}

/// Sites `update_manual_task_status(…, "in_progress")` de la moitié
/// production de `src`, rendus sous la forme `<chemin>:<ligne>`.
///
/// Deux écartements composés, et l'ordre compte : la **classification par
/// chemin** (mika#2321) écarte un fichier de test entièrement — il n'a pas
/// de moitié production — avant que le **masquage** (mika#2398) ne retire
/// les régions de test d'un fichier de production.
fn dispatch_stamp_violations(path: &std::path::Path, src: &str) -> Vec<String> {
    if crate::source_scan::is_test_source_path(path) {
        return Vec::new();
    }

    // Masquer les régions de test inline : ces fixtures posent légitimement
    // des rows `in_progress` à la main. Le masquage est celui de
    // `mika_common::source_guard` (mika#2398), pas une troncature au premier
    // `#[cfg(test)]` — qui perdait 14 043 lignes de production sur 22
    // fichiers, dont `prompt.rs` coupé ligne 142 par un doc-comment.
    let production = mika_common::source_guard::mask_test_regions(src);

    // Les lignes de commentaire sont neutralisées (et non supprimées,
    // pour que les numéros de ligne restent ceux du fichier) : la prose
    // doit pouvoir décrire ce qui est interdit — y compris la doc de
    // `mark_parent_dispatched`, qui nomme l'appel proscrit. Même idiome
    // que `milestone_manager/no_dispatch_test.rs`.
    let code: String = production
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("//") {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let needle = "update_manual_task_status";
    let mut violations = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = code[from..].find(needle) {
        let at = from + rel;
        // Fenêtre volontairement large : l'appel est régulièrement
        // reformaté sur trois lignes par `cargo fmt`, et une fenêtre
        // trop courte ferait passer le prochain site sous la garde.
        let end = (at + 200).min(code.len());
        if code[at..end].contains("\"in_progress\"") {
            let line = code[..at].matches('\n').count() + 1;
            violations.push(format!("{}:{line}", path.display()));
        }
        from = at + needle.len();
    }
    violations
}

fn make_task(label: &str) -> NewTask {
    NewTask {
        agent_id: "mika".to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
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
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

// ── mika#1742 Problem B — refuse-to-zombie guard on
//    create_recurring_task_if_absent ────────────────────────────────

fn zombie_recurring_task(agent_id: &str, label: &str) -> NewTask {
    NewTask {
        agent_id: agent_id.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some("0 0 * * * *".to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "inject_context".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

// ── mika#2337 — la levée du veto pour la classe « trigger inconnu » ──
//
// Trois tests jumeaux. Ils se lisent ensemble : le premier dit ce que la
// levée achète, le deuxième ce qu'elle ne touche pas, le troisième ce
// qu'elle coûte. Pris isolément, aucun des trois n'atteste l'invariant —
// une levée sans bornage passerait le premier, une levée sur *toute* mort
// passerait le premier et le troisième.

/// Tue la récurrence `label` maintenant, `age` en arrière, en marquant (ou
/// non) la mort comme « trigger inconnu ». Rend l'id de la ligne morte.
fn kill_recurring(db: &Database, label: &str, age: &str, unknown_trigger: bool) -> String {
    let id = db
        .create_recurring_task_if_absent(zombie_recurring_task("mika", label))
        .unwrap()
        .expect("la ligne doit s'inscrire avant de pouvoir mourir");
    if unknown_trigger {
        // L'ordre de production : le marqueur est posé AVANT le passage en
        // `failed`, pour qu'aucun instant n'expose une mort non marquée.
        assert_eq!(
            db.mark_recurring_unknown_trigger(&id).unwrap(),
            1,
            "le marqueur doit s'écrire sur une ligne récurrente"
        );
    }
    db.conn
        .execute(
            "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                 WHERE id = ?1",
            params![id, age],
        )
        .unwrap();
    id
}

// ── mika#2360 — registre des récurrences en lecture seule ─────────────

/// Insère une ligne du registre en contournant la garde mika#1742 (l'objet
/// des tests de tri est la projection, pas la garde). Rend l'id.
fn registry_row(db: &Database, label: &str, status: &str, created_at: &str) -> String {
    let mut t = zombie_recurring_task("mika", label);
    t.action_type = "send_message".to_string();
    t.action_config = r#"{"text":"SECRET-MEDICATION-REMINDER"}"#.to_string();
    t.metadata = Some(r#"{"note":"SECRET-METADATA-BLOB"}"#.to_string());
    let id = db.create_task(&t).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = ?2, created_at = ?3 WHERE id = ?1",
            params![id, status, created_at],
        )
        .unwrap();
    id
}

fn registry_labels(db: &Database) -> Vec<String> {
    db.list_recurring_registry(None, 200, 0)
        .unwrap()
        .0
        .into_iter()
        .map(|r| r.label)
        .collect()
}

fn callback_task(agent_id: &str) -> NewTask {
    NewTask {
        agent_id: agent_id.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "analyze_codebase".to_string(),
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
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

// -- find_orphaned_parent_tasks tests (#871) --

/// Helper: create a parent self_dev task (manual, in_progress, source=self_dev)
/// and a delivered callback child. Returns (parent_id, child_id).
fn create_orphaned_parent_setup(db: &Database) -> (String, String) {
    // Parent: manual, in_progress, source=self_dev
    let mut parent = new_task("mika", "Implement mika#868", "manual", "none");
    parent.source = Some("self_dev".to_string());
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    // Child: callback, resume_agent, delivered
    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    // Complete then deliver the child
    assert!(
        db.update_task_completed(&child_id, "mika", Some("done"))
            .unwrap()
    );
    assert!(db.mark_task_delivered(&child_id).unwrap());

    (parent_id, child_id)
}

/// The mika#2405 population: a `manual`/`none` tracking row left `in_progress`
/// with **no** `source` (the shape `tools/create_task.rs` writes outside the
/// self-dev paths), and one `delivered` callback child backdated `age_secs`
/// into the past.
///
/// Deliberately different from [`create_orphaned_parent_setup`] on exactly the
/// term that separates the two populations: `source` stays NULL here, which is
/// what `COALESCE(parent.source,'') != 'self_dev'` is written for.
fn create_settleable_parent_setup(db: &Database, age_secs: i64) -> (String, String) {
    let parent = new_task("mika", "long_running:build_mika", "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = ?1",
            params![parent_id],
        )
        .unwrap();

    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    assert!(
        db.update_task_completed(&child_id, "mika", Some("build ok"))
            .unwrap()
    );
    assert!(db.mark_task_delivered(&child_id).unwrap());

    backdate_task_updated_at(db, &child_id, age_secs);

    (parent_id, child_id)
}

/// Push a task's `updated_at` `secs` seconds into the past.
fn backdate_task_updated_at(db: &Database, task_id: &str, secs: i64) {
    db.conn
        .execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2) WHERE id = ?1",
            params![task_id, format!("-{secs} seconds")],
        )
        .unwrap();
}

// -- find_phantom_tracking_tasks tests (mika#1712) --

/// Insert a phantom-shape tracking row (`action_type='none'`,
/// `process_id IS NULL`, `status='in_progress'` by default) with
/// `updated_at` aged `age_secs` into the past. Returns the row id.
fn create_phantom_tracking_row(
    db: &Database,
    agent_id: &str,
    label: &str,
    status: &str,
    age_secs: i64,
) -> String {
    let task = new_task(agent_id, label, "manual", "none");
    let id = db.create_task(&task).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = ?2,
                 process_id = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                 WHERE id = ?1",
            params![id, status, format!("-{age_secs} seconds")],
        )
        .unwrap();
    id
}

// -- find_dispatch_children_with_pid tests (mika#2156) --

/// Insert a recall child under `parent_id` carrying `process_id`, and
/// optionally the `metadata.process_start_time` string the executor writes.
fn create_recall_child(
    db: &Database,
    agent_id: &str,
    parent_id: &str,
    label: &str,
    process_id: Option<i64>,
    start_time: Option<&str>,
) -> String {
    let mut task = new_task(agent_id, label, "manual", "resume_agent");
    task.parent_task_id = Some(parent_id.to_string());
    let id = db.create_task(&task).unwrap();
    if let Some(pid) = process_id {
        db.set_task_process_id(&id, Some(pid)).unwrap();
    }
    if let Some(st) = start_time {
        db.set_task_metadata_field(&id, "process_start_time", st)
            .unwrap();
    }
    id
}

// -- find_orphaned_pending_issue_tasks tests (mika#2045) --

/// Create a `pending` self_dev **issue** parent with a `reference_url`,
/// backdated `age_secs` into the past. This is the shape `ready_label_handler`
/// pre-creates before it asks `validate_dispatch_readiness` for the slot.
fn create_pending_issue_parent(db: &Database, issue: u32, age_secs: i64) -> String {
    let mut parent = new_task(
        "mika",
        &format!("ready-label: x/y#{issue}"),
        "manual",
        "none",
    );
    parent.source = Some("self_dev".to_string());
    parent.reference_url = Some(format!("https://github.com/x/y/issues/{issue}"));
    parent.dispatch_class = Some("implement".to_string());
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                 WHERE id = ?1",
            params![parent_id, format!("-{age_secs} seconds")],
        )
        .unwrap();
    parent_id
}

/// Attach a deferred-dispatch wrapper child in the given status.
fn attach_deferred_wrapper(db: &Database, parent_id: &str, status: &str) -> String {
    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.to_string());
    child.label = crate::agent::DEFERRED_DISPATCH_LABEL.to_string();
    child.dispatch_class = Some("implement".to_string());
    let id = db.create_task(&child).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = ?2 WHERE id = ?1",
            params![id, status],
        )
        .unwrap();
    id
}

/// Attach a deferred wrapper in the given status **with an explicit
/// `completed_at`**, `offset_secs` into the past (mika#2181).
///
/// The sibling `attach_deferred_wrapper` leaves `completed_at` NULL, which
/// is the fail-safe "cannot prove it is recent" path. This one is how a
/// promoted wrapper actually looks: promotion writes both the status and the
/// timestamp.
fn attach_deferred_wrapper_at(
    db: &Database,
    parent_id: &str,
    status: &str,
    completed_at_offset_secs: i64,
) -> String {
    let id = attach_deferred_wrapper(db, parent_id, status);
    db.conn
        .execute(
            "UPDATE tasks SET completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                 WHERE id = ?1",
            params![id, format!("-{completed_at_offset_secs} seconds")],
        )
        .unwrap();
    id
}

/// Attach a real (non-deferred) callback child in the given status.
fn attach_real_callback(db: &Database, parent_id: &str, status: &str) -> String {
    let mut child = callback_task("mika");
    child.parent_task_id = Some(parent_id.to_string());
    child.label = "long_running:run_claude_pilot".to_string();
    child.dispatch_class = Some("implement".to_string());
    let id = db.create_task(&child).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = ?2 WHERE id = ?1",
            params![id, status],
        )
        .unwrap();
    id
}

// -- find_dispatches_expecting_transcripts tests (mika#2040 AC7) --

/// Create a callback dispatch stamped as expecting a transcript, left in
/// `status`, with `updated_at` aged `age_secs` into the past. Mirrors what
/// `spawn_long_running_exec` stamps after a successful spawn.
fn create_stamped_dispatch(db: &Database, agent_id: &str, status: &str, age_secs: i64) -> String {
    let task_id = db.create_task(&callback_task(agent_id)).unwrap();
    db.set_task_metadata_field(
        &task_id,
        crate::task_engine::engine::PILOT_TRANSCRIPT_EXPECTED_KEY,
        &format!("/tmp/pilot-transcripts/{task_id}.jsonl"),
    )
    .unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                 WHERE id = ?1",
            params![task_id, status, format!("-{age_secs} seconds")],
        )
        .unwrap();
    task_id
}

// -- find_childless_stuck_parent_tasks tests (mika#1687) --

/// Create a childless self_dev **issue** parent left `in_progress`, with
/// `updated_at` aged `age_secs` into the past. Returns `parent_id`. The
/// zero-child complement of `create_orphaned_parent_setup` — no callback
/// child is spawned (the silent-pilot-death signature).
fn create_childless_stuck_parent(db: &Database, age_secs: i64) -> String {
    let mut parent = new_task("mika", "Implement mika#1687", "manual", "none");
    parent.source = Some("self_dev".to_string());
    // type defaults to 'issue' via SQL DEFAULT (r#type: None).
    let parent_id = db.create_task(&parent).unwrap();
    db.conn
        .execute(
            "UPDATE tasks SET status = 'in_progress',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
                 WHERE id = ?1",
            params![parent_id, format!("-{age_secs} seconds")],
        )
        .unwrap();
    parent_id
}

// -- find_completable_parent_tasks_on_pr_url tests (mika#1162) --

/// Helper: like `create_orphaned_parent_setup` but stamps a `pr_url` on the
/// parent's metadata to make it a success-side candidate.
fn create_completable_parent_setup(db: &Database, pr_url: &str) -> (String, String) {
    let (parent_id, child_id) = create_orphaned_parent_setup(db);
    let meta = format!(r#"{{"claude_pilot":{{"pr_url":"{pr_url}"}}}}"#);
    db.conn
        .execute(
            "UPDATE tasks SET metadata = ?1 WHERE id = ?2",
            params![meta, parent_id],
        )
        .unwrap();
    (parent_id, child_id)
}

/// Seed a `skill_overrides` row with explicit curator-relevant columns for
/// the archival-candidate query tests (AC9–AC12). Bypasses the upsert
/// helpers so `lifecycle_state` and `last_used_at` can be set to arbitrary
/// values (the helpers only manage `always_on`/`llm_*`/`enabled`).
fn seed_curator_skill_row(
    db: &Database,
    agent_id: &str,
    skill_name: &str,
    lifecycle_state: Option<&str>,
    use_count: i64,
    last_used_at: Option<&str>,
) {
    db.conn
        .execute(
            "INSERT INTO skill_overrides
                    (agent_id, skill_name, lifecycle_state, use_count, last_used_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                agent_id,
                skill_name,
                lifecycle_state,
                use_count,
                last_used_at
            ],
        )
        .unwrap();
}

// -- Task health summary tests --

fn new_task(agent: &str, label: &str, trigger: &str, action: &str) -> NewTask {
    NewTask {
        agent_id: agent.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: trigger.to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: action.to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

// -- Dispatch failure anomaly tests (#980) --

/// Helper: insert a tool_call row directly for testing anomaly #7.
fn insert_tool_call(
    db: &Database,
    agent_id: &str,
    session_id: &str,
    tool_name: &str,
    success: bool,
    created_at: &str,
) {
    let id = uuid::Uuid::new_v4().to_string();
    db.conn
        .execute(
            "INSERT INTO tool_calls (id, agent_id, session_id, tool_name, tool_source, success, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'builtin', ?5, ?6)",
            params![id, agent_id, session_id, tool_name, success as i32, created_at],
        )
        .unwrap();
}

// -- mika#1646: destructive-action repeat detection --

/// Helper: insert a `run_gh` row with a real serialized argv, the shape the
/// repeat query actually matches against.
fn insert_run_gh(db: &Database, agent_id: &str, session_id: &str, argv: &[&str], at: &str) {
    let id = uuid::Uuid::new_v4().to_string();
    let input = serde_json::json!({ "command": argv }).to_string();
    db.conn
        .execute(
            "INSERT INTO tool_calls (id, agent_id, session_id, tool_name, tool_source, input, success, created_at)
                 VALUES (?1, ?2, ?3, 'run_gh', 'builtin', ?4, 1, ?5)",
            params![id, agent_id, session_id, input, at],
        )
        .unwrap();
}

// -- Callback watchdog DB helpers (#959) --

/// Helper: create a parent task and a callback child task, returning (parent_id, child_id).
fn create_callback_task_pair(db: &Database, agent: &str, label: &str) -> (String, String) {
    let parent = new_task(agent, &format!("{label}-parent"), "manual", "none");
    let parent_id = db.create_task(&parent).unwrap();
    let mut child = new_task(agent, label, "callback", "resume_agent");
    child.parent_task_id = Some(parent_id.clone());
    let child_id = db.create_task(&child).unwrap();
    (parent_id, child_id)
}

// ===== Agent Reset tests (#964) =====

/// Helper to create a manual task for the given agent.
fn make_manual_task(agent_id: &str, label: &str) -> NewTask {
    NewTask {
        agent_id: agent_id.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
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
    }
}

// --- kg_resolution_outcome_stats integration tests (#1077) ---

/// Helper: seed a kg_subject_entity row and return its id.
fn seed_subject_entity(db: &Database, docs_root_hash: &str, name: &str) -> i64 {
    let entity_key = format!("concept:{name}");
    db.conn
        .execute(
            "INSERT INTO kg_subject_entities (name, type, entity_key, docs_root_hash, docs_root, confidence, created_at) \
                 VALUES (?1, 'concept', ?2, ?3, '/test/path', 0.9, datetime('now'))",
            params![name, entity_key, docs_root_hash],
        )
        .unwrap();
    db.conn.last_insert_rowid()
}

/// Helper: seed a kg_resolutions_log row with a given outcome and age.
fn seed_resolution_log(
    db: &Database,
    agent_id: &str,
    subject_entity_id: i64,
    outcome: &str,
    days_ago: i32,
) {
    let resolved_at = if days_ago == 0 {
        "datetime('now')".to_string()
    } else {
        format!("datetime('now', '-{days_ago} days')")
    };
    db.conn
        .execute(
            &format!(
                "INSERT INTO kg_resolutions_log \
                     (agent_id, subject_entity_id, outcome, resolved_at, source_extraction_trace_id, resolution_trace_id) \
                     VALUES (?1, ?2, ?3, {resolved_at}, 'trace-test', 'trace-test')"
            ),
            params![agent_id, subject_entity_id, outcome],
        )
        .unwrap();
}

// ===== task_messages (mika#974) =====

/// Helper to create a manual task for task_messages tests.
fn create_test_task(db: &Database, task_id: &str, task_type: &str, parent_id: Option<&str>) {
    db.conn
        .execute(
            "INSERT INTO tasks (id, agent_id, depth, label, trigger_type, action_type, action_config, status, type, parent_task_id)
                 VALUES (?1, 'mika', 0, 'test', 'manual', 'none', '{}', 'pending', ?2, ?3)",
            params![task_id, task_type, parent_id],
        )
        .unwrap();
}

// ── Force-promote regression tests (mika#1453) ──────────────────────

/// Helper: create a deferred callback with a specific dispatch class.
fn deferred_wrapper(agent: &str, class: &str) -> NewTask {
    let mut t = new_task(
        agent,
        "long_running:run_claude_pilot:deferred",
        "callback",
        "resume_agent",
    );
    t.dispatch_class = Some(class.to_string());
    t
}

/// Helper: create a real (non-deferred) callback with a specific dispatch class.
fn real_callback(agent: &str, class: &str) -> NewTask {
    let mut t = new_task(
        agent,
        "long_running:run_claude_pilot",
        "callback",
        "resume_agent",
    );
    t.dispatch_class = Some(class.to_string());
    t
}

// --- cancel_orphan_recurring_tasks (mika#1436) ---

fn recurring_task(agent: &str, label: &str) -> NewTask {
    NewTask {
        agent_id: agent.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some("0 0 * * * *".to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: Some("2286-11-20T17:46:39Z".to_string()),
        timeout_at: None,
        action_type: "inject_context".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    }
}

// --- has_completed_groom_for_issue (#1620, rewritten for mika#2287) ---
//
// The proof is the groom CALLBACK row joined to its parent, never the
// parent row alone: producers write the bare URL and the engine flips the
// parent groom→implement before it is terminal. Every row below is born
// from the production write API (`create_task` + `update_task_completed`
// / `update_task_status`) — no raw SQL INSERT.

pub(crate) const GROOM_ISSUE_URL: &str = "https://github.com/senara-solutions/mika/issues/123";

pub(crate) const GROOM_CALLBACK_PLAN_GROOMED: &str =
    "claude-pilot completed (status: done).\nOutcome: PLAN_GROOMED\nSession: sess-2287";

const GROOM_CALLBACK_PLAN_ITERATE: &str =
    "claude-pilot completed (status: done).\nOutcome: PLAN_ITERATE\nSession: sess-2287";

/// Groom parent as the structural ready-label handler creates it
/// (`trigger_type='manual'`, bare issue URL, `dispatch_class='groom'`).
pub(crate) fn groom_parent(agent_id: &str, reference_url: &str) -> NewTask {
    NewTask {
        agent_id: agent_id.to_string(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: "ready-label: senara-solutions/mika#123".to_string(),
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
        reference_url: Some(reference_url.to_string()),
        source: Some("self_dev".to_string()),
        metadata: None,
        r#type: Some("issue".to_string()),
        dispatch_class: Some("groom".to_string()),
    }
}

/// Groom callback child as `build_callback_task` creates it
/// (`trigger_type='callback'`, `reference_url: None`, class from the skill).
pub(crate) fn groom_callback(agent_id: &str, parent_id: &str, dispatch_class: &str) -> NewTask {
    NewTask {
        agent_id: agent_id.to_string(),
        team_run_id: None,
        parent_task_id: Some(parent_id.to_string()),
        depth: 1,
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
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: Some(dispatch_class.to_string()),
    }
}

/// Build a groom parent + groom callback pair for `agent_id` and complete
/// the callback with `result` through the production write path. Returns
/// `(parent_id, callback_id)`.
pub(crate) fn completed_groom_pair(
    db: &Database,
    agent_id: &str,
    reference_url: &str,
    result: &str,
) -> (String, String) {
    let parent_id = db
        .create_task(&groom_parent(agent_id, reference_url))
        .unwrap();
    db.update_task_status(&parent_id, "in_progress").unwrap();
    let callback_id = db
        .create_task(&groom_callback(agent_id, &parent_id, "groom"))
        .unwrap();
    assert!(
        db.update_task_completed(&callback_id, agent_id, Some(result))
            .unwrap(),
        "callback must complete through the production write path"
    );
    (parent_id, callback_id)
}

/// mika#2310 — harnais isolé pour la porte mika#1620 / mika#2287 (cas 4 et 8,
/// niveau prédicat). Le chemin de module `db::tests::harnais_porte` est ce que
/// le critère de sortie du ticket interroge
/// (`cargo test -p mika-agent harnais_porte`, un filtre par sous-chaîne sur le
/// chemin complet — inchangé par mika#2321).
///
/// Il vivait déjà dans son propre fichier avant mika#2321, mais atteint par un
/// `#[path = "harnais_porte.rs"]` : `mod tests` était alors un module *inline*
/// de `db.rs`, donc la résolution naturelle aurait cherché
/// `db/tests/tests/harnais_porte.rs`. Maintenant que `tests` est lui-même un
/// module de fichier sous `db/tests/`, le fichier est un frère et l'attribut
/// n'a plus d'objet.
mod harnais_porte;
