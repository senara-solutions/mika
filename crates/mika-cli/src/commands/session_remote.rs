//! Le transport des routes de session de mika-spirit — `POST /api/v1/sessions`,
//! `POST /api/v1/sessions/{id}/end` et `POST /api/v1/compact` (mika#1727, vols
//! V2d-3a et V2d-3b) : vocabulaire d'erreur, un `post` unique, et les portes
//! qui s'y posent. L'URL de base et le jeton arrivent **par argument**, pour
//! la raison déjà écrite sur [`super::rewind_remote`] : un client qui résout
//! ses propres accès ne se pilote pas depuis un test.
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
//! **État du vol.** Les trois portes sont là — `ouvrir`, `clore` et
//! `compacter` — et le vol V2d-4a en a câblé **une** : `commands/chat.rs`
//! atteint `ouvrir` au démarrage de la TUI, et lit les deux champs de
//! `CreatedSessionMirror`. `clore` et `compacter`, avec `EndedSessionMirror` et
//! `CompactedMirror`, attendent V2d-4b : hors test, personne ne les atteint
//! encore. Les `expect(dead_code)` qui le disent sont donc **quatre** — deux
//! portes, deux miroirs —, et ce chiffre se **mesure** sous
//! `cargo clippy -p mika-cli --all-targets`, il ne se devine pas. Une passe
//! verte le tient exact dans les deux sens : un `expect` de trop, laissé sur un
//! item qu'un appelant vient d'atteindre, serait rendu « unfulfilled » ; un de
//! moins laisserait un `dead_code` nu. C'est la seule chose qui le tiendra à
//! jour quand V2d-4b câblera les deux dernières.

use serde::Deserialize;

// Le miroir du fil. `CreateSessionResponse` ne dérive que `Serialize` et vit
// dans `crates/mika-agent/`, hors périmètre : ce miroir **est** le contrat de
// fil côté client, sans règle de domaine — la même phrase que
// `rewind_remote.rs:19-21`, et la même raison.
#[derive(Debug, Deserialize)]
pub(crate) struct CreatedSessionMirror {
    pub(crate) session_id: String,
    /// Vrai quand l'identifiant rendu est la session canonique de l'agent
    /// (mika#1401) : le client ne la clôt **jamais**. Depuis V2d-4a, c'est ce
    /// champ qui donne `is_singleton_session` à la TUI — le serveur est la
    /// seule autorité sur ce fait, et V2d-4b s'en servira pour décider s'il
    /// appelle `clore` en fin de session.
    pub(crate) canonical: bool,
}

// Même raison que ci-dessus pour `Deserialize` seul : `EndSessionResponse` ne
// dérive que `Serialize` et vit dans `crates/mika-agent/`, hors périmètre.
#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) struct EndedSessionMirror {
    /// Faux **sans erreur** quand la session visée est la canonique de l'agent :
    /// elle ne se clôt jamais (mika#1401, `server/sessions.rs:162-164`). La
    /// porte rend le couple tel quel ; c'est V2d-4 qui le lira.
    pub(crate) ended: bool,
    pub(crate) canonical: bool,
}

// Même raison encore pour `Deserialize` seul : `CompactResponse` ne dérive que
// `Serialize` et vit dans `crates/mika-agent/`, hors périmètre.
#[derive(Debug, Deserialize)]
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) struct CompactedMirror {
    /// « La compaction a été **demandée et n'a pas échoué** », et jamais « une
    /// ligne `summary` a été écrite ». Les deux booléens sont des 200 : `false`
    /// dit que le compte était sous le seuil (`server/compact.rs:93-95`), et
    /// `true` que `maybe_compact` a rendu `Ok` — mais elle relit le seuil
    /// elle-même et peut légitimement n'avoir rien fait. Une TUI qui écrirait
    /// « conversation compactée » sur ce `true` affirmerait donc un fait que le
    /// serveur s'est gardé d'attester.
    pub(crate) compacted: bool,
    /// Le compte **d'avant**, dans les deux issues : celui sur lequel la
    /// décision de compacter a été prise. Le lire comme « ce qui reste »
    /// afficherait un nombre inchangé là où la conversation vient d'être
    /// condensée, ou croissant là où elle a raccourci.
    pub(crate) messages: usize,
}

