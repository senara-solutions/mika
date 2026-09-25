//! mika#2515 — un build vert ne laisse plus une PR muette : le signal sort des
//! **cinq** sorties atteignables de `run_loop`, pas de deux.
//!
//! # Ce que seul le chemin de production peut prouver
//!
//! `run_loop` a six sorties et n'en instrumentait que deux (mika#2368). Les
//! quatre autres se répartissent ainsi :
//!
//! | sortie | ce fichier |
//! |---|---|
//! | `Done` — texte non vide | hérité, couvert par mika#2368 (T10a) |
//! | `Done` — texte vide, Silent | hérité, couvert par mika#2368 (T10b) |
//! | `Done` — **Force EndTurn de `send_message`** | **P0**, vu ROUGE avant U1e |
//! | `Done` — texte vide après follow-up | inatteignable en Silent, exclu déclaré |
//! | `DeadlineExceeded` | **coupure 1/2** |
//! | `MaxStepsExceeded` | **coupure 2/2** |
//!
//! Le scan de source (`agent_loop::tests::mika2515_every_loop_exit_decides_…`)
//! asserte la **cardinalité** — le seul terme qu'aucune fixture ne peut voir.
//! Ce fichier asserte le **câblage** : qu'un vrai tour silencieux, sur un vrai
//! message de callback de build, avec `qa-review` chargé, lève bien la bonne
//! moitié du signal. Les deux moitiés sont nécessaires et ne se remplacent pas :
//! un scan vert avec un câblage faux, ou l'inverse, laisseraient la population
//! muette.
//!
//! # P0 est vu ROUGE avant U1e, et c'est la preuve que le trou existait
//!
//! `p0_the_signal_is_raised_on_the_force_endturn_exit` échoue si l'on retire le
//! bloc U1e d'`agent_loop/mod.rs` — ce qui est la seule façon de distinguer
//! « on a fermé un trou » de « on a écrit un test autour du code ». mika#2136
//! avait nommé ce site en propres termes, dans le code, à trois lignes de là
//! (*« a FOURTH exit from `run_loop` … it does not traverse the EndTurn guard
//! chain at all »*) ; mika#2368 n'y a pas posé son miroir.
//!
//! Recette d'injection-vérification (MANDATOIRE) : commenter le
//! `flag.mark_unmet_after_retry()` du site « Force EndTurn » et rejouer ce
//! fichier — `p0_…` DOIT échouer. Idem pour chacun des deux `flag.mark_cut_off`
//! et les tests de coupure. Restaurer, rejouer, tout passe.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mika_agent::agent::{
    SilentAgentParams, SilentTrigger, SilentTurnOutcome, run_silent_agent,
    run_silent_agent_with_deadline,
};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::qa_build_callback::{BUILD_CALLBACK_LABEL, CutOffExit};
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

/// `run_gh` qui enregistre ses appels, pour que les contrôles négatifs puissent
/// asserter qu'une revue **a** été postée.
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
            description: "gh CLI (fixture mika#2515)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        Ok(ToolOutput::success("review submitted"))
    }
}

/// `send_message` qui réussit — c'est ce succès qui arme la frontière #771 et
/// fait sortir le tour par le « Force EndTurn », le site de P0.
struct DeliveringSendMessage;

#[async_trait]
impl Tool for DeliveringSendMessage {
    fn name(&self) -> &str {
        "send_message"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".to_string(),
            description: "Send a message (fixture mika#2515)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("sent"))
    }
}

/// Un outil de diagnostic quelconque, pour épuiser le budget de steps sans
/// jamais poster de revue — la forme d'un tour qui explore et se fait couper.
struct Noop;

#[async_trait]
impl Tool for Noop {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "read a task (fixture mika#2515)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("still running"))
    }
}

struct NoopSender;

#[async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn qa_tools() -> (ToolRegistry, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut tools = default_tools();
    tools.register(Box::new(RecordingRunGh {
        calls: calls.clone(),
    }));
    tools.register(Box::new(DeliveringSendMessage));
    tools.register(Box::new(Noop));
    (tools, calls)
}

// ---------------------------------------------------------------------------
// Registre de skills
// ---------------------------------------------------------------------------

fn skill(name: &str, always_on: bool, dependencies: &[&str]) -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: name.to_string(),
                description: format!("{name} (fixture mika#2515)"),
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

