//! Database methods for the `tasks` table — the `Task CRUD` section formerly
//! inlined in `db.rs` (mika#2396).
//!
//! Follows the `operational.rs` / `kg_schema.rs` pattern for DB module
//! separation: every method stays an inherent method on [`Database`], reached
//! through the same `db.foo(…)` path as before. `async_db.rs` and every other
//! caller are untouched by the move.
//!
//! ## Two items of this domain deliberately stayed in the parent
//!
//! `Database::row_to_task` and `Database::TASK_COLUMNS` — together *the* `Task`
//! deserializer — are still defined in `db.rs`. They have callers on both sides
//! of the boundary: the `Rewind` and `Dashboard` sections consume them at ten
//! sites. `db::tasks` is a **child** of `db`, so it sees the parent's private
//! items without a single visibility change. Moving them the other way would
//! have made `db` and `db::tasks` siblings with respect to them, forcing
//! `pub(super)` on both the `fn` **and** the `const` — two visibility widenings,
//! to house the `Task` deserializer in a file that is not its only consumer.
//! The price of leaving them behind is ~1.5 KB of `db.rs` that still looks like
//! Task CRUD; it is the cheaper of the two, and it is why this whole move needs
//! no visibility change anywhere.
//!
//! `format_age` is the mirror case: defined in `db.rs`, called from
//! `get_task_health_summary` here, visible through that same parent/child
//! relation, and merely brought into scope by the glob below.
//!
//! ## Six methods of the old line range are deliberately NOT here
//!
//! Their table is not `tasks`: `count_recent_audit_events_for_target`,
//! `count_audit_events_by_tool_name`, `get_audit_event_target_keys_by_tool_name`
//! and `get_audit_event_rows_by_tool_name` (`audit_events`), plus
//! `get_schema_meta` and `stamp_schema_meta_epoch_if_absent` (`schema_meta`).
//! A file named `tasks.rs` carrying them would state something false on its
//! first line, and a file's name is what the next reader searches on.
//!
//! The cost is that the move spans **six** disjoint ranges instead of one
//! contiguous block — the four the plan's P2 enumerates, plus the two its table
//! implies without counting: `row_to_task` / `TASK_COLUMNS` split the first
//! range in two, and the three `#[doc(hidden)]` `UPDATE tasks` helpers
//! (`backdate_task_updated_at`, `backdate_task_completed_at`,
//! `set_task_id_for_test`) sit between two `audit_events` methods that stayed.
//! Each range is verified as a verbatim move on its own, which is what makes
//! the extra ranges payable.

// Glob rather than an explicit list, deliberately: this file is a continuation
// of `impl Database` and must see exactly the name environment the section had
// while it lived in `db.rs` — the parent's imports plus its private items
// (`format_age`, `RECURRING_ZOMBIE_GRACE_HOURS`, the worktree-claim and
// dispatch-slot constants, the `task_state::tasks` re-export). A child module
// sees its parent's private items, so this costs no visibility change; that
// property is the entire reason the extraction is mechanical.
use super::*;

/// L'acte d'estampiller `fired_at`, écrit **une seule fois** (mika#2133 AC4).
///
/// Fragment SQL à interpoler dans la clause `SET` d'un `UPDATE tasks`. Il pose
/// l'instant courant **si et seulement si** la colonne est encore NULL, et rend
/// la valeur existante sinon.
///
/// # Pourquoi un fragment et pas une fonction
///
/// AC4 demande « une seule primitive d'estampillage » et précise sa lettre :
/// *ne pas dupliquer le `UPDATE`*. La fusion en une fonction unique n'est pas
/// disponible — les quatre écrivains diffèrent par leur garde
/// (`trigger_type = 'manual'`, `status IN (…)`, `?1 IS NOT NULL`), par ce
/// qu'ils écrivent d'autre (`status`, `process_id`) et par leur atomicité
/// requise : la transition et le stamp doivent rester **un seul acte**, donc un
/// seul aller-retour. Un second `UPDATE` peut échouer entre les deux écritures
/// et laisser exactement la ligne `in_progress` sans `fired_at` que ce ticket
/// ferme. Ce que la constante garantit à la place est plus étroit et suffisant :
/// une seule **définition textuelle** de l'acte, plus la garde de source
/// `mika2133_fired_at_has_a_single_literal_definition` qui refuse qu'un
/// cinquième écrivain en écrive une seconde en silence.
///
/// # NULL-only, jamais un écrasement
///
/// Les faucheurs mesurent l'âge d'un dispatch depuis ce champ
/// (`MIKA_PILOT_STALL_REAP_AGE_SECONDS`, le balayage phantom, le watchdog
/// #959, et la sonde `long_running` de [`Database::get_task_health_summary`],
/// qui trie littéralement sur `fired_at ASC` avec un `fired_at < ?2`), donc un
/// re-stamp remettrait cet âge à zéro sous eux. Le
/// besoin n'est pas théorique : le tour de livraison d'un callback est
/// explicitement ré-essayé par le backoff de mika#2179, et sans cette clause un
/// callback en quarantaine verrait son `fired_at` avancer d'une heure à chaque
/// tentative — une estampille qui a l'air d'une mesure et qui suit l'horloge du
/// réessai.
///
/// **Exception déclarée, non exemptée : [`Database::claim_and_fire_task`]**
/// écrase délibérément (mika#2133 D4). Pour une tâche `recurring`, l'estampille
/// dit « dernier tir », pas « premier tir » — c'est la seule population qui
/// fonctionnait avant ce ticket (56/64) et l'uniformiser vers NULL-only la
/// figerait sur son tir inaugural.
pub(crate) const FIRED_AT_STAMP_IF_NULL: &str = "fired_at = CASE \
     WHEN fired_at IS NULL \
     THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') \
     ELSE fired_at END";

