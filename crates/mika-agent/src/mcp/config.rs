//! MCP server configuration.
//!
//! MCP servers are configured via `~/.mika/mcp.json` (or `{agent_home}/mcp.json`
//! in multi-agent mode). The format matches the Claude Desktop convention:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "filesystem": {
//!       "transport": "stdio",
//!       "command": "npx",
//!       "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"],
//!       "env": {},
//!       "enabled": true
//!     }
//!   }
//! }
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::skills::manifest::DataGrade;

/// Top-level MCP configuration, loaded from `mcp.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server.
#[derive(Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub transport: McpTransport,
    /// Command to run for stdio transport.
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Environment variables to set for the child process.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// URL for HTTP transport.
    #[serde(default)]
    pub url: Option<String>,
    /// HTTP headers for Streamable HTTP transport (e.g. Authorization, API keys).
    /// Ignored for stdio transport.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Whether this server is enabled (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Grade des données exposées par les outils de ce serveur (mika#1959,
    /// doctrine non-transit mika#1798).
    ///
    /// Absent ⇒ `DataGrade::Operational` par le `#[default]` de l'enum : c'est
    /// la porte porteuse. Un manifeste qui oublie de déclarer son grade ne peut
    /// pas acquérir par accident un privilège testimony ; il faut le consentement
    /// explicite `"data_grade": "testimony"` — qui vaut refus de L4, pas
    /// permission (voir la doc opérateur).
    ///
    /// Valeurs : `"operational"` (défaut) | `"testimony"`. Le `reference` du
    /// corps de mika#1959 se lit `operational` ici — l'enum livré par PR#1956
    /// est binaire (décision D1 du plan).
    ///
    /// Le grade vaut pour **tous** les outils du serveur. La granularité par
    /// outil est une affaire de v2 : aucun serveur v1 connu n'expose des outils
    /// de grades mêlés.
    ///
    /// Depuis mika#2814, un grade `testimony` ferme les **deux** moitiés de la
    /// doctrine : les outils du serveur ne sont jamais publiés au modèle
    /// (`mcp::retain_publiables`, L2) *et* un appel halluciné est refusé avant
    /// `call_tool` (L4). Avant, seule la seconde mordait.
    ///
    /// Une clé mal orthographiée (`"dataGrade"`) reste ignorée en silence par
    /// `serde` — le chargement est fail-open (mika#1764) — mais elle **avertit
    /// désormais au chargement** : [`KNOWN_SERVER_KEYS`] et
    /// `auditer_cles_de_serveur` plus bas dans ce fichier (mika#2814, vol 3).
    /// Un avertissement, jamais un refus, et aucune valeur n'est réécrite.
    #[serde(default)]
    pub data_grade: DataGrade,
}

/// Les clés qu'une entrée de serveur de `mcp.json` peut légitimement porter
/// (mika#2814, vol 3) — les huit champs sérialisés de [`McpServerConfig`].
///
/// C'est un **littéral**, et non un dérivé : `serde` n'expose pas les noms de
/// champs d'une structure à l'exécution, donc rien dans le langage ne permet de
/// demander au type la liste de ce qu'il sérialise.
///
/// Ce qui l'empêche de dériver sans qu'on le sache est le test de round-trip
/// `mika2814_les_cles_connues_egalent_les_champs_serialises` : il sérialise un
/// `McpServerConfig` et compare l'ensemble des clés obtenues à cette const,
/// **dans les deux sens**. Les deux divergences coûtent, et pas la même chose :
/// un neuvième champ ajouté au type sans entrée ici ferait avertir l'audit sur
/// une clé parfaitement valide — l'avertissement perdrait sa crédibilité le jour
/// où il a raison — et une entrée devenue fantôme le rendrait aveugle sur un
/// champ retiré. L'assertion bidirectionnelle est ce qui rougit dans les deux
/// cas, et c'est aussi ce qui la rend auto-nettoyante : le geste qui rend une
/// entrée inutile est celui qui la fait expirer (mika#2309).
const KNOWN_SERVER_KEYS: &[&str] = &[
    "transport",
    "command",
    "args",
    "env",
    "url",
    "headers",
    "enabled",
    "data_grade",
];

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("McpServerConfig");
        s.field("transport", &self.transport);
        s.field("command", &self.command);
        s.field("args", &self.args);
        // Redact env values (may contain secrets like GITHUB_TOKEN)
        if let Some(env) = &self.env {
            let redacted: HashMap<&String, &str> = env.keys().map(|k| (k, "[REDACTED]")).collect();
            s.field("env", &redacted);
        } else {
            s.field("env", &self.env);
        }
        s.field("url", &self.url);
        // Redact header values (may contain bearer tokens / API keys)
        if let Some(headers) = &self.headers {
            let redacted: HashMap<&String, &str> =
                headers.keys().map(|k| (k, "[REDACTED]")).collect();
            s.field("headers", &redacted);
        } else {
            s.field("headers", &self.headers);
        }
        s.field("enabled", &self.enabled);
        s.field("data_grade", &self.data_grade);
        s.finish()
    }
}

/// MCP transport type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}

fn default_true() -> bool {
    true
}

