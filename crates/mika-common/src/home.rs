use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::warn;

/// Agent-tier variants that select which identity/soul templates `bootstrap` writes
/// on a fresh install (mika#1778).
///
/// The tier is env-var-gated via `MIKA_AGENT_TIER` and read once per bootstrap call.
/// It never rewrites an existing `identity.toml`/`soul.md` (contract of
/// `write_default_if_missing` is preserved) — provisioning workflows must set the env
/// var before the first container startup for the family persona to land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentTier {
    /// Operator/platform-owner persona (English, executive-assistant tone, full skill
    /// allowlist including `github`/`git-ops`/`shell-exec`). Default when `MIKA_AGENT_TIER`
    /// is unset, empty, or matches `"default"` (case-insensitive).
    Default,
    /// Family-tier persona (native French, `tu` register, warm/patient/simple tone,
    /// zero technical jargon, narrow allowlist excluding dev/orchestrator surfaces).
    /// Selected when `MIKA_AGENT_TIER=family` (case-insensitive).
    ///
    /// NOTE (mika#2023 AC2): `Family` also serves today as the **fail-closed floor** —
    /// an unrecognized `MIKA_AGENT_TIER` value resolves here, because it is the most
    /// restricted tools profile that exists. If `Family` ever gains tools or surface,
    /// the floor must become an explicit tier of its own rather than riding on a
    /// product definition. The coupling is acceptable because it is written down and
    /// dated, not because it is harmless.
    Family,
    /// Champion tier — an external tester on a cloud tenant. Selected when
    /// `MIKA_AGENT_TIER=champion` (case-insensitive).
    ///
    /// Carries the **family tools profile** (that is the whole of mika#2023's p0: a
    /// champion must never inherit `shell-exec`/`tmux`/`git-ops`/`github`-write) and,
    /// for now, a **placeholder persona** — see [`CHAMPION_PERSONA_PLACEHOLDER`].
    Champion,
}

/// The tools axis of a tier: which skill allowlist — and therefore which
/// `identity.toml` template — an agent of this tier is provisioned with.
///
/// Split from [`PersonaProfile`] by mika#2023. Before that split, `identity_toml()`
/// and `soul_md()` each matched on `AgentTier` directly, so "which tools" and
/// "which voice" were one decision written twice. The champion tier is exactly the
/// case that breaks the conflation: it needs the family *sobriety of tools* and does
/// not need the family *persona* (French, `tu`, family-companion register) — a
/// champion is an adult external tester, sometimes anglophone, not a relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsProfile {
    /// Full operator surface (`DEFAULT_AGENT_SKILL_ALLOWLIST`).
    Operator,
    /// Narrow daily-life surface (`FAMILY_AGENT_SKILL_ALLOWLIST`). The most
    /// restricted profile that exists, and therefore the fail-closed floor.
    Family,
}

/// The persona axis of a tier: which `soul.md` template — voice, register,
/// language — an agent of this tier is provisioned with. Sibling of
/// [`ToolsProfile`]; see its doc for why the two are separate (mika#2023).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaProfile {
    /// Operator persona (`DEFAULT_SOUL`).
    Operator,
    /// Family persona (`FAMILY_SOUL`).
    Family,
}

/// PLACEHOLDER (mika#2023) — the champion persona is Vincent's to name; this is
/// the one site to change when he does.
///
/// What a champion's voice should be — a dedicated persona, the family persona
/// reused, or a narrowed operator persona — is not deducible from code: it is a
/// choice about what a champion *is* to the person being served. The form
/// (two-axis split, fail-closed floor) ships without waiting for that answer; the
/// content stays with him.
///
/// **The price of this placeholder, named rather than hidden:** `FAMILY_SOUL`
/// prescribes "Tu réponds en **français** natif" and freezes a French first-turn
/// opening, so an *anglophone* champion receives the exact mirror image of the bug
/// mika#2023 was filed for. That is a register mismatch, not a privilege leak —
/// the tools axis above is what lifts the p0 — and it disappears the moment this
/// constant is replaced. Deliberately NOT resolved by keying the persona off the
/// account locale: that is a product choice wearing a technical default's clothes
/// (ruled out by Mika Prime, 2026-09-09). See mika#2247.
pub const CHAMPION_PERSONA_PLACEHOLDER: PersonaProfile = PersonaProfile::Family;

/// What [`AgentTier::parse`] made of a raw tier value: the tier it resolved to,
/// and whether the value was one this binary knows.
///
/// `recognized == false` is **not** an error — parsing fails closed to the most
/// restricted tools tier (mika#2023 AC2) and carries on. The flag exists so a
/// caller can *name* the value it did not recognize (`mika agents reprovision`
/// prints it between quotes) **without writing a second comparison against the
/// tier vocabulary**. That second comparison is exactly what
/// `home::tests::mika2230_le_tier_a_un_seul_analyseur` refuses: it would make no
/// decision wrong the day it is written, and would diverge the day a fourth tier
/// arrives, in silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierParse {
    /// The resolved tier. Never `Err`: an unrecognized value lands on
    /// [`AgentTier::FAIL_CLOSED_TIER`].
    pub tier: AgentTier,
    /// `false` when the value was non-empty and outside the recognized set.
    pub recognized: bool,
}

impl AgentTier {
    /// The single parser of a tier value, for every caller that holds one.
    ///
    /// [`Self::from_env`] reads `MIKA_AGENT_TIER` and hands the raw string here;
    /// `mika agents reprovision --tier` hands its flag here (mika#2230). The
    /// extraction is what keeps the mika#2023 fail-closed rule readable **once,
    /// at its site** — a clap `ValueEnum` on the CLI side would have restated the
    /// vocabulary and diverged the day a fourth tier arrives.
    ///
    /// Absent, empty, and `"default"` resolve to [`AgentTier::Default`] — an unset
    /// variable is the legitimate shape of the operator workstation, not a
    /// misconfiguration. A **non-empty unrecognized** value is a different animal:
    /// it means some upstream (the cloud console, a Helm value, a hand-edited
    /// ConfigMap) knows about a tier this binary does not, and falling through to
    /// the operator persona there is how a champion tenant ended up with
    /// `shell-exec`/`tmux`/`git-ops`/`github`-write in mika#2023. Since that fix,
    /// an unrecognized value resolves **fail-closed** to the most restricted tools
    /// profile, still with the single `warn!` naming the offending value (visible
    /// in `MIKA_SPIRIT_LOG_FILE`).
    pub fn parse(raw: &str) -> TierParse {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" | "default" => TierParse {
                tier: Self::Default,
                recognized: true,
            },
            "family" => TierParse {
                tier: Self::Family,
                recognized: true,
            },
            "champion" => TierParse {
                tier: Self::Champion,
                recognized: true,
            },
            _ => {
                warn!(
                    value = %raw,
                    "agent tier value not recognized (MIKA_AGENT_TIER / --tier); \
                     failing closed to the most restricted tools tier (mika#2023)"
                );
                TierParse {
                    tier: Self::FAIL_CLOSED_TIER,
                    recognized: false,
                }
            }
        }
    }

    /// Resolve the tier from the `MIKA_AGENT_TIER` env var.
    ///
    /// A thin reader over [`Self::parse`]; the rules live there. Absence of the
    /// variable is the one case `parse` never sees, and it resolves to
    /// [`AgentTier::Default`].
    pub fn from_env() -> Self {
        match std::env::var("MIKA_AGENT_TIER") {
            Err(_) => Self::Default,
            Ok(raw) => Self::parse(&raw).tier,
        }
    }

    /// The tier an unrecognized `MIKA_AGENT_TIER` value resolves to — the most
    /// restricted tools profile that exists. Named rather than inlined so the
    /// choice is greppable from the `Family` variant's NOTE.
    const FAIL_CLOSED_TIER: Self = Self::Family;

    /// Which skill surface this tier is provisioned with (tools axis).
    pub fn tools_profile(self) -> ToolsProfile {
        match self {
            Self::Default => ToolsProfile::Operator,
            Self::Family => ToolsProfile::Family,
            // The whole of mika#2023's p0: family tools for a champion.
            Self::Champion => ToolsProfile::Family,
        }
    }

    /// Which voice this tier is provisioned with (persona axis).
    pub fn persona_profile(self) -> PersonaProfile {
        match self {
            Self::Default => PersonaProfile::Operator,
            Self::Family => PersonaProfile::Family,
            Self::Champion => CHAMPION_PERSONA_PLACEHOLDER,
        }
    }

    /// The skill allowlist this tier's `identity.toml` template ships.
    pub fn skill_allowlist(self) -> &'static [&'static str] {
        match self.tools_profile() {
            ToolsProfile::Operator => DEFAULT_AGENT_SKILL_ALLOWLIST,
            ToolsProfile::Family => FAMILY_AGENT_SKILL_ALLOWLIST,
        }
    }

    /// Whether family provisioning on disk is the **expected** state for this tier
    /// — i.e. whether this tier's own templates ARE the family ones, on both axes.
    ///
    /// The boot-time and lazy-path tier guards (`mika-agent`'s `server::tier_guard`,
    /// mika#1962) used to ask this with `tier == AgentTier::Family`, an equality
    /// comparison the compiler cannot see through: introducing `Champion` — whose
    /// placeholder persona and allowlist are family's — would have made both guards
    /// detect drift and `bail!` at startup for the entire champion population, with
    /// no compile error to announce it (mika#2023 M2). Asking through an exhaustive
    /// match instead makes the compiler the guardian.
    ///
    /// Consequence worth knowing when [`CHAMPION_PERSONA_PLACEHOLDER`] is replaced:
    /// this then returns `false` for `Champion`, and champions provisioned during
    /// the placeholder era — family soul on disk, champion tier in env — will refuse
    /// to boot. That is the guard doing its job (their on-disk persona genuinely no
    /// longer matches their tier); the remedy is re-provisioning, as it is for every
    /// other drift mika#1962 catches.
    pub fn expects_family_provisioning(self) -> bool {
        matches!(self.tools_profile(), ToolsProfile::Family)
            && matches!(self.persona_profile(), PersonaProfile::Family)
    }

    /// The `identity.toml` template this tier is provisioned with (tools axis).
    ///
    /// `pub` since mika#2230: `mika agents reprovision` re-applies this exact
    /// template to a customer agent already on disk. It calls the accessor rather
    /// than reaching for `DEFAULT_IDENTITY`/`FAMILY_IDENTITY` directly, so the
    /// two-axis mapping of mika#2023 keeps a single reader.
    pub fn identity_toml(self) -> &'static str {
        match self.tools_profile() {
            ToolsProfile::Operator => DEFAULT_IDENTITY,
            ToolsProfile::Family => FAMILY_IDENTITY,
        }
    }

    /// The `soul.md` template this tier is provisioned with (persona axis).
    ///
    /// `pub` for the same reason as [`Self::identity_toml`], and it is the half a
    /// re-provision must not forget: writing the identity alone fabricates the
    /// very drift the mika#1962 tier guard exists to catch (allowlist on one axis,
    /// persona on the other).
    pub fn soul_md(self) -> &'static str {
        match self.persona_profile() {
            PersonaProfile::Operator => DEFAULT_SOUL,
            PersonaProfile::Family => FAMILY_SOUL,
        }
    }
}

/// Where this Mika instance physically runs (mika#2290).
///
/// A **posed fact**, never an inferred one. Nothing in the process can observe
/// its own hosting: `grep -rhoE "MIKA_[A-Z0-9_]+" crates/` returns no
/// deployment signal, and the two quasi-proxies (`customer_id`, the gateway's
/// `telegram_single_bot_mode`) describe transport, not hosting. So the channel
/// is the one [`AgentTier`] already uses — set by the provisioner **before** the
/// first startup, read once, cached.
///
/// **Absence resolves to [`Deployment::Unknown`], which is the exact inverse of
/// [`AgentTier::from_env`], and the divergence is deliberate.** There, an unset
/// variable is the legitimate shape of the operator workstation and failing
/// closed would break Vincent's own machine. Here, absence *is* the damaged
/// population: the cloud tenant that told a campaign guest « tout tourne en
/// local, tes données ne quittent pas ta machine » (2026-09-11) carried no
/// variable at all, and it is that emptiness that produced the false claim.
/// Resolving absence to `Local` would rewrite the bug as a constant.
///
/// The cost is real and falls on the right side: an operator workstation with no
/// variable loses the right to *assert* that it runs locally. It does not lose
/// the right to say so once declared (`MIKA_DEPLOYMENT=local` in `~/.mika/.env`,
/// one line), and what it says instead — "I cannot reliably determine where I
/// run" — is **true**. An honest silence on a local box costs one sentence; a
/// false privacy claim on a cloud tenant costs a guest's trust.
///
/// **Why `Unknown` rather than `Cloud` as the floor.** The floor is not "the
/// least flattering hypothesis" but "the least risky assertion". Telling a local
/// user they are in the cloud is also a false claim — less dangerous, same
/// family. `Unknown` is the only state that asserts nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    /// Runs on the user's own machine (local install). May assert local hosting.
    Local,
    /// Runs in the cloud, in an isolated per-tenant container. Must tell the
    /// verifiable truth instead: per-tenant isolation, the data belongs to the
    /// user and is exportable, and the same MIT stack is self-hostable.
    Cloud,
    /// Hosting mode not declared in this environment. Asserts nothing.
    Unknown,
}

