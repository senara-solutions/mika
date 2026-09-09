//! Harness eval **multi-agents** — N `agent_id` distincts sur une seule base
//! container (mika#2265).
//!
//! # Pourquoi ce module existe
//!
//! Le harness eval historique ([`super::harness::EvalHarness`]) est mono-agent
//! par construction : `build()` fait `Database::open_in_memory()` puis
//! `AsyncDatabase::new(db)`, qui délègue à `new_with_agent(db, "mika")`. Toutes
//! les sondes d'audit sont alors scopées sur cet unique `agent_id`, si bien que
//! la question « *lequel* des deux agents a atteint ce callsite ? » n'a aucune
//! forme exprimable — une assertion de cette forme y est vacuously satisfaite.
//!
//! Or la classe de défauts du 2026-09-09 (mika#2248, #2260, #2263) n'est pas
//! « le mauvais résultat » : c'est « le bon résultat, sous le mauvais acteur »,
//! ou « deux acteurs là où un seul était prévu ». Le mot load-bearing est donc
//! **attribution**, et c'est exactement ce que ce module rend mesurable.
//!
//! # La topologie reproduite (celle de la production, pas une approximation)
//!
//! `server/mod.rs` ouvre `home::container_db_path(global_home)` —
//! `{home}/data/mika.db` dans `home.rs` — puis `Database::open(&db_path)` et
//! `AsyncDatabase::new_with_agent(db, agent_name)`. Un container a donc **une
//! seule base fichier** et **une connexion rusqlite par agent** (donc un thread
//! `mika-db` par agent), en WAL avec `busy_timeout = 5000` : les écritures de
//! l'un sont visibles de l'autre.
//!
//! [`MultiAgentHarness`] reproduit cela à l'identique : un `TempDir`, un
//! `data/mika.db`, un `Database::open` **par agent**.
//!
//! ## Le montage à NE PAS refaire
//!
//! Deux `AsyncDatabase::new_with_agent(Database::open_in_memory(), …)` produisent
//! deux mémoires **disjointes** : chaque agent écrit dans son propre néant, rien
//! n'est visible de l'autre, et le montage *ressemble* à du multi-agents sans
//! rien partager. C'est la forme qu'a prise `db_for_agent` dans
//! `test_merge_identity_2248.rs` — correct là-bas, qui n'assert que des
//! invariants structurels sur la source et n'a jamais eu besoin d'un état
//! croisé — mais généralisée telle quelle elle livrerait un harness qui ne voit
//! pas plus que l'actuel. [`MultiAgentHarness::cross_read_count`] est la sonde
//! qui épingle le partage, précisément pour que cette régression soit rouge.
//!
//! `AsyncDatabase::with_agent()` est rejeté pour la même raison inverse : il
//! re-scope un handle en partageant le même `Arc<Inner>` — une connexion, un
//! thread. Ce n'est pas deux agents, c'est un re-scope intra-process, et il ne
//! reproduit ni la concurrence inter-connexion ni les contentions WAL,
//! c'est-à-dire précisément la dimension où vivent #2248 et #2260.
//!
//! # Les deux patrons de diffusion
//!
//! Le harness fournit l'état partagé et les sondes ; **le test compose lui-même
//! la diffusion**. Il ne choisit pas à la place du test, parce que #2248 est un
//! *ordre* et #2260 est une *course* : les deux formes sont nécessaires, aucune
//! n'est le défaut de l'autre.
//!
//! Ordre déterministe (primaire d'abord, puis secondaires) :
//!
//! ```ignore
//! for (agent_id, db) in h.agents() {
//!     handle_event(db, &event).await?;
//! }
//! ```
//!
//! Course (les deux handlers concurrents sur la base partagée) :
//!
//! ```ignore
//! let dev = h.db("mika-dev");
//! let qa = h.db("mika-qa");
//! let (a, b) = tokio::join!(handle_event(dev, &event), handle_event(qa, &event));
//! ```
//!
//! `tokio::join!` est une macro sans feature ; `futures_util::future::join_all`
//! n'est pas disponible ici (`futures-util` est déclaré au workspace avec
//! `default-features = false`, ce qui exclut la feature `alloc`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use tempfile::TempDir;

/// N agents distincts, chacun avec sa propre connexion, sur une seule base
/// container partagée.
///
/// Construit via [`MultiAgentHarness::builder`]. L'ordre de déclaration des
/// agents est préservé par [`MultiAgentHarness::agents`] — un fan-out a un
/// primaire et des secondaires, et l'ordre est parfois ce qu'on assert.
pub struct MultiAgentHarness {
    /// Possédé par le harness : la base tombe avec lui. Jamais lu directement —
    /// c'est le `Drop` qui porte le sens.
    _tmp: TempDir,
    db_path: PathBuf,
    /// `(agent_id, handle, session_id)`, dans l'ordre du builder.
    agents: Vec<MountedAgent>,
}

struct MountedAgent {
    agent_id: String,
    db: AsyncDatabase,
    session_id: String,
}

impl MultiAgentHarness {
    /// Ouvre un builder. Déclarer les agents avec
    /// [`MultiAgentHarnessBuilder::agent`], dans l'ordre voulu.
    pub fn builder() -> MultiAgentHarnessBuilder {
        MultiAgentHarnessBuilder::default()
    }

    /// Handle de l'agent nommé.
    ///
    /// # Panics
    /// Si `agent_id` n'a pas été monté — le message nomme les agents présents,
    /// pour qu'une faute de frappe ne se lise pas comme une absence de données.
    pub fn db(&self, agent_id: &str) -> &AsyncDatabase {
        self.agents
            .iter()
            .find(|a| a.agent_id == agent_id)
            .map(|a| &a.db)
            .unwrap_or_else(|| {
                panic!(
                    "agent `{agent_id}` non monté sur ce MultiAgentHarness ; agents montés : {:?}",
                    self.agent_ids()
                )
            })
    }