/// Le vocabulaire d'erreur que les trois routes partagent.
///
/// `Busy` a son producteur dans [`post`] — le 409 — et une seule porte peut
/// l'obtenir, [`compacter`] : les deux routes de session ne prennent aucun
/// verrou d'agent (`server/sessions.rs` le dit et explique pourquoi), là où
/// `/api/v1/compact` tient le sien pendant tout son handler. Le bras est donc
/// mesuré **deux fois**, sur `post` et par la porte, et garder la première
/// lecture est un **choix** plutôt qu'un reste : c'est elle qui tient la
/// traduction `409 → Busy` du transport, donc elle seule qui dirait, le jour
/// d'une rougeur, que le transport a bougé et non la porte.
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

/// Le dernier geste des trois portes : relire le statut, puis désérialiser le
/// corps en traduisant l'échec de lecture.
///
/// Extraite parce que « un 200 au corps illisible n'est **jamais** un succès »
/// est une propriété des trois portes et non de chacune : tenue en trois
/// exemplaires, elle dériverait sur deux d'entre eux sans qu'un test rougisse —
/// la panne que le § V3 de mika#1727 nomme déjà pour l'assemblage du
/// `SkillRegistry`. `quoi` nomme ce qui n'a pas pu être lu, et c'est tout ce
/// qui varie d'une porte à l'autre.
fn lire_json<T: serde::de::DeserializeOwned>(
    statut: u16,
    corps: String,
    url: String,
    quoi: &str,
) -> Result<T, SessionRemoteIssue> {
    // Statut lu avant la forme : `post` ne rend `Ok` que sur 200, mais la
    // condition est écrite ici plutôt que supposée — le jour où un bras de
    // `post` rendrait un autre statut sur son chemin `Ok`, un corps d'erreur
    // serait désérialisé comme un succès.
    if statut != 200 {
        return Err(SessionRemoteIssue::Unexpected {
            status: Some(statut),
            url,
            body: corps,
        });
    }
    serde_json::from_str(&corps).map_err(|cause| SessionRemoteIssue::Unexpected {
        status: Some(statut),
        url,
        // Le corps reçu voyage avec la cause : sans lui, « champ manquant
        // `canonical` » ne dit pas si le serveur a répondu autre chose.
        body: format!("unreadable {quoi} ({cause}): {corps}"),
    })
}

const ROUTE_SESSIONS: &str = "/api/v1/sessions";

// En constante, là où le chemin de `clore` est composé au site : celui-ci est
// entier et a plusieurs lecteurs — la porte, et chacun de ses tests —, tandis
// qu'un chemin qui porte un identifiant concret ne se nomme pas une fois pour
// toutes.
const ROUTE_COMPACT: &str = "/api/v1/compact";

