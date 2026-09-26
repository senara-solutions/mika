//! Customer configuration key allowlist and per-key validation.
//!
//! Shared between the CLI `/config set` handler and the agent's
//! `set_config` tool so both enforce the same rules.
//!
//! # mika#2358 — the site of inscription for a frequency instruction
//!
//! Al asked for one technical digest a day and got three. The recurrence
//! registry said `0 0 9 * * *` — once a day, conforming. The surplus messages
//! came from the **heartbeat**, which wakes the agent up to three times a day
//! and knows nothing of any frequency instruction, and from the **curator
//! review** (handled in `task_engine::dispatcher`). Asked about it, Mika
//! promised a correction she had no mechanical way to carry out.
//!
//! `customer_config` is the site that fixes that, and it was chosen for one
//! measured property: **nothing rewrites it at startup**. Cancelling the
//! `heartbeat` row is undone by `revert_config_cancel_recurring_task`, whose
//! predicate is `status = 'cancelled'` with no discrimination of who cancelled
//! (mika#2271); editing `identity.toml` is exposed to the code-owned-section
//! reconciliation (mika#2330). This table has neither. And
//! `heartbeat_should_run` already reads it for the timezone, so the budget read
//! lands at the right place without a new access path.
//!
//! **Two keys, not one**, because Al's promise has two halves of different
//! natures: « plus aucun message de veille aujourd'hui » is a *dated
//! suspension*, « demain, un seul » is a *standing regime*. A budget alone
//! cannot express the first (setting it to 0 and trusting someone to raise it
//! tomorrow is a state that leaks); a pause alone cannot express the second.

use tracing::warn;

/// Config keys that may be set via agent tools or `/config set`.
///
/// **This constant is the tool surface.** `SetConfigTool::definition` builds its
/// `enum` and its description from it, so adding a key here makes it
/// discoverable by the model without a line of prompt — which is why mika#2358
/// adds no new tool (KTD2).
///
/// # The omission is the gesture (mika#2425 refus 2)
///
/// [`CONTEXT_HISTORY_SCOPE_KEY`] and [`CONTEXT_HISTORY_MAX_TOKENS_KEY`] are
/// **deliberately absent**. Because this constant *is* the surface, keeping a
/// key out of it requires writing nothing — and the refusal has a measurement
/// behind it: the only population mika#2425 measured is an **operator** need
/// (L8, 2026-09-19), not a user request. Inferring a conversational amnesia
/// from an ordinary sentence (« oublie ce qu'on a dit ») is a failure mode
/// nobody has measured, and the house rule is to measure before instructing.
/// Exposing them is a follow-up whose precondition is a measured user request
/// plus a green S2 probe on the population concerned.
///
/// The absence is a decision rather than an oversight, so it is asserted:
/// `mika2425_the_db_half_is_absent_from_the_tool_surface`.
pub const SETTABLE_CONFIG_KEYS: &[&str] = &[
    "chat_id",
    "timezone",
    "thinking_level",
    PROACTIVE_DAILY_BUDGET_KEY,
    PROACTIVE_PAUSE_UNTIL_KEY,
    TENANT_LANGUAGE_KEY,
];

/// Maximum number of **proactive wake-ups** allowed per day (mika#2358).
///
/// See [`resolve_proactive_daily_budget`] for the read and its three tiers, and
/// note the deliberate wording: this bounds *wake-ups*, not messages — see
/// that function's doc for why, and for what it costs.
pub const PROACTIVE_DAILY_BUDGET_KEY: &str = "proactive_daily_budget";

/// Instant (RFC 3339, UTC) until which no proactive wake-up may fire, or the
/// literal [`PROACTIVE_PAUSE_NONE`] to lift the pause (mika#2358).
///
/// It carries an instant and never a boolean, so it expires on its own and
/// cannot become a permanent silence nobody remembers arming.
pub const PROACTIVE_PAUSE_UNTIL_KEY: &str = "proactive_pause_until";

/// The value that lifts a pause. Chosen over an empty string because
/// `set_config` rejects an empty `value` before validation ever runs.
pub const PROACTIVE_PAUSE_NONE: &str = "none";

