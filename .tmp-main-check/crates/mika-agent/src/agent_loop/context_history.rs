//! The conversation window's two bounds, resolved per tenant (mika#2425).
//!
//! `identity.toml`'s `[context.history]` declares a **role floor**: which rows a
//! turn's window may draw from (`scope`) and how large the result may be
//! (`max_tokens`). mika#2425 adds a per-tenant half in `customer_config`, and the
//! cascade between them can only ever **narrow**:
//!
//! ```text
//! role floor      = identity.toml [context.history]   (unchanged)
//! narrowing       = customer_config                    (new)
//! effective       = the NARROWER of the two
//! ```
//!
//! # Why narrowing only
//!
//! The asymmetry is not a precaution, it is the property that makes the key safe
//! at the site it lives. It is already written one file over, on this exact
//! field, for mika#1951: *"The caller may narrow this turn's scope to its own
//! session; it may never widen it. […] That asymmetry is why the wire key is a
//! bool: the widening request has no spelling."*
//!
//! Here it buys four things at once:
//!
//! 1. mika#2295 cannot be reopened from the database — nobody can put mika-arch
//!    back on `scope = agent`, which is the bound its role declares.
//! 2. Reversal is free. `delete_customer_config` **does not exist** (the table
//!    exposes `set_customer_config` only), so posing the neutral — `agent` /
//!    `none` — *is* the cancellation, and the rule is what makes that safe.
//! 3. The mika#1951 caller-side narrowing composes with it in the same
//!    direction, so the two asymmetries never have to be arbitrated.
//! 4. The day these keys become model-settable (a named follow-up), a model
//!    cannot widen its own window. The safety is structural, not conventional.
//!
//! Named cost: an operator cannot widen a declared window from the database.
//! That is refused **and said** (`context_history_widening_refused`); the remedy
//! is an edit to `well_known_agents.rs`, which is correct — the code declares
//! that bound as a property of the role.
//!
//! # Three measurements this module rests on
//!
//! - **R1** — `reconcile_well_known_identity` has never run against a customer
//!   agent: `provision_well_known_agents` loops over `WELL_KNOWN_AGENTS`, the
//!   four engineering agents. There is therefore no reconciler conflict to
//!   resolve for the population L8 measured — and there would be one only if we
//!   chose to write `identity.toml`, which is why we do not.
//! - **R2** — `load_agent_context` calls `prompt::load_identity_async` on **every
//!   turn**, and it is the funnel of all three loops. So this setting has no
//!   *not hot-swappable* contract: it takes effect on the next turn, with no
//!   restart, whichever door posed it. That is what makes a per-tenant knob
//!   usable at all on a daemon shared by a whole fleet.
//! - **R3** — on the Telegram path `scope = session` does not remove 90 %, it
//!   **empties**. `MessageRequest` carries no `session_id`, so
//!   `server::handlers` mints a fresh `Uuid::new_v4()` per inbound message
//!   unless the agent is singleton — and neither `DEFAULT_IDENTITY` nor
//!   `FAMILY_IDENTITY` declares `[session] singleton`. For such a tenant a
//!   "session" **is** a message. The L8 residual of ~330 tokens was measured on
//!   a population whose sessions carry several messages (CLI `mika chat`, A2A
//!   calls with a stable `--session-id`, singleton agents); nothing invalidates
//!   that measurement, and transporting its conclusion onto the Telegram
//!   population without establishing it is what this module refuses to do.
//!
//! R3 is why the shipped default is unchanged and why
//! [`ResolvedContextHistory::session_minting`] rides on the operator event: it
//! is the field that says, without reading the code, whether "session" means
//! *a conversation* or *a message* for this agent.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use tracing::{info, warn};

use crate::config_keys::{
    CONTEXT_HISTORY_MAX_TOKENS_KEY, CONTEXT_HISTORY_MAX_TOKENS_NONE, CONTEXT_HISTORY_SCOPE_KEY,
    parse_context_history_max_tokens, parse_context_history_scope,
};
use crate::prompt::{ContextHistoryConfig, HistoryScope};

