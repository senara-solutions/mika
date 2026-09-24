//! mika#1910 — le tour de continuation dit enfin ce qu'il a produit.
//!
//! # Ce que ces tests couvrent et que leurs voisins ne couvrent pas
//!
//! `test_max_steps_continuation.rs` prouve déjà que le tour de continuation a
//! lieu et que ses outils sont désactivés ; les unités de
//! `agent_loop::tests` prouvent que `build_turn_usage_fields` rend le champ
//! qu'on lui passe. **Ni l'un ni l'autre ne rougirait** si le site d'émission
//! passait `RESPONSE_CHARS_UNMEASURED` au lieu du compte réel : le tour
//! fonctionnerait, la sortie serait la même, et la seule différence serait que
//! la ligne redevient muette sur exactement la question que mika#1910 pose.
//!
//! C'est la forme mika#2205 d'un cran plus loin : là un prédicat juste n'était
//! jamais appelé ; ici un constructeur juste serait appelé avec le mauvais
//! argument. Seul un test qui lit **l'événement émis** après un vrai tour le
//! ferme.
//!
//! # Pourquoi les trois cas, séparément
//!
//! Les trois valeurs que `response_chars` peut prendre sur la ligne de
//! continuation sont exactement les trois populations que le plan de mika#1910
//! refuse de confondre (R5) :
//!
//! | tour | `response_chars` | ce que ça veut dire |
//! |---|---|---|
//! | T1 — la continuation résume | `Some(n)`, `n > 0` | le tour a produit |
//! | T2 — la continuation rend du vide | **`Some(0)`** | **la classe mika#1910** |
//! | T3 — la continuation échoue | `None` | rien à mesurer |
//!
//! T2 est le test porteur : un instrument qui ne saurait pas dire `0` passerait
//! T1 et T3 et ne vaudrait rien. T3 est son contrôle négatif : un instrument
//! qui dirait `0` partout passerait T2 et ferait compter chaque panne réseau
//! sous mika#1910.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// Le sentinelle `u32::MAX`, partagé par le journal et la base pour distinguer
/// la ligne de continuation des index de pas de la boucle. Écrit ici tel que
/// l'opérateur le lit dans `jq`, parce que c'est cette forme-là que le README
/// et `scripts/measure-empty-turns` emploient.
const CONTINUATION_STEP: &str = "4294967295";

// -- capture tracing (même forme que `test_context_scope_observability_2305.rs`) --

type Captured = Arc<Mutex<Vec<HashMap<String, String>>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        if let Ok(mut events) = self.events.lock() {
            events.push(fields);
        }
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// Voir le commentaire homonyme de `test_context_scope_observability_2305.rs` :
/// sans un second dispatcher vivant, `tracing-core` peut mettre en cache un
/// `Interest::never` pour le callsite depuis un autre thread de test et cette
/// capture ne verrait rien.
fn keep_registry_multi_dispatcher() {
    static KEEPER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEPER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
}

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    keep_registry_multi_dispatcher();
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

/// L'unique ligne `turn_usage` du tour de continuation.
///
/// Exiger qu'il y en ait **exactement une** fait partie du contrat : le
/// prédicat de `scripts/measure-empty-turns` est bâti sur l'unicité de cette
/// ligne par `trace_id` (règle 0), donc un tour qui en émettrait deux rendrait
/// la mesure ambiguë sans faire échouer quoi que ce soit.
fn the_continuation_turn_usage(events: &Captured) -> HashMap<String, String> {
    let captured = events.lock().expect("capture mutex");
    let mut matching: Vec<_> = captured
        .iter()
        .filter(|f| {
            f.get("event").map(String::as_str) == Some("turn_usage")
                && f.get("step").map(String::as_str) == Some(CONTINUATION_STEP)
        })
        .cloned()
        .collect();

    assert_eq!(
        matching.len(),
        1,
        "un tour produit zéro ou une ligne de continuation, jamais deux — \
         la règle 0 du prédicat de mesure en dépend ; obtenu {} : {:?}",
        matching.len(),
        captured
            .iter()
            .filter(|f| f.get("event").map(String::as_str) == Some("turn_usage"))
            .map(|f| (f.get("step").cloned(), f.get("response_chars").cloned()))
            .collect::<Vec<_>>()
    );
    matching.pop().expect("checked non-empty")
}

/// Vingt appels d'outil, de quoi épuiser `MAX_TOOL_STEPS` et atteindre
/// `LoopResult::MaxStepsExceeded` — c'est-à-dire le seul chemin du moteur où la
/// classe mika#1910 se manifeste.
fn twenty_tool_steps() -> Vec<MockResponse> {
    (0..20)
        .map(|i| tool_call_response("search_memory", json!({"query": format!("query_{i}")})))
        .collect()
}

