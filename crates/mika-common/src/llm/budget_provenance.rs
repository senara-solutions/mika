//! Where each half of the `(per-call plafond, agent envelope)` pair actually
//! came from — and the `llm_budget_resolved` event that says so (mika#2293).
//!
//! # The failure this exists to close
//!
//! mika#2189 gave mika-arch the pair `240/900` in its per-agent `config.toml`
//! on 2026-09-06. Five days later, on 2026-09-11, the measurement that opened
//! mika#2293 still saw generations cut at **120 s exactly** — the fleet
//! default. Something annulled the setting, and *no line of log anywhere could
//! say what*: nothing in this codebase exposed the pair a given agent was
//! actually running under.
//!
//! That hole is why a setting shipped on the 6th could fail in silence until
//! the 11th, and it is the hole that would have swallowed the next setting the
//! same way. **A setting you cannot observe is not a setting, it is a hope.**
//!
//! # The three worlds one line of log separates
//!
//! The provenance of the plafond discriminates three hypotheses that call for
//! three disjoint remedies — which is the whole reason this module reports a
//! *source* and not just a value:
//!
//! - [`BudgetSource::AgentConfig`] at 240 → **H1**: the setting *is* in force
//!   and the cause of the 120 s cuts is elsewhere. Do not touch the values.
//! - [`BudgetSource::ProcessEnv`] → **H2**: a fleet-wide `MIKA_*` variable is
//!   shadowing the per-agent `config.toml`. The remedy is to remove it from the
//!   service environment — not to raise anything.
//! - [`BudgetSource::Default`] → **H3**: the `config.toml` was never read or
//!   never carried the key (provisioning frozen behind
//!   `MIKA_DISABLE_AGENT_PROVISIONING` or `dev_mode`). The remedy is a
//!   provisioning gesture.
//!
//! None of the three is deducible from a plafond someone raised.
//!
//! # Why the cascade is rebuilt here rather than recorded in `load_for_agent`
//!
//! `Settings` has already merged its sources by the time anyone reads a field,
//! so provenance has to be established some other way. Making the cascade carry
//! it would mean instrumenting config-rs for **every** key in order to serve
//! two, and would put a provenance field on the load path of every agent and
//! every binary — a permanent debt paid by callers that will never read it. A
//! dedicated reader of two keys, by contrast, is deleted the day it stops being
//! useful.
//!
//! The assumed cost is a duplication of the cascade order, and it is **pinned
//! rather than hoped for**: [`tests::mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`]
//! asserts that what this module reconstructs equals what
//! `Settings::load_for_agent` merges, on each of the four cascade positions. The
//! day that order changes in `config.rs`, that test goes red instead of letting
//! the provenance drift in silence — because **a false provenance is strictly
//! worse than no provenance**: it answers the one question this module exists
//! to settle, with authority, and wrongly.
//!
//! # The cascade order is inverted, and reproducing it exactly is the point
//!
//! Since mika#2218 the per-agent `.env` sits **above** the process environment,
//! not below it. The real order, highest priority first, is:
//!
//! 1. `{agent_home}/.env`     ([`BudgetSource::AgentDotenv`])
//! 2. `MIKA_*` process env    ([`BudgetSource::ProcessEnv`])
//! 3. `{agent_home}/config.toml`  ([`BudgetSource::AgentConfig`])
//! 4. `{global_home}/config.toml` ([`BudgetSource::GlobalConfig`])
//! 5. the compiled-in constant    ([`BudgetSource::Default`])
//!
//! Positions 1 and 3 exist only when `global_home != agent_home` — exactly the
//! condition `load_for_agent` itself branches on.
//!
//! **This is the third walk of that cascade in the workspace, and the count is
//! written down so the next reader does not have to discover it.** The other
//! two are `Settings::load_for_agent` itself (via config-rs) and
//! `mika-cli`'s `commands::config::resolve_source`, added by mika#2218 to make
//! the inverted order legible where `mika config get` reads it. The three differ
//! in what they return — merged value, source label, and source + raw + parsed
//! — so none is a drop-in for another, but the *order* is one fact in three
//! places. If a fourth appears, the right move is a shared primitive in this
//! crate rather than a fourth copy; what keeps this one honest meanwhile is the
//! equality test below, which the CLI's walk has no counterpart for.
//!
//! # `default` is not ambiguous here, and that is deliberate (M11)
//!
//! `Settings::effective_llm_http_timeout_secs` falls back, when its field is
//! `None`, to [`super::http_timeout_secs`] — a *second*, out-of-cascade read of
//! the process environment. A naive reporter would therefore have had to answer
//! `default` for two different worlds: "compiled-in constant" and "process env
//! re-read through another door" — the two answers this module exists to
//! separate.
//!
//! Rebuilding the cascade explicitly closes that by construction: this reader
//! consults the process environment **itself**, so a value living there is
//! reported as [`BudgetSource::ProcessEnv`] and never reaches the `Default`
//! arm. Pinned by [`tests::mika2293_process_env_is_never_reported_as_default`].
//! Each resolution also carries its [`ResolvedBudgetValue::raw`] string, so the
//! reading can be checked rather than trusted.
//!
//! # The other half of "why was this turn cut?" — the model (mika#2328)
//!
//! mika#2293 answers *which timeout pair* an agent runs under. It did not answer
//! *which model*, and mika#2328 measured why that matters: `zai_model =
//! "glm-5.2"` is the only value `well_known_agents.rs` has ever declared for
//! mika-qa, while the incident of 2026-09-15 was produced by a **glm-5.3** in
//! service — an out-of-repo edit of the agent's `config.toml`, which
//! `reconcile_well_known_config` preserves as long as provisioning is frozen.
//! The repo said one thing, the runtime ran another, indefinitely, and nothing
//! anywhere said so.
//!
//! `turn_usage` does carry `provider` and `model`, but per turn, inside 19 GB of
//! log, with no provenance. What was missing is a *configuration* event, and the
//! emission site already existed — hence [`ModelProvenance`], resolved through
//! the **same** [`CascadeLayers`] as the two budget keys and emitted on the same
//! line. The two facts are read together (a cut turn is diagnosed with the model
//! *and* the envelope), which is also why the deduplication signature covers
//! both: the mika#2362 motif, where a change moving only one field must be
//! re-emitted rather than swallowed.
//!
//! **The model key's NAME depends on the provider** (`zai_model`,
//! `openrouter_model`, `anthropic_model`, …), so it is derived from the provider
//! in force rather than hard-coded. A fixed key would report `default` for an
//! agent whose model is perfectly well declared — a *false* provenance, which
//! this module holds to be strictly worse than none.
//!
//! # The guard mika#2328 said was missing
//!
//! Everything above reports what the **runtime** resolves. It does not, on its
//! own, say whether that is what the *repo* declares — and the 2026-09-15
//! incident was exactly that gap: `llm_budget_resolved` would have printed
//! `glm-5.3`, truthfully, forever, and no reader had the other half to compare
//! it with. mika#2473 adds it, in this file, deliberately: [`declared_model`]
//! reads the constant through the **same** [`model_config_key`] derivation and
//! the same default rule as the cascade above, so the two sides cannot drift
//! apart in the reader that is supposed to detect drift (KTD6).
//! [`ModelDriftCheck::compare`] is pure; [`emit_model_drift`] is the one
//! function of that half with an effect, and it reports — it never refuses
//! (KTD1). The freshness half, "has the file changed since this process read
//! it?", is [`super::config_freshness`].

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use super::ProviderKind;

use super::budget::{
    AGENT_TOTAL_TIMEOUT_ENV_VAR, DEFAULT_AGENT_TOTAL_TIMEOUT_SECS, LlmTimeoutBudget,
};
use super::{DEFAULT_HTTP_TIMEOUT_SECS, HTTP_TIMEOUT_ENV_VAR, MIN_HTTP_TIMEOUT_SECS};

/// `config.toml` key for the per-call plafond.
pub const HTTP_TIMEOUT_CONFIG_KEY: &str = "llm_http_timeout_secs";
/// `config.toml` key for the per-agent envelope.
pub const AGENT_TOTAL_TIMEOUT_CONFIG_KEY: &str = "agent_total_timeout_secs";
/// `config.toml` key for the per-call output-token budget (mika#2280).
pub const MAX_TOKENS_CONFIG_KEY: &str = "llm_max_tokens";
/// Environment variable overriding the output-token budget (mika#2280).
pub const MAX_TOKENS_ENV_VAR: &str = "MIKA_LLM_MAX_TOKENS";

/// `config.toml` key selecting the active provider (mika#2328).
pub const LLM_PROVIDER_CONFIG_KEY: &str = "llm_provider";
/// Environment variable selecting the active provider (mika#2328).
pub const LLM_PROVIDER_ENV_VAR: &str = "MIKA_LLM_PROVIDER";

/// The provider that applies when no door of the cascade carries one.
///
/// Mirrors `config::default_llm_provider`, which is private to that module. The
/// duplication is pinned by
/// [`tests::mika2328_model_reconstruction_equals_load_for_agent_on_every_cascade_position`],
/// which compares this reader against `Settings::load_for_agent` — including on
/// the position where nothing is declared.
const DEFAULT_PROVIDER: ProviderKind = ProviderKind::Anthropic;

/// The `model_source` value used when a door carried a provider this reader
/// cannot parse, so the model key's *name* is unknown.
///
/// A sixth word in the `*_source` vocabulary rather than a `default`: reporting
/// `default` there would state "no door carried the model", which is not known
/// and may well be false. Unreachable in production — `Settings::load_for_agent`
/// refuses such a value outright, so the agent never starts — but this reader
/// must never panic and must never answer wrongly (mika#2293).
pub const MODEL_SOURCE_UNKNOWN_PROVIDER: &str = "unknown_provider";

/// Which door of the cascade a value came through.
///
/// Five states, where mika#2293 asks for four: `GlobalConfig` is split out of
/// `AgentConfig` rather than folded into it. The ticket's question is "did the
/// *per-agent* setting take?", and answering `agent_config` for a value that
/// actually came from the shared `~/.mika/config.toml` would answer it wrongly
/// — the one failure mode this module must not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BudgetSource {
    /// `{agent_home}/.env` — highest priority since mika#2218.
    AgentDotenv,
    /// A `MIKA_*` variable in the process environment.
    ProcessEnv,
    /// `{agent_home}/config.toml` — the per-agent setting mika#2189 writes.
    AgentConfig,
    /// `{global_home}/config.toml` — the shared, fleet-wide file.
    GlobalConfig,
    /// No source carried the key; the compiled-in constant applies.
    Default,
}

impl BudgetSource {
    /// The wire name, as it appears in `llm_budget_resolved`.
    ///
    /// These strings are a log format an operator greps and groups by: two
    /// spellings of one source would split a population without saying so.
    /// Pinned by [`tests::mika2293_source_names_are_a_wire_format`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AgentDotenv => "agent_dotenv",
            Self::ProcessEnv => "process_env",
            Self::AgentConfig => "agent_config",
            Self::GlobalConfig => "global_config",
            Self::Default => "default",
        }
    }
}

/// One budget key, resolved: where it came from, what was written there, and
/// what that parses to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBudgetValue {
    /// The door it came through.
    pub source: BudgetSource,
    /// The raw string as written. `None` exactly when `source` is
    /// [`BudgetSource::Default`].
    ///
    /// Reported so a reading can be checked rather than trusted — a stray space
    /// or a quoted value is visible here and nowhere else.
    pub raw: Option<String>,
    /// The parsed seconds, or `None` when `raw` is not a non-negative integer.
    ///
    /// `None` is **not** a fallback to the default: it is the "an operator typed
    /// something that is not a number of seconds" state, which the startup guard
    /// refuses by name instead of swallowing. `0` parses to `Some(0)` rather
    /// than `None` for the same reason — it is a value an operator wrote, and
    /// the guard names it as such instead of calling it unreadable.
    pub value: Option<u64>,
}

impl ResolvedBudgetValue {
    /// The compiled-in constant, with nothing read.
    fn compiled_default() -> Self {
        Self {
            source: BudgetSource::Default,
            raw: None,
            value: None,
        }
    }

    fn from_raw(source: BudgetSource, raw: &str) -> Self {
        Self {
            source,
            raw: Some(raw.to_string()),
            value: raw.trim().parse::<u64>().ok(),
        }
    }

    /// The raw string, or `""` when nothing was read — a log-field shape.
    pub fn raw_or_empty(&self) -> &str {
        self.raw.as_deref().unwrap_or("")
    }
}

/// One **string-valued** key, resolved through the same cascade (mika#2328).
///
/// The twin of [`ResolvedBudgetValue`] minus the `u64` parse. The obstacle to
/// reporting a model's provenance was never the cascade —
/// [`CascadeLayers::resolve_key`] already took its two key names as parameters
/// and knows nothing about which key it is walking — it was the **type of the
/// value**: `ResolvedBudgetValue::value` is an `Option<u64>` built by
/// `parse::<u64>()`, and a model is a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTextValue {
    /// The door it came through.
    pub source: BudgetSource,
    /// The raw string as written. `None` exactly when `source` is
    /// [`BudgetSource::Default`].
    pub raw: Option<String>,
}

