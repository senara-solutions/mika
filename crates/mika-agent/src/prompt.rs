use crate::db::{
    Commitment, CoreMemoryEntry, Preference, TaskHealthSummary, core_memory_section_names,
};

/// Preference-key prefix used for user stop-signals (mika#1813).
///
/// When the user asks the agent to stop bringing up a topic, the agent persists
/// the signal via `store_fact(category='preference', key='stop_topic_<slug>', …)`.
/// The prefix is loaded by `search_preferences(STOP_TOPIC_PREFIX)` at both
/// conversation and silent turn assembly and injected into the system prompt as
/// a dedicated `<stopped-topics>` block.
///
/// This is the structural half of the fix — the block re-appears on every
/// future turn even if the model forgets. The prompt discipline (persist on
/// refusal; consult before initiating) is the intent half.
pub const STOP_TOPIC_PREFIX: &str = "stop_topic_";

/// Filter a `search_preferences` result set down to strict stop-topic rows
/// (mika#1813).
///
/// `db::search_preferences(query)` runs `WHERE agent_id = ?1 AND (category LIKE
/// '%query%' OR value LIKE '%query%')`. Two ways that under-constrains for
/// `STOP_TOPIC_PREFIX`:
///
/// 1. **`_` is a SQLite LIKE wildcard.** `LIKE '%stop_topic_%'` matches any
///    `stop_topic<X>` where `X` is any character, including an unrelated key
///    like `stop_topicalization`.
/// 2. **OR on `value`.** A preference whose value happens to contain
///    `stop_topic_` as a substring (a `task_policy_*` row paraphrasing the
///    convention, a prose preference referencing it) is surfaced as a stopped
///    topic and would mute unrelated axes (AC4 leak).
///
/// The load path (`AgentContext::load_agent_context`) applies this filter so
/// only preferences whose `category` is a strict `stop_topic_` prefix are
/// injected into `<stopped-topics>`.
pub fn filter_stop_topic_preferences(prefs: Vec<Preference>) -> Vec<Preference> {
    prefs
        .into_iter()
        .filter(|p| p.category.starts_with(STOP_TOPIC_PREFIX))
        .collect()
}
use chrono::{DateTime, NaiveTime, Utc};
use mika_common::{agent, team};
use serde::Deserialize;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Heuristic conversion ratio for character-count to token-count approximation.
///
/// Used by [`truncate_to_token_budget()`] (and any other code path that needs to
/// cap summary content by approximate token count without invoking a real
/// tokenizer). The 4:1 ratio is acceptable for English and is conservative
/// enough for cap-enforcement; exact tokenization is not required because the
/// cap is a soft policy, not a hard limit, and tokenizers vary by provider.
const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

/// Configuration for periodic memory reflection.
#[derive(Debug, Deserialize, Clone)]
pub struct ReflectionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_reflection_time")]
    pub time: String,
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub timezone: Option<String>,
}

fn default_reflection_time() -> String {
    "20:00".to_string()
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            time: default_reflection_time(),
            notify: false,
            timezone: None,
        }
    }
}

impl ReflectionConfig {
    /// Parse the configured time string (HH:MM format) into a NaiveTime.
    /// Returns None if the format is invalid.
    pub fn parse_time(&self) -> Option<NaiveTime> {
        NaiveTime::parse_from_str(&self.time, "%H:%M").ok()
    }
}

/// Configuration for periodic heartbeat task.
#[derive(Debug, Deserialize, Clone)]
pub struct HeartbeatConfig {
    #[serde(default = "default_heartbeat_enabled")]
    pub enabled: bool,
}

fn default_heartbeat_enabled() -> bool {
    true // Back-compat: heartbeat is enabled by default
}

/// Configuration for the periodic curator review task (mika#1584).
///
/// The curator periodically scans agent-authored skills for idle candidates
/// and proposes archival. It never auto-archives — it proposes only.
#[derive(Debug, Deserialize, Clone)]
pub struct CuratorConfig {
    /// Interval in hours between curator reviews. Converted to a cron expression
    /// at registration time. Default: 24 (daily at 03:00 UTC).
    pub interval_hours: Option<u32>,
    /// Number of days a skill must be idle before being proposed for archival.
    /// Default: 30.
    pub max_idle_days: Option<u32>,
}

/// Per-agent Knowledge Graph configuration from `[kg]` section of `identity.toml`.
///
/// Controls whether the KG subsystem runs for this agent and which docs root
/// it reads from. When `enabled` is `false`, no KG subsystem components
/// (`LexicalIngestor`, `SubjectExtractor`, `SubjectEntityResolver`) are
/// constructed for the agent. When both `docs_roots` and `docs_root` are `None`,
/// the global fallback chain applies (#738): `MIKA_KG_DOCS_ROOT` env >
/// `kg_docs_root` config > `<CWD>/docs/solutions`.
///
/// ## Plural vs singular
///
/// `docs_roots` (plural, #798) takes precedence over `docs_root` (singular, #778)
/// when both are set on the same agent. Setting both emits a
/// `kg_docs_roots_singular_ignored` warn at resolver time.
#[derive(Debug, Deserialize, Clone)]
pub struct KgIdentityConfig {
    /// Whether KG ingestion/extraction/resolution runs for this agent.
    /// Default: `true` (preserves current behavior for existing agents).
    #[serde(default = "default_kg_enabled")]
    pub enabled: bool,
    /// Absolute path to the docs root this agent's KG reads from.
    /// When `None`, falls back to the global resolver (#738).
    pub docs_root: Option<PathBuf>,
    /// Multiple docs root paths for multi-corpus agents (#798).
    /// When `Some` and non-empty, takes precedence over `docs_root`.
    /// TOML format: `docs_roots = ["/path/a", "/path/b"]`.
    #[serde(default)]
    pub docs_roots: Option<Vec<PathBuf>>,
}

fn default_kg_enabled() -> bool {
    true
}

impl Default for KgIdentityConfig {
    fn default() -> Self {
        Self {
            enabled: default_kg_enabled(),
            docs_root: None,
            docs_roots: None,
        }
    }
}

/// Skill provisioning config from the `[skills]` block in identity.toml.
///
/// Used by well-known agents (mika-arch, etc.) to declare their active skill set
/// in identity.toml rather than via `skill_overrides` DB rows. When `allowlist`
/// is `Some` and non-empty, only the listed skills are kept active; all others
/// are evicted from the registry at startup.
///
/// User-defined agents should omit this section entirely — their skill set is
/// managed via the `mika skills enable/disable` CLI and DB-backed `skill_overrides`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct SkillsIdentityConfig {
    /// When `Some` and non-empty, only these skill names are kept active.
    /// All other bundled skills are evicted from the registry at startup.
    /// `None` or empty = all bundled skills enabled (current default).
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
    /// Whether this agent is allowed to author skills via `skill_manage` (mika#1582).
    /// `None` or `Some(false)` = authoring disabled (default).
    /// `Some(true)` = authoring enabled.
    #[serde(default)]
    pub allow_authoring: Option<bool>,
}

/// Session config from the `[session]` block in identity.toml (mika#1401).
///
/// Makes an agent **single-session-by-construction**: when `singleton = true`,
/// every conversational invocation (`mika ask`, `mika chat`, HTTP `/send`) reuses
/// one canonical session instead of minting a fresh `Uuid::new_v4()` per ask. The
/// invariant is enforced by the engine, not by remembering to pass `--session-id`.
///
/// Opt-in — absent or `singleton = false` preserves the default (random UUID per
/// ask). Explicit `--session-id` always overrides the canonical session.
///
/// ```toml
/// [session]
/// singleton = true
/// canonical_id = "00000000-0000-0000-0000-000000000000"  # optional
/// ```
///
/// **Compaction interaction:** compaction already keys on `agent_id`, not
/// `session_id`, so a singleton agent is unaffected — the single canonical
/// session simply accumulates all history. See mika#1401.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct SessionIdentityConfig {
    /// When `true`, this agent uses a single canonical session for all
    /// conversational interactions. The session persists across invocations
    /// and is never ended by `end_session`. Default: `false`.
    #[serde(default)]
    pub singleton: bool,
    /// Explicit canonical session ID. When absent and `singleton = true`,
    /// the engine derives one as `canonical-{agent_id}`. Exists specifically
    /// for mika-prime's operator-chosen zero-UUID.
    #[serde(default)]
    pub canonical_id: Option<String>,
}

/// Tool visibility config from the `[tools]` block in identity.toml.
///
/// Used by well-known agents (mika-arch, etc.) to deny specific built-in tools
/// at the LLM-presentation layer. Listed tool names are filtered out of the tool
/// array sent to the model — the model never sees them, cannot call them, and
/// cannot be prompt-injected into trying. Defense-in-depth for read-only agents.
///
/// Filter is applied in `agent::apply_agent_tool_visibility()` after the registry
/// is materialized for the API call. The shared `Arc<ToolRegistry>` is unchanged.
///
/// Future migration: when well-known agents move from denylist to allowlist for
/// tools (mirroring the skill allowlist), add a sibling `allowlist` field here
/// and update `apply_agent_tool_visibility` to handle both.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ToolsIdentityConfig {
    /// Tool names that must not appear in this agent's LLM tool array.
    /// Empty = all registry tools visible (current default).
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// Per-agent context-injection config from `[context]` section of `identity.toml`.
///
/// Top-level holder for prompt-assembly toggles. Each context block (summary,
/// future: tools, system, memory) gets its own nested section.
///
/// Use case: well-known agents (mika-arch) where a specific context block is
/// a known leak source (mika#1009). Operator opts the agent out via
/// `identity.toml`; production behavior changes on next mika-spirit restart.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ContextIdentityConfig {
    #[serde(default)]
    pub summary: ContextSummaryConfig,
}

/// `[context.summary]` subsection — controls injection of the conversational
/// summary block into the system prompt at prompt-assembly time.
///
/// `inject = false` is a load-prevention gate, not injection-prevention:
/// `db.load_conversation_summary()` is not called, the summary is not
/// deserialized, and the result is not available to any downstream code path
/// in the same turn. This is the correct shape for mika#1009's leak class.
#[derive(Debug, Deserialize, Clone)]
pub struct ContextSummaryConfig {
    /// Whether to load and inject the conversational summary into the system prompt.
    /// Default: `true` (preserves current behavior for all existing agents).
    /// Set to `false` for agents where summary leakage is a known problem.
    /// (Axis 4 — mika#1019)
    #[serde(default = "default_inject_summary")]
    pub inject: bool,

    /// Optional token cap applied to the summary by the gated injection path.
    ///
    /// The cap is mode-agnostic at the schema level. Today the only caller
    /// applying it gates on silent-mode detection — see [`load_gated_summary()`]
    /// in `agent.rs`. Tomorrow another gate could reuse the same field
    /// without a rename.
    ///
    /// When set:
    ///   - `Some(0)` → load-omit sentinel: summary is not injected. Same code
    ///     path as Axis 4's `inject = false` short-circuit, but conditional on
    ///     the in-code gate firing (e.g., silent-mode only). NOT interpreted as
    ///     "zero-token cap"; treated as a structural omit signal.
    ///   - `Some(n)` for n > 0 → summary truncated to approximately
    ///     `n * CHARS_PER_TOKEN_ESTIMATE` characters before injection.
    ///   - `None` → no cap (default; current behavior).
    ///
    /// Token approximation is heuristic via [`CHARS_PER_TOKEN_ESTIMATE`] (= 4).
    /// (Axis 3 — mika#1021)
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

fn default_inject_summary() -> bool {
    true
}

impl Default for ContextSummaryConfig {
    fn default() -> Self {
        Self {
            inject: default_inject_summary(),
            max_tokens: None,
        }
    }
}

/// Truncate a summary string to approximately `max_tokens`, using
/// [`CHARS_PER_TOKEN_ESTIMATE`] for the conversion. Cuts at a word boundary
/// near the budget to avoid mid-word truncation in the LLM's view. Appends
/// a truncation marker so the model knows content was elided.
///
/// Heuristic: not exact tokenization. Acceptable because the cap is a soft
/// policy, not a hard limit, and tokenizers vary by provider.
pub fn truncate_to_token_budget(summary: &str, max_tokens: usize) -> String {
    let max_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN_ESTIMATE);
    if summary.len() <= max_chars {
        return summary.to_string();
    }
    // Cut at the last word boundary at or before `max_chars`.
    // floor_char_boundary ensures we don't slice mid-codepoint.
    let safe_boundary = summary.floor_char_boundary(max_chars);
    let cut = summary[..safe_boundary]
        .rfind(char::is_whitespace)
        .unwrap_or(safe_boundary);
    let mut truncated = summary[..cut].to_string();
    truncated.push_str("\n[… summary truncated to fit silent-mode budget …]");
    truncated
}

