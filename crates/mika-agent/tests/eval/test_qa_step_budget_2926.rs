//! mika#2926 AC2 + R1b — le plafond de steps d'un tour de mika-qa, mesuré sur le
//! chemin de production, et le plafond que le WARN de coupure annonce.
//!
//! # Le corpus rejoué, et pourquoi c'est celui-là
//!
//! La forme est celle de la session `callback-0f07c520` du 2026-10-26T18:10:28Z :
//! un callback de build réussi sur une PR de la boucle (mika#2920 à `0a55d28a`),
//! **23 `run_shell`** d'exploration puis la revue. Sous un plafond de 20 ce tour
//! est coupé au step 20 et la revue — qui venait au step 24 — n'est jamais
//! servie : c'est exactement ce qui a été mesuré, et la PR est restée sans
//! verdict.
//!
//! Les trois tours de ce fichier ne sont pas trois variantes du même : chacun
//! atteste un terme qu'aucun des deux autres ne peut voir.
//!
//! | tour | agent | ce qu'il atteste |
//! |---|---|---|
//! | positif | `mika-qa` | la revue du step 24 **est** servie : aucun décompte de coupure, la revue part |
//! | négatif | `mika-dev` | **la même séquence** épuise le budget : un décompte, `max_steps = 20`, aucune revue |
//! | R1b | `mika-qa` (silencieux) | le WARN de coupure annonce le plafond **servi**, pas celui du déclencheur |
//!
//! Le contrôle négatif tourne sur la **séquence identique** : c'est ce qui sépare
//! « le plafond de mika-qa a monté » de « le plafond a monté pour tout le monde »
//! (AC3). Un contrôle avec une séquence plus courte n'aurait attesté que
//! lui-même.
//!
//! # Pourquoi le troisième tour passe par `run_silent_agent`
//!
//! `EvalHarness::run` appelle `run_agent` : `callback_turn(true)` y pose un
//! drapeau d'`AgentParams`, mais le mode reste `LoopMode::Conversation`. Le WARN
//! `"silent agent exceeded max tool steps"` de R1b vit dans `run_silent_inner`,
//! donc seul un vrai tour silencieux l'atteint — d'où le pilotage direct, sur le
//! modèle des onze tests eval qui le font déjà (`test_qa_cut_off_verdict_2515.rs`
//! est le plus proche). C'est aussi le seul tour des trois où les deux plafonds
//! **divergent** : `SilentTrigger::Callback::max_steps()` rend 20 tandis que la
//! boucle sert 30, et c'est très exactement la dérive que R1b ferme.
//!
//! # Rouge-avant, recette exécutable par le relecteur
//!
//! 1. `git stash` du commit `feat(2926)` (ou revenir au parent de ce commit).
//! 2. `cargo test -p mika-agent --test eval qa_step_budget_2926`
//! 3. **Le tour positif rougit** : la revue n'est pas postée (coupure au step 20)
//!    et un `max_steps_tool_census` est émis.
//! 4. **Le tour de R1b rougit** : le WARN porte `max_steps = "20"`.
//! 5. **Le tour négatif reste vert** — sur mika-dev, 20 = 20 des deux côtés. C'est
//!    l'information qui dit que le changement est *par agent* : si ce tour
//!    rougissait après le correctif, le plafond se serait appliqué à tout le
//!    monde.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mika_agent::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::qa_build_callback::BUILD_CALLBACK_LABEL;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::{Value, json};

use super::harness::EvalHarness;
// Le capteur de tracing de mika#2305, réutilisé et non réécrit — on hérite ainsi
// de son `keep_registry_multi_dispatcher()` et de son raisonnement sur le cache
// d'intérêt mono-dispatcher de `tracing-core` 0.1.36. Même précédent que
// `test_max_steps_tool_census_2877.rs`, dont ce fichier est le frère.
use super::test_context_scope_observability_2305::{Captured, capture};

/// Le nom de l'événement de recensement, écrit en littéral plutôt qu'importé de
/// la production : c'est un **format de fil** que l'opérateur grep (la commande
/// d'AC4 le compte), donc le test doit rougir si quelqu'un le renomme.
const CENSUS_EVENT: &str = "max_steps_tool_census";

/// Le message du WARN de coupure du tour silencieux, lui aussi en littéral.
///
/// Attention en le lisant dans un journal : `"agent exceeded max tool steps"` est
/// une **sous-chaîne** de celui-ci, donc un `grep` du premier ramasse les deux
/// populations. Ici l'égalité est exacte, donc le piège ne s'applique pas ; la
/// sonde opérateur, elle, doit soustraire (voir `docs/agent-internals/observability.md`).
const SILENT_CUT_OFF_WARN: &str = "silent agent exceeded max tool steps";

/// Nombre de `run_shell` du corpus rejoué — la mesure de `callback-0f07c520`.
const RUN_SHELL_CALLS: usize = 23;

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

