//! Integration tests: intent-precondition registry (#702).
//!
//! Verifies the registry-driven intent-precondition guard that generalizes
//! the webhook zero-tools guard (#696) and adds the resume-reconcile guard.
//!
//! The webhook guard tests remain in `test_webhook_zero_tools_guard.rs` for
//! backward compatibility; this file covers the resume-reconcile entry and
//! verifies the registry dispatch mechanism.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mika_agent::agent_loop::VERDICT_PRODUCER_SKILLS;
use mika_agent::agent_loop::review_anchor::{RETRY_PLAN_BEGIN, RETRY_PLAN_END};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::assertions::*;
use super::harness::EvalHarness;
use super::trace::AgentTrace;

/// Stub tool that simulates `list_tasks` with a successful result.
struct StubListTasksTool;

#[async_trait]
impl Tool for StubListTasksTool {
    fn name(&self) -> &str {
        "list_tasks"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_tasks".to_string(),
            description: "Stub list_tasks for intent guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "1 item — 1 in_progress\n| WI | Status | Type | Source | Label |\n| abc-123 | in_progress | milestone | manual | mika milestone#8 |".to_string(),
        ))
    }
}

/// Stub tool that simulates `check_task` with a successful result.
struct StubCheckTaskTool;

#[async_trait]
impl Tool for StubCheckTaskTool {
    fn name(&self) -> &str {
        "check_task"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "check_task".to_string(),
            description: "Stub check_task for intent guard tests".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(
            "Task abc-123: in_progress, type=milestone, label=mika milestone#8".to_string(),
        ))
    }
}

/// Build a tool registry with reconciliation stub tools.
fn tools_with_reconciliation() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(StubListTasksTool));
    tools.register(Box::new(StubCheckTaskTool));
    tools
}

/// Guard fires when agent receives a resume-intent message and responds
/// with text only (zero reconciliation tool calls).
#[tokio::test]
async fn resume_guard_fires_on_zero_reconciliation_tools() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent narrates without calling tools — guard rejects
            text_response(
                "I'll resume the milestone work. Let me check what needs to be done next.",
            ),
            // Step 2: After re-prompt, agent calls list_tasks then responds
            tool_call_response("list_tasks", json!({})),
            text_response("Found the milestone. Resuming child issue dispatch."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    // 3 steps: rejected text + tool call + final text
    assert_exact_steps(&trace, 3);
    assert_output_contains(&trace, "Resuming child issue dispatch");
}

/// Guard does NOT fire when the agent calls check_task successfully.
#[tokio::test]
async fn resume_guard_skips_when_check_task_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("check_task", json!({})),
            // The agent loop may issue a follow-up after the tool result;
            // provide enough responses to avoid exhaustion.
            tool_call_response("list_tasks", json!({})),
            text_response("Milestone is in_progress. Resuming next child."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    assert_output_contains(&trace, "Resuming next child");
}

/// Guard does NOT fire when the agent calls list_tasks successfully.
#[tokio::test]
async fn resume_guard_skips_when_list_tasks_called() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("list_tasks", json!({})),
            text_response("Found in-progress milestone. Dispatching next child."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("continue mika project#3").await.unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "Dispatching next child");
}

/// Guard does NOT fire on regular messages without resume intent.
#[tokio::test]
async fn resume_guard_skips_on_regular_messages() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "Here's an overview of the project architecture.",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("Can you explain the project architecture?")
        .await
        .unwrap();

    assert_has_output(&trace);
    // 1 step only — no re-prompt
    assert_exact_steps(&trace, 1);
}

/// Guard does NOT fire when "resume" appears without a process reference.
#[tokio::test]
async fn resume_guard_skips_without_process_ref() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response(
            "I'll resume working on the feature now.",
        )])
        .build()
        .await
        .unwrap();

    let trace = harness
        .run("resume working on the auth feature")
        .await
        .unwrap();

    assert_has_output(&trace);
    assert_exact_steps(&trace, 1);
}

