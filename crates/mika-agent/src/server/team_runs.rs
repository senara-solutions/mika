//! `POST /api/v1/teams/{name}/runs` — spirit lance une équipe et en diffuse
//! les événements (mika#1727 V4-a).
//!
//! **Un canal par requête, et non un canal diffusé.** `tasks_stream.rs` sert un
//! `broadcast` par processus parce que ses trames décrivent le cycle de vie des
//! tâches, que tout observateur a le droit de suivre. Ici la trame décrit **un**
//! run, lancé par **cet** appel : un `mpsc` borné par requête, aucun état
//! partagé, et le flux se ferme sur la trame finale.
//!
//! **Le run se détache de la requête.** Il tourne dans une tâche `tokio` qui
//! possède le `tx` ; la réponse ne possède que le `rx`. Rien ne relie le run au
//! futur de la requête, donc une déconnexion du client ne l'arrête pas — c'est
//! une propriété de la forme, pas une garde qu'on pourrait oublier d'armer.
//!
//! **Jeton interne seul**, clos par `route_layer` sur son propre groupe de
//! routeur (`server/mod.rs`) : lancer une équipe n'est pas une lecture
//! d'observabilité, et le jeton du tableau de bord n'y donne pas droit.
//!
//! Le catalogue des flux et les trois écarts assumés à sa discipline partagée
//! (pas d'`Unknown`, pas d'`OverflowMarker`, pas de troncature) sont dans
//! `crates/mika-agent/docs/architecture.md` § 14.2.

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};
use mika_common::team;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tracing::warn;

use crate::server::state::{AppState, TeamLaunchRequest};
use crate::teams::types::{TeamEvent, TeamEventCallback, TeamRun};

/// La valeur de `tasks_stream.rs`, pour que les deux surfaces ne divergent pas
/// sur un chiffre que rien ne distingue.
const CHANNEL_CAP: usize = 256;

/// Corps de la requête : `{"goal": "…", "reference_run_id": "…"}`.
#[derive(Debug, Deserialize)]
pub struct CreateTeamRunRequest {
    pub goal: String,
    /// Le run dont l'espace de travail sert de contexte en lecture seule.
    /// Facultatif : le corps minimal est `{"goal": "…"}`.
    #[serde(default)]
    pub reference_run_id: Option<String>,
}

/// L'enveloppe du flux. Exactement **une** trame finale par flux
/// (`run_completed` ou `run_error`), puis le flux se ferme.
///
/// `run_error` et non `run_failed` : la collision de vocabulaire avec
/// `TeamEvent::RunFailed` serait réelle — le moteur peut émettre l'événement
/// `run_failed` **et** le run rendre `Ok`, ou l'inverse. Deux mots pour deux
/// faits. Et `event` ici contre `kind` dans `payload` (`TeamEvent`), pour que
/// les deux étages d'un même objet JSON ne portent jamais le même mot.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TeamRunFrame {
    TeamEvent { payload: TeamEvent },
    RunCompleted { run: TeamRun },
    RunError { error: String },
}

fn nom_invalide(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": message})),
    )
        .into_response()
}

fn equipe_absente(name: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": format!("team '{name}' not found")})),
    )
        .into_response()
}

fn agent_par_defaut_indisponible() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "default agent unavailable"})),
    )
        .into_response()
}