/// The language a tenant's thread is held in (mika#2247, AC2).
///
/// # Why `customer_config` and not an environment variable
///
/// The first draft of the mika#2247 plan proposed `MIKA_TENANT_LANGUAGE` plus a
/// `[locale]` section in `identity.toml`, on the mika#2290 model. Four
/// measurements refused that, and the fourth is eliminating:
///
/// 1. **The site already exists and already carries the exact neighbour.**
///    `timezone` lives here and is read by `agent_loop::load_agent_context` in
///    one line. Axis 3 of this very ticket reads that value; putting the
///    language beside it is one line against a whole mechanism (three-tier
///    parse, cache on `AgentState`, `identity.toml` section, threading from the
///    process environment).
/// 2. **[`SETTABLE_CONFIG_KEYS`] *is* the tool surface.** Being in that constant
///    is what puts a key in `set_config`'s schema enum — so the language becomes
///    settable by the model, and by the operator's `/config set`, without a line
///    of prompt and without a new tool.
/// 3. **mika#2358 settled this choice in writing, for a key of the same
///    nature.** See this module's header: `customer_config` is the only site
///    nothing rewrites at startup — a cancelled row is revived by
///    `revert_config_cancel_recurring_task` (mika#2271), an `identity.toml` edit
///    is exposed to the code-owned-section reconciliation (mika#2330).
/// 4. **Eliminating: nothing emits `MIKA_TENANT_LANGUAGE`, so axis 2 would ship
///    entirely inert.** The guard arms only on a *declared* language; with no
///    declaration it is `Unknown` and nothing is guarded. Setting the variable
///    is a `mika-cloud` gesture, outside this workspace — so the measured
///    tenant, the only population there is, would not be served by the delivery.
///    mika#2290 accepted that same dependency for its `cloud` signal, but it
///    could afford to: its guard 5d reads the outgoing text and closes its p1
///    *without* the variable. Here the guard depends on the value. Same shape,
///    opposite consequence.
///
/// # Hot-swap is the requirement, not a bonus
///
/// « Parle-moi en anglais » is an ordinary conversational request. A
/// non-hot-swappable axis would make it unexecutable — and mika#2358 measured
/// what an unexecutable setting request costs: Mika promises a correction she
/// has no way to apply, and guard 5e had to be written for it.
///
/// # The risk of this site, named rather than discovered
///
/// A guard armed by a value the model can itself write is not a guard against a
/// malicious model. It never claimed to be: it protects against **drift** — the
/// turn that flips to English while `fr` is posted. Changing the key
/// deliberately is an **act**, traced by the `audit_events` row `set_config`
/// already writes; flipping language mid-thread is a drift, and that is what is
/// refused. Same trade-off as `timezone`, model-settable since day one.
pub const TENANT_LANGUAGE_KEY: &str = "language";

/// The language of a tenant's thread (mika#2247).
///
/// Exactly `{fr, en}`, and the bound is the detector's
/// (`evidence::guards::detect_response_language_drift`): a value outside this
/// set is refused at the door rather than arming a guard that cannot measure
/// it. Any other language is out of scope and follow-up — the tenant simply
/// keeps today's behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantLanguage {
    /// French — the language `FAMILY_SOUL` prescribes.
    French,
    /// English.
    English,
}

impl TenantLanguage {
    /// Stable wire label, also the value an operator writes.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::French => "fr",
            Self::English => "en",
        }
    }

    /// The human name, for the `## Runtime` ground-truth line.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::French => "French",
            Self::English => "English",
        }
    }

    /// Parse a raw `customer_config` value. Case-insensitive and trimmed, the
    /// house convention for every key of this table.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fr" => Some(Self::French),
            "en" => Some(Self::English),
            _ => None,
        }
    }
}

/// Where a resolved tenant language came from (mika#2247).
///
/// Two values, and they name two opposite remedies — which is the whole reason
/// the provenance is reported at all, per mika#2293's rule that *a setting one
/// cannot observe is not a setting, it is a hope*. `Config` says the instruction
/// is in force, so a surviving symptom is somebody else's (read the compact
/// carve-out first); `Default` says the write never landed, so the cause is in
/// `set_config` or in the turn that should have called it — **not** in the
/// guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantLanguageSource {
    /// Read from `customer_config` and inside the domain.
    Config,
    /// Key absent, empty, or unreadable — nothing is posed and nothing is guarded.
    Default,
}

impl TenantLanguageSource {
    /// Stable wire label for the log field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

/// A resolved tenant language and its provenance (mika#2247).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTenantLanguage {
    /// `None` is the third state — nothing is posed in the prompt and the drift
    /// guard does not arm. It is the behaviour of every agent that declares
    /// nothing, which is every engineering agent and every tenant until an
    /// operator or the model writes the key.
    pub language: Option<TenantLanguage>,
    /// Which of the two tiers produced `language`.
    pub source: TenantLanguageSource,
}

impl ResolvedTenantLanguage {
    /// The value of the `language` field on `tenant_language_resolved`.
    ///
    /// `"unknown"` rather than an absent field: the event exists to answer
    /// "which language is in force for this tenant?", and *no language is in
    /// force* is an answer.
    #[must_use]
    pub fn language_label(&self) -> &'static str {
        self.language.map_or("unknown", TenantLanguage::as_str)
    }
}

/// Resolve the tenant language from a raw `customer_config` value (mika#2247).
///
/// # Three states, and absence poses nothing
///
/// Absent or empty → `None` + [`TenantLanguageSource::Default`]: no ground-truth
/// line in `## Runtime`, no drift guard. Unrecognised → the same, **with a WARN
/// naming the offending value between quotes** so a stray space is visible.
/// `fr` / `en` → the fact is posed and the guard arms.
///
/// **Absence is deliberately not "the language of the persona".** `FAMILY_SOUL`
/// hard-codes French, and mika#2023 already named in writing what that coding
/// costs: *"an anglophone champion gets the mirror image of the bug mika#2023
/// was filed for"*. Making absence an implicit French would write that defect at
/// a second site — and deriving it from the account locale is exactly what Prime
/// ruled out on 2026-09-09 ("a product choice wearing a technical default's
/// clothes"), a ruling mika#2290 has already had to carry over once.
///
/// The unreadable tier can only come from a write made **outside** the tool:
/// [`validate_config_value`] refuses an out-of-domain value at the door, so a
/// `set_config` call returns an error the model can correct within its turn.
#[must_use]
pub fn resolve_tenant_language(raw: Option<&str>) -> ResolvedTenantLanguage {
    let default = ResolvedTenantLanguage {
        language: None,
        source: TenantLanguageSource::Default,
    };

    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return default;
    };

    match TenantLanguage::parse(value) {
        Some(language) => ResolvedTenantLanguage {
            language: Some(language),
            source: TenantLanguageSource::Config,
        },
        None => {
            warn!(
                key = TENANT_LANGUAGE_KEY,
                value = %format!("{value:?}"),
                allowed = "fr, en",
                event = "tenant_language_unrecognized_value",
                "language is outside the supported set (fr, en) — posing nothing \
                 and arming no guard"
            );
            default
        }
    }
}

