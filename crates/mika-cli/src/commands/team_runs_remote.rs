//! Le transport de la route des runs d'équipe de spirit
//! (`POST /api/v1/teams/{name}/runs`, mika#1727 vol V4-b1) : un vocabulaire de
//! refus à cinq variants et la porte [`lancer_run_equipe`].
//!
//! **Rien n'est mis en miroir.** `TeamRunFrame`, `TeamEvent` et `TeamRun` sont
//! `pub` et `Deserialize` chez le producteur, et `mika-cli` dépend déjà de
//! `mika-agent` : le décodage voyage donc en lockstep avec la route. Un miroir
//! local serait une seconde copie libre de diverger sur le nom d'une variante,
//! ce qui est précisément ce qu'on ne veut pas ici — contrairement à
//! [`super::tasks::TaskDetailMirror`], où le type du serveur ne dérive que
//! `Serialize` et où il n'y avait pas d'autre forme disponible.
//!
//! **Le découpeur SSE est celui du voisin.** `tasks::drain_events` est
//! `pub(crate)` depuis ce vol : deux flux SSE dans le crate, un découpeur, dont
//! la propriété de re-raboutage est attestée là où elle vit.
//!
//! L'URL de base et le jeton arrivent **par argument**, pour la raison déjà
//! écrite sur [`super::tasks::TaskEventStream::open`] : un consommateur qui
//! résout ses propres accès ne se pilote pas depuis un test.
//!
//! **Une perte d'événement n'est pas détectable ici.** La route pousse ses
//! trames dans un `mpsc` de 256 en `try_send` et un client lent en perd sans
//! marqueur (`crates/mika-agent/docs/architecture.md` § 14.2). Ce client ne
//! l'essaie pas : c'est dit ici pour qu'un trou dans la suite d'événements ne se
//! lise pas plus tard comme un défaut du client.

use mika_agent::server::team_runs::TeamRunFrame;
use mika_agent::teams::types::{TeamEvent, TeamRun};

/// Ce qui peut empêcher ce client de rendre le `TeamRun` d'un run d'équipe.
///
/// Un enum et non un `anyhow::Error` nu : « flux fermé sans trame finale » doit
/// se distinguer de tout autre échec, et un `contains` sur un message mesurerait
/// la phrase, pas le fait.
///
/// `Denied` ne porte que l'URL, sur le motif de
/// [`super::skills_remote::SkillsRemoteIssue::JetonRefuse`] : un 401 d'ici a un
/// corps vide ou générique, et le 401 est déjà dans le nom. L'URL y est parce
/// que `base_url` est un argument — « quel serveur a refusé » est la moitié de
/// la question quand deux daemons tournent.
///
/// `RunError` ne porte **ni URL ni statut** : le run a bien tourné et le serveur
/// a bien répondu. C'est un échec du run, pas du transport, et les mêler
/// enverrait l'opérateur regarder le réseau.
#[derive(Debug)]
pub(crate) enum TeamRunRemoteIssue {
    Denied {
        url: String,
    },
    Refus {
        status: u16,
        url: String,
        body: String,
    },
    Transport {
        url: String,
        cause: String,
    },
    SansTrameFinale {
        url: String,
    },
    RunError {
        error: String,
    },
}

// `Display` à la main : rien à chaîner, aucune frontière de bibliothèque.
impl std::fmt::Display for TeamRunRemoteIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // La seconde moitié est la phrase du `Denied` de
            // `session_remote::SessionRemoteIssue` : le geste de remédiation est
            // le même, et deux formulations d'un même geste dérivent.
            Self::Denied { url } => write!(
                f,
                "mika-spirit denied the request for {url}: set MIKA_INTERNAL_TOKEN to the server's token"
            ),
            Self::Refus { status, url, body } => {
                write!(f, "mika-spirit returned {status} for {url}: {body}")
            }
            Self::Transport { url, cause } => {
                write!(f, "mika-spirit is unreachable at {url}: {cause}")
            }
            // Le message dit ce que l'opérateur n'a **pas** : le run tourne
            // toujours côté spirit — il se détache de la requête — mais aucune
            // trame n'a nommé son identifiant, donc il n'y a pas de run à
            // relire. Le croire disponible serait la mauvaise lecture.
            Self::SansTrameFinale { url } => write!(
                f,
                "the team-run stream at {url} closed before any final frame: the run may still \
                 be going on mika-spirit, but no frame named its id, so there is no run to re-read"
            ),
            Self::RunError { error } => write!(f, "the team run failed: {error}"),
        }
    }
}

