//! Le transport des routes de session de mika-spirit — `POST /api/v1/sessions`,
//! `POST /api/v1/sessions/{id}/end` et `POST /api/v1/compact` (mika#1727, vol
//! V2d-3a) : vocabulaire d'erreur, un `post` unique, et les portes qui s'y
//! posent. L'URL de base et le jeton arrivent **par argument**, pour la raison
//! déjà écrite sur [`super::rewind_remote`] : un client qui résout ses propres
//! accès ne se pilote pas depuis un test.
//!
//! **Pourquoi ce transport n'est pas mis en commun avec `rewind_remote.rs`.**
//! Une entrée d'egress déclare **un** site de construction de client, et sa
//! ligne `data` décrit ce qui part de ce site. Un `post` partagé obligerait soit
//! une entrée à décrire deux familles de routes — une ligne `data` qu'on ne sait
//! pas écrire sans mentir sur l'une des deux —, soit deux entrées à pointer le
//! même `client_site`, et la propriété « une entrée, un site » serait perdue. La
//! soixantaine de lignes dupliquée est donc payée délibérément. La moitié
//! voisine de ce raisonnement est écrite sur `rewind_remote::post` : le client
//! est construit au site de l'appel parce que c'est ce que l'entrée déclare.
//!
//! **État du vol.** Une seule porte ici, `ouvrir`. « Clore » et « compacter »
//! arrivent en V2d-3b sur ce même transport, et **aucun appelant** ne les
//! atteint avant V2d-4. Le seul `expect(dead_code)` du module est donc sur
//! `CreatedSessionMirror`, dont hors test personne ne lit les deux champs : les
//! attributs se **mesurent** sous `cargo clippy --all-targets`, ils ne se
//! devinent pas — un `expect` posé par symétrie sur `ouvrir` ou sur `post`
//! serait rendu « unfulfilled », donc un avertissement de plus.

use serde::Deserialize;

// Le miroir du fil. `CreateSessionResponse` ne dérive que `Serialize` et vit
// dans `crates/mika-agent/`, hors périmètre : ce miroir **est** le contrat de
// fil côté client, sans règle de domaine — la même phrase que
// `rewind_remote.rs:19-21`, et la même raison.
#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) struct CreatedSessionMirror {
    pub(crate) session_id: String,
    /// Vrai quand l'identifiant rendu est la session canonique de l'agent
    /// (mika#1401) : le client ne la clôt **jamais**. C'est V2d-4 qui lira ce
    /// champ pour décider s'il appelle `clore` en fin de session.
    pub(crate) canonical: bool,
}