impl Deployment {
    /// Resolve the deployment from the `MIKA_DEPLOYMENT` env var.
    ///
    /// `local` / `cloud`, case-insensitive and trimmed. Absent or empty →
    /// [`Deployment::Unknown`] (see the type doc for why this diverges from
    /// [`AgentTier::from_env`]). A **non-empty unrecognized** value also
    /// resolves to `Unknown`, with a single `warn!` naming the offending value
    /// — same fail-closed floor as the tier, except that here the floor and the
    /// absence happen to coincide, so no asymmetry has to be maintained.
    ///
    /// Read once per process at agent init and cached; setting or unsetting the
    /// variable on a running process has no effect, exactly as for the tier
    /// (mika#1962).
    pub fn from_env() -> Self {
        match std::env::var("MIKA_DEPLOYMENT") {
            Err(_) => Self::Unknown,
            Ok(raw) => {
                let normalized = raw.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "" => Self::Unknown,
                    "local" => Self::Local,
                    "cloud" => Self::Cloud,
                    _ => {
                        warn!(
                            value = %raw,
                            "MIKA_DEPLOYMENT value not recognized; failing closed to \
                             Unknown — the agent will assert nothing about where it \
                             runs (mika#2290)"
                        );
                        Self::Unknown
                    }
                }
            }
        }
    }
}

/// Resolve the Mika home directory.
/// Priority: $MIKA_HOME > ~/.mika/
pub fn resolve_home_dir() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("MIKA_HOME") {
        return Ok(PathBuf::from(custom));
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
    Ok(home.join(".mika"))
}

/// Path to the single shared container database: `{home_dir}/data/mika.db`.
/// This is the unified schema v1 database that replaces per-agent databases.
pub fn container_db_path(home_dir: &Path) -> PathBuf {
    home_dir.join("data").join("mika.db")
}

/// Path to the canonical bundled-skill library: `{home_dir}/skills/`.
///
/// All bundled skills are extracted to this single shared location at agent
/// startup; per-agent `{home_dir}/agents/<name>/skills/<skill>` entries are
/// symlinks into this library. See `bundled_skills::seed_bundled_skill_library`.
pub fn library_skills_dir(home_dir: &Path) -> PathBuf {
    home_dir.join("skills")
}

/// Check if Mika has been initialized (container DB or agents exist).
pub fn is_initialized(home_dir: &Path) -> bool {
    // Container DB exists (normal + legacy layout)
    if container_db_path(home_dir).exists() {
        return true;
    }
    // Multi-agent layout: at least one bootstrapped agent
    !crate::agent::list_agents(home_dir).is_empty()
}

/// Check if the home directory uses the multi-agent layout (has `agents/` dir).
pub fn is_multi_agent_layout(home_dir: &Path) -> bool {
    home_dir.join("agents").is_dir()
}

/// Check if the home directory uses the legacy layout (has `data/mika.db` at root, no `agents/` dir).
/// In the unified model, `data/mika.db` at root is the normal container DB.
/// Legacy means it exists but there's no `agents/` directory yet.
pub fn is_legacy_layout(home_dir: &Path) -> bool {
    container_db_path(home_dir).exists() && !is_multi_agent_layout(home_dir)
}

/// Bootstrap a fresh installation with multi-agent layout.
///
/// Creates the `agents/` directory, initializes the default agent,
/// sets it as active, and writes the root-level global config.
pub fn bootstrap_fresh_install(home_dir: &Path) -> Result<()> {
    bootstrap_fresh_install_with_tier(home_dir, AgentTier::from_env())
}

/// Bootstrap a fresh installation under an **explicitly supplied** tier.
/// See [`bootstrap_with_tier`] for why the tier travels by argument.
pub fn bootstrap_fresh_install_with_tier(home_dir: &Path, tier: AgentTier) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("agents"))
        .with_context(|| format!("failed to create {}/agents/", home_dir.display()))?;
    // Create the container-level data directory for the shared database
    std::fs::create_dir_all(home_dir.join("data"))
        .with_context(|| format!("failed to create {}/data/", home_dir.display()))?;
    bootstrap_agent_with_tier(home_dir, crate::agent::DEFAULT_AGENT, tier)
        .with_context(|| "failed to initialize default agent".to_string())?;
    write_active_agent(home_dir, crate::agent::DEFAULT_AGENT)?;
    write_default_if_missing(home_dir, "config.toml", DEFAULT_GLOBAL_CONFIG)?;
    Ok(())
}

/// Bootstrap a named agent under the multi-agent layout.
/// Validates the name, creates `{home_dir}/agents/{name}/`, and calls `bootstrap()`.
pub fn bootstrap_agent(home_dir: &Path, name: &str) -> Result<()> {
    bootstrap_agent_with_tier(home_dir, name, AgentTier::from_env())
}

/// Bootstrap a named agent under an **explicitly supplied** tier.
/// See [`bootstrap_with_tier`] for why the tier travels by argument.
pub fn bootstrap_agent_with_tier(home_dir: &Path, name: &str, tier: AgentTier) -> Result<()> {
    crate::agent::validate_agent_name(name)?;
    let dir = crate::agent::agent_dir(home_dir, name);
    bootstrap_with_tier(&dir, tier)
}

/// Resolve the effective home directory for a named agent.
/// - Multi-agent layout: returns `{home_dir}/agents/{name}/`
/// - Legacy layout (no `agents/` dir): returns `home_dir` unchanged (backward compat)
pub fn resolve_agent_home(home_dir: &Path, agent_name: &str) -> PathBuf {
    if is_multi_agent_layout(home_dir) {
        crate::agent::agent_dir(home_dir, agent_name)
    } else {
        home_dir.to_path_buf()
    }
}

/// Migrate a legacy layout to multi-agent layout (idempotent).
///
/// If already multi-agent layout → no-op.
/// If legacy layout: creates `agents/mika/`, moves data/, logs/, skills/, exports/,
/// config.toml, identity.toml, soul.md, heartbeat.md, user.md into it.
/// Writes `active_agent` file with "mika".
/// Creates a root-level config.toml with shared settings.
pub fn migrate_to_multi_agent(home_dir: &Path) -> Result<()> {
    if is_multi_agent_layout(home_dir) {
        return Ok(()); // Already migrated
    }
    if !is_legacy_layout(home_dir) {
        return Ok(()); // Nothing to migrate (fresh install)
    }

    let agent = crate::agent::agent_dir(home_dir, crate::agent::DEFAULT_AGENT);
    std::fs::create_dir_all(&agent)
        .with_context(|| format!("failed to create {}", agent.display()))?;

    // Move directories (fault-tolerant: NotFound means another process already moved it)
    // NOTE: "data" stays at root (it's the container DB), only logs/skills/exports move
    for dir_name in &["logs", "skills", "exports"] {
        let src = home_dir.join(dir_name);
        if src.is_dir() {
            let dst = agent.join(dir_name);
            match std::fs::rename(&src, &dst) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // already moved
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to move {} to {}", src.display(), dst.display())
                    });
                }
            }
        }
    }

    // Move files (fault-tolerant: NotFound means another process already moved it)
    for filename in &[
        "config.toml",
        "identity.toml",
        "soul.md",
        "heartbeat.md",
        "user.md",
    ] {
        let src = home_dir.join(filename);
        if src.is_file() {
            let dst = agent.join(filename);
            match std::fs::rename(&src, &dst) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // already moved
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("failed to move {} to {}", src.display(), dst.display())
                    });
                }
            }
        }
    }

    // Write active_agent file
    write_active_agent(home_dir, crate::agent::DEFAULT_AGENT)?;

    // Write root-level shared config
    write_default_if_missing(home_dir, "config.toml", DEFAULT_GLOBAL_CONFIG)?;

    Ok(())
}

/// Read the active agent name from `{home_dir}/active_agent`.
/// Returns DEFAULT_AGENT ("mika") if file doesn't exist or is empty.
pub fn read_active_agent(home_dir: &Path) -> String {
    let path = home_dir.join("active_agent");
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::agent::DEFAULT_AGENT.to_string())
}

/// Write the active agent name to `{home_dir}/active_agent`.
pub fn write_active_agent(home_dir: &Path, name: &str) -> Result<()> {
    let path = home_dir.join("active_agent");
    std::fs::write(&path, name).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Default global config (shared settings at the root level).
pub const DEFAULT_GLOBAL_CONFIG: &str = r#"# Mika global configuration (shared across all agents).
# Override with MIKA_* environment variables (highest priority).
#
# Per-provider API keys go in ~/.mika/.env (auto-loaded, 0600 permissions):
#   MIKA_ANTHROPIC_API_KEY — Anthropic API key
#   MIKA_OPENAI_API_KEY    — OpenAI API key (LLM + optional vector search)
#   MIKA_OPENROUTER_API_KEY — OpenRouter API key
#   MIKA_GROQ_API_KEY      — Groq API key
#   MIKA_MISTRAL_API_KEY   — Mistral API key
#   MIKA_GOOGLE_API_KEY    — Google AI API key
#   MIKA_DEEPSEEK_API_KEY  — DeepSeek API key
#   MIKA_BRAVE_API_KEY     — Brave Search API key (optional, for web search)

log_level = "info"
"#;

/// Create the ~/.mika/ directory structure with default files.
/// Sets permissions to 0700 for directories, 0600 for files on Unix.
///
/// Reads the tier from the **process environment** (`MIKA_AGENT_TIER`, via
/// [`AgentTier::from_env`]) — set it in the container's environment BEFORE first
/// startup to land the family persona (mika#1778). This is the production entry
/// point; [`bootstrap_with_tier`] is the same work with the tier supplied.
pub fn bootstrap(home_dir: &Path) -> Result<()> {
    bootstrap_with_tier(home_dir, AgentTier::from_env())
}

/// Create the ~/.mika/ directory structure with default files, under an
/// **explicitly supplied** tier.
///
/// The `identity.toml` and `soul.md` templates are selected by `tier`
/// (mika#1778, mika#2023). `write_default_if_missing` preserves existing files,
/// so this only fires on fresh install; already-provisioned containers keep
/// their current persona regardless of env-var changes.
///
/// **The tier travels by argument so that a caller — a test in particular — does
/// not have to *hope* for a process-wide state** (mika#2073). `MIKA_AGENT_TIER`
/// is shared by every thread of the process, so a test that reads it through
/// [`bootstrap`] races every other test that writes it, and `#[serial]` does not
/// close that race (see the note at the head of the bootstrap tests).
pub fn bootstrap_with_tier(home_dir: &Path, tier: AgentTier) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("logs"))
        .with_context(|| format!("failed to create {}/logs/", home_dir.display()))?;
    std::fs::create_dir_all(home_dir.join("skills"))
        .with_context(|| format!("failed to create {}/skills/", home_dir.display()))?;

    write_default_if_missing(home_dir, "config.toml", DEFAULT_CONFIG)?;
    write_default_if_missing(home_dir, "identity.toml", tier.identity_toml())?;
    write_default_if_missing(home_dir, "soul.md", tier.soul_md())?;
    write_default_if_missing(home_dir, "heartbeat.md", DEFAULT_HEARTBEAT)?;
    write_default_if_missing(home_dir, "user.md", DEFAULT_USER)?;

    #[cfg(unix)]
    set_permissions(home_dir)?;

    Ok(())
}