impl ResolvedTextValue {
    fn nothing_read() -> Self {
        Self {
            source: BudgetSource::Default,
            raw: None,
        }
    }

    /// The raw string, or `""` when nothing was read — a log-field shape.
    pub fn raw_or_empty(&self) -> &str {
        self.raw.as_deref().unwrap_or("")
    }
}

/// Which model one agent actually runs, and through which door (mika#2328).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProvenance {
    /// The active provider, or `None` when a door carried a value this reader
    /// cannot parse. `None` is **not** "the default applies": it is "the model
    /// key's name is unknown", which is why the model half is `None` with it.
    pub provider: Option<ProviderKind>,
    /// The `llm_provider` key as resolved — door and raw string.
    pub provider_value: ResolvedTextValue,
    /// The per-provider model key as resolved. `None` iff `provider` is `None`.
    pub model: Option<ResolvedTextValue>,
}

impl ModelProvenance {
    /// Rebuild the cascade for the provider key and the model key it names.
    ///
    /// Never panics, for the same reason [`BudgetProvenance::resolve`] does not:
    /// this reader precedes the paths that refuse a bad value, it does not
    /// replace them.
    pub fn resolve(global_home: &Path, agent_home: &Path) -> Self {
        Self::from_layers(&CascadeLayers::read(global_home, agent_home))
    }

    fn from_layers(layers: &CascadeLayers) -> Self {
        let provider_value = layers.resolve_text(LLM_PROVIDER_CONFIG_KEY, LLM_PROVIDER_ENV_VAR);

        // An absent provider is the compiled-in default — the shape every
        // agent without an `llm_provider` line has. A *present but unparseable*
        // one is a different world: the key's name cannot be derived, so no
        // model provenance can be stated at all.
        let provider = match provider_value.raw.as_deref() {
            None => Some(DEFAULT_PROVIDER),
            Some(raw) => ProviderKind::from_str(raw.trim()).ok(),
        };

        let model = provider.map(|p| layers.resolve_text(&model_config_key(p), &model_env_var(p)));

        Self {
            provider,
            provider_value,
            model,
        }
    }

    /// The model the agent actually runs — the declared value, or the
    /// provider's default when no door carried one.
    ///
    /// Mirrors `Settings::active_llm_config` arm for arm (`model.unwrap_or(
    /// provider.default_model())`), deliberately **without** filtering an empty
    /// or space-padded value: the number this reports is the one the agent runs
    /// under, not a second opinion about it. `None` when the provider itself is
    /// unreadable.
    pub fn effective_model(&self) -> Option<&str> {
        let provider = self.provider?;
        Some(match self.model.as_ref().and_then(|m| m.raw.as_deref()) {
            Some(raw) => raw,
            None => provider.default_model(),
        })
    }

    /// The wire name of the door the model came through, or
    /// [`MODEL_SOURCE_UNKNOWN_PROVIDER`] when the provider is unreadable.
    pub fn model_source_name(&self) -> &'static str {
        match &self.model {
            Some(model) => model.source.as_str(),
            None => MODEL_SOURCE_UNKNOWN_PROVIDER,
        }
    }

    /// The `config.toml` key this agent's model is read from (`zai_model`,
    /// `openrouter_model`, …) — the key an operator would have to edit. Empty
    /// when the provider is unreadable.
    pub fn model_config_key(&self) -> String {
        self.provider.map(model_config_key).unwrap_or_default()
    }

    /// The provider as a log field: its canonical key prefix, or the raw string
    /// that failed to parse — never a fallback silently presented as a reading.
    pub fn provider_name(&self) -> &str {
        match self.provider {
            Some(p) => p.config_prefix(),
            None => self.provider_value.raw_or_empty(),
        }
    }
}

/// `config.toml` key holding one provider's model (`zai` → `zai_model`).
fn model_config_key(provider: ProviderKind) -> String {
    format!("{}_model", provider.config_prefix())
}

/// Environment variable holding one provider's model (`zai` →
/// `MIKA_ZAI_MODEL`).
///
/// Derived from the same prefix rather than tabulated, because a table would be
/// a second place for the mapping to be wrong — and a wrong env-var name here
/// does not fail, it reports `default` for a model the environment is in fact
/// setting. Pinned empirically against `Settings::load_for_agent` for **every**
/// provider by
/// [`tests::mika2328_the_model_key_is_the_one_settings_reads_for_every_provider`].
fn model_env_var(provider: ProviderKind) -> String {
    format!("MIKA_{}_MODEL", provider.config_prefix().to_uppercase())
}

/// The provenance of one agent's budget: the time pair, plus the output-token
/// budget that runs inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetProvenance {
    /// The per-call plafond (`llm_http_timeout_secs` / `MIKA_LLM_HTTP_TIMEOUT_SECS`).
    pub http: ResolvedBudgetValue,
    /// The per-agent envelope (`agent_total_timeout_secs` / `MIKA_AGENT_TOTAL_TIMEOUT_SECS`).
    pub agent_total: ResolvedBudgetValue,
    /// The per-call output-token budget (`llm_max_tokens` / `MIKA_LLM_MAX_TOKENS`),
    /// added by mika#2280.
    ///
    /// It walks the **same** cascade as the two time keys — `resolve_key` is
    /// generic, so the third key is one line and `CascadeLayers::read` already
    /// reads each file once, making the cost nil.
    ///
    /// It closes a hole of the same nature as the one mika#2293 closed for the
    /// time pair: nothing said which output budget an agent ran under, **nor
    /// through which door it arrived**. That is the only thing that can settle
    /// whether mika-dev's runtime still carries the constant's 8192 or a value
    /// raised by hand — as mika-arch's had been — a question the checkout alone
    /// cannot answer.
    pub max_tokens: ResolvedBudgetValue,
}

impl BudgetProvenance {
    /// Rebuild the cascade for the two budget keys of one agent.
    ///
    /// **Never panics and never reads through [`super::http_timeout_secs`]**,
    /// which is what lets the mika#2293 startup guard call it on a value that
    /// would otherwise abort the process without naming the agent that carries
    /// it (mika#1660 keeps that panic on its own cold path; this reader
    /// precedes it, it does not replace it).
    pub fn resolve(global_home: &Path, agent_home: &Path) -> Self {
        Self::from_layers(&CascadeLayers::read(global_home, agent_home))
    }

    fn from_layers(layers: &CascadeLayers) -> Self {
        Self {
            http: layers.resolve_key(HTTP_TIMEOUT_CONFIG_KEY, HTTP_TIMEOUT_ENV_VAR),
            agent_total: layers
                .resolve_key(AGENT_TOTAL_TIMEOUT_CONFIG_KEY, AGENT_TOTAL_TIMEOUT_ENV_VAR),
            max_tokens: layers.resolve_key(MAX_TOKENS_CONFIG_KEY, MAX_TOKENS_ENV_VAR),
        }
    }

    /// The pair as the runtime would compute it, fallbacks and floors included.
    ///
    /// Mirrors `Settings::effective_llm_http_timeout_secs` and
    /// `Settings::effective_agent_total_timeout_secs` arm for arm — the plafond
    /// is floored at [`MIN_HTTP_TIMEOUT_SECS`], an envelope of `0` falls back to
    /// its default — so the number this reports is the number the agent runs
    /// under and not a second opinion about it.
    ///
    /// An **unreadable** value maps to the compiled default here. That is a
    /// reporting convention, not a reproduction: config-rs refuses to
    /// deserialize such a value, so the real runtime never gets that far. The
    /// mika#2293 startup guard refuses it by name first, which is the point.
    ///
    /// Returned **unvalidated**: refusing a pair is the guard's job and the
    /// provider constructor's, not a getter's.
    pub fn effective_budget(&self) -> LlmTimeoutBudget {
        let http = self
            .http
            .value
            .map(|v| v.max(MIN_HTTP_TIMEOUT_SECS))
            .unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS);
        let total = match self.agent_total.value {
            Some(0) | None => DEFAULT_AGENT_TOTAL_TIMEOUT_SECS,
            Some(secs) => secs,
        };
        LlmTimeoutBudget::unvalidated(http, total)
    }

    /// The output-token budget as the runtime would compute it (mika#2280).
    ///
    /// Mirror of [`Self::effective_budget`] for the third key: when no door
    /// carries `llm_max_tokens`, or carries something unreadable, this reports
    /// the same compiled-in fallback `Settings` itself uses — read from
    /// `crate::config::default_max_tokens` rather than written out a second
    /// time, so a "default" reported here cannot disagree with the value in
    /// force.
    ///
    /// A value too large for a `u32` saturates rather than wrapping: reporting
    /// a small number for a huge one would be worse than reporting a clamp.
    pub fn effective_max_tokens(&self) -> u32 {
        self.max_tokens
            .value
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX))
            .unwrap_or_else(crate::config::default_max_tokens)
    }
}

/// The file-backed cascade positions, read **once** per [`BudgetProvenance`].
///
/// Both budget keys walk the same four doors, so reading per key would parse
/// `{agent_home}/.env` twice and each `config.toml` up to twice, discarding a
/// whole `HashMap` or `toml::Table` after pulling one key out of it. Cheap at
/// the boot sites, but `teams::engine` re-runs its per-member loop on every
/// suspend/resume of a team run — so the waste recurs there for the life of the
/// run.
///
/// The `global_home != agent_home` discriminant is the same one
/// `Settings::load_for_agent` branches on, for the same reason: under the legacy
/// single-agent layout there is no per-agent file of either kind, and the
/// process environment keeps its usual "shell always wins" semantics.
struct CascadeLayers {
    /// `{agent_home}/.env`, keyed by full `MIKA_*` variable name. `None` under
    /// the legacy layout, where no per-agent `.env` participates.
    agent_dotenv: Option<std::collections::HashMap<String, String>>,
    /// `{agent_home}/config.toml`. `None` under the legacy layout — there the
    /// file *is* the global one, and counting it twice would report
    /// `agent_config` for a value that came from the shared file.
    agent_config: Option<toml::Table>,
    /// `{global_home}/config.toml`.
    global_config: Option<toml::Table>,
}

impl CascadeLayers {
    fn read(global_home: &Path, agent_home: &Path) -> Self {
        let has_agent_home = global_home != agent_home;
        Self {
            agent_dotenv: has_agent_home.then(|| crate::dotenv::parse_dotenv(agent_home)),
            agent_config: has_agent_home
                .then(|| read_config_table(&agent_home.join("config.toml")))
                .flatten(),
            global_config: read_config_table(&global_home.join("config.toml")),
        }
    }

    /// Resolve one key through the four positions, highest priority first —
    /// the door it came through and the raw string as written, or `None` when
    /// no door carried it.
    ///
    /// Untyped on purpose (mika#2328): the walk is identical for a timeout and
    /// for a model name, and duplicating it for the second would be a second
    /// place for the mika#2218 inverted order to drift.
    fn resolve_raw(&self, config_key: &str, env_var: &str) -> Option<(BudgetSource, String)> {
        // 1. Per-agent `.env` — highest priority since mika#2218.
        if let Some(raw) = self.agent_dotenv.as_ref().and_then(|v| v.get(env_var)) {
            return Some((BudgetSource::AgentDotenv, raw.clone()));
        }

        // 2. Process environment.
        if let Ok(raw) = std::env::var(env_var)
            && !raw.trim().is_empty()
        {
            return Some((BudgetSource::ProcessEnv, raw));
        }

        // 3. Per-agent `config.toml`.
        if let Some(raw) = self
            .agent_config
            .as_ref()
            .and_then(|t| config_key_as_string(t, config_key))
        {
            return Some((BudgetSource::AgentConfig, raw));
        }

        // 4. Global `config.toml`.
        if let Some(raw) = self
            .global_config
            .as_ref()
            .and_then(|t| config_key_as_string(t, config_key))
        {
            return Some((BudgetSource::GlobalConfig, raw));
        }

        None
    }

    /// Resolve one **integer-valued** key through the four positions.
    fn resolve_key(&self, config_key: &str, env_var: &str) -> ResolvedBudgetValue {
        match self.resolve_raw(config_key, env_var) {
            Some((source, raw)) => ResolvedBudgetValue::from_raw(source, &raw),
            None => ResolvedBudgetValue::compiled_default(),
        }
    }

    /// Resolve one **string-valued** key through the four positions.
    fn resolve_text(&self, config_key: &str, env_var: &str) -> ResolvedTextValue {
        match self.resolve_raw(config_key, env_var) {
            Some((source, raw)) => ResolvedTextValue {
                source,
                raw: Some(raw),
            },
            None => ResolvedTextValue::nothing_read(),
        }
    }
}