/// Le corps de la requête, écrit ici plutôt que repris de
/// `server::team_runs::CreateTeamRunRequest` : ce dernier ne dérive que
/// `Deserialize`, donc il ne peut pas servir d'écrivain, et ce vol ne touche pas
/// `crates/mika-agent/`. Un `derive(Serialize)` et non un `json!` : l'ordre des
/// champs est alors celui qui est déclaré, et le test 1 compare le corps octet
/// pour octet.
#[derive(serde::Serialize)]
struct CorpsDeLancement<'a> {
    goal: &'a str,
    /// Omis quand absent, parce que le corps minimal que la route accepte est
    /// `{"goal": "…"}` et qu'un `null` explicite y ajouterait un champ que rien
    /// n'exige.
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_run_id: Option<&'a str>,
}

/// Lancer un run d'équipe sur mika-spirit et en lire le flux jusqu'à sa trame
/// finale.
///
/// **Aucun délai global**, délibérément : un run d'équipe dure des minutes, et
/// un `.timeout(…)` posé ici « par hygiène » couperait le flux au milieu d'un
/// run qui va bien. C'est la régression à lire dans un tel ajout.
///
/// Le statut se lit **avant** le corps, comme les transports voisins : un corps
/// d'erreur désérialisé comme un succès est la panne à ne pas produire.
///
/// `on_event` reçoit chaque `TeamEvent` **dans l'ordre reçu**. Une trame que ce
/// client ne sait pas décoder se dit sur stderr et la lecture continue — un
/// `kind` de `TeamEvent` inconnu tombe ici par construction, l'enum refusant
/// `#[serde(other)]` (`teams/types.rs`), donc une variante neuve chez le
/// producteur coûte un événement perdu et jamais le run.
///
/// Un flux fermé sans trame finale rend [`TeamRunRemoteIssue::SansTrameFinale`].
/// C'est le seul chemin où la boucle sort sans trame terminale, donc le
/// distinguer est structurel et non une garde qu'on pourrait oublier d'armer.
pub(crate) async fn lancer_run_equipe(
    base_url: &str,
    token: &str,
    team_name: &str,
    goal: &str,
    reference_run_id: Option<&str>,
    mut on_event: impl FnMut(TeamEvent),
) -> Result<TeamRun, TeamRunRemoteIssue> {
    // Le nom d'équipe va dans le chemin par `format!`. Un nom portant un
    // caractère réservé produirait un autre chemin, donc un 404 là où la route
    // aurait rendu son 400 : écart assumé, parce que `team::validate_team_name`
    // est le juge de ce nom et qu'il tourne côté serveur.
    let url = format!(
        "{}/api/v1/teams/{team_name}/runs",
        base_url.trim_end_matches('/')
    );
    // Client construit ici, au site de l'appel : c'est ce que déclare l'entrée
    // d'egress `cli-spirit-team-runs`, qu'un client en argument rendrait
    // aveugle. Le raisonnement entier est sur `rewind_remote::post`.
    let mut reponse = reqwest::Client::new()
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "text/event-stream")
        .json(&CorpsDeLancement {
            goal,
            reference_run_id,
        })
        .send()
        .await
        .map_err(|cause| TeamRunRemoteIssue::Transport {
            url: url.clone(),
            cause: cause.to_string(),
        })?;

    let statut = reponse.status().as_u16();
    if statut != 200 {
        let corps = reponse.text().await.unwrap_or_default();
        return Err(match statut {
            401 => TeamRunRemoteIssue::Denied { url },
            // 400 et 404 n'ont pas de variant à eux : ce client n'en sait rien
            // de plus que ce que le serveur en a dit, et son corps JSON porte
            // déjà le `error` que la route a composé.
            _ => TeamRunRemoteIssue::Refus {
                status: statut,
                url,
                body: corps,
            },
        });
    }

    let mut tampon: Vec<u8> = Vec::with_capacity(4096);
    let mut fin: Option<Result<TeamRun, String>> = None;
    // `fin.is_none()` en tête de boucle, et non un `break` dans le rappel : le
    // découpeur rend toutes les trames complètes du tampon d'un coup, donc
    // l'arrêt se décide entre deux lectures de socket.
    while fin.is_none() {
        let Some(morceau) =
            reponse
                .chunk()
                .await
                .map_err(|cause| TeamRunRemoteIssue::Transport {
                    url: url.clone(),
                    cause: cause.to_string(),
                })?
        else {
            break;
        };
        tampon.extend_from_slice(&morceau);
        let mut relais = |charge: &str| match serde_json::from_str::<TeamRunFrame>(charge) {
            Ok(TeamRunFrame::TeamEvent { payload }) => on_event(payload),
            Ok(TeamRunFrame::RunCompleted { run }) => fin = Some(Ok(run)),
            Ok(TeamRunFrame::RunError { error }) => fin = Some(Err(error)),
            Err(cause) => {
                eprintln!("  > (trame illisible, lecture poursuivie) {cause}: {charge}");
            }
        };
        super::tasks::drain_events(&mut tampon, &mut relais);
    }

    match fin {
        Some(Ok(run)) => Ok(run),
        Some(Err(error)) => Err(TeamRunRemoteIssue::RunError { error }),
        None => Err(TeamRunRemoteIssue::SansTrameFinale { url }),
    }
}

