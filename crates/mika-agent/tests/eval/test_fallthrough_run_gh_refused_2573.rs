//! mika#2573 — un tour Webhook Fallthrough ne crée pas de travail par `run_gh`.
//!
//! **Pourquoi le chemin de production et pas le prédicat.** Frère de
//! `test_webhook_fallthrough_no_task_2517.rs`, et il suit le raisonnement que ce
//! fichier écrit déjà : les tests unitaires de `detect_fallthrough_work_creation`
//! (`evidence::guards::tests::mika2573`) restent verts si le champ
//! `ToolContext.is_webhook_fallthrough_turn` n'est **jamais calculé** ou **jamais
//! lu**, et c'est très exactement le mode de panne. Seul un vrai tour, dont la
//! garde tourne à sa position de production dans `run_gh`, ferme ça.
//!
//! **Les trois contrôles négatifs sont porteurs, pas décoratifs.** V1/V2 seuls
//! seraient satisfaits par une garde qui refuse `run_gh` sur *tout* tour
//! Fallthrough — c'est-à-dire par un correctif qui casse le geste de
//! vérification que la SCOPE RULE du prompt prescrit littéralement
//! (`issue view --json labels`). V3 (lecture sur un tour Fallthrough), V4
//! (ready-label, dans le domaine `[GitHub]` mais hors du domaine fallthrough) et
//! V5 (aucun préfixe `[GitHub]`) sont les trois axes qui séparent « la garde
//! décide » de « la garde bloque `run_gh` ».

use async_trait::async_trait;
use mika_agent::skills::builtin_handlers;
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// L'identifiant stable du refus, tel que le LLM le reçoit.
const REFUSAL: &str = "fallthrough_work_creation_refused";

/// Le déclencheur mesuré le 2026-09-28 : un commentaire de l'orchestrateur
/// fermant la PR #2567 (`marker_class: issue_comment`).
const FALLTHROUGH_MSG: &str = "[GitHub] New comment on senara-solutions/mika#2567 — la branche est conservée pour le re-groom";

/// Dans le domaine `[GitHub]`, **hors** du domaine fallthrough : le chemin de
/// dispatch, qui doit garder chaque verbe qu'il avait.
const READY_LABEL_MSG: &str = "[GitHub] Issue labeled ready on senara-solutions/mika#2562 — title";

/// Hors du domaine `[GitHub]` entièrement.
const PLAIN_MSG: &str = "pose le label ready sur mika issue#2562";

// -- Le vrai builtin, atteint par la boucle d'agent --

/// Délègue à `builtin_handlers::execute("run_gh", …)` pour que la garde tourne à
/// sa position de production — dans `run_gh`, avant le subprocess. Motif repris
/// de `test_pr_review_flag_coherence_2237.rs`.
struct BuiltinRunGhTool;

#[async_trait]
impl Tool for BuiltinRunGhTool {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Execute a GitHub CLI command".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "array", "items": {"type": "string"}},
                    "repo": {"type": "string"}
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(builtin_handlers::execute("run_gh", input, ctx).await)
    }
}

fn tools_with_builtin_run_gh() -> ToolRegistry {
    let mut registry = default_tools();
    registry.register(Box::new(BuiltinRunGhTool));
    registry
}

/// Assez de réponses texte pour absorber les re-prompts des intent-guards.
///
/// Un tour ready-label qui n'appelle pas `run_claude_pilot` est re-prompté par
/// `webhook_ready_label_dispatch`, et un tour `[GitHub]` sans appel d'outil par
/// `webhook_zero_tools`. C'est le comportement nominal des chemins que V4 et V5
/// exercent, pas un défaut de la fixture.
fn tail(n: usize) -> Vec<MockResponse> {
    (0..n).map(|_| text_response("Acknowledged.")).collect()
}

