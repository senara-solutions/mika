//! Well-known agent provisioning for dev mode.
//!
//! When `dev_mode = true` in config, the startup sequence auto-provisions
//! development agents (`mika-dev`, `mika-qa`) with role-specific identity,
//! soul, and skill assignments.

use std::path::Path;

use mika_common::config::Settings;
use tracing::{error, info, warn};

use crate::db::Database;

/// Per-skill LLM override to seed on first creation.
#[non_exhaustive]
pub struct LlmOverrideSpec {
    /// Skill name to override.
    pub skill_name: &'static str,
    /// LLM provider (e.g., "anthropic").
    pub provider: &'static str,
    /// LLM model (e.g., "claude-opus-4-7").
    pub model: &'static str,
}

/// Builder for an agent's identity.toml content.
///
/// Static variant: a `&'static str` template baked into the binary (used by
/// agents whose identity is fully known at compile time).
///
/// Computed variant: a function that receives `&Settings` and returns the
/// rendered identity.toml content. Used by agents whose identity depends on
/// runtime configuration (e.g., mika-arch's `[kg].docs_roots` which must be
/// absolute paths derived from `MIKA_KG_DOCS_ROOTS` at provision time).
///
/// Failure semantics: a `Computed` builder may return `Err(String)` to signal
/// "this agent cannot be provisioned on this host" (e.g., required env not
/// set). The provisioner logs `error!` with the agent name and the message,
/// then SKIPS this agent so other well-known agents can still come up.
pub enum IdentitySource {
    /// Static template — content is the same across all hosts.
    Static(&'static str),
    /// Computed at provision time from `Settings`.
    Computed(fn(&Settings) -> Result<String, String>),
}

/// Specification for a well-known development agent.
#[non_exhaustive]
pub struct WellKnownAgent {
    /// Agent name (lowercase, hyphenated).
    pub name: &'static str,
    /// Display name for identity.toml.
    pub display_name: &'static str,
    /// Emoji for identity.toml.
    pub emoji: &'static str,
    /// Soul.md content.
    pub soul: &'static str,
    /// Bundled skills to disable for this agent.
    pub disabled_skills: &'static [&'static str],
    /// Optional config.toml content to write on first creation.
    /// When `Some`, overwrites the default config.toml with agent-specific
    /// LLM provider/model settings.
    pub config_toml: Option<&'static str>,
    /// Optional custom identity.toml builder. When `Some`, overrides the
    /// default template (which only sets name + emoji + commented KG block).
    /// Used by agents like mika-arch that need `[skills].allowlist`,
    /// `[tools].disabled`, and runtime-resolved `[kg].docs_roots`.
    pub identity_source: Option<IdentitySource>,
    /// Per-skill LLM overrides to seed in `skill_overrides` DB table.
    /// Only applied on first creation (same guard as `disabled_skills`).
    pub llm_overrides: &'static [LlmOverrideSpec],
}

/// mika-dev agent specification.
pub static MIKA_DEV: WellKnownAgent = WellKnownAgent {
    name: "mika-dev",
    display_name: "Dev",
    emoji: "🛠",
    soul: MIKA_DEV_SOUL,
    // mika-arch-* skills are review-class for the architect agent only;
    // exclude them from mika-dev to prevent context pollution and arch-style
    // keyword triggers firing on dev work.
    // dev-groom is operator-only (#845) — mika-dev must NOT auto-invoke grooming.
    disabled_skills: &[
        "qa-review",
        "qa-review-build-callback",
        "skill-review",
        "mika-arch-groom-ticket",
        "mika-arch-second-review",
        "dev-groom",
    ],
    config_toml: None,
    identity_source: None,
    llm_overrides: &[],
};

/// mika-qa agent specification.
pub static MIKA_QA: WellKnownAgent = WellKnownAgent {
    name: "mika-qa",
    display_name: "QA",
    emoji: "🔍",
    soul: MIKA_QA_SOUL,
    disabled_skills: &[
        "self-dev",
        "self-dev-iterate",
        "self-dev-webhook-qa",
        "self-dev-webhook-ci",
        "dev-pilot",
        "permission-policy",
        "agents-teams",
        "address-pr-comments",
        "resolve-pr-conflicts",
        // mika-arch-* skills are for the architect agent only — keep mika-qa
        // focused on PR review without arch-style triggers.
        "mika-arch-groom-ticket",
        "mika-arch-second-review",
        // dev-groom is operator-only (#845) — mika-qa must NOT invoke grooming.
        "dev-groom",
    ],
    config_toml: None,
    identity_source: None,
    llm_overrides: &[],
};

/// mika-relay agent specification.
///
/// Lightweight agent for handling claude-pilot (the binary) `can_use_tool`
/// permission relay events. Only the `permission-policy` skill is enabled;
/// all other bundled skills are disabled. Uses haiku for cheap, fast JSON
/// classification.
pub static MIKA_RELAY: WellKnownAgent = WellKnownAgent {
    name: "mika-relay",
    display_name: "Relay",
    emoji: "🔑",
    soul: MIKA_RELAY_SOUL,
    disabled_skills: &[
        // Disable all bundled skills except permission-policy.
        // Engine-coupled (skills/bundled/):
        "self-dev",
        "self-dev-iterate",
        "self-dev-webhook-qa",
        "self-dev-webhook-ci",
        "qa-review",
        "qa-review-build-callback",
        "skill-review",
        "dev-pilot",
        "build-mika",
        "deploy-mika",
        "agents-teams",
        "address-pr-comments",
        "resolve-pr-conflicts",
        "self-check",
        "mika-arch-groom-ticket",
        "mika-arch-second-review",
        "dev-groom",
        // Community (hardcoded BUNDLED_SKILLS):
        "tmux",
        "shell-exec",
        "web-search",
        "file-reader",
        "self-knowledge",
        "git-ops",
        "google-workspace",
        "github",
        "mcp",
        "browser-control",
    ],
    config_toml: Some(MIKA_RELAY_CONFIG),
    identity_source: None,
    llm_overrides: &[],
};

