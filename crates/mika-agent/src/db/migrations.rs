//! L'échelle de migrations de schéma, sortie de `db.rs` par mika#2321.
//!
//! ## Pourquoi cette région-là
//!
//! `db.rs` a deux régions dont la taille est **monotone** : le module de test
//! (sorti par le volet A) et cette échelle. Une migration n'est jamais
//! supprimée — v1 à v54, ajout pur, pour toujours. Les sections `impl Database`
//! restantes croissent aussi, mais elles *churnent* : une méthode y est
//! réécrite, remplacée, supprimée. Borner `db.rs` voulait donc dire sortir ses
//! deux régions monotones, et cette phrase reste vraie quel que soit le taux de
//! croissance mesuré — c'est un argument de structure, pas une extrapolation
//! (plan mika#2321 E1).
//!
//! ## Un seul élargissement de visibilité, et il est ici
//!
//! `migrate` est `pub(super)` parce que ses deux appelants — `Database::open`
//! et `Database::open_in_memory` — sont restés dans `db.rs`, c'est-à-dire dans
//! le module *parent*. Un enfant voit les items privés de son parent ; l'inverse
//! est faux.
//!
//! Les `migrate_vN_to_vM` restent **privées**, et c'est la raison pour laquelle
//! leurs tests vivent dans le `mod tests` de ce fichier plutôt qu'avec les
//! autres tests de `db` : un module de test voyage avec le code qu'il teste
//! (plan mika#2321 D3). Les remonter dans `db::tests` en ferait des *frères* de
//! ce module, et il aurait fallu élargir une trentaine de signatures pour rien.
//!
//! ## Ce qui n'a PAS bougé, et pourquoi
//!
//! - `CURRENT_SCHEMA_VERSION` reste dans `db.rs` : `kg_fixtures::PINNED_SCHEMA_VERSION`
//!   y est lié par un const-assert de compilation
//!   (`docs/solutions/best-practices/schema-bump-fixture-pin-co-edit-2026-05-01.md`)
//!   et la constante est lue bien au-delà des migrations.
//! - `column_exists`, `column_exists_tx`, `current_version`, `schema_version` et
//!   `check_v27_coalesce_guard` restent dans `db.rs`. Ce sont des helpers, pas
//!   des migrations, et ils ont des lecteurs hors de cette échelle — les
//!   descendre ici les rendrait invisibles à `db::tests`, ce qui aurait exigé
//!   exactement les élargissements que D3 évite.

use super::*;

impl Database {
    pub(super) fn migrate(&mut self) -> Result<()> {
        let version = self.current_version()?;
        debug!(
            current_version = version,
            target_version = CURRENT_SCHEMA_VERSION,
            "checking migrations"
        );
        if version < 1 {
            if version > 0 {
                info!(from = version, to = 1, "applying clean-slate migration");
            }
            self.migrate_v1()?;
            info!(
                version = CURRENT_SCHEMA_VERSION,
                "database migrated to v{CURRENT_SCHEMA_VERSION}"
            );
        }
        if (1..3).contains(&version) {
            self.migrate_v3()?;
            info!(version = 3, "database migrated to v3");
        }
        if version == 3 {
            self.migrate_v3_to_v4()?;
            info!(version = 4, "database migrated to v4");
        }
        if version == 4 || version == 3 {
            self.migrate_v4_to_v5()?;
            info!(version = 5, "database migrated to v5");
        }
        if (3..=5).contains(&version) {
            self.migrate_v5_to_v6()?;
            info!(version = 6, "database migrated to v6");
        }
        if (3..=6).contains(&version) {
            self.migrate_v6_to_v7()?;
            info!(version = 7, "database migrated to v7");
        }
        if (3..=7).contains(&version) {
            self.migrate_v7_to_v8()?;
            info!(version = 8, "database migrated to v8");
        }
        if (3..=8).contains(&version) {
            self.migrate_v8_to_v9()?;
            info!(version = 9, "database migrated to v9");
        }
        if (3..=9).contains(&version) {
            self.migrate_v9_to_v10()?;
            info!(version = 10, "database migrated to v10");
        }
        if (3..=10).contains(&version) {
            self.migrate_v10_to_v11()?;
            info!(version = 11, "database migrated to v11");
        }
        if (3..=11).contains(&version) {
            self.migrate_v11_to_v12()?;
            info!(version = 12, "database migrated to v12");
        }
        if (3..=12).contains(&version) {
            self.migrate_v12_to_v13()?;
            info!(version = 13, "database migrated to v13");
        }
        if (3..=13).contains(&version) {
            self.migrate_v13_to_v14()?;
            info!(version = 14, "database migrated to v14");
        }
        if (3..=14).contains(&version) {
            self.migrate_v14_to_v15()?;
            info!(version = 15, "database migrated to v15");
        }
        if (3..=15).contains(&version) {
            self.migrate_v15_to_v16()?;
            info!(version = 16, "database migrated to v16");
        }
        if (3..=16).contains(&version) {
            self.migrate_v16_to_v17()?;
            info!(version = 17, "database migrated to v17");
        }
        if (3..=17).contains(&version) {
            self.migrate_v17_to_v18()?;
            info!(version = 18, "database migrated to v18");
        }
        if (3..=18).contains(&version) {
            self.migrate_v18_to_v19()?;
            info!(version = 19, "database migrated to v19");
        }
        if (3..=19).contains(&version) {
            self.migrate_v19_to_v20()?;
            info!(version = 20, "database migrated to v20");
        }
        if (3..=20).contains(&version) {
            self.migrate_v20_to_v21()?;
            info!(version = 21, "database migrated to v21");
        }
        if (3..=21).contains(&version) {
            self.migrate_v21_to_v22()?;
            info!(version = 22, "database migrated to v22");
        }
        if (3..=22).contains(&version) {
            self.migrate_v22_to_v23()?;
            info!(version = 23, "database migrated to v23");
        }
        if (3..=23).contains(&version) {
            self.migrate_v23_to_v24()?;
            info!(version = 24, "database migrated to v24");
        }
        if (3..=24).contains(&version) {
            self.migrate_v24_to_v25()?;
            info!(version = 25, "database migrated to v25");
        }
        if (3..=25).contains(&version) {
            self.migrate_v25_to_v26()?;
            info!(version = 26, "database migrated to v26");
        }
        if (3..=26).contains(&version) {
            self.migrate_v26_to_v27()?;
            info!(version = 27, "database migrated to v27");
        }

        // v27 startup guard: refuse to return a Database handle if the
        // coalesce step from #787 has not run. Fresh installs write the
        // marker in migrate_v1; existing DBs upgraded via the stub get
        // the marker only when #787's coalesce SQL runs.
        self.check_v27_coalesce_guard()?;

        if (3..=27).contains(&version) {
            self.migrate_v27_to_v28()?;
            info!(version = 28, "database migrated to v28");
        }

        if (3..=28).contains(&version) {
            self.migrate_v28_to_v29()?;
            info!(version = 29, "database migrated to v29");
        }

        if (3..=29).contains(&version) {
            self.migrate_v29_to_v30()?;
            info!(version = 30, "database migrated to v30");
        }

        if (3..=30).contains(&version) {
            self.migrate_v30_to_v31()?;
            info!(version = 31, "database migrated to v31");
        }

        if (3..=31).contains(&version) {
            self.migrate_v31_to_v32()?;
            info!(version = 32, "database migrated to v32");
        }

        if (3..=32).contains(&version) {
            self.migrate_v32_to_v33()?;
            info!(version = 33, "database migrated to v33");
        }

        if (3..=33).contains(&version) {
            self.migrate_v33_to_v34()?;
            info!(version = 34, "database migrated to v34");
        }

        if (3..=34).contains(&version) {
            self.migrate_v34_to_v35()?;
            info!(version = 35, "database migrated to v35");
        }

        if (3..=35).contains(&version) {
            self.migrate_v35_to_v36()?;
            info!(version = 36, "database migrated to v36");
        }

        if (3..=36).contains(&version) {
            self.migrate_v36_to_v37()?;
            info!(version = 37, "database migrated to v37");
        }

        if (3..=37).contains(&version) {
            self.migrate_v37_to_v38()?;
            info!(version = 38, "database migrated to v38");
        }

        if (3..=38).contains(&version) {
            self.migrate_v38_to_v39()?;
            info!(version = 39, "database migrated to v39");
        }

        if (3..=39).contains(&version) {
            self.migrate_v39_to_v40()?;
            info!(version = 40, "database migrated to v40");
        }

        if (3..=40).contains(&version) {
            self.migrate_v40_to_v41()?;
            info!(version = 41, "database migrated to v41");
        }

        if (3..=41).contains(&version) {
            self.migrate_v41_to_v42()?;
            info!(version = 42, "database migrated to v42");
        }

        if (3..=42).contains(&version) {
            self.migrate_v42_to_v43()?;
            info!(version = 43, "database migrated to v43");
        }

        if (3..=43).contains(&version) {
            self.migrate_v43_to_v44()?;
            info!(version = 44, "database migrated to v44");
        }

        if (3..=44).contains(&version) {
            self.migrate_v44_to_v45()?;
            info!(version = 45, "database migrated to v45");
        }

        if (3..=45).contains(&version) {
            self.migrate_v45_to_v46()?;
            info!(version = 46, "database migrated to v46");
        }

        if (3..=46).contains(&version) {
            self.migrate_v46_to_v47()?;
            info!(version = 47, "database migrated to v47");
        }

        if (3..=47).contains(&version) {
            self.migrate_v47_to_v48()?;
            info!(version = 48, "database migrated to v48");
        }

        if (3..=48).contains(&version) {
            self.migrate_v48_to_v49()?;
            info!(version = 49, "database migrated to v49");
        }

        if (3..=49).contains(&version) {
            self.migrate_v49_to_v50()?;
            info!(version = 50, "database migrated to v50");
        }

        if (3..=50).contains(&version) {
            self.migrate_v50_to_v51()?;
            info!(version = 51, "database migrated to v51");
        }

        if (3..=51).contains(&version) {
            self.migrate_v51_to_v52()?;
            info!(version = 52, "database migrated to v52");
        }

        if (3..=52).contains(&version) {
            self.migrate_v52_to_v53()?;
            info!(version = 53, "database migrated to v53");
        }

        if (3..=53).contains(&version) {
            self.migrate_v53_to_v54()?;
            info!(version = 54, "database migrated to v54");
        }

        Ok(())
    }

