//! L'héritage d'équipe : un enfant hérite des stop-topics de son parent.
//!
//! Le stop-topic du parent conversationnel atteint le prompt de l'enfant, son
//! contrôle négatif sans parent déclaré, et l'union dédoublonnée en NOCASE où
//! l'enfant gagne (mika#1926).
//!
//! Visibilité : enfant de `agent_loop::tests`, donc descendant
//! d'`agent_loop` — les items privés d'`agent_loop` et les helpers promus
//! dans `tests/mod.rs` restent visibles via `use super::*`.

use super::*;

// ===========================================================================
// mika#1926 — a team child inherits the conversational parent's stop-topics
// ===========================================================================

const CHILD_AGENT: &str = "mika-1926-child";
const PARENT_AGENT: &str = "mika-1926-parent";

/// The system prompt of one real team-child turn, seeded with the child's own
/// and the parent's `stop_topic_*` rows.
///
/// End-to-end by construction (AC3): it goes through `run_team_agent`, so a
/// regression in the field wiring, in the merge, or in the `<stopped-topics>`
/// rendering reddens it — none of which a unit test on the merge alone could
/// see.
///
/// **The two agents share ONE handle, re-scoped with `with_agent`.** There is
/// one SQLite file per container, so the parent read is a different
/// `agent_id` on the same file. Two `Database::open_in_memory` calls would be
/// two distinct files: the parent's rows would be unreachable and the test
/// would pass for `parent: None` and fail for everything else, for a reason
/// that has nothing to do with the behaviour under test.
async fn team_turn_system_prompt(
    parent: Option<&str>,
    child_prefs: &[(&str, &str)],
    parent_prefs: &[(&str, &str)],
) -> String {
    use mika_common::llm::mock::{MockLlmProvider, text_response};

    let db =
        AsyncDatabase::new_with_agent(crate::db::Database::open_in_memory().unwrap(), CHILD_AGENT);
    // `preferences.agent_id` is an FK onto `agents(id)` — both ends must exist
    // before either can carry a row.
    db.register_agent(CHILD_AGENT, "child", "").await.unwrap();
    db.register_agent(PARENT_AGENT, "parent", "").await.unwrap();
    for (category, value) in child_prefs {
        db.set_preference(category, value).await.unwrap();
    }
    let parent_db = db.with_agent(PARENT_AGENT);
    for (category, value) in parent_prefs {
        parent_db.set_preference(category, value).await.unwrap();
    }

    let session_id = "team-mika1926-child";
    db.create_session_if_not_exists(session_id, CHILD_AGENT, "team", None)
        .await
        .unwrap();

    // Three identical responses rather than one: an EndTurn post-condition
    // guard may force a correction turn, and an exhausted mock sequence
    // panics. The assertion reads `captured_requests()[0]` either way.
    let mock = MockLlmProvider::builder()
        .response(text_response("travail fait"))
        .response(text_response("travail fait"))
        .response(text_response("travail fait"))
        .build();
    let tools = crate::tools::ToolRegistry::new();
    let skills = SkillRegistry::empty();
    let home_dir = tempfile::tempdir().unwrap();
    let skills_dirty = AtomicBool::new(false);
    let params = TeamAgentParams {
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        db: &db,
        llm: &mock,
        tools: &tools,
        skills: &skills,
        home_dir: home_dir.path(),
        task_message: "rédige la section",
        team_context: "Tu fais partie d'une équipe.",
        session_id,
        embedding_client: None,
        brave_api_key: None,
        github_token: None,
        gateway_url: None,
        internal_token: None,
        github_app: None,
        skills_dirty: &skills_dirty,
        settings: None,
        mcp_manager: None,
        agent_name: "child",
        child_task_id: None,
        message_sender: None,
        trace_id: None,
        parent_agent_id: parent,
    };
    run_team_agent(&params).await.expect("team turn");
    mock.captured_requests()
        .first()
        .expect("the turn must have reached the provider")
        .system
        .clone()
        .expect("a team turn always carries a system prompt")
}

/// W1 — the decision itself: a stop the user gave the parent reaches the
/// child's prompt.
#[tokio::test]
async fn mika1926_un_stop_du_parent_atteint_le_prompt_de_lenfant() {
    let system = team_turn_system_prompt(
        Some(PARENT_AGENT),
        &[],
        &[("stop_topic_vacances", "ne me reparle pas des vacances")],
    )
    .await;

    assert!(
        system.contains("<stopped-topics>"),
        "no <stopped-topics> block: the parent's row did not reach the child.\n{system}"
    );
    assert!(
        system.contains("vacances"),
        "the block is there but the parent's topic is not in it.\n{system}"
    );
}

/// W2 — the negative control, without which W1 proves nothing: the block
/// must be ABSENT when no parent is declared. A test that only ever sees the
/// positive case cannot tell inheritance from unconditional rendering.
#[tokio::test]
async fn mika1926_sans_parent_declare_lenfant_nherite_de_rien() {
    let system = team_turn_system_prompt(
        None,
        &[],
        &[("stop_topic_vacances", "ne me reparle pas des vacances")],
    )
    .await;

    assert!(
        !system.contains("<stopped-topics>"),
        "the block rendered without a declared parent.\n{system}"
    );
    assert!(
        !system.contains("vacances"),
        "the parent's topic leaked without a declared parent.\n{system}"
    );
}

/// W3 + W4 — the merge's three properties in one table: NOCASE dedup, the
/// child winning a collision, the case-insensitive order, and the
/// parent == child short-circuit.
#[tokio::test]
async fn mika1926_lunion_dedoublonne_en_nocase_et_lenfant_gagne() {
    // `preferences.category` is COLLATE NOCASE with PRIMARY KEY
    // (agent_id, category): `stop_topic_Web-Config` and
    // `stop_topic_web-config` are ONE topic, declared on both sides with
    // conflicting values.
    let system = team_turn_system_prompt(
        Some(PARENT_AGENT),
        &[
            ("stop_topic_Web-Config", "enfant"),
            ("stop_topic_zebra", "enfant-zebra"),
        ],
        &[
            ("stop_topic_web-config", "parent"),
            ("stop_topic_alpha", "parent-alpha"),
        ],
    )
    .await;

    let block = system
        .split_once("<stopped-topics>")
        .expect("block present")
        .1
        .split_once("</stopped-topics>")
        .expect("block closed")
        .0;
    assert_eq!(
        block.lines().filter(|l| l.contains("stop_topic")).count(),
        3,
        "three topics, not four — the collision must collapse.\n{block}"
    );
    assert!(
        block.contains("enfant") && !block.contains("parent\n"),
        "the child's value must win the collision.\n{block}"
    );
    let (alpha, web, zebra) = (
        block.find("alpha").expect("alpha"),
        block.find("Web-Config").expect("Web-Config"),
        block.find("zebra").expect("zebra"),
    );
    assert!(
        alpha < web && web < zebra,
        "order must be case-insensitive on `category`: alpha < Web-Config < zebra.\n{block}"
    );

    // W4 — the conversational agent can run as a member of its own team.
    // Re-reading its own scope would double every row, so the short-circuit
    // must leave exactly one line per topic.
    let self_parent = team_turn_system_prompt(
        Some(CHILD_AGENT),
        &[("stop_topic_zebra", "enfant-zebra")],
        &[],
    )
    .await;
    assert_eq!(
        self_parent.matches("stop_topic_zebra").count(),
        1,
        "parent == child must not duplicate the row.\n{self_parent}"
    );
}
