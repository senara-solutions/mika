//! mika#2877 AC1 + AC2 — le recensement par outil du tour coupé au plafond de
//! steps, attesté de bout en bout à travers `run_agent`.
//!
//! AC1 pousse un tour jusqu'à `MaxSteps` avec des appels d'outils **connus** et
//! vérifie que le décompte émis par outil est exact. AC2 est son contrôle
//! négatif : un tour qui ne s'épuise pas n'émet pas ce décompte, lu par le
//! **même** filtre.
//!
//! La composition d'AC1 est délibérément **mixte** : `search_memory` pour la
//! majorité des steps, et un step portant deux appels via
//! `multi_tool_response`. Sans ce step double, un décompte qui compterait les
//! **steps** au lieu des **appels** passerait le test — les deux nombres
//! seraient égaux. Avec lui, `total_calls` (21) dépasse `steps_with_tools` (20)
//! et seul un décompte par appel est vert.
//!
//! # AC3 — recette de rouge-avant, exécutable par le relecteur
//!
//! 1. Commenter la **seule** ligne d'appel de `emit_max_steps_tool_census(` dans
//!    `crates/mika-agent/src/agent_loop/mod.rs`. C'est l'état du code actuel, à
//!    l'octet près du reste.
//! 2. `cargo test -p mika-agent --test eval max_steps_tool_census`
//! 3. **AC1 rougit** :
//!    `expected exactly one max_steps_tool_census event for the turn, got 0`.
//! 4. **AC2 reste vert** — il atteste une absence, et sous la recette l'absence
//!    est totale. C'est l'information qui dit qu'**AC2 seul n'atteste rien** :
//!    c'est la paire qui est le livrable, et la recette le démontre au lieu de
//!    l'affirmer.
//! 5. Décommenter ; les deux passent.
//!
//! Pourquoi commenter l'appel plutôt que `git stash` : retirer le module entier
//! ne compilerait pas, et « ne compile pas » n'est pas un rouge-avant — il ne dit
//! pas *quelle* assertion portait la charge.

use std::collections::HashMap;

use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;
// Le capteur de tracing de mika#2305, réutilisé et non réécrit : on hérite ainsi
// de son `keep_registry_multi_dispatcher()` et de son raisonnement sur le cache
// d'intérêt mono-dispatcher de `tracing-core` 0.1.36 — sans quoi ce test serait
// vert ou rouge selon l'ordre des threads du binaire de test. Le précédent
// d'import inter-fichiers est `test_dispatch_stall_p0_1381.rs`.
use super::test_context_scope_observability_2305::{Captured, capture};

/// Le nom de l'événement, écrit ici en littéral plutôt qu'importé de la
/// production : c'est un **format de fil** que l'opérateur grep, donc le test
/// doit rougir si quelqu'un le renomme, pas suivre le renommage en silence.
const EVENT: &str = "max_steps_tool_census";

/// Tous les événements `max_steps_tool_census` captés — le filtre partagé par
/// AC1 et AC2. AC2 n'atteste une absence que s'il regarde exactement là où AC1
/// trouve quelque chose.
fn census_events(events: &Captured) -> Vec<HashMap<String, String>> {
    let captured = events.lock().expect("capture mutex");
    captured
        .iter()
        .filter(|(_, f)| f.get("event").map(String::as_str) == Some(EVENT))
        .map(|(_, f)| f.clone())
        .collect()
}

/// L'unique événement `max_steps_tool_census` du tour.
///
/// Exiger qu'il y en ait **exactement un** fait partie du contrat : la commande
/// opérateur du plan (§ 7) compare le compte de cette ligne à celui du WARN
/// voisin, donc un tour qui en émettrait deux rendrait cette lecture ambiguë
/// sans rien faire rougir.
fn the_census_event(events: &Captured) -> HashMap<String, String> {
    let mut matching = census_events(events);
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one {EVENT} event for the turn, got {}",
        matching.len()
    );
    matching.pop().expect("checked non-empty")
}