/// Parse one `config.toml`, or `None` for an absent or unparseable file.
///
/// Both failures mean "this door carries nothing", which is the only
/// distinction the cascade needs — a malformed file is refused loudly by
/// `Settings::load_for_agent` itself, not here.
fn read_config_table(path: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

/// One top-level key, as the string it was written as.
///
/// Integers and quoted strings both come back as their decimal text so the
/// caller parses one shape.
fn config_key_as_string(table: &toml::Table, key: &str) -> Option<String> {
    match table.get(key)? {
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Last `llm_budget_resolved` signature emitted per agent, for deduplication.
///
/// A map rather than a set: the contract is "a repetition is silent, a *change*
/// is re-emitted", and a set would swallow an A → B → A oscillation, which is
/// precisely the budget movement worth seeing.
static LAST_EMITTED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// The provider attempt ceiling used to report `max_attempts`.
///
/// [`super::DEFAULT_ATTEMPTS_HARD_CAP`], the ceiling every OpenAI-compatible
/// rail runs under — which is every well-known agent today. Read from the
/// shared constant since mika#2342; it used to reach into `openai`'s private
/// copy, which is the same value by a longer route.
const REPORTED_ATTEMPT_HARD_CAP: u32 = super::DEFAULT_ATTEMPTS_HARD_CAP;

/// One agent's resolved budget and model, as a value (mika#2457).
///
/// # Why this type exists
///
/// `llm_budget_resolved` already carried every field below, and it answered the
/// question mika#2293 was opened for. What it could not do is answer it **on
/// demand**: it is emitted once per agent per resolved pair, deduplicated, into
/// a log file measured in gigabytes. An operator asking *"which model and which
/// budget is mika-arch running under, and through which door?"* had no surface
/// to ask. That gap is what let mika-arch's `240/900`, shipped 2026-09-06, fail
/// in silence until the 2026-09-11 measurement, and it is what swallowed
/// mika#2457: three different plafonds were asserted for the same agent — 240
/// (the checkout), 420 (mika#2342), 300 (mika#2457) — and **not one** had ever
/// been established by reading an instrument.
///
/// # The field this type adds, and the one it must not
///
/// [`Self::resolved_at`] is the only addition, and it is a fact **about the
/// record**, not a second opinion about the budget: it dates the resolution, it
/// does not redo it. It is deliberately **absent from
/// [`dedup_signature`]** — two identical resolutions at two instants must stay
/// one line, or the field meant to document the deduplication would annul it.
/// Pinned by `mika2457_resolved_at_dates_the_record_and_stays_out_of_the_dedup_key`,
/// which **forces** two distinct instants: the field is stamped to the second,
/// so two resolutions back to back almost always carry the same string, and a
/// test relying on the clock to separate them would stay green over the leak.
///
/// # Why it is a value and not a re-resolution
///
/// A consumer that re-resolved would read `process_env` **of its own process**.
/// On a workstation where a CLI and the daemon share an environment that reads
/// identically; on a server where a service variable shadows the per-agent
/// `config.toml` it reports a setting that is not in force, with the authority
/// of a measurement. That is mika#2304's defect one field over (`--verbose`
/// printing the *requested* model while the turn ran under another). So this
/// record is resolved **once, by the process that serves the agent**, and every
/// consumer renders it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedBudgetRecord {
    /// The agent this record was resolved for.
    pub agent_id: String,
    /// The per-call plafond in force, floors applied ([`LlmTimeoutBudget`]).
    pub http_timeout_secs: u64,
    /// The per-agent envelope in force, fallbacks applied.
    pub agent_total_timeout_secs: u64,
    /// `floor(envelope / plafond)`, clamped to the provider hard cap.
    pub max_attempts: u32,
    /// How many of those attempts the deadline guard can actually reach
    /// (mika#2362).
    pub effective_max_attempts: u32,
    /// `effective_max_attempts == max_attempts`.
    pub retry_reachable: bool,
    /// What the mika#2342 watchdog is sized on.
    pub worst_case_failure_secs: u64,
    /// The per-call output-token budget in force (mika#2280).
    pub llm_max_tokens: u32,
    /// What the plafond can physically carry at the assumed throughput floor.
    pub reachable_output_tokens: u64,
    /// Door the plafond came through — `agent_config` / `process_env` / …
    pub http_source: String,
    /// Door the envelope came through.
    pub total_source: String,
    /// Door the output budget came through.
    pub max_tokens_source: String,
    /// The plafond as written, or `""` when nothing was read.
    pub http_raw: String,
    /// The envelope as written, or `""`.
    pub total_raw: String,
    /// The output budget as written, or `""`.
    pub max_tokens_raw: String,
    /// The provider in force, or the raw string that failed to parse.
    pub provider: String,
    /// Door the provider came through.
    pub provider_source: String,
    /// The model in force, or `""` when the provider itself is unreadable.
    pub model: String,
    /// Door the model came through, or [`MODEL_SOURCE_UNKNOWN_PROVIDER`].
    pub model_source: String,
    /// The `config.toml` key an operator would edit to change the model.
    pub model_config_key: String,
    /// When this record was resolved — RFC 3339 UTC (mika#2457).
    ///
    /// The record is frozen at `init_agent` and reports **the state the agent
    /// runs under**, never the state of the disk at the instant of the read.
    /// The two coincide unless someone edited the `config.toml` since boot —
    /// which is precisely the out-of-repo drift mika#2328 measured. Without a
    /// date, `model = moonshotai/kimi-k2.5` cannot be told apart from "the disk
    /// changed after this record was taken". This field does not lift that
    /// ambiguity on its own; it makes it **decidable** against the file's mtime.
    pub resolved_at: String,
}

/// Resolve one agent's budget and model into a [`ResolvedBudgetRecord`].
///
/// **The single constructor of the record** (mika#2457). A second one would be
/// free to diverge from this one — the class `grooming_marker` (mika#2158) had
/// to close once, where promotion and dispatch routing answered the same
/// question differently for months while nothing broke.
///
/// Never panics, for the same reason [`BudgetProvenance::resolve`] does not.
pub fn resolve_llm_budget_record(
    agent_id: &str,
    global_home: &Path,
    agent_home: &Path,
) -> ResolvedBudgetRecord {
    // One read of the four cascade doors, two facts drawn from it: the budget
    // pair and the model. They are read together by whoever diagnoses a cut
    // turn, so they are resolved together.
    let layers = CascadeLayers::read(global_home, agent_home);
    let provenance = BudgetProvenance::from_layers(&layers);
    let model = ModelProvenance::from_layers(&layers);
    let budget = provenance.effective_budget();

    let max_attempts = budget.max_attempts(REPORTED_ATTEMPT_HARD_CAP);
    let effective_max_attempts = budget.effective_max_attempts(REPORTED_ATTEMPT_HARD_CAP);

    ResolvedBudgetRecord {
        agent_id: agent_id.to_string(),
        http_timeout_secs: budget.http_timeout_secs(),
        agent_total_timeout_secs: budget.agent_total_timeout_secs(),
        max_attempts,
        effective_max_attempts,
        retry_reachable: effective_max_attempts == max_attempts,
        worst_case_failure_secs: budget.worst_case_failure_secs(REPORTED_ATTEMPT_HARD_CAP),
        // mika#2280: the output budget in force, and what the plafond can
        // physically carry. Context only — **no WARN is emitted for it**. A
        // declared budget above the reachable figure is not a defect in itself:
        // mika#2296 chose exactly that for mika-arch, as "a ceiling made
        // non-binding". A guard firing on the declaration would contradict a
        // documented decision at every startup, and a warning that contradicts a
        // decision gets muted. What earns an operator's attention is a
        // *crossing*, once per cut call — that is `llm_call_cap_exhausted`, on
        // the rails that apply the plafond.
        llm_max_tokens: provenance.effective_max_tokens(),
        reachable_output_tokens: budget
            .reachable_output_tokens(super::budget::output_tokens_per_sec_floor()),
        http_source: provenance.http.source.as_str().to_string(),
        total_source: provenance.agent_total.source.as_str().to_string(),
        max_tokens_source: provenance.max_tokens.source.as_str().to_string(),
        http_raw: provenance.http.raw_or_empty().to_string(),
        total_raw: provenance.agent_total.raw_or_empty().to_string(),
        max_tokens_raw: provenance.max_tokens.raw_or_empty().to_string(),
        // mika#2328 — the model half. `model_config_key` names the key an
        // operator would edit, which is how the "resolved from the provider,
        // never hard-coded" rule becomes checkable from the record alone.
        provider: model.provider_name().to_string(),
        provider_source: model.provider_value.source.as_str().to_string(),
        model: model.effective_model().unwrap_or("").to_string(),
        model_source: model.model_source_name().to_string(),
        model_config_key: model.model_config_key(),
        resolved_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

/// The deduplication key of one resolved budget.
///
/// Written once and read by the emitter *and* by its test (mika#2362): the test
/// used to rebuild this string by hand, so extending the key silently broke it
/// — a copy of a predicate, in the ticket that exists to remove copies of a
/// predicate.
///
/// `effective_max_attempts` is part of the key: without it, a geometry change
/// that moves only *reachability* — the very thing the new fields report —
/// would be deduplicated away as "no change".
///
/// `llm_max_tokens` and `reachable_output_tokens` joined it in mika#2280, and
/// the reason is that lesson applied twice rather than a second paragraph: a
/// configuration change moving only the output budget is exactly what the new
/// fields exist to say, so leaving it out of the key would silence the event on
/// the only change it was added for.
///
/// The model and its provenance are part of it for the same reason (mika#2328):
/// an out-of-repo model swap moves neither timeout, and a signature blind to it
/// would silence the single line that reports the swap.
///
/// **`resolved_at` is deliberately NOT part of it** (mika#2457): it is a fact
/// about the record, not about the budget, and including it would make every
/// resolution a "change" and annul the deduplication outright.
fn dedup_signature(record: &ResolvedBudgetRecord) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        record.http_timeout_secs,
        record.agent_total_timeout_secs,
        record.http_source,
        record.total_source,
        record.effective_max_attempts,
        record.llm_max_tokens,
        record.max_tokens_source,
        record.reachable_output_tokens,
        record.provider,
        record.provider_source,
        record.model,
        record.model_source,
    )
}

/// Resolve `agent_id`'s budget and emit `llm_budget_resolved` for it — the line
/// that answers "under which plafond did this turn run?" (mika#2293 AC1).
///
/// Since mika#2457 this is [`resolve_llm_budget_record`] followed by
/// [`emit_llm_budget_resolved`], which carries the gating, deduplication and
/// scope doctrine; kept for the callers that have no use for the record itself
/// (`teams::engine`).
pub fn log_llm_budget_resolved(agent_id: &str, global_home: &Path, agent_home: &Path) {
    emit_llm_budget_resolved(&resolve_llm_budget_record(
        agent_id,
        global_home,
        agent_home,
    ));
}

/// Emit `llm_budget_resolved` for an already-resolved record (mika#2457).
///
/// Split from [`log_llm_budget_resolved`] so `server::init_agent` can **keep**
/// the record it emits instead of resolving the cascade twice — two resolutions
/// milliseconds apart being two chances to disagree, and the second one being
/// the value nobody would have checked.
///
/// # Ungated on purpose
///
/// This does not depend on `MIKA_STORE_LLM_CALLS`. It is a *configuration*
/// event, not call telemetry, and it has to stay readable precisely when an
/// operator has turned telemetry off to cut noise.
///
/// # Deduplicated, which is not the same as gated
///
/// Provider construction is a per-turn event, not a per-boot one. The
/// deduplication bounds the repetition; it subordinates the event to no
/// setting. A pair that *changes* is re-emitted in full.
///
/// # What this deliberately does not see
///
/// Called from the two sites that already know which agent they are building
/// for. The per-skill `[llm]` override path (`agent_loop`'s `make_provider_for`)
/// is **out of scope for mika#2293** and emits nothing: the question the ticket
/// asks is about an agent's *nominal* budget, not what a skill overrides for one
/// turn. Said here so a reader looking for an override's budget knows it was
/// never written, rather than concluding the instrument is broken.
pub fn emit_llm_budget_resolved(record: &ResolvedBudgetRecord) {
    let agent_id = record.agent_id.as_str();
    let signature = dedup_signature(record);

    let mut seen = LAST_EMITTED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if seen.get(agent_id) == Some(&signature) {
        return;
    }
    seen.insert(agent_id.to_string(), signature);
    drop(seen);

    let max_attempts = record.max_attempts;
    let effective_max_attempts = record.effective_max_attempts;
    let retry_reachable = record.retry_reachable;

    tracing::info!(
        event = "llm_budget_resolved",
        agent_id,
        http_timeout_secs = record.http_timeout_secs,
        agent_total_timeout_secs = record.agent_total_timeout_secs,
        max_attempts,
        effective_max_attempts,
        retry_reachable,
        worst_case_failure_secs = record.worst_case_failure_secs,
        llm_max_tokens = record.llm_max_tokens,
        reachable_output_tokens = record.reachable_output_tokens,
        http_source = record.http_source,
        total_source = record.total_source,
        max_tokens_source = record.max_tokens_source,
        http_raw = record.http_raw,
        total_raw = record.total_raw,
        max_tokens_raw = record.max_tokens_raw,
        provider = record.provider,
        provider_source = record.provider_source,
        model = record.model,
        model_source = record.model_source,
        model_config_key = record.model_config_key,
        "resolved LLM timeout budget (mika#2293)"
    );

    // mika#2362 D4 — the measurement, emitted where the geometry is known and
    // nowhere else.
    //
    // **It sits after the dedup `return` on purpose**, so it fires once per
    // agent per resolved pair, exactly like the INFO line it accompanies — not
    // once per turn. The two describe one configuration event, and a WARN that
    // repeated while its own INFO stayed silent would read as a new condition
    // each time. Said out loud in the operator section of the root `CLAUDE.md`
    // too: a grep finding a handful of lines after days of running is the
    // nominal regime, not evidence the geometry was fixed.
    //
    // **Not a startup refusal.** `envelope = k × cap` is a *valid, working*
    // configuration: calls run, and the transport class still retries. Refusing
    // to boot on it would lay the fleet down over a suboptimal setting — the
    // exact failure mode mika#2293 had to name for its own guard. This produces
    // the number; the decision belongs to the operator, with it in hand.
    if !retry_reachable {
        tracing::warn!(
            event = "llm_budget_retry_unreachable",
            agent_id,
            http_timeout_secs = record.http_timeout_secs,
            agent_total_timeout_secs = record.agent_total_timeout_secs,
            max_attempts,
            effective_max_attempts,
            http_source = record.http_source,
            total_source = record.total_source,
            "this agent's last nominal LLM retry cannot run: after an attempt consuming the \
             full per-call cap the remaining envelope is at or below the non-transport retry \
             threshold (1.0 × cap), so the deadline guard refuses it. The envelope being an \
             exact multiple of the cap is the usual cause — move either number off the \
             multiple to make the attempt reachable (mika#2362)"
        );
    }
}

// ---------------------------------------------------------------------------
// mika#2473 — the declared side, and the drift between it and the runtime
// ---------------------------------------------------------------------------

/// What `well_known_agents.rs` says an agent runs — the **repo's** side of the
/// question mika#2328 left open (mika#2473).
///
/// Everything above this line reads the *runtime*: four cascade doors, the
/// value in force. This reads the other side, the compiled-in constant, and it
/// reads it through the **same** key derivation ([`model_config_key`]) and the
/// same default rule (`provider.default_model()`). A second parser would be
/// free to diverge from the first, and the divergence would not surface as a
/// crash: it would surface as a *false* drift reported on a correct
/// configuration, which this module holds to be strictly worse than none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredModel {
    /// The provider the constant selects, or [`DEFAULT_PROVIDER`] when it
    /// carries no `llm_provider` line.
    pub provider: ProviderKind,
    /// The model the constant declares, or the provider's default.
    pub model: String,
    /// `true` when the constant carries no model line and the provider's
    /// default applies.
    ///
    /// Reported rather than left implicit: "the repo declares `glm-5.2`" and
    /// "the repo declares nothing and `glm-5.2` is what z.ai defaults to" are
    /// two different facts, and reconciling a drift (mika#2472) means knowing
    /// which one is on the table.
    pub model_is_provider_default: bool,
}

/// Read the `(provider, model)` pair one `config_toml` constant declares.
///
/// `None` in exactly two cases, both meaning *"this text states no pair"*
/// rather than *"the pair is the default"*:
///
/// - the text does not parse as TOML — the same reading as
///   [`read_config_table`], where an absent and a malformed file are one
///   answer, because the loud refusal belongs to `Settings::load_for_agent`,
///   not to this reader;
/// - `llm_provider` is present but unparseable — the same rule as
///   [`ModelProvenance::from_layers`], which cannot name the model key either.
///
/// # Why the provider is trimmed before `from_str`
///
/// Because [`ModelProvenance::from_layers`] trims on its side. A constant
/// written `llm_provider = " zai "` resolves perfectly well through the
/// cascade; a declared side that did not trim would answer `None` here and a
/// valid pair there — i.e. a **false** `Drift` carrying `unknown_provider` on a
/// correct configuration, or worse, a `NotApplicable` that reads as "this agent
/// has no constant" and switches the guard off in silence. The two readers face
/// each other; they trim together or one of them lies.
///
/// # Why it does not walk the cascade
///
/// [`CascadeLayers::resolve_raw`] reads the process environment. The constant
/// in the repo has **no** environment door: it is a literal in a `.rs` file. A
/// declared side resolved through the cascade would read the *runtime*'s
/// variables and agree with the runtime by construction — a guard comparing a
/// value with itself (KTD6).
pub fn declared_model(config_toml: &str) -> Option<DeclaredModel> {
    let table: toml::Table = config_toml.parse().ok()?;

    let provider = match config_key_as_string(&table, LLM_PROVIDER_CONFIG_KEY) {
        None => DEFAULT_PROVIDER,
        Some(raw) => ProviderKind::from_str(raw.trim()).ok()?,
    };

    let written = config_key_as_string(&table, &model_config_key(provider));
    Some(DeclaredModel {
        provider,
        model_is_provider_default: written.is_none(),
        model: written.unwrap_or_else(|| provider.default_model().to_string()),
    })
}

/// The file the declared side is read from, as a log field (mika#2473).
///
/// A drift line without it says "the repo disagrees" and leaves the reader to
/// find where the repo said so. Named once, here, so the WARN and the route
/// cannot spell it two ways.
pub const DECLARED_BY: &str = "well_known_agents.rs";

/// Whether one agent runs the model its constant declares (mika#2473).
///
/// # Why this is a sibling of the record and not a field of it
///
/// [`ResolvedBudgetRecord`] has a single constructor
/// ([`resolve_llm_budget_record`]) and knows nothing about well-known agents —
/// `WellKnownAgent` lives in `mika-agent`. The check is therefore **built** in
/// `mika-agent`, where the constant is in scope, and **lives** here so the
/// route and the CLI can serialise it without depending on that crate (KTD3).
///
/// # Three arms, and why the third is not an absence
///
/// `NotApplicable` is an agent whose spec carries no `config_toml`: nothing was
/// declared, so nothing can be out of phase. It is a distinct word rather than
/// a silent `InSync` for the same reason [`MODEL_SOURCE_UNKNOWN_PROVIDER`] is a
/// sixth word rather than a `default` — answering "in phase" about a comparison
/// that never happened is a false statement made with authority.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ModelDriftCheck {
    /// This agent declares no model — nothing to compare.
    NotApplicable,
    /// The runtime serves exactly what the repo declares.
    InSync {
        /// The provider the constant selects.
        declared_provider: String,
        /// The model the constant declares.
        declared_model: String,
    },
    /// The runtime serves something else — reported with the door it came
    /// through, never with a refusal (KTD1).
    Drift {
        /// The provider the constant selects.
        declared_provider: String,
        /// The model the constant declares.
        declared_model: String,
        /// The provider in force, or the raw string that failed to parse.
        runtime_provider: String,
        /// The model in force, or `""` when the provider itself is unreadable.
        runtime_model: String,
        /// Door the model came through, or [`MODEL_SOURCE_UNKNOWN_PROVIDER`].
        runtime_model_source: String,
        /// Door the provider came through.
        runtime_provider_source: String,
        /// The `config.toml` key an operator would edit to resolve the drift.
        ///
        /// Empty exactly when the runtime provider is unreadable — there is no
        /// model key to name, and `runtime_model_source = unknown_provider`
        /// already says which line to look at. Naming a plausible key there
        /// would be a false provenance.
        model_config_key: String,
    },
}

