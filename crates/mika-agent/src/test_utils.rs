#[cfg(test)]
pub mod test_helpers {
    use crate::async_db::AsyncDatabase;
    use crate::db::Database;
    use crate::tools::ToolContext;
    use mika_common::config::Settings;
    use std::sync::atomic::{AtomicBool, AtomicU32};

    /// Installe un abonné de capture pour le thread courant (mika#2646).
    ///
    /// **Site unique volontairement.** Sept appels nus vivaient dans ce crate,
    /// dans quatre fichiers, et aucun ne ré-interrogeait le cache de callsites
    /// alors que le piège était documenté à la ligne près dans
    /// `mika-common/tests/llm_retry.rs`. Un geste qu'on peut oublier est un
    /// geste qu'on oublie ; celui-ci n'a plus de site où être oublié. Tenu par
    /// `mika2646_set_default_a_un_site_dinstallation_unique`.
    ///
    /// # Ce que `rebuild_interest_cache` achète ici — et ce qu'il n'achète PAS
    ///
    /// Le ticket, et `llm_retry.rs` avant lui, décrivent ce mécanisme : un
    /// callsite `tracing` met son `Interest` en cache **globalement**, décidé
    /// par le premier thread qui l'atteint ; dans un binaire de test c'est
    /// couramment un thread sans abonné, qui répond `never`, après quoi le
    /// test capturant ne voit plus rien.
    ///
    /// Le mécanisme existe, mais **pas dans la direction temporelle que ce
    /// remède suppose**, et c'est mesuré plutôt que déduit (mika#2646, AC3) :
    ///
    /// - Un empoisonnement **antérieur** à l'installation est **déjà réparé
    ///   par `set_default` lui-même** : `Dispatch::new` appelle
    ///   `callsite::register_dispatch`, qui appelle `CALLSITES.rebuild_interest`
    ///   (`tracing-core-0.1.36/src/{dispatcher.rs:479,callsite.rs:484-488}`).
    ///   Mesuré : empoisonner puis installer **sans** rebuild capture quand
    ///   même les 13 événements. L'appel explicite est donc **redondant** sur
    ///   ce chemin — il est conservé parce qu'il est inoffensif, qu'il aligne
    ///   ce crate sur `llm_retry.rs`, et qu'il reste correct si une version
    ///   future de `tracing-core` cesse de rebuilder à l'enregistrement.
    /// - La fenêtre qui **casse** réellement est **postérieure** à
    ///   l'installation : un callsite ne s'enregistre qu'à sa *première*
    ///   atteinte, et son `Interest` est alors décidé par `get_default` **du
    ///   thread qui l'enregistre** (`callsite.rs:236-253` → `rebuilder()` →
    ///   `JustOne` → `callsite.rs:562-567`). Un test voisin qui atteint le
    ///   callsite en premier, sans abonné, l'éteint globalement — y compris
    ///   pour un abonné déjà installé sur un autre thread. Mesuré : capture à
    ///   **0** dans cette configuration, et le rebuild appelé *après*
    ///   l'empoisonnement la ramène à 1.
    ///
    /// **C'est donc `#[serial_test::serial]` qui est la moitié porteuse**, et
    /// elle doit couvrir les **poisonneurs** autant que les capturants : tout
    /// test qui atteint un callsite capturé. Un capturant seul à être annoté
    /// laisse la course entière ouverte, `#[serial]` ne sérialisant que contre
    /// ses propres porteurs (`wip_rescue.rs:2289-2292`).
    ///
    /// Deux chemins de perte supplémentaires ont été écartés **par lecture**,
    /// pour que personne n'ait à les re-soupçonner : l'AND entre dispatchers
    /// rend `sometimes` et jamais `never` (`subscriber.rs:652-664`), donc il
    /// fait consulter `enabled()` par événement ; et `LevelFilter::set_max`
    /// prend le **max** des hints avec `unwrap_or(TRACE)`
    /// (`callsite.rs:407-422`), `NoSubscriber` ne surchargeant pas
    /// `max_level_hint`. Ni l'un ni l'autre ne peut éteindre une capture.
    ///
    /// Défaut fondateur : PR #2643, tête `26e70b43`, run `37037943870`, job
    /// « Test with telemetry feature », 2026-10-02 17:24Z.
    pub fn install_capturing_subscriber<S>(subscriber: S) -> tracing::subscriber::DefaultGuard
    where
        S: tracing::Subscriber + Send + Sync + 'static,
    {
        let guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        guard
    }

