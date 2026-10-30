//! Boot-time refusal of the fabrication-detected P0 gate armed without an
//! escalation chat (mika#2871 AC3).
//!
//! # The failure this closes
//!
//! `MIKA_FABRICATION_P0_NOTIFY=true` arms the operator notification of the three
//! sites of the `fabrication-detected` class (`assert_grounded`,
//! `false_local_hosting_claim`, and the "accomplished state" half of
//! `unactioned_frequency_promise`). Where that notification *goes* is decided
//! elsewhere, by `MIKA_OPERATOR_ESCALATION_CHAT_ID`: when no escalation chat is
//! configured, mika#1381's decision D4 has a P0 **fall back to the normal chat**
//! rather than be dropped — with the `operator_escalation_chat_unset` WARN
//! naming it.
//!
//! That fallback is right for a single-channel operator workstation and wrong
//! for a tenant: the normal chat there is the *end user's* chat, so an armed
//! gate with no escalation chat sends « the machine fabricated a claim » to the
//! person the machine was talking to. Only the documentation required the pair;
//! nothing in the code did. This guard is that requirement.
//!
//! # Why refuse startup rather than resolve the gate to `false`
//!
//! Silently resolving an armed gate to `false` would **correct an operator's
//! value** — the operator asked for the notification, and would get a fleet that
//! boots, reports nothing, and never notifies, with no line saying why. Of the
//! two readings of a half-configured pair ("they meant to arm it" / "they meant
//! not to"), a guard cannot know which; refusing names both exits and lets the
//! operator pick. Same arbitration, same shape, as
//! [`super::budget_guard`]: an incoherent *combination* is refused at boot, with
//! the offending agent named and the two keys that repair it.
//!
//! # Why the predicate is not written here
//!
//! "Is this a usable P0 route?" is asked by the transport too, and the two must
//! not be free to disagree: `Some(0)` is the documented "no reply channel"
//! sentinel (mika#650/#1090) that `send()` short-circuits on, so an escalation
//! chat set to `0` loses every P0 exactly like an absent variable. A guard
//! testing `is_none()` alone would be **green** on that configuration. The one
//! answer lives at [`crate::messaging::p0_route_is_usable`].
//!
//! # What it does not do
//!
//! It lives in `run_server` only. The `mika` CLI keeps reading a misconfigured
//! install — refusing there would lock the operator out of the very commands
//! (`mika config get`, `mika agents budget`) that diagnose the refusal.
//!
//! It does not touch the six `warn!` detectors, which are the counting
//! instrument and stay armed independently (mika#2867), nor the gate's shipped
//! default (`false`), nor mika#1381's D4 fallback — a P0 on an install with no
//! escalation chat and the gate **disarmed** is still delivered to the normal
//! chat with its WARN.

use std::path::Path;

use anyhow::{Context, Result, bail};
use mika_common::config::Settings;

/// Env var arming the gate, for the "what to fix" line.
const NOTIFY_ENV_VAR: &str = "MIKA_FABRICATION_P0_NOTIFY";
/// Env var carrying the escalation chat.
const ESCALATION_ENV_VAR: &str = "MIKA_OPERATOR_ESCALATION_CHAT_ID";