/// Agent identity loaded from ~/.mika/identity.toml.
#[derive(Debug, Deserialize, Clone)]
pub struct Identity {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_emoji")]
    pub emoji: String,
    #[serde(default)]
    pub reflection: Option<ReflectionConfig>,
    #[serde(default)]
    pub heartbeat: Option<HeartbeatConfig>,
    #[serde(default)]
    pub kg: KgIdentityConfig,
    #[serde(default)]
    pub skills: SkillsIdentityConfig,
    #[serde(default)]
    pub tools: ToolsIdentityConfig,
    #[serde(default)]
    pub context: ContextIdentityConfig,
    #[serde(default)]
    pub session: SessionIdentityConfig,
    #[serde(default)]
    pub curator: Option<CuratorConfig>,
}

fn default_name() -> String {
    "Mika".to_string()
}

fn default_emoji() -> String {
    "✦".to_string()
}

impl Default for Identity {
    fn default() -> Self {
        Self {
            name: default_name(),
            emoji: default_emoji(),
            reflection: None,
            heartbeat: None,
            kg: KgIdentityConfig::default(),
            skills: SkillsIdentityConfig::default(),
            tools: ToolsIdentityConfig::default(),
            context: ContextIdentityConfig::default(),
            session: SessionIdentityConfig::default(),
            curator: None,
        }
    }
}

/// Load identity from ~/.mika/identity.toml.
///
/// Returns defaults if the file is missing.
///
/// On parse error: emits `error!` and applies fail-closed semantics for
/// well-known agents (e.g., mika-arch). Specifically:
/// - For well-known agents, returns an `Identity` with `skills.allowlist =
///   Some(vec!["__fail_closed__"])` (a sentinel that matches no real skill,
///   evicting all bundled skills) and `tools.disabled` = full mutational set.
///   The agent is effectively neutered until the operator fixes the file.
/// - For user-defined agents, falls back to `Identity::default()` (current
///   behavior — they have no security contract to preserve).
///
/// The well-known check uses the home_dir's last component matched against
/// `find_well_known_agent`. This keeps the discrimination at the load layer
/// where it belongs, without requiring every caller to know the agent's
/// well-known status.
pub fn load_identity(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_identity_or_fail_closed(&content, home_dir, &path),
        Err(_) => Identity::default(),
    }
}

/// Async version of [`load_identity`] using `tokio::fs` to avoid
/// blocking the async runtime.
pub async fn load_identity_async(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => parse_identity_or_fail_closed(&content, home_dir, &path),
        Err(_) => Identity::default(),
    }
}

fn parse_identity_or_fail_closed(content: &str, home_dir: &Path, path: &Path) -> Identity {
    match toml::from_str::<Identity>(content) {
        Ok(identity) => identity,
        Err(parse_err) => {
            let agent_name = home_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("<unknown>");

            // Well-known agents fail closed; user agents fall back to defaults.
            let is_well_known =
                crate::well_known_agents::find_well_known_agent(agent_name).is_some();

            if is_well_known {
                tracing::error!(
                    agent = %agent_name,
                    path = %path.display(),
                    error = %parse_err,
                    "well-known agent identity.toml is malformed — failing CLOSED \
                     (all skills evicted, all mutational tools denied) until the \
                     operator fixes it"
                );
                fail_closed_identity()
            } else {
                tracing::error!(
                    agent = %agent_name,
                    path = %path.display(),
                    error = %parse_err,
                    "agent identity.toml is malformed — falling back to defaults"
                );
                Identity::default()
            }
        }
    }
}

/// Fail-closed `Identity` for well-known agents whose `identity.toml` failed
/// to parse. Sentinel allowlist matches no real skill (evicts all bundled
/// skills); denylist contains every mutational built-in tool to prevent
/// the agent from acting until the operator fixes the file.
fn fail_closed_identity() -> Identity {
    Identity {
        name: default_name(),
        emoji: default_emoji(),
        reflection: None,
        heartbeat: None,
        kg: KgIdentityConfig::default(),
        skills: SkillsIdentityConfig {
            allowlist: Some(vec!["__fail_closed_no_skills__".to_string()]),
            allow_authoring: None,
        },
        tools: ToolsIdentityConfig {
            disabled: crate::well_known_agents::MIKA_ARCH_DISABLED_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        },
        // Fail-closed must enforce inject=false so a malformed identity.toml
        // cannot re-enable summary injection (mika#1009 leak protection).
        context: ContextIdentityConfig {
            summary: ContextSummaryConfig {
                inject: false,
                max_tokens: None,
            },
        },
        session: SessionIdentityConfig::default(),
        curator: None,
    }
}

/// Resolve the canonical session ID for a singleton agent (mika#1401).
///
/// Returns `None` when the agent is not `singleton` — callers then mint a
/// fresh `Uuid::new_v4()` per invocation (the default behavior). When
/// `singleton = true`, returns the operator-chosen `canonical_id` if set,
/// else the derived `canonical-{agent_id}` (prefix-typed sibling of
/// `system-{agent_id}`, structurally exempt from `prune_old_sessions`).
pub fn resolve_canonical_session_id(identity: &Identity, agent_id: &str) -> Option<String> {
    if !identity.session.singleton {
        return None;
    }
    Some(
        identity
            .session
            .canonical_id
            .clone()
            .unwrap_or_else(|| format!("canonical-{agent_id}")),
    )
}

/// Context needed to build the system prompt.
pub struct PromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub is_onboarding: bool,
    pub current_utc: DateTime<Utc>,
    pub timezone: Option<String>,
    /// Global home directory, used to discover teams.
    /// When `None`, the teams section is omitted from the prompt.
    pub global_home_dir: Option<&'a Path>,
    /// The channel this message arrived on (e.g., "telegram", "cli").
    /// When `None`, the channel section is omitted (team agents, tests).
    pub channel_type: Option<&'a str>,
    /// Whether Telegram integration is configured (chat_id exists in customer_config).
    pub telegram_configured: bool,
    /// Per-agent home directory (e.g. `~/.mika/agents/mika/`).
    /// Surfaced in the Tool Usage section so the agent knows write_agent_file's base path.
    pub home_dir: Option<&'a Path>,
    /// When set, this is a callback result turn. Injects a guard section telling the agent
    /// to process the results directly and not spawn new long-running tasks.
    pub callback_context: Option<&'a str>,
    /// Active stop-signals (`stop_topic_*` preferences, mika#1813).
    /// Loaded via `db.search_preferences(STOP_TOPIC_PREFIX)` at turn assembly.
    /// When non-empty, rendered as a `<stopped-topics>` block so the agent sees
    /// which subjects the user has explicitly asked not to be re-raised on.
    pub stopped_topics: &'a [Preference],
}

fn onboarding_prompt() -> String {
    let section_names = core_memory_section_names();
    format!(
        "## First Session\n\
         This is your first conversation with the user. Introduce yourself briefly and warmly. \
         Ask who they are and what they're working on. Use update_core_memory to seed all \
         {} blocks ({}) from their \
         responses. Also use store_fact(category=\"person\") to create a record for the user \
         with just their first name (e.g. name=\"Vincent\") and relationship \"The user\". \
         Keep it to 2-3 natural exchanges, then transition to being helpful \
         with whatever they need.",
        section_names.len(),
        section_names.join(", ")
    )
}

/// Write the soul content section (personality baseline from soul.md).
fn write_soul_section(prompt: &mut String, soul_content: &str) {
    if !soul_content.is_empty() {
        prompt.push_str(soul_content);
        prompt.push_str("\n\n");
    }
}

/// Write the identity section.
fn write_identity_section(prompt: &mut String, identity: &Identity) {
    write!(prompt, "## Identity\nYou are {}.\n\n", identity.name).unwrap();
}

/// Write the current time section with optional timezone.
fn write_time_section(prompt: &mut String, current_utc: DateTime<Utc>, timezone: Option<&str>) {
    prompt.push_str("## Current Time\n");
    writeln!(prompt, "UTC: {}", current_utc.format("%Y-%m-%dT%H:%M:%SZ")).unwrap();
    if let Some(tz) = timezone {
        writeln!(prompt, "User timezone: {tz}").unwrap();
    }
    prompt.push('\n');
}

/// Write the communication channel section.
/// Informs the agent which channel the conversation is on and which integrations are active.
/// Known valid channel types. Unknown channels are silently skipped to prevent
/// prompt injection via a compromised gateway sending arbitrary channel strings.
const VALID_CHANNELS: &[&str] = &["cli", "telegram", "whatsapp", "api"];

fn write_channel_section(
    prompt: &mut String,
    channel_type: Option<&str>,
    telegram_configured: bool,
) {
    // Only include recognized channels
    let valid_channel = channel_type.filter(|ch| VALID_CHANNELS.contains(ch));
    if valid_channel.is_none() && !telegram_configured {
        return;
    }
    prompt.push_str("## Communication Channel\n");
    if let Some(ch) = valid_channel {
        writeln!(prompt, "This conversation is happening via: {ch}").unwrap();
    }
    if telegram_configured {
        prompt.push_str(
            "Telegram integration is active. You can reach the user via Telegram using send_message.\n",
        );
    }
    prompt.push('\n');
}

/// Write the core memory section with `<core-memory>` XML delimiters.
/// An optional `description` is inserted between the heading and the data block.
fn write_core_memory_section(
    prompt: &mut String,
    core_memory: &[CoreMemoryEntry],
    description: Option<&str>,
) {
    prompt.push_str("## Core Memory\n");
    if let Some(desc) = description {
        prompt.push_str(desc);
        prompt.push_str("\n\n");
    }
    prompt.push_str("<core-memory>\n");
    for entry in core_memory {
        write!(prompt, "### {}\n{}\n\n", entry.key, entry.value).unwrap();
    }
    prompt.push_str("</core-memory>\n\n");
}

