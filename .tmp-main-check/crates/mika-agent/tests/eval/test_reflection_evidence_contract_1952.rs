//! mika#1952 — en mode réflexion, le schéma **servi** déclare `evidence` requis.
//!
//! Ces tests pilotent le **chemin de production** : `run_silent_agent` avec
//! `SilentTrigger::Reflection`, puis assertion sur
//! `MockLlmProvider::captured_requests()[0].tools` — c'est-à-dire sur le tableau
//! d'outils réellement envoyé au fournisseur, pas sur la valeur de retour du
//! filtre.
//!
//! **Cette distinction est le test.** `apply_reflection_evidence_contract` mute
//! un `&mut [ToolDefinition]` en place, et la conversion
//! `From<ToolDefinition> for LlmToolDefinition` déplace le schéma sans le lire.
//! Un filtre déplacé *après* cette conversion, ou supprimé du site d'appel, ne
//! ferait rougir **aucune** assertion unitaire : la fonction continuerait de
//! faire ce qu'on lui demande, personne ne la demanderait plus. C'est le risque
//! R1 du plan, et c'est pourquoi la preuve est prise à la sortie du tuyau.
//!
//! Ce que chaque test épingle :
//! - **U4-b / AC5** — les trois schémas servis en réflexion portent `evidence`
//!   dans `required`.
//! - **U4-c / M3** — contrôle négatif sur `SilentTrigger::Heartbeat` : les trois
//!   schémas sont inchangés, **et** la liste des noms d'outils servis est
//!   identique entre les deux tours (M4 : la surface d'outils n'a pas bougé, ce
//!   n'est pas l'Option 2 que le ticket met hors périmètre).
//! - **U4-d / M5** — `required` n'est pas une garantie dure : un appel réel
//!   portant `"evidence": ""` est toujours refusé par
//!   `check_reflection_evidence`. Lire ce test avant de croire la garde
//!   d'exécution redondante.

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

use mika_agent::agent::{SilentAgentParams, SilentTrigger, run_silent_agent};
use mika_common::llm::mock::*;
use mika_common::llm::{
    LlmContent, LlmContentBlock, LlmRequest, LlmToolDefinition, LlmToolResultContent,
};
use serde_json::json;

use super::harness::EvalHarness;

/// Le message exact que `tools::check_reflection_evidence` renvoie. Écrit ici
/// plutôt qu'importé : le contrat que mika#1770 a mesuré est ce texte-là, et
/// s'il change, ce test doit le dire plutôt que de suivre.
const GUARD_MESSAGE: &str =
    "Reflection mode requires an evidence field citing specific conversation content.";

/// Les trois outils que `check_reflection_evidence` garde.
const GATED: [&str; 3] = ["update_fact", "store_fact", "update_core_memory"];

// ---------------------------------------------------------------------------
// Pilotage du tour silencieux
// ---------------------------------------------------------------------------