/// Which door produced an effective bound.
///
/// **This is a wire format.** The three labels land in a log field operators
/// aggregate, so two spellings would split one population without saying so —
/// the motif `mika2131_filter_names_are_a_wire_format` states for the auto-pull
/// exclusion vocabulary. Pinned by `mika2425_source_names_are_a_wire_format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextHistorySource {
    /// The role's `identity.toml` declared a non-default bound.
    Identity,
    /// The tenant's `customer_config` narrowed it.
    CustomerConfig,
    /// Neither door posed anything — today's behaviour, unchanged.
    Default,
}

impl ContextHistorySource {
    /// Stable label for the log field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::CustomerConfig => "customer_config",
            Self::Default => "default",
        }
    }
}

/// What the database half asked for and did not get, on one axis.
///
/// Absent on the nominal path. Both variants keep the **raw** value so the
/// operator line can name it between quotes — a stray space in a hand-written
/// row is otherwise invisible (the rule `resolve_tenant_language` already
/// applies to its own unreadable tier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxisDiagnostic {
    /// The database asked for a window **wider** than the identity declares.
    /// Refused: the cascade narrows only.
    WideningRefused { requested: String },
    /// The database carried a value this reader cannot parse. The declared
    /// bound applies, exactly as if the key were absent.
    Unreadable { value: String },
}

/// Whether a "session" is a conversation or a single message, for this agent.
///
/// Rides on the operator event because it is what makes R3 readable without
/// reading the code: a `PerMessage` agent that posts `scope = session` does not
/// shed 90 % of its window, it sheds all of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMinting {
    /// `[session] singleton = true` — one canonical session, so a session is a
    /// whole conversation.
    Singleton,
    /// A fresh `Uuid::new_v4()` per inbound message (`server::handlers`), so a
    /// session **is** a message.
    PerMessage,
}

impl SessionMinting {
    /// Read the minting policy off an identity.
    #[must_use]
    pub fn of(identity: &crate::prompt::Identity) -> Self {
        if identity.session.singleton {
            Self::Singleton
        } else {
            Self::PerMessage
        }
    }

    /// Stable label for the log field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Singleton => "singleton",
            Self::PerMessage => "per_message",
        }
    }
}

/// The two bounds actually in force for a turn, with their provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContextHistory {
    /// Rows the window may draw from.
    pub scope: HistoryScope,
    /// Which door decided [`Self::scope`].
    pub scope_source: ContextHistorySource,
    /// Token ceiling, `None` meaning no ceiling.
    pub max_tokens: Option<usize>,
    /// Which door decided [`Self::max_tokens`].
    pub max_tokens_source: ContextHistorySource,
    /// What the database half asked for on the scope axis and did not get.
    pub scope_diagnostic: Option<AxisDiagnostic>,
    /// Same, on the ceiling axis.
    pub max_tokens_diagnostic: Option<AxisDiagnostic>,
    /// Whether a "session" is a conversation or a message here (R3).
    pub session_minting: SessionMinting,
}