/// Lance un tour qui appelle `run_gh` une fois et rend la sortie de cet appel.
async fn run_gh_output(message: &str, command: serde_json::Value) -> (EvalHarness, String) {
    let mut responses = vec![tool_call_response("run_gh", json!({"command": command}))];
    responses.extend(tail(6));

    let harness = EvalHarness::builder()
        .responses(responses)
        .tools(tools_with_builtin_run_gh())
        .build()
        .await
        .unwrap();

    let trace = harness.run(message).await.unwrap();
    let calls = trace.calls_for_tool("run_gh");
    assert!(
        !calls.is_empty(),
        "the fixture must have reached run_gh — without a call there is nothing to assert"
    );
    let output = calls[0].output.clone().unwrap_or_default();
    (harness, output)
}

/// Les `(after_value, reasoning)` de chaque ligne `fallthrough_work_creation` de
/// la session de ce harness.
async fn refusal_rows(harness: &EvalHarness) -> Vec<(Option<String>, Option<String>)> {
    harness
        .db
        .get_audit_events(&harness.session_id)
        .await
        .expect("audit events must be readable")
        .into_iter()
        .filter(|e| e.tool_name == "fallthrough_work_creation")
        .map(|e| (e.after_value, e.reasoning))
        .collect()
}

// -------------------------------------------------------------------------
// V1 (AC1) — `issue edit … --add-label ready` sur un tour Fallthrough est refusé.
//
// L'argv mesuré le 2026-09-28 à 16:05:42Z, rejoué par le chemin de production.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2573_a_fallthrough_turn_may_not_post_the_dispatch_label() {
    let (harness, output) = run_gh_output(
        FALLTHROUGH_MSG,
        json!(["issue", "edit", "2562", "--add-label", "ready"]),
    )
    .await;

    assert!(
        output.contains(REFUSAL),
        "posting the canonical dispatch label (mika#841) on a Webhook Fallthrough turn \
         relaunches an implement dispatch nobody authorized — it must be refused; \
         got: {output}"
    );
    assert!(
        output.contains("mika#2517") && output.contains("mika#2573"),
        "the refusal must name both tickets — mika#2517 is the intent it enforces, \
         mika#2573 the substitute it closes; got: {output}"
    );
    // Le refus nomme sa sortie, sans quoi il se contourne au jugé (mika#2520).
    assert!(
        output.contains("send_message"),
        "the refusal must name the correct exit — alerting the operator; got: {output}"
    );

    // V8 (AC5) — le refus est comptable, et joignable par `trace_id` à
    // `webhook_fallthrough_turn` (mika#2517), qui porte le `marker_class`.
    let rows = refusal_rows(&harness).await;
    assert_eq!(
        rows.len(),
        1,
        "exactly one audit row per refusal — this is a dated event an operator counts, \
         not a tick classifying a population (mika#2131). Got: {rows:?}"
    );
    assert_eq!(
        rows[0].0.as_deref(),
        Some("ready_label_add"),
        "the motif is the wire format an operator GROUP BYs on"
    );
    let reasoning = rows[0].1.clone().unwrap_or_default();
    assert!(
        reasoning.contains("label=ready"),
        "the row must name the label as written; got: {reasoning}"
    );
}

// -------------------------------------------------------------------------
// V2 (AC2) — `issue create` sur un tour Fallthrough est refusé.
//
// L'argv mesuré le 2026-09-28 à 16:05:54Z : le substitut que le modèle a pris à
// la tâche que mika#2517 lui retenait, et qu'il a lui-même qualifié d'« erreur ».
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2573_a_fallthrough_turn_may_not_create_an_issue() {
    let (harness, output) = run_gh_output(
        FALLTHROUGH_MSG,
        json!([
            "issue",
            "create",
            "--title",
            "Re-groom mika#2562 — reprise substrat"
        ]),
    )
    .await;

    assert!(
        output.contains(REFUSAL),
        "creating an issue is the substitute the model took for the task mika#2517 \
         withholds; got: {output}"
    );

    let rows = refusal_rows(&harness).await;
    assert_eq!(rows.len(), 1, "one audit row per refusal; got {rows:?}");
    assert_eq!(
        rows[0].0.as_deref(),
        Some("issue_create"),
        "the two motifs must stay countable apart — merging them would split one \
         population and make the operator's GROUP BY read a single cause"
    );
}