/// Une clé inconnue relevée dans l'entrée d'un serveur (mika#2814, vol 3).
#[derive(Debug, Clone, PartialEq, Eq)]
struct CleInconnue {
    serveur: String,
    cle: String,
    /// Clé connue dont celle-ci est vraisemblablement une faute de frappe.
    ///
    /// `None` quand aucune ne correspond après normalisation — et la ligne de
    /// journal **omet** alors le champ `suggestion` plutôt que de l'écrire vide :
    /// « aucune suggestion » et « suggestion vide » ne sont pas le même fait
    /// (motif mika#2331).
    suggestion: Option<&'static str>,
}

/// Ce qu'un audit a vu, qu'il ait trouvé quelque chose ou non (mika#2814, vol 3).
///
/// `serveurs` est un **vrai compte** : le parcours a eu lieu et a rendu ce
/// nombre, donc la ligne de contrôle positif l'écrit même à zéro. C'est l'inverse
/// exact du `suggestion` absent de [`CleInconnue`], et les deux cohabitent
/// volontairement — un compte qui a eu lieu se dit `0`, un fait qui n'existe pas
/// s'omet.
#[derive(Debug, Default, PartialEq, Eq)]
struct AuditCles {
    /// Entrées de serveur réellement parcourues (une entrée non-objet n'en est
    /// pas une : il n'y a aucune clé à y auditer, et `from_value` rendra l'erreur
    /// de forme juste après).
    serveurs: usize,
    inconnues: Vec<CleInconnue>,
}

/// Clé connue dont `cle` est vraisemblablement une faute de frappe, s'il en
/// existe une (mika#2814, vol 3).
///
/// La comparaison se fait après **normalisation** — minuscules, retrait des `_`
/// et des `-` — donc `dataGrade` → `datagrade` ← `data_grade` : correspondance
/// trouvée. Cela couvre la famille réelle de fautes, celle d'un `mcp.json`
/// recopié depuis un autre hôte MCP ou tapé de mémoire : camelCase, kebab-case,
/// SCREAMING_SNAKE.
///
/// **Pas de distance d'édition, et c'est un refus argumenté** : le dépôt n'a
/// aucune implémentation de Levenshtein, et en ajouter une dépendance pour huit
/// clés serait disproportionné. Une clé sans correspondance normalisée n'obtient
/// donc **pas** de suggestion — le message de l'avertissement liste de toute
/// façon les huit clés reconnues, donc l'opérateur a l'information sans elle.
fn cle_connue_equivalente(cle: &str) -> Option<&'static str> {
    let cible = normaliser_cle(cle);
    KNOWN_SERVER_KEYS
        .iter()
        .copied()
        .find(|connue| normaliser_cle(connue) == cible)
}