/// Guard fires at most once (single-retry semantics). If the agent still
/// produces zero reconciliation tool calls after the re-prompt, the turn
/// ends normally.
#[tokio::test]
async fn resume_guard_fires_only_once() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent narrates — guard rejects
            text_response("I see the milestone needs resuming."),
            // Step 2: Agent narrates AGAIN — guard already fired, lets through
            text_response("The milestone has been noted for resumption."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    // 2 steps: first rejected + second allowed through
    assert_exact_steps(&trace, 2);
    assert_output_contains(&trace, "noted for resumption");
}

/// Guard fires when list_tasks is called but fails (success=false).
#[tokio::test]
async fn resume_guard_fires_when_tool_fails() {
    // Stub that always fails
    struct FailingListTasksTool;

    #[async_trait]
    impl Tool for FailingListTasksTool {
        fn name(&self) -> &str {
            "list_tasks"
        }

        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "list_tasks".to_string(),
                description: "Failing stub".to_string(),
                input_schema: json!({"type": "object"}),
            }
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput::error("Database error".to_string()))
        }
    }

    let mut tools = default_tools();
    tools.register(Box::new(FailingListTasksTool));

    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Agent responds text-only (no tool calls) — guard fires
            text_response("I see the milestone needs to be resumed."),
            // Step 2 (after re-prompt): Agent calls list_tasks but it fails,
            // then narrates — guard already fired (single-retry), lets through.
            tool_call_response("list_tasks", json!({})),
            text_response("I tried to list tasks but encountered an error."),
        ])
        .tools(tools)
        .build()
        .await
        .unwrap();

    let trace = harness.run("resume mika milestone#8").await.unwrap();

    assert_has_output(&trace);
    // 3 steps: text (rejected) + tool call (list_tasks fails) + text (passes)
    assert_exact_steps(&trace, 3);
    assert_output_contains(&trace, "encountered an error");
}

/// Guard fires on "continue" verb with project reference.
#[tokio::test]
async fn resume_guard_fires_on_continue_with_project() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Step 1: Text-only on continue+project — rejected
            text_response("I'll continue the project work."),
            // Step 2: Agent acts properly
            tool_call_response("list_tasks", json!({})),
            text_response("Checked project status and resuming."),
        ])
        .tools(tools_with_reconciliation())
        .build()
        .await
        .unwrap();

    let trace = harness.run("continue mika project#5").await.unwrap();

    assert_has_output(&trace);
    // 3 steps: rejected text + tool call + final text
    assert_exact_steps(&trace, 3);
}

// ---------------------------------------------------------------------------
// mika#3002 — le stand-down sur un tour de revue d'architecte
// ---------------------------------------------------------------------------

/// Début fixe du re-prompt correctif de `resume_reconcile` (`agent_loop/mod.rs`, #702).
///
/// C'est l'unique chose que ce test compte. Un `assert_exact_steps` resterait vert, ou
/// rougirait, pour les raisons d'une autre garde ; cette tête-là ne bouge que si l'entrée
/// qu'on mesure bouge. Même instrument que `ANCHOR_REPROMPT_HEAD` dans
/// `grounding_regressions/review_anchor_unparsed_retry_2996.rs`.
const RESUME_REPROMPT_HEAD: &str = "[mika-engine] A resume/continue instruction was received \
     for a milestone or project but no reconciliation tools were called";

/// Noms de [`VERDICT_PRODUCER_SKILLS`] exemptés du contrôle de répertoire ci-dessous.
///
/// **Livrée VIDE, et épinglée vide** par l'assertion sœur du test. Une entrée ajoutée un
/// jour doit l'être délibérément, en éteignant l'épingle — ce qui est le geste visible que
/// l'exception exige (mika#2425, mika#1833).
const SKILL_DIR_EXEMPTIONS: &[&str] = &[];