/// Daily proactive wake-up budget in force when the key is absent.
///
/// **This is the only site where the number lives.** It is the value the
/// literal `>= 3` in `heartbeat_should_run` used to carry, so a tenant that
/// sets nothing keeps exactly the behaviour it had before mika#2358 — the
/// negative control the Verification Contract requires.
pub const PROACTIVE_DAILY_BUDGET_DEFAULT: u32 = 3;

/// Upper bound of the budget's domain: the number of wake-ups an hourly cron
/// can produce in a day. Past it the value describes nothing the engine can do.
pub const PROACTIVE_DAILY_BUDGET_MAX: u32 = 24;

/// Where a resolved proactive setting came from.
///
/// Reported on `proactive_budget_resolved` for the reason mika#2293 had to
/// write down for `llm_budget_resolved`: a setting one cannot observe is not a
/// setting. `Config` at 1 means the instruction is in force and a surviving
/// symptom is somebody else's; `Default` means the write never landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactiveBudgetSource {
    /// Read from `customer_config` and inside the domain.
    Config,
    /// Key absent, empty, or unreadable — the constant applies.
    Default,
}

impl ProactiveBudgetSource {
    /// Stable wire label for the log field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

/// A resolved daily proactive budget and its provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedProactiveBudget {
    /// Maximum proactive wake-ups per day. `0` means none at all.
    pub budget: u32,
    /// Which of the two tiers produced `budget`.
    pub source: ProactiveBudgetSource,
}

/// Verdict of the dated-pause read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactivePause {
    /// No key, the literal `none`, or an instant already past.
    Inactive,
    /// Armed: no proactive wake-up until this instant.
    Until(chrono::DateTime<chrono::Utc>),
    /// Present but unreadable. **Fail-open** — the caller lets the wake-up
    /// through, consistent with every other read in `heartbeat_should_run`,
    /// each of which passes on a failed lookup.
    Unreadable,
}

/// Resolve the daily proactive wake-up budget from a raw `customer_config` value.
///
/// # Three tiers, and `0` is not an invalid value
///
/// Absent or empty → [`PROACTIVE_DAILY_BUDGET_DEFAULT`]. Unparseable or outside
/// `0..=`[`PROACTIVE_DAILY_BUDGET_MAX`] → the default **with a WARN naming the
/// offending value between quotes**. A valid `0` → honoured as "no proactive
/// wake-up at all".
///
/// That last distinction is load-bearing in both directions: `0` is the « plus
/// aucun message » lever, and confusing it with an error would make the cut
/// impossible; conversely a typo that silently muted a tenant's messages would
/// be exactly the class of silent failure this work exists to close.
///
/// The unreadable tier only covers a value written outside the tool
/// ([`validate_config_value`] refuses it at the door) — a direct DB edit, or an
/// inheritance from some future writer.
///
/// # What the number bounds, said plainly
///
/// `record_heartbeat_send` is called **unconditionally** after the silent turn,
/// including when the turn sent nothing, so `count_heartbeat_sends_today`
/// counts *wake-ups*. A budget of `1` therefore allows at most one heartbeat
/// wake-up plus the user's own recurrence: **two messages a day, not one**.
/// That is an honest bound and a measurable improvement (from four), not the
/// exact bound the word "frequency" suggests. Making the counter exact means
/// moving `record_heartbeat_send` behind an actual send, which would change the
/// hourly rate-limit's meaning at the same time — a separate ticket.
pub fn resolve_proactive_daily_budget(raw: Option<&str>) -> ResolvedProactiveBudget {
    let default = ResolvedProactiveBudget {
        budget: PROACTIVE_DAILY_BUDGET_DEFAULT,
        source: ProactiveBudgetSource::Default,
    };

    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return default;
    };

    match value.parse::<u32>() {
        Ok(n) if n <= PROACTIVE_DAILY_BUDGET_MAX => ResolvedProactiveBudget {
            budget: n,
            source: ProactiveBudgetSource::Config,
        },
        _ => {
            warn!(
                key = PROACTIVE_DAILY_BUDGET_KEY,
                value = %format!("{value:?}"),
                max = PROACTIVE_DAILY_BUDGET_MAX,
                default = PROACTIVE_DAILY_BUDGET_DEFAULT,
                event = "proactive_budget_invalid",
                "proactive_daily_budget is outside 0..=24 — falling back to the default"
            );
            default
        }
    }
}

/// Resolve the dated pause from a raw `customer_config` value.
///
/// `none` and an instant already past are both [`ProactivePause::Inactive`] —
/// the pause expires by itself, which is the whole reason it carries an instant
/// rather than a flag. An unreadable value is reported as
/// [`ProactivePause::Unreadable`] **and warned about**, so a pause that is not
/// biting is distinguishable from a pause that was never armed.
pub fn resolve_proactive_pause(
    raw: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> ProactivePause {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return ProactivePause::Inactive;
    };

    if value.eq_ignore_ascii_case(PROACTIVE_PAUSE_NONE) {
        return ProactivePause::Inactive;
    }

    match chrono::DateTime::parse_from_rfc3339(value) {
        Ok(dt) => {
            let until = dt.with_timezone(&chrono::Utc);
            if until > now {
                ProactivePause::Until(until)
            } else {
                ProactivePause::Inactive
            }
        }
        Err(_) => {
            warn!(
                key = PROACTIVE_PAUSE_UNTIL_KEY,
                value = %format!("{value:?}"),
                event = "proactive_pause_invalid",
                "proactive_pause_until is not an RFC 3339 instant nor `none` — \
                 letting the proactive wake-up through"
            );
            ProactivePause::Unreadable
        }
    }
}

