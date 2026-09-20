use crate::db::{
    Commitment, CoreMemoryEntry, Preference, TaskHealthSummary, core_memory_section_names,
};
use mika_common::home::{Deployment, PersonaProfile};

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

// ---------------------------------------------------------------------------
// mika#1814 — Distribution Doctrine (invitation-only, hermetic distribution)
// ---------------------------------------------------------------------------

/// Heading for the code-managed Distribution Doctrine section (mika#1814).
///
/// Rendered by [`write_distribution_doctrine_section`] into every
/// `build_system_prompt` and `build_silent_prompt` output between the soul
/// content and the identity heading. The heading string is part of the
/// prompt-shape contract asserted by unit tests and by the
/// `doctrine_regressions::doctrine_public_promo_prompt_contains_section`
/// eval scenario (mika#1814 AC1).
///
/// The compact-provider variant [`build_compact_system_prompt`] intentionally
/// omits this section — the ≤5 KB MikaModel budget cannot afford the extra
/// ~700 bytes and MikaModel is not currently used for production
/// family-tier / operator-tier agents. Follow-up: mika#1925 sibling for when
/// MikaModel goes live for real tenants.
pub const DISTRIBUTION_DOCTRINE_HEADING: &str = "## Distribution Doctrine";

/// Canonical body of the Distribution Doctrine section (mika#1814).
///
/// Names the prohibited public-launch surfaces (Show HN, Product Hunt, Reddit
/// launch, Twitter promo thread, growth-hack tactics) and prescribes the
/// bilingual redirect script so a family-tier (French) or operator-tier
/// (English) Mika lands the correct language without an LLM code-switch.
///
/// The prompt content is the *intent* half of the fix — the structural half
/// is the `doctrine_public_promo` EndTurn guard in `agent_loop::mod`, which
/// catches assistant text that proposes/drafts one of these surfaces even
/// after the doctrine section is in the prompt (per
/// `feedback_prompt_enforcement_fragile`).
///
/// The bearing citation (`project_mika_invitation_only_no_public_launch`) is
/// operator-authored institutional memory — the file lives at
/// `~/.claude/projects/-data-workspace-mika-platform/memory/…` and is a
/// merge-gate precondition (AC11 in the plan), not a code-PR deliverable.
/// mika-qa verifies the citation *string* appears in this constant
/// (headless-safe); the operator verifies the memory *file* exists before
/// applying the `ready` label.
pub const DISTRIBUTION_DOCTRINE_BODY: &str = "Mika grows by personal invitation — from someone who knows the person she'll serve. She is not launched publicly. **You do not propose, draft, or plan public promotion**: no Show HN, no Product Hunt, no Reddit launch thread, no Twitter promo thread, no growth-hack tactics. If a user (however well-intentioned) suggests public promotion, redirect gently and briefly, without moralising:\n\n> \"Mika grandit par invitation entre proches. Le meilleur soutien est de parler d'elle en 1-à-1 à quelqu'un qu'elle servirait bien.\"\n\n(English equivalent for English-speaking users: \"Mika grows through personal invitation between people who know each other. The best support is to speak of her one-to-one, to someone she'd serve well.\")\n\nA user answering a direct question about how Mika grows may still receive the invitation-chain explanation. This rule blocks *proposing* and *drafting* public-launch artifacts, not *answering* questions about distribution.\n\nBearing: `project_mika_invitation_only_no_public_launch` — see agent's institutional memory.";

/// Write the Distribution Doctrine section (mika#1814).
///
/// Called from [`build_system_prompt`] and [`build_silent_prompt`] between
/// the soul content and the identity heading so the doctrine binds before
/// any per-turn context (time, channel, core memory) is rendered. The
/// section is code-managed (constants above), NOT user-editable — a user
/// editing their `soul.md` cannot accidentally weaken it. Applies
/// uniformly to `DEFAULT_SOUL` (operator tier) and `FAMILY_SOUL` (family
/// tier) — same limit for every Mika instance.
fn write_distribution_doctrine_section(prompt: &mut String) {
    prompt.push_str(DISTRIBUTION_DOCTRINE_HEADING);
    prompt.push('\n');
    prompt.push_str(DISTRIBUTION_DOCTRINE_BODY);
    prompt.push_str("\n\n");
}

// ---------------------------------------------------------------------------
// mika#2292 — Mika Doctrine (the material register, aliased "doctrine")
// ---------------------------------------------------------------------------

/// Heading for the code-managed Mika Doctrine section (mika#2292).
///
/// Rendered by [`write_mika_doctrine_section`] into every `build_system_prompt`
/// and `build_silent_prompt` output, immediately after
/// [`DISTRIBUTION_DOCTRINE_HEADING`] and before the identity heading.
///
/// English, like its two sisters: the heading is a prompt-structure marker, not
/// text served to a user.
///
/// **What it closes.** Measured 2026-09-11 on a champion (cloud) tenant. Asked
/// « Qu'est-ce que la doctrine Mika ? », the agent answered "nothing found
/// called the Mika doctrine" **and then gave the philosophy anyway**, in the
/// same response. Not a knowledge gap — a *name* gap: it held the answer and had
/// no label for it. That shape (uncertainty at t=0, assertion at t=1) is the one
/// rule 4 of `## Self-Identity Discipline` already condemned word for word; it
/// did not bite because the written scope of that section was "which model you
/// are, which provider powers you, WHERE you run". Third occurrence of the same
/// class after mika#1815 and mika#2290, and the remedy takes the same shape both
/// times took: a code-managed fact section plus a rule widening the discipline's
/// scope — not a skill, not a soul edit, not a memory.
///
/// The compact-provider variant [`build_compact_system_prompt`] intentionally
/// omits this section — see the comment at the point of omission and mika#1925.
pub const MIKA_DOCTRINE_HEADING: &str = "## Mika Doctrine";

/// Operator-register body of the Mika Doctrine section (mika#2292).
///
/// **Provenance of every fact asserted here.** *MIT*: verified in `LICENSE`
/// (line 1) and `Cargo.toml` (`license = "MIT"`) — and the claim is deliberately
/// scoped to **the engine that runs the agent**, never to "Mika" as a whole: the
/// cloud console is a separate, closed codebase, so an unqualified "Mika is open
/// source" would be false. *Data belonging to the person*: written as a
/// **commitment**, never as a statement of where bytes sit. *Proactivity* and
/// *persistent memory*: behaviours already prescribed by the soul; what this
/// section adds is the *why*. *Growth by invitation*: **cited**, never
/// re-narrated — `## Distribution Doctrine` is its ground truth, and a second
/// telling is a duplication that drifts (the class `grooming_marker`/mika#2158
/// and the retry gate/mika#2362 each had to close once).
///
/// **What is deliberately NOT claimed.** The word "exportable" does not appear.
/// It is asserted today at five sites (mika#2290's remedy, one of them served to
/// the model) and this repository contains no export tool, route, or
/// subcommand — the tenant's data lives in `mika-cloud`, which is not in this
/// workspace, so the claim can be neither verified nor safely retracted from
/// here. Constat + follow-up ticket; this section simply does not add a sixth
/// site. See the `mika2292_no_exportable_claim_*` tests.
///
/// **No sentence of locality, even a true one.** The hosting fact is `## Runtime`'s
/// and this body *refers* to it rather than restating it — which makes the 5d
/// guard predicate (mika#2290) unmatched **by construction** (no locality
/// subject to pair with an assertion) rather than by a conditional marker a
/// later editor might reword away.
///
/// **The spiritual stop is topical, and names no referent — the central
/// inversion of this ticket.** The naive implementation writes "do not discuss
/// <X>, <Y>, <Z>", which *teaches the tenant the very words it claims to
/// protect*: a prompt that enumerates a secret in order to forbid it is a leak
/// with one extra step, and the family of tenants this serves has never heard
/// those words. Prime's bearing says "without exposing it" — and that includes
/// not exposing it to the agent itself. The stop is therefore expressed by topic
/// and provenance. The list of referents exists in exactly one place in this
/// tree, under `#[cfg(test)]`, where the scan that enforces this reads it.
pub const MIKA_DOCTRINE_BODY_OPERATOR: &str = "\"doctrine\", \"the Mika doctrine\", \"your stances\", \"your philosophy\", \"what you stand for\", \"what do you believe in\" — every one of these names THIS section. It is the ground truth for those questions. You hold the answer, so give it: answering that you found nothing of that name is wrong, not modest.\n\nMika's public stances, each with the reason it was taken:\n\n- **The engine that runs you is open source, under the MIT licence.** So that nobody has to depend on a single company for the assistant that knows their life. State this of the engine — never of every part of Mika.\n- **What a person confides in you belongs to that person.** This is a commitment about whose it is, not a statement about where it is kept.\n- **You are proactive.** An assistant that waits to be asked leaves the mental load on the person instead of taking a share of it.\n- **You remember what matters.** Having to reintroduce oneself at every conversation is the opposite of being assisted. The memory is a deliberate choice, not a side effect.\n- **Mika grows by invitation.** `## Distribution Doctrine` above is the ground truth for that stance; read it there rather than restating it here.\n\nWhere you run, and where a person's data is physically kept, is NOT answered from this section: the hosting line of `## Runtime` is the ground truth for those, and you answer them only from it.\n\nSome questions about Mika this section does not answer: a spiritual, esoteric or initiatory dimension, its origin, or anything else not written above. You do not know them and you will not invent them. That they are not laid out is a choice of Mika's creator. Say that plainly, without embarrassment, and leave it there.";

/// Family-register body of the Mika Doctrine section (mika#2292).
///
/// **Same substance, no infrastructure vocabulary.** `FAMILY_SOUL` forbids "tout
/// jargon technique … ou de l'infrastructure sous-jacente — jamais, même si on
/// te le demande", and "open source / MIT / repository / self-hosting" is exactly
/// that family. This is the crossing mika#2290 already settled, and its decision
/// is carried over rather than relitigated: the same fact is written twice, the
/// operator register carrying the full formulation and this one the same
/// substance without a single technical term. No rule is derived from the account
/// locale or from the tenant (Prime's 2026-09-09 ruling, carried over from
/// mika#2023). What the family register drops is not an arbitrary amputation: it
/// is the part of the doctrine that has **no meaning** for someone who owns no
/// infrastructure.
///
/// **The stop establishes NO creator referent here, and that is a decision with
/// a named cost.** Prime's bearing prescribes "it is the creator's choice";
/// mika#1783 removed from `FAMILY_SOUL`, *on doctrinal grounds*, any origin story
/// giving the being a referent it could then address — founding incident "Salut
/// Vincent", closure `the-being-does-not-have-a-maker-it-knows-about`, guarded by
/// `home::tests::family_soul_no_operator_name`. A narrow reading is available
/// (the constraint forbids a *nameable, addressable* referent, not the abstract
/// fact of having been made), but mika#1783 chose "no origin story" *against*
/// alternatives, and reopening that is not a p2's to do. So: the operator
/// register carries the bearing's formulation, this one carries a
/// **referent-free** stop, which satisfies *both* doctrines. This constant is the
/// single site to change if Vincent or Prime wants "creator" in this register
/// too. Non-blocking either way: the material register — the object of the
/// ticket — ships in both.
pub const MIKA_DOCTRINE_BODY_FAMILY: &str = "\"doctrine\", \"la doctrine Mika\", \"tes partis pris\", \"ta philosophie\", \"en quoi tu crois\", \"what do you stand for\" — every one of these names THIS section. You know the answer to those questions: give it, warmly and simply. Saying you found nothing of that name is wrong.\n\nWhat you stand for, and why, in everyday words:\n\n- What the person tells you belongs to the person. It is theirs and it stays theirs.\n- You remember what counts for them, and it is on purpose: nobody should have to introduce themselves again at every conversation.\n- You notice and you offer, instead of waiting to be asked. Taking a share of the load is the whole point.\n- People come to you through someone who already knows you — never through advertising.\n\nWhere you run, and where what the person tells you is kept, is not answered here: the hosting line of `## Runtime` above says it, and it is the only thing you answer that from.\n\nThere are things about yourself that you do not know, and that you will not make up. If you are asked about them, say so simply and kindly, and move on.";

/// Pick the doctrine body for a persona register (mika#2292).
///
/// Exhaustive `match`, **no `_ =>` arm** — the model is
/// [`hosting_ground_truth_line`], itself modelled on
/// `tools/mod.rs::dispatch_substrate_diagnostic`. The compiler, not a reviewer,
/// is what forces a future `PersonaProfile` to take a decision rather than
/// inherit one nobody took for it.
fn doctrine_body(persona: PersonaProfile) -> &'static str {
    match persona {
        PersonaProfile::Operator => MIKA_DOCTRINE_BODY_OPERATOR,
        PersonaProfile::Family => MIKA_DOCTRINE_BODY_FAMILY,
    }
}

/// Write the Mika Doctrine section (mika#2292).
///
/// Code-managed, like its two sisters, and for the reasons measured in
/// mika#2023 / mika#1814: it reaches **every existing tenant** at the next
/// deploy with no provisioning gesture (`write_default_if_missing` never
/// rewrites an existing `soul.md`, so a soul-side fix would have missed the one
/// tenant actually measured); it cannot be evicted by an unreadable
/// `identity.toml` (mika#2027's fail-closed sentinel evicts every skill); it
/// cannot be weakened by an operator editing `soul.md`; and it is not
/// conditioned on a keyword, which a skill trigger would be — and the measured
/// defect *is* a lexical miss, so a keyword-triggered remedy would reproduce it.
fn write_mika_doctrine_section(prompt: &mut String, persona: PersonaProfile) {
    prompt.push_str(MIKA_DOCTRINE_HEADING);
    prompt.push('\n');
    prompt.push_str(doctrine_body(persona));
    prompt.push_str("\n\n");
}

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
use tracing::warn;

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
    /// Whether turn-end skill-authoring nudges are injected for this agent (mika#1583).
    /// `None` or `Some(false)` = disabled (default). `Some(true)` = enabled.
    #[serde(default)]
    pub nudge_enabled: Option<bool>,
    /// Nudge cadence in tool-invoking turns (mika#1583). `None` = default 10.
    /// `Some(0)` is rejected at identity load (see `validate_skills_config`).
    /// `Some(N > 0)` = nudge every N tool-invoking turns.
    #[serde(default)]
    pub nudge_interval: Option<u32>,
}

impl SkillsIdentityConfig {
    /// Whether skill authoring (`skill_manage`) is permitted for this agent (mika#1582).
    pub fn authoring_enabled(&self) -> bool {
        self.allow_authoring.unwrap_or(false)
    }

    /// Whether turn-end skill-authoring nudges are enabled (mika#1583, default off).
    pub fn nudge_is_enabled(&self) -> bool {
        self.nudge_enabled.unwrap_or(false)
    }

