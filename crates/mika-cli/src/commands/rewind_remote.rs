//! Le transport des routes rewind de mika-spirit
//! (`POST /api/v1/rewind/{resolve,preview,execute}`, mika#1727 vol V2c-1a-1) :
//! miroirs du fil, vocabulaire d'erreur, et un `post` unique. L'URL de base et
//! le jeton arrivent **par argument**, pour la raison déjà écrite sur
//! [`super::tasks::fetch_task_detail`] : un client qui résout ses propres accès
//! ne se pilote pas depuis un test. V2c-1a-2 câble les portes et la conversion
//! vers [`RewindPreview`], et retire avec elles les `expect(dead_code)` d'ici.

use mika_agent::rewind::RewindPreview;
use serde::Deserialize;

// Les six miroirs du fil. Les réponses de `mika-agent/src/server/rewind.rs` ne
// dérivent que `Serialize`, et `crates/mika-agent/` est hors périmètre : ces
// structs *sont* le contrat de fil côté client, sans règle de domaine (§ 2.4).

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct ResolveMirror {
    pub(crate) session_id: String,
    pub(crate) after_message_id: i64,
    pub(crate) trace_ids: Vec<String>,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct PreviewMirror {
    pub(crate) messages: Vec<MessageMirror>,
    pub(crate) reversals: Vec<ReversalMirror>,
    pub(crate) tasks: Vec<TaskMirror>,
    pub(crate) warnings: Vec<String>,
    pub(crate) blocked: bool,
    pub(crate) block_reason: Option<String>,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct MessageMirror {
    pub(crate) id: i64,
    pub(crate) role: String,
    pub(crate) content_snippet: String,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct ReversalMirror {
    pub(crate) audit_event_id: i64,
    pub(crate) tool_name: String,
    pub(crate) target_key: String,
    pub(crate) action: String,
    pub(crate) description: String,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct TaskMirror {
    pub(crate) task_id: String,
    pub(crate) label: String,
    pub(crate) actioned: bool,
}

#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug, Deserialize)]
pub(crate) struct ResultMirror {
    pub(crate) messages_deleted: usize,
    pub(crate) reversals_applied: usize,
    pub(crate) reversals_skipped: usize,
    pub(crate) tasks_deleted: usize,
    pub(crate) tasks_cancelled: usize,
    pub(crate) warnings: Vec<String>,
    pub(crate) reversal_descriptions: Vec<String>,
    pub(crate) rewind_trace_id: String,
}

/// Le vocabulaire d'erreur que les trois routes partagent. `Blocked` n'a aucun
/// producteur ici (`post` rend le 422 à l'appelant), mais serait à **redéfinir**
/// par V2c-1a-2 s'il manquait ; son `preview` est un `Option` parce que les deux
/// 422 diffèrent — aperçu entier pour `preview`, raison seule pour `execute` —
/// et un `Box` parce que c'est ici, non aux portes, que `result_large_err` pèse
/// le variant `Err`. Le message de `NotFound` nomme les **deux** causes du 404
/// sans en choisir une : en choisir une enverrait le lecteur chercher une
/// session là où le défaut est un nom d'agent. `Denied` ne porte rien, le nom de
/// la variable étant la moitié utile du message. Et `None` sur `status` dit
/// « aucune réponse reçue », là où un `0` se lirait comme un statut HTTP.
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
#[derive(Debug)]
pub(crate) enum RewindRemoteIssue {
    Blocked {
        reason: String,
        preview: Option<Box<RewindPreview>>,
    },
    NotFound {
        url: String,
    },
    Busy,
    Denied,
    Unexpected {
        status: Option<u16>,
        url: String,
        body: String,
    },
}

// `Display` à la main : rien à chaîner, aucune frontière de bibliothèque.
impl std::fmt::Display for RewindRemoteIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocked { reason, .. } => write!(f, "mika-spirit refuses the rewind: {reason}"),
            Self::NotFound { url } => write!(
                f,
                "mika-spirit found nothing at {url}: no exchanges to rewind, or unknown agent"
            ),
            Self::Busy => write!(f, "the agent is busy: another turn holds its lock"),
            Self::Denied => write!(
                f,
                "mika-spirit denied the request: set MIKA_INTERNAL_TOKEN to the server's token"
            ),
            Self::Unexpected { status, url, body } => match status {
                Some(statut) => write!(f, "mika-spirit returned {statut} for {url}: {body}"),
                None => write!(f, "mika-spirit is unreachable at {url}: {body}"),
            },
        }
    }
}

/// Le POST que les trois routes partagent. `route` est un `&str` et non un enum
/// à trois variants sans producteur : la socket mesure, pas le typage.
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
async fn post(
    base_url: &str,
    token: &str,
    route: &str,
    body: &serde_json::Value,
) -> Result<(u16, String), RewindRemoteIssue> {
    let url = format!("{}/api/v1/rewind/{route}", base_url.trim_end_matches('/'));
    // Client construit ici, au site de l'appel : c'est ce que déclare l'entrée
    // d'egress `cli-spirit-rewind`, qu'un client en argument rendrait aveugle.
    let reponse = reqwest::Client::new()
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .json(body)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|cause| RewindRemoteIssue::Unexpected {
            status: None,
            url: url.clone(),
            body: cause.to_string(),
        })?;

    // Statut lu avant le corps, comme `super::tasks::fetch_task_detail` : un
    // corps d'erreur désérialisé comme un succès est la panne à ne pas produire.
    let statut = reponse.status().as_u16();
    let corps = reponse.text().await.unwrap_or_default();
    match statut {
        // 200 et 422 remontent tels quels : les deux 422 n'ont pas la même forme
        // et seule la porte sait laquelle attendre. Frontière, pas arbitrage.
        200 | 422 => Ok((statut, corps)),
        401 => Err(RewindRemoteIssue::Denied),
        404 => Err(RewindRemoteIssue::NotFound { url }),
        409 => Err(RewindRemoteIssue::Busy),
        // Pas décoratif : `validate_rewind_request` rend des 400 côté serveur.
        _ => Err(RewindRemoteIssue::Unexpected {
            status: Some(statut),
            url,
            body: corps,
        }),
    }
}

