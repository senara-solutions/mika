//! AC3 — le gate de merge ne lit jamais un échec de lecture comme du vert
//! (mika#2963).
//!
//! Ce fichier porte la moitié **comportementale** d'AC3, sur le chemin de
//! production, à travers `run_agent` avec `MockLlmProvider` — sans réseau, sans
//! clé LLM et sans `gh`.
//!
//! # Le cas couvert ici, et pourquoi c'est celui-là
//!
//! *« L'échange de jeton App échoue ET le repli PAT échoue aussi. »* Les deux
//! moitiés de cette conjonction ont des portées différentes dans le moteur, et
//! une seule est observable à travers la boucle :
//!
//! - **Les deux absents.** `Settings::resolve_ci_read_token` rend `None` et
//!   `ctx.github_token` rend `None`. `pr_merge_with_gate::execute` refuse alors
//!   **en tête**, avant tout sous-processus — c'est le site de mika#1964, et
//!   c'est le cas que ce fichier atteste. Aucun `gh` n'est lancé, donc rien à
//!   substituer : le chemin exercé est le chemin de production entier.
//! - **Le PAT présent mais incapable de lire.** L'étape 2 n'est atteignable
//!   qu'après l'étape 1, un vrai `gh pr view` en sous-processus — du réseau, que
//!   la suite refuse (motif `DeadEndpoint`, mika#2569). Cette moitié est donc
//!   tenue par deux assertions dans le `mod tests` de `pr_merge_with_gate.rs`,
//!   `mika2963_ac3_*`, qui portent exactement sur le terme qui compte : le bras
//!   `Err` **retourne**, donc `decide_merge_gate` n'est jamais atteint depuis un
//!   échec de lecture, et `AllPassed` est inatteignable par cette voie.
//!
//! Le mettre par écrit ici plutôt que de le taire : une assertion sur le
//! deuxième cas écrite à travers `gh` serait verte sur un poste à réseau et
//! rouge en CI, ce qui est pire que son absence.

use std::sync::Arc;

use dashmap::DashMap;
use mika_agent::tools::default_tools;
use mika_common::llm::mock::*;
use serde_json::json;
use std::collections::HashSet;

use super::harness::EvalHarness;

/// AC3 — sans jeton d'App **et** sans PAT, le gate refuse ; il ne merge pas, et
/// il ne rend pas « tout vert ».
///
/// L'observable est le **refus**, jamais un merge. Un gate qui dégraderait vers
/// une liste de checks vide trouverait `AllPassed` — c'est la forme exacte que
/// « il ne lit jamais un échec comme du vert » refuse, et la raison pour
/// laquelle ce test assertit l'absence de tout vocabulaire de succès en plus de
/// la présence du refus.
#[tokio::test]
async fn mika2963_ac3_sans_app_ni_pat_le_gate_refuse_en_tete() {
    // Le vrai `pr_merge_with_gate` est déjà dans `default_tools()` : on ne
    // substitue rien, c'est tout l'intérêt de ce cas — il refuse avant le
    // premier sous-processus.
    let registry = default_tools();

    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response(
                "pr_merge_with_gate",
                json!({"pr_number": 2963, "repo": "senara-solutions/mika"}),
            ),
            text_response("Le gate a refusé : aucun jeton pour lire la CI."),
        ])
        .tools(registry)
        .pr_reviews_posted(Arc::new(DashMap::<String, HashSet<String>>::new()))
        .build()
        .await
        .unwrap();

    // Ni PAT ni App : `resolve_ci_read_token` rend `None`, `ctx.github_token`
    // rend `None`.
    assert!(
        harness.settings.github_token.is_none(),
        "mika#2963 AC3 — le harnais doit partir SANS PAT, sinon ce test mesure \
         l'autre branche"
    );

    let trace = harness.run("Merge la PR 2963").await.unwrap();
    let calls = trace.calls_for_tool("pr_merge_with_gate");
    assert!(
        !calls.is_empty(),
        "mika#2963 AC3 — le gate n'a pas été appelé : ce test ne mesure plus rien \
         (mika#2103)"
    );
    let sortie = calls[0].output.as_deref().unwrap_or("");

    assert!(
        sortie.contains("Nothing was") || sortie.contains("not available"),
        "mika#2963 AC3 — sans aucun jeton, le gate doit refuser en tête et le dire. \
         Sortie : {sortie}"
    );
    for vert in [
        "All required checks passed",
        "Merged",
        "merged successfully",
    ] {
        assert!(
            !sortie.contains(vert),
            "mika#2963 AC3 — le gate a rendu du vocabulaire de succès (`{vert}`) alors \
             qu'aucun jeton ne lui permettait de lire la CI. C'est la phrase « il ne lit \
             jamais un échec comme du vert », prise en défaut. Sortie : {sortie}"
        );
    }
}