async fn run_silent(harness: &EvalHarness, trigger: SilentTrigger) {
    let skills_dirty = AtomicBool::new(false);
    let params = SilentAgentParams {
        tier: harness.tier,
        deployment: harness.deployment,
        db: &harness.db,
        llm: harness.llm.as_ref(),
        tools: &harness.tools,
        skills: &harness.skills,
        trigger,
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

/// Le `required` déclaré par l'outil `name` dans le tableau **servi**.
fn served_required(tools: &[LlmToolDefinition], name: &str) -> Vec<String> {
    let def = tools
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("`{name}` is in the served tool array"));
    def.parameters
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn served_tools(req: &LlmRequest) -> &[LlmToolDefinition] {
    req.tools
        .as_deref()
        .expect("a silent turn serves a non-empty tool array")
}

fn served_tool_names(req: &LlmRequest) -> HashSet<String> {
    served_tools(req).iter().map(|t| t.name.clone()).collect()
}

/// Tous les `tool_result` portés par une requête capturée, avec leur drapeau
/// d'erreur.
fn tool_results(req: &LlmRequest) -> Vec<(String, bool)> {
    req.messages
        .iter()
        .filter_map(|m| match &m.content {
            LlmContent::Blocks(blocks) => Some(blocks),
            LlmContent::Text(_) => None,
        })
        .flatten()
        .filter_map(|b| match b {
            LlmContentBlock::ToolResult {
                content, is_error, ..
            } => match content {
                LlmToolResultContent::Text(t) => Some((t.clone(), *is_error)),
                LlmToolResultContent::Blocks(_) => None,
            },
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// U4-b — le contrat servi en mode réflexion
// ---------------------------------------------------------------------------

/// Le test que le plan nomme. Il prend la preuve sur `captured_requests()`,
/// c'est-à-dire sur ce que le fournisseur a reçu.
#[tokio::test]
async fn mika1952_the_reflection_turn_serves_evidence_as_required() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Nothing to consolidate today.")])
        .build()
        .await
        .unwrap();

    run_silent(&harness, SilentTrigger::Reflection).await;

    let requests = harness.mock().captured_requests();
    assert_eq!(requests.len(), 1, "one EndTurn, one LLM call");
    let tools = served_tools(&requests[0]);

    for name in GATED {
        let required = served_required(tools, name);
        assert!(
            required.contains(&"evidence".to_string()),
            "mika#1952 AC5 — the schema SERVED for `{name}` in reflection mode must declare \
             `evidence` in `required`. Got {required:?}.\n\n\
             If this is red, the likely cause is not the filter but its call site: \
             `apply_reflection_evidence_contract` must run in `run_silent_agent` BEFORE the \
             `From<ToolDefinition> for LlmToolDefinition` conversion moves the schema."
        );
    }

    // Additif : les champs déjà obligatoires survivent.
    assert_eq!(
        served_required(tools, "update_fact"),
        vec!["id", "category", "updates", "evidence"],
        "the filter appends; it must not reorder or drop what the tool already required"
    );
}

/// La description du champ est celle du site unique (D5), et elle est la même
/// pour les trois — y compris `update_core_memory`, dont le « **Only** required
/// in reflection mode » avait déjà dérivé vers la minimisation (M1).
#[tokio::test]
async fn mika1952_the_three_tools_carry_one_evidence_description() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Nothing to consolidate today.")])
        .build()
        .await
        .unwrap();

    run_silent(&harness, SilentTrigger::Reflection).await;

    let requests = harness.mock().captured_requests();
    let tools = served_tools(&requests[0]);

    let evidence_description = |name: &str| -> String {
        let def = tools.iter().find(|t| t.name == name).unwrap();
        def.parameters["properties"]["evidence"]["description"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    let descriptions: Vec<String> = GATED.iter().map(|n| evidence_description(n)).collect();

    assert!(
        descriptions[0].starts_with("REQUIRED IN REFLECTION MODE."),
        "AC1 — the imposed text, verbatim. Got: {}",
        descriptions[0]
    );
    assert!(
        descriptions.windows(2).all(|w| w[0] == w[1]),
        "D5 — the three descriptions are one text, from one site. Got:\n{descriptions:#?}"
    );
    assert!(
        !descriptions[2].contains("Only required"),
        "M1 — `update_core_memory`'s minimising wording must be gone"
    );
}

// ---------------------------------------------------------------------------
// U4-c — contrôle négatif : les autres modes sont hors population
// ---------------------------------------------------------------------------

/// **M3** — `is_reflection` est écrit `false` en dur aux deux autres sites de
/// construction de `ToolContext`, donc la mutation ne peut atteindre qu'un tour
/// `Reflection`. C'est une lecture du code, pas une preuve : ce test la rend
/// vérifiable sur un tour silencieux voisin.
///
/// **M4** — et la liste des noms servis est identique entre les deux tours.
/// C'est la distinction d'avec l'Option 2 que mika#1952 met hors périmètre :
/// son critère est la *surface* (« doubles the tool surface area »), pas le
/// mécanisme. Zéro entrée ajoutée, zéro retirée.
#[tokio::test]
async fn mika1952_a_heartbeat_turn_serves_the_unchanged_schema_and_the_same_surface() {
    let reflection = EvalHarness::builder()
        .responses(vec![text_response("Nothing to consolidate today.")])
        .build()
        .await
        .unwrap();
    run_silent(&reflection, SilentTrigger::Reflection).await;

    let heartbeat = EvalHarness::builder()
        .responses(vec![text_response("Nothing to report.")])
        .build()
        .await
        .unwrap();
    run_silent(&heartbeat, SilentTrigger::Heartbeat).await;

    let reflection_req = reflection.mock().captured_requests();
    let heartbeat_req = heartbeat.mock().captured_requests();
    let reflection_tools = served_tools(&reflection_req[0]);
    let heartbeat_tools = served_tools(&heartbeat_req[0]);

    // (a) Le schéma d'un tour heartbeat est inchangé.
    for name in GATED {
        let required = served_required(heartbeat_tools, name);
        assert!(
            !required.contains(&"evidence".to_string()),
            "M3 — a heartbeat turn does not carry the runtime guard, so its served schema for \
             `{name}` must be untouched. Got {required:?}"
        );
    }

    // (b) La surface — la liste des noms — est la même des deux côtés.
    assert_eq!(
        served_tool_names(&reflection_req[0]),
        served_tool_names(&heartbeat_req[0]),
        "M4 — mika#1952 changes the VALUE of a field in an existing schema; it registers no \
         tool and removes none. If this is red, something added or dropped a tool name and \
         the change has become the Option 2 the ticket put out of scope."
    );
    assert_eq!(
        reflection_tools.len(),
        heartbeat_tools.len(),
        "M4 — same count, both modes"
    );
}

// ---------------------------------------------------------------------------
// U4-d — la garde d'exécution reste la seule barrière dure
// ---------------------------------------------------------------------------

/// **M5** — Mika n'émet pas `strict: true`, et ni l'API Anthropic ni les rails
/// OpenAI-compatibles ne refusent côté serveur un appel auquel manque une clé
/// `required`. Le tableau `required` **oriente** le modèle ; il ne le contraint
/// pas. Et `"evidence": ""` satisfait `required` tout en échouant la garde
/// (`trim().is_empty()`).
///
/// Ce test existe pour qu'un futur lecteur ne retire pas
/// `check_reflection_evidence` en la croyant rendue redondante par la
/// correction du schéma.
#[tokio::test]
async fn mika1952_the_runtime_guard_is_still_the_hard_barrier() {
    let harness = EvalHarness::builder()
        .responses(vec![
            // Le modèle remplit le champ que le schéma lui demande… avec du vide.
            tool_call_response(
                "update_fact",
                json!({
                    "id": 52,
                    "category": "commitment",
                    "updates": {"status": "cancelled"},
                    "evidence": "   "
                }),
            ),
            text_response("I could not record that."),
        ])
        .build()
        .await
        .unwrap();

    run_silent(&harness, SilentTrigger::Reflection).await;

    let requests = harness.mock().captured_requests();
    assert_eq!(requests.len(), 2, "tool call → tool result → EndTurn");

    let results = tool_results(&requests[1]);
    assert_eq!(results.len(), 1, "exactly one tool result, got {results:?}");
    let (text, is_error) = &results[0];
    assert!(
        text.contains(GUARD_MESSAGE),
        "M5 — a blank `evidence` satisfies `required` and must still be refused by \
         `check_reflection_evidence`. Got: {text}"
    );
    assert!(is_error, "the refusal is a tool error, not a success");
}
