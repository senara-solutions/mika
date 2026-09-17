//! mika#2355 — le callback de build QA poste son verdict (B2 + B3 moteur).
//!
//! Ces tests pilotent **le chemin de production** : `run_silent_agent` avec
//! `SilentTrigger::Callback { label: "long_running:build_mika" }`, le jeu de
//! skills résolu par `callback_safe_skills()`, et un `run_gh` enregistreur. Un
//! test en mode conversation aurait été plus court et n'aurait rien prouvé : le
//! défaut (`7309c48c`/#2352, `52780caa`/#2353, `1ea9f92c`/#2350 le 2026-09-17)
//! est né sur le tour silencieux, et c'est le tour silencieux qui a deux sites
//! de sortie EndTurn (texte non-vide via le registre, texte vide via le miroir
//! inline) que le mode conversation ne visite pas tous les deux.
//!
//! Ce que chaque test épingle :
//! - **AC8** — un callback de build QA qui répond « Build succeeded » est
//!   re-prompté et **poste** un `run_gh pr review` (pas juste un texte).
//! - **AC4** — le re-prompt nomme `run_gh` / `pr review` / `VERDICT:`, et il
//!   fire sur le chemin texte VIDE aussi.
//! - **AC4b** — un callback `build_mika` **sans** `qa-review` chargé (le cas
//!   mika-dev) n'est pas re-prompté. C'est la moitié qui compte.
//! - **AC2 / AC3** — sur ce flux, la garde #870 ne fire plus et le framing ne
//!   prescrit plus `update_task_status` + `send_message`.
//! - **AC9** — un callback `run_claude_pilot` garde le contrat #870 à l'identique.
//! - Le re-prompt est **à un coup** : la garde ne boucle pas.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mika_agent::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use mika_agent::qa_build_callback::{BUILD_CALLBACK_LABEL, QA_VERDICT_REQUIRED_CORRECTION};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use mika_common::llm::{LlmContent, LlmRequest, LlmRole};
use serde_json::{Value, json};

use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

/// Un `run_gh` qui enregistre ce qu'on lui demande et répond succès. Le
/// contrat AC8 est « une revue EST postée », ce qu'un test ne voit qu'en
/// tenant l'exécution — d'où l'enregistreur plutôt qu'une assertion sur le
/// texte de sortie.
struct RecordingRunGh {
    calls: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl Tool for RecordingRunGh {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Recording run_gh stub (mika#2355)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        Ok(ToolOutput::success(
            "https://github.com/senara-solutions/mika/pull/2359#pullrequestreview-1".to_string(),
        ))
    }
}

/// Stub `update_task_status` — nécessaire au contrôle AC9 (le contrat #870).
struct StubUpdateTaskStatus;

#[async_trait]
impl Tool for StubUpdateTaskStatus {
    fn name(&self) -> &str {
        "update_task_status"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_task_status".to_string(),
            description: "stub".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("Task updated.".to_string()))
    }
}

fn tools_with_recording_run_gh() -> (ToolRegistry, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut registry = default_tools();
    registry.register(Box::new(RecordingRunGh {
        calls: calls.clone(),
    }));
    registry.register(Box::new(StubUpdateTaskStatus));
    (registry, calls)
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

fn skill(name: &str, always_on: bool, dependencies: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} (mika#2355 fixture)"),
                version: "0.1.0".to_string(),
                always_on,
                timeout_secs: 30,
                dependencies: dependencies.iter().map(|d| d.to_string()).collect(),
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec![name.to_string()],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec![],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{name}")),
        keywords_lower: vec![name.to_string()],
        prompt_snippet: format!("You are running the {name} skill."),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// La topologie de mika-qa après B1 : `qa-review` est `always_on` et tire
/// `qa-review-build-callback` par une arête SORTANTE.
fn qa_registry() -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![
        skill(
            "qa-review",
            true,
            &["build-mika", "qa-review-build-callback"],
        ),
        skill("build-mika", false, &[]),
        skill("qa-review-build-callback", false, &["qa-review"]),
    ])
}

/// La topologie de mika-dev : `build-mika` est dans son allowlist, `qa-review`
/// n'y est pas. Un build lancé d'ici ne doit aucun verdict à personne.
fn dev_registry() -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![
        skill("self-dev", true, &["build-mika", "dev-pilot"]),
        skill("build-mika", false, &[]),
        skill("dev-pilot", false, &[]),
    ])
}

// ---------------------------------------------------------------------------
// Pilotage du tour silencieux
// ---------------------------------------------------------------------------

