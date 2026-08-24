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
    Family,
}

impl AgentTier {
    /// Resolve the tier from the `MIKA_AGENT_TIER` env var. Unknown values (anything
    /// outside `{"default", "family"}` after case-folding) fall through to `Default`
    /// with a single `warn!` log naming the offending value — visible in
    /// `MIKA_SPIRIT_LOG_FILE`.
    pub fn from_env() -> Self {
        match std::env::var("MIKA_AGENT_TIER") {
            Err(_) => Self::Default,
            Ok(raw) => {
                let normalized = raw.trim().to_ascii_lowercase();
                match normalized.as_str() {
                    "" | "default" => Self::Default,
                    "family" => Self::Family,
                    _ => {
                        warn!(
                            value = %raw,
                            "MIKA_AGENT_TIER value not recognized; falling through to Default persona"
                        );
                        Self::Default
                    }
                }
            }
        }
    }

    fn identity_toml(self) -> &'static str {
        match self {
            Self::Default => DEFAULT_IDENTITY,
            Self::Family => FAMILY_IDENTITY,
        }
    }

    fn soul_md(self) -> &'static str {
        match self {
            Self::Default => DEFAULT_SOUL,
            Self::Family => FAMILY_SOUL,
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
    std::fs::create_dir_all(home_dir.join("agents"))
        .with_context(|| format!("failed to create {}/agents/", home_dir.display()))?;
    // Create the container-level data directory for the shared database
    std::fs::create_dir_all(home_dir.join("data"))
        .with_context(|| format!("failed to create {}/data/", home_dir.display()))?;
    bootstrap_agent(home_dir, crate::agent::DEFAULT_AGENT)
        .with_context(|| "failed to initialize default agent".to_string())?;
    write_active_agent(home_dir, crate::agent::DEFAULT_AGENT)?;
    write_default_if_missing(home_dir, "config.toml", DEFAULT_GLOBAL_CONFIG)?;
    Ok(())
}

/// Bootstrap a named agent under the multi-agent layout.
/// Validates the name, creates `{home_dir}/agents/{name}/`, and calls `bootstrap()`.
pub fn bootstrap_agent(home_dir: &Path, name: &str) -> Result<()> {
    crate::agent::validate_agent_name(name)?;
    let dir = crate::agent::agent_dir(home_dir, name);
    bootstrap(&dir)
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
/// The `identity.toml` and `soul.md` templates are selected by `AgentTier::from_env()`
/// — set `MIKA_AGENT_TIER=family` in the container's environment BEFORE first startup
/// to land the family persona (mika#1778). `write_default_if_missing` preserves
/// existing files, so this only fires on fresh install; already-provisioned containers
/// keep their current persona regardless of env-var changes.
pub fn bootstrap(home_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(home_dir.join("logs"))
        .with_context(|| format!("failed to create {}/logs/", home_dir.display()))?;
    std::fs::create_dir_all(home_dir.join("skills"))
        .with_context(|| format!("failed to create {}/skills/", home_dir.display()))?;

    let tier = AgentTier::from_env();
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
"#;

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

    #[test]
    fn test_bootstrap_creates_structure() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap(&home).unwrap();

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

        bootstrap(&home).unwrap();

        // Modify a file
        fs::write(home.join("soul.md"), "custom soul").unwrap();

        // Bootstrap again — should NOT overwrite
        bootstrap(&home).unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert_eq!(soul, "custom soul");
    }

    #[cfg(unix)]
    #[test]
    fn test_bootstrap_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("mika-test");

        bootstrap(&home).unwrap();

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
        bootstrap_agent(tmp.path(), "work").unwrap();

        let agent = tmp.path().join("agents").join("work");
        assert!(agent.join("logs").is_dir());
        assert!(agent.join("skills").is_dir());
        assert!(agent.join("config.toml").is_file());
        assert!(agent.join("soul.md").is_file());
        assert!(agent.join("identity.toml").is_file());
    }

    #[test]
    fn test_bootstrap_agent_rejects_invalid_name() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(bootstrap_agent(tmp.path(), "INVALID").is_err());
        assert!(bootstrap_agent(tmp.path(), "").is_err());
        assert!(bootstrap_agent(tmp.path(), "-bad").is_err());
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
        bootstrap(home).unwrap();
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
        bootstrap(home).unwrap();
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

        bootstrap_fresh_install(home).unwrap();

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
    /// `#[serial]` (mika#1778): reads the default identity, which depends on
    /// `MIKA_AGENT_TIER` being unset/default — must not race the family-tier serial tests.
    #[test]
    #[serial]
    fn test_bootstrap_fresh_install_writes_narrow_skill_allowlist() {
        // Ensure no leaked family tier from a co-running test (defensive; #[serial]
        // already sequences with the family tests below).
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        bootstrap_fresh_install(home).unwrap();

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

    /// An unknown tier value falls through to Default (with a `warn!` in the
    /// live path; the test just asserts persona selection).
    #[test]
    #[serial]
    fn test_bootstrap_writes_default_persona_on_unknown_tier() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        unsafe { std::env::set_var("MIKA_AGENT_TIER", "quantum") };
        let res = bootstrap(home);
        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
        res.unwrap();

        let soul = fs::read_to_string(home.join("soul.md")).unwrap();
        assert!(
            soul.contains("senior executive assistant"),
            "unknown tier must fall through to the default operator persona"
        );
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

        // Unknown → Default (fall-through)
        unsafe { std::env::set_var("MIKA_AGENT_TIER", "quantum") };
        assert_eq!(AgentTier::from_env(), AgentTier::Default);

        unsafe { std::env::remove_var("MIKA_AGENT_TIER") };
    }
}