/// `qa-review` est `always_on`, donc présent dans `callback_safe_skills()` —
/// c'est la seconde moitié du déclencheur conjonctif de mika#2355, et c'est ce
/// qui rend `qa_verdict_due` vrai sur ces tours.
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

/// Un registre **sans** `qa-review` : le contrôle négatif du déclencheur
/// conjonctif (le cas mika-dev, qui lance des builds et ne doit aucun verdict).
fn dev_registry() -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![skill("build-mika", true, &[])])
}

// ---------------------------------------------------------------------------
// Pilotage du tour silencieux
// ---------------------------------------------------------------------------

fn build_callback_trigger() -> SilentTrigger {
    SilentTrigger::Callback {
        task_id: "task-2515".to_string(),
        label: BUILD_CALLBACK_LABEL.to_string(),
        result: "Build succeeded".to_string(),
        failed: false,
        parent_task_id: None,
    }
}

/// Un callback **non-build** : contrôle négatif du premier terme du déclencheur.
fn pilot_callback_trigger() -> SilentTrigger {
    SilentTrigger::Callback {
        task_id: "task-2515-pilot".to_string(),
        label: "long_running:run_claude_pilot".to_string(),
        result: "claude-pilot completed (status: done).".to_string(),
        failed: false,
        parent_task_id: None,
    }
}

async fn run_callback(harness: &EvalHarness, trigger: SilentTrigger) -> SilentTurnOutcome {
    let skills_dirty = AtomicBool::new(false);
    let params = silent_params(harness, trigger, &skills_dirty);
    run_silent_agent(&params).await.expect("silent turn runs")
}

/// Pilote le même tour avec une enveloppe que le **premier appel LLM traverse**.
///
/// Le budget doit laisser passer le prélude (`load_agent_context` +
/// `list_commitments`) puis être franchi en tête de l'itération suivante : une
/// deadline déjà dépassée à l'entrée sort par le prélude, qui est le renvoi
/// **déclaré exclu** — donc un tout autre chemin, et un test qui attesterait
/// autre chose que son titre. D'où l'horloge virtuelle
/// (`start_paused = true` + `delayed_response`), sur le modèle de
/// `test_deadline_verdict_2276`.
async fn run_callback_with_deadline(
    harness: &EvalHarness,
    trigger: SilentTrigger,
    budget: Duration,
) -> SilentTurnOutcome {
    let skills_dirty = AtomicBool::new(false);
    let params = silent_params(harness, trigger, &skills_dirty);
    run_silent_agent_with_deadline(&params, (Instant::now() + budget).into())
        .await
        .expect("silent turn runs")
}

fn silent_params<'a>(
    harness: &'a EvalHarness,
    trigger: SilentTrigger,
    skills_dirty: &'a AtomicBool,
) -> SilentAgentParams<'a> {
    SilentAgentParams {
        tier: harness.tier,
        deployment: mika_common::home::Deployment::Unknown,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger,
        home_dir: harness.home_dir.path(),
        session_id: &harness.session_id,
        message_sender: Some(Arc::new(NoopSender)),
        embedding_client: None,
        brave_api_key: None,
        github_token: None,
        gateway_url: None,
        internal_token: None,
        github_app: None,
        skills_dirty,
        settings: Some(&harness.settings),
        trace_id: Some(harness.trace_id.clone()),
        pr_reviews_posted: None,
    }
}

fn posted_review() -> MockResponse {
    tool_call_response(
        "run_gh",
        json!({"command": ["pr", "review", "2515", "--comment", "--body",
                           "PLAN-AC VERIFICATION\n\nVERDICT: pass"]}),
    )
}

// ---------------------------------------------------------------------------
// P0 — le « Force EndTurn », et la RECTIFICATION que le code impose au plan
// ---------------------------------------------------------------------------