/// Assert that no agent this process will serve has the fabrication-detected P0
/// gate armed without a usable escalation chat.
///
/// Must run **after** `well_known_agents::provision_well_known_agents`, for the
/// reason [`super::budget_guard::assert_llm_budgets_valid`] gives: that call is
/// what writes the per-agent `config.toml` this reads, so scanning before it
/// would validate the previous boot's state.
///
/// Reads through [`Settings::load_for_agent`] rather than the files directly —
/// the full cascade (per-agent `.env`, process env, per-agent `config.toml`,
/// global `config.toml`) is what the runtime itself resolves at `init_agent`, so
/// a guard reading anything narrower would be green on a pair the runtime sees
/// as armed.
///
/// A `Settings` that fails to load is **not** an arming violation: the error is
/// propagated with the agent's name attached, so "this agent is misconfigured on
/// another axis" never reads as "this agent armed the gate without a chat".
pub fn assert_fabrication_p0_arming_valid(home_dir: &Path) -> Result<()> {
    let mut offenders: Vec<String> = Vec::new();

    for agent_name in super::tier_guard::servable_agent_names(home_dir) {
        let agent_home = mika_common::home::resolve_agent_home(home_dir, &agent_name);
        let settings = Settings::load_for_agent(home_dir, &agent_home).with_context(|| {
            format!("failed to load settings for agent `{agent_name}` (mika#2871)")
        })?;

        if settings.fabrication_p0_notify
            && !crate::messaging::p0_route_is_usable(settings.operator_escalation_chat_id)
        {
            let read_value = match settings.operator_escalation_chat_id {
                None => "absent".to_string(),
                Some(id) => id.to_string(),
            };
            offenders.push(format!(
                "  - {agent_name}: fabrication_p0_notify = true, \
                 operator_escalation_chat_id = {read_value}"
            ));
        }
    }

    if offenders.is_empty() {
        return Ok(());
    }

    bail!(
        "fabrication-detected P0 notification armed without an escalation chat — \
         refusing to start (mika#2871).\n\n\
         {}\n\n\
         The two settings travel together. With the gate armed and no usable \
         escalation chat, a P0 falls back to the normal chat (mika#1381 D4) — on \
         a tenant that is the end user's own chat, so « the machine fabricated a \
         claim » reaches the person it was talking to. A chat id of 0 counts as \
         absent: it is the documented « no reply channel » sentinel \
         (mika#650/#1090), so it loses the notification just as silently.\n\n\
         Two exits, and the choice is the operator's:\n\
         \x20 - set {ESCALATION_ENV_VAR}=<non-zero chat id> (or \
         `operator_escalation_chat_id` in ~/.mika/agents/<name>/config.toml), or\n\
         \x20 - set {NOTIFY_ENV_VAR}=false (or `fabrication_p0_notify = false`) \
         to leave the gate disarmed — the `*_uncorrected` WARN detectors keep \
         counting either way (mika#2867).",
        offenders.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    /// Clear both variables from the process env.
    ///
    /// Not hygiene for its own sake: these tests assert on what the *cascade*
    /// resolves, and the process environment is one of its doors. An operator
    /// who has `MIKA_FABRICATION_P0_NOTIFY` exported — precisely the operator
    /// this guard is about — would otherwise watch a healthy test go red.
    ///
    /// # Safety
    /// Test-only, serialized by `#[serial]`.
    fn clean_p0_env() {
        unsafe {
            std::env::remove_var(NOTIFY_ENV_VAR);
            std::env::remove_var(ESCALATION_ENV_VAR);
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

    /// mika#2871 AC3 — one test, both controls.
    ///
    /// A probe that can only observe the refusing side cannot tell "the guard
    /// rejects an armed gate without a chat" from "the guard rejects every armed
    /// gate", which is the failure a guard is least able to notice about itself.
    /// So the armed-and-routed pair and the armed-and-unrouted one are asserted
    /// in the same test.
    #[test]
    #[serial]
    fn mika2871_la_porte_armee_sans_chat_descalade_refuse_le_demarrage_en_nommant_lagent() {
        clean_p0_env();

        let ok = home_with_agent(
            "mika-arch",
            "fabrication_p0_notify = true\noperator_escalation_chat_id = -100123\n",
        );
        assert!(
            assert_fabrication_p0_arming_valid(ok.path()).is_ok(),
            "porte armée AVEC chat d'escalade doit démarrer — sans ce contrôle, \
             une garde qui refuse tout armement passerait pour une garde qui marche"
        );

        let bad = home_with_agent("mika-qa", "fabrication_p0_notify = true\n");
        let err = assert_fabrication_p0_arming_valid(bad.path())
            .expect_err("porte armée sans chat d'escalade doit refuser le démarrage");
        let msg = format!("{err}");
        assert!(
            msg.contains("mika-qa"),
            "le message doit nommer l'agent fautif — sur une flotte, c'est la \
             seule information qui dit où corriger. Message : {msg}"
        );
        assert!(
            msg.contains(ESCALATION_ENV_VAR),
            "le message doit nommer la variable qui manque. Message : {msg}"
        );
        assert!(
            msg.contains(NOTIFY_ENV_VAR),
            "le message doit nommer les DEUX sorties : armer la route, ou \
             désarmer la porte. Message : {msg}"
        );
        assert!(
            msg.contains("operator_escalation_chat_id = absent"),
            "la valeur lue doit se dire « absent », distinct du `0` du test \
             suivant : les deux perdent le P0, mais ne se corrigent pas au même \
             endroit. Message : {msg}"
        );
    }

    /// mika#2871 — `0` est refusé comme une absence.
    ///
    /// C'est la configuration sur laquelle une garde en `is_none()` serait
    /// verte : `chat_id == 0` est le sentinelle « aucun canal de réponse »
    /// (mika#650/#1090) que `send()` court-circuite, donc la notification est
    /// perdue aussi sûrement qu'avec une variable absente.
    #[test]
    #[serial]
    fn mika2871_un_chat_descalade_a_zero_est_refuse_comme_une_absence() {
        clean_p0_env();
        let home = home_with_agent(
            "mika-dev",
            "fabrication_p0_notify = true\noperator_escalation_chat_id = 0\n",
        );
        let err = assert_fabrication_p0_arming_valid(home.path())
            .expect_err("un chat d'escalade à 0 ne route aucun P0 — il doit refuser");
        let msg = format!("{err}");
        assert!(msg.contains("mika-dev"), "message: {msg}");
        assert!(
            msg.contains("operator_escalation_chat_id = 0"),
            "le message doit porter la valeur LUE — `= 0` et non `absent` — pour \
             que l'opérateur voie que c'est son `0` qui est en cause et non une \
             variable oubliée. Message : {msg}"
        );
    }

    /// mika#2871 — la géométrie livrée n'est pas une violation.
    ///
    /// La porte part désarmée (`fabrication_p0_notify = false` par défaut,
    /// Fire-Disposition mika#1574) et l'écrasante majorité des installations ne
    /// configure aucun chat d'escalade : c'est l'état nominal. Une garde qui
    /// refuserait de démarrer là serait une régression déguisée en sécurité.
    ///
    /// C'est aussi l'assertion auto-nettoyante de la Fire-Disposition : le jour
    /// où quelqu'un arme la porte par défaut sans livrer la route, ce test
    /// rougit.
    #[test]
    #[serial]
    fn mika2871_la_porte_desarmee_sans_chat_nest_pas_une_violation() {
        clean_p0_env();
        let home = home_with_agent("mika-relay", "llm_provider = \"anthropic\"\n");
        assert!(
            assert_fabrication_p0_arming_valid(home.path()).is_ok(),
            "la géométrie livrée — porte désarmée, aucun chat d'escalade — doit démarrer"
        );
    }

    /// mika#2871 — un home vide n'a rien à refuser.
    #[test]
    #[serial]
    fn mika2871_aucun_agent_nest_pas_une_violation() {
        clean_p0_env();
        let tmp = tempfile::tempdir().unwrap();
        assert!(assert_fabrication_p0_arming_valid(tmp.path()).is_ok());
    }
}