/// Un plan de groom ordinaire : il parle de reprise parce que c'est son sujet, et il cite
/// un jalon. Les deux conjoints de `detect_resume_intent` sont là, et aucune intention de
/// reprise n'est adressée à l'agent.
///
/// Les mots qui portent les conjoints sont ceux **mesurés** sur le plan de mika#2995
/// (`tests/fixtures/review_anchor_unparsed_retry_2996/plan-mika2995-61a3fb572.md`, l. 479
/// et 506), le brief de l'incident `3a066c59` : le verbe vient du français « le prompt
/// servi *continue* de prescrire », que `\b(resume|continue)\b` attrape mot pour mot, et le
/// processus d'un « milestone #33 » **cité**. La ligne « reprendre le vol suivant » que
/// nomme AC2 est là aussi, et ne porte ni l'un ni l'autre — c'est le vocabulaire du défaut,
/// pas son déclencheur, et un test construit sur elle seule ne mesurerait rien.
const PLAN_SOUS_REVUE: &str = "## 4. Ordre de vol\n\n\
     Le premier vol livre le détecteur ; il faudra reprendre le vol suivant une fois la \
     sonde relevée.\n\n\
     Le défaut survit au correctif : le prompt servi continue de prescrire l'ancienne \
     forme, et le détecteur sans la correction laisse passer la forme neuve.\n\n\
     Lié : milestone #33, dont ce ticket est l'enfant.\n";

/// Un registre d'un seul skill, nommé depuis [`VERDICT_PRODUCER_SKILLS`], qui fait du tour
/// un tour de revue d'architecte — c'est `has_verdict_producer_skill` sur ce registre qui
/// résout `is_verdict_producer`.
///
/// `always_on` comme en production : un brief de plan ne porte aucun mot-clé de
/// déclenchement. `[output]` est laissé **par défaut**, à la différence de
/// `registry_from_manifest` du fichier mika#2996 : ici la garde d'ancrage n'est pas le
/// sujet, et son re-prompt consommerait les réponses simulées sans rien dire de celle-ci —
/// même raisonnement que la doc de `registry_from_manifest` tient sur `required_tools`.
fn registre_de_revue(skill: &str) -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: skill.to_string(),
                description: format!("{skill} (tour de revue, contrat [output] par défaut)"),
                version: "0.1.0".to_string(),
                always_on: true,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers { keywords: vec![] },
            llm: Default::default(),
            constraints: Default::default(),
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from(format!("/skills/{skill}")),
        keywords_lower: vec![],
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }])
}