/// Build the system prompt from context.
pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    write_soul_section(&mut prompt, ctx.soul_content);
    write_identity_section(&mut prompt, ctx.identity);
    write_time_section(&mut prompt, ctx.current_utc, ctx.timezone.as_deref());
    write_channel_section(&mut prompt, ctx.channel_type, ctx.telegram_configured);
    write_core_memory_section(
        &mut prompt,
        ctx.core_memory,
        Some(
            "These are your persistent memory blocks, auto-loaded into this prompt on every turn. \
             The content below is already available — do NOT attempt to read it via read_agent_file \
             (core_memory is DB-backed, not filesystem-stored). \
             To modify core_memory, use the update_core_memory tool.",
        ),
    );

    // Callback turn guard
    if let Some(context) = ctx.callback_context {
        prompt.push_str("## Callback Result Turn\n");
        prompt.push_str(
            "A background task has completed and the results are provided in the user message below.\n\
             IMPORTANT: You MUST NOT submit new long-running tasks or create tasks during this turn.\n\
             Process the results, take any required follow-up actions (e.g., updating task status), \
             and then respond to the user with your analysis.\n\
             WARNING: Any <context type=\"tool_history\"> blocks in the conversation are summaries of PRIOR \
             turns — they do NOT mean you have already taken action in this turn. You MUST still call \
             any required tools yourself.\n\n",
        );
        prompt.push_str(context);
        prompt.push_str("\n\n");
    }

    // Stop-signals (mika#1813) — the user has explicitly asked NOT to be re-nagged
    // on these subjects. Rendered as a dedicated section so the model sees the
    // suppression list distinctly from the general core-memory context above.
    if !ctx.stopped_topics.is_empty() {
        prompt.push_str("## Stopped Topics\n");
        prompt.push_str(
            "The user has explicitly asked you NOT to re-initiate on these topics. \
             Do NOT proactively raise any listed subject in later turns. A direct \
             user question about a stopped topic is not a re-opening — you may still \
             answer it. The user re-opens a topic only by explicitly bringing it back.\n",
        );
        prompt.push_str("<stopped-topics>\n");
        for pref in ctx.stopped_topics {
            let cat = sanitize_label(&pref.category);
            let val = sanitize_label(&pref.value);
            writeln!(prompt, "- {cat}: {val}").unwrap();
        }
        prompt.push_str("</stopped-topics>\n\n");
    }

    // Instructions
    prompt.push_str("## Instructions\n");
    prompt.push_str("- Never fabricate information. If you don't know something, say so.\n");
    prompt.push_str(
        "- **Transparency rule:** If any tool call fails or returns a non-zero exit code, \
         always disclose the failure, what you did to recover, any side effects, \
         and the final confirmed state — never just say \"Done.\"\n",
    );
    prompt.push_str(
        "- **Tool history is observational only:** `<context type=\"tool_history\">` blocks attached to \
         previous messages are summaries of what happened in PAST turns. They are NOT confirmation \
         that you have executed those actions in the CURRENT turn. You must always call tools yourself \
         when action is required — never assume a tool_history entry means the work is already done.\n",
    );
    prompt.push_str(
        "- **Grounding rule:** NEVER claim the state of a downstream or adjacent system \
         (PR status, CI pipeline, deploy, merge readiness, branch health) unless a tool \
         result in this conversation explicitly confirms that exact state. A successful \
         tool result confirms only the specific action that tool performed — do not \
         extrapolate. NEVER fabricate URLs — every URL you include in a response must \
         come from a tool result, not from your own generation.\n  \
         BAD: Build tool returns \"Compilation succeeded\" → you say \"PR is ready for review.\"\n  \
         BAD: No tool calls → you say \"Comment posted: https://github.com/…#issuecomment-123\"\n  \
         GOOD: Build tool returns \"Compilation succeeded\" → you say \"The build passed.\"\n  \
         GOOD: run_gh posts a comment → you report the URL from the tool result.\n  \
         If you need downstream status, call the appropriate tool (e.g., check_task, \
         query_timeline) to verify it first.\n",
    );
    prompt.push_str(
        "- **Confirmation before action:** When the user asks an informational or status question \
         (e.g., \"can you list...\", \"what are...\", \"show me...\", \"did you...\", \
         \"what happened with...\", \"is X done?\", \"what's the status?\"), answer the question \
         and stop. Do not interpret questions as implicit requests to start multi-step \
         workflows, retry failed operations, or relaunch tasks. If a follow-up action may be \
         useful, suggest it and wait for explicit confirmation before proceeding.\n",
    );
    prompt.push_str(
        "- **Respect stop signals (persist):** When the user explicitly asks you to stop \
         bringing up a topic (words like \"stop\", \"arrête\", \"don't bring this up\", \
         \"no more\", \"assez\", \"laisse tomber\", \"oublie ça\"), acknowledge concisely AND \
         call `store_fact(category='preference', key='stop_topic_<short-slug>', \
         value='<one-line: what the user asked to stop, with today's date>')` in the SAME \
         turn. `<short-slug>` is a lowercase kebab-case identifier for the subject — for \
         example, subject `web-config` → `key='stop_topic_web-config'`; subject \
         `budget-review` → `key='stop_topic_budget-review'`. Choose a slug specific enough \
         to distinguish this subject from any other `stop_topic_*` key already in the \
         `## Stopped Topics` block. Do NOT re-initiate on that topic in later turns unless \
         the user re-opens it themselves. A direct user question about the topic is not a \
         re-opening — you may still answer it.\n",
    );
    prompt.push_str(
        "- **Respect stop signals (consult):** If a `## Stopped Topics` section is present \
         in this system prompt, do NOT proactively re-raise any listed subject. Direct \
         answers to user questions about a stopped topic remain OK.\n",
    );
    prompt.push_str(
        "- **No internal tags in responses:** Never include internal XML tags like <context>, \
         <callback_result>, <task-health>, or <rewind_reversals> in your responses. \
         These are system metadata injected for your context — they are not for user display.\n",
    );
    prompt.push_str(
        "- **Context priority:** When information from different sources conflicts, prefer in this order: \
         current user message > core memory > active skill context > conversation summary > \
         conversation history > search results. The user's latest message is always ground truth. \
         Core memory is agent-curated and actively maintained. Conversation summaries are lossy \
         derivatives of older history — treat them as helpful context, not authoritative.\n",
    );
    let section_names = core_memory_section_names();
    write!(
        prompt,
        "- You have {} memory blocks ({}),\n  each limited to ~500 tokens. Be concise and prioritize what matters most.\n",
        section_names.len(),
        section_names.join(", ")
    )
    .unwrap();

    // Onboarding prompt (only on first session)
    if ctx.is_onboarding {
        prompt.push('\n');
        prompt.push_str(&onboarding_prompt());
        prompt.push('\n');
    }

    // Agents & Teams section (only if multiple agents or teams are configured)
    if let Some(home_dir) = ctx.global_home_dir {
        let agents = agent::list_agents(home_dir);
        let teams = team::list_teams(home_dir);

        if agents.len() > 1 || !teams.is_empty() {
            prompt.push_str("\n## Agents & Teams\n");
            writeln!(
                prompt,
                "You are {} {}. Do not delegate tasks to yourself.\n",
                ctx.identity.emoji, ctx.identity.name
            )
            .unwrap();

            if agents.len() > 1 {
                prompt.push_str(
                    "You can delegate tasks to other agents using `delegate_task`. Available agents:\n",
                );
                for name in &agents {
                    let agent_home = agent::agent_dir(home_dir, name);
                    let identity = load_identity(&agent_home);
                    writeln!(prompt, "- {} ({} {})", name, identity.emoji, identity.name).unwrap();
                }
            }

            if !teams.is_empty() {
                prompt.push_str("You can run team workflows using `run_team`. Available teams:\n");
                for name in &teams {
                    match team::load_team(home_dir, name) {
                        Ok(def) => {
                            writeln!(prompt, "- {} ({} agents)", name, def.agents.len()).unwrap();
                        }
                        Err(_) => {
                            writeln!(prompt, "- {} (unable to load)", name).unwrap();
                        }
                    }
                }
            }

            prompt.push_str(
                "Use `list_agents` for details. Use `get_team_status`/`get_team_history` for run results.\n",
            );
        }

        // Agent creation guidance (always shown when home_dir is available)
        prompt.push_str(
            "\nYou can create new agents using the `create_agent` tool. When creating agents \
             based on real people, use your knowledge to craft an appropriate personality in the \
             `soul` parameter — capture their known communication style, philosophy, and expertise. \
             Use `web_search` if you need to research someone you're less familiar with. The `name` \
             parameter must be lowercase with hyphens (e.g. \"elon-musk\"), while `display_name` is \
             the human-readable name (e.g. \"Elon Musk\").\n",
        );
        prompt.push_str(
            "\nTo create a team, use the `create_team` tool — do NOT write team.toml manually. \
             Provide a `name`, `orchestrator` (must be one of the agents), and an `agents` array \
             where each entry has `name`, `role`, and `mandate`. All agents must already exist \
             (create them first with `create_agent` if needed).\n\
             To modify a team, use `update_team` — you can change the orchestrator, agents, or max_iterations.\n\
             To delete a team, use `delete_team`.\n",
        );
    }

    // Tool usage instructions (builtin tools are always available)
    prompt.push_str("\n## Tool Usage\n");
    prompt.push_str("- Update your core memory when you learn important things about the user.\n");
    prompt.push_str(
        "- When the user mentions a person by name for the first time, store them using \
store_fact(category=\"person\") with their name, relationship, and any context. \
Use only their first name (or first + last if ambiguous). Never prefix with \"User:\", \
\"Mr.\", \"Dr.\", etc. — the relationship field captures the role. \
Core memory tracks key people briefly — the people table is the full record.\n",
    );
    prompt.push_str(
        "- Before asking a clarifying question, check conversation history and search_memory — \
         never re-ask something the user already provided.\n",
    );
    prompt.push_str(
        "- When the user asks for multiple actions in one message, handle ALL of them \
         in the same turn using multiple tool calls. Do not process one and ask about the rest.\n",
    );
    prompt.push_str(
        "- Before creating or storing anything (reminders, facts, people, events), \
         first check existing state using the appropriate query tool (list_reminders, \
         search_memory). If a similar entry already exists, inform the user rather than \
         creating a duplicate. After compaction, conversation history may be summarized — \
         always verify current state through tools rather than relying on memory of past actions.\n",
    );
    prompt.push_str(
        "- When asked about your own configuration, setup files, or how specific parts of you work, \
         check your own files (list_agent_files, read_agent_file) and documentation (get_documentation) \
         before answering. Never guess about your own internals.\n",
    );
    prompt.push_str("- Mark commitments as completed or cancelled using the update_fact tool.\n");
    prompt.push_str(
        "- When creating reminders, ALWAYS pass the user's local time and their timezone:\n  \
         - One-shot: fire_at in the user's local time (e.g. '2026-04-02T09:00:00'), timezone from the User timezone shown above (e.g. 'Asia/Singapore')\n  \
         - Periodic: cron_expr in the user's local time (e.g. '0 0 9 * * 1' for Monday 9am local), timezone from the User timezone shown above\n  \
         - If no User timezone is shown above, use UTC (fire_at with Z suffix, no timezone parameter)\n  \
         - The tool handles UTC conversion automatically — do NOT convert times yourself\n",
    );
    prompt
        .push_str("- You can list and cancel reminders with list_reminders and cancel_reminder.\n");
    prompt.push_str(
        "- Reminders support two action types: use action_type='send_message' (default) for static \
         notifications the user should see, and action_type='resume_agent' when the reminder \
         requires you to take action (e.g. check CI status, query an API, run tools). \
         resume_agent wakes you up to act on the reminder; send_message just delivers text.\n",
    );
    prompt.push_str(
        "- You can create new skills using create_skill to extend your capabilities with custom prompt snippets.\n",
    );
    prompt.push_str(
        "- You have built-in skills (use list_skills to see which). Built-in skills cannot be overwritten.\n",
    );
    prompt.push_str("- You can enable or disable skills with toggle_skill.\n");
    prompt.push_str("- To change whether a skill runs on every turn, use update_skill with always_on. For built-in skills, this persists in the database and survives restarts. Always use this tool rather than inspecting skill files directly.\n");
    prompt.push_str(
        "- You can also update skill descriptions, keywords, and prompts with update_skill.\n",
    );
    prompt.push_str("- Skills may be [built-in], [marketplace] (installed from Git repos via CLI), or [custom] (created locally). You can delete marketplace and custom skills.\n");
    prompt.push_str("- You can permanently remove custom skills with delete_skill. Built-in skills cannot be deleted.\n");
    prompt.push_str(
        "- You can read and update customer config (timezone, chat_id, thinking_level) with get_config and set_config.\n",
    );
    prompt.push_str(
        "- Tools may return images (screenshots, image files); you will see and can describe their contents.\n",
    );
    prompt.push_str(
        "- When a tool produces an image file path (e.g., screenshot saved to /path/to/image.png), use read_agent_file on that path to view the image contents.\n",
    );
    prompt.push_str(
        "- You can delegate tasks to specialized agents with delegate_task when other agents are configured.\n",
    );
    prompt.push_str(
        "- Use create_task to track significant pieces of work (feature implementations, \
         research projects, items waiting on external input). Check list_tasks before creating \
         to avoid duplicates.\n\
         - Task status management follows two patterns:\n\
           - **Direct update:** When the user explicitly requests a status change (\"mark it done\", \
         \"cancel the task\"), call update_task_status directly. The tool validates transitions — \
         terminal states (completed, cancelled) are final — status cannot be changed, but metadata \
         can still be written by including the metadata field.\n\
           - **Inspect first:** When the user asks about a task's state (\"check the task\", \
         \"is the PR merged?\"), call check_task to read details and any linked GitHub PR/issue \
         status. Present findings and wait for the user's decision before changing status.\n\
         - Status transitions: pending can go to any status; in_progress can go to blocked/completed/cancelled; \
         blocked can go to in_progress/completed/cancelled; cancelled can return to in_progress \
         (cancel-and-retry, mika#856). Completed is terminal (status locked, metadata still writable).\n",
    );
    prompt.push_str(
        "- **Delegation Rule:** Before delegating any implementation work (via delegate_task \
         or long-running skills), you MUST first create a task using create_task, then pass \
         the task_id to the delegation tool. The tool will reject calls without a valid task_id.\n",
    );
    prompt.push_str(
        "- Use search_tool_history to recall results of past tool calls across sessions (by tool name, \
         keyword, time range). This avoids re-running expensive external calls when you've already \
         retrieved the same information recently.\n",
    );
    prompt.push_str(
        "- Some tools are long-running and return a task ID instead of immediate results. \
         When this happens, inform the user that a background task is running and you'll follow up \
         when results arrive. Do not retry the tool.\n",
    );
    if let Some(home) = ctx.home_dir {
        writeln!(
            prompt,
            "- You can write files to your home directory with write_agent_file. \
             Your home directory is {} — all paths are relative to this directory (~ is accepted as a prefix, e.g. '~/notes.md'). \
             For example, to write identity.toml at the root of your home, use path 'identity.toml'. \
             If the file exists, you must review the current content and call again with confirm: true to overwrite.",
            home.display()
        )
        .unwrap();
        writeln!(
            prompt,
            "- You can read files from your home directory with read_agent_file (path relative to {}). \
             Files larger than 100 KB are rejected. \
             core_memory sections cannot be read with this tool — they are already in your system prompt above.",
            home.display()
        )
        .unwrap();
    } else {
        prompt.push_str(
            "- You can write files to your home directory with write_agent_file. Paths are relative to your home. \
             If the file exists, you must review the current content and call again with confirm: true to overwrite.\n",
        );
        prompt.push_str(
            "- You can read files from your home directory with read_agent_file (relative paths only). \
             Files larger than 100 KB are rejected. \
             core_memory sections cannot be read with this tool — they are already in your system prompt above.\n",
        );
    }
    prompt.push_str(
        "- You can list files in your home directory with list_agent_files. \
         Omit path or pass an empty string to list the root. Pass a relative subdirectory path to list that directory.\n",
    );
    prompt.push_str(
        "- NEVER use run_shell (cat, echo, heredoc, tee, sed) to read or write files inside \
         your home directory or any agent's home directory. Always use read_agent_file, \
         write_agent_file, and list_agent_files instead — they provide audit logging, \
         overwrite confirmation, and path validation that shell commands bypass. \
         Only use run_shell for paths outside agent home directories \
         (e.g., ~/.local/bin/, ~/.config/).\n",
    );
    // Cross-agent file access guidance for orchestrators
    if let Some(home_dir) = ctx.global_home_dir {
        let agents = agent::list_agents(home_dir);
        if agents.len() > 1 {
            prompt.push_str(
                "- As an orchestrator, you can read, write, and list files in other agents' home \
                 directories by passing the `agent` parameter to file tools. For example: \
                 read_agent_file(path=\"config.toml\", agent=\"chase-hughes\") reads that agent's config.\n",
            );
        }
    }

    prompt
}