// -- mika#2425: the per-tenant half of `[context.history]` ------------------

/// Per-tenant conversation-window scope — `agent` (neutral) or `session`.
///
/// # Why a DB key and not the `identity.toml` verb mika#2425 asked for
///
/// The ticket prescribed « un verbe CLI/console qui écrit la section nested
/// `[context.history]` de l'`identity.toml` ». That is the *means*, and three
/// measurements refused it:
///
/// 1. **No backend writes `identity.toml`.** `ConfigBackend::File` targets
///    `config.toml` and `write_config_toml` only poses **flat** keys. A nested
///    writer would have to be built whole — atomicity, permissions, and its
///    interaction with the fail-closed parse of `prompt::load_identity`, where
///    mika#2027 established that a corrupted `identity.toml` evicts *every*
///    skill of the agent. A buggy writer there costs the agent, not the setting.
/// 2. **The reconciler touches no tenant.** `WELL_KNOWN_AGENTS` is the four
///    engineering agents and `provision_well_known_agents` loops over that
///    constant alone, so `reconcile_well_known_identity` has never run against
///    a customer agent. The ticket's requirement #2 describes a population the
///    L8 measurement is not about — and satisfying it for the four well-known
///    agents would **invert** mika#2330, which wrote down in as many words that
///    a hand edit inside a code-owned section is now overwritten at the next
///    startup so that the loss is legible rather than silent.
/// 3. **`customer_config` is the site the house already chose for this class**,
///    with its reason written at the head of this module: *nothing rewrites it
///    at startup*. And `agent_loop::load_agent_context` already reads this table
///    twice, one line from `prompt::load_identity_async` — so the reader lands
///    where it belongs without a new access path.
///
/// # Narrowing only, and it is structural rather than conventional
///
/// The identity declares a **role floor**; this key may only make the window
/// narrower. `session` always wins, `agent` is the neutral and never releases a
/// `session` declared in identity. The same asymmetry is already written one
/// file over for mika#1951, on this exact field: *"The caller may narrow this
/// turn's scope to its own session; it may never widen it."*
///
/// It buys four things at once: mika#2295 cannot be reopened from the database
/// (nobody can put mika-arch back on `scope = agent`); reversal is free, which
/// matters because `delete_customer_config` **does not exist** — posing the
/// neutral *is* the cancellation; and the day these keys become model-settable,
/// a model cannot widen its own window.
///
/// Named cost: an operator cannot widen mika-arch's window from the database.
/// That is refused **and said** (`context_history_widening_refused`), and the
/// remedy is an edit to `well_known_agents.rs` — which is correct, since the
/// code declares that bound as a property of the role.
pub const CONTEXT_HISTORY_SCOPE_KEY: &str = "context_history_scope";

/// Per-tenant conversation-window token ceiling — `none` or an integer
/// `>= CONTEXT_HISTORY_MAX_TOKENS_FLOOR`.
///
/// The effective ceiling is the **narrower** of this and the identity's, with
/// `None` meaning "no ceiling". See [`CONTEXT_HISTORY_SCOPE_KEY`] for why the
/// cascade can only narrow.
pub const CONTEXT_HISTORY_MAX_TOKENS_KEY: &str = "context_history_max_tokens";

/// The neutral value of [`CONTEXT_HISTORY_SCOPE_KEY`] — the whole-agent window,
/// i.e. today's default and the value that cancels a narrowing.
pub const CONTEXT_HISTORY_SCOPE_AGENT: &str = "agent";

/// The narrowing value of [`CONTEXT_HISTORY_SCOPE_KEY`] — the turn's own session.
pub const CONTEXT_HISTORY_SCOPE_SESSION: &str = "session";

/// The neutral value of [`CONTEXT_HISTORY_MAX_TOKENS_KEY`] — no ceiling.
///
/// Chosen over an empty string for [`PROACTIVE_PAUSE_NONE`]'s reason:
/// `set_config` refuses an empty `value` before validation ever runs.
pub const CONTEXT_HISTORY_MAX_TOKENS_NONE: &str = "none";

/// Smallest ceiling this key accepts.
///
/// **`0` is refused here, and that is a decision rather than a range check.**
/// `Some(0)` is the omission sentinel of
/// `prompt::ContextHistoryConfig::max_tokens`: it empties the history entirely.
/// That is a **role** decision, carried by the identity, not a tenant
/// preference — and a `0` typed by mistake is, in the words
/// `deserialize_history_max_tokens` already carries, *a context wipe wearing a
/// configuration's clothes*. The floor sits well above the few hundred bytes a
/// single exchange costs, so a value that parses is a value that leaves a window.
pub const CONTEXT_HISTORY_MAX_TOKENS_FLOOR: usize = 500;

