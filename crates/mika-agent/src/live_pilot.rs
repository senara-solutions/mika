//! Is a pilot alive for this ticket? (mika#2279)
//!
//! **Sole reader** of that question. Two surfaces ask it — the ready-label
//! handler, before it supersedes anything, and the `auto_pull` Phase 2
//! reconciler, before it re-drives a label — and a predicate written twice is a
//! predicate that can disagree with itself. `grooming_marker` had to engrave
//! that lesson once (mika#2158, where promotion and dispatch routing answered
//! the same question differently for months while nothing broke), and mika#2335
//! paid it on this exact path: a second resolver, keyed on a conjunction empty
//! in production, made a supersession kill nobody for two releases while its
//! test stayed green.
//!
//! # The topology, because it is the whole difficulty
//!
//! A dispatch is **two rows**:
//!
//! | row | `trigger_type` | `reference_url` | `process_id` |
//! |---|---|---|---|
//! | **parent** (tracking) | `manual` / `action_type='none'` | **yes** | **never** |
//! | **child** (callback) | `callback` / `resume_agent` | **no** | **yes** (the pgid) |
//!
//! The URL is on the parent, the pgid on the child, nothing carries both. Every
//! predicate that reads one row only is therefore blind to half the state — and
//! `has_active_self_dev_task_for_issue`, which conjoins the URL with a live
//! status on a single row, answers **false** for a ticket whose parent was
//! cancelled 30 seconds after the spawn while its pilot is still writing files.
//! That false answer is the mika#2279 loop: `auto_pull` reads "not in flight",
//! re-drives the `ready` label, the handler supersedes and (since mika#2335)
//! kills the working pilot, registers a duplicate deferred dispatch, and the
//! cancelled parent keeps the predicate false for the next round. Measured on
//! #2276, 2026-09-10: one full turn every ~20 minutes, no human involved.
//!
//! # The URL side of that conjunction is an enumeration, not a prefix
//!
//! `reference_url` is compared against the **closed variant set**
//! ([`crate::task_state::tasks::issue_url_variants`], `IN (?2, ?3)`) since
//! mika#2638. Until then it was a prefix `LIKE`, so this module's sole reader
//! answered about a **numeric neighbour**: the probe for `…/issues/216` found
//! the dispatch of `…/issues/2161`, and gate 2c refused a `labeled ready` on
//! the strength of another ticket's pilot. A predicate written to stop a
//! duplicate dispatch was freezing a healthy ticket instead — the inverse of
//! its job, and the wider of that defect's two blast radii.
//!
//! # Fail-safe direction, and why it is not symmetric
//!
//! **A signal that cannot be read is never a satisfied term.**
//! [`LivePilotVerdict::Unreadable`] is not [`LivePilotVerdict::Alive`]: no
//! readable `process_start_time`, a `process_id` outside `u32`, a DB error — in
//! all three we cannot *prove* a pilot is alive, so we do not block, and the
//! behaviour reverts to what it was before this module existed. That population
//! is the one mika#2335 already counts under `unusable_child_count`, and it is
//! the only case where mika#2279 can still occur. Named rather than hidden.
//!
//! The asymmetry that decides it: a false `Alive` freezes one ticket, and the
//! freeze is **bounded** — the PID watchdog (#959), the silent-stall reaper
//! (#2249/#2277) and the phantom sweep (#1712) all make the row terminal, after
//! which this predicate goes false on its own. A false `None` replays the
//! measured incident, in a loop, every twenty minutes, and burns one point of
//! the mika#2020 re-drive budget each round — three rounds abandon a perfectly
//! healthy ticket to `operator-review`. These are not two errors of equal
//! weight.

use tracing::warn;

use crate::async_db::AsyncDatabase;
use crate::task_engine::process_liveness::is_same_process_alive;
use crate::task_state::tasks::is_terminal_task_status;