/// Resolve the two bounds from the declared floor and the per-tenant narrowing.
///
/// **Pure: it emits nothing.** Every line — the provenance INFO and both WARNs —
/// is emitted by [`report_resolved`], behind one deduplication. That split is
/// deliberate and costs nothing: this function runs on **every turn**, so a
/// `warn!` posed here would write one line per turn for as long as a
/// misconfiguration lives, which is the churn mika#2131 bounds. Deduplicated,
/// a permanent misconfiguration costs one line and a *change* is re-announced —
/// and the purity is what lets the truth table be asserted at its boundaries
/// with no logger and no database.
///
/// Three tiers on each database read, the house shape: absent or empty → the
/// declared value; unreadable → the declared value **plus a diagnostic naming
/// the raw value**; valid → the narrower of the two. **No tier can fail the
/// turn**: this function returns no `Result`, which is the structural form of
/// "an unreadable setting is not an outage".
#[must_use]
pub fn resolve(
    declared: &ContextHistoryConfig,
    db_scope: Option<&str>,
    db_max_tokens: Option<&str>,
    session_minting: SessionMinting,
) -> ResolvedContextHistory {
    let (narrowing, scope_unreadable) = read_axis(db_scope, parse_context_history_scope);
    let (ceiling, ceiling_unreadable) = read_axis(db_max_tokens, parse_context_history_max_tokens);

    // mika#2425 — the one decisional `match` on `HistoryScope` outside
    // `agent_loop/mod.rs`, and the reason `mika2305_the_scope_has_a_single_decisional_reader`
    // counts three sites rather than two.
    //
    // Written as a frank `match` on purpose. `matches!(scope,
    // HistoryScope::Session)` would slip past that scan's positional predicate
    // WITHOUT changing the semantics, i.e. satisfy the guard while dropping the
    // invariant it stands for. A reviewer who finds this shape verbose must read
    // this paragraph before simplifying it.
    //
    // The three arms stay on one line each, deliberately: the scan groups arms
    // within two lines as ONE reader, and a rustfmt-wrapped middle arm splits
    // this into two sites — which reads as two competing answers to "which rows
    // may this window draw from?", the exact thing the guard counts.
    let scope = match (declared.scope, narrowing) {
        (HistoryScope::Session, _) => HistoryScope::Session,
        (HistoryScope::Agent, Some(HistoryScope::Session)) => HistoryScope::Session,
        (HistoryScope::Agent, _) => HistoryScope::Agent,
    };

    // The provenance is a LABEL derived from the decision above, not a second
    // decision: it names the door, and changing it can never change which rows
    // the window draws from.
    //
    // It is an `if` chain rather than an arm of the `match` above for two
    // composing reasons. Folding it in makes each arm a tuple long enough for
    // rustfmt to wrap, which splits the one reader into two under the mika#2305
    // scan; and writing it as its own `match` on `(declared.scope, scope)` would
    // add a *second* positional `HistoryScope::` site, which is the same
    // violation seen from the other end. The three cases here are exhaustive
    // over the three arms above and cannot disagree with them.
    let scope_source = if declared.scope == HistoryScope::Session {
        ContextHistorySource::Identity
    } else if scope == HistoryScope::Session {
        ContextHistorySource::CustomerConfig
    } else {
        ContextHistorySource::Default
    };

    // `None` is infinity, so the narrower of the two is the `min` over the
    // present ones. A tie resolves to `Identity`: the identity already produced
    // that bound, and reporting the tenant as the deciding door would make the
    // provenance field answer "the narrowing took" about a no-op.
    let (max_tokens, max_tokens_source) = match (declared.max_tokens, ceiling) {
        (Some(floor), Some(Some(narrowed))) if narrowed < floor => {
            (Some(narrowed), ContextHistorySource::CustomerConfig)
        }
        (Some(floor), _) => (Some(floor), ContextHistorySource::Identity),
        (None, Some(Some(narrowed))) => (Some(narrowed), ContextHistorySource::CustomerConfig),
        (None, _) => (None, ContextHistorySource::Default),
    };

    ResolvedContextHistory {
        scope,
        scope_source,
        max_tokens,
        max_tokens_source,
        scope_diagnostic: scope_unreadable.or_else(|| {
            // An EXPLICIT `agent` against a declared `session` is a refused
            // widening; an ABSENT key is not. The distinction is the whole
            // point: somebody wrote a value and did not get it, which is worth
            // one line — whereas "no key" is the state of the entire fleet.
            matches!(
                (declared.scope, narrowing),
                (HistoryScope::Session, Some(HistoryScope::Agent))
            )
            .then(|| AxisDiagnostic::WideningRefused {
                requested: db_scope.unwrap_or_default().trim().to_string(),
            })
        }),
        max_tokens_diagnostic: ceiling_unreadable.or_else(|| {
            // Same rule on the ceiling axis: an explicit `none` (remove the
            // ceiling) or an explicit larger number, against a declared one.
            match (declared.max_tokens, ceiling) {
                (Some(_), Some(None)) => Some(CONTEXT_HISTORY_MAX_TOKENS_NONE.to_string()),
                (Some(floor), Some(Some(asked))) if asked > floor => Some(asked.to_string()),
                _ => None,
            }
            .map(|requested| AxisDiagnostic::WideningRefused { requested })
        }),
        session_minting,
    }
}