/// Read a raw [`CONTEXT_HISTORY_SCOPE_KEY`] value, `None` when unreadable.
///
/// Case-insensitive and trimmed, the house convention. Lives here rather than
/// in the resolver so the door (`validate_config_value`) and the reader
/// (`agent_loop::context_history`) share **one** definition of what the key
/// accepts — two spellings of a wire vocabulary is the class
/// `grooming_marker` had to close once (mika#2158).
#[must_use]
pub fn parse_context_history_scope(raw: &str) -> Option<crate::prompt::HistoryScope> {
    let value = raw.trim();
    if value.eq_ignore_ascii_case(CONTEXT_HISTORY_SCOPE_AGENT) {
        Some(crate::prompt::HistoryScope::Agent)
    } else if value.eq_ignore_ascii_case(CONTEXT_HISTORY_SCOPE_SESSION) {
        Some(crate::prompt::HistoryScope::Session)
    } else {
        None
    }
}

/// Read a raw [`CONTEXT_HISTORY_MAX_TOKENS_KEY`] value, `None` when unreadable.
///
/// `Some(None)` is the neutral (`none` — no ceiling); `Some(Some(n))` is a
/// ceiling of `n` tokens. The outer `None` means the value could not be read —
/// which includes `0` and anything under
/// [`CONTEXT_HISTORY_MAX_TOKENS_FLOOR`], refused for the reason written on that
/// constant.
#[must_use]
pub fn parse_context_history_max_tokens(raw: &str) -> Option<Option<usize>> {
    let value = raw.trim();
    if value.eq_ignore_ascii_case(CONTEXT_HISTORY_MAX_TOKENS_NONE) {
        return Some(None);
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= CONTEXT_HISTORY_MAX_TOKENS_FLOOR)
        .map(Some)
}

/// Validate a config value for the given key.
///
/// Returns `Ok(())` if the value is valid, or `Err(message)` describing what
/// is wrong with the value.
pub fn validate_config_value(key: &str, value: &str) -> Result<(), String> {
    if value.len() > 1000 {
        return Err("Config value too long (max 1000 characters)".to_string());
    }

    match key {
        "chat_id" => {
            if value.parse::<i64>().is_err() {
                return Err(format!(
                    "Invalid chat_id: {value}\nchat_id must be a numeric Telegram chat ID"
                ));
            }
        }
        "timezone" => {
            if value.parse::<chrono_tz::Tz>().is_err() {
                return Err(format!(
                    "Invalid timezone: {value}\nExample: Asia/Singapore, America/New_York, Europe/London"
                ));
            }
        }
        "thinking_level" => {
            if !matches!(value, "low" | "medium" | "med" | "high" | "off") {
                return Err(format!(
                    "Invalid thinking_level: {value}\nAllowed values: low, medium, med, high, off"
                ));
            }
        }
        PROACTIVE_DAILY_BUDGET_KEY => {
            let ok = value
                .parse::<u32>()
                .is_ok_and(|n| n <= PROACTIVE_DAILY_BUDGET_MAX);
            if !ok {
                return Err(format!(
                    "Invalid {PROACTIVE_DAILY_BUDGET_KEY}: {value}\nAllowed values: a whole \
                     number from 0 to {PROACTIVE_DAILY_BUDGET_MAX}. 0 means no proactive \
                     message at all; the default is {PROACTIVE_DAILY_BUDGET_DEFAULT}."
                ));
            }
        }
        TENANT_LANGUAGE_KEY => {
            if TenantLanguage::parse(value).is_none() {
                return Err(format!(
                    "Invalid {TENANT_LANGUAGE_KEY}: {value}\nAllowed values: `fr` or `en`. \
                     Leaving this unset keeps today's behaviour — no language is posed and \
                     none is enforced."
                ));
            }
        }
        CONTEXT_HISTORY_SCOPE_KEY => {
            if parse_context_history_scope(value).is_none() {
                return Err(format!(
                    "Invalid {CONTEXT_HISTORY_SCOPE_KEY}: {value}\nAllowed values: \
                     `{CONTEXT_HISTORY_SCOPE_SESSION}` (the window sees only this \
                     turn's session) or `{CONTEXT_HISTORY_SCOPE_AGENT}` (every session \
                     of this agent — the neutral, and how you cancel a narrowing). \
                     This setting can only narrow what identity.toml declares; it \
                     never widens it."
                ));
            }
        }
        CONTEXT_HISTORY_MAX_TOKENS_KEY => {
            if parse_context_history_max_tokens(value).is_none() {
                return Err(format!(
                    "Invalid {CONTEXT_HISTORY_MAX_TOKENS_KEY}: {value}\nAllowed values: \
                     `{CONTEXT_HISTORY_MAX_TOKENS_NONE}` (no ceiling — the neutral, and \
                     how you cancel a narrowing) or a whole number of at least \
                     {CONTEXT_HISTORY_MAX_TOKENS_FLOOR}. `0` is refused on purpose: it \
                     is the omission sentinel that empties the history, which is a role \
                     decision carried by identity.toml, not a tenant preference."
                ));
            }
        }
        PROACTIVE_PAUSE_UNTIL_KEY => {
            let ok = value.eq_ignore_ascii_case(PROACTIVE_PAUSE_NONE)
                || chrono::DateTime::parse_from_rfc3339(value).is_ok();
            if !ok {
                return Err(format!(
                    "Invalid {PROACTIVE_PAUSE_UNTIL_KEY}: {value}\nAllowed values: an RFC 3339 \
                     UTC instant such as 2026-09-20T00:00:00Z, or `{PROACTIVE_PAUSE_NONE}` to \
                     lift the pause."
                ));
            }
        }
        _ => {}
    }

    Ok(())
}