/// AC1 + AC2 (mika#3002) — sur un tour de revue, les mots de reprise du PLAN ne
/// déclenchent pas la garde ; une instruction de reprise HORS du plan, si.
///
/// Un test, deux moitiés, même fixture de plan et même harnais, parce que c'est la paire
/// qui dit quelque chose : le stand-down seul se lirait aussi bien comme « la garde ne
/// s'arme plus jamais », et le tir seul comme « le plan ne déclenchait rien » (le fichier
/// mika#2996 a déjà engravé le raisonnement).
#[tokio::test]
async fn mika3002_le_plan_revu_ne_declenche_pas_la_garde_mais_une_instruction_hors_plan_si()
-> anyhow::Result<()> {
    let skill = VERDICT_PRODUCER_SKILLS[0];

    // --- Positif (AC1) : le brief EST le plan, forme exacte de la première passe et de
    // l'incident `3a066c59`.
    //
    // DEUX réponses pour UN pas attendu, et la seconde est du rab délibéré : sans elle, un
    // re-prompt épuise la mock et le test rougit sur « exhausted all 1 responses » — un
    // rouge qui ne nomme pas ce qu'il mesure, et que n'importe quelle autre garde rendrait
    // à l'identique. Avec elle, c'est l'assertion de compte ci-dessous qui parle (même
    // raisonnement que `REPLIES` dans `grounding_regressions/review_anchor_unparsed_retry_2996.rs`).
    let reponse =
        text_response("A1: « il faudra reprendre le vol suivant » — noté.\n\nDisposition: READY");
    let positif = EvalHarness::builder()
        .responses(vec![reponse.clone(), reponse])
        .skills(registre_de_revue(skill))
        .tools(tools_with_reconciliation())
        .build()
        .await?;
    let trace = positif.run(PLAN_SOUS_REVUE).await?;
    assert_has_output(&trace);
    assert_eq!(
        resume_reprompts(&trace),
        0,
        "mika#3002 : les mots de reprise du plan revu ont armé la garde sur un tour de \
         revue — c'est l'incident `3a066c59`, où l'architecte a répondu en 264 caractères \
         qu'il cherchait des engagements actifs au lieu de rendre une revue.\n\
         Réponse finale :\n{}",
        trace.output.text.clone().unwrap_or_default()
    );

    // --- Négatif (AC2) : même plan, mais sous l'enveloppe de mika#2996 et précédé d'une
    // VRAIE instruction adressée à l'agent. Elle vit hors du plan, donc dans la région
    // lue : la garde doit s'armer exactement comme avant.
    let brief = format!(
        "resume mika milestone#8\n\n{RETRY_PLAN_BEGIN}\n{PLAN_SOUS_REVUE}{RETRY_PLAN_END}\n"
    );
    let negatif = EvalHarness::builder()
        .responses(vec![
            // Tour 1 : l'agent narre sans réconcilier — la garde re-prompte.
            text_response("Je reprends le jalon. Voyons ce qu'il reste à faire."),
            // Tour 2 : il appelle `list_tasks`, ce qui satisfait la précondition.
            tool_call_response("list_tasks", json!({})),
            text_response("Jalon in_progress. Je reprends l'enfant suivant."),
        ])
        .skills(registre_de_revue(skill))
        .tools(tools_with_reconciliation())
        .build()
        .await?;
    let trace = negatif.run(&brief).await?;
    assert_has_output(&trace);
    assert_eq!(
        resume_reprompts(&trace),
        1,
        "mika#3002 : le stand-down a fui hors de sa population — une instruction de reprise \
         adressée à l'agent, hors du plan revu, doit toujours armer la garde.\n\
         Réponse finale :\n{}",
        trace.output.text.clone().unwrap_or_default()
    );
    assert_output_contains(&trace, "l'enfant suivant");

    Ok(())
}

/// Le contrat sur lequel tout le reste repose, et la seule chose qui pourrisse en silence
/// dans ce dessin : chaque nom de [`VERDICT_PRODUCER_SKILLS`] nomme un répertoire existant
/// sous `skills/bundled/`.
///
/// Un skill renommé ou retiré ne casse rien de visible : `has_verdict_producer_skill` cesse
/// simplement de dire vrai, `is_verdict_producer` reste faux sur tous les tours de revue,
/// et la garde recommence à s'armer sur les plans — sans qu'aucun test ne rougisse, puisque
/// le test ci-dessus construit son registre depuis la même constante devenue fausse.
#[test]
fn mika3002_les_noms_de_verdict_producer_skills_nomment_des_bundles_existants() {
    assert!(
        SKILL_DIR_EXEMPTIONS.is_empty(),
        "l'allowlist de ce détecteur est livrée vide et épinglée vide (mika#3002 § 7). \
         Pour y ajouter un nom, éteignez cette épingle dans le même commit et dites \
         pourquoi le skill n'a pas de bundle : c'est le geste délibéré que l'exception \
         exige, pas un oubli silencieux."
    );

    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/bundled");
    for skill in VERDICT_PRODUCER_SKILLS {
        if SKILL_DIR_EXEMPTIONS.contains(skill) {
            continue;
        }
        let dir = bundled.join(skill);
        assert!(
            dir.is_dir(),
            "VERDICT_PRODUCER_SKILLS nomme `{skill}`, mais {} n'existe pas. La population \
             du stand-down de la garde de reprise (mika#3002) est vide pour ce nom, et rien \
             d'autre ne le dirait.",
            dir.display()
        );
    }
}

/// Combien de requêtes LLM dont le dernier message est le re-prompt de `resume_reconcile`.
fn resume_reprompts(trace: &AgentTrace) -> usize {
    trace
        .captured_requests
        .iter()
        .filter(|req| {
            matches!(
                req.messages.last().map(|m| &m.content),
                Some(mika_common::llm::LlmContent::Text(t))
                    if t.starts_with(RESUME_REPROMPT_HEAD)
            )
        })
        .count()
}
