//! mika#2748 — an agent declaring `[tools].allowed` is served that allowlist.
//!
//! **Why a real turn and not the predicate.** `apply_agent_tool_visibility` has
//! its own table of five cases in `agent_loop` (D8), and
//! `build_mika_arch_identity` has its emission and reconciliation tests in
//! `well_known_agents` (D9). Both stay **green** if the three production sites
//! never thread `ctx.identity.tools.allowed` into the hook — a predicate nobody
//! consults and a field nobody reads are indistinguishable from a working
//! switch, by any assertion that does not look at the array the model actually
//! received. That array is what this file reads.
//!
//! **The negative control is load-bearing.** On its own, "`store_fact` is
//! absent" is satisfied by a registry that never carried `store_fact` — a
//! vacuous green. The second test runs the same harness with no `[tools]` block
//! at all and asserts the same name is **present**, which is what makes the
//! absence above mean *the allowlist removed it* rather than *it was never
//! there*.

use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// A builtin named in the allowlist below. Also mika-arch's own constitutive
/// tool set (`test_mika_arch_disabled_tools_excludes_agent_self_state`, #811),
/// which under mika#2748 is a name **present** in the allowlist rather than one
/// absent from the denylist.
const ADMITTED: &str = "send_message";

/// A builtin **not** named in the allowlist — registered by `default_tools()`,
/// so its absence from the served array can only come from the filter.
const WITHHELD: &str = "store_fact";

/// The tool names the model was served on the first LLM call of the turn.
async fn served_tool_names(identity_toml: &str) -> Vec<String> {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noted.")])
        .build()
        .await
        .unwrap();

    // Written after `build()`, which seeds a `name`/`emoji`-only file (mika#2027
    // makes an absent identity.toml fail-closed, a state no provisioned agent is
    // in).
    std::fs::write(harness.home_dir.path().join("identity.toml"), identity_toml).unwrap();

    let trace = harness.run("Review the plan.").await.unwrap();
    trace
        .captured_requests
        .first()
        .expect("the mock provider must have captured the request")
        .tools
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

/// **AC2 + AC5, behavioural half** — the declared allowlist reaches the array.
#[tokio::test]
async fn mika2748_un_agent_a_allowlist_ne_recoit_que_les_builtins_quil_declare() {
    let names = served_tool_names(&format!(
        r#"
name = "Architect"

[tools]
allowed = ["{ADMITTED}"]
"#
    ))
    .await;

    assert!(
        names.iter().any(|n| n == ADMITTED),
        "a name written in [tools].allowed must survive the filter — losing one \
         silently is failure mode n° 1 of the switch (§ Décision 6), since a typo \
         in an allowlist REMOVES a tool where a typo in a denylist kept one. \
         Served: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == WITHHELD),
        "a builtin absent from [tools].allowed must not reach the model — that \
         absence is the whole point of the switch: a future register(...) reaches \
         nobody until someone writes its name. Served: {names:?}"
    );
}

/// **Negative control** — with no `[tools]` block, the same builtin is served.
///
/// Must be seen green **beside** the test above: without it, the absence
/// asserted there would also hold for a registry that simply never carried
/// `store_fact`, and for a filter that strips every builtin from every agent.
#[tokio::test]
async fn mika2748_sans_bloc_tools_le_meme_builtin_est_servi() {
    let names = served_tool_names("name = \"Architect\"\n").await;

    assert!(
        names.iter().any(|n| n == WITHHELD),
        "`allowed = None` is the three-state's 'no allowlist, current behaviour' \
         arm — an agent declaring nothing keeps every builtin. Served: {names:?}"
    );
}