/// AC1 — un tour poussé jusqu'à `MaxSteps` émet un décompte par outil exact.
#[tokio::test]
async fn mika2877_un_tour_epuise_nomme_les_outils_qui_ont_consomme_le_budget() {
    // 20 steps d'outils — le plafond `MAX_TOOL_STEPS` — dont un portant DEUX
    // appels. Puis le `text_response` du tour de continuation.
    let mut responses: Vec<MockResponse> = Vec::new();
    for i in 0..20 {
        if i == 5 {
            responses.push(multi_tool_response(vec![
                ("search_memory", json!({"query": "query_5"})),
                (
                    "store_fact",
                    json!({"category": "preference", "key": "coffee", "value": "black"}),
                ),
            ]));
        } else {
            responses.push(tool_call_response(
                "search_memory",
                json!({"query": format!("query_{i}")}),
            ));
        }
    }
    responses.push(text_response(
        "Here is a summary: I looked up many items without a definitive answer.",
    ));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness.run("Search for everything").await.unwrap();

    let event = the_census_event(&events);

    // `per_tool` par égalité COMPLÈTE, pas « contient » : un `contains`
    // laisserait passer un outil fantôme. JSON compact sous clés triées, donc
    // l'octet est déterministe.
    assert_eq!(
        event.get("per_tool").map(String::as_str),
        Some(r#"{"search_memory":20,"store_fact":1}"#),
        "la carte par outil doit être exacte et déterministe : {event:?}"
    );

    // 21 appels sur 20 steps : c'est le step double qui sépare « décompte par
    // appel » de « décompte par step ».
    assert_eq!(
        event.get("total_calls").map(String::as_str),
        Some("21"),
        "total_calls compte les APPELS : {event:?}"
    );
    assert_eq!(
        event.get("steps_with_tools").map(String::as_str),
        Some("20"),
        "steps_with_tools compte les STEPS distincts : {event:?}"
    );
    assert_eq!(
        event.get("distinct_tools").map(String::as_str),
        Some("2"),
        "deux noms d'outils distincts : {event:?}"
    );

    // Le plafond et le label voisinent le WARN existant — même terme, donc les
    // deux populations se soustraient.
    assert_eq!(
        event.get("max_steps").map(String::as_str),
        Some("20"),
        "max_steps doit porter le plafond en vigueur : {event:?}"
    );
    assert_eq!(
        event.get("label").map(String::as_str),
        Some("agent"),
        "label doit porter la même valeur que le WARN voisin : {event:?}"
    );

    // Les identités sont portées par l'événement lui-même : aucune jointure avec
    // le WARN voisin n'est requise pour lire la mesure.
    for field in ["agent_id", "session_id", "trace_id"] {
        assert!(
            event.get(field).is_some_and(|v| !v.is_empty()),
            "l'événement doit porter `{field}` non vide : {event:?}"
        );
    }
}

/// AC2 — un tour qui ne s'épuise pas n'émet pas ce décompte.
///
/// Le contrôle est **non dégénéré** : le tour appelle de vrais outils, donc le
/// décompte aurait de la matière à émettre s'il était émis hors coupure. Un
/// contrôle sans appel d'outil ne distinguerait pas « pas émis parce que pas
/// d'épuisement » de « pas émis parce que rien à compter ».
#[tokio::test]
async fn mika2877_un_tour_qui_ne_sepuise_pas_nemet_aucun_decompte() {
    let responses = vec![
        tool_call_response("search_memory", json!({"query": "coffee"})),
        tool_call_response(
            "store_fact",
            json!({"category": "preference", "key": "coffee", "value": "black"}),
        ),
        text_response("Here is a summary: I looked up one item and recorded one preference."),
    ];

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness.run("Remember how I take my coffee").await.unwrap();

    let found = census_events(&events);
    assert!(
        found.is_empty(),
        "un tour qui n'épuise pas son budget ne doit émettre aucun {EVENT}, \
         or {} ont été vus : {found:?}",
        found.len()
    );
}
