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
//!
//! **`POST /api/v1/sessions/{id}/end` — la clôture (mika#1727, vol V2d-1b).**
//! La route symétrique de la création : elle pose l'`ended_at` d'une session de
//! l'agent servi. Elle rend `ended: false` **sans erreur** quand la session est
//! la canonique de l'agent : une session canonique ne se clôt jamais
//! (mika#1401), donc le client lit ce booléen comme un fait sur la nature de la
//! session, et non comme un refus qu'il devrait réessayer ou remonter.
//!
//! Elle non plus n'a aucun appelant dans ce vol.

use axum::extract::{Path, State};
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

    // `canonical_session_id` est le seul discriminant du singleton. Il est figé
    // à l'`init_agent` par `prompt::resolve_canonical_session_id` et ne vaut
    // `Some` que pour un agent `[session] singleton = true` : la route ne relit
    // ni `identity.toml`, ni les `Settings`.
    match agent_state.canonical_session_id.as_deref() {
        Some(canonical) => {
            match agent_state
                .db
                .get_or_create_canonical_session(canonical, CLI_CHANNEL)
                .await
            {
                // L'identifiant rendu est celui que la base rend, jamais
                // `canonical.to_string()` : une session canonique ne se
                // remplace pas (mika#1401), donc c'est la base qui dit laquelle
                // est en place, pas l'appelant.
                Ok(session_id) => ok(session_id, true),
                Err(e) => write_failed(e),
            }
        }
        None => {
            let session_id = Uuid::new_v4().to_string();
            match agent_state
                .db
                .create_session(&session_id, agent_state.db.agent_id(), CLI_CHANNEL)
                .await
            {
                Ok(()) => ok(session_id, false),
                Err(e) => write_failed(e),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EndSessionRequest {
    /// Agent servi. Toujours envoyé, comme pour la création ; une chaîne vide
    /// désigne l'agent par défaut, par `resolve_agent`.
    #[serde(default)]
    pub agent: String,
}

#[derive(Debug, Serialize)]
pub struct EndSessionResponse {
    /// Faux **sans erreur** quand la session est la canonique de l'agent : elle
    /// ne se clôt jamais (mika#1401). Le client lit un fait, pas un refus.
    pub ended: bool,
    pub canonical: bool,
}

pub async fn handle_end_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<EndSessionRequest>,
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

    // Pas de verrou d'agent ici non plus : clore la fenêtre d'une TUI pendant
    // que l'agent répond à un webhook n'invalide aucun tour en cours, puisque le
    // tour en cours n'est pas celui de cette session. Le raisonnement complet est
    // sur `handle_create_session` ci-dessus — une seconde copie serait une
    // seconde chose à tenir à jour.

    // La garde canonique passe **avant** la lecture en base, et cet ordre est la
    // décision : la ligne de la session canonique ne naît qu'au premier
    // `get_or_create_canonical_session`, donc un client qui clôt avant de l'avoir
    // ouverte recevrait un 404 — une issue qui dépendrait de l'ordre de ses
    // propres appels, c'est-à-dire d'un état qu'il ne peut pas observer.
    //
    // Court-circuiter ici saute le contrôle d'appartenance ci-dessous, et c'est
    // sans risque par construction : `canonical_session_id` est un champ de
    // l'`AgentState` **de l'agent servi**, résolu à son `init_agent` depuis sa
    // propre `identity.toml`. Il ne peut donc pas désigner la session d'un autre
    // agent — ce n'est pas une donnée de la requête.
    if agent_state.canonical_session_id.as_deref() == Some(session_id.as_str()) {
        return ended(false, true);
    }

    // Le contrôle d'appartenance est nécessaire **parce que la base est unique
    // par conteneur** : `agent_state.db` est une poignée dont l'`agent_id` est
    // un identifiant porté, pas une partition des données (`with_agent` est un
    // clone qui partage l'`Arc` interne), et `get_session` fait un `SELECT …
    // WHERE id = ?` sans clause sur `agent_id`. Sans cette comparaison, la route
    // clôt la session de n'importe quel agent du conteneur sur simple
    // connaissance de son identifiant.
    match agent_state.db.get_session(&session_id).await {
        Ok(Some(s)) if s.agent_id == agent_state.db.agent_id() => {}
        Ok(Some(_)) | Ok(None) => return session_not_found(&session_id),
        Err(e) => return end_failed(e),
    }

    // `canonical_session_id` est passé bien que le cas soit traité en amont, et
    // la redondance est délibérée : les deux gardes répondent à deux questions
    // différentes. En amont se décide la **réponse** (`canonical: true`,
    // `ended: false` — un fait, pas une erreur) ; ici se tient l'**écriture**,
    // c'est-à-dire l'invariant mika#1401 lui-même (l'`ended_at` de la canonique
    // reste `NULL` pour toujours). Appeler `end_session` « puisque le cas est
    // déjà traité » déplacerait cet invariant dans l'ordre des lignes du
    // handler, que V2d-4 réordonnera.
    match agent_state
        .db
        .end_session_unless_canonical(&session_id, agent_state.canonical_session_id.as_deref())
        .await
    {
        Ok(()) => ended(true, false),
        Err(e) => end_failed(e),
    }
}

/// Un seul site pour les deux cas — session inconnue et session appartenant à un
/// autre agent — et c'est le fond de la décision de sécurité du vol : un 403, ou
/// un message qui différerait entre les deux, répondrait à une question que
/// l'appelant n'a pas le droit de poser, *cette session existe-t-elle chez
/// quelqu'un d'autre ?*. Avec deux issues distinguables, un porteur du jeton
/// interne énumère les identifiants de session de tout le conteneur. Deux
/// `format!` voisins seraient la même chose en sursis : une édition future les
/// ferait diverger sans rien casser. Le texte ne nomme donc jamais l'agent
/// propriétaire.
fn session_not_found(session_id: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": format!("session '{session_id}' not found")})),
    )
        .into_response()
}

fn ended(ended: bool, canonical: bool) -> axum::response::Response {
    let resp = EndSessionResponse { ended, canonical };
    (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
}

/// Un second site plutôt qu'un message générique partagé avec `write_failed` :
/// un 500 dont le texte ne dit pas quelle opération a échoué est un 500 qu'on
/// diagnostique en relisant le code. Couvre la lecture comme l'écriture — les
/// deux échouent sur la même base, et l'appelant n'a rien à faire de différent
/// selon laquelle.
fn end_failed(e: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": format!("session end failed: {e}")})),
    )
        .into_response()
}

fn ok(session_id: String, canonical: bool) -> axum::response::Response {
    let resp = CreateSessionResponse {
        session_id,
        canonical,
    };
    (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response()
}

/// Le corps ne porte aucun `session_id`, et c'est tout l'objet de la route :
/// rendre un identifiant dont la ligne n'existe pas, c'est livrer exactement le
/// défaut que l'en-tête de module décrit. `/send` avale le sien en `warn!`
/// parce qu'il possède le tour qui suit ; ici le tour appartient à un client
/// qu'on ne reverra qu'à la requête suivante.
fn write_failed(e: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": format!("session creation failed: {e}")})),
    )
        .into_response()
}