/// Check whether a key is in the settable allowlist.
pub fn is_settable_key(key: &str) -> bool {
    SETTABLE_CONFIG_KEYS.contains(&key)
}

/// Format the allowlist as a comma-separated string (for error messages).
pub fn settable_keys_display() -> String {
    SETTABLE_CONFIG_KEYS.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_contains_expected_keys() {
        assert!(is_settable_key("chat_id"));
        assert!(is_settable_key("timezone"));
        assert!(is_settable_key("thinking_level"));
        // mika#2358 — the allowlist IS the tool surface (KTD2): being here is
        // what puts these two in `set_config`'s schema enum and description.
        assert!(is_settable_key(PROACTIVE_DAILY_BUDGET_KEY));
        assert!(is_settable_key(PROACTIVE_PAUSE_UNTIL_KEY));
        // mika#2247 — same reason, and it is what makes AC2 settable on a live
        // tenant by a sentence in the conversation rather than by a redeploy.
        assert!(is_settable_key(TENANT_LANGUAGE_KEY));
        assert!(!is_settable_key("api_key"));
        assert!(!is_settable_key("db_path"));
    }

    // -- mika#2247: tenant language -----------------------------------------

    /// The three states, and the one that must pose nothing.
    #[test]
    fn mika2247_language_three_states() {
        let fr = resolve_tenant_language(Some("fr"));
        assert_eq!(fr.language, Some(TenantLanguage::French));
        assert_eq!(fr.source, TenantLanguageSource::Config);

        let en = resolve_tenant_language(Some("EN"));
        assert_eq!(
            en.language,
            Some(TenantLanguage::English),
            "the house convention is case-insensitive and trimmed"
        );
        assert_eq!(en.source, TenantLanguageSource::Config);

        for absent in [None, Some(""), Some("   ")] {
            let r = resolve_tenant_language(absent);
            assert_eq!(
                r.language, None,
                "absence must pose NOTHING: no ground-truth line, no guard armed. \
                 Making it an implicit French would write mika#2023's anglophone-champion \
                 defect at a second site."
            );
            assert_eq!(r.source, TenantLanguageSource::Default);
            assert_eq!(r.language_label(), "unknown");
        }

        // Unreadable is the *same* third state — it can only come from a write
        // made outside the tool, since `validate_config_value` refuses at the door.
        let bad = resolve_tenant_language(Some("fr-CA"));
        assert_eq!(bad.language, None);
        assert_eq!(bad.source, TenantLanguageSource::Default);
    }

    /// A value outside `{fr, en}` is refused by `set_config` itself, so the
    /// model can correct itself within its own turn instead of writing a row
    /// that silently arms nothing.
    #[test]
    fn mika2247_unknown_value_is_refused_at_the_door() {
        assert!(validate_config_value(TENANT_LANGUAGE_KEY, "fr").is_ok());
        assert!(validate_config_value(TENANT_LANGUAGE_KEY, "en").is_ok());
        assert!(validate_config_value(TENANT_LANGUAGE_KEY, "FR").is_ok());

        for rejected in ["es", "fr-CA", "french", "français", "fr en", "0"] {
            let err = validate_config_value(TENANT_LANGUAGE_KEY, rejected)
                .expect_err("{rejected} must be refused at the door");
            assert!(
                err.contains(TENANT_LANGUAGE_KEY) && err.contains("fr"),
                "the refusal must name the key and its domain, got: {err}"
            );
        }
    }

    /// The provenance is a wire format: it lands on `tenant_language_resolved`
    /// and an operator reads it to decide **which of two opposite remedies** to
    /// apply. Two spellings of one provenance would split that population
    /// without saying so. Model: `ProactiveBudgetSource::as_str` just above.
    #[test]
    fn mika2247_resolved_source_is_a_wire_format() {
        assert_eq!(TenantLanguageSource::Config.as_str(), "config");
        assert_eq!(TenantLanguageSource::Default.as_str(), "default");
        assert_eq!(TenantLanguage::French.as_str(), "fr");
        assert_eq!(TenantLanguage::English.as_str(), "en");
        assert_eq!(
            resolve_tenant_language(Some("fr")).language_label(),
            "fr",
            "the label an operator greps must be the value they wrote"
        );
    }

    #[test]
    fn test_validate_chat_id_valid() {
        assert!(validate_config_value("chat_id", "123456789").is_ok());
        assert!(validate_config_value("chat_id", "-987654321").is_ok());
    }

    #[test]
    fn test_validate_chat_id_invalid() {
        let err = validate_config_value("chat_id", "not-a-number").unwrap_err();
        assert!(err.contains("Invalid chat_id"));
    }

    #[test]
    fn test_validate_timezone_valid() {
        assert!(validate_config_value("timezone", "Asia/Singapore").is_ok());
        assert!(validate_config_value("timezone", "America/New_York").is_ok());
        assert!(validate_config_value("timezone", "Europe/London").is_ok());
        assert!(validate_config_value("timezone", "UTC").is_ok());
    }

    #[test]
    fn test_validate_timezone_invalid() {
        let err = validate_config_value("timezone", "Not/A/Timezone").unwrap_err();
        assert!(err.contains("Invalid timezone"));
    }

    #[test]
    fn test_validate_value_too_long() {
        let long = "x".repeat(1001);
        let err = validate_config_value("chat_id", &long).unwrap_err();
        assert!(err.contains("too long"));
    }

    #[test]
    fn test_validate_thinking_level_valid() {
        assert!(validate_config_value("thinking_level", "low").is_ok());
        assert!(validate_config_value("thinking_level", "medium").is_ok());
        assert!(validate_config_value("thinking_level", "high").is_ok());
        assert!(validate_config_value("thinking_level", "off").is_ok());
    }

    #[test]
    fn test_validate_thinking_level_invalid() {
        let err = validate_config_value("thinking_level", "max").unwrap_err();
        assert!(err.contains("Invalid thinking_level"));
    }

    #[test]
    fn test_settable_keys_display() {
        let display = settable_keys_display();
        assert!(display.contains("chat_id"));
        assert!(display.contains("timezone"));
        assert!(display.contains("thinking_level"));
        assert!(display.contains(PROACTIVE_DAILY_BUDGET_KEY));
        assert!(display.contains(PROACTIVE_PAUSE_UNTIL_KEY));
    }

    // -- mika#2358: proactive budget + pause --------------------------------

    fn t(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn mika2358_budget_domain_accepts_its_bounds() {
        for value in ["0", "1", "3", "24"] {
            assert!(
                validate_config_value(PROACTIVE_DAILY_BUDGET_KEY, value).is_ok(),
                "{value} should be inside 0..=24"
            );
        }
    }

    #[test]
    fn mika2358_budget_domain_refuses_outside_and_names_the_domain() {
        for value in ["25", "-1", "abc", "1.5", "", " ", "3 "] {
            let err = validate_config_value(PROACTIVE_DAILY_BUDGET_KEY, value)
                .expect_err("{value} should be refused");
            assert!(
                err.contains(PROACTIVE_DAILY_BUDGET_KEY),
                "error must name the key: {err}"
            );
            assert!(
                err.contains("0 to 24"),
                "error must name the domain, like the three pre-existing arms: {err}"
            );
        }
    }

    #[test]
    fn mika2358_pause_domain_accepts_none_and_an_rfc3339_instant() {
        assert!(validate_config_value(PROACTIVE_PAUSE_UNTIL_KEY, PROACTIVE_PAUSE_NONE).is_ok());
        assert!(validate_config_value(PROACTIVE_PAUSE_UNTIL_KEY, "NONE").is_ok());
        assert!(validate_config_value(PROACTIVE_PAUSE_UNTIL_KEY, "2026-09-20T00:00:00Z").is_ok());
        assert!(
            validate_config_value(PROACTIVE_PAUSE_UNTIL_KEY, "2026-09-20T07:00:00+07:00").is_ok(),
            "an offset instant is unambiguous and normalizes to UTC"
        );
    }

    #[test]
    fn mika2358_pause_domain_refuses_outside_and_names_the_domain() {
        for value in ["demain", "2026-09-20", "0", ""] {
            let err = validate_config_value(PROACTIVE_PAUSE_UNTIL_KEY, value)
                .expect_err("{value} should be refused");
            assert!(err.contains(PROACTIVE_PAUSE_UNTIL_KEY), "{err}");
            assert!(
                err.contains("RFC 3339"),
                "error must name the domain: {err}"
            );
            assert!(err.contains(PROACTIVE_PAUSE_NONE), "{err}");
        }
    }

    /// The three tiers of KTD7, including the one that matters: a valid `0` is
    /// honoured, and is distinguishable from an absent key.
    #[test]
    fn mika2358_budget_resolution_has_three_tiers_and_zero_is_not_an_error() {
        // Tier 1 — absent / empty.
        for raw in [None, Some(""), Some("   ")] {
            let r = resolve_proactive_daily_budget(raw);
            assert_eq!(r.budget, PROACTIVE_DAILY_BUDGET_DEFAULT);
            assert_eq!(r.source, ProactiveBudgetSource::Default);
        }

        // Tier 2 — unreadable or out of domain: the default, still `Default`.
        for raw in ["25", "-1", "abc", "1.5"] {
            let r = resolve_proactive_daily_budget(Some(raw));
            assert_eq!(r.budget, PROACTIVE_DAILY_BUDGET_DEFAULT, "raw = {raw}");
            assert_eq!(r.source, ProactiveBudgetSource::Default, "raw = {raw}");
        }

        // Tier 3 — valid, including the `0` that R3 asks for.
        let zero = resolve_proactive_daily_budget(Some("0"));
        assert_eq!(zero.budget, 0);
        assert_eq!(
            zero.source,
            ProactiveBudgetSource::Config,
            "a configured 0 must be distinguishable from an absent key — it is the \
             « plus aucun message » lever, not an error"
        );
        assert_eq!(resolve_proactive_daily_budget(Some("1")).budget, 1);
        assert_eq!(resolve_proactive_daily_budget(Some("24")).budget, 24);
    }

    #[test]
    fn mika2358_pause_resolution_covers_its_four_cases() {
        let now = t("2026-09-19T12:00:00Z");

        assert_eq!(resolve_proactive_pause(None, now), ProactivePause::Inactive);
        assert_eq!(
            resolve_proactive_pause(Some(PROACTIVE_PAUSE_NONE), now),
            ProactivePause::Inactive
        );
        assert_eq!(
            resolve_proactive_pause(Some("2026-09-19T08:00:00Z"), now),
            ProactivePause::Inactive,
            "a pause carries an instant so it expires on its own"
        );
        assert_eq!(
            resolve_proactive_pause(Some("2026-09-20T00:00:00Z"), now),
            ProactivePause::Until(t("2026-09-20T00:00:00Z"))
        );
        assert_eq!(
            resolve_proactive_pause(Some("demain"), now),
            ProactivePause::Unreadable,
            "unreadable is its own verdict: the caller fails open, but a pause that \
             is not biting must stay distinguishable from one never armed"
        );
    }

    // -- mika#2425: the per-tenant half of `[context.history]` --------------

    /// **D4** — the two keys are NOT reachable by the model.
    ///
    /// [`SETTABLE_CONFIG_KEYS`] is the `set_config` schema, so this absence is
    /// the entirety of mika#2425's refus 2. Without an assertion, a later
    /// addition would be invisible — a decision undone by a one-line diff that
    /// reads like a completion.
    #[test]
    fn mika2425_the_db_half_is_absent_from_the_tool_surface() {
        for key in [CONTEXT_HISTORY_SCOPE_KEY, CONTEXT_HISTORY_MAX_TOKENS_KEY] {
            assert!(
                !is_settable_key(key),
                "mika#2425 refus 2 — `{key}` must not be in SETTABLE_CONFIG_KEYS: that \
                 constant IS the `set_config` tool surface, so adding it there makes the \
                 key model-settable. The only measured population is an operator need \
                 (L8, 2026-09-19); exposing it needs a measured user request and a green \
                 S2 probe first. If you are landing that follow-up, delete this test in \
                 the same commit and say so."
            );
        }
    }

    /// The two keys are declared in `mika_common::config::CONFIG_KEYS`, on the
    /// `Database` backend.
    ///
    /// The key literal is written twice — here as a constant, there as a string
    /// — because `mika-common` cannot depend on this crate. That duplication is
    /// exactly what `lookup_config_key` resolves against, so a divergence would
    /// make `mika config set context_history_scope …` answer *Unknown config
    /// key* while every test in this file stayed green. Same shape and same
    /// reason as `mika2267_every_manager_env_const_is_declared_in_env_example`.
    #[test]
    fn mika2425_both_keys_are_declared_as_database_backed() {
        for key in [CONTEXT_HISTORY_SCOPE_KEY, CONTEXT_HISTORY_MAX_TOKENS_KEY] {
            let info = mika_common::config::lookup_config_key(key).unwrap_or_else(|| {
                panic!(
                    "mika#2425 — `{key}` is not declared in CONFIG_KEYS, so `mika config \
                     set {key}` answers \"Unknown config key\" whatever this module does"
                )
            });
            assert_eq!(
                info.backend,
                mika_common::config::ConfigBackend::Database,
                "mika#2425 — `{key}` must be Database-backed: the whole point of the site \
                 is that nothing rewrites `customer_config` at startup"
            );
            assert!(
                info.env_var.is_none(),
                "mika#2425 — `{key}` must have no env var: a fleet-wide variable would \
                 shadow the per-tenant setting, which is the defect mika#2293 measured \
                 for the timeout pair"
            );
        }
    }

    /// **V7** — `0` is refused at the door, `none` and the floor accepted.
    #[test]
    fn mika2425_max_tokens_refuses_zero_and_floors_at_500() {
        for accepted in ["none", "NONE", " none ", "500", "8000", "  8000"] {
            assert!(
                validate_config_value(CONTEXT_HISTORY_MAX_TOKENS_KEY, accepted).is_ok(),
                "{accepted:?} should be accepted"
            );
        }

        for rejected in [
            "0",
            "1",
            "499",
            "-1",
            "8000.5",
            "",
            "huit mille",
            "none please",
        ] {
            let err = validate_config_value(CONTEXT_HISTORY_MAX_TOKENS_KEY, rejected)
                .expect_err("{rejected:?} should be refused");
            assert!(
                err.contains(CONTEXT_HISTORY_MAX_TOKENS_NONE),
                "the refusal must name the neutral — an operator who meets it while \
                 mistyping is the operator who needs to know how to cancel: {err}"
            );
        }

        assert_eq!(
            parse_context_history_max_tokens("0"),
            None,
            "`Some(0)` is the omission sentinel that empties the history — a role \
             decision carried by identity.toml, never a tenant preference"
        );
        assert_eq!(parse_context_history_max_tokens("none"), Some(None));
        assert_eq!(parse_context_history_max_tokens("500"), Some(Some(500)));
    }

    /// The scope key accepts exactly the two names, and its refusal names the
    /// neutral.
    #[test]
    fn mika2425_scope_accepts_two_names_and_names_the_neutral() {
        for accepted in ["agent", "session", "AGENT", "Session", " session "] {
            assert!(
                validate_config_value(CONTEXT_HISTORY_SCOPE_KEY, accepted).is_ok(),
                "{accepted:?} should be accepted"
            );
        }

        for rejected in ["", "sesion", "channel", "none", "0", "agent session"] {
            let err = validate_config_value(CONTEXT_HISTORY_SCOPE_KEY, rejected)
                .expect_err("{rejected:?} should be refused");
            assert!(
                err.contains(CONTEXT_HISTORY_SCOPE_AGENT),
                "the refusal must name the neutral, which is how a narrowing is \
                 cancelled — `delete_customer_config` does not exist: {err}"
            );
        }

        assert_eq!(
            parse_context_history_scope("SESSION"),
            Some(crate::prompt::HistoryScope::Session)
        );
        assert_eq!(
            parse_context_history_scope(" agent "),
            Some(crate::prompt::HistoryScope::Agent)
        );
        assert_eq!(parse_context_history_scope("channel"), None);
    }
}