// Les tests vivent ici, **en fin de fichier** : le scanner d'egress coupe la
// production au premier `#[cfg(test)]`, et un item d'après en sortirait.
#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::{Request, State};
    use axum::http::StatusCode;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Ce que le factice a vu sur la socket : le témoin est ce qui fait du
    /// `Bearer` et du corps intact des mesures, non des relectures du code.
    struct RequeteRecue {
        methode: String,
        chemin: String,
        autorisation: Option<String>,
        corps: String,
    }

    #[derive(Clone)]
    struct Factice {
        recues: Arc<Mutex<Vec<RequeteRecue>>>,
        reponses: Arc<HashMap<String, (StatusCode, String)>>,
    }

    async fn servir(State(etat): State<Factice>, requete: Request) -> (StatusCode, String) {
        let methode = requete.method().to_string();
        let chemin = requete.uri().path().to_string();
        let autorisation = requete
            .headers()
            .get("authorization")
            .map(|valeur| String::from_utf8_lossy(valeur.as_bytes()).into_owned());
        let corps = axum::body::to_bytes(requete.into_body(), 64 * 1024)
            .await
            .map(|octets| String::from_utf8_lossy(&octets).into_owned())
            .unwrap_or_default();
        let reponse = etat.reponses.get(&chemin).cloned();
        etat.recues.lock().unwrap().push(RequeteRecue {
            methode,
            chemin,
            autorisation,
            corps,
        });
        // Un chemin non associé rend 501, qu'aucun bras de `post` ne traduit :
        // une faute de chemin ne se lit jamais comme un `NotFound`.
        reponse.unwrap_or((StatusCode::NOT_IMPLEMENTED, String::new()))
    }

    /// Un `fallback` unique, et non trois routes déclarées : un seul factice sert
    /// les trois **et** prouve la visée, là où une route déclarée rendrait un 404
    /// qui se lirait comme le cas métier de `NotFound`.
    async fn fake_spirit(
        reponses: &[(&str, StatusCode, &str)],
    ) -> (String, Arc<Mutex<Vec<RequeteRecue>>>) {
        let recues = Arc::new(Mutex::new(Vec::new()));
        let table = reponses
            .iter()
            .map(|(chemin, statut, corps)| ((*chemin).to_string(), (*statut, (*corps).to_string())))
            .collect::<HashMap<_, _>>();
        let routeur = Router::new().fallback(servir).with_state(Factice {
            recues: Arc::clone(&recues),
            reponses: Arc::new(table),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adresse = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, routeur.into_make_service()).await;
        });
        (format!("http://{adresse}"), recues)
    }

    const TROIS_ROUTES: [&str; 3] = ["resolve", "preview", "execute"];

    #[tokio::test]
    async fn mika1727_post_vise_les_trois_routes_avec_son_jeton() {
        let (base, recues) = fake_spirit(&[
            ("/api/v1/rewind/resolve", StatusCode::OK, "{}"),
            ("/api/v1/rewind/preview", StatusCode::OK, "{}"),
            ("/api/v1/rewind/execute", StatusCode::OK, "{}"),
        ])
        .await;

        for route in TROIS_ROUTES {
            let corps = serde_json::json!({ "session_id": format!("s-{route}") });
            post(&base, "jeton-secret", route, &corps)
                .await
                .expect("le factice répond 200 sur les trois chemins");
        }

        let vues = recues.lock().unwrap();
        assert_eq!(vues.len(), 3);
        for (vue, route) in vues.iter().zip(TROIS_ROUTES) {
            assert_eq!(vue.methode, "POST");
            assert_eq!(vue.chemin, format!("/api/v1/rewind/{route}"));
            assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
            assert_eq!(vue.corps, format!(r#"{{"session_id":"s-{route}"}}"#));
        }
    }

    /// Un factice qui ne sert qu'un couple sur `route`, et l'appel que `post` y
    /// fait. L'URL de base revient avec, parce que T12 l'attend dans le message.
    async fn un_statut(
        route: &str,
        statut: StatusCode,
        corps: &str,
    ) -> (String, Result<(u16, String), RewindRemoteIssue>) {
        let chemin = format!("/api/v1/rewind/{route}");
        let (base, _) = fake_spirit(&[(chemin.as_str(), statut, corps)]).await;
        let rendu = post(&base, "jeton", route, &serde_json::json!({})).await;
        (base, rendu)
    }

    #[tokio::test]
    async fn mika1727_un_422_remonte_intact_a_lappelant() {
        const CORPS: &str = r#"{"blocked":true,"block_reason":"un verrou tient l'agent"}"#;
        let (_, rendu) = un_statut("preview", StatusCode::UNPROCESSABLE_ENTITY, CORPS).await;
        let (statut, corps) = rendu.expect("un 422 n'est pas une erreur de transport");
        assert_eq!(statut, 422);
        // Octet pour octet, et non un `contains` : c'est l'aperçu entier que la
        // porte devra désérialiser, donc une troncature rougit ici, pas chez elle.
        assert_eq!(corps, CORPS);
    }

    #[tokio::test]
    async fn mika1727_un_404_nomme_ses_deux_causes() {
        let (_, rendu) = un_statut("resolve", StatusCode::NOT_FOUND, "").await;
        let probleme = rendu.expect_err("un 404 est une erreur");
        let message = probleme.to_string();
        assert!(matches!(probleme, RewindRemoteIssue::NotFound { .. }));
        assert!(message.contains("no exchanges to rewind"), "{message}");
        assert!(message.contains("unknown agent"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_409_est_un_verrou() {
        let (_, rendu) = un_statut("execute", StatusCode::CONFLICT, "").await;
        let probleme = rendu.expect_err("un 409 est une erreur");
        assert!(matches!(probleme, RewindRemoteIssue::Busy), "{probleme:?}");
    }

    #[tokio::test]
    async fn mika1727_un_401_nomme_la_variable_a_poser() {
        let (_, rendu) = un_statut("preview", StatusCode::UNAUTHORIZED, "").await;
        let probleme = rendu.expect_err("un 401 est une erreur");
        assert!(
            matches!(probleme, RewindRemoteIssue::Denied),
            "{probleme:?}"
        );
        // La variable se lit dans le `Display` et non dans le code : c'est le
        // message que l'opérateur verra, et lui seul tient la promesse.
        let message = probleme.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_statut_hors_contrat_porte_son_statut_et_son_url() {
        let (base, rendu) = un_statut("execute", StatusCode::INTERNAL_SERVER_ERROR, "boom").await;
        let probleme = rendu.expect_err("un 500 est une erreur");
        let message = probleme.to_string();
        let forme = matches!(
            probleme,
            RewindRemoteIssue::Unexpected {
                status: Some(500),
                ..
            }
        );
        assert!(forme, "{probleme:?}");
        assert!(message.contains("500"), "{message}");
        assert!(
            message.contains(&format!("{base}/api/v1/rewind/execute")),
            "{message}"
        );
    }

    #[test]
    fn mika1727_les_miroirs_se_lisent_depuis_la_forme_du_serveur() {
        let apercu: PreviewMirror = serde_json::from_str(
            r#"{"messages":[{"id":41,"role":"user","content_snippet":"pose la question"}],
                "reversals":[{"audit_event_id":7,"tool_name":"store_fact","target_key":"people:vincent",
                              "action":"Restore","description":"restaure la fiche"}],
                "tasks":[{"task_id":"t-1","label":"relire le plan","actioned":true}],
                "warnings":["un avertissement"],"blocked":true,"block_reason":"verrou"}"#,
        )
        .expect("la forme de RewindPreviewResponse");
        assert_eq!(apercu.messages.len(), 1);
        assert_eq!(apercu.messages[0].id, 41);
        assert_eq!(apercu.messages[0].role, "user");
        assert_eq!(apercu.messages[0].content_snippet, "pose la question");
        assert_eq!(apercu.reversals.len(), 1);
        assert_eq!(apercu.reversals[0].audit_event_id, 7);
        assert_eq!(apercu.reversals[0].tool_name, "store_fact");
        assert_eq!(apercu.reversals[0].target_key, "people:vincent");
        // `action` arrive sous sa forme `Debug` côté serveur (F3) et reste une
        // `String` : la table des trois graphies est le § 2.4, hors de ce vol.
        assert_eq!(apercu.reversals[0].action, "Restore");
        assert_eq!(apercu.reversals[0].description, "restaure la fiche");
        assert_eq!(apercu.tasks.len(), 1);
        assert_eq!(apercu.tasks[0].task_id, "t-1");
        assert_eq!(apercu.tasks[0].label, "relire le plan");
        assert!(apercu.tasks[0].actioned);
        assert_eq!(apercu.warnings, vec!["un avertissement".to_string()]);
        assert!(apercu.blocked);
        assert_eq!(apercu.block_reason.as_deref(), Some("verrou"));

        let resolu: ResolveMirror = serde_json::from_str(
            r#"{"session_id":"s-1","after_message_id":40,"trace_ids":["tr-1","tr-2"]}"#,
        )
        .expect("la forme de RewindResolveResponse");
        assert_eq!(resolu.session_id, "s-1");
        assert_eq!(resolu.after_message_id, 40);
        assert_eq!(resolu.trace_ids, ["tr-1", "tr-2"]);

        let resultat: ResultMirror = serde_json::from_str(
            r#"{"messages_deleted":2,"reversals_applied":3,"reversals_skipped":1,
                "tasks_deleted":4,"tasks_cancelled":5,"warnings":["w"],
                "reversal_descriptions":["d"],"rewind_trace_id":"rw-1"}"#,
        )
        .expect("la forme de RewindResultResponse");
        assert_eq!(resultat.messages_deleted, 2);
        assert_eq!(resultat.reversals_applied, 3);
        assert_eq!(resultat.reversals_skipped, 1);
        assert_eq!(resultat.tasks_deleted, 4);
        assert_eq!(resultat.tasks_cancelled, 5);
        assert_eq!(resultat.warnings, vec!["w".to_string()]);
        assert_eq!(resultat.reversal_descriptions, vec!["d".to_string()]);
        assert_eq!(resultat.rewind_trace_id, "rw-1");
    }

    // Le seul bras qu'aucun statut ne produit ici : sans ce test, aucune mesure.
    #[test]
    fn mika1727_le_bras_bloque_se_lit_sans_apercu() {
        let bloque = RewindRemoteIssue::Blocked {
            reason: "une tâche tient le verrou".to_string(),
            preview: None,
        };
        assert!(
            bloque.to_string().contains("une tâche tient le verrou"),
            "le Display doit porter la raison, pas seulement le nom du bras : {bloque}"
        );
        // Le message tient sans aperçu : la forme du 422 d'`execute` (F5).
        assert!(matches!(
            bloque,
            RewindRemoteIssue::Blocked { preview: None, .. }
        ));
    }
}
