//! Le transport des routes rewind de mika-spirit
//! (`POST /api/v1/rewind/{resolve,preview,execute}`, mika#1727 vol V2c-1a-1) :
//! miroirs du fil, vocabulaire d'erreur, et un `post` unique. L'URL de base et
//! le jeton arrivent **par argument**, pour la raison déjà écrite sur
//! [`super::tasks::fetch_task_detail`] : un client qui résout ses propres accès
//! ne se pilote pas depuis un test. Les **trois** portes sont posées —
//! [`preview`] et la conversion vers [`RewindPreview`] (vol V2c-1a-2a),
//! [`execute`] (vol V2c-1a-2b), [`resolve`] (vol V2c-1a-2c) — et depuis le vol
//! V2c-1b **elles ont leur appelant** : `tui/commands/handlers.rs::rewind_par_spirit`,
//! qui y fait passer `/undo`, `/rewind <n>` et `/rewind to <id>`. Les six
//! `expect(dead_code)` que ce fichier portait sont tombées avec ce câblage ; le
//! `dead_code` qui mordait ici est désormais celui qui mord chez l'appelant.

use mika_agent::rewind::{
    MessagePreview, ReversalAction, ReversalPreview, RewindPreview, TaskPreview,
};
use serde::Deserialize;

// Les sept miroirs du fil. Les réponses de `mika-agent/src/server/rewind.rs` ne
// dérivent que `Serialize`, et `crates/mika-agent/` est hors périmètre : ces
// structs *sont* le contrat de fil côté client, sans règle de domaine (§ 2.4).

