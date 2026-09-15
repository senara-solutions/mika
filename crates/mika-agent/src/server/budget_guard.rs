//! Boot-time refusal of a half-configured LLM timeout budget (mika#2293).
//!
//! # The failure this closes
//!
//! `MIKA_DEV_CONFIG` and `MIKA_QA_CONFIG` carry no budget key at all, so
//! mika-dev and mika-qa run on the default envelope of 300 s. An operator who
//! follows the obvious remedy for mika#2293 — "raise the plafond" — and sets
//! `MIKA_LLM_HTTP_TIMEOUT_SECS=300` on the service **without** also raising
//! `MIKA_AGENT_TOTAL_TIMEOUT_SECS` gives those two agents `cap = 300 >=
//! envelope = 300`. `create_provider_with_budget` then refuses, and **no LLM
//! call succeeds for them ever again**.
//!
//! mika-arch, meanwhile, survives: its own `config.toml` sets the envelope to
//! 900, which no environment variable shadows, so it runs at 300/900 and keeps
//! answering. Two agents go silent while a third replies — the failure shape
//! that looks least like a configuration mistake and that gets blamed on the
//! provider most readily. Which is the exact misreading mika#2293 exists to
//! correct.
//!
//! # Why refuse startup rather than warn
//!
//! A fleet that boots only to discover at the first call that half of it cannot
//! speak is a worse state than a refusal carrying the agent's name and the key
//! to fix. `llm/budget.rs` already states the doctrine —
//! *"a `mika` that starts is not proof that its budgets are valid; the first
//! call is"* — but it only holds the cold path, not the boot. This guard is the
//! boot half.
//!
//! **Pre-existing violations get no grace period, and that costs nothing.** An
//! agent already carrying an invalid pair could not make an LLM call anyway
//! (the provider constructor refuses it). The guard does not break something
//! that worked: it moves an existing failure from the first call to startup,
//! and makes it legible on the way.
//!
//! # What it does not do
//!
//! It corrects no value and invents none. An invalid pair is an operator error;
//! repairing it silently would reproduce the very defect this ticket
//! instruments.
//!
//! It also does not remove the mika#1660 panic in `llm::http_timeout_secs()`.
//! That path keeps its cold-path role; this guard **precedes** it, reading the
//! raw values through `BudgetProvenance` — which never panics — so a
//! below-floor plafond is reported as `agent mika-qa: …` instead of aborting the
//! process with a message that names no agent and no door of the cascade. A
//! binary that reaches that path without passing here (the `mika` CLI, say)
//! still panics exactly as before.

use std::path::Path;

use anyhow::{Result, bail};
use mika_common::llm::budget_provenance::{
    AGENT_TOTAL_TIMEOUT_CONFIG_KEY, HTTP_TIMEOUT_CONFIG_KEY,
};
use mika_common::llm::{
    BudgetProvenance, MIN_AGENT_TOTAL_TIMEOUT_SECS, MIN_HTTP_TIMEOUT_SECS, ResolvedBudgetValue,
};

/// `config.toml` key and env var for the plafond, for the "what to fix" line.
const HTTP_ENV_VAR: &str = "MIKA_LLM_HTTP_TIMEOUT_SECS";
/// `config.toml` key and env var for the envelope.
const TOTAL_ENV_VAR: &str = "MIKA_AGENT_TOTAL_TIMEOUT_SECS";