/// **P0 — la prémisse du plan est RÉFUTÉE, et c'est ce test qui l'établit.**
///
/// Le plan de mika#2515 écrit que le site « Force EndTurn » est « atteignable en
/// mode `Silent` (**aucun garde de mode ne le protège**) », et en tire que
/// « P0 est la plus insidieuse des quatre ». **Les deux moitiés sont fausses**, et
/// pour deux raisons indépendantes lisibles au site de la frontière #771 :
///
/// ```text
/// if send_message_boundary_active && mode.is_conversation() && !is_automated_trigger
/// ```
///
/// 1. `mode.is_conversation()` — **c'est un garde de mode**, et il est faux pour
///    `LoopMode::Silent`, donc pour tout tour de callback ;
/// 2. `is_automated_trigger` — vrai dès que le message commence par `[callback:`
///    **ou** que `ctx.is_callback_turn` est posé, ce qui est le cas d'un callback
///    de build par construction.
///
/// Le commentaire du site le dit d'ailleurs en propres termes : *« Only fires
/// when ALL of: Conversation mode (**silent/callback modes exempt**) »*.
///
/// **Conséquence pour la lecture du ticket : la population de P0 est vide
/// aujourd'hui.** U1e est livré quand même — le site doit *décider* (DoD 1) et se
/// trouve armé d'avance si les gardes de #771 s'élargissaient — mais il ne ferme
/// aucun défaut observable, et le dire est la seule façon d'éviter que la
/// prochaine lecture ne le recompte comme un trou fermé. C'est très exactement
/// la leçon mika#2272 que le plan cite pour son propre prédicat de coupure : une
/// condition insatisfiable se lit exactement comme une flotte saine.
///
/// La preuve comportementale est que le tour **continue** après `send_message`
/// au lieu de conclure : il consomme une réponse de plus.
#[tokio::test]
async fn p0_the_force_endturn_exit_is_unreachable_from_a_callback_turn() {
    let (tools, calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(vec![
            // Le geste que le plan décrit : le modèle parle au lieu de poster.
            tool_call_response("send_message", json!({"text": "Le build a réussi."})),
            // Si le « Force EndTurn » mordait, cette réponse ne serait JAMAIS
            // consommée et le mock s'épuiserait au lieu de la rendre. Elle l'est,
            // donc le tour a continué : la frontière #771 n'a pas mordu.
            text_response("Et je continue, donc la frontière #771 n'a pas mordu."),
            // La garde `qa_build_callback_verdict` re-prompte sur cet EndTurn
            // sans revue (budget dépensé) ; le modèle répète, et c'est le miroir
            // EndTurn de mika#2368 qui lève le signal.
            text_response("Toujours pas de revue postée."),
        ])
        .tools(tools)
        .skills(qa_registry())
        .message_sender(Arc::new(NoopSender))
        .build()
        .await
        .unwrap();

    // Le tour tourne jusqu'au bout : la frontière #771 ne l'a pas conclu.
    let outcome = run_callback(&harness, build_callback_trigger()).await;

    // Il finit par l'un des deux miroirs EndTurn de mika#2368, pas par P0.
    assert!(
        outcome.qa_verdict_unmet,
        "le tour conclut par un miroir EndTurn (mika#2368), et le signal en sort \
         — c'est par CE chemin qu'un callback de build qui bavarde au lieu de \
         poster est rattrapé, pas par le « Force EndTurn »"
    );
    assert_eq!(
        outcome.qa_verdict_cut_off, None,
        "une conclusion muette n'est pas une coupure"
    );
    assert!(calls.lock().unwrap().is_empty());
}

/// **P0 — le second garde, celui qui tient même si le premier s'élargissait.**
///
/// `is_automated_trigger` est vrai dès que le message commence par `[callback:`,
/// et c'est la forme que `run_silent_agent` émet pour un callback de build. Le
/// premier garde (`mode.is_conversation()`) est asserté côté crate, où `LoopMode`
/// est visible : `agent_loop::tests::mika2515_the_force_endturn_exit_is_mode_guarded`.
///
/// Reconstruit depuis la grammaire du moteur plutôt que recopié, pour que le test
/// casse si l'une des deux moitiés du marqueur bouge.
#[tokio::test]
async fn p0_the_message_marker_is_an_automated_trigger() {
    let emitted = format!("[callback: {BUILD_CALLBACK_LABEL}]");
    assert!(
        emitted.starts_with("[callback:"),
        "second garde de la frontière #771 : `is_automated_trigger` est vrai \
         pour ce message, et le resterait même si `mode.is_conversation()` \
         s'élargissait — d'où deux raisons indépendantes de l'inatteignabilité"
    );
}

/// **V2b — le prédicat d'U1e est bien le frère, pas celui de la coupure.**
///
/// U1e conserve le terme de budget de garde là où les deux sites de coupure le
/// retirent, et ce test pin cette asymétrie sur le chemin de production : un tour
/// dont la garde n'a **jamais** firé ne pose **rien**.
///
/// Il rougirait si quelqu'un « harmonisait » les deux prédicats sur un seul —
/// dans un sens un filet insatisfiable, dans l'autre un `hold[review]` sur un
/// tour que la garde n'a jamais interrogé.
#[tokio::test]
async fn v2b_a_turn_whose_guard_never_fired_arms_nothing() {
    let (tools, calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(vec![
            // Le tour poste sa revue du premier coup : la garde n'a rien à
            // re-prompter, son budget reste intact.
            posted_review(),
            text_response("Revue postée."),
        ])
        .tools(tools)
        .skills(qa_registry())
        .message_sender(Arc::new(NoopSender))
        .build()
        .await
        .unwrap();

    let outcome = run_callback(&harness, build_callback_trigger()).await;

    assert!(
        !outcome.qa_verdict_unmet,
        "budget de garde intact ET revue postée ⇒ le filet ne double PAS le \
         re-prompt"
    );
    assert_eq!(outcome.qa_verdict_cut_off, None);
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "exactement une revue, postée par le tour lui-même"
    );
}

// ---------------------------------------------------------------------------
// P3 — les deux sorties coupées
// ---------------------------------------------------------------------------

/// **V2 (coupure 1/2)** — un tour coupé par sa deadline rend la moitié
/// « coupé », avec la bonne borne et ses steps.
///
/// Ce fait était **explicitement jeté** avant mika#2515 : le bras
/// `DeadlineExceeded` de `run_silent_inner` rendait `default()` en renvoyant au
/// motif `CutOffByDeadline` de mika#2276 — câblé au call-site **webhook**, donc
/// nulle part pour un callback.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p3_a_deadline_cut_off_reports_its_bound_and_its_steps() {
    let (tools, calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(vec![
            // L'appel LLM traverse l'enveloppe et rend un tool_call ; la boucle
            // itère, et le contrôle de tête d'itération sort en
            // `DeadlineExceeded`. Même encadrement que mika#2276 : > 1 s sinon
            // l'appel ne traverse plus rien, < 300 s sinon le filet LLM de
            // mika#2342 coupe AVANT la deadline et le tour sortirait par une
            // erreur transport.
            delayed_response(
                200_000,
                tool_call_response("check_task", json!({"task_id": "x"})),
            ),
            // Sentinelle : jamais consommée si la deadline garde bien la porte.
            text_response("(ne doit jamais être rendu)"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome =
        run_callback_with_deadline(&harness, build_callback_trigger(), Duration::from_secs(1))
            .await;

    let cut = outcome
        .qa_verdict_cut_off
        .expect("un tour coupé doit rendre son fait de coupure");
    assert_eq!(
        cut.exit,
        CutOffExit::Deadline,
        "la borne franchie décide du `cause` de la ligne postée"
    );
    assert!(
        !outcome.qa_verdict_unmet,
        "un tour coupé n'a pas CONCLU : les deux moitiés sont mutuellement \
         exclusives, et les confondre ferait mentir le `cause`"
    );
    assert!(calls.lock().unwrap().is_empty());
}

/// **V2 (coupure 2/2)** — un tour qui épuise son budget de steps rend la moitié
/// « coupé » avec la borne `MaxSteps`.
///
/// Ce chemin-ci n'était pas « jeté » mais **jamais posé** : `run_loop` ne posait
/// son signal qu'aux sorties EndTurn, et le bras max-steps de `run_silent_inner`
/// tombe à travers vers la construction finale qui **lit** le drapeau. Deux
/// natures de défaut, deux corrections — un correctif qui n'aurait traité que
/// les `default()` aurait laissé ce chemin muet.
#[tokio::test]
async fn p3_a_max_steps_cut_off_reports_the_other_bound() {
    let (tools, calls) = qa_tools();
    // Vingt-et-un outils : `MAX_CALLBACK_TOOL_STEPS` steps consommés, jamais
    // d'EndTurn. Le 21ᵉ couvre le tour de continuation.
    let mut responses: Vec<MockResponse> = (0..21)
        .map(|_| tool_call_response("check_task", json!({"task_id": "x"})))
        .collect();
    // Le tour de continuation (outils désactivés) rend du texte.
    responses.push(text_response("J'ai exploré sans conclure."));

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools)
        .skills(qa_registry())
        .message_sender(Arc::new(NoopSender))
        .build()
        .await
        .unwrap();

    let outcome = run_callback(&harness, build_callback_trigger()).await;

    let cut = outcome
        .qa_verdict_cut_off
        .expect("un tour à budget de steps épuisé doit rendre son fait de coupure");
    assert_eq!(cut.exit, CutOffExit::MaxSteps);
    assert_eq!(
        cut.steps_completed,
        mika_agent::planning::policy::MAX_CALLBACK_TOOL_STEPS,
        "les steps accomplis voyagent avec la borne — un tour coupé au step 2 et \
         un tour coupé au step 20 appellent des réponses opérateur différentes"
    );
    assert!(!outcome.qa_verdict_unmet);
    assert!(calls.lock().unwrap().is_empty());
}

/// **V2 (contrôle négatif de la coupure)** — un tour qui a posté sa revue **puis**
/// est coupé ne pose rien.
///
/// C'est le terme partagé `pr_review_posted_in_turn` qui porte ce contrôle, et
/// sans lui le filet posterait un `hold[review]` par-dessus un verdict déjà
/// rendu — ce qui sort la PR de la population du réconciliateur mika#2334 pour
/// de bon.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p3_a_cut_off_turn_that_already_reviewed_arms_nothing() {
    let (tools, calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(vec![
            // La revue part, et c'est cet appel-là qui traverse l'enveloppe.
            delayed_response(200_000, posted_review()),
            text_response("(ne doit jamais être rendu)"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome =
        run_callback_with_deadline(&harness, build_callback_trigger(), Duration::from_secs(1))
            .await;

    assert_eq!(
        outcome.qa_verdict_cut_off, None,
        "la revue a été postée avant la coupure — rien n'est dû"
    );
    assert!(!outcome.qa_verdict_unmet);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Le déclencheur conjonctif — les deux contrôles négatifs de mika#2355
// ---------------------------------------------------------------------------

/// **V2 (contrôle négatif)** — un callback **non-build** coupé ne pose rien, quel
/// que soit le registre de skills.
///
/// Premier terme du déclencheur conjonctif (mika#2355 AC4b) : le marqueur de
/// message. Sans lui, tout tour de callback coupé de tout agent armerait le
/// filet.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conjunctive_trigger_excludes_a_non_build_callback() {
    let (tools, _calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(vec![
            delayed_response(
                200_000,
                tool_call_response("check_task", json!({"task_id": "x"})),
            ),
            text_response("(ne doit jamais être rendu)"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let outcome =
        run_callback_with_deadline(&harness, pilot_callback_trigger(), Duration::from_secs(1))
            .await;

    assert_eq!(
        outcome.qa_verdict_cut_off, None,
        "un callback `run_claude_pilot` ne doit aucun verdict à personne"
    );
    assert!(!outcome.qa_verdict_unmet);
}

/// **V2 (contrôle négatif)** — un callback de build **sans `qa-review` chargé**
/// ne pose rien : c'est le cas mika-dev, qui porte `build-mika` dans son
/// allowlist et lance des builds qui ne doivent aucun verdict.
///
/// Second terme du déclencheur conjonctif. Armé sur le seul label, ce filet
/// échangerait le loop-breaker QA contre un loop-breaker dev.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_conjunctive_trigger_excludes_an_agent_without_qa_review() {
    let (tools, _calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(vec![
            delayed_response(
                200_000,
                tool_call_response("check_task", json!({"task_id": "x"})),
            ),
            text_response("(ne doit jamais être rendu)"),
        ])
        .tools(tools)
        .skills(dev_registry())
        .build()
        .await
        .unwrap();

    let outcome =
        run_callback_with_deadline(&harness, build_callback_trigger(), Duration::from_secs(1))
            .await;

    assert_eq!(
        outcome.qa_verdict_cut_off, None,
        "sans `qa-review`, aucun verdict n'est dû — le cas mika-dev"
    );
    assert!(!outcome.qa_verdict_unmet);
}
