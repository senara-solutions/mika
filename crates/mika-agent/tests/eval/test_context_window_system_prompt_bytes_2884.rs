//! mika#2884 AC1 + AC2 — la ligne d'attribution d'un tour porte la taille du
//! prompt système, et c'est la **même** valeur que celle de son voisin.
//!
//! # Ce que ces deux tests couvrent et qu'un unitaire ne couvrirait pas
//!
//! Un unitaire sur `emit_context_window_assembled` prouverait que l'émetteur
//! rend l'argument qu'on lui passe. Il resterait vert si le site d'appel passait
//! `0`, ou la longueur d'un **autre** `String` — c'est la forme mika#2205, un
//! prédicat correct jamais appelé avec la bonne entrée, et la même raison pour
//! laquelle mika#2305 a écrit un test de harnais plutôt qu'un unitaire sur
//! `build_context_window_fields`. Seul un tour réel, lu sur l'événement émis
//! **et** sur `trace.captured_requests[0].system`, relie les deux bouts.
//!
//! # L'asymétrie de sigil, qui décide de la forme de l'assertion AC2
//!
//! Le capteur transcrit les champs en `String` via son `FieldVisitor`, et les
//! deux émissions ne passent pas par la même méthode de visite :
//!
//! - `turn_usage` émet `system_prompt_bytes = ?fields.system_prompt_bytes` sur
//!   un `Option<i64>` — donc `record_debug`, donc le capteur voit la chaîne
//!   `"Some(1234)"`.
//! - `context_window_assembled` émet une valeur **nue** sur un `usize` — donc
//!   `record_u64`, donc le capteur voit `"1234"`.
//!
//! Les deux formes sont justes à leur place : au site d'émission de la ligne
//! d'attribution le prompt système est un `String` en main, donc « une mesure
//! qui n'a pas eu lieu » y est inatteignable et le motif mika#2331 (*absent
//! n'est jamais zéro*) tient par construction ; `turn_usage`, lui, est émis
//! depuis un chemin où le prompt peut être absent et garde son `Option`. AC2
//! compare donc à travers un `format!("Some({})")` — ne pas « corriger » le
//! sigil de `turn_usage` pour simplifier cette ligne.
//!
//! # AC3 — recette de rouge-avant, rejouable après le merge
//!
//! 1. Retirer la ligne `system_prompt_bytes = system_prompt_bytes,` du `info!`
//!    de `emit_context_window_assembled` (`crates/mika-agent/src/agent_loop/mod.rs`).
//!    C'est l'état du code d'avant ce ticket, à l'octet près du reste.
//! 2. `cargo test -p mika-agent --test eval mika2884_la_fenetre_assemblee`
//! 3. **AC1 rougit** sur `None` au lieu de `Some("<n>")` — une valeur absente,
//!    et non une panique de compilation. « Ne compile pas » n'est pas un
//!    rouge-avant : ça ne dit pas *quelle* assertion portait la charge.
//! 4. Remettre la ligne ; les deux passent.
//!
//! AC2 rougit au même instant et pour la même raison ; AC1 est le critère que
//! le ticket nomme, c'est donc lui que la recette cite.

use std::collections::HashMap;

use mika_common::llm::mock::*;

use super::harness::EvalHarness;
// Le capteur de tracing de mika#2305, réutilisé et non réécrit — ce fichier en
// est le **troisième** consommateur. On hérite ainsi de son
// `keep_registry_multi_dispatcher()` et de son raisonnement sur le cache
// d'intérêt mono-dispatcher de `tracing-core` 0.1.36, sans quoi 8 runs sur 25
// captureraient zéro événement sous `--features telemetry`.
use super::test_context_scope_observability_2305::{Captured, capture};

/// Les deux noms d'événement, écrits en littéraux plutôt qu'importés de la
/// production : ce sont des **formats de fil** que l'opérateur grep, donc le
/// test doit rougir si quelqu'un les renomme, pas suivre le renommage en
/// silence.
const WINDOW_EVENT: &str = "context_window_assembled";
const TURN_USAGE_EVENT: &str = "turn_usage";

/// Tous les événements captés portant ce nom.
fn events_named(events: &Captured, name: &str) -> Vec<HashMap<String, String>> {
    let captured = events.lock().expect("capture mutex");
    captured
        .iter()
        .filter(|(_, f)| f.get("event").map(String::as_str) == Some(name))
        .map(|(_, f)| f.clone())
        .collect()
}

/// L'unique événement `context_window_assembled` du tour.
///
/// Exiger qu'il y en ait **exactement un** fait partie du contrat, pour la même
/// raison que le `the_window_event` de mika#2305 : la sonde opérateur lit les
/// lignes qu'elle obtient, donc un tour qui émettrait l'événement deux fois
/// rendrait cette lecture ambiguë sans rien faire rougir.
fn the_window_event(events: &Captured) -> HashMap<String, String> {
    let mut matching = events_named(events, WINDOW_EVENT);
    assert_eq!(
        matching.len(),
        1,
        "attendu exactement un {WINDOW_EVENT} pour le tour, obtenu {}",
        matching.len()
    );
    matching.pop().expect("non vide, vérifié juste au-dessus")
}

