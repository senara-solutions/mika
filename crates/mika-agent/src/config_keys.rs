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
pub const SETTABLE_CONFIG_KEYS: &[&str] = &[
    "chat_id",
    "timezone",
    "thinking_level",
    PROACTIVE_DAILY_BUDGET_KEY,
    PROACTIVE_PAUSE_UNTIL_KEY,
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
        assert!(!is_settable_key("api_key"));
        assert!(!is_settable_key("db_path"));
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
            assert!(err.contains("RFC 3339"), "error must name the domain: {err}");
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
}
