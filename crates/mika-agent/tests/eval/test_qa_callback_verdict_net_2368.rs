//! mika#2368 — le filet moteur : une PR de la boucle n'est plus jamais muette.
//!
//! Ce fichier couvre ce qui ne se prouve **que** sur le chemin de production du
//! tour silencieux : que le signal `SilentTurnOutcome.qa_verdict_unmet` est levé
//! sur **chacun** des deux sites de sortie EndTurn, et que le registre
//! anti-double-post atteint bien le `ToolContext` de ce tour.
//!
//! Le reste des AC vit dans les tests unitaires, au plus près de ce qu'il
//! épingle — et c'est délibéré, pas une dispersion :
//!
//! - AC5 / AC5b / AC5c / AC7 (corps, tokens interdits, gel de mika#2276,
//!   anti-double-post) → `server::deadline_verdict::tests`, où vivent le corps
//!   et le registre ;
//! - AC6 (les quatre contrôles négatifs du fail-safe) et le kill-switch →
//!   `task_engine::dispatcher::tests`, où vit le lecteur du stamp ;
//! - C2 / T5 (la paire écrivain↔lecteur du stamp) →
//!   `skills::executor::tests`, où vit le producteur ;
//! - le prédicat complémentaire de la garde →
//!   `qa_build_callback::tests`.
//!
//! **T10 — un contrôle positif PAR SITE DE SORTIE, et deux tests plutôt qu'un
//! paramétré sur la forme du texte.** C'est la seule construction qui rougisse
//! quand un seul des deux sites est câblé. Le site à texte vide est le plus
//! probable en production : un EndTurn sec est la forme que prend un tour qui
//! n'a rien à dire, et c'est le cas nominal de ce ticket. Un signal posé au seul
//! site texte-non-vide laisserait le filet aveugle sur la moitié la plus
//! probable de sa population — sans que rien ne le signale, puisqu'un filet
//! aveugle est silencieux, exactement comme un filet qui n'a rien à faire.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use mika_agent::qa_build_callback::BUILD_CALLBACK_LABEL;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::{Value, json};

use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

/// Ce que le tour a vu du registre anti-double-post, et ce qu'il y a écrit.
#[derive(Default)]
struct RegistryObservation {
    /// `ctx.pr_reviews_posted.is_some()` au moment de l'appel.
    saw_registry: bool,
    calls: Vec<Value>,
}

/// Un `run_gh` qui **inspecte** `ToolContext.pr_reviews_posted` puis y inscrit
/// la clé, comme le fait `builtin_handlers::run_gh` sur succès de
/// `gh pr review`.
///
/// L'inspection est le point : AC7 ne demande pas que `run_gh` écrive (c'est
/// déjà le contrat #821), il demande que le registre **arrive jusqu'au tour
/// silencieux**. C'est ce maillon-là, et lui seul, que mika#2368 ajoute — et le
/// `debug_assert!` de `builtin_handlers` (*"pr_reviews_posted must be threaded
/// for production pr review calls"*) est ce qu'il rend enfin vrai sur ce flux.
struct RegistryAwareRunGh {
    observed: Arc<Mutex<RegistryObservation>>,
}

#[async_trait]
impl Tool for RegistryAwareRunGh {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Registry-aware run_gh stub (mika#2368)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        {
            let mut obs = self.observed.lock().unwrap();
            obs.saw_registry = ctx.pr_reviews_posted.is_some();
            obs.calls.push(input);
        }
        if let Some(registry) = ctx.pr_reviews_posted {
            registry
                .entry(ctx.session_id.to_string())
                .or_default()
                .insert("senara-solutions/mika|2368".to_string());
        }
        Ok(ToolOutput::success(
            "https://github.com/senara-solutions/mika/pull/2368#pullrequestreview-1".to_string(),
        ))
    }
}

fn tools_with_registry_aware_run_gh() -> (ToolRegistry, Arc<Mutex<RegistryObservation>>) {
    let observed = Arc::new(Mutex::new(RegistryObservation::default()));
    let mut registry = default_tools();
    registry.register(Box::new(RegistryAwareRunGh {
        observed: observed.clone(),
    }));
    (registry, observed)
}

// ---------------------------------------------------------------------------
// Skills — la topologie de mika-qa après mika#2355 B1
// ---------------------------------------------------------------------------