impl Database {
    pub fn create_task(&self, task: &NewTask) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        // `type` defaults to "issue" when the caller passes None or an empty string.
        // The DB CHECK constraint enforces the same allowlist as VALID_TASK_TYPES.
        let task_type = task
            .r#type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(TASK_TYPE_ISSUE);
        self.conn.execute(
            "INSERT INTO tasks (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                input_context, created_by_session, created_trace_id,
                reference_url, source, metadata, type, dispatch_class
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17, ?18,
                ?19, ?20, ?21, ?22, ?23
             )",
            params![
                id,
                task.agent_id,
                task.team_run_id,
                task.parent_task_id,
                task.depth,
                task.label,
                task.trigger_type,
                task.cron_expr,
                task.event_source,
                task.event_offset_secs,
                task.condition_expr,
                task.next_fire_at,
                task.timeout_at,
                task.action_type,
                task.action_config,
                task.input_context,
                task.created_by_session,
                task.created_trace_id,
                task.reference_url,
                task.source,
                task.metadata,
                task_type,
                task.dispatch_class,
            ],
        )?;
        Ok(id)
    }

    /// Insert a recurring task only if no task with the same (agent_id, label) and
    /// trigger_type='recurring' exists. Returns the task ID if created, None if
    /// already existed or if a recent dead sibling refuses re-registration.
    ///
    /// **mika#1742 Problem B — refuse-to-zombie guard.** The partial unique index
    /// `idx_tasks_unique_recurring` explicitly excludes `'cancelled' | 'failed' |
    /// 'expired' | 'delivered'` statuses. So a bare `INSERT OR IGNORE` silently
    /// creates a fresh row whenever every prior instance died — and every
    /// mika-spirit restart re-triggers the fresh registration. That's the
    /// curator_review zombie root cause (8 dead Mika rows across 8 days per
    /// root-claude's forensic).
    ///
    /// This guard adds a pre-insert query: if any recurring row for the same
    /// `(agent_id, label)` was `failed`/`cancelled` within the last
    /// [`RECURRING_ZOMBIE_GRACE_HOURS`] window, refuse to re-register and log a
    /// `warn!` so the operator sees the surface. The operator can:
    ///
    /// - Wait for the grace window to elapse (fresh registration then proceeds).
    /// - Investigate the previous instance's failure via `mika tasks get <id>`.
    /// - Manually clear the dead row and let the next startup re-register.
    ///
    /// **mika#2271 — config-cancel exemption.** A `cancelled` row is not always
    /// a death: a knob (`MIKA_DEV_AUTO_PULL=0`) or an `identity.toml` toggle
    /// cancels the recurring row on purpose, and removing the knob is meant to
    /// bring the task back. Rows whose `metadata` carries
    /// [`RECURRING_CONFIG_CANCEL_REVERTED_PATH`] — written by
    /// [`Database::revert_config_cancel_recurring_task`] when the boot path
    /// re-declares the task as wanted — are therefore skipped by this guard.
    /// `failed` / `expired` rows are never exempted.
    ///
    /// **mika#2337 — unknown-trigger exemption, single-use.** A death caused by
    /// [`crate::task_engine::DispatchError::UnknownTrigger`] carries
    /// [`RECURRING_UNKNOWN_TRIGGER_PATH`], and this guard skips it: the binary
    /// that re-registers a trigger name is the binary that carries its match
    /// arm, so that death predicts nothing about the next one and the veto only
    /// prolongs the outage. The exemption is **spent** when honoured — the
    /// marker is removed and [`RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH`]
    /// is written in its place — so a *second* unknown-trigger death on the same
    /// label inside the window meets a fully armed veto. The lift buys one
    /// restart, not immunity.
    ///
    /// Non-goal here: fixing the *underlying* dispatch failure for Mika's
    /// specific `curator_review` (Problem A in the ticket). Root-claude's
    /// diagnosis notes PR#1726 (RouteFuture/dashmap wedge) likely already
    /// resolves it. Verification is a Phase-2 follow-up under the Problem-A
    /// investigation.
    pub fn create_recurring_task_if_absent(&self, task: NewTask) -> Result<Option<String>> {
        // mika#2337 — has a lift already been spent on this (agent, label)
        // inside the grace window? The marker rides on the row it was spent on,
        // so it ages out of the window together with the death it absolved:
        // past the grace, the exemption re-arms rather than being lost for good.
        let lift_already_spent: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks
               WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
                 AND trigger_type = 'recurring'
                 AND updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                 AND json_valid(metadata)
                 AND COALESCE(json_extract(metadata, ?4), 0) = 1)",
            params![
                task.agent_id,
                task.label,
                RECURRING_ZOMBIE_GRACE_SQL,
                RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH
            ],
            |r| r.get::<_, i64>(0).map(|n| n == 1),
        )?;

        // Zombie guard — refuse to re-register a recurring label whose most
        // recent instance died in the grace window.
        let dead_sibling: Option<(String, String, String)> = self
            .conn
            .query_row(
                "SELECT id, status, updated_at FROM tasks
                 WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
                   AND trigger_type = 'recurring'
                   AND status IN ('failed', 'cancelled', 'expired')
                   AND updated_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)
                   AND NOT (json_valid(metadata)
                            AND COALESCE(json_extract(metadata, ?4), 0) = 1)
                   AND NOT (?5 = 0
                            AND json_valid(metadata)
                            AND COALESCE(json_extract(metadata, ?6), 0) = 1)
                 ORDER BY updated_at DESC LIMIT 1",
                params![
                    task.agent_id,
                    task.label,
                    RECURRING_ZOMBIE_GRACE_SQL,
                    RECURRING_CONFIG_CANCEL_REVERTED_PATH,
                    i64::from(lift_already_spent),
                    RECURRING_UNKNOWN_TRIGGER_PATH
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((prev_id, prev_status, prev_updated)) = dead_sibling {
            tracing::warn!(
                agent_id = %task.agent_id,
                label = %task.label,
                previous_task_id = %prev_id,
                previous_status = %prev_status,
                previous_updated_at = %prev_updated,
                grace_hours = RECURRING_ZOMBIE_GRACE_HOURS,
                "mika#1742: refusing to re-register recurring task — recent same-label \
                 instance ended in a terminal-failure state. Investigate root cause \
                 (`mika tasks get {prev_id}`) before re-enabling; re-registration \
                 automatically re-attempts after the grace window elapses."
            );
            return Ok(None);
        }

        let id = uuid::Uuid::new_v4().to_string();
        let task_type = task
            .r#type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(TASK_TYPE_ISSUE);
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO tasks
             (id, agent_id, team_run_id, parent_task_id, depth, label,
              trigger_type, cron_expr, event_source, event_offset_secs,
              condition_expr, next_fire_at, timeout_at, action_type,
              action_config, status, input_context, created_by_session, created_trace_id,
              reference_url, source, metadata, type)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'recurring_active',?16,?17,?18,?19,?20,?21,?22)",
            params![
                id, task.agent_id, task.team_run_id, task.parent_task_id,
                task.depth, task.label, task.trigger_type, task.cron_expr,
                task.event_source, task.event_offset_secs, task.condition_expr,
                task.next_fire_at, task.timeout_at, task.action_type,
                task.action_config, task.input_context, task.created_by_session,
                task.created_trace_id, task.reference_url, task.source,
                task.metadata, task_type
            ],
        )?;
        if n > 0 {
            // mika#2337 — the registration went through; if an unknown-trigger
            // marker is what let it through, spend it now. Conditioned on
            // `!lift_already_spent` so a re-registration that never needed the
            // exemption cannot silently burn a fresh one.
            if !lift_already_spent
                && let Err(e) = self.spend_unknown_trigger_lift(&task.agent_id, &task.label)
            {
                // Fail-open on the *bookkeeping*, never on the guard: the row is
                // already registered and the engine is running again. An unspent
                // marker costs at most one extra lift on the next death;
                // refusing the registration here would restore the outage this
                // whole path exists to end.
                tracing::warn!(
                    agent_id = %task.agent_id,
                    label = %task.label,
                    error = %e,
                    "mika#2337: failed to spend the unknown-trigger veto lift — \
                     the next unknown-trigger death on this label may be \
                     forgiven a second time"
                );
            }
            Ok(Some(id))
        } else {
            Ok(None) // already existed (unique-index conflict on an active row)
        }
    }

    /// mika#2337 — consume the unknown-trigger exemption for `(agent_id, label)`.
    ///
    /// Removes [`RECURRING_UNKNOWN_TRIGGER_PATH`] from every row that carries it
    /// and writes [`RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH`] in the same
    /// statement. Returns the number of rows whose marker was spent.
    ///
    /// **Why both halves.** Removing the marker alone does not bound anything:
    /// a second death writes a *fresh* marker on a *fresh* row and would absolve
    /// itself. The `lift_consumed` marker is what the next call reads to find
    /// the exemption already spent — and because it is never given a new
    /// `updated_at`, it ages out of the grace window together with the death it
    /// absolved, so the exemption re-arms rather than being lost permanently.
    fn spend_unknown_trigger_lift(&self, agent_id: &str, label: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET metadata = json_set(
                     json_remove(
                         CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                         ?3),
                     ?4, 1)
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'recurring'
               AND json_valid(metadata)
               AND COALESCE(json_extract(metadata, ?3), 0) = 1",
            params![
                agent_id,
                label,
                RECURRING_UNKNOWN_TRIGGER_PATH,
                RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH
            ],
        )?;
        Ok(n)
    }

    /// Get the cron expression for an existing recurring task by label.
    pub fn get_recurring_task_cron(&self, agent_id: &str, label: &str) -> Result<Option<String>> {
        let cron: Option<Option<String>> = self.conn.query_row(
            "SELECT cron_expr FROM tasks WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring' AND status IN ('recurring_active', 'pending', 'in_progress') LIMIT 1",
            params![agent_id, label],
            |r| r.get(0),
        ).optional()?;
        Ok(cron.flatten())
    }

    /// Update the cron expression and next_fire_at for an existing recurring task.
    pub fn update_recurring_task_cron(
        &self,
        agent_id: &str,
        label: &str,
        new_cron: &str,
        next_fire_at: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET cron_expr = ?1, next_fire_at = ?2
             WHERE agent_id = ?3 AND label = ?4 AND trigger_type = 'recurring'
               AND status IN ('recurring_active', 'pending', 'in_progress')",
            params![new_cron, next_fire_at, agent_id, label],
        )?;
        Ok(())
    }

    /// Mark every `cancelled` recurring row for `(agent_id, label)` as a
    /// *reverted config cancel* (mika#2271). Returns the number of rows marked.
    ///
    /// Called from `ensure_recurring_task` — whose invocation *is* the config
    /// declaring the task must run — right before re-registration. Without it,
    /// the knob-off → knob-on cycle leaves the feeder dead: the knob-off boot
    /// cancels the row, and the knob-on boot hits the mika#1742 refuse-to-zombie
    /// guard, which cannot tell a deliberate config cancel from a terminal
    /// failure.
    ///
    /// Deliberately does **not** touch `updated_at`: the row keeps the timestamp
    /// of its actual cancel so the audit trail stays truthful. Exemption is
    /// carried by the metadata marker, not by ageing the row out of the grace
    /// window. Non-JSON `metadata` is replaced by a fresh object rather than
    /// erroring — the marker matters more than a malformed legacy blob.
    pub fn revert_config_cancel_recurring_task(
        &self,
        agent_id: &str,
        label: &str,
    ) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET metadata = json_set(
                     CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                     ?3, 1)
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'recurring'
               AND status = 'cancelled'
               AND NOT (json_valid(metadata)
                        AND COALESCE(json_extract(metadata, ?3), 0) = 1)",
            params![agent_id, label, RECURRING_CONFIG_CANCEL_REVERTED_PATH],
        )?;
        Ok(n)
    }

    /// Mark a recurring task's row as having died on an **unknown trigger**
    /// (mika#2337). Returns the number of rows marked (0 or 1).
    ///
    /// Called from `TaskEngine::fire_task` on the
    /// [`crate::task_engine::DispatchError::UnknownTrigger`] arm, *before*
    /// `update_task_failed`, so the row is never a plain unmarked corpse — not
    /// even for the instant between the two writes.
    ///
    /// Writes the integer `1`, not the string `"1"`: the reader compares with
    /// `COALESCE(json_extract(metadata, ?), 0) = 1`, and SQLite orders INTEGER
    /// before TEXT, so `'1' = 1` is false. A string would produce a marker that
    /// is visible in the row and invisible to the guard — the worst shape.
    ///
    /// Deliberately does **not** touch `updated_at`, mirroring
    /// [`Database::revert_config_cancel_recurring_task`]: the row keeps the
    /// timestamp of its actual death so the audit trail stays truthful, and the
    /// exemption is carried by the marker rather than by ageing the row out of
    /// the grace window. Non-JSON `metadata` is replaced by a fresh object
    /// rather than erroring — the marker matters more than a malformed legacy
    /// blob.
    pub fn mark_recurring_unknown_trigger(&self, task_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET metadata = json_set(
                     CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                     ?2, 1)
             WHERE id = ?1 AND trigger_type = 'recurring'",
            params![task_id, RECURRING_UNKNOWN_TRIGGER_PATH],
        )?;
        Ok(n)
    }

    /// Cancel a recurring task by label (e.g. when reflection is disabled in identity.toml).
    pub fn cancel_recurring_task_by_label(&self, agent_id: &str, label: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?1 AND label = ?2 AND trigger_type = 'recurring'
               AND status NOT IN ('completed','failed','cancelled','expired')",
            params![agent_id, label],
        )?;
        Ok(())
    }

    /// Cancel active recurring tasks for agents no longer on disk (mika#1436).
    ///
    /// Called once at startup after the filesystem walk populates the agent set.
    /// Cancels (not deletes) so the audit trail via audit_events is preserved.
    /// Returns the list of (task_id, agent_id) pairs for operator observability.
    ///
    /// Agent-unscoped: operates across all agents in the DB, not filtered by
    /// this `Database` instance's implicit agent context.
    pub fn cancel_orphan_recurring_tasks(
        &self,
        known_agent_ids: &[String],
    ) -> Result<Vec<(String, String)>> {
        if known_agent_ids.is_empty() {
            return Ok(vec![]);
        }

        let tx = self.conn.unchecked_transaction()?;

        // Build a parameterized placeholder list for the NOT IN clause.
        let placeholders: Vec<String> = (1..=known_agent_ids.len())
            .map(|i| format!("?{}", i))
            .collect();
        let placeholders_str = placeholders.join(", ");

        // SELECT orphan tasks first for logging.
        let select_sql = format!(
            "SELECT id, agent_id FROM tasks
             WHERE trigger_type = 'recurring'
               AND status IN ('pending', 'recurring_active', 'in_progress')
               AND agent_id NOT IN ({placeholders_str})"
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = known_agent_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let orphans: Vec<(String, String)> = {
            let mut stmt = tx.prepare(&select_sql)?;
            stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };

        if orphans.is_empty() {
            tx.commit()?;
            return Ok(vec![]);
        }

        // UPDATE to cancelled.
        let update_sql = format!(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE trigger_type = 'recurring'
               AND status IN ('pending', 'recurring_active', 'in_progress')
               AND agent_id NOT IN ({placeholders_str})"
        );
        tx.execute(&update_sql, params.as_slice())?;

        tx.commit()?;
        Ok(orphans)
    }

    pub fn get_task(&self, id: &str, agent_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND agent_id = ?2",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![id, agent_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Resolve a task ID prefix to matching full task IDs, scoped to the given agent.
    /// Returns up to 10 matching IDs for ambiguity reporting.
    pub fn resolve_task_id_by_prefix(&self, prefix: &str, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks WHERE id LIKE ?1 || '%' AND agent_id = ?2 ORDER BY id LIMIT 10",
        )?;
        let ids: Vec<String> = stmt
            .query_map(params![prefix, agent_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Get a manual (task) task by ID, scoped to the given agent.
    pub fn get_manual_task(&self, id: &str, agent_id: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![id, agent_id], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Walk the `parent_task_id` chain to the nearest scope root — a task with
    /// `type IN ('issue', 'milestone', 'project')`. Returns `None` if no scope
    /// ancestor exists, the starting task is not found, or the chain exceeds the
    /// depth limit (mika#974).
    ///
    /// The depth limit of 20 gives 6× headroom above the deployed maximum of
    /// N=3 (project → milestone → issue) while bounding worst-case walk cost.
    pub fn resolve_scope_root_task_id(&self, task_id: &str) -> Result<Option<String>> {
        /// Maximum parent-chain hops before giving up. Deployed task hierarchies
        /// never exceed N=3 today (project → milestone → issue). 20 gives 6×
        /// headroom against pathological chains.
        const SCOPE_ROOT_WALK_DEPTH_LIMIT: usize = 20;
        const SCOPE_TYPES: &[&str] = &[TASK_TYPE_ISSUE, TASK_TYPE_MILESTONE, TASK_TYPE_PROJECT];

        let mut current_id = task_id.to_owned();

        for _ in 0..SCOPE_ROOT_WALK_DEPTH_LIMIT {
            let row: Option<(String, String, Option<String>)> = self
                .conn
                .query_row(
                    "SELECT type, trigger_type, parent_task_id FROM tasks WHERE id = ?1",
                    params![current_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;

            match row {
                // A manual task with a scope type is the scope root.
                // Callback/recurring tasks are not scope roots even if typed as 'issue'.
                Some((task_type, trigger_type, _))
                    if SCOPE_TYPES.contains(&task_type.as_str()) && trigger_type == "manual" =>
                {
                    return Ok(Some(current_id));
                }
                Some((_, _, Some(parent_id))) => {
                    current_id = parent_id;
                }
                // Task not a scope root and no parent — chain exhausted.
                Some((_, _, None)) | None => return Ok(None),
            }
        }

        // Depth limit exceeded — likely a circular chain.
        warn!(
            task_id = task_id,
            limit = SCOPE_ROOT_WALK_DEPTH_LIMIT,
            "scope_root_walk_depth_limit_exceeded"
        );
        Ok(None)
    }

    /// Get IDs of pending callback tasks for a given session.
    ///
    /// Used by `mika ask` to detect background tasks that were spawned during the
    /// agent loop but won't be consumed until TUI or server starts. See #265.
    pub fn get_pending_callbacks_for_session(&self, session_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM tasks
             WHERE created_by_session = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
             ORDER BY created_at ASC",
        )?;
        let ids = stmt
            .query_map(params![session_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Count child tasks for a given parent task (manual tasks only).
    pub fn count_child_tasks(
        &self,
        parent_task_id: &str,
        agent_id: &str,
    ) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT status, COUNT(*) FROM tasks
             WHERE parent_task_id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'
             GROUP BY status",
        )?;
        let rows = stmt.query_map(params![parent_task_id, agent_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_schedulable_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','recurring_active')
               AND trigger_type NOT IN ('callback', 'manual')
             ORDER BY next_fire_at ASC NULLS LAST",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn update_task_status(&self, id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// Record the trace_id of the execution that ran this task.
    /// Deliberately does NOT scope by agent_id — the dispatcher may write
    /// execution_trace_id for tasks owned by different agents (cross-agent team tasks).
    pub fn update_task_execution_trace_id(&self, id: &str, trace_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET execution_trace_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![trace_id, id],
        )?;
        Ok(())
    }

    /// Atomically claim a task and record its fired_at in a single UPDATE.
    /// Returns true if the task was claimed (was in 'pending' or 'recurring_active' state).
    /// Returns false if the task was already claimed, cancelled, or completed.
    pub fn claim_and_fire_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'in_progress',
                              fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                              updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2 AND status IN ('pending', 'recurring_active')",
            params![id, agent_id],
        )?;
        Ok(n > 0)
    }

    pub fn update_task_completed(
        &self,
        id: &str,
        agent_id: &str,
        result: Option<&str>,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'completed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3 AND status IN ('pending', 'in_progress')",
            params![result, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_failed(&self, id: &str, agent_id: &str, error: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'failed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
             AND status NOT IN ('completed', 'failed', 'cancelled', 'expired', 'delivered')",
            params![error, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Update the dispatch class of a task (#1001).
    ///
    /// Used when a task transitions between grooming and implementation phases
    /// (e.g., after dev-groom completes and dev-pilot is about to dispatch on
    /// the same task_id per mika#996's task-reuse pattern). Idempotent —
    /// setting the same class is a no-op (updated_at still advances).
    pub fn update_task_dispatch_class(
        &self,
        id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET dispatch_class = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3",
            params![dispatch_class, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Write a dispatch-rejection reason to `tasks.result` without changing status (#1108).
    ///
    /// Used by `validate_dispatch_readiness()` to surface rejection reasons to
    /// operator-visible surfaces (`tasks.result` column). The task's status is
    /// preserved — only `result` and `updated_at` are modified. Returns `true`
    /// if the row was updated. Agent-unscoped because the caller may not know
    /// the agent_id (e.g., the unauthorized-webhook check fires before task fetch).
    pub fn write_task_dispatch_rejection(&self, id: &str, reason_json: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET result = ?1,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND trigger_type = 'manual'",
            params![reason_json, id],
        )?;
        Ok(rows > 0)
    }

    /// Promote a task from `failed` → `completed` (#958).
    ///
    /// Symmetric to `update_task_failed()`. Only transitions tasks currently
    /// in `failed` status — guarded WHERE clause prevents promotion from any
    /// other state. Returns `true` if the transition happened.
    pub fn promote_task_completed(&self, id: &str, agent_id: &str, reason: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'completed', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
             AND status = 'failed'",
            params![reason, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    pub fn update_task_next_fire_at(&self, id: &str, next_fire_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    /// Atomically reschedule a recurring task: set next_fire_at and status = 'recurring_active'
    /// in a single UPDATE, replacing two sequential writes.
    pub fn update_task_rescheduled(&self, id: &str, next_fire_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET next_fire_at = ?1, status = 'recurring_active', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
            params![next_fire_at, id],
        )?;
        Ok(())
    }

    pub fn cancel_task(&self, id: &str, agent_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2 AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![id, agent_id],
        )?;
        if n > 0 {
            // Cascade: cancel active callback children (mika#1011 Phase 0.7).
            // Prevents orphan-pending deferred-dispatch callbacks (and closes a
            // latent gap for immediate callbacks too). Non-callback children
            // (e.g., manual sub-tasks) are intentionally left untouched.
            self.conn.execute(
                "UPDATE tasks SET status = 'cancelled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE parent_task_id = ?1 AND agent_id = ?2
                   AND trigger_type = 'callback'
                   AND status IN ('pending', 'in_progress')",
                params![id, agent_id],
            )?;
        }
        Ok(n > 0)
    }

    /// Update the status of a manual (task) task. Free transitions allowed.
    /// Sets `completed_at` when transitioning to `completed`.
    /// Returns the old status for audit logging.
    pub fn update_manual_task_status(
        &self,
        task_id: &str,
        agent_id: &str,
        new_status: &str,
    ) -> Result<Option<String>> {
        let old_status: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()?;

        let Some(old) = &old_status else {
            return Ok(None);
        };

        if old == new_status {
            return Ok(Some(old.clone()));
        }

        self.conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                    completed_at = CASE WHEN ?1 = 'completed' THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') ELSE NULL END
             WHERE id = ?2 AND agent_id = ?3 AND trigger_type = 'manual'",
            params![new_status, task_id, agent_id],
        )?;

        Ok(old_status)
    }

    /// Transition a **parent tracking row** to `in_progress` on dispatch **and**
    /// stamp `fired_at` (mika#2335, facette 2).
    ///
    /// SOLE WRITER of the parent-side dispatch transition. The three production
    /// dispatch paths — `skills::executor::execute_long_running` (#525, the
    /// original), `server::ready_label_handler` and `server::verdict_handler`
    /// (both of which say in their own comments that they *mirror* the first) —
    /// call this instead of `update_manual_task_status(…, "in_progress")`.
    ///
    /// **The defect this closes.** `set_task_process_id` stamps `fired_at`
    /// (mika#2263 défaut (b)), and it is the sole writer of `process_id` — but
    /// its only production caller writes the **callback child**. The parent
    /// tracking row, the one every operator surface reads (`mika tasks`, the
    /// dashboard, the health probes), went through `update_manual_task_status`,
    /// which writes `status`, `updated_at` and `completed_at` and nothing else.
    /// So a live dispatch's parent read `fired_at = NULL` — "never fired" — for
    /// the whole life of the pilot. On 2026-09-15 an operator read exactly that
    /// and cancelled a pilot that had been running for 38 minutes.
    ///
    /// **Why a dedicated writer and not a `CASE` added to
    /// [`Self::update_manual_task_status`].** That method also serves
    /// `rewind.rs`, which *restores a prior status* and must stamp nothing — a
    /// rewind is not a dispatch. Naming the dispatch transition separates the
    /// two intents at the call site instead of relying on a conditional that a
    /// future caller would inherit silently.
    ///
    /// **An existing `fired_at` is never overwritten**, same clause and same
    /// reason as `set_task_process_id`: the reapers measure a dispatch's age
    /// from it, and a re-stamp would reset that age under them.
    ///
    /// Returns the prior status (`None` when no such manual row exists), so
    /// callers keep the non-fatal `warn!`-and-continue shape they already had —
    /// the stamp is observability and must never fail a dispatch.
    pub fn mark_parent_dispatched(&self, task_id: &str, agent_id: &str) -> Result<Option<String>> {
        let old_status: Option<String> = self
            .conn
            .query_row(
                "SELECT status FROM tasks WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()?;

        if old_status.is_none() {
            return Ok(None);
        }

        // Unlike `update_manual_task_status`, an unchanged status is NOT an
        // early return: a row already `in_progress` whose dispatch is only now
        // firing still needs its `fired_at`. Skipping the write there would
        // reproduce the defect on every path that transitions before it
        // dispatches.
        //
        // **The `status IN ('pending','in_progress')` guard is load-bearing,
        // and it is the one place this method is STRICTER than the free
        // transition it replaces.** Two of the three call sites observe the
        // row's status and then do real work before stamping —
        // `ready_label_handler` creates the callback child and stats the
        // handler script in between, `verdict_handler` likewise — and a
        // concurrent supersession for the same `reference_url` is a designed-
        // for event on exactly that path (`supersede_prior_tracking_rows` runs
        // at the top of the same handler). Without the guard, a dispatch could
        // resurrect a parent another dispatch had just cancelled, putting two
        // live dispatches back on one ticket through the bookkeeping instead of
        // through the missing kill. A refusal writes nothing and is silent by
        // design: the callers already treat this whole call as non-fatal, and
        // the prior status is returned either way.
        // Le fragment d'estampillage est interpolé depuis
        // [`FIRED_AT_STAMP_IF_NULL`] (mika#2133 AC4) : l'acte a une seule
        // définition textuelle, et la garde de source refuse qu'un écrivain en
        // écrive une seconde.
        self.conn.execute(
            &format!(
                "UPDATE tasks
                SET status = 'in_progress',
                    {FIRED_AT_STAMP_IF_NULL},
                    completed_at = NULL,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?1 AND agent_id = ?2 AND trigger_type = 'manual'
                AND status IN ('pending', 'in_progress')"
            ),
            params![task_id, agent_id],
        )?;

        Ok(old_status)
    }

    /// Estampille `fired_at` **sans toucher au statut** (mika#2133 R2).
    ///
    /// # La population qu'il sert
    ///
    /// Une ligne `callback` a deux phases. Celle qui porte un pilote est
    /// estampillée au spawn par [`Self::set_task_process_id`] (mika#2263). Celle
    /// qui n'en porte pas — un wrapper différé, un rendez-vous de build — ne
    /// traverse jamais ce chemin, et le seul instant où le moteur travaille
    /// sous elle est son **tour de livraison** : un tour LLM qui prend le verrou
    /// d'agent pour jusqu'à l'enveloppe complète. Avant ce ticket, rien ne le
    /// marquait, donc la ligne se lisait « jamais firée » pendant tout ce temps.
    ///
    /// # Pourquoi il ne touche pas au statut
    ///
    /// C'est ce qui le distingue de [`Self::mark_parent_dispatched`] et ce qui
    /// le met hors du périmètre de la décision D6 : poser `in_progress` sur une
    /// ligne callback ferait entrer toute la population des pilotes vivants dans
    /// [`Self::get_active_callback_tasks_with_pid`], la requête du watchdog
    /// #959 — **qui marque la tâche `failed`** quand le processus meurt. Ce
    /// périmètre a été borné par écrit par mika#2272 ; l'élargir est un
    /// changement de comportement moteur, pas une observabilité, et c'est un
    /// ticket distinct.
    ///
    /// L'estampille est NULL-only (voir [`FIRED_AT_STAMP_IF_NULL`]) : sur une
    /// ligne qui porte déjà un pilote, la livraison arrive après le spawn et ne
    /// réécrit rien — c'est ce qui rend vraie la définition unique de D7,
    /// *« le premier instant où le moteur a commencé à travailler sous cette
    /// ligne »*, appliquée à deux natures de travail.
    ///
    /// Scopé par `agent_id` comme tout écrivain de cette table. Une ligne
    /// absente n'est pas une erreur : l'appelant traite ce stamp en
    /// `warn!`-et-continue, l'observabilité ne doit jamais faire échouer un tour.
    pub fn stamp_task_fired_at_if_null(&self, id: &str, agent_id: &str) -> Result<()> {
        self.conn.execute(
            &format!(
                "UPDATE tasks
                SET {FIRED_AT_STAMP_IF_NULL},
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?1 AND agent_id = ?2"
            ),
            params![id, agent_id],
        )?;
        Ok(())
    }

    /// Number of columns in TASK_COLUMNS (used for child_count ordinal in list_manual_tasks).
    /// Bumped to 32 in mika#1948 for `dispatcher_source`.
    const TASK_COLUMN_COUNT: usize = 32;

    /// List manual (task) tasks for an agent with optional filters.
    /// Uses parameterized NULL checks to avoid dynamic SQL construction.
    pub fn list_manual_tasks(
        &self,
        agent_id: &str,
        status_filter: Option<&str>,
        source_filter: Option<&str>,
        include_children: bool,
    ) -> Result<Vec<(Task, Option<i64>)>> {
        let child_expr = if include_children {
            "(SELECT COUNT(*) FROM tasks c WHERE c.parent_task_id = t.id)"
        } else {
            "NULL"
        };

        let sql = format!(
            "SELECT {columns}, {child_expr} AS child_count
             FROM tasks t
             WHERE t.agent_id = ?1 AND t.trigger_type = 'manual'
               AND (?2 IS NULL OR t.status = ?2)
               AND (?3 IS NULL OR t.source = ?3)
             ORDER BY t.created_at DESC LIMIT 50",
            columns = Self::TASK_COLUMNS
                .split(", ")
                .map(|c| format!("t.{c}"))
                .collect::<Vec<_>>()
                .join(", "),
        );

        let status_param: Option<String> = status_filter.map(|s| s.to_string());
        let source_param: Option<String> = source_filter.map(|s| s.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, status_param, source_param], |r| {
                let task = Self::row_to_task(r)?;
                // child_count is at ordinal Self::TASK_COLUMN_COUNT (one past last task column)
                let child_count: Option<i64> = r.get(Self::TASK_COLUMN_COUNT)?;
                Ok((task, child_count))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Count **active** agent-created tasks in a session (for per-session cap enforcement).
    /// Only pending/in_progress/blocked items count — completed/cancelled/failed/delivered
    /// items are terminal and should not block new task creation (sprint mode).
    /// Scoped to agent_id for defense-in-depth.
    pub fn count_session_tasks(&self, agent_id: &str, session_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1 AND created_by_session = ?2 AND trigger_type = 'manual'
               AND (source IS NULL OR source != 'user_request')
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')",
            params![agent_id, session_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Find an active manual task by agent_id and reference_url.
    /// Used for dedup when `create_task` is called with a reference_url that
    /// already has an active (non-terminal) task.
    pub fn find_active_task_by_ref_url(
        &self,
        agent_id: &str,
        reference_url: &str,
    ) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND reference_url = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, reference_url], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and PR URL stored in metadata.
    /// Looks up `json_extract(metadata, '$.claude_pilot.pr_url')` for matching.
    /// Used to locate the parent task when a PR review verdict arrives.
    pub fn find_active_task_by_pr_url(&self, agent_id: &str, pr_url: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND json_extract(metadata, '$.claude_pilot.pr_url') = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, pr_url], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and branch stored in metadata.
    /// Looks up `json_extract(metadata, '$.claude_pilot.branch')` for matching.
    /// Used to locate the parent task when a PR webhook arrives before
    /// the PR URL has been recorded (in-flight tasks only have a branch).
    pub fn find_active_task_by_branch(&self, agent_id: &str, branch: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND json_extract(metadata, '$.claude_pilot.branch') = ?2
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, branch], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find an active manual task by agent_id and label (case-insensitive).
    /// Used as a fallback dedup path when `create_task` is called without a reference_url.
    pub fn find_active_task_by_label(&self, agent_id: &str, label: &str) -> Result<Option<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
               AND trigger_type = 'manual'
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             LIMIT 1",
            Self::TASK_COLUMNS
        );
        self.conn
            .query_row(&sql, params![agent_id, label], Self::row_to_task)
            .optional()
            .map_err(Into::into)
    }

    /// Find active phantom-shape tracking rows for an issue's base URL AND its
    /// `?phase=groom` variant (mika#1934 AC2.2 / AC4.b).
    ///
    /// Returns rows in the phantom shape — `trigger_type='manual'`,
    /// `action_type='none'`, `process_id IS NULL`, `status IN ('blocked',
    /// 'in_progress')` — whose `reference_url` is either the exact `base_url` or
    /// `<base_url>?phase=groom`. `base_url` MUST be canonical (no `?phase=groom`
    /// suffix); callers strip it via
    /// [`crate::task_state::tasks::strip_groom_phase_suffix`].
    ///
    /// A single helper serves both cleanup surfaces: supersede-on-new-dispatch
    /// (AC2) and complete-on-upstream-close (AC4) both need the same
    /// exact-URL + groom-variant fan-out. Reuses the partial unique index
    /// `idx_tasks_manual_active_ref_url` (`blocked`/`in_progress` are inside the
    /// index's "active" set) — no new migration (AC2.1).
    pub fn find_active_tracking_rows_by_reference_url_and_variants(
        &self,
        agent_id: &str,
        base_url: &str,
    ) -> Result<Vec<Task>> {
        let groom_url = format!("{base_url}{}", crate::task_state::tasks::GROOM_PHASE_SUFFIX);
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'manual'
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('blocked', 'in_progress')
               AND reference_url IN (?2, ?3)
             ORDER BY id",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, base_url, groom_url], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    // mika#2335 — `find_live_dispatch_rows_by_reference_url_and_variants` lived
    // here and is deleted, not left in place. Its WHERE clause was
    // `process_id IS NOT NULL AND reference_url IN (?2, ?3)`, a conjunction
    // **empty on the topology production writes**: the `reference_url` is on
    // the parent tracking row, which never carries a `process_id`; the pgid is
    // on the callback child, which never carries a `reference_url`. It could
    // not return a row for any normal dispatch — not in the mika#2263 incident,
    // in none. A query that can return nothing and stays in the file will one
    // day be read as a guarantee. The traversal that does work already existed
    // and is tested: [`Self::find_dispatch_children_with_pid`].

    /// Guarded transition of a phantom tracking row → `cancelled` with the
    /// canonical supersede reason (mika#1934 AC2).
    ///
    /// SOLE WRITER of `result = 'superseded_by_new_dispatch'`
    /// ([`crate::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH`]). Only
    /// transitions from the phantom shape (blocked/in_progress,
    /// `action_type='none'`, `process_id IS NULL`), so a caller passing a
    /// genuine dispatched row or a `pending` row gets `false` and no write.
    /// Returns `true` iff a row transitioned.
    pub fn cancel_task_superseded(&self, id: &str, agent_id: &str) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = 'cancelled', result = ?1,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2 AND agent_id = ?3
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('blocked', 'in_progress')",
            params![
                crate::task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH,
                id,
                agent_id
            ],
        )?;
        Ok(rows > 0)
    }

    /// Guarded terminal transition of a phantom tracking row on upstream close
    /// (mika#1934 AC4).
    ///
    /// `new_status` is `'cancelled'` or `'completed'`; `result` is one of the
    /// upstream-close reason constants
    /// ([`crate::task_state::tasks::ISSUE_CLOSED_UPSTREAM`],
    /// [`crate::task_state::tasks::UPSTREAM_PR_MERGED`],
    /// [`crate::task_state::tasks::UPSTREAM_PR_CLOSED_UNMERGED`]). Same
    /// phantom-shape guard as [`Self::cancel_task_superseded`]. Idempotent: an
    /// already-terminal row matches nothing (the `status IN (...)` guard
    /// short-circuits on re-delivery) and returns `false` (AC4.c). Returns
    /// `true` iff transitioned.
    pub fn terminal_mark_tracking_row_upstream_closed(
        &self,
        id: &str,
        agent_id: &str,
        new_status: &str,
        result: &str,
    ) -> Result<bool> {
        let rows = self.conn.execute(
            "UPDATE tasks SET status = ?1, result = ?2,
             completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3 AND agent_id = ?4
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('blocked', 'in_progress')",
            params![new_status, result, id, agent_id],
        )?;
        Ok(rows > 0)
    }

    /// Get the depth of a task by ID (for computing child depth).
    /// Scoped to agent_id to prevent cross-agent parent linking.
    pub fn get_task_depth(&self, task_id: &str, agent_id: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT depth FROM tasks WHERE id = ?1 AND agent_id = ?2",
                params![task_id, agent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// List pending/in_progress/blocked manual tasks for prompt injection (heartbeat awareness).
    pub fn list_active_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND trigger_type = 'manual'
               AND status IN ('pending', 'in_progress', 'blocked')
             ORDER BY created_at DESC LIMIT 10",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Build a task health summary for heartbeat prompt injection.
    ///
    /// Returns active manual tasks plus anomalous task states across all trigger types.
    /// Anomalies are capped at [`crate::task_engine::types::health_thresholds::MAX_ANOMALIES`].
    pub fn get_task_health_summary(&self, agent_id: &str) -> Result<TaskHealthSummary> {
        use crate::task_engine::types::health_thresholds;

        let active_tasks = self.list_active_tasks(agent_id)?;
        let now = Utc::now();
        let mut anomalies: Vec<TaskHealthAnomaly> = Vec::new();

        // Helper: query anomaly rows and return them as TaskHealthAnomaly values.
        // `describe_age` receives the 5th SELECT column (a timestamp or ignored)
        // and returns the human-readable age_description for each anomaly.
        let query_anomalies = |sql: &str,
                               sql_params: &[&dyn rusqlite::types::ToSql],
                               anomaly_type: &str,
                               describe_age: &dyn Fn(&str) -> String|
         -> Result<Vec<TaskHealthAnomaly>> {
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(sql_params, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?;
            let mut result = Vec::new();
            for row in rows {
                let (id, label, trigger_type, status, ts_col, reference_url) = row?;
                result.push(TaskHealthAnomaly {
                    task_id: id,
                    label,
                    trigger_type,
                    status,
                    anomaly_type: anomaly_type.to_string(),
                    age_description: describe_age(&ts_col),
                    reference_url,
                });
            }
            Ok(result)
        };

        // 1. Stuck callbacks: completed but not delivered for > threshold
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STUCK_CALLBACK_SECS)),
            );
            let limit = health_thresholds::MAX_ANOMALIES as i64;
            anomalies.extend(query_anomalies(
                "SELECT id, label, trigger_type, status, updated_at, reference_url
                 FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'completed'
                   AND updated_at < ?2
                 ORDER BY updated_at ASC
                 LIMIT ?3",
                &[&agent_id, &threshold as &dyn rusqlite::types::ToSql, &limit],
                "stuck_callback",
                &|ts| format!("stuck {}", format_age(ts, now)),
            )?);
        }

        // 2. Failed recurring tasks
        {
            let since = timestamp::format(&(now - Duration::seconds(86_400)));
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, updated_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'recurring'
                       AND status = 'failed'
                       AND updated_at > ?2
                     ORDER BY updated_at DESC
                     LIMIT ?3",
                    &[&agent_id, &since as &dyn rusqlite::types::ToSql, &remaining],
                    "failed_recurring",
                    &|ts| format!("failed {} ago", format_age(ts, now)),
                )?);
            }
        }

        // 3. Long-running in_progress tasks
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, fired_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND status = 'in_progress'
                       AND trigger_type != 'manual'
                       AND fired_at IS NOT NULL
                       AND fired_at < ?2
                     ORDER BY fired_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "long_running",
                    &|ts| format!("running for {}", format_age(ts, now)),
                )?);
            }
        }

        // 4. Stale blocked manual tasks
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STALE_BLOCKED_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, updated_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'manual'
                       AND status = 'blocked'
                       AND updated_at < ?2
                     ORDER BY updated_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "stale_blocked",
                    &|ts| format!("blocked for {}", format_age(ts, now)),
                )?);
            }
        }

        // 5. Stale pending manual tasks with no callback child (#583)
        {
            let threshold = timestamp::format(
                &(now - Duration::seconds(health_thresholds::STALE_PENDING_SECS)),
            );
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT t.id, t.label, t.trigger_type, t.status, t.created_at, t.reference_url
                     FROM tasks t
                     WHERE t.agent_id = ?1
                       AND t.trigger_type = 'manual'
                       AND t.status = 'pending'
                       AND t.created_at < ?2
                       AND NOT EXISTS (
                           SELECT 1 FROM tasks c
                           WHERE c.parent_task_id = t.id
                             AND c.trigger_type = 'callback'
                       )
                     ORDER BY t.created_at ASC
                     LIMIT ?3",
                    &[
                        &agent_id,
                        &threshold as &dyn rusqlite::types::ToSql,
                        &remaining,
                    ],
                    "stale_pending",
                    &|ts| format!("pending for {}", format_age(ts, now)),
                )?);
            }
        }

        // 6. GitHub-linked manual tasks (active, with reference_url containing github.com)
        {
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len()) as i64;
            if remaining > 0 {
                anomalies.extend(query_anomalies(
                    "SELECT id, label, trigger_type, status, created_at, reference_url
                     FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'manual'
                       AND status IN ('pending', 'in_progress')
                       AND reference_url LIKE '%github.com%'
                     ORDER BY created_at DESC
                     LIMIT ?2",
                    &[&agent_id, &remaining as &dyn rusqlite::types::ToSql],
                    "github_linked",
                    &|_| "has linked GitHub PR".to_string(),
                )?);
            }
        }

        // 7. Dispatch failures: dual-signal wedge detection (#980)
        //    Signal A: >= THRESHOLD recent run_claude_pilot failures in the sliding window
        //    Signal B: stale dispatch — no run_claude_pilot attempt in > 1h while task is in_progress
        {
            let remaining = health_thresholds::MAX_ANOMALIES.saturating_sub(anomalies.len());
            if remaining > 0 {
                let window_start = timestamp::format(
                    &(now - Duration::seconds(health_thresholds::DISPATCH_FAILURE_WINDOW_SECS)),
                );
                let stale_threshold = timestamp::format(
                    &(now - Duration::seconds(health_thresholds::LONG_RUNNING_DEFAULT_SECS)),
                );

                // Signal A: Count recent failures with session→task JOIN for correlation
                let signal_a: Option<(u32, Option<String>, Option<String>)> = self
                    .conn
                    .prepare(
                        "SELECT
                            COUNT(*) as failure_count,
                            t.id as task_id,
                            t.label as task_label
                         FROM tool_calls tc
                         LEFT JOIN sessions s ON tc.session_id = s.id
                         LEFT JOIN tasks t ON s.task_id = t.id AND t.status = 'in_progress'
                         WHERE tc.agent_id = ?1
                           AND tc.tool_name = 'run_claude_pilot'
                           AND tc.success = 0
                           AND tc.created_at >= ?2
                         GROUP BY t.id
                         ORDER BY failure_count DESC
                         LIMIT 1",
                    )?
                    .query_row(params![agent_id, &window_start], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .ok();

                // Signal B: Stale dispatch — most recent run_claude_pilot attempt is older than 1h
                // while an in_progress manual task exists
                let signal_b: Option<(String, String)> = self
                    .conn
                    .prepare(
                        "SELECT t.id, t.label
                         FROM tasks t
                         WHERE t.agent_id = ?1
                           AND t.status = 'in_progress'
                           AND t.trigger_type = 'manual'
                           AND NOT EXISTS (
                               SELECT 1 FROM tool_calls tc2
                               WHERE tc2.agent_id = ?1
                                 AND tc2.tool_name = 'run_claude_pilot'
                                 AND tc2.created_at >= ?2
                           )
                         ORDER BY t.updated_at DESC
                         LIMIT 1",
                    )?
                    .query_row(params![agent_id, &stale_threshold], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .ok();

                let mut dispatch_anomaly_fired = false;

                // Emit anomaly from Signal A (threshold met)
                if let Some((count, task_id, task_label)) = signal_a
                    && count >= health_thresholds::DISPATCH_FAILURE_THRESHOLD
                {
                    let (tid, tlabel) = match (task_id, task_label) {
                        (Some(id), Some(label)) => (id, label),
                        _ => (
                            agent_id.to_string(),
                            "run_claude_pilot dispatch".to_string(),
                        ),
                    };
                    anomalies.push(TaskHealthAnomaly {
                        task_id: tid,
                        label: tlabel,
                        trigger_type: "manual".to_string(),
                        status: "in_progress".to_string(),
                        anomaly_type: "dispatch_failures".to_string(),
                        age_description: format!("{} failures in last 2h", count),
                        reference_url: None,
                    });
                    dispatch_anomaly_fired = true;
                }

                // Emit anomaly from Signal B (stale dispatch) — only if Signal A didn't fire
                if !dispatch_anomaly_fired && let Some((task_id, task_label)) = signal_b {
                    anomalies.push(TaskHealthAnomaly {
                        task_id,
                        label: task_label,
                        trigger_type: "manual".to_string(),
                        status: "in_progress".to_string(),
                        anomaly_type: "dispatch_stale".to_string(),
                        age_description: "no dispatch attempt in >1h".to_string(),
                        reference_url: None,
                    });
                }
            }
        }

        // Cap total anomalies
        anomalies.truncate(health_thresholds::MAX_ANOMALIES); // safe-byte-slice: Vec<TaskHealthAnomaly> — element count, no char boundary

        Ok(TaskHealthSummary {
            active_tasks,
            anomalies,
        })
    }

    pub fn mark_tasks_expired(&self, now: &str, agent_id: &str) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'expired', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?2
               AND timeout_at IS NOT NULL AND timeout_at < ?1
               AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![now, agent_id],
        )?;
        Ok(n)
    }

    /// Get IDs of expired tasks whose parent is still pending (for sibling completion checks).
    pub fn get_expired_child_task_ids(&self, agent_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id FROM tasks t
             JOIN tasks p ON t.parent_task_id = p.id
             WHERE t.agent_id = ?1
               AND t.status = 'expired'
               AND t.parent_task_id IS NOT NULL
               AND p.status = 'pending'",
        )?;
        let ids = stmt
            .query_map(params![agent_id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    pub fn count_pending_tasks(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1 AND status IN ('pending','in_progress','recurring_active')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Count active self_dev tasks for mika-dev (mika#1363 F2).
    ///
    /// Returns the number of tasks that indicate mika-dev is not idle:
    /// - status='in_progress' (currently running) or status='pending' (awaiting dispatch)
    /// - source='self_dev' (excludes system tasks: heartbeat, reflection, recurring auto_pull)
    /// - Excludes 'completed', 'failed', 'blocked', 'cancelled' (terminal states)
    pub fn count_active_self_dev_tasks(&self, agent_id: &str) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND source = 'self_dev'
               AND status IN ('in_progress', 'pending')",
            params![agent_id],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// True if an active (pending/in_progress) self_dev task references this issue
    /// (mika#1824 D6). Used by the Phase 2 stuck-ready reconciler to skip tickets
    /// that already have in-flight work of their own.
    ///
    /// `issue_url` is the canonical issue URL (e.g.
    /// `https://github.com/senara-solutions/mika/issues/123`). The match is a
    /// prefix `LIKE` so the `?phase=groom` suffix variant is covered.
    pub fn has_active_self_dev_task_for_issue(
        &self,
        agent_id: &str,
        issue_url: &str,
    ) -> Result<bool> {
        let prefix = format!("{}%", issue_url);
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND source = 'self_dev'
               AND status IN ('pending', 'in_progress')
               AND reference_url LIKE ?2",
            params![agent_id, prefix],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get all pending user-visible tasks (reminders and callbacks, excludes heartbeat/reflection).
    /// Returns user-visible reminder tasks (both `send_message` and `resume_agent`).
    ///
    /// Intentionally excludes `trigger_type = 'callback'` tasks: those are system-internal
    /// tasks created by long-running exec handlers, not user-created reminders. Callback
    /// delivery is handled separately by `get_undelivered_callback_tasks()` (server mode)
    /// and `poll_callback_tasks()` (CLI mode). See #363.
    pub fn get_user_visible_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND action_type IN ('send_message', 'resume_agent')
               AND trigger_type NOT IN ('callback')
               AND status IN ('pending', 'in_progress', 'recurring_active')
             ORDER BY next_fire_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Split counts of active background tasks: executing (subprocess alive) vs queued (waiting).
    /// Uses `process_id IS NOT NULL` as the discriminator for executing tasks.
    /// Used by TUI footer badge to show `[1 running, 2 queued]` instead of `[3 running]`.
    pub fn get_background_task_counts(&self, agent_id: &str) -> Result<BackgroundTaskCounts> {
        let (executing, queued): (i64, i64) = self.conn.query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN process_id IS NOT NULL THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN process_id IS NULL THEN 1 ELSE 0 END), 0)
             FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')",
            params![agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(BackgroundTaskCounts {
            executing: executing as usize,
            queued: queued as usize,
        })
    }

    /// Count active background tasks (long-running callback tasks that are pending or in-progress).
    /// Convenience wrapper that sums executing + queued counts.
    pub fn get_active_background_task_count(&self, agent_id: &str) -> Result<usize> {
        let counts = self.get_background_task_counts(agent_id)?;
        Ok(counts.executing + counts.queued)
    }

    pub fn get_inject_context_tasks(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1 AND action_type = 'inject_context'
               AND status IN ('pending', 'in_progress')
             ORDER BY next_fire_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get callback tasks that completed or failed but have not yet been delivered to the user.
    /// Bounded by `since` (ISO 8601) to avoid processing stale callbacks.
    pub fn get_undelivered_callback_tasks(&self, agent_id: &str, since: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get callback tasks that completed or failed but have not yet been delivered,
    /// scoped to a specific session. Used by TUI to avoid cross-session leakage.
    pub fn get_undelivered_callback_tasks_for_session(
        &self,
        agent_id: &str,
        since: &str,
        session_id: &str,
    ) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('completed', 'failed')
               AND completed_at IS NOT NULL
               AND completed_at > ?2
               AND created_by_session = ?3
             ORDER BY completed_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id, since, session_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Atomically mark a completed or failed callback task as delivered.
    /// Returns `false` if the task was already claimed (not in 'completed'/'failed' status).
    pub fn mark_task_delivered(&self, id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET status = 'delivered', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND status IN ('completed', 'failed')",
            params![id],
        )?;
        Ok(n > 0)
    }

    /// Terminal record for a deferred wrapper consumed without dispatching
    /// (mika#2169).
    ///
    /// `expired` is deliberate: it is already in the `status` CHECK constraint,
    /// it is terminal, and it is NOT in the `('completed','failed')` set that
    /// `get_undelivered_callback_tasks` scans — so the wrapper leaves the
    /// delivery queue instead of re-entering it. Marking it `failed` would
    /// livelock the delivery scan.
    ///
    /// Three fins de vie, trois mots distincts: `delivered` = the turn
    /// dispatched; `expired` + reason = the turn happened and dispatched
    /// nothing; `completed` with no successor = the promotion never fired —
    /// which is exactly the exclusivity `count_promoted_undelivered_wrappers`
    /// relies on.
    ///
    /// The guard on `label` and on the departure status makes the write
    /// idempotent and impossible to aim at anything but a deferred wrapper.
    pub fn mark_deferred_wrapper_noop(&self, id: &str, reason: &str) -> Result<bool> {
        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'expired',
                 result = ?2,
                 completed_at = ?3,
                 updated_at = ?3
             WHERE id = ?1
               AND label = ?4
               AND status IN ('completed', 'delivered')",
            params![id, reason, now, crate::agent::DEFERRED_DISPATCH_LABEL],
        )?;
        Ok(n > 0)
    }

    /// Count deferred wrappers promoted (`completed`) but never taken by the
    /// engine, older than `stale_seconds` and born at or after `epoch`
    /// (mika#2169, L2b).
    ///
    /// After L1 and L2a, `status = 'completed'` on this label is **exclusively**
    /// "promoted, not yet taken": delivery writes `delivered`, and a sterile
    /// consumption writes `expired`. That exclusivity is what makes the count
    /// readable — without it the number mixes promotion starvation with the very
    /// defect this ticket repairs.
    ///
    /// `epoch` is the instant L1 first ran in production (`schema_meta` key
    /// `deferred_promotion_epoch`). Rows older than it predate the exclusivity
    /// and would fire the indicator permanently, for a reason unrelated to
    /// starvation. Nothing mutates them — the bound only narrows the count.
    pub fn count_promoted_undelivered_wrappers(
        &self,
        agent_id: &str,
        stale_seconds: i64,
        epoch: &str,
    ) -> Result<(i64, i64)> {
        let stale_modifier = format!("-{stale_seconds} seconds");
        let row = self.conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(
                      MAX(CAST(strftime('%s','now') - strftime('%s', completed_at) AS INTEGER)),
                      0)
             FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND label = ?2
               AND status = 'completed'
               AND completed_at IS NOT NULL
               AND completed_at >= ?4
               AND completed_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?3)",
            params![
                agent_id,
                crate::agent::DEFERRED_DISPATCH_LABEL,
                stale_modifier,
                epoch
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(row)
    }

    /// Find implement-class parent self_dev tasks left `in_progress` whose
    /// callback subtask delivered without producing a PR (#871).
    ///
    /// A parent is "orphaned" when:
    /// - `status = 'in_progress'`, `source = 'self_dev'`, `trigger_type = 'manual'`
    /// - Its latest callback subtask is `status = 'delivered'`
    /// - **The callback's** `dispatch_class = 'implement'` (or NULL — pre-v34 via COALESCE).
    ///   The class is keyed off the child (per-dispatch) rather than the parent
    ///   because reused parents (mika#920 pattern) carry stale class data from
    ///   their original dispatch.
    /// - The callback's `updated_at` is older than `grace_seconds` ago
    /// - Parent metadata does NOT contain `$.claude_pilot.pr_url`
    /// - No other active callback child exists (defers to #870's retry loop)
    ///
    /// Groom-class callbacks (mika#1001) are NOT reaped here — their expected
    /// artifact is a plan commit pushed to the branch, not a PR url. Groom-class
    /// leak detection is a separate follow-up (mika#1118 Option B).
    ///
    /// **Coupled pair:** `find_completable_parent_tasks_on_pr_url` is the
    /// success-side sibling (mika#1162). Any filter change here (agent_id,
    /// status, source, trigger_type, dispatch_class, sibling guard, grace
    /// window) MUST be applied symmetrically there. The two queries differ
    /// only on the `pr_url` predicate (`IS NULL` here vs `IS NOT NULL` there).
    pub fn find_orphaned_parent_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<OrphanedParentTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MIN(child.id) AS callback_task_id
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND COALESCE(child.dispatch_class, 'implement') = 'implement'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND child.status = 'delivered'
               AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND (parent.metadata IS NULL
                    OR json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NULL)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.id != child.id
                   AND sibling.status IN ('pending', 'in_progress')
               )
             GROUP BY parent.id
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(OrphanedParentTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    callback_task_id: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find phantom tracking rows: `action_type='none'`, `process_id IS NULL`,
    /// `status IN ('in_progress','blocked')`, `updated_at` older than
    /// `age_seconds` ago. These are the NULL-PID tracking-task orphans the
    /// callback watchdog cannot see (its very first predicate is
    /// `process_id IS NOT NULL`) and the orphaned-parent reaper does not
    /// select (no `resume_agent` callback child). See mika#1712.
    ///
    /// **Scope narrower than the ticket's Problem statement (ADV-2, 2026-08-21).**
    /// Plan §3 (Problem) lists `status IN ('in_progress','blocked','pending')` as
    /// the observed leak class, and one of the two cited leaked samples
    /// (`613a996d ... | pending`) is a `pending` phantom. The mika#1712 AC
    /// narrows to `('in_progress','blocked')` only — `pending` is deferred to
    /// the mika#1934 cause-racine investigation. Rationale: `pending` is the
    /// default initial state of every tracking task from `create_task`, and
    /// sweeping it at the default grace would false-positive on any newly-created
    /// tracking row the agent hasn't yet transitioned. `in_progress` and
    /// `blocked` are the "started but abandoned" states where the phantom
    /// signal is unambiguous. If mika#1934's telemetry shows `pending` phantoms
    /// remain a meaningful leak class, the predicate can be widened there.
    ///
    /// `age_seconds = 0` matches every candidate row regardless of freshness.
    /// The comparison uses `<=` (not `<`) so a row inserted in the same
    /// second the WHERE clause evaluates is still selected — SQLite's
    /// `strftime('%Y-%m-%dT%H:%M:%SZ', ...)` truncates to seconds, and a `<`
    /// would silently miss same-second injections. `<=` is safe for the AC3
    /// path too: the default grace has 1-second slack to spare, and a
    /// row updated exactly at "now - grace" being caught 1s early is
    /// behaviorally identical to being caught 1s later.
    ///
    /// SOLE READER — this query is the sole source of candidates for the
    /// `phantom_aged_out` audit-event transition emitted by
    /// [`super::task_engine::engine::TaskEngine::sweep_null_pid_phantoms`]
    /// (AC3 watchdog) and the equivalent step inside
    /// [`super::task_engine::engine::TaskEngine::startup_recovery`] (AC5).
    ///
    /// Candidates, not verdicts (mika#2156). Both callers run each row past a
    /// liveness guard — `TaskEngine::live_dispatch_child`, backed by
    /// [`Self::find_dispatch_children_with_pid`] — before writing the failure,
    /// because a PID's liveness is not expressible in SQL. Age alone cannot
    /// tell an orphaned tracking row from one whose dispatch is still running:
    /// the row's `updated_at` is never bumped while the work proceeds.
    pub fn find_phantom_tracking_tasks(
        &self,
        agent_id: &str,
        age_seconds: i64,
    ) -> Result<Vec<PhantomTrackingTask>> {
        let age_modifier = format!("-{age_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, label, status, created_at, updated_at
             FROM tasks
             WHERE agent_id = ?1
               AND action_type = 'none'
               AND process_id IS NULL
               AND status IN ('in_progress', 'blocked')
               AND updated_at <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, age_modifier], |row| {
                Ok(PhantomTrackingTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    label: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Read `metadata.process_start_time` out of the value the two dispatch-child
    /// queries below select for it.
    ///
    /// The executor writes it as a JSON **string** (`skills/executor.rs`), but an
    /// integer is accepted too so a hand-written or future-shaped `metadata` row
    /// is not silently treated as missing. Anything else — NULL, malformed, a
    /// negative integer — degrades to `None`.
    ///
    /// **`None` never means "dead" and never means "alive"; it means the pair
    /// that identifies a process *instance* is incomplete.** Each caller decides
    /// what to do about that, and they deliberately decide differently: the
    /// phantom sweep sweeps (mika#2156 D-3), the supersession declines to signal
    /// (mika#2335), the live-pilot predicate answers `Unreadable` (mika#2279).
    ///
    /// Shared rather than written twice: the two queries below carry the same
    /// rule, and a rule written twice is a rule that can disagree with itself —
    /// which is the sentence [`is_terminal_task_status`] already had to have
    /// engraved on it one file over.
    fn parse_process_start_time(value: rusqlite::types::Value) -> Option<u64> {
        match value {
            rusqlite::types::Value::Text(t) => t.parse::<u64>().ok(),
            rusqlite::types::Value::Integer(i) => u64::try_from(i).ok(),
            _ => None,
        }
    }

    /// The dispatch children of a tracking row that carry a `process_id`.
    ///
    /// Companion to [`Self::find_phantom_tracking_tasks`] (mika#2156), placed
    /// next to it so a reader meets the guard where they meet the query it
    /// guards. The sweep query cannot tell "orphaned tracking row" from
    /// "tracking row whose dispatch is still running": its three non-temporal
    /// criteria (`action_type='none'`, `process_id IS NULL`,
    /// `status IN ('in_progress','blocked')`) are satisfied *by construction*
    /// for every healthy tracking row, because the real process lives on a
    /// separate `long_running:*` recall row. That recall row already points
    /// back via `parent_task_id` — the missing link is read here, not added.
    ///
    /// Deliberately does NOT filter on the child's status. Per plan mika#2156
    /// D-2: 1146 of the 1147 PID-carrying children measured in production are
    /// `delivered`, and nothing in the code proves `delivered` implies a dead
    /// process. Treating the child's status as the discriminator would disarm
    /// the sweeper on 98% of its historical population (measure M2). Liveness
    /// is the discriminator; the caller applies it.
    ///
    /// `process_start_time` is extracted from `metadata` JSON, where the
    /// executor writes it as a *string* (see `skills/executor.rs`). Rows
    /// without it come back as `None` — the caller sweeps them (D-3).
    pub fn find_dispatch_children_with_pid(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<DispatchChild>> {
        let mut stmt = self.conn.prepare(
            // `json_valid` guard is load-bearing, not belt-and-braces:
            // SQLite's `json_extract` raises a hard "malformed JSON" error
            // (not NULL) on a non-JSON `metadata`, and that error propagates
            // out of the whole `query_map`. Without the guard, ONE bad sibling
            // row would fail the lookup for the entire parent — and the caller
            // would then have no answer about a parent whose OTHER child may
            // be alive. Wrapped, a malformed row degrades to NULL and only
            // that child stops being able to spare (plan mika#2156 D-3).
            "SELECT id,
                    process_id,
                    CASE WHEN json_valid(metadata)
                         THEN json_extract(metadata, '$.process_start_time')
                    END,
                    status
             FROM tasks
             WHERE parent_task_id = ?1
               AND process_id IS NOT NULL
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![parent_task_id], |row| {
                Ok(DispatchChild {
                    id: row.get(0)?,
                    process_id: row.get(1)?,
                    process_start_time: Self::parse_process_start_time(row.get(2)?),
                    status: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The dispatch children reachable from an **issue URL**, whatever the
    /// status of the tracking row that carries it (mika#2279).
    ///
    /// Sibling of [`Self::find_dispatch_children_with_pid`], which starts from a
    /// known parent id. This one starts from the URL, because the caller —
    /// "is a pilot alive for this ticket?" — holds an issue, not a task.
    ///
    /// **The absent predicate is the fix.** There is deliberately no condition
    /// on `parent.status`. `has_active_self_dev_task_for_issue` conjoins
    /// `reference_url LIKE …` with `status IN ('pending','in_progress')` on one
    /// row, and that conjunction goes false the instant a supersession cancels
    /// the parent — while the pilot on the child keeps running. That false
    /// answer is what let the mika#2279 loop re-drive a ticket every 20 minutes
    /// with its pilot working: a cancelled parent is not the absence of a
    /// dispatch, it is exactly the state where the question needs asking.
    ///
    /// `issue_url` is matched as a prefix `LIKE` so the `?phase=groom` variant
    /// is covered — same rule, same reason, as
    /// [`Self::has_active_self_dev_task_for_issue`].
    ///
    /// Two things this query does **not** decide, both left to the caller:
    /// liveness (only `/proc` can answer that) and the child's terminal status
    /// (`is_terminal_task_status` owns that vocabulary; spelling it here would
    /// be a third copy in a dialect that cannot express its "unknown is not
    /// terminal" rule).
    ///
    /// **One predicate it carries that the sibling does not**, named here so the
    /// asymmetry is not read later as an accident:
    /// `child.trigger_type = 'callback'`. The sibling narrows by its
    /// `parent_task_id` argument, which already scopes it to one tracking row's
    /// offspring; this one starts from a URL and so must say which of a parent's
    /// children is a dispatch. It is redundant *today* — `set_task_process_id`
    /// has exactly one production call site and it writes a callback child
    /// (`skills/executor.rs`) — and it is kept because the day something else
    /// records a `process_id`, a URL-keyed query that did not say
    /// "callback" would start answering about a process that is not a pilot.
    ///
    /// The `json_valid` guard is carried over verbatim from the sibling, and is
    /// load-bearing for the same reason: `json_extract` raises a hard error on a
    /// non-JSON `metadata`, and that error propagates out of the whole
    /// `query_map` — so one malformed sibling row would make the answer for the
    /// entire ticket "no pilot", which is the fail-open direction *and* the
    /// wrong one here.
    pub fn find_dispatch_children_for_issue_url(
        &self,
        agent_id: &str,
        issue_url: &str,
    ) -> Result<Vec<IssueDispatchChild>> {
        let prefix = format!("{}%", issue_url);
        let mut stmt = self.conn.prepare(
            "SELECT child.id,
                    child.process_id,
                    CASE WHEN json_valid(child.metadata)
                         THEN json_extract(child.metadata, '$.process_start_time')
                    END,
                    child.status,
                    parent.id
             FROM tasks child
             JOIN tasks parent ON child.parent_task_id = parent.id
             WHERE parent.agent_id = ?1
               AND parent.reference_url LIKE ?2
               AND child.trigger_type = 'callback'
               AND child.process_id IS NOT NULL
             ORDER BY child.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, prefix], |row| {
                Ok(IssueDispatchChild {
                    parent_task_id: row.get(4)?,
                    child: DispatchChild {
                        id: row.get(0)?,
                        process_id: row.get(1)?,
                        process_start_time: Self::parse_process_start_time(row.get(2)?),
                        status: row.get(3)?,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Test-only helper: backdate a task's `updated_at` by `seconds_ago`
    /// seconds relative to now. Used by mika#1712 integration tests to inject
    /// phantom-shape rows aged past the sweep grace window without waiting on
    /// wall-clock time. Not intended for production use — timestamps are
    /// otherwise engine-owned.
    #[doc(hidden)]
    pub fn backdate_task_updated_at(&self, task_id: &str, seconds_ago: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             WHERE id = ?1",
            params![task_id, format!("-{seconds_ago} seconds")],
        )?;
        Ok(())
    }

    /// Test-only helper: backdate a task's `completed_at` by `seconds_ago`
    /// seconds relative to now. Sibling of [`Self::backdate_task_updated_at`],
    /// and needed for the same reason a different column needs it:
    /// `update_task_completed` writes `completed_at = now`, so a mika#2179 test
    /// cannot otherwise seed the incident's `2026-09-03T22:03:24Z` completion
    /// and measure the 5 h 06 wait that followed. Deliberately leaves
    /// `updated_at` alone — the delivery latency under test is
    /// `completed_at → delivery`, and moving both would hide a regression in
    /// which column the measurement reads. Not intended for production use —
    /// timestamps are otherwise engine-owned.
    #[doc(hidden)]
    pub fn backdate_task_completed_at(&self, task_id: &str, seconds_ago: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             WHERE id = ?1",
            params![task_id, format!("-{seconds_ago} seconds")],
        )?;
        Ok(())
    }

    /// Test-only helper: rewrite a task's primary key.
    ///
    /// `find_dispatch_children_with_pid` orders by `id`, and `create_task`
    /// generates a UUIDv4 — so a mika#2156 test that needs a *specific*
    /// examination order (a dead child examined before a live one) cannot get
    /// it from insertion order. Forcing the ids is what makes that test
    /// deterministic, and therefore what makes its injection check mean
    /// something. Safe only for a leaf row: nothing here rewrites the
    /// `parent_task_id` of any grandchild. Not for production use.
    #[doc(hidden)]
    pub fn set_task_id_for_test(&self, old_id: &str, new_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET id = ?2 WHERE id = ?1",
            params![old_id, new_id],
        )?;
        Ok(())
    }

    /// Find parent self_dev tasks left `in_progress` after their callback
    /// subtask delivered WITH a `pr_url` (success indicator). Sibling to
    /// `find_orphaned_parent_tasks` — same JOIN shape, same guards, but
    /// inverted on the `pr_url` predicate. Used by the success-side engine
    /// backstop (mika#1162).
    ///
    /// A parent is `completable` when:
    /// - Parent is `status='in_progress'`, `source='self_dev'`, `trigger_type='manual'`
    /// - Its latest callback subtask is `status='delivered'`
    /// - **The callback's** `dispatch_class='implement'` (or NULL — pre-v34 via COALESCE)
    /// - The callback's `updated_at` is older than `grace_seconds` ago
    /// - Parent metadata HAS a non-empty `$.claude_pilot.pr_url`
    /// - No other active callback child exists (mirrors the reaper's guard)
    ///
    /// Groom-class callbacks (mika#1001) cannot trip this path because they
    /// never emit `PR:` lines — the `dispatch_class` filter is defense-in-depth.
    ///
    /// SOLE WRITER warning: this method is the only DB query that selects
    /// candidates for the `parent_completed_from_callback` audit transition.
    pub fn find_completable_parent_tasks_on_pr_url(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<CompletableParentTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MIN(child.id) AS callback_task_id,
                    json_extract(parent.metadata, '$.claude_pilot.pr_url') AS pr_url
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND COALESCE(child.dispatch_class, 'implement') = 'implement'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND child.status = 'delivered'
               AND child.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND json_extract(parent.metadata, '$.claude_pilot.pr_url') IS NOT NULL
               AND json_extract(parent.metadata, '$.claude_pilot.pr_url') != ''
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.id != child.id
                   AND sibling.status IN ('pending', 'in_progress')
               )
             GROUP BY parent.id
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(CompletableParentTask {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    callback_task_id: row.get(3)?,
                    pr_url: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find `manual` tracking rows whose dispatch is over: **every** callback
    /// child has reached a terminal status and the last of them stopped moving
    /// longer ago than `grace_seconds` (mika#2405).
    ///
    /// # Why this is NOT a third member of the self_dev pair
    ///
    /// It sits next to [`Self::find_orphaned_parent_tasks`] and
    /// [`Self::find_completable_parent_tasks_on_pr_url`] because it must be
    /// re-read with them, and it is deliberately **not** one of them. That pair
    /// is cut for the self_dev contract: its terminal discriminator is the
    /// presence of `$.claude_pilot.pr_url` (absent ⇒ `failed`, present ⇒
    /// `completed`), and a QA review never produces a `pr_url`. Widening the
    /// pair's `source` filter — the one-term fix that suggests itself — would
    /// send the whole non-self_dev population down the `failed` branch. Hence,
    /// here: no `pr_url` predicate, a single verdict (`completed`), and a
    /// `source` term that is the pair's **complement** rather than an overlap.
    ///
    /// The symmetry invariant stated on that pair therefore does not extend to
    /// this query: it names a symmetry *between the two of them*, not a licence
    /// to widen their population.
    ///
    /// # The four load-bearing terms
    ///
    /// - `COALESCE(parent.source, '') != 'self_dev'` — complement, never
    ///   overlap. The `COALESCE` is required, not defensive: `source` is NULL
    ///   on the target population (`create_task` writes no `source` outside the
    ///   self-dev paths) and `NULL != 'self_dev'` evaluates to NULL in SQL, so
    ///   without it the query returns **nothing** — the silent failure this
    ///   whole fix exists to close.
    /// - `NOT EXISTS (… sibling.status IN ('pending','in_progress','completed','blocked'))`
    ///   — **`completed` is in that list on purpose.** On a callback row
    ///   `completed` means "the pilot returned, delivery has not happened yet";
    ///   `delivered` is the terminal state. Settling a parent whose child is
    ///   merely `completed` would close the row before its verdict turn ran.
    ///   Same vocabulary as the mika#2179 quarantine.
    ///   The guard does not exclude the joined child (no `sibling.id !=
    ///   child.id`, unlike the two neighbours): the status list holds no
    ///   terminal state, so a terminal `child` cannot count itself, and a
    ///   non-terminal one *must* exclude its parent. One term instead of two.
    /// - Grace on `MAX(child.updated_at)` — time since the **last** child
    ///   moved, never since the parent was created: a parent reused across
    ///   dispatches (mika#920) is old by construction. It lives in `HAVING`
    ///   rather than `WHERE` because it is a predicate on an aggregate and
    ///   SQLite refuses an aggregate function in `WHERE`. The two neighbours
    ///   put their grace in `WHERE` because theirs is row-wise on a single
    ///   `child.updated_at`; this is the only shape difference and it is
    ///   intentional.
    /// - No predicate on the parent's `action_type`, `type`, or
    ///   `reference_url`. Those are precisely the three terms by which the
    ///   existing reapers exclude this population, and `action_type` in
    ///   particular is what makes the phantom sweep (mika#1712) a partial net:
    ///   a `manual` row carrying a real `action_type` has no reaper at all.
    ///
    /// # Fail-safe
    ///
    /// A parent with **no** callback child is out of the population — the
    /// `JOIN` excludes it. That is deliberate: a dispatch refused before the
    /// child was created (`dispatch_limit_exceeded`, `skills/executor.rs`)
    /// leaves a childless row that never received a dispatch, and this query
    /// has nothing to say about it. That residue stays with the phantom sweep
    /// for as long as it carries `action_type='none'`.
    ///
    /// SOLE WRITER context: candidates selected here are transitioned to
    /// `completed` — never `failed` — by
    /// `TaskEngine::settle_dispatch_parents`, through the guarded
    /// [`Self::update_task_completed`].
    pub fn find_settleable_dispatch_parents(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<SettleableDispatchParent>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at,
                    MAX(child.updated_at) AS last_child_at,
                    COUNT(child.id) AS child_count
             FROM tasks parent
             JOIN tasks child ON parent.id = child.parent_task_id
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.trigger_type = 'manual'
               AND COALESCE(parent.source, '') != 'self_dev'
               AND child.trigger_type = 'callback'
               AND child.action_type = 'resume_agent'
               AND NOT EXISTS (
                 SELECT 1 FROM tasks sibling
                 WHERE sibling.parent_task_id = parent.id
                   AND sibling.status IN ('pending', 'in_progress', 'completed', 'blocked')
               )
             GROUP BY parent.id
             HAVING MAX(child.updated_at) < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(SettleableDispatchParent {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    last_child_at: row.get(3)?,
                    child_count: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find parent self_dev **issue** tasks left `in_progress` with **zero**
    /// callback children, aged past `grace_seconds` (mika#1687).
    ///
    /// This is the zero-child complement of `find_orphaned_parent_tasks` and
    /// `find_completable_parent_tasks_on_pr_url`: both of those INNER-JOIN a
    /// delivered callback child, so a parent that reached `in_progress` without
    /// ever recording a callback child (silent pilot death — hypothesis 3 of
    /// mika#1687) produces zero rows in either and is never reaped. The
    /// `NOT EXISTS (SELECT 1 FROM tasks child …)` predicate here is the exact
    /// complement of their JOIN, so the three selection sets are disjoint by
    /// construction (they require a child; this requires none).
    ///
    /// Staleness keys on `parent.updated_at` (there is no delivered-child
    /// `updated_at` to key on). Scoped to `type='issue'` in v1 — milestone and
    /// project parents legitimately sit childless between child dispatches and
    /// carry their own advancement backstops (mika#991, #1218); their
    /// childless-stuck detection is a deferred follow-up.
    ///
    /// SOLE WRITER context: candidates selected here are transitioned to
    /// `failed` with the distinct `stuck_in_progress_no_callback_child` reason
    /// by `TaskEngine::reap_childless_stuck_parent_tasks`.
    pub fn find_childless_stuck_parent_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<ChildlessStuckParent>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id, parent.agent_id, parent.created_at, parent.updated_at
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'in_progress'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks child
                 WHERE child.parent_task_id = parent.id
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(params![agent_id, grace_modifier], |row| {
                Ok(ChildlessStuckParent {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// True when `parent_task_id` still has a **live** deferred-dispatch wrapper
    /// representing it (mika#2045, widened by mika#2181).
    ///
    /// This is the task-scoped answer to the question `mika tasks
    /// promote-deferred` answers per dispatch class. The class-scoped answer
    /// cannot distinguish "this task is queued" from "some other task is
    /// queued"; `register_deferred_callback` sets `parent_task_id` on the
    /// wrapper, so the task-scoped predicate is exact.
    ///
    /// It asks the same question as clause (1) of
    /// [`Self::find_orphaned_pending_issue_tasks`] — *is this parent
    /// represented?* — and must therefore answer it with the same predicate.
    /// The old name said `pending`, which described the implementation rather
    /// than the question, and the narrow predicate it named was the mika#2181
    /// defect. It has no production caller today; that is not a stable property,
    /// and two functions named as equivalents that disagree are a trap armed for
    /// the next caller. Renamed so the compiler catches every site.
    pub fn has_live_deferred_wrapper_child(
        &self,
        agent_id: &str,
        parent_task_id: &str,
        promoted_liveness_seconds: i64,
    ) -> Result<bool> {
        Ok(self
            .find_live_deferred_wrapper_child(
                agent_id,
                parent_task_id,
                promoted_liveness_seconds,
                None,
            )?
            .is_some())
    }

    /// Same question as [`Self::has_live_deferred_wrapper_child`], answered with
    /// the identity of the wrapper rather than a boolean, and with one wrapper
    /// optionally taken out of the population (mika#2413).
    ///
    /// **One SQL, two entry points.** The boolean sibling delegates here with
    /// `exclude_task_id = None`, so the two cannot drift — which is the trap the
    /// sibling's own doc-comment names, and `find_orphaned_pending_issue_tasks`
    /// clause (1) is pinned against the sibling by a dedicated twin test.
    ///
    /// **Why an exclusion at all.** The mika#2413 caller is
    /// `rearm_deferred_callback`, which asks *"is this parent represented by
    /// anything OTHER than the wrapper whose sterility I am treating?"*. On the
    /// `silent_turn_error` path the consumed wrapper is still `completed` with a
    /// fresh `completed_at`, so without the exclusion it would count itself as
    /// live and no re-arm would ever be possible again. On the `noop_completion`
    /// path it is already `delivered` (terminal, never live) — but only when
    /// `mark_task_delivered` landed, and that write can fail. The exclusion makes
    /// the predicate independent of that ordering instead of relying on it.
    ///
    /// **Why the id and not a bool.** The caller emits
    /// `deferred_rearm_skipped_parent_represented` naming the wrapper that is
    /// holding the parent; a skipped re-arm with no way to see *what* holds the
    /// parent reads exactly like a re-arm that never had a reason to happen.
    ///
    /// The oldest live wrapper wins, matching the FIFO order promotion uses, so
    /// the id an operator reads is the one that will actually fire next.
    pub fn find_live_deferred_wrapper_child(
        &self,
        agent_id: &str,
        parent_task_id: &str,
        promoted_liveness_seconds: i64,
        exclude_task_id: Option<&str>,
    ) -> Result<Option<String>> {
        let liveness_modifier = format!("-{promoted_liveness_seconds} seconds");
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND parent_task_id = ?2
                   AND trigger_type = 'callback'
                   AND label = ?3
                   AND (?5 IS NULL OR id != ?5)
                   AND (
                     status = 'pending'
                     OR (status = 'completed'
                         AND completed_at IS NOT NULL
                         AND completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4))
                   )
                 ORDER BY created_at, id
                 LIMIT 1",
                params![
                    agent_id,
                    parent_task_id,
                    crate::agent::DEFERRED_DISPATCH_LABEL,
                    liveness_modifier,
                    exclude_task_id
                ],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    /// Find `pending` self_dev issue parents older than `grace_seconds` that no
    /// callback child represents any more (mika#2045).
    ///
    /// Two `NOT EXISTS` clauses, and both are load-bearing:
    /// 1. no **live** deferred wrapper — `pending` (queued behind a busy
    ///    dispatch slot), or `completed` within `promoted_liveness_seconds`
    ///    (promoted, and the silent turn it feeds has not returned yet);
    /// 2. no active non-deferred callback — the task's real dispatch is not in
    ///    flight.
    ///
    /// Clause (1) counts promoted wrappers on purpose (mika#2181). Promotion
    /// writes `status = 'completed'`; `delivered` only arrives when the silent
    /// turn *returns*, minutes later. Counting `pending` alone made the reaper
    /// call a healthy parent unrepresented a few milliseconds after promotion —
    /// the three engine steps share one 60s pass, in the order promote ->
    /// dispatch -> reap — and burn `MAX_STUCK_REARMS` in two ticks. The founding
    /// trace: promoted 15:31:03Z, expired 15:33:03Z, turn answered 15:34:43Z.
    ///
    /// The window is bounded because `completed`-without-`delivered` is not
    /// guaranteed transient: on the silent-turn error path the wrapper is
    /// re-armed but never marked `delivered`, so it stays `completed` forever.
    /// An unbounded clause would make that corpse a permanent shield against
    /// repair. `completed_at IS NULL` reads as *not* live for the same reason —
    /// a wrapper that cannot prove it is recent does not get to hide its parent.
    /// `completed` is named explicitly rather than `status NOT IN (...)`:
    /// `delivered`, `failed` and `cancelled` are spent wrappers, never live.
    ///
    /// Dropping (2) would classify a parent whose pilot is actually running as
    /// orphaned during the window before the parent reaches `in_progress`.
    ///
    /// Age is measured on `created_at`, not `updated_at`: a re-armed parent
    /// regains a `pending` wrapper, so clause (1) already protects it and the
    /// grace window has no reason to restart.
    ///
    /// `rearm_count` reads `metadata.stuck_rearm_count` through a `json_valid`
    /// guard so unreadable metadata yields 0 instead of raising.
    pub fn find_orphaned_pending_issue_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
        promoted_liveness_seconds: i64,
    ) -> Result<Vec<OrphanedPendingTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let liveness_modifier = format!("-{promoted_liveness_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id,
                    parent.reference_url,
                    parent.created_at,
                    CAST(strftime('%s', 'now') - strftime('%s', parent.created_at) AS INTEGER),
                    COALESCE(
                      CASE WHEN json_valid(parent.metadata)
                           THEN CAST(json_extract(parent.metadata, '$.stuck_rearm_count') AS INTEGER)
                      END, 0),
                    COALESCE(parent.dispatch_class, 'implement')
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'pending'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.reference_url IS NOT NULL
               AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks w
                 WHERE w.parent_task_id = parent.id
                   AND w.agent_id = ?1
                   AND w.trigger_type = 'callback'
                   AND w.label = ?3
                   AND (
                     w.status = 'pending'
                     OR (w.status = 'completed'
                         AND w.completed_at IS NOT NULL
                         AND w.completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4))
                   )
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tasks c
                 WHERE c.parent_task_id = parent.id
                   AND c.trigger_type = 'callback'
                   AND c.status IN ('pending', 'in_progress')
                   AND c.label != ?3
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    grace_modifier,
                    crate::agent::DEFERRED_DISPATCH_LABEL,
                    liveness_modifier
                ],
                |row| {
                    Ok(OrphanedPendingTask {
                        id: row.get(0)?,
                        reference_url: row.get(1)?,
                        created_at: row.get(2)?,
                        age_seconds: row.get(3)?,
                        rearm_count: row.get(4)?,
                        dispatch_class: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Find `blocked` self_dev issue parents refused on a busy dispatch slot,
    /// older than `grace_seconds` and with nothing left representing them
    /// (mika#2169, L3b).
    ///
    /// **Discriminated, not widened.** The population is separated from the
    /// deliberate operator gates by `json_extract(result, '$.error') =
    /// 'global_dispatch_active'` — the only value the slot-refusal path writes.
    /// A `blocked` written by an auto-merge refusal
    /// (`server/verdict_handler.rs`) or a QA escalation does not carry it, so
    /// this sweep can never re-drive a dispatch an operator deliberately
    /// stopped. Measured negative control: task `662d9752` carries
    /// `unauthorized_webhook_dispatch` and is excluded.
    ///
    /// The two `NOT EXISTS` clauses are taken verbatim from
    /// `find_orphaned_pending_issue_tasks` and carry the same meaning: no
    /// `pending` wrapper still queued, and no live real dispatch.
    pub fn find_stale_blocked_dispatch_tasks(
        &self,
        agent_id: &str,
        grace_seconds: i64,
    ) -> Result<Vec<StaleBlockedTask>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.id,
                    parent.reference_url,
                    parent.created_at,
                    CAST(strftime('%s', 'now') - strftime('%s', parent.created_at) AS INTEGER),
                    COALESCE(
                      CASE WHEN json_valid(parent.metadata)
                           THEN CAST(json_extract(parent.metadata, '$.stuck_rearm_count') AS INTEGER)
                      END, 0),
                    COALESCE(parent.dispatch_class, 'implement'),
                    json_extract(parent.result, '$.blocking_callback_id')
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'blocked'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.reference_url IS NOT NULL
               AND json_valid(parent.result)
               AND json_extract(parent.result, '$.error') = 'global_dispatch_active'
               AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND NOT EXISTS (
                 SELECT 1 FROM tasks w
                 WHERE w.parent_task_id = parent.id
                   AND w.trigger_type = 'callback'
                   AND w.status = 'pending'
                   AND w.label = ?3
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tasks c
                 WHERE c.parent_task_id = parent.id
                   AND c.trigger_type = 'callback'
                   AND c.status IN ('pending', 'in_progress')
                   AND c.label != ?3
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    grace_modifier,
                    crate::agent::DEFERRED_DISPATCH_LABEL
                ],
                |row| {
                    Ok(StaleBlockedTask {
                        id: row.get(0)?,
                        reference_url: row.get(1)?,
                        created_at: row.get(2)?,
                        age_seconds: row.get(3)?,
                        rearm_count: row.get(4)?,
                        dispatch_class: row.get(5)?,
                        blocking_callback_id: row.get(6)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Read `metadata.stuck_rearm_count` for a task, tolerating absent or
    /// unreadable metadata as 0 (mika#2045).
    ///
    /// This counter records *repairs* only — the deferred wrappers this task
    /// needed because an earlier one was consumed without dispatching. It is
    /// deliberately not the count of all wrappers ever registered for the task:
    /// the ordinary rejection path registers those, and conflating the two
    /// would spend the repair budget on healthy queueing.
    pub fn get_stuck_rearm_count(&self, task_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COALESCE(
                      CASE WHEN json_valid(metadata)
                           THEN CAST(json_extract(metadata, '$.stuck_rearm_count') AS INTEGER)
                      END, 0)
             FROM tasks WHERE id = ?1",
            params![task_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Increment `metadata.stuck_rearm_count`, returning the new value
    /// (mika#2045). Unreadable metadata is replaced by a fresh object rather
    /// than raising, so a corrupt row still repairs and still terminates.
    pub fn increment_stuck_rearm_count(&self, task_id: &str) -> Result<i64> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_set(
                  CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                  '$.stuck_rearm_count',
                  COALESCE(
                    CASE WHEN json_valid(metadata)
                         THEN CAST(json_extract(metadata, '$.stuck_rearm_count') AS INTEGER)
                    END, 0) + 1),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![task_id],
        )?;
        self.get_stuck_rearm_count(task_id)
    }

    /// Reset `metadata.stuck_rearm_count` to 0 on proof that a real dispatch
    /// happened (mika#2413). Returns whether a non-zero counter was cleared.
    ///
    /// **Why a reset exists at all.** `increment_stuck_rearm_count` was the only
    /// writer, so the counter was monotone for the row's whole life — and a
    /// groom parent *becomes* the implementation parent (mika#1614 task reuse,
    /// `update_task_dispatch_class` flips `groom` → `implement` on the same
    /// row). Two contentions suffered while grooming therefore condemned the
    /// implementation before it began. A counter the success never clears ends
    /// up bounding something other than what it measures.
    ///
    /// **Why this is NOT the reset mika#2158 had to remove.** That one fired on
    /// `in_flight_self_dev` — on *having started*, the very action the counter
    /// counted — which made the counter unreachable (31 re-drives reading 1).
    /// This one fires on *having reached a real dispatch*, which is precisely
    /// what the counter does not count: the budget bounds the hypothesis "this
    /// parent's turns never dispatch", and a spawned non-deferred child refutes
    /// that hypothesis outright. Do not move this call to the start of a
    /// dispatch attempt; that is the regression, not the fix.
    ///
    /// The `> 0` guard keeps the nominal dispatch free of a write and makes the
    /// return value mean "there was something to clear", so the caller can log
    /// only the resets that carry information.
    pub fn reset_stuck_rearm_count(&self, task_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE tasks SET
                metadata = json_set(
                  CASE WHEN json_valid(metadata) THEN metadata ELSE '{}' END,
                  '$.stuck_rearm_count', 0),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1
               AND json_valid(metadata)
               AND COALESCE(
                     CAST(json_extract(metadata, '$.stuck_rearm_count') AS INTEGER),
                     0) > 0",
            params![task_id],
        )?;
        Ok(n > 0)
    }

    /// Parents past the grace that the reaper is sheltering **only** because a
    /// promoted wrapper is still inside the liveness window (mika#2181).
    ///
    /// The sheltering happens inside a SQL `NOT EXISTS`, so a spared parent
    /// never reaches Rust: it produces no candidate, no log line, no audit
    /// event, and `mika tasks stuck` does not print it. If the widened predicate
    /// ever disarmed the reaper systematically, its counters would fall to zero
    /// — indistinguishable from a healthy loop. That silence is the failure mode
    /// `docs/solutions/best-practices/a-reaper-that-reads-a-proxy-instead-of-the-process-2026-09-04.md`
    /// names ("faucher, **et le compter**") and fixed for the sibling sweep with
    /// `phantom_sweep_spared`. This is that counter for this reaper.
    ///
    /// It is the exact complement of clause (1): same parent filters, same
    /// grace, but requiring a live *promoted* wrapper rather than the absence of
    /// any live one. A parent held by a plain `pending` wrapper is ordinary
    /// queueing and is not counted — only the new shelter this fix introduced.
    pub fn find_parents_sheltered_by_promoted_wrapper(
        &self,
        agent_id: &str,
        grace_seconds: i64,
        promoted_liveness_seconds: i64,
    ) -> Result<Vec<String>> {
        let grace_modifier = format!("-{grace_seconds} seconds");
        let liveness_modifier = format!("-{promoted_liveness_seconds} seconds");
        let mut stmt = self.conn.prepare(
            "SELECT parent.reference_url
             FROM tasks parent
             WHERE parent.agent_id = ?1
               AND parent.status = 'pending'
               AND parent.source = 'self_dev'
               AND parent.trigger_type = 'manual'
               AND parent.type = 'issue'
               AND parent.reference_url IS NOT NULL
               AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2)
               AND EXISTS (
                 SELECT 1 FROM tasks w
                 WHERE w.parent_task_id = parent.id
                   AND w.agent_id = ?1
                   AND w.trigger_type = 'callback'
                   AND w.label = ?3
                   AND w.status = 'completed'
                   AND w.completed_at IS NOT NULL
                   AND w.completed_at > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?4)
               )
               AND NOT EXISTS (
                 SELECT 1 FROM tasks w2
                 WHERE w2.parent_task_id = parent.id
                   AND w2.agent_id = ?1
                   AND w2.trigger_type = 'callback'
                   AND w2.label = ?3
                   AND w2.status = 'pending'
               )
             ORDER BY parent.id",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    grace_modifier,
                    crate::agent::DEFERRED_DISPATCH_LABEL,
                    liveness_modifier
                ],
                |row| row.get(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every deferred wrapper of a parent with the fields the stuck-pending
    /// reaper's verdict turns on (mika#2181 AC4), oldest first.
    ///
    /// Read once per candidate, before the re-arm decision, and written into the
    /// `details` of both terminal reaper events. Unfiltered by status on
    /// purpose: the point is to show what was there, including the statuses that
    /// did *not* count as live.
    pub fn summarize_deferred_wrappers_of_parent(
        &self,
        agent_id: &str,
        parent_task_id: &str,
    ) -> Result<Vec<DeferredWrapperSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, status, completed_at FROM tasks
             WHERE agent_id = ?1
               AND parent_task_id = ?2
               AND trigger_type = 'callback'
               AND label = ?3
             -- `created_at` has one-second resolution, so it is not a total
             -- order: two wrappers minted in the same second would render in an
             -- arbitrary order in the audit line. `rowid` breaks the tie by
             -- insertion, which is the order the reaper's reader expects.
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt
            .query_map(
                params![
                    agent_id,
                    parent_task_id,
                    crate::agent::DEFERRED_DISPATCH_LABEL
                ],
                |row| {
                    Ok(DeferredWrapperSummary {
                        id: row.get(0)?,
                        status: row.get(1)?,
                        completed_at: row.get(2)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Cancel every deferred wrapper of a parent that is not already terminal
    /// (mika#2045). Called immediately before expiring the parent: a wrapper
    /// that survives the expiry can still be promoted, and would then replay a
    /// dispatch against a dead parent while the `ready` sweep has already
    /// created a live replacement for the same issue — a double dispatch.
    /// Returns the number of wrappers cancelled.
    pub fn cancel_deferred_wrappers_of_parent(
        &self,
        agent_id: &str,
        parent_task_id: &str,
    ) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'cancelled',
                 result = 'parent expired by the stuck-pending reaper (mika#2045)',
                 completed_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE agent_id = ?1
               AND parent_task_id = ?2
               AND label = ?3
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')",
            params![
                agent_id,
                parent_task_id,
                crate::agent::DEFERRED_DISPATCH_LABEL
            ],
        )?;
        Ok(n)
    }

    /// The `action_config` of a parent's most recent deferred wrapper, whatever
    /// its status (mika#2045). The reaper replays this so a repaired dispatch is
    /// byte-identical to the one that was refused. `None` when the parent never
    /// had a wrapper — the reaper then rebuilds the call from the parent's own
    /// columns.
    pub fn latest_deferred_wrapper_action_config(
        &self,
        agent_id: &str,
        parent_task_id: &str,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT action_config FROM tasks
                 WHERE agent_id = ?1
                   AND parent_task_id = ?2
                   AND label = ?3
                 ORDER BY created_at DESC
                 LIMIT 1",
                params![
                    agent_id,
                    parent_task_id,
                    crate::agent::DEFERRED_DISPATCH_LABEL
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Return ALL children of a parent task for the reaper's structured log event
    /// (`task_engine_reaper.evaluated`). Captures a point-in-time snapshot at kill
    /// time so post-incident diagnosis can see what the reaper saw (mika#1126).
    pub fn get_reaper_child_snapshot(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<ReaperChildSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, dispatch_class, status, trigger_type, action_type, updated_at, label
             FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![parent_task_id], |row| {
                Ok(ReaperChildSnapshot {
                    id: row.get(0)?,
                    dispatch_class: row.get(1)?,
                    status: row.get(2)?,
                    trigger_type: row.get(3)?,
                    action_type: row.get(4)?,
                    updated_at: row.get(5)?,
                    label: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Record (or clear) the OS process running under a task.
    ///
    /// **Also stamps `fired_at` when a process is recorded and the row has none
    /// (mika#2263 défaut (b)).** Registering a PID is the moment the engine
    /// learns a pilot is alive under this task — `skills/executor.rs` calls it
    /// immediately after the spawn — so a row that leaves this function with a
    /// `process_id` and no `fired_at` is a live dispatch that reads as *never
    /// dispatched*. That is precisely what `b429a658` (#2252) and `4e867d85`
    /// (#2212) looked like on 2026-09-09 while their bwrap pilots ran for 69
    /// and 45 minutes: `pending`, `fired_at` NULL, invisible to every probe
    /// that uses `fired_at` to tell *not yet dispatched* from *zombie*.
    ///
    /// The stamp lives here, at the single chokepoint every spawn path passes
    /// through, rather than in each caller — one site cannot drift from
    /// another the way a per-caller convention does.
    ///
    /// Two deliberate non-effects, both pinned by tests:
    /// - clearing (`process_id = None`, what a disposal does after a kill)
    ///   stamps nothing — that is not a dispatch;
    /// - an existing `fired_at` is never overwritten, so a re-record cannot
    ///   reset a dispatch's age under the reapers that measure it.
    ///
    /// **mika#2133 AC4 — le stamp vient de [`FIRED_AT_STAMP_IF_NULL`].** La
    /// garde « un effacement ne stampe pas » a changé de couche : elle était le
    /// `?1 IS NOT NULL` du `CASE` SQL, elle est maintenant la branche Rust
    /// ci-dessous. Même sémantique, même aller-retour unique, et l'acte
    /// d'estampiller n'est plus écrit ici — c'est ce qui fait qu'un cinquième
    /// écrivain ne peut pas apparaître en silence.
    pub fn set_task_process_id(&self, id: &str, process_id: Option<i64>) -> Result<()> {
        // Le prédicat est en Rust, jamais dans le `CASE` : le SQL ne porte
        // alors que l'acte partagé, et la seule question propre à ce site —
        // « enregistre-t-on un PID ou l'efface-t-on ? » — se lit au-dessus.
        let sql = if process_id.is_some() {
            format!(
                "UPDATE tasks
                SET process_id = ?1,
                    {FIRED_AT_STAMP_IF_NULL},
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?2"
            )
        } else {
            "UPDATE tasks
                SET process_id = ?1,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
              WHERE id = ?2"
                .to_string()
        };
        self.conn.execute(&sql, params![process_id, id])?;
        Ok(())
    }

    /// Returns (task_id, process_id) pairs for expired tasks that still have a process_id set.
    pub fn get_expired_tasks_with_process_id(&self, agent_id: &str) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, process_id FROM tasks
             WHERE agent_id = ?1 AND status = 'expired' AND process_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Clear the process_id after killing an orphan process to prevent repeated kill attempts.
    pub fn clear_task_process_id(&self, id: &str, agent_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET process_id = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND agent_id = ?2",
            params![id, agent_id],
        )?;
        Ok(())
    }

    /// Get active callback tasks that have a process_id set (#959).
    ///
    /// Returns callback tasks in `in_progress` status with a non-null process_id,
    /// used by the callback watchdog to detect dead subprocesses.
    ///
    /// # Sa population est VIDE par construction, et l'élargir a un prix (mika#2133 D6)
    ///
    /// Une ligne `callback` porte son PID en **`pending`** : l'enfant est créé
    /// sans statut (donc `pending`), le PID y est posé juste après le spawn, et
    /// la transition `in_progress` de #525 s'applique au **parent**. Mesuré sur
    /// la base de production le 2026-09-09, sur chaque ligne ayant jamais porté
    /// un `process_id` : 876 `delivered`, 19 `cancelled`, 1 `failed`, 1
    /// `pending` — et **zéro** `in_progress`. Voir
    /// [`Self::get_live_dispatch_callback_tasks_with_pid`], qui existe pour
    /// cette raison.
    ///
    /// La conséquence visible est un second symptôme, mesuré le 2026-09-01 : un
    /// garde-tableau qui compte `status='in_progress'` affiche **zéro pendant
    /// qu'un pilote travaille**. Poser `in_progress` sur ces lignes y ferait
    /// entrer d'un coup **toute** la population des pilotes vivants — et **ce
    /// watchdog marque la tâche `failed`** quand le processus meurt. mika#2272 a
    /// borné ce périmètre par écrit : *« élargir cette population-là change qui
    /// marque une tâche `failed` quand un processus meurt, ce qui court contre
    /// le moniteur de spawn ; blast radius distinct, ticket distinct. »* Les deux
    /// compteurs de concurrence lisent déjà `IN ('pending','in_progress')` et
    /// seraient insensibles ; celui-ci ne l'est pas, et c'est lui qui tue.
    ///
    /// mika#2133 a donc estampillé `fired_at` sur ce chemin **sans** toucher au
    /// statut ([`Self::stamp_task_fired_at_if_null`]) et laissé cette requête
    /// telle quelle. Le signal de vie faisant foi est par ailleurs déjà rendu à
    /// l'opérateur ailleurs : `mika tasks <id>` affiche une ligne
    /// `Dispatch pilot:` alimentée par `PilotLiveness` (mika#2335), PID vérifié
    /// par `process_start_time` et mtime du log du pilote. **Suivi nommé :** un
    /// état intermédiaire pour la ligne callback en travail, dont le préalable
    /// écrit est de décider qui marque `failed` quand un pilote meurt une fois
    /// cette population non vide.
    pub fn get_active_callback_tasks_with_pid(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status = 'in_progress'
               AND process_id IS NOT NULL",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Callback tasks whose dispatch may still be **alive**, with a
    /// `process_id` set (mika#2272).
    ///
    /// # Why this is not [`Self::get_active_callback_tasks_with_pid`]
    ///
    /// That method filters `status = 'in_progress'`, and on this row that
    /// status **never occurs**. A dispatch's callback child is created by
    /// `build_callback_task` with no status at all, so `create_task` writes
    /// `pending`; the auto-transition to `in_progress` (`executor.rs`, #525)
    /// applies to the **parent** manual task, not to the child. The child
    /// carries the PID (`set_task_process_id`, right after the spawn) and stays
    /// `pending` until the callback lands and moves it to `delivered`.
    ///
    /// Measured on the production database on 2026-09-09, over every row that
    /// has ever carried a `process_id`: 876 `delivered`, 19 `cancelled`, 1
    /// `failed`, 1 `pending` — and **zero** `in_progress`. A reaper reading the
    /// other method is not looking at a small population, it is looking at an
    /// empty one, which is exactly why mika#2261 shipped and never fired.
    ///
    /// So this scans both surfaces on which a live dispatch can sit: `pending`,
    /// which is where it actually sits, and `in_progress`, kept because nothing
    /// guarantees the state machine keeps this shape and a reaper that silently
    /// narrows again is the defect this method exists to close.
    ///
    /// The #959 watchdog deliberately keeps the narrower query: widening the
    /// population of *that* mechanism changes who marks a task `failed` when a
    /// process dies, which races the spawn monitor. Separate blast radius,
    /// separate ticket.
    pub fn get_live_dispatch_callback_tasks_with_pid(&self, agent_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status IN ('pending', 'in_progress')
               AND process_id IS NOT NULL",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![agent_id], Self::row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Set a single field in the task's metadata JSON (#959).
    ///
    /// Uses SQLite's `json_set()` to merge the field into existing metadata,
    /// initializing with `'{}'` if metadata is currently NULL.
    pub fn set_task_metadata_field(&self, task_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_set(COALESCE(metadata, '{}'), '$.' || ?1, ?2),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?3",
            params![key, value, task_id],
        )?;
        Ok(())
    }

    /// Remove a single field from the task's metadata JSON (#959).
    ///
    /// Uses SQLite's `json_remove()` to delete the key. No-op if the key doesn't exist.
    pub fn remove_task_metadata_field(&self, task_id: &str, key: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET
                metadata = json_remove(COALESCE(metadata, '{}'), '$.' || ?1),
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?2",
            params![key, task_id],
        )?;
        Ok(())
    }

    /// Get a single metadata field from a task's metadata JSON as a string.
    pub fn get_task_metadata_field(&self, task_id: &str, key: &str) -> Result<Option<String>> {
        let result: Option<String> = self.conn.query_row(
            "SELECT json_extract(metadata, '$.' || ?1) FROM tasks WHERE id = ?2",
            params![key, task_id],
            |row| row.get(0),
        )?;
        Ok(result)
    }

    /// Check if all siblings of a completed task are done. If so, atomically
    /// claim the parent task for dispatch.
    ///
    /// Returns `Some(parent_id)` when the parent was claimed, `None` otherwise.
    /// Uses a single SQLite transaction to prevent races.
    pub fn try_complete_parent_on_sibling_done(&self, task_id: &str) -> Result<Option<String>> {
        let tx = self.conn.unchecked_transaction()?;

        // 1. Get parent_task_id for this task (no agent_id filter — parent-child
        //    relationships are structural, and team task trees have mixed agent_ids)
        let parent_id: Option<String> = tx
            .query_row(
                "SELECT parent_task_id FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let parent_id = match parent_id {
            Some(id) => id,
            None => return Ok(None),
        };

        // 2. Count incomplete siblings (same parent, not in terminal state).
        // No agent_id filter — siblings in a team task tree have different agent_ids.
        let incomplete: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1
             AND status NOT IN ('completed','failed','cancelled','expired','delivered')",
            params![&parent_id],
            |row| row.get(0),
        )?;

        if incomplete > 0 {
            tx.commit()?;
            return Ok(None);
        }

        // 3. Atomically claim parent task (only if still pending).
        // No agent_id filter — the parent may have a different agent_id than children.
        let changed = tx.execute(
            "UPDATE tasks SET status = 'in_progress', fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1 AND status = 'pending'",
            params![&parent_id],
        )?;

        tx.commit()?;

        if changed > 0 {
            Ok(Some(parent_id))
        } else {
            Ok(None) // already claimed by another thread
        }
    }

    /// Get all child tasks for a given parent task.
    /// No agent_id filter — team task trees have children with different agent_ids.
    pub fn get_child_tasks(&self, parent_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {} FROM tasks
             WHERE parent_task_id = ?1
             ORDER BY created_at ASC",
            Self::TASK_COLUMNS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![parent_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Get all descendant tasks for a given root task (recursive).
    /// Returns all tasks in the subtree below root_task_id, excluding the root itself.
    /// No agent_id filter — team task trees have children with different agent_ids.
    /// Depth guard (depth <= 3) mirrors the CHECK constraint as defense-in-depth.
    pub fn get_task_descendants(&self, root_task_id: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "WITH RECURSIVE descendant_ids(id) AS (
                 SELECT id FROM tasks WHERE parent_task_id = ?1
                 UNION ALL
                 SELECT t.id FROM tasks t
                 JOIN descendant_ids d ON t.parent_task_id = d.id
                 WHERE t.depth <= 3
             )
             SELECT {cols} FROM tasks
             WHERE id IN (SELECT id FROM descendant_ids)
             ORDER BY created_at ASC",
            cols = Self::TASK_COLUMNS,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![root_task_id], Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Check if any active callback tasks exist for a task OTHER than the excluded one,
    /// filtered by dispatch class.
    ///
    /// Returns `Some((parent_task_id, callback_task_id))` if an active callback task exists
    /// whose parent differs from `excluded_parent_id` and whose dispatch class matches.
    /// Used by the per-class dispatch guard to enforce one-slot-per-class (#583, #1001).
    ///
    /// Pre-v34 rows with `dispatch_class IS NULL` are treated as `'implement'` via
    /// `COALESCE` — no application-layer NULL coercion needed (architect NF1).
    ///
    /// mika#1163: Excludes `:deferred` wrappers via `label NOT LIKE '%:deferred'`.
    /// Deferred wrappers are pending markers waiting for promotion, NOT active
    /// dispatches occupying a slot. Without this exclusion, two parents each
    /// holding a pending wrapper deadlock — every dispatch attempt from one
    /// wrapper sees the OTHER as slot-occupied and registers yet another
    /// wrapper. Mirrors the equivalent clause in `has_any_active_callback`
    /// (mika#1070), which the engine-level promotion backstop uses.
    /// Returns `(parent_task_id, callback_id, callback_label)` of the blocking
    /// callback, or `None` if no conflicting dispatch exists. The label enables
    /// callers to derive `blocker_kind` for rejection JSON (#1172 W3).
    pub fn has_active_callback_tasks_excluding(
        &self,
        excluded_parent_id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<BlockingDispatch>> {
        let mut stmt = self.conn.prepare(
            // mika#1948: the blocking row's `dispatcher_source` rides along so a
            // rejection can name WHO holds the slot, not just that it is held.
            // Deliberately NOT wrapped in COALESCE here — the on-disk NULL of a
            // pre-v51 row is a real distinction ("unknown, predates the column")
            // and collapsing it to 'mika_dev' at this depth would manufacture a
            // certainty the row does not carry. Callers that need a default
            // apply it themselves.
            "SELECT t.parent_task_id, t.id, t.label, p.dispatcher_source
             FROM tasks t
             LEFT JOIN tasks p ON p.id = t.parent_task_id
             WHERE t.trigger_type = 'callback'
               AND t.status IN ('pending', 'in_progress')
               AND t.parent_task_id IS NOT NULL
               AND t.parent_task_id != ?1
               AND t.agent_id = ?2
               AND COALESCE(t.dispatch_class, 'implement') = ?3
               AND t.label NOT LIKE '%:deferred'
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![excluded_parent_id, agent_id, dispatch_class])?;
        if let Some(row) = rows.next()? {
            Ok(Some(BlockingDispatch {
                parent_task_id: row.get(0)?,
                callback_task_id: row.get(1)?,
                label: row.get(2)?,
                dispatcher_source: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// How many *distinct* dispatches of this class are active for this agent,
    /// excluding `excluded_parent_id` (mika#2160).
    ///
    /// The companion to [`Database::has_active_callback_tasks_excluding`], with
    /// the identical WHERE clause. It exists because a configurable cap needs a
    /// number and the predicate only ever answered "is there at least one" —
    /// the shape of a cap of exactly 1. KTD5: the existing signature is left
    /// alone, it has callers in production, in tests, and an async twin.
    ///
    /// `COUNT(DISTINCT parent_task_id)`, not `COUNT(*)`: a dispatch is a
    /// parent, and a parent that happens to carry two callback rows of the same
    /// class is still one dispatch. Counting rows would fill the cap with a
    /// single dispatch and re-serialize the class behind the operator's back.
    pub fn count_active_callback_tasks_excluding(
        &self,
        excluded_parent_id: &str,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT t.parent_task_id)
             FROM tasks t
             WHERE t.trigger_type = 'callback'
               AND t.status IN ('pending', 'in_progress')
               AND t.parent_task_id IS NOT NULL
               AND t.parent_task_id != ?1
               AND t.agent_id = ?2
               AND COALESCE(t.dispatch_class, 'implement') = ?3
               AND t.label NOT LIKE '%:deferred'",
            params![excluded_parent_id, agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Record which dispatcher initiated a task (mika#1948).
    ///
    /// A separate write rather than a `NewTask` field on purpose: `NewTask` has
    /// 175 construction sites, and all but a handful are the autonomous loop,
    /// whose correct value is exactly the NULL default. Making every one of them
    /// restate "mika_dev" would be churn that buries the few sites where the
    /// answer is genuinely something else.
    ///
    /// Callers pass the value they mean; leaving it unset keeps the row NULL,
    /// which reads as `mika_dev`. Rejects unknown values rather than storing
    /// them — the CHECK constraint would too, but failing here names the
    /// offending value.
    pub fn set_task_dispatcher_source(
        &self,
        task_id: &str,
        agent_id: &str,
        dispatcher_source: &str,
    ) -> Result<bool> {
        if !matches!(dispatcher_source, "mika_dev" | "mika_manager" | "operator") {
            anyhow::bail!(
                "unknown dispatcher_source '{dispatcher_source}' — \
                 expected one of: mika_dev, mika_manager, operator"
            );
        }
        let n = self.conn.execute(
            "UPDATE tasks SET dispatcher_source = ?3 WHERE id = ?1 AND agent_id = ?2",
            params![task_id, agent_id, dispatcher_source],
        )?;
        Ok(n > 0)
    }

    /// Whether an operator-sourced task is waiting to run in this dispatch class
    /// (mika#1948 AC3).
    ///
    /// Backs the operator-priority rule in `promote_pending_deferred_if_idle`:
    /// when the operator has drafted work in a class, an automatic
    /// deferred-wrapper promotion must not take the slot out from under it.
    /// Scoped to `pending` — an operator task that is already `in_progress`
    /// holds the slot through the ordinary active-callback check instead.
    pub fn has_pending_operator_task_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND status = 'pending'
               AND dispatcher_source = 'operator'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Atomically claim the exec slot for `(agent_id, dispatch_class)`
    /// (mika#1948 — Porte 2).
    ///
    /// # Why a lease exists at all
    ///
    /// `has_active_callback_tasks_excluding` only ever *checked* the slot. It is
    /// a bare SELECT, and on `None` the dispatch path simply proceeds — the row
    /// that makes the slot observably held (the callback task) is written much
    /// later, by the caller. In between, `validate_dispatch_readiness` performs
    /// several GitHub round-trips (issue body, open-PR check, grooming markers),
    /// each with a 10s timeout. So the gap between "the slot looked free" and
    /// "the slot is held" is measured in seconds, and there are four production
    /// entry points into that function.
    ///
    /// Two dispatchers could therefore both read "free" and both proceed. A slot
    /// that two claimants can simultaneously believe they hold is not
    /// arbitration — it is a convention. This makes the claim a FACT: the
    /// PRIMARY KEY on `(agent_id, dispatch_class)` means a second claimant's
    /// INSERT collides rather than races.
    ///
    /// # Semantics
    ///
    /// One statement, inside an IMMEDIATE transaction. The lease is taken when
    /// no row exists, when the existing row has expired, or when the caller is
    /// re-entering with its own `holder_task_id` (a retry refreshes its own
    /// lease instead of deadlocking against itself). Otherwise the current
    /// holder is returned and the caller must not dispatch.
    ///
    /// # On the TTL, and why it is short
    ///
    /// The lease guards one narrow window: from the moment validation completes
    /// to the moment the callback row exists. That is a process spawn — seconds.
    /// The TTL must exceed that, and must stay far below the duration of a real
    /// dispatch (minutes), so that a lease can never outlive the work it
    /// guarded and block a slot that has legitimately freed. Once the callback
    /// row exists, the ordinary active-callback check is the durable holder and
    /// the lease is redundant; letting it lapse is the intended end of life.
    ///
    /// Expiry is what keeps this fail-closed without being loop-breaking: a
    /// dispatcher that dies mid-claim stalls its class for at most one TTL
    /// rather than forever.
    pub fn try_acquire_dispatch_slot(
        &mut self,
        agent_id: &str,
        dispatch_class: &str,
        holder_task_id: &str,
        dispatcher_source: Option<&str>,
        ttl_secs: i64,
        max_slots: i64,
    ) -> Result<SlotClaim> {
        let now = Utc::now();
        let now_s = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let expires_s = (now + Duration::seconds(ttl_secs))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // 1. Idempotent re-claim. Before v52 this was the `OR holder_task_id =
        //    ?3` arm of the upsert; with several slots it has to come first, or
        //    a holder that already owns slot 1 would take a free slot 0 as well
        //    and spend two slots on one dispatch.
        let refreshed = tx.execute(
            "UPDATE dispatch_slot_leases
                SET dispatcher_source = ?4, acquired_at = ?5, expires_at = ?6
              WHERE agent_id = ?1 AND dispatch_class = ?2 AND holder_task_id = ?3",
            params![
                agent_id,
                dispatch_class,
                holder_task_id,
                dispatcher_source,
                now_s,
                expires_s
            ],
        )?;
        if refreshed > 0 {
            tx.commit()?;
            return Ok(SlotClaim::Acquired);
        }

        // 2. Take the first index whose lease is free or expired. `max_slots <=
        //    0` is the explicit disable sentinel (KTD3, the grammar of
        //    `MIKA_AUTO_PULL_MAX_BEHIND` and `MAX_REDRIVES_ENV`): the cap is
        //    lifted, so a fresh index is appended rather than contended for.
        if max_slots <= 0 {
            // Reclaim an expired index before minting a new one. Without this,
            // every claim under the disable sentinel appends a row that nothing
            // ever removes: `release_dispatch_slot` has no production caller,
            // and before v52 the two-column PRIMARY KEY forced each claim to
            // overwrite the single row. `slot_index` is what makes unbounded
            // accumulation representable, so the reclaim has to be explicit.
            // The table is then bounded by the high-water mark of *live*
            // leases, not by the total number of dispatches ever made.
            let reclaimed: Option<i64> = tx
                .query_row(
                    "SELECT MIN(slot_index) FROM dispatch_slot_leases
                      WHERE agent_id = ?1 AND dispatch_class = ?2
                        AND expires_at <= ?3",
                    params![agent_id, dispatch_class, now_s],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();

            if let Some(idx) = reclaimed {
                tx.execute(
                    "UPDATE dispatch_slot_leases
                        SET holder_task_id = ?4, dispatcher_source = ?5,
                            acquired_at = ?6, expires_at = ?7
                      WHERE agent_id = ?1 AND dispatch_class = ?2 AND slot_index = ?3",
                    params![
                        agent_id,
                        dispatch_class,
                        idx,
                        holder_task_id,
                        dispatcher_source,
                        now_s,
                        expires_s
                    ],
                )?;
                tx.commit()?;
                return Ok(SlotClaim::Acquired);
            }

            let next: i64 = tx.query_row(
                "SELECT COALESCE(MAX(slot_index), -1) + 1 FROM dispatch_slot_leases
                  WHERE agent_id = ?1 AND dispatch_class = ?2",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO dispatch_slot_leases
                     (agent_id, dispatch_class, slot_index, holder_task_id,
                      dispatcher_source, acquired_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    agent_id,
                    dispatch_class,
                    next,
                    holder_task_id,
                    dispatcher_source,
                    now_s,
                    expires_s
                ],
            )?;
            tx.commit()?;
            return Ok(SlotClaim::Acquired);
        }

        // A cap lowered while leases are live can leave rows ABOVE the new
        // ceiling. Scanning `0..max_slots` alone would not see them: at cap 2
        // slots 0 and 1 are taken, the operator reverts to 1, slot 0 expires —
        // and a fresh claim would win slot 0 while slot 1 is still live, so two
        // leases exist under a cap of one. Count what is live first; the index
        // scan below then only has to find a free slot, not to police the cap.
        let live: i64 = tx.query_row(
            "SELECT COUNT(*) FROM dispatch_slot_leases
              WHERE agent_id = ?1 AND dispatch_class = ?2 AND expires_at > ?3",
            params![agent_id, dispatch_class, now_s],
            |row| row.get(0),
        )?;
        if live < max_slots {
            for slot_index in 0..max_slots {
                let changed = tx.execute(
                    "INSERT INTO dispatch_slot_leases
                     (agent_id, dispatch_class, slot_index, holder_task_id,
                      dispatcher_source, acquired_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(agent_id, dispatch_class, slot_index) DO UPDATE SET
                     holder_task_id    = excluded.holder_task_id,
                     dispatcher_source = excluded.dispatcher_source,
                     acquired_at       = excluded.acquired_at,
                     expires_at        = excluded.expires_at
                 WHERE dispatch_slot_leases.expires_at <= ?6",
                    params![
                        agent_id,
                        dispatch_class,
                        slot_index,
                        holder_task_id,
                        dispatcher_source,
                        now_s,
                        expires_s
                    ],
                )?;
                if changed > 0 {
                    tx.commit()?;
                    return Ok(SlotClaim::Acquired);
                }
            }
        }

        // Every slot collided with a live lease, or the cap is already met. Read a holder inside the same
        // transaction so the reported blocker is one we actually lost to; the
        // lowest live index is chosen so the answer is deterministic.
        let held = tx
            .query_row(
                "SELECT holder_task_id, dispatcher_source, expires_at
                 FROM dispatch_slot_leases
                 WHERE agent_id = ?1 AND dispatch_class = ?2 AND expires_at > ?3
                 ORDER BY slot_index ASC
                 LIMIT 1",
                params![agent_id, dispatch_class, now_s],
                |row| {
                    Ok(SlotClaim::Held {
                        holder_task_id: row.get(0)?,
                        dispatcher_source: row.get(1)?,
                        expires_at: row.get(2)?,
                    })
                },
            )
            .optional()?;
        tx.commit()?;

        // A row that vanished between the failed INSERT and this SELECT cannot
        // happen inside one IMMEDIATE transaction, but the query is fallible in
        // principle. Report contention rather than inventing a free slot —
        // fail-closed is the whole point of the lease.
        Ok(held.unwrap_or(SlotClaim::Held {
            holder_task_id: "<unknown>".to_string(),
            dispatcher_source: None,
            expires_at: expires_s,
        }))
    }

    /// Release a slot lease, but only if `holder_task_id` still owns it
    /// (mika#1948).
    ///
    /// The holder predicate matters: without it, a late release from a
    /// dispatcher whose lease already expired and was re-taken by someone else
    /// would free a slot it no longer owns. Returns whether a row was removed.
    pub fn release_dispatch_slot(
        &self,
        agent_id: &str,
        dispatch_class: &str,
        holder_task_id: &str,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM dispatch_slot_leases
             WHERE agent_id = ?1 AND dispatch_class = ?2 AND holder_task_id = ?3",
            params![agent_id, dispatch_class, holder_task_id],
        )?;
        Ok(n > 0)
    }

    /// A live holder of a slot lease for this class, if any (mika#1948).
    /// Expired leases read as free — they are reclaimable by definition.
    ///
    /// Since mika#2160 a class may hold several live leases, so this returns
    /// the **lowest-indexed** one. The ordering is explicit rather than left to
    /// SQLite's scan order: before v52 the PRIMARY KEY made "the holder"
    /// singular and the question could not arise, and a caller that inherited
    /// that assumption would otherwise get a different answer run to run.
    pub fn dispatch_slot_lease_holder(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<(String, Option<String>)>> {
        let now_s = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let row = self
            .conn
            .query_row(
                "SELECT holder_task_id, dispatcher_source
                 FROM dispatch_slot_leases
                 WHERE agent_id = ?1 AND dispatch_class = ?2 AND expires_at > ?3
                 ORDER BY slot_index ASC
                 LIMIT 1",
                params![agent_id, dispatch_class, now_s],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(row)
    }

    /// Claim the worktree of `(repo, issue_number)` (mika#2192).
    ///
    /// # Why a claim exists at all
    ///
    /// `dispatch_slot_leases` says WHO may run. It says nothing about WHERE,
    /// and the where is a shared directory: `derive-worktree-path` is a pure
    /// function of the branch, so two actors on one ticket land on the same
    /// path by construction. An orchestrator session is not a dispatch and
    /// holds no lease, so the lease can never see it — yet it is precisely the
    /// actor of the founding incident.
    ///
    /// # Semantics
    ///
    /// One IMMEDIATE transaction. The claim is taken when no row exists, when
    /// the existing row has expired, or when the caller re-enters with its own
    /// `owner_id` (idempotent refresh — a resumed dispatch must not deadlock
    /// against the claim it took on its first pass). Otherwise the live holder
    /// is returned and the caller must not enter the directory.
    ///
    /// # On liveness
    ///
    /// `expires_at > now`, and nothing else. No `pid`, no `pgrep -f` (vacuous
    /// in this harness), no `/proc` (the agent may be containerised, so the
    /// host's process table is not a surface to build on). The TTL is what
    /// keeps fail-closed from becoming loop-breaking: a claimant that dies
    /// mid-run blocks its own ticket for one TTL, never forever, and the CLI
    /// `mika worktree release` is the manual escape.
    pub fn try_claim_worktree(
        &mut self,
        repo: &str,
        issue_number: i64,
        owner_kind: &str,
        owner_id: &str,
        owner_label: Option<&str>,
        ttl_secs: i64,
    ) -> Result<WorktreeClaimOutcome> {
        let now = Utc::now();
        let now_s = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let expires_s = (now + Duration::seconds(ttl_secs))
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Take it, refresh it, or lose it — in one statement, so a second
        // claimant collides on the PRIMARY KEY instead of racing a SELECT.
        // The `WHERE` is what makes losing possible: without it the upsert
        // would silently steal a live claim, which is the defect, not the fix.
        let changed = tx.execute(
            "INSERT INTO worktree_claims
                 (repo, issue_number, owner_kind, owner_id, owner_label,
                  claimed_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(repo, issue_number) DO UPDATE SET
                 owner_kind  = excluded.owner_kind,
                 owner_id    = excluded.owner_id,
                 owner_label = excluded.owner_label,
                 claimed_at  = excluded.claimed_at,
                 expires_at  = excluded.expires_at
             WHERE worktree_claims.expires_at <= ?6
                OR worktree_claims.owner_id = ?4",
            params![
                repo,
                issue_number,
                owner_kind,
                owner_id,
                owner_label,
                now_s,
                expires_s
            ],
        )?;

        // Read the row back inside the same transaction either way: on success
        // it is the claim we just wrote, on failure it is the holder we
        // actually lost to — never a second read that could see a third state.
        let row = tx
            .query_row(
                "SELECT repo, issue_number, owner_kind, owner_id, owner_label,
                        claimed_at, expires_at
                 FROM worktree_claims
                 WHERE repo = ?1 AND issue_number = ?2",
                params![repo, issue_number],
                |row| {
                    Ok(WorktreeClaim {
                        repo: row.get(0)?,
                        issue_number: row.get(1)?,
                        owner_kind: row.get(2)?,
                        owner_id: row.get(3)?,
                        owner_label: row.get(4)?,
                        claimed_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        tx.commit()?;

        match row {
            Some(claim) if changed > 0 => Ok(WorktreeClaimOutcome::Claimed(claim)),
            Some(claim) => Ok(WorktreeClaimOutcome::HeldByOther(claim)),
            // A row that vanished between the write and the read cannot happen
            // inside one IMMEDIATE transaction. Report contention rather than
            // inventing a free worktree — fail-closed is the whole point.
            None => Ok(WorktreeClaimOutcome::HeldByOther(WorktreeClaim {
                repo: repo.to_string(),
                issue_number,
                owner_kind: "unknown".to_string(),
                owner_id: "<unknown>".to_string(),
                owner_label: None,
                claimed_at: now_s,
                expires_at: expires_s,
            })),
        }
    }

    /// The live owner of `(repo, issue_number)`'s worktree, if any (mika#2192).
    ///
    /// Filters on `expires_at > now`, so `None` **is** the answer "expired or
    /// absent" — a caller never has to re-derive liveness and never has two
    /// ways to get it wrong.
    pub fn worktree_claim_holder(
        &self,
        repo: &str,
        issue_number: i64,
    ) -> Result<Option<WorktreeClaim>> {
        let now_s = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let row = self
            .conn
            .query_row(
                "SELECT repo, issue_number, owner_kind, owner_id, owner_label,
                        claimed_at, expires_at
                 FROM worktree_claims
                 WHERE repo = ?1 AND issue_number = ?2 AND expires_at > ?3",
                params![repo, issue_number, now_s],
                |row| {
                    Ok(WorktreeClaim {
                        repo: row.get(0)?,
                        issue_number: row.get(1)?,
                        owner_kind: row.get(2)?,
                        owner_id: row.get(3)?,
                        owner_label: row.get(4)?,
                        claimed_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Release a worktree claim, but only if `owner_id` still owns it
    /// (mika#2192).
    ///
    /// The owner predicate is the same one `release_dispatch_slot` carries and
    /// for the same reason: a late release from an actor whose claim already
    /// expired and was re-taken by someone else would free a directory it no
    /// longer owns — handing a live writer's worktree to the next dispatch.
    pub fn release_worktree_claim(
        &self,
        repo: &str,
        issue_number: i64,
        owner_id: &str,
    ) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM worktree_claims
             WHERE repo = ?1 AND issue_number = ?2 AND owner_id = ?3",
            params![repo, issue_number, owner_id],
        )?;
        Ok(n > 0)
    }

    /// Drop lapsed claims (mika#2192). Returns how many rows went.
    ///
    /// Purely hygienic: `worktree_claim_holder` and `try_claim_worktree`
    /// already treat an expired row as absent, so nothing depends on this
    /// having run. It keeps the table bounded by live claims rather than by
    /// every ticket ever dispatched.
    pub fn purge_expired_worktree_claims(&self) -> Result<usize> {
        let now_s = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let n = self.conn.execute(
            "DELETE FROM worktree_claims WHERE expires_at <= ?1",
            params![now_s],
        )?;
        Ok(n)
    }

    /// Check whether the given parent task has any active (pending/in_progress)
    /// non-deferred callback child. Used by the R9 no-op wrapper detection (#1172)
    /// to determine if a deferred wrapper completed without spawning a real dispatch.
    pub fn has_non_deferred_active_callback_child(&self, parent_task_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE parent_task_id = ?1
               AND trigger_type = 'callback'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'",
            params![parent_task_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Check whether the autonomous `dev-groom` loop has really groomed a
    /// GitHub issue. Used by the dispatch-classification gate (#1620) so that
    /// grooming markers in an issue body are only trusted when a groom actually
    /// ran and converged — not when they were pre-stamped by hand.
    ///
    /// **Proof = the groom callback row (mika#2287).** The parent dispatch row
    /// is not durable evidence: the structural producers write the bare issue
    /// URL (mika#1572), and the engine flips the parent's `dispatch_class`
    /// groom→implement before it ever reaches a terminal status (mika#1614
    /// task-reuse). The callback child survives both: it keeps
    /// `dispatch_class='groom'` (derived from the `skill` input), reaches
    /// `completed` then `delivered`, and its `result` is the dispatch-lib RESULT
    /// written by `POST /tasks/{id}/complete`, which carries
    /// [`crate::task_state::tasks::GROOM_SUCCESS_MARKER`] on convergence — the
    /// same marker `try_dispatch_pilot_after_groom_success` already trusts.
    ///
    /// One query serves every groom producer: the parent `reference_url` is
    /// accepted in bare form or with the legacy
    /// [`crate::task_state::tasks::GROOM_PHASE_SUFFIX`] the LLM-driven path
    /// appends. Nothing is appended to `issue_url` by the caller.
    ///
    /// Read-only, fail-closed: a single `SELECT`; a DB error propagates as
    /// `Err` and the caller refuses the dispatch. A proof pruned by
    /// `prune_completed_tasks` (30-day retention, either row) is a refusal.
    pub fn has_completed_groom_for_issue(&self, agent_id: &str, issue_url: &str) -> Result<bool> {
        let legacy_groom_url = format!(
            "{}{}",
            issue_url,
            crate::task_state::tasks::GROOM_PHASE_SUFFIX
        );
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks child
             JOIN tasks parent ON child.parent_task_id = parent.id
             WHERE child.agent_id = ?1
               AND child.trigger_type = 'callback'
               AND child.dispatch_class = 'groom'
               AND child.status IN ('completed', 'delivered')
               AND instr(child.result, ?4) > 0
               AND parent.reference_url IN (?2, ?3)",
            params![
                agent_id,
                issue_url,
                legacy_groom_url,
                crate::task_state::tasks::GROOM_SUCCESS_MARKER
            ],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Count pending callback tasks for a given team run with depth > 1.
    /// Used to detect grandchild long-running tasks spawned during a team run.
    pub fn count_pending_callback_tasks_by_team_run(&self, team_run_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE team_run_id = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND depth > 1",
            params![team_run_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count pending deferred-dispatch callback tasks for this agent (mika#1011).
    /// Used by the executor flood-cap check before registering a new deferred callback.
    pub fn count_pending_deferred_callbacks(&self, agent_id: &str) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND status = 'pending'
               AND label = 'long_running:run_claude_pilot:deferred'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Promote the next pending deferred-dispatch callback for dispatch (FIFO).
    ///
    /// Sets `next_fire_at` to now and marks with a synthetic result so the task
    /// engine's periodic scan picks it up and routes through `dispatch_resume_agent`
    /// within one tick (~1 second). Returns `Some(task_id)` if a task was promoted,
    /// `None` if no pending deferred callback existed. Called by the dispatcher
    /// after a blocking callback completes (mika#1011).
    pub fn promote_next_deferred_callback(&self, agent_id: &str) -> Result<Option<String>> {
        // First, find the candidate task ID
        let candidate_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![agent_id],
                |row| row.get(0),
            )
            .optional()?;

        let Some(task_id) = candidate_id else {
            return Ok(None);
        };

        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?2,
                 next_fire_at = ?2,
                 updated_at = ?2
             WHERE id = ?1
               AND status = 'pending'",
            params![task_id, now],
        )?;
        Ok(if n > 0 { Some(task_id) } else { None })
    }

    /// Class-scoped sibling of `promote_next_deferred_callback`. Promotes the
    /// oldest pending deferred wrapper matching the given `dispatch_class`.
    /// Returns `Some(task_id)` if a task was promoted, `None` otherwise. Used by
    /// the periodic backstop's per-class iteration (mika#1175). Pre-v34 NULL
    /// rows treated as 'implement' via COALESCE (matches
    /// `has_active_callback_tasks_excluding` semantics).
    pub fn promote_next_deferred_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        // First, find the candidate task ID
        let candidate_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND status = 'pending'
                   AND label = 'long_running:run_claude_pilot:deferred'
                   AND COALESCE(dispatch_class, 'implement') = ?2
                 ORDER BY created_at ASC
                 LIMIT 1",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )
            .optional()?;

        let Some(task_id) = candidate_id else {
            return Ok(None);
        };

        let now = crate::timestamp::now();
        let n = self.conn.execute(
            "UPDATE tasks
             SET status = 'completed',
                 result = 'deferred dispatch slot freed',
                 completed_at = ?2,
                 next_fire_at = ?2,
                 updated_at = ?2
             WHERE id = ?1
               AND status = 'pending'",
            params![task_id, now],
        )?;
        Ok(if n > 0 { Some(task_id) } else { None })
    }

    /// Returns true if any non-deferred callback task is in pending or in_progress
    /// status (i.e., a dispatch slot is occupied). Was used by the engine-level
    /// deferred-dispatch backstop (mika#1070). Post-mika#1175, the engine
    /// backstop calls `has_any_active_callback_for_class` per-class; this
    /// agent-wide form has no remaining production callers and is retained as
    /// a regression-test baseline + as a sibling reference for the class-scoped
    /// shape. See `has_any_active_callback_for_class` for production usage.
    pub fn has_any_active_callback(&self, agent_id: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'",
            params![agent_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Class-scoped sibling of `has_any_active_callback`. Returns `true` if any
    /// non-deferred callback task in the given `dispatch_class` is `pending` or
    /// `in_progress` (i.e., the per-class dispatch slot is occupied). Used by
    /// the periodic backstop's per-class slot check (mika#1175). Excludes
    /// `:deferred` wrappers (parity with mika#1163's symmetric exclusion).
    /// Pre-v34 NULL rows treated as 'implement' via COALESCE.
    pub fn has_any_active_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// How many *dispatches* of this class occupy the per-class slots
    /// (mika#2160). Counting companion to
    /// [`Database::has_any_active_callback_for_class`], with the identical
    /// WHERE clause.
    ///
    /// It exists because that predicate is a boolean — the shape of a cap of
    /// exactly one — and it gates the deferred-promotion backstop
    /// (`engine.rs::promote_pending_deferred_if_idle`) and the force-promote
    /// override. Leaving them boolean while the dispatch guard learned to count
    /// would make a cap above 1 half-effective: new dispatches would be
    /// admitted, but a *deferred* one would wait for the class to fall back to
    /// **zero** rather than below the cap — the asymmetric-predicate drift
    /// mika#1163 names, rebuilt.
    ///
    /// `COUNT(DISTINCT COALESCE(parent_task_id, id))`: a dispatch is a parent,
    /// so two callback rows under one parent are one occupant. The COALESCE
    /// keeps a parentless row counting as itself, preserving the boolean
    /// predicate's answer for that shape rather than collapsing several such
    /// rows into one.
    pub fn count_active_callbacks_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT COALESCE(parent_task_id, id)) FROM tasks
             WHERE agent_id = ?1
               AND trigger_type = 'callback'
               AND action_type = 'resume_agent'
               AND status IN ('pending', 'in_progress')
               AND label NOT LIKE '%:deferred'
               AND COALESCE(dispatch_class, 'implement') = ?2",
            params![agent_id, dispatch_class],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Force-promote the next pending deferred wrapper for a dispatch class,
    /// with fail-closed slot-availability semantics. Checks
    /// `has_any_active_callback_for_class()` first; if the slot is occupied,
    /// returns `RejectedSlotBusy` without mutating state. If free, promotes
    /// via `promote_next_deferred_callback_for_class()` and returns `Promoted`
    /// or `NoPendingWrapper`. Used by both the CLI verb and agent tool
    /// (mika#1453).
    ///
    /// Paired predicates: the slot check shares the same SQL predicate as
    /// `has_any_active_callback_for_class` — see mika#1163 for the
    /// asymmetric-predicate-drift failure class.
    pub fn force_promote_deferred_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
        max_slots: i64,
    ) -> Result<ForcePromoteResult> {
        // Slot-availability check (fail-closed) — same predicate as the
        // periodic backstop in engine.rs (mika#1163 parity). Since mika#2160
        // that predicate counts and compares against the class cap; `max_slots`
        // arrives as a parameter, the way `ttl_secs` does for the lease, so the
        // layer that owns the setting stays the one that reads the environment.
        let active = self.count_active_callbacks_for_class(agent_id, dispatch_class)?;
        if max_slots > 0 && active >= max_slots {
            // Fetch the blocker's label for the rejection message.
            let blocking_label: String = self
                .conn
                .query_row(
                    "SELECT label FROM tasks
                     WHERE agent_id = ?1
                       AND trigger_type = 'callback'
                       AND action_type = 'resume_agent'
                       AND status IN ('pending', 'in_progress')
                       AND label NOT LIKE '%:deferred'
                       AND COALESCE(dispatch_class, 'implement') = ?2
                     LIMIT 1",
                    params![agent_id, dispatch_class],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "<unknown>".to_string());

            return Ok(ForcePromoteResult::RejectedSlotBusy { blocking_label });
        }

        match self.promote_next_deferred_callback_for_class(agent_id, dispatch_class)? {
            Some(task_id) => Ok(ForcePromoteResult::Promoted { task_id }),
            None => Ok(ForcePromoteResult::NoPendingWrapper),
        }
    }

    /// Returns the task ID of the active non-deferred callback occupying the
    /// per-class dispatch slot. Same SQL predicate as
    /// `has_any_active_callback_for_class` but `SELECT id LIMIT 1` instead of
    /// `SELECT COUNT(*)`. Used by the CLI override path to identify the blocker
    /// before cancellation (mika#1453).
    ///
    /// Paired predicate: shares the `:deferred` exclusion and COALESCE
    /// semantics with `has_any_active_callback_for_class` — keep in sync
    /// (mika#1163).
    pub fn find_active_callback_for_class(
        &self,
        agent_id: &str,
        dispatch_class: &str,
    ) -> Result<Option<String>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM tasks
                 WHERE agent_id = ?1
                   AND trigger_type = 'callback'
                   AND action_type = 'resume_agent'
                   AND status IN ('pending', 'in_progress')
                   AND label NOT LIKE '%:deferred'
                   AND COALESCE(dispatch_class, 'implement') = ?2
                 LIMIT 1",
                params![agent_id, dispatch_class],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id)
    }

    pub fn prune_completed_tasks(&self, older_than_secs: i64) -> Result<usize> {
        let cutoff = timestamp::now_minus(Duration::seconds(older_than_secs));
        let n = self.conn.execute(
            "DELETE FROM tasks WHERE status IN ('completed','cancelled','expired','failed','delivered')
             AND completed_at IS NOT NULL AND completed_at < ?1",
            params![cutoff],
        )?;
        Ok(n)
    }

    pub fn get_tasks_by_status(&self, agent_id: &str, statuses: &[&str]) -> Result<Vec<Task>> {
        self.get_tasks_by_status_and_label(agent_id, statuses, None)
    }

    /// Like `get_tasks_by_status`, but with an optional `label_contains` substring filter.
    /// When `label_contains` is `Some`, only tasks whose label contains the substring
    /// (case-sensitive) are returned. The substring is parameterized (no SQL injection).
    pub fn get_tasks_by_status_and_label(
        &self,
        agent_id: &str,
        statuses: &[&str],
        label_contains: Option<&str>,
    ) -> Result<Vec<Task>> {
        if statuses.is_empty() {
            return Ok(vec![]);
        }
        let placeholders: String = (1..=statuses.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let label_param_idx = statuses.len() + 2; // next param index after agent_id + statuses
        let label_clause = if label_contains.is_some() {
            format!(" AND label LIKE '%' || ?{label_param_idx} || '%'")
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT {} FROM tasks WHERE agent_id = ?1 AND status IN ({}){} ORDER BY created_at DESC",
            Self::TASK_COLUMNS,
            placeholders,
            label_clause,
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(agent_id.to_string())];
        for s in statuses {
            bind.push(Box::new(s.to_string()));
        }
        if let Some(lc) = label_contains {
            bind.push(Box::new(lc.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), Self::row_to_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}