/// Un outil qui enregistre ses appels et ne fait rien d'autre.
///
/// `run_shell` et `run_gh` sont des outils de skill, absents de
/// `default_tools()` : la fixture est donc le seul fournisseur du nom, et aucun
/// `gh` ni aucun shell n'est exécuté.
struct Recording {
    name: &'static str,
    calls: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl Tool for Recording {
    fn name(&self) -> &str {
        self.name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: format!("{} (fixture mika#2926)", self.name),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        Ok(ToolOutput::success("ok"))
    }
}

struct NoopSender;

#[async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Le registre du tour : `run_shell` et `run_gh`, chacun avec son journal
/// d'appels.
fn qa_tools() -> (ToolRegistry, Arc<Mutex<Vec<Value>>>, Arc<Mutex<Vec<Value>>>) {
    let shell_calls = Arc::new(Mutex::new(Vec::new()));
    let gh_calls = Arc::new(Mutex::new(Vec::new()));
    let mut tools = default_tools();
    tools.register(Box::new(Recording {
        name: "run_shell",
        calls: shell_calls.clone(),
    }));
    tools.register(Box::new(Recording {
        name: "run_gh",
        calls: gh_calls.clone(),
    }));
    (tools, shell_calls, gh_calls)
}

// ---------------------------------------------------------------------------
// Le corpus
// ---------------------------------------------------------------------------

/// La séquence rejouée : `RUN_SHELL_CALLS` explorations, la revue, puis l'EndTurn.
///
/// **Une seule séquence pour le tour positif et pour son contrôle négatif** —
/// c'est la condition d'AC2 : le corpus ne change pas, seul l'agent change.
fn replayed_corpus() -> Vec<MockResponse> {
    let mut responses: Vec<MockResponse> = (0..RUN_SHELL_CALLS)
        .map(|i| {
            tool_call_response(
                "run_shell",
                json!({"command": format!("git diff --stat HEAD~{i}")}),
            )
        })
        .collect();
    // Le step 24 : la revue que le plafond de 20 n'a jamais laissé servir.
    responses.push(tool_call_response(
        "run_gh",
        json!({"command": ["pr", "review", "2920", "--comment", "--body",
                           "PLAN-AC VERIFICATION\n\nVERDICT: hold[review]"]}),
    ));
    responses.push(text_response(
        "Revue postée : VERDICT: hold[review] sur la tête 0a55d28a.",
    ));
    responses
}

/// Tous les événements de recensement captés — le filtre partagé par les deux
/// premiers tours. Un contrôle d'absence n'atteste quelque chose que s'il regarde
/// exactement là où son frère trouve quelque chose.
fn census_events(events: &Captured) -> Vec<HashMap<String, String>> {
    let captured = events.lock().expect("capture mutex");
    captured
        .iter()
        .filter(|(_, f)| f.get("event").map(String::as_str) == Some(CENSUS_EVENT))
        .map(|(_, f)| f.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// AC2 — le tour positif et son contrôle négatif
// ---------------------------------------------------------------------------

/// **AC2 (positif)** — sur le corpus rejoué, mika-qa sert la revue du step 24.
#[tokio::test]
async fn mika2926_le_corpus_rejoue_pose_son_verdict_sous_le_budget() {
    let (tools, shell_calls, gh_calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(replayed_corpus())
        .tools(tools)
        .agent_id("mika-qa")
        .callback_turn(true)
        .message_sender(Arc::new(NoopSender))
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness
        .run("[callback: build] Build succeeded for PR #2920")
        .await
        .unwrap();

    assert_eq!(
        census_events(&events).len(),
        0,
        "un tour qui conclut ne doit émettre aucun {CENSUS_EVENT} — c'est le \
         compte que la sonde d'AC4 regarde baisser"
    );
    assert_eq!(
        shell_calls.lock().unwrap().len(),
        RUN_SHELL_CALLS,
        "les {RUN_SHELL_CALLS} explorations du corpus ont toutes été servies"
    );

    let reviews = gh_calls.lock().unwrap();
    assert_eq!(
        reviews.len(),
        1,
        "exactement une revue, postée par le tour lui-même"
    );
    assert!(
        reviews[0].to_string().contains("VERDICT: hold[review]"),
        "la revue postée porte le verdict du corpus : {:?}",
        reviews[0]
    );
}

/// **AC2 (contrôle négatif, dans le même test)** — la **même** séquence épuise le
/// budget sur un autre agent, et aucune revue ne part.
///
/// C'est la moitié qui tient AC3 : le plafond relevé est celui de mika-qa, pas
/// celui du moteur. Si ce tour verdissait sur 30, le changement aurait fuité vers
/// tous les agents sans que rien ne le dise.
#[tokio::test]
async fn mika2926_la_meme_forme_epuise_le_budget_sur_un_autre_agent() {
    let (tools, shell_calls, gh_calls) = qa_tools();
    let harness = EvalHarness::builder()
        .responses(replayed_corpus())
        .tools(tools)
        .agent_id("mika-dev")
        .callback_turn(true)
        .message_sender(Arc::new(NoopSender))
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness
        .run("[callback: build] Build succeeded for PR #2920")
        .await
        .unwrap();

    let mut found = census_events(&events);
    assert_eq!(
        found.len(),
        1,
        "exactement un {CENSUS_EVENT} pour le tour coupé, or {} vus : {found:?}",
        found.len()
    );
    let event = found.pop().expect("checked non-empty");

    assert_eq!(
        event.get("max_steps").map(String::as_str),
        Some("20"),
        "le plafond du moteur reste 20 pour les autres agents : {event:?}"
    );
    assert_eq!(
        event.get("per_tool").map(String::as_str),
        Some(r#"{"run_shell":20}"#),
        "le budget a brûlé en `run_shell`, et la carte est exacte : {event:?}"
    );
    assert_eq!(
        shell_calls.lock().unwrap().len(),
        mika_agent::planning::policy::MAX_TOOL_STEPS,
        "la coupure tombe au plafond du moteur, pas plus loin"
    );
    assert!(
        gh_calls.lock().unwrap().is_empty(),
        "la revue du step 24 n'a jamais été servie — c'est la PR sans verdict de \
         mika#2920"
    );
}

// ---------------------------------------------------------------------------
// R1b — le plafond annoncé est le plafond servi
// ---------------------------------------------------------------------------

/// **R1b** — sur un tour de callback de mika-qa coupé, le WARN annonce **30**.
///
/// Ni le tour positif (qui ne s'épuise pas) ni le contrôle négatif (où 20 = 20
/// des deux côtés) ne peut révéler cette dérive : le WARN lisait
/// `params.trigger.max_steps()`, la boucle servait le plafond résolu par agent, et
/// les deux ne coïncident que sur les agents que ce ticket ne change pas. Un
/// opérateur lisant ce WARN aurait diagnostiqué un tour coupé à 20 steps alors
/// qu'il en avait consommé 30.
#[tokio::test]
async fn mika2926_le_warn_de_coupure_annonce_le_plafond_servi() {
    let (tools, shell_calls, _gh_calls) = qa_tools();
    // Un step de plus que le plafond attendu : si la boucle en servait 31 ou
    // plus, l'EndTurn final serait consommé et aucun WARN ne serait émis — le
    // test rougirait plutôt que de passer pour la mauvaise raison.
    let mut responses: Vec<MockResponse> = (0..31)
        .map(|i| {
            tool_call_response(
                "run_shell",
                json!({"command": format!("cargo test --quiet pass_{i}")}),
            )
        })
        .collect();
    responses.push(text_response("(ne doit jamais être rendu)"));

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools)
        .agent_id("mika-qa")
        .build()
        .await
        .unwrap();

    let skills_dirty = AtomicBool::new(false);
    let params = SilentAgentParams {
        tier: harness.tier,
        deployment: harness.deployment,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger: SilentTrigger::Callback {
            task_id: "task-2926".to_string(),
            label: BUILD_CALLBACK_LABEL.to_string(),
            result: "Build succeeded".to_string(),
            failed: false,
            parent_task_id: None,
        },
        home_dir: harness.home_dir.path(),
        session_id: &harness.session_id,
        message_sender: Some(Arc::new(NoopSender)),
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

    let (_guard, events) = capture();
    run_silent_agent(&params).await.expect("silent turn runs");

    let warns: Vec<HashMap<String, String>> = {
        let captured = events.lock().expect("capture mutex");
        captured
            .iter()
            .filter(|(message, _)| message == SILENT_CUT_OFF_WARN)
            .map(|(_, f)| f.clone())
            .collect()
    };
    assert_eq!(
        warns.len(),
        1,
        "un tour silencieux coupé émet exactement un {SILENT_CUT_OFF_WARN}, or \
         {} vus : {warns:?}",
        warns.len()
    );
    assert_eq!(
        warns[0].get("max_steps").map(String::as_str),
        Some("30"),
        "le WARN doit porter le plafond que la boucle a SERVI, non celui que le \
         déclencheur déclare : {:?}",
        warns[0]
    );
    // 30 en littéral, et non `MAX_TOOL_STEPS_MIKA_QA` : ce commit doit **compiler**
    // avant le correctif pour que son rouge dise *quelle* assertion portait la
    // charge — « ne compile pas » n'est pas un rouge-avant (doctrine du frère
    // mika#2877). La constante est tenue ailleurs, par le garde du prompt (R2).
    assert_eq!(
        shell_calls.lock().unwrap().len(),
        30,
        "la coupure tombe au plafond de mika-qa"
    );
}
