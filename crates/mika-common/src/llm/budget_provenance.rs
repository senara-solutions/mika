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

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use super::budget::{
    AGENT_TOTAL_TIMEOUT_ENV_VAR, DEFAULT_AGENT_TOTAL_TIMEOUT_SECS, LlmTimeoutBudget,
};
use super::{DEFAULT_HTTP_TIMEOUT_SECS, HTTP_TIMEOUT_ENV_VAR, MIN_HTTP_TIMEOUT_SECS};

/// `config.toml` key for the per-call plafond.
pub const HTTP_TIMEOUT_CONFIG_KEY: &str = "llm_http_timeout_secs";
/// `config.toml` key for the per-agent envelope.
pub const AGENT_TOTAL_TIMEOUT_CONFIG_KEY: &str = "agent_total_timeout_secs";

/// Which door of the cascade a budget value came through.
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

/// The provenance of both halves of one agent's budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetProvenance {
    /// The per-call plafond (`llm_http_timeout_secs` / `MIKA_LLM_HTTP_TIMEOUT_SECS`).
    pub http: ResolvedBudgetValue,
    /// The per-agent envelope (`agent_total_timeout_secs` / `MIKA_AGENT_TOTAL_TIMEOUT_SECS`).
    pub agent_total: ResolvedBudgetValue,
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
        let layers = CascadeLayers::read(global_home, agent_home);
        Self {
            http: layers.resolve_key(HTTP_TIMEOUT_CONFIG_KEY, HTTP_TIMEOUT_ENV_VAR),
            agent_total: layers
                .resolve_key(AGENT_TOTAL_TIMEOUT_CONFIG_KEY, AGENT_TOTAL_TIMEOUT_ENV_VAR),
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

    /// Resolve one key through the four positions, highest priority first.
    fn resolve_key(&self, config_key: &str, env_var: &str) -> ResolvedBudgetValue {
        // 1. Per-agent `.env` — highest priority since mika#2218.
        if let Some(raw) = self.agent_dotenv.as_ref().and_then(|v| v.get(env_var)) {
            return ResolvedBudgetValue::from_raw(BudgetSource::AgentDotenv, raw);
        }

        // 2. Process environment.
        if let Ok(raw) = std::env::var(env_var)
            && !raw.trim().is_empty()
        {
            return ResolvedBudgetValue::from_raw(BudgetSource::ProcessEnv, &raw);
        }

        // 3. Per-agent `config.toml`.
        if let Some(raw) = self
            .agent_config
            .as_ref()
            .and_then(|t| config_key_as_string(t, config_key))
        {
            return ResolvedBudgetValue::from_raw(BudgetSource::AgentConfig, &raw);
        }

        // 4. Global `config.toml`.
        if let Some(raw) = self
            .global_config
            .as_ref()
            .and_then(|t| config_key_as_string(t, config_key))
        {
            return ResolvedBudgetValue::from_raw(BudgetSource::GlobalConfig, &raw);
        }

        ResolvedBudgetValue::compiled_default()
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
/// `openai::MAX_ATTEMPTS_HARD_CAP`, the ceiling every OpenAI-compatible rail
/// runs under — which is every well-known agent today.
const REPORTED_ATTEMPT_HARD_CAP: u32 = super::openai::MAX_ATTEMPTS_HARD_CAP;

/// Emit `llm_budget_resolved` for one agent — the line that answers "under
/// which plafond did this turn run?" (mika#2293 AC1).
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
pub fn log_llm_budget_resolved(agent_id: &str, global_home: &Path, agent_home: &Path) {
    let provenance = BudgetProvenance::resolve(global_home, agent_home);
    let budget = provenance.effective_budget();

    let signature = format!(
        "{}|{}|{}|{}",
        budget.http_timeout_secs(),
        budget.agent_total_timeout_secs(),
        provenance.http.source.as_str(),
        provenance.agent_total.source.as_str(),
    );

    let mut seen = LAST_EMITTED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if seen.get(agent_id) == Some(&signature) {
        return;
    }
    seen.insert(agent_id.to_string(), signature);
    drop(seen);

    tracing::info!(
        event = "llm_budget_resolved",
        agent_id,
        http_timeout_secs = budget.http_timeout_secs(),
        agent_total_timeout_secs = budget.agent_total_timeout_secs(),
        max_attempts = budget.max_attempts(REPORTED_ATTEMPT_HARD_CAP),
        worst_case_failure_secs = budget.worst_case_failure_secs(REPORTED_ATTEMPT_HARD_CAP),
        http_source = provenance.http.source.as_str(),
        total_source = provenance.agent_total.source.as_str(),
        http_raw = provenance.http.raw_or_empty(),
        total_raw = provenance.agent_total.raw_or_empty(),
        "resolved LLM timeout budget (mika#2293)"
    );
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

    /// Clear both budget variables from the process env.
    ///
    /// # Safety
    /// Test-only, serialized by `#[serial]`.
    fn clean_budget_env() {
        unsafe {
            std::env::remove_var(HTTP_TIMEOUT_ENV_VAR);
            std::env::remove_var(AGENT_TOTAL_TIMEOUT_ENV_VAR);
        }
    }

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
    /// per cascade position, checked on **both** budget keys: the day the order
    /// changes in `config.rs`, this goes red rather than letting the reported
    /// provenance drift away from the value actually in force.
    #[test]
    #[serial]
    fn mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position() {
        clean_budget_env();
        let (_tmp, global, agent) = homes();

        // Position 4 — global config.toml only.
        std::fs::write(
            global.join("config.toml"),
            "llm_http_timeout_secs = 130\nagent_total_timeout_secs = 400\n",
        )
        .unwrap();
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::GlobalConfig);
        assert_eq!(p.agent_total.source, BudgetSource::GlobalConfig);
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

        // Position 3 — per-agent config.toml beats the global one.
        std::fs::write(
            agent.join("config.toml"),
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n",
        )
        .unwrap();
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::AgentConfig);
        assert_eq!(p.effective_budget().http_timeout_secs(), 240);
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

        // Position 2 — the process env beats both config files. This is H2.
        // Safety: test-only env vars, serialized by `#[serial]`.
        unsafe {
            std::env::set_var(HTTP_TIMEOUT_ENV_VAR, "150");
            std::env::set_var(AGENT_TOTAL_TIMEOUT_ENV_VAR, "500");
        }
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::ProcessEnv);
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

        // Position 1 — the per-agent `.env` beats the process env (mika#2218).
        std::fs::write(
            agent.join(".env"),
            format!("{HTTP_TIMEOUT_ENV_VAR}=180\n{AGENT_TOTAL_TIMEOUT_ENV_VAR}=700\n"),
        )
        .unwrap();
        let p = BudgetProvenance::resolve(&global, &agent);
        let settings = Settings::load_for_agent(&global, &agent).unwrap();
        assert_eq!(p.http.source, BudgetSource::AgentDotenv);
        assert_eq!(p.effective_budget().http_timeout_secs(), 180);
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

        let signature_now = || {
            let p = BudgetProvenance::resolve(&global, &agent);
            let b = p.effective_budget();
            format!(
                "{}|{}|{}|{}",
                b.http_timeout_secs(),
                b.agent_total_timeout_secs(),
                p.http.source.as_str(),
                p.agent_total.source.as_str()
            )
        };

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
    }
}