/// Le vocabulaire d'erreur que les trois routes partagent.
///
/// `Busy` a son producteur dans [`post`] — le 409 — mais aucune porte de ce vol
/// ne peut l'obtenir : `POST /api/v1/sessions` n'émet jamais 409, le producteur
/// réel est `/api/v1/compact`, dont la porte arrive en V2d-3b. Son test appelle
/// donc `post` directement, et c'est la seule lecture qui mesure le bras.
///
/// Le message de `NotFound` nomme les **deux** causes du 404 sans en choisir
/// une : en choisir une enverrait le lecteur chercher une session là où le
/// défaut est un nom d'agent (motif mika#2331 — une provenance fausse coûte
/// plus cher qu'une provenance absente). `Denied` ne porte rien, le nom de la
/// variable à poser étant la moitié utile du message. Et `None` sur `status` dit
/// « aucune réponse reçue », là où un `0` se lirait comme un statut HTTP.
#[derive(Debug)]
pub(crate) enum SessionRemoteIssue {
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
impl std::fmt::Display for SessionRemoteIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { url } => write!(
                f,
                "mika-spirit found nothing at {url}: unknown agent, or unknown session"
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

/// L'URL d'une route de session, extraite parce qu'elle a **deux** lecteurs :
/// [`post`] ne rend pas l'URL sur son chemin `Ok`, donc une porte qui veut la
/// nommer dans un `Unexpected` doit la recomposer. Deux `format!` jumeaux
/// dériveraient ; ici le `trim_end_matches` qui absorbe un `base_url` à barre
/// finale vit une fois.
fn session_url(base_url: &str, chemin: &str) -> String {
    format!("{}{chemin}", base_url.trim_end_matches('/'))
}

/// Le POST que les trois routes partagent. `chemin` est le chemin **entier** et
/// non un segment de route : `/api/v1/sessions`, `/api/v1/sessions/{id}/end` et
/// `/api/v1/compact` ne partagent que `/api/v1`, donc un préfixe commun collé
/// ici mentirait sur deux des trois. Et il reste un `&str` plutôt qu'un enum
/// sans producteur : la socket mesure, pas le typage.
async fn post(
    base_url: &str,
    token: &str,
    chemin: &str,
    corps: &serde_json::Value,
) -> Result<(u16, String), SessionRemoteIssue> {
    let url = session_url(base_url, chemin);
    // Client construit ici, au site de l'appel : c'est ce que déclare l'entrée
    // d'egress `cli-spirit-sessions`, qu'un client en argument rendrait aveugle.
    let reponse = reqwest::Client::new()
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .json(corps)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|cause| SessionRemoteIssue::Unexpected {
            status: None,
            url: url.clone(),
            body: cause.to_string(),
        })?;

    // Statut lu avant le corps, comme `super::rewind_remote::post` : un corps
    // d'erreur désérialisé comme un succès est la panne à ne pas produire.
    let statut = reponse.status().as_u16();
    let corps = reponse.text().await.unwrap_or_default();
    match statut {
        200 => Ok((statut, corps)),
        401 => Err(SessionRemoteIssue::Denied),
        404 => Err(SessionRemoteIssue::NotFound { url }),
        // Le verrou d'agent de `/api/v1/compact`. Les deux routes de session
        // n'en prennent pas (`server/sessions.rs` le dit et explique pourquoi).
        409 => Err(SessionRemoteIssue::Busy),
        // Pas décoratif : les trois routes rendent un 500 sur échec d'écriture.
        _ => Err(SessionRemoteIssue::Unexpected {
            status: Some(statut),
            url,
            body: corps,
        }),
    }
}

const ROUTE_SESSIONS: &str = "/api/v1/sessions";

/// Ouvre une session pour l'agent servi, et rend ce que mika-spirit a inscrit.
///
/// `agent` part **toujours**, y compris vide : c'est le serveur qui décide ce
/// qu'un nom vide vaut (`#[serde(default)]` puis 404 sur agent inconnu), et un
/// client qui tranche ici se mettrait à répondre à sa place.
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) async fn ouvrir(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<CreatedSessionMirror, SessionRemoteIssue> {
    let envoye = serde_json::json!({ "agent": agent });
    let (statut, corps) = post(base_url, token, ROUTE_SESSIONS, &envoye).await?;
    // Statut lu avant la forme : `post` ne rend `Ok` que sur 200, mais la
    // condition est écrite ici plutôt que supposée — le jour où un bras de
    // `post` rendrait un autre statut sur son chemin `Ok`, un corps d'erreur
    // serait désérialisé comme un succès.
    if statut != 200 {
        return Err(SessionRemoteIssue::Unexpected {
            status: Some(statut),
            url: session_url(base_url, ROUTE_SESSIONS),
            body: corps,
        });
    }
    serde_json::from_str(&corps).map_err(|cause| SessionRemoteIssue::Unexpected {
        status: Some(statut),
        url: session_url(base_url, ROUTE_SESSIONS),
        // Le corps reçu voyage avec la cause : sans lui, « champ manquant
        // `canonical` » ne dit pas si le serveur a répondu autre chose.
        body: format!("unreadable created session ({cause}): {corps}"),
    })
}

// Les tests vivent ici, **en fin de fichier** : le scanner d'egress coupe la
// production au premier `#[cfg(test)]`, et un item d'après en sortirait.
#[cfg(test)]
mod tests {
    use super::*;
    // Banc **emprunté** au voisin, et non recopié : un second témoin de socket
    // ouvrirait une seconde vérité sur ce que le serveur reçoit, libre d'en
    // diverger. Il est `pub(crate)` depuis V2c-1b, pour ce motif-là.
    use crate::commands::rewind_remote::tests::fake_spirit;
    use axum::http::StatusCode;

    /// Un factice qui ne sert qu'un couple sur `chemin`, et l'appel que `post` y
    /// fait. L'URL de base revient avec, parce que le critère 3 l'attend dans le
    /// message.
    async fn un_statut(
        chemin: &str,
        statut: StatusCode,
        corps: &str,
    ) -> (String, Result<(u16, String), SessionRemoteIssue>) {
        let (base, _) = fake_spirit(&[(chemin, statut, corps)]).await;
        let rendu = post(&base, "jeton", chemin, &serde_json::json!({})).await;
        (base, rendu)
    }

    #[tokio::test]
    async fn mika1727_ouvrir_vise_la_route_des_sessions_avec_son_jeton() {
        let (base, recues) = fake_spirit(&[(
            ROUTE_SESSIONS,
            StatusCode::OK,
            r#"{"session_id":"s-neuve","canonical":false}"#,
        )])
        .await;

        let rendu = ouvrir(&base, "jeton-secret", "mika-dev")
            .await
            .expect("un 200 au corps lisible est un succès");

        // Les trois faits se lisent sur la socket et non sur l'appel : c'est ce
        // que le serveur reçoit qui tient le contrat, pas ce qu'on croit poster.
        let vues = recues.lock().expect("le témoin du banc");
        // Le compte sans rendu `Debug` : `RequeteRecue` n'en dérive pas, et le
        // banc du voisin reste intact dans ce vol.
        assert_eq!(vues.len(), 1, "un seul appel attendu");
        let vue = &vues[0];
        assert_eq!(vue.methode, "POST");
        assert_eq!(vue.chemin, ROUTE_SESSIONS);
        assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
        // Le corps se lit désérialisé plutôt qu'octet pour octet : le serveur
        // lit un champ, pas un ordre de clés.
        let envoye: serde_json::Value = serde_json::from_str(&vue.corps).expect("un corps JSON");
        assert_eq!(envoye["agent"], "mika-dev");

        assert_eq!(rendu.session_id, "s-neuve");
        assert!(!rendu.canonical);
    }

    #[tokio::test]
    async fn mika1727_un_401_nomme_la_variable_a_poser() {
        let (_, rendu) = un_statut(ROUTE_SESSIONS, StatusCode::UNAUTHORIZED, "").await;
        let probleme = rendu.expect_err("un 401 est une erreur");
        assert!(
            matches!(probleme, SessionRemoteIssue::Denied),
            "{probleme:?}"
        );
        // La variable se lit dans le `Display` et non dans le code : c'est le
        // message que l'opérateur verra, et lui seul tient la promesse.
        let message = probleme.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_404_nomme_ses_deux_causes() {
        let (_, rendu) = un_statut(ROUTE_SESSIONS, StatusCode::NOT_FOUND, "").await;
        let probleme = rendu.expect_err("un 404 est une erreur");
        let message = probleme.to_string();
        assert!(matches!(probleme, SessionRemoteIssue::NotFound { .. }));
        assert!(message.contains("unknown agent"), "{message}");
        assert!(message.contains("unknown session"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_409_est_un_verrou() {
        // Sur `post` directement : aucune porte de ce vol ne peut obtenir ce
        // bras, et son producteur réel (`/api/v1/compact`) arrive en V2d-3b.
        let (_, rendu) = un_statut("/api/v1/compact", StatusCode::CONFLICT, "").await;
        let probleme = rendu.expect_err("un 409 est une erreur");
        assert!(matches!(probleme, SessionRemoteIssue::Busy), "{probleme:?}");
    }

    #[tokio::test]
    async fn mika1727_un_statut_hors_contrat_porte_son_statut_et_son_url() {
        let (base, rendu) =
            un_statut(ROUTE_SESSIONS, StatusCode::INTERNAL_SERVER_ERROR, "boom").await;
        let probleme = rendu.expect_err("un 500 est une erreur");
        assert!(
            matches!(
                probleme,
                SessionRemoteIssue::Unexpected {
                    status: Some(500),
                    ..
                }
            ),
            "{probleme:?}"
        );
        // Le statut et l'URL visée dans le message : sans eux, l'opérateur sait
        // qu'un appel a échoué sans savoir lequel ni contre quoi.
        let message = probleme.to_string();
        assert!(message.contains("500"), "{message}");
        assert!(
            message.contains(&format!("{base}{ROUTE_SESSIONS}")),
            "{message}"
        );
    }
}