async fn run_callback(harness: &EvalHarness, label: &str, result: &str) {
    let skills_dirty = AtomicBool::new(false);
    let params = SilentAgentParams {
        tier: harness.tier,
        deployment: mika_common::home::Deployment::Unknown,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger: SilentTrigger::Callback {
            task_id: "task-2355".to_string(),
            label: label.to_string(),
            result: result.to_string(),
            failed: false,
            parent_task_id: None,
        },
        home_dir: harness.home_dir.path(),
        session_id: &harness.session_id,
        message_sender: None,
        embedding_client: None,
        brave_api_key: None,
        github_token: None,
        gateway_url: None,
        internal_token: None,
        github_app: None,
        skills_dirty: &skills_dirty,
        settings: Some(&harness.settings),
        trace_id: Some(harness.trace_id.clone()),
    };
    run_silent_agent(&params).await.expect("silent turn runs");
}

/// Les messages `User` d'une requête capturée, à plat.
fn user_texts(req: &LlmRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| matches!(m.role, LlmRole::User))
        .filter_map(|m| match &m.content {
            LlmContent::Text(t) => Some(t.clone()),
            LlmContent::Blocks(_) => None,
        })
        .collect()
}

/// Combien de requêtes portent la correction `needle` dans un message User.
fn corrections_seen(requests: &[LlmRequest], needle: &str) -> usize {
    requests
        .iter()
        .filter(|r| user_texts(r).iter().any(|t| t.contains(needle)))
        .count()
}

/// Un fragment du re-prompt #870 qui n'apparaît dans aucun autre message.
const SELF_DEV_CORRECTION_NEEDLE: &str = "without the required terminal actions";

fn build_callback_marker() -> String {
    format!("[callback: {BUILD_CALLBACK_LABEL}]")
}

// ---------------------------------------------------------------------------
// AC8 — le test nommé par le ticket
// ---------------------------------------------------------------------------

/// Le tour répond « Build succeeded » et s'arrête. Avant B2/B3 c'était un
/// EndTurn accepté (ou re-prompté vers `update_task_status` + `send_message`,
/// que le tour satisfaisait sans rien poster). Après : re-prompt nommant
/// `run_gh pr review`, puis la revue est **postée**.
#[tokio::test]
async fn ac8_a_qa_build_callback_that_only_says_build_succeeded_ends_up_posting_the_review() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            // Pas 1 : la forme exacte du défaut mesuré.
            text_response("Build succeeded. All 4701 tests passed."),
            // Pas 2 (après re-prompt) : la revue est postée.
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2359", "--comment", "--body",
                    "## PLAN-AC VERIFICATION\n…\n\nVERDICT: pass"]}),
            ),
            // Pas 3 : EndTurn, la garde est satisfaite.
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness, BUILD_CALLBACK_LABEL, "Build succeeded").await;

    // AC8 — un `run_gh pr review` a été POSTÉ.
    let calls = gh_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "exactly one run_gh call, got {calls:?}");
    let argv = calls[0]["command"].as_array().expect("command argv");
    assert_eq!(argv[0], "pr");
    assert_eq!(argv[1], "review");
    drop(calls);

    // AC4 — le re-prompt est le bon, injecté une fois, après le premier EndTurn.
    let requests = harness.mock().captured_requests();
    assert_eq!(
        requests.len(),
        3,
        "text → correction → run_gh → text = 3 LLM calls"
    );
    assert!(
        user_texts(&requests[0])
            .iter()
            .any(|t| t.starts_with(&build_callback_marker())),
        "the silent turn opens on the build-callback marker"
    );
    assert_eq!(
        corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION),
        2,
        "the correction is in the history of every request after the re-prompt"
    );
    assert!(
        user_texts(&requests[1])
            .iter()
            .any(|t| t == QA_VERDICT_REQUIRED_CORRECTION),
        "request 2 carries the verdict correction verbatim"
    );
    for needle in ["`run_gh`", "`pr review`", "`VERDICT:`"] {
        assert!(
            QA_VERDICT_REQUIRED_CORRECTION.contains(needle),
            "the correction must name {needle}"
        );
    }

    // AC2 — la garde #870 ne s'est PAS armée sur ce flux.
    assert_eq!(
        corrections_seen(&requests, SELF_DEV_CORRECTION_NEEDLE),
        0,
        "callback_terminal_action must not fire on a build callback"
    );

    // AC3 — le framing (dans le system prompt) ne prescrit plus le contrat
    // self_dev, et dit le contrat de ce flux.
    let system = requests[0].system.as_deref().unwrap_or_default();
    assert!(system.contains("This is a BUILD callback"));
    assert!(!system.contains("This turn MUST end with both of the following"));
    assert!(!system.contains("mark the parent self_dev task terminal"));
}

// ---------------------------------------------------------------------------
// AC4 — le chemin texte VIDE est couvert par le miroir inline
// ---------------------------------------------------------------------------