/// Ouvre une session pour l'agent servi, et rend ce que mika-spirit a inscrit.
///
/// `agent` part **toujours**, y compris vide : c'est le serveur qui décide ce
/// qu'un nom vide vaut (`#[serde(default)]` puis 404 sur agent inconnu), et un
/// client qui tranche ici se mettrait à répondre à sa place.
pub(crate) async fn ouvrir(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<CreatedSessionMirror, SessionRemoteIssue> {
    let envoye = serde_json::json!({ "agent": agent });
    let (statut, corps) = post(base_url, token, ROUTE_SESSIONS, &envoye).await?;
    lire_json(
        statut,
        corps,
        session_url(base_url, ROUTE_SESSIONS),
        "created session",
    )
}

/// Clôt une session de l'agent servi, et rend ce que mika-spirit a décidé.
///
/// Les deux champs rendus ne sont **ni lus ni interprétés ici** :
/// `{ended: false, canonical: true}` est un 200, pas un refus — la session
/// canonique de l'agent ne se clôt jamais (mika#1401,
/// `server/sessions.rs:162-164`). Les convertir en erreur inventerait un échec
/// là où le serveur a répondu normalement ; les jeter priverait V2d-4 de la
/// seule chose dont il a besoin au moment de décider s'il rouvre une session.
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) async fn clore(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
) -> Result<EndedSessionMirror, SessionRemoteIssue> {
    // L'identifiant est inséré **tel quel**, sans encodage, et c'est une dette
    // bornée par le corps de mika#1727 : mesuré le 2026-11-14 sur la
    // table `sessions` de gentux (aucune ligne ne porte `/`, `?`, `#`, `%` ni
    // d'espace), et V2d-4 ne clôt que des identifiants rendus par `ouvrir`
    // (UUIDv4, ou la canonique). `Tracked in: senara-solutions/mika#1727`.
    //
    // Composé une fois parce qu'il a deux lecteurs : `post` et le `session_url`
    // que `lire_json` recompose pour nommer l'URL dans un `Unexpected`.
    let chemin = format!("{ROUTE_SESSIONS}/{session_id}/end");
    // `agent` part **toujours**, y compris vide, pour la raison d'`ouvrir` et
    // une de plus : le serveur déclare le champ `#[serde(default)]`, donc un
    // corps sans `agent` lui fait résoudre *son* agent par défaut. La porte
    // clorait la session d'un autre agent, et le contrôle d'appartenance ne la
    // rattraperait pas — `resolve_agent` aurait réussi.
    let envoye = serde_json::json!({ "agent": agent });
    let (statut, corps) = post(base_url, token, &chemin, &envoye).await?;
    lire_json(
        statut,
        corps,
        session_url(base_url, &chemin),
        "ended session",
    )
}