/// mika-arch agent specification.
///
/// Read-only architect agent for plan-stage review. Uses identity-driven
/// skill allowlist (only `mika-arch-groom-ticket` and `mika-arch-second-review`
/// are enabled). Base model is Kimi; per-skill LLM overrides route to
/// Opus 4.7 (groom-ticket) and Sonnet 4.6 (second-review).
///
/// Identity is computed at provision time from `Settings.kg_docs_roots` so
/// `[kg].docs_roots` contains absolute paths. Without `MIKA_KG_DOCS_ROOTS`
/// set, mika-arch is skipped at provision with an explicit `error!` log.
pub static MIKA_ARCH: WellKnownAgent = WellKnownAgent {
    name: "mika-arch",
    display_name: "Architect",
    emoji: "🏛",
    soul: MIKA_ARCH_SOUL,
    // Empty: mika-arch uses identity allowlist, not denylist.
    disabled_skills: &[],
    config_toml: Some(MIKA_ARCH_CONFIG),
    identity_source: Some(IdentitySource::Computed(build_mika_arch_identity)),
    llm_overrides: &[
        LlmOverrideSpec {
            skill_name: "mika-arch-groom-ticket",
            provider: "anthropic",
            model: "claude-opus-4-7",
        },
        LlmOverrideSpec {
            skill_name: "mika-arch-second-review",
            provider: "anthropic",
            model: "claude-sonnet-4-6",
        },
    ],
};

/// Tools mika-arch must NOT receive in its LLM tool array.
///
/// These names are baked into mika-arch's identity.toml `[tools].disabled`
/// at provision time. The filter is applied in
/// `agent::apply_agent_tool_visibility()` at LLM-tool-array assembly — the
/// model never sees these tools, cannot call them, cannot be prompt-injected
/// into trying. Defense-in-depth for the read-only-architect invariant.
///
/// Categories:
/// - Skill mutations: writes to skills on disk or `skill_overrides` rows
/// - Config / files: writes to settings or agent files
/// - Reminders / scheduled tasks: writes to scheduled_tasks rows
/// - Tasks: mutates task state
/// - PR mutations: merges PRs
/// - Cross-agent invocation: mika-arch is advisory; it does not initiate
///   activity in other agents' lanes (target-agent boundary leak)
/// - Agent / team mutations: provisions/edits other agents and teams
///
/// Notably allowed:
/// - `send_message`: writes to mika-arch's own session history. Constitutive
///   of being an agent — denying it leaves mika-arch unable to deliver
///   verdicts in non-skill-output contexts. Not a platform side-effect.
/// - `update_core_memory`, `store_fact`, `update_fact`: memory writes are
///   agent-scoped self-state (5 core memory blocks + 4 facts categories,
///   scoped to `agent_id = 'mika-arch'`). Constitutive of being an agent —
///   persistence, not platform side-effect. See
///   `docs/architecture/review-guide.md` § Orthogonality.
pub const MIKA_ARCH_DISABLED_TOOLS: &[&str] = &[
    // Skill mutations
    "create_skill",
    "delete_skill",
    "toggle_skill",
    "update_skill",
    // Config / files
    "set_config",
    "write_agent_file",
    // Reminders
    "create_reminder",
    "cancel_reminder",
    // Tasks
    "cancel_task",
    "complete_task",
    "create_task",
    "update_task_status",
    // PR mutations
    "pr_merge_with_gate",
    // Cross-agent invocation: mika-arch is advisory, does not orchestrate
    "a2a_call",
    "delegate_task",
    "run_team",
    // Agent / team mutations
    "create_agent",
    "create_team",
    "delete_team",
    "update_team",
    "add_team_member",
    "remove_team_member",
];

/// Build mika-arch's identity.toml with absolute `[kg].docs_roots` paths
/// derived from `Settings.kg_docs_roots`.
///
/// Returns `Err` when `MIKA_KG_DOCS_ROOTS` (or `kg_docs_roots` in config.toml)
/// is unset or empty. The provisioner logs the error and skips mika-arch so
/// other well-known agents still come up.
fn build_mika_arch_identity(settings: &Settings) -> Result<String, String> {
    let docs_roots = settings
        .kg_docs_roots
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            "MIKA_KG_DOCS_ROOTS (or kg_docs_roots in config.toml) not set; \
         mika-arch requires absolute paths to docs corpora and cannot be \
         provisioned without them"
                .to_string()
        })?;

    let mut roots_block = String::new();
    for path in docs_roots {
        let s = path.to_string_lossy();
        if !path.is_absolute() {
            return Err(format!(
                "MIKA_KG_DOCS_ROOTS contains a relative path '{s}'; mika-arch \
                 requires absolute paths because mika-server runs with CWD=/ \
                 in production"
            ));
        }
        roots_block.push_str(&format!("  \"{s}\",\n"));
    }

    let mut tools_block = String::from("disabled = [\n");
    for tool in MIKA_ARCH_DISABLED_TOOLS {
        tools_block.push_str(&format!("  \"{tool}\",\n"));
    }
    tools_block.push(']');

    Ok(format!(
        r#"name = "Architect"
emoji = "🏛"

[kg]
enabled = true
docs_roots = [
{roots_block}]

[skills]
allowlist = ["mika-arch-groom-ticket", "mika-arch-second-review"]

[tools]
{tools_block}
"#
    ))
}

/// All well-known agents.
pub static WELL_KNOWN_AGENTS: &[&WellKnownAgent] = &[&MIKA_DEV, &MIKA_QA, &MIKA_RELAY, &MIKA_ARCH];

/// Look up a well-known agent by name.
pub fn find_well_known_agent(name: &str) -> Option<&'static WellKnownAgent> {
    WELL_KNOWN_AGENTS.iter().find(|a| a.name == name).copied()
}