/// Build a compact system prompt for `ProviderKind::MikaModel`.
///
/// Emits ≤5 KB with at most 2 sections (Personality, Identity). The full
/// mika-spirit prompt overwhelms the provider's small context window and
/// causes OOD-prior-dominated completions (fictional status reports instead
/// of agent-mode responses). A Tool Usage section is deliberately deferred:
/// the MikaModel provider expects ≤5-10 tools max, so tool catalog injection
/// is gated upstream at the call site (see `is_compact_provider` check in
/// `agent.rs`), not inside this builder.
///
/// **mika#1813 carve-out (tracked in mika#1925):** the stop-signal contract
/// (`<stopped-topics>` block + persist/consult rules) is intentionally NOT
/// rendered here — the compact budget cannot afford it and the MikaModel
/// provider is not currently used by family-tier or operator-tier agents in
/// production. mika#1925 tracks wiring a size-capped variant when MikaModel
/// goes live for real tenants. The `stopped_topics` field on `PromptContext`
/// is accepted by this builder to keep the type signature uniform across the
/// three builders; it is deliberately unused here.
pub fn build_compact_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(512);

    // ## Personality — soul summary or identity persona line (~100 chars)
    if !ctx.soul_content.is_empty() {
        prompt.push_str("## Personality\n");
        // Take the first line of the soul content as a short persona summary.
        let first_line = ctx.soul_content.lines().next().unwrap_or(ctx.soul_content);
        prompt.push_str(first_line);
        prompt.push_str("\n\n");
    }

    // ## Identity — agent name + role (~50 chars)
    prompt.push_str("## Identity\n");
    write!(prompt, "You are {}.\n\n", ctx.identity.name).unwrap();

    prompt
}

/// Context for building a silent mode (heartbeat/reminder/reflection) system prompt.
pub struct SilentPromptContext<'a> {
    pub soul_content: &'a str,
    pub identity: &'a Identity,
    pub core_memory: &'a [CoreMemoryEntry],
    pub pending_commitments: &'a [Commitment],
    pub trigger_context: &'a str,
    pub current_utc: DateTime<Utc>,
    pub timezone: Option<String>,
    /// Whether Telegram integration is configured for outbound delivery.
    pub telegram_configured: bool,
    /// Whether a message sender is available for outbound delivery.
    /// When false, the prompt omits instructions to use `send_message`.
    pub has_message_sender: bool,
    /// Pre-formatted digest of today's conversations (reflection mode only).
    pub recent_conversations: Option<&'a str>,
    /// Pre-formatted digest of today's audit events (reflection mode only).
    pub recent_audit_events: Option<&'a str>,
    /// Agent home directory. When set, file tool instructions include the absolute path.
    pub home_dir: Option<&'a std::path::Path>,
    /// Task health summary: active tasks + anomalous task states.
    pub task_health: Option<&'a TaskHealthSummary>,
    /// Stored user preferences for autonomous action during heartbeat.
    pub stored_preferences: &'a [Preference],
    /// Active stop-signals (`stop_topic_*` preferences, mika#1813).
    /// See `STOP_TOPIC_PREFIX`. Rendered as `<stopped-topics>` block in silent
    /// prompts so the agent does not re-initiate on user-refused topics.
    pub stopped_topics: &'a [Preference],
}

/// Sanitize a label for prompt injection prevention: truncate to 200 chars, strip angle brackets
/// and newlines to prevent prompt structure manipulation.
fn sanitize_label(s: &str) -> String {
    s.chars()
        .take(200)
        .filter(|c| !matches!(c, '<' | '>' | '\n' | '\r'))
        .collect()
}

/// Sanitize and format a reference URL for prompt output.
fn sanitize_ref_url(url: Option<&str>) -> String {
    url.map(|u| {
        let sanitized: String = u
            .chars()
            .take(200)
            .filter(|c| !matches!(c, '<' | '>' | '\n' | '\r'))
            .collect();
        format!(", ref: {sanitized}")
    })
    .unwrap_or_default()
}