// Les tests vivent ici, **en fin de fichier** : le scanner d'egress coupe la
// production au premier `#[cfg(test)]`, et un item de production placé après en
// sortirait — le lint d'egress ne le verrait plus. `skills_remote.rs` porte la
// même note, répétée ici plutôt que pointée : un lecteur qui déplace un item ne
// lit pas le fichier d'à côté.
#[cfg(test)]
mod tests {
    use super::*;
    // Banc **emprunté** au voisin, et non recopié : un second témoin de socket
    // ouvrirait une seconde vérité sur ce que le serveur reçoit, libre d'en
    // diverger.
    use crate::commands::rewind_remote::tests::fake_spirit;
    use axum::http::StatusCode;
    use mika_common::dead_endpoint::DeadEndpoint;

    const EQUIPE: &str = "squad";
    const ROUTE: &str = "/api/v1/teams/squad/runs";
    const JETON: &str = "jeton-interne";

    /// Les trames jointes par le séparateur SSE, et un séparateur final pour
    /// que la dernière soit complète : `drain_events` laisse en place une trame
    /// partielle, donc une dernière sans séparateur ne serait jamais rendue.
    fn flux(trames: &[&str]) -> String {
        trames
            .iter()
            .map(|t| format!("data: {t}\n\n"))
            .collect::<Vec<_>>()
            .concat()
    }

    /// La porte lancée contre un factice qui sert `corps` sur [`ROUTE`], avec
    /// les événements reçus collectés dans l'ordre.
    async fn contre_le_factice(
        corps: &str,
    ) -> (Vec<TeamEvent>, Result<TeamRun, TeamRunRemoteIssue>) {
        let (base, _) = fake_spirit(&[(ROUTE, StatusCode::OK, corps)]).await;
        let mut recus = Vec::new();
        let rendu =
            lancer_run_equipe(&base, JETON, EQUIPE, "le but", None, |e| recus.push(e)).await;
        (recus, rendu)
    }