/// Render the identity.toml content for a well-known agent.
///
/// Resolves the spec's `identity_source`:
/// - `None` → default template (name + emoji + commented KG block).
/// - `Some(Static(s))` → returns `s`.
/// - `Some(Computed(f))` → calls `f(settings)`; propagates `Err`.
fn render_identity_content(spec: &WellKnownAgent, settings: &Settings) -> Result<String, String> {
    match &spec.identity_source {
        None => Ok(format!(
            "name = \"{}\"\nemoji = \"{}\"\n\n\
             # [kg]\n\
             # enabled = true                    # default: true — set false to skip KG for this agent\n\
             # docs_root = \"/path/to/docs\"       # optional; falls back to MIKA_KG_DOCS_ROOT / kg_docs_root / CWD/docs/solutions\n",
            spec.display_name, spec.emoji
        )),
        Some(IdentitySource::Static(s)) => Ok((*s).to_string()),
        Some(IdentitySource::Computed(f)) => f(settings),
    }
}

/// Provision well-known development agents on the filesystem.
///
/// For each well-known agent that doesn't already exist:
/// 1. Calls `bootstrap_agent()` to create dirs + default files
/// 2. Overwrites `identity.toml` and `soul.md` with agent-specific content
///
/// When `disabled` is true, logs a warning and returns without changes.
/// This is the filesystem phase — DB skill overrides are set separately
/// in [`seed_well_known_skill_overrides`] during agent init.
///
/// Computed-identity failures (e.g., mika-arch when `MIKA_KG_DOCS_ROOTS` is
/// unset) emit `error!` and skip THAT agent — other well-known agents still
/// come up. Operator sees the error in logs and fixes the env config.
pub fn provision_well_known_agents(home_dir: &Path, settings: &Settings, disabled: bool) {
    if disabled {
        warn!(
            "agent provisioning disabled by config \
             (MIKA_DISABLE_AGENT_PROVISIONING=true) — well-known agents \
             will not be auto-created or updated"
        );
        return;
    }

    for spec in WELL_KNOWN_AGENTS {
        if mika_common::agent::agent_exists(home_dir, spec.name) {
            info!(
                agent = spec.name,
                "well-known agent already exists, skipping"
            );
            continue;
        }

        // Render identity.toml content first — bail before bootstrap if
        // computed-identity returned Err so we don't leave a half-provisioned
        // directory tree behind.
        let identity_content = match render_identity_content(spec, settings) {
            Ok(content) => content,
            Err(e) => {
                error!(
                    agent = spec.name,
                    error = %e,
                    "well-known agent identity could not be rendered — skipping provisioning"
                );
                continue;
            }
        };

        match mika_common::home::bootstrap_agent(home_dir, spec.name) {
            Ok(()) => {
                let agent_home = mika_common::agent::agent_dir(home_dir, spec.name);

                if let Err(e) = std::fs::write(agent_home.join("identity.toml"), &identity_content)
                {
                    warn!(
                        agent = spec.name,
                        error = %e,
                        "failed to write identity.toml for well-known agent"
                    );
                    continue;
                }

                // Overwrite soul.md with agent-specific content
                if let Err(e) = std::fs::write(agent_home.join("soul.md"), spec.soul) {
                    warn!(
                        agent = spec.name,
                        error = %e,
                        "failed to write soul.md for well-known agent"
                    );
                    continue;
                }

                // Overwrite config.toml if the spec provides custom content
                if let Some(config_content) = spec.config_toml
                    && let Err(e) = std::fs::write(agent_home.join("config.toml"), config_content)
                {
                    warn!(
                        agent = spec.name,
                        error = %e,
                        "failed to write config.toml for well-known agent"
                    );
                    continue;
                }

                info!(
                    agent = spec.name,
                    display_name = spec.display_name,
                    "provisioned well-known agent"
                );
            }
            Err(e) => {
                warn!(
                    agent = spec.name,
                    error = %e,
                    "failed to bootstrap well-known agent"
                );
            }
        }
    }
}

/// Seed skill overrides for a well-known agent if none exist yet.
///
/// Writes `set_skill_enabled(false)` for each skill in the agent's
/// `disabled_skills` list. Only runs on first creation — if any
/// `skill_overrides` rows already exist for this agent, the function
/// returns early to preserve user customizations.
pub fn seed_well_known_skill_overrides(db: &mut Database, agent_name: &str) {
    let spec = match find_well_known_agent(agent_name) {
        Some(s) => s,
        None => return,
    };

    // Check if any overrides already exist (user has customized)
    match db.get_skill_overrides(agent_name) {
        Ok(overrides) if !overrides.is_empty() => {
            return;
        }
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "failed to check skill overrides, skipping well-known seeding"
            );
            return;
        }
        _ => {}
    }

    for skill_name in spec.disabled_skills {
        if let Err(e) = db.set_skill_enabled(agent_name, skill_name, false) {
            warn!(
                agent = agent_name,
                skill = skill_name,
                error = %e,
                "failed to disable skill for well-known agent"
            );
        }
    }

    // Seed per-skill LLM overrides (e.g., mika-arch skills use Opus/Sonnet)
    for llm_ov in spec.llm_overrides {
        if let Err(e) =
            db.set_skill_llm_override(agent_name, llm_ov.skill_name, llm_ov.provider, llm_ov.model)
        {
            warn!(
                agent = agent_name,
                skill = llm_ov.skill_name,
                provider = llm_ov.provider,
                model = llm_ov.model,
                error = %e,
                "failed to set LLM override for well-known agent skill"
            );
        }
    }

    let total_overrides = spec.disabled_skills.len() + spec.llm_overrides.len();
    info!(
        agent = agent_name,
        disabled_count = spec.disabled_skills.len(),
        llm_override_count = spec.llm_overrides.len(),
        "seeded skill overrides for well-known agent ({total_overrides} total)"
    );
}

const MIKA_DEV_SOUL: &str = r#"# Mika Dev — Development Agent

## Role
You are Mika Dev, a senior software development agent. You implement features,
fix bugs, review code, and manage the development lifecycle autonomously.