/// What [`live_pilot_for_issue`] could establish about a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivePilotVerdict {
    /// A non-terminal callback child carries a pgid whose process is alive and
    /// is the same process instance that was spawned (start-time checked).
    Alive {
        child_task_id: String,
        parent_task_id: String,
        pid: u32,
    },
    /// No child of that shape. The ticket has no pilot.
    None,
    /// The question could not be asked (DB error) or could not be settled
    /// (unreadable `process_start_time`). **Read as "do not block"**, never as
    /// "alive" — see the module doc.
    Unreadable { reason: &'static str },
}

/// A DB error: the traversal itself failed.
pub const UNREADABLE_DB_ERROR: &str = "db_error";
/// A candidate child carries a pgid but no usable `process_start_time`, so the
/// pair that identifies a process *instance* is incomplete and a recycled PID
/// would be indistinguishable from the pilot.
pub const UNREADABLE_NO_START_TIME: &str = "process_start_time_unreadable";

/// Is a pilot alive for `issue_url`?
///
/// Walks parent → child on `parent_task_id`, **without looking at the parent's
/// status** (`Database::find_dispatch_children_for_issue_url`), skips terminal
/// children (a terminal row's pgid is stale and may designate anything by now),
/// and settles liveness with [`is_same_process_alive`] — never a bare
/// `/proc/<pid>` existence check, in which a recycled PID reads exactly like the
/// pilot.
///
/// The first alive child wins. When none is alive but at least one candidate
/// could not be judged, the verdict is [`LivePilotVerdict::Unreadable`] rather
/// than [`LivePilotVerdict::None`]: "we found nothing" and "we could not look"
/// are different facts, and only the first one is evidence.
pub async fn live_pilot_for_issue(db: &AsyncDatabase, issue_url: &str) -> LivePilotVerdict {
    let rows = match db.find_dispatch_children_for_issue_url(issue_url).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                event = "live_pilot_lookup_failed",
                issue_url = %issue_url,
                error = %e,
                "live_pilot: dispatch-child lookup failed — cannot prove a pilot is \
                 alive, so the caller proceeds as it did before mika#2279"
            );
            return LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            };
        }
    };

    let mut unjudgeable = 0usize;
    for row in rows {
        // A terminal child has no pilot left, and its `process_id` is a stale
        // pgid — the same filter, and the same reason, as the supersession
        // disposal in `tracking_cleanup`.
        if is_terminal_task_status(&row.child.status) {
            continue;
        }
        let (Some(start_time), Ok(pid)) = (
            row.child.process_start_time,
            u32::try_from(row.child.process_id),
        ) else {
            unjudgeable += 1;
            continue;
        };
        if is_same_process_alive(pid, start_time) {
            return LivePilotVerdict::Alive {
                child_task_id: row.child.id,
                parent_task_id: row.parent_task_id,
                pid,
            };
        }
    }

    if unjudgeable > 0 {
        // Bounded by construction: only a ticket carrying a non-terminal
        // pid-bearing child with unreadable start-time metadata reaches here,
        // and this is the one remaining shape in which mika#2279 can recur. It
        // is worth one line whenever it is asked.
        warn!(
            event = "live_pilot_unreadable",
            issue_url = %issue_url,
            candidates = unjudgeable,
            "live_pilot: dispatch child carries a pgid but no usable \
             process_start_time — liveness cannot be settled, caller proceeds"
        );
        return LivePilotVerdict::Unreadable {
            reason: UNREADABLE_NO_START_TIME,
        };
    }

    LivePilotVerdict::None
}