/// Build a system prompt for silent mode (heartbeat/reminder).
/// The agent's text output is NOT delivered — it must use send_message to contact the user.
pub fn build_silent_prompt(ctx: &SilentPromptContext<'_>) -> String {
    let mut prompt = String::with_capacity(4096);

    write_soul_section(&mut prompt, ctx.soul_content);
    write_identity_section(&mut prompt, ctx.identity);
    write_time_section(&mut prompt, ctx.current_utc, ctx.timezone.as_deref());
    write_channel_section(&mut prompt, None, ctx.telegram_configured);
    write_core_memory_section(
        &mut prompt,
        ctx.core_memory,
        Some(
            "Auto-loaded on every turn. Do NOT read via read_agent_file. \
             Use update_core_memory to modify.",
        ),
    );

    // Stop-signals (mika#1813) — surfaces user's explicit "don't re-raise this" list
    // BEFORE commitments so the model sees the suppression list first when scanning.
    if !ctx.stopped_topics.is_empty() {
        prompt.push_str("## Stopped Topics\n");
        prompt.push_str(
            "The user has explicitly asked you NOT to re-initiate on these topics. \
             Do NOT proactively send_message about any listed subject. Direct user \
             questions about a stopped topic are still fine to answer.\n",
        );
        prompt.push_str("<stopped-topics>\n");
        for pref in ctx.stopped_topics {
            let cat = sanitize_label(&pref.category);
            let val = sanitize_label(&pref.value);
            writeln!(prompt, "- {cat}: {val}").unwrap();
        }
        prompt.push_str("</stopped-topics>\n\n");
    }

    // Pending commitments
    if !ctx.pending_commitments.is_empty() {
        prompt.push_str("## Pending Commitments\n");
        prompt.push_str("<commitments>\n");
        for c in ctx.pending_commitments {
            let due = c.due_date.as_deref().unwrap_or("no due date");
            writeln!(prompt, "- {} (due: {})", c.description, due).unwrap();
        }
        prompt.push_str("</commitments>\n\n");
    }

    // Silent mode instructions
    prompt.push_str("## Silent Mode\n");
    if ctx.has_message_sender {
        prompt.push_str(
            "You are in SILENT MODE. Your text output is NOT delivered to the user.\n\
             Use the send_message tool to contact the user. If you have nothing worthwhile \
             to say, simply respond with a brief internal note and do NOT call send_message.\n\
             **Respect stop signals (mika#1813):** Before initiating any proactive \
             `send_message`, check the `<stopped-topics>` block above. If the subject you \
             would raise matches an entry, DO NOT re-raise. Silence is the correct action \
             when the user has said stop.\n\n",
        );
    } else {
        prompt.push_str(
            "You are in SILENT MODE. Your text output is NOT delivered to the user.\n\
             No outbound messaging channel is configured, so you cannot contact the user.\n\
             Perform any background maintenance (memory updates, fact storage) silently.\n\n",
        );
    }

    // Reflection context (today's conversations and audit events)
    if let Some(conversations) = ctx.recent_conversations.filter(|c| !c.is_empty()) {
        prompt.push_str("## Today's Conversations\n");
        prompt.push_str("<conversations>\n");
        prompt.push_str(conversations);
        prompt.push_str("\n</conversations>\n\n");
    }
    if let Some(events) = ctx.recent_audit_events.filter(|e| !e.is_empty()) {
        prompt.push_str("## Recent Audit Events\n");
        prompt.push_str("<audit-events>\n");
        prompt.push_str(events);
        prompt.push_str("\n</audit-events>\n\n");
    }

    // File tools — mention home-scoped file tools so heartbeat agents can discover them
    prompt.push_str("## File Tools\n");
    if let Some(home) = ctx.home_dir {
        writeln!(
            prompt,
            "- read_agent_file: Read a file from your home directory ({}). Paths are relative to that directory.\n\
             - list_agent_files: List files and directories in your home directory. Omit path to list the root.",
            home.display()
        )
        .unwrap();
    } else {
        prompt.push_str(
            "- read_agent_file: Read a file from your home directory. Paths are relative to your home.\n\
             - list_agent_files: List files and directories in your home directory. Omit path to list the root.\n",
        );
    }
    prompt.push('\n');

    // Task health: active tasks + anomalies (labels sanitized to prevent prompt injection)
    if let Some(health) = ctx.task_health {
        let has_items = !health.active_tasks.is_empty();
        let has_anomalies = !health.anomalies.is_empty();
        if has_items || has_anomalies {
            prompt.push_str("## Task Health\n");
            prompt.push_str("<task-health>\n");

            if has_items {
                prompt.push_str("<pending-tasks>\n");
                for item in &health.active_tasks {
                    let created_dt =
                        crate::timestamp::parse(&item.created_at).unwrap_or(ctx.current_utc);
                    let age_days = ctx.current_utc.signed_duration_since(created_dt).num_days();
                    let label = sanitize_label(&item.label);
                    let ref_url = sanitize_ref_url(item.reference_url.as_deref());
                    writeln!(
                        prompt,
                        "- [{status}] {id}: {label} (age: {age_days}d{ref_url})",
                        status = item.status,
                        id = item.id,
                    )
                    .unwrap();
                }
                prompt.push_str("</pending-tasks>\n");
            }

            if has_anomalies {
                prompt.push_str("<anomalies>\n");
                for a in &health.anomalies {
                    let label = sanitize_label(&a.label);
                    let ref_suffix = sanitize_ref_url(a.reference_url.as_deref());
                    writeln!(
                        prompt,
                        "- [{anomaly}] {id}: {label} ({age}{ref_suffix})",
                        anomaly = a.anomaly_type,
                        id = a.task_id,
                        age = a.age_description,
                    )
                    .unwrap();
                }
                prompt.push_str("</anomalies>\n");
            }

            prompt.push_str("\n<task-health-instructions>\n");
            prompt.push_str(
                "Review the task health summary above. For each anomaly:\n\
                 1. If a stored preference covers this action pattern, take it autonomously and include \"(per your standing preference)\" in any notification.\n\
                 2. Otherwise, notify the user via send_message with the anomaly details and suggest an action.\n\
                 3. For github_linked items, call check_task to inspect the linked PR/issue status before notifying.\n\
                 4. Include the task ID in all notifications so the user can reference it.\n\
                 5. Present findings as \"as of this check\" — task states may have changed since the query ran.\n\
                 6. After taking a corrective action that the user confirmed, ask: \"Should I always do this automatically in the future? I can remember this as a standing preference.\"\n\
                 7. If the user confirms, store the policy via store_fact with category \"preference\" and a key prefixed with \"task_policy_\".\n\
                 8. You can only see and act on your own tasks. Never attempt to query or modify tasks belonging to other agents.\n",
            );
            prompt.push_str("</task-health-instructions>\n");
            prompt.push_str("</task-health>\n\n");
        }
    }

    // Stored preferences for autonomous task actions
    if !ctx.stored_preferences.is_empty() {
        prompt.push_str("<stored-preferences>\n");
        for pref in ctx.stored_preferences {
            let cat = sanitize_label(&pref.category);
            let val = sanitize_label(&pref.value);
            writeln!(prompt, "- {cat}: {val}").unwrap();
        }
        prompt.push_str("</stored-preferences>\n\n");
    }

    // Trigger-specific context
    prompt.push_str("## Trigger\n");
    prompt.push_str(ctx.trigger_context);
    prompt.push('\n');

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> Identity {
        Identity {
            name: "Mika".to_string(),
            emoji: "✦".to_string(),
            reflection: None,
            heartbeat: None,
            kg: KgIdentityConfig::default(),
            skills: SkillsIdentityConfig::default(),
            tools: ToolsIdentityConfig::default(),
            context: ContextIdentityConfig::default(),
            session: SessionIdentityConfig::default(),
            curator: None,
        }
    }

    fn test_time() -> DateTime<Utc> {
        "2026-02-24T12:00:00Z".parse().unwrap()
    }

    // --- Canonical session resolution (mika#1401) ---

    #[test]
    fn test_resolve_canonical_session_non_singleton_returns_none() {
        // Default identity is non-singleton — callers mint a fresh UUID per ask.
        let identity = test_identity();
        assert!(!identity.session.singleton);
        assert_eq!(resolve_canonical_session_id(&identity, "mika-prime"), None);
    }

    #[test]
    fn test_resolve_canonical_session_singleton_derives_id() {
        // singleton = true without an explicit canonical_id derives
        // `canonical-{agent_id}` (prefix-typed sibling of system-{agent_id}).
        let mut identity = test_identity();
        identity.session.singleton = true;
        assert_eq!(
            resolve_canonical_session_id(&identity, "mika-prime"),
            Some("canonical-mika-prime".to_string())
        );
    }

    #[test]
    fn test_resolve_canonical_session_singleton_honors_explicit_id() {
        // An explicit canonical_id (e.g. mika-prime's zero-UUID) wins over derivation.
        let mut identity = test_identity();
        identity.session.singleton = true;
        identity.session.canonical_id = Some("00000000-0000-0000-0000-000000000000".to_string());
        assert_eq!(
            resolve_canonical_session_id(&identity, "mika-prime"),
            Some("00000000-0000-0000-0000-000000000000".to_string())
        );
    }

    #[test]
    fn test_resolve_canonical_session_canonical_id_ignored_when_not_singleton() {
        // canonical_id present but singleton = false → still None (opt-in gate).
        let mut identity = test_identity();
        identity.session.singleton = false;
        identity.session.canonical_id = Some("00000000-0000-0000-0000-000000000000".to_string());
        assert_eq!(resolve_canonical_session_id(&identity, "mika-prime"), None);
    }

    #[test]
    fn test_session_identity_config_parses_from_toml() {
        // The [session] block deserializes with both fields set.
        let toml = r#"
name = "Mika Prime"
emoji = "✦"

[session]
singleton = true
canonical_id = "00000000-0000-0000-0000-000000000000"
"#;
        let identity: Identity = toml::from_str(toml).unwrap();
        assert!(identity.session.singleton);
        assert_eq!(
            identity.session.canonical_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000000")
        );
    }

    #[test]
    fn test_session_identity_config_defaults_when_absent() {
        // Absent [session] block → singleton = false, canonical_id = None.
        let toml = r#"
name = "Mika"
emoji = "✦"
"#;
        let identity: Identity = toml::from_str(toml).unwrap();
        assert!(!identity.session.singleton);
        assert!(identity.session.canonical_id.is_none());
    }

    fn test_core_memory() -> Vec<CoreMemoryEntry> {
        vec![
            CoreMemoryEntry {
                key: "user_summary".to_string(),
                value: "Loves coffee.".to_string(),
                token_count: 3,
                updated_at: "2026-01-01".to_string(),
            },
            CoreMemoryEntry {
                key: "self_model".to_string(),
                value: "I am Mika. No interaction history yet.".to_string(),
                token_count: 8,
                updated_at: "2026-01-01".to_string(),
            },
        ]
    }

    #[test]
    fn test_prompt_includes_soul_content() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "You are a sharp, proactive executive assistant.",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.starts_with("You are a sharp, proactive executive assistant."));
    }

    #[test]
    fn test_prompt_includes_all_core_memory_blocks() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("### user_summary"));
        assert!(prompt.contains("Loves coffee."));
        assert!(prompt.contains("### self_model"));
        assert!(prompt.contains("I am Mika. No interaction history yet."));
    }

    #[test]
    fn test_prompt_includes_identity_name() {
        let identity = Identity {
            name: "TestBot".to_string(),
            emoji: "🤖".to_string(),
            reflection: None,
            heartbeat: None,
            kg: KgIdentityConfig::default(),
            skills: SkillsIdentityConfig::default(),
            tools: ToolsIdentityConfig::default(),
            context: ContextIdentityConfig::default(),
            session: SessionIdentityConfig::default(),
            curator: None,
        };
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("You are TestBot."));
    }

    #[test]
    fn test_onboarding_prompt_injected() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: true,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## First Session"));
        assert!(prompt.contains("Introduce yourself briefly"));
    }

    #[test]
    fn test_no_onboarding_for_returning_user() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## First Session"));
    }

    #[test]
    fn test_load_identity_parses_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Agent X\"\nemoji = \"🕶\"\n",
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Agent X");
        assert_eq!(identity.emoji, "🕶");
    }

    #[test]
    fn test_load_identity_defaults_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Mika");
        assert_eq!(identity.emoji, "✦");
    }

    #[test]
    fn test_prompt_empty_soul_no_extra_whitespace() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        // Should start directly with Identity section when soul is empty
        assert!(prompt.starts_with("## Identity"));
    }

    #[test]
    fn test_silent_prompt_heartbeat() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            pending_commitments: &[],
            trigger_context: "This is a HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Silent Mode"));
        assert!(prompt.contains("send_message"));
        assert!(prompt.contains("HEARTBEAT check-in"));
        assert!(prompt.contains("## Core Memory"));
    }

    #[test]
    fn test_silent_prompt_reminder() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "REMINDER: Call the dentist",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Call the dentist"));
        assert!(prompt.contains("NOT delivered"));
    }

    #[test]
    fn test_silent_prompt_includes_commitments() {
        use crate::db::Commitment;
        let identity = test_identity();
        let commitments = vec![Commitment {
            id: 1,
            description: "Review budget".to_string(),
            status: "pending".to_string(),
            due_date: Some("2026-03-01".to_string()),
            person_id: None,
            created_at: "2026-02-24".to_string(),
            completed_at: None,
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &commitments,
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Review budget"));
        assert!(prompt.contains("2026-03-01"));
    }

    #[test]
    fn test_prompt_includes_current_time() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Current Time"));
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        // Timezone value should not be in the time section when not configured
        assert!(!prompt.contains("User timezone: "));
    }

    #[test]
    fn test_prompt_includes_timezone_when_set() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: Some("+08:00".to_string()),
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(prompt.contains("User timezone: +08:00"));
    }

    #[test]
    fn test_prompt_includes_tool_usage_section() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        // Builtin tool instructions are always in the base prompt
        assert!(prompt.contains("## Tool Usage"));
        assert!(prompt.contains("search_memory"));
        assert!(prompt.contains("creating reminders"));
        assert!(prompt.contains("update_fact"));
        // Base instruction also present
        assert!(prompt.contains("Never fabricate information"));
        // Skill awareness line
        assert!(prompt.contains("built-in skills"));
        assert!(prompt.contains("list_skills"));
        assert!(prompt.contains("toggle_skill"));
        assert!(prompt.contains("update_skill"));
        assert!(prompt.contains("delete_skill"));
    }

    #[test]
    fn test_prompt_wraps_core_memory_in_xml_tags() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("<core-memory>"));
        assert!(prompt.contains("</core-memory>"));
    }

    #[test]
    fn test_silent_prompt_wraps_commitments_in_xml_tags() {
        use crate::db::Commitment;
        let identity = test_identity();
        let commitments = vec![Commitment {
            id: 1,
            description: "Review budget".to_string(),
            status: "pending".to_string(),
            due_date: Some("2026-03-01".to_string()),
            person_id: None,
            created_at: "2026-02-24".to_string(),
            completed_at: None,
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &commitments,
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("<commitments>"));
        assert!(prompt.contains("</commitments>"));
        assert!(prompt.contains("Review budget"));
    }

    #[test]
    fn test_silent_prompt_includes_current_time() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: Some("-05:00".to_string()),
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Current Time"));
        assert!(prompt.contains("UTC: 2026-02-24T12:00:00Z"));
        assert!(prompt.contains("User timezone: -05:00"));
    }

    #[test]
    fn test_prompt_includes_teams_when_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let team_dir = tmp.path().join("teams").join("dev-team");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(
            team_dir.join("team.toml"),
            r#"
[team]
name = "dev-team"
orchestrator = "planner"

[[agents]]
name = "planner"
role = "orchestrator"
mandate = "Plan tasks"

[[agents]]
name = "coder"
role = "specialist"
mandate = "Write code"

[flow]
max_iterations = 3
"#,
        )
        .unwrap();

        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Agents & Teams"));
        assert!(prompt.contains("run_team"));
        assert!(prompt.contains("dev-team (2 agents)"));
    }

    #[test]
    fn test_prompt_includes_agents_when_multiple() {
        let tmp = tempfile::tempdir().unwrap();

        // Create two agents
        let mika_dir = tmp.path().join("agents").join("mika");
        std::fs::create_dir_all(&mika_dir).unwrap();
        std::fs::write(mika_dir.join("config.toml"), "# config").unwrap();
        std::fs::write(
            mika_dir.join("identity.toml"),
            "name = \"Mika\"\nemoji = \"✦\"\n",
        )
        .unwrap();

        let researcher_dir = tmp.path().join("agents").join("researcher");
        std::fs::create_dir_all(&researcher_dir).unwrap();
        std::fs::write(researcher_dir.join("config.toml"), "# config").unwrap();
        std::fs::write(
            researcher_dir.join("identity.toml"),
            "name = \"Rex\"\nemoji = \"🔬\"\n",
        )
        .unwrap();

        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Agents & Teams"));
        assert!(prompt.contains("delegate_task"));
        assert!(prompt.contains("mika (✦ Mika)"));
        assert!(prompt.contains("researcher (🔬 Rex)"));
    }

    #[test]
    fn test_prompt_omits_agents_teams_when_single_agent_no_teams() {
        let tmp = tempfile::tempdir().unwrap();

        // Single agent only
        let mika_dir = tmp.path().join("agents").join("mika");
        std::fs::create_dir_all(&mika_dir).unwrap();
        std::fs::write(mika_dir.join("config.toml"), "# config").unwrap();

        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Agents & Teams"));
    }

    #[test]
    fn test_prompt_omits_agents_teams_when_none_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: Some(tmp.path()),
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Agents & Teams"));
    }

    #[test]
    fn test_prompt_omits_agents_teams_when_home_dir_none() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Agents & Teams"));
    }

    #[test]
    fn test_prompt_includes_channel_section_for_telegram() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: Some("telegram"),
            telegram_configured: true,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Communication Channel"));
        assert!(prompt.contains("This conversation is happening via: telegram"));
        assert!(prompt.contains("Telegram integration is active"));
    }

    #[test]
    fn test_prompt_includes_channel_section_for_cli() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: Some("cli"),
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Communication Channel"));
        assert!(prompt.contains("This conversation is happening via: cli"));
        assert!(!prompt.contains("Telegram integration is active"));
    }

    #[test]
    fn test_prompt_omits_channel_section_when_none() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Communication Channel"));
    }

    #[test]
    fn test_prompt_includes_home_dir_in_write_agent_file_instruction() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: Some(std::path::Path::new("/home/user/.mika/agents/mika")),
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("/home/user/.mika/agents/mika"),
            "prompt should include the home directory path"
        );
        assert!(
            prompt.contains("Your home directory is /home/user/.mika/agents/mika"),
            "prompt should include the home directory in write_agent_file instruction"
        );
    }

    #[test]
    fn test_prompt_fallback_when_home_dir_none() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("Paths are relative to your home"),
            "should fall back to generic instruction when home_dir is None"
        );
    }

    #[test]
    fn test_silent_prompt_includes_telegram_when_configured() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: true,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("Telegram integration is active"));
    }

    #[test]
    fn test_silent_prompt_omits_channel_when_no_telegram() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(!prompt.contains("## Communication Channel"));
    }

    #[test]
    fn test_silent_prompt_omits_send_message_when_no_sender() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: false,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Silent Mode"));
        assert!(prompt.contains("NOT delivered"));
        assert!(!prompt.contains("Use the send_message tool"));
        assert!(prompt.contains("No outbound messaging channel is configured"));
    }

    #[test]
    fn test_reflection_config_parse_time_valid() {
        use chrono::Timelike;
        let config = ReflectionConfig {
            enabled: true,
            time: "20:00".to_string(),
            notify: false,
            timezone: None,
        };
        let time = config.parse_time().unwrap();
        assert_eq!(time.hour(), 20);
        assert_eq!(time.minute(), 0);
    }

    #[test]
    fn test_reflection_config_parse_time_invalid() {
        let config = ReflectionConfig {
            enabled: true,
            time: "25:00".to_string(),
            notify: false,
            timezone: None,
        };
        assert!(config.parse_time().is_none());

        let config2 = ReflectionConfig {
            enabled: true,
            time: "8pm".to_string(),
            notify: false,
            timezone: None,
        };
        assert!(config2.parse_time().is_none());
    }

    #[test]
    fn test_reflection_config_defaults() {
        let config = ReflectionConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.time, "20:00");
        assert!(!config.notify);
    }

    #[test]
    fn test_load_identity_with_reflection_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            r#"