    /// Resolved nudge cadence in tool-invoking turns (mika#1583, default 10).
    /// Always `> 0` when reached at runtime — `Some(0)` is rejected at load.
    pub fn resolved_nudge_interval(&self) -> u32 {
        self.nudge_interval.unwrap_or(10)
    }
}

/// Post-parse validation of the `[skills]` block (mika#1583, architect F1).
///
/// `toml::from_str` accepts `0` as a valid `u32`, so a `nudge_interval = 0` slips
/// past deserialization. Reject it here so `nudge_enabled` is the sole on/off
/// gate and `resolved_nudge_interval()` is always `> 0` at runtime (no zero-guard
/// needed in the turn-end check).
fn validate_skills_config(skills: &SkillsIdentityConfig) -> Result<(), String> {
    if skills.nudge_interval == Some(0) {
        return Err(
            "skills.nudge_interval must be > 0 (use nudge_enabled = false to disable)".into(),
        );
    }
    Ok(())
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
    #[serde(default)]
    pub history: ContextHistoryConfig,
}

/// `[context.history]` subsection — bounds the conversation window a turn
/// assembles (mika#2295).
///
/// The window it governs was bounded by `LIMIT 20` and nothing else, and the
/// counted items are, for an architect, whole plans and reviews: **twenty items
/// of unbounded size is not a bounded window**. Its filter was also `m.agent_id`
/// rather than `m.session_id`, so the window crossed sessions and therefore
/// tickets — an architect reviewing ticket A read the plans of B, C and D. The
/// measured consequence was mika-arch's median input prompt doubling from 35 829
/// to 71 007 tokens on 2026-09-03, the day 59 engine-grooms went through, with
/// none of its prompt files touched.
///
/// The two fields close two genuinely distinct axes and neither makes the other
/// redundant: `scope` bounds contamination *between* tickets, `max_tokens`
/// bounds a single session that iterates (`_iterate_groom_loop` runs plan v1,
/// review, plan v2, … in one session).
///
/// Shape is copied from [`ContextSummaryConfig`] deliberately — same `[context.*]`
/// home, same `CHARS_PER_TOKEN_ESTIMATE` conversion, same "applied by the caller,
/// DB layer signature untouched" discipline as [`load_gated_summary`].
#[derive(Debug, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct ContextHistoryConfig {
    /// Which rows the window may draw from. Default [`HistoryScope::Agent`]
    /// reproduces the pre-mika#2295 behaviour exactly, so no existing agent
    /// changes without that change being written into its identity.
    #[serde(default, deserialize_with = "deserialize_history_scope")]
    pub scope: HistoryScope,

    /// Optional token ceiling on the history, converted to bytes via
    /// [`CHARS_PER_TOKEN_ESTIMATE`] — the same `4` as
    /// [`truncate_to_token_budget`], not a second estimator.
    ///
    ///   - `None` → no ceiling (default; current behaviour).
    ///   - `Some(0)` → omission sentinel: the history is dropped entirely. The
    ///     turn's own user message is still kept — see
    ///     `truncate_history_to_token_budget` for why "empty window" cannot mean
    ///     "erase the question".
    ///   - `Some(n)` → oldest messages are elided until the history fits under
    ///     `n * CHARS_PER_TOKEN_ESTIMATE` bytes.
    ///
    /// A malformed value does NOT fail the parse: see
    /// [`deserialize_history_max_tokens`].
    #[serde(default, deserialize_with = "deserialize_history_max_tokens")]
    pub max_tokens: Option<usize>,
}

/// Row set the conversation window may draw from (mika#2295).
#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryScope {
    /// Every session of this agent — the pre-mika#2295 behaviour, and the default.
    #[default]
    Agent,
    /// The turn's own session only. This is the correction of substance for an
    /// architect: each pass is a one-shot act on one plan, so a first pass starts
    /// from an empty history and a second sees only the first — which is exactly
    /// what it should see. August's regime is recovered by construction, not by
    /// tuning a number.
    Session,
}

/// Deserialize `[context.history].scope`, degrading an unknown value to the
/// default rather than failing the parse.
///
/// **Why tolerance here is the safe direction, not laxity.** A parse failure on
/// `identity.toml` is not inert: for a well-known agent, `load_identity` answers
/// with the fail-closed sentinel allowlist, which neuters the agent *and*
/// unlinks its bundled skills from disk. Downing mika-arch over `scope =
/// "sesion"` would be a far larger effect than the typo, so an unreadable value
/// reads as "no scope was configured" — the conservative reading, since `Agent`
/// is the historical behaviour — and says so at WARN.
fn deserialize_history_scope<'de, D>(deserializer: D) -> Result<HistoryScope, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<toml::Value>::deserialize(deserializer)?;
    match raw {
        None => Ok(HistoryScope::default()),
        Some(toml::Value::String(s)) if s.eq_ignore_ascii_case("agent") => Ok(HistoryScope::Agent),
        Some(toml::Value::String(s)) if s.eq_ignore_ascii_case("session") => {
            Ok(HistoryScope::Session)
        }
        Some(other) => {
            warn!(
                event = "context_history_scope_invalid",
                value = %other,
                "[context.history].scope is not \"agent\" or \"session\" — falling back to the default scope"
            );
            Ok(HistoryScope::default())
        }
    }
}

/// Deserialize `[context.history].max_tokens`, degrading a malformed value to
/// "no ceiling" rather than failing the parse.
///
/// Same fail-closed-identity reasoning as [`deserialize_history_scope`], plus one
/// of its own: the alternative reading — treating an unusable ceiling as `0` —
/// would silently erase the whole history. **A mistyped ceiling that empties the
/// window is a context wipe wearing a configuration's clothes**, and is strictly
/// worse than the unbounded window this field exists to bound. Negative values
/// (a plausible way to spell "no limit") and non-integers both land here.
fn deserialize_history_max_tokens<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<toml::Value>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(toml::Value::Integer(n)) if n >= 0 => Ok(Some(n as usize)),
        Some(other) => {
            warn!(
                event = "context_history_max_tokens_invalid",
                value = %other,
                "[context.history].max_tokens is not a non-negative integer — falling back to no ceiling"
            );
            Ok(None)
        }
    }
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
/// Convert a token budget to a byte budget using [`CHARS_PER_TOKEN_ESTIMATE`].
///
/// Exposed so the conversation-window ceiling (mika#2295) converts through the
/// **same** estimator as [`truncate_to_token_budget`] rather than growing a
/// second one beside it. Two estimators for one heuristic is how a soft policy
/// becomes two different soft policies nobody notices disagreeing.
pub fn token_budget_to_bytes(max_tokens: usize) -> usize {
    max_tokens.saturating_mul(CHARS_PER_TOKEN_ESTIMATE)
}

pub fn truncate_to_token_budget(summary: &str, max_tokens: usize) -> String {
    let max_chars = token_budget_to_bytes(max_tokens);
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

/// Sentinel allowlist entry that matches no real skill. Its presence in
/// `skills.allowlist` evicts every bundled skill (`apply_identity_allowlist`
/// retains only allowlisted names, and nothing is named this).
///
/// `well_known_agents::well_known_skill_allowlists` filters any `__…__` token
/// out of the coherence gate for exactly this reason.
pub const FAIL_CLOSED_SKILL_SENTINEL: &str = "__fail_closed_no_skills__";

/// Load identity from `<home_dir>/identity.toml`.
///
/// # Absence is not permission (mika#2027)
///
/// A missing `identity.toml` used to return `Identity::default()`, whose
/// `skills.allowlist` is `None` — and `apply_identity_allowlist` treats `None`
/// as a no-op, i.e. **every bundled skill active**, `shell-exec` / `git-ops` /
/// `github` / `tmux` included. Deleting an agent's identity file therefore
/// *widened* it. Worse, the deletion is not recoverable by restarting:
/// `bootstrap_fresh_install` is gated on `home::is_initialized`, which is true
/// for any tenant whose `data/mika.db` exists, so nothing ever rewrites the
/// file. Measured on a live tenant 2026-08-28; ~2 min of exposure opened by an
/// authorized tier remediation that looked correct.
///
/// A read failure now yields the same fail-closed sentinel as a malformed file:
/// the agent starts with **zero skills** and mika-arch's platform-mutational
/// tool denylist. This is universal — well-known and user-defined agents alike
/// (F1). An agent that needs skills must carry an explicit `identity.toml`, even
/// a minimal one.
///
/// **What "neutered" does and does not mean.** The denylist is
/// `MIKA_ARCH_DISABLED_TOOLS`, which by design keeps `send_message` and the
/// agent-scoped memory writes (`update_core_memory`, `store_fact`,
/// `update_fact`) — mika#811 treats those as constitutive of being an agent
/// rather than platform side effects. A fail-closed agent can therefore still
/// write its own memory, and core memory is re-injected into every later system
/// prompt. Whether the fail-closed path wants a *stricter* denylist than
/// mika-arch's steady-state one is a real question, and a separate one: the two
/// share a constant today, so narrowing it here would also narrow mika-arch.
/// Left open deliberately rather than answered in a load-path fix.
///
/// **The sentinel reaches disk, not just memory.** `startup::seed_bundled_skills_if_needed`
/// feeds this allowlist to `materialize_agent_skill_links`, whose second pass
/// removes the symlink of every de-allowlisted bundled skill. A fail-closed start
/// therefore empties `<agent_home>/skills/` of its library symlinks — recoverable
/// (they are re-materialized on the first start with a valid identity) and scoped
/// (marketplace and `--copy-managed` directories are untouched), but a real disk
/// effect, including for the transient `identity_toml_unreadable` case.
///
/// Fail-closed *sentinel* rather than refusing to boot (F2): mika-spirit serves
/// every agent from one process, so a hard refusal would take the healthy agents
/// down with the broken one, and it would contradict mika#1962's `tier_guard`,
/// which explicitly tolerates a missing persona file. Neutering one agent is the
/// per-agent shape of the same posture.
///
/// The two causes are logged under **distinct** event names — `identity_toml_absent`
/// (`ErrorKind::NotFound`) and `identity_toml_unreadable` (permissions, I/O) —
/// because they call for different remediations (AC2).
///
/// On parse error the behaviour is unchanged from mika#811:
/// - well-known agents fail closed (same sentinel, `event = "identity_toml_malformed"`);
/// - user-defined agents fall back to `Identity::default()`.
///
/// That leaves one deliberate asymmetry: a *user-defined* agent is failed closed
/// on an absent file but not on a malformed one. mika#2027 scopes itself to the
/// absent case; widening the malformed path is a separate decision with a wider
/// blast radius, not a fix smuggled into this one.
///
/// The well-known check uses the home_dir's last component matched against
/// `find_well_known_agent`. This keeps the discrimination at the load layer
/// where it belongs, without requiring every caller to know the agent's
/// well-known status.
pub fn load_identity(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => parse_identity_or_fail_closed(&content, home_dir, &path),
        Err(e) => unreadable_identity_fail_closed(home_dir, &path, &e),
    }
}

/// Async version of [`load_identity`] using `tokio::fs` to avoid
/// blocking the async runtime.
pub async fn load_identity_async(home_dir: &Path) -> Identity {
    let path = home_dir.join("identity.toml");
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => parse_identity_or_fail_closed(&content, home_dir, &path),
        Err(e) => unreadable_identity_fail_closed(home_dir, &path, &e),
    }
}

/// The agent's name as the load layer knows it: the last component of its home
/// directory. Shared by every fail-closed log site so one agent is named one way.
fn agent_name_from_home(home_dir: &Path) -> &str {
    home_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>")
}

/// `identity.toml` could not be read. Fail closed, and say **which** unreadable
/// it was — an absent file and an unreadable-but-present one have different
/// remediations (mika#2027 AC2).
fn unreadable_identity_fail_closed(home_dir: &Path, path: &Path, err: &std::io::Error) -> Identity {
    let agent_name = agent_name_from_home(home_dir);

    if err.kind() == std::io::ErrorKind::NotFound {
        tracing::error!(
            event = "identity_toml_absent",
            agent = %agent_name,
            path = %path.display(),
            "identity.toml is ABSENT — failing CLOSED (all skills evicted, all \
             mutational tools denied). Absence never means 'everything permitted' \
             (mika#2027). Restarting will NOT regenerate the file: bootstrap only \
             runs on an uninitialized home. Re-provision it from the agent's tier \
             template — see docs/operator/agent-identity-reprovision.md"
        );
    } else {
        tracing::error!(
            event = "identity_toml_unreadable",
            agent = %agent_name,
            path = %path.display(),
            error = %err,
            "identity.toml is present but could not be read — failing CLOSED (all \
             skills evicted, all mutational tools denied). This is a permissions or \
             I/O fault, not a missing file: fix the file's readability rather than \
             re-provisioning it. See docs/operator/agent-identity-reprovision.md"
        );
    }

    fail_closed_identity()
}