/// POST /api/v1/teams/{name}/runs — lance un run d'équipe et en rend le flux.
pub async fn handle_create_team_run(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<CreateTeamRunRequest>,
) -> Response {
    if let Err(e) = team::validate_team_name(&name) {
        return nom_invalide(&e.to_string());
    }
    if !team::team_exists(&state.global_home_dir, &name) {
        return equipe_absente(&name);
    }
    // `tier` et `deployment` sont portés par l'agent, pas par le serveur, et le
    // lanceur est un champ de l'`AppState` : c'est donc ici que l'autorité se
    // résout (voir `state::TeamLaunchRequest`). Un `unwrap` ferait tomber le
    // serveur sur une route ouverte ; inatteignable en service nominal.
    let Some(agent) = state.resolve_agent("").await else {
        return agent_par_defaut_indisponible();
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<TeamRunFrame>(CHANNEL_CAP);
    let tx_final = tx.clone();
    // `try_send` et **jamais** `blocking_send` : `TeamEventCallback` est
    // synchrone et le moteur l'appelle depuis un contexte async, où un envoi
    // bloquant bloquerait un worker tokio. La saturation perd la trame et le
    // run continue — un `mpsc` borné n'expose aucun compte de perte, donc il
    // n'y a pas de marqueur de débordement à annoncer (doc § 14.2).
    let callback: TeamEventCallback = Box::new(move |ev| {
        let _ = tx.try_send(TeamRunFrame::TeamEvent { payload: ev });
    });

    let launcher = state.team_launcher.clone();
    let lancement = TeamLaunchRequest {
        team_name: name,
        goal: req.goal,
        reference_run_id: req.reference_run_id,
        tier: agent.tier,
        deployment: agent.deployment,
    };
    // Les deux `let _ =` — celui du rappel ci-dessus et celui-ci — **sont** le
    // contrat du critère 5, pas de la négligence : client parti ⇒ `rx` dropé ⇒
    // les envois rendent `Err`, et les ignorer est ce qui laisse le run aller à
    // son terme. Les remonter par `?` ou par un `return` coupe le run avec le
    // client ; c'est le geste « propre » qui casse le critère.
    tokio::spawn(async move {
        let frame = match launcher.launch(lancement, callback).await {
            Ok(run) => TeamRunFrame::RunCompleted { run },
            Err(e) => TeamRunFrame::RunError {
                error: format!("{e:#}"),
            },
        };
        let _ = tx_final.send(frame).await;
    });

    let stream = ReceiverStream::new(rx).filter_map(|frame| match serde_json::to_string(&frame) {
        Ok(json) => Some(Ok::<_, std::convert::Infallible>(
            Event::default().data(json),
        )),
        Err(e) => {
            warn!(error = %e, "failed to serialize team run frame");
            None
        }
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::state::TeamLauncher;
    use crate::server::tests::{test_app, test_state, test_state_with_dashboard_token};
    use crate::teams::types::TeamPhase;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tower::ServiceExt;

    const JETON_INTERNE: &str = "test-token-secret";
    const JETON_TABLEAU: &str = "dashboard-token-secret";

    /// Le `TeamRun` que le lanceur scripté rend.
    ///
    /// Construit par `serde` et non par littéral : `TeamRun` porte seize champs
    /// dont treize ont un `#[serde(default)]`, et les écrire à la main ferait de
    /// chaque champ ajouté au domaine une édition de ce banc.
    fn run_temoin(team_name: &str, goal: &str) -> TeamRun {
        serde_json::from_value(serde_json::json!({
            "run_id": "run-v4a-1",
            "team_name": team_name,
            "goal": goal,
            "status": "Completed",
        }))
        .expect("TeamRun témoin")
    }

    /// Le lanceur des tests : un scénario d'événements, un compte de lancements,
    /// une attestation d'achèvement, un mode d'erreur et une pause.
    ///
    /// L'attestation porte le critère 5 : elle est posée **après** que le
    /// scénario est allé à son terme, donc un run que le départ du client
    /// couperait ne la poserait jamais. `Notify::notify_one` garde son permis,
    /// donc l'attente du test n'a pas à être armée avant l'achèvement.
    #[derive(Clone)]
    struct LanceurScripte {
        evenements: Vec<TeamEvent>,
        lancements: Arc<AtomicUsize>,
        acheve: Arc<tokio::sync::Notify>,
        erreur: Option<String>,
        /// Pause entre deux émissions, pour que le test de coupure ait un
        /// milieu où partir.
        pause: Duration,
    }

    impl LanceurScripte {
        fn nouveau(evenements: Vec<TeamEvent>) -> Self {
            Self {
                evenements,
                lancements: Arc::new(AtomicUsize::new(0)),
                acheve: Arc::new(tokio::sync::Notify::new()),
                erreur: None,
                pause: Duration::ZERO,
            }
        }

        fn lancements(&self) -> usize {
            self.lancements.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl TeamLauncher for LanceurScripte {
        async fn launch(
            &self,
            req: TeamLaunchRequest,
            events: TeamEventCallback,
        ) -> anyhow::Result<TeamRun> {
            self.lancements.fetch_add(1, Ordering::SeqCst);
            for (i, ev) in self.evenements.iter().enumerate() {
                if i > 0 && !self.pause.is_zero() {
                    tokio::time::sleep(self.pause).await;
                }
                events(ev.clone());
            }
            let rendu = match &self.erreur {
                Some(message) => Err(anyhow::anyhow!("{message}")),
                None => Ok(run_temoin(&req.team_name, &req.goal)),
            };
            self.acheve.notify_one();
            rendu
        }
    }

    /// Un état dont `global_home_dir` porte l'équipe `dev-team`, avec le lanceur
    /// scripté posé **en mutant le champ** : le littéral de test de
    /// `server/mod.rs` garde la valeur de production, qui ne lance rien faute
    /// d'équipe sur le disque. Le `TempDir` est rendu pour rester vivant.
    fn etat_avec_equipe(base: AppState, lanceur: &LanceurScripte) -> (AppState, tempfile::TempDir) {
        let home = tempfile::tempdir().expect("dossier temporaire");
        let dir = home.path().join("teams").join("dev-team");
        std::fs::create_dir_all(&dir).expect("dossier d'équipe");
        std::fs::write(dir.join("team.toml"), "").expect("team.toml");
        let mut state = base;
        state.global_home_dir = home.path().to_path_buf();
        state.team_launcher = Arc::new(lanceur.clone());
        (state, home)
    }

    fn requete(name: &str, jeton: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/teams/{name}/runs"))
            .header("content-type", "application/json");
        if let Some(j) = jeton {
            builder = builder.header("authorization", format!("Bearer {j}"));
        }
        builder
            .body(Body::from(
                serde_json::json!({"goal": "livrer le rapport"}).to_string(),
            ))
            .expect("requête")
    }

    /// Les trames d'un corps SSE lu en entier, dans l'ordre du fil. Que
    /// `collect` rende est en soi la preuve que le flux s'est fermé.
    async fn trames(resp: Response) -> Vec<serde_json::Value> {
        let bytes = resp.into_body().collect().await.expect("corps").to_bytes();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .map(|d| serde_json::from_str(d).expect("trame JSON"))
            .collect()
    }

    #[tokio::test]
    async fn mika1727_v4a_sans_jeton_la_route_rend_401_et_ne_lance_rien() {
        let lanceur = LanceurScripte::nouveau(vec![]);
        let (state, _home) = etat_avec_equipe(test_state(), &lanceur);
        let resp = test_app(state)
            .oneshot(requete("dev-team", None))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(lanceur.lancements(), 0, "aucun run ne doit être lancé");
    }

    /// Deux tests pour le critère 2, et non un : « sans jeton » et « avec le
    /// jeton du tableau de bord » n'empruntent pas le même chemin dans
    /// `auth.rs` — absence d'en-tête contre comparaison en temps constant d'un
    /// jeton présent. Les fondre perdrait la seconde moitié au premier `assert`
    /// rouge.
    #[tokio::test]
    async fn mika1727_v4a_le_jeton_du_tableau_de_bord_rend_401_et_ne_lance_rien() {
        let lanceur = LanceurScripte::nouveau(vec![]);
        let (state, _home) = etat_avec_equipe(test_state_with_dashboard_token(), &lanceur);
        let resp = test_app(state)
            .oneshot(requete("dev-team", Some(JETON_TABLEAU)))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(lanceur.lancements(), 0, "aucun run ne doit être lancé");
    }

    #[tokio::test]
    async fn mika1727_v4a_un_nom_invalide_rend_400_sans_lancer() {
        let lanceur = LanceurScripte::nouveau(vec![]);
        let (state, _home) = etat_avec_equipe(test_state(), &lanceur);
        // Le tiret bas est hors du jeu de caractères de `validate_team_name`.
        let resp = test_app(state)
            .oneshot(requete("dev_team", Some(JETON_INTERNE)))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(lanceur.lancements(), 0, "aucun run ne doit être lancé");
    }

    #[tokio::test]
    async fn mika1727_v4a_une_equipe_absente_rend_404_sans_lancer() {
        let lanceur = LanceurScripte::nouveau(vec![]);
        let (state, _home) = etat_avec_equipe(test_state(), &lanceur);
        let resp = test_app(state)
            .oneshot(requete("equipe-absente", Some(JETON_INTERNE)))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(lanceur.lancements(), 0, "aucun run ne doit être lancé");
    }

    #[tokio::test]
    async fn mika1727_v4a_les_trames_arrivent_dans_lordre_puis_la_finale() {
        let lanceur = LanceurScripte::nouveau(vec![
            TeamEvent::Progress("décomposition du but".to_string()),
            TeamEvent::PhaseChanged {
                phase: TeamPhase::Execute,
                iteration: 1,
            },
            TeamEvent::Deliverable("# Rapport".to_string()),
        ]);
        let (state, _home) = etat_avec_equipe(test_state(), &lanceur);
        let resp = test_app(state)
            .oneshot(requete("dev-team", Some(JETON_INTERNE)))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::OK);

        let trames = trames(resp).await;
        assert_eq!(
            trames.len(),
            4,
            "trois événements puis la finale : {trames:?}"
        );
        assert_eq!(trames[0]["event"], "team_event");
        assert_eq!(trames[0]["payload"]["kind"], "progress");
        assert_eq!(trames[1]["payload"]["kind"], "phase_changed");
        assert_eq!(trames[2]["payload"]["kind"], "deliverable");
        assert_eq!(trames[3]["event"], "run_completed");
        assert_eq!(trames[3]["run"]["run_id"], "run-v4a-1");
        assert_eq!(trames[3]["run"]["goal"], "livrer le rapport");
        assert_eq!(lanceur.lancements(), 1);
    }

    #[tokio::test]
    async fn mika1727_v4a_un_run_en_erreur_donne_une_trame_derreur_et_ferme() {
        let mut lanceur =
            LanceurScripte::nouveau(vec![TeamEvent::Progress("décomposition".to_string())]);
        lanceur.erreur = Some("l'orchestrateur n'a délégué à personne".to_string());
        let (state, _home) = etat_avec_equipe(test_state(), &lanceur);
        let resp = test_app(state)
            .oneshot(requete("dev-team", Some(JETON_INTERNE)))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::OK);

        let trames = trames(resp).await;
        assert_eq!(trames.len(), 2, "un événement puis la finale : {trames:?}");
        assert_eq!(trames[0]["event"], "team_event");
        assert_eq!(trames[1]["event"], "run_error");
        assert!(
            trames[1]["error"]
                .as_str()
                .expect("message d'erreur")
                .contains("délégué"),
            "la trame finale porte l'erreur du run : {trames:?}"
        );
    }

    #[tokio::test]
    async fn mika1727_v4a_la_coupure_du_client_ne_coupe_pas_le_run() {
        let mut lanceur = LanceurScripte::nouveau(vec![
            TeamEvent::Progress("première".to_string()),
            TeamEvent::Progress("deuxième".to_string()),
            TeamEvent::Progress("troisième".to_string()),
        ]);
        lanceur.pause = Duration::from_millis(20);
        let acheve = lanceur.acheve.clone();
        let (state, _home) = etat_avec_equipe(test_state(), &lanceur);
        let mut resp = test_app(state)
            .oneshot(requete("dev-team", Some(JETON_INTERNE)))
            .await
            .expect("réponse");
        assert_eq!(resp.status(), StatusCode::OK);

        // Lecture **incrémentale** d'une seule trame : `collect` lirait le corps
        // entier, et il n'y aurait plus de milieu où partir.
        let premiere = resp
            .body_mut()
            .frame()
            .await
            .expect("une première trame")
            .expect("trame lisible");
        assert!(premiere.into_data().is_ok(), "la trame porte des données");
        drop(resp);

        tokio::time::timeout(Duration::from_secs(5), acheve.notified())
            .await
            .expect("le run doit aller à son terme malgré le départ du client");
        assert_eq!(lanceur.lancements(), 1);
    }
}