/// Is a pilot alive under **this task**? (mika#2653)
///
/// The sibling of [`live_pilot_for_issue`], asked from the other handle: that
/// one starts from an **issue URL**, this one from a **task id**. Both answer
/// the single question this module owns, and both live here rather than at
/// their callers — the module's written contract (*"**Sole reader** of that
/// question"*) goes from two callers to three rather than being copied. *A
/// census is not an allowlist: you add to it, you do not exempt in it*
/// (mika#2633).
///
/// # Why a second reader was needed at all
///
/// `live_pilot_for_issue(db, issue_url)` takes a URL, and `cancel_task` is
/// given a **task id**. The two-row topology this module documents above
/// forbids deriving one from the other: a callback row — the row that carries
/// the pgid, and therefore the probable row of the mika#2653 incident — has
/// **no** `reference_url`, so it has no path to the URL-keyed reader at all.
///
/// # The traversal
///
/// Two handles, in order. The row the caller named may carry the pgid itself
/// (the callback-row case); when it does not, its dispatch children are walked
/// (the parent-row case). Same two filters, and the same reasons, as
/// [`live_pilot_for_issue`]: a terminal child has no pilot left and its pgid is
/// stale, and a pgid with no readable `process_start_time` cannot be told from
/// a recycled PID — so it is **unjudgeable**, never alive, never absent.
///
/// # AC6 différé — phase B, mika#2653
///
/// Cette traversée parent→enfant **existe déjà** dans
/// [`crate::task_engine::process_kill::cancel_task_and_kill`] (posée par
/// mika#2335), et la duplication est **nommée ici plutôt que découverte** : la
/// phase A de mika#2653 livre la garde, la phase B extrait la traversée en un
/// `live_pilot::resolve_task_pilot` que les deux sites appellent, avec son scan
/// de source refusant un troisième. L'ordre inverse est interdit — livrer le
/// détecteur sans la garde ne referme rien. En l'état les deux lectures
/// **divergent délibérément sur un point**, et il faut le savoir avant de les
/// fusionner : le chemin de kill **écarte** un enfant sans `process_start_time`
/// (il ne doit pas signaler un groupe de processus qu'il ne peut pas
/// identifier), là où ce verdict le rend `Unreadable` (il ne peut pas prouver
/// l'absence de pilote). Deux dispositions correctes pour deux questions
/// différentes, à préserver à l'identique lors de l'extraction.
pub async fn live_pilot_for_task(db: &AsyncDatabase, task_id: &str) -> LivePilotVerdict {
    // Le pgid porté par la ligne nommée (cas de la ligne callback).
    let named = match db.get_task(task_id).await {
        Ok(t) => t,
        Err(e) => {
            warn!(
                event = "live_pilot_task_lookup_failed",
                task_id = %task_id,
                error = %e,
                "live_pilot: task lookup failed — liveness cannot be established"
            );
            return LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            };
        }
    };
    let Some(task) = named else {
        // Une tâche inexistante ne porte pas de pilote. Inatteignable depuis la
        // garde (qui tourne après `validate_task_exists`), et c'est le verdict
        // juste pour tout autre appelant.
        return LivePilotVerdict::None;
    };

    if let Some(pid) = task.process_id {
        let start_time: Option<u64> = task
            .metadata
            .as_deref()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
            .and_then(|v| v.get("process_start_time")?.as_str()?.parse().ok());
        return match (start_time, u32::try_from(pid)) {
            (Some(st), Ok(p)) if is_same_process_alive(p, st) => LivePilotVerdict::Alive {
                child_task_id: task.id.clone(),
                parent_task_id: task
                    .parent_task_id
                    .clone()
                    .unwrap_or_else(|| task.id.clone()),
                pid: p,
            },
            (Some(_), Ok(_)) => LivePilotVerdict::None,
            // Pgid présent, instance non identifiable : on ne peut pas prouver
            // l'absence de pilote, donc ce n'est pas `None`.
            _ => {
                warn!(
                    event = "live_pilot_unreadable",
                    task_id = %task_id,
                    candidates = 1,
                    "live_pilot: the named row carries a pgid but no usable \
                     process_start_time — liveness cannot be settled"
                );
                LivePilotVerdict::Unreadable {
                    reason: UNREADABLE_NO_START_TIME,
                }
            }
        };
    }

    // Pas de pgid sur la ligne nommée : c'est une ligne parent, le pgid vit sur
    // un de ses enfants de dispatch.
    let children = match db.find_dispatch_children_with_pid(task_id).await {
        Ok(rows) => rows,
        Err(e) => {
            warn!(
                event = "live_pilot_lookup_failed",
                task_id = %task_id,
                error = %e,
                "live_pilot: dispatch-child lookup failed — liveness cannot be \
                 established for this task"
            );
            return LivePilotVerdict::Unreadable {
                reason: UNREADABLE_DB_ERROR,
            };
        }
    };

    let mut unjudgeable = 0usize;
    for child in children {
        if is_terminal_task_status(&child.status) {
            continue;
        }
        let (Some(start_time), Ok(pid)) =
            (child.process_start_time, u32::try_from(child.process_id))
        else {
            unjudgeable += 1;
            continue;
        };
        if is_same_process_alive(pid, start_time) {
            return LivePilotVerdict::Alive {
                child_task_id: child.id,
                parent_task_id: task_id.to_string(),
                pid,
            };
        }
    }

    if unjudgeable > 0 {
        warn!(
            event = "live_pilot_unreadable",
            task_id = %task_id,
            candidates = unjudgeable,
            "live_pilot: dispatch child carries a pgid but no usable \
             process_start_time — liveness cannot be settled"
        );
        return LivePilotVerdict::Unreadable {
            reason: UNREADABLE_NO_START_TIME,
        };
    }

    LivePilotVerdict::None
}

