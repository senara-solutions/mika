use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::startup;
use mika_common::config::Settings;
use mika_common::home;
use std::path::{Path, PathBuf};

/// Lightweight context for commands that only need the database.
/// Dropping this shuts down the async database automatically.
pub struct DbContext {
    pub settings: Settings,
    pub async_db: AsyncDatabase,
    pub home_dir: PathBuf,
    /// The global Mika home directory (e.g. ~/.mika/).
    /// In multi-agent mode this differs from `home_dir` (which is the agent's dir).
    pub global_home: PathBuf,
}

impl Drop for DbContext {
    fn drop(&mut self) {
        self.async_db.shutdown();
    }
}

/// Shared initialization for an agent: migrate, resolve agent home, load config, open DB.
fn init_base_for_agent(agent_name: &str) -> Result<(Settings, AsyncDatabase, PathBuf, PathBuf)> {
    let global_home = home::resolve_home_dir()?;

    // Auto-migrate legacy layout to multi-agent on every startup
    home::migrate_to_multi_agent(&global_home)?;

    let agent_home = home::resolve_agent_home(&global_home, agent_name);
    ensure_initialized_for_agent(&global_home, &agent_home, agent_name)?;

    let settings = Settings::load_for_agent(&global_home, &agent_home)
        .context("Failed to load config (run `mika setup` first).")?;

    let mut db = open_db(&settings)?;
    let identity = mika_agent::prompt::load_identity(&agent_home);
    db.register_agent(
        agent_name,
        &identity.name,
        agent_home.to_str().unwrap_or(""),
    )?;
    startup::seed_core_memory_if_empty(&db, &agent_home, agent_name)?;
    startup::seed_bundled_skills_if_needed(&agent_home, settings.disable_bundled_skills);
    if settings.dev_mode {
        mika_agent::well_known_agents::seed_well_known_skill_overrides(&mut db, agent_name);
    }
    let async_db = AsyncDatabase::new_with_agent(db, agent_name);

    Ok((settings, async_db, agent_home, global_home))
}

/// Initialize database-only context for a named agent.
///
/// mika#1727 vol V3b-2 — c'est désormais la **seule** porte d'entrée des
/// commandes interactives. `init_for_agent` et `AppContext::override_model`
/// sont partis avec le provider qu'ils bâtissaient : plus aucun tour ne tourne
/// dans ce processus, et pourquoi ce provider local ne pouvait plus que
/// **refuser** se lit sur `commands::ask::run`.
pub fn init_db_only_for_agent(agent_name: &str) -> Result<DbContext> {
    let (settings, async_db, home_dir, global_home) = init_base_for_agent(agent_name)?;
    Ok(DbContext {
        settings,
        async_db,
        home_dir,
        global_home,
    })
}

/// Resolve the active agent name from the home directory.
pub fn resolve_active_agent() -> Result<String> {
    let global_home = home::resolve_home_dir()?;
    Ok(home::read_active_agent(&global_home))
}

fn ensure_initialized_for_agent(
    global_home: &Path,
    agent_home: &Path,
    agent_name: &str,
) -> Result<()> {
    // In multi-agent layout, check the specific agent's home
    if home::is_multi_agent_layout(global_home) {
        if !agent_home.join("config.toml").exists() {
            // Auto-provision well-known agents if dev_mode is enabled
            if mika_agent::well_known_agents::find_well_known_agent(agent_name).is_some()
                && let Ok(global_settings) = Settings::load(global_home)
                && global_settings.dev_mode
            {
                mika_agent::well_known_agents::provision_well_known_agents(
                    global_home,
                    &global_settings,
                    global_settings.disable_agent_provisioning,
                );
            }
            // Re-check after provisioning attempt
            if !agent_home.join("config.toml").exists() {
                anyhow::bail!(
                    "Agent '{agent_name}' not found. Create it with `mika agents create {agent_name}`."
                );
            }
        }
    } else if !home::is_initialized(global_home) {
        anyhow::bail!(
            "Mika not initialized. Run `mika setup` first, or just run `mika` to auto-setup."
        );
    }
    Ok(())
}

fn open_db(settings: &Settings) -> Result<Database> {
    let db_path = &settings.db_path;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    Database::open(db_path).context("failed to open database")
}

