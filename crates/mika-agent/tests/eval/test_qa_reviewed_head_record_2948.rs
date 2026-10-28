//! mika#2948 — le callback de build inscrit la tête qu'il a jugée.
//!
//! Le défaut : le tour `qa-review-build-callback` poste son verdict
//! (`run_gh pr review`) et n'appelle pas `record_reviewed_head`. Le pré-filtre
//! (g′) de mika#2917 lit alors un registre vide, sort en
//! `release=reviewed_head_absent` et paie un tour webhook pour mika-qa comme
//! pour mika-dev. Mesure du 2026-10-28 : dix callbacks ont posté un `pr review`
//! réussi depuis le 2026-10-27 09:20Z, **un seul** a inscrit (session
//! `43057a9e`). Le cas révélateur est #2947@`da3d86df`, rejoué ici.
//!
//! **Le seul test du vol qui mesure la fin de la chaîne.** Les tests unitaires
//! de `qa_reviewed_head.rs` attestent les prédicats et l'émetteur ; ils ne
//! peuvent pas dire qu'une **ligne** atteint `audit_events`. Ici le tour est le
//! tour de production — `run_silent_agent` avec
//! `SilentTrigger::Callback { label: "long_running:build_mika" }`, le jeu de
//! skills résolu par `callback_safe_skills()` — et `record_reviewed_head` est
//! l'**outil réel** de `default_tools()`, écrivain unique du registre. Seul
//! `run_gh` est bouchonné, parce que c'est la forge.
//!
//! **Les littéraux du registre sont retapés ici, et c'est sans risque.**
//! `crate::qa_reviewed_head` est `pub(crate)` : un test d'intégration ne le voit
//! pas, et élargir une visibilité pour un test est précisément ce que la boucle
//! d'implémentation refuse. Une dérive de la grammaire de clé ou du `tool_name`
//! rend ce test **rouge** (la ligne attendue n'est plus trouvée), jamais vert à
//! tort — la copie est dans le sens sûr. La grammaire elle-même est épinglée à
//! son site par `mika2917_le_module_ne_retape_pas_la_grammaire_de_cle`.

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
// Le cas rejoué — #2947@da3d86df, celui de la session 58de1325
// ---------------------------------------------------------------------------

const REPO: &str = "senara-solutions/mika";
const PR: u64 = 2947;
const TETE: &str = "da3d86df1c4b7e2a9f0d6c3b8e5a1f7d4c2b9e60";

/// Le `tool_name` sous lequel l'écrivain unique inscrit ses lignes.
const REGISTRE_TOOL_NAME: &str = "qa_reviewed_head";

/// La clé que le pré-filtre (g′) relira : `head_key(repo, pr, head_sha)`.
fn cle_attendue() -> String {
    format!("pr:{REPO}#{PR}@{TETE}")
}

/// Un fragment du re-prompt de mika#2948 qui n'apparaît dans aucun autre
/// message — ni dans celui de mika#2355, ni dans celui de #870.
const CORRECTION_INSCRIPTION: &str =
    "What is missing is the one call that records the head you just ruled on";

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

/// Un `run_gh` qui enregistre ce qu'on lui demande et rend le succès **ou**
/// l'échec. Les deux moitiés d'AC2 passent par ce seul bouchon : la condition
/// est le succès du post, donc un test qui ne sait pas échouer ne peut pas la
/// mesurer.
struct RecordingRunGh {
    calls: Arc<Mutex<Vec<Value>>>,
    reussit: bool,
}

#[async_trait]
impl Tool for RecordingRunGh {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Recording run_gh stub (mika#2948)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        if self.reussit {
            Ok(ToolOutput::success(
                format!("https://github.com/{REPO}/pull/{PR}#pullrequestreview-1").to_string(),
            ))
        } else {
            Ok(ToolOutput::error(
                "gh: HTTP 422 — Unprocessable Entity (createPullRequestReview)".to_string(),
            ))
        }
    }
}

/// `update_task_status` et `store_fact` : les deux actions de clôture que la
/// session `58de1325` a exécutées **à la place** de l'inscription. Le tour du
/// test positif les rejoue, parce que la forme exacte du défaut est « il a
/// clôturé, pas « il n'a rien fait » — et la garde doit mordre là.
struct StubClosing(&'static str);

#[async_trait]
impl Tool for StubClosing {
    fn name(&self) -> &str {
        self.0
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.0.to_string(),
            description: "stub".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("ok".to_string()))
    }
}

/// `record_reviewed_head` n'est **pas** bouchonné : c'est l'outil réel de
/// `default_tools()`, et c'est tout l'objet du test.
fn tools_with_recording_run_gh(reussit: bool) -> (ToolRegistry, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut registry = default_tools();
    registry.register(Box::new(RecordingRunGh {
        calls: calls.clone(),
        reussit,
    }));
    registry.register(Box::new(StubClosing("update_task_status")));
    (registry, calls)
}

// ---------------------------------------------------------------------------
// Skills — la topologie de mika-qa, celle qui rend un verdict dû
// ---------------------------------------------------------------------------