impl ModelDriftCheck {
    /// Compare the declared pair to the resolved record. **Pure**: it reads no
    /// file, touches no environment, and cannot refuse anything (KTD1).
    ///
    /// Drift iff the provider or the model differs. A record whose `model` is
    /// `""` — the [`MODEL_SOURCE_UNKNOWN_PROVIDER`] shape — is a drift like any
    /// other, reported with that word rather than absorbed: an agent whose
    /// `llm_provider` line has been made unreadable is precisely the state an
    /// operator needs told.
    pub fn compare(declared: Option<&DeclaredModel>, record: &ResolvedBudgetRecord) -> Self {
        let Some(declared) = declared else {
            return Self::NotApplicable;
        };

        let declared_provider = declared.provider.config_prefix().to_string();
        if record.provider == declared_provider && record.model == declared.model {
            return Self::InSync {
                declared_provider,
                declared_model: declared.model.clone(),
            };
        }

        Self::Drift {
            declared_provider,
            declared_model: declared.model.clone(),
            runtime_provider: record.provider.clone(),
            runtime_model: record.model.clone(),
            runtime_model_source: record.model_source.clone(),
            runtime_provider_source: record.provider_source.clone(),
            model_config_key: record.model_config_key.clone(),
        }
    }
}

/// Emit the mika#2473 D1 line for one agent — **the single emission site** of
/// the code↔runtime guard.
///
/// The sibling of [`emit_llm_budget_resolved`], and posted here rather than in
/// `mika-agent` for the reason that function gives for its own placement: the
/// event's field names are a log format an operator greps, and a format written
/// at the call site is a format that acquires a second spelling.
///
/// # One line per agent per init, and no deduplication
///
/// [`emit_llm_budget_resolved`] deduplicates because provider construction is a
/// *per-turn* event. This is called once per agent at `init_agent`, so the
/// repetition it would bound does not exist; a dedup map here would only be a
/// second thing to reset in tests. The frequency argument is the one this
/// module already makes above: *"a warning that contradicts a decision gets
/// muted. What earns an operator's attention is a crossing."* Three agents,
/// three lines, at startup — that is a crossing being reported, not a drumbeat.
///
/// # Reports, never refuses (KTD1)
///
/// No `bail!`, no `Err`, no panic. The drift on this fleet is a sequence of
/// dated operator decisions in three `config.toml` files; refusing to boot on
/// it would lay the fleet down over a *valid* configuration — which is exactly
/// the failure mode `llm_budget_retry_unreachable` had to name for itself. This
/// produces the fact; the decision (restore, recalibrate, or reconcile the
/// constant) belongs to the operator with the record in hand.
///
/// `NotApplicable` emits **nothing**: an agent with no constant has no drift to
/// report, and a line saying so on every boot would be the noise that gets the
/// other two muted.
pub fn emit_model_drift(agent_id: &str, check: &ModelDriftCheck) {
    match check {
        ModelDriftCheck::NotApplicable => {}
        ModelDriftCheck::InSync {
            declared_provider,
            declared_model,
        } => {
            // Emitted on purpose, at INFO. Without it, "no WARN" is
            // indistinguishable from "the guard did not run" — the exact
            // ambiguity that let a glm-5.3 serve mika-qa for weeks while the
            // repo said glm-5.2 and nothing anywhere said otherwise.
            tracing::info!(
                event = "well_known_model_in_sync",
                agent_id,
                declared_provider,
                declared_model,
                declared_by = DECLARED_BY,
                "this agent runs the model its constant declares (mika#2473)"
            );
        }
        ModelDriftCheck::Drift {
            declared_provider,
            declared_model,
            runtime_provider,
            runtime_model,
            runtime_model_source,
            runtime_provider_source,
            model_config_key,
        } => {
            tracing::warn!(
                event = "well_known_model_drift",
                agent_id,
                declared_provider,
                declared_model,
                runtime_provider,
                runtime_model,
                runtime_model_source,
                runtime_provider_source,
                model_config_key,
                declared_by = DECLARED_BY,
                "this agent does NOT run the model the repo declares for it: the runtime \
                 value came through the door named by `runtime_model_source`, and the \
                 declaration lives in `declared_by`. Nothing is refused (mika#2473 KTD1) — \
                 restoring the file, recalibrating the agent, or reconciling the constant \
                 is an operator decision, and this line is the fact it needs"
            );
        }
    }
}

/// Clear every budget variable — and, since mika#2328, every variable that can
/// move the resolved model — from the process env.
///
/// The model half has to clear `MIKA_LLM_PROVIDER` and all thirteen
/// `MIKA_*_MODEL` variables: an ambient one would silently put a test's cascade
/// one door higher than the door it means to exercise.
///
/// # Why it is exported rather than private to `mod tests` (mika#2473)
///
/// The `MIKA_{PREFIX}_MODEL` mapping is derived here, from [`model_env_var`],
/// and a second caller needing a clean cascade — `llm::config_freshness`, and
/// `mika-agent`'s boot tests after it — would otherwise have to re-derive it.
/// A re-derivation of this list does not fail loudly when it drifts: it leaves
/// one ambient variable set, which puts the test's cascade one door higher than
/// the door it means to exercise, and the test stays green while measuring
/// something else. So the list stays single, under the `test-utils` feature the
/// crate already carries for exactly this shape (mika#2331 §3.4).
///
/// # Safety
/// Test-only, serialized by `#[serial]`.
#[cfg(any(test, feature = "test-utils"))]
pub fn clean_budget_env() {
    unsafe {
        std::env::remove_var(HTTP_TIMEOUT_ENV_VAR);
        std::env::remove_var(AGENT_TOTAL_TIMEOUT_ENV_VAR);
        std::env::remove_var(MAX_TOKENS_ENV_VAR);
        std::env::remove_var(LLM_PROVIDER_ENV_VAR);
        for provider in ProviderKind::ALL {
            std::env::remove_var(model_env_var(*provider));
        }
    }
}