    /// Le `TeamRun` minimal du fil : tous ses champs sauf trois sont en
    /// `#[serde(default)]`, donc la trame s'écrit à la main plutôt qu'en
    /// construisant un `TeamRun` Rust puis en le sérialisant — ce qui mesurerait
    /// serde contre lui-même.
    ///
    /// `"status":"Completed"` en PascalCase, parce que `RunStatus` dérive
    /// `Serialize` sans `rename_all` : c'est bien le fil que la route produit,
    /// là où son `Display` — celui que l'enveloppe JSON de `mika ask` imprime —
    /// rend `completed`. Les deux casses sont voulues et cette trame porte celle
    /// du producteur.
    const RUN_COMPLETE: &str = r#"{"event":"run_completed","run":{"run_id":"run-7","team_name":"squad","goal":"le but","status":"Completed","iteration":2,"deliverable":"la réponse"}}"#;

    #[tokio::test]
    async fn mika1727_v4b1_la_requete_vise_la_route_avec_son_jeton_et_son_corps() {
        let (base, recues) = fake_spirit(&[(ROUTE, StatusCode::OK, &flux(&[RUN_COMPLETE]))]).await;
        lancer_run_equipe(&base, JETON, EQUIPE, "le but", Some("run-3"), |_| {})
            .await
            .expect("le factice sert une trame finale");

        let vues = recues.lock().unwrap();
        assert_eq!(vues.len(), 1);
        assert_eq!(vues[0].methode, "POST");
        assert_eq!(vues[0].chemin, ROUTE);
        assert_eq!(
            vues[0].autorisation.as_deref(),
            Some("Bearer jeton-interne")
        );
        // Octet pour octet : c'est le corps que la route désérialise, donc un
        // champ renommé ou un ordre changé rougit ici, pas chez elle.
        assert_eq!(
            vues[0].corps,
            r#"{"goal":"le but","reference_run_id":"run-3"}"#
        );
    }

    #[tokio::test]
    async fn mika1727_v4b1_les_evenements_arrivent_dans_lordre_puis_le_run() {
        let corps = flux(&[
            r#"{"event":"team_event","payload":{"kind":"progress","data":"un"}}"#,
            r#"{"event":"team_event","payload":{"kind":"progress","data":"deux"}}"#,
            r#"{"event":"team_event","payload":{"kind":"deliverable","data":"trois"}}"#,
            RUN_COMPLETE,
        ]);
        let (recus, rendu) = contre_le_factice(&corps).await;
        let run = rendu.expect("une trame finale ferme le flux");

        // En séquence, et non par un compte : c'est l'ordre qui est le critère.
        let lus: Vec<String> = recus
            .iter()
            .map(|e| match e {
                TeamEvent::Progress(m) => format!("progress:{m}"),
                TeamEvent::Deliverable(m) => format!("deliverable:{m}"),
                autre => format!("inattendu:{autre:?}"),
            })
            .collect();
        assert_eq!(lus, ["progress:un", "progress:deux", "deliverable:trois"]);
        assert_eq!(run.run_id, "run-7");
        assert_eq!(run.deliverable.as_deref(), Some("la réponse"));
    }