/// Un EndTurn sec, sans texte, est la forme que prend un tour qui n'a rien à
/// dire — et c'est celle que le registre `INTENT_GUARDS` ne voit jamais.
#[tokio::test]
async fn ac4_an_empty_endturn_on_the_silent_path_is_reprompted_too() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response(""),
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2359", "--comment", "--body", "VERDICT: hold[review]"]}),
            ),
            text_response(""),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness, BUILD_CALLBACK_LABEL, "Build succeeded").await;

    assert_eq!(
        gh_calls.lock().unwrap().len(),
        1,
        "the review is posted after the re-prompt"
    );
    let requests = harness.mock().captured_requests();
    assert_eq!(requests.len(), 3);
    assert!(
        user_texts(&requests[1])
            .iter()
            .any(|t| t == QA_VERDICT_REQUIRED_CORRECTION),
        "the empty-text mirror injected the verdict correction"
    );
    assert_eq!(corrections_seen(&requests, SELF_DEV_CORRECTION_NEEDLE), 0);
}

// ---------------------------------------------------------------------------
// AC4b — le contrôle négatif : mika-dev n'est pas re-prompté
// ---------------------------------------------------------------------------

/// Même label, même message, `qa-review` absent du tour. Une garde armée sur
/// le seul label re-prompterait mika-dev pour poster une revue de PR qu'il n'a
/// aucune raison de poster — elle échangerait un loop-breaker contre un autre.
#[tokio::test]
async fn ac4b_a_build_callback_without_qa_review_loaded_is_not_reprompted() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Build succeeded. Continuing the pilot flow.",
        )])
        .tools(tools)
        .skills(dev_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness, BUILD_CALLBACK_LABEL, "Build succeeded").await;

    let requests = harness.mock().captured_requests();
    assert_eq!(requests.len(), 1, "one LLM call, no re-prompt of any kind");
    assert_eq!(
        corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION),
        0
    );
    // AC2, vu du côté mika-dev : le build callback n'est plus gouverné par la
    // garde #870 non plus — accepté et motivé (plan § B2).
    assert_eq!(corrections_seen(&requests, SELF_DEV_CORRECTION_NEEDLE), 0);
    assert!(gh_calls.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Satisfaite d'emblée — pas de re-prompt quand la revue est déjà postée
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_qa_build_callback_that_posts_its_review_first_is_not_reprompted() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2359", "--approve", "--body", "VERDICT: pass"]}),
            ),
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness, BUILD_CALLBACK_LABEL, "Build succeeded").await;

    assert_eq!(gh_calls.lock().unwrap().len(), 1);
    let requests = harness.mock().captured_requests();
    assert_eq!(requests.len(), 2, "run_gh → text, and nothing in between");
    assert_eq!(
        corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION),
        0
    );
}

// ---------------------------------------------------------------------------
// Un coup, pas une boucle
// ---------------------------------------------------------------------------

/// Après le re-prompt, un second EndTurn sans revue est **accepté** — la
/// garde est à un coup (`intent_guard_retries`), comme toutes ses voisines.
/// Ce qui reste alors est du ressort du filet (plan § B3, hors de cette PR).
#[tokio::test]
async fn the_verdict_guard_fires_once_and_does_not_loop() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("Build succeeded."),
            text_response("Build succeeded, really."),
            // Jamais atteint si la garde est à un coup.
            text_response("Build succeeded, I promise."),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness, BUILD_CALLBACK_LABEL, "Build succeeded").await;

    let requests = harness.mock().captured_requests();
    assert_eq!(
        requests.len(),
        2,
        "text → correction → text, then the loop ends"
    );
    assert!(
        user_texts(&requests[1])
            .iter()
            .any(|t| t == QA_VERDICT_REQUIRED_CORRECTION)
    );
    assert!(
        gh_calls.lock().unwrap().is_empty(),
        "nothing was posted — the net's job"
    );
}

// ---------------------------------------------------------------------------
// AC9 — le flux self_dev garde son contrat #870 sur le chemin silencieux
// ---------------------------------------------------------------------------

/// Le carve-out est étroit : `run_claude_pilot` est toujours re-prompté vers
/// `update_task_status` + `send_message`, et jamais vers `run_gh pr review`.
#[tokio::test]
async fn ac9_a_pilot_callback_keeps_the_self_dev_contract_on_the_silent_path() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("The pilot finished; PR #2359 was opened."),
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-2355", "status": "completed"}),
            ),
            tool_call_response("send_message", json!({"text": "Pilot done, PR #2359."})),
            text_response("Done."),
        ])
        .tools(tools)
        // Même registre que mika-qa, pour prouver que c'est le LABEL qui
        // discrimine ici et non la présence de qa-review.
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(
        &harness,
        "long_running:run_claude_pilot",
        "PR: https://github.com/x/y/pull/2359",
    )
    .await;

    let requests = harness.mock().captured_requests();
    assert!(
        corrections_seen(&requests, SELF_DEV_CORRECTION_NEEDLE) >= 1,
        "callback_terminal_action must still fire on a pilot callback"
    );
    assert_eq!(
        corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION),
        0,
        "the QA verdict guard must not arm on a pilot callback"
    );
    let system = requests[0].system.as_deref().unwrap_or_default();
    assert!(system.contains("This turn MUST end with both of the following"));
    assert!(!system.contains("This is a BUILD callback"));
    assert!(gh_calls.lock().unwrap().is_empty());
}