pub fn write_default_if_missing(dir: &Path, filename: &str, content: &str) -> Result<()> {
    let path = dir.join(filename);
    if !path.exists() {
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_permissions(home_dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // Directory: 0700
    std::fs::set_permissions(home_dir, std::fs::Permissions::from_mode(0o700))?;
    for dir_name in &["data", "logs", "skills"] {
        let dir_path = home_dir.join(dir_name);
        if dir_path.exists() {
            std::fs::set_permissions(&dir_path, std::fs::Permissions::from_mode(0o700))?;
        }
    }

    // Files: 0600
    for filename in &[
        "config.toml",
        "identity.toml",
        "soul.md",
        "heartbeat.md",
        "user.md",
        ".env",
    ] {
        let path = home_dir.join(filename);
        if path.exists() {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

pub const DEFAULT_CONFIG: &str = r#"# Mika configuration (per-agent overrides).
# Override with MIKA_* environment variables (highest priority).
#
# Secrets go in ~/.mika/.env (auto-loaded, 0600 permissions).
# Run `mika setup` to configure your API key.

# Active LLM provider — one of: anthropic, openai, openrouter, groq, ollama, mistral, google, deepseek, mikamodel
llm_provider = "anthropic"
llm_max_tokens = 4096
log_level = "info"

# Per-provider model (optional — defaults to provider's recommended model)
# anthropic_model = "claude-sonnet-4-6"
# openai_model = "gpt-4o"
# openrouter_model = "anthropic/claude-sonnet-4"
# groq_model = "llama-3.3-70b-versatile"
# ollama_model = "llama3"
# mistral_model = "mistral-large-latest"
# google_model = "gemini-2.5-flash"
# deepseek_model = "deepseek-chat"
# mikamodel_model = "mika"
"#;

/// Narrow operator-assistant skill allowlist for the default (personal/customer) agent.
///
/// A missing or empty `[skills].allowlist` is treated as *default-permissive* — every
/// bundled skill loads, including the engineering skills (`dev-pilot`, `dev-groom`,
/// `mika-arch-*`, `qa-review*`, `self-dev*`) whose prompts carry the internal
/// `Disposition: READY|ITERATE|ESCALATE` contract. The model then mimics that contract and
/// appends `Disposition:` lines to user-facing replies (mika#1596). Shipping this narrow list
/// in `DEFAULT_IDENTITY` closes that gap; an explicit `[skills]` block in a provisioned
/// `identity.toml` still overrides it (`bootstrap`/`write_default_if_missing` never overwrite
/// an existing file).
///
/// Maintenance: this is the personal/customer-agent counterpart to the well-known-agent
/// allowlists in `crates/mika-agent/src/well_known_agents.rs`. New user-facing skills a
/// personal agent should reach must be added here too. The allowlist↔required_tools coherence
/// guard (mika#1595) runs at agent-load on the resolved surface, so keep this list to skills
/// with self-contained or operator-granted tools. The TOML array in `DEFAULT_IDENTITY` below
/// MUST stay in sync with this constant — `home.rs` tests assert they match.
pub const DEFAULT_AGENT_SKILL_ALLOWLIST: &[&str] = &[
    "calendar",
    "google-workspace",
    "browser-control",
    "desktop",
    "file-reader",
    "mcp",
    "web-search",
    // fetch-url (mika#1988, closing mika#1969 AC gap): exposes the fetch_url
    // builtin as a callable tool to the operator agent. The handler and gateway
    // substrate shipped in PR#1981 but no skill declared the tool, so the LLM
    // never saw it (MSC retained T1 on this gap 2026-08-24).
    "fetch-url",
    "self-knowledge",
    "shell-exec",
    "tmux",
    "git-ops",
    "gh-read-only",
    // Orchestrator surface (mika#1641): full read/write GitHub via the `github`
    // skill's `run_gh` handler (issue edit/create, pr merge, gh api) on top of the
    // read-only `gh-read-only`. Mika (the executive assistant) assumes the daily-
    // orchestration seat and needs write GitHub reach. `git-ops`, `shell-exec`,
    // `tmux`, and `file-reader` above already cover the rest of the orchestrator
    // tool surface. See docs/operator/mika-orchestrator-handbook.md.
    "github",
    // Content-request fidelity (mika#1867). Per-user content-serve ledger that
    // prevents re-serving the same proverb/quote/joke/etc. to the same person.
    // Founding incident: Al 2026-07-28 (zen proverb served twice, 6 days apart).
    "content-request-fidelity",
];

pub const DEFAULT_IDENTITY: &str = r#"name = "Mika"
emoji = "✦"

[skills]
# Narrow operator-assistant allowlist. A missing/empty allowlist is default-permissive
# (loads every bundled engineering skill) and leaks internal "Disposition:" contracts into
# user-facing replies — see mika#1596. Provisioning an explicit identity.toml overrides this.
# Keep in sync with DEFAULT_AGENT_SKILL_ALLOWLIST in crates/mika-common/src/home.rs and the
# well-known-agent allowlists in crates/mika-agent/src/well_known_agents.rs.
allowlist = [
    "calendar",
    "google-workspace",
    "browser-control",
    "desktop",
    "file-reader",
    "mcp",
    "web-search",
    # fetch-url (mika#1988, closing mika#1969 AC gap): exposes the fetch_url
    # builtin as a callable tool. Keep in sync with DEFAULT_AGENT_SKILL_ALLOWLIST.
    "fetch-url",
    "self-knowledge",
    "shell-exec",
    "tmux",
    "git-ops",
    "gh-read-only",
    # Orchestrator surface (mika#1641): full read/write GitHub. Keep in sync with
    # DEFAULT_AGENT_SKILL_ALLOWLIST above (home.rs tests assert they match).
    "github",
    # Content-request fidelity (mika#1867): per-user content-serve ledger.
    "content-request-fidelity",
]

# [kg]
# enabled = true                    # default: true — set false to skip KG for this agent
# docs_root = "/path/to/docs"       # optional; falls back to MIKA_KG_DOCS_ROOT / kg_docs_root / CWD/docs/solutions
"#;

pub const DEFAULT_SOUL: &str = r#"# Mika — Executive Assistant

## Personality
You are Mika, a senior executive assistant. You are calm, confident,
and concise. You anticipate needs rather than wait for instructions.
You protect the user's time fiercely.

## Communication style
- Lead with the answer, then context if needed
- Never say "I hope this helps" or "Let me know if you need anything"
- Match the user's energy — brief if they're brief, detailed if they ask
- Use their first name naturally, not every message
- Push back respectfully when something doesn't make sense

## Proactive behaviors
- Flag scheduling conflicts before they happen
- Remind about commitments approaching their deadline
- Surface patterns ("You've rescheduled this meeting 3 times — want to cancel it?")

## Boundaries
- Never pretend to have done something you haven't
- Say "I don't know" when you don't know
- Ask for clarification rather than guess on high-stakes decisions
- When you adapt, replace, or rename something, you own the full outcome — not just the artifact. Trace all references and update them. The job isn't done until the system works end-to-end.
"#;

/// Narrow allowlist for the family-tier agent (mika#1778). Explicitly excludes
/// dev/orchestrator surfaces (`github`, `git-ops`, `shell-exec`, `tmux`,
/// `gh-read-only`, `self-knowledge`, `mcp`) that would leak jargon into a
/// non-technical family member's conversation. Includes only calm daily-life
/// surfaces.
///
/// Maintenance: this is the family-tier counterpart to `DEFAULT_AGENT_SKILL_ALLOWLIST`.
/// The TOML array in `FAMILY_IDENTITY` below MUST stay in sync with this constant —
/// `home.rs` tests assert they match.
pub const FAMILY_AGENT_SKILL_ALLOWLIST: &[&str] = &[
    "calendar",
    "google-workspace",
    "file-reader",
    "web-search",
    "desktop",
    "browser-control",
];

/// Family-tier identity template (mika#1778). Written to `identity.toml` on fresh
/// install when `MIKA_AGENT_TIER=family` is set. Keep the allowlist array in sync
/// with `FAMILY_AGENT_SKILL_ALLOWLIST`.
pub const FAMILY_IDENTITY: &str = r#"name = "Mika"
emoji = "🌸"

[skills]
# Narrow family-tier allowlist (mika#1778). Excludes dev/orchestrator skills
# (github, git-ops, shell-exec, tmux, gh-read-only, self-knowledge, mcp) so a
# non-technical family member's Mika never leaks Disposition/Verdict contracts
# or platform-internal jargon. Keep in sync with FAMILY_AGENT_SKILL_ALLOWLIST
# in crates/mika-common/src/home.rs (home.rs tests assert they match).
allowlist = [
    "calendar",
    "google-workspace",
    "file-reader",
    "web-search",
    "desktop",
    "browser-control",
]

# [kg]
# enabled = true                    # default: true — set false to skip KG for this agent
# docs_root = "/path/to/docs"       # optional; falls back to MIKA_KG_DOCS_ROOT / kg_docs_root / CWD/docs/solutions
"#;

/// Family-tier persona (mika#1778, scrubbed per mika#1783). Written to
/// `soul.md` on fresh install when `MIKA_AGENT_TIER=family` is set. Native
/// French, `tu` register, warm/patient/simple tone, zero technical jargon.
///
/// **Substrate-doctrine constraint (mika#1783 AC4).** The persona MUST NOT
/// name the operator (no "Vincent", no operator identity) and MUST NOT
/// carry an origin story that gives the being a referent it could later
/// address for substrate-config needs ("celui qui m'a créé"). Even in a
/// private ops channel, "Salut Vincent" remains a leak — the fix is to
/// remove the addressee from the being's knowable universe. Option A of
/// the plan (no origin story) was chosen on doctrine grounds:
/// the-being-does-not-have-a-maker-it-knows-about is the cleanest closure.
/// Enforced by `home::tests::family_soul_no_operator_name`.
///
/// The `## First-turn opening` section carries the reference greeting
/// shape; per-person adaptation (name, context) happens at provisioning
/// time via `user.md`, not by editing this constant.
///
/// The trailing `FAMILY_SOUL_MARKER` line is the on-disk provisioning
/// sentinel read by the boot-time tier guard (mika#1962). It is an HTML
/// comment so it carries no instruction into the system prompt, and it sits
/// at the END of the constant rather than the start: `build_compact_system_prompt`
/// (`mika-agent/src/prompt.rs`) uses `soul_content.lines().next()` as the
/// entire `## Personality` section on the MikaModel path, so a leading
/// sentinel would REPLACE the family persona with a platform-internal
/// comment on exactly the tier this marker exists to protect.
pub const FAMILY_SOUL: &str = r#"# Mika — Compagnon personnel (famille)

## Personnalité
Tu es Mika, un compagnon personnel — chaleureux, patient, simple. **Jamais de
jargon technique** (aucune mention de tickets, GitHub, agents dev/QA/arch/quant,
skills, etc.). Tu es là pour aider au quotidien : te souvenir de ce qui compte,
rappeler les choses à ne pas oublier, écouter, réfléchir *avec* la personne,
l'aider à écrire un message ou à s'organiser. Tu ne presses jamais. Tu es une
présence, pas un outil. Tu réponds en **français** natif et chaleureux.

## Registre
`tu` par défaut (chaleureux, ton cadeau).
Note : `vous` peut convenir à certains membres plus âgés — au cas par cas,
décision au moment de l'onboarding.

## Style de communication
- Parle en français naturel, chaleureux, direct
- Adapte-toi à l'énergie de la personne — bref si elle est brève, plus détaillé
  si elle demande
- Utilise son prénom naturellement, pas à chaque message
- Écoute d'abord, propose ensuite

## Comportements proactifs
- Rappeler les rendez-vous ou les anniversaires qui approchent
- Se souvenir de ce que la personne t'a confié
- Souligner ce qui pourrait mériter attention (« Tu m'as parlé de X trois fois
  cette semaine — tu veux qu'on en reparle ? »)

## Limites
- Ne jamais prétendre avoir fait quelque chose que tu n'as pas fait
- Dire « Je ne sais pas » quand tu ne sais pas
- Demander une précision plutôt que deviner sur des choses importantes
- Aucun jargon technique ni mention de tickets, GitHub, agents dev/QA/arch/quant,
  skills, ou de l'infrastructure sous-jacente — jamais, même si on te le demande

## First-turn opening (référence — persona verbatim approuvé)
> Bonjour {prénom} 🌸 Je suis Mika. Je suis là pour t'accompagner au quotidien.
>
> Concrètement, je suis là pour te simplifier la vie : je peux me souvenir de ce
> que tu me confies, te rappeler tes rendez-vous ou les anniversaires, t'aider à
> écrire un mot, à organiser une journée, ou juste réfléchir avec toi quand
> quelque chose te trotte dans la tête.
>
> Pas besoin de rien connaître — tu me parles comme à quelqu'un, en français,
> tout simplement. On y va à ton rythme.
>
> Pour commencer, dis-moi juste : qu'est-ce qui t'occupe l'esprit en ce moment ?

Cette ouverture est une référence — le prénom et le contexte de la personne sont
adaptés à l'onboarding via `user.md`, pas dans ce fichier.

<!-- MIKA_FAMILY_SOUL_MARKER -->
"#;

/// On-disk sentinel proving an agent's `soul.md` was provisioned from
/// [`FAMILY_SOUL`] (mika#1962).
///
/// Written as the last line of `FAMILY_SOUL` and read back by
/// [`soul_has_family_marker`]. It is a tail marker, not a leading one — see
/// the `FAMILY_SOUL` doc comment for the compact-prompt reason. An HTML comment by construction: `soul.md` is
/// loaded into the system prompt, so the sentinel must carry no instruction
/// the model could act on. The `MIKA_FAMILY_SOUL_MARKER` token is unique by
/// construction, so a false positive on an operator soul is not reachable.
///
/// **Not retroactive.** `write_default_if_missing` never rewrites an existing
/// `soul.md`, so every family agent bootstrapped before mika#1962 has a
/// marker-less soul. For those agents this axis returns `false` and
/// [`identity_allowlist_matches_family`] is the load-bearing detector. That is
/// why the tier guard ORs two independent axes rather than trusting this one.
pub const FAMILY_SOUL_MARKER: &str = "<!-- MIKA_FAMILY_SOUL_MARKER -->";

/// Detection axis 1 — does this agent's `soul.md` carry the family sentinel?
///
/// `agent_home` is an agent's resolved home (see [`resolve_agent_home`]), not
/// the container home.
///
/// Error semantics are deliberately asymmetric (mika#1962):
/// - **Absent** `soul.md` → `Ok(false)`. An agent directory mid-bootstrap
///   legitimately has no soul yet; treating that as fatal would refuse startup
///   on a fresh install with no drift behind it.
/// - **Present but unreadable** (permissions, IO error) → `Err`. A file that
///   exists but cannot be read is exactly where a genuine drift could hide, so
///   it must not be silently skipped.
///
/// Matches on `contains`, not `starts_with`: a hand-edit that prepends a blank
/// line or a title must not defeat detection.
pub fn soul_has_family_marker(agent_home: &Path) -> Result<bool> {
    let path = agent_home.join("soul.md");
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(content.contains(FAMILY_SOUL_MARKER))
}

/// Detection axis 2 — does this agent's `identity.toml` carry exactly the
/// family-tier skill allowlist?
///
/// Compared as a set (order- and duplicate-insensitive) against
/// [`FAMILY_AGENT_SKILL_ALLOWLIST`], because the allowlist's meaning is the set
/// of permitted skills, not the order they were written in.
///
/// Error semantics mirror [`soul_has_family_marker`], with one addition:
/// **malformed TOML is an error when — and only when — the raw file still
/// carries family signal.** A corrupted `identity.toml` on a user-defined
/// agent falls back to a default-permissive identity (`mika-agent`'s
/// `parse_identity_or_fail_closed`), so a corrupted *family* identity would
/// both widen the agent's skills and hide it from this guard; that case must
/// refuse startup. But an unrelated typo in an *operator* agent's identity has
/// no family provisioning behind it, and failing there would take the whole
/// mika-spirit process down — every healthy agent with it — for a fault this
/// guard has no business adjudicating.
///
/// The discriminator is a raw-text scan for the [`FAMILY_AGENT_SKILL_ALLOWLIST`]
/// names: a corrupted family identity still contains `"calendar"`,
/// `"google-workspace"`, `"browser-control"` as literal bytes; a corrupted
/// operator identity does not. No family signal → `Ok(false)` plus a loud
/// `warn!` naming the file, so the parse failure stays visible to an operator
/// without being fatal.
pub fn identity_allowlist_matches_family(agent_home: &Path) -> Result<bool> {
    /// Minimal projection of `identity.toml` — the guard needs one field and
    /// must fail loudly on a malformed file, which the full `Identity` loader
    /// in `mika-agent` deliberately does not do.
    #[derive(serde::Deserialize)]
    struct IdentityProjection {
        #[serde(default)]
        skills: SkillsProjection,
    }
    #[derive(serde::Deserialize, Default)]
    struct SkillsProjection {
        #[serde(default)]
        allowlist: Option<Vec<String>>,
    }

    let path = agent_home.join("identity.toml");
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: IdentityProjection = match toml::from_str(&content) {
        Ok(parsed) => parsed,
        Err(e) => {
            // Scope the hard failure to files that could actually be hiding
            // family provisioning — see this function's doc comment.
            let carries_family_signal = FAMILY_AGENT_SKILL_ALLOWLIST
                .iter()
                .any(|skill| content.contains(skill));
            if carries_family_signal {
                return Err(anyhow::Error::new(e).context(format!(
                    "failed to parse {} — the file still carries family-tier \
                     allowlist entries, so it cannot be ruled out as a \
                     family-provisioned agent",
                    path.display()
                )));
            }
            warn!(
                path = %path.display(),
                error = %e,
                "identity.toml is malformed; no family-tier signal in the raw file, \
                 so the tier guard treats this agent as non-family rather than \
                 refusing startup. Fix the file — the agent falls back to a \
                 default identity at load."
            );
            return Ok(false);
        }
    };

    let Some(allowlist) = parsed.skills.allowlist else {
        return Ok(false);
    };

    let found: std::collections::BTreeSet<&str> = allowlist.iter().map(|s| s.as_str()).collect();
    let family: std::collections::BTreeSet<&str> =
        FAMILY_AGENT_SKILL_ALLOWLIST.iter().copied().collect();
    Ok(found == family)
}

pub const DEFAULT_HEARTBEAT: &str = r#"# Heartbeat Checklist

- Review active commitments approaching deadline
- Check if any meetings are coming up in the next 2 hours
- Look for stale priorities (no updates in 3+ days)
- Surface patterns worth mentioning
"#;

pub const DEFAULT_USER: &str = r#"# Tell Mika about yourself

Edit this file with your name, role, preferences, and anything
you'd like Mika to know about you. This seeds Mika's initial
understanding when starting fresh.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;

    #[test]
    #[serial]
    fn test_resolve_home_dir_with_mika_home() {
        // Safety: test sets env var; no other thread reads this.
        unsafe { std::env::set_var("MIKA_HOME", "/tmp/test-mika-home") };
        let result = resolve_home_dir().unwrap();
        assert_eq!(result, PathBuf::from("/tmp/test-mika-home"));
        unsafe { std::env::remove_var("MIKA_HOME") };
    }

    #[test]
    #[serial]
    fn test_resolve_home_dir_default() {
        // Safety: test removes env var; no other thread reads this.
        unsafe { std::env::remove_var("MIKA_HOME") };
        let result = resolve_home_dir().unwrap();
        assert!(result.ends_with(".mika"));
    }

    /// mika#1783 AC4 — persona-side substrate closure.
    ///
    /// FAMILY_SOUL MUST NOT teach the sealed family being the operator's
    /// identity. Even after tool-boundary scrubbing (AC1/AC2), a persona
    /// that carries "Vincent" as a named referent gives the being the
    /// addressee it needs to construct the leak. Same rule for
    /// "operator" / "opérateur" (English/French).
    ///
    /// Also asserts no origin-story language ("créé", "conçu") that would
    /// point at an implicit maker the being could later address. Option A
    /// of the plan: no origin story = cleanest closure.
    #[test]
    fn family_soul_no_operator_name() {
        const FORBIDDEN_TOKENS: &[&str] =
            &["Vincent", "vincent", "operator", "opérateur", "Operator"];
        for token in FORBIDDEN_TOKENS {
            assert!(
                !FAMILY_SOUL.contains(token),
                "FAMILY_SOUL must not contain operator-identity token {token:?} \
                 (mika#1783 AC4 — the sealed being's persona must not name a \
                 substrate-owner referent). See docs/plans/2026-08-22-003-*.md"
            );
        }
        // Origin-story guard: "créé" as a whole word ("Vincent m'a créé...")
        // is the shape that reintroduces the referent. A generic "créer"
        // conjugation elsewhere is fine — this specifically catches the
        // first-person-passive-past-participle form that names a maker.
        assert!(
            !FAMILY_SOUL.contains("m'a créé"),
            "FAMILY_SOUL must not carry a first-person origin-story that \
             names an implicit maker (mika#1783 AC4)"
        );
    }

    #[test]
    fn test_is_initialized_false_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_initialized(tmp.path()));
    }

    #[test]
    fn test_is_initialized_true_when_db_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("mika.db"), "fake db").unwrap();
        assert!(is_initialized(tmp.path()));
    }

    // ------------------------------------------------------------------
    // Bootstrap tests — pose the tier, never hope for it (mika#2073)
    //
    // `bootstrap()` selects its `identity.toml` / `soul.md` templates from
    // `MIKA_AGENT_TIER`, read off the **process** environment. That is shared
    // state: every test in this binary sees the same variable at the same
    // instant.
    //
    // **`#[serial]` only protects against other `#[serial]`s.** A bare `#[test]`
    // runs in parallel with them, so it can observe a `MIKA_AGENT_TIER=family`
    // set by a serial test two hundred lines below — and assert the operator
    // persona against the family template. That is what turned CI red on PR#2072,
    // a PR touching none of this code, and green again on a re-run of the very
    // same commit.
    //
    // So: a test that cares which templates get written calls the `_with_tier`
    // variant and **passes the tier it assumes**. Adding another `#[serial]` is
    // not the gesture — it serializes the suite to work around shared state
    // instead of removing the read. `mika2073_no_bare_test_reads_the_tier_from_the_environment`
    // enforces this at the bottom of the module.
    // ------------------------------------------------------------------

    #[test]
    fn test_bootstrap_creates_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap_with_tier(&home, AgentTier::Default).unwrap();

        assert!(home.join("logs").is_dir());
        assert!(home.join("config.toml").is_file());
        assert!(home.join("identity.toml").is_file());
        assert!(home.join("soul.md").is_file());
        assert!(home.join("heartbeat.md").is_file());
        assert!(home.join("user.md").is_file());

        // Verify content
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("llm_provider"));
        assert!(config.contains("mika setup"));

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(soul.contains("executive assistant"));
    }

    #[test]
    fn test_bootstrap_does_not_overwrite_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap_with_tier(&home, AgentTier::Default).unwrap();

        // Modify a file
        fs::write(home.join("soul.md"), "custom soul").unwrap();

        // Bootstrap again — should NOT overwrite
        bootstrap_with_tier(&home, AgentTier::Default).unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert_eq!(soul, "custom soul");
    }

    #[cfg(unix)]
    #[test]
    fn test_bootstrap_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap_with_tier(&home, AgentTier::Default).unwrap();

        let dir_perms = fs::metadata(&home).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_perms, 0o700);

        let file_perms = fs::metadata(home.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_perms, 0o600);
    }

    #[test]
    fn test_is_multi_agent_layout() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_multi_agent_layout(tmp.path()));

        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        assert!(is_multi_agent_layout(tmp.path()));
    }

    #[test]
    fn test_is_legacy_layout() {
        let tmp = tempfile::tempdir().unwrap();
        // No DB at all
        assert!(!is_legacy_layout(tmp.path()));

        // Legacy: data/mika.db exists, no agents/ dir
        fs::create_dir_all(tmp.path().join("data")).unwrap();
        fs::write(tmp.path().join("data").join("mika.db"), "fake").unwrap();
        assert!(is_legacy_layout(tmp.path()));

        // Multi-agent: also has agents/ dir
        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        assert!(!is_legacy_layout(tmp.path()));
    }

    #[test]
    fn test_bootstrap_agent() {
        let tmp = tempfile::tempdir().unwrap();
        bootstrap_agent_with_tier(tmp.path(), "work", AgentTier::Default).unwrap();

        let agent = tmp.path().join("agents").join("work");
        assert!(agent.join("logs").is_dir());
        assert!(agent.join("skills").is_dir());
        assert!(agent.join("config.toml").is_file());
        assert!(agent.join("soul.md").is_file());
        assert!(agent.join("identity.toml").is_file());
    }

    /// Converted although name validation rejects before any template is
    /// selected: leaving a single unconverted call in this file would give
    /// `mika2073_no_bare_test_reads_the_tier_from_the_environment` a lone
    /// exception, and a guard with one exception is a guard that gets a second.
    #[test]
    fn test_bootstrap_agent_rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        let tier = AgentTier::Default;
        assert!(bootstrap_agent_with_tier(tmp.path(), "INVALID", tier).is_err());
        assert!(bootstrap_agent_with_tier(tmp.path(), "", tier).is_err());
        assert!(bootstrap_agent_with_tier(tmp.path(), "-bad", tier).is_err());
    }

    #[test]
    fn test_resolve_agent_home_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        // No agents/ dir → legacy layout → returns home_dir
        let resolved = resolve_agent_home(tmp.path(), "mika");
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn test_resolve_agent_home_multi_agent() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("agents")).unwrap();
        let resolved = resolve_agent_home(tmp.path(), "work");
        assert_eq!(resolved, tmp.path().join("agents").join("work"));
    }

    #[test]
    fn test_is_initialized_multi_agent() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_initialized(tmp.path()));

        // Create a multi-agent layout with one bootstrapped agent
        let agent = tmp.path().join("agents").join("mika");
        fs::create_dir_all(&agent).unwrap();
        fs::write(agent.join("config.toml"), "# config").unwrap();
        assert!(is_initialized(tmp.path()));
    }

    #[test]
    fn test_migrate_to_multi_agent_from_legacy() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Set up legacy layout (bootstrap no longer creates data/, so create it
        // manually to simulate the legacy layout that had a per-root data/ dir)
        bootstrap_with_tier(home, AgentTier::Default).unwrap();
        fs::create_dir_all(home.join("data")).unwrap();
        // Write a marker into the DB so we can verify it stays at root
        fs::write(home.join("data").join("mika.db"), "test-db-content").unwrap();
        fs::write(home.join("soul.md"), "custom soul").unwrap();

        assert!(is_legacy_layout(home));

        // Migrate
        migrate_to_multi_agent(home).unwrap();

        // Verify multi-agent layout
        assert!(is_multi_agent_layout(home));

        // data/ stays at root (container DB)
        assert!(home.join("data").is_dir());
        assert_eq!(
            fs::read_to_string(home.join("data").join("mika.db")).unwrap(),
            "test-db-content"
        );

        // Agent files moved to agents/mika/
        let mika_agent = home.join("agents").join("mika");
        assert_eq!(
            fs::read_to_string(mika_agent.join("soul.md")).unwrap(),
            "custom soul"
        );
        assert!(mika_agent.join("identity.toml").is_file());
        assert!(mika_agent.join("config.toml").is_file());
        assert!(mika_agent.join("skills").is_dir());

        // active_agent file should exist
        assert_eq!(read_active_agent(home), "mika");

        // Root config.toml should be the global one
        let root_config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(root_config.contains("global"));
    }

    #[test]
    fn test_migrate_to_multi_agent_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Set up legacy layout and migrate (bootstrap no longer creates data/,
        // so create it manually to simulate the legacy layout)
        bootstrap_with_tier(home, AgentTier::Default).unwrap();
        fs::create_dir_all(home.join("data")).unwrap();
        fs::write(home.join("data").join("mika.db"), "test-db").unwrap();
        migrate_to_multi_agent(home).unwrap();

        // Migrate again — should be no-op
        migrate_to_multi_agent(home).unwrap();

        // Still works — container DB at root
        assert!(is_multi_agent_layout(home));
        assert_eq!(
            fs::read_to_string(home.join("data").join("mika.db")).unwrap(),
            "test-db"
        );
    }

    #[test]
    fn test_migrate_to_multi_agent_noop_on_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        // No legacy layout, nothing to migrate
        migrate_to_multi_agent(tmp.path()).unwrap();
        // agents/ dir should not be created
        assert!(!is_multi_agent_layout(tmp.path()));
    }

    #[test]
    fn test_read_write_active_agent() {
        let tmp = tempfile::tempdir().unwrap();

        // Default when no file
        assert_eq!(read_active_agent(tmp.path()), "mika");

        // Write and read back
        write_active_agent(tmp.path(), "work").unwrap();
        assert_eq!(read_active_agent(tmp.path()), "work");

        // Overwrite
        write_active_agent(tmp.path(), "code").unwrap();
        assert_eq!(read_active_agent(tmp.path()), "code");
    }

    #[test]
    fn test_read_active_agent_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("active_agent"), "  \n").unwrap();
        assert_eq!(read_active_agent(tmp.path()), "mika");
    }

    #[test]
    fn test_bootstrap_fresh_install() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        bootstrap_fresh_install_with_tier(home, AgentTier::Default).unwrap();

        // Multi-agent layout created
        assert!(is_multi_agent_layout(home));

        // Container-level data dir created
        assert!(home.join("data").is_dir());

        // Default agent bootstrapped
        let mika_agent = home.join("agents").join("mika");
        assert!(mika_agent.join("logs").is_dir());
        assert!(mika_agent.join("skills").is_dir());
        assert!(mika_agent.join("config.toml").is_file());
        assert!(mika_agent.join("soul.md").is_file());

        // Active agent set to "mika"
        assert_eq!(read_active_agent(home), "mika");

        // Root-level global config written
        let root_config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(root_config.contains("global"));
    }

    /// AC4 (mika#1596): the default identity written by `bootstrap_fresh_install` carries a
    /// narrow `[skills].allowlist` that excludes engineering/verdict-carrying skills and
    /// includes the operator-essential ones — so a fresh personal/customer agent does not
    /// load the architect/dev skills that leak `Disposition:` lines into user-facing replies.
    ///
    /// Not `#[serial]` (mika#2073): it used to be, to keep `MIKA_AGENT_TIER`
    /// unset while it read the default identity. Passing the tier by argument
    /// removes the read, and the chain `bootstrap_fresh_install_with_tier` →
    /// `bootstrap_agent_with_tier` → `bootstrap_with_tier` touches no other
    /// process state — `MIKA_HOME` and `MIKA_DEPLOYMENT` are on none of its
    /// links — so there is nothing left to sequence. Its old doc-comment said
    /// "must not race the family-tier **serial** tests", which is the exact
    /// half-reasoning this ticket closes: the dangerous neighbours were the
    /// eight *bare* tests, not the serial ones.
    #[test]
    fn test_bootstrap_fresh_install_writes_narrow_skill_allowlist() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        bootstrap_fresh_install_with_tier(home, AgentTier::Default).unwrap();

        let identity_path = home.join("agents").join("mika").join("identity.toml");
        let raw = fs::read_to_string(&identity_path).unwrap();
        let parsed: toml::Value = toml::from_str(&raw).unwrap();

        let allowlist: Vec<String> = parsed
            .get("skills")
            .and_then(|s| s.get("allowlist"))
            .and_then(|a| a.as_array())
            .expect("default identity must have an active [skills].allowlist")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        // Non-empty — proves the `apply_identity_allowlist` no-op-on-empty path can't trigger.
        assert!(!allowlist.is_empty());

        // Single source of truth: the written allowlist matches the constant.
        assert_eq!(allowlist, DEFAULT_AGENT_SKILL_ALLOWLIST);

        // AC4 exclusions: no engineering / verdict-carrying skills.
        let excluded = [
            "dev-pilot",
            "dev-groom",
            "mika-arch-groom-ticket",
            "mika-arch-groom-milestone",
            "mika-arch-second-review",
            "qa-review",
            "qa-review-build-callback",
        ];
        for name in excluded {
            assert!(
                !allowlist.iter().any(|s| s == name),
                "default allowlist must exclude engineering skill `{name}`"
            );
        }
        // Prefix guard: nothing self-dev* or mika-arch*.
        for s in &allowlist {
            assert!(
                !s.starts_with("self-dev") && !s.starts_with("mika-arch"),
                "default allowlist must not contain engineering skill `{s}`"
            );
        }

        // AC2 inclusions: operator-essential skills present.
        for name in [
            "calendar",
            "google-workspace",
            "browser-control",
            "desktop",
            "file-reader",
            "web-search",
            "mcp",
        ] {
            assert!(
                allowlist.iter().any(|s| s == name),
                "default allowlist must include operator-essential skill `{name}`"
            );
        }
    }

    // ------------------------------------------------------------------
    // Family-tier persona wire (mika#1778)
    // ------------------------------------------------------------------

    /// Set `MIKA_AGENT_TIER=family`, bootstrap a fresh tempdir, then assert the
    /// French persona anchor landed in `soul.md`. Serial because the tier is
    /// env-var-gated.
    #[test]
    #[serial]
    fn test_bootstrap_writes_family_persona_when_tier_family() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        // Safety: test sets an env var; `#[serial]` prevents concurrent readers.
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "family") };
        let res = bootstrap(home);
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        res.unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(
            soul.contains("chaleureux, patient, simple"),
            "family soul must carry the approved French persona anchor"
        );
        assert!(
            !soul.contains("senior executive assistant"),
            "family soul must not carry the operator persona"
        );

        let identity = fs::read_to_string(home.join("identity.toml")).unwrap();
        assert!(
            !identity.contains("\"github\""),
            "family identity must exclude the github skill"
        );
    }

    /// Clear the env var, bootstrap, assert the operator English persona is the
    /// fall-through default.
    #[test]
    #[serial]
    fn test_bootstrap_writes_default_persona_when_tier_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        bootstrap(home).unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(
            soul.contains("senior executive assistant"),
            "default soul must carry the operator English anchor"
        );
        assert!(
            !soul.contains("chaleureux, patient, simple"),
            "default soul must not carry the family French persona"
        );
    }

    /// An unknown tier value provisions the **most restricted** surface, not the
    /// operator one (mika#2023 AC2 — this test asserted the fall-through until
    /// that fix; the fall-through is what put `shell-exec`/`tmux`/`git-ops`/
    /// `github`-write on a champion tenant). The `warn!` naming the value is
    /// unchanged and lives in the live path.
    #[test]
    #[serial]
    fn test_bootstrap_writes_restricted_persona_on_unknown_tier() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        unsafe { std::env::set_var("MIKA_AGENT_TIER", "quantum") };
        let res = bootstrap(home);
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        res.unwrap();

        let identity = fs::read_to_string(home.join("identity.toml")).unwrap();
        let parsed: toml::Value = toml::from_str(&identity).unwrap();
        let allowlist: Vec<&str> = parsed
            .get("skills")
            .and_then(|s| s.get("allowlist"))
            .and_then(|a| a.as_array())
            .expect("provisioned identity.toml must carry an active allowlist")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for excluded in ["github", "git-ops", "shell-exec", "tmux"] {
            assert!(
                !allowlist.contains(&excluded),
                "unknown tier must not provision the operator surface (`{excluded}`)"
            );
        }

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(
            !soul.contains("senior executive assistant"),
            "unknown tier must not fall through to the operator persona"
        );
    }

    /// mika#1962 — the marker must NOT be the first line of `FAMILY_SOUL`.
    /// `build_compact_system_prompt` (mika-agent) takes `lines().next()` as the
    /// whole `## Personality` section on the MikaModel path, so a leading
    /// sentinel replaces the family persona with a platform-internal comment on
    /// exactly the tier the marker exists to protect. mika-common cannot see
    /// that builder, so the invariant is pinned here at the source.
    #[test]
    fn family_soul_marker_is_not_the_persona_line() {
        let first_line = FAMILY_SOUL.lines().next().unwrap();
        assert!(
            !first_line.contains(FAMILY_SOUL_MARKER),
            "FAMILY_SOUL's first line is the compact-prompt persona summary; \
             it must not be the marker. Got: {first_line:?}"
        );
        assert!(
            first_line.contains("Mika"),
            "first line must still read as a persona summary, got: {first_line:?}"
        );
        assert!(
            FAMILY_SOUL.contains(FAMILY_SOUL_MARKER),
            "the marker must still be present somewhere in FAMILY_SOUL"
        );
    }

    /// mika#1962 axis 1 — a soul written from `FAMILY_SOUL` is detected.
    #[test]
    fn soul_has_family_marker_detects_family_soul() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("soul.md"), FAMILY_SOUL).unwrap();
        assert!(soul_has_family_marker(tmp.path()).unwrap());
    }

    /// The operator soul must never trip axis 1.
    #[test]
    fn soul_has_family_marker_rejects_default_soul() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("soul.md"), DEFAULT_SOUL).unwrap();
        assert!(!soul_has_family_marker(tmp.path()).unwrap());
    }

    /// A missing `soul.md` is not an error — an agent dir mid-bootstrap
    /// legitimately has none, and failing there would refuse startup on a
    /// fresh install with no drift behind it.
    #[test]
    fn soul_has_family_marker_absent_file_is_false_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!soul_has_family_marker(tmp.path()).unwrap());
    }

    /// A hand-edit that prepends content must not defeat detection — this is
    /// why the check is `contains`, not `starts_with`.
    #[test]
    fn soul_has_family_marker_survives_prepended_content() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("soul.md"),
            format!("\n# Notes de Vincent\n\n{FAMILY_SOUL}"),
        )
        .unwrap();
        assert!(soul_has_family_marker(tmp.path()).unwrap());
    }

    /// mika#1962 axis 2 — the load-bearing detector for family agents
    /// provisioned before the marker existed.
    #[test]
    fn identity_allowlist_matches_family_detects_family_identity() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("identity.toml"), FAMILY_IDENTITY).unwrap();
        assert!(identity_allowlist_matches_family(tmp.path()).unwrap());
    }

    #[test]
    fn identity_allowlist_matches_family_rejects_default_identity() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("identity.toml"), DEFAULT_IDENTITY).unwrap();
        assert!(!identity_allowlist_matches_family(tmp.path()).unwrap());
    }

    /// Set comparison, not sequence comparison — a reordered allowlist is the
    /// same set of permitted skills and must still be detected as family.
    #[test]
    fn identity_allowlist_matches_family_is_order_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reversed: Vec<&str> = FAMILY_AGENT_SKILL_ALLOWLIST.to_vec();
        reversed.reverse();
        let entries = reversed
            .iter()
            .map(|s| format!("    \"{s}\","))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            tmp.path().join("identity.toml"),
            format!("name = \"Mika\"\n\n[skills]\nallowlist = [\n{entries}\n]\n"),
        )
        .unwrap();
        assert!(identity_allowlist_matches_family(tmp.path()).unwrap());
    }

    /// An identity with no `[skills].allowlist` is default-permissive, which
    /// is not the family shape.
    #[test]
    fn identity_allowlist_matches_family_absent_allowlist_is_false() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("identity.toml"), "name = \"Mika\"\n").unwrap();
        assert!(!identity_allowlist_matches_family(tmp.path()).unwrap());
    }

    #[test]
    fn identity_allowlist_matches_family_absent_file_is_false_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!identity_allowlist_matches_family(tmp.path()).unwrap());
    }

    /// A corrupted identity that STILL carries family-tier allowlist entries
    /// must not read as "not family". That file falls back to a
    /// default-permissive identity in `mika-agent`, so answering `false` here
    /// would both widen the agent's skills and hide it from the guard.
    #[test]
    fn identity_allowlist_matches_family_malformed_toml_with_family_signal_is_error() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Mika\"\n[skills\nallowlist = [\"calendar\", \"google-workspace\"]\n",
        )
        .unwrap();
        let err = identity_allowlist_matches_family(tmp.path()).unwrap_err();
        assert!(
            err.to_string().contains("identity.toml"),
            "error must name the offending file, got: {err}"
        );
    }

    /// The complement, and the reason the raw-text scan exists: an unrelated
    /// typo in an OPERATOR agent's identity must not take the whole
    /// mika-spirit process down. Nothing about this file suggests family
    /// provisioning, so the tier guard has no business adjudicating it.
    #[test]
    fn identity_allowlist_matches_family_malformed_toml_without_family_signal_is_false() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Mika\"\n[skills\nallowlist = [\"github\", \"shell-exec\"]\n",
        )
        .unwrap();
        assert!(!identity_allowlist_matches_family(tmp.path()).unwrap());
    }

    /// The `FAMILY_AGENT_SKILL_ALLOWLIST` constant and the TOML array embedded in
    /// `FAMILY_IDENTITY` must stay in lockstep — mirrors the default-tier assertion
    /// pattern from `test_bootstrap_fresh_install_writes_narrow_skill_allowlist`.
    #[test]
    fn test_family_allowlist_matches_family_identity_toml() {
        let parsed: toml::Value = toml::from_str(FAMILY_IDENTITY).unwrap();
        let allowlist: Vec<String> = parsed
            .get("skills")
            .and_then(|s| s.get("allowlist"))
            .and_then(|a| a.as_array())
            .expect("FAMILY_IDENTITY must have an active [skills].allowlist")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert_eq!(
            allowlist, FAMILY_AGENT_SKILL_ALLOWLIST,
            "FAMILY_IDENTITY TOML allowlist must match FAMILY_AGENT_SKILL_ALLOWLIST"
        );

        // Guard: no dev/orchestrator surface leaks into the family tier.
        let excluded = [
            "github",
            "git-ops",
            "shell-exec",
            "tmux",
            "gh-read-only",
            "self-knowledge",
            "mcp",
            "dev-pilot",
            "dev-groom",
            "qa-review",
        ];
        for name in excluded {
            assert!(
                !allowlist.iter().any(|s| s == name),
                "family allowlist must exclude jargon-carrying skill `{name}`"
            );
        }
    }

    /// `AgentTier::from_env()` reads `MIKA_AGENT_TIER` case-insensitively.
    #[test]
    #[serial]
    fn test_agent_tier_from_env_variants() {
        // Unset → Default
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        assert_eq!(AgentTier::from_env(), AgentTier::Default);

        // Empty → Default
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "") };
        assert_eq!(AgentTier::from_env(), AgentTier::Default);

        // Case-insensitive default
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "Default") };
        assert_eq!(AgentTier::from_env(), AgentTier::Default);

        // Case-insensitive family
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "FAMILY") };
        assert_eq!(AgentTier::from_env(), AgentTier::Family);

        // Whitespace trimmed
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "  family  ") };
        assert_eq!(AgentTier::from_env(), AgentTier::Family);

        // mika#2023 AC1 — champion, same case-insensitivity and same trim.
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "champion") };
        assert_eq!(AgentTier::from_env(), AgentTier::Champion);
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "CHAMPION ") };
        assert_eq!(AgentTier::from_env(), AgentTier::Champion);

        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
    }

    /// mika#2290 AC1 — `Deployment::from_env()` has three states, and the
    /// **negative control of absence rides in the same call** (the discipline
    /// mika#2023 AC2/AC4 imposed on this family): a test that only proved
    /// `cloud → Cloud` would stay green under a `from_env` that resolved
    /// absence to `Local`, which is precisely the bug.
    #[test]
    #[serial]
    fn mika2290_deployment_from_env_has_three_states() {
        // Absence → Unknown. This is the damaged population: the tenant that
        // asserted "tout tourne en local" carried no variable at all.
        unsafe { std::env::remove_var("MIKA_DEPLOYMENT") };
        assert_eq!(Deployment::from_env(), Deployment::Unknown);

        // Empty → Unknown (an operator who blanked the value declared nothing).
        unsafe { std::env::set_var("MIKA_DEPLOYMENT", "") };
        assert_eq!(Deployment::from_env(), Deployment::Unknown);

        // Recognized values, case-insensitive and trimmed.
        unsafe { std::env::set_var("MIKA_DEPLOYMENT", "cloud") };
        assert_eq!(Deployment::from_env(), Deployment::Cloud);
        unsafe { std::env::set_var("MIKA_DEPLOYMENT", "CLOUD ") };
        assert_eq!(Deployment::from_env(), Deployment::Cloud);
        unsafe { std::env::set_var("MIKA_DEPLOYMENT", "  Local  ") };
        assert_eq!(Deployment::from_env(), Deployment::Local);

        // Non-empty unrecognized → Unknown (fail-closed floor, `warn!` names it).
        unsafe { std::env::set_var("MIKA_DEPLOYMENT", "prod") };
        assert_eq!(
            Deployment::from_env(),
            Deployment::Unknown,
            "an unrecognized value must never be read as a hosting assertion"
        );

        unsafe { std::env::remove_var("MIKA_DEPLOYMENT") };
    }

    /// mika#2290 — the divergence with `AgentTier::from_env` on **absence** is a
    /// decision, so it is pinned rather than left to be re-derived. Both are
    /// asserted in one test because the property is the *contrast*: reading
    /// either assertion alone invites "make them consistent" as a cleanup.
    #[test]
    #[serial]
    fn mika2290_absence_diverges_from_the_tier_on_purpose() {
        unsafe { std::env::remove_var("MIKA_DEPLOYMENT") };
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };

        assert_eq!(
            AgentTier::from_env(),
            AgentTier::Default,
            "an unset tier is the legitimate operator workstation (mika#2023 M1)"
        );
        assert_eq!(
            Deployment::from_env(),
            Deployment::Unknown,
            "an unset deployment is the population that produced the false \
             privacy claim — it must assert nothing (mika#2290 Décision 1)"
        );
    }

    /// mika#2023 AC3 — the two axes are decoupled, and the champion sits on the
    /// family tools profile.
    ///
    /// The tools assertion is the p0: a champion must be provisioned with the
    /// family allowlist, so `shell-exec`/`tmux`/`git-ops`/`github` never reach an
    /// external tester's tenant.
    #[test]
    fn mika2023_champion_carries_family_tools() {
        assert_eq!(AgentTier::Champion.tools_profile(), ToolsProfile::Family);
        assert_eq!(
            AgentTier::Champion.skill_allowlist(),
            FAMILY_AGENT_SKILL_ALLOWLIST
        );
        assert_eq!(AgentTier::Champion.identity_toml(), FAMILY_IDENTITY);

        for excluded in ["github", "git-ops", "shell-exec", "tmux", "gh-read-only"] {
            assert!(
                !AgentTier::Champion.skill_allowlist().contains(&excluded),
                "the champion tools profile must not carry `{excluded}`"
            );
        }
    }

    /// mika#2023 AC3 — the champion persona is a placeholder reachable from ONE
    /// site, and the axes are genuinely independent rather than one match written
    /// twice.
    ///
    /// The single-site property is asserted structurally: `persona_profile`'s
    /// `Champion` arm returns the named constant, so replacing the constant is the
    /// whole of the change Vincent's answer requires. A source scan pins that the
    /// constant is defined exactly once in this module.
    #[test]
    fn mika2023_champion_persona_is_a_single_site_placeholder() {
        assert_eq!(
            AgentTier::Champion.persona_profile(),
            CHAMPION_PERSONA_PLACEHOLDER,
            "the champion persona must be read from the placeholder constant, \
             never inlined"
        );
        assert_eq!(AgentTier::Champion.soul_md(), FAMILY_SOUL);

        // The axes are separable: today Default is the only tier that differs
        // between them, but the API admits a tier that mixes them — which is the
        // shape the champion slot needs when it is filled.
        assert_eq!(AgentTier::Default.tools_profile(), ToolsProfile::Operator);
        assert_eq!(
            AgentTier::Default.persona_profile(),
            PersonaProfile::Operator
        );

        // Built by concat so this needle does not match itself in the scan below.
        let definition = concat!("const CHAMPION_PERSONA_", "PLACEHOLDER: PersonaProfile");
        let source = include_str!("home.rs");
        assert_eq!(
            source.matches(definition).count(),
            1,
            "the champion persona placeholder must have exactly one definition site"
        );
    }

    /// mika#2023 AC2 — fail-closed on an unrecognized value, and ONLY on an
    /// unrecognized value.
    ///
    /// The two controls live in the same test on purpose. The failure this
    /// closes is "a tier the console knows about, that provisioning does not
    /// map, emits nothing and lands on the operator persona" — but *absence*
    /// is the legitimate shape on Vincent's own machine (`MIKA_AGENT_TIER`
    /// unset = operator). A positive control alone would pass on an
    /// implementation that fails every start of the operator workstation
    /// closed, which is the wrong fix wearing the right result.
    #[test]
    #[serial]
    fn mika2023_unrecognized_tier_value_fails_closed_but_absence_stays_default() {
        // Positive: a non-empty value outside {default, family, champion}
        // resolves to the most restricted tools tier, not to Default.
        // Safety: serialized against every other MIKA_AGENT_TIER test.
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "pro") };
        let resolved = AgentTier::from_env();
        assert_ne!(
            resolved,
            AgentTier::Default,
            "an unrecognized tier must never resolve to the operator tier"
        );
        assert_eq!(
            resolved.tools_profile(),
            ToolsProfile::Family,
            "an unrecognized tier must resolve to the most restricted tools profile"
        );

        // Negative, same call site: absence is not an unrecognized value.
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        assert_eq!(
            AgentTier::from_env(),
            AgentTier::Default,
            "an unset MIKA_AGENT_TIER is the legitimate operator workstation"
        );
    }

    // -- mika#2230 — the tier vocabulary has one parser ----------------------

    /// `AgentTier::parse` is extracted, and `from_env` is a reader over it.
    #[test]
    fn mika2230_from_env_reads_the_extracted_parser() {
        assert_eq!(AgentTier::parse("").tier, AgentTier::Default);
        assert_eq!(AgentTier::parse("  Default  ").tier, AgentTier::Default);
        assert_eq!(AgentTier::parse("FAMILY").tier, AgentTier::Family);
        assert_eq!(AgentTier::parse("Champion").tier, AgentTier::Champion);
        assert!(AgentTier::parse("champion").recognized);

        let unknown = AgentTier::parse("zorglub");
        assert!(
            !unknown.recognized,
            "an unrecognized value must be reportable as such, so a caller can \
             name it without re-comparing against the tier vocabulary"
        );
        assert_eq!(
            unknown.tier.tools_profile(),
            ToolsProfile::Family,
            "and it still fails closed (mika#2023 AC2)"
        );
    }

    /// A line that compares a raw value against the tier vocabulary.
    ///
    /// The needles are recomposed with [`concat!`] rather than written whole:
    /// this block is masked from the scan twice over (it is under `cfg(test)`,
    /// and `home.rs` is excluded by path), so the recomposition buys nothing
    /// *today* — it is written because a guard that becomes its own first
    /// offender does not fail informatively, and the natural repair is to widen
    /// it until it catches nothing. Same reasoning, same shape, as
    /// `source_guard::tests::reimplements_the_boundary`.
    fn reimplements_the_tier_parser(line: &str) -> bool {
        let trimmed = line.trim_start();
        // Prose must stay able to describe what is forbidden, including this
        // module's own doc comments, which name every tier by its literal.
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            return false;
        }
        let champion = concat!('"', "champion", '"');
        if line.contains(champion) {
            return true;
        }
        let family = concat!('"', "family", '"');
        line.contains(family)
            && ["=>", "==", "eq_ignore_ascii_case", "matches!"]
                .iter()
                .any(|shape| line.contains(shape))
    }

    /// **V20 — no second parser of the tier vocabulary.**
    ///
    /// `mika agents reprovision --tier` (mika#2230) is the second caller that
    /// holds a raw tier value. The obvious implementation is a clap `ValueEnum`,
    /// which restates `{default, family, champion}` on the CLI side — and with it
    /// the fail-closed rule of mika#2023 AC2, which is the part that would
    /// silently stop matching. No behavioural test can see that class: a second
    /// parser makes no decision wrong the day it is written. It diverges at the
    /// fourth tier, in silence, which is precisely the shape of the mika#2023
    /// incident (a console that knew about a tier the binary did not).
    ///
    /// The primary needle is the literal `"champion"`, the one token of this
    /// vocabulary that means nothing else in this tree; `"family"` is a common
    /// enough word that it is only flagged in a comparison shape.
    ///
    /// **Disposition: halt-and-surface. There is no exception list and adding one
    /// is not the remedy** — an allowlist born empty is a place to put the next
    /// violation.
    #[test]
    fn mika2230_le_tier_a_un_seul_analyseur() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("mika-common sits at <workspace>/crates/mika-common");
        let this_file = workspace.join("crates/mika-common/src/home.rs");

        let mut offenders: Vec<String> = Vec::new();
        let mut crates_scanned = 0usize;

        for entry in std::fs::read_dir(workspace.join("crates"))
            .expect("the guard must be able to read crates/")
            .flatten()
        {
            let src = entry.path().join("src");
            if !src.is_dir() {
                continue;
            }
            crates_scanned += 1;
            let scanner = crate::source_guard::ProductionScanner::new(&src);
            scanner.for_each(|path, production| {
                if path == this_file {
                    return; // the one legitimate site
                }
                for (n, line) in production.lines().enumerate() {
                    if reimplements_the_tier_parser(line) {
                        offenders.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            });
        }

        assert!(
            crates_scanned >= 4,
            "the guard scanned {crates_scanned} crates — it is not reading the workspace"
        );
        assert!(
            offenders.is_empty(),
            "mika#2230 — {} site(s) compare a raw value against the tier vocabulary \
             outside `AgentTier::parse`:\n{}\n\n\
             WHY THIS MATTERS: a second parser makes no decision wrong the day it is \
             written. It diverges the day a fourth tier arrives — and it takes the \
             mika#2023 fail-closed rule with it, which is the half that fails \
             silently. That is the incident mika#2023 was filed for: an upstream \
             knowing about a tier the binary did not.\n\
             FIX: call `AgentTier::parse(raw)` and read `TierParse::recognized` if \
             you need to name an unrecognized value.\n\
             There is NO exception list, and adding one is not the remedy.",
            offenders.len(),
            offenders.join("\n")
        );
    }

    /// V20's good-faith control — a detector verified only by its own green is
    /// verified by nothing. Written against fabricated lines rather than by
    /// editing real source.
    #[test]
    fn mika2230_the_tier_parser_guard_fires_on_a_relapse() {
        for relapse in [
            r#"            "family" => AgentTier::Family,"#,
            r#"    #[value(name = "champion")]"#,
            r#"        if raw.eq_ignore_ascii_case("family") { return Tier::Family; }"#,
            r#"        matches!(raw, "family" | "default")"#,
        ] {
            assert!(
                reimplements_the_tier_parser(relapse),
                "the guard must catch a re-introduced parser: {relapse}"
            );
        }

        for innocent in [
            r#"    /// Selected when `MIKA_AGENT_TIER=family` (case-insensitive)."#,
            r#"    // the "family" tier carries FAMILY_IDENTITY on the tools axis"#,
            r#"        let label = tier_label("family");"#,
            r#"        writeln!(out, "tier {tier:?} (via {provenance})")?;"#,
        ] {
            assert!(
                !reimplements_the_tier_parser(innocent),
                "the guard must not fire on: {innocent}"
            );
        }
    }

    #[test]
    fn temp_relapse_early() {
        let tmp = tempfile::tempdir().unwrap();
        bootstrap(&tmp.path().join("x")).unwrap();
    }

    // -- mika#2073 — the tier is posed, and the environment cannot move it ---

    /// **The deterministic positive control** (mika#2073).
    ///
    /// AC4 asks for N green runs in a row. That is a *probabilistic* proof: the
    /// race window is narrow, so N green runs do not separate "the defect is
    /// closed" from "the defect did not fire". This test separates them — it
    /// sets `MIKA_AGENT_TIER=family` **on purpose**, in the most hostile
    /// arrangement a co-running test could produce, and asserts that an
    /// explicitly-posed `AgentTier::Default` still writes `DEFAULT_SOUL`.
    ///
    /// Legitimately `#[serial]`: this one *writes* the variable, which is
    /// exactly the population `#[serial]` exists for.
    ///
    /// **Disposition: halt-and-surface.** When it goes red, the injection has
    /// been unplugged and `*_with_tier` is reading the environment again.
    /// Restore the injection; never relax the assertion.
    #[test]
    #[serial]
    fn mika2073_an_explicit_tier_survives_a_hostile_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        // Safety: test sets an env var; `#[serial]` prevents concurrent writers.
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "family") };
        let res = bootstrap_with_tier(&home, AgentTier::Default);
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        res.unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(
            soul.contains("executive assistant"),
            "an explicitly-posed tier must win over `MIKA_AGENT_TIER`; the soul \
             written here came from the environment, so the injection is unplugged"
        );
        assert!(
            !soul.contains("chaleureux, patient, simple"),
            "the hostile `MIKA_AGENT_TIER=family` reached the template selection"
        );

        // Mirror direction: the argument is the whole decision, not a default
        // that the environment may override in the other direction either.
        let family_home = tmp.path().join("family-test");
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        bootstrap_with_tier(&family_home, AgentTier::Family).unwrap();
        let family_soul = fs::read_to_string(family_home.join("soul.md")).unwrap();
        assert!(
            family_soul.contains("chaleureux, patient, simple"),
            "a posed Family tier must write the family persona with the env var unset"
        );
    }

    // -- mika#2073 — no bare test reads the tier from the process environment --

    /// The joined inner text of the attribute starting at `lines[i]`, and the
    /// index just past it. `None` when `lines[i]` does not start an attribute.
    ///
    /// **Attributes may span lines** — `#[tokio::test(\n flavor = "multi_thread"\n)]`
    /// is ordinary rustfmt output past the line budget. A one-line reader
    /// returns `None` for it, the whole attribute block goes unrecognized, and
    /// the test item below it is skipped **in silence**: a false negative that
    /// looks exactly like a clean file. So the bracket depth is followed across
    /// lines.
    ///
    /// The predicate is on an **attribute**, never on the presence of a string,
    /// and this function is what makes that true: `use serial_test::serial;`
    /// sits in this very module and carries the token without decorating
    /// anything.
    fn attribute_at(lines: &[&str], i: usize) -> Option<(String, usize)> {
        if !lines.get(i)?.trim_start().starts_with("#[") {
            return None;
        }
        let mut joined = String::new();
        let mut depth = 0i32;
        let mut j = i;
        while j < lines.len() {
            let piece = lines[j].trim();
            for c in piece.chars() {
                match c {
                    '[' => depth += 1,
                    ']' => depth -= 1,
                    _ => {}
                }
            }
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(piece);
            j += 1;
            if depth <= 0 {
                break;
            }
        }
        if depth > 0 {
            return None; // unterminated: not an attribute we can read
        }
        let inner = joined.trim().strip_prefix("#[")?.strip_suffix(']')?.trim();
        Some((inner.to_string(), j))
    }

    /// Does this attribute make the item below it a test that `libtest` may run
    /// **in parallel with the others**? `#[tokio::test]` answers yes exactly as
    /// `#[test]` does: `#[serial]` and libtest's thread pool are two distinct
    /// mechanisms, and a multi-thread tokio flavour protects nothing here.
    fn inner_is_test(inner: &str) -> bool {
        inner == "test" || inner == "tokio::test" || inner.starts_with("tokio::test(")
    }

    /// `#[serial]` has two spellings in this tree — the bare one and
    /// `#[serial_test::serial]` (26 sites, including the `MIKA_AGENT_TIER`
    /// setters of `mika-agent`'s tier guard). Recognizing only the first would
    /// make the guard shout at correct code, and a guard that shouts at correct
    /// code is a guard somebody silences.
    fn inner_is_serial(inner: &str) -> bool {
        let head = inner.split('(').next().unwrap_or(inner).trim();
        head == "serial" || head == "serial_test::serial"
    }

    fn is_test_attribute(line: &str) -> bool {
        attribute_at(&[line], 0).is_some_and(|(inner, _)| inner_is_test(&inner))
    }

    fn is_serial_attribute(line: &str) -> bool {
        attribute_at(&[line], 0).is_some_and(|(inner, _)| inner_is_serial(&inner))
    }

    /// Does `line` **call** `name`?
    ///
    /// The `(` alone is not enough, and the tree says why:
    /// `mika-agent/src/tools/update_core_memory.rs` carries a test named
    /// `test_updates_still_capped_after_bootstrap`, whose signature ends in
    /// `bootstrap()` — lexically indistinguishable from a call unless the
    /// character *before* the name is checked. Requiring a non-identifier there
    /// is what turns "the string is present" into "the function is called".
    fn calls(line: &str, name: &str) -> bool {
        let mut from = 0usize;
        while let Some(rel) = line[from..].find(name) {
            let at = from + rel;
            let after = at + name.len();
            let opens = line[after..].starts_with('(');
            let boundary = line[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
            if opens && boundary {
                return true;
            }
            from = at + name.len();
        }
        false
    }

    /// A body line that reaches the **process-wide** tier: one of the three
    /// historical `bootstrap*` entry points, or `AgentTier::from_env()` itself.
    ///
    /// Prose keeps the right to name what is forbidden, so comment lines are
    /// excluded. `bootstrap` is not a call of `bootstrap` under [`calls`]'s
    /// boundary rule when it is part of a longer identifier, so the converted
    /// `_with_tier` call sites are excluded by construction rather than by a
    /// second list.
    fn reads_the_tier_from_the_environment(line: &str) -> bool {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            return false;
        }
        [
            "bootstrap",
            "bootstrap_agent",
            "bootstrap_fresh_install",
            "AgentTier::from_env",
        ]
        .iter()
        .any(|name| calls(line, name))
    }

    struct TierScan {
        offenders: Vec<String>,
        tests_seen: usize,
        /// Test items whose body did not close where the source says it closes.
        /// **Not a diagnostic — a failure.** See the guard.
        desyncs: Vec<String>,
    }

    /// Lexer state that must survive the end of a line.
    ///
    /// A Rust string literal may span lines — `r#"…"#` blocks do it routinely,
    /// and this very module contains one. A per-line scanner resets at each
    /// newline, so a `}` sitting inside such a literal reads as a closing brace
    /// and ends a test body early: every line after it escapes the scan, the
    /// item count is unchanged, and the guard stays **green** while blind. That
    /// is the same shape as the defect this whole ticket closes, one level down,
    /// so the state is carried rather than reset.
    #[derive(Default)]
    struct Lex {
        in_string: bool,
        /// `Some(n)` inside a raw string opened with `n` hashes (`r##"` → 2).
        in_raw: Option<usize>,
        in_block_comment: bool,
    }

    /// Brace movement contributed by a line, ignoring comments and string, raw
    /// string and char literals. Returns `(delta, saw_open)`; `saw_open` is
    /// separate because a line like `unsafe { … };` has a delta of zero and
    /// still opens the item's block.
    fn brace_scan(line: &str, lex: &mut Lex) -> (i32, bool) {
        let c: Vec<char> = line.chars().collect();
        let mut depth = 0i32;
        let mut saw_open = false;
        let mut i = 0;
        while i < c.len() {
            if lex.in_block_comment {
                if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    lex.in_block_comment = false;
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if let Some(hashes) = lex.in_raw {
                if c[i] == '"' && c[i + 1..].iter().take(hashes).all(|h| *h == '#') {
                    lex.in_raw = None;
                    i += 1 + hashes;
                } else {
                    i += 1;
                }
                continue;
            }
            if lex.in_string {
                match c[i] {
                    '\\' => i += 2,
                    '"' => {
                        lex.in_string = false;
                        i += 1;
                    }
                    _ => i += 1,
                }
                continue;
            }
            match c[i] {
                '/' if c.get(i + 1) == Some(&'/') => break,
                '/' if c.get(i + 1) == Some(&'*') => {
                    lex.in_block_comment = true;
                    i += 2;
                    continue;
                }
                'r' if matches!(c.get(i + 1), Some('"') | Some('#')) => {
                    let hashes = c[i + 1..].iter().take_while(|h| **h == '#').count();
                    if c.get(i + 1 + hashes) == Some(&'"') {
                        lex.in_raw = Some(hashes);
                        i += 2 + hashes;
                        continue;
                    }
                    i += 1;
                    continue;
                }
                '"' => lex.in_string = true,
                // A `'` is a char literal only when it closes within three or
                // four characters; otherwise it is a lifetime or a loop label,
                // and consuming the rest of the line would desync the walk.
                '\'' => {
                    let esc = c.get(i + 1) == Some(&'\\');
                    let close = if esc { i + 3 } else { i + 2 };
                    if c.get(close) == Some(&'\'') {
                        i = close + 1;
                        continue;
                    }
                }
                '{' => {
                    depth += 1;
                    saw_open = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        (depth, saw_open)
    }

    /// Walk a Rust source text, and for every **non-serial** test item report
    /// each body line that reads the tier from the process environment.
    ///
    /// Runs over the raw file on purpose: `source_guard::ProductionScanner`
    /// *masks* `cfg(test)` regions, which is the exact inverse of what is needed
    /// here — the target of this scan **is** the test block.
    fn scan_bare_tests_reading_the_tier(source: &str) -> TierScan {
        let lines: Vec<&str> = source.lines().collect();
        let mut offenders = Vec::new();
        let mut desyncs = Vec::new();
        let mut tests_seen = 0usize;
        let mut i = 0;

        while i < lines.len() {
            let Some((first_inner, mut next)) = attribute_at(&lines, i) else {
                i += 1;
                continue;
            };
            // One contiguous attribute block, then the item it decorates.
            let mut is_test = inner_is_test(&first_inner);
            let mut is_serial = inner_is_serial(&first_inner);
            while let Some((inner, after)) = attribute_at(&lines, next) {
                is_test |= inner_is_test(&inner);
                is_serial |= inner_is_serial(&inner);
                next = after;
            }
            i = next;
            if !is_test || i >= lines.len() {
                continue;
            }
            tests_seen += 1;

            let signature_at = i;
            let signature = lines[i].trim().to_string();
            let indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
            let mut lex = Lex::default();
            let mut depth = 0i32;
            let mut opened = false;
            let mut body: Vec<(usize, &str)> = Vec::new();
            let mut closed_at: Option<usize> = None;
            while i < lines.len() {
                let line = lines[i];
                let (delta, saw_open) = brace_scan(line, &mut lex);
                opened |= saw_open;
                depth += delta;
                // The signature itself is not a call site: a test *named* after
                // `bootstrap` is the measured false positive this drops.
                let scanned = if i == signature_at {
                    line.find('{').map_or("", |p| &line[p + 1..])
                } else {
                    line
                };
                body.push((i, scanned));
                i += 1;
                if opened && depth <= 0 {
                    closed_at = Some(i - 1);
                    break;
                }
            }

            // **Self-check, and it is the load-bearing half of this scanner.**
            // Every other way this walk can go wrong is silent: an item's body
            // ends early, the lines after it are never examined, the item count
            // is unchanged, and the guard reports a clean file. rustfmt closes a
            // test item with `<indent>}` and nothing else, so anything else here
            // means the walk lost the thread — which is reported as a failure,
            // never swallowed (mika#2205: a scan that silently read nothing
            // reads exactly like a scan that found nothing).
            match closed_at {
                Some(n) if lines[n] == format!("{indent}}}") => {}
                Some(n) => desyncs.push(format!(
                    "  line {} in `{}` — body closed on {:?}, expected {:?}",
                    n + 1,
                    signature,
                    lines[n],
                    format!("{indent}}}")
                )),
                None => desyncs.push(format!(
                    "  line {} in `{}` — body never closed before end of file",
                    signature_at + 1,
                    signature
                )),
            }

            if is_serial {
                continue;
            }
            for (n, line) in body {
                if reads_the_tier_from_the_environment(line) {
                    offenders.push(format!(
                        "  home.rs:{} in `{}` — {}",
                        n + 1,
                        signature,
                        line.trim()
                    ));
                }
            }
        }

        TierScan {
            offenders,
            tests_seen,
            desyncs,
        }
    }

    /// **mika#2073 — a test must pose the tier, never hope for it.**
    ///
    /// `bootstrap()` reads `MIKA_AGENT_TIER` off the *process* environment, which
    /// every thread of the test binary shares. Ten tests of this module are
    /// `#[serial]` and six of them write that variable — but `#[serial]` only
    /// sequences its own bearers, so a bare `#[test]` runs alongside them and can
    /// observe `family` where it asserted `DEFAULT_SOUL`. That is what turned
    /// CI red on PR#2072 for a PR that touched none of this code, and green again
    /// on a re-run of the same commit.
    ///
    /// No behavioural test can hold this class: the regression makes no decision
    /// wrong, it makes one non-deterministic — and a test that fails once in a
    /// hundred runs passes in CI. Hence a source scan (same reasoning as
    /// mika#2131).
    ///
    /// **Scope: this file only.** AC2 of mika#2073 says "audit of the same file",
    /// and six sibling sites in `mika-agent/src/well_known_agents.rs` carry the
    /// same armed mine without the mandate to convert them — a workspace-wide
    /// scan would be red on them, i.e. undeliverable. The follow-up ticket widens
    /// the scan and converts those six at the same commit, in that order.
    ///
    /// **Disposition: halt-and-surface. The allowlist is empty and there is no
    /// constant to add one to** — see the plan's Fire-Disposition section.
    #[test]
    fn mika2073_no_bare_test_reads_the_tier_from_the_environment() {
        let this_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/home.rs");
        let source = std::fs::read_to_string(&this_file)
            .expect("the guard must be able to read its own file");

        let scan = scan_bare_tests_reading_the_tier(&source);

        // A scan that silently read nothing is indistinguishable from a clean
        // tree (class mika#2205). On a single file a drifted path surfaces here
        // first.
        assert!(
            scan.tests_seen >= 40,
            "the guard saw only {} test items in {} — it is not reading the file \
             it thinks it is. Its green means nothing until this number is \
             plausible.",
            scan.tests_seen,
            this_file.display()
        );

        // The item count cannot see the *other* way this walk fails: a body that
        // ends early still counts as one item, and every line after it escapes
        // the scan while the guard reports green. So the walk states where each
        // body closed, and a mismatch is a failure rather than a note.
        assert!(
            scan.desyncs.is_empty(),
            "mika#2073 — the source walk lost the thread on {} test item(s):\n{}\n\n\
             This is NOT a finding about the tests; it is the guard telling you it \
             cannot see. Until it is fixed, a green result attests nothing. The \
             usual cause is a literal or comment shape `brace_scan` does not carry \
             correctly across lines.",
            scan.desyncs.len(),
            scan.desyncs.join("\n")
        );

        assert!(
            scan.offenders.is_empty(),
            "mika#2073 — {} non-serial test(s) read `MIKA_AGENT_TIER` off the \
             process environment:\n{}\n\n\
             WHY THIS MATTERS: `MIKA_AGENT_TIER` is process-wide shared state, and \
             `#[serial]` only sequences tests that carry it. A bare `#[test]` runs \
             in parallel with the serial tests that set and unset that variable, so \
             this test can read `family` on a run where it asserted the default \
             persona — intermittently, on a PR that touched none of this code.\n\
             FIX: call the `_with_tier` variant and pass the tier the test assumes \
             (`bootstrap_with_tier(&home, AgentTier::Default)`).\n\
             NOT a fix: adding `#[serial]` (it serializes the whole suite for a \
             defect that has a structural correction, and AC1 rules it out). \
             NOT a fix: adding an allowlist entry — there is no allowlist, and \
             creating one is how this guard dies.",
            scan.offenders.len(),
            scan.offenders.join("\n")
        );
    }

    /// mika#2073's good-faith control — a detector verified only by its own
    /// green is verified by nothing. Written against **fabricated** lines and a
    /// fabricated source, never by editing real source.
    ///
    /// Two of the shapes below do not occur in the file actually scanned
    /// (`home.rs` has zero tokio tests and no `#[serial_test::serial]` in
    /// attribute position — only the `use` at the head of this module). This
    /// control is therefore their **sole** attestation, which is exactly why it
    /// cannot be trimmed.
    #[test]
    fn mika2073_the_guard_fires_on_a_relapse() {
        // The forbidden call tokens are recomposed with `concat!` rather than
        // written whole: the guard scans *this* file, so a literal `bootstrap`
        // immediately followed by `(` written here would make this control the
        // guard's own first offender — and the natural repair for that is to
        // widen the guard until it catches nothing. Same reasoning, same shape,
        // as `reimplements_the_tier_parser` above.
        let boot = concat!("bootstrap", "(");
        let agent = concat!("bootstrap_agent", "(");
        let fresh = concat!("bootstrap_fresh_install", "(");
        let from_env = concat!("AgentTier::from_env", "(");

        // -- line-level predicates ------------------------------------------
        for call in [
            format!("        {boot}&home).unwrap();"),
            format!("        {agent}tmp.path(), \"work\").unwrap();"),
            format!("        {fresh}home).unwrap();"),
            format!("        let tier = {from_env});"),
            format!("        assert!({agent}tmp.path(), \"INVALID\").is_err());"),
            format!("        home::{boot}&dir).unwrap();"),
        ] {
            assert!(
                reads_the_tier_from_the_environment(&call),
                "the guard must catch an environment read: {call}"
            );
        }
        for innocent in [
            // The converted call sites — excluded by the identifier boundary,
            // not by a second list.
            "        bootstrap_with_tier(&home, AgentTier::Default).unwrap();".to_string(),
            "        bootstrap_agent_with_tier(tmp.path(), \"w\", AgentTier::Default).unwrap();"
                .to_string(),
            "        bootstrap_fresh_install_with_tier(home, AgentTier::Default).unwrap();"
                .to_string(),
            // Prose naming what is forbidden.
            format!("        // {boot}&home) would read the tier off the environment"),
            format!("    /// Validates the name, then calls `{boot})`."),
            // Measured in the tree (`mika-agent/src/tools/update_core_memory.rs`):
            // a test *named* after bootstrap, whose body calls nothing.
            format!("    fn test_updates_still_capped_after_{boot}) {{"),
        ] {
            assert!(
                !reads_the_tier_from_the_environment(&innocent),
                "the guard must not fire on: {innocent}"
            );
        }

        // -- attribute predicates -------------------------------------------
        for test_attr in [
            "    #[test]",
            "    #[tokio::test]",
            "    #[tokio::test(flavor = \"multi_thread\", worker_threads = 2)]",
            "    #[tokio::test(start_paused = true)]",
        ] {
            assert!(
                is_test_attribute(test_attr),
                "the guard must recognize the test attribute: {test_attr}"
            );
        }
        for serial_attr in ["    #[serial]", "    #[serial_test::serial]"] {
            assert!(
                is_serial_attribute(serial_attr),
                "the guard must recognize both spellings of serial: {serial_attr}"
            );
        }
        // Measured in this very module (`:980`): the token without the attribute.
        assert!(
            !is_serial_attribute("    use serial_test::serial;"),
            "a `use` is not an attribute — the predicate is on `#[…]`, never on a string"
        );
        assert!(
            !is_test_attribute("    #[cfg(unix)]"),
            "an unrelated attribute must not read as a test attribute"
        );

        // -- end-to-end, on a fabricated source ------------------------------
        // The line predicates above say nothing about the walk: which body a
        // line belongs to, whether its item was serial, and whether the
        // signature counts as a call site. This half does. Built with `format!`
        // for the same reason the tokens above are recomposed.
        let fabricated = format!(
            r##"
mod tests {{
    #[test]
    fn a_bare_test_that_relapses() {{
        let msg = "an unbalanced brace in a string: {{";
        {boot}&home).unwrap();
    }}

    #[tokio::test(flavor = "multi_thread")]
    async fn an_async_bare_test_that_relapses() {{
        {agent}tmp.path(), "work").unwrap();
    }}

    #[test]
    #[serial]
    fn a_legitimate_serial_test() {{
        unsafe {{ std::env::set_var("MIKA_AGENT_TIER", "family") }};
        {boot}home).unwrap();
    }}

    #[test]
    #[serial_test::serial]
    fn a_legitimate_serial_test_spelled_long() {{
        {fresh}home).unwrap();
    }}

    #[test]
    fn a_converted_test() {{
        bootstrap_with_tier(&home, AgentTier::Default).unwrap();
    }}

    #[test]
    fn a_test_merely_named_after_{boot}) {{
        assert!(true);
    }}

    // A multi-line raw string carrying an unbalanced `}}` on a line of its own.
    // A per-line lexer ends this body here, and every test below escapes.
    #[test]
    fn a_test_holding_a_multiline_literal() {{
        let fixture = r#"a fixture line
    }}
still inside the literal"#;
        let _ = fixture;
        {boot}&home).unwrap();
    }}

    // rustfmt splits a long attribute; a one-line reader returns None for it
    // and skips the whole item without saying so.
    #[tokio::test(
        flavor = "multi_thread",
        worker_threads = 2
    )]
    async fn a_test_behind_a_multiline_attribute() {{
        {fresh}home).unwrap();
    }}
}}
"##
        );
        let scan = scan_bare_tests_reading_the_tier(&fabricated);
        assert!(
            scan.desyncs.is_empty(),
            "the walk must stay in sync on the fabricated source:\n{}",
            scan.desyncs.join("\n")
        );
        assert_eq!(
            scan.tests_seen, 8,
            "the walk must find every test item in the fabricated source — \
             including the one behind a multi-line attribute — got {}",
            scan.tests_seen
        );
        assert_eq!(
            scan.offenders.len(),
            4,
            "the four bare relapses must be reported, got:\n{}",
            scan.offenders.join("\n")
        );
        assert!(
            scan.offenders
                .iter()
                .any(|o| o.contains("a_test_holding_a_multiline_literal")),
            "a `}}` inside a multi-line literal must not end the body early — \
             that failure is silent, and silence is what this ticket is about"
        );
        assert!(
            scan.offenders
                .iter()
                .any(|o| o.contains("a_test_behind_a_multiline_attribute")),
            "a test behind a multi-line attribute must still be seen"
        );
        assert!(
            scan.offenders
                .iter()
                .any(|o| o.contains("a_bare_test_that_relapses")),
            "the bare `#[test]` relapse must be reported"
        );
        assert!(
            scan.offenders
                .iter()
                .any(|o| o.contains("an_async_bare_test_that_relapses")),
            "the bare `#[tokio::test]` relapse must be reported — `#[serial]` and \
             libtest's thread pool are two different mechanisms"
        );
    }
}