/// L'événement `turn_usage` du **premier** appel au modèle.
///
/// `system_prompt_len` est calculé une fois, avant la boucle de steps, donc tous
/// les steps portent la même valeur ; c'est le step 0 qui correspond au prompt
/// tel qu'envoyé au premier appel, soit exactement ce que
/// `context_window_assembled` mesure.
///
/// Cette fonction **panique bruyamment** quand elle ne trouve rien, en nommant
/// la cause probable. Elle ne se replie jamais sur « pas de `turn_usage`, donc
/// rien à comparer » : un test qui skippe ce qu'il ne trouve pas est vert et
/// muet sur le fait qu'il n'a rien vérifié.
fn the_first_turn_usage_event(events: &Captured) -> HashMap<String, String> {
    let all = events_named(events, TURN_USAGE_EVENT);
    assert!(
        !all.is_empty(),
        "aucun {TURN_USAGE_EVENT} capté : le tour n'a pas atteint le modèle, \
         ou l'émission a cessé d'être inconditionnelle — dans les deux cas AC2 \
         n'a rien à comparer et doit le dire, pas passer"
    );
    all.into_iter()
        .find(|f| f.get("step").map(String::as_str) == Some("0"))
        .expect("un tour qui atteint le modèle émet un turn_usage au step 0")
}

/// La référence : la longueur en octets du prompt système **réellement envoyé**
/// au provider sur le premier appel.
fn system_prompt_bytes_sent(trace: &super::trace::AgentTrace) -> usize {
    assert!(
        !trace.captured_requests.is_empty(),
        "le tour doit avoir atteint le provider"
    );
    trace.captured_requests[0]
        .system
        .as_ref()
        .expect("le tour doit avoir atteint le provider avec un prompt système")
        .len()
}

/// **AC1** — l'événement porte `system_prompt_bytes`, et il vaut la longueur en
/// octets du prompt système envoyé.
///
/// L'assertion est une **égalité**, pas un `contains` ni un `> 0` : un prompt
/// système mesuré à zéro passerait un test de présence, et c'est exactement le
/// mode de panne que le motif mika#2331 nomme.
#[tokio::test]
async fn mika2884_la_fenetre_assemblee_porte_la_taille_du_prompt_systeme() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noté.")])
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    let trace = harness.run("Résume l'état du ticket.").await.unwrap();

    let reference = system_prompt_bytes_sent(&trace);
    assert!(
        reference > 0,
        "contrôle de sanité de la référence : un tour conversationnel assemble \
         toujours un prompt système non vide"
    );

    let fields = the_window_event(&events);
    assert_eq!(
        fields.get("system_prompt_bytes").map(String::as_str),
        Some(reference.to_string().as_str()),
        "la ligne d'attribution doit porter la taille du prompt système \
         réellement envoyé ({reference} octets), pas une autre longueur ni un \
         zéro : {fields:?}"
    );
}

/// **AC2** — sur le même tour, la valeur est celle que `turn_usage` rapporte.
///
/// Les deux nombres dérivent de `system.len()` sur le même `String`, par deux
/// chemins distincts : le site d'appel de l'émetteur lit `system.len()` avant la
/// construction de la requête, `turn_usage` lit
/// `request.system.as_ref().map_or(0, |s| s.len())` avant la boucle de steps —
/// donc avant le *nudge* de fin de budget qui fait `system.push_str(nudge)`.
/// C'est cette seconde propriété qui rend l'assertion non triviale : sur un tour
/// multi-steps avec nudge, une implémentation naïve de `turn_usage` aurait
/// divergé. Elle ne diverge pas, et ce test le fige.
#[tokio::test]
async fn mika2884_la_valeur_est_celle_que_turn_usage_rapporte_pour_le_meme_tour() {
    let harness = EvalHarness::builder()
        .responses(vec![text_response("Noté.")])
        .build()
        .await
        .unwrap();

    let (_guard, events) = capture();
    harness.run("Résume l'état du ticket.").await.unwrap();

    let window = the_window_event(&events);
    let window_bytes = window
        .get("system_prompt_bytes")
        .expect("AC1 couvre l'absence du champ ; ici il doit être là")
        .clone();

    let usage = the_first_turn_usage_event(&events);
    assert_eq!(
        usage.get("system_prompt_bytes").map(String::as_str),
        // Le `format!("Some(…)")` est load-bearing : voir l'asymétrie de sigil
        // de l'en-tête de ce fichier.
        Some(format!("Some({window_bytes})").as_str()),
        "les deux surfaces du même tour doivent s'accorder ; si elles divergent, \
         le point de rupture à regarder est le nudge de fin de budget :\n\
         context_window_assembled = {window:?}\nturn_usage = {usage:?}"
    );
}