/// **T1 — le tour de continuation a produit, et la ligne le dit.**
#[tokio::test]
async fn mika1910_a_productive_continuation_turn_reports_what_it_produced() {
    const SUMMARY: &str = "J'ai cherché sans trouver de réponse définitive.";

    let mut responses = twenty_tool_steps();
    responses.push(text_response(SUMMARY));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness.run("Cherche partout").await.unwrap();

    let fields = the_continuation_turn_usage(&events);
    assert_eq!(
        fields.get("response_chars").map(String::as_str),
        Some(format!("Some({})", SUMMARY.chars().count()).as_str()),
        "le compte doit être celui du texte réellement produit, par le \
         sérialiseur canonique — pas une constante : {fields:?}"
    );
    assert_eq!(
        fields.get("status").map(String::as_str),
        Some("success"),
        "contrôle de bonne foi : ce tour a bien réussi : {fields:?}"
    );
}

/// **T2 — le tour de continuation n'a rien produit, et c'est `0`, pas `null`.**
///
/// Le test porteur. C'est la forme exacte que décrit le corps de mika#1910 —
/// les tours brûlés puis un message final vide — reproduite dans le moteur, et
/// c'est la seule ligne dont la lecture décide du verdict du ticket. Avant U1,
/// cette ligne ne portait aucun champ et `llm_calls.response_text` y était NULL
/// **à 100 %, succès compris** : ni la base ni le journal ne pouvaient
/// distinguer ce tour-ci du tour T1 ci-dessus.
#[tokio::test]
async fn mika1910_an_empty_continuation_turn_is_measured_as_zero_never_as_unmeasured() {
    let mut responses = twenty_tool_steps();
    // Le modèle rend un contenu texte vide : `serialize_response_text` rend
    // `None` (résultat vide après `strip_internal_tags`), et le site doit
    // traduire ça en `Some(0)` — l'appel a rendu la main, donc il a été mesuré.
    responses.push(text_response(""));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness.run("Cherche partout").await.unwrap();

    let fields = the_continuation_turn_usage(&events);
    assert_eq!(
        fields.get("response_chars").map(String::as_str),
        Some("Some(0)"),
        "un tour de continuation qui a RENDU LA MAIN sans produire un caractère \
         est la classe mika#1910 elle-même ; le replier sur `null` la rangerait \
         dans la population « non mesuré » (R5) et rendrait le compte faux : \
         {fields:?}"
    );
    assert_ne!(
        fields.get("response_chars").map(String::as_str),
        Some("None"),
        "`null` dit « non mesuré » et ne peut pas décrire un appel qui a abouti"
    );
}

/// **T3 — contrôle négatif : l'appel a échoué, donc il n'y a rien à mesurer.**
///
/// Sans lui, un site qui écrirait `Some(0)` partout passerait T2 et ferait
/// compter chaque panne de transport sous mika#1910 — la population (b) de R5.
#[tokio::test]
async fn mika1910_a_failed_continuation_turn_reports_nothing_measured() {
    let mut responses = twenty_tool_steps();
    responses.push(MockResponse::Error(mika_common::llm::LlmError::HttpError {
        status: 500,
        message: "Internal server error during continuation".into(),
        retryable: false,
    }));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness.run("Cherche partout").await.unwrap();

    let fields = the_continuation_turn_usage(&events);
    assert_eq!(
        fields.get("response_chars").map(String::as_str),
        Some("None"),
        "aucune réponse n'existe sur ce bras : `0` y serait un mensonge lisible \
         (mika#2331, la règle `null` ≠ `0`) : {fields:?}"
    );
    assert_eq!(
        fields.get("status").map(String::as_str),
        Some("error"),
        "contrôle de bonne foi : ce tour a bien échoué : {fields:?}"
    );
}

/// **T4 (U2) — la ligne `llm_calls` de la continuation porte enfin son texte.**
///
/// Moitié secondaire et *gated* (D3) : elle ne sert pas à compter, elle sert à
/// **lire une occurrence** quand le compte en signale une — l'investigation 2
/// du ticket (« turn-by-turn analysis »). Jusqu'à mika#1910 ce site passait
/// `None, None` aux positions `response_text` / `reasoning` sur **toutes** ses
/// branches, succès compris.
#[tokio::test]
async fn mika1910_the_continuation_row_carries_its_response_text() {
    const SUMMARY: &str = "Résumé du tour de continuation.";

    let mut responses = twenty_tool_steps();
    responses.push(text_response(SUMMARY));

    let harness = EvalHarness::builder()
        .responses(responses)
        .build()
        .await
        .unwrap();

    let trace = harness.run("Cherche partout").await.unwrap();

    let continuation: Vec<_> = trace
        .llm_calls
        .iter()
        .filter(|r| r.step == u32::MAX)
        .collect();
    assert_eq!(
        continuation.len(),
        1,
        "une ligne de continuation par tour : {:?}",
        trace.llm_calls.iter().map(|r| r.step).collect::<Vec<_>>()
    );

    let row = continuation[0];
    assert!(
        row.has_response_text,
        "la lacune : `response_text` était NULL sur CETTE ligne à 100 %, \
         succès compris, ce qui rendait la classe mika#1910 non mesurable en base"
    );

    let detail = harness
        .db
        .get_llm_call_by_id(&row.id)
        .await
        .unwrap()
        .expect("la ligne vient d'être lue");
    assert_eq!(
        detail.response_text.as_deref(),
        Some(SUMMARY),
        "et le texte stocké est celui que le tour a produit, par le même \
         sérialiseur que le compte du journal — deux mesures libres de diverger \
         seraient un second lecteur"
    );
}