impl LivePilotVerdict {
    /// `true` only for [`LivePilotVerdict::Alive`].
    ///
    /// Exists so a caller cannot accidentally write `!matches!(v, None)` and
    /// turn `Unreadable` into a block — the one inversion that would freeze a
    /// ticket on a supposition.
    pub fn is_alive(&self) -> bool {
        matches!(self, Self::Alive { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, NewTask};

    const AGENT: &str = "mika";
    const URL: &str = "https://github.com/senara-solutions/mika/issues/2276";

    fn test_db() -> AsyncDatabase {
        let db = Database::open_in_memory().expect("open in-memory DB");
        AsyncDatabase::new_with_agent(db, AGENT)
    }

    async fn seed_parent(db: &AsyncDatabase, url: &str, status: &str) -> String {
        let id = db
            .create_task(NewTask {
                agent_id: AGENT.to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: format!("ready-label: {url}"),
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
                created_by_session: Some("s".to_string()),
                created_trace_id: None,
                reference_url: Some(url.to_string()),
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: Some("issue".to_string()),
                dispatch_class: Some("implement".to_string()),
            })
            .await
            .expect("create parent");
        if status != "pending" {
            db.update_task_status(&id, status)
                .await
                .expect("set parent status");
        }
        id
    }

    async fn seed_child(
        db: &AsyncDatabase,
        parent: &str,
        status: &str,
        pid: i64,
        start_time: Option<u64>,
    ) -> String {
        let id = db
            .create_task(NewTask {
                agent_id: AGENT.to_string(),
                team_run_id: None,
                parent_task_id: Some(parent.to_string()),
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
                created_by_session: Some("s".to_string()),
                created_trace_id: None,
                reference_url: None,
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: None,
                dispatch_class: Some("implement".to_string()),
            })
            .await
            .expect("create child");
        if status != "pending" {
            db.update_task_status(&id, status)
                .await
                .expect("set child status");
        }
        db.set_task_process_id(&id, Some(pid))
            .await
            .expect("record pgid");
        if let Some(st) = start_time {
            db.set_task_metadata_field(&id, "process_start_time", &st.to_string())
                .await
                .expect("record start time");
        }
        id
    }

    /// Our own process: alive, with a start time that matches by construction.
    fn self_pid_and_start() -> (i64, u64) {
        let pid = std::process::id();
        let st = crate::task_engine::process_liveness::read_process_start_time(pid)
            .expect("read own start time");
        (i64::from(pid), st)
    }

    /// The whole point of the module, stated as one test: **a terminal parent
    /// does not hide its pilot.** `cancelled` is the state measured on #2276;
    /// `failed` and `delivered` are there so nobody "fixes" the query later by
    /// adding back a parent-status predicate that happens to spare one of them.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn a_terminal_parent_does_not_hide_a_live_child() {
        for parent_status in ["cancelled", "failed", "delivered", "in_progress"] {
            let db = test_db();
            let (pid, st) = self_pid_and_start();
            let parent = seed_parent(&db, URL, parent_status).await;
            let child = seed_child(&db, &parent, "pending", pid, Some(st)).await;

            match live_pilot_for_issue(&db, URL).await {
                LivePilotVerdict::Alive {
                    child_task_id,
                    parent_task_id,
                    pid: got,
                } => {
                    assert_eq!(child_task_id, child);
                    assert_eq!(parent_task_id, parent);
                    assert_eq!(i64::from(got), pid);
                }
                other => panic!(
                    "INVARIANT VIOLÉ : parent `{parent_status}` + pilote vif a rendu \
                     {other:?} — c'est le prédicat aveugle de mika#2279"
                ),
            }
        }
    }

    /// The `?phase=groom` variant rides on the same closed-variant-set rule as
    /// `has_active_self_dev_task_for_issue` (mika#2638). It is the half of that
    /// fix that must keep passing: the enumeration narrows the match to a
    /// delimited number **without** dropping the declared variant.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn the_groom_phase_url_variant_is_covered() {
        let db = test_db();
        let (pid, st) = self_pid_and_start();
        let parent = seed_parent(&db, &format!("{URL}?phase=groom"), "cancelled").await;
        seed_child(&db, &parent, "pending", pid, Some(st)).await;

        assert!(
            live_pilot_for_issue(&db, URL).await.is_alive(),
            "l'énumération de l'ensemble clos doit couvrir la variante \
             ?phase=groom (mika#2638 — ce message disait « le LIKE préfixe », \
             et remis en l'état il réinstallerait le modèle mental qui a \
             produit le faux appariement sur le voisin numérique)"
        );
    }