fn skill(name: &str, always_on: bool, dependencies: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} (mika#2948 fixture)"),
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

// ---------------------------------------------------------------------------
// Pilotage et lecture
// ---------------------------------------------------------------------------

async fn run_callback(harness: &EvalHarness) {
    let skills_dirty = AtomicBool::new(false);
    let params = SilentAgentParams {
        tier: harness.tier,
        deployment: mika_common::home::Deployment::Unknown,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger: SilentTrigger::Callback {
            task_id: "task-2948".to_string(),
            label: BUILD_CALLBACK_LABEL.to_string(),
            result: "Build succeeded. All 7000 tests passed.".to_string(),
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
        pr_reviews_posted: None,
    };
    run_silent_agent(&params).await.expect("silent turn runs");
}

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

fn corrections_seen(requests: &[LlmRequest], needle: &str) -> usize {
    requests
        .iter()
        .filter(|r| user_texts(r).iter().any(|t| t.contains(needle)))
        .count()
}

/// Ce que le registre porte pour cette session : `(target_key, outcome)` des
/// seules lignes de l'écrivain unique.
async fn lignes_du_registre(harness: &EvalHarness) -> Vec<(String, String)> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events")
        .iter()
        .filter(|e| e.tool_name == REGISTRE_TOOL_NAME)
        .map(|e| {
            (
                e.target_key.clone(),
                e.after_value.clone().unwrap_or_default(),
            )
        })
        .collect()
}

/// Un appel `record_reviewed_head` tel que le prompt le prescrit : la tête vient
/// de l'étape 1, pas d'un `gh` frais.
fn appel_dinscription() -> MockResponse {
    tool_call_response(
        "record_reviewed_head",
        json!({
            "repo": REPO,
            "pr": PR,
            "head_sha": TETE,
            "outcome": "verdict_posted",
        }),
    )
}

fn post_de_la_revue() -> MockResponse {
    tool_call_response(
        "run_gh",
        json!({"command": ["pr", "review", "2947", "--approve", "--body",
            "## PLAN-AC VERIFICATION\n…\n\nVERDICT: pass"]}),
    )
}

// ---------------------------------------------------------------------------
// AC1 + AC2 — le même test, parce que la comparaison EST le contrat
// ---------------------------------------------------------------------------

/// **AC1.** Le tour poste sa revue, clôture et s'arrête sans inscrire : la garde
/// refuse la sortie, le re-prompt nomme l'appel, et `audit_events` porte
/// `verdict_posted` sur `head_key(repo, pr, headRefOid)`.
///
/// **AC2, dans le même test.** Deux populations négatives traversent le même
/// harnais et le même chemin : un `pr review` que la forge refuse (la ligne
/// `POST_FAILED`), et un tour qui ne poste rien du tout. Aucune des deux n'arme
/// la garde et aucune n'écrit. Séparées en trois tests voisins, « le registre
/// porte une ligne » et « le registre n'en porte aucune » seraient deux
/// affirmations sur deux fixtures, dont rien ne dirait qu'elles diffèrent par le
/// terme qu'on croit (classe mika#2205) ; ici elles ne diffèrent que par le
/// succès du post.
///
/// **Ce que les populations négatives arment, elles, c'est la garde de
/// mika#2355** — un verdict dû et aucun `pr review` réussi —, et c'est
/// l'exclusion mutuelle vue de bout en bout : le re-prompt qui arrive est celui
/// du verdict, jamais celui de l'inscription.
#[tokio::test]
async fn mika2948_ac1_ac2_la_tete_sinscrit_apres_un_post_reussi_et_jamais_autrement() {
    // ---- population 1 : la forme exacte du défaut (AC1) -------------------
    let (tools, gh_calls) = tools_with_recording_run_gh(true);
    let harness = EvalHarness::builder()
        .responses(vec![
            // Pas 1 : la revue est postée, avec succès.
            post_de_la_revue(),
            // Pas 2 : la clôture que `58de1325` a exécutée à la place.
            tool_call_response(
                "update_task_status",
                json!({"task_id": "task-2948", "status": "completed"}),
            ),
            // Pas 3 : EndTurn. Rien d'inscrit — la garde doit mordre ici.
            text_response("VERDICT: pass"),
            // Pas 4 (après re-prompt) : l'inscription, par l'outil réel.
            appel_dinscription(),
            // Pas 5 : EndTurn, la garde est satisfaite.
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness).await;

    // AC1 — la ligne est dans le registre, sur la clé que (g′) relira.
    let lignes = lignes_du_registre(&harness).await;
    assert_eq!(
        lignes,
        vec![(cle_attendue(), "verdict_posted".to_string())],
        "exactement une ligne, sur head_key(repo, pr, headRefOid), issue verdict_posted"
    );

    // La revue n'a pas été re-postée : le re-prompt dit « ne la repose pas », et
    // un second post aurait coûté une revue en double sur la forge.
    assert_eq!(
        gh_calls.lock().unwrap().len(),
        1,
        "un seul `run_gh`, celui du pas 1"
    );

    let requests = harness.mock().captured_requests();
    assert_eq!(
        requests.len(),
        5,
        "run_gh → clôture → text → correction → record → text = 5 appels LLM"
    );
    assert!(
        user_texts(&requests[3])
            .iter()
            .any(|t| t.contains(CORRECTION_INSCRIPTION)),
        "la requête 4 porte le re-prompt de l'inscription, injecté après le premier EndTurn"
    );
    // L'exclusion mutuelle, vue du chemin réel : la garde de mika#2355 ne peut
    // pas s'armer sur un tour qui a posté.
    assert_eq!(
        corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION),
        0,
        "le verdict EST posté : la garde de mika#2355 ne s'arme pas ici"
    );

    // ---- population 2 : le post a échoué (AC2) ----------------------------
    let (tools, gh_calls) = tools_with_recording_run_gh(false);
    let harness = EvalHarness::builder()
        .responses(vec![
            // Le `pr review` part et la forge le refuse.
            post_de_la_revue(),
            // La ligne `POST_FAILED` du prompt, et rien d'inscrit — c'est la
            // règle « la condition est le succès du post ».
            text_response("POST_FAILED: gh refused the review twice (HTTP 422)."),
            // Après le re-prompt de mika#2355, le tour redit la même chose.
            text_response("POST_FAILED: gh refused the review twice (HTTP 422)."),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness).await;

    assert!(
        lignes_du_registre(&harness).await.is_empty(),
        "un post refusé par la forge n'a statué sur aucune tête : rien ne s'inscrit"
    );
    assert_eq!(
        gh_calls.lock().unwrap().len(),
        1,
        "le post a bien été tenté"
    );
    let requests = harness.mock().captured_requests();
    assert_eq!(
        corrections_seen(&requests, CORRECTION_INSCRIPTION),
        0,
        "la garde de l'inscription ne s'arme pas sur un post en échec"
    );
    assert!(
        corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION) >= 1,
        "c'est la garde du VERDICT qui gouverne cette population — contrôle positif \
         de l'exclusion mutuelle, sans lequel « aucun re-prompt d'inscription » serait \
         indistinguable de « aucune garde n'a tourné »"
    );

    // ---- population 3 : aucun `pr review` du tout (AC2) -------------------
    let (tools, gh_calls) = tools_with_recording_run_gh(true);
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("Build succeeded. All 7000 tests passed."),
            text_response("Build succeeded, nothing else to add."),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness).await;

    assert!(
        lignes_du_registre(&harness).await.is_empty(),
        "un tour qui n'a rien posté n'a statué sur aucune tête : rien ne s'inscrit"
    );
    assert!(
        gh_calls.lock().unwrap().is_empty(),
        "aucun appel à la forge dans cette population"
    );
    let requests = harness.mock().captured_requests();
    assert_eq!(corrections_seen(&requests, CORRECTION_INSCRIPTION), 0);
    assert!(corrections_seen(&requests, QA_VERDICT_REQUIRED_CORRECTION) >= 1);
}