/// Forget every emitted signature — test-only, so one test's emission cannot
/// silence another's.
#[cfg(test)]
fn reset_dedup_for_test() {
    LAST_EMITTED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use serial_test::serial;

    fn homes() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let global = tmp.path().join("global");
        let agent = tmp.path().join("agents").join("mika-arch");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::create_dir_all(&agent).unwrap();
        (tmp, global, agent)
    }

    /// mika#2293 F1 — the load-bearing test of this module.
    ///
    /// The duplication of the cascade is only tenable while what this module
    /// reconstructs equals what `Settings::load_for_agent` merges. One fixture
    /// per cascade position, checked on **all three** budget keys since
    /// mika#2280 (AC3): the day the order changes in `config.rs`, this goes red
    /// rather than letting the reported provenance drift away from the value
    /// actually in force.
    ///
    /// **If the third key alone goes red, halt and surface** (mika#2280 FD3):
    /// it means `max_tokens_source` would name the wrong door, and a false
    /// provenance is strictly worse than none — it answers, with authority, the
    /// one question probe (a) exists to settle. Do not add an allowlist and do
    /// not `#[ignore]`; resolving the divergence *is* the scope decision.
    #[test]
    #[serial]
    fn mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        // Position 4 — global config.toml only.
        std::fs::write(
            global.join("config.toml"),
            "llm_http_timeout_secs = 130\nagent_total_timeout_secs = 400\nllm_max_tokens = 2048\n",
        )
        .unwrap();
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::GlobalConfig);
        assert_eq!(p.agent_total.source, BudgetSource::GlobalConfig);
        assert_eq!(p.max_tokens.source, BudgetSource::GlobalConfig);
        assert_eq!(
            p.effective_budget().http_timeout_secs(),
            settings.effective_llm_http_timeout_secs(),
            "position 4 (global config.toml): plafond reconstruit ≠ plafond fusionné"
        );
        assert_eq!(
            p.effective_budget().agent_total_timeout_secs(),
            settings.effective_agent_total_timeout_secs(),
            "position 4 (global config.toml): enveloppe reconstruite ≠ enveloppe fusionnée"
        );
        assert_eq!(
            p.effective_max_tokens(),
            settings.llm_max_tokens,
            "position 4 (global config.toml): budget de sortie reconstruit ≠ fusionné"
        );

        // Position 3 — per-agent config.toml beats the global one.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\nllm_max_tokens = 32768\n",
        )
        .unwrap();
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::AgentConfig);
        assert_eq!(p.max_tokens.source, BudgetSource::AgentConfig);
        assert_eq!(p.effective_budget().http_timeout_secs(), 240);
        assert_eq!(p.effective_max_tokens(), 32_768);
        assert_eq!(
            p.effective_budget().http_timeout_secs(),
            settings.effective_llm_http_timeout_secs(),
            "position 3 (config.toml per-agent): plafond reconstruit ≠ plafond fusionné"
        );
        assert_eq!(
            p.effective_budget().agent_total_timeout_secs(),
            settings.effective_agent_total_timeout_secs(),
            "position 3 (config.toml per-agent): enveloppe reconstruite ≠ enveloppe fusionnée"
        );
        assert_eq!(
            p.effective_max_tokens(),
            settings.llm_max_tokens,
            "position 3 (config.toml per-agent): budget de sortie reconstruit ≠ fusionné"
        );

        // Position 2 — the process env beats both config files. This is H2.
        // Safety: test-only env vars, serialized by `#[serial]`.
        unsafe {
            std::env::set_var(HTTP_TIMEOUT_ENV_VAR, "150");
            std::env::set_var(AGENT_TOTAL_TIMEOUT_ENV_VAR, "500");
            std::env::set_var(MAX_TOKENS_ENV_VAR, "4096");
        }
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::ProcessEnv);
        assert_eq!(p.max_tokens.source, BudgetSource::ProcessEnv);
        assert_eq!(p.effective_budget().http_timeout_secs(), 150);
        assert_eq!(
            p.effective_budget().http_timeout_secs(),
            settings.effective_llm_http_timeout_secs(),
            "position 2 (env du process): plafond reconstruit ≠ plafond fusionné — \
             c'est exactement l'écrasement que M2 décrit"
        );
        assert_eq!(
            p.effective_budget().agent_total_timeout_secs(),
            settings.effective_agent_total_timeout_secs(),
            "position 2 (env du process): enveloppe reconstruite ≠ enveloppe fusionnée"
        );
        assert_eq!(
            p.effective_max_tokens(),
            settings.llm_max_tokens,
            "position 2 (env du process): budget de sortie reconstruit ≠ fusionné — \
             c'est le monde H2 appliqué à la troisième clé"
        );

        // Position 1 — the per-agent `.env` beats the process env (mika#2218).
        std::fs::write(
            agent.join(".env"),
            format!(
                "{HTTP_TIMEOUT_ENV_VAR}=180\n{AGENT_TOTAL_TIMEOUT_ENV_VAR}=700\n\
                 {MAX_TOKENS_ENV_VAR}=8192\n"
            ),
        )
        .unwrap();
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::AgentDotenv);
        assert_eq!(p.max_tokens.source, BudgetSource::AgentDotenv);
        assert_eq!(p.effective_budget().http_timeout_secs(), 180);
        assert_eq!(p.effective_max_tokens(), 8_192);
        assert_eq!(
            p.effective_budget().http_timeout_secs(),
            settings.effective_llm_http_timeout_secs(),
            "position 1 (.env per-agent): l'ordre est INVERSÉ depuis mika#2218 — \
             une reconstruction qui suppose l'ordre usuel rapporte une provenance fausse"
        );
        assert_eq!(
            p.effective_budget().agent_total_timeout_secs(),
            settings.effective_agent_total_timeout_secs(),
            "position 1 (.env per-agent): enveloppe reconstruite ≠ enveloppe fusionnée"
        );
        assert_eq!(
            p.effective_max_tokens(),
            settings.llm_max_tokens,
            "position 1 (.env per-agent): budget de sortie reconstruit ≠ fusionné"
        );

        clean_budget_env();
    }

    /// mika#2280 — nothing anywhere carries `llm_max_tokens`, so the reported
    /// default must be the one `Settings` itself falls back to.
    ///
    /// The `Default` source is H3 applied to the third key: the `config.toml`
    /// was never read or never carried the key. Reporting a number that
    /// disagreed with the merged one would make probe (a) send an operator to
    /// the wrong remedy.
    #[test]
    #[serial]
    fn mika2280_absent_max_tokens_reports_the_same_default_settings_uses() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();

        assert_eq!(p.max_tokens.source, BudgetSource::Default);
        assert_eq!(p.max_tokens.raw, None);
        assert_eq!(p.effective_max_tokens(), settings.llm_max_tokens);

        clean_budget_env();
    }

    /// mika#2293 M11 — `default` must never absorb a value that lives in the
    /// process environment.
    ///
    /// `Settings::effective_llm_http_timeout_secs` falls back to
    /// `llm::http_timeout_secs()`, a second out-of-cascade read of the same
    /// variable. A reporter that inherited that shape would answer `default` for
    /// both "compiled-in constant" and "process env through another door" — the
    /// two worlds this module exists to separate.
    #[test]
    #[serial]
    fn mika2293_process_env_is_never_reported_as_default() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        // Nothing on disk at all: the only door left is the environment.
        // Safety: test-only env var, serialized by `#[serial]`.
        unsafe { std::env::set_var(HTTP_TIMEOUT_ENV_VAR, "300") };

        let p = BudgetProvenance::resolve(&global, &agent);
        assert_eq!(
            p.http.source,
            BudgetSource::ProcessEnv,
            "une valeur posée dans l'env du process ne doit jamais être rapportée `default`"
        );
        assert_eq!(p.http.raw.as_deref(), Some("300"));
        assert_eq!(p.effective_budget().http_timeout_secs(), 300);

        clean_budget_env();

        // And with the variable gone, `default` means the constant — H3's shape.
        let p = BudgetProvenance::resolve(&global, &agent);
        assert_eq!(p.http.source, BudgetSource::Default);
        assert_eq!(p.http.raw, None);
        assert_eq!(
            p.effective_budget().http_timeout_secs(),
            DEFAULT_HTTP_TIMEOUT_SECS,
            "H3 : rien nulle part → la constante compilée, à 120 s — la signature \
             même des coupures mesurées le 11/09"
        );
    }

    /// mika#2293 — an unreadable value is reported as such, not as a default.
    ///
    /// `value: None` is the state the startup guard refuses by name. Collapsing
    /// it into the default here would hide an operator typo behind a number
    /// that looks deliberate.
    #[test]
    #[serial]
    fn mika2293_unparseable_value_keeps_its_raw_and_reports_no_value() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = \"deux minutes\"\n",
        )
        .unwrap();

        let p = BudgetProvenance::resolve(&global, &agent);
        assert_eq!(p.http.source, BudgetSource::AgentConfig);
        assert_eq!(p.http.raw.as_deref(), Some("deux minutes"));
        assert_eq!(
            p.http.value, None,
            "une valeur non-parsable n'est pas un défaut : c'est une erreur de saisie, \
             et la garde de démarrage la refuse en la nommant"
        );

        clean_budget_env();
    }

    /// mika#2293 AC1 — a repetition is silent, a change is re-emitted.
    ///
    /// Tested on the deduplication primitive rather than on the log line: the
    /// contract is "which signatures are suppressed", and asserting on that is
    /// what distinguishes a working dedup from one that suppresses everything.
    #[test]
    #[serial]
    fn mika2293_dedup_silences_repetition_and_re_emits_a_change() {
        clean_budget_env();
        reset_dedup_for_test();
        let (_tmp, global, agent) = homes();
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n",
        )
        .unwrap();

        // Reads the emitter's own key rather than rebuilding it: a hand-written
        // copy here is what broke when mika#2362 extended the signature.
        //
        // mika#2457 — it now reads the emitter's own *record*. That is NOT a
        // control on `resolved_at`: the field is stamped to the second, so two
        // calls back to back almost always carry the same string. The leak is
        // pinned by `mika2457_resolved_at_dates_the_record_and_stays_out_of_the_dedup_key`,
        // which forces the two instants apart.
        let signature_now =
            || dedup_signature(&resolve_llm_budget_record("mika-arch", &global, &agent));

        let first = signature_now();
        log_llm_budget_resolved("mika-arch", &global, &agent);
        {
            let seen = LAST_EMITTED.get().unwrap().lock().unwrap();
            assert_eq!(seen.get("mika-arch"), Some(&first));
        }

        // Same pair again: nothing changes, so nothing is re-recorded.
        log_llm_budget_resolved("mika-arch", &global, &agent);
        assert_eq!(signature_now(), first, "le couple n'a pas bougé");

        // The pair changes — the whole reason the dedup is a map and not a set.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 300\nagent_total_timeout_secs = 900\n",
        )
        .unwrap();
        let second = signature_now();
        assert_ne!(first, second);
        log_llm_budget_resolved("mika-arch", &global, &agent);
        {
            let seen = LAST_EMITTED.get().unwrap().lock().unwrap();
            assert_eq!(
                seen.get("mika-arch"),
                Some(&second),
                "un budget qui change doit être ré-émis — c'est précisément \
                 l'information qu'on veut voir"
            );
        }

        // Back to the first pair: a set would stay silent here, a map re-emits.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n",
        )
        .unwrap();
        log_llm_budget_resolved("mika-arch", &global, &agent);
        {
            let seen = LAST_EMITTED.get().unwrap().lock().unwrap();
            assert_eq!(seen.get("mika-arch"), Some(&first));
        }

        reset_dedup_for_test();
        clean_budget_env();
    }

    /// mika#2280 AC4 — a geometry that moves **only** the output budget is
    /// re-emitted, and a repetition stays silent.
    ///
    /// This is mika#2362's lesson applied to the third key: the fields added by
    /// this ticket exist to report the output budget, so a key that ignored it
    /// would silence the event on the one change it was added for. Both
    /// controls are in the same call — a probe that only ever saw the silent
    /// side could not tell a working dedup from one that suppresses everything.
    #[test]
    #[serial]
    fn mika2280_a_change_in_max_tokens_alone_re_emits() {
        clean_budget_env();
        reset_dedup_for_test();
        let (_tmp, global, agent) = homes();

        let write_config = |max_tokens: u32| {
            std::fs::write(
                agent.join("config.toml"),
                format!(
                    "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n\
                     llm_max_tokens = {max_tokens}\n"
                ),
            )
            .unwrap();
        };
        let signature_now =
            || dedup_signature(&resolve_llm_budget_record("mika-arch", &global, &agent));

        write_config(8_192);
        let first = signature_now();
        log_llm_budget_resolved("mika-arch", &global, &agent);
        {
            let seen = LAST_EMITTED.get().unwrap().lock().unwrap();
            assert_eq!(seen.get("mika-arch"), Some(&first));
        }

        // Negative control: the identical pair stays silent.
        assert_eq!(signature_now(), first, "rien n'a bougé");

        // Positive control: the time pair is UNCHANGED, only the output budget
        // moves — precisely the change the new fields report.
        write_config(32_768);
        let second = signature_now();
        assert_ne!(
            first, second,
            "un changement qui ne bouge que llm_max_tokens doit ré-émettre : \
             sans ça, le champ neuf serait tu sur le seul changement qu'il décrit"
        );

        let before = BudgetProvenance::resolve(&global, &agent).effective_budget();
        assert_eq!(before.http_timeout_secs(), 240);
        assert_eq!(before.agent_total_timeout_secs(), 900);

        log_llm_budget_resolved("mika-arch", &global, &agent);
        {
            let seen = LAST_EMITTED.get().unwrap().lock().unwrap();
            assert_eq!(seen.get("mika-arch"), Some(&second));
        }

        reset_dedup_for_test();
        clean_budget_env();
    }

    /// mika#2362 AC5 — the reachability line fires on the incident's geometry
    /// and stays silent on the fleet's.
    ///
    /// Both controls in one call: a warning that fired for every agent would be
    /// muted within a week, and a probe that only ever observed the firing side
    /// could not tell the two apart.
    #[test]
    #[serial]
    fn mika2362_retry_unreachable_fires_on_the_incident_geometry_only() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Default)]
        struct Events(Arc<Mutex<Vec<String>>>);
        struct Layer(Arc<Mutex<Vec<String>>>);
        struct Visitor<'a>(&'a mut Vec<String>);

        impl tracing::field::Visit for Visitor<'_> {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                if f.name() == "event" {
                    self.0.push(format!("{v:?}").trim_matches('"').to_string());
                }
            }
            fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
                if f.name() == "event" {
                    self.0.push(v.to_string());
                }
            }
        }

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if let Ok(mut seen) = self.0.lock() {
                    event.record(&mut Visitor(&mut seen));
                }
            }
        }

        let collected = Events::default().0;
        let subscriber = tracing_subscriber::registry().with(Layer(Arc::clone(&collected)));
        let _guard = tracing::subscriber::set_default(subscriber);

        clean_budget_env();
        reset_dedup_for_test();
        let (_tmp, global, agent) = homes();

        // Negative control: the fleet geometry, whose second attempt is real.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 120\nagent_total_timeout_secs = 300\n",
        )
        .unwrap();
        log_llm_budget_resolved("fleet-agent", &global, &agent);
        assert!(
            !collected
                .lock()
                .unwrap()
                .iter()
                .any(|e| e == "llm_budget_retry_unreachable"),
            "120/300 reaches both its attempts and must not warn"
        );

        // Positive control: the mika#2362 incident's geometry.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 300\nagent_total_timeout_secs = 600\n",
        )
        .unwrap();
        log_llm_budget_resolved("incident-agent", &global, &agent);
        let seen = collected.lock().unwrap();
        assert!(
            seen.iter().any(|e| e == "llm_budget_retry_unreachable"),
            "300/600 cannot reach its second attempt and must say so: {seen:?}"
        );
        drop(seen);

        reset_dedup_for_test();
        clean_budget_env();
    }

    /// mika#2293 — the source names are a wire format.
    ///
    /// They land in `llm_budget_resolved` and an operator greps and groups by
    /// them. Two spellings of one source would split a population in half
    /// without saying so.
    #[test]
    fn mika2293_source_names_are_a_wire_format() {
        assert_eq!(BudgetSource::AgentDotenv.as_str(), "agent_dotenv");
        assert_eq!(BudgetSource::ProcessEnv.as_str(), "process_env");
        assert_eq!(BudgetSource::AgentConfig.as_str(), "agent_config");
        assert_eq!(BudgetSource::GlobalConfig.as_str(), "global_config");
        assert_eq!(BudgetSource::Default.as_str(), "default");
        // mika#2328 — the sixth word, on the model half only.
        assert_eq!(MODEL_SOURCE_UNKNOWN_PROVIDER, "unknown_provider");
    }

    /// mika#2328 V3 — the load-bearing test of the model half.
    ///
    /// Built on the gabarit of
    /// [`mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`]:
    /// the model this module reports must equal the model
    /// `Settings::load_for_agent` merges, on **each** of the four cascade
    /// positions plus the compiled default. A reconstruction that assumed the
    /// usual file-below-env order would report a *false* provenance, which this
    /// module holds to be strictly worse than none.
    #[test]
    #[serial]
    fn mika2328_model_reconstruction_equals_load_for_agent_on_every_cascade_position() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        let reported = |global: &std::path::Path, agent: &std::path::Path| {
            let p = ModelProvenance::resolve(global, agent);
            (
                p.provider,
                p.model.as_ref().map(|m| m.source),
                p.effective_model().map(str::to_string),
            )
        };
        let merged = |global: &std::path::Path, agent: &std::path::Path| {
            let s = Settings::load_for_agent(global, agent).unwrap();
            (s.llm_provider, s.active_llm_config().model)
        };

        // Position 5 — nothing declared anywhere: the compiled defaults.
        let (provider, source, model) = reported(&global, &agent);
        let (merged_provider, merged_model) = merged(&global, &agent);
        assert_eq!(provider, Some(DEFAULT_PROVIDER));
        assert_eq!(provider, Some(merged_provider));
        assert_eq!(source, Some(BudgetSource::Default));
        assert_eq!(
            model.as_deref(),
            Some(merged_model.as_str()),
            "position 5 (constante compilée) : modèle reconstruit ≠ modèle fusionné"
        );

        // Position 4 — global config.toml only.
        std::fs::write(
            global.join("config.toml"),
            "llm_provider = \"zai\"\nzai_model = \"glm-4.9\"\n",
        )
        .unwrap();
        let (provider, source, model) = reported(&global, &agent);
        let (merged_provider, merged_model) = merged(&global, &agent);
        assert_eq!(provider, Some(ProviderKind::ZAi));
        assert_eq!(provider, Some(merged_provider));
        assert_eq!(source, Some(BudgetSource::GlobalConfig));
        assert_eq!(model.as_deref(), Some(merged_model.as_str()));
        assert_eq!(model.as_deref(), Some("glm-4.9"));

        // Position 3 — per-agent config.toml beats the global one. This is the
        // door mika-qa's `zai_model` actually comes through.
        std::fs::write(
            agent.join("config.toml"),
            "llm_provider = \"zai\"\nzai_model = \"glm-5.2\"\n",
        )
        .unwrap();
        let (provider, source, model) = reported(&global, &agent);
        let (merged_provider, merged_model) = merged(&global, &agent);
        assert_eq!(provider, Some(merged_provider));
        assert_eq!(source, Some(BudgetSource::AgentConfig));
        assert_eq!(model.as_deref(), Some(merged_model.as_str()));
        assert_eq!(model.as_deref(), Some("glm-5.2"));

        // Position 2 — the process env beats both config files. This is H2: a
        // fleet-wide variable shadowing the per-agent file, and the reading
        // that tells an operator the repo is not authoritative here.
        // Safety: test-only env var, serialized by `#[serial]`.
        unsafe { std::env::set_var(model_env_var(ProviderKind::ZAi), "glm-5.3") };
        let (_, source, model) = reported(&global, &agent);
        let (_, merged_model) = merged(&global, &agent);
        assert_eq!(source, Some(BudgetSource::ProcessEnv));
        assert_eq!(
            model.as_deref(),
            Some(merged_model.as_str()),
            "position 2 (env du process) : c'est exactement l'écrasement que H2 décrit"
        );
        assert_eq!(model.as_deref(), Some("glm-5.3"));

        // Position 1 — the per-agent `.env` beats the process env (mika#2218).
        std::fs::write(
            agent.join(".env"),
            format!("{}=glm-5.4\n", model_env_var(ProviderKind::ZAi)),
        )
        .unwrap();
        let (_, source, model) = reported(&global, &agent);
        let (_, merged_model) = merged(&global, &agent);
        assert_eq!(source, Some(BudgetSource::AgentDotenv));
        assert_eq!(
            model.as_deref(),
            Some(merged_model.as_str()),
            "position 1 (.env per-agent) : l'ordre est INVERSÉ depuis mika#2218"
        );
        assert_eq!(model.as_deref(), Some("glm-5.4"));

        clean_budget_env();
    }

    /// mika#2328 — the model key follows the provider, on both its doors.
    ///
    /// Empirical rather than tabulated: for **every** provider, the key and the
    /// env var this module derives are the ones `Settings` actually reads. A
    /// hard-coded or mistyped name does not fail loudly — it reports `default`
    /// for a model that is in fact declared, i.e. a false provenance. Three
    /// providers (`kimi`, `qwen`, `zai`) are absent from `config::CONFIG_KEYS`
    /// altogether, so asserting against that registry would have proved less
    /// than this does.
    #[test]
    #[serial]
    fn mika2328_the_model_key_is_the_one_settings_reads_for_every_provider() {
        clean_budget_env();

        for provider in ProviderKind::ALL {
            let (_tmp, global, agent) = homes();
            let prefix = provider.config_prefix();

            // The config.toml door.
            std::fs::write(
                agent.join("config.toml"),
                format!(
                    "llm_provider = \"{prefix}\"\n{} = \"probe-from-config\"\n",
                    model_config_key(*provider)
                ),
            )
            .unwrap();
            let settings = Settings::load_for_agent(&global, &agent).unwrap();
            assert_eq!(
                settings.active_llm_config().model,
                "probe-from-config",
                "{prefix}: la clé dérivée n'est pas celle que Settings lit"
            );
            let p = ModelProvenance::resolve(&global, &agent);
            assert_eq!(p.provider, Some(*provider));
            assert_eq!(p.effective_model(), Some("probe-from-config"));
            assert_eq!(
                p.model.as_ref().map(|m| m.source),
                Some(BudgetSource::AgentConfig)
            );
            assert_eq!(p.model_config_key(), format!("{prefix}_model"));

            // The process-env door.
            // Safety: test-only env var, serialized by `#[serial]`.
            unsafe { std::env::set_var(model_env_var(*provider), "probe-from-env") };
            let settings = Settings::load_for_agent(&global, &agent).unwrap();
            assert_eq!(
                settings.active_llm_config().model,
                "probe-from-env",
                "{prefix}: la variable dérivée n'est pas celle que Settings lit — \
                 une provenance `default` serait rapportée pour un modèle bel et bien posé"
            );
            let p = ModelProvenance::resolve(&global, &agent);
            assert_eq!(p.effective_model(), Some("probe-from-env"));
            assert_eq!(
                p.model.as_ref().map(|m| m.source),
                Some(BudgetSource::ProcessEnv)
            );
            // Safety: test-only env var, serialized by `#[serial]`.
            unsafe { std::env::remove_var(model_env_var(*provider)) };
        }

        clean_budget_env();
    }

    /// mika#2328 — a fixed model key would report `default` for a declared model.
    ///
    /// The negative control of the test above: `zai_model` is on disk, the
    /// provider is `openrouter`, and the correct answer is that **openrouter's**
    /// model was never declared. A reader hard-coding `zai_model` would answer
    /// `agent_config` / `glm-5.2` here — confidently, and wrongly.
    #[test]
    #[serial]
    fn mika2328_a_model_key_of_another_provider_is_not_read() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();
        std::fs::write(
            agent.join("config.toml"),
            "llm_provider = \"openrouter\"\nzai_model = \"glm-5.2\"\n",
        )
        .unwrap();

        let p = ModelProvenance::resolve(&global, &agent);
        assert_eq!(p.provider, Some(ProviderKind::OpenRouter));
        assert_eq!(
            p.model.as_ref().map(|m| m.source),
            Some(BudgetSource::Default)
        );
        assert_eq!(
            p.effective_model(),
            Some(ProviderKind::OpenRouter.default_model()),
            "aucune clé openrouter n'est déclarée : le défaut du provider s'applique"
        );
        assert_eq!(p.model_config_key(), "openrouter_model");

        clean_budget_env();
    }

    /// mika#2328 — an unreadable provider yields no model provenance at all.
    ///
    /// Not `default`, which would state "no door carried the model" — unknown,
    /// and possibly false. Unreachable in production (`Settings::load_for_agent`
    /// refuses the value outright, so the agent never starts), which is checked
    /// here too: this reader must stay readable precisely where that one aborts.
    #[test]
    #[serial]
    fn mika2328_an_unreadable_provider_is_never_reported_as_a_default_model() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();
        std::fs::write(
            agent.join("config.toml"),
            "llm_provider = \"zaii\"\nzai_model = \"glm-5.2\"\n",
        )
        .unwrap();

        assert!(
            Settings::load_for_agent(&global, &agent).is_err(),
            "l'état est inatteignable en production — Settings refuse ce fichier"
        );

        let p = ModelProvenance::resolve(&global, &agent);
        assert_eq!(p.provider, None);
        assert_eq!(p.model, None);
        assert_eq!(p.effective_model(), None);
        assert_eq!(p.model_source_name(), MODEL_SOURCE_UNKNOWN_PROVIDER);
        assert_eq!(
            p.provider_name(),
            "zaii",
            "le champ `provider` rend la valeur qui a échoué, jamais un repli \
             présenté comme une lecture"
        );
        assert_eq!(p.model_config_key(), "");

        clean_budget_env();
    }

    /// mika#2328 V4 — a model change is re-emitted, not deduplicated away.
    ///
    /// The motif of mika#2362: an out-of-repo model swap moves neither timeout,
    /// so a signature blind to the model would silence the one line that
    /// reports the swap — on exactly the event that exists to report it.
    #[test]
    #[serial]
    fn mika2328_dedup_re_emits_when_only_the_model_changes() {
        clean_budget_env();
        reset_dedup_for_test();
        let (_tmp, global, agent) = homes();

        let write_model = |model: &str| {
            std::fs::write(
                agent.join("config.toml"),
                format!(
                    "llm_provider = \"zai\"\nzai_model = \"{model}\"\n\
                     llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n"
                ),
            )
            .unwrap();
        };
        let signature_now =
            || dedup_signature(&resolve_llm_budget_record("mika-qa", &global, &agent));

        write_model("glm-5.2");
        let before = signature_now();
        let budget_before = BudgetProvenance::resolve(&global, &agent);

        write_model("glm-5.3");
        let after = signature_now();

        assert_eq!(
            budget_before,
            BudgetProvenance::resolve(&global, &agent),
            "le couple de timeouts n'a pas bougé — c'est tout l'intérêt du cas"
        );
        assert_ne!(
            before, after,
            "un swap de modèle hors dépôt doit ré-émettre la ligne"
        );

        // And the emitter honours it: the recorded signature follows.
        log_llm_budget_resolved("mika-qa", &global, &agent);
        {
            let seen = LAST_EMITTED.get().unwrap().lock().unwrap();
            assert_eq!(seen.get("mika-qa"), Some(&after));
        }

        reset_dedup_for_test();
        clean_budget_env();
    }

    /// mika#2457 U1 — the record carries the same resolution as the two
    /// provenance readers, on **every** position of the cascade.
    ///
    /// This is the test that makes the record honest rather than merely
    /// well-typed. `ResolvedBudgetRecord` flattens `BudgetProvenance` and
    /// `ModelProvenance` into strings a route can serve; a field wired to the
    /// wrong source would compile, would serve, and would answer the one
    /// question this ticket exists to settle — *through which door?* — with
    /// authority and wrongly. Those two readers are themselves pinned against
    /// `Settings::load_for_agent` by the two `*_equals_load_for_agent_*` tests
    /// above, so chaining to them transitively anchors the record to what the
    /// runtime actually merges.
    ///
    /// Four positions, because a record that only ever met `agent_config` would
    /// pass while reporting `agent_config` unconditionally.
    #[test]
    #[serial]
    fn mika2457_the_record_equals_the_provenance_readers_on_every_cascade_position() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        let check = |position: &str| {
            let record = resolve_llm_budget_record("mika-arch", &global, &agent);
            let provenance = BudgetProvenance::resolve(&global, &agent);
            let model = ModelProvenance::resolve(&global, &agent);
            let budget = provenance.effective_budget();

            assert_eq!(
                record.http_timeout_secs,
                budget.http_timeout_secs(),
                "{position}: plafond du record ≠ plafond résolu"
            );
            assert_eq!(
                record.agent_total_timeout_secs,
                budget.agent_total_timeout_secs(),
                "{position}: enveloppe du record ≠ enveloppe résolue"
            );
            assert_eq!(
                record.llm_max_tokens,
                provenance.effective_max_tokens(),
                "{position}: budget de sortie du record ≠ budget résolu"
            );
            assert_eq!(
                record.http_source,
                provenance.http.source.as_str(),
                "{position}: provenance du plafond perdue en traversant le record"
            );
            assert_eq!(
                record.total_source,
                provenance.agent_total.source.as_str(),
                "{position}: provenance de l'enveloppe perdue"
            );
            assert_eq!(
                record.max_tokens_source,
                provenance.max_tokens.source.as_str(),
                "{position}: provenance du budget de sortie perdue"
            );
            assert_eq!(
                record.http_raw,
                provenance.http.raw_or_empty(),
                "{position}: brut du plafond perdu"
            );
            assert_eq!(
                record.provider,
                model.provider_name(),
                "{position}: provider perdu"
            );
            assert_eq!(
                record.provider_source,
                model.provider_value.source.as_str(),
                "{position}: provenance du provider perdue"
            );
            assert_eq!(
                record.model,
                model.effective_model().unwrap_or(""),
                "{position}: modèle perdu"
            );
            assert_eq!(
                record.model_source,
                model.model_source_name(),
                "{position}: provenance du modèle perdue"
            );
            assert_eq!(
                record.model_config_key,
                model.model_config_key(),
                "{position}: clé de config du modèle perdue"
            );
            assert_eq!(record.agent_id, "mika-arch");
            assert!(
                !record.resolved_at.is_empty(),
                "{position}: un record non daté ne dit pas de quand il est vrai"
            );
            record
        };

        // Position 5 — nothing declared anywhere: the compiled-in constants.
        let default_record = check("position 5 (défaut compilé)");
        assert_eq!(default_record.http_source, "default");
        assert_eq!(default_record.http_raw, "", "rien n'a été lu");

        // Position 4 — global config.toml only.
        std::fs::write(
            global.join("config.toml"),
            "llm_http_timeout_secs = 130\nagent_total_timeout_secs = 400\n\
             llm_max_tokens = 2048\nllm_provider = \"openrouter\"\n\
             openrouter_model = \"global/model\"\n",
        )
        .unwrap();
        let global_record = check("position 4 (config.toml global)");
        assert_eq!(global_record.http_source, "global_config");
        assert_eq!(
            global_record.model_source, "global_config",
            "un modèle venant du fichier partagé ne doit pas se rapporter per-agent"
        );

        // Position 3 — per-agent config.toml beats the global one.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n\
             llm_max_tokens = 32768\nllm_provider = \"openrouter\"\n\
             openrouter_model = \"moonshotai/kimi-k2.5\"\n",
        )
        .unwrap();
        let agent_record = check("position 3 (config.toml per-agent)");
        assert_eq!(agent_record.http_source, "agent_config");
        assert_eq!(agent_record.http_timeout_secs, 240);
        assert_eq!(agent_record.model, "moonshotai/kimi-k2.5");
        assert_eq!(agent_record.model_config_key, "openrouter_model");

        // Position 2 — the process env beats both files. This is H2, the world
        // the mika#2457 probe exists to separate from the other two.
        // Safety: test-only env vars, serialized by `#[serial]`.
        unsafe {
            std::env::set_var(HTTP_TIMEOUT_ENV_VAR, "300");
        }
        let env_record = check("position 2 (env du process)");
        assert_eq!(
            env_record.http_source, "process_env",
            "une variable de service qui écrase le config.toml doit se dire"
        );
        assert_eq!(env_record.http_timeout_secs, 300);
        assert_eq!(
            env_record.agent_total_timeout_secs, 900,
            "l'enveloppe n'a pas bougé — la provenance est par clé, pas par couple"
        );
        assert_eq!(env_record.total_source, "agent_config");

        // Position 1 — the per-agent `.env` beats the process env (mika#2218).
        std::fs::write(agent.join(".env"), format!("{HTTP_TIMEOUT_ENV_VAR}=420\n")).unwrap();
        let dotenv_record = check("position 1 (.env per-agent)");
        assert_eq!(dotenv_record.http_source, "agent_dotenv");
        assert_eq!(dotenv_record.http_timeout_secs, 420);

        clean_budget_env();
    }

    /// mika#2457 U1 — the record has **one** construction site.
    ///
    /// A source scan, because a behavioural test cannot see this class: a second
    /// constructor would make no assertion fail the day it is written. It would
    /// be a resolver free to diverge from this one — silently, later, with every
    /// test still green. That is the class `grooming_marker` (mika#2158) had to
    /// close after promotion and dispatch routing answered the same question
    /// differently for months, and the reason the plan's Fire-Disposition puts
    /// this detector under option (a) rather than calling it redundant.
    ///
    /// Scope: this crate, which owns the type, plus `mika-agent`, the one
    /// consumer that holds a record and could be tempted to assemble its own in
    /// `init_agent`. `mika-agent` is reached by relative path and **skipped when
    /// absent** (a crates.io checkout of `mika-common` alone), so this half is
    /// best-effort — the this-crate half below is not, and is asserted
    /// non-vacuous.
    ///
    /// **Allowlist shipped empty, and it is not a slot**: when this fires, the
    /// second constructor is removed, not listed.
    #[test]
    fn mika2457_the_record_has_a_single_construction_site() {
        use crate::source_guard::ProductionScanner;

        const ALLOWED: &[&str] = &[];
        const NEEDLE: &str = "ResolvedBudgetRecord {";

        let collect = |scanner: &ProductionScanner, label: &str| {
            let mut hits: Vec<String> = Vec::new();
            scanner.for_each(|path, source| {
                let rel = path
                    .strip_prefix(scanner.src_root())
                    .unwrap_or(path)
                    .display()
                    .to_string();
                if ALLOWED.contains(&rel.as_str()) {
                    return;
                }
                for (idx, line) in source.lines().enumerate() {
                    // A doc-comment naming the type is not a construction of it.
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("//") {
                        continue;
                    }
                    // Two shapes carry the needle without constructing
                    // anything, and both are excluded by their own syntax
                    // rather than by a path allowlist: the declaration
                    // (`pub struct ResolvedBudgetRecord {`) and a function's
                    // return type followed by its body brace
                    // (`) -> ResolvedBudgetRecord {`). A *real* construction
                    // inside such a function still sits on its own line and is
                    // caught, so neither exclusion widens the guard.
                    if trimmed.starts_with("pub struct") || trimmed.starts_with("struct") {
                        continue;
                    }
                    if line.contains("-> ResolvedBudgetRecord {") {
                        continue;
                    }
                    if line.contains(NEEDLE) {
                        hits.push(format!("{label}/{rel}:{}", idx + 1));
                    }
                }
            });
            hits
        };

        let own = ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        assert!(
            own.files().len() > 5,
            "le scan doit voir ce crate, sinon il est vide de sens"
        );
        let mut hits = collect(&own, "mika-common");

        let agent_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("mika-agent")
            .join("src");
        if agent_src.is_dir() {
            hits.extend(collect(&ProductionScanner::new(&agent_src), "mika-agent"));
        }

        assert_eq!(
            hits.len(),
            1,
            "le record doit avoir UN seul constructeur (`resolve_llm_budget_record`) : \
             un second serait libre d'en diverger, sans qu'aucune assertion ne rougisse \
             le jour où il est écrit. Quand ce scan tire, on retire le second \
             constructeur — on ne l'allowliste pas.\nsites trouvés : {hits:?}"
        );
    }

    /// mika#2457 U1 — `resolved_at` dates the record and never keys it.
    ///
    /// The only control on that key. `resolved_at` is stamped to the second, so
    /// two resolutions back to back almost always carry the same string and a
    /// signature that included it would still compare equal — green over the
    /// leak, red only when a run happened to straddle a second boundary. The
    /// second record is therefore the first one **re-dated by hand**, and the
    /// test first asserts the two instants differ (the negative control)
    /// before asserting the signatures are still equal. It also asserts the field is
    /// populated: a record nobody can date is the ambiguity § 6 exists to
    /// resolve.
    #[test]
    #[serial]
    fn mika2457_resolved_at_dates_the_record_and_stays_out_of_the_dedup_key() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n",
        )
        .unwrap();

        let first = resolve_llm_budget_record("mika-arch", &global, &agent);
        // Re-dated by hand rather than resolved a second time: the clock would
        // almost always hand back the same second, and the control would be
        // a flake instead of a guard.
        let mut second = first.clone();
        second.resolved_at = "1970-01-01T00:00:00Z".to_string();

        assert!(
            chrono::DateTime::parse_from_rfc3339(&first.resolved_at).is_ok(),
            "resolved_at doit être un instant RFC 3339 lisible, pas une chaîne libre : {}",
            first.resolved_at
        );
        assert_ne!(
            first.resolved_at, second.resolved_at,
            "contrôle négatif : les deux enregistrements doivent porter deux instants \
             distincts, sinon l'égalité ci-dessous ne prouve rien"
        );
        assert_eq!(
            dedup_signature(&first),
            dedup_signature(&second),
            "deux résolutions identiques doivent rester UNE ligne : un resolved_at \
             dans la clé annulerait la déduplication que mika#2293 a posée"
        );

        clean_budget_env();
    }

    // -----------------------------------------------------------------------
    // mika#2473 U1 — the declared side, the comparison, and the line it emits
    // -----------------------------------------------------------------------

    /// One captured `tracing` event: its level and **every** field.
    ///
    /// Richer than the capture inside
    /// [`tests::mika2362_retry_unreachable_fires_on_the_incident_geometry_only`],
    /// which keeps the `event` name alone. AC3 of mika#2473 reads "the drift is
    /// said once, **with its door**": a probe that read the name and stopped
    /// would sign that sentence without ever looking at the door.
    #[derive(Debug, Clone)]
    struct CapturedEvent {
        level: tracing::Level,
        fields: HashMap<String, String>,
    }

    struct CaptureVisitor<'a>(&'a mut HashMap<String, String>);

    impl tracing::field::Visit for CaptureVisitor<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(
                field.name().to_string(),
                format!("{value:?}").trim_matches('"').to_string(),
            );
        }
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    struct CaptureLayer(std::sync::Arc<Mutex<Vec<CapturedEvent>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut fields = HashMap::new();
            event.record(&mut CaptureVisitor(&mut fields));
            if let Ok(mut seen) = self.0.lock() {
                seen.push(CapturedEvent {
                    level: *event.metadata().level(),
                    fields,
                });
            }
        }
    }

    /// Install a capturing subscriber on **this thread** and hand back its sink.
    ///
    /// `set_default` is thread-local, so a test holding the guard sees its own
    /// emissions and nobody else's — which is what makes the "emits nothing"
    /// arm of the AC3 test a real control rather than a race.
    fn capture_events() -> (
        tracing::subscriber::DefaultGuard,
        std::sync::Arc<Mutex<Vec<CapturedEvent>>>,
    ) {
        use tracing_subscriber::layer::SubscriberExt;

        let sink = std::sync::Arc::new(Mutex::new(Vec::new()));
        let subscriber =
            tracing_subscriber::registry().with(CaptureLayer(std::sync::Arc::clone(&sink)));
        (tracing::subscriber::set_default(subscriber), sink)
    }

    /// mika#2473 U1 / R2 — the declared side is read by the **same** key
    /// derivation as the resolved side, for every provider.
    ///
    /// The equivalence KTD6 rests on, pinned empirically rather than argued:
    /// write the constant's text into a virgin `agent_home` with no variable
    /// set, resolve it through [`ModelProvenance`], and the pair must be the
    /// one [`declared_model`] reads from the text alone. A second parser would
    /// be free to diverge from the first — and the divergence would surface as
    /// a *false* drift, which this module holds to be strictly worse than none.
    ///
    /// The padded-provider case at the end is not decoration: `from_layers`
    /// trims before `from_str`, so a constant written `llm_provider = " zai "`
    /// resolves fine through the cascade. A declared side that did not trim
    /// would answer `None` there — reported as `NotApplicable`, i.e. a guard
    /// silently switched off on a correct configuration.
    #[test]
    #[serial]
    fn mika2473_declared_model_equals_the_cascade_on_a_clean_home() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        for provider in ProviderKind::ALL {
            let prefix = provider.config_prefix();

            let with_model = format!("llm_provider = \"{prefix}\"\n{prefix}_model = \"probe\"\n");
            std::fs::write(agent.join("config.toml"), &with_model).unwrap();
            let cascade = ModelProvenance::resolve(&global, &agent);
            let declared = declared_model(&with_model)
                .unwrap_or_else(|| panic!("{prefix} : la constante déclare un couple lisible"));
            assert_eq!(
                Some(declared.provider),
                cascade.provider,
                "{prefix} : le fournisseur déclaré doit être celui que la cascade résout"
            );
            assert_eq!(
                Some(declared.model.as_str()),
                cascade.effective_model(),
                "{prefix} : le modèle déclaré doit être celui que la cascade résout"
            );
            assert!(
                !declared.model_is_provider_default,
                "{prefix} : un modèle écrit n'est pas le défaut du fournisseur"
            );

            let without_model = format!("llm_provider = \"{prefix}\"\n");
            std::fs::write(agent.join("config.toml"), &without_model).unwrap();
            let cascade = ModelProvenance::resolve(&global, &agent);
            let declared = declared_model(&without_model).unwrap();
            assert_eq!(
                declared.model,
                provider.default_model(),
                "{prefix} : sans ligne modèle, la règle de défaut est celle du fournisseur"
            );
            assert!(
                declared.model_is_provider_default,
                "{prefix} : et elle est dite, pas devinée par le lecteur"
            );
            assert_eq!(
                Some(declared.model.as_str()),
                cascade.effective_model(),
                "{prefix} : la même règle de défaut que la cascade applique"
            );
        }

        assert!(
            declared_model("llm_provider = \"nope\"\n").is_none(),
            "un fournisseur illisible ne se lit pas : même règle que ModelProvenance::from_layers"
        );
        assert_eq!(
            declared_model(" llm_provider = \" zai \"\n")
                .map(|d| (d.provider, d.model))
                .unwrap(),
            (
                ProviderKind::ZAi,
                ProviderKind::ZAi.default_model().to_string()
            ),
            "le fournisseur déclaré est trimmé avant from_str, comme from_layers le fait"
        );

        clean_budget_env();
    }

    /// mika#2473 U1 / R3, R4 — `compare` a trois bras, et un fournisseur
    /// illisible est une dérive rapportée, jamais absorbée.
    #[test]
    #[serial]
    fn mika2473_compare_has_three_arms_and_unknown_provider_is_a_drift() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        let constant =
            "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k2.5\"\n";
        std::fs::write(agent.join("config.toml"), constant).unwrap();
        let declared = declared_model(constant).unwrap();

        let record = resolve_llm_budget_record("mika-arch", &global, &agent);
        assert_eq!(
            ModelDriftCheck::compare(None, &record),
            ModelDriftCheck::NotApplicable,
            "un agent sans constante n'est pas comparable : il n'est pas « en phase »"
        );
        assert_eq!(
            ModelDriftCheck::compare(Some(&declared), &record),
            ModelDriftCheck::InSync {
                declared_provider: "openrouter".to_string(),
                declared_model: "moonshotai/kimi-k2.5".to_string(),
            },
            "constante et disque identiques : en phase"
        );

        std::fs::write(
            agent.join("config.toml"),
            "llm_provider = \"openrouter\"\nopenrouter_model = \"moonshotai/kimi-k3\"\n",
        )
        .unwrap();
        let drifted = resolve_llm_budget_record("mika-arch", &global, &agent);
        match ModelDriftCheck::compare(Some(&declared), &drifted) {
            ModelDriftCheck::Drift {
                declared_model,
                runtime_model,
                runtime_model_source,
                model_config_key,
                ..
            } => {
                assert_eq!(declared_model, "moonshotai/kimi-k2.5");
                assert_eq!(runtime_model, "moonshotai/kimi-k3");
                assert_eq!(
                    runtime_model_source, "agent_config",
                    "la dérive est rapportée AVEC sa porte"
                );
                assert_eq!(
                    model_config_key, "openrouter_model",
                    "et avec la clé qu'un opérateur éditerait"
                );
            }
            other => panic!("une édition du modèle sur disque est une dérive : {other:?}"),
        }

        std::fs::write(agent.join("config.toml"), "llm_provider = \"nope\"\n").unwrap();
        let unknown = resolve_llm_budget_record("mika-arch", &global, &agent);
        match ModelDriftCheck::compare(Some(&declared), &unknown) {
            ModelDriftCheck::Drift {
                runtime_provider,
                runtime_model,
                runtime_model_source,
                ..
            } => {
                assert_eq!(runtime_provider, "nope");
                assert_eq!(runtime_model, "");
                assert_eq!(
                    runtime_model_source, MODEL_SOURCE_UNKNOWN_PROVIDER,
                    "un modèle vide est une dérive dite unknown_provider, jamais absorbée en « en phase »"
                );
            }
            other => panic!("un fournisseur illisible est une dérive : {other:?}"),
        }

        clean_budget_env();
    }

    /// mika#2473 U1 / R3 — un modèle posé par l'environnement est une dérive,
    /// rapportée avec sa porte.
    ///
    /// Les deux contrôles dans le même appel (`a_probe_needs_both`) : sans le
    /// bras « sans la variable », le test ne distingue pas une garde qui voit
    /// la porte d'une garde qui rapporte `Drift` en toutes circonstances.
    #[test]
    #[serial]
    fn mika2473_a_model_set_by_env_is_a_drift_with_its_door() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        let constant = "llm_provider = \"zai\"\nzai_model = \"glm-5.2\"\n";
        std::fs::write(agent.join("config.toml"), constant).unwrap();
        let declared = declared_model(constant).unwrap();

        // Contrôle négatif : sans la variable, la constante est en phase.
        let clean = resolve_llm_budget_record("mika-qa", &global, &agent);
        assert!(
            matches!(
                ModelDriftCheck::compare(Some(&declared), &clean),
                ModelDriftCheck::InSync { .. }
            ),
            "sans variable d'environnement, le disque porte la constante : en phase"
        );

        // SAFETY: test-only, sérialisé par `#[serial]`, nettoyé en sortie.
        unsafe { std::env::set_var("MIKA_ZAI_MODEL", "glm-5.3") };
        let doored = resolve_llm_budget_record("mika-qa", &global, &agent);
        match ModelDriftCheck::compare(Some(&declared), &doored) {
            ModelDriftCheck::Drift {
                runtime_model,
                runtime_model_source,
                ..
            } => {
                assert_eq!(runtime_model, "glm-5.3");
                assert_eq!(
                    runtime_model_source, "process_env",
                    "une variable de service est une dérive au même titre qu'une édition du fichier"
                );
            }
            other => panic!("MIKA_ZAI_MODEL déplace le modèle en service : {other:?}"),
        }

        clean_budget_env();
    }

    /// mika#2473 U1 / AC3 — **le seul test qui observe l'émission**.
    ///
    /// Sans lui, « la dérive est dite une fois, avec sa porte » n'est attestée
    /// que par un `grep` post-déploiement, que le plan lui-même dit confondable
    /// avec un binaire périmé. Les trois bras, dont le troisième est le
    /// contrôle négatif : sans lui le test ne distingue pas « n'émet rien » de
    /// « émet toujours ».
    #[test]
    #[serial]
    fn mika2473_the_drift_line_carries_its_level_and_its_fields() {
        let (_guard, seen) = capture_events();

        emit_model_drift("mika-cli", &ModelDriftCheck::NotApplicable);
        assert!(
            seen.lock().unwrap().is_empty(),
            "contrôle négatif : un agent sans constante n'émet rien"
        );

        emit_model_drift(
            "mika-dev",
            &ModelDriftCheck::InSync {
                declared_provider: "zai".to_string(),
                declared_model: "glm-5.2".to_string(),
            },
        );
        {
            let events = seen.lock().unwrap();
            assert_eq!(events.len(), 1, "en phase : une ligne, pas zéro");
            let in_sync = &events[0];
            assert_eq!(
                in_sync.fields.get("event").map(String::as_str),
                Some("well_known_model_in_sync"),
                "une absence de WARN doit être distinguable d'une garde qui n'a pas tourné"
            );
            assert_eq!(in_sync.level, tracing::Level::INFO);
            assert_eq!(
                in_sync.fields.get("agent_id").map(String::as_str),
                Some("mika-dev")
            );
            assert_eq!(
                in_sync.fields.get("declared_by").map(String::as_str),
                Some(DECLARED_BY)
            );
        }

        emit_model_drift(
            "mika-arch",
            &ModelDriftCheck::Drift {
                declared_provider: "openrouter".to_string(),
                declared_model: "moonshotai/kimi-k2.5".to_string(),
                runtime_provider: "openrouter".to_string(),
                runtime_model: "moonshotai/kimi-k3".to_string(),
                runtime_model_source: "agent_config".to_string(),
                runtime_provider_source: "agent_config".to_string(),
                model_config_key: "openrouter_model".to_string(),
            },
        );
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 2, "la dérive ajoute UNE ligne");
        let drift = &events[1];
        assert_eq!(
            drift.level,
            tracing::Level::WARN,
            "la dérive est un WARN : un INFO se perdrait dans 19 Go de journal"
        );
        for (name, expected) in [
            ("event", "well_known_model_drift"),
            ("agent_id", "mika-arch"),
            ("declared_provider", "openrouter"),
            ("declared_model", "moonshotai/kimi-k2.5"),
            ("runtime_provider", "openrouter"),
            ("runtime_model", "moonshotai/kimi-k3"),
            ("runtime_model_source", "agent_config"),
            ("runtime_provider_source", "agent_config"),
            ("model_config_key", "openrouter_model"),
            ("declared_by", "well_known_agents.rs"),
        ] {
            assert_eq!(
                drift.fields.get(name).map(String::as_str),
                Some(expected),
                "champ {name} de R4"
            );
        }
    }
}