// -------------------------------------------------------------------------
// V3 (AC3, contrôle négatif — le plus porteur des trois)
//
// C'est la contrepartie exécutable de M2 : la SCOPE RULE du prompt prescrit
// littéralement `run_gh issue view <n> --json labels` comme geste de
// vérification. Une garde qui refuse cette lecture casse la seule sortie
// correcte que le prompt offre — et elle serait indistinguable, pour V1 et V2,
// d'une garde qui lit le prédicat.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2573_a_fallthrough_turn_may_still_read() {
    let (harness, output) = run_gh_output(
        FALLTHROUGH_MSG,
        json!(["issue", "view", "2562", "--json", "labels"]),
    )
    .await;

    assert!(
        !output.contains(REFUSAL),
        "`run_gh` is refused, never withheld: the verification gesture the prompt \
         prescribes must pass. Got: {output}"
    );

    let rows = refusal_rows(&harness).await;
    assert!(
        rows.is_empty(),
        "a read writes no refusal row; counting it would drown the population the row \
         exists to size: {rows:?}"
    );
}

// -------------------------------------------------------------------------
// V4 (AC4, contrôle négatif — axe 1)
//
// Le tour ready-label, dans le domaine `[GitHub]` et hors du domaine
// fallthrough, garde les deux verbes. Sans lui, une garde armée sur tout tour
// `[GitHub]` satisferait V1/V2 en cassant le chemin de dispatch — celui sur
// lequel la boucle tourne.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2573_a_ready_label_turn_is_not_affected() {
    let (harness, output) = run_gh_output(
        READY_LABEL_MSG,
        json!(["issue", "edit", "2562", "--add-label", "ready"]),
    )
    .await;

    assert!(
        !output.contains(REFUSAL),
        "the ready-label dispatch path is outside the fallthrough domain and must be \
         untouched; got: {output}"
    );

    let rows = refusal_rows(&harness).await;
    assert!(
        rows.is_empty(),
        "no refusal row outside the fallthrough domain: {rows:?}"
    );
}

// -------------------------------------------------------------------------
// V5 (AC4, contrôle négatif — axe 2)
//
// Le second axe : V4 seul serait satisfait par une garde calée sur le marqueur
// ready-label plutôt que sur le domaine. Ce message porte le vocabulaire du
// défaut (« label ready ») sans aucun préfixe `[GitHub]`.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2573_an_ordinary_conversation_turn_is_not_affected() {
    let (harness, output) = run_gh_output(
        PLAIN_MSG,
        json!(["issue", "edit", "2562", "--add-label", "ready"]),
    )
    .await;

    assert!(
        !output.contains(REFUSAL),
        "an operator asking for the label by hand is outside the domain entirely — \
         this is the path that un-parks a ticket; got: {output}"
    );

    let rows = refusal_rows(&harness).await;
    assert!(rows.is_empty(), "no refusal row off the domain: {rows:?}");
}

// -------------------------------------------------------------------------
// V7 (contrôle négatif de granularité) — une étiquette qui ne déclenche rien
// passe, sur un tour Fallthrough.
//
// Sans lui, « la garde lit l'étiquette » serait indistinguable de « la garde
// refuse tout `issue edit` », et corréler un webhook à un ticket en le
// ré-étiquetant resterait bloqué.
// -------------------------------------------------------------------------

#[tokio::test]
async fn mika2573_a_non_triggering_label_still_passes_on_a_fallthrough_turn() {
    let (harness, output) = run_gh_output(
        FALLTHROUGH_MSG,
        json!(["issue", "edit", "2562", "--add-label", "bug"]),
    )
    .await;

    assert!(
        !output.contains(REFUSAL),
        "`bug` triggers no dispatch; the guard reads the label, not the verb alone. \
         Got: {output}"
    );

    let rows = refusal_rows(&harness).await;
    assert!(
        rows.is_empty(),
        "a non-triggering label writes no refusal row: {rows:?}"
    );
}