/// Compacte la conversation de l'agent servi, et rend ce que mika-spirit a fait.
///
/// La seule des trois portes qui puisse rendre [`SessionRemoteIssue::Busy`] :
/// le handler de `/api/v1/compact` prend le verrou de l'agent et rend 409 quand
/// `try_lock_owned` échoue (`server/compact.rs:73-82`). C'est un refus
/// temporaire, pas une panne — V2d-4 a de quoi proposer un nouvel essai, ce
/// qu'un `Unexpected` lui aurait retiré.
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2d-4, mika#1727"))]
pub(crate) async fn compacter(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<CompactedMirror, SessionRemoteIssue> {
    // `agent` part **toujours**, pour la raison des deux autres portes : la
    // route déclare le champ `#[serde(default)]`, donc un corps sans `agent`
    // ferait compacter la conversation de l'agent par défaut du conteneur — une
    // réécriture d'historique chez un tiers, en 200 et sans trace.
    let envoye = serde_json::json!({ "agent": agent });
    let (statut, corps) = post(base_url, token, ROUTE_COMPACT, &envoye).await?;
    lire_json(
        statut,
        corps,
        session_url(base_url, ROUTE_COMPACT),
        "compaction result",
    )
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
    use mika_common::dead_endpoint::DeadEndpoint;

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

    /// Le chemin de clôture tel que le banc doit le voir : `fake_spirit` associe
    /// par le chemin **brut**, donc l'identifiant concret en fait partie. Un
    /// `format!` mal composé dans la porte tombe sur le `fallback` à 501,
    /// qu'aucun bras de `post` ne traduit — c'est ce qui fait de ces tests une
    /// mesure de la **visée** du chemin, et pas seulement du statut rendu.
    const CHEMIN_FIN: &str = "/api/v1/sessions/s-1/end";

    #[tokio::test]
    async fn mika1727_clore_vise_la_route_de_fin_avec_son_jeton() {
        let (base, recues) = fake_spirit(&[(
            CHEMIN_FIN,
            StatusCode::OK,
            r#"{"ended":true,"canonical":false}"#,
        )])
        .await;

        let rendu = clore(&base, "jeton-secret", "mika-dev", "s-1")
            .await
            .expect("un 200 au corps lisible est un succès");

        // Les trois faits se lisent sur la socket et non sur l'appel, comme
        // pour `ouvrir` : c'est ce que le serveur reçoit qui tient le contrat.
        let vues = recues.lock().expect("le témoin du banc");
        assert_eq!(vues.len(), 1, "un seul appel attendu");
        let vue = &vues[0];
        assert_eq!(vue.methode, "POST");
        assert_eq!(vue.chemin, CHEMIN_FIN);
        assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
        let envoye: serde_json::Value = serde_json::from_str(&vue.corps).expect("un corps JSON");
        assert_eq!(envoye["agent"], "mika-dev");

        assert!(rendu.ended);
        assert!(!rendu.canonical);
    }

    #[tokio::test]
    async fn mika1727_clore_rend_la_canonique_telle_quelle() {
        let (base, _) = fake_spirit(&[(
            CHEMIN_FIN,
            StatusCode::OK,
            r#"{"ended":false,"canonical":true}"#,
        )])
        .await;

        // `expect` et non `expect_err` **est** la mesure : la traduction tentante
        // est de rendre une erreur sur `ended: false`, et le serveur n'a rien
        // refusé — la canonique ne se clôt simplement jamais.
        let rendu = clore(&base, "jeton", "mika-dev", "s-1")
            .await
            .expect("la canonique non close est un fait, pas une erreur");
        assert!(!rendu.ended);
        assert!(rendu.canonical);
    }

    #[tokio::test]
    async fn mika1727_clore_sur_404_rend_notfound() {
        // Par la **porte** et non par `un_statut` : le 404 du transport est déjà
        // mesuré plus bas, et le critère porte sur `clore`.
        let (base, _) = fake_spirit(&[(CHEMIN_FIN, StatusCode::NOT_FOUND, "")]).await;

        let probleme = clore(&base, "jeton", "mika-dev", "s-1")
            .await
            .expect_err("un 404 est une erreur");
        assert!(
            matches!(probleme, SessionRemoteIssue::NotFound { .. }),
            "{probleme:?}"
        );
        // Les deux causes du 404 sont nommées sans qu'une soit choisie : le
        // serveur rend le même 404 pour un agent inconnu, une session inconnue
        // et une session étrangère, par un seul site.
        let message = probleme.to_string();
        assert!(message.contains("unknown agent"), "{message}");
        assert!(message.contains("unknown session"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_clore_un_200_illisible_nest_jamais_un_succes() {
        // `canonical` manquant : la forme qu'un écart de version produit
        // réellement, et le champ dont un repli silencieux coûterait le plus —
        // c'est lui qui décidera si V2d-4 réouvre une session.
        let (base, _) = fake_spirit(&[(CHEMIN_FIN, StatusCode::OK, r#"{"ended":true}"#)]).await;

        let probleme = clore(&base, "jeton", "mika-dev", "s-1")
            .await
            .expect_err("un corps illisible n'est jamais un succès");
        assert!(
            matches!(
                probleme,
                SessionRemoteIssue::Unexpected {
                    status: Some(200),
                    ..
                }
            ),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(message.contains("unreadable ended session"), "{message}");
        assert!(message.contains(r#"{"ended":true}"#), "{message}");
    }

    #[tokio::test]
    async fn mika1727_compacter_vise_la_route_de_compaction_avec_son_jeton() {
        // `messages: 42` et non `0` : le compte d'avant est la valeur dont un
        // repli silencieux — un `usize` par défaut — se lirait comme « rien à
        // compacter », donc le seul chiffre qui mesure qu'il a bien été lu.
        let (base, recues) = fake_spirit(&[(
            ROUTE_COMPACT,
            StatusCode::OK,
            r#"{"compacted":true,"messages":42}"#,
        )])
        .await;

        let rendu = compacter(&base, "jeton-secret", "mika-dev")
            .await
            .expect("un 200 au corps lisible est un succès");

        let vues = recues.lock().expect("le témoin du banc");
        assert_eq!(vues.len(), 1, "un seul appel attendu");
        let vue = &vues[0];
        assert_eq!(vue.methode, "POST");
        assert_eq!(vue.chemin, ROUTE_COMPACT);
        assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
        let envoye: serde_json::Value = serde_json::from_str(&vue.corps).expect("un corps JSON");
        assert_eq!(envoye["agent"], "mika-dev");

        assert!(rendu.compacted);
        assert_eq!(rendu.messages, 42);
    }

    #[tokio::test]
    async fn mika1727_compacter_sur_409_rend_un_verrou() {
        // Par la **porte**, ce qu'exige le critère 5 : le bras du transport est
        // déjà mesuré plus bas, et ce que le vol ajoute est qu'une porte peut
        // désormais l'atteindre.
        let (base, _) = fake_spirit(&[(ROUTE_COMPACT, StatusCode::CONFLICT, "")]).await;

        let probleme = compacter(&base, "jeton", "mika-dev")
            .await
            .expect_err("un 409 est une erreur");
        assert!(matches!(probleme, SessionRemoteIssue::Busy), "{probleme:?}");
        // Le message dit le verrou et non l'indisponibilité : l'appelant a de
        // quoi réessayer, et l'opérateur de quoi ne pas chercher une panne.
        let message = probleme.to_string();
        assert!(message.contains("another turn holds its lock"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_compacter_un_200_illisible_nest_jamais_un_succes() {
        // `messages` manquant, `compacted` présent : la forme qu'un écart de
        // version produit, et celle où un `Default` rendrait le succès le plus
        // plausible — `compacted: true, messages: 0`, qu'on afficherait.
        let (base, _) =
            fake_spirit(&[(ROUTE_COMPACT, StatusCode::OK, r#"{"compacted":true}"#)]).await;

        let probleme = compacter(&base, "jeton", "mika-dev")
            .await
            .expect_err("un corps illisible n'est jamais un succès");
        assert!(
            matches!(
                probleme,
                SessionRemoteIssue::Unexpected {
                    status: Some(200),
                    ..
                }
            ),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(
            message.contains("unreadable compaction result"),
            "{message}"
        );
        assert!(message.contains(r#"{"compacted":true}"#), "{message}");
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
        // Sur `post` directement, et gardé maintenant que `compacter` mesure le
        // même bras par la porte : celui-ci tient la traduction `409 → Busy` du
        // transport, et il est le seul qui, en rougissant, dirait que c'est le
        // transport qui a bougé plutôt que la porte.
        let (_, rendu) = un_statut(ROUTE_COMPACT, StatusCode::CONFLICT, "").await;
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

    #[tokio::test]
    async fn mika1727_un_point_injoignable_na_pas_de_statut() {
        // mika#2569 : le port est **tenu** pour toute la durée de l'appel. Le
        // motif qui liait puis libérait a vu un factice voisin du même binaire
        // servir l'appel censé échouer (2026-09-28, job `Check` de mika#2561).
        let dead = DeadEndpoint::reserve();
        let base = format!("http://{}", dead.addr());

        let probleme = ouvrir(&base, "jeton", "mika-dev")
            .await
            .expect_err("rien n'écoute : l'appel doit échouer");
        assert!(
            matches!(
                probleme,
                SessionRemoteIssue::Unexpected { status: None, .. }
            ),
            "{probleme:?}"
        );
        // `None` et non `0` : l'opérateur doit lire « injoignable » et non un
        // statut que le serveur n'a jamais rendu.
        let message = probleme.to_string();
        assert!(message.contains("unreachable"), "{message}");
        assert!(
            message.contains(&format!("{base}{ROUTE_SESSIONS}")),
            "{message}"
        );
    }

    #[tokio::test]
    async fn mika1727_un_200_illisible_nest_jamais_un_succes() {
        // Un corps plausible auquel `canonical` manque : c'est la panne d'un
        // serveur d'une autre version, pas un corps absurde.
        let (base, recues) =
            fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, r#"{"session_id":"s-1"}"#)]).await;

        let probleme = ouvrir(&base, "jeton", "mika-dev")
            .await
            .expect_err("un corps illisible n'est jamais un succès");
        assert!(
            matches!(
                probleme,
                SessionRemoteIssue::Unexpected {
                    status: Some(200),
                    ..
                }
            ),
            "{probleme:?}"
        );
        // Le corps reçu voyage dans le message : sans lui, « champ manquant
        // `canonical` » ne dit pas ce que le serveur a répondu.
        let message = probleme.to_string();
        assert!(message.contains("unreadable created session"), "{message}");
        assert!(message.contains(r#"{"session_id":"s-1"}"#), "{message}");
        // L'appel est bien parti : l'échec est une lecture, pas un refus posé
        // avant le fil.
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            1,
            "un seul appel attendu"
        );
    }
}
