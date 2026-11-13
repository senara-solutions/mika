//! `POST /api/v1/compact` — mika-spirit compacte la conversation d'un agent
//! servi, avec le LLM de cet agent (mika#1727, vol V2d-2).
//!
//! **Pourquoi la route existe alors que la TUI sait déjà compacter.** Son
//! `/compact` (`crates/mika-cli/src/tui/commands/handlers.rs`) résume avec
//! `app.llm`, le fournisseur que la TUI a construit pour elle-même à son
//! démarrage. Rien ne garantit que ce soit celui de l'agent : un agent servi par
//! un modèle et résumé par un autre voit son historique réécrit par une voix qui
//! n'est pas la sienne, sans erreur et sans trace. Ici le résumé sort de
//! `agent_state.llm`, celui que l'agent fait tourner.
//!
//! Ce vol n'écrit aucune logique de compaction et aucun SQL : il appelle la même
//! `compaction::maybe_compact` que la compaction d'après tour. Il n'a non plus
//! aucun appelant — la bascule de la TUI est en V2d-4.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::compaction;
use crate::server::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CompactRequest {
    /// Agent servi. Toujours envoyé par le client, comme pour les routes de
    /// session ; une chaîne vide désigne l'agent par défaut, par `resolve_agent`.
    #[serde(default)]
    pub agent: String,
}

#[derive(Debug, Serialize)]
pub struct CompactResponse {
    /// Vrai quand la compaction a été **demandée et n'a pas échoué** — jamais
    /// une attestation qu'une ligne `summary` a été écrite. `maybe_compact` rend
    /// un `Result<()>`, relit le seuil elle-même et peut légitimement ne rien
    /// faire ; et la compaction d'après tour tourne hors du verrou, donc elle
    /// peut avoir condensé juste avant. Un booléen qui prétendrait au fait
    /// mentirait sur les deux chemins.
    pub compacted: bool,
    /// Compte **d'avant**, dans les deux issues. C'est celui sur lequel la
    /// décision de compacter a été prise ; un second `count_messages()` après
    /// coup rendrait un nombre vrai au moment où il est lu et faux comme
    /// explication de l'issue rendue.
    pub messages: usize,
}

pub async fn handle_compact(
    State(state): State<AppState>,
    Json(req): Json<CompactRequest>,
) -> impl IntoResponse {
    let agent_state = match state.resolve_agent(&req.agent).await {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("agent '{}' not found", req.agent)})),
            )
                .into_response();
        }
    };

    // Le verrou est tenu pendant **tout** le handler, appel au LLM compris, et
    // c'est la divergence délibérée avec `sessions.rs`, qui n'en prend aucun.
    // `maybe_compact` lit les anciens messages, les fait résumer, puis les
    // remplace par la ligne de résumé : un tour qui écrirait entre la lecture et
    // `replace_with_summary` verrait son message avalé par un résumé qui ne le
    // contient pas. Un message qui arrive pendant ce temps attend son tour, au
    // lieu de courir à côté.
    //
    // Le nom de la variable porte la garantie : `let _ =` relâcherait le verrou
    // sur place, donc avant l'appel au LLM, et aucun test ne rougirait.
    let _lock = match agent_state.agent_lock.clone().try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Agent is busy. Try again when idle."})),
            )
                .into_response();
        }
    };

    let messages = match agent_state.db.count_messages().await {
        Ok(n) => n,
        Err(e) => return count_failed(e),
    };

    // Le seuil est **lu** dans `compaction`, jamais retapé, et la comparaison est
    // le `<=` de `compaction.rs`. Les deux doivent répondre pareil : une route
    // qui annoncerait `compacted: true` au rang exact où `maybe_compact` ne fait
    // rien dirait à l'appelant qu'il a payé un appel au LLM qui n'a pas eu lieu.
    if messages <= compaction::COMPACTION_THRESHOLD {
        return ok(false, messages);
    }

    // Au-delà du seuil, l'appel à `maybe_compact` arrive au commit suivant de ce
    // vol ; d'ici là la route rend l'issue qu'elle sait tenir.
    ok(false, messages)
}

fn ok(compacted: bool, messages: usize) -> axum::response::Response {
    let resp = CompactResponse {
        compacted,
        messages,
    };
    (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
}

/// Un site par opération plutôt qu'un 500 générique : les deux échouent sur la
/// même base et l'appelant n'a rien à faire de différent selon laquelle, mais
/// celui qui lit le journal, si. Un 500 dont le texte ne dit pas quelle opération
/// a échoué est un 500 qu'on diagnostique en relisant le code.
fn count_failed(e: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": format!("message count failed: {e}")})),
    )
        .into_response()
}