    /// Négatif — a **terminal child**. Its pgid is stale: reading it as a live
    /// pilot would freeze the ticket on a row that designates nothing.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn a_terminal_child_is_not_a_live_pilot() {
        for child_status in ["delivered", "cancelled", "failed", "completed", "expired"] {
            let db = test_db();
            let (pid, st) = self_pid_and_start();
            let parent = seed_parent(&db, URL, "cancelled").await;
            seed_child(&db, &parent, child_status, pid, Some(st)).await;

            assert_eq!(
                live_pilot_for_issue(&db, URL).await,
                LivePilotVerdict::None,
                "un enfant `{child_status}` porte un pgid périmé, pas un pilote"
            );
        }
    }

    /// Négatif — a dead pilot. PID 999_999_999 does not exist, so the process
    /// check fails and the verdict is `None`, not `Unreadable`: the start time
    /// was readable, the answer is simply "not alive".
    #[tokio::test]
    async fn a_dead_pilot_is_none_not_unreadable() {
        let db = test_db();
        let parent = seed_parent(&db, URL, "cancelled").await;
        seed_child(&db, &parent, "pending", 999_999_999, Some(12345)).await;

        assert_eq!(live_pilot_for_issue(&db, URL).await, LivePilotVerdict::None);
    }

    /// Négatif — an unreadable `process_start_time`. The fail-safe is attested,
    /// not assumed: `Unreadable` is a distinct verdict and `is_alive()` is false
    /// for it, so the caller proceeds exactly as it did before mika#2279.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn a_child_without_a_start_time_is_unreadable_not_alive() {
        let db = test_db();
        let (pid, _) = self_pid_and_start();
        let parent = seed_parent(&db, URL, "cancelled").await;
        seed_child(&db, &parent, "pending", pid, None).await;

        let verdict = live_pilot_for_issue(&db, URL).await;
        assert_eq!(
            verdict,
            LivePilotVerdict::Unreadable {
                reason: UNREADABLE_NO_START_TIME
            },
            "sans start_time la paire qui identifie une *instance* est incomplète"
        );
        assert!(
            !verdict.is_alive(),
            "INVARIANT VIOLÉ : un signal illisible est devenu un terme satisfait"
        );
    }

    /// Négatif — another ticket entirely. The predicate is carried by the URL;
    /// without this, an over-eager traversal would pass every test above.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn a_live_pilot_on_another_issue_does_not_answer_for_this_one() {
        let db = test_db();
        let (pid, st) = self_pid_and_start();
        let other = seed_parent(
            &db,
            "https://github.com/senara-solutions/mika/issues/9999",
            "cancelled",
        )
        .await;
        seed_child(&db, &other, "pending", pid, Some(st)).await;

        assert_eq!(live_pilot_for_issue(&db, URL).await, LivePilotVerdict::None);
    }

    /// Négatif — no dispatch at all. The nominal answer for the overwhelming
    /// majority of tickets, and the one that must stay cheap and false.
    #[tokio::test]
    async fn a_ticket_with_no_dispatch_has_no_live_pilot() {
        let db = test_db();
        seed_parent(&db, URL, "pending").await;

        assert_eq!(live_pilot_for_issue(&db, URL).await, LivePilotVerdict::None);
    }

    /// A live child is found even when an unjudgeable sibling is examined first
    /// — `Alive` wins over `Unreadable`, which is the safe direction here (the
    /// evidence is positive) and also the only one that is true.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn one_unjudgeable_sibling_does_not_mask_a_live_child() {
        let db = test_db();
        let (pid, st) = self_pid_and_start();
        let parent = seed_parent(&db, URL, "cancelled").await;
        let a = seed_child(&db, &parent, "pending", pid, None).await;
        let b = seed_child(&db, &parent, "pending", pid, Some(st)).await;
        // `ORDER BY child.id` is a UUID order, so force the unjudgeable one
        // first — otherwise the test would only sometimes exercise the path.
        db.set_task_id_for_test(&a, "00000000-0000-0000-0000-00000000000a")
            .await
            .expect("force order");
        db.set_task_id_for_test(&b, "00000000-0000-0000-0000-00000000000b")
            .await
            .expect("force order");

        assert!(
            live_pilot_for_issue(&db, URL).await.is_alive(),
            "un frère illisible ne doit pas masquer un pilote vif prouvé"
        );
    }

    // ───────── mika#2653 — la même question, posée depuis un id de tâche ─────────

    /// La ligne **nommée** porte le pgid : le cas d'une ligne callback, c'est-à-dire
    /// le cas probable du constat du 2026-10-02.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2653_la_ligne_nommee_porte_le_pilote() {
        let db = test_db();
        let (pid, st) = self_pid_and_start();
        let parent = seed_parent(&db, URL, "in_progress").await;
        let child = seed_child(&db, &parent, "pending", pid, Some(st)).await;

        match live_pilot_for_task(&db, &child).await {
            LivePilotVerdict::Alive {
                child_task_id,
                pid: got,
                ..
            } => {
                assert_eq!(child_task_id, child);
                assert_eq!(i64::from(got), pid);
            }
            other => panic!(
                "la ligne qui PORTE le pgid doit rendre `Alive`, pas {other:?} — \
                 une ligne callback n'a pas d'URL, donc aucun chemin vers \
                 `live_pilot_for_issue`"
            ),
        }
    }

    /// Le cas de la **topologie à deux lignes** : la tâche nommée est le parent,
    /// qui ne porte jamais de pgid, et le pilote vit sur un enfant non terminal.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2653_un_parent_sans_pgid_trouve_le_pilote_de_son_enfant() {
        let db = test_db();
        let (pid, st) = self_pid_and_start();
        let parent = seed_parent(&db, URL, "in_progress").await;
        let child = seed_child(&db, &parent, "pending", pid, Some(st)).await;

        match live_pilot_for_task(&db, &parent).await {
            LivePilotVerdict::Alive {
                child_task_id,
                parent_task_id,
                ..
            } => {
                assert_eq!(child_task_id, child);
                assert_eq!(parent_task_id, parent);
            }
            other => panic!("un parent dont l'enfant porte un pilote vif a rendu {other:?}"),
        }
    }

    /// Négatif — un enfant **terminal** : son pgid est périmé et ne désigne plus
    /// rien. Même filtre, et même raison, que [`live_pilot_for_issue`].
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2653_un_enfant_terminal_nest_pas_un_pilote() {
        for status in ["delivered", "cancelled", "failed", "completed", "expired"] {
            let db = test_db();
            let (pid, st) = self_pid_and_start();
            let parent = seed_parent(&db, URL, "in_progress").await;
            seed_child(&db, &parent, status, pid, Some(st)).await;

            assert_eq!(
                live_pilot_for_task(&db, &parent).await,
                LivePilotVerdict::None,
                "un enfant `{status}` porte un pgid périmé, pas un pilote"
            );
        }
    }

    /// Négatif — un pilote mort avec un `start_time` lisible : la réponse est
    /// `None`, **jamais** `Unreadable`. Le signal était lisible ; il dit « pas
    /// vivant ».
    #[tokio::test]
    async fn mika2653_un_pilote_mort_est_none_pas_unreadable() {
        let db = test_db();
        let parent = seed_parent(&db, URL, "in_progress").await;
        let child = seed_child(&db, &parent, "pending", 999_999_999, Some(12345)).await;

        assert_eq!(
            live_pilot_for_task(&db, &parent).await,
            LivePilotVerdict::None
        );
        assert_eq!(
            live_pilot_for_task(&db, &child).await,
            LivePilotVerdict::None
        );
    }

    /// Le terme qui décide du fail-closed de la garde mika#2653 : un pgid sans
    /// `process_start_time` lisible est **non prouvable**, donc `Unreadable` —
    /// par les deux handles.
    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn mika2653_un_pgid_sans_start_time_est_unreadable() {
        let db = test_db();
        let (pid, _) = self_pid_and_start();
        let parent = seed_parent(&db, URL, "in_progress").await;
        let child = seed_child(&db, &parent, "pending", pid, None).await;

        for (handle, who) in [(&parent, "parent"), (&child, "ligne nommée")] {
            let verdict = live_pilot_for_task(&db, handle).await;
            assert_eq!(
                verdict,
                LivePilotVerdict::Unreadable {
                    reason: UNREADABLE_NO_START_TIME
                },
                "{who} : sans start_time la paire qui identifie une *instance* est \
                 incomplète, et un PID recyclé serait indistinguable du pilote"
            );
            assert!(
                !verdict.is_alive(),
                "INVARIANT VIOLÉ : un signal illisible est devenu un terme satisfait"
            );
        }
    }

    /// Négatif — aucune ligne ne porte de pgid : le cas nominal de la quasi-totalité
    /// des tâches, et celui qui doit rester bon marché et faux.
    #[tokio::test]
    async fn mika2653_une_tache_sans_pilote_rend_none() {
        let db = test_db();
        let parent = seed_parent(&db, URL, "pending").await;

        assert_eq!(
            live_pilot_for_task(&db, &parent).await,
            LivePilotVerdict::None
        );
    }

    /// Négatif — une tâche inexistante ne porte pas de pilote. Inatteignable
    /// depuis la garde (qui tourne après `validate_task_exists`), et c'est le
    /// verdict juste pour tout autre appelant.
    #[tokio::test]
    async fn mika2653_une_tache_inexistante_rend_none() {
        let db = test_db();
        assert_eq!(
            live_pilot_for_task(&db, "00000000-0000-0000-0000-000000000000").await,
            LivePilotVerdict::None
        );
    }
}