    #[tokio::test]
    async fn mika1727_v4b1_un_run_error_rend_lerreur_du_run() {
        let corps = flux(&[r#"{"event":"run_error","error":"la décomposition a échoué"}"#]);
        let (_, rendu) = contre_le_factice(&corps).await;
        let probleme = rendu.expect_err("une trame `run_error` est un échec");
        assert!(
            matches!(&probleme, TeamRunRemoteIssue::RunError { error } if error == "la décomposition a échoué"),
            "{probleme:?}"
        );
    }

    #[tokio::test]
    async fn mika1727_v4b1_un_flux_sans_trame_finale_nest_jamais_un_succes() {
        let corps = flux(&[
            r#"{"event":"team_event","payload":{"kind":"progress","data":"un"}}"#,
            r#"{"event":"team_event","payload":{"kind":"progress","data":"deux"}}"#,
        ]);
        let (recus, rendu) = contre_le_factice(&corps).await;
        // Les deux événements ont bien été vus : l'échec ne vient pas d'une
        // lecture qui n'a rien lu, mais de l'absence de trame terminale.
        assert_eq!(recus.len(), 2);
        let probleme = rendu.expect_err("un flux sans trame finale est un échec");
        assert!(
            matches!(probleme, TeamRunRemoteIssue::SansTrameFinale { .. }),
            "{probleme:?}"
        );
    }

    #[tokio::test]
    async fn mika1727_v4b1_une_trame_illisible_narrete_pas_la_lecture() {
        let corps = flux(&[
            r#"{"event":"team_event","payload":{"kind":"progress","data":"un"}}"#,
            // Deux illisibles de deux causes : une étiquette d'enveloppe
            // inconnue, et un `kind` de `TeamEvent` inconnu — ce dernier fait
            // échouer la trame **entière**, l'enum refusant `#[serde(other)]`.
            r#"{"event":"inconnu"}"#,
            r#"{"event":"team_event","payload":{"kind":"inconnu","data":"x"}}"#,
            RUN_COMPLETE,
        ]);
        let (recus, rendu) = contre_le_factice(&corps).await;
        let run = rendu.expect("la finale est lue malgré les trames illisibles");
        assert_eq!(run.run_id, "run-7");
        // Un seul événement passé au rappel : les deux illisibles n'ont pas été
        // inventées en événements vides.
        assert_eq!(recus.len(), 1);
    }

    #[tokio::test]
    async fn mika1727_v4b1_un_401_nomme_la_variable_a_poser() {
        let (base, _) = fake_spirit(&[(ROUTE, StatusCode::UNAUTHORIZED, "")]).await;
        let probleme = lancer_run_equipe(&base, JETON, EQUIPE, "le but", None, |_| {})
            .await
            .expect_err("un 401 est un échec");
        assert!(
            matches!(probleme, TeamRunRemoteIssue::Denied { .. }),
            "{probleme:?}"
        );
        // La variable et l'URL se lisent dans le `Display` et non dans le code :
        // c'est le message que l'opérateur verra, et lui seul tient la promesse.
        let message = probleme.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
    }

    #[tokio::test]
    async fn mika1727_v4b1_un_404_remonte_son_statut_et_son_corps() {
        const CORPS: &str = r#"{"error":"team 'squad' not found"}"#;
        let (base, _) = fake_spirit(&[(ROUTE, StatusCode::NOT_FOUND, CORPS)]).await;
        let probleme = lancer_run_equipe(&base, JETON, EQUIPE, "le but", None, |_| {})
            .await
            .expect_err("un 404 est un échec");
        assert!(
            matches!(&probleme, TeamRunRemoteIssue::Refus { status: 404, .. }),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
        assert!(message.contains(CORPS), "{message}");
    }

    #[tokio::test]
    async fn mika1727_v4b1_un_point_injoignable_est_une_erreur_de_transport() {
        // mika#2569 : le port est **tenu** pour toute la durée de l'appel. Le
        // motif qui liait puis libérait a vu un factice voisin du même binaire
        // servir l'appel censé échouer (2026-09-28, job `Check` de mika#2561).
        let dead = DeadEndpoint::reserve();
        let base = format!("http://{}", dead.addr());

        let probleme = lancer_run_equipe(&base, JETON, EQUIPE, "le but", None, |_| {})
            .await
            .expect_err("rien n'écoute : l'appel doit échouer");
        assert!(
            matches!(probleme, TeamRunRemoteIssue::Transport { .. }),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
    }
}