// ---------------------------------------------------------------------------
// Un coup, pas une boucle — et la population résiduelle d'AC4
// ---------------------------------------------------------------------------

/// Le re-prompt est **à un coup**, comme toutes ses voisines : un second EndTurn
/// sans inscription est accepté, et c'est la population résiduelle que la sonde
/// d'AC4 lira dans `~/.mika/logs/mika-spirit.log*` sous
/// `qa_reviewed_head_unrecorded`.
///
/// Sans ce test, une garde qui boucle serait verte sur le test d'AC1 : la ligne
/// finirait par s'écrire, au prix d'un tour qui ne se termine qu'au plafond de
/// pas. Le coût du correctif est **un** pas, et c'est ici qu'il est dit.
#[tokio::test]
async fn mika2948_le_reprompt_de_linscription_est_a_un_coup() {
    let (tools, gh_calls) = tools_with_recording_run_gh(true);
    let harness = EvalHarness::builder()
        .responses(vec![
            post_de_la_revue(),
            text_response("VERDICT: pass"),
            // Après le re-prompt : toujours rien d'inscrit.
            text_response("VERDICT: pass, really."),
            // Jamais atteint si la garde est à un coup.
            appel_dinscription(),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_callback(&harness).await;

    let requests = harness.mock().captured_requests();
    assert_eq!(
        requests.len(),
        3,
        "run_gh → text → correction → text, puis la boucle s'arrête"
    );
    assert_eq!(
        corrections_seen(&requests, CORRECTION_INSCRIPTION),
        1,
        "la correction est injectée une fois, et une seule"
    );
    assert!(
        lignes_du_registre(&harness).await.is_empty(),
        "le modèle n'a pas inscrit : le moteur n'écrit pas à sa place — SOLE WRITER tient, \
         et ce tour est exactement la population résiduelle d'AC4"
    );
    assert_eq!(gh_calls.lock().unwrap().len(), 1);
}