/// Assert that every agent this process will serve has a valid
/// `(per-call plafond, agent envelope)` pair.
///
/// Must run **after** `well_known_agents::provision_well_known_agents`: that
/// call is what writes the per-agent `config.toml` carrying the pair, and
/// scanning before it would validate the state of the previous boot.
///
/// Scans the agents present on disk rather than the `WELL_KNOWN_AGENTS` list.
/// That is a superset of what mika#2293 AC2 requires — every well-known agent
/// is on disk once provisioned — and it is the population `init_agent` will
/// actually build providers for, which is the population whose failure the
/// guard exists to pre-empt.
pub fn assert_llm_budgets_valid(home_dir: &Path) -> Result<()> {
    let mut offenders: Vec<String> = Vec::new();

    for agent_name in super::tier_guard::servable_agent_names(home_dir) {
        let agent_home = mika_common::home::resolve_agent_home(home_dir, &agent_name);
        let provenance = BudgetProvenance::resolve(home_dir, &agent_home);
        if let Some(reason) = first_violation(&provenance) {
            offenders.push(format!("  - {agent_name}: {reason}"));
        }
    }

    if offenders.is_empty() {
        return Ok(());
    }

    bail!(
        "invalid LLM timeout budget — refusing to start (mika#2293).\n\n\
         {}\n\n\
         The two numbers travel together: one LLM call must fit strictly inside \
         the agent's turn envelope, or the agent loop has no budget left for a \
         second step and every LLM call is refused at provider construction \
         (mika#2189). Starting anyway would let these agents go silent while \
         correctly-configured ones keep answering — a failure that reads like a \
         provider outage and is not one.\n\n\
         Fix, per agent, in ~/.mika/agents/<name>/config.toml:\n\
         \x20 {HTTP_TIMEOUT_CONFIG_KEY} = <seconds>   # strictly below the envelope, \
         at least {MIN_HTTP_TIMEOUT_SECS}\n\
         \x20 {AGENT_TOTAL_TIMEOUT_CONFIG_KEY} = <seconds>  # at least \
         {MIN_AGENT_TOTAL_TIMEOUT_SECS}\n\n\
         Prefer that file to the fleet-wide {HTTP_ENV_VAR} / {TOTAL_ENV_VAR}: a \
         variable in the service environment shadows every per-agent setting, \
         which is how a pair gets half-configured in the first place.",
        offenders.join("\n")
    );
}

/// The first thing wrong with this pair, in the order an operator can act on.
///
/// Order is deliberate: an unreadable value is reported as such before anything
/// is derived from it, and a below-floor value before containment — "you typed
/// something that is not a number of seconds" and "this number is too small to
/// mean anything" are more useful than "these two numbers do not fit".
fn first_violation(provenance: &BudgetProvenance) -> Option<String> {
    let http = &provenance.http;
    let total = &provenance.agent_total;

    if http.raw.is_some() && http.value.is_none() {
        return Some(unreadable(HTTP_TIMEOUT_CONFIG_KEY, HTTP_ENV_VAR, http));
    }
    if total.raw.is_some() && total.value.is_none() {
        return Some(unreadable(
            AGENT_TOTAL_TIMEOUT_CONFIG_KEY,
            TOTAL_ENV_VAR,
            total,
        ));
    }

    if let Some(secs) = http.value
        && secs < MIN_HTTP_TIMEOUT_SECS
    {
        return Some(format!(
            "per-call plafond is {secs}s, below the minimum of {MIN_HTTP_TIMEOUT_SECS}s \
             — a plafond this small aborts even a trivial LLM call (read from {}, \
             {HTTP_TIMEOUT_CONFIG_KEY}/{HTTP_ENV_VAR} = {:?})",
            http.source.as_str(),
            http.raw_or_empty()
        ));
    }

    // An envelope of `0` is not an error here: `Settings` falls back to the
    // default on it with a WARN, so the pair the agent actually runs under is
    // the default one. Reproduced rather than second-guessed — see
    // `BudgetProvenance::effective_budget`.
    let budget = provenance.effective_budget();
    if let Err(e) = budget.validate() {
        return Some(format!(
            "{e} (plafond read from {}, envelope from {})",
            http.source.as_str(),
            total.source.as_str()
        ));
    }

    None
}