    fn migrate_v1(&mut self) -> Result<()> {
        info!("applying migration v1: unified task engine schema (clean slate)");

        // Drop all existing tables (clean slate — no backward compat constraint)
        let drops = [
            "DROP TABLE IF EXISTS fts_search",
            "DROP TABLE IF EXISTS vec_search",
            "DROP TABLE IF EXISTS tool_calls",
            "DROP TABLE IF EXISTS llm_calls",
            "DROP TABLE IF EXISTS a2a_push_notification_configs",
            "DROP TABLE IF EXISTS a2a_artifacts",
            "DROP TABLE IF EXISTS a2a_messages",
            "DROP TABLE IF EXISTS a2a_tasks",
            "DROP TABLE IF EXISTS a2a_task_map",
            "DROP TABLE IF EXISTS reminders",
            "DROP TABLE IF EXISTS heartbeat_sends",
            "DROP TABLE IF EXISTS reflection_runs",
            "DROP TABLE IF EXISTS failed_sends",
            "DROP TABLE IF EXISTS customer_config",
            "DROP TABLE IF EXISTS audit_event_summaries",
            "DROP TABLE IF EXISTS audit_events",
            "DROP TABLE IF EXISTS memory_event_summaries",
            "DROP TABLE IF EXISTS memory_events",
            "DROP TABLE IF EXISTS search_content",
            "DROP TABLE IF EXISTS events",
            "DROP TABLE IF EXISTS preferences",
            "DROP TABLE IF EXISTS commitments",
            "DROP TABLE IF EXISTS people",
            "DROP TABLE IF EXISTS core_memory",
            "DROP TABLE IF EXISTS team_workspace",
            "DROP TABLE IF EXISTS team_messages",
            "DROP TABLE IF EXISTS team_runs",
            "DROP TABLE IF EXISTS messages",
            "DROP TABLE IF EXISTS sessions",
            "DROP TABLE IF EXISTS conversations",
            "DROP TABLE IF EXISTS tasks",
            "DROP TABLE IF EXISTS teams",
            "DROP TABLE IF EXISTS agents",
            "DROP TABLE IF EXISTS schema_version",
        ];
        for drop in &drops {
            self.conn.execute_batch(drop)?;
        }

        let tx = self.conn.transaction()?;
        tx.execute_batch(
                "
            CREATE TABLE schema_version (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO schema_version (version) VALUES (54);

            -- Schema meta table for migration state tracking (v27+).
            CREATE TABLE schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            -- Fresh installs are trivially coalesce-complete (no v26 data).
            INSERT INTO schema_meta (key, value) VALUES ('v27_coalesce_complete', '1');

            CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            -- Per-agent KG corpus mapping (#798). Maps agent_id to
            -- docs_root_hash for multi-corpus query fan-out.
            CREATE TABLE agent_kg_corpora (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                docs_root_hash TEXT NOT NULL,
                docs_root_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, docs_root_hash)
            );
            CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);

            CREATE TABLE teams (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE team_runs (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','cancelled','suspended','failed_no_delegation','failed_transport')),
                failure_reason TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                max_iterations INTEGER NOT NULL DEFAULT 3,
                deliverable TEXT,
                checkpoint TEXT,
                trace_id TEXT,
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                delegation_count INTEGER NOT NULL DEFAULT 0,
                solo_absorption INTEGER NOT NULL DEFAULT 0,
                failure_context TEXT
            );
            CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);

            CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual','a2a')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                metadata TEXT,
                type TEXT NOT NULL DEFAULT 'issue' CHECK (
                    type IN ('issue', 'milestone', 'project')
                ),
                dispatch_class TEXT CHECK (
                    dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom')
                ),
                -- mika#1948 Porte 2: which dispatcher inside this engine
                -- initiated the task. NULL = pre-v51 row, read as 'mika_dev'.
                dispatcher_source TEXT CHECK (
                    dispatcher_source IS NULL
                    OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                ),
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
            CREATE INDEX idx_tasks_dispatcher_source
                ON tasks(agent_id, dispatcher_source, status)
                WHERE dispatcher_source IS NOT NULL;

            -- mika#1948 Porte 2 — one row per (agent, class, slot) = one exec
            -- slot. The PRIMARY KEY is what makes the claim atomic: a second
            -- claimant's INSERT collides instead of racing a SELECT.
            -- mika#2160: `slot_index` joined the key so the class ceases to be
            -- a hard cap of one. At the default cap of 1 exactly one index (0)
            -- is ever written, which is the pre-v52 shape bit for bit.
            CREATE TABLE dispatch_slot_leases (
                agent_id TEXT NOT NULL,
                dispatch_class TEXT NOT NULL,
                slot_index INTEGER NOT NULL DEFAULT 0,
                holder_task_id TEXT NOT NULL,
                dispatcher_source TEXT CHECK (
                    dispatcher_source IS NULL
                    OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                ),
                acquired_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (agent_id, dispatch_class, slot_index)
            );

            -- mika#2192 — one row per (repo, issue) = one declared worktree
            -- owner. Twin of dispatch_slot_leases: the PRIMARY KEY is what
            -- makes the claim a fact rather than a convention, and expiry is
            -- what keeps a dead claimant from freezing a ticket forever.
            --
            -- The key is (repo, issue_number) and NOT the worktree path. The
            -- invariants worktree_path_slug == sanitize(branch_ref) and
            -- branch_ref == derive-branch-name(title, issue, labels) make the
            -- pair sufficient, and it is what the tool boundary already holds
            -- (`tool_input.prompt` is `mika#2192`). Keying by path would force
            -- the Rust side to re-derive the branch — the duplication
            -- mika-platform#58 closed.
            CREATE TABLE worktree_claims (
                repo TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                owner_kind TEXT NOT NULL CHECK (
                    owner_kind IN ('pilot', 'orchestrator', 'spawn')
                ),
                owner_id TEXT NOT NULL,
                owner_label TEXT,
                claimed_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                PRIMARY KEY (repo, issue_number)
            );
            CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');
            CREATE UNIQUE INDEX idx_tasks_manual_active_ref_url
                ON tasks(agent_id, reference_url)
                WHERE trigger_type = 'manual'
                AND reference_url IS NOT NULL
                AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');
            CREATE INDEX idx_tasks_dispatch_class
                ON tasks(agent_id, dispatch_class, status)
                WHERE dispatch_class IS NOT NULL;

            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                metadata TEXT,
                parent_session_id TEXT,
                task_id TEXT
            );
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;
            CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;

            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                internal INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;

            CREATE TABLE task_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                agent_id   TEXT NOT NULL,
                session_id TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                trace_id   TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_task_messages_task_created
                ON task_messages (task_id, created_at);
            CREATE INDEX idx_task_messages_agent_created
                ON task_messages (agent_id, created_at);

            CREATE TABLE core_memory (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE people (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                last_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                mention_count INTEGER NOT NULL DEFAULT 1,
                UNIQUE (agent_id, canonical_name)
            );

            CREATE TABLE commitments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','completed','cancelled')),
                due_date TEXT,
                person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                completed_at TEXT
            );
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                ON commitments(agent_id, description COLLATE NOCASE, due_date)
                WHERE status = 'pending';

            CREATE TABLE preferences (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, category)
            );

            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
            CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id) WHERE rewound_by_trace_id IS NOT NULL;

            CREATE TABLE audit_event_summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, year, month)
            );

            CREATE TABLE search_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);

            CREATE TABLE team_workspace (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_workspace(id),
                agent_name TEXT,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                trace_id TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
            CREATE INDEX idx_team_ws_trace ON team_workspace(trace_id)
                WHERE trace_id IS NOT NULL;

            CREATE TABLE heartbeat_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);

            CREATE TABLE reflection_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);

            CREATE TABLE customer_config (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );

            CREATE TABLE failed_sends (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE skill_overrides (
                agent_id        TEXT NOT NULL COLLATE NOCASE,
                skill_name      TEXT NOT NULL COLLATE NOCASE,
                always_on       INTEGER,
                llm_provider    TEXT,
                llm_model       TEXT,
                enabled         INTEGER,
                lifecycle_state TEXT CHECK (lifecycle_state IN ('staged', 'active', 'archived')),
                use_count       INTEGER NOT NULL DEFAULT 0,
                last_used_at    TEXT,
                PRIMARY KEY (agent_id, skill_name)
            );

            -- A2A Protocol tables: thin mapping table + genuinely new tables
            CREATE TABLE a2a_task_map (
                a2a_task_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                context_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_a2a_task_map_task ON a2a_task_map(task_id);

            CREATE TABLE a2a_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                artifact_id TEXT NOT NULL,
                name TEXT,
                description TEXT,
                parts TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE a2a_push_notification_configs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                token TEXT,
                auth_scheme TEXT,
                auth_credentials TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');

            CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

            -- Observability: LLM call tracking
            CREATE TABLE llm_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                stop_reason TEXT,
                status TEXT NOT NULL DEFAULT 'success',
                error_message TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                prompt_variant TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                response_text TEXT,
                reasoning TEXT,
                system_prompt_bytes INTEGER,
                request_bytes INTEGER
            );
            CREATE INDEX idx_llm_calls_trace ON llm_calls(trace_id);
            CREATE INDEX idx_llm_calls_session ON llm_calls(session_id);
            CREATE INDEX idx_llm_calls_agent_created ON llm_calls(agent_id, created_at);

            -- Observability: Tool call tracking (full I/O)
            CREATE TABLE tool_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                llm_call_id TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                tool_source TEXT NOT NULL DEFAULT 'builtin',
                skill_name TEXT,
                input TEXT,
                output TEXT,
                success INTEGER NOT NULL DEFAULT 1,
                non_zero_exit INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_tool_calls_trace ON tool_calls(trace_id);
            CREATE INDEX idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX idx_tool_calls_llm_call ON tool_calls(llm_call_id);
            CREATE INDEX idx_tool_calls_agent_created ON tool_calls(agent_id, created_at);

            -- KG domain layer (global, no agent_id)
            CREATE TABLE kg_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entity_key TEXT NOT NULL UNIQUE,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                CHECK (entity_key = type || ':' || name)
            );
            CREATE INDEX idx_kg_entities_type ON kg_entities(type);

            CREATE TABLE kg_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_kg_rel_from ON kg_relationships(from_entity_id, type);
            CREATE INDEX idx_kg_rel_to ON kg_relationships(to_entity_id, type);

            -- KG lexical layer (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                seq_id INTEGER NOT NULL,
                source_doc_path TEXT NOT NULL,
                source_doc_hash TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (docs_root_hash, source_doc_path, seq_id)
            );
            CREATE INDEX idx_kg_chunks_docs_root_hash_doc ON kg_chunks(docs_root_hash, source_doc_path);

            -- KG subject layer (shared by docs_root_hash — v27)
            CREATE TABLE kg_subject_entities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                entity_key TEXT NOT NULL,
                type TEXT NOT NULL,
                name TEXT NOT NULL,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                discovered INTEGER NOT NULL DEFAULT 0,
                discovery_reason TEXT,
                CHECK (entity_key = type || ':' || name),
                UNIQUE (docs_root_hash, entity_key)
            );
            CREATE INDEX idx_kg_subj_entities_drh_type ON kg_subject_entities(docs_root_hash, type);

            CREATE TABLE kg_subject_resolutions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (agent_id, subject_entity_id, domain_entity_id)
            );
            CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
            CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

            -- KG subject-to-subject edges / fact triples (shared by docs_root_hash — v27)
            CREATE TABLE kg_subject_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                type TEXT NOT NULL,
                confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                properties_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                trace_id TEXT,
                UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
            );
            CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(docs_root_hash, from_entity_id, type);
            CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(docs_root_hash, to_entity_id, type);
            CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(docs_root_hash, type);

            -- KG entity provenance: chunk -> subject entity (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunk_subjects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
            );
            CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(docs_root_hash, chunk_id);
            CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash, subject_entity_id);
            CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(docs_root_hash, extraction_trace_id);

            -- KG relationship provenance: chunk -> subject relationship (shared by docs_root_hash — v27)
            CREATE TABLE kg_chunk_subject_relationships (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
            );
            CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(docs_root_hash, chunk_id);
            CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(docs_root_hash, subject_relationship_id);

            -- KG extraction tracking (shared by docs_root_hash — v27; first-writer-wins)
            CREATE TABLE kg_extractions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                docs_root_hash TEXT NOT NULL,
                docs_root TEXT NOT NULL,
                source_doc_path TEXT NOT NULL,
                source_doc_hash TEXT,
                extraction_model TEXT NOT NULL,
                entities_extracted INTEGER NOT NULL DEFAULT 0,
                relationships_extracted INTEGER NOT NULL DEFAULT 0,
                extraction_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (docs_root_hash, source_doc_path)
            );
            CREATE INDEX idx_kg_extractions_drh ON kg_extractions(docs_root_hash);

            -- KG resolution tracking
            CREATE TABLE kg_resolutions_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                outcome TEXT NOT NULL CHECK (outcome IN (
                    'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                    'no_match', 'no_candidate_of_type',
                    'skipped_discovered_type', 'skipped_discovered_subject',
                    'skipped_no_llm', 'error'
                )),
                resolution_trace_id TEXT NOT NULL,
                source_extraction_trace_id TEXT,
                model TEXT,
                duration_ms INTEGER,
                resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, subject_entity_id)
            );
            CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

            -- KG invalidation markers (#961): ephemeral sidecar for tracking
            -- entities whose no_match resolution log rows were deleted by
            -- domain-graph rebuild invalidation (#960).
            CREATE TABLE kg_invalidated_no_match (
                subject_entity_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (subject_entity_id, agent_id)
            );

            -- Operational ledger (#1262): canonical operational-item store.
            CREATE TABLE operational_items (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
                title TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
                owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
                owner_name TEXT,
                priority REAL NOT NULL DEFAULT 0.0,
                user_importance REAL NOT NULL DEFAULT 0.0,
                due_at TEXT,
                blocked_by TEXT,
                next_action TEXT,
                evidence_refs TEXT NOT NULL DEFAULT '[]',
                confidence REAL NOT NULL DEFAULT 1.0,
                source_table TEXT,
                source_id TEXT,
                agent_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
            CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
            CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
            CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
            CREATE UNIQUE INDEX idx_operational_items_source_unique
                ON operational_items(agent_id, source_table, source_id)
                WHERE source_table IS NOT NULL AND source_id IS NOT NULL;

            -- Auto-pull circuit-breaker stats (mika#1363) + re-drive budget (mika#2020)
            CREATE TABLE auto_pull_stats (
                repo_full_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                last_auto_pull_at TEXT,
                last_failure_at TEXT,
                redrive_count INTEGER NOT NULL DEFAULT 0,
                last_redrive_at TEXT,
                redrive_abandoned_at TEXT,
                PRIMARY KEY (repo_full_name, issue_number)
            );

            -- Permission-decision provenance ledger (mika#1733 AC4)
            CREATE TABLE permission_decisions (
                id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_summary TEXT,
                classifier_verdict TEXT NOT NULL
                    CHECK (classifier_verdict IN ('approved', 'denied', 'held')),
                operator_decision TEXT
                    CHECK (operator_decision IN ('approve', 'deny')),
                override_used INTEGER NOT NULL DEFAULT 0
                    CHECK (override_used IN (0, 1)),
                decision_authority TEXT NOT NULL
                    CHECK (decision_authority IN ('strict', 'override')),
                tenant_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_permission_decisions_request_id
                ON permission_decisions(request_id);
            CREATE INDEX idx_permission_decisions_created_at
                ON permission_decisions(created_at DESC);

            -- v45: pilot_transcripts (mika#1705). Must be in v1 DDL so fresh installs
            -- get the table without depending on the v44→v45 migration chain (which is
            -- skipped on fresh install because migrate() captures version BEFORE
            -- migrate_v1 runs and the (3..=44).contains(&0) guard fails).
            CREATE TABLE pilot_transcripts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                timestamp TEXT,
                provider TEXT,
                model TEXT,
                request_body TEXT,
                response_body TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                latency_ms INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_pilot_transcripts_task_id
                ON pilot_transcripts(task_id);
            CREATE INDEX idx_pilot_transcripts_created_at
                ON pilot_transcripts(created_at DESC);

            -- v46: served_content (mika#1867). Per-(agent, person, category) ledger
            -- of content Mika has served (proverb, quote, joke, poem, recommendation,
            -- story, fact) so re-generation on future turns can dedup. Founding
            -- incident: Al (Vietnam tester) 2026-07-28 — same zen proverb served
            -- twice, 6 days apart. Must be in v1 DDL so fresh installs get the
            -- table without depending on the v45→v46 migration chain.
            CREATE TABLE served_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
                category TEXT NOT NULL CHECK (category IN (
                    'proverb','quote','joke','poem','recommendation','story','fact'
                )),
                content_text TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                -- Reserved for v2 fuzzy dedup (embedding cosine similarity per AC6).
                -- Format TBD — likely 384-dim float array as BLOB or hex string.
                content_signature TEXT,
                served_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                UNIQUE(agent_id, person_id, content_hash)
            );
            CREATE INDEX idx_served_content_person_cat
                ON served_content(agent_id, person_id, category, served_at DESC);
            CREATE INDEX idx_served_content_hash
                ON served_content(agent_id, person_id, content_hash);

            -- Pre-register the default 'mika' agent
            INSERT INTO agents (id, name, home_dir) VALUES ('mika', 'Mika', '');
            ",
            )
            .context("failed to create v1 schema")?;
        tx.commit()?;

        // Unified timeline VIEW (uses shared constant)
        self.conn
            .execute_batch(UNIFIED_TIMELINE_VIEW_SQL)
            .context("failed to create unified_timeline view")?;

        // Virtual tables must be outside transactions
        let _ = self.conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_search
                 USING fts5(content, content='search_content', content_rowid='id');
             CREATE VIRTUAL TABLE IF NOT EXISTS vec_search
                 USING vec0(embedding float[512]);",
        );

        Ok(())
    }

    /// Migration v3: Sessions + Messages schema redesign (clean-slate).
    ///
    /// Single user, no data to preserve. Drop and recreate via migrate_v1.
    fn migrate_v3(&mut self) -> Result<()> {
        info!("applying migration v3: sessions + messages schema redesign (clean slate)");
        self.migrate_v1()
    }

    /// Migration v3 → v4: Add duplicate-prevention indexes to existing v3 databases.
    ///
    /// New databases already get these indexes via `migrate_v1`, but databases
    /// created at v3 before these indexes were added need them applied retroactively.
    fn migrate_v3_to_v4(&self) -> Result<()> {
        info!("migrating database schema v3 → v4 (duplicate-prevention indexes)");
        self.conn.execute_batch(
            "DROP INDEX IF EXISTS idx_tasks_unique_recurring;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');

             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');

             CREATE UNIQUE INDEX IF NOT EXISTS idx_events_unique_description
                ON events(agent_id, description COLLATE NOCASE, event_date)
                WHERE event_date IS NOT NULL;

             -- Rebuild commitments table: replace inline UNIQUE(agent_id, description)
             -- with partial unique index scoped to pending status
             CREATE TABLE commitments_new (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 description TEXT NOT NULL COLLATE NOCASE,
                 status TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending','completed','cancelled')),
                 due_date TEXT,
                 person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                 created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                 completed_at INTEGER
             );
             INSERT INTO commitments_new SELECT * FROM commitments;
             DROP TABLE commitments;
             ALTER TABLE commitments_new RENAME TO commitments;
             CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_commitments_unique_pending
                 ON commitments(agent_id, description COLLATE NOCASE, due_date)
                 WHERE status = 'pending';

             PRAGMA user_version = 4;",
        )?;
        // Update the schema_version table to reflect v4
        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (4)", [])?;
        Ok(())
    }

    /// Migration v4 → v5: Rename memory_events → audit_events, add trace_id columns,
    /// create unified_timeline VIEW.
    ///
    /// Idempotent: each step checks existence before acting, since `ALTER TABLE RENAME TO`
    /// auto-commits outside transactions in SQLite and a crash mid-migration could leave
    /// partial state.
    fn migrate_v4_to_v5(&self) -> Result<()> {
        info!("migrating database schema v4 → v5 (orthogonal observability)");

        // 1. Rename memory_events → audit_events (idempotent)
        let has_old: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_events'",
            [],
            |r| r.get(0),
        )?;
        if has_old {
            self.conn
                .execute_batch("ALTER TABLE memory_events RENAME TO audit_events")?;
        }

        // 2. Recreate indexes with new names
        self.conn.execute_batch(
            "DROP INDEX IF EXISTS idx_memev_agent_created;
             DROP INDEX IF EXISTS idx_memev_session;
             CREATE INDEX IF NOT EXISTS idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_events(session_id);",
        )?;

        // 3. Rename memory_event_summaries → audit_event_summaries (idempotent)
        let has_old_summaries: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_event_summaries'",
            [],
            |r| r.get(0),
        )?;
        if has_old_summaries {
            self.conn.execute_batch(
                "ALTER TABLE memory_event_summaries RENAME TO audit_event_summaries",
            )?;
        }

        // 4. Add trace_id columns (idempotent — ALTER TABLE ADD COLUMN is a no-op if exists)
        // We check column existence via pragma to avoid "duplicate column" errors on re-run.
        if !self.column_exists("messages", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE messages ADD COLUMN trace_id TEXT")?;
        }
        if !self.column_exists("tasks", "created_trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE tasks ADD COLUMN created_trace_id TEXT")?;
        }
        if !self.column_exists("audit_events", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE audit_events ADD COLUMN trace_id TEXT")?;
        }
        if !self.column_exists("team_workspace", "trace_id")? {
            self.conn
                .execute_batch("ALTER TABLE team_workspace ADD COLUMN trace_id TEXT")?;
        }

        // 5. Create partial indexes on trace_id columns
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;",
        )?;

        // 6. Create unified_timeline VIEW (uses shared constant)
        self.conn
            .execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // 7. Update schema version
        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (5)", [])?;

        Ok(())
    }

    fn migrate_v5_to_v6(&self) -> Result<()> {
        info!("migrating database schema v5 → v6 (people mention_count)");

        if !self.column_exists("people", "mention_count")? {
            self.conn.execute_batch(
                "ALTER TABLE people ADD COLUMN mention_count INTEGER NOT NULL DEFAULT 1",
            )?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (6)", [])?;

        Ok(())
    }

    fn migrate_v6_to_v7(&self) -> Result<()> {
        info!("migrating database schema v6 → v7 (skill_overrides table)");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS skill_overrides (
                agent_id   TEXT NOT NULL COLLATE NOCASE,
                skill_name TEXT NOT NULL COLLATE NOCASE,
                always_on  INTEGER,
                PRIMARY KEY (agent_id, skill_name)
            )",
        )?;

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (7)", [])?;

        Ok(())
    }

    fn migrate_v7_to_v8(&mut self) -> Result<()> {
        info!(
            "migrating database schema v7 → v8 (tasks: manual trigger_type, blocked status, none action_type, reference_url, source)"
        );

        // SQLite cannot ALTER CHECK constraints, so we must rebuild the tasks table.
        // Entire migration wrapped in a transaction to prevent partial state on crash.
        //
        // PRAGMA foreign_keys must be OFF during the table rebuild because the INSERT
        // copies self-referencing parent_task_id rows, and ALTER TABLE RENAME validates
        // FK references. Also disable FK checks to avoid issues with the temporary table.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop the unified_timeline VIEW first — it references the `tasks` table.
        // SQLite 3.25+ validates all views/triggers during ALTER TABLE RENAME,
        // so the view must not exist when we rename tasks_new → tasks.
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at INTEGER,
                timeout_at INTEGER,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                fired_at INTEGER,
                completed_at INTEGER
            );

            INSERT INTO tasks_new (
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                status, process_id, input_context, result,
                created_by_session, created_trace_id,
                created_at, updated_at, fired_at, completed_at
            )
            SELECT
                id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                next_fire_at, timeout_at, action_type, action_config,
                status, process_id, input_context, result,
                created_by_session, created_trace_id,
                created_at, updated_at, fired_at, completed_at
            FROM tasks;

            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;",
        )?;

        // Recreate all indexes (still within the transaction)
        tx.execute_batch(
            "CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
             CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
             CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');
             CREATE INDEX IF NOT EXISTS idx_tasks_callback_delivery
                ON tasks(agent_id, completed_at)
                WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');
             CREATE INDEX IF NOT EXISTS idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');",
        )?;

        // Recreate unified_timeline VIEW (was dropped before table rebuild)
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (8)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v8_to_v9(&mut self) -> Result<()> {
        info!(
            "migrating database schema v8 → v9 (rewind: nullable after_value, rewound_by_trace_id)"
        );

        // Rebuild audit_events to make after_value nullable and add rewound_by_trace_id.
        // SQLite cannot ALTER a NOT NULL constraint, so we must rebuild the table.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop the unified_timeline VIEW — it references audit_events.
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        tx.execute_batch(
            "CREATE TABLE audit_events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at INTEGER NOT NULL DEFAULT (unixepoch())
            );

            INSERT INTO audit_events_new (id, agent_id, session_id, tool_name, target_key,
                before_value, after_value, reasoning, trace_id, created_at)
            SELECT id, agent_id, session_id, tool_name, target_key,
                before_value, after_value, reasoning, trace_id, created_at
            FROM audit_events;

            DROP TABLE audit_events;
            ALTER TABLE audit_events_new RENAME TO audit_events;",
        )?;

        // Recreate existing indexes + new rewound index
        tx.execute_batch(
            "CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at);
             CREATE INDEX idx_audit_session ON audit_events(session_id);
             CREATE INDEX idx_audit_trace ON audit_events(trace_id)
                 WHERE trace_id IS NOT NULL;
             CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id)
                 WHERE rewound_by_trace_id IS NOT NULL;",
        )?;

        // Recreate unified_timeline VIEW
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (9)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v9_to_v10(&mut self) -> Result<()> {
        info!(
            "migrating database schema v9 → v10 (team_runs.trace_id, unified_timeline + team_workspace)"
        );

        // Hoist column_exists check before creating the transaction (borrow checker constraint).
        let has_trace_id = self.column_exists("team_runs", "trace_id")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Add trace_id column to team_runs (idempotent guard for crash recovery)
        if !has_trace_id {
            tx.execute_batch("ALTER TABLE team_runs ADD COLUMN trace_id TEXT;")?;
        }

        // Recreate unified_timeline VIEW with team_workspace union
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // Add partial index on team_workspace.trace_id (matches other timeline tables)
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_team_ws_trace ON team_workspace(trace_id)
                 WHERE trace_id IS NOT NULL;",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (10)", [])?;

        tx.commit()?;
        Ok(())
    }

    fn migrate_v10_to_v11(&mut self) -> Result<()> {
        info!(
            "migrating database schema v10 → v11 (tasks.execution_trace_id, sessions.parent_session_id)"
        );

        // Hoist column_exists checks before creating the transaction (borrow checker constraint).
        let has_exec_trace = self.column_exists("tasks", "execution_trace_id")?;
        let has_parent_session = self.column_exists("sessions", "parent_session_id")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Add execution_trace_id column to tasks (idempotent guard)
        if !has_exec_trace {
            tx.execute_batch("ALTER TABLE tasks ADD COLUMN execution_trace_id TEXT;")?;
        }

        // Add parent_session_id column to sessions (idempotent guard)
        if !has_parent_session {
            tx.execute_batch("ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;")?;
        }

        // Partial indexes for new columns
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;",
        )?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;",
        )?;

        // Recreate unified_timeline VIEW with COALESCE for execution_trace_id
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (11)", [])?;

        tx.commit()?;
        Ok(())
    }

    fn migrate_v11_to_v12(&mut self) -> Result<()> {
        info!("migrating database schema v11 → v12 (INTEGER timestamps → ISO 8601 TEXT)");

        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop views that reference tables we're rebuilding
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        // --- agents ---
        tx.execute_batch(
            "CREATE TABLE agents_new (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                home_dir TEXT NOT NULL DEFAULT '',
                active BOOLEAN NOT NULL DEFAULT 1,
                last_seen TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO agents_new SELECT id, name, home_dir, active,
                CASE WHEN last_seen IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', last_seen, 'unixepoch') ELSE NULL END,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM agents;
            DROP TABLE agents;
            ALTER TABLE agents_new RENAME TO agents;")?;

        // --- teams ---
        tx.execute_batch(
            "CREATE TABLE teams_new (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL COLLATE NOCASE,
                config_path TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO teams_new SELECT id, name, config_path,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM teams;
            DROP TABLE teams;
            ALTER TABLE teams_new RENAME TO teams;",
        )?;

        // --- team_runs ---
        tx.execute_batch(
            "CREATE TABLE team_runs_new (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','cancelled','suspended')),
                failure_reason TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                max_iterations INTEGER NOT NULL DEFAULT 3,
                deliverable TEXT,
                checkpoint TEXT,
                trace_id TEXT,
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT
            );
            INSERT INTO team_runs_new SELECT id, team_id, goal, status, failure_reason,
                iteration, max_iterations, deliverable, checkpoint, trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
                CASE WHEN ended_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch') ELSE NULL END
            FROM team_runs;
            DROP TABLE team_runs;
            ALTER TABLE team_runs_new RENAME TO team_runs;
            CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);")?;

        // --- sessions (must be before messages due to FK) ---
        tx.execute_batch(
            "CREATE TABLE sessions_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                channel_type TEXT NOT NULL DEFAULT 'cli',
                started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                ended_at TEXT,
                metadata TEXT,
                parent_session_id TEXT
            );
            INSERT INTO sessions_new SELECT id, agent_id, channel_type,
                strftime('%Y-%m-%dT%H:%M:%SZ', started_at, 'unixepoch'),
                CASE WHEN ended_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', ended_at, 'unixepoch') ELSE NULL END,
                metadata, parent_session_id
            FROM sessions;
            DROP TABLE sessions;
            ALTER TABLE sessions_new RENAME TO sessions;
            CREATE INDEX idx_sessions_agent ON sessions(agent_id, started_at DESC);
            CREATE INDEX idx_sessions_parent ON sessions(parent_session_id) WHERE parent_session_id IS NOT NULL;")?;

        // --- tasks ---
        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            INSERT INTO tasks_new SELECT id, agent_id, team_run_id, parent_task_id, depth, label,
                trigger_type, cron_expr, event_source, event_offset_secs, condition_expr,
                CASE WHEN next_fire_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', next_fire_at, 'unixepoch') ELSE NULL END,
                CASE WHEN timeout_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', timeout_at, 'unixepoch') ELSE NULL END,
                action_type, action_config, status, process_id, input_context, result,
                reference_url, source, created_by_session, created_trace_id, execution_trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch'),
                CASE WHEN fired_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', fired_at, 'unixepoch') ELSE NULL END,
                CASE WHEN completed_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', completed_at, 'unixepoch') ELSE NULL END
            FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;")?;

        // Recreate task indexes
        tx.execute_batch(
            "CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
             CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at) WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_schedulable ON tasks(agent_id, next_fire_at ASC) WHERE status IN ('pending','recurring_active');
             CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
             CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
             CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
             CREATE INDEX idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
             CREATE INDEX idx_tasks_manual_active ON tasks(agent_id, created_at DESC) WHERE trigger_type = 'manual' AND status IN ('pending', 'in_progress', 'blocked');
             CREATE UNIQUE INDEX idx_tasks_unique_recurring ON tasks(agent_id, label COLLATE NOCASE) WHERE trigger_type = 'recurring' AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
             CREATE UNIQUE INDEX idx_tasks_unique_reminder ON tasks(agent_id, label COLLATE NOCASE) WHERE status IN ('pending', 'in_progress', 'recurring_active') AND (action_type = 'send_message' OR action_type = 'resume_agent') AND trigger_type NOT IN ('callback');
             CREATE INDEX idx_tasks_callback_delivery ON tasks(agent_id, completed_at) WHERE trigger_type='callback' AND action_type='resume_agent' AND status IN ('completed','failed');")?;

        // --- messages ---
        tx.execute_batch(
            "CREATE TABLE messages_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system','summary','tool_result')),
                content TEXT NOT NULL,
                metadata TEXT,
                trace_id TEXT,
                compacted_through_id INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO messages_new SELECT id, session_id, agent_id, role, content, metadata,
                trace_id, compacted_through_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM messages;
            DROP TABLE messages;
            ALTER TABLE messages_new RENAME TO messages;
            CREATE INDEX idx_msg_session ON messages(session_id, created_at ASC);
            CREATE INDEX idx_msg_agent_created ON messages(agent_id, created_at DESC);
            CREATE INDEX idx_msg_trace ON messages(trace_id) WHERE trace_id IS NOT NULL;")?;

        // --- core_memory ---
        tx.execute_batch(
            "CREATE TABLE core_memory_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                token_count INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );
            INSERT INTO core_memory_new SELECT agent_id, key, value, token_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM core_memory;
            DROP TABLE core_memory;
            ALTER TABLE core_memory_new RENAME TO core_memory;",
        )?;

        // --- people ---
        tx.execute_batch(
            "CREATE TABLE people_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                relationship TEXT,
                notes TEXT,
                first_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                last_mentioned TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                mention_count INTEGER NOT NULL DEFAULT 1,
                UNIQUE (agent_id, canonical_name)
            );
            INSERT INTO people_new SELECT id, agent_id, canonical_name, relationship, notes,
                strftime('%Y-%m-%dT%H:%M:%SZ', first_mentioned, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', last_mentioned, 'unixepoch'),
                mention_count
            FROM people;
            DROP TABLE people;
            ALTER TABLE people_new RENAME TO people;",
        )?;

        // --- commitments ---
        tx.execute_batch(
            "CREATE TABLE commitments_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL COLLATE NOCASE,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','completed','cancelled')),
                due_date TEXT,
                person_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                completed_at TEXT
            );
            INSERT INTO commitments_new SELECT id, agent_id, description, status, due_date, person_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                CASE WHEN completed_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', completed_at, 'unixepoch') ELSE NULL END
            FROM commitments;
            DROP TABLE commitments;
            ALTER TABLE commitments_new RENAME TO commitments;
            CREATE INDEX idx_commit_agent_status ON commitments(agent_id, status);
            CREATE UNIQUE INDEX idx_commitments_unique_pending ON commitments(agent_id, description COLLATE NOCASE, due_date) WHERE status = 'pending';")?;

        // --- preferences ---
        tx.execute_batch(
            "CREATE TABLE preferences_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                category TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, category)
            );
            INSERT INTO preferences_new SELECT agent_id, category, value,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM preferences;
            DROP TABLE preferences;
            ALTER TABLE preferences_new RENAME TO preferences;",
        )?;

        // --- events ---
        tx.execute_batch(
            "CREATE TABLE events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                event_date TEXT,
                context TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO events_new SELECT id, agent_id, description, event_date, context,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM events;
            DROP TABLE events;
            ALTER TABLE events_new RENAME TO events;
            CREATE UNIQUE INDEX idx_events_unique_description ON events(agent_id, description COLLATE NOCASE, event_date) WHERE event_date IS NOT NULL;")?;

        // --- audit_events ---
        tx.execute_batch(
            "CREATE TABLE audit_events_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                target_key TEXT NOT NULL,
                before_value TEXT,
                after_value TEXT,
                reasoning TEXT,
                trace_id TEXT,
                rewound_by_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO audit_events_new SELECT id, agent_id, session_id, tool_name,
                target_key, before_value, after_value, reasoning, trace_id, rewound_by_trace_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM audit_events;
            DROP TABLE audit_events;
            ALTER TABLE audit_events_new RENAME TO audit_events;
            CREATE INDEX idx_audit_agent_created ON audit_events(agent_id, created_at DESC);
            CREATE INDEX idx_audit_session ON audit_events(session_id);
            CREATE INDEX idx_audit_trace ON audit_events(trace_id) WHERE trace_id IS NOT NULL;
            CREATE INDEX idx_audit_rewound ON audit_events(rewound_by_trace_id) WHERE rewound_by_trace_id IS NOT NULL;")?;

        // --- audit_event_summaries ---
        tx.execute_batch(
            "CREATE TABLE audit_event_summaries_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                summary TEXT NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                UNIQUE (agent_id, year, month)
            );
            INSERT INTO audit_event_summaries_new SELECT id, agent_id, year, month,
                summary, event_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM audit_event_summaries;
            DROP TABLE audit_event_summaries;
            ALTER TABLE audit_event_summaries_new RENAME TO audit_event_summaries;",
        )?;

        // --- search_content ---
        tx.execute_batch(
            "CREATE TABLE search_content_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_id INTEGER,
                content TEXT NOT NULL,
                embedding_json TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO search_content_new SELECT id, agent_id, source_type, source_id, content,
                embedding_json,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM search_content;
            DROP TABLE search_content;
            ALTER TABLE search_content_new RENAME TO search_content;
            CREATE INDEX idx_search_agent ON search_content(agent_id, source_type);",
        )?;

        // --- team_workspace ---
        tx.execute_batch(
            "CREATE TABLE team_workspace_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL REFERENCES team_runs(id) ON DELETE CASCADE,
                parent_id INTEGER REFERENCES team_workspace_new(id),
                agent_name TEXT,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                trace_id TEXT,
                iteration INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO team_workspace_new SELECT id, run_id, parent_id, agent_name,
                entry_type, content, trace_id, iteration,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM team_workspace;
            DROP TABLE team_workspace;
            ALTER TABLE team_workspace_new RENAME TO team_workspace;
            CREATE INDEX idx_team_ws_run ON team_workspace(run_id, created_at);
            CREATE INDEX idx_team_ws_trace ON team_workspace(trace_id) WHERE trace_id IS NOT NULL;",
        )?;

        // --- heartbeat_sends ---
        tx.execute_batch(
            "CREATE TABLE heartbeat_sends_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                sent_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO heartbeat_sends_new SELECT id, agent_id,
                strftime('%Y-%m-%dT%H:%M:%SZ', sent_at, 'unixepoch')
            FROM heartbeat_sends;
            DROP TABLE heartbeat_sends;
            ALTER TABLE heartbeat_sends_new RENAME TO heartbeat_sends;
            CREATE INDEX idx_heartbeat_agent ON heartbeat_sends(agent_id, sent_at DESC);",
        )?;

        // --- reflection_runs ---
        tx.execute_batch(
            "CREATE TABLE reflection_runs_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                changes_made INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO reflection_runs_new SELECT id, agent_id, status, changes_made, summary,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM reflection_runs;
            DROP TABLE reflection_runs;
            ALTER TABLE reflection_runs_new RENAME TO reflection_runs;
            CREATE INDEX idx_reflect_agent ON reflection_runs(agent_id, created_at DESC);",
        )?;

        // --- customer_config ---
        tx.execute_batch(
            "CREATE TABLE customer_config_new (
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                key TEXT NOT NULL COLLATE NOCASE,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (agent_id, key)
            );
            INSERT INTO customer_config_new SELECT agent_id, key, value,
                strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch')
            FROM customer_config;
            DROP TABLE customer_config;
            ALTER TABLE customer_config_new RENAME TO customer_config;",
        )?;

        // --- failed_sends ---
        tx.execute_batch(
            "CREATE TABLE failed_sends_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                request_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO failed_sends_new SELECT id, agent_id, text, request_id, retry_count,
                strftime('%Y-%m-%dT%H:%M:%SZ', created_at, 'unixepoch')
            FROM failed_sends;
            DROP TABLE failed_sends;
            ALTER TABLE failed_sends_new RENAME TO failed_sends;",
        )?;

        // --- schema_version ---
        tx.execute_batch(
            "CREATE TABLE schema_version_new (
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            INSERT INTO schema_version_new SELECT version,
                strftime('%Y-%m-%dT%H:%M:%SZ', applied_at, 'unixepoch')
            FROM schema_version;
            DROP TABLE schema_version;
            ALTER TABLE schema_version_new RENAME TO schema_version;",
        )?;

        // Recreate unified_timeline VIEW
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        // Record migration
        tx.execute("INSERT INTO schema_version (version) VALUES (12)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v12_to_v13(&mut self) -> Result<()> {
        info!("migrating database schema v12 → v13 (A2A orthogonal persistence)");

        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Drop view first — it references the tasks table we're about to rebuild
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;

        // Rebuild tasks table to add 'a2a' to trigger_type CHECK constraint
        tx.execute_batch(
            "CREATE TABLE tasks_new (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                team_run_id TEXT REFERENCES team_runs(id) ON DELETE SET NULL,
                parent_task_id TEXT REFERENCES tasks_new(id) ON DELETE SET NULL,
                depth INTEGER NOT NULL DEFAULT 0 CHECK (depth BETWEEN 0 AND 3),
                label TEXT NOT NULL,
                trigger_type TEXT NOT NULL CHECK (
                    trigger_type IN ('time','recurring','callback','user_reply','event','condition','manual','a2a')
                ),
                cron_expr TEXT,
                event_source TEXT,
                event_offset_secs INTEGER,
                condition_expr TEXT,
                next_fire_at TEXT,
                timeout_at TEXT,
                action_type TEXT NOT NULL CHECK (
                    action_type IN (
                        'send_message','resume_agent','inject_context',
                        'run_skill','invoke_orchestrator','none'
                    )
                ),
                action_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending' CHECK (
                    status IN ('pending','in_progress','completed','failed',
                               'cancelled','expired','recurring_active','delivered','blocked')
                ),
                process_id INTEGER,
                input_context TEXT,
                result TEXT,
                reference_url TEXT,
                source TEXT,
                created_by_session TEXT,
                created_trace_id TEXT,
                execution_trace_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                fired_at TEXT,
                completed_at TEXT
            );
            INSERT INTO tasks_new SELECT * FROM tasks;
            DROP TABLE tasks;
            ALTER TABLE tasks_new RENAME TO tasks;
            CREATE INDEX idx_tasks_agent_status ON tasks(agent_id, status);
            CREATE INDEX idx_tasks_next_fire ON tasks(next_fire_at)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_schedulable
                ON tasks(agent_id, next_fire_at ASC)
                WHERE status IN ('pending','recurring_active');
            CREATE INDEX idx_tasks_parent ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
            CREATE INDEX idx_tasks_session ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
            CREATE INDEX idx_tasks_trace ON tasks(created_trace_id) WHERE created_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_exec_trace ON tasks(execution_trace_id) WHERE execution_trace_id IS NOT NULL;
            CREATE INDEX idx_tasks_manual_active
                ON tasks(agent_id, created_at DESC)
                WHERE trigger_type = 'manual'
                AND status IN ('pending', 'in_progress', 'blocked');
            CREATE UNIQUE INDEX idx_tasks_unique_recurring
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE trigger_type = 'recurring'
                AND status NOT IN ('cancelled', 'failed', 'expired', 'delivered');
            CREATE UNIQUE INDEX idx_tasks_unique_reminder
                ON tasks(agent_id, label COLLATE NOCASE)
                WHERE status IN ('pending', 'in_progress', 'recurring_active')
                AND (action_type = 'send_message' OR action_type = 'resume_agent')
                AND trigger_type NOT IN ('callback');")?;

        // Create thin mapping table
        tx.execute_batch(
            "CREATE TABLE a2a_task_map (
                a2a_task_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                context_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX idx_a2a_task_map_task ON a2a_task_map(task_id);",
        )?;

        // Create A2A tables with FK to mapping table
        tx.execute_batch(
            "CREATE TABLE a2a_artifacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                artifact_id TEXT NOT NULL,
                name TEXT,
                description TEXT,
                parts TEXT NOT NULL,
                metadata TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE TABLE a2a_push_notification_configs (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES a2a_task_map(a2a_task_id) ON DELETE CASCADE,
                url TEXT NOT NULL,
                token TEXT,
                auth_scheme TEXT,
                auth_credentials TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );",
        )?;

        // Recreate unified_timeline VIEW (tasks table was rebuilt)
        tx.execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        tx.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        tx.execute("INSERT INTO schema_version (version) VALUES (13)", [])?;

        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(())
    }

    fn migrate_v13_to_v14(&self) -> Result<()> {
        info!("migrating database schema v13 → v14 (task metadata column)");

        // Simple ALTER TABLE — SQLite supports adding nullable columns without table rebuild.
        // Idempotent: check if column already exists before adding.
        if !self.column_exists("tasks", "metadata")? {
            self.conn
                .execute_batch("ALTER TABLE tasks ADD COLUMN metadata TEXT;")?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (14)", [])?;

        Ok(())
    }

    fn migrate_v14_to_v15(&self) -> Result<()> {
        info!("migrating database schema v14 → v15 (llm_calls + tool_calls tables)");

        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS llm_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                stop_reason TEXT,
                status TEXT NOT NULL DEFAULT 'success',
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_llm_calls_trace ON llm_calls(trace_id);
            CREATE INDEX IF NOT EXISTS idx_llm_calls_session ON llm_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_llm_calls_agent_created ON llm_calls(agent_id, created_at);

            CREATE TABLE IF NOT EXISTS tool_calls (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                trace_id TEXT,
                llm_call_id TEXT,
                step INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                tool_source TEXT NOT NULL DEFAULT 'builtin',
                skill_name TEXT,
                input TEXT,
                output TEXT,
                success INTEGER NOT NULL DEFAULT 1,
                non_zero_exit INTEGER NOT NULL DEFAULT 0,
                latency_ms INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_tool_calls_trace ON tool_calls(trace_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_llm_call ON tool_calls(llm_call_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_agent_created ON tool_calls(agent_id, created_at);",
        )?;

        // Recreate unified_timeline VIEW with new UNION ALL legs
        self.conn
            .execute_batch("DROP VIEW IF EXISTS unified_timeline;")?;
        self.conn.execute_batch(UNIFIED_TIMELINE_VIEW_SQL)?;

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (15)", [])?;

        Ok(())
    }

    fn migrate_v15_to_v16(&self) -> Result<()> {
        info!("migrating database schema v15 → v16 (add step column to llm_calls)");

        if !self.column_exists("llm_calls", "step")? {
            self.conn.execute_batch(
                "ALTER TABLE llm_calls ADD COLUMN step INTEGER NOT NULL DEFAULT 0;",
            )?;
        }

        self.conn
            .execute("INSERT INTO schema_version (version) VALUES (16)", [])?;

        Ok(())
    }

    fn migrate_v16_to_v17(&mut self) -> Result<()> {
        info!("migrating database schema v16 → v17 (add task dedup index on reference_url)");

        // Wrap in transaction for atomicity (matches pattern in migrate_v7_to_v8, etc.)
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "-- Step 1: Cancel duplicate active tasks with the same (agent_id, reference_url).
             -- Keep the earliest-created item per group (by rowid for deterministic tiebreaking
             -- when created_at timestamps collide), cancel the rest with a metadata breadcrumb.
             UPDATE tasks SET status = 'cancelled',
                 metadata = json_set(COALESCE(metadata, '{}'), '$.cancelled_reason', 'dedup_migration_v17')
             WHERE rowid IN (
                 SELECT t.rowid FROM tasks t
                 INNER JOIN (
                     SELECT agent_id, reference_url, MIN(rowid) as keeper_rowid
                     FROM tasks
                     WHERE trigger_type = 'manual'
                       AND reference_url IS NOT NULL
                       AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
                     GROUP BY agent_id, reference_url
                     HAVING COUNT(*) > 1
                 ) dups ON t.agent_id = dups.agent_id
                        AND t.reference_url = dups.reference_url
                        AND t.rowid != dups.keeper_rowid
                 WHERE t.trigger_type = 'manual'
                   AND t.status NOT IN ('completed', 'cancelled', 'failed', 'delivered')
             );

             -- Step 2: Create partial unique index. NULLs are exempt (SQLite skips NULL in
             -- unique indexes), so label-only dedup is handled at the tool level.
             CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_manual_active_ref_url
             ON tasks(agent_id, reference_url)
             WHERE trigger_type = 'manual'
               AND reference_url IS NOT NULL
               AND status NOT IN ('completed', 'cancelled', 'failed', 'delivered');

             INSERT INTO schema_version (version) VALUES (17);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v17_to_v18(&mut self) -> Result<()> {
        info!("migrating database schema v17 → v18 (widen reminder dedup index for resume_agent)");

        // The old index only covered action_type = 'send_message'. The new index covers
        // both 'send_message' and 'resume_agent' to prevent duplicate reminders regardless
        // of action type. See #363.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_tasks_unique_reminder;
             CREATE UNIQUE INDEX idx_tasks_unique_reminder
             ON tasks(agent_id, label COLLATE NOCASE)
             WHERE status IN ('pending', 'in_progress', 'recurring_active')
               AND (action_type = 'send_message' OR action_type = 'resume_agent')
               AND trigger_type NOT IN ('callback');

             INSERT INTO schema_version (version) VALUES (18);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v18_to_v19(&mut self) -> Result<()> {
        info!("migrating database schema v18 → v19 (add task_id to sessions for reverse lookup)");

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "ALTER TABLE sessions ADD COLUMN task_id TEXT;

             CREATE INDEX idx_sessions_task_id ON sessions(task_id) WHERE task_id IS NOT NULL;

             -- Backfill from existing metadata JSON (cli --task-id sessions)
             UPDATE sessions SET task_id = json_extract(metadata, '$.task_id')
               WHERE json_extract(metadata, '$.task_id') IS NOT NULL AND task_id IS NULL;

             INSERT INTO schema_version (version) VALUES (19);",
        )?;
        tx.commit()?;

        Ok(())
    }

    fn migrate_v19_to_v20(&mut self) -> Result<()> {
        info!("migrating database schema v19 → v20 (skill_overrides: llm_provider, llm_model)");

        // Idempotent: skip ALTER if columns already exist (defensive — re-runs).
        let has_provider = self.column_exists("skill_overrides", "llm_provider")?;
        let has_model = self.column_exists("skill_overrides", "llm_model")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_provider {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN llm_provider TEXT;\n");
        }
        if !has_model {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN llm_model TEXT;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (20);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v20_to_v21(&mut self) -> Result<()> {
        info!("migrating database schema v20 → v21 (llm_calls: prompt_variant)");

        let has_col = self.column_exists("llm_calls", "prompt_variant")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE llm_calls ADD COLUMN prompt_variant TEXT;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (21);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v21_to_v22(&mut self) -> Result<()> {
        info!("migrating database schema v21 → v22 (messages: internal flag)");

        let has_col = self.column_exists("messages", "internal")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE messages ADD COLUMN internal INTEGER NOT NULL DEFAULT 0;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (22);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v22_to_v23(&mut self) -> Result<()> {
        info!("migrating database schema v22 → v23 (tasks: type column)");

        let has_col = self.column_exists("tasks", "type")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            // SQLite 3.37+ supports CHECK constraints in ALTER TABLE ADD COLUMN.
            // The DEFAULT backfills all existing rows to 'issue', preserving behavior.
            sql.push_str(
                "ALTER TABLE tasks ADD COLUMN type TEXT NOT NULL DEFAULT 'issue' \
                 CHECK (type IN ('issue', 'milestone', 'project'));\n",
            );
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (23);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_v23_to_v24(&mut self) -> Result<()> {
        info!("migrating database schema v23 → v24 (skill_overrides: enabled column)");

        let has_col = self.column_exists("skill_overrides", "enabled")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sql = String::new();
        if !has_col {
            sql.push_str("ALTER TABLE skill_overrides ADD COLUMN enabled INTEGER;\n");
        }
        sql.push_str("INSERT INTO schema_version (version) VALUES (24);");

        tx.execute_batch(&sql)?;
        tx.commit()?;
        Ok(())
    }

    /// Migration v24 -> v25: Knowledge Graph schema tables.
    ///
    /// Adds 10 tables for the three-layer KG:
    /// - Domain layer: `kg_entities`, `kg_relationships`
    /// - Lexical layer: `kg_chunks`
    /// - Subject layer: `kg_subject_entities`, `kg_subject_resolutions`,
    ///   `kg_subject_relationships`
    /// - Provenance: `kg_chunk_subjects`, `kg_chunk_subject_relationships`
    /// - Tracking: `kg_extractions`, `kg_resolutions_log`
    fn migrate_v24_to_v25(&mut self) -> Result<()> {
        info!("migrating database schema v24 -> v25 (knowledge graph tables)");

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
                "-- KG domain layer (global, no agent_id)
                CREATE TABLE IF NOT EXISTS kg_entities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    entity_key TEXT NOT NULL UNIQUE,
                    type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    CHECK (entity_key = type || ':' || name)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_entities_type ON kg_entities(type);

                CREATE TABLE IF NOT EXISTS kg_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    from_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    to_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    type TEXT NOT NULL,
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                );
                CREATE INDEX IF NOT EXISTS idx_kg_rel_from ON kg_relationships(from_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_rel_to ON kg_relationships(to_entity_id, type);

                -- KG lexical layer (per-agent)
                CREATE TABLE IF NOT EXISTS kg_chunks (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    seq_id INTEGER NOT NULL,
                    source_doc_path TEXT NOT NULL,
                    source_doc_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, source_doc_path, seq_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_chunks_agent_doc ON kg_chunks(agent_id, source_doc_path);

                -- KG subject layer (per-agent)
                CREATE TABLE IF NOT EXISTS kg_subject_entities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    entity_key TEXT NOT NULL,
                    type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    CHECK (entity_key = type || ':' || name),
                    UNIQUE (agent_id, entity_key)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_subj_entities_agent_type ON kg_subject_entities(agent_id, type);

                CREATE TABLE IF NOT EXISTS kg_subject_resolutions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, subject_entity_id, domain_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

                -- KG subject-to-subject edges / fact triples (per-agent)
                CREATE TABLE IF NOT EXISTS kg_subject_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    type TEXT NOT NULL,
                    confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                    properties_json TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    trace_id TEXT,
                    UNIQUE (agent_id, from_entity_id, to_entity_id, type)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_from ON kg_subject_relationships(agent_id, from_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_to ON kg_subject_relationships(agent_id, to_entity_id, type);
                CREATE INDEX IF NOT EXISTS idx_kg_subj_rel_type ON kg_subject_relationships(agent_id, type);

                -- KG entity provenance: chunk -> subject entity (many-to-many)
                CREATE TABLE IF NOT EXISTS kg_chunk_subjects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, chunk_id, subject_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_cs_chunk ON kg_chunk_subjects(agent_id, chunk_id);
                CREATE INDEX IF NOT EXISTS idx_kg_cs_entity ON kg_chunk_subjects(agent_id, subject_entity_id);
                CREATE INDEX IF NOT EXISTS idx_kg_cs_trace ON kg_chunk_subjects(agent_id, extraction_trace_id);

                -- KG relationship provenance: chunk -> subject relationship (many-to-many)
                CREATE TABLE IF NOT EXISTS kg_chunk_subject_relationships (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                    subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, chunk_id, subject_relationship_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_csr_chunk ON kg_chunk_subject_relationships(agent_id, chunk_id);
                CREATE INDEX IF NOT EXISTS idx_kg_csr_rel ON kg_chunk_subject_relationships(agent_id, subject_relationship_id);

                -- KG extraction tracking.
                -- Historical shape as shipped at v25. `source_doc_hash` is
                -- added at v26 via ALTER TABLE in migrate_v25_to_v26 (#757);
                -- keeping migrate_v24_to_v25 as the record of v25's actual
                -- schema preserves migration immutability and means the
                -- convergence test exercises the ALTER path rather than
                -- skipping it.
                CREATE TABLE IF NOT EXISTS kg_extractions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    source_doc_path TEXT NOT NULL,
                    extraction_model TEXT NOT NULL,
                    entities_extracted INTEGER NOT NULL DEFAULT 0,
                    relationships_extracted INTEGER NOT NULL DEFAULT 0,
                    extraction_trace_id TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, source_doc_path)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_extractions_agent ON kg_extractions(agent_id);

                -- KG resolution tracking
                CREATE TABLE IF NOT EXISTS kg_resolutions_log (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                    subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                    outcome TEXT NOT NULL CHECK (outcome IN (
                        'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                    )),
                    resolution_trace_id TEXT NOT NULL,
                    source_extraction_trace_id TEXT,
                    model TEXT,
                    duration_ms INTEGER,
                    resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                    UNIQUE (agent_id, subject_entity_id)
                );
                CREATE INDEX IF NOT EXISTS idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

                INSERT INTO schema_version (version) VALUES (25);",
            )
            .context("failed to migrate v24 -> v25 (knowledge graph tables)")?;
        tx.commit()?;

        Ok(())
    }

    /// v25 -> v26: add nullable `source_doc_hash` column to `kg_extractions`
    /// so the pending-doc query can skip re-extraction when chunk content is
    /// unchanged (#757). Pre-existing rows get NULL and will re-extract once
    /// on the next run (bounded by MIKA_KG_BATCH_BUDGET), then populate the
    /// hash on success so subsequent runs are no-ops.
    ///
    /// Idempotent at the migration-chain level (the version gate in
    /// `run_migrations` prevents re-run); `column_exists` guards the inner
    /// ALTER against manual invocation.
    fn migrate_v25_to_v26(&mut self) -> Result<()> {
        info!("migrating database schema v25 -> v26 (kg_extractions.source_doc_hash)");

        if !self.column_exists("kg_extractions", "source_doc_hash")? {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "ALTER TABLE kg_extractions ADD COLUMN source_doc_hash TEXT;
                 INSERT INTO schema_version (version) VALUES (26);",
            )
            .context("failed to migrate v25 -> v26 (add kg_extractions.source_doc_hash)")?;
            tx.commit()?;
        } else {
            // Column already exists (manual re-run in a test / recovery scenario).
            // Wrap in TransactionBehavior::Immediate to match the true-branch envelope
            // (mika#1391): bare INSERT could leave column-exists + schema_version-not-bumped
            // inconsistent state on failure mid-recovery.
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO schema_version (version) VALUES (26)", [])?;
            tx.commit()?;
        }

        Ok(())
    }

    /// v26 -> v27: Schema v27 — docs_root_hash as shared-corpus primary key (#786 + #787).
    ///
    /// Two-phase migration in a single transaction:
    /// 1. **DDL** (#786): Renames six shared-layer KG tables to `*_v26_backup`,
    ///    creates fresh v27 tables keyed by `docs_root_hash` instead of `agent_id`.
    ///    Rebuilds per-agent tables to fix FK refs.
    /// 2. **Coalesce** (#787): Reads from backup tables, deduplicates across agents
    ///    via majority-vote (normalized entity_key, agent-count tiebreak), rewires
    ///    FKs via temp lookup tables, drops backups, writes `v27_coalesce_complete`
    ///    marker to `schema_meta`. `docs_root` resolved from `MIKA_KG_DOCS_ROOT`
    ///    env var or CWD fallback.
    fn migrate_v26_to_v27(&mut self) -> Result<()> {
        info!("migrating database schema v26 -> v27 (docs_root_hash shared-corpus)");

        // Idempotency guard: if kg_chunks already has docs_root_hash, we've run.
        if self.column_exists("kg_chunks", "docs_root_hash")? {
            // Already upgraded — just record the version bump.
            self.conn
                .execute("INSERT INTO schema_version (version) VALUES (27)", [])?;
            return Ok(());
        }

        // Resolve docs_root for v26 data coalescing.
        let docs_root_path = std::env::var("MIKA_KG_DOCS_ROOT")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("docs")
                    .join("solutions")
            });
        let docs_root_escaped = docs_root_path.to_string_lossy().replace('\'', "''");
        let docs_root_hash = crate::kg::config::hash_docs_root(&docs_root_path);

        info!(
            docs_root = %docs_root_path.display(),
            docs_root_hash = %docs_root_hash,
            "v26->v27 coalesce: resolved docs_root for migration"
        );

        // Log pre-coalesce counts from v26 tables (before DDL renames them).
        let pre_chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_entities: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_entities", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_relationships: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_relationships", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let pre_extractions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_extractions", [], |r| r.get(0))
            .unwrap_or(0);
        let pre_resolutions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_resolutions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let pre_res_log: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            pre_chunks,
            pre_entities,
            pre_relationships,
            pre_extractions,
            pre_resolutions,
            pre_res_log,
            "v26->v27 coalesce: pre-migration row counts"
        );

        // Generate the coalesce SQL for the resolved docs_root.
        let coalesce = v27_coalesce_sql(&docs_root_escaped, &docs_root_hash);

        // Build the full migration: DDL (rename to backup + create v27 tables) +
        // coalesce (read from backups, dedup, write to v27, drop backups) + finalize.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let sql = format!(
            "-- Create schema_meta table for migration state tracking.
                 CREATE TABLE IF NOT EXISTS schema_meta (
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );

                 -- Drop v25/v26 indexes that will be recreated (SQLite keeps
                 -- index names across ALTER TABLE RENAME, so they'd conflict).
                 DROP INDEX IF EXISTS idx_kg_chunks_agent_doc;
                 DROP INDEX IF EXISTS idx_kg_subj_entities_agent_type;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_from;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_to;
                 DROP INDEX IF EXISTS idx_kg_subj_rel_type;
                 DROP INDEX IF EXISTS idx_kg_cs_chunk;
                 DROP INDEX IF EXISTS idx_kg_cs_entity;
                 DROP INDEX IF EXISTS idx_kg_cs_trace;
                 DROP INDEX IF EXISTS idx_kg_csr_chunk;
                 DROP INDEX IF EXISTS idx_kg_csr_rel;
                 DROP INDEX IF EXISTS idx_kg_extractions_agent;
                 DROP INDEX IF EXISTS idx_kg_resolutions_agent_subj;
                 DROP INDEX IF EXISTS idx_kg_resolutions_agent_dom;
                 DROP INDEX IF EXISTS idx_kg_res_log_pending;

                 -- 1. kg_chunks: rename to backup, create v27 table.
                 ALTER TABLE kg_chunks RENAME TO kg_chunks_v26_backup;
                 CREATE TABLE kg_chunks (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     seq_id INTEGER NOT NULL,
                     source_doc_path TEXT NOT NULL,
                     source_doc_hash TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (docs_root_hash, source_doc_path, seq_id)
                 );
                 CREATE INDEX idx_kg_chunks_docs_root_hash_doc ON kg_chunks(docs_root_hash, source_doc_path);

                 -- 2. kg_subject_entities: rename to backup, create v27 table.
                 ALTER TABLE kg_subject_entities RENAME TO kg_subject_entities_v26_backup;
                 CREATE TABLE kg_subject_entities (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     entity_key TEXT NOT NULL,
                     type TEXT NOT NULL,
                     name TEXT NOT NULL,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     properties_json TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     CHECK (entity_key = type || ':' || name),
                     UNIQUE (docs_root_hash, entity_key)
                 );
                 CREATE INDEX idx_kg_subj_entities_drh_type ON kg_subject_entities(docs_root_hash, type);

                 -- 3. kg_subject_relationships: rename to backup, create v27 table.
                 ALTER TABLE kg_subject_relationships RENAME TO kg_subject_relationships_v26_backup;
                 CREATE TABLE kg_subject_relationships (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     from_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     to_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     type TEXT NOT NULL,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     properties_json TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (docs_root_hash, from_entity_id, to_entity_id, type)
                 );
                 CREATE INDEX idx_kg_subj_rel_from ON kg_subject_relationships(docs_root_hash, from_entity_id, type);
                 CREATE INDEX idx_kg_subj_rel_to ON kg_subject_relationships(docs_root_hash, to_entity_id, type);
                 CREATE INDEX idx_kg_subj_rel_type ON kg_subject_relationships(docs_root_hash, type);

                 -- 4. kg_chunk_subjects: rename to backup, create v27 table.
                 ALTER TABLE kg_chunk_subjects RENAME TO kg_chunk_subjects_v26_backup;
                 CREATE TABLE kg_chunk_subjects (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, chunk_id, subject_entity_id)
                 );
                 CREATE INDEX idx_kg_cs_chunk ON kg_chunk_subjects(docs_root_hash, chunk_id);
                 CREATE INDEX idx_kg_cs_entity ON kg_chunk_subjects(docs_root_hash, subject_entity_id);
                 CREATE INDEX idx_kg_cs_trace ON kg_chunk_subjects(docs_root_hash, extraction_trace_id);

                 -- 5. kg_chunk_subject_relationships: rename to backup, create v27 table.
                 ALTER TABLE kg_chunk_subject_relationships RENAME TO kg_chunk_subject_relationships_v26_backup;
                 CREATE TABLE kg_chunk_subject_relationships (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     chunk_id INTEGER NOT NULL REFERENCES kg_chunks(id) ON DELETE CASCADE,
                     subject_relationship_id INTEGER NOT NULL REFERENCES kg_subject_relationships(id) ON DELETE CASCADE,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, chunk_id, subject_relationship_id)
                 );
                 CREATE INDEX idx_kg_csr_chunk ON kg_chunk_subject_relationships(docs_root_hash, chunk_id);
                 CREATE INDEX idx_kg_csr_rel ON kg_chunk_subject_relationships(docs_root_hash, subject_relationship_id);

                 -- 6. kg_extractions: rename to backup, create v27 table.
                 ALTER TABLE kg_extractions RENAME TO kg_extractions_v26_backup;
                 CREATE TABLE kg_extractions (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     docs_root_hash TEXT NOT NULL,
                     docs_root TEXT NOT NULL,
                     source_doc_path TEXT NOT NULL,
                     source_doc_hash TEXT,
                     extraction_model TEXT NOT NULL,
                     entities_extracted INTEGER NOT NULL DEFAULT 0,
                     relationships_extracted INTEGER NOT NULL DEFAULT 0,
                     extraction_trace_id TEXT,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (docs_root_hash, source_doc_path)
                 );
                 CREATE INDEX idx_kg_extractions_drh ON kg_extractions(docs_root_hash);

                 -- 7. kg_subject_resolutions: rebuild to fix FK refs broken by
                 -- kg_subject_entities rename (SQLite rewrites FK targets on
                 -- ALTER TABLE RENAME).
                 ALTER TABLE kg_subject_resolutions RENAME TO kg_subject_resolutions_v26_backup;
                 CREATE TABLE kg_subject_resolutions (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     domain_entity_id INTEGER NOT NULL REFERENCES kg_entities(id) ON DELETE CASCADE,
                     confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     trace_id TEXT,
                     UNIQUE (agent_id, subject_entity_id, domain_entity_id)
                 );
                 CREATE INDEX idx_kg_resolutions_agent_subj ON kg_subject_resolutions(agent_id, subject_entity_id);
                 CREATE INDEX idx_kg_resolutions_agent_dom ON kg_subject_resolutions(agent_id, domain_entity_id);

                 -- 8. kg_resolutions_log: rebuild to fix FK refs broken by
                 -- kg_subject_entities rename.
                 ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v26_backup;
                 CREATE TABLE kg_resolutions_log (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                     subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                     outcome TEXT NOT NULL CHECK (outcome IN (
                         'matched_exact', 'matched_llm', 'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                     )),
                     resolution_trace_id TEXT NOT NULL,
                     source_extraction_trace_id TEXT,
                     model TEXT,
                     duration_ms INTEGER,
                     resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                     UNIQUE (agent_id, subject_entity_id)
                 );
                 CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

                 -- v27 coalesce: read from backup tables, dedup, write to v27 tables.
                 {coalesce}

                 INSERT INTO schema_version (version) VALUES (27);"
        );

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(&sql)
            .context("failed to migrate v26 -> v27 (docs_root_hash shared-corpus)")?;
        tx.commit()?;
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        // Log post-coalesce counts from the new v27 tables.
        let post_chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_chunks", [], |r| r.get(0))
            .unwrap_or(0);
        let post_entities: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_entities", [], |r| r.get(0))
            .unwrap_or(0);
        let post_relationships: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_relationships", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let post_extractions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_extractions", [], |r| r.get(0))
            .unwrap_or(0);
        let post_resolutions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_subject_resolutions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let post_res_log: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            post_chunks,
            post_entities,
            post_relationships,
            post_extractions,
            post_resolutions,
            post_res_log,
            chunks_deduped = pre_chunks - post_chunks,
            entities_deduped = pre_entities - post_entities,
            relationships_deduped = pre_relationships - post_relationships,
            extractions_deduped = pre_extractions - post_extractions,
            "v26->v27 coalesce: migration complete"
        );

        if pre_chunks > 0 && post_chunks == 0 {
            warn!("v26->v27 coalesce: all chunks were lost — this may indicate a migration bug");
        }

        Ok(())
    }

    /// v27→v28: Add `agent_kg_corpora` table for multi-corpus per-agent KG (#798).
    /// Maps `agent_id → {docs_root_hash, docs_root_path}` so the query path knows
    /// which corpora to fan out across without re-deriving from identity.
    /// Backfills from existing `kg_subject_resolutions → kg_subject_entities` joins.
    fn migrate_v27_to_v28(&mut self) -> Result<()> {
        // Idempotency: skip if table already exists.
        let exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_kg_corpora'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if exists {
            self.conn.execute(
                "UPDATE schema_version SET version = 28 WHERE version < 28",
                [],
            )?;
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE agent_kg_corpora (
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 docs_root_hash TEXT NOT NULL,
                 docs_root_path TEXT NOT NULL,
                 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 PRIMARY KEY (agent_id, docs_root_hash)
             );

             CREATE INDEX idx_agent_kg_corpora_hash ON agent_kg_corpora(docs_root_hash);

             INSERT OR IGNORE INTO agent_kg_corpora (agent_id, docs_root_hash, docs_root_path)
                 SELECT DISTINCT r.agent_id, e.docs_root_hash, e.docs_root
                 FROM kg_subject_resolutions r
                 JOIN kg_subject_entities e ON e.id = r.subject_entity_id
                 WHERE e.docs_root_hash IS NOT NULL AND e.docs_root IS NOT NULL;

             UPDATE schema_version SET version = 28;",
        )?;
        tx.commit()?;

        Ok(())
    }

    /// Backfill migration: scrub secret-shaped values from existing tool_calls
    /// rows (#908). Data-only — no DDL changes. Applies `scrub_secrets()` to
    /// `input` and `output` columns, updating only rows that change.
    fn migrate_v28_to_v29(&mut self) -> Result<()> {
        use crate::secret_scrubber::scrub_secrets;

        let version = self.schema_version()?;
        if version >= 29 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Collect IDs and content of rows that have non-NULL text fields.
        let mut stmt = tx.prepare(
            "SELECT id, input, output, error_message FROM tool_calls
             WHERE input IS NOT NULL OR output IS NOT NULL OR error_message IS NOT NULL",
        )?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, Option<String>, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut updated = 0u64;
        for (id, input, output, error_msg) in &rows {
            let scrubbed_input = input.as_deref().map(scrub_secrets);
            let scrubbed_output = output.as_deref().map(scrub_secrets);
            let scrubbed_error = error_msg.as_deref().map(scrub_secrets);

            // Only UPDATE if scrubbing changed at least one field.
            let input_changed = matches!(&scrubbed_input, Some(std::borrow::Cow::Owned(_)));
            let output_changed = matches!(&scrubbed_output, Some(std::borrow::Cow::Owned(_)));
            let error_changed = matches!(&scrubbed_error, Some(std::borrow::Cow::Owned(_)));

            if input_changed || output_changed || error_changed {
                tx.execute(
                    "UPDATE tool_calls SET input = ?1, output = ?2, error_message = ?3 WHERE id = ?4",
                    params![
                        scrubbed_input.as_deref(),
                        scrubbed_output.as_deref(),
                        scrubbed_error.as_deref(),
                        id,
                    ],
                )?;
                updated += 1;
            }
        }

        tx.execute("UPDATE schema_version SET version = 29", [])?;
        tx.commit()?;

        if updated > 0 {
            info!(
                updated_rows = updated,
                total_rows = rows.len(),
                "v28→v29: scrubbed secrets from existing tool_calls rows"
            );
        }

        Ok(())
    }

    /// v29→v30: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'matched_llm_db_fallback'` (#874). Table rebuild mirroring the v26→v27
    /// shape: RENAME → CREATE → INSERT INTO ... SELECT → DROP → recreate index.
    fn migrate_v29_to_v30(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 30 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v29_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'skipped_discovered_type', 'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v29_backup;

             DROP TABLE kg_resolutions_log_v29_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 30;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v29→v30: expanded kg_resolutions_log outcome CHECK constraint (#874)"
        );

        Ok(())
    }

    fn migrate_v30_to_v31(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 31 {
            return Ok(());
        }

        // Columns may already exist in clean-slate DBs; check each independently
        // to handle partial-crash recovery scenarios.
        let has_response_text = self.column_exists("llm_calls", "response_text")?;
        let has_reasoning = self.column_exists("llm_calls", "reasoning")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_response_text {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN response_text TEXT;")?;
        }
        if !has_reasoning {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN reasoning TEXT;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (31)", [])?;
        tx.commit()?;

        info!("v30→v31: added response_text and reasoning columns to llm_calls (#653)");

        Ok(())
    }

    /// v31→v32: Add `kg_invalidated_no_match` sidecar table (#961).
    ///
    /// Ephemeral marker table for tracking entities whose `no_match` resolution
    /// log rows were deleted by domain-graph rebuild invalidation (#960).
    /// The resolver reads and cleans up these markers during resolution.
    fn migrate_v31_to_v32(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 32 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS kg_invalidated_no_match (
                subject_entity_id INTEGER NOT NULL,
                agent_id TEXT NOT NULL,
                invalidated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (subject_entity_id, agent_id)
            );",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (32)", [])?;
        tx.commit()?;

        info!("v31→v32: added kg_invalidated_no_match sidecar table (#961)");

        Ok(())
    }

    fn migrate_v32_to_v33(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 33 {
            return Ok(());
        }

        // #1052: Delete kg_extractions rows with NULL source_doc_hash.
        // These are pre-v26 rows that escaped the v27 backfill due to
        // NULL = NULL being falsy in SQL. They create a deadlock: the
        // pending query says "extract me" but INSERT OR IGNORE (now
        // replaced with upsert in #1052) would skip them. Deleting makes
        // them cleanly pending for re-extraction with the new upsert.
        // Safe because kg_extractions is an idempotency marker, not data.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted: usize = tx.execute(
            "DELETE FROM kg_extractions WHERE source_doc_hash IS NULL",
            [],
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (33)", [])?;
        tx.commit()?;

        if deleted > 0 {
            info!(
                deleted = deleted,
                "v32→v33: deleted {deleted} NULL-hash kg_extractions rows (#1052)"
            );
        } else {
            info!("v32→v33: no NULL-hash kg_extractions rows to clean up (#1052)");
        }

        Ok(())
    }

    fn migrate_v33_to_v34(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 34 {
            return Ok(());
        }

        // #1001: Add dispatch_class column for per-class dispatch slot split.
        // Nullable — pre-v34 rows stay NULL, treated as 'implement' via COALESCE
        // in the dispatch guard query. CHECK constraint limits values.
        // Column-exists guard for crash-recovery and convergence-test safety.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_col: bool = tx
            .prepare("PRAGMA table_info(tasks)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "dispatch_class");
        if !has_col {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN dispatch_class TEXT
                   CHECK (dispatch_class IS NULL OR dispatch_class IN ('implement', 'groom'));",
            )?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_tasks_dispatch_class
               ON tasks(agent_id, dispatch_class, status)
               WHERE dispatch_class IS NOT NULL;",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (34)", [])?;
        tx.commit()?;

        info!("v33→v34: added dispatch_class column to tasks (#1001)");
        Ok(())
    }

    /// v34→v35: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'no_candidate_of_type'` (#1154). Table rebuild mirroring the v29→v30
    /// shape: RENAME → CREATE → INSERT INTO ... SELECT → DROP → recreate index.
    fn migrate_v34_to_v35(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 35 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v34_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'no_candidate_of_type',
                     'skipped_discovered_type', 'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v34_backup;

             DROP TABLE kg_resolutions_log_v34_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 35;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v34→v35: expanded kg_resolutions_log outcome CHECK to include 'no_candidate_of_type' (#1154)"
        );

        Ok(())
    }

    /// v35→v36: Add `discovered` and `discovery_reason` columns to
    /// `kg_subject_entities` for roster-grounding (#1158).
    fn migrate_v35_to_v36(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 36 {
            return Ok(());
        }

        // Column-exists guards for crash-recovery safety (per v30→v31 precedent).
        let has_discovered = self.column_exists("kg_subject_entities", "discovered")?;
        let has_discovery_reason = self.column_exists("kg_subject_entities", "discovery_reason")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if !has_discovered {
            tx.execute(
                "ALTER TABLE kg_subject_entities ADD COLUMN discovered INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !has_discovery_reason {
            tx.execute(
                "ALTER TABLE kg_subject_entities ADD COLUMN discovery_reason TEXT",
                [],
            )?;
        }

        tx.execute("UPDATE schema_version SET version = 36", [])?;
        tx.commit()?;

        info!(
            "v35→v36: added discovered + discovery_reason columns to kg_subject_entities (#1158)"
        );

        Ok(())
    }

    /// v36→v37: Expand `kg_resolutions_log.outcome` CHECK constraint to include
    /// `'skipped_discovered_subject'` (#1158). Table rebuild.
    fn migrate_v36_to_v37(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 37 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             ALTER TABLE kg_resolutions_log RENAME TO kg_resolutions_log_v36_backup;

             CREATE TABLE kg_resolutions_log (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                 subject_entity_id INTEGER NOT NULL REFERENCES kg_subject_entities(id) ON DELETE CASCADE,
                 outcome TEXT NOT NULL CHECK (outcome IN (
                     'matched_exact', 'matched_llm', 'matched_llm_db_fallback',
                     'no_match', 'no_candidate_of_type',
                     'skipped_discovered_type', 'skipped_discovered_subject',
                     'skipped_no_llm', 'error'
                 )),
                 resolution_trace_id TEXT NOT NULL,
                 source_extraction_trace_id TEXT,
                 model TEXT,
                 duration_ms INTEGER,
                 resolved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE (agent_id, subject_entity_id)
             );

             INSERT INTO kg_resolutions_log
                 (id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                  source_extraction_trace_id, model, duration_ms, resolved_at)
             SELECT id, agent_id, subject_entity_id, outcome, resolution_trace_id,
                    source_extraction_trace_id, model, duration_ms, resolved_at
             FROM kg_resolutions_log_v36_backup;

             DROP TABLE kg_resolutions_log_v36_backup;

             CREATE INDEX idx_kg_res_log_pending ON kg_resolutions_log(agent_id, outcome);

             PRAGMA foreign_keys = ON;

             UPDATE schema_version SET version = 37;",
        )?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM kg_resolutions_log", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v36→v37: expanded kg_resolutions_log outcome CHECK to include 'skipped_discovered_subject' (#1158)"
        );

        Ok(())
    }

    /// v37→v38: Add `system_prompt_bytes` column to `llm_calls` (mika#1217).
    ///
    /// Per-call assembled-system-prompt byte count for context-budget
    /// observability. Nullable; pre-v38 rows stay NULL. Mirrors v30→v31's
    /// additive-nullable shape and the `column_exists` guard pattern.
    fn migrate_v37_to_v38(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 38 {
            return Ok(());
        }

        let has_column = self.column_exists("llm_calls", "system_prompt_bytes")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_column {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN system_prompt_bytes INTEGER;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (38)", [])?;
        tx.commit()?;

        info!("v37→v38: added system_prompt_bytes column to llm_calls (mika#1217)");

        Ok(())
    }

    /// v38→v39: Add `operational_items` table (mika#1262).
    ///
    /// Canonical operational-item ledger for the What's Next engine.
    /// New table with indexes, no existing table changes.
    fn migrate_v38_to_v39(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 39 {
            return Ok(());
        }

        let has_table: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='operational_items'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if !has_table {
            tx.execute_batch(
                "CREATE TABLE operational_items (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL CHECK (kind IN ('goal', 'task', 'commitment', 'decision', 'blocker', 'evidence', 'next_action')),
                    title TEXT NOT NULL,
                    status TEXT NOT NULL CHECK (status IN ('now', 'waiting', 'delegated', 'scheduled', 'at_risk', 'done')),
                    owner_type TEXT NOT NULL CHECK (owner_type IN ('user', 'mika', 'person', 'agent')),
                    owner_name TEXT,
                    priority REAL NOT NULL DEFAULT 0.0,
                    user_importance REAL NOT NULL DEFAULT 0.0,
                    due_at TEXT,
                    blocked_by TEXT,
                    next_action TEXT,
                    evidence_refs TEXT NOT NULL DEFAULT '[]',
                    confidence REAL NOT NULL DEFAULT 1.0,
                    source_table TEXT,
                    source_id TEXT,
                    agent_id TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE INDEX idx_operational_items_agent_status ON operational_items(agent_id, status);
                CREATE INDEX idx_operational_items_agent_kind ON operational_items(agent_id, kind);
                CREATE INDEX idx_operational_items_agent_priority ON operational_items(agent_id, priority DESC);
                CREATE INDEX idx_operational_items_source ON operational_items(source_table, source_id);
                CREATE UNIQUE INDEX idx_operational_items_source_unique
                    ON operational_items(agent_id, source_table, source_id)
                    WHERE source_table IS NOT NULL AND source_id IS NOT NULL;",
            )?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (39)", [])?;
        tx.commit()?;

        info!("v38→v39: added operational_items table (mika#1262)");

        Ok(())
    }

    /// v39→v40: Delete all mika-relay agent data (mika#1193).
    ///
    /// Self-contained: explicit deletes in reverse-dependency order. Correctness
    /// does NOT depend on PRAGMA foreign_keys being ON.
    fn migrate_v39_to_v40(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 40 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Delete in reverse-dependency order: descendants before ancestors.
        // Each statement is idempotent (no-op if rows are already gone).
        tx.execute_batch(
            "-- v40: mika#1193 retire mika-relay agent.
            DELETE FROM tool_calls
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM llm_calls
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM messages
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM skill_overrides
              WHERE agent_id = 'mika-relay';

            DELETE FROM audit_events
              WHERE session_id IN (SELECT id FROM sessions WHERE agent_id = 'mika-relay');

            DELETE FROM operational_items
              WHERE agent_id = 'mika-relay';

            DELETE FROM tasks
              WHERE agent_id = 'mika-relay';

            DELETE FROM sessions
              WHERE agent_id = 'mika-relay';

            DELETE FROM agents
              WHERE id = 'mika-relay';",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (40)", [])?;
        tx.commit()?;

        info!("v39→v40: deleted mika-relay agent data (mika#1193)");

        Ok(())
    }

    /// v40→v41: Add `task_messages` parallel narrative table (mika#974).
    ///
    /// Additive — no existing table altered, no data touched. Safe on live DB.
    fn migrate_v40_to_v41(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 41 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v41: mika#974 task_messages parallel narrative table.
            CREATE TABLE IF NOT EXISTS task_messages (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                agent_id   TEXT NOT NULL,
                session_id TEXT NOT NULL,
                role       TEXT NOT NULL,
                content    TEXT NOT NULL,
                metadata   TEXT,
                trace_id   TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE INDEX IF NOT EXISTS idx_task_messages_task_created
                ON task_messages (task_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_task_messages_agent_created
                ON task_messages (agent_id, created_at);

            INSERT INTO schema_version (version) VALUES (41);",
        )?;

        tx.commit()?;

        info!("v40→v41: added task_messages table (mika#974)");

        Ok(())
    }

    /// v41→v42: Add `auto_pull_stats` circuit-breaker tracking table (mika#1363).
    fn migrate_v41_to_v42(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 42 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v42: mika#1363 auto-pull circuit-breaker stats table.
            CREATE TABLE IF NOT EXISTS auto_pull_stats (
                repo_full_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                last_auto_pull_at TEXT,
                last_failure_at TEXT,
                PRIMARY KEY (repo_full_name, issue_number)
            );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (42)", [])?;
        tx.commit()?;

        info!("v41→v42: created auto_pull_stats table (mika#1363)");

        Ok(())
    }

    /// v42→v43: Add `lifecycle_state`, `use_count`, `last_used_at` columns to
    /// `skill_overrides` for curator background task (mika#1584).
    fn migrate_v42_to_v43(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 43 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Additive ALTER TABLE with column_exists guard for crash-recovery safety.
        if !Self::column_exists_tx(&tx, "skill_overrides", "lifecycle_state")? {
            tx.execute_batch(
                "ALTER TABLE skill_overrides ADD COLUMN lifecycle_state TEXT
                 CHECK (lifecycle_state IN ('staged', 'active', 'archived'));",
            )?;
        }
        if !Self::column_exists_tx(&tx, "skill_overrides", "use_count")? {
            tx.execute_batch(
                "ALTER TABLE skill_overrides ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        if !Self::column_exists_tx(&tx, "skill_overrides", "last_used_at")? {
            tx.execute_batch("ALTER TABLE skill_overrides ADD COLUMN last_used_at TEXT;")?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (43)", [])?;
        tx.commit()?;

        info!(
            "v42→v43: added lifecycle_state, use_count, last_used_at to skill_overrides (mika#1584)"
        );

        Ok(())
    }

    /// v43→v44: additive `permission_decisions` provenance ledger (mika#1733 AC4).
    ///
    /// Records every operator permission decision routed through
    /// `PermissionsChannel::resolve_decision`, including the classifier
    /// verdict, operator ratification, derived `override_used` flag, and the
    /// scope (tenant/agent) at decision time. Additive-only — no rebuild of
    /// existing tables. Two indexes support the two expected query shapes:
    /// per-request lookup and time-window scans.
    fn migrate_v43_to_v44(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 44 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS permission_decisions (
                id TEXT PRIMARY KEY,
                request_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                args_summary TEXT,
                classifier_verdict TEXT NOT NULL
                    CHECK (classifier_verdict IN ('approved', 'denied', 'held')),
                operator_decision TEXT
                    CHECK (operator_decision IN ('approve', 'deny')),
                override_used INTEGER NOT NULL DEFAULT 0
                    CHECK (override_used IN (0, 1)),
                decision_authority TEXT NOT NULL
                    CHECK (decision_authority IN ('strict', 'override')),
                tenant_id TEXT,
                agent_id TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_permission_decisions_request_id
                ON permission_decisions(request_id);
            CREATE INDEX IF NOT EXISTS idx_permission_decisions_created_at
                ON permission_decisions(created_at DESC);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (44)", [])?;
        tx.commit()?;

        info!("v43→v44: added permission_decisions provenance table (mika#1733 AC4)");

        Ok(())
    }

    /// v48→v49: expand the `team_runs.status` CHECK constraint to include
    /// `'failed_transport'` (mika#1671 D3). Composes on top of v46→v47's
    /// `'failed_no_delegation'` addition (mika#1676) — both terminal states
    /// cover different failure classes (all-transport-failed short-circuit vs
    /// zero-delegation gate) and coexist in the CHECK; the D3 architect pin
    /// explicitly says they're orthogonal.
    ///
    /// SQLite cannot alter a CHECK in place, so this is a table rebuild.
    /// `team_runs` is FK-referenced by `tasks.team_run_id`, so the rebuild uses
    /// the **build-new-then-swap** shape (CREATE `team_runs_new` →
    /// INSERT SELECT → DROP `team_runs` → RENAME `team_runs_new` → `team_runs`),
    /// NOT the rename-to-backup shape used by v34→v35. Renaming the *referenced*
    /// table first would make SQLite (with the default `legacy_alter_table = OFF`)
    /// rewrite `tasks.team_run_id`'s FK target to the backup name, leaving a
    /// dangling reference after the backup is dropped (caught by
    /// `test_v1_and_incremental_schemas_converge`). Renaming the *new* table into
    /// place instead leaves `tasks`'s existing `team_runs` reference untouched.
    /// Symmetric to v46→v47's shape. `foreign_keys` is toggled OFF around the
    /// rebuild so the DROP/RENAME does not trip FK enforcement. Carries forward
    /// the v46→v47 columns (`delegation_count`, `solo_absorption`,
    /// `failure_context`) untouched. Row count is preserved.
    fn migrate_v48_to_v49(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 49 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        // PRAGMA foreign_keys is a no-op inside a transaction, so it is set on
        // the connection before BEGIN and restored after COMMIT.
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE team_runs_new (
                 id TEXT PRIMARY KEY,
                 team_id TEXT NOT NULL REFERENCES teams(id),
                 goal TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'running'
                     CHECK (status IN (
                         'running','completed','failed','cancelled','suspended',
                         'failed_no_delegation','failed_transport'
                     )),
                 failure_reason TEXT,
                 iteration INTEGER NOT NULL DEFAULT 1,
                 max_iterations INTEGER NOT NULL DEFAULT 3,
                 deliverable TEXT,
                 checkpoint TEXT,
                 trace_id TEXT,
                 started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 ended_at TEXT,
                 delegation_count INTEGER NOT NULL DEFAULT 0,
                 solo_absorption INTEGER NOT NULL DEFAULT 0,
                 failure_context TEXT
             );

             INSERT INTO team_runs_new
                 (id, team_id, goal, status, failure_reason,
                  iteration, max_iterations, deliverable, checkpoint,
                  trace_id, started_at, ended_at,
                  delegation_count, solo_absorption, failure_context)
             SELECT id, team_id, goal, status, failure_reason,
                    iteration, max_iterations, deliverable, checkpoint,
                    trace_id, started_at, ended_at,
                    delegation_count, solo_absorption, failure_context
             FROM team_runs;

             DROP TABLE team_runs;

             ALTER TABLE team_runs_new RENAME TO team_runs;

             CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (49)", [])?;
        tx.commit()?;

        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v48→v49: expanded team_runs.status CHECK to include 'failed_transport' (mika#1671)"
        );

        Ok(())
    }

    /// v49→v50: additive re-drive accounting on `auto_pull_stats` (mika#2020).
    ///
    /// `failure_count` means "the `gh` call failed" and is reset on **every**
    /// successful Phase 2 rescue and Phase 1 promotion. A re-drive counter has
    /// to increment on exactly that event. The two semantics are opposed at the
    /// same point in the code, which is why mika#1901 could be re-driven 16
    /// times in 19 h without the circuit breaker ever seeing it: each rescue
    /// succeeded at the API, so each rescue zeroed the only counter that
    /// existed. Three additive columns, no table rebuild:
    ///
    /// - `redrive_count` — successful Phase 2 re-drives since the last observed
    ///   progress (an open PR closing the ticket, or an in-flight self_dev task).
    /// - `last_redrive_at` — timestamp of the most recent re-drive.
    /// - `redrive_abandoned_at` — set when the budget is exhausted and the
    ///   ticket is handed to the operator. Its presence is what distinguishes
    ///   "the budget just ran out, the label is not posted yet" from "the budget
    ///   ran out earlier and the operator has since removed the label", which is
    ///   the re-entry gesture.
    fn migrate_v49_to_v50(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 50 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v50: mika#2020 per-ticket re-drive budget + named abandonment.
            ALTER TABLE auto_pull_stats ADD COLUMN redrive_count INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE auto_pull_stats ADD COLUMN last_redrive_at TEXT;
            ALTER TABLE auto_pull_stats ADD COLUMN redrive_abandoned_at TEXT;",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (50)", [])?;
        tx.commit()?;

        info!("v49→v50: added re-drive accounting to auto_pull_stats (mika#2020)");

        Ok(())
    }

    /// v50 → v51 (mika#1948, Porte 2) — three-dispatcher exec-slot arbitration.
    ///
    /// Two additions, one axis each:
    ///
    /// 1. `tasks.dispatcher_source` — WHICH dispatcher inside this engine
    ///    initiated a task (`mika_dev` | `mika_manager` | `operator`). Nullable:
    ///    pre-v51 rows are all mika-dev by definition (the autonomous loop was
    ///    the only dispatcher), so NULL is an unambiguous "pre-v51, therefore
    ///    mika_dev" sentinel read through `COALESCE(dispatcher_source,
    ///    'mika_dev')`. That keeps the migration O(1) instead of rewriting every
    ///    existing row, and preserves the forensic distinction between "pre-v51
    ///    row" and "post-v51 row explicitly written by mika-dev".
    ///
    /// 2. `dispatch_slot_leases` — makes an assigned exec slot an OBSERVABLE
    ///    FACT rather than a convention. See `try_acquire_dispatch_slot` for the
    ///    full reasoning; the short version is that
    ///    `has_active_callback_tasks_excluding` only ever *checked* the slot, and
    ///    the row that makes it held is written seconds later, so two dispatchers
    ///    could both read "free" and both proceed.
    ///
    /// NOTE: this is a DIFFERENT axis from the `dispatch:*` seat label (mika#2084,
    /// `webhook_dispatch::CURRENT_DISPATCH_SEAT`). The seat says which *engine*
    /// owns a TICKET and is carried on the GitHub issue; `dispatcher_source` says
    /// which *role inside this engine* initiated a TASK. The seat gate refuses a
    /// ticket that belongs to another engine; this arbitrates the exec slot among
    /// the dispatchers of one engine. Neither subsumes the other, and they are
    /// deliberately not merged into one column.
    // FIXME(mika#1948-AC10): once this PR merges, update
    // `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`
    // § 3 Porte 2 to `**Statut : DISCHARGED**`, naming this ticket and PR. That
    // file lives in the meta-repo, outside this worktree, so it cannot be
    // touched here — this marker is the searchable reminder that survives the
    // squash. Remove it in the follow-up commit that lands the doc update.
    fn migrate_v50_to_v51(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 51 {
            return Ok(());
        }
        // Fail fast on an unexpected baseline. The idempotency guard above
        // already returned for >=51, so reaching here with anything but v50
        // means the migration-order assumption is broken — applying anyway
        // would corrupt the chain silently.
        if version != 50 {
            anyhow::bail!(
                "migrate_v50_to_v51 called with unexpected baseline version {version} \
                 (expected 50) — refusing to apply migration; investigate migration order"
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // The column may already exist if a previous run was interrupted
        // between the ALTER and the version bump; PRAGMA-detect, do not assume.
        let has_col: bool = {
            let mut stmt = tx.prepare("PRAGMA table_info(tasks)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            names.iter().any(|n| n == "dispatcher_source")
        };
        if !has_col {
            tx.execute_batch(
                "ALTER TABLE tasks ADD COLUMN dispatcher_source TEXT CHECK (
                     dispatcher_source IS NULL
                     OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                 );",
            )?;
        }

        tx.execute_batch(
            "-- v51: mika#1948 Porte 2 — 3-dispatcher exec-slot arbitration.
             CREATE INDEX IF NOT EXISTS idx_tasks_dispatcher_source
                 ON tasks(agent_id, dispatcher_source, status)
                 WHERE dispatcher_source IS NOT NULL;

             -- One row per (agent, class) = one exec slot. The PRIMARY KEY is
             -- what makes the claim atomic: a second claimant's INSERT collides
             -- instead of racing a SELECT.
             CREATE TABLE IF NOT EXISTS dispatch_slot_leases (
                 agent_id TEXT NOT NULL,
                 dispatch_class TEXT NOT NULL,
                 holder_task_id TEXT NOT NULL,
                 dispatcher_source TEXT CHECK (
                     dispatcher_source IS NULL
                     OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                 ),
                 acquired_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL,
                 PRIMARY KEY (agent_id, dispatch_class)
             );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (51)", [])?;
        tx.commit()?;

        info!("v50→v51: added tasks.dispatcher_source + dispatch_slot_leases (mika#1948 Porte 2)");

        Ok(())
    }

    /// v51 → v52 (mika#2160) — the exec-slot lease stops being a hard cap of one.
    ///
    /// `dispatch_slot_leases` carried `PRIMARY KEY (agent_id, dispatch_class)`,
    /// which is itself a cap of one independent of any predicate: a class
    /// cannot hold two live leases whatever the TTL. Adding `slot_index` to the
    /// key is what makes a configurable cap real rather than decorative — see
    /// KTD1 in the mika#2160 plan, and the test that asserts two acquisitions.
    ///
    /// The migration is a table rebuild (SQLite cannot extend a PRIMARY KEY in
    /// place). Existing rows land at `slot_index = 0`, so a database migrated
    /// mid-dispatch keeps its live lease and the holder that owns it.
    ///
    /// At the default cap of 1 exactly one index is ever written and the
    /// behaviour is the pre-v52 behaviour, bit for bit.
    fn migrate_v51_to_v52(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 52 {
            return Ok(());
        }
        if version != 51 {
            anyhow::bail!(
                "migrate_v51_to_v52 called with unexpected baseline version {version} \
                 (expected 51) — refusing to apply migration; investigate migration order"
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        // The column may already exist if a previous run was interrupted
        // between the rebuild and the version bump; PRAGMA-detect, do not assume.
        let has_col: bool = {
            let mut stmt = tx.prepare("PRAGMA table_info(dispatch_slot_leases)")?;
            let names: Vec<String> = stmt
                .query_map([], |r| r.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            names.iter().any(|n| n == "slot_index")
        };

        if !has_col {
            tx.execute_batch(
                "-- v52: mika#2160 — slot_index joins the lease key.
                 CREATE TABLE dispatch_slot_leases_v52 (
                     agent_id TEXT NOT NULL,
                     dispatch_class TEXT NOT NULL,
                     slot_index INTEGER NOT NULL DEFAULT 0,
                     holder_task_id TEXT NOT NULL,
                     dispatcher_source TEXT CHECK (
                         dispatcher_source IS NULL
                         OR dispatcher_source IN ('mika_dev', 'mika_manager', 'operator')
                     ),
                     acquired_at TEXT NOT NULL,
                     expires_at TEXT NOT NULL,
                     PRIMARY KEY (agent_id, dispatch_class, slot_index)
                 );

                 INSERT INTO dispatch_slot_leases_v52
                     (agent_id, dispatch_class, slot_index, holder_task_id,
                      dispatcher_source, acquired_at, expires_at)
                 SELECT agent_id, dispatch_class, 0, holder_task_id,
                        dispatcher_source, acquired_at, expires_at
                 FROM dispatch_slot_leases;

                 DROP TABLE dispatch_slot_leases;
                 ALTER TABLE dispatch_slot_leases_v52 RENAME TO dispatch_slot_leases;",
            )?;
        }

        tx.execute("INSERT INTO schema_version (version) VALUES (52)", [])?;
        tx.commit()?;

        info!("v51→v52: dispatch_slot_leases gained slot_index in its key (mika#2160)");

        Ok(())
    }

    /// v52→v53: Add `request_bytes` column to `llm_calls` (mika#2189 D5/Q2).
    ///
    /// # The hole this closes
    ///
    /// mika#2189's AC1 asks for the expiry distribution "by brief size". On the
    /// **error** path `agent_loop` writes `input_tokens = 0` and
    /// `output_tokens = 0` as literals — a failed call carries no token count at
    /// all — so the size of the request that timed out was not recoverable after
    /// the fact. The measurement had to fall back to `system_prompt_bytes`
    /// (mika#1217), which discriminates well but is only half the payload.
    ///
    /// An estimated `input_tokens` was rejected in Q2: the axis exists to
    /// *correlate* size with expiry, and a correlation built on an estimate
    /// cannot settle anything. This is the measured byte length of the
    /// serialized request, written on **both** paths — write it on the success
    /// path only and the hole reopens on exactly the side that matters.
    ///
    /// Nullable and non-retroactive: pre-v53 rows stay NULL, and this does not
    /// recover the 209 failures that motivated the ticket. It makes the *next*
    /// measurement whole. Additive shape and `column_exists` guard mirror
    /// v37→v38, which added `system_prompt_bytes` for the same family of reason.
    fn migrate_v52_to_v53(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 53 {
            return Ok(());
        }

        let has_column = self.column_exists("llm_calls", "request_bytes")?;

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !has_column {
            tx.execute_batch("ALTER TABLE llm_calls ADD COLUMN request_bytes INTEGER;")?;
        }
        tx.execute("INSERT INTO schema_version (version) VALUES (53)", [])?;
        tx.commit()?;

        info!("v52→v53: added request_bytes column to llm_calls (mika#2189)");

        Ok(())
    }

    /// v53 → v54 (mika#2192) — a worktree stops being an unclaimed shared
    /// resource.
    ///
    /// `_set_up_worktree` derives `WORKTREE_DIR` from the branch and walks in.
    /// Between the derivation and the entry nothing reads any state: the path
    /// is a FUNCTION of the branch, never an allocation. Two actors on the same
    /// ticket therefore get the same directory by construction, and the first
    /// gesture on the resume path is `git stash push --include-untracked` — so
    /// a file someone else has just written disappears between two tool calls,
    /// silently, into a stash stack shared by every worktree of the repo.
    ///
    /// `dispatch_slot_leases` arbitrates the exec SLOT and does it correctly.
    /// It never claimed to arbitrate a DIRECTORY, and an orchestrator session
    /// does not appear in it at all — it is not a dispatch.
    ///
    /// Additive `CREATE TABLE`, nothing FK-references it, no existing row is
    /// read or rewritten. Renuméroté de v52→v53 à v53→v54 (mika#2202) : #2189
    /// a pris le slot v53 (colonne request_bytes) entre le grooming et le merge.
    ///
    /// **No `pid`, no `pgrep`, no `/proc`.** Liveness is `expires_at > now`, the
    /// exact property `try_acquire_dispatch_slot` already documents: a claimant
    /// that dies mid-flight blocks its ticket for at most one TTL, not forever.
    fn migrate_v53_to_v54(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 54 {
            return Ok(());
        }
        if version != 53 {
            anyhow::bail!(
                "migrate_v53_to_v54 called with unexpected baseline version {version} \
                 (expected 53) — refusing to apply migration; investigate migration order"
            );
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "-- v54: mika#2192 — worktree ownership registry.
             CREATE TABLE IF NOT EXISTS worktree_claims (
                 repo TEXT NOT NULL,
                 issue_number INTEGER NOT NULL,
                 owner_kind TEXT NOT NULL CHECK (
                     owner_kind IN ('pilot', 'orchestrator', 'spawn')
                 ),
                 owner_id TEXT NOT NULL,
                 owner_label TEXT,
                 claimed_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL,
                 PRIMARY KEY (repo, issue_number)
             );",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (54)", [])?;
        tx.commit()?;

        info!("v53→v54: added worktree_claims (mika#2192)");

        Ok(())
    }

    /// v44→v45: additive `pilot_transcripts` table (mika#1705).
    ///
    /// Captures the LLM-call transcripts emitted by claude-pilot subprocesses
    /// (the implementation-reasoning corpus, ~90% of the trajectory that the
    /// in-process `llm_calls` table never sees). Rows are ingested by the
    /// engine tick from `~/.mika/data/pilot-transcripts/<task-id>.jsonl` files
    /// written by claude-pilot-py and linked back to the dispatching callback
    /// `task_id`. Additive-only — no rebuild of existing tables. Two indexes
    /// support the two query shapes: per-task correlation and time-window
    /// retention scans.
    fn migrate_v44_to_v45(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 45 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS pilot_transcripts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                timestamp TEXT,
                provider TEXT,
                model TEXT,
                request_body TEXT,
                response_body TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                latency_ms INTEGER,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_pilot_transcripts_task_id
                ON pilot_transcripts(task_id);
            CREATE INDEX IF NOT EXISTS idx_pilot_transcripts_created_at
                ON pilot_transcripts(created_at DESC);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (45)", [])?;
        tx.commit()?;

        info!("v44→v45: added pilot_transcripts table (mika#1705)");

        Ok(())
    }

    /// v45→v46: additive `served_content` table (mika#1867).
    ///
    /// Per-(agent, person, category) ledger of content Mika has served —
    /// proverb, quote, joke, poem, recommendation, story, fact — so that
    /// re-generation on future turns can dedup against exact-match hashes.
    /// Founding incident: Al (Vietnam tester) 2026-07-28 — same zen proverb
    /// served twice, 6 days apart, because history-fetch is global-recency
    /// (not per-user) and there was no content-serve ledger.
    ///
    /// Backward compat: reads return empty for rows pre-migration (agent that
    /// has never served anything = no dedup, safe direction per AC1).
    fn migrate_v45_to_v46(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 46 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS served_content (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
                person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
                category TEXT NOT NULL CHECK (category IN (
                    'proverb','quote','joke','poem','recommendation','story','fact'
                )),
                content_text TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                -- Reserved for v2 fuzzy dedup (embedding cosine similarity per AC6).
                -- Format TBD — likely 384-dim float array as BLOB or hex string.
                content_signature TEXT,
                served_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                UNIQUE(agent_id, person_id, content_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_served_content_person_cat
                ON served_content(agent_id, person_id, category, served_at DESC);
            CREATE INDEX IF NOT EXISTS idx_served_content_hash
                ON served_content(agent_id, person_id, content_hash);",
        )?;

        tx.execute("INSERT INTO schema_version (version) VALUES (46)", [])?;
        tx.commit()?;

        info!("v45→v46: added served_content table (mika#1867)");

        Ok(())
    }

    /// v46→v47: team_runs delegation-visibility (mika#1676).
    ///
    /// Two coupled schema changes, one atomic table-rebuild:
    ///
    /// 1. **CHECK expansion** on `team_runs.status` to add
    ///    `'failed_no_delegation'` — the terminal state Unit A's delegation
    ///    gate transitions to when the orchestrator returns Conversational
    ///    for an actionable goal (after one reinforced retry). The v1 DDL
    ///    already declares the expanded set for fresh installs; this
    ///    migration lifts existing databases into parity.
    /// 2. **Additive columns** for Unit B observability:
    ///    - `delegation_count INTEGER NOT NULL DEFAULT 0` — incremented per
    ///      spawned member session in `execute_tasks()`.
    ///    - `solo_absorption INTEGER NOT NULL DEFAULT 0` — flag set by
    ///      `finalize_and_shutdown()` when the run completed with zero
    ///      delegations.
    ///    - `failure_context TEXT` — nullable JSON `{"phase": "…"}` carrying
    ///      the phase in which the delegation gate fired
    ///      (`first_decompose` / `revision_after_critic`).
    ///
    /// Table-rebuild is mandatory here because SQLite does not support
    /// `ALTER TABLE … MODIFY CONSTRAINT` and the CHECK constraint must widen.
    /// Follows the exact shape of `migrate_v34_to_v35` (kg_resolutions_log
    /// outcome CHECK expansion, #1154): rename existing table to `_v46_backup`,
    /// create new table with expanded CHECK + new columns, INSERT SELECT
    /// (backfilling new columns with their DEFAULTs), DROP backup, recreate
    /// index. `PRAGMA foreign_keys = OFF` for the rebuild window because
    /// `tasks.team_run_id` FKs into `team_runs(id)`; the RENAME/DROP would
    /// otherwise violate FK enforcement even though row IDs are preserved.
    fn migrate_v46_to_v47(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 47 {
            return Ok(());
        }

        let count_before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        // CREATE-INSERT-DROP-RENAME sequence (per the v11→v12 rebuild
        // precedent at `crates/mika-agent/src/db.rs:2696`), NOT the
        // RENAME-CREATE-INSERT-DROP sequence used by the v34→v35
        // kg_resolutions_log rebuild.
        //
        // Rationale — child-FK preservation:
        // Since SQLite 3.26 (2018), a plain `ALTER TABLE … RENAME TO …`
        // rewrites referring FK metadata to track the new name. That is
        // safe for `kg_resolutions_log` (no child table FKs into it), but
        // catastrophic here because `tasks.team_run_id REFERENCES team_runs(id)`.
        // A RENAME-first sequence would silently retarget the FK to
        // `team_runs_v46_backup`, then leave it pointing at a dropped
        // table — every subsequent `INSERT INTO tasks` would fail
        // "no such table: team_runs_v46_backup". `PRAGMA legacy_alter_table = ON`
        // does NOT prevent this rewrite reliably across sqlite versions.
        //
        // The CREATE-INSERT-DROP-RENAME sequence sidesteps the trap:
        // the OLD `team_runs` is dropped BEFORE the new table exists under
        // that name, and the RENAME then creates the target name fresh —
        // so `tasks.team_run_id` continues to name `team_runs` throughout
        // and resolves against the new table when FK checks re-enable.
        // (Symmetric bug would have corrupted every FK-user of team_runs
        // at first write post-deploy — caught by the
        // `test_migrate_v46_to_v47_*` tests and `test_v1_and_incremental_schemas_converge`.)
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "PRAGMA foreign_keys = OFF;

             CREATE TABLE team_runs_new (
                 id TEXT PRIMARY KEY,
                 team_id TEXT NOT NULL REFERENCES teams(id),
                 goal TEXT NOT NULL,
                 status TEXT NOT NULL DEFAULT 'running'
                     CHECK (status IN (
                         'running','completed','failed','cancelled','suspended',
                         'failed_no_delegation'
                     )),
                 failure_reason TEXT,
                 iteration INTEGER NOT NULL DEFAULT 1,
                 max_iterations INTEGER NOT NULL DEFAULT 3,
                 deliverable TEXT,
                 checkpoint TEXT,
                 trace_id TEXT,
                 started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 ended_at TEXT,
                 delegation_count INTEGER NOT NULL DEFAULT 0,
                 solo_absorption INTEGER NOT NULL DEFAULT 0,
                 failure_context TEXT
             );

             INSERT INTO team_runs_new
                 (id, team_id, goal, status, failure_reason,
                  iteration, max_iterations, deliverable, checkpoint,
                  trace_id, started_at, ended_at,
                  delegation_count, solo_absorption, failure_context)
             SELECT id, team_id, goal, status, failure_reason,
                    iteration, max_iterations, deliverable, checkpoint,
                    trace_id, started_at, ended_at,
                    0, 0, NULL
             FROM team_runs;

             DROP TABLE team_runs;

             ALTER TABLE team_runs_new RENAME TO team_runs;

             CREATE INDEX idx_team_runs_team ON team_runs(team_id, started_at DESC);

             PRAGMA foreign_keys = ON;",
        )?;
        tx.execute("INSERT INTO schema_version (version) VALUES (47)", [])?;
        tx.commit()?;

        let count_after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM team_runs", [], |r| r.get(0))
            .unwrap_or(0);

        info!(
            count_before = count_before,
            count_after = count_after,
            "v46→v47: expanded team_runs.status CHECK to include 'failed_no_delegation' + added delegation_count/solo_absorption/failure_context columns (mika#1676)"
        );

        Ok(())
    }

    /// v47→v48 (mika#1712): behavioral marker only — no DDL. Anchors the
    /// phantom NULL-PID sweep semantics added in
    /// [`super::task_engine::engine::TaskEngine::sweep_null_pid_phantoms`] so
    /// operators reading the migration ledger can pinpoint the schema head at
    /// which sweep telemetry began. Reserved for future DDL if write-time
    /// enforcement (mika#1934 cause-racine) ever lands.
    fn migrate_v47_to_v48(&mut self) -> Result<()> {
        let version = self.schema_version()?;
        if version >= 48 {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO schema_version (version) VALUES (48)", [])?;
        tx.commit()?;

        info!("v47→v48: no DDL; behavioral marker for mika#1712 phantom sweep");

        Ok(())
    }
}

#[cfg(test)]
mod tests;