fn skill(name: &str, always_on: bool, dependencies: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} (mika#2368 fixture)"),
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
        model_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
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
// Pilotage du tour silencieux
// ---------------------------------------------------------------------------

/// Pilote un callback de build QA comme le dispatcher le fait, registre compris.
async fn run_build_callback(
    harness: &EvalHarness,
    pr_reviews_posted: Option<&Arc<DashMap<String, HashSet<String>>>>,
) -> mika_agent::agent::SilentTurnOutcome {
    let skills_dirty = AtomicBool::new(false);
    let params = SilentAgentParams {
        tier: harness.tier,
        deployment: mika_common::home::Deployment::Unknown,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger: SilentTrigger::Callback {
            task_id: "task-2368".to_string(),
            label: BUILD_CALLBACK_LABEL.to_string(),
            result: "Build succeeded".to_string(),
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
        pr_reviews_posted,
    };
    run_silent_agent(&params).await.expect("silent turn runs")
}

// ---------------------------------------------------------------------------
// T10 — un contrôle positif PAR SITE DE SORTIE
// ---------------------------------------------------------------------------

/// **T10 (a)** — second EndTurn **avec** texte : le site de sortie qui suit le
/// registre `INTENT_GUARDS`.
#[tokio::test]
async fn t10a_the_signal_is_raised_on_the_non_empty_text_exit() {
    let (tools, observed) = tools_with_registry_aware_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            // Le défaut mesuré : « Build succeeded » et rien d'autre.
            text_response("Build succeeded. All tests passed."),
            // Après le re-prompt de la garde : le modèle répète, avec du texte.
            text_response("Build succeeded, really."),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome = run_build_callback(&harness, None).await;

    assert!(
        outcome.qa_verdict_unmet,
        "le signal doit être levé sur le chemin texte non vide"
    );
    assert!(
        observed.lock().unwrap().calls.is_empty(),
        "rien n'est posté ici — le tour signale, le dispatcher poste"
    );
}

/// **T10 (b)** — second EndTurn **à texte vide**, et c'est le cas nominal.
///
/// Sans ce test, une implémentation qui ne câble que le premier site resterait
/// verte : le filet serait aveugle sur la moitié la plus probable de sa
/// population et se tairait, ce qui est indistinguable d'un filet qui n'a rien
/// à faire.
#[tokio::test]
async fn t10b_the_signal_is_raised_on_the_empty_text_exit() {
    let (tools, observed) = tools_with_registry_aware_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            text_response("Build succeeded."),
            // Un EndTurn sec : la forme que prend un tour qui n'a rien à dire.
            text_response(""),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome = run_build_callback(&harness, None).await;

    assert!(
        outcome.qa_verdict_unmet,
        "le signal doit être levé sur le chemin texte VIDE aussi — c'est la \
         moitié la plus probable de la population du filet"
    );
    assert!(observed.lock().unwrap().calls.is_empty());
}

/// **T10 (contrôle négatif commun)** — un tour dont le budget n'est **pas**
/// consommé ne doit pas armer le signal.
///
/// C'est la garde qui travaille à ce moment-là, pas le filet. Sans ce terme, le
/// filet **doublerait** le re-prompt au lieu de lui succéder : il posterait un
/// `hold[review]` sur une PR que le tour suivant allait revoir.
#[tokio::test]
async fn t10_the_signal_is_not_raised_while_the_guard_still_has_its_budget() {
    let (tools, observed) = tools_with_registry_aware_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            // Premier EndTurn sans revue : la garde re-prompte (budget dépensé
            // à cet instant), puis le modèle poste.
            text_response("Build succeeded."),
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2368", "--comment", "--body",
                    "## DIFF ANALYSIS\n…\n\nVERDICT: pass"]}),
            ),
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome = run_build_callback(&harness, None).await;

    assert!(
        !outcome.qa_verdict_unmet,
        "la revue a été postée — le filet n'a rien à faire"
    );
    assert_eq!(
        observed.lock().unwrap().calls.len(),
        1,
        "exactement une revue postée par le tour lui-même"
    );
}

/// Un tour qui poste **du premier coup** n'arme évidemment rien non plus — le
/// filet est strictement additif sur le chemin nominal.
#[tokio::test]
async fn t10_a_turn_that_reviews_immediately_never_arms_the_net() {
    let (tools, observed) = tools_with_registry_aware_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2368", "--comment", "--body",
                    "## DIFF ANALYSIS\n…\n\nVERDICT: pass"]}),
            ),
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome = run_build_callback(&harness, None).await;

    assert!(!outcome.qa_verdict_unmet);
    assert_eq!(observed.lock().unwrap().calls.len(), 1);
}

// ---------------------------------------------------------------------------
// T7 — le registre atteint bien le tour silencieux (AC7)
// ---------------------------------------------------------------------------

/// **T7** — sur le chemin de production, le registre passé par le dispatcher
/// arrive jusqu'au `ToolContext` du tour.
///
/// Sans ce test, AC7 tiendrait **dans** le filet et serait faux dans la vraie
/// vie : `agent_loop` posait `None` avec le commentaire « Silent mode: no
/// session-scoped dedup needed » pendant que `builtin_handlers` portait un
/// `debug_assert!(ctx.pr_reviews_posted.is_some())` disant l'inverse. Un
/// callback QA qui poste sa revue **est** un « production pr review call ».
#[tokio::test]
async fn t7_the_dedup_registry_reaches_the_silent_turns_tool_context() {
    let (tools, observed) = tools_with_registry_aware_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2368", "--comment", "--body",
                    "## DIFF ANALYSIS\n…\n\nVERDICT: pass"]}),
            ),
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let registry: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());
    let outcome = run_build_callback(&harness, Some(&registry)).await;

    assert!(!outcome.qa_verdict_unmet);
    assert!(
        observed.lock().unwrap().saw_registry,
        "AC7 : le tour silencieux doit VOIR le registre — c'est le maillon que \
         mika#2368 ajoute, et le seul"
    );
    let posted = registry
        .get(&harness.session_id)
        .expect("la session du callback doit porter une entrée");
    assert!(
        posted.contains("senara-solutions/mika|2368"),
        "la clé écrite par le tour doit être lisible dans le registre partagé — \
         c'est elle que le filet relit pour ne pas re-poster"
    );
}

/// Le contrôle négatif du même maillon : hors dispatcher, le registre est
/// absent, le tour le voit absent, et le filet s'abstiendra — le terme
/// d'abstention hérité de mika#2276, conservé tel quel.
#[tokio::test]
async fn t7_without_a_registry_the_turn_sees_none() {
    let (tools, observed) = tools_with_registry_aware_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2368", "--comment", "--body",
                    "VERDICT: pass"]}),
            ),
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    run_build_callback(&harness, None).await;

    assert!(
        !observed.lock().unwrap().saw_registry,
        "hors mode serveur il n'y a pas de registre, et le filet doit s'abstenir \
         plutôt que de risquer un double-post"
    );
}