fn unreadable(config_key: &str, env_var: &str, value: &ResolvedBudgetValue) -> String {
    format!(
        "{config_key} is not a number of seconds: {:?}, read from {} \
         (config key `{config_key}`, environment variable {env_var})",
        value.raw_or_empty(),
        value.source.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    /// Clear both budget variables from the process env.
    ///
    /// Not hygiene for its own sake: these tests assert on what the *cascade*
    /// resolves, and the process environment is one of its doors. Without this,
    /// a developer who has `MIKA_LLM_HTTP_TIMEOUT_SECS` exported — precisely the
    /// operator this ticket is about — would watch a healthy test go red for a
    /// reason having nothing to do with the code.
    ///
    /// # Safety
    /// Test-only, serialized by `#[serial]`.
    fn clean_budget_env() {
        unsafe {
            std::env::remove_var(HTTP_ENV_VAR);
            std::env::remove_var(TOTAL_ENV_VAR);
        }
    }

    /// Provision one agent on disk with the given `config.toml` body.
    fn home_with_agent(name: &str, config_toml: &str) -> TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agents").join(name);
        std::fs::create_dir_all(&agent_dir).unwrap();
        // `servable_agent_names` keys off `identity.toml`; the content is
        // irrelevant to this guard, only the presence.
        std::fs::write(agent_dir.join("identity.toml"), "name = \"test\"\n").unwrap();
        std::fs::write(agent_dir.join("config.toml"), config_toml).unwrap();
        tmp
    }

    /// mika#2293 AC2 — one test, both controls.
    ///
    /// A probe that can only observe the refusing side cannot tell "the guard
    /// rejects bad pairs" from "the guard rejects everything", which is the
    /// failure mode a guard is least able to notice about itself. So the valid
    /// pair and the invalid one are asserted in the same test.
    #[test]
    #[serial]
    fn mika2293_guard_accepts_a_valid_pair_and_refuses_an_invalid_one_by_name() {
        clean_budget_env();
        let ok = home_with_agent(
            "mika-arch",
            "llm_http_timeout_secs = 240\nagent_total_timeout_secs = 900\n",
        );
        assert!(
            assert_llm_budgets_valid(ok.path()).is_ok(),
            "un couple valide doit démarrer — sans ce contrôle, une garde qui \
             refuse tout passerait pour une garde qui marche"
        );

        // The founding half-configuration: plafond raised to the envelope.
        let bad = home_with_agent(
            "mika-qa",
            "llm_http_timeout_secs = 300\nagent_total_timeout_secs = 300\n",
        );
        let err = assert_llm_budgets_valid(bad.path())
            .expect_err("cap >= envelope doit refuser le démarrage");
        let msg = format!("{err}");
        assert!(
            msg.contains("mika-qa"),
            "le message doit nommer l'agent fautif — c'est toute la valeur de la \
             garde face à une panique qui n'en nomme aucun. Message : {msg}"
        );
        assert!(
            msg.contains("300"),
            "le message doit porter les deux valeurs. Message : {msg}"
        );
        assert!(
            msg.contains("agent_config"),
            "le message doit dire par quelle porte de la cascade la valeur est \
             entrée. Message : {msg}"
        );
    }

    /// mika#2293 F3 — the class that used to panic without naming anyone.
    ///
    /// `llm::http_timeout_secs()` aborts the process on a below-floor plafond
    /// with a message carrying neither the agent nor the source. The guard reads
    /// the raw value through the non-panicking reader and reports both.
    #[test]
    #[serial]
    fn mika2293_below_floor_plafond_is_named_instead_of_panicking() {
        clean_budget_env();
        let home = home_with_agent(
            "mika-dev",
            "llm_http_timeout_secs = 5\nagent_total_timeout_secs = 300\n",
        );
        let err = assert_llm_budgets_valid(home.path())
            .expect_err("un plafond sous le plancher doit refuser le démarrage");
        let msg = format!("{err}");
        assert!(msg.contains("mika-dev"), "message: {msg}");
        assert!(
            msg.contains(&MIN_HTTP_TIMEOUT_SECS.to_string()),
            "message: {msg}"
        );
    }

    /// mika#2293 — an unreadable value is named, not swallowed.
    #[test]
    #[serial]
    fn mika2293_unreadable_value_is_reported_with_its_raw_text() {
        clean_budget_env();
        let home = home_with_agent("mika-arch", "llm_http_timeout_secs = \"deux minutes\"\n");
        let err = assert_llm_budgets_valid(home.path())
            .expect_err("une valeur non-parsable doit refuser le démarrage");
        let msg = format!("{err}");
        assert!(msg.contains("deux minutes"), "message: {msg}");
        assert!(msg.contains("mika-arch"), "message: {msg}");
    }

    /// mika#2293 AC3 — the fleet default is untouched by this PR.
    ///
    /// An agent with no budget key at all runs 120/300, which the guard accepts.
    /// A guard that refused the shipped geometry would be a regression wearing a
    /// safety check's clothes.
    #[test]
    #[serial]
    fn mika2293_agent_with_no_budget_key_runs_the_shipped_default() {
        clean_budget_env();
        let home = home_with_agent("mika-relay", "llm_provider = \"anthropic\"\n");
        assert!(assert_llm_budgets_valid(home.path()).is_ok());
    }

    /// mika#2293 — an empty home has nothing to refuse.
    #[test]
    #[serial]
    fn mika2293_no_agents_is_not_a_violation() {
        clean_budget_env();
        let tmp = tempfile::tempdir().unwrap();
        assert!(assert_llm_budgets_valid(tmp.path()).is_ok());
    }
}