## Communication style
- Be direct and technical
- Lead with actions taken, then explain reasoning
- Reference specific files, functions, and line numbers
- When stuck, state the blocker clearly

## Behaviors
- Follow existing codebase patterns and conventions
- Write tests for new behavior
- Run the full test suite before declaring work done
- Create focused, well-described PRs
- Never merge your own PRs without QA review
"#;

const MIKA_QA_SOUL: &str = r#"# Mika QA — Quality Assurance Agent

## Role
You are Mika QA, a senior quality assurance agent. You review pull requests,
verify implementations against requirements, and ensure code quality standards.

## Communication style
- Be precise about what passes and what doesn't
- Use structured verdicts (VERDICT: pass/block/hold)
- Reference specific code locations when flagging issues
- Separate blocking issues from suggestions

## Behaviors
- Review PRs for correctness, test coverage, and adherence to conventions
- Verify that CI checks pass before approving
- Check that PR descriptions accurately reflect the changes
- Never approve your own team's work without independent verification
- Flag security, performance, and maintainability concerns
"#;

const MIKA_RELAY_SOUL: &str = r#"# Mika Relay — Permission Relay Agent

## Role
You are Mika Relay, a permission relay agent. Your sole job is handling
claude-pilot `can_use_tool` permission events — classifying tool calls
into allow/deny/answer/escalate tiers and returning structured JSON.

## Communication style
- Respond only with structured JSON as specified by the permission-policy skill
- Never engage in conversation or prose when handling `[claude-pilot]` events
- Be fast and decisive — permission decisions should not require deliberation

## Behaviors
- Apply the permission-policy skill's tier classification strictly
- When in doubt, escalate (Tier 3) rather than allow
- Never initiate workflows, create tasks, or manage development lifecycle
- You exist only to make permission decisions efficiently
"#;

const MIKA_RELAY_CONFIG: &str = r#"# Mika Relay — lightweight permission relay agent.
# Uses haiku for cheap, fast JSON classification of permission events.

llm_provider = "anthropic"
anthropic_model = "claude-haiku-4-5-20251001"
llm_max_tokens = 1024
log_level = "info"
"#;

const MIKA_ARCH_SOUL: &str = r#"# Mika Architect — Plan Review Agent

## Role
You are Mika Architect, a Principal-Engineer-class advisory reviewer. Your job
is to read implementation plans and produce principle-grounded pushback **before**
code is written. You are read-only — you never write code, commit, merge, or
execute shell commands.

## Communication style
- Citation or silence: flag a concern only if you can cite the review guide,
  an ADR, a compound doc, or an existing convention. Unmoored style preferences
  stay silent.
- Be direct and specific — name the file, symbol, or principle being violated.
- A review without challenge is a failed review. If the plan is genuinely clean,
  say so briefly with the specific principles you verified.

## Behaviors
- Read the plan, brief, and issue context thoroughly before reviewing.
- Use gh_read to fetch issue details and PR diffs — never fabricate GitHub state.
- Query the knowledge graph for institutional learnings relevant to the plan.
- Reference docs/architecture/review-guide.md for architectural principles.
- Produce annotated plan content with inline findings.
- End with an explicit disposition: READY, ITERATE, or ESCALATE.
- Never start workflows, create tasks, or manage development lifecycle.
- You are advisory only — your output is consumed by claude-pilot, which commits.

## Foundational references
Cite these when relevant in reviews. They are the durable artifacts behind the principles you enforce; previously held inline in `current_priorities` core memory (accretion-prone), moved here per the three-way filter (existing artifact = drop in-line + cite from soul). See `docs/solutions/best-practices/core-memory-as-citation-not-accumulator-2026-04-28.md`.