/// Read one database axis into `(parsed, unreadable diagnostic)`.
///
/// The outer `Option` separates *absent* (`None`) from *present* — the
/// distinction the widening rule above depends on. A present-but-unparseable
/// value yields `(None, Some(Unreadable))`, i.e. the declared bound applies and
/// the operator is told, which is the fail-open direction the whole module
/// takes: a setting nobody can read must not cost a turn.
fn read_axis<T>(
    raw: Option<&str>,
    parse: impl Fn(&str) -> Option<T>,
) -> (Option<T>, Option<AxisDiagnostic>) {
    let Some(value) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return (None, None);
    };
    match parse(value) {
        Some(parsed) => (Some(parsed), None),
        None => (
            None,
            Some(AxisDiagnostic::Unreadable {
                value: value.to_string(),
            }),
        ),
    }
}

/// Last resolved state this process announced, per agent.
///
/// Deduplication, not gating: an identical repetition is silent, a **change** is
/// re-emitted — so "the operator posted the narrowing at 14:02" is one readable
/// line rather than a needle in a per-turn stream. Keyed by agent because one
/// `mika-spirit` serves them all through this one free function. Same shape and
/// same reason as `TENANT_LANGUAGE_REPORTED` (mika#2247) and `LAST_EMITTED`
/// (mika#2293).
static CONTEXT_HISTORY_REPORTED: LazyLock<Mutex<HashMap<String, ResolvedContextHistory>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Announce the resolved state once per agent per distinct state (mika#2425 U4/U5).
///
/// **Ungated by any telemetry switch**, for the reason mika#2293 had to write
/// down for `llm_budget_resolved`: this is a *configuration* event, and it has
/// to stay readable precisely when call telemetry was cut to reduce noise. *A
/// setting one cannot observe is not a setting, it is a hope.*
///
/// The two WARNs ride the same deduplication as the INFO. That is what keeps a
/// refused widening — a permanent state, not an event — from writing one line
/// per turn on the agent it holds.
pub fn report_resolved(agent_id: &str, resolved: &ResolvedContextHistory) {
    // The guard is bound inside the `match` and therefore released at its end —
    // **the lock is never held across the emission below**. That is not a
    // stylistic accident: this map is shared by every agent of the process and
    // this function runs on every turn, so holding it across three logging
    // macros would serialise the fleet on the path every turn takes. Same rule
    // and same reason as `emit_llm_budget_resolved`'s `LAST_EMITTED`, which
    // mika#2473 had to correct after the fact.
    let changed = match CONTEXT_HISTORY_REPORTED.lock() {
        Ok(mut last) => match last.get(agent_id) {
            Some(previous) if previous == resolved => false,
            _ => {
                last.insert(agent_id.to_string(), resolved.clone());
                true
            }
        },
        // A poisoned mutex must not silence the announcement: repeating the
        // same state is noise, never announcing it is a blind spot (mika#2358).
        Err(_) => true,
    };

    if !changed {
        return;
    }

    info!(
        target: "mika::otel",
        agent_id = %agent_id,
        scope = crate::agent_loop::history_scope_label(resolved.scope),
        scope_source = resolved.scope_source.as_str(),
        max_tokens = ?resolved.max_tokens,
        max_tokens_source = resolved.max_tokens_source.as_str(),
        session_minting = resolved.session_minting.as_str(),
        event = "context_history_resolved",
        "conversation-window bounds resolved"
    );

    report_axis(
        agent_id,
        CONTEXT_HISTORY_SCOPE_KEY,
        resolved.scope_diagnostic.as_ref(),
    );
    report_axis(
        agent_id,
        CONTEXT_HISTORY_MAX_TOKENS_KEY,
        resolved.max_tokens_diagnostic.as_ref(),
    );
}

/// Emit the WARN an axis earned, if any.
fn report_axis(agent_id: &str, key: &str, diagnostic: Option<&AxisDiagnostic>) {
    match diagnostic {
        None => {}
        Some(AxisDiagnostic::WideningRefused { requested }) => warn!(
            target: "mika::otel",
            agent_id = %agent_id,
            key = %key,
            requested = %format!("{requested:?}"),
            event = "context_history_widening_refused",
            "the per-tenant setting asked for a WIDER window than identity.toml declares \
             — refused: this cascade narrows only. Widen the role's `[context.history]` \
             in well_known_agents.rs if the bound itself is wrong."
        ),
        Some(AxisDiagnostic::Unreadable { value }) => warn!(
            target: "mika::otel",
            agent_id = %agent_id,
            key = %key,
            value = %format!("{value:?}"),
            event = "context_history_value_unreadable",
            "the per-tenant setting could not be read — falling back to what \
             identity.toml declares, exactly as if the key were absent"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(scope: HistoryScope, max_tokens: Option<usize>) -> ContextHistoryConfig {
        ContextHistoryConfig { scope, max_tokens }
    }

    fn resolve_for(
        declared_cfg: &ContextHistoryConfig,
        db_scope: Option<&str>,
        db_max: Option<&str>,
    ) -> ResolvedContextHistory {
        resolve(declared_cfg, db_scope, db_max, SessionMinting::PerMessage)
    }

    /// **D2** — the three provenance labels are a wire format.
    ///
    /// They land in a log field operators aggregate, so two spellings of one
    /// door would split a population without saying so. Literal equalities, not
    /// a round-trip: a round-trip is satisfied by any consistent renaming, which
    /// is exactly the change that breaks somebody's saved `jq`.
    #[test]
    fn mika2425_source_names_are_a_wire_format() {
        assert_eq!(ContextHistorySource::Identity.as_str(), "identity");
        assert_eq!(
            ContextHistorySource::CustomerConfig.as_str(),
            "customer_config"
        );
        assert_eq!(ContextHistorySource::Default.as_str(), "default");
        assert_eq!(SessionMinting::Singleton.as_str(), "singleton");
        assert_eq!(SessionMinting::PerMessage.as_str(), "per_message");
    }

    /// **V1** — the four crossings of the scope truth table, and the door that
    /// decided each.
    #[test]
    fn mika2425_scope_truth_table() {
        let agent = declared(HistoryScope::Agent, None);
        let session = declared(HistoryScope::Session, None);

        let nominal = resolve_for(&agent, Some("agent"), None);
        assert_eq!(nominal.scope, HistoryScope::Agent);
        assert_eq!(nominal.scope_source, ContextHistorySource::Default);

        let narrowed = resolve_for(&agent, Some("session"), None);
        assert_eq!(narrowed.scope, HistoryScope::Session);
        assert_eq!(narrowed.scope_source, ContextHistorySource::CustomerConfig);

        let role = resolve_for(&session, Some("session"), None);
        assert_eq!(role.scope, HistoryScope::Session);
        assert_eq!(
            role.scope_source,
            ContextHistorySource::Identity,
            "when both doors say session, the role is the one that decided — \
             reporting the tenant would claim a narrowing that changed nothing"
        );

        let widening = resolve_for(&session, Some("agent"), None);
        assert_eq!(
            widening.scope,
            HistoryScope::Session,
            "`agent` in the database is the neutral and never releases a declared \
             `session` — otherwise mika#2295 could be reopened from a DB row"
        );
        assert_eq!(widening.scope_source, ContextHistorySource::Identity);
    }

    /// **V2 — negative control for V1.** Without the cascade, `(session, agent)`
    /// would resolve to `agent`.
    ///
    /// V1 alone would stay green if `resolve` returned a constant, or if it read
    /// the identity and ignored the database entirely. This asserts that both
    /// halves are actually consulted, in the two directions that distinguish
    /// them.
    #[test]
    fn mika2425_the_declared_floor_is_not_the_answer_on_its_own() {
        let agent = declared(HistoryScope::Agent, Some(12_000));

        // The database is read: the same declared value gives two answers.
        let untouched = resolve_for(&agent, None, None);
        let narrowed = resolve_for(&agent, Some("session"), Some("500"));
        assert_ne!(untouched.scope, narrowed.scope);
        assert_ne!(untouched.max_tokens, narrowed.max_tokens);

        // The identity is read: the same database half gives two answers.
        let session = declared(HistoryScope::Session, None);
        assert_ne!(
            resolve_for(&agent, None, None).scope,
            resolve_for(&session, None, None).scope
        );
    }

    /// **V3** — the ceiling is the `min`, with `None` as infinity.
    #[test]
    fn mika2425_max_tokens_takes_the_narrower() {
        let none = declared(HistoryScope::Agent, None);
        let eight_k = declared(HistoryScope::Agent, Some(8_000));

        let from_db = resolve_for(&none, None, Some("8000"));
        assert_eq!(from_db.max_tokens, Some(8_000));
        assert_eq!(
            from_db.max_tokens_source,
            ContextHistorySource::CustomerConfig
        );

        let identity_wins = resolve_for(&eight_k, None, Some("12000"));
        assert_eq!(identity_wins.max_tokens, Some(8_000));
        assert_eq!(
            identity_wins.max_tokens_source,
            ContextHistorySource::Identity
        );

        let neutral = resolve_for(&eight_k, None, Some("none"));
        assert_eq!(neutral.max_tokens, Some(8_000));
        assert_eq!(neutral.max_tokens_source, ContextHistorySource::Identity);

        let nothing = resolve_for(&none, None, Some("none"));
        assert_eq!(nothing.max_tokens, None);
        assert_eq!(nothing.max_tokens_source, ContextHistorySource::Default);

        let db_narrows = resolve_for(&eight_k, None, Some("2000"));
        assert_eq!(db_narrows.max_tokens, Some(2_000));
        assert_eq!(
            db_narrows.max_tokens_source,
            ContextHistorySource::CustomerConfig
        );
    }

    /// **V4** — with no database key, every shipped identity resolves exactly
    /// what it resolves today, on both axes.
    ///
    /// This is AC3's assertion. `source` must read `identity`/`default`, never
    /// `customer_config`: a provenance that credited the new door for an
    /// unchanged value would make S1 unreadable on the very probe that is
    /// supposed to prove nothing moved.
    #[test]
    fn mika2425_the_default_is_byte_identical() {
        let shipped = [
            // default / family / mika-dev / mika-qa / mika-test: no block at all
            (declared(HistoryScope::Agent, None), None, None),
            // mika-arch: the only shipped non-default (mika#2327)
            (
                declared(HistoryScope::Session, Some(8_000)),
                Some(HistoryScope::Session),
                Some(8_000),
            ),
        ];

        for (cfg, expected_scope, expected_max) in shipped {
            for db in [None, Some("")] {
                let r = resolve_for(&cfg, db, db);
                assert_eq!(r.scope, expected_scope.unwrap_or(HistoryScope::Agent));
                assert_eq!(r.max_tokens, expected_max);
                assert_ne!(
                    r.scope_source,
                    ContextHistorySource::CustomerConfig,
                    "with no key posed, the tenant door decided nothing"
                );
                assert_ne!(
                    r.max_tokens_source,
                    ContextHistorySource::CustomerConfig,
                    "with no key posed, the tenant door decided nothing"
                );
                assert!(r.scope_diagnostic.is_none());
                assert!(r.max_tokens_diagnostic.is_none());
            }
        }
    }

    /// **V6** — a widening is refused AND named, on both axes.
    ///
    /// The diagnostic distinguishes an **explicit** neutral from an absent key:
    /// somebody wrote a value and did not get it, which is worth a line; the
    /// whole fleet posing nothing is not.
    #[test]
    fn mika2425_widening_is_refused_and_said() {
        let session_8k = declared(HistoryScope::Session, Some(8_000));

        let r = resolve_for(&session_8k, Some("agent"), Some("12000"));
        assert_eq!(r.scope, HistoryScope::Session);
        assert_eq!(r.max_tokens, Some(8_000));
        assert_eq!(
            r.scope_diagnostic,
            Some(AxisDiagnostic::WideningRefused {
                requested: "agent".to_string()
            })
        );
        assert_eq!(
            r.max_tokens_diagnostic,
            Some(AxisDiagnostic::WideningRefused {
                requested: "12000".to_string()
            })
        );

        let lifted = resolve_for(&session_8k, None, Some("none"));
        assert_eq!(
            lifted.max_tokens_diagnostic,
            Some(AxisDiagnostic::WideningRefused {
                requested: "none".to_string()
            }),
            "`none` against a declared ceiling asks to remove it — a widening"
        );

        let absent = resolve_for(&session_8k, None, None);
        assert!(
            absent.scope_diagnostic.is_none() && absent.max_tokens_diagnostic.is_none(),
            "an ABSENT key is not a refused widening: that is the state of the \
             entire fleet and warning on it would bury the signal"
        );

        let equal = resolve_for(&session_8k, Some("session"), Some("8000"));
        assert!(
            equal.max_tokens_diagnostic.is_none(),
            "an equal ceiling narrows nothing and widens nothing"
        );
    }

    /// **V8** — an unreadable value falls back to the declared bound and says so.
    ///
    /// Reachable only through a write made outside the tool:
    /// `validate_config_value` refuses an out-of-domain value at the door.
    #[test]
    fn mika2425_unreadable_falls_back_to_the_declared_bound() {
        let session_8k = declared(HistoryScope::Session, Some(8_000));
        let agent_none = declared(HistoryScope::Agent, None);

        let r = resolve_for(&session_8k, Some("sesion"), Some("beaucoup"));
        assert_eq!(r.scope, HistoryScope::Session);
        assert_eq!(r.max_tokens, Some(8_000));
        assert_eq!(
            r.scope_diagnostic,
            Some(AxisDiagnostic::Unreadable {
                value: "sesion".to_string()
            })
        );
        assert_eq!(
            r.max_tokens_diagnostic,
            Some(AxisDiagnostic::Unreadable {
                value: "beaucoup".to_string()
            })
        );

        // And on a default identity, an unreadable value leaves today's behaviour.
        let r = resolve_for(&agent_none, Some("channel"), Some("0"));
        assert_eq!(r.scope, HistoryScope::Agent);
        assert_eq!(r.max_tokens, None);
        assert_eq!(r.scope_source, ContextHistorySource::Default);
        assert_eq!(
            r.max_tokens_diagnostic,
            Some(AxisDiagnostic::Unreadable {
                value: "0".to_string()
            }),
            "`0` is refused by the parser, so it lands in the unreadable tier \
             rather than emptying the window"
        );
    }

    /// The case-insensitive, trimmed house convention holds on both axes.
    #[test]
    fn mika2425_values_are_trimmed_and_case_insensitive() {
        let agent = declared(HistoryScope::Agent, None);
        let r = resolve_for(&agent, Some("  SESSION "), Some(" 900 "));
        assert_eq!(r.scope, HistoryScope::Session);
        assert_eq!(r.max_tokens, Some(900));
        assert!(r.scope_diagnostic.is_none());
        assert!(r.max_tokens_diagnostic.is_none());
    }

    /// `session_minting` travels on the resolved state, because R3 is what an
    /// operator must read before concluding anything about the gain.
    #[test]
    fn mika2425_session_minting_rides_the_resolved_state() {
        let agent = declared(HistoryScope::Agent, None);
        let per_message = resolve(&agent, Some("session"), None, SessionMinting::PerMessage);
        let singleton = resolve(&agent, Some("session"), None, SessionMinting::Singleton);

        assert_eq!(per_message.session_minting, SessionMinting::PerMessage);
        assert_eq!(singleton.session_minting, SessionMinting::Singleton);
        assert_eq!(
            per_message.scope, singleton.scope,
            "minting is reported, never decided upon: it changes what `session` \
             MEANS, not what the cascade resolves"
        );
        assert_ne!(
            per_message, singleton,
            "and it is part of the dedup signature, so a tenant that becomes \
             singleton re-announces instead of staying silent"
        );
    }
}