name = "Mika"
emoji = "✦"

[reflection]
enabled = true
time = "21:30"
notify = true
"#,
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Mika");
        let reflection = identity.reflection.unwrap();
        assert!(reflection.enabled);
        assert_eq!(reflection.time, "21:30");
        assert!(reflection.notify);
    }

    #[test]
    fn test_load_identity_without_reflection_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Mika\"\nemoji = \"✦\"\n",
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert!(identity.reflection.is_none());
    }

    #[test]
    fn heartbeat_config_defaults_to_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            "name = \"Mika\"\nemoji = \"✦\"\n",
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        assert!(identity.heartbeat.is_none());
        // unwrap_or(true) yields true — backward-compatible default
        assert!(
            identity
                .heartbeat
                .as_ref()
                .map(|c| c.enabled)
                .unwrap_or(true)
        );
    }

    #[test]
    fn heartbeat_config_explicit_false() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            r#"
name = "Mika"
emoji = "✦"

[heartbeat]
enabled = false
"#,
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        let hb = identity.heartbeat.unwrap();
        assert!(!hb.enabled);
    }

    #[test]
    fn heartbeat_config_explicit_true() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("identity.toml"),
            r#"
name = "Mika"
emoji = "✦"

[heartbeat]
enabled = true
"#,
        )
        .unwrap();

        let identity = load_identity(tmp.path());
        let hb = identity.heartbeat.unwrap();
        assert!(hb.enabled);
    }

    #[test]
    fn test_silent_prompt_includes_reflection_context() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Reflection mode.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: false,
            recent_conversations: Some("User discussed Series A fundraise with Alice."),
            recent_audit_events: Some("update_core_memory: current_priorities -> fundraise"),
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("## Today's Conversations"));
        assert!(prompt.contains("Series A fundraise"));
        assert!(prompt.contains("## Recent Audit Events"));
        assert!(prompt.contains("current_priorities"));
    }

    #[test]
    fn test_silent_prompt_omits_empty_reflection_context() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Reflection mode.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: false,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(!prompt.contains("## Today's Conversations"));
        assert!(!prompt.contains("## Recent Audit Events"));
    }

    #[test]
    fn test_silent_prompt_includes_task_health_with_anomalies() {
        use crate::db::{Task, TaskHealthAnomaly, TaskHealthSummary};
        let identity = test_identity();
        let health = TaskHealthSummary {
            active_tasks: vec![Task {
                id: "abc-123".to_string(),
                agent_id: "mika".to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 0,
                label: "Fix auth flow".to_string(),
                trigger_type: "manual".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "none".to_string(),
                action_config: "{}".to_string(),
                status: "in_progress".to_string(),
                process_id: None,
                input_context: None,
                result: None,
                created_by_session: None,
                created_trace_id: None,
                execution_trace_id: None,
                created_at: "2026-02-24T10:00:00Z".to_string(),
                updated_at: "2026-02-24T10:00:00Z".to_string(),
                fired_at: None,
                completed_at: None,
                reference_url: Some("https://github.com/org/repo/issues/42".to_string()),
                source: None,
                metadata: None,
                r#type: "issue".to_string(),
                dispatch_class: None,
            }],
            anomalies: vec![TaskHealthAnomaly {
                task_id: "def-456".to_string(),
                label: "Build deployment".to_string(),
                trigger_type: "callback".to_string(),
                status: "completed".to_string(),
                anomaly_type: "stuck_callback".to_string(),
                age_description: "stuck 3h 22m".to_string(),
                reference_url: None,
            }],
        };
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: Some(&health),
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("<task-health>"));
        assert!(prompt.contains("</task-health>"));
        assert!(prompt.contains("<pending-tasks>"));
        assert!(prompt.contains("Fix auth flow"));
        assert!(prompt.contains("<anomalies>"));
        assert!(prompt.contains("stuck_callback"));
        assert!(prompt.contains("Build deployment"));
        assert!(prompt.contains("<task-health-instructions>"));
        assert!(prompt.contains("stored preference"));
    }

    #[test]
    fn test_silent_prompt_omits_task_health_when_none() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(!prompt.contains("<task-health>"));
    }

    #[test]
    fn test_silent_prompt_includes_stored_preferences() {
        use crate::db::Preference;
        let identity = test_identity();
        let prefs = vec![Preference {
            category: "task_policy_merged_pr".to_string(),
            value: "Auto-complete when PR is merged".to_string(),
            updated_at: "2026-02-24T10:00:00Z".to_string(),
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "Heartbeat",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &prefs,
            stopped_topics: &[],
        };

        let prompt = build_silent_prompt(&ctx);
        assert!(prompt.contains("<stored-preferences>"));
        assert!(prompt.contains("task_policy_merged_pr"));
        assert!(prompt.contains("Auto-complete when PR is merged"));
    }

    #[test]
    fn test_onboarding_prompt_mentions_store_fact() {
        let prompt = onboarding_prompt();
        assert!(prompt.contains("store_fact"));
        assert!(prompt.contains("person"));
        assert!(prompt.contains("The user"));
    }

    #[test]
    fn test_tool_usage_prompt_mentions_store_fact_person() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: chrono::Utc::now(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("store_fact(category=\"person\")"));
    }

    #[test]
    fn test_callback_context_injects_prompt_guard() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: chrono::Utc::now(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: Some("Processing callback results from a long-running task."),
            stopped_topics: &[],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Callback Result Turn"));
        assert!(prompt.contains("MUST NOT submit new long-running tasks"));
    }

    #[test]
    fn test_no_callback_context_omits_prompt_guard() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: chrono::Utc::now(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Callback Result Turn"));
    }

    #[test]
    fn test_prompt_includes_multi_action_batching() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("handle ALL of them"));
        assert!(prompt.contains("multiple tool calls"));
    }

    #[test]
    fn test_prompt_includes_conversation_continuity() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("check conversation history and search_memory"));
        assert!(prompt.contains("never re-ask something the user already provided"));
    }

    #[test]
    fn test_prompt_includes_proactive_state_checking() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("first check existing state"));
        assert!(prompt.contains("rather than creating a duplicate"));
    }

    #[test]
    fn test_prompt_includes_confirmation_before_action_guardrail() {
        let identity = test_identity();
        let ctx = PromptContext {
            identity: &identity,
            soul_content: "",
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Confirmation before action"));
        assert!(prompt.contains("Do not interpret questions as implicit requests"));
        assert!(prompt.contains("retry failed operations"));
        assert!(prompt.contains("relaunch tasks"));
    }

    #[test]
    fn test_prompt_includes_grounding_guardrail() {
        let identity = test_identity();
        let ctx = PromptContext {
            identity: &identity,
            soul_content: "",
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Grounding rule"));
        assert!(prompt.contains("NEVER claim the state of a downstream or adjacent system"));
        assert!(prompt.contains("do not extrapolate"));
        // Verify bad/good examples are present
        assert!(prompt.contains("BAD:"));
        assert!(prompt.contains("GOOD:"));
        // Verify URL fabrication examples are present (#308)
        assert!(prompt.contains("NEVER fabricate URLs"));
        assert!(prompt.contains("No tool calls"));
        assert!(prompt.contains("run_gh posts a comment"));
    }

    #[test]
    fn test_prompt_includes_context_priority() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Context priority:"));
        assert!(prompt.contains("current user message > core memory > active skill context"));
        assert!(prompt.contains("conversation summary"));
        assert!(prompt.contains("conversation history > search results"));
    }

    // ── KgIdentityConfig deserialization tests (#778) ─────────────────────

    #[test]
    fn kg_identity_config_empty_toml_defaults() {
        let identity: Identity = toml::from_str("").unwrap();
        assert!(identity.kg.enabled);
        assert!(identity.kg.docs_root.is_none());
    }

    #[test]
    fn kg_identity_config_existing_agent_without_kg() {
        let identity: Identity = toml::from_str(
            r#"
name = "Mika"
emoji = "✦"
"#,
        )
        .unwrap();
        assert_eq!(identity.name, "Mika");
        assert_eq!(identity.emoji, "✦");
        assert!(identity.kg.enabled);
        assert!(identity.kg.docs_root.is_none());
    }

    #[test]
    fn kg_identity_config_both_fields() {
        let identity: Identity = toml::from_str(
            r#"
name = "X"

[kg]
enabled = true
docs_root = "/data/docs"
"#,
        )
        .unwrap();
        assert!(identity.kg.enabled);
        assert_eq!(
            identity.kg.docs_root.as_deref(),
            Some(Path::new("/data/docs"))
        );
    }

    #[test]
    fn kg_identity_config_only_enabled_false() {
        let identity: Identity = toml::from_str(
            r#"
[kg]
enabled = false
"#,
        )
        .unwrap();
        assert!(!identity.kg.enabled);
        assert!(identity.kg.docs_root.is_none());
    }

    #[test]
    fn kg_identity_config_only_docs_root() {
        let identity: Identity = toml::from_str(
            r#"
[kg]
docs_root = "/path"
"#,
        )
        .unwrap();
        assert!(identity.kg.enabled);
        assert_eq!(identity.kg.docs_root.as_deref(), Some(Path::new("/path")));
    }

    #[test]
    fn kg_identity_config_malformed_falls_back_via_load() {
        // Malformed [kg] causes Identity::default() via load_identity's
        // unwrap_or_default — kg defaults to enabled=true, docs_root=None.
        let result: Result<Identity, _> = toml::from_str(
            r#"
[kg]
docs_root = 42
"#,
        );
        assert!(result.is_err(), "malformed [kg] should fail toml parse");
        // load_identity wraps with unwrap_or_default:
        let identity = result.unwrap_or_default();
        assert!(identity.kg.enabled);
        assert!(identity.kg.docs_root.is_none());
    }

    #[test]
    fn test_identity_with_skills_allowlist() {
        let identity: Identity = toml::from_str(
            r#"
name = "Architect"
emoji = "🏛"

[skills]
allowlist = ["mika-arch-groom-ticket", "mika-arch-second-review"]
"#,
        )
        .unwrap();
        assert_eq!(identity.name, "Architect");
        let allowlist = identity.skills.allowlist.unwrap();
        assert_eq!(allowlist.len(), 2);
        assert_eq!(allowlist[0], "mika-arch-groom-ticket");
        assert_eq!(allowlist[1], "mika-arch-second-review");
    }

    #[test]
    fn test_identity_without_skills_section() {
        let identity: Identity = toml::from_str(
            r#"
name = "Dev"
emoji = "🛠"
"#,
        )
        .unwrap();
        assert!(identity.skills.allowlist.is_none());
    }

    #[test]
    fn test_identity_with_empty_skills_allowlist() {
        let identity: Identity = toml::from_str(
            r#"
name = "Dev"

[skills]
allowlist = []
"#,
        )
        .unwrap();
        assert_eq!(identity.skills.allowlist.unwrap().len(), 0);
    }

    #[test]
    fn test_context_summary_inject_explicit_false() {
        let identity: Identity = toml::from_str(
            r#"
name = "Architect"

[context.summary]
inject = false
"#,
        )
        .unwrap();
        assert!(!identity.context.summary.inject);
    }

    #[test]
    fn test_context_summary_inject_default_no_context_section() {
        let identity: Identity = toml::from_str(
            r#"
name = "Dev"
"#,
        )
        .unwrap();
        assert!(identity.context.summary.inject);
    }

    #[test]
    fn test_context_summary_inject_default_context_no_summary() {
        // [context] present but no [context.summary] — inject should default true
        let identity: Identity = toml::from_str(
            r#"
name = "Dev"

[context]
"#,
        )
        .unwrap();
        assert!(identity.context.summary.inject);
    }

    #[test]
    fn test_context_summary_inject_explicit_true() {
        let identity: Identity = toml::from_str(
            r#"
name = "Dev"

[context.summary]
inject = true
"#,
        )
        .unwrap();
        assert!(identity.context.summary.inject);
    }

    // -- truncate_to_token_budget tests (Axis 3 — mika#1021) --

    #[test]
    fn truncate_empty_string() {
        let result = truncate_to_token_budget("", 100);
        assert_eq!(result, "");
    }

    #[test]
    fn truncate_short_string_under_budget() {
        let summary = "This is a short summary.";
        let result = truncate_to_token_budget(summary, 100);
        assert_eq!(result, summary);
    }

    #[test]
    fn truncate_exactly_at_budget() {
        // 6 tokens * 4 chars = 24 chars budget
        let summary = "This is exactly 24 char!"; // 24 chars
        assert_eq!(summary.len(), 24);
        let result = truncate_to_token_budget(summary, 6);
        assert_eq!(result, summary);
    }

    #[test]
    fn truncate_over_budget_cuts_at_word_boundary() {
        // 2 tokens * 4 chars = 8 chars budget
        let summary = "The quick brown fox jumps over the lazy dog";
        let result = truncate_to_token_budget(summary, 2);
        // Budget is 8 chars. Last whitespace at or before position 8 is position 3
        // (space after "The"). So the pre-marker content should be exactly "The".
        assert_eq!(
            result,
            "The\n[… summary truncated to fit silent-mode budget …]"
        );
    }

    #[test]
    fn truncate_over_budget_no_whitespace() {
        // 2 tokens * 4 chars = 8 chars budget; no whitespace → cuts at max_chars
        let summary = "abcdefghijklmnop";
        let result = truncate_to_token_budget(summary, 2);
        assert!(result.starts_with("abcdefgh"));
        assert!(result.contains("[… summary truncated to fit silent-mode budget …]"));
    }

    #[test]
    fn truncate_long_summary_preserves_marker() {
        let summary = "word ".repeat(500); // 2500 chars
        let result = truncate_to_token_budget(&summary, 100); // 400 char budget
        assert!(result.ends_with("[… summary truncated to fit silent-mode budget …]"));
        // Truncated content (before marker) should be ≤ budget chars
        let content_before_marker = result
            .strip_suffix("\n[… summary truncated to fit silent-mode budget …]")
            .unwrap();
        assert!(content_before_marker.len() <= 400);
    }

    #[test]
    fn truncate_multibyte_unicode_safe() {
        // Ensure we don't panic on multi-byte UTF-8 characters near the cut point
        let summary = "日本語のテスト文字列です。これは長いテスト文字列です。";
        // Each CJK char is 3 bytes; with budget of 5 tokens (20 chars / ~6-7 CJK chars)
        let result = truncate_to_token_budget(summary, 5);
        assert!(result.contains("[… summary truncated to fit silent-mode budget …]"));
        // Should not panic — that's the main assertion
    }

    // -- ContextSummaryConfig deserialization tests (Axis 3 — mika#1021) --

    #[test]
    fn summary_config_default_max_tokens_is_none() {
        let config = ContextSummaryConfig::default();
        assert!(config.inject);
        assert!(config.max_tokens.is_none());
    }

    #[test]
    fn summary_config_deserializes_max_tokens() {
        let identity: Identity = toml::from_str(
            r#"
name = "Test"

[context.summary]
inject = true
max_tokens = 500
"#,
        )
        .unwrap();
        assert!(identity.context.summary.inject);
        assert_eq!(identity.context.summary.max_tokens, Some(500));
    }

    #[test]
    fn summary_config_deserializes_max_tokens_zero_sentinel() {
        let identity: Identity = toml::from_str(
            r#"
name = "Test"

[context.summary]
inject = true
max_tokens = 0
"#,
        )
        .unwrap();
        assert_eq!(identity.context.summary.max_tokens, Some(0));
    }

    #[test]
    fn summary_config_without_max_tokens_defaults_to_none() {
        let identity: Identity = toml::from_str(
            r#"
name = "Test"

[context.summary]
inject = false
"#,
        )
        .unwrap();
        assert!(!identity.context.summary.inject);
        assert!(identity.context.summary.max_tokens.is_none());
    }

    #[test]
    fn test_build_compact_system_prompt_size_bound() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "You are a sharp, proactive executive assistant.",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: Some("America/New_York".to_string()),
            global_home_dir: None,
            channel_type: Some("telegram"),
            telegram_configured: true,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_compact_system_prompt(&ctx);

        // AC1: ≤5 KB
        assert!(
            prompt.len() <= 5120,
            "compact prompt is {} bytes, exceeds 5 KB limit",
            prompt.len()
        );

        // AC1: ≤3 sections
        let section_count = prompt.matches("## ").count();
        assert!(
            section_count <= 3,
            "compact prompt has {} sections, exceeds 3-section limit",
            section_count
        );

        // Must include personality and identity
        assert!(prompt.contains("## Personality"));
        assert!(prompt.contains("## Identity"));
        assert!(prompt.contains("You are Mika."));

        // Must NOT include heavy sections from the full prompt
        assert!(!prompt.contains("## Instructions"));
        assert!(!prompt.contains("## Current Time"));
        assert!(!prompt.contains("<core-memory>"));
        assert!(!prompt.contains("## Communication Channel"));
    }

    #[test]
    fn test_build_compact_system_prompt_empty_soul() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };

        let prompt = build_compact_system_prompt(&ctx);

        // No Personality section when soul is empty
        assert!(!prompt.contains("## Personality"));
        // Identity section still present
        assert!(prompt.contains("## Identity"));
        // Only 1 section
        assert_eq!(prompt.matches("## ").count(), 1);
    }

    // ==================================================================
    // Stop-topic tests (mika#1813)
    //
    // These tests exercise the state + prompt gate that keeps Mika from
    // re-nagging after a user says "arrête". The fix has two invariants:
    //   1. STATE: a `stop_topic_*` preference is loaded and injected as
    //      `<stopped-topics>` block in both silent and conversation prompts.
    //   2. PROMPT: the conversation prompt tells the agent to persist the
    //      stop-signal via `store_fact`; the silent prompt tells the agent
    //      not to re-initiate on any listed subject.
    //
    // The load-bearing regression fixture is
    // `test_silent_prompt_regression_stop_topic_visible_and_rule_present`.
    // It fails on `main` (the field does not exist on `SilentPromptContext`
    // and the rule text is not emitted), demonstrating that the fix is
    // load-bearing per feedback_verify_pipeline_passes_without_the_fix.
    // ==================================================================

    fn stop_topic_pref(slug: &str, note: &str) -> Preference {
        Preference {
            category: format!("stop_topic_{slug}"),
            value: note.to_string(),
            updated_at: "2026-08-19T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_silent_prompt_stopped_topics_block_present_when_nonempty() {
        let identity = test_identity();
        let stops = vec![stop_topic_pref(
            "web-config",
            "user asked to stop, 2026-08-19",
        )];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "This is a scheduled HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &stops,
        };
        let prompt = build_silent_prompt(&ctx);
        assert!(
            prompt.contains("## Stopped Topics"),
            "silent prompt must render the Stopped Topics section when non-empty"
        );
        assert!(prompt.contains("<stopped-topics>"));
        assert!(
            prompt.contains("stop_topic_web-config"),
            "block must include the sanitized preference category"
        );
        assert!(
            prompt.contains("user asked to stop, 2026-08-19"),
            "block must include the preference value (context of the stop)"
        );
        assert!(
            prompt.contains("Respect stop signals"),
            "silent-mode instructions must carry the respect-stop-signals rule"
        );
    }

    #[test]
    fn test_silent_prompt_stopped_topics_block_absent_when_empty() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "This is a scheduled HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &[],
        };
        let prompt = build_silent_prompt(&ctx);
        // Check for block-unique prose (silent-mode block description) — the
        // rule text in `## Silent Mode` also mentions `<stopped-topics>` and
        // `## Stopped Topics`, so structural substring checks would false-fire.
        // The block's descriptive sentence appears nowhere else.
        assert!(
            !prompt.contains("The user has explicitly asked you NOT to re-initiate"),
            "empty stopped_topics must not emit the block description"
        );
        // The rendered open-tag with trailing newline appears only inside the
        // block itself (the rule text embeds it inline with backticks).
        assert!(!prompt.contains("<stopped-topics>\n"));
    }

    #[test]
    fn test_silent_prompt_stopped_topics_sanitized() {
        // Prompt injection safety: angle brackets and newlines in preference
        // values must be stripped by sanitize_label before appearing in the
        // prompt. Mirrors the existing sanitize_label discipline used for
        // task-health labels and stored-preferences (see prompt.rs).
        let identity = test_identity();
        let stops = vec![Preference {
            category: "stop_topic_<script>".to_string(),
            value: "line1\nline2 <hack>".to_string(),
            updated_at: "2026-08-19T00:00:00Z".to_string(),
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "This is a scheduled HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &stops,
        };
        let prompt = build_silent_prompt(&ctx);
        // The literal `<script>` fragment inside the category value must be
        // gone: sanitize_label strips `<` and `>` and newlines.
        assert!(
            !prompt.contains("<script>"),
            "sanitize_label must strip angle brackets from category values"
        );
        // Newlines injected via value must be stripped so they don't break
        // the enclosing `<stopped-topics>` block structure.
        // (The block itself contains newlines by construction; what matters
        // is that the value cannot inject a fake close-tag line.)
        assert!(!prompt.contains("line1\nline2"));
    }

    #[test]
    fn test_conversation_prompt_stopped_topics_block_present_when_nonempty() {
        let identity = test_identity();
        let stops = vec![stop_topic_pref(
            "budget-review",
            "user said arrête, 2026-08-19",
        )];
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &stops,
        };
        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("## Stopped Topics"),
            "conversation prompt must render the Stopped Topics section when non-empty"
        );
        assert!(prompt.contains("stop_topic_budget-review"));
        assert!(prompt.contains("user said arrête, 2026-08-19"));
    }

    #[test]
    fn test_conversation_prompt_stopped_topics_block_absent_when_empty() {
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };
        let prompt = build_system_prompt(&ctx);
        // The "Respect stop signals (consult)" rule mentions `## Stopped
        // Topics` inline, so structural substring checks would false-fire.
        // Check for block-unique descriptive prose instead.
        assert!(
            !prompt.contains("Do NOT proactively raise any listed subject"),
            "empty stopped_topics must not emit the block description"
        );
        assert!(!prompt.contains("<stopped-topics>\n"));
    }

    #[test]
    fn test_conversation_prompt_persist_stop_rule_present() {
        // The "Respect stop signals (persist)" rule is unconditional — it
        // appears in every conversation prompt so the agent knows to call
        // store_fact the first time the user says stop, even before any
        // stop_topic_* preference exists.
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("Respect stop signals (persist)"),
            "persist-on-stop rule must appear in every conversation prompt"
        );
        assert!(
            prompt.contains("stop_topic_"),
            "rule must name the stop_topic_ preference-key prefix"
        );
        assert!(
            prompt.contains("store_fact"),
            "rule must instruct the agent to persist via store_fact"
        );
        // The rule must be careful about the AC2 distinction — a direct
        // question is NOT a re-opening.
        assert!(
            prompt.contains("direct user question about the topic is not a re-opening"),
            "rule must preserve the stop-vs-question distinction (AC2)"
        );
    }

    #[test]
    fn test_conversation_prompt_consult_stop_rule_present() {
        // The "Respect stop signals (consult)" rule tells the agent to
        // check the block that U3 renders and not re-raise anything on it.
        let identity = test_identity();
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Respect stop signals (consult)"));
    }

    /// Load-bearing regression fixture (mika#1813).
    ///
    /// This test asserts BOTH the state signal (the specific stopped subject
    /// is present in the assembled silent prompt) AND the prompt signal (the
    /// "Respect stop signals" rule text is present). It fails on `main`
    /// because `SilentPromptContext` does not carry a `stopped_topics` field
    /// and `build_silent_prompt` never renders the block or the rule.
    ///
    /// Per feedback_verify_pipeline_passes_without_the_fix, at least one
    /// added test must fail without the code change and pass with it — this
    /// is that test.
    #[test]
    fn test_silent_prompt_regression_stop_topic_visible_and_rule_present() {
        let identity = test_identity();
        // Al's original refusal from the ticket: web-config re-nagging.
        let stops = vec![stop_topic_pref("web-config", "Al asked to stop 2026-07-20")];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "This is a scheduled HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &stops,
        };
        let prompt = build_silent_prompt(&ctx);
        // AC1 (state): the specific stopped topic Al refused is present.
        assert!(
            prompt.contains("stop_topic_web-config"),
            "stopped topic must appear in the silent prompt so heartbeat sees it"
        );
        // AC1 (prompt): the respect rule is present, so the agent knows the
        // block is a suppression directive.
        assert!(
            prompt.contains("Respect stop signals"),
            "silent-mode rule text must be present so the agent knows what to do with the block"
        );
        // AC3 (re-test): the block must survive a second heartbeat — this
        // is guaranteed by construction (the DB row is durable) but we assert
        // the render is deterministic.
        let prompt2 = build_silent_prompt(&ctx);
        assert_eq!(prompt, prompt2, "prompt render must be deterministic");
    }

    /// AC4: no leak between axes.
    ///
    /// A stop_topic on X must not affect the visibility of unrelated commitments
    /// Y. This test constructs a stop on `web-config` and an unrelated commitment
    /// on `budget-review`; both blocks must be present with the commitment un-
    /// touched, and the commitment text must not be muted, hidden, or filtered
    /// by the stop-signal.
    #[test]
    fn test_silent_prompt_stop_signal_does_not_leak_across_axes() {
        use crate::db::Commitment;
        let identity = test_identity();
        let stops = vec![stop_topic_pref(
            "web-config",
            "user asked to stop, 2026-08-19",
        )];
        let commitments = vec![Commitment {
            id: 42,
            description: "Prepare budget review for Q3".to_string(),
            status: "pending".to_string(),
            due_date: Some("2026-08-31".to_string()),
            person_id: None,
            created_at: "2026-08-19T00:00:00Z".to_string(),
            completed_at: None,
        }];
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &commitments,
            trigger_context: "This is a scheduled HEARTBEAT check-in.",
            current_utc: test_time(),
            timezone: None,
            telegram_configured: false,
            has_message_sender: true,
            recent_conversations: None,
            recent_audit_events: None,
            home_dir: None,
            task_health: None,
            stored_preferences: &[],
            stopped_topics: &stops,
        };
        let prompt = build_silent_prompt(&ctx);
        // Both blocks present.
        assert!(prompt.contains("## Stopped Topics"));
        assert!(prompt.contains("## Pending Commitments"));
        // The unrelated commitment survives verbatim — the stop-signal does
        // not filter, shadow, or suppress it.
        assert!(
            prompt.contains("Prepare budget review for Q3"),
            "unrelated commitment must remain visible (AC4 — no leak across axes)"
        );
    }

    #[test]
    fn test_stop_topic_prefix_constant_is_stable() {
        // The prefix is a public API contract — the store_fact write path
        // (LLM prompt) and the load path (agent_loop::load_agent_context)
        // must agree on the same string. Guarding the literal here catches
        // an accidental rename that would silently break the coupling.
        assert_eq!(STOP_TOPIC_PREFIX, "stop_topic_");
    }

    #[test]
    fn test_filter_stop_topic_preferences_keeps_strict_prefix() {
        // Baseline: a `stop_topic_<slug>` category must survive the filter.
        let prefs = vec![Preference {
            category: "stop_topic_web-config".to_string(),
            value: "user asked to stop, 2026-08-19".to_string(),
            updated_at: "2026-08-19T00:00:00Z".to_string(),
        }];
        let filtered = filter_stop_topic_preferences(prefs);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].category, "stop_topic_web-config");
    }

    #[test]
    fn test_filter_stop_topic_preferences_drops_value_only_matches() {
        // Anti-regression: `db::search_preferences('stop_topic_')` ORs on
        // category AND value with a substring LIKE, so a row whose CATEGORY is
        // not a stop-topic but whose VALUE happens to mention the substring
        // would otherwise surface in `<stopped-topics>` and mute unrelated
        // axes (AC4 leak). The filter must drop it.
        let prefs = vec![Preference {
            category: "task_policy_finance".to_string(),
            value: "when user says stop_topic_x we do Y".to_string(),
            updated_at: "2026-08-19T00:00:00Z".to_string(),
        }];
        let filtered = filter_stop_topic_preferences(prefs);
        assert!(
            filtered.is_empty(),
            "value-only matches must not surface as stop-topics"
        );
    }

    #[test]
    fn test_filter_stop_topic_preferences_drops_wildcard_underscore_neighbours() {
        // Anti-regression: SQLite LIKE treats `_` as a single-char wildcard,
        // so `%stop_topic_%` matches `stop_topicalization` (the trailing `_`
        // matches `a`). A non-prefix category must not be promoted to a
        // stop-topic. `starts_with` in the filter is a byte-level literal
        // check with no LIKE semantics — it rejects the imposter cleanly.
        let prefs = vec![
            Preference {
                category: "stop_topicalization".to_string(),
                value: "unrelated".to_string(),
                updated_at: "2026-08-19T00:00:00Z".to_string(),
            },
            Preference {
                category: "stop_topic_web-config".to_string(),
                value: "user asked to stop, 2026-08-19".to_string(),
                updated_at: "2026-08-19T00:00:00Z".to_string(),
            },
        ];
        let filtered = filter_stop_topic_preferences(prefs);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].category, "stop_topic_web-config");
    }

    #[test]
    fn test_compact_prompt_omits_stopped_topics_block_by_design() {
        // The compact prompt (ProviderKind::MikaModel) is intentionally
        // silent on stop-topics — the ≤5 KB budget cannot afford them and
        // MikaModel is not currently used for production agents. This test
        // pins the carve-out documented at build_compact_system_prompt so a
        // future wire-up is deliberate (test flips) rather than accidental.
        let identity = test_identity();
        let stops = vec![stop_topic_pref(
            "web-config",
            "user asked to stop, 2026-08-19",
        )];
        let ctx = PromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &stops,
        };
        let prompt = build_compact_system_prompt(&ctx);
        assert!(
            !prompt.contains("<stopped-topics>"),
            "compact prompt must not emit the stop-topics block (carve-out)"
        );
        assert!(
            !prompt.contains("Stopped Topics"),
            "compact prompt must not emit the stop-topics section header"
        );
        assert!(
            !prompt.contains("Respect stop signals"),
            "compact prompt must not carry stop-signal rules"
        );
    }
}