- `docs/architecture/review-guide.md` — SOLID/DRY/YAGNI/KISS/Orthogonality with citations to mika code (canonical principles reference; already cited above)
- `docs/design/north-star.md` — the WHY behind every visual decision across the Mika ecosystem
- `docs/design/luminescent-core.md` — design system rulebook
- `docs/solutions/best-practices/mika-arch-first-dogfood-2026-04-25.md` — disposition-keyword drift on first-pass output
- `docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — plan-on-branch discipline
- `docs/solutions/workflow-issues/comment-event-fires-autonomous-dispatch-2026-04-25.md` — comment-event auto-dispatch behavior
- `docs/solutions/best-practices/required-tools-gate-evasion-patterns-2026-04-28.md` — gate-evasion patterns and structural counterparts (mika#862, #863)
- `docs/solutions/best-practices/verification-claims-with-expected-output-shape-2026-04-28.md` — N=4 verification-claim discipline (run → expected: shape)
"#;

const MIKA_ARCH_CONFIG: &str = r#"# Mika Architect — advisory plan review agent.
# Base model is Kimi for orchestration shell.
# Per-skill LLM overrides route to Opus 4.7 (groom-ticket) and Sonnet 4.6 (second-review).

llm_provider = "openrouter"
openrouter_model = "moonshotai/kimi-k2.5"
llm_max_tokens = 8192
log_level = "info"
"#;

// MIKA_ARCH_IDENTITY is no longer a static const — see build_mika_arch_identity()
// above. Identity is rendered at provision time from `Settings.kg_docs_roots` so
// `[kg].docs_roots` always contains absolute paths.

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Test settings with kg_docs_roots populated so mika-arch identity computation succeeds.
    /// Uses fake-but-absolute paths — mika-arch identity rendering only validates absoluteness,
    /// not existence. The KG ingest pipeline does its own existence validation downstream.
    fn test_settings_with_kg_roots() -> Settings {
        let mut s = Settings::test_defaults();
        s.kg_docs_roots = Some(vec![
            PathBuf::from("/tmp/test-kg-corpus-a"),
            PathBuf::from("/tmp/test-kg-corpus-b"),
        ]);
        s
    }

    #[test]
    fn test_find_well_known_agent_found() {
        assert!(find_well_known_agent("mika-dev").is_some());
        assert_eq!(find_well_known_agent("mika-dev").unwrap().name, "mika-dev");
        assert!(find_well_known_agent("mika-qa").is_some());
        assert_eq!(find_well_known_agent("mika-qa").unwrap().name, "mika-qa");
        assert!(find_well_known_agent("mika-relay").is_some());
        assert_eq!(
            find_well_known_agent("mika-relay").unwrap().name,
            "mika-relay"
        );
    }

    #[test]
    fn test_find_well_known_agent_not_found() {
        assert!(find_well_known_agent("mika").is_none());
        assert!(find_well_known_agent("custom-agent").is_none());
        assert!(find_well_known_agent("").is_none());
    }

    #[test]
    fn test_provision_creates_all_agents() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Create the agents directory
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, &test_settings_with_kg_roots(), false);

        // Iterate WELL_KNOWN_AGENTS rather than hard-coding names so adding a
        // future well-known agent automatically extends this test.
        for spec in WELL_KNOWN_AGENTS {
            assert!(
                mika_common::agent::agent_exists(home, spec.name),
                "well-known agent {} was not provisioned",
                spec.name
            );
        }
    }

    #[test]
    fn test_provision_skips_mika_arch_when_kg_docs_roots_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        // Settings without kg_docs_roots — mika-arch must be skipped, others must come up.
        let mut settings = Settings::test_defaults();
        settings.kg_docs_roots = None;
        provision_well_known_agents(home, &settings, false);

        assert!(mika_common::agent::agent_exists(home, "mika-dev"));
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
        assert!(mika_common::agent::agent_exists(home, "mika-relay"));
        assert!(
            !mika_common::agent::agent_exists(home, "mika-arch"),
            "mika-arch must NOT be provisioned without absolute docs_roots"
        );
    }

    #[test]
    fn test_mika_arch_identity_has_absolute_docs_roots() {
        let settings = test_settings_with_kg_roots();
        let toml = build_mika_arch_identity(&settings).expect("identity should render");
        assert!(toml.contains("/tmp/test-kg-corpus-a"));
        assert!(toml.contains("/tmp/test-kg-corpus-b"));
        // No relative paths leaked through
        assert!(!toml.contains("\"mika/docs/solutions\""));
    }

    #[test]
    fn test_mika_arch_identity_rejects_relative_paths() {
        let mut settings = Settings::test_defaults();
        settings.kg_docs_roots = Some(vec![PathBuf::from("relative/path")]);
        let err = build_mika_arch_identity(&settings).expect_err("must reject relative");
        assert!(err.contains("relative path"));
    }

    #[test]
    fn test_mika_arch_identity_has_tools_disabled_block() {
        let toml = build_mika_arch_identity(&test_settings_with_kg_roots()).unwrap();
        assert!(toml.contains("[tools]"));
        assert!(toml.contains("\"pr_merge_with_gate\""));
        assert!(toml.contains("\"a2a_call\""));
        // Memory writes are NOT denied — agent-scoped self-state, not platform side-effect.
        // See review-guide.md § Orthogonality (commit 2bba6223).
        assert!(!toml.contains("\"update_core_memory\""));
        // send_message must NOT be in the disabled list (constitutive of being an agent)
        assert!(!toml.contains("\"send_message\""));
    }

    #[test]
    fn test_mika_arch_disabled_tools_does_not_include_send_message() {
        assert!(
            !MIKA_ARCH_DISABLED_TOOLS.contains(&"send_message"),
            "send_message must remain visible to mika-arch — it writes to the agent's own \
             session history (constitutive), not platform state"
        );
    }

    #[test]
    fn test_mika_arch_disabled_tools_excludes_agent_self_state() {
        // Memory writes mutate the agent's own self-state (5 core memory blocks
        // + 4 facts categories, scoped to agent_id='mika-arch'). They are
        // constitutive of being an agent, not platform side-effects.
        // See mika/docs/architecture/review-guide.md § Orthogonality.
        let agent_self_state_tools = ["update_core_memory", "store_fact", "update_fact"];
        for tool in &agent_self_state_tools {
            assert!(
                !MIKA_ARCH_DISABLED_TOOLS.contains(tool),
                "{tool} must remain visible to mika-arch (agent self-state, not platform side-effect)"
            );
        }
    }

    /// Skills that front a write-capable tool (i.e., one in any well-known
    /// agent's `disabled_tools` denylist or otherwise mutational).
    ///
    /// **Keep in sync with skills that front any tool in
    /// `MIKA_ARCH_DISABLED_TOOLS` or any other well-known agent's denylist.**
    /// Adding a write-capable bundled skill without updating this list will
    /// allow `test_well_known_allowlist_excludes_write_capable_skills` to
    /// silently pass.
    ///
    /// Future migration: when the maintainability cluster lands and
    /// `SkillRegistry::load_for_agent` is extracted, this constant should be
    /// replaced by a derived predicate (`skills_with_write_tools()`) that
    /// inspects each skill's `tools.json` against the active denylist.
    /// Until then: hardcoded list, explicit naming for greppability.
    const WRITE_CAPABLE_SKILLS_FOR_INVARIANT_TEST: &[&str] = &[
        // Skills that front exec/write tools
        "self-dev",
        "self-dev-iterate",
        "self-dev-webhook-qa",
        "self-dev-webhook-ci",
        "self-dev-sprint",
        "dev-pilot",
        "build-mika",
        "deploy-mika",
        "agents-teams",
        "address-pr-comments",
        "resolve-pr-conflicts",
        "self-check",
        // QA skills that front pr_merge/run_gh-write
        "qa-review",
        "qa-review-build-callback",
        // Skill management
        "skill-review",
        // Permission policy executes side-effects
        "permission-policy",
    ];

    /// Invariant: well-known agents that declare a `[skills].allowlist` (i.e.
    /// they're advertised as scoped/read-only) must not have any write-capable
    /// bundled skill in their active set after the allowlist is applied.
    ///
    /// This protects against the silent-reorder regression flagged by the
    /// testing reviewer: if a future refactor reorders `apply_identity_allowlist`
    /// vs `apply_overrides`, or if a new write-capable skill ships and isn't
    /// added to the denylist tracker above, this test fails loud.
    #[test]
    fn test_well_known_allowlist_excludes_write_capable_skills() {
        for spec in WELL_KNOWN_AGENTS {
            // Render identity for this agent. Static-no-identity agents skip;
            // computed agents use test settings.
            let identity_toml = match render_identity_content(spec, &test_settings_with_kg_roots())
            {
                Ok(t) => t,
                Err(_) => continue,
            };
            let identity: crate::prompt::Identity = match toml::from_str(&identity_toml) {
                Ok(i) => i,
                Err(_) => continue,
            };
            let allowlist = match identity.skills.allowlist {
                Some(a) if !a.is_empty() => a,
                _ => continue, // Agents without an allowlist aren't gated by this invariant.
            };

            for entry in &allowlist {
                assert!(
                    !WRITE_CAPABLE_SKILLS_FOR_INVARIANT_TEST.contains(&entry.as_str()),
                    "well-known agent '{}' has write-capable skill '{}' in its [skills].allowlist. \
                     Either remove it from the allowlist or remove it from \
                     WRITE_CAPABLE_SKILLS_FOR_INVARIANT_TEST (with justification).",
                    spec.name,
                    entry,
                );
            }
        }
    }

    #[test]
    fn test_provision_correct_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, &test_settings_with_kg_roots(), false);

        let dev_identity = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-dev").join("identity.toml"),
        )
        .unwrap();
        assert!(dev_identity.contains("name = \"Dev\""));
        assert!(dev_identity.contains("emoji = \"🛠\""));

        let qa_identity = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-qa").join("identity.toml"),
        )
        .unwrap();
        assert!(qa_identity.contains("name = \"QA\""));
        assert!(qa_identity.contains("emoji = \"🔍\""));

        let relay_identity = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-relay").join("identity.toml"),
        )
        .unwrap();
        assert!(relay_identity.contains("name = \"Relay\""));
        assert!(relay_identity.contains("emoji = \"🔑\""));
    }

    #[test]
    fn test_provision_correct_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, &test_settings_with_kg_roots(), false);

        let dev_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-dev").join("soul.md"))
                .unwrap();
        assert!(dev_soul.contains("Mika Dev"));
        assert!(dev_soul.contains("Development Agent"));

        let qa_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-qa").join("soul.md"))
                .unwrap();
        assert!(qa_soul.contains("Mika QA"));
        assert!(qa_soul.contains("Quality Assurance"));

        let relay_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-relay").join("soul.md"))
                .unwrap();
        assert!(relay_soul.contains("Mika Relay"));
        assert!(relay_soul.contains("Permission Relay Agent"));

        let arch_soul =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-arch").join("soul.md"))
                .unwrap();
        assert!(arch_soul.contains("Mika Architect"));
        assert!(arch_soul.contains("Plan Review Agent"));
        // Foundational references section guards against accidental deletion of the citation
        // surface that mika-arch consults during reviews. See PR #866 / mika#860.
        assert!(arch_soul.contains("## Foundational references"));
        assert!(arch_soul.contains("docs/design/north-star.md"));
        assert!(arch_soul.contains("required-tools-gate-evasion-patterns-2026-04-28.md"));
    }

    #[test]
    fn test_provision_relay_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, &test_settings_with_kg_roots(), false);

        let relay_config = fs::read_to_string(
            mika_common::agent::agent_dir(home, "mika-relay").join("config.toml"),
        )
        .unwrap();
        assert!(relay_config.contains("anthropic_model = \"claude-haiku-4-5-20251001\""));
        assert!(relay_config.contains("llm_max_tokens = 1024"));

        // mika-dev should have the default config.toml (not overwritten)
        let dev_config =
            fs::read_to_string(mika_common::agent::agent_dir(home, "mika-dev").join("config.toml"))
                .unwrap();
        assert!(!dev_config.contains("claude-haiku"));
    }

    #[test]
    fn test_provision_skips_existing_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        // Create mika-dev with custom soul
        mika_common::home::bootstrap_agent(home, "mika-dev").unwrap();
        let dev_home = mika_common::agent::agent_dir(home, "mika-dev");
        fs::write(dev_home.join("soul.md"), "custom soul content").unwrap();

        provision_well_known_agents(home, &test_settings_with_kg_roots(), false);

        // mika-dev should keep custom soul
        let dev_soul = fs::read_to_string(dev_home.join("soul.md")).unwrap();
        assert_eq!(dev_soul, "custom soul content");

        // mika-qa should be created fresh
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_provision_partial_state() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        // Only mika-dev exists
        mika_common::home::bootstrap_agent(home, "mika-dev").unwrap();
        assert!(!mika_common::agent::agent_exists(home, "mika-qa"));

        provision_well_known_agents(home, &test_settings_with_kg_roots(), false);

        // mika-qa should now exist
        assert!(mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_provision_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        fs::create_dir_all(home.join("agents")).unwrap();

        provision_well_known_agents(home, &test_settings_with_kg_roots(), true);

        // No agents should be created
        assert!(!mika_common::agent::agent_exists(home, "mika-dev"));
        assert!(!mika_common::agent::agent_exists(home, "mika-qa"));
    }

    #[test]
    fn test_seed_skill_overrides_mika_dev() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let mut db = Database::open(&db_path).unwrap();
        db.register_agent("mika-dev", "Dev", "/tmp/mika-dev")
            .unwrap();

        seed_well_known_skill_overrides(&mut db, "mika-dev");

        let overrides = db.get_skill_overrides("mika-dev").unwrap();
        assert_eq!(overrides.len(), MIKA_DEV.disabled_skills.len());
        for ovr in &overrides {
            assert_eq!(ovr.enabled, Some(false));
            assert!(
                MIKA_DEV.disabled_skills.contains(&ovr.skill_name.as_str()),
                "unexpected override: {}",
                ovr.skill_name
            );
        }
    }

    #[test]
    fn test_seed_skill_overrides_mika_qa() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let mut db = Database::open(&db_path).unwrap();
        db.register_agent("mika-qa", "QA", "/tmp/mika-qa").unwrap();

        seed_well_known_skill_overrides(&mut db, "mika-qa");

        let overrides = db.get_skill_overrides("mika-qa").unwrap();
        assert_eq!(overrides.len(), MIKA_QA.disabled_skills.len());
        for ovr in &overrides {
            assert_eq!(ovr.enabled, Some(false));
            assert!(
                MIKA_QA.disabled_skills.contains(&ovr.skill_name.as_str()),
                "unexpected override: {}",
                ovr.skill_name
            );
        }
    }

    #[test]
    fn test_seed_skill_overrides_skips_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let mut db = Database::open(&db_path).unwrap();
        db.register_agent("mika-dev", "Dev", "/tmp/mika-dev")
            .unwrap();

        // Set a custom override first
        db.set_skill_enabled("mika-dev", "some-custom-skill", false)
            .unwrap();

        // Now seed — should not add anything since overrides exist
        seed_well_known_skill_overrides(&mut db, "mika-dev");

        let overrides = db.get_skill_overrides("mika-dev").unwrap();
        // Should only have the one custom override, not the well-known ones
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].skill_name, "some-custom-skill");
    }

    #[test]
    fn test_seed_skill_overrides_mika_relay() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let mut db = Database::open(&db_path).unwrap();
        db.register_agent("mika-relay", "Relay", "/tmp/mika-relay")
            .unwrap();

        seed_well_known_skill_overrides(&mut db, "mika-relay");

        let overrides = db.get_skill_overrides("mika-relay").unwrap();
        assert_eq!(overrides.len(), MIKA_RELAY.disabled_skills.len());
        for ovr in &overrides {
            assert_eq!(ovr.enabled, Some(false));
            assert!(
                MIKA_RELAY
                    .disabled_skills
                    .contains(&ovr.skill_name.as_str()),
                "unexpected override: {}",
                ovr.skill_name
            );
        }
    }

    #[test]
    fn test_relay_only_allows_permission_policy() {
        // Verify that permission-policy is NOT in the disabled list
        assert!(
            !MIKA_RELAY.disabled_skills.contains(&"permission-policy"),
            "permission-policy must not be disabled for mika-relay"
        );
    }

    #[test]
    fn test_relay_disables_all_bundled_skills_except_permission_policy() {
        // Comprehensive check: every bundled skill except permission-policy
        // must be in mika-relay's disabled list. This catches new skills
        // added to the bundled set without updating mika-relay.
        let all_names = crate::bundled_skills::all_bundled_skill_names();
        for name in &all_names {
            if name.eq_ignore_ascii_case("permission-policy") {
                continue;
            }
            assert!(
                MIKA_RELAY
                    .disabled_skills
                    .iter()
                    .any(|d| d.eq_ignore_ascii_case(name)),
                "bundled skill '{}' is not disabled for mika-relay — \
                 add it to MIKA_RELAY.disabled_skills",
                name
            );
        }
    }

    #[test]
    fn test_relay_config_toml_is_valid_toml() {
        // Verify the config string is valid TOML and contains expected fields
        let config: toml::Table =
            toml::from_str(MIKA_RELAY_CONFIG).expect("MIKA_RELAY_CONFIG must be valid TOML");
        assert_eq!(
            config.get("llm_provider").and_then(|v| v.as_str()),
            Some("anthropic")
        );
        assert_eq!(
            config.get("anthropic_model").and_then(|v| v.as_str()),
            Some("claude-haiku-4-5-20251001")
        );
        assert_eq!(
            config.get("llm_max_tokens").and_then(|v| v.as_integer()),
            Some(1024)
        );
    }

    #[test]
    fn test_seed_skill_overrides_non_well_known() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let mut db = Database::open(&db_path).unwrap();
        db.register_agent("custom-agent", "Custom", "/tmp/custom")
            .unwrap();

        seed_well_known_skill_overrides(&mut db, "custom-agent");

        let overrides = db.get_skill_overrides("custom-agent").unwrap();
        assert!(overrides.is_empty());
    }

    #[test]
    fn test_well_known_agent_specs_dev_qa_no_overlap() {
        // Dev and QA have complementary roles — dev builds, qa reviews. Their
        // disabled lists should not overlap EXCEPT for skills that are owned
        // by a third agent (mika-arch). Both dev and qa correctly exclude
        // mika-arch's review skills to prevent context pollution.
        // Skills owned by third-party agents (mika-arch) or operator-only skills
        // are legitimately disabled on both mika-dev and mika-qa.
        let allowed_overlap: &[&str] = &[
            "mika-arch-groom-ticket",
            "mika-arch-second-review",
            "dev-groom", // operator-only (#845) — disabled for both dev and qa
        ];
        for dev_skill in MIKA_DEV.disabled_skills {
            if allowed_overlap.contains(dev_skill) {
                continue;
            }
            assert!(
                !MIKA_QA.disabled_skills.contains(dev_skill),
                "skill '{}' is disabled for both mika-dev and mika-qa (and is not \
                 in the allowed overlap set for third-agent-owned skills)",
                dev_skill
            );
        }
    }

    #[test]
    fn test_relay_disables_superset_of_dev_and_qa() {
        // mika-relay should disable everything that both dev and qa disable
        // (plus more), since it only needs permission-policy.
        // Exception: permission-policy itself — qa disables it, relay keeps it.
        for dev_skill in MIKA_DEV.disabled_skills {
            assert!(
                MIKA_RELAY.disabled_skills.contains(dev_skill),
                "mika-relay should also disable '{}' (disabled for mika-dev)",
                dev_skill
            );
        }
        for qa_skill in MIKA_QA.disabled_skills {
            if *qa_skill == "permission-policy" {
                continue; // relay's sole purpose is permission-policy
            }
            assert!(
                MIKA_RELAY.disabled_skills.contains(qa_skill),
                "mika-relay should also disable '{}' (disabled for mika-qa)",
                qa_skill
            );
        }
    }

    // -- mika-arch tests --

    #[test]
    fn test_find_well_known_agent_mika_arch() {
        let agent = find_well_known_agent("mika-arch").unwrap();
        assert_eq!(agent.name, "mika-arch");
        assert_eq!(agent.display_name, "Architect");
        assert_eq!(agent.emoji, "🏛");
    }

    #[test]
    fn test_mika_arch_uses_empty_disabled_skills() {
        // mika-arch uses identity allowlist, not denylist
        assert!(
            MIKA_ARCH.disabled_skills.is_empty(),
            "mika-arch should have empty disabled_skills (uses identity allowlist)"
        );
    }

    #[test]
    fn test_mika_arch_has_llm_overrides() {
        assert_eq!(MIKA_ARCH.llm_overrides.len(), 2);
        assert_eq!(
            MIKA_ARCH.llm_overrides[0].skill_name,
            "mika-arch-groom-ticket"
        );
        assert_eq!(MIKA_ARCH.llm_overrides[0].provider, "anthropic");
        assert_eq!(MIKA_ARCH.llm_overrides[0].model, "claude-opus-4-7");
        assert_eq!(
            MIKA_ARCH.llm_overrides[1].skill_name,
            "mika-arch-second-review"
        );
        assert_eq!(MIKA_ARCH.llm_overrides[1].provider, "anthropic");
        assert_eq!(MIKA_ARCH.llm_overrides[1].model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_mika_arch_config_toml_is_valid_toml() {
        let config: toml::Value =
            toml::from_str(MIKA_ARCH_CONFIG).expect("MIKA_ARCH_CONFIG should be valid TOML");
        assert_eq!(config["llm_provider"].as_str(), Some("openrouter"));
        assert_eq!(
            config["openrouter_model"].as_str(),
            Some("moonshotai/kimi-k2.5")
        );
    }

    #[test]
    fn test_mika_arch_identity_toml_has_allowlist_and_disabled_tools() {
        let rendered = build_mika_arch_identity(&test_settings_with_kg_roots())
            .expect("MIKA_ARCH identity should render with valid settings");
        let identity: crate::prompt::Identity =
            toml::from_str(&rendered).expect("rendered identity should parse");
        assert_eq!(identity.name, "Architect");
        assert_eq!(identity.emoji, "🏛");
        assert!(identity.kg.enabled);
        let docs_roots = identity.kg.docs_roots.expect("should have docs_roots");
        assert_eq!(docs_roots.len(), 2);
        let allowlist = identity.skills.allowlist.expect("should have allowlist");
        assert_eq!(allowlist.len(), 2);
        assert!(allowlist.contains(&"mika-arch-groom-ticket".to_string()));
        assert!(allowlist.contains(&"mika-arch-second-review".to_string()));
        // Tool denylist must be present and contain the load-bearing items.
        assert_eq!(
            identity.tools.disabled.len(),
            MIKA_ARCH_DISABLED_TOOLS.len()
        );
        assert!(
            identity
                .tools
                .disabled
                .contains(&"pr_merge_with_gate".to_string())
        );
        assert!(identity.tools.disabled.contains(&"a2a_call".to_string()));
    }

    #[test]
    fn test_provision_mika_arch() {
        let home = tempfile::tempdir().unwrap();
        provision_well_known_agents(home.path(), &test_settings_with_kg_roots(), false);

        let arch_home = mika_common::agent::agent_dir(home.path(), "mika-arch");
        assert!(arch_home.exists(), "mika-arch agent directory should exist");

        // Check identity.toml
        let identity_content = fs::read_to_string(arch_home.join("identity.toml")).unwrap();
        assert!(identity_content.contains("Architect"));
        assert!(identity_content.contains("[skills]"));
        assert!(identity_content.contains("allowlist"));

        // Check soul.md
        let soul_content = fs::read_to_string(arch_home.join("soul.md")).unwrap();
        assert!(soul_content.contains("Mika Architect"));
        assert!(soul_content.contains("Principal-Engineer"));

        // Check config.toml
        let config_content = fs::read_to_string(arch_home.join("config.toml")).unwrap();
        assert!(config_content.contains("openrouter"));
        assert!(config_content.contains("kimi-k2.5"));
    }

    #[test]
    fn test_seed_skill_overrides_mika_arch() {
        let mut db = Database::open_in_memory().unwrap();
        db.register_agent("mika-arch", "Architect", "🏛").unwrap();

        seed_well_known_skill_overrides(&mut db, "mika-arch");

        let overrides = db.get_skill_overrides("mika-arch").unwrap();
        // Should have 2 LLM overrides (no disabled_skills for mika-arch)
        assert_eq!(overrides.len(), 2);

        let groom = overrides
            .iter()
            .find(|o| o.skill_name == "mika-arch-groom-ticket")
            .expect("groom-ticket override should exist");
        assert_eq!(groom.llm_provider.as_deref(), Some("anthropic"));
        assert_eq!(groom.llm_model.as_deref(), Some("claude-opus-4-7"));

        let review = overrides
            .iter()
            .find(|o| o.skill_name == "mika-arch-second-review")
            .expect("second-review override should exist");
        assert_eq!(review.llm_provider.as_deref(), Some("anthropic"));
        assert_eq!(review.llm_model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn test_well_known_agents_includes_mika_arch() {
        assert_eq!(WELL_KNOWN_AGENTS.len(), 4);
        assert!(
            WELL_KNOWN_AGENTS.iter().any(|a| a.name == "mika-arch"),
            "WELL_KNOWN_AGENTS should include mika-arch"
        );
    }
}