    /// Create an in-memory database for tests (sync — for db module tests).
    pub fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    /// Create an async database for tool/agent tests.
    /// Pre-creates a "test-session" session for agent "mika" so FK constraints are satisfied.
    pub fn test_async_db() -> AsyncDatabase {
        let db = Database::open_in_memory().unwrap();
        db.create_session("test-session", "mika", "cli").unwrap();
        AsyncDatabase::new(db)
    }

    /// Create a ToolContext for tests (non-onboarding).
    pub fn test_ctx<'a>(db: &'a AsyncDatabase, edit_count: &'a AtomicU32) -> ToolContext<'a> {
        test_ctx_with_onboarding(db, edit_count, false)
    }

    /// Create a ToolContext for tests with configurable onboarding flag.
    pub fn test_ctx_with_onboarding<'a>(
        db: &'a AsyncDatabase,
        edit_count: &'a AtomicU32,
        is_onboarding: bool,
    ) -> ToolContext<'a> {
        static HOME_DIR: &str = "/tmp/mika-test";
        static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
        static PR_REVIEW_POSTED: AtomicBool = AtomicBool::new(false);
        static TOOL_ARG_SUFFIX_REJECTED: AtomicBool = AtomicBool::new(false);
        ToolContext {
            db,
            session_id: "test-session",
            trace_id: "00000000000000000000000000000000",
            home_dir: std::path::Path::new(HOME_DIR),
            global_home_dir: None,
            core_memory_edit_count: edit_count,
            is_onboarding,
            message_sender: None,
            embedding_client: None,
            brave_api_key: None,
            github_token: None,
            gateway_url: None,
            internal_token: None,
            skills_dirty: &SKILLS_DIRTY,
            is_reflection: false,
            is_task_context: false,
            is_callback_turn: false,
            is_webhook_fallthrough_turn: false,
            provider_name: "anthropic",
            model_name: "claude-sonnet-4-6",
            active_skill_paths: &[],
            max_tasks_per_session: 25,
            pr_review_posted: &PR_REVIEW_POSTED,
            pr_reviews_posted: None,
            callback_task_id: None,
            required_tool_arg_suffixes: &[],
            tool_arg_suffix_rejected: &TOOL_ARG_SUFFIX_REJECTED,
            tier: mika_common::home::AgentTier::Default,
            deployment: mika_common::home::Deployment::Unknown,
            scope_task_id: None,
        }
    }

    /// Test harness that owns the database and edit counter, reducing
    /// boilerplate in tool tests. Use `harness.ctx()` to get a `ToolContext`
    /// and `harness.db` for direct database access during test setup.
    pub struct TestHarness {
        pub db: AsyncDatabase,
        pub counter: AtomicU32,
    }

    impl Default for TestHarness {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TestHarness {
        pub fn new() -> Self {
            Self {
                db: test_async_db(),
                counter: AtomicU32::new(0),
            }
        }

        /// Create a harness with a specific agent ID.
        pub fn with_agent(agent_id: &str) -> Self {
            let db = Database::open_in_memory().unwrap();
            // Ensure the agent exists (default "mika" is seeded by migrate)
            if agent_id != "mika" {
                db.register_agent(agent_id, agent_id, "").unwrap();
            }
            db.create_session("test-session", agent_id, "cli").unwrap();
            Self {
                db: AsyncDatabase::new_with_agent(db, agent_id),
                counter: AtomicU32::new(0),
            }
        }

        /// Create a non-onboarding ToolContext borrowing from this harness.
        pub fn ctx(&self) -> ToolContext<'_> {
            test_ctx(&self.db, &self.counter)
        }

        /// Create a ToolContext with configurable onboarding flag.
        pub fn ctx_with_onboarding(&self, is_onboarding: bool) -> ToolContext<'_> {
            test_ctx_with_onboarding(&self.db, &self.counter, is_onboarding)
        }

        /// Create a ToolContext with explicit provider/model overrides
        /// (mika#1815). Uses `'static` string literals so the returned
        /// context is 'static-safe for provider/model fields.
        pub fn ctx_with_llm(&self, provider: &'static str, model: &'static str) -> ToolContext<'_> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            static PR_REVIEW_POSTED: AtomicBool = AtomicBool::new(false);
            static TOOL_ARG_SUFFIX_REJECTED: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: std::path::Path::new("/tmp/mika-test"),
                global_home_dir: None,
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                gateway_url: None,
                internal_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: false,
                is_task_context: false,
                is_callback_turn: false,
                is_webhook_fallthrough_turn: false,
                provider_name: provider,
                model_name: model,
                active_skill_paths: &[],
                max_tasks_per_session: 25,
                pr_review_posted: &PR_REVIEW_POSTED,
                pr_reviews_posted: None,
                callback_task_id: None,
                required_tool_arg_suffixes: &[],
                tool_arg_suffix_rejected: &TOOL_ARG_SUFFIX_REJECTED,
                tier: mika_common::home::AgentTier::Default,
                deployment: mika_common::home::Deployment::Unknown,
                scope_task_id: None,
            }
        }

        /// Create a ToolContext in reflection mode.
        pub fn ctx_with_reflection(&self) -> ToolContext<'_> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            static PR_REVIEW_POSTED: AtomicBool = AtomicBool::new(false);
            static TOOL_ARG_SUFFIX_REJECTED: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: std::path::Path::new("/tmp/mika-test"),
                global_home_dir: None,
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                gateway_url: None,
                internal_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: true,
                is_task_context: false,
                is_callback_turn: false,
                is_webhook_fallthrough_turn: false,
                provider_name: "anthropic",
                model_name: "claude-sonnet-4-6",
                active_skill_paths: &[],
                max_tasks_per_session: 25,
                pr_review_posted: &PR_REVIEW_POSTED,
                pr_reviews_posted: None,
                callback_task_id: None,
                required_tool_arg_suffixes: &[],
                tool_arg_suffix_rejected: &TOOL_ARG_SUFFIX_REJECTED,
                tier: mika_common::home::AgentTier::Default,
                deployment: mika_common::home::Deployment::Unknown,
                scope_task_id: None,
            }
        }

        /// Create a ToolContext for a specific agent tier (mika#1783).
        /// Used by substrate-doctrine tests that need to exercise
        /// tier-conditional handler behavior.
        pub fn ctx_with_tier(&self, tier: mika_common::home::AgentTier) -> ToolContext<'_> {
            let mut ctx = self.ctx();
            ctx.tier = tier;
            ctx
        }

        /// Create a ToolContext for a specific agent tier with a `brave_api_key`
        /// slot (mika#1783). The key ref is scoped to the caller's lifetime.
        pub fn ctx_with_tier_and_brave<'a>(
            &'a self,
            tier: mika_common::home::AgentTier,
            brave_key: Option<&'a str>,
        ) -> ToolContext<'a> {
            let mut ctx = self.ctx();
            ctx.tier = tier;
            ctx.brave_api_key = brave_key;
            ctx
        }

        /// Create a ToolContext with a custom home directory.
        pub fn ctx_with_home<'a>(&'a self, home: &'a std::path::Path) -> ToolContext<'a> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            static PR_REVIEW_POSTED: AtomicBool = AtomicBool::new(false);
            static TOOL_ARG_SUFFIX_REJECTED: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: home,
                global_home_dir: None,
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                gateway_url: None,
                internal_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: false,
                is_task_context: false,
                is_callback_turn: false,
                is_webhook_fallthrough_turn: false,
                provider_name: "anthropic",
                model_name: "claude-sonnet-4-6",
                active_skill_paths: &[],
                max_tasks_per_session: 25,
                pr_review_posted: &PR_REVIEW_POSTED,
                pr_reviews_posted: None,
                callback_task_id: None,
                required_tool_arg_suffixes: &[],
                tool_arg_suffix_rejected: &TOOL_ARG_SUFFIX_REJECTED,
                tier: mika_common::home::AgentTier::Default,
                deployment: mika_common::home::Deployment::Unknown,
                scope_task_id: None,
            }
        }
        /// Create a ToolContext with custom home and global home directories.
        /// Used for testing cross-agent file access.
        pub fn ctx_with_home_and_global<'a>(
            &'a self,
            home: &'a std::path::Path,
            global: &'a std::path::Path,
        ) -> ToolContext<'a> {
            static SKILLS_DIRTY: AtomicBool = AtomicBool::new(false);
            static PR_REVIEW_POSTED: AtomicBool = AtomicBool::new(false);
            static TOOL_ARG_SUFFIX_REJECTED: AtomicBool = AtomicBool::new(false);
            ToolContext {
                db: &self.db,
                session_id: "test-session",
                trace_id: "00000000000000000000000000000000",
                home_dir: home,
                global_home_dir: Some(global),
                core_memory_edit_count: &self.counter,
                is_onboarding: false,
                message_sender: None,
                embedding_client: None,
                brave_api_key: None,
                github_token: None,
                gateway_url: None,
                internal_token: None,
                skills_dirty: &SKILLS_DIRTY,
                is_reflection: false,
                is_task_context: false,
                is_callback_turn: false,
                is_webhook_fallthrough_turn: false,
                provider_name: "anthropic",
                model_name: "claude-sonnet-4-6",
                active_skill_paths: &[],
                max_tasks_per_session: 25,
                pr_review_posted: &PR_REVIEW_POSTED,
                pr_reviews_posted: None,
                callback_task_id: None,
                required_tool_arg_suffixes: &[],
                tool_arg_suffix_rejected: &TOOL_ARG_SUFFIX_REJECTED,
                tier: mika_common::home::AgentTier::Default,
                deployment: mika_common::home::Deployment::Unknown,
                scope_task_id: None,
            }
        }
    }

    /// Create a manual task in the test DB and return its ID.
    pub async fn create_test_task(db: &crate::async_db::AsyncDatabase) -> String {
        use crate::db::NewTask;
        use crate::task_engine::types::{action_type, trigger_type};

        let task = NewTask {
            agent_id: db.agent_id().to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "test task".to_string(),
            trigger_type: trigger_type::MANUAL.to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: action_type::NONE.to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some("test-session".to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: None,
        };
        db.create_task(task).await.unwrap()
    }

    /// Minimal Settings for validation-only tests (no API key needed).
    ///
    /// Delegates to `Settings::test_defaults()` — the canonical test constructor
    /// in mika-common. This wrapper exists for backward compatibility with
    /// existing call sites in mika-agent unit tests.
    pub fn dummy_settings() -> Settings {
        Settings::test_defaults()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// **Livrée vide, et elle le reste.**
    ///
    /// Quand la garde ci-dessous tire, on **route le site** vers
    /// `test_helpers::install_capturing_subscriber` ; on ne l'allowliste pas.
    /// Une allowlist née vide est un emplacement où déposer la prochaine
    /// infraction (doctrine mika#2201, motif mika#2323).
    const SET_DEFAULT_SITES_ALLOWED: &[&str] = &[];

    /// Le seul site d'installation autorisé, relatif à `src/`.
    ///
    /// Nommé plutôt que recopié : le prédicat (l'exemption du scan) et la
    /// fixture du contrôle de bonne foi doivent désigner le **même** fichier,
    /// sans quoi le contrôle cesse d'attester le prédicat sans rien rougir.
    const INSTALLER_SITE: &str = "test_utils.rs";

    /// L'aiguille, assemblée par `concat!` plutôt qu'écrite d'un bloc.
    ///
    /// `concat!` produit le littéral à la compilation, donc la chaîne cherchée
    /// **n'apparaît nulle part dans ce fichier**. Écrite d'un bloc, elle ferait
    /// de la garde son propre second site : le scan n'exclut pas le code de
    /// test (voir [`all_sources`] pour pourquoi), donc il se compterait
    /// lui-même et serait rouge au jour de sa naissance — c'est-à-dire désarmé
    /// le lendemain.
    ///
    /// **Borne du prédicat, nommée plutôt que découverte.** Il cherche le
    /// chemin **pleinement qualifié**. `use tracing::subscriber::set_default;`
    /// reste attrapé par sa ligne `use`, mais une forme aliasée
    /// (`use tracing::subscriber as ts; ts::set_default(…)`) passerait. Aucune
    /// n'existe sous `src/` aujourd'hui ; c'est un faux négatif réparable en
    /// élargissant l'aiguille, pas un trou qu'une allowlist comblerait.
    fn needle() -> &'static str {
        concat!("tracing::subscriber::", "set_default")
    }

    fn src_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// Tous les `.rs` sous `crates/mika-agent/src`, **sans filtre de test**.
    ///
    /// PIÈGE, et c'est l'inverse de toutes les gardes voisines : celles-ci
    /// écartent le code de test (`source_scan::is_test_source_path`,
    /// `production_half`) parce qu'elles cherchent dans la production un motif
    /// que la production ne doit pas porter. **Ici les sept sites vivent tous
    /// dans du code de test.** Une garde bâtie sur `production_sources()` —
    /// ou sur `ProductionScanner`, qui masque les régions `cfg(test)` —
    /// trouverait **zéro** site et se lirait comme un arbre propre : la classe
    /// mika#2205 appliquée à la garde elle-même.
    ///
    /// L'énumération passe par [`mika_common::source_guard::rust_sources_under`],
    /// le lecteur unique que quinze gardes écrivaient à l'identique — c'est la
    /// règle que `source_scan` énonce pour lui-même (« un seul lecteur, pas une
    /// copie par garde », mika#2158). Il n'applique aucun filtre de test, ce
    /// qui est exactement ce qu'il faut ici, et il trie, ce qui rend l'ordre
    /// des fautifs déterministe d'une machine à l'autre.
    ///
    /// **Portée : `src/` seulement, et c'est une décision du plan.** Les neuf
    /// fichiers de `crates/mika-agent/tests/` qui installent un abonné sont des
    /// **binaires distincts** — un processus chacun, donc un cache d'`Interest`
    /// chacun — et sont hors de la population mesurée. Le silence du scan sur
    /// eux n'est pas une couverture.
    ///
    /// Un fichier illisible **panique** plutôt que d'être sauté : une garde qui
    /// saute un fichier en silence est une garde qui a cessé de regarder, et le
    /// fichier sauté peut être le fautif — l'assertion d'anti-vacuité compte
    /// les fichiers *après* le saut et ne le verrait pas.
    fn all_sources() -> Vec<(String, String)> {
        let root = src_root();
        mika_common::source_guard::rust_sources_under(&root)
            .into_iter()
            .map(|path| {
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("la garde doit pouvoir lire {} : {e}", path.display())
                });
                let rel = path
                    .strip_prefix(&root)
                    .expect("chemin sous src/")
                    .to_string_lossy()
                    .replace('\\', "/");
                (rel, content)
            })
            .collect()
    }

    /// Les fichiers portant l'aiguille, commentaires retirés.
    ///
    /// Le dépouillement est porteur : le doc-comment de l'installateur **cite**
    /// `tracing-core` et décrit le mécanisme, et une prose qui décrit le motif
    /// interdit n'en est pas une violation (faux positif mesuré sur le Signal S,
    /// mika#2050).
    ///
    /// Le court-circuit sur la source brute est **exactement** préservant, et
    /// la raison mérite d'être écrite pour que personne n'ait à la redériver :
    /// `strip_comment_lines` ne fait que **supprimer des lignes entières** et
    /// rejoindre par `"\n"`, donc toute sous-chaîne sans retour à la ligne de
    /// sa sortie est une sous-chaîne d'une ligne de l'entrée. L'aiguille n'en
    /// contient pas, donc `needle ∈ strip(s) ⟹ needle ∈ s` : le pré-filtre ne
    /// peut écarter que des fichiers qui n'auraient pas pu matcher. Il évite
    /// d'allouer une copie dépouillée des ~12 Mo de l'arbre pour trouver un
    /// seul fichier (mesuré : 87,5 ms → 2,3 ms).
    fn sites_carrying_the_needle(sources: &[(String, String)]) -> Vec<String> {
        sources
            .iter()
            .filter(|(_, src)| {
                src.contains(needle())
                    && crate::source_scan::strip_comment_lines(src).contains(needle())
            })
            .map(|(rel, _)| rel.clone())
            .collect()
    }

    /// **U3 — un seul site d'installation d'abonné de capture (mika#2646).**
    ///
    /// Aucun test comportemental ne peut voir cette classe : un huitième
    /// `set_default` nu ne rend **aucune** décision fausse le jour où il est
    /// écrit — la capture marche, toutes les assertions restent vertes, et
    /// seule la course se rouvre, en silence. C'est la classe
    /// `grooming_marker` (mika#2158).
    #[test]
    fn mika2646_set_default_a_un_site_dinstallation_unique() {
        let sources = all_sources();
        assert!(
            sources.len() > 100,
            "le scan n'a énuméré que {} fichiers — un scan qui ne scanne rien est \
             un laissez-passer vide, pas un arbre propre (mika#2103)",
            sources.len()
        );

        let sites = sites_carrying_the_needle(&sources);

        // ANTI-VACUITÉ. Un scan qui vise un nom mort (module renommé, helper
        // déplacé) rend zéro infraction et se lit exactement comme un arbre
        // sain — motif de la cardinalité de mika#2496 et du `!declared.is_empty()`
        // de mika#2201.
        assert!(
            sites.iter().any(|s| s == INSTALLER_SITE),
            "le site de l'installateur est introuvable : la garde vise un nom mort. \
             Sites vus : {sites:?}"
        );

        let offenders: Vec<&String> = sites
            .iter()
            .filter(|rel| rel.as_str() != INSTALLER_SITE)
            .filter(|rel| !SET_DEFAULT_SITES_ALLOWED.contains(&rel.as_str()))
            .collect();

        assert!(
            offenders.is_empty(),
            "installation d'abonné hors de l'installateur unique, dans : {offenders:?}\n\
             \n\
             RÉSOLUTION : router le site vers \
             `crate::test_utils::test_helpers::install_capturing_subscriber(subscriber)`, \
             et poser `#[serial_test::serial]` sur le test appelant **et sur tout test \
             voisin qui atteint le même callsite** — c'est cette seconde moitié qui \
             porte (mika#2646). Ne PAS ajouter d'entrée à `SET_DEFAULT_SITES_ALLOWED` : \
             on déclare, on n'allowliste pas (mika#2201)."
        );
    }

    /// Contrôle de bonne foi : le prédicat mord sur un second site.
    ///
    /// Sans lui, « la garde décide » est indistinguable de « la garde ne
    /// regarde rien » — et c'est exactement l'état qu'un dépouillement de
    /// commentaires trop large produirait.
    #[test]
    fn mika2646_le_scan_voit_un_second_site() {
        let synthetic = vec![
            (
                INSTALLER_SITE.to_string(),
                format!("let g = {}(s);", needle()),
            ),
            (
                "voisin.rs".to_string(),
                format!("    let _guard = {}(subscriber);\n", needle()),
            ),
            (
                "prose.rs".to_string(),
                format!("    /// On n'appelle plus {} ici.\n", needle()),
            ),
        ];
        let sites = sites_carrying_the_needle(&synthetic);
        assert!(
            sites.contains(&"voisin.rs".to_string()),
            "le scan doit accuser un second site : {sites:?}"
        );
        assert!(
            !sites.contains(&"prose.rs".to_string()),
            "une prose qui DÉCRIT le motif n'en est pas une violation : {sites:?}"
        );
    }

    /// L'allowlist est livrée vide **et épinglée vide** — l'épinglage EST
    /// l'assertion auto-nettoyante : il rougit dès qu'on y ajoute une ligne.
    #[test]
    fn mika2646_lallowlist_du_scan_est_livree_vide() {
        assert!(
            SET_DEFAULT_SITES_ALLOWED.is_empty(),
            "il n'y a rien à excepter : les sept sites sont routés vers \
             l'installateur. Une entrée ici est une course rouverte."
        );
    }
}