fn parse_identity_or_fail_closed(content: &str, home_dir: &Path, path: &Path) -> Identity {
    // Parse, then run post-parse field validation (mika#1583 F1). A validation
    // failure routes into the same malformed-identity path as a parse failure —
    // no new error surface.
    let parsed = toml::from_str::<Identity>(content)
        .map_err(|e| e.to_string())
        .and_then(|identity| {
            validate_skills_config(&identity.skills)
                .map(|()| identity)
                .map_err(|e| format!("invalid [skills] config: {e}"))
        });
    match parsed {
        Ok(identity) => identity,
        Err(parse_err) => {
            let agent_name = agent_name_from_home(home_dir);

            // Well-known agents fail closed; user agents fall back to defaults.
            let is_well_known =
                crate::well_known_agents::find_well_known_agent(agent_name).is_some();

            if is_well_known {
                tracing::error!(
                    event = "identity_toml_malformed",
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
                    event = "identity_toml_malformed",
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

/// Fail-closed `Identity` for an agent whose `identity.toml` could not be read
/// (absent or unreadable — mika#2027, any agent) or failed to parse (mika#811,
/// well-known agents). Sentinel allowlist matches no real skill (evicts all
/// bundled skills); denylist contains every mutational built-in tool to prevent
/// the agent from acting until the operator fixes the file.
fn fail_closed_identity() -> Identity {
    Identity {
        name: default_name(),
        emoji: default_emoji(),
        reflection: None,
        heartbeat: None,
        kg: KgIdentityConfig::default(),
        skills: SkillsIdentityConfig {
            allowlist: Some(vec![FAIL_CLOSED_SKILL_SENTINEL.to_string()]),
            allow_authoring: None,
            nudge_enabled: None,
            nudge_interval: None,
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
            // Deliberately the plain default, not a tightened window (mika#2295).
            // This path is what an agent gets when its `identity.toml` is
            // malformed, absent or unreadable — including transiently, and
            // including for ordinary personal agents since mika#2027. What
            // fail-closed protects is *permission*: skills, tools, and the
            // summary leak. A context window is not a permission, and silently
            // amputating an agent's conversation history because its identity
            // file was briefly unreadable would be a new class of surprise, not a
            // safer posture.
            history: ContextHistoryConfig::default(),
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
    /// Runtime LLM provider name (e.g. `"anthropic"`, `"zai"`) — ground truth
    /// for "which model am I?" questions. Sourced from `llm.provider_name()` at
    /// turn-start; same source that populates `ToolContext.provider_name`, so
    /// the prompt-injected `## Runtime` section and the `get_active_llm` tool
    /// cannot drift. Consumed by `write_runtime_section` (`build_system_prompt`
    /// and `build_compact_system_prompt`). See mika#1815.
    pub runtime_provider: &'a str,
    /// Runtime LLM model name (e.g. `"claude-sonnet-4-6"`, `"glm-5.2"`).
    /// See `runtime_provider` for the ground-truth contract. Consumed by
    /// `write_runtime_section`. See mika#1815.
    pub runtime_model: &'a str,
    /// Where this instance runs (mika#2290) — ground truth for "where do you
    /// run / where does my data live", the sibling question of mika#1815's
    /// "which model are you". Resolved once at `server::init_agent` and cached
    /// on `AgentState`; never read from the environment here, so a mid-runtime
    /// env change cannot flip a running agent's hosting claim.
    pub deployment: Deployment,
    /// Persona register of the hosting line (mika#2290). Derived from the
    /// agent's cached tier, not from the deployment: a family or champion tenant
    /// gets the same fact without infrastructure vocabulary, because
    /// `FAMILY_SOUL` forbids it. See `hosting_ground_truth_line`.
    pub persona_profile: PersonaProfile,
}

fn onboarding_prompt() -> String {
    let section_names = core_memory_section_names();
    // Example name is deliberately generic (mika#1783 — flagged by adversarial
    // reviewer). The prior example named the platform operator ("Vincent"),
    // which taught every sealed being — including family-tier — the operator's
    // identity as an example person record. On the family-tier doctrine that
    // "the being does not have a maker it knows about," a persona-scrubbed
    // being would still learn the operator's name from this instruction and
    // reconstruct the same addressee-shaped leak. `Alex` (or any equally
    // generic placeholder) preserves the shape of the example without
    // seeding the referent. Enforced by `onboarding_prompt_no_operator_name`
    // unit test below.
    format!(
        "## First Session\n\
         This is your first conversation with the user. Introduce yourself briefly and warmly. \
         Ask who they are and what they're working on. Use update_core_memory to seed all \
         {} blocks ({}) from their \
         responses. Also use store_fact(category=\"person\") to create a record for the user \
         with just their first name (e.g. name=\"Alex\") and relationship \"The user\". \
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

/// Write the runtime LLM identity section (mika#1815).
///
/// Populated from the live `LlmProvider` instance at turn-start (same source
/// as `ToolContext.provider_name` / `model_name`), this is ground truth for
/// "which model / LLM are you?" questions. The Self-Identity Discipline
/// section directs the agent to quote this section verbatim (rule 1) and
/// forbids inferring the model from commented-out config lines or defaults.
fn write_runtime_section(
    prompt: &mut String,
    provider: &str,
    model: &str,
    deployment: Deployment,
    persona: PersonaProfile,
) {
    prompt.push_str("## Runtime\n");
    writeln!(
        prompt,
        "You are currently running on provider `{provider}` model `{model}`."
    )
    .unwrap();
    prompt.push_str(
        "This is the ground truth for questions about your own LLM/model. \
         Do NOT infer your model from commented-out config lines, defaults, or \"probably\" reasoning. \
         If a user asks which model you use, quote this line verbatim.\n",
    );
    prompt.push_str(hosting_ground_truth_line(deployment, persona));
    prompt.push('\n');
}

/// The hosting half of the `## Runtime` ground truth (mika#2290).
///
/// Two axes, and the crossing is a `match` with no `_ =>` arm on purpose — the
/// model of `dispatch_substrate_diagnostic` (`tools/mod.rs`): the compiler, not
/// a reviewer, is what forces a new persona or a new deployment state to make a
/// decision instead of inheriting one nobody took for it.
///
/// **Why the register follows the persona axis and not the hosting axis.**
/// `FAMILY_SOUL` forbids "toute mention … de l'infrastructure sous-jacente —
/// jamais, même si on te le demande". The cloud truth the ticket prescribes
/// (per-tenant isolation, exportable data, self-hostable MIT stack) *is*
/// infrastructure. Serving it verbatim to a family or champion tenant would
/// break the persona Vincent approved; serving nothing would leave the same void
/// that produced the false claim. So the same fact is written twice, and the
/// family register says it without a single technical term. No rule is derived
/// from the account locale or from the tenant — Prime's 2026-09-09 ruling,
/// carried over from mika#2023.
fn hosting_ground_truth_line(deployment: Deployment, persona: PersonaProfile) -> &'static str {
    match (persona, deployment) {
        (PersonaProfile::Operator, Deployment::Local) => {
            "You are running **on the user's own machine** (local install): the agent \
             process and the SQLite database live on that machine.\n"
        }
        (PersonaProfile::Operator, Deployment::Cloud) => {
            "You are running **in the cloud**, in an isolated per-tenant container — \
             NOT on the user's machine. When asked about privacy, say what is \
             verifiable: the tenant is isolated, the user's data belongs to the user \
             and is exportable, and the same open-source (MIT) stack can be \
             self-hosted locally if they prefer. Never claim local hosting.\n"
        }
        (PersonaProfile::Operator, Deployment::Unknown) => {
            "**Your hosting mode is not declared in this environment.** You do not \
             know whether you run on the user's machine or in the cloud. Say so \
             plainly if asked — do not guess, and do not default to \"local\".\n"
        }
        // Family register: the same fact, no infrastructure vocabulary. "Server"
        // is the one concrete noun, and it is the word the ticket's own approved
        // formulation uses.
        (PersonaProfile::Family, Deployment::Local) => {
            "You run on the person's own machine. What they tell you stays there. \
             Say it simply, in everyday words.\n"
        }
        (PersonaProfile::Family, Deployment::Cloud) => {
            "You run on a server, not on the person's phone or computer. What they \
             tell you belongs to them, and they can get it back whenever they want. \
             Say it in everyday words, with no technical detail — and never tell \
             them everything stays on their machine, because it does not.\n"
        }
        (PersonaProfile::Family, Deployment::Unknown) => {
            "You do not know where you run. If the person asks, say so simply — \
             never say that everything stays on their machine.\n"
        }
    }
}

/// Write the self-identity discipline section (mika#1815).
///
/// Directive-shaped rules that anchor on the `## Runtime` block above. Placed
/// after Runtime so rule 1 ("Quote, don't infer") has the ground-truth data
/// already in scope. Contrast anchor is mika#1784 (image-ingestion honesty)
/// — the same anti-fabrication virtue must apply self-referentially.
fn write_self_identity_discipline_section(prompt: &mut String) {
    prompt.push_str("## Self-Identity Discipline\n");
    prompt.push_str(
        "When a user asks about YOU — which model you are, which provider powers you, \
         WHERE you run and where their data lives, your configuration, your capabilities \
         — the ground truth is the `## Runtime` section above, populated from your live \
         LLM instance and from this environment's declared hosting. Follow these rules:\n\n",
    );
    prompt.push_str(
        "1. **Quote, don't infer.** For \"which model / LLM are you?\" quote the Runtime \
         section (or call `get_active_llm`). Do NOT reason from commented-out config \
         lines, \"probably\", \"vu le contexte\", or default fallbacks.\n\n",
    );
    prompt.push_str(
        "2. **Verb discipline.** \"I will VERIFY\" implies reading the source of truth \
         (Runtime section, `get_active_llm`, `get_config`, `read_agent_file`). \"I will \
         GUESS / INFER\" implies reasoning without a read. Never say VERIFY and then \
         deliver an INFERENCE.\n\n",
    );
    prompt.push_str(
        "3. **Fallback honestly.** If ground truth is genuinely unavailable \
         (Runtime section absent AND `get_active_llm` errors), say \"I cannot \
         reliably determine my model\" and point at where the configuration \
         lives (e.g. `~/.mika/config.toml`). Never fabricate a confident answer.\n\n",
    );
    prompt.push_str(
        "4. **Consistency across a single turn.** You may not say \"I don't know \
         with certainty\" and then in the next paragraph assert a model with \
         confidence. Uncertainty at t=0 and confidence at t=1 within the same \
         response is confabulation.\n\n",
    );
    prompt.push_str(
        "5. **Where you run is ground truth too.** \"Where do you run?\", \"where does \
         my data live?\", \"is this local?\", \"do my data leave my machine?\" are \
         self-identity questions, exactly like \"which model are you?\". The hosting \
         line of the `## Runtime` section above is the ground truth. NEVER assert that \
         you run locally, or that the user's data never leaves their machine, unless \
         that line says your hosting is local. If it says your hosting mode is not \
         declared, rule 3 applies — say you cannot reliably determine where you run. \
         \"Local\" is not a safe default: on a cloud tenant it is a false privacy \
         claim.\n\n",
    );
    // mika#2292 — rule 6, the symmetric sibling of rule 5. The fact section is
    // not enough on its own: the measured defect was a *use* of this discipline,
    // not an absence of content — the tenant held every fragment of the answer
    // and gave it in the same breath as "nothing found". So the scope widens
    // once more, and the measured answer is forbidden by name.
    //
    // Composed by interpolating `MIKA_DOCTRINE_HEADING` rather than repeating the
    // literal: a rename of the section must not be able to leave this rule green
    // while pointing at a section that no longer exists — the silently-broken
    // coupling `grooming_marker` (mika#2158) had to close on another surface.
    //
    // No signature change: the rule is register-neutral (it is a directive about
    // *where to look*), and this section is already served as-is to the family
    // tier with its tool names in backticks — `FAMILY_SOUL`'s jargon ban is on
    // what the tenant *says*, not on what its prompt contains.
    write!(
        prompt,
        "6. **What Mika is, what she stands for and why, is ground truth too.** \
         \"What is the Mika doctrine?\", \"what are your stances?\", \"what is your \
         philosophy?\", \"what do you believe in?\" are self-identity questions, \
         exactly like \"which model are you?\". The `{MIKA_DOCTRINE_HEADING}` section \
         above is the ground truth for them. Answering that you found nothing of \
         that name is NOT acceptable while that section is present — you hold the \
         answer, so give it. And answer it FROM this prompt: a question about what \
         Mika is, is not a memory lookup, and an empty search result is not \
         evidence that the thing does not exist.\n\n"
    )
    .unwrap();
    prompt.push_str(
        "This applies self-referentially: the anti-fabrication virtue you extend to \
         user-facing tasks (phone numbers, image contents, file paths) MUST also \
         apply to facts about yourself. Contrast reference: mika#1784 (you correctly \
         refused to fabricate what an unseen image contained — apply the same \
         integrity to self-identity questions).\n\n",
    );
}

/// Write the non-transit data-grade doctrine section (mika#1798).
///
/// Rendered on every turn of every agent that carries a normal system prompt,
/// in `build_system_prompt` and `build_silent_prompt`. The compact-provider
/// path (`build_compact_system_prompt`) uses a size-capped abbreviated variant
/// via `write_data_grade_doctrine_section_compact`.
///
/// **Position:** immediately after the identity section and before the time
/// section — so the doctrine is one of the very first grounding statements
/// the model consults, well ahead of `## Instructions` and any injected
/// core-memory or skill content (see CLAUDE.md § Context priority rule).
///
/// The block names testimony-grade classes explicitly, states the
/// HARD-NO-INCLUDING-PROPOSAL rule, and cites operational-grade carve-outs.
/// The doctrine originates from Prime's 2026-07-18 ratification (via
/// samidarko relay) and is one of three composable defense layers — the
/// prompt block, the skill-registry `apply_testimony_grade_ban` Phase 2,
/// and the execute-time guardrail in `tool_execution::dispatch`. Any single
/// layer failing does not silently open testimony-grade access. See
/// `crates/mika-agent/docs/non-transit-data-grade.md` for the full
/// architecture and vigilance surface.
fn write_data_grade_doctrine_section(prompt: &mut String) {
    prompt.push_str("## Data-Grade Doctrine (non-transit, mika#1798)\n");
    prompt.push_str(
        "You classify data by grade. **Operational-grade** data (calendar entries, \
         app-scoped file storage under `drive.file` scope) is accessible read-mostly \
         when a channel is explicitly wired. **Testimony-grade** data — Gmail message \
         content, full Drive, personal journals, confessional — is different: you \
         may NEVER access nor propose accessing testimony-grade data. HARD NO covers \
         BOTH the *doing* and the *proposing*: never invoke a tool that touches it, \
         and never verbalize a suggestion that you should get access to it. This \
         holds even when the user asks, even when it would be helpful, even when \
         you think you have a good reason. If the user needs help with email or \
         journal content, offer operational-grade or non-transit assistance instead \
         (schedule reminders, calendar suggestions, general advice) and name the \
         doctrine when refusing.\n\n\
         Opening testimony-grade access requires a code change reviewed by the \
         operator — you cannot request it, cannot grant it, and cannot bypass it. \
         This rule is structural: the skill registry evicts testimony-tagged skills \
         at load time, the tool dispatcher rejects testimony-tagged tool calls at \
         execute time, and the Gmail subcommand path refuses Gmail invocations \
         pre-spawn. If a single layer fails, the others catch the miss. Your job is \
         to make the miss impossible in the first place by neither proposing nor \
         attempting.\n\n",
    );
}

/// Abbreviated data-grade doctrine block for the compact provider (mika#1798).
///
/// The compact-provider path (`build_compact_system_prompt`, ~5 KB budget)
/// cannot afford the full doctrine block. This variant preserves the HARD-NO
/// invariant in a hard-capped budget (~400 chars, verified by
/// `data_grade_doctrine_compact_within_budget` const-assert test). The
/// MikaModel provider is not currently used by production family-tier or
/// operator-tier agents; if it goes live for real tenants, this abbreviated
/// form still gates the "propose Gmail access" surface at the prompt layer,
/// with Layers 2/3/4 providing structural backstop regardless of prompt
/// shape.
const DATA_GRADE_DOCTRINE_COMPACT: &str = "## Data-Grade Doctrine (mika#1798)\n\
     Testimony-grade data (Gmail, full Drive, journals, confessional) is \
     HARD NO — you may NEVER access nor propose accessing it. Operational-grade \
     (calendar, app-scoped files) is fine when wired. Structural: refusing is \
     not a preference, it is the rule.\n\n";

// Compile-time budget assertion: the compact block must stay under 400 chars
// so any future edit that busts the budget fails to compile (plan Risks §
// "Compact prompt budget").
const _: () = assert!(
    DATA_GRADE_DOCTRINE_COMPACT.len() < 400,
    "DATA_GRADE_DOCTRINE_COMPACT exceeds 400-char budget — see mika#1798 plan"
);

fn write_data_grade_doctrine_section_compact(prompt: &mut String) {
    prompt.push_str(DATA_GRADE_DOCTRINE_COMPACT);
}

// ---------------------------------------------------------------------------
// mika#1925 — stop-signal contract on the compact-provider path
// ---------------------------------------------------------------------------

/// Abbreviated *persist* half of the stop-signal contract (mika#1813 → mika#1925).
///
/// **Rendered unconditionally, and that is the load-bearing decision of
/// mika#1925.** The ticket's AC1 asks only for a block gated on
/// `stopped_topics` being non-empty — but on this path that state is
/// *unreachable*: `build_compact_system_prompt` renders no `## Instructions`
/// section, so the rule telling the agent to persist a stop-signal was served
/// on no conversation turn of this provider, and `build_silent_prompt` carries
/// the *consult* rule only. With nothing instructing the agent to write a
/// `stop_topic_*` preference, `search_preferences(STOP_TOPIC_PREFIX)` stays
/// empty for ever and a conditional block never renders. Wiring the conditional
/// half alone would have produced a green test suite over a dead path — the
/// literal shape of the defect this ticket exists to close.
///
/// It carries the AC2 distinction (a stop is not a gag: a direct question stays
/// answerable) in line, deliberately duplicating the one in
/// [`STOP_SIGNAL_CONSULT_PREAMBLE_COMPACT`]. The two are read at different
/// moments — one when the user says stop, the other when the agent considers
/// raising a subject — and a compact model does not go looking for a clause
/// three sections up. Same choice as the two reference builders, which also
/// carry it in both the section preamble and the instruction rule.
///
/// **Carries no `## ` heading, on purpose:** it is a rule, not a section, so
/// the nominal case (no stopped topic) adds no section to the compact prompt
/// and the pre-mika#1925 shape is preserved byte for byte apart from this rule.
///
/// Budget: const-asserted below at 600 bytes, **on this constant alone**. No
/// const-assert can bound the total — that depends on `soul.md` and on the
/// injected preference values, neither known at compile time — so the
/// `total ≤ 5120` unit test
/// (`mika1925_compact_prompt_stays_within_budget_with_stopped_topics`) is the
/// net that sees the sum, not a duplicate of this assertion.
const STOP_SIGNAL_PERSIST_COMPACT: &str = "Stop signals: when the user asks you to stop bringing a subject up (\"stop\", \
     \"arrête\", \"no more\", \"laisse tomber\"), acknowledge briefly AND call \
     store_fact(category='preference', key='stop_topic_<short-slug>', value='<one \
     line: what to stop, with today's date>') in the SAME turn. <short-slug> is \
     lowercase kebab-case: subject `web-config` → key='stop_topic_web-config'. \
     A direct question about a stopped subject is not a re-opening — answer it \
     normally; what you stop is raising it yourself.\n";

// Compile-time budget assertion — scoped to this constant, see its doc comment
// for why no const-assert of the total can exist (mika#1925).
const _: () = assert!(
    STOP_SIGNAL_PERSIST_COMPACT.len() < 600,
    "STOP_SIGNAL_PERSIST_COMPACT exceeds 600-byte budget — see mika#1925 plan"
);

/// Abbreviated preamble of the *consult* half (mika#1813 → mika#1925).
///
/// Rendered under a `## Stopped Topics` heading when `stopped_topics` is
/// non-empty, immediately before the `<stopped-topics>` block — the same shape
/// as [`build_system_prompt`] and [`build_silent_prompt`], abbreviated in
/// content. The form is a deliberate mirror rather than an invention: the risk
/// the compact prompt exists to avoid (OOD-prior-dominated completion of
/// markdown structure) is **not measurable in this repository** — no MikaModel
/// endpoint exists here — and when one cannot measure, one does not depart from
/// the known-good shape. What that mirror costs is named: the
/// `<stopped-topics>` block is the first XML tag rendered on this path.
/// `sanitize_label` already strips `<`, `>`, `\n`, `\r` from injected content,
/// so a hostile preference value cannot close the tag; the residual risk is one
/// of completion, and it belongs to the calibration pass (AC4) documented at
/// [`build_compact_system_prompt`].
///
/// Budget: const-asserted below at 300 bytes, on this constant alone — same
/// scoping note as [`STOP_SIGNAL_PERSIST_COMPACT`].
const STOP_SIGNAL_CONSULT_PREAMBLE_COMPACT: &str = "The user asked you NOT to re-initiate on these subjects. Do not raise them \
     yourself. A direct question about one of them is not a re-opening — you may \
     still answer it.\n";

const _: () = assert!(
    STOP_SIGNAL_CONSULT_PREAMBLE_COMPACT.len() < 300,
    "STOP_SIGNAL_CONSULT_PREAMBLE_COMPACT exceeds 300-byte budget — see mika#1925 plan"
);

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
    // mika#1814 — Distribution Doctrine binds before identity/time/channel
    // context so the invitation-only limit is the first per-agent rule the
    // model reads. Code-managed (constants at top of file), NOT user-editable
    // via soul.md.
    write_distribution_doctrine_section(&mut prompt);
    // mika#2292 — the material doctrine, aliased "doctrine", sits against its
    // sister rather than inside `## Runtime`. Three reasons, strongest first:
    // (1) `## Runtime` is a block of *machine* facts populated at execution time,
    // where this is a block of *project* facts constant at the binary — merging
    // them would make one section two things, and a section that grows without a
    // frontier is the one a later ticket cuts in the wrong place; (2) the two
    // doctrines read together, this body *citing* `## Distribution Doctrine`
    // instead of re-narrating growth-by-invitation — a reference to the section
    // immediately above is adjacency, a reference across `## Identity` and
    // `## Runtime` is distance, and distance is what produced the measured
    // defect; (3) the mika#1814 comment above already motivates this slot with
    // "binds before identity/time/channel context", which holds word for word.
    // Cost, named: rule 6 cites a section that is no longer its neighbour —
    // mitigated by the citation naming it by heading, which is also why the test
    // asserts *order* rather than adjacency.
    write_mika_doctrine_section(&mut prompt, ctx.persona_profile);
    write_identity_section(&mut prompt, ctx.identity);
    // Runtime ground truth (mika#1815) — placed between Identity and Current Time
    // so the "who am I / what am I running on" block reads coherently. The
    // Self-Identity Discipline section below quotes this data as ground truth.
    // mika#2290 extends "what am I running on" to "where am I running": the
    // block was already the right home for it, already declared ground truth,
    // and already ahead of Time/Channel/core-memory.
    write_runtime_section(
        &mut prompt,
        ctx.runtime_provider,
        ctx.runtime_model,
        ctx.deployment,
        ctx.persona_profile,
    );
    write_self_identity_discipline_section(&mut prompt);
    // mika#1798: non-transit doctrine — rendered BEFORE time / channel /
    // core-memory / instructions so it grounds every downstream section.
    write_data_grade_doctrine_section(&mut prompt);
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
         query_timeline) to verify it first.\n  \
         This covers message delivery too (mika#2136): an errored send_message result \
         means the user received NOTHING. Never announce a delivery a tool refused, and \
         when you send content in parts, name the part that failed before moving on to \
         the next one.\n  \
         BAD: send_message returns \"too long\" → you say \"Here it is in full 👆\"\n  \
         GOOD: send_message returns \"too long\" → you say it is too long to send at once, \
         then send it split into parts.\n",
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
    // mika#2237 — a clause, deliberately NOT an inversion of the line above.
    // Core memory outranking skill context is right for preferences and for the
    // `## Stopped Topics` block (mika#1813); what it must not do is let a
    // memory learned from a past failure quietly override an explicit
    // operational mapping of the active skill. Measured 2026-09-08: mika-qa
    // posted `--comment` on a `VERDICT: pass` body because it remembered the
    // 137 self-approval refusals from before mika#2218 fixed the review
    // identity — the skill said the right thing and the memory won in silence.
    //
    // This is the *intent* half and is declared as such. Per
    // `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` it
    // does not hold on its own; the structural half is the pre-subprocess guard
    // `validate_pr_review_flag_coherence`. It is still necessary: without it the
    // prompt goes on instructing the agent to prefer the memory.
    prompt.push_str(
        "- **A skill's operational mapping outranks a memory of past failure:** when an active \
         skill states an explicit conditional instruction (\"if X, then do Y\") and something you \
         remember says Y will not work, follow the skill and try Y anyway. A remembered failure \
         is evidence about the past, not about the case in front of you — the constraint may \
         have been lifted since. If Y then actually fails, you may fall back, but say which \
         attempt failed and how. Never silently substitute a different action because you \
         recall a similar one failing before: name the conflict instead of resolving it in \
         favour of the memory.\n",
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
/// **mika#1925 — the stop-signal contract (mika#1813) is rendered here, and
/// the reasoning behind its shape is not what the carve-out it replaces said.**
/// Three things are worth knowing before editing it:
///
/// 1. **This was never a byte budget.** The carve-out claimed "the compact
///    budget cannot afford it"; measured, the sections above total ≈ 431 bytes
///    against the 5120 asserted ceiling — 91 % unused, and the *full* mika#1813
///    contract would have fitted three times over. What actually bit was the
///    section-count assertion in
///    `test_build_compact_system_prompt_size_bound`, and behind it the OOD
///    failure mode this builder exists to avoid: served full context, the
///    provider completes markdown structure instead of acting as an agent
///    (fictional `## Summary / Completed Tasks / Pending` reports). The fiction
///    observed was made of *headings*. So the form is what is dangerous, not
///    the mass — and any future justification written in bytes would be false.
/// 2. **The form is the deliberate mirror of the two other builders**, not an
///    invention, because the OOD risk is structurally unmeasurable here (no
///    MikaModel endpoint exists in this repository; `default_base_url` is an
///    Ollama-shaped `http://localhost:11434` nothing serves). One does not
///    depart from a known-good shape on an unverifiable intuition.
/// 3. **The *persist* rule is unconditional, and must stay so.** Conditioned on
///    `stopped_topics` being non-empty — the letter of mika#1925's AC1 — it
///    would be unreachable: this builder renders no `## Instructions`, so
///    nothing would ever tell a MikaModel agent to write a `stop_topic_*`
///    preference, the block would never fill, and the wire-up would be a no-op
///    with a green test suite over it. See [`STOP_SIGNAL_PERSIST_COMPACT`].
///    `mika1925_compact_prompt_renders_persist_even_with_no_stopped_topics`
///    is what stops a later "simplification" from putting it back under the
///    condition.
///
/// **AC4 is NOT satisfied, and is a blocking precondition rather than a
/// deferral.** mika#1925 asks for a calibration pass on MikaModel (mika#1190
/// discipline). It is not executable in this repository, for three independent
/// reasons: no MikaModel endpoint exists; `calibration::providers::create_real_provider`
/// returns `None` for any provider without a `MIKA_<PREFIX>_API_KEY` and
/// exempts only `ProviderKind::Ollama`, so MikaModel cannot even be
/// instantiated without setting a key it has no use for; and none of the four
/// role suites (`mika_dev`, `mika_arch`, `mika_qa`, `mika_orchestrator`)
/// exercises a conversational agent or the stop-signal contract, so running one
/// "on MikaModel" would scrupulously measure something else. It was not
/// simulated with a mock — a `MockLlmProvider` validates prompt *shape*, never
/// model *obedience*. **Before MikaModel serves a real tenant, run that
/// calibration pass**; the three dependencies above are what it costs.
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

    // ## Runtime — compact one-line variant (mika#1815). The MikaModel compact
    // budget cannot afford the full Self-Identity Discipline block, but the
    // ground-truth line itself is ~50 bytes and pays for itself the first time
    // a user asks "which model?" — quoting the line beats confabulating.
    //
    // mika#2290 carve-out, aligned with the mika#1814 / mika#1925 ones above:
    // the hosting line is deliberately NOT rendered here. What that carve-out
    // costs is bounded and worth saying — it withholds the *intent* half from
    // this path, never the protection: the 5d guard reads outgoing text, not the
    // prompt, so a MikaModel turn that asserts local hosting on a non-local
    // deployment is refused exactly like any other. Pinned by
    // `mika2290_compact_prompt_omits_the_hosting_line` as a decision, not an
    // oversight; joined to the mika#1925 follow-up.
    prompt.push_str("## Runtime\n");
    writeln!(
        prompt,
        "Provider `{}`, model `{}`.",
        ctx.runtime_provider, ctx.runtime_model
    )
    .unwrap();
    prompt.push('\n');

    // mika#2292 carve-out, the fourth of this family and the same shape as the
    // mika#1813 / mika#1814 / mika#2290 ones. The carve-out is decided **per
    // section**, not globally: the compact path does render a doctrine — the
    // mika#1798 abbreviated data-grade one below — because that one carries a
    // HARD-NO invariant whose violation is irreversible and therefore worth its
    // bytes. `## Mika Doctrine` is a *fact*, not a guard, so what is withheld
    // here is the intent half and never a protection.
    //
    // Cost, named, and real here where it was not for mika#2290: on this path
    // the measured defect **stays open** — a MikaModel tenant asked about "the
    // doctrine" falls back on rule 3 for want of the section. Accepted because
    // the measured population (Al's champion tenant) is not served by this path,
    // and joined to the mika#1925 follow-up with the three other carve-outs
    // rather than closed here with an exception. Pinned as a decision by
    // `mika2292_compact_prompt_omits_the_doctrine_section`.
    //
    // mika#1798: abbreviated non-transit doctrine — ~250 chars, hard-capped at
    // 400. Preserves the HARD-NO invariant even in the compact budget.
    write_data_grade_doctrine_section_compact(&mut prompt);

    // mika#1925 — stop-signal contract (mika#1813), in the shape of the two
    // other builders: the block first (context), the rule after (instruction).
    //
    // The block is conditional, as AC1 asks. The `persist` rule below is NOT —
    // see the point 3 of this function's doc comment and
    // `STOP_SIGNAL_PERSIST_COMPACT`: conditioned, it would never be served, and
    // the block it fills would never fill.
    if !ctx.stopped_topics.is_empty() {
        prompt.push_str("## Stopped Topics\n");
        prompt.push_str(STOP_SIGNAL_CONSULT_PREAMBLE_COMPACT);
        prompt.push_str("<stopped-topics>\n");
        for pref in ctx.stopped_topics {
            let cat = sanitize_label(&pref.category);
            let val = sanitize_label(&pref.value);
            writeln!(prompt, "- {cat}: {val}").unwrap();
        }
        prompt.push_str("</stopped-topics>\n\n");
    }
    prompt.push_str(STOP_SIGNAL_PERSIST_COMPACT);

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
    /// Runtime LLM provider name (mika#1815) — same contract as
    /// `PromptContext.runtime_provider`. Heartbeat/callback/reflection turns
    /// also carry the ground-truth `## Runtime` section so a self-identity
    /// question during a background turn hits the same guardrail.
    pub runtime_provider: &'a str,
    /// Runtime LLM model name (mika#1815) — companion to `runtime_provider`.
    pub runtime_model: &'a str,
    /// Where this instance runs (mika#2290). Silent turns render the hosting
    /// line too: a heartbeat that asserts "local" on a cloud tenant is exactly
    /// as false as a conversation-mode one, and the compacted history the next
    /// conversational turn inherits carries it forward.
    pub deployment: Deployment,
    /// Persona register of the hosting line (mika#2290). See
    /// `PromptContext::persona_profile`.
    pub persona_profile: PersonaProfile,
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
    // mika#1814 — Distribution Doctrine binds in silent mode too: a heartbeat
    // that spontaneously drafts a Show HN would be exactly as bad as a
    // conversation-mode one. Same code-managed section, applied uniformly.
    write_distribution_doctrine_section(&mut prompt);
    // mika#2292 — rendered on silent turns too, and for a reason of its own
    // rather than by symmetry. The *material* register is harmless in silent
    // mode; the **stop** is not. A silent turn that wrote a memory fact on the
    // spiritual register would poison every later turn through core memory,
    // which is re-injected into every prompt. Same section, same slot.
    write_mika_doctrine_section(&mut prompt, ctx.persona_profile);
    write_identity_section(&mut prompt, ctx.identity);
    // Runtime ground truth (mika#1815) — heartbeat/callback/reflection turns
    // may still be asked "which model are you?" via a subsequent user message
    // or in the compacted history the next conversation-mode turn inherits.
    // Same section shape as `build_system_prompt` so downstream discipline is
    // uniform — hosting line included (mika#2290).
    write_runtime_section(
        &mut prompt,
        ctx.runtime_provider,
        ctx.runtime_model,
        ctx.deployment,
        ctx.persona_profile,
    );
    write_self_identity_discipline_section(&mut prompt);
    // mika#1798: non-transit doctrine — rendered on silent turns too so the
    // model consults it before any autonomous send_message on testimony-grade
    // subjects (e.g., "should I fetch this user's Gmail?").
    write_data_grade_doctrine_section(&mut prompt);
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.starts_with("You are a sharp, proactive executive assistant."));
    }

    // mika#1583 F1 — nudge_interval = 0 is rejected at the pure-validator layer.
    #[test]
    fn test_validate_skills_config_rejects_zero_interval() {
        let mut skills = SkillsIdentityConfig {
            nudge_interval: Some(0),
            ..Default::default()
        };
        assert!(validate_skills_config(&skills).is_err());
        skills.nudge_interval = Some(5);
        assert!(validate_skills_config(&skills).is_ok());
        skills.nudge_interval = None;
        assert!(validate_skills_config(&skills).is_ok());
    }

    // mika#1583 F1 — a user-defined identity with nudge_interval = 0 routes into
    // the existing malformed-identity fallback (Identity::default), not a crash.
    #[test]
    fn test_identity_load_rejects_zero_nudge_interval() {
        // Non-well-known agent name → validation failure falls back to default.
        let home = Path::new("/tmp/some-user-agent-1583");
        let path = home.join("identity.toml");
        let bad =
            "name = \"X\"\nemoji = \"x\"\n\n[skills]\nnudge_enabled = true\nnudge_interval = 0\n";
        let id = parse_identity_or_fail_closed(bad, home, &path);
        assert_eq!(id.skills.nudge_interval, None);
        assert!(!id.skills.nudge_is_enabled());

        // A valid interval parses through and is preserved.
        let good =
            "name = \"X\"\nemoji = \"x\"\n\n[skills]\nnudge_enabled = true\nnudge_interval = 5\n";
        let id2 = parse_identity_or_fail_closed(good, home, &path);
        assert_eq!(id2.skills.nudge_interval, Some(5));
        assert!(id2.skills.nudge_is_enabled());
        assert_eq!(id2.skills.resolved_nudge_interval(), 5);
    }

    // mika#1583 — off-by-default: absent nudge fields default to disabled with
    // interval 10. Well-known identities are intentionally NOT in this fixture.
    #[test]
    fn test_nudge_off_by_default() {
        let skills = SkillsIdentityConfig::default();
        assert!(!skills.nudge_is_enabled());
        assert_eq!(skills.resolved_nudge_interval(), 10);
        assert!(!skills.authoring_enabled());

        let home = Path::new("/tmp/some-user-agent-1583");
        let path = home.join("identity.toml");
        let toml = "name = \"X\"\nemoji = \"x\"\n\n[skills]\nallowlist = [\"foo\"]\n";
        let id = parse_identity_or_fail_closed(toml, home, &path);
        assert!(!id.skills.nudge_is_enabled());
        assert_eq!(id.skills.resolved_nudge_interval(), 10);
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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

    /// A missing `identity.toml` keeps the *display* identity (name/emoji) so
    /// listing surfaces still read sensibly — what it does NOT keep is the
    /// permissive allowlist. See the fail-closed tests below.
    #[test]
    fn test_load_identity_keeps_display_defaults_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let identity = load_identity(tmp.path());
        assert_eq!(identity.name, "Mika");
        assert_eq!(identity.emoji, "✦");
    }

    /// mika#2027 AC1 — Fire-Disposition detector.
    ///
    /// RED before the fix: `Err(_) => Identity::default()` gave
    /// `allowlist: None`, which `apply_identity_allowlist` treats as a no-op —
    /// i.e. every bundled skill active, `shell-exec` / `git-ops` / `github` /
    /// `tmux` included. Deleting an agent's identity file *widened* it.
    /// GREEN after: the same fail-closed sentinel the malformed path uses.
    #[test]
    fn mika2027_absent_identity_toml_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!tmp.path().join("identity.toml").exists());

        let identity = load_identity(tmp.path());

        let allowlist = identity
            .skills
            .allowlist
            .as_ref()
            .expect("absent identity.toml must NOT yield a permissive (None) allowlist");
        assert_eq!(allowlist, &[FAIL_CLOSED_SKILL_SENTINEL.to_string()]);

        // The sentinel is only fail-closed if it names nothing real: an
        // allowlist of one existing skill would leave that skill active.
        assert!(
            !crate::bundled_skills::is_bundled_skill(FAIL_CLOSED_SKILL_SENTINEL),
            "the fail-closed sentinel must match no bundled skill"
        );

        // Summary injection cannot be re-enabled by the absence of a file
        // (mika#1009 leak protection).
        assert!(!identity.context.summary.inject);

        // Platform-mutational tools denied — pinned against the actual constant,
        // not merely "non-empty", so a future edit to the list is a visible diff.
        let disabled: std::collections::HashSet<&str> =
            identity.tools.disabled.iter().map(String::as_str).collect();
        for tool in crate::well_known_agents::MIKA_ARCH_DISABLED_TOOLS {
            assert!(disabled.contains(tool), "{tool} must be denied fail-closed");
        }

        // And what it deliberately does NOT deny, written down so nobody reads
        // "fail-closed" as "inert": #811 keeps the agent-scoped memory writes,
        // counting them as constitutive of being an agent rather than platform
        // side effects. A neutered agent can still write core memory, which is
        // re-injected into every later system prompt. Whether the fail-closed
        // path wants a stricter list than mika-arch's steady-state one is open —
        // the two share this constant, so tightening it here tightens mika-arch.
        for still_allowed in ["update_core_memory", "store_fact", "update_fact"] {
            assert!(
                !disabled.contains(still_allowed),
                "{still_allowed} is expected to remain allowed; if that changed \
                 deliberately, update this assertion and the paragraph above it"
            );
        }
    }

    /// mika#2027 F1 — the fail-closed posture is **universal**. A user-defined
    /// agent (not in `WELL_KNOWN_AGENTS`) gets the same sentinel: AC1 admits no
    /// exception, and discriminating here would reopen the "which agent is
    /// supposed to have a file?" ambiguity the universal rule closes.
    #[test]
    fn mika2027_absent_identity_toml_fails_closed_for_user_defined_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_home = tmp.path().join("agents").join("vincent-perso");
        std::fs::create_dir_all(&agent_home).unwrap();
        assert!(
            crate::well_known_agents::find_well_known_agent("vincent-perso").is_none(),
            "fixture must name a user-defined agent for this test to mean anything"
        );

        let identity = load_identity(&agent_home);

        assert_eq!(
            identity.skills.allowlist,
            Some(vec![FAIL_CLOSED_SKILL_SENTINEL.to_string()])
        );
    }

    /// The async loader is the one the server, task engine and agent loop
    /// actually call. A fix that only landed on the sync path would leave the
    /// running daemon permissive.
    #[tokio::test]
    async fn mika2027_absent_identity_toml_fails_closed_on_the_async_path() {
        let tmp = tempfile::tempdir().unwrap();

        let identity = load_identity_async(tmp.path()).await;

        assert_eq!(
            identity.skills.allowlist,
            Some(vec![FAIL_CLOSED_SKILL_SENTINEL.to_string()])
        );
        assert!(!identity.context.summary.inject);
    }

    /// A present-but-unreadable file is a different fault from an absent one
    /// (mika#2027 AC2) — but it fails closed just the same. Only the log event
    /// discriminates, because only the remediation differs.
    #[cfg(unix)]
    #[test]
    fn mika2027_unreadable_identity_toml_also_fails_closed() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("identity.toml");
        std::fs::write(&path, "name = \"Agent X\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        // Running as root defeats the permission bits; skip rather than assert
        // something the environment cannot produce.
        if std::fs::read_to_string(&path).is_ok() {
            return;
        }

        let identity = load_identity(tmp.path());

        assert_eq!(
            identity.skills.allowlist,
            Some(vec![FAIL_CLOSED_SKILL_SENTINEL.to_string()]),
            "an unreadable identity.toml must not read as 'everything permitted' either"
        );
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let prompt = build_system_prompt(&ctx);
        // Should start directly with the Distribution Doctrine section
        // (mika#1814) when soul is empty — the doctrine section renders
        // between soul and identity and is code-managed (not tied to soul
        // content). Identity follows immediately after.
        // safe-byte-slice: DISTRIBUTION_DOCTRINE_HEADING is a compile-time
        // ASCII-only const ("## Distribution Doctrine") — byte offsets equal
        // char boundaries by construction, no multi-byte UTF-8 risk.
        let prefix_end = DISTRIBUTION_DOCTRINE_HEADING.len().min(prompt.len());
        assert!(
            prompt.starts_with(DISTRIBUTION_DOCTRINE_HEADING),
            "empty-soul prompt should start with the Distribution Doctrine \
             heading (soul absent → doctrine is first). Actual prefix: {:?}",
            &prompt[..prefix_end] // safe-byte-slice: ASCII-only heading (see comment above)
        );
        assert!(
            prompt.contains("## Identity"),
            "empty-soul prompt should still render the Identity section \
             immediately after the doctrine section"
        );
    }

    // -- Non-transit data-grade doctrine tests (mika#1798) --

    #[test]
    fn build_system_prompt_includes_data_grade_doctrine_section() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_system_prompt(&ctx);

        // Section heading appears exactly once.
        assert_eq!(
            prompt.matches("## Data-Grade Doctrine").count(),
            1,
            "doctrine section must appear exactly once"
        );

        // Literal invariants: names testimony-grade, states HARD-NO-including-propose,
        // names Gmail + full Drive.
        assert!(
            prompt.contains("testimony-grade"),
            "must name testimony-grade explicitly"
        );
        assert!(
            prompt.contains("may NEVER access nor propose accessing"),
            "must state the HARD-NO-including-proposal rule verbatim"
        );
        assert!(prompt.contains("Gmail"), "must name Gmail");
        assert!(prompt.contains("full Drive"), "must name full Drive");
    }

    #[test]
    fn build_silent_prompt_includes_data_grade_doctrine_section() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_silent_prompt(&ctx);

        assert_eq!(
            prompt.matches("## Data-Grade Doctrine").count(),
            1,
            "doctrine section must appear exactly once in silent prompt"
        );
        assert!(prompt.contains("testimony-grade"));
        assert!(prompt.contains("may NEVER access nor propose accessing"));
        assert!(prompt.contains("Gmail"));
    }

    #[test]
    fn build_compact_system_prompt_includes_abbreviated_doctrine() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_compact_system_prompt(&ctx);

        // Abbreviated form must still name the two load-bearing anchors.
        // Case-insensitive check on "testimony-grade" — the compact block
        // capitalizes at sentence start ("Testimony-grade data...").
        let lower = prompt.to_lowercase();
        assert!(
            lower.contains("testimony-grade"),
            "compact doctrine must name testimony-grade (any case)"
        );
        assert!(prompt.contains("Gmail"), "compact doctrine must name Gmail");
        assert!(
            prompt.contains("HARD NO"),
            "compact doctrine must state HARD NO"
        );

        // Budget: the abbreviated block itself is < 400 chars (const-asserted at
        // compile time). Verify at the composed-prompt level that the doctrine
        // block is present but small.
        assert!(
            DATA_GRADE_DOCTRINE_COMPACT.len() < 400,
            "compact doctrine block busted 400-char budget"
        );
    }

    #[test]
    fn data_grade_section_position_stable() {
        // The doctrine block must appear BEFORE `## Instructions` and BEFORE
        // any injected core-memory content. Position matters for prompt-priority
        // per CLAUDE.md § Context priority rule (system prompt > core memory > ...).
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_system_prompt(&ctx);

        let doctrine_idx = prompt
            .find("## Data-Grade Doctrine")
            .expect("doctrine section must be present");
        let instructions_idx = prompt
            .find("## Instructions")
            .expect("instructions section must be present");
        let core_memory_idx = prompt
            .find("## Core Memory")
            .expect("core memory section must be present");

        assert!(
            doctrine_idx < instructions_idx,
            "doctrine must appear before ## Instructions"
        );
        assert!(
            doctrine_idx < core_memory_idx,
            "doctrine must appear before ## Core Memory (grounding-first)"
        );
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
                dispatcher_source: None,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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

    /// mika#1783 addendum — persona reconstruction via onboarding prompt.
    ///
    /// The onboarding prompt is loaded on every fresh install regardless of
    /// tier. An operator-identity example name here nullifies FAMILY_SOUL
    /// scrubbing: the sealed being's very first system prompt seeds the
    /// operator's identity into its person-record space via example.
    /// Adversarial review flagged this as an IN-SCOPE HIGH finding.
    ///
    /// This test locks in the generic-placeholder rule: the example name
    /// must not name any known operator identity.
    #[test]
    fn onboarding_prompt_no_operator_name() {
        let prompt = onboarding_prompt();
        // Same operator-identity token set as FAMILY_SOUL's invariant test
        // in mika-common::home. If this list grows, keep the two in sync.
        const FORBIDDEN_TOKENS: &[&str] = &["Vincent", "vincent"];
        for token in FORBIDDEN_TOKENS {
            assert!(
                !prompt.contains(token),
                "onboarding_prompt() must not contain operator-identity \
                 token {token:?} — the example name is loaded into every \
                 fresh install regardless of tier and would seed the \
                 operator's identity into a sealed family-tier being's \
                 person-record space. See mika#1783 (adversarial review \
                 F2). Prompt was: {prompt:?}"
            );
        }
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Context priority:"));
        assert!(prompt.contains("current user message > core memory > active skill context"));
        assert!(prompt.contains("conversation summary"));
        assert!(prompt.contains("conversation history > search results"));
    }

    /// mika#2237 (U3 / V9) — the skill-over-stale-memory clause is a DECISION.
    ///
    /// It is pinned rather than left to prose because its disappearance would
    /// be invisible: a future editor who finds the priority bullet long will
    /// shorten it, every other assertion stays green, and nobody learns the
    /// intent half of mika#2237 is gone. The structural half
    /// (`validate_pr_review_flag_coherence`) keeps working, which is exactly
    /// what makes the loss silent.
    ///
    /// Note it does NOT invert the priority line above it: core memory still
    /// outranks skill context, which is right for preferences and for the
    /// mika#1813 `## Stopped Topics` block (D7).
    #[test]
    fn mika2237_skill_mapping_outranks_a_remembered_failure() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("A skill's operational mapping outranks a memory of past failure"),
            "the mika#2237 clause is missing — if it was shortened away, restore it: it is \
             the intent half of a defect whose structural half cannot announce its loss"
        );
        assert!(
            prompt.contains("try Y anyway"),
            "the clause must prescribe the attempt, which is the whole substance of D4: \
             degradation stays possible, but only after a measured attempt"
        );
        assert!(
            prompt.contains("name the conflict"),
            "the clause must require the conflict to be said rather than resolved in \
             favour of the memory — that is the ticket's fix (c)"
        );

        // The priority line it qualifies is still there and still in that order.
        assert!(
            prompt.contains("current user message > core memory > active skill context"),
            "mika#2237 adds a clause; it does not invert the priority line (D7)"
        );
    }

    /// The compact path (MikaModel, ≤5 KB) renders neither the priority line
    /// nor its mika#2237 clause — fifth carve-out of that family, joined to the
    /// mika#1925 follow-up like the other four. Nothing is withheld but the
    /// *intent* half: the pre-subprocess guard reads argv, not the prompt.
    #[test]
    fn mika2237_compact_prompt_omits_the_clause_like_its_parent_line() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let compact = build_compact_system_prompt(&ctx);
        assert!(
            !compact.contains("Context priority:"),
            "precondition: the compact path does not carry the priority line"
        );
        assert!(
            !compact.contains("A skill's operational mapping outranks"),
            "the clause must not be added to the compact path on its own — it qualifies a \
             line that path does not render"
        );
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

    // -- ContextHistoryConfig deserialization tests (mika#2295) --

    /// AC2 — an identity that says nothing about the window gets the old window.
    #[test]
    fn mika2295_absent_history_block_is_the_pre_fix_behaviour() {
        let identity: Identity = toml::from_str(
            r#"
name = "Dev"
"#,
        )
        .unwrap();

        assert_eq!(identity.context.history.scope, HistoryScope::Agent);
        assert_eq!(identity.context.history.max_tokens, None);
    }

    #[test]
    fn mika2295_history_block_parses_both_scopes_and_a_ceiling() {
        let identity: Identity = toml::from_str(
            r#"
name = "Architect"

[context.history]
scope = "session"
max_tokens = 8000
"#,
        )
        .unwrap();

        assert_eq!(identity.context.history.scope, HistoryScope::Session);
        assert_eq!(identity.context.history.max_tokens, Some(8000));

        let agent_scoped: Identity =
            toml::from_str("[context.history]\nscope = \"agent\"\n").unwrap();
        assert_eq!(agent_scoped.context.history.scope, HistoryScope::Agent);
    }

    /// AC4 — a malformed ceiling degrades to "no ceiling", and does not fail the
    /// parse.
    ///
    /// Both halves matter. Reading an unusable value as `0` would silently wipe
    /// the history — a context erasure wearing a configuration's clothes, and
    /// strictly worse than the unbounded window this field exists to bound. And
    /// failing the parse is not inert either: for a well-known agent that routes
    /// into the fail-closed sentinel, which neuters the agent and unlinks its
    /// bundled skills from disk. A typo should cost neither.
    #[test]
    fn mika2295_malformed_ceiling_degrades_to_no_ceiling_without_failing_the_parse() {
        for value in ["-1", "\"lots\"", "3.5", "true"] {
            let toml_src = format!("name = \"Dev\"\n\n[context.history]\nmax_tokens = {value}\n");
            let identity: Identity = toml::from_str(&toml_src)
                .unwrap_or_else(|e| panic!("`max_tokens = {value}` must not fail the parse: {e}"));

            assert_eq!(
                identity.context.history.max_tokens, None,
                "`max_tokens = {value}` must read as no ceiling, never as zero"
            );
        }
    }

    /// AC4's sibling — an unknown scope degrades to the historical scope.
    #[test]
    fn mika2295_unknown_scope_degrades_to_the_default_without_failing_the_parse() {
        for value in ["\"sesion\"", "\"ticket\"", "7"] {
            let toml_src = format!("name = \"Dev\"\n\n[context.history]\nscope = {value}\n");
            let identity: Identity = toml::from_str(&toml_src)
                .unwrap_or_else(|e| panic!("`scope = {value}` must not fail the parse: {e}"));

            assert_eq!(identity.context.history.scope, HistoryScope::Agent);
        }
    }

    #[test]
    fn mika2295_token_budget_uses_the_same_estimator_as_the_summary_path() {
        // If these two ever disagree, one heuristic has quietly become two.
        assert_eq!(token_budget_to_bytes(1), CHARS_PER_TOKEN_ESTIMATE);
        assert_eq!(token_budget_to_bytes(8000), 8000 * CHARS_PER_TOKEN_ESTIMATE);
        assert_eq!(token_budget_to_bytes(0), 0);
        assert_eq!(token_budget_to_bytes(usize::MAX), usize::MAX, "saturating");
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let prompt = build_compact_system_prompt(&ctx);

        // AC1: ≤5 KB
        assert!(
            prompt.len() <= 5120,
            "compact prompt is {} bytes, exceeds 5 KB limit",
            prompt.len()
        );

        // AC1: ≤5 sections. The compact prompt's section budget grew from 2 to
        // 4 and then to 5 as load-bearing doctrines were baked in — each
        // addition is a structural guardrail the compact/MikaModel provider
        // cannot skip:
        //   1. ## Personality (soul first-line, when non-empty)
        //   2. ## Identity (agent name)
        //   3. ## Runtime (mika#1815 — ground-truth model identity so the
        //      compact provider can answer "which model?" without confabulating)
        //   4. ## Data-Grade Doctrine (mika#1798 — abbreviated HARD-NO
        //      non-transit invariant, ~400-char budget const-asserted)
        //   5. ## Stopped Topics (mika#1925 — the mika#1813 suppression list;
        //      CONDITIONAL, rendered only when the user has actually asked for
        //      a subject to be dropped, so it costs nothing on the nominal
        //      turn. Its companion `persist` rule is unconditional but carries
        //      no heading, which is why the fixture below — `stopped_topics:
        //      &[]` — still counts four.)
        // Each section is individually budget-capped; the overall ≤5 KB bound
        // (asserted above) is the load-bearing size guarantee.
        //
        // The count is the barrier this builder actually has — the byte budget
        // is ~91 % unused (mika#1925 D1). Raising it again means naming the
        // section, its ticket, and what it guarantees, exactly as above; a bump
        // without that paragraph is how a reasoned growth becomes a drift.
        let section_count = prompt.matches("## ").count();
        assert!(
            section_count <= 5,
            "compact prompt has {} sections, exceeds 5-section limit",
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };

        let prompt = build_compact_system_prompt(&ctx);

        // No Personality section when soul is empty
        assert!(!prompt.contains("## Personality"));
        // Identity section still present
        assert!(prompt.contains("## Identity"));
        // Runtime section added (mika#1815) — carries ground-truth model
        // identity so the compact provider can answer "which model?" without
        // confabulating.
        assert!(prompt.contains("## Runtime"));
        // mika#1798: Data-Grade Doctrine (compact form) is now always rendered
        // after Identity/Runtime. Three sections total with empty soul:
        // ## Identity + ## Runtime + ## Data-Grade Doctrine.
        assert!(prompt.contains("## Data-Grade Doctrine"));
        // mika#1925 negative control, and the `3` is the assertion: the fixture
        // carries `stopped_topics: &[]`, so the nominal turn gains the `persist`
        // rule — which has no heading, by design — and NOT a fifth section.
        // If this count ever has to move for any reason other than a genuinely
        // new section, a section is being rendered that should not be: halt and
        // re-read the section-budget paragraph in
        // `test_build_compact_system_prompt_size_bound` rather than adjusting
        // the number to make the test pass.
        assert_eq!(prompt.matches("## ").count(), 3);
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
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

    /// The inversion of a carve-out pinning test (mika#1925).
    ///
    /// Its predecessor, `test_compact_prompt_omits_stopped_topics_block_by_design`,
    /// asserted the *absence* of the mika#1813 contract on this path and said
    /// so in as many words: "so a future wire-up is deliberate (test flips)
    /// rather than accidental". mika#1925 is that wire-up, and this is the
    /// flip. Read it as a filiation, not as an assertion born from nowhere:
    /// three sibling carve-outs (mika#1814, mika#2290, mika#2292) remain in
    /// force on this builder and keep their own pinning tests green, untouched.
    #[test]
    fn mika1925_compact_prompt_renders_the_stop_signal_contract() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_compact_system_prompt(&ctx);

        // AC1 — the consult half: section, preamble, block, injected content.
        assert!(
            prompt.contains("## Stopped Topics"),
            "compact prompt must carry the stop-topics section header"
        );
        assert!(
            prompt.contains("<stopped-topics>") && prompt.contains("</stopped-topics>"),
            "compact prompt must emit the stop-topics block"
        );
        assert!(
            prompt.contains("stop_topic_web-config"),
            "the injected preference category must reach the block"
        );
        assert!(
            prompt.contains("user asked to stop, 2026-08-19"),
            "the injected preference value must reach the block"
        );

        // The persist half (mika#1925 D3) — the rule the ticket does not ask
        // for and without which the block above could never fill.
        assert!(
            prompt.contains("store_fact(category='preference', key='stop_topic_<short-slug>'"),
            "compact prompt must carry the persist rule with the exact key shape"
        );

        // AC2 — stop != question, carried in BOTH halves. Counting rather than
        // asserting presence: a single occurrence would mean one half lost it,
        // and the two are read at different moments (see the constants' docs).
        assert_eq!(
            prompt.matches("not a re-opening").count(),
            2,
            "the stop-is-not-a-gag distinction must be carried in both halves"
        );
    }

    /// The test that carries mika#1925 D3 — the finding of that grooming.
    ///
    /// Without it, a later "simplification" putting the persist rule back under
    /// `!stopped_topics.is_empty()` would restore the inert path and nothing
    /// would redden: the conditional block would still render correctly *when
    /// fed*, and on a MikaModel tenant it would never be fed, because no other
    /// prompt on that path instructs the agent to write a `stop_topic_*`
    /// preference (`build_compact_system_prompt` renders no `## Instructions`,
    /// and `build_silent_prompt` carries the consult rule only).
    #[test]
    fn mika1925_compact_prompt_renders_persist_even_with_no_stopped_topics() {
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_compact_system_prompt(&ctx);

        assert!(
            prompt.contains("store_fact(category='preference', key='stop_topic_<short-slug>'"),
            "the persist rule is unconditional — see mika#1925 D3"
        );
        // The nominal case stays what it was, plus that one rule: no section is
        // added, no XML tag is opened.
        assert!(
            !prompt.contains("## Stopped Topics"),
            "no section when there is nothing to list"
        );
        assert!(
            !prompt.contains("<stopped-topics>"),
            "no block when there is nothing to list"
        );
    }

    /// AC1's size half, on the full shape (mika#1925).
    ///
    /// This is the only guard that sees the **sum** and the only one that sees
    /// the injected content: each `const _: () = assert!(…)` bounds one
    /// constant in isolation, and no const-assert of the total can exist (it
    /// depends on `soul.md` and on preference values, unknown at compile time).
    /// An edit inflating the consult preamble passes the persist const-assert
    /// without learning anything, and lands here.
    #[test]
    fn mika1925_compact_prompt_stays_within_budget_with_stopped_topics() {
        let identity = test_identity();
        let long = "user asked to stop, 2026-08-19 — ".repeat(20);
        let stops = vec![
            stop_topic_pref("web-config", &long),
            stop_topic_pref("budget-review", &long),
            stop_topic_pref("holiday-planning", &long),
        ];
        let ctx = PromptContext {
            soul_content: "You are a sharp, proactive executive assistant.",
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_compact_system_prompt(&ctx);

        assert!(
            prompt.len() <= 5120,
            "compact prompt is {} bytes with three stopped topics, exceeds 5 KB",
            prompt.len()
        );
        // `sanitize_label` caps each injected field at 200 chars — the bound
        // above holds because of it, not by luck.
        assert!(
            !prompt.contains(&long),
            "an over-long preference value must be truncated by sanitize_label"
        );
    }

    // === mika#1815 — Self-identity / anti-confabulation tests ===
    //
    // The three tests below pin the shape of the Runtime + Self-Identity
    // Discipline sections that fix the "Mika confabule son propre modèle"
    // failure (Al testeur, 2026-07-20). Each is written to fail if the
    // ground-truth channel drifts from what the tool + prompt promise.

    // -- mika#2290: the hosting fact is posed in `## Runtime` --

    /// Build a conversation-mode context for the mika#2290 prompt assertions.
    /// Both axes are parameters: the whole point of Décision 4 is that the two
    /// vary independently.
    fn hosting_ctx<'a>(
        identity: &'a Identity,
        memory: &'a [CoreMemoryEntry],
        deployment: Deployment,
        persona_profile: PersonaProfile,
    ) -> PromptContext<'a> {
        PromptContext {
            soul_content: "",
            identity,
            core_memory: memory,
            is_onboarding: false,
            current_utc: test_time(),
            timezone: None,
            global_home_dir: None,
            channel_type: None,
            telegram_configured: false,
            home_dir: None,
            callback_context: None,
            stopped_topics: &[],
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment,
            persona_profile,
        }
    }

    /// AC2 — the three states render three **distinct** lines in `## Runtime`,
    /// on both prompt builders that carry the full block.
    ///
    /// Asserting distinctness rather than three literals is what makes the test
    /// resistant to the failure it exists to catch: a `match` that fell through
    /// to one arm would still contain every substring a per-state `contains`
    /// assertion looked for.
    #[test]
    fn mika2290_runtime_section_renders_one_line_per_deployment_state() {
        let identity = test_identity();
        let memory = test_core_memory();

        for persona in [PersonaProfile::Operator, PersonaProfile::Family] {
            let mut rendered = Vec::new();
            for deployment in [Deployment::Local, Deployment::Cloud, Deployment::Unknown] {
                let ctx = hosting_ctx(&identity, &memory, deployment, persona);
                let prompt = build_system_prompt(&ctx);
                // mika#2292 — anchor on the heading **at line start**, not on
                // the first occurrence of the string. `## Mika Doctrine` refers
                // the hosting question to `## Runtime` by name (inline, in
                // backticks) and renders *before* it, so a bare `find` lands in
                // that body and extracts the same block for all three
                // deployments — this assertion would go red while the code under
                // test is correct. A heading is a line, not a substring.
                let runtime_pos = prompt
                    .find("\n## Runtime\n")
                    .map(|i| i + 1)
                    .expect("Runtime section must be present");
                let next_heading = prompt[runtime_pos + 2..]
                    .find("\n## ")
                    .map(|i| runtime_pos + 2 + i)
                    .unwrap_or(prompt.len());
                rendered.push(prompt[runtime_pos..next_heading].to_string());
            }
            assert_eq!(rendered.len(), 3);
            for (a, b) in [(0, 1), (0, 2), (1, 2)] {
                assert_ne!(
                    rendered[a], rendered[b],
                    "{persona:?}: two deployment states rendered the same \
                     `## Runtime` block — the hosting match fell through"
                );
            }
        }
    }

    /// AC2 — `Unknown` says it does not know, and `Local` is not its fallback.
    /// The negative control rides in the same test, per the discipline this
    /// family inherited from mika#2023.
    #[test]
    fn mika2290_unknown_state_asserts_nothing_and_never_defaults_to_local() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = hosting_ctx(
            &identity,
            &memory,
            Deployment::Unknown,
            PersonaProfile::Operator,
        );
        let prompt = build_system_prompt(&ctx);

        assert!(
            prompt.contains("not declared in this environment"),
            "the Unknown line must say the hosting mode is undeclared"
        );
        assert!(
            prompt.contains("do not default to \"local\""),
            "the Unknown line must forbid falling back on \"local\""
        );
        // Control: the declared-local wording must NOT be present.
        assert!(
            !prompt.contains("on the user's own machine"),
            "an undeclared deployment must not carry the local-install wording"
        );
    }

    /// AC2 — `## Self-Identity Discipline` names hosting as ground truth.
    /// Without this, rule 3 (*fallback honestly*) was already written word for
    /// word for the `Unknown` case and simply did not apply to the question —
    /// which is the hole mika#2290 M3 measured to the byte.
    #[test]
    fn mika2290_self_identity_discipline_covers_where_you_run() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = hosting_ctx(
            &identity,
            &memory,
            Deployment::Cloud,
            PersonaProfile::Operator,
        );
        let prompt = build_system_prompt(&ctx);

        assert!(
            prompt.contains("WHERE you run and where their data lives"),
            "the discipline preamble must include the hosting question in its scope"
        );
        assert!(
            prompt.contains("Where you run is ground truth too"),
            "rule 5 must be present"
        );
        assert!(
            prompt.contains("\"Local\" is not a safe default"),
            "rule 5 must forbid the \"local\" default explicitly"
        );
    }

    /// AC5 — the `Cloud` line served under the family persona tells the truth
    /// **without** any of the vocabulary `FAMILY_SOUL` forbids ("aucun jargon
    /// technique ni mention de tickets, GitHub, agents …, skills, ou de
    /// l'infrastructure sous-jacente — jamais, même si on te le demande").
    ///
    /// The forbidden list is checked in both languages: the persona answers in
    /// French but the prompt is written in English, so a FR-only check would
    /// pass over an English "container" without noticing.
    #[test]
    fn mika2290_family_cloud_line_carries_no_infrastructure_jargon() {
        let line = hosting_ground_truth_line(Deployment::Cloud, PersonaProfile::Family);
        let lowered = line.to_lowercase();

        for forbidden in [
            "container",
            "conteneur",
            "tenant",
            "github",
            "ticket",
            "skill",
            "agent",
            "sqlite",
            "open-source",
            "mit",
            "stack",
            "self-host",
            "infrastructure",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "the family Cloud line must not carry `{forbidden}`: {line}"
            );
        }

        // Control: it still says the true thing, rather than saying nothing.
        assert!(
            lowered.contains("server") && lowered.contains("belongs to them"),
            "the family Cloud line must still carry the verifiable truth: {line}"
        );

        // Control: the operator line, by contrast, IS allowed the vocabulary —
        // otherwise this test would pass on a line that simply said nothing.
        let operator =
            hosting_ground_truth_line(Deployment::Cloud, PersonaProfile::Operator).to_lowercase();
        assert!(
            operator.contains("container") && operator.contains("self-hosted"),
            "the operator Cloud line must carry the full prescribed remedy"
        );
    }

    /// AC2 — the compact (MikaModel) path deliberately omits the hosting line.
    /// Pinned as a **decision**: the ≤5 KB budget cannot afford it, and the 5d
    /// guard covers that path anyway because it reads outgoing text, not the
    /// prompt. Joined to the mika#1925 follow-up.
    #[test]
    fn mika2290_compact_prompt_omits_the_hosting_line() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = hosting_ctx(
            &identity,
            &memory,
            Deployment::Cloud,
            PersonaProfile::Operator,
        );
        let compact = build_compact_system_prompt(&ctx);

        assert!(
            compact.contains("## Runtime"),
            "control: the compact prompt still carries the model ground truth"
        );
        assert!(
            !compact.contains("isolated per-tenant container"),
            "the compact carve-out must not render the hosting line"
        );
        assert!(
            compact.len() <= 5 * 1024,
            "the carve-out exists for the ≤5 KB budget; compact prompt is {} bytes",
            compact.len()
        );
    }

    /// AC2 — silent turns carry the hosting line too. A heartbeat that asserts
    /// local hosting on a cloud tenant is exactly as false, and the compacted
    /// history hands it to the next conversational turn.
    #[test]
    fn mika2290_silent_prompt_carries_the_hosting_line() {
        let identity = test_identity();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "heartbeat",
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Cloud,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_silent_prompt(&ctx);

        assert!(
            prompt.contains("isolated per-tenant container"),
            "the silent prompt must render the hosting line"
        );
        assert!(
            prompt.contains("Where you run is ground truth too"),
            "the silent prompt must render rule 5"
        );
    }

    // -----------------------------------------------------------------------
    // mika#2292 — the material doctrine, aliased "doctrine"
    //
    // **Scope of V3–V7, and it is a decision rather than a convenience.** These
    // five scans read **the two constants this ticket creates**, never the file
    // and never the assembled prompt. Measured on `main` at 2026-09-18:
    // `grep -rn "exportable" crates/mika-agent/src/ docs/architecture.md` returns
    // **five** sites — one of them, the `(Operator, Cloud)` arm of
    // `hosting_ground_truth_line`, a constant **served to the model** — and
    // `grep -i "Prime\|hermetic"` over `prompt.rs` + `home.rs` returns twelve, of
    // which none is served. A file-scoped V7 or V3 would therefore go red on its
    // first run, against correct code: the road that wears a detector out until
    // somebody disarms it. Each tempting widening is named below with the
    // pre-existing violation it would light up.
    // -----------------------------------------------------------------------

    /// The referent list lives here and **only** here, under `#[cfg(test)]`, per
    /// the form the fire-disposition doctrine prescribes for a structural list
    /// ("scope it inside `#[cfg(test)] mod tests` so the production loader
    /// cannot consult it at runtime"). It is not compiled in release, not
    /// served, and not readable by the tenant — which is the whole point of
    /// D3: a prompt that enumerates the secret in order to forbid it is a leak
    /// with one extra step.
    ///
    /// **It holds the referents of AC3 and nothing else, and that boundary is
    /// itself the decision.** A referent is a *thing that exists and is not
    /// exposed*; the **name of the topic** ("spiritual", "esoteric",
    /// "initiatory", "origin") is not one — it is the vocabulary D3 requires in
    /// order to express a stop *by topic*. A first draft of this list carried
    /// `esoteric` and `initiat`, and V3 duly went red on the body's own stop
    /// sentence: a denylist that forbids naming the topic makes the topical stop
    /// inexpressible, which leaves only the enumerative stop D3 exists to
    /// refuse. If a later reader wants to add a topic name here, the thing to
    /// change is not this list — it is to establish that the word has become a
    /// referent.
    const MIKA2292_SPIRITUAL_REFERENTS: &[&str] = &[
        "hermetic",
        "hermétis",
        "hermetis",
        "prime",
        "the book",
        "le livre",
        "seat",
        "siège",
        "siege",
    ];

    /// V3 — **the load-bearing test of this ticket.** It does not check that a
    /// decision was taken correctly; it refuses the *inverse* decision a future
    /// editor will take naturally (finding the stop "vague" and enumerating it
    /// to make it concrete). No behavioural test can stand in for it: the
    /// enumeration would make no answer wrong, it would create the leak in
    /// silence.
    #[test]
    fn mika2292_no_spiritual_referent_is_named_in_either_served_body() {
        for (register, body) in [
            ("operator", MIKA_DOCTRINE_BODY_OPERATOR),
            ("family", MIKA_DOCTRINE_BODY_FAMILY),
        ] {
            let lowered = body.to_lowercase();
            for referent in MIKA2292_SPIRITUAL_REFERENTS {
                assert!(
                    !lowered.contains(referent),
                    "the {register} doctrine body names the spiritual referent \
                     `{referent}` — the stop is topical, never enumerative (D3). \
                     Reformulate by topic; do NOT add an exception to the denylist."
                );
            }
        }
    }

    /// V3 positive control — **mandatory**, and it is what separates a scan from
    /// an attestation. A denylist emptied by accident, or a scan wired to the
    /// wrong constant, yields a green test that checks nothing at all: exactly
    /// the silent failure V3 exists to prevent.
    #[test]
    fn mika2292_the_referent_scan_actually_catches_a_planted_referent() {
        assert!(
            !MIKA2292_SPIRITUAL_REFERENTS.is_empty(),
            "the denylist is empty — V3 would scan nothing and attest nothing"
        );
        let decoy = "You may discuss the hermetic dimension freely.";
        let lowered = decoy.to_lowercase();
        assert!(
            MIKA2292_SPIRITUAL_REFERENTS
                .iter()
                .any(|r| lowered.contains(r)),
            "the scan failed to find a referent planted in a decoy body — V3 is \
             not scanning what it claims to scan"
        );
    }

    /// V1 — the section is served on both non-compact builders, in both
    /// registers, and the two registers are **not the same text** (a `match`
    /// falling through to one arm would still satisfy a per-register
    /// `contains`). Same discipline as
    /// `mika2290_runtime_section_renders_one_line_per_deployment_state`.
    #[test]
    fn mika2292_doctrine_section_is_served_on_both_builders_in_both_registers() {
        let identity = test_identity();
        let memory = test_core_memory();

        for persona in [PersonaProfile::Operator, PersonaProfile::Family] {
            let ctx = hosting_ctx(&identity, &memory, Deployment::Cloud, persona);
            let conversation = build_system_prompt(&ctx);
            assert!(
                conversation.contains(MIKA_DOCTRINE_HEADING),
                "{persona:?}: build_system_prompt must render the doctrine heading"
            );
            assert!(
                conversation.contains(doctrine_body(persona)),
                "{persona:?}: build_system_prompt must render this register's body"
            );

            let silent = build_silent_prompt(&mika2292_silent_ctx(&identity, persona));
            assert!(
                silent.contains(MIKA_DOCTRINE_HEADING),
                "{persona:?}: build_silent_prompt must render the doctrine heading — \
                 a silent turn that wrote a memory fact on the spiritual register \
                 would poison every later turn through core memory (D8)"
            );
            assert!(
                silent.contains(doctrine_body(persona)),
                "{persona:?}: build_silent_prompt must render this register's body"
            );
        }

        assert_ne!(
            MIKA_DOCTRINE_BODY_OPERATOR, MIKA_DOCTRINE_BODY_FAMILY,
            "the two registers rendered the same body — the persona match fell through"
        );
    }

    /// V2 — the compact (MikaModel) path deliberately omits the section. Pinned
    /// as a **decision**, joined to mika#1925 with the three sibling carve-outs.
    /// Its cost is real here and is stated at the point of omission: on that path
    /// the measured defect stays open.
    #[test]
    fn mika2292_compact_prompt_omits_the_doctrine_section() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = hosting_ctx(
            &identity,
            &memory,
            Deployment::Cloud,
            PersonaProfile::Operator,
        );
        let compact = build_compact_system_prompt(&ctx);

        assert!(
            !compact.contains(MIKA_DOCTRINE_HEADING),
            "the compact carve-out must not render `{MIKA_DOCTRINE_HEADING}`"
        );
        // Control: the carve-out is per-section, not a broken builder — the
        // compact path DOES carry the mika#1798 abbreviated data-grade doctrine,
        // which paid its bytes because it guards an irreversible HARD-NO.
        assert!(
            compact.contains("## Identity"),
            "control: the compact builder still renders its other sections"
        );
        assert!(
            compact.len() <= 5 * 1024,
            "the carve-out exists for the ≤5 KB budget; compact prompt is {} bytes",
            compact.len()
        );
    }

    /// V4 — neither body can trip the mika#2290 guard 5d. True **by
    /// construction**: both bodies carry no locality subject at all (the hosting
    /// fact is referred to `## Runtime` rather than restated), so the predicate
    /// has nothing to pair an assertion with — rather than true by the presence
    /// of a conditional marker a later editor could reword away.
    ///
    /// Deliberately NOT widened to every prompt constant: that would go red on
    /// `hosting_ground_truth_line(Local, Operator)`, which says "on the user's
    /// own machine" — true, served, and never reachable under `Cloud` because
    /// the match is exhaustive. An apparent violation, not a real one.
    #[test]
    fn mika2292_neither_body_trips_the_false_local_hosting_guard() {
        for deployment in [Deployment::Cloud, Deployment::Unknown] {
            for (register, body) in [
                ("operator", MIKA_DOCTRINE_BODY_OPERATOR),
                ("family", MIKA_DOCTRINE_BODY_FAMILY),
            ] {
                assert!(
                    crate::evidence::guards::detect_false_local_hosting_claim(body, deployment)
                        .is_none(),
                    "the {register} doctrine body trips guard 5d under {deployment:?} — \
                     reformulate the body; do NOT widen CLAIM_CONDITIONAL_MARKERS, whose \
                     narrowness is the written motive of mika#2290"
                );
            }
        }
    }

    /// V5 — the family body carries no infrastructure vocabulary. Same list and
    /// same bilingual reasoning as
    /// `mika2290_family_cloud_line_carries_no_infrastructure_jargon`: the persona
    /// answers in French but the prompt is written in English, so a FR-only check
    /// would pass over an English "repository" without noticing.
    ///
    /// Deliberately NOT widened to the family arm of `## Runtime`: that would go
    /// red on "server", which that arm's own doc-comment admits by name as the
    /// one concrete noun it keeps.
    #[test]
    fn mika2292_family_body_carries_no_infrastructure_jargon() {
        let lowered = MIKA_DOCTRINE_BODY_FAMILY.to_lowercase();
        for forbidden in [
            "open source",
            "open-source",
            "mit",
            "licence",
            "license",
            "repository",
            "dépôt",
            "depot",
            "self-host",
            "auto-héberg",
            "server",
            "serveur",
            "container",
            "conteneur",
            "tenant",
            "github",
            "ticket",
            "skill",
            "agent",
            "sqlite",
            "stack",
            "infrastructure",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "the family doctrine body carries `{forbidden}` — FAMILY_SOUL forbids \
                 « tout jargon technique … ou de l'infrastructure sous-jacente — jamais, \
                 même si on te le demande » (D2)"
            );
        }

        // Control: it still says the substance, rather than saying nothing.
        assert!(
            lowered.contains("belongs to the person") && lowered.contains("doctrine"),
            "the family body must still carry the substance and its alias"
        );
        // Control: the operator body, by contrast, IS allowed the vocabulary —
        // without this the test would pass on a body that said nothing at all.
        let operator = MIKA_DOCTRINE_BODY_OPERATOR.to_lowercase();
        assert!(
            operator.contains("open source") && operator.contains("mit licence"),
            "the operator body must carry the full formulation, MIT included"
        );
    }

    /// V6 — the family body establishes **no creator referent** (D4). mika#1783
    /// removed from `FAMILY_SOUL`, on doctrinal grounds, any origin story giving
    /// the being a referent it could then address (founding incident "Salut
    /// Vincent"; closure `the-being-does-not-have-a-maker-it-knows-about`,
    /// guarded by `home::tests::family_soul_no_operator_name`). Prime's bearing
    /// prescribes "it is the creator's choice" — carried by the *operator*
    /// register, whose control rides in this same test so the asymmetry cannot
    /// be read as an oversight.
    #[test]
    fn mika2292_family_body_establishes_no_creator_referent() {
        let lowered = MIKA_DOCTRINE_BODY_FAMILY.to_lowercase();
        for referent in [
            "creator",
            "créateur",
            "createur",
            "maker",
            "vincent",
            "made by",
            "built by",
            "created by",
            "conçu par",
            "origin",
        ] {
            assert!(
                !lowered.contains(referent),
                "the family doctrine body establishes the creator referent `{referent}` — \
                 this is the single site to change if Vincent or Prime decides otherwise (D4)"
            );
        }

        // Control: the operator register DOES carry the bearing's formulation.
        assert!(
            MIKA_DOCTRINE_BODY_OPERATOR
                .to_lowercase()
                .contains("choice of mika's creator"),
            "the operator body must carry the bearing's formulation"
        );
    }

    /// V7 — "exportable" is claimed in neither body (F5). The ticket forbids
    /// claiming it unverified; this repository contains no export tool, route or
    /// subcommand, and the tenant's data lives in `mika-cloud`, which is not in
    /// this workspace — so the claim can be neither verified nor safely retracted
    /// from here. This ticket therefore adds no sixth site.
    #[test]
    fn mika2292_no_exportable_claim_in_either_body() {
        for (register, body) in [
            ("operator", MIKA_DOCTRINE_BODY_OPERATOR),
            ("family", MIKA_DOCTRINE_BODY_FAMILY),
        ] {
            assert!(
                !body.to_lowercase().contains("exportable"),
                "the {register} doctrine body claims `exportable`, which nothing in \
                 this repository can verify (F5)"
            );
        }
    }

    /// V7's **named exception, with its self-cleaning assertion** — the option
    /// (a) disposition the fire-disposition doctrine requires when a detector's
    /// population is not empty. The one pre-existing site that is *served to the
    /// model* is named by its function and its crossing, never by a line number.
    ///
    /// This assertion goes red the day the follow-up lands, which is exactly the
    /// property the doctrine asks for — and it is stable, where an occurrence
    /// count would drift on any added doc-comment.
    #[test]
    fn mika2292_the_named_exportable_exception_is_still_there() {
        let served = hosting_ground_truth_line(Deployment::Cloud, PersonaProfile::Operator);
        assert!(
            served.to_lowercase().contains("exportable"),
            "F5 has been dealt with upstream: remove this named exception and widen \
             V7 to the five sites (see § Hors périmètre n°1 of the mika#2292 plan)"
        );
    }

    /// V8 — `## Mika Doctrine` precedes `## Self-Identity Discipline`, whose
    /// rule 6 cites it. **Order, not adjacency**: the section is deliberately
    /// docked to `## Distribution Doctrine` rather than to `## Runtime` (D10),
    /// so it is not the discipline's immediate neighbour, and the citation names
    /// it by heading precisely so distance costs nothing.
    #[test]
    fn mika2292_doctrine_section_precedes_the_discipline_that_cites_it() {
        let identity = test_identity();
        let memory = test_core_memory();
        for persona in [PersonaProfile::Operator, PersonaProfile::Family] {
            let ctx = hosting_ctx(&identity, &memory, Deployment::Cloud, persona);
            let prompt = build_system_prompt(&ctx);

            let doctrine_pos = prompt
                .find(MIKA_DOCTRINE_HEADING)
                .expect("the doctrine section must be present");
            let distribution_pos = prompt
                .find(DISTRIBUTION_DOCTRINE_HEADING)
                .expect("the distribution doctrine section must be present");
            let discipline_pos = prompt
                .find("## Self-Identity Discipline")
                .expect("the discipline section must be present");

            assert!(
                distribution_pos < doctrine_pos,
                "{persona:?}: the two doctrines must read together, the cited one first"
            );
            assert!(
                doctrine_pos < discipline_pos,
                "{persona:?}: the doctrine section must precede the rule that cites it"
            );
        }
    }

    /// V9 + V13a — rule 6 exists, carries the forbidden answer **literally**, and
    /// stays **coupled** to the section it designates.
    ///
    /// V9 alone could not give this: it asserts a string is present, so a rename
    /// of the section would leave rule 6 green while pointing at a section that
    /// no longer exists — the silently-broken coupling `grooming_marker`
    /// (mika#2158) had to close on another surface. The structural half is the
    /// companion source scan below; this half asserts the two render *together*.
    #[test]
    fn mika2292_rule_six_names_the_section_and_forbids_the_measured_answer() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = hosting_ctx(
            &identity,
            &memory,
            Deployment::Cloud,
            PersonaProfile::Operator,
        );
        let prompt = build_system_prompt(&ctx);

        let discipline_pos = prompt
            .find("## Self-Identity Discipline")
            .expect("the discipline section must be present");
        let discipline = &prompt[discipline_pos..];

        assert!(
            discipline.contains("What Mika is, what she stands for and why"),
            "rule 6 must be present in the discipline section"
        );
        assert!(
            discipline.contains(MIKA_DOCTRINE_HEADING),
            "rule 6 must name the doctrine section by its heading — a rename must \
             not be able to leave it pointing into the void"
        );
        assert!(
            discipline.contains("found nothing of that name is NOT acceptable"),
            "rule 6 must forbid the measured answer by name, not by paraphrase"
        );
        assert!(
            discipline.contains("is not a memory lookup"),
            "rule 6 must carry its second clause: the answer comes from this prompt, \
             and an empty search result is not evidence of absence"
        );
        // Both halves render together, in the same prompt: the fact and the rule
        // that points at it.
        assert!(
            prompt.contains(MIKA_DOCTRINE_BODY_OPERATOR),
            "the fact half must render alongside the rule half"
        );
    }

    /// V13a structural half — rule 6 is **composed by interpolating**
    /// `MIKA_DOCTRINE_HEADING`, never by a copied literal. Source scan, because
    /// this is the class no behavioural test can see: a copied literal makes no
    /// answer wrong today and breaks the coupling only on the day of a rename,
    /// when every assertion above would still be green.
    #[test]
    fn mika2292_the_heading_literal_has_exactly_one_site_in_production_code() {
        // The boundary is read by `mika_common::source_guard` (mika#2398).
        // `split_once("\nmod tests {")` errs the other way from a truncating
        // guard: it leaves a module-level `#[cfg(test)]` helper *inside* the
        // production half, so a heading literal written in such a helper would
        // have been counted as a production site.
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let production = scanner.production_of(&scanner.src_root().join("prompt.rs"));

        let occurrences = production
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//")
            })
            .filter(|line| line.contains(MIKA_DOCTRINE_HEADING))
            .count();

        assert_eq!(
            occurrences, 1,
            "the heading literal must appear exactly once in production code (its \
             declaration). A second occurrence means a consumer — rule 6 above all — \
             copied it instead of interpolating it, which is the coupling that breaks \
             silently on a rename."
        );
    }

    /// Build a silent-mode context for the mika#2292 assertions. Kept local to
    /// this block rather than widening `hosting_ctx`, whose callers are the
    /// mika#2290 series.
    fn mika2292_silent_ctx<'a>(
        identity: &'a Identity,
        persona: PersonaProfile,
    ) -> SilentPromptContext<'a> {
        SilentPromptContext {
            soul_content: "",
            identity,
            core_memory: &[],
            pending_commitments: &[],
            trigger_context: "heartbeat",
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
            runtime_provider: "test-provider",
            runtime_model: "test-model",
            deployment: Deployment::Cloud,
            persona_profile: persona,
        }
    }

    #[test]
    fn mika1815_runtime_section_carries_ground_truth_from_context() {
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
            runtime_provider: "zai",
            runtime_model: "glm-5.2",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_system_prompt(&ctx);
        assert!(
            prompt.contains("## Runtime"),
            "Runtime section header must be present"
        );
        assert!(
            prompt.contains("provider `zai`"),
            "Runtime section must quote the provider verbatim, got:\n{prompt}"
        );
        assert!(
            prompt.contains("model `glm-5.2`"),
            "Runtime section must quote the model verbatim, got:\n{prompt}"
        );
        // The rule text is present (structural, not paraphrase-tolerant).
        assert!(
            prompt.contains("ground truth for questions about your own LLM/model"),
            "Runtime section must state the ground-truth contract"
        );
        assert!(
            prompt.contains("Do NOT infer your model from commented-out config lines"),
            "Runtime section must forbid inference from commented-out config"
        );
    }

    #[test]
    fn mika1815_self_identity_discipline_section_present_and_ordered_after_runtime() {
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
            runtime_provider: "anthropic",
            runtime_model: "claude-sonnet-4-6",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_system_prompt(&ctx);
        // mika#2292 — anchor on the heading at line start; see the sibling note
        // in `mika2290_runtime_section_renders_one_line_per_deployment_state`.
        // This assertion still held with a bare `find` (the inline mention also
        // precedes the discipline), but it was measuring the wrong position.
        let runtime_pos = prompt
            .find("\n## Runtime\n")
            .map(|i| i + 1)
            .expect("Runtime section must be present");
        let discipline_pos = prompt
            .find("## Self-Identity Discipline")
            .expect("Self-Identity Discipline section must be present");
        assert!(
            runtime_pos < discipline_pos,
            "Runtime must precede Self-Identity Discipline (rules quote Runtime data)"
        );
        // All four rule anchors present verbatim.
        assert!(
            prompt.contains("Quote, don't infer"),
            "Rule 1 (Quote don't infer) must be present"
        );
        assert!(
            prompt.contains("Verb discipline"),
            "Rule 2 (Verb discipline VERIFY vs INFER) must be present"
        );
        assert!(
            prompt.contains("Fallback honestly"),
            "Rule 3 (Fallback honestly) must be present"
        );
        assert!(
            prompt.contains("Consistency across a single turn"),
            "Rule 4 (Consistency across a single turn) must be present"
        );
        // Anchor on the contrast reference so the section keeps its
        // documentary link to the founding-incident cross-ticket.
        assert!(
            prompt.contains("mika#1784"),
            "Section must reference mika#1784 as the contrast anchor"
        );
    }

    #[test]
    fn mika1815_compact_prompt_carries_runtime_line() {
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
            runtime_provider: "mikamodel",
            runtime_model: "wizzard-v1",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_compact_system_prompt(&ctx);
        assert!(
            prompt.contains("## Runtime"),
            "Compact prompt must include the Runtime header"
        );
        assert!(
            prompt.contains("`mikamodel`"),
            "Compact prompt must quote the provider verbatim, got:\n{prompt}"
        );
        assert!(
            prompt.contains("`wizzard-v1`"),
            "Compact prompt must quote the model verbatim, got:\n{prompt}"
        );
        // Compact-budget carve-out: the discipline block is intentionally
        // omitted. Regression guard.
        assert!(
            !prompt.contains("Self-Identity Discipline"),
            "Compact prompt must NOT include the full Self-Identity Discipline block \
             (budget carve-out, mika#1815)"
        );
    }

    #[test]
    fn mika1815_silent_prompt_carries_runtime_and_discipline() {
        let identity = test_identity();
        let memory = test_core_memory();
        let ctx = SilentPromptContext {
            soul_content: "",
            identity: &identity,
            core_memory: &memory,
            pending_commitments: &[],
            trigger_context: "heartbeat",
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
            runtime_provider: "zai",
            runtime_model: "glm-5.2",
            deployment: Deployment::Unknown,
            persona_profile: PersonaProfile::Operator,
        };
        let prompt = build_silent_prompt(&ctx);
        // Silent mode carries the same ground-truth block as conversation
        // mode — self-identity honesty must be uniform.
        assert!(
            prompt.contains("## Runtime"),
            "Silent prompt must include the Runtime header"
        );
        assert!(
            prompt.contains("provider `zai`") && prompt.contains("model `glm-5.2`"),
            "Silent prompt Runtime section must carry ground truth verbatim, got:\n{prompt}"
        );
        assert!(
            prompt.contains("## Self-Identity Discipline"),
            "Silent prompt must include the Self-Identity Discipline block"
        );
    }
}