// mika#1727 vol V3b-2 — `connect_mcp` est parti d'ici avec son seul appelant,
// `mika chat`, dont aucun outil ne touchait ces connexions depuis V1. La
// migration mika#1737 AC5 de `{agent_home}/mcp.json` qu'elle déclenchait au
// passage garde quatre autres déclencheurs vivants — `commands/mcp.rs`,
// `mika-agent/src/validate.rs`, le démarrage du daemon et leurs tests —
// mesurés avant le retrait, non supposés (mika#2331).

#[cfg(test)]
mod tests {
    // The behavioural tests for alias resolution, prefix stripping and the
    // API-key check moved with their functions to
    // `mika_common::llm::model_override` (mika#2304). They run there unchanged,
    // which is what attests the move is a move and not a rewrite — `mika chat`
    // still gets the same answers (T8).
    //
    // What stays here is the guard a behavioural test cannot provide.

    /// **T7 — `mika-cli` keeps no second resolver (mika#2304, D5).**
    ///
    /// A behavioural test cannot see this class of regression. A re-added local
    /// copy of the alias table or of the prefix-strip rule would make no tested
    /// decision wrong: it would simply let `mika chat` and the A2A turn answer
    /// the same question differently, with only one of the two paths covered.
    /// That is the shape mika#2158 had to engrave after a grooming regex was
    /// copied and then missed two widenings for months — nothing broke, the two
    /// readers just stopped agreeing.
    ///
    /// The allowlist is empty on purpose: there is no legitimate reason for this
    /// crate to define either. Re-exporting from `mika_common` (`pub use`) is
    /// not a definition and is how the four existing call sites still read
    /// `crate::cli::MODEL_ALIASES`.
    #[test]
    fn mika2304_the_cli_keeps_no_second_resolver() {
        // (needle, what a hit would mean). Each needle is assembled with
        // `concat!` so this file does not contain the strings it forbids —
        // otherwise the guard would flag itself and could only be silenced by
        // an allowlist, which is exactly what it must not have.
        const FORBIDDEN: &[(&str, &str)] = &[
            (
                concat!("MODEL_ALIASES", ": &["),
                "a second alias table — `mika_common::llm::model_override::MODEL_ALIASES` is the one",
            ),
            (
                concat!("fn ", "resolve_model_alias"),
                "a second alias resolver — re-export `mika_common::llm::model_override::resolve_model_alias`",
            ),
            (
                concat!("fn ", "parse_model_override"),
                "a second prefix-strip rule — call `mika_common::llm::model_override::parse_model_override`",
            ),
            (
                concat!("fn ", "check_provider_key"),
                "a second API-key check — call `mika_common::llm::model_override::check_provider_key`",
            ),
            (
                concat!("fn ", "provider_requires_api_key"),
                "a second key-requirement table — call `mika_common::llm::model_override::provider_requires_api_key`",
            ),
        ];

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offences = Vec::new();
        visit_rust_files(&src, &mut |path, body| {
            for (needle, why) in FORBIDDEN {
                if body.contains(needle) {
                    offences.push(format!("{}: {needle} — {why}", path.display()));
                }
            }
        });

        assert!(
            offences.is_empty(),
            "mika#2304 D5: the resolution of a `--model` override lives at exactly one \
             site, in `mika-common`, because `mika-agent` must reach the same one. \
             Found:\n  {}",
            offences.join("\n  ")
        );
    }

    /// Good-faith control for the scan above: it must actually read files, or
    /// the guard would pass on an empty walk and attest nothing.
    #[test]
    fn mika2304_the_resolver_scan_reads_the_crate() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut seen = 0usize;
        let mut found_this_file = false;
        visit_rust_files(&src, &mut |path, body| {
            seen += 1;
            if path.ends_with("init.rs")
                && body.contains("mika2304_the_cli_keeps_no_second_resolver")
            {
                found_this_file = true;
            }
        });
        assert!(seen > 10, "the scan only read {seen} files");
        assert!(found_this_file, "the scan did not read init.rs itself");
    }

    fn visit_rust_files(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_rust_files(&path, f);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(body) = std::fs::read_to_string(&path)
            {
                f(&path, &body);
            }
        }
    }
}