// Le miroir ne déclare plus `trace_ids`, que le serveur continue d'envoyer :
// `serde` ignore par défaut ce qu'un `Deserialize` ne déclare pas, donc rien ne
// casse sur le fil. Ce qui change, c'est que le miroir ne déclare plus que ce
// qu'il **consomme** — l'ancrage repart vers [`preview`] puis [`execute`] et
// n'est jamais montré à l'opérateur, donc les traces n'avaient aucun lecteur
// naturel. Un champ déclaré sans lecteur est du code mort sous une attente. Le
// jour où un appelant veut les traces, il rajoute le champ et son lecteur dans
// le même geste — la seule façon de ne jamais retomber ici.
#[derive(Debug, Deserialize)]
pub(crate) struct ResolveMirror {
    pub(crate) session_id: String,
    pub(crate) after_message_id: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PreviewMirror {
    pub(crate) messages: Vec<MessageMirror>,
    pub(crate) reversals: Vec<ReversalMirror>,
    pub(crate) tasks: Vec<TaskMirror>,
    pub(crate) warnings: Vec<String>,
    pub(crate) blocked: bool,
    pub(crate) block_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageMirror {
    pub(crate) id: i64,
    pub(crate) role: String,
    pub(crate) content_snippet: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReversalMirror {
    pub(crate) audit_event_id: i64,
    pub(crate) tool_name: String,
    pub(crate) target_key: String,
    pub(crate) action: String,
    pub(crate) description: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskMirror {
    pub(crate) task_id: String,
    pub(crate) label: String,
    pub(crate) actioned: bool,
}

// Les huit champs ont désormais leur lecteur, dans le rendu de
// `rewind_par_spirit` — y compris les trois que l'attente couvrait seule
// (`reversals_skipped`, `reversal_descriptions`, `rewind_trace_id`).
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

/// Le corps du 422 d'`execute` : `{"error": "…"}`, et non l'aperçu entier que
/// `preview` renvoie sous le même statut (F5).
///
/// **Pourquoi un type, et non un `serde_json::Value`.** Lire la raison par
/// `valeur.get("error").and_then(Value::as_str)` tiendrait en une ligne, et
/// c'est cette ligne qui serait fragile : la clé ne serait alors déclarée nulle
/// part comme un contrat, et un `error` renommé côté serveur donnerait
/// silencieusement le repli de la porte — un refus affiché comme « sans raison
/// lisible » là où le serveur a dit pourquoi. Déclaré ici, c'est `from_str` qui
/// échoue, dans le bras même que les tests mesurent.
#[derive(Debug, Deserialize)]
pub(crate) struct ErrorMirror {
    pub(crate) error: String,
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
#[derive(Debug)]
pub(crate) enum RewindRemoteIssue {
    Blocked {
        reason: String,
        // Lu par `rewind_par_spirit`, qui affiche l'aperçu du refus sous sa
        // raison quand le serveur l'a joint.
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

/// L'URL d'une route rewind, extraite parce qu'elle a **deux** lecteurs. `post`
/// rend `Result<(u16, String), _>` : sur le chemin `Ok` l'URL n'est pas rendue,
/// et une porte qui veut la nommer dans un `Unexpected` doit la recomposer. Deux
/// `format!` jumeaux dériveraient le jour où le préfixe de route change ; ici le
/// `trim_end_matches` qui absorbe un `base_url` à barre finale vit une fois.
fn rewind_url(base_url: &str, route: &str) -> String {
    format!("{}/api/v1/rewind/{route}", base_url.trim_end_matches('/'))
}

/// Le POST que les trois routes partagent. `route` est un `&str` et non un enum
/// à trois variants sans producteur : la socket mesure, pas le typage.
async fn post(
    base_url: &str,
    token: &str,
    route: &str,
    body: &serde_json::Value,
) -> Result<(u16, String), RewindRemoteIssue> {
    let url = rewind_url(base_url, route);
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

/// L'ancrage d'un rembobinage de `count` échanges, tel que mika-spirit le
/// calcule : la session visée et le `after_message_id` jusqu'où remonter.
///
/// Rendu en [`ResolveMirror`] et non converti vers un type de domaine, parce
/// qu'il n'y en a pas : les deux champs repartent tels quels vers [`preview`]
/// puis [`execute`], et l'ancrage n'est jamais montré à l'opérateur.
pub(crate) async fn resolve(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
    count: usize,
) -> Result<ResolveMirror, RewindRemoteIssue> {
    // `agent` part **toujours**, pour la raison écrite sur [`preview`]. La
    // conséquence immédiate d'une omission est ici plus douce — un ancrage lu sur
    // le fil d'un autre agent, pas un rembobinage appliqué dessus — mais c'est
    // **le même ancrage** que V2c-1b passera ensuite à [`execute`] : l'erreur ne
    // serait pas rattrapée plus loin, elle serait seulement commise plus tôt.
    //
    // `count` part toujours lui aussi : le serveur le déclare
    // `#[serde(default = "default_count")]` à **1**, donc une clé absente ferait
    // rembobiner un seul échange là où l'appelant en demande N, en silence et
    // sous un 200.
    //
    // Pas de garde client sur `count == 0` : le serveur rend un 400 avec sa
    // raison, que `post` transmet en `Unexpected`. Une garde ici dupliquerait la
    // validation du serveur — deux vérités pour une règle — et masquerait le jour
    // où la règle change là-bas.
    let envoye = serde_json::json!({
        "session_id": session_id,
        "count": count,
        "agent": agent,
    });
    let (statut, corps) = post(base_url, token, "resolve", &envoye).await?;

    // Le statut est lu **avant** la forme, comme sur [`execute`] : `post` rend
    // 200 *et* 422 sur son chemin `Ok`, et tenter `ResolveMirror` d'abord ferait
    // porter au message d'un 422 une cause de serde au lieu du statut reçu.
    //
    // Un statut autre que 200 est `Unexpected`, **jamais** `Blocked`, et c'est la
    // conduite inverse de celle d'`execute` pour la même règle : le statut dit ce
    // que le serveur a décidé, et `handle_rewind_resolve` n'émet aucun 422 — il
    // ne refuse pas un rembobinage, il cherche un ancrage. Rendre `Blocked`
    // inventerait une décision que le serveur n'a pas prise ; le lire comme un
    // succès donnerait un ancrage vide.
    //
    // La condition est `!= 200` et non `== 422` : rien ne garantit que le chemin
    // `Ok` de `post` restera à deux statuts.
    if statut != 200 {
        return Err(RewindRemoteIssue::Unexpected {
            status: Some(statut),
            url: rewind_url(base_url, "resolve"),
            body: corps,
        });
    }

    serde_json::from_str(&corps).map_err(|cause| RewindRemoteIssue::Unexpected {
        status: Some(statut),
        url: rewind_url(base_url, "resolve"),
        body: format!("unreadable rewind anchor ({cause}): {corps}"),
    })
}

/// Les trois graphies que le serveur écrit par `format!("{:?}")`, et le refus de
/// tout le reste.
///
/// **Aucun repli sur `Skip`.** `Skip` veut dire « rien à inverser », et
/// `format_preview` lui met l'icône `⏭` : un repli ferait donc afficher « rien à
/// inverser » pour une variante que le serveur aurait ajoutée — il mentirait sur
/// un effet mémoire non réversible, en présentant une mutation qu'on ne sait pas
/// défaire comme une mutation qu'il n'y a pas à défaire. Un repli sur `Restore`
/// ou `Delete` serait pire encore : il annoncerait un effet.
///
/// Pas de `Deserialize` sur `ReversalAction` pour s'en dispenser : ce type vit
/// dans `crates/mika-agent/`, hors périmètre, et un dérive ajouté là produirait
/// par défaut le même refus **sans** le message qui nomme la valeur reçue.
fn reversal_action_from_label(label: &str) -> Result<ReversalAction, String> {
    match label {
        "Restore" => Ok(ReversalAction::Restore),
        "Delete" => Ok(ReversalAction::Delete),
        "Skip" => Ok(ReversalAction::Skip),
        inconnue => Err(format!(
            "mika-spirit sent an unknown reversal action {inconnue:?} \
             (expected one of \"Restore\", \"Delete\", \"Skip\")"
        )),
    }
}

/// L'aperçu du fil reconstruit sous la forme du domaine, pour que l'affichage
/// reste [`mika_agent::rewind::format_preview`] : c'est la seule mise en forme
/// de l'aperçu, elle est testée chez elle, et un second formateur côté CLI
/// serait exactement la duplication que mika#1727 existe pour retirer. La
/// conversion est le prix d'admission, et elle garde une seule vérité
/// d'affichage — d'autant qu'aucun des quatre types d'aperçu ne dérive
/// `PartialEq` : comparer deux aperçus ne peut passer que par ce texte.
fn preview_to_domain(miroir: PreviewMirror) -> Result<RewindPreview, String> {
    let reversals = miroir
        .reversals
        .into_iter()
        .map(|reversal| {
            Ok(ReversalPreview {
                audit_event_id: reversal.audit_event_id,
                tool_name: reversal.tool_name,
                target_key: reversal.target_key,
                // Une action inconnue fait échouer l'aperçu **entier**, par `?`,
                // et n'est pas un reversal silencieusement sauté : sauter
                // rendrait un aperçu *plus court que la réalité* — un opérateur
                // lirait « 2 changements mémoire » là où il y en a trois, et
                // validerait un rembobinage dont une partie ne se verrait jamais.
                action: reversal_action_from_label(&reversal.action)?,
                description: reversal.description,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(RewindPreview {
        messages: miroir
            .messages
            .into_iter()
            .map(|message| MessagePreview {
                id: message.id,
                role: message.role,
                content_snippet: message.content_snippet,
                // Le fil ne porte pas ce champ (`MessagePreviewResponse`,
                // `server/rewind.rs:31-35`) et `format_preview` ne le lit pas :
                // il lit le *nombre* de messages, puis les reversals, les tasks
                // et les warnings. L'absence est donc sans effet sur le texte
                // **aujourd'hui**, et c'est la seule divergence que cette
                // conversion introduit volontairement. La ligne qui rougira à la
                // relecture le jour où elle cesse d'être sans effet est
                // `rewind.rs:739` : si `format_preview` se met à lire `trace_id`,
                // c'est le fil qu'il faut élargir, pas ce `None` qu'il faut
                // deviner.
                trace_id: None,
            })
            .collect(),
        reversals,
        tasks: miroir
            .tasks
            .into_iter()
            .map(|tache| TaskPreview {
                task_id: tache.task_id,
                label: tache.label,
                actioned: tache.actioned,
            })
            .collect(),
        warnings: miroir.warnings,
        // Les deux traversent la conversion sans être consommés par elle. La
        // porte les lit **avant**, pour sa propre décision, mais
        // `format_preview` les lit en premier (`rewind.rs:742-748`) : les
        // laisser passer n'est donc pas une redite, c'est ce qui permet au
        // `BLOCKED: …` d'être rendu si un appelant formate un aperçu bloqué.
        blocked: miroir.blocked,
        block_reason: miroir.block_reason,
    })
}

/// L'aperçu d'un rembobinage, sous la forme du domaine, pour que l'appelant
/// l'affiche par [`mika_agent::rewind::format_preview`] et rien d'autre.
///
/// Un refus du serveur est un `Blocked` et non un `Ok` portant un aperçu dont
/// `blocked` serait vrai : `format_preview` rend alors `BLOCKED: …`, et ce texte
/// en position de succès se lirait comme un aperçu — l'opérateur validerait un
/// rembobinage que le serveur a déjà refusé.
pub(crate) async fn preview(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
    after_message_id: i64,
) -> Result<RewindPreview, RewindRemoteIssue> {
    // `agent` part **toujours**, y compris vide. Le serveur le déclare
    // `#[serde(default)]` (`server/rewind.rs:14-15`) : un champ absent lui fait
    // résoudre *son* agent par défaut, qui n'est pas celui de la TUI. L'aperçu
    // rendu serait celui d'un autre fil que celui qu'on croit viser — et comme
    // `execute` porte le même corps, l'omission appliquerait un effet
    // destructeur sur la mauvaise base. Le champ présent et vide dit au serveur
    // la même chose, mais il le dit, et T8 le lit sur la socket.
    let envoye = serde_json::json!({
        "session_id": session_id,
        "after_message_id": after_message_id,
        "agent": agent,
    });
    let (statut, corps) = post(base_url, token, "preview", &envoye).await?;

    // Une forme illisible est `Unexpected`, jamais `Blocked`. Un blocage est une
    // décision du serveur ; une forme illisible est une incapacité du client, et
    // l'ordinaire de sa cause est un écart de version (rien ne tient les miroirs
    // en lockstep avec `server/rewind.rs`). Les confondre afficherait
    // « mika-spirit refuses the rewind » là où mika-spirit a dit oui.
    // `Unexpected` est aussi le seul bras qui porte à la fois le statut, l'URL et
    // le texte reçu — les trois choses qu'un écart de version demande.
    let miroir: PreviewMirror =
        serde_json::from_str(&corps).map_err(|cause| RewindRemoteIssue::Unexpected {
            status: Some(statut),
            url: rewind_url(base_url, "preview"),
            body: format!("unreadable rewind preview ({cause}): {corps}"),
        })?;

    // Le 422 **et** le 200 dont `blocked` est vrai. Le second n'est pas produit
    // par ce serveur, qui choisit son statut sur `resp.blocked`
    // (`server/rewind.rs:235-243`) : la branche couvre une incohérence serveur,
    // et c'est le champ qui décide, non le statut, parce que propager un aperçu
    // bloqué en `Ok` rendrait `BLOCKED: …` en position de succès.
    if statut == 422 || miroir.blocked {
        // Le repli nomme ce qui s'est passé plutôt qu'il n'invente une cause. Le
        // domaine remplit toujours `block_reason` avec `blocked`, mais le type
        // autorise l'absence, et `Display` rendrait alors « mika-spirit refuses
        // the rewind: » — une phrase coupée là où l'opérateur attend la cause.
        let reason = miroir
            .block_reason
            .clone()
            .unwrap_or_else(|| "mika-spirit blocked the rewind without saying why".to_string());
        // L'aperçu est best-effort ; le blocage ne l'est pas. Si le corps du
        // refus porte une action inconnue, rendre `Unexpected` perdrait la
        // **raison du refus** — l'information que l'opérateur est venu chercher
        // — pour un diagnostic de version qu'il n'a pas demandé.
        return Err(RewindRemoteIssue::Blocked {
            reason,
            preview: preview_to_domain(miroir).ok().map(Box::new),
        });
    }

    // Un succès dont la conversion échoue n'a pas de raison de refus à préserver,
    // donc il dit l'écart de version au lieu de le taire.
    preview_to_domain(miroir).map_err(|cause| RewindRemoteIssue::Unexpected {
        status: Some(statut),
        url: rewind_url(base_url, "preview"),
        body: cause,
    })
}

/// Le rembobinage **appliqué** sur mika-spirit, et son résultat tel que le
/// serveur l'a écrit.
///
/// Rendu en `ResultMirror` et non converti vers `mika_agent::rewind::RewindResult`,
/// parce qu'il n'existe aucun formateur de domaine pour un résultat — `rewind.rs`
/// n'expose que `format_preview`, et la mise en forme vit en ligne chez
/// l'appelant, dans un fichier que ce vol n'ouvre pas. V2c-1b a tranché en
/// gardant ce miroir : le rendu reste en ligne chez l'appelant, là où il était
/// déjà, et aucun des huit champs n'est recopié vers un type intermédiaire.
///
/// `Option<&str>` pour l'origine : l'appelant tient son `session_id` d'origine
/// dans son propre état et n'a pas à l'allouer pour le prêter.
pub(crate) async fn execute(
    base_url: &str,
    token: &str,
    agent: &str,
    session_id: &str,
    after_message_id: i64,
    originating_session_id: Option<&str>,
) -> Result<ResultMirror, RewindRemoteIssue> {
    // `agent` part **toujours**, pour la raison écrite sur [`preview`] — et ici
    // elle est d'un cran plus grave : `handle_rewind_execute` résout l'agent en
    // *première* instruction, avant même de valider la requête, donc un champ
    // absent ne ferait pas afficher le mauvais aperçu, il **appliquerait** le
    // rembobinage sur la base d'un autre fil.
    let mut envoye = serde_json::json!({
        "session_id": session_id,
        "after_message_id": after_message_id,
        "agent": agent,
    });
    // Une clé **absente**, et non un `null` — d'où le `mut`, `json!` ne sachant
    // pas omettre. Les deux se valent aujourd'hui (le champ est `Option<String>`
    // avec `#[serde(default)]` côté serveur), mais le jour où il deviendrait
    // `String` + `default` — la forme que `agent` porte déjà trois lignes plus
    // haut — l'absence prendrait le défaut tandis que le `null` ferait échouer la
    // désérialisation **entière**, donc le rembobinage. L'absence est strictement
    // plus robuste, pas seulement plus propre.
    if let Some(origine) = originating_session_id {
        envoye["originating_session_id"] = serde_json::Value::String(origine.to_string());
    }
    let (statut, corps) = post(base_url, token, "execute", &envoye).await?;

    // Le statut est lu **avant** la forme, et c'est le 422 qui passe en premier :
    // `post` rend 200 *et* 422 sur son chemin `Ok`, donc tenter `ResultMirror`
    // d'abord ferait sortir un 422 — dont le corps n'a aucun des huit champs — en
    // `Unexpected`, et un refus du serveur se lirait comme un écart de version.
    if statut == 422 {
        // Un 422 est `Blocked` quelle que soit la lisibilité de son corps, et
        // c'est l'inverse de la conduite de `preview` ci-dessus, pour la règle
        // qu'elle énonce : la forme du corps renseigne sur le *client*, le statut
        // 422 est une décision du *serveur*.
        let reason = serde_json::from_str::<ErrorMirror>(&corps)
            .map(|miroir| miroir.error)
            // Le repli nomme ce qui s'est passé et **cite le corps reçu**, seul
            // matériau d'un diagnostic. Il ne se mutualise pas avec celui de
            // `preview` : là-bas le corps est parfaitement lisible et c'est
            // `block_reason` qui manque (le serveur a refusé sans dire pourquoi) ;
            // ici le client ne sait pas lire le corps du tout (écart de version).
            // Deux populations, deux remèdes — élargir le domaine contre
            // réaligner les versions.
            .unwrap_or_else(|_| format!("the 422 body carries no readable reason: {corps}"));
        // `preview: None` : le 422 d'`execute` ne porte aucun aperçu (F5), et en
        // fabriquer un serait une invention.
        return Err(RewindRemoteIssue::Blocked {
            reason,
            preview: None,
        });
    }

    // Un 200 illisible n'a aucun refus à préserver, lui : le taire rendrait un
    // résultat vide en position de succès.
    serde_json::from_str(&corps).map_err(|cause| RewindRemoteIssue::Unexpected {
        status: Some(statut),
        url: rewind_url(base_url, "execute"),
        body: format!("unreadable rewind result ({cause}): {corps}"),
    })
}

// Les tests vivent ici, **en fin de fichier** : le scanner d'egress coupe la
// production au premier `#[cfg(test)]`, et un item d'après en sortirait.
//
// `pub(crate)` sur le module et sur le factice : depuis V2c-1b les tests de
// `tui/commands/handlers.rs` visent le même serveur de test que ceux d'ici,
// plutôt qu'une seconde copie libre d'en diverger. Le module reste sous
// `#[cfg(test)]`, donc rien de cela n'existe en production. Précédents du même
// geste : `mika-agent/src/grooming_marker.rs`, `db.rs`,
// `server/webhook_prefilter.rs`.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::{Request, State};
    use axum::http::StatusCode;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Ce que le factice a vu sur la socket : le témoin est ce qui fait du
    /// `Bearer` et du corps intact des mesures, non des relectures du code.
    pub(crate) struct RequeteRecue {
        pub(crate) methode: String,
        pub(crate) chemin: String,
        pub(crate) autorisation: Option<String>,
        pub(crate) corps: String,
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
    pub(crate) async fn fake_spirit(
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
        // La fixture garde `trace_ids`, que le serveur envoie toujours et que le
        // miroir ne déclare plus : la lecture au-dessus atteste que `serde` le
        // laisse passer au lieu d'échouer sur un champ non déclaré.

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

    #[test]
    fn mika1727_les_trois_graphies_du_serveur_donnent_les_trois_actions() {
        // Une par une, et non une seule : trois bras distincts, trois icônes
        // distinctes dans `format_preview`. Un seul cas laisserait passer une
        // table **permutée**, l'erreur la plus probable à l'écriture.
        for (graphie, attendue) in [
            ("Restore", ReversalAction::Restore),
            ("Delete", ReversalAction::Delete),
            ("Skip", ReversalAction::Skip),
        ] {
            let rendue = reversal_action_from_label(graphie)
                .unwrap_or_else(|erreur| panic!("{graphie} est une graphie du serveur : {erreur}"));
            assert_eq!(rendue, attendue, "graphie {graphie}");
        }
    }

    #[test]
    fn mika1727_une_action_inconnue_est_refusee_en_nommant_la_valeur_recue() {
        // `expect_err` **est** la mesure du non-repli : une table qui se
        // replierait sur `Skip` rendrait un `Ok` et rougirait ici.
        let message = reversal_action_from_label("Rollback")
            .expect_err("une quatrième graphie n'a aucun repli");
        assert!(message.contains("Rollback"), "{message}");
        // Les trois graphies acceptées rendent le message exploitable, et non
        // seulement présent : c'est ce qui dit à l'opérateur ce qu'il a lu.
        for graphie in ["Restore", "Delete", "Skip"] {
            assert!(message.contains(graphie), "{message}");
        }
    }

    /// Un miroir à **deux** reversals, dont le second porte la graphie passée :
    /// c'est ce qui permet à T4 de mesurer que l'aperçu entier échoue, et non
    /// qu'il revient avec un reversal de moins.
    fn un_miroir(action_du_second: &str) -> PreviewMirror {
        let reversal = |id: i64, action: &str| ReversalMirror {
            audit_event_id: id,
            tool_name: "store_fact".to_string(),
            target_key: format!("people:vincent:{id}"),
            action: action.to_string(),
            description: format!("change mémoire n°{id}"),
        };
        PreviewMirror {
            messages: vec![
                MessageMirror {
                    id: 41,
                    role: "user".to_string(),
                    content_snippet: "pose la question".to_string(),
                },
                MessageMirror {
                    id: 42,
                    role: "assistant".to_string(),
                    content_snippet: "y répond".to_string(),
                },
            ],
            reversals: vec![reversal(7, "Restore"), reversal(8, action_du_second)],
            tasks: vec![TaskMirror {
                task_id: "t-1".to_string(),
                label: "relire le plan".to_string(),
                actioned: true,
            }],
            warnings: vec!["un avertissement".to_string()],
            blocked: false,
            block_reason: None,
        }
    }

    #[test]
    fn mika1727_la_conversion_rend_tout_lapercu_du_fil() {
        let apercu = preview_to_domain(un_miroir("Delete")).expect("deux graphies du serveur");

        assert_eq!(apercu.messages.len(), 2);
        assert_eq!(apercu.messages[0].id, 41);
        assert_eq!(apercu.messages[0].role, "user");
        assert_eq!(apercu.messages[0].content_snippet, "pose la question");
        // `trace_id` à `None` sur **chaque** message : le fil ne le porte pas, et
        // c'est la seule divergence volontaire de la conversion.
        for message in &apercu.messages {
            assert!(message.trace_id.is_none(), "{:?}", message.trace_id);
        }

        assert_eq!(apercu.reversals.len(), 2);
        assert_eq!(apercu.reversals[0].audit_event_id, 7);
        assert_eq!(apercu.reversals[0].tool_name, "store_fact");
        assert_eq!(apercu.reversals[0].target_key, "people:vincent:7");
        assert_eq!(apercu.reversals[0].action, ReversalAction::Restore);
        assert_eq!(apercu.reversals[0].description, "change mémoire n°7");
        assert_eq!(apercu.reversals[1].action, ReversalAction::Delete);

        assert_eq!(apercu.tasks.len(), 1);
        assert_eq!(apercu.tasks[0].task_id, "t-1");
        assert_eq!(apercu.tasks[0].label, "relire le plan");
        assert!(apercu.tasks[0].actioned);

        assert_eq!(apercu.warnings, vec!["un avertissement".to_string()]);
        assert!(!apercu.blocked);
        assert!(apercu.block_reason.is_none());

        // `blocked` et `block_reason` traversent bien la conversion, mesuré sur
        // le bras qui les porte : sans ce second appel, deux champs passeraient
        // la conversion sans jamais être lus à autre chose que leur défaut.
        let mut bloque = un_miroir("Skip");
        bloque.blocked = true;
        bloque.block_reason = Some("un verrou tient l'agent".to_string());
        let bloque = preview_to_domain(bloque).expect("un aperçu bloqué se convertit aussi");
        assert!(bloque.blocked);
        assert_eq!(
            bloque.block_reason.as_deref(),
            Some("un verrou tient l'agent")
        );
    }

    #[test]
    fn mika1727_une_action_inconnue_fait_echouer_lapercu_entier() {
        // Le miroir porte **deux** reversals dont un seul est illisible, et c'est
        // ce qui fait de l'`expect_err` une mesure : la variante qui saute le
        // reversal inconnu rendrait un `Ok` à un reversal de moins — un aperçu
        // *plus court que la réalité*, où l'opérateur lirait « 1 changement
        // mémoire » pour deux. Sur un miroir à un seul reversal, les deux
        // conduites rendraient un résultat également vide de ce reversal.
        let message = preview_to_domain(un_miroir("Rollback"))
            .expect_err("une action inconnue n'est pas sautée");
        assert!(message.contains("Rollback"), "{message}");
    }

    /// Le couple *(corps que le factice sert, aperçu du domaine attendu)* pour
    /// une action. L'attendu est bâti **à la main**, et non par
    /// `preview_to_domain` : passer par la conversion ferait comparer le résultat
    /// à lui-même, et les trois tests seraient verts sur une table permutée.
    fn un_apercu(graphie: &str, action: ReversalAction) -> (String, RewindPreview) {
        let servi = serde_json::json!({
            "messages": [{"id": 7, "role": "user", "content_snippet": "la demande"}],
            "reversals": [{
                "audit_event_id": 3,
                "tool_name": "store_fact",
                "target_key": "people:vincent",
                "action": graphie,
                "description": "un fait posé",
            }],
            "tasks": [{"task_id": "t-9", "label": "une tâche", "actioned": false}],
            "warnings": ["un avertissement"],
            "blocked": false,
            "block_reason": null,
        });
        let attendu = RewindPreview {
            messages: vec![MessagePreview {
                id: 7,
                role: "user".to_string(),
                content_snippet: "la demande".to_string(),
                trace_id: None,
            }],
            reversals: vec![ReversalPreview {
                audit_event_id: 3,
                tool_name: "store_fact".to_string(),
                target_key: "people:vincent".to_string(),
                action,
                description: "un fait posé".to_string(),
            }],
            tasks: vec![TaskPreview {
                task_id: "t-9".to_string(),
                label: "une tâche".to_string(),
                actioned: false,
            }],
            warnings: vec!["un avertissement".to_string()],
            blocked: false,
            block_reason: None,
        };
        (servi.to_string(), attendu)
    }

    /// La porte appelée contre un factice qui sert `statut`/`corps` sur
    /// `preview`, et le témoin de ce qu'il a vu sur la socket.
    async fn par_la_porte(
        statut: StatusCode,
        corps: &str,
    ) -> (
        Result<RewindPreview, RewindRemoteIssue>,
        Arc<Mutex<Vec<RequeteRecue>>>,
    ) {
        let (base, recues) = fake_spirit(&[("/api/v1/rewind/preview", statut, corps)]).await;
        let rendu = preview(&base, "jeton", "mika-dev", "s-1", 42).await;
        (rendu, recues)
    }

    /// T5-T7 : l'égalité du critère 1 se mesure par `format_preview`, parce
    /// qu'aucun des quatre types d'aperçu ne dérive `PartialEq` — le texte est
    /// la seule égalité disponible, et c'est aussi la seule qui compte ici,
    /// puisque c'est ce texte que l'opérateur lit.
    async fn la_porte_rend_laction(graphie: &str, action: ReversalAction) {
        let (servi, attendu) = un_apercu(graphie, action);
        let (rendu, _) = par_la_porte(StatusCode::OK, &servi).await;
        let rendu = rendu.unwrap_or_else(|erreur| panic!("{graphie} passe la porte : {erreur}"));
        assert_eq!(
            mika_agent::rewind::format_preview(&rendu),
            mika_agent::rewind::format_preview(&attendu),
            "graphie {graphie}"
        );
    }

    #[tokio::test]
    async fn mika1727_la_porte_rend_un_apercu_restore() {
        la_porte_rend_laction("Restore", ReversalAction::Restore).await;
    }

    #[tokio::test]
    async fn mika1727_la_porte_rend_un_apercu_delete() {
        la_porte_rend_laction("Delete", ReversalAction::Delete).await;
    }

    #[tokio::test]
    async fn mika1727_la_porte_rend_un_apercu_skip() {
        la_porte_rend_laction("Skip", ReversalAction::Skip).await;
    }

    #[tokio::test]
    async fn mika1727_le_corps_de_la_porte_porte_ses_trois_champs() {
        let (servi, _) = un_apercu("Restore", ReversalAction::Restore);
        let (rendu, recues) = par_la_porte(StatusCode::OK, &servi).await;
        rendu.expect("le factice sert un aperçu lisible");

        let vues = recues.lock().unwrap();
        let corps: serde_json::Value =
            serde_json::from_str(&vues[0].corps).expect("le corps envoyé est du JSON");
        // Lu sur la socket, et non sur la valeur passée à `post` : c'est la
        // seule lecture qui mesure ce que le serveur recevra.
        assert_eq!(corps["session_id"], "s-1");
        assert_eq!(corps["after_message_id"], 42);
        assert_eq!(corps["agent"], "mika-dev");
    }

    #[tokio::test]
    async fn mika1727_un_422_est_un_blocage_avec_sa_raison_et_son_apercu() {
        let (servi, _) = un_apercu("Restore", ReversalAction::Restore);
        let mut servi: serde_json::Value = serde_json::from_str(&servi).unwrap();
        servi["blocked"] = serde_json::json!(true);
        servi["block_reason"] = serde_json::json!("un verrou tient l'agent");
        let (rendu, _) = par_la_porte(StatusCode::UNPROCESSABLE_ENTITY, &servi.to_string()).await;

        match rendu.expect_err("un 422 est un refus du serveur") {
            RewindRemoteIssue::Blocked { reason, preview } => {
                // Égale, et non `contains` : la raison du serveur arrive à
                // l'opérateur telle qu'elle a été écrite.
                assert_eq!(reason, "un verrou tient l'agent");
                assert!(preview.is_some(), "l'aperçu du refus est lisible");
            }
            autre => panic!("{autre:?}"),
        }
    }

    #[tokio::test]
    async fn mika1727_un_200_dont_blocked_est_vrai_est_aussi_un_blocage() {
        // Ce serveur ne produit pas cette forme — il choisit son statut sur
        // `resp.blocked`. C'est une incohérence serveur, et la propager en `Ok`
        // ferait rendre `BLOCKED: …` en position de succès.
        let (servi, _) = un_apercu("Restore", ReversalAction::Restore);
        let mut servi: serde_json::Value = serde_json::from_str(&servi).unwrap();
        servi["blocked"] = serde_json::json!(true);
        servi["block_reason"] = serde_json::json!("incohérent, mais dit");
        let (rendu, _) = par_la_porte(StatusCode::OK, &servi.to_string()).await;

        match rendu.expect_err("un aperçu bloqué n'est pas un succès") {
            RewindRemoteIssue::Blocked { reason, .. } => {
                assert_eq!(reason, "incohérent, mais dit");
            }
            autre => panic!("{autre:?}"),
        }
    }

    #[tokio::test]
    async fn mika1727_une_action_inconnue_par_la_porte_ne_rend_aucune_icone() {
        let (servi, _) = un_apercu("Rollback", ReversalAction::Skip);
        let (rendu, _) = par_la_porte(StatusCode::OK, &servi).await;
        let erreur = rendu.expect_err("une action inconnue n'est pas un succès");
        let message = erreur.to_string();
        assert!(message.contains("Rollback"), "{message}");
        // La seconde moitié du critère 2, et elle ne se mesure que par la porte :
        // les trois icônes, non `⏭` seule, parce qu'un repli sur `Restore` ou
        // `Delete` est tout aussi possible à l'écriture.
        for icone in ["↩", "🗑", "⏭"] {
            assert!(!message.contains(icone), "{icone} dans {message}");
        }
        assert!(
            matches!(erreur, RewindRemoteIssue::Unexpected { .. }),
            "{erreur:?}"
        );
    }

    #[tokio::test]
    async fn mika1727_un_422_garde_sa_raison_quand_son_apercu_est_illisible() {
        let (servi, _) = un_apercu("Rollback", ReversalAction::Skip);
        let mut servi: serde_json::Value = serde_json::from_str(&servi).unwrap();
        servi["block_reason"] = serde_json::json!("la session est verrouillée");
        let (rendu, _) = par_la_porte(StatusCode::UNPROCESSABLE_ENTITY, &servi.to_string()).await;

        match rendu.expect_err("un 422 reste un refus") {
            // La raison survit à l'échec de la conversion : c'est elle que
            // l'opérateur est venu chercher, pas le diagnostic de version.
            RewindRemoteIssue::Blocked { reason, preview } => {
                assert_eq!(reason, "la session est verrouillée");
                assert!(preview.is_none(), "l'aperçu, lui, est perdu");
            }
            autre => panic!("{autre:?}"),
        }
    }

    #[tokio::test]
    async fn mika1727_un_422_sans_raison_porte_un_repli_nomme() {
        let (servi, _) = un_apercu("Restore", ReversalAction::Restore);
        let (rendu, _) = par_la_porte(StatusCode::UNPROCESSABLE_ENTITY, &servi).await;

        match rendu.expect_err("le statut suffit à faire le refus") {
            RewindRemoteIssue::Blocked { reason, .. } => {
                // Non vide : `Display` rendrait sinon « refuses the rewind: »,
                // une phrase coupée à l'endroit où la cause est attendue.
                assert!(reason.contains("without saying why"), "{reason}");
            }
            autre => panic!("{autre:?}"),
        }
    }

    /// Un résultat à huit champs, tous distincts : cinq compteurs permutés
    /// entre eux resteraient verts sur des valeurs égales.
    const UN_RESULTAT: &str = r#"{"messages_deleted":2,"reversals_applied":3,
        "reversals_skipped":1,"tasks_deleted":4,"tasks_cancelled":5,
        "warnings":["un avertissement"],"reversal_descriptions":["restaure la fiche"],
        "rewind_trace_id":"rw-7"}"#;

    /// La porte `execute` contre un factice qui sert `statut`/`corps` sur
    /// `execute`, et le témoin de ce qu'il a vu sur la socket. Frère de
    /// [`par_la_porte`], spécifique à `preview` et qui rend un aperçu. L'origine
    /// est un paramètre parce que deux tests mesurent la clé qu'elle fait
    /// apparaître — ou pas.
    async fn par_la_porte_execute(
        statut: StatusCode,
        corps: &str,
        origine: Option<&str>,
    ) -> (
        Result<ResultMirror, RewindRemoteIssue>,
        Arc<Mutex<Vec<RequeteRecue>>>,
    ) {
        let (base, recues) = fake_spirit(&[("/api/v1/rewind/execute", statut, corps)]).await;
        let rendu = execute(&base, "jeton", "mika-dev", "s-1", 42, origine).await;
        (rendu, recues)
    }

    #[tokio::test]
    async fn mika1727_la_porte_execute_rend_le_resultat_servi() {
        let (rendu, _) = par_la_porte_execute(StatusCode::OK, UN_RESULTAT, None).await;
        let resultat =
            rendu.unwrap_or_else(|erreur| panic!("un 200 lisible passe la porte : {erreur}"));
        // Les **huit** champs un à un : ni `ResultMirror` ni `RewindResult` ne
        // dérive `PartialEq`, donc l'énumération est la seule mesure disponible
        // — et c'est aussi la seule qui dit *lequel* a dérivé. Ce test ne double
        // pas `mika1727_les_miroirs_se_lisent_depuis_la_forme_du_serveur`, qui
        // appelle `from_str` en direct et mesure serde : si la porte régressait
        // vers `Unexpected`, celui-là resterait vert.
        assert_eq!(resultat.messages_deleted, 2);
        assert_eq!(resultat.reversals_applied, 3);
        assert_eq!(resultat.reversals_skipped, 1);
        assert_eq!(resultat.tasks_deleted, 4);
        assert_eq!(resultat.tasks_cancelled, 5);
        assert_eq!(resultat.warnings, vec!["un avertissement".to_string()]);
        assert_eq!(
            resultat.reversal_descriptions,
            vec!["restaure la fiche".to_string()]
        );
        assert_eq!(resultat.rewind_trace_id, "rw-7");
    }

    #[tokio::test]
    async fn mika1727_le_corps_dexecute_porte_son_origine_quand_elle_existe() {
        let (rendu, recues) =
            par_la_porte_execute(StatusCode::OK, UN_RESULTAT, Some("sess-b")).await;
        rendu.expect("le factice sert un résultat lisible");

        let vues = recues.lock().unwrap();
        let corps: serde_json::Value =
            serde_json::from_str(&vues[0].corps).expect("le corps envoyé est du JSON");
        // Lu sur la socket, et non sur la valeur passée à `post` : la seule
        // lecture qui mesure ce que le serveur recevra.
        assert_eq!(corps["session_id"], "s-1");
        assert_eq!(corps["after_message_id"], 42);
        assert_eq!(corps["agent"], "mika-dev");
        assert_eq!(corps["originating_session_id"], "sess-b");
    }

    #[tokio::test]
    async fn mika1727_sans_origine_la_cle_dorigine_est_absente_et_non_nulle() {
        let (rendu, recues) = par_la_porte_execute(StatusCode::OK, UN_RESULTAT, None).await;
        rendu.expect("le factice sert un résultat lisible");

        let vues = recues.lock().unwrap();
        let corps: serde_json::Value =
            serde_json::from_str(&vues[0].corps).expect("le corps envoyé est du JSON");
        // `is_none()` et non `!= ""` : un `null` passerait un test sur la valeur,
        // et c'est exactement la forme que la clé absente existe pour éviter.
        assert!(corps.get("originating_session_id").is_none(), "{corps}");
        // Les trois autres toujours là : ce test et son frère ci-dessus sont une
        // paire, et ni l'un ni l'autre ne suffit. Celui-là seul passerait sur une
        // porte qui n'écrit **jamais** la clé — c'est-à-dire qui perd en silence
        // le rembobinage inter-sessions ; l'autre seul passerait sur une porte
        // qui l'écrit toujours, en `null` quand l'origine est absente.
        assert_eq!(corps["session_id"], "s-1");
        assert_eq!(corps["after_message_id"], 42);
        assert_eq!(corps["agent"], "mika-dev");
    }

    #[tokio::test]
    async fn mika1727_un_422_dexecute_rend_la_raison_du_serveur() {
        const CORPS: &str = r#"{"error":"Rewind blocked: a task holds the lock"}"#;
        let (rendu, _) = par_la_porte_execute(StatusCode::UNPROCESSABLE_ENTITY, CORPS, None).await;

        match rendu.expect_err("un 422 est un refus du serveur") {
            RewindRemoteIssue::Blocked { reason, preview } => {
                // Égale, et non `contains` : la raison arrive à l'opérateur telle
                // que le serveur l'a écrite.
                assert_eq!(reason, "Rewind blocked: a task holds the lock");
                // Le 422 d'`execute` ne porte aucun aperçu, contrairement à celui
                // de `preview` : même refus, autre forme.
                assert!(preview.is_none());
            }
            autre => panic!("{autre:?}"),
        }
    }

    #[tokio::test]
    async fn mika1727_un_422_dexecute_illisible_reste_un_refus_et_cite_son_corps() {
        // La clé `error` renommée : l'écart de version, et non un corps absurde.
        const CORPS: &str = r#"{"message":"refusé, mais sous une autre clé"}"#;
        let (rendu, _) = par_la_porte_execute(StatusCode::UNPROCESSABLE_ENTITY, CORPS, None).await;

        // Le témoin négatif du test précédent, et le seul qui mesure le critère
        // en entier : si le bras 422 se repliait sur `Unexpected`, l'autre
        // resterait vert — son corps est lisible — et seul celui-ci rougirait.
        match rendu.expect_err("un 422 reste un refus") {
            RewindRemoteIssue::Blocked { reason, preview } => {
                assert!(reason.contains("no readable reason"), "{reason}");
                // Le corps reçu est cité : le seul matériau d'un diagnostic.
                assert!(reason.contains("une autre clé"), "{reason}");
                assert!(preview.is_none());
            }
            autre => panic!("{autre:?}"),
        }
    }

    #[tokio::test]
    async fn mika1727_un_200_dexecute_illisible_porte_son_statut() {
        // Un objet JSON **valide mais tronqué**, et non « pas du json » : aucun
        // des huit champs ne porte `#[serde(default)]`, donc c'est là la forme
        // réelle de l'écart de version — un serveur qui retire ou renomme un
        // champ —, et la seule des deux qu'une lecture en `Value` à défauts
        // laisserait passer, en rendant un résultat à sept zéros en position de
        // succès.
        let (rendu, _) =
            par_la_porte_execute(StatusCode::OK, r#"{"messages_deleted":3}"#, None).await;
        let erreur = rendu.expect_err("un corps tronqué n'est pas un succès");
        // Le statut est lu *dans* l'`Unexpected` : sans statut, le message dit
        // « injoignable » — une autre cause, et un autre geste pour l'opérateur.
        assert!(
            matches!(
                erreur,
                RewindRemoteIssue::Unexpected {
                    status: Some(200),
                    ..
                }
            ),
            "{erreur:?}"
        );
        let message = erreur.to_string();
        assert!(message.contains("/api/v1/rewind/execute"), "{message}");
    }

    /// Un ancrage tel que le serveur l'écrit, `trace_ids` compris — que le
    /// miroir ne déclare plus depuis V2c-1b, et que `serde` laisse passer.
    const UN_ANCRAGE: &str =
        r#"{"session_id":"s-1","after_message_id":40,"trace_ids":["tr-1","tr-2"]}"#;

    /// La porte `resolve` contre un factice qui sert `statut`/`corps` sur
    /// `resolve`, et le témoin de ce qu'il a vu sur la socket. `count` est un
    /// paramètre parce qu'un test le mesure sur le corps envoyé.
    async fn par_la_porte_resolve(
        statut: StatusCode,
        corps: &str,
        count: usize,
    ) -> (
        Result<ResolveMirror, RewindRemoteIssue>,
        Arc<Mutex<Vec<RequeteRecue>>>,
    ) {
        let (base, recues) = fake_spirit(&[("/api/v1/rewind/resolve", statut, corps)]).await;
        let rendu = resolve(&base, "jeton", "mika-dev", "s-1", count).await;
        (rendu, recues)
    }

    #[tokio::test]
    async fn mika1727_la_porte_resolve_rend_lancrage_servi() {
        let (rendu, _) = par_la_porte_resolve(StatusCode::OK, UN_ANCRAGE, 1).await;
        let ancrage =
            rendu.unwrap_or_else(|erreur| panic!("un 200 lisible passe la porte : {erreur}"));
        // Les deux champs un à un : `ResolveMirror` ne dérive pas `PartialEq`,
        // et l'énumération est seule à dire *lequel* a dérivé.
        assert_eq!(ancrage.session_id, "s-1");
        assert_eq!(ancrage.after_message_id, 40);
    }

    #[tokio::test]
    async fn mika1727_le_corps_de_resolve_porte_son_compte_tel_quel() {
        // `3` et non `1` : le serveur défaute `count` à 1, donc une clé omise
        // resterait verte sur la valeur par défaut.
        let (rendu, recues) = par_la_porte_resolve(StatusCode::OK, UN_ANCRAGE, 3).await;
        rendu.expect("le factice sert un ancrage lisible");

        let vues = recues.lock().unwrap();
        let corps: serde_json::Value =
            serde_json::from_str(&vues[0].corps).expect("le corps envoyé est du JSON");
        // Lu sur la socket, et non sur la valeur passée à `post` : la seule
        // lecture qui mesure ce que le serveur recevra.
        assert_eq!(corps["session_id"], "s-1");
        assert_eq!(corps["agent"], "mika-dev");
        assert_eq!(corps["count"], 3);
    }

    #[tokio::test]
    async fn mika1727_un_422_de_resolve_est_un_ecart_et_non_un_refus() {
        // `handle_rewind_resolve` n'émet aucun 422 ; `post` en rend pourtant un à
        // l'appelant sur son chemin `Ok`. C'est donc l'incohérence serveur que ce
        // test mesure, et la porte doit la nommer telle quelle.
        let (rendu, _) =
            par_la_porte_resolve(StatusCode::UNPROCESSABLE_ENTITY, r#"{"error":"refusé"}"#, 1)
                .await;

        let erreur = rendu.expect_err("un 422 ne passe pas la porte");
        assert!(
            matches!(
                erreur,
                RewindRemoteIssue::Unexpected {
                    status: Some(422),
                    ..
                }
            ),
            "{erreur:?}"
        );
        // La seconde moitié du critère, et pas une redite : un corps portant la
        // clé `error` est exactement celui qu'un bras `Blocked` copié depuis
        // `execute` saurait lire, donc l'assertion de forme ci-dessus ne suffit
        // pas à dire que ce bras n'existe pas.
        assert!(
            !matches!(erreur, RewindRemoteIssue::Blocked { .. }),
            "{erreur:?}"
        );
        let message = erreur.to_string();
        assert!(message.contains("422"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_200_de_resolve_illisible_porte_son_statut() {
        // Un objet JSON **valide mais tronqué**, et non « pas du json » : aucun
        // des trois champs ne porte `#[serde(default)]`, donc c'est là la forme
        // réelle de l'écart de version, et la seule qu'une lecture à défauts
        // laisserait passer — en rendant un ancrage à `after_message_id` nul, qui
        // rembobinerait le fil entier.
        let (rendu, _) = par_la_porte_resolve(StatusCode::OK, r#"{"session_id":"s-1"}"#, 1).await;
        let erreur = rendu.expect_err("un corps tronqué n'est pas un succès");
        // Le statut est lu *dans* l'`Unexpected` : sans statut, le message dit
        // « injoignable » — une autre cause, et un autre geste pour l'opérateur.
        assert!(
            matches!(
                erreur,
                RewindRemoteIssue::Unexpected {
                    status: Some(200),
                    ..
                }
            ),
            "{erreur:?}"
        );
        let message = erreur.to_string();
        assert!(message.contains("/api/v1/rewind/resolve"), "{message}");
    }
}
