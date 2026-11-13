//! `POST /api/v1/sessions` — mika-spirit ouvre une session pour un client servi
//! (mika#1727, vol V2d-1a).
//!
//! **Pourquoi c'est spirit qui écrit la ligne, et jamais le client.** La porte
//! A2A n'adopte un `caller_session_id` (`mika.caller_session_id`, mika#2070) que
//! si la session existe déjà en base et appartient à l'agent — c'est
//! `caller_session_is_adoptable`, dans `crates/mika-agent/src/a2a_db.rs`. Sinon
//! `a2a_create_task` fabrique une session `a2a-<id>` **à chaque tour**. Un
//! client qui inventerait son identifiant de son côté perdrait donc son
//! historique d'un tour à l'autre, en silence et sans erreur. Cette route est le
//! seul endroit où l'identifiant et sa ligne naissent ensemble.
//!
//! Ce vol n'a aucun appelant : la bascule de la TUI est en V2d-4.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::server::state::AppState;

/// Graphie du canal des sessions ouvertes pour un client en ligne de commande.
///
/// Un seul site, parce que la TUI écrit déjà `"cli"` de son côté
/// (`crates/mika-cli/src/commands/chat.rs`) et que V2d-4 branchera cet
/// appelant-là sur cette route : deux graphies pour un même canal rendraient
/// les deux moitiés du parc non comparables.
const CLI_CHANNEL: &str = "cli";

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    /// Agent servi. Toujours envoyé par le client, comme pour rewind ; une
    /// chaîne vide désigne l'agent par défaut, par `resolve_agent`.
    #[serde(default)]
    pub agent: String,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    /// Vrai quand l'identifiant rendu est la session canonique de l'agent
    /// (mika#1401). Le client doit le savoir parce qu'il ne la clôt **jamais** :
    /// une session canonique ne se remplace pas. V2d-1b s'appuiera dessus.
    pub canonical: bool,
}

pub async fn handle_create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
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

    // Pas de verrou d'agent ici, et c'est une divergence délibérée avec les
    // trois portes de `rewind.rs`, qui prennent `try_lock_owned` et rendent 409.
    // Rembobiner pendant un tour corromprait le tour ; ouvrir une session
    // n'entre en conflit avec rien. Prendre le verrou ferait rendre 409 à une
    // TUI qui démarre pendant que l'agent répond à un webhook — refuser
    // d'ouvrir une fenêtre parce que l'agent travaille, c'est l'inverse du
    // service rendu.

    let session_id = Uuid::new_v4().to_string();
    match agent_state
        .db
        .create_session(&session_id, agent_state.db.agent_id(), CLI_CHANNEL)
        .await
    {
        Ok(()) => {
            let resp = CreateSessionResponse {
                session_id,
                canonical: false,
            };
            (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
        }
        // Le corps ne porte aucun `session_id`, et c'est tout l'objet de la
        // route : rendre un identifiant dont la ligne n'existe pas, c'est
        // livrer exactement le défaut que l'en-tête de module décrit. `/send`
        // avale le sien en `warn!` parce qu'il possède le tour qui suit ; ici
        // le tour appartient à un client qu'on ne reverra qu'à la requête
        // suivante.
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("session creation failed: {e}")})),
        )
            .into_response(),
    }
}