/// Minuscules, et retrait des `_` et des `-` — la forme sous laquelle deux
/// orthographes d'une même clé se rejoignent (mika#2814, vol 3).
fn normaliser_cle(cle: &str) -> String {
    cle.chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Les clés inconnues de chaque entrée de serveur du `mcp.json` déjà parsé
/// (mika#2814, vol 3).
///
/// Fonction pure, et c'est elle que les tests interrogent :
/// [`auditer_cles_de_serveur`] ne fait que journaliser ce qu'elle rend. Une
/// assertion sur des lignes de `tracing` capturées épinglerait la formulation,
/// pas la propriété.
///
/// Un `mcpServers` absent ou non-objet, et une entrée non-objet : rien à
/// auditer, aucun relevé. L'erreur de forme est rendue par
/// `serde_json::from_value` juste après, inchangée.
fn collecter_cles_inconnues(brut: &serde_json::Value) -> AuditCles {
    let mut audit = AuditCles::default();

    let Some(serveurs) = brut
        .get("mcpServers")
        .and_then(serde_json::Value::as_object)
    else {
        return audit;
    };

    for (nom, entree) in serveurs {
        let Some(entree) = entree.as_object() else {
            continue;
        };
        audit.serveurs += 1;
        for cle in entree.keys() {
            if KNOWN_SERVER_KEYS.contains(&cle.as_str()) {
                continue;
            }
            audit.inconnues.push(CleInconnue {
                serveur: nom.clone(),
                cle: cle.clone(),
                suggestion: cle_connue_equivalente(cle),
            });
        }
    }

    audit
}

/// Message des avertissements de clé inconnue, partagé par les deux arms de
/// [`auditer_cles_de_serveur`] (avec et sans `suggestion`) pour qu'ils ne
/// puissent pas diverger.
const MSG_CLE_INCONNUE: &str = "clé inconnue dans l'entrée d'un serveur de mcp.json : \
     serde l'ignore sans un octet, donc ce qui est déclaré là n'a aucun effet — \
     un `\"dataGrade\": \"testimony\"` laisse le serveur publier ses outils comme \
     `operational`. Le chargement n'est pas refusé et aucune valeur n'est \
     réécrite : fail-open (mika#1764), parce qu'un mcp.json écrit pour un autre \
     hôte MCP porte légitimement des clés que Mika ignore.";

/// Journalise les clés inconnues des entrées de serveur d'un `mcp.json` parsé
/// (mika#2814, vol 3).
///
/// Ne rend rien, parce qu'un audit ne décide rien — même forme et même raison
/// qu'`AuthBoundaryLedger` (mika#1949) et que le filet d'audit de mika#2456. Un
/// `WARN mcp_config_unknown_server_key` par clé inconnue, **et** un
/// `INFO mcp_config_keys_audited` par chargement, y compris sur la branche saine :
/// sans lui, « aucune clé inconnue » et « le binaire servi précède ce
/// correctif » rendraient les mêmes octets (classe mika#2340, que ce dépôt a
/// déjà payée deux fois).
///
/// `source` n'entre que dans les lignes de journal, et il y est indispensable :
/// la cascade `resolve_operator_mcp_config_path` désigne jusqu'à quatre fichiers,
/// donc un opérateur qui lit un avertissement sur `dataGrade` doit savoir
/// **lequel** le porte.
fn auditer_cles_de_serveur(brut: &serde_json::Value, source: &Path) {
    let audit = collecter_cles_inconnues(brut);
    let connues = KNOWN_SERVER_KEYS.join(", ");

    for inconnue in &audit.inconnues {
        match inconnue.suggestion {
            Some(suggestion) => tracing::warn!(
                event = "mcp_config_unknown_server_key",
                source = %source.display(),
                server = %inconnue.serveur,
                key = %inconnue.cle,
                known_keys = %connues,
                suggestion,
                "{MSG_CLE_INCONNUE}"
            ),
            // Champ `suggestion` absent, jamais `""` (mika#2331).
            None => tracing::warn!(
                event = "mcp_config_unknown_server_key",
                source = %source.display(),
                server = %inconnue.serveur,
                key = %inconnue.cle,
                known_keys = %connues,
                "{MSG_CLE_INCONNUE}"
            ),
        }
    }

    tracing::info!(
        event = "mcp_config_keys_audited",
        source = %source.display(),
        servers = audit.serveurs,
        // Vrai zéro : le comptage a eu lieu et a rendu zéro.
        unknown_keys = audit.inconnues.len(),
        "audit des clés des entrées de serveur de mcp.json"
    );
}

impl McpConfig {
    /// Désérialise un `mcp.json` **en auditant ses clés** au passage
    /// (mika#2814, vol 3).
    ///
    /// C'est le seul chemin de désérialisation de la production : `load` et
    /// `load_operator_shell` passent tous les deux par ici. Un unique
    /// constructeur, et il audite — plutôt qu'un appel à placer correctement
    /// à chaque site et une garde pour vérifier qu'on l'a fait. Le vol 2 de ce
    /// même ticket a choisi l'inverse (`publier_outils_mcp` + sa garde
    /// d'appariement) parce que là une omission laissait **partir un appel
    /// distant** ; ici elle ne perd qu'un avertissement, donc la garde ne se
    /// paie pas.
    ///
    /// `source` ne sert qu'aux lignes de journal — la cascade
    /// `resolve_operator_mcp_config_path` désigne jusqu'à quatre fichiers, donc
    /// un avertissement muet sur son origine n'est pas actionnable.
    ///
    /// Le détour par `serde_json::Value` coûte les positions ligne/colonne sur
    /// les erreurs de **type** (`from_value` ne les porte pas). Les erreurs de
    /// **syntaxe** les gardent : elles sortent du `from_str` ci-dessous, avant
    /// tout audit.
    pub fn depuis_json(content: &str, source: &Path) -> anyhow::Result<Self> {
        let brut: serde_json::Value = serde_json::from_str(content)?;
        auditer_cles_de_serveur(&brut, source);
        Ok(serde_json::from_value(brut)?)
    }

    /// Load MCP configuration from `{home_dir}/mcp.json`.
    ///
    /// Returns an empty config if the file does not exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(home_dir: &Path) -> anyhow::Result<Self> {
        let path = home_dir.join("mcp.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Self::depuis_json(&content, &path)
    }

    /// Save MCP configuration to `{home_dir}/mcp.json`.
    ///
    /// Sets file permissions to `0600` on Unix (may contain secrets in headers/env).
    pub fn save(&self, home_dir: &Path) -> anyhow::Result<()> {
        let path = home_dir.join("mcp.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// Return only enabled servers.
    pub fn enabled_servers(&self) -> impl Iterator<Item = (&str, &McpServerConfig)> {
        self.mcp_servers
            .iter()
            .filter(|(_, cfg)| cfg.enabled)
            .map(|(name, cfg)| (name.as_str(), cfg))
    }

    /// Load MCP configuration from the operator-shell path resolved by
    /// [`mika_common::mcp_config_path::resolve_operator_mcp_config_path`]
    /// (mika#1737 AC3, AC4). Returns an empty config if the file does
    /// not exist. Returns an error only when the file exists but is
    /// malformed. The resolved source tier is logged so operators can
    /// verify which resolution rung fired.
    ///
    /// **Callers are responsible for the fail-open policy on their
    /// path.** `init_agent()` warns and continues with an empty config
    /// on parse error (mika#1764 — MCP is a bolt-on capability and must
    /// not wedge server startup); `validate::validate_agent()` surfaces
    /// the parse error as a `Fail` diagnostic (operators need to know
    /// their config is broken).
    pub fn load_operator_shell() -> anyhow::Result<Self> {
        let (path, source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
        if matches!(
            source,
            mika_common::mcp_config_path::McpConfigPathSource::CwdFallback
        ) {
            tracing::warn!(
                path = %path.display(),
                "MCP config path fell back to CWD; set MIKA_MCP_CONFIG, XDG_CONFIG_HOME, or HOME"
            );
        } else {
            tracing::debug!(
                path = %path.display(),
                ?source,
                "resolved operator-shell MCP config path"
            );
        }
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Self::depuis_json(&content, &path)
    }

    /// Fail-open wrapper around [`load_operator_shell`] for server-startup
    /// callers. On any load error (unreadable file, malformed JSON) warns
    /// with the path + error and returns an empty config. MCP is a
    /// bolt-on capability; a corrupt operator-shell file MUST NOT wedge
    /// `init_agent()` (mika#1764 — the actual failure surfaced on the
    /// gentux CI runner where `~/.config/mika/mcp-servers.json` contained
    /// `{ invalid json`). The strict `load_operator_shell` stays available
    /// for the validate path that needs to surface the error as a Fail
    /// diagnostic.
    pub fn load_operator_shell_or_empty() -> Self {
        match Self::load_operator_shell() {
            Ok(config) => config,
            Err(err) => {
                let (path, _) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "operator-shell MCP config failed to load; continuing with no MCP servers (mika#1764)"
                );
                Self::default()
            }
        }
    }

    /// Save MCP configuration to the operator-shell path (mika#1737
    /// AC1). Creates parent directories as needed; sets `0600` on Unix
    /// because headers/env may contain secrets.
    pub fn save_operator_shell(&self) -> anyhow::Result<()> {
        let (path, _source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    /// One-shot AC5 migration: copy an existing per-agent
    /// `{agent_home}/mcp.json` to the operator-shell path if the
    /// operator-shell path does not yet exist. Idempotent — subsequent
    /// invocations skip the copy once the operator-shell path exists.
    ///
    /// Returns `Ok(true)` if a copy happened, `Ok(false)` if not, or
    /// `Err` on IO failure. IO errors are non-fatal to the caller (the
    /// operator can hand-copy) so callers should log-and-continue.
    ///
    /// Multi-agent semantics: this migration is deliberately per-invocation
    /// / per-agent. If several agents each have a distinct `mcp.json`, the
    /// FIRST invocation across all agents wins and populates the shared
    /// operator-shell config. Later invocations see the operator-shell
    /// path already exists and become no-ops. Operators with divergent
    /// per-agent configs must hand-merge — the ratified disposition
    /// explicitly makes MCP config operator-shell scoped, so divergent
    /// per-agent state was already an anti-pattern.
    pub fn migrate_from_agent_home_if_needed(agent_home: &Path) -> anyhow::Result<bool> {
        let (target, _source) = mika_common::mcp_config_path::resolve_operator_mcp_config_path();
        if target.exists() {
            return Ok(false);
        }
        let source_path = mika_common::mcp_config_path::legacy_per_agent_mcp_path(agent_home);
        if !source_path.exists() {
            return Ok(false);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Copy contents (do NOT symlink) so a subsequent delete of the
        // per-agent file does not lose the config.
        let content = std::fs::read_to_string(&source_path)?;
        std::fs::write(&target, &content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
        }
        tracing::info!(
            from = %source_path.display(),
            to = %target.display(),
            "migrated per-agent MCP config to operator-shell path (mika#1737 AC5)"
        );
        Ok(true)
    }
}

impl McpServerConfig {
    /// Validate that the config has the required fields for its transport type.
    pub fn validate(&self, name: &str) -> anyhow::Result<()> {
        // Server names must be lowercase alphanumeric with single hyphens/underscores.
        // Double underscores (__) are the namespacing separator and must not appear in names.
        if name.is_empty()
            || name.contains("__")
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            anyhow::bail!(
                "MCP server name '{name}' is invalid: use lowercase alphanumeric, hyphens, \
                 or single underscores (no '__')"
            );
        }

        match self.transport {
            McpTransport::Stdio => {
                if self.command.as_ref().is_none_or(|c| c.trim().is_empty()) {
                    anyhow::bail!("MCP server '{name}': stdio transport requires 'command'");
                }
                if self.headers.as_ref().is_some_and(|h| !h.is_empty()) {
                    tracing::warn!(
                        server = name,
                        "MCP server '{name}': headers are ignored for stdio transport"
                    );
                }
            }
            McpTransport::Http => {
                let url = match self.url.as_deref() {
                    Some(u) if !u.trim().is_empty() => u.trim(),
                    _ => {
                        anyhow::bail!("MCP server '{name}': http transport requires 'url'");
                    }
                };
                // Validate URL scheme
                match url.split_once("://") {
                    Some(("http" | "https", _)) => {}
                    _ => {
                        anyhow::bail!("MCP server '{name}': url must use http or https scheme");
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "transport": "stdio",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                    "enabled": true
                },
                "remote": {
                    "transport": "http",
                    "url": "http://localhost:8000/mcp",
                    "enabled": false
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.mcp_servers.len(), 2);

        let fs = &config.mcp_servers["filesystem"];
        assert_eq!(fs.transport, McpTransport::Stdio);
        assert_eq!(fs.command.as_deref(), Some("npx"));
        assert!(fs.enabled);

        let remote = &config.mcp_servers["remote"];
        assert_eq!(remote.transport, McpTransport::Http);
        assert_eq!(remote.url.as_deref(), Some("http://localhost:8000/mcp"));
        assert!(!remote.enabled);
    }

    #[test]
    fn test_parse_empty_config() {
        let json = "{}";
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_enabled_default_true() {
        let json = r#"{
            "mcpServers": {
                "test": {
                    "transport": "stdio",
                    "command": "echo"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers["test"].enabled);
    }

    #[test]
    fn test_enabled_servers_filter() {
        let json = r#"{
            "mcpServers": {
                "a": { "transport": "stdio", "command": "a", "enabled": true },
                "b": { "transport": "stdio", "command": "b", "enabled": false },
                "c": { "transport": "stdio", "command": "c", "enabled": true }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let enabled: Vec<&str> = config.enabled_servers().map(|(n, _)| n).collect();
        assert_eq!(enabled.len(), 2);
        assert!(enabled.contains(&"a"));
        assert!(enabled.contains(&"c"));
    }

    #[test]
    fn test_validate_stdio_requires_command() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: None,
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("test").is_err());
    }

    #[test]
    fn test_validate_http_requires_url() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("test").is_err());
    }

    #[test]
    fn test_validate_valid_stdio() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("npx".to_string()),
            args: Some(vec!["-y".to_string(), "server".to_string()]),
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("test").is_ok());
    }

    #[test]
    fn test_validate_valid_http() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: Some("http://localhost:8000/mcp".to_string()),
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("test").is_ok());
    }

    #[test]
    fn test_load_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let config = McpConfig::load(tmp.path()).unwrap();
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = McpConfig::default();
        config.mcp_servers.insert(
            "test".to_string(),
            McpServerConfig {
                transport: McpTransport::Stdio,
                command: Some("echo".to_string()),
                args: None,
                env: None,
                url: None,
                headers: None,
                enabled: true,
                data_grade: DataGrade::default(),
            },
        );
        config.save(tmp.path()).unwrap();

        let loaded = McpConfig::load(tmp.path()).unwrap();
        assert_eq!(loaded.mcp_servers.len(), 1);
        assert!(loaded.mcp_servers.contains_key("test"));
    }

    #[test]
    fn test_validate_rejects_double_underscore_name() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("my__server").is_err());
    }

    #[test]
    fn test_validate_rejects_uppercase_name() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("MyServer").is_err());
    }

    #[test]
    fn test_validate_accepts_hyphen_underscore_name() {
        let cfg = McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("echo".to_string()),
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("my-server_1").is_ok());
    }

    #[test]
    fn test_validate_http_rejects_ftp_scheme() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: Some("ftp://evil.com/mcp".to_string()),
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("test").is_err());
    }

    #[test]
    fn test_validate_http_accepts_https() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: None,
            url: Some("https://mcp.example.com/v1".to_string()),
            headers: None,
            enabled: true,
            data_grade: DataGrade::default(),
        };
        assert!(cfg.validate("test").is_ok());
    }

    #[test]
    fn test_parse_config_with_headers() {
        let json = r#"{
            "mcpServers": {
                "remote": {
                    "transport": "http",
                    "url": "https://mcp.example.com/v1",
                    "headers": {
                        "Authorization": "Bearer sk-test-123",
                        "X-API-Key": "abc"
                    }
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let remote = &config.mcp_servers["remote"];
        let headers = remote.headers.as_ref().unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers["Authorization"], "Bearer sk-test-123");
        assert_eq!(headers["X-API-Key"], "abc");
    }

    #[test]
    fn test_parse_config_without_headers_backwards_compatible() {
        let json = r#"{
            "mcpServers": {
                "old": {
                    "transport": "http",
                    "url": "http://localhost:8000/mcp"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.mcp_servers["old"].headers.is_none());
    }

    #[test]
    fn test_parse_config_empty_headers() {
        let json = r#"{
            "mcpServers": {
                "test": {
                    "transport": "http",
                    "url": "http://localhost:8000/mcp",
                    "headers": {}
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        let headers = config.mcp_servers["test"].headers.as_ref().unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn test_debug_redacts_header_and_env_values() {
        let cfg = McpServerConfig {
            transport: McpTransport::Http,
            command: None,
            args: None,
            env: Some(HashMap::from([(
                "GITHUB_TOKEN".to_string(),
                "ghp_secret123".to_string(),
            )])),
            url: Some("http://localhost/mcp".to_string()),
            headers: Some(HashMap::from([(
                "Authorization".to_string(),
                "Bearer secret-token".to_string(),
            )])),
            enabled: true,
            data_grade: DataGrade::default(),
        };
        let debug_output = format!("{:?}", cfg);
        assert!(debug_output.contains("[REDACTED]"));
        // Header values redacted
        assert!(!debug_output.contains("secret-token"));
        assert!(debug_output.contains("Authorization"));
        // Env values redacted
        assert!(!debug_output.contains("ghp_secret123"));
        assert!(debug_output.contains("GITHUB_TOKEN"));
    }

    #[test]
    fn test_headers_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = McpConfig::default();
        config.mcp_servers.insert(
            "api".to_string(),
            McpServerConfig {
                transport: McpTransport::Http,
                command: None,
                args: None,
                env: None,
                url: Some("https://api.example.com/mcp".to_string()),
                headers: Some(HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer tok".to_string(),
                )])),
                enabled: true,
                data_grade: DataGrade::default(),
            },
        );
        config.save(tmp.path()).unwrap();

        let loaded = McpConfig::load(tmp.path()).unwrap();
        let headers = loaded.mcp_servers["api"].headers.as_ref().unwrap();
        assert_eq!(headers["Authorization"], "Bearer tok");
    }

    /// mika#1764: `load_operator_shell()` is the strict primitive that
    /// surfaces parse errors — the validate path relies on this to
    /// produce Fail diagnostics. The startup-side fail-open policy
    /// lives in `init_agent()`; see `load_operator_shell_or_empty` for
    /// its coverage.
    #[test]
    #[serial_test::serial]
    fn load_operator_shell_propagates_malformed_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mcp-servers.json");
        std::fs::write(&path, "{ invalid json").unwrap();

        let prior = std::env::var(mika_common::mcp_config_path::MCP_CONFIG_ENV).ok();
        // SAFETY: single-threaded test body via serial_test.
        unsafe {
            std::env::set_var(
                mika_common::mcp_config_path::MCP_CONFIG_ENV,
                path.to_str().unwrap(),
            );
        }

        let result = McpConfig::load_operator_shell();

        // SAFETY: single-threaded test body via serial_test.
        unsafe {
            match prior {
                Some(v) => std::env::set_var(mika_common::mcp_config_path::MCP_CONFIG_ENV, v),
                None => std::env::remove_var(mika_common::mcp_config_path::MCP_CONFIG_ENV),
            }
        }

        assert!(
            result.is_err(),
            "malformed JSON must propagate as anyhow error (validate path needs this)"
        );
    }

    /// Existing well-formed JSON must still parse — regression guard
    /// covering the happy path.
    #[test]
    #[serial_test::serial]
    fn load_operator_shell_parses_valid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mcp-servers.json");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"fs":{"transport":"stdio","command":"echo"}}}"#,
        )
        .unwrap();

        let prior = std::env::var(mika_common::mcp_config_path::MCP_CONFIG_ENV).ok();
        // SAFETY: single-threaded test body via serial_test.
        unsafe {
            std::env::set_var(
                mika_common::mcp_config_path::MCP_CONFIG_ENV,
                path.to_str().unwrap(),
            );
        }

        let result = McpConfig::load_operator_shell();

        // SAFETY: single-threaded test body via serial_test.
        unsafe {
            match prior {
                Some(v) => std::env::set_var(mika_common::mcp_config_path::MCP_CONFIG_ENV, v),
                None => std::env::remove_var(mika_common::mcp_config_path::MCP_CONFIG_ENV),
            }
        }

        let config = result.expect("valid JSON must parse");
        assert_eq!(config.mcp_servers.len(), 1);
        assert!(config.mcp_servers.contains_key("fs"));
    }

    /// mika#1764: a corrupt operator-shell MCP config on disk must NOT
    /// wedge server startup. This wraps the strict `load_operator_shell`
    /// primitive with a warn-and-continue policy — the exact fail-open
    /// behavior `init_agent()` needs.
    #[test]
    #[serial_test::serial]
    fn load_operator_shell_or_empty_falls_back_on_malformed_json() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mcp-servers.json");
        std::fs::write(&path, "{ invalid json").unwrap();

        let prior = std::env::var(mika_common::mcp_config_path::MCP_CONFIG_ENV).ok();
        // SAFETY: single-threaded test body via serial_test.
        unsafe {
            std::env::set_var(
                mika_common::mcp_config_path::MCP_CONFIG_ENV,
                path.to_str().unwrap(),
            );
        }

        let config = McpConfig::load_operator_shell_or_empty();

        // SAFETY: single-threaded test body via serial_test.
        unsafe {
            match prior {
                Some(v) => std::env::set_var(mika_common::mcp_config_path::MCP_CONFIG_ENV, v),
                None => std::env::remove_var(mika_common::mcp_config_path::MCP_CONFIG_ENV),
            }
        }

        assert!(
            config.mcp_servers.is_empty(),
            "startup path must fall through to empty config on malformed JSON"
        );
    }

    // -- mika#1959 : le grade de données du manifeste MCP --------------------

    /// AC1 — un manifeste sans `data_grade` vaut `Operational`.
    ///
    /// C'est la porte porteuse de tout le ticket : la population modale des
    /// `mcp.json` installés ne porte pas le champ, et aucun de ces serveurs ne
    /// doit acquérir par accident le privilège testimony — qui, du côté L4,
    /// vaut **refus**. Un défaut inversé ferait refuser toute la flotte.
    #[test]
    fn mika1959_un_manifeste_sans_grade_vaut_operational() {
        let json = r#"{
            "mcpServers": {
                "serveur-quelconque": {
                    "transport": "stdio",
                    "command": "some-mcp"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.mcp_servers["serveur-quelconque"].data_grade,
            DataGrade::Operational,
            "mika#1959 — champ absent ⇒ `Operational`. Un défaut `Testimony` \
             refuserait tous les serveurs installés avant ce ticket."
        );
    }

    /// AC1 — les deux variantes se déclarent explicitement et se relisent.
    #[test]
    fn mika1959_les_deux_variantes_se_declarent_explicitement() {
        let json = r#"{
            "mcpServers": {
                "journal-perso": {
                    "transport": "stdio",
                    "command": "npx",
                    "args": ["-y", "@exemple/mcp-journal"],
                    "data_grade": "testimony"
                },
                "wikipedia": {
                    "transport": "http",
                    "url": "https://mcp.wikipedia.org/",
                    "data_grade": "operational"
                }
            }
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.mcp_servers["journal-perso"].data_grade,
            DataGrade::Testimony
        );
        assert_eq!(
            config.mcp_servers["wikipedia"].data_grade,
            DataGrade::Operational
        );
    }

    /// AC1 — un grade hors enum fait **échouer** le chargement, `"reference"`
    /// nommément.
    ///
    /// `"reference"` est le cas nommé parce que le *Scope* du corps de mika#1959
    /// écrit `testimony | evidence | reference` : un opérateur qui suit le
    /// ticket plutôt que la doc opérateur écrira exactement cette valeur. La
    /// décision D1 du plan la lit `operational` — et le refus est bruyant, pas
    /// silencieux : **un grade n'est jamais deviné.**
    #[test]
    fn mika1959_un_grade_inconnu_dont_reference_fait_echouer_le_parse() {
        for grade in ["reference", "evidence", "Testimony", "secret"] {
            let json = format!(
                r#"{{"mcpServers": {{"s": {{"transport": "stdio", "command": "x", "data_grade": "{grade}"}}}}}}"#
            );
            let parsed: Result<McpConfig, _> = serde_json::from_str(&json);
            assert!(
                parsed.is_err(),
                "mika#1959 — `data_grade = \"{grade}\"` doit faire échouer le chargement de \
                 mcp.json, pas retomber en silence sur un défaut. Un grade deviné est \
                 exactement ce que la doctrine non-transit (mika#1798) refuse."
            );
        }
    }

    /// mika#2814 vol 3 — `KNOWN_SERVER_KEYS` **égale** l'ensemble des clés que
    /// `McpServerConfig` sérialise, dans les deux sens.
    ///
    /// Ce qui empêche la const de dériver sans qu'on le sache, puisque `serde`
    /// n'expose pas les noms de champs à l'exécution. Les deux divergences
    /// coûtent, et pas la même chose : un champ du type absent de la const ferait
    /// avertir l'audit sur une clé parfaitement valide — l'avertissement perdrait
    /// sa crédibilité le jour où il a raison ; une entrée devenue fantôme le
    /// rendrait aveugle sur un champ retiré.
    ///
    /// **Précondition :** aucun champ ne porte `skip_serializing_if`, donc une
    /// valeur construite tout à `None` sérialise quand même ses huit clés (en
    /// `null`). Le jour où un champ en acquiert un, ce test rougit — et c'est
    /// la bonne lecture : l'audit ne verrait plus la clé passer.
    #[test]
    fn mika2814_les_cles_connues_egalent_les_champs_serialises() {
        let nu = McpServerConfig {
            transport: McpTransport::Stdio,
            command: None,
            args: None,
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade: DataGrade::Operational,
        };

        let valeur = serde_json::to_value(&nu).expect("un McpServerConfig sérialise");
        let objet = valeur
            .as_object()
            .expect("un McpServerConfig sérialise en objet JSON");

        let serialisees: std::collections::BTreeSet<&str> =
            objet.keys().map(String::as_str).collect();
        let declarees: std::collections::BTreeSet<&str> =
            KNOWN_SERVER_KEYS.iter().copied().collect();

        let manquantes: Vec<&str> = serialisees.difference(&declarees).copied().collect();
        assert!(
            manquantes.is_empty(),
            "mika#2814 — {manquantes:?} : champ(s) sérialisé(s) par McpServerConfig et absent(s) de \
             KNOWN_SERVER_KEYS. L'audit avertirait sur une clé valide. Geste : ajouter chaque nom \
             à la const."
        );

        let fantomes: Vec<&str> = declarees.difference(&serialisees).copied().collect();
        assert!(
            fantomes.is_empty(),
            "mika#2814 — {fantomes:?} : entrée(s) de KNOWN_SERVER_KEYS que McpServerConfig ne \
             sérialise plus. L'audit serait aveugle sur le champ retiré. Geste : retirer chaque \
             entrée périmée de la const — ou, si le champ existe toujours, vérifier qu'il n'a pas \
             acquis un `skip_serializing_if` (la précondition de ce test)."
        );
    }

    /// mika#2814 vol 3, cas 1 — `"dataGrade"` est signalée, suggérée, et
    /// **non réparée**.
    ///
    /// La troisième assertion est la porte : un audit qui corrigerait la valeur
    /// serait un audit qui décide. `data_grade` vaut toujours `Operational`
    /// après l'audit, exactement comme avant lui — ce qui change est qu'on le
    /// sait.
    #[test]
    fn mika2814_une_cle_camelcase_est_signalee_avec_sa_suggestion() {
        let json = r#"{
            "mcpServers": {
                "journal-perso": {
                    "transport": "stdio",
                    "command": "npx",
                    "dataGrade": "testimony"
                }
            }
        }"#;
        let brut: serde_json::Value = serde_json::from_str(json).unwrap();
        let audit = collecter_cles_inconnues(&brut);

        assert_eq!(
            audit.inconnues,
            vec![CleInconnue {
                serveur: "journal-perso".to_string(),
                cle: "dataGrade".to_string(),
                suggestion: Some("data_grade"),
            }],
            "mika#2814 — `dataGrade` doit être signalée et suggérer `data_grade` \
             (correspondance après normalisation)."
        );

        let config = McpConfig::depuis_json(json, Path::new("mcp.json")).unwrap();
        assert_eq!(
            config.mcp_servers["journal-perso"].data_grade,
            DataGrade::Operational,
            "mika#2814 — l'audit avertit, il ne répare pas : la valeur mal orthographiée reste \
             ignorée par serde et le grade reste au défaut. Un audit qui réécrirait la valeur \
             devinerait un grade, ce que mika#1798 refuse."
        );
    }

    /// mika#2814 vol 3, cas 2 — une clé sans correspondance normalisée est
    /// signalée **sans** champ `suggestion`.
    ///
    /// `None`, jamais `Some("")` : « aucune suggestion » et « suggestion vide »
    /// ne sont pas le même fait (motif mika#2331), et c'est ce `None` que les
    /// deux arms de `auditer_cles_de_serveur` distinguent — l'un écrit le champ,
    /// l'autre l'omet.
    #[test]
    fn mika2814_une_cle_sans_correspondance_na_pas_de_suggestion() {
        let json = r#"{
            "mcpServers": {
                "wikipedia": {
                    "transport": "http",
                    "url": "https://mcp.wikipedia.org/",
                    "timeoutMs": 30000
                }
            }
        }"#;
        let brut: serde_json::Value = serde_json::from_str(json).unwrap();
        let audit = collecter_cles_inconnues(&brut);

        assert_eq!(
            audit.inconnues,
            vec![CleInconnue {
                serveur: "wikipedia".to_string(),
                cle: "timeoutMs".to_string(),
                suggestion: None,
            }],
            "mika#2814 — `timeoutMs` ne correspond à aucune clé connue après normalisation : \
             signalée, et `suggestion` reste `None` pour que la ligne de journal omette le champ \
             au lieu de l'écrire vide (mika#2331)."
        );
    }

    /// mika#2814 vol 3, cas 3 — une entrée aux huit clés connues ne signale
    /// rien, et le contrôle positif porte un **vrai zéro**.
    ///
    /// `unknown_keys: 0` plutôt que pas de ligne : sans lui, « aucune clé
    /// inconnue » et « le binaire servi précède ce correctif » rendraient les
    /// mêmes octets (classe mika#2340). La présence du champ est structurelle —
    /// il est inconditionnel dans la macro `info!` — donc ce qui s'épingle ici
    /// est la valeur qu'il porte et le compte de serveurs parcourus.
    #[test]
    fn mika2814_une_entree_saine_ne_signale_rien_et_compte_son_serveur() {
        let json = r#"{
            "mcpServers": {
                "complet": {
                    "transport": "stdio",
                    "command": "npx",
                    "args": ["-y", "@exemple/mcp"],
                    "env": {"TOKEN": "x"},
                    "url": null,
                    "headers": null,
                    "enabled": true,
                    "data_grade": "testimony"
                }
            }
        }"#;
        let brut: serde_json::Value = serde_json::from_str(json).unwrap();
        let audit = collecter_cles_inconnues(&brut);

        assert!(
            audit.inconnues.is_empty(),
            "mika#2814 — les huit clés sont connues, rien ne doit être signalé ; relevé : {:?}",
            audit.inconnues
        );
        assert_eq!(
            audit.serveurs, 1,
            "mika#2814 — le parcours a eu lieu sur une entrée, donc le contrôle positif porte \
             `servers: 1` et `unknown_keys: 0` : un vrai zéro, pas une ligne absente (mika#2340)."
        );
    }

    /// mika#2814 vol 3, cas 4 — deux serveurs, une clé inconnue chacun : deux
    /// signalements, chacun nommant **son** serveur.
    ///
    /// Un relevé qui perdrait l'attribution ferait lire à l'opérateur « il y a
    /// deux clés inconnues quelque part » — deux populations sous un seul nom
    /// (classe mika#2205), et rien à corriger.
    #[test]
    fn mika2814_deux_serveurs_signalent_chacun_le_sien() {
        let json = r#"{
            "mcpServers": {
                "alpha": {
                    "transport": "stdio",
                    "command": "a",
                    "dataGrade": "testimony"
                },
                "beta": {
                    "transport": "http",
                    "url": "https://b/",
                    "DATA-GRADE": "testimony"
                }
            }
        }"#;
        let brut: serde_json::Value = serde_json::from_str(json).unwrap();
        let audit = collecter_cles_inconnues(&brut);

        let mut releve: Vec<(&str, &str, Option<&str>)> = audit
            .inconnues
            .iter()
            .map(|i| (i.serveur.as_str(), i.cle.as_str(), i.suggestion))
            .collect();
        releve.sort_unstable();

        assert_eq!(
            releve,
            vec![
                ("alpha", "dataGrade", Some("data_grade")),
                ("beta", "DATA-GRADE", Some("data_grade")),
            ],
            "mika#2814 — un signalement par clé inconnue, chacun portant le nom de son serveur. \
             Un relevé sans attribution ne serait pas actionnable (classe mika#2205)."
        );
        assert_eq!(audit.serveurs, 2);
    }

    /// mika#2814 vol 3, cas 5 — sur un JSON malformé, l'erreur rendue est celle
    /// du parse, **inchangée** par l'audit.
    ///
    /// L'audit s'intercale entre `from_str` et `from_value` : il ne peut donc
    /// pas voir un JSON qui ne parse pas, et ne peut rien ajouter à son erreur.
    /// C'est ce qui garde intact le fail-open de mika#1764, dont le cas réel
    /// était un `{ invalid json` sur un runner CI.
    #[test]
    fn mika2814_un_json_malforme_rend_lerreur_de_parse_inchangee() {
        let malforme = "{ invalid json";
        let source = Path::new("/quelque/part/mcp.json");

        let attendue = serde_json::from_str::<serde_json::Value>(malforme)
            .expect_err("`{ invalid json` ne parse pas")
            .to_string();
        let obtenue = McpConfig::depuis_json(malforme, source)
            .expect_err("depuis_json doit propager l'erreur de parse")
            .to_string();

        assert_eq!(
            obtenue, attendue,
            "mika#2814 — l'audit ne s'interpose pas sur l'erreur de syntaxe : elle sort de \
             `from_str`, avant tout audit, et arrive au `load_operator_shell_or_empty` de \
             mika#1764 telle quelle."
        );
    }
}