    /// Les agents montés, **dans l'ordre de déclaration**.
    pub fn agents(&self) -> impl Iterator<Item = (&str, &AsyncDatabase)> {
        self.agents.iter().map(|a| (a.agent_id.as_str(), &a.db))
    }

    /// Les identifiants montés, dans l'ordre de déclaration.
    pub fn agent_ids(&self) -> Vec<&str> {
        self.agents.iter().map(|a| a.agent_id.as_str()).collect()
    }

    /// La session ouverte pour cet agent au montage.
    ///
    /// # Panics
    /// Si `agent_id` n'a pas été monté.
    pub fn session_id(&self, agent_id: &str) -> &str {
        self.agents
            .iter()
            .find(|a| a.agent_id == agent_id)
            .map(|a| a.session_id.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "agent `{agent_id}` non monté sur ce MultiAgentHarness ; agents montés : {:?}",
                    self.agent_ids()
                )
            })
    }

    /// Le fichier de base partagé par tous les agents montés.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// **Mesure fan-out** : combien d'`audit_events` portant `tool_name`, par
    /// agent monté.
    ///
    /// C'est l'assertion que le harness mono-agent ne peut pas produire :
    /// `count_audit_events_by_tool_name` est scopée sur le `agent_id` du handle,
    /// donc appelée une fois par handle elle *est* la distribution du fan-out.
    /// Une carte à deux clés dit « deux agents ont atteint ce callsite » ; une
    /// carte à une clé dit qu'un seul l'a atteint.
    pub async fn audit_counts_by_agent(&self, tool_name: &str) -> Result<BTreeMap<String, i64>> {
        let mut out = BTreeMap::new();
        for a in &self.agents {
            let n =
                a.db.count_audit_events_by_tool_name(tool_name)
                    .await
                    .with_context(|| {
                        format!(
                            "count_audit_events_by_tool_name({tool_name}) pour `{}`",
                            a.agent_id
                        )
                    })?;
            out.insert(a.agent_id.clone(), n);
        }
        Ok(out)
    }

    /// **Sonde de partage** : ce que le handle de `reader` voit de la tranche de
    /// `subject`.
    ///
    /// `count_audit_events` prend l'`agent_id` en paramètre explicite, donc un
    /// handle peut interroger la tranche d'un autre. Sur un montage à mémoires
    /// disjointes cette valeur est `0` alors que `subject` a bel et bien écrit :
    /// c'est le contrôle positif du montage lui-même.
    ///
    /// # Panics
    /// Si `reader` n'a pas été monté.
    pub async fn cross_read_count(&self, reader: &str, subject: &str) -> Result<u64> {
        self.db(reader)
            .count_audit_events(subject)
            .await
            .with_context(|| format!("`{reader}` lisant la tranche de `{subject}`"))
    }

    /// Ferme proprement chaque thread `mika-db`, puis laisse tomber le `TempDir`.
    pub fn shutdown(self) {
        for a in &self.agents {
            a.db.shutdown();
        }
    }
}

/// Builder de [`MultiAgentHarness`]. Les agents sont montés dans l'ordre où
/// [`Self::agent`] les déclare.
#[derive(Default)]
pub struct MultiAgentHarnessBuilder {
    agent_ids: Vec<String>,
}

impl MultiAgentHarnessBuilder {
    /// Déclare un agent à monter. Appeler une fois par `agent_id`.
    pub fn agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_ids.push(agent_id.into());
        self
    }

    /// Monte la base partagée et un handle par agent déclaré.
    ///
    /// Pour chaque agent, dans l'ordre : `Database::open(&db_path)` (le premier
    /// open migre, les suivants trouvent le schéma à jour — `migrate()` est
    /// idempotent) → `register_agent` (obligatoire : `audit_events.agent_id`
    /// porte une FK vers `agents(id)`) → `create_session` →
    /// `AsyncDatabase::new_with_agent`.
    pub fn build(self) -> Result<MultiAgentHarness> {
        anyhow::ensure!(
            !self.agent_ids.is_empty(),
            "MultiAgentHarness sans agent : déclarer au moins un `.agent(id)`"
        );
        for (i, id) in self.agent_ids.iter().enumerate() {
            anyhow::ensure!(
                !self.agent_ids[..i].contains(id),
                "agent `{id}` déclaré deux fois — un `agent_id` identifie une tranche d'audit, \
                 le dupliquer rendrait toute attribution ambiguë"
            );
        }

        let tmp = TempDir::new().context("TempDir pour la base container partagée")?;
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).context("créer data/")?;
        // Même forme que `home::container_db_path` : `{home}/data/mika.db`.
        let db_path = data_dir.join("mika.db");

        let mut agents = Vec::with_capacity(self.agent_ids.len());
        for agent_id in self.agent_ids {
            let db = Database::open(&db_path)
                .with_context(|| format!("open({}) pour `{agent_id}`", db_path.display()))?;
            db.register_agent(&agent_id, &agent_id, "")
                .with_context(|| format!("register_agent({agent_id})"))?;
            let session_id = format!("eval-multi-{agent_id}-{}", uuid::Uuid::new_v4());
            db.create_session(&session_id, &agent_id, "github")
                .with_context(|| format!("create_session pour `{agent_id}`"))?;
            let async_db = AsyncDatabase::new_with_agent(db, &agent_id);
            agents.push(MountedAgent {
                agent_id,
                db: async_db,
                session_id,
            });
        }

        Ok(MultiAgentHarness {
            _tmp: tmp,
            db_path,
            agents,
        })
    }
}
