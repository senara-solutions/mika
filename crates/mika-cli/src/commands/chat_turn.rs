//! L'unique site par lequel un tour de la TUI atteint mika-spirit (mika#1727, V1).
//!
//! # Pourquoi un module, et pas une fonction dans `chat.rs`
//!
//! Le worker de la TUI vit dans `chat::spawn_agent_worker`, qui exige une base
//! ouverte, une identité sur disque et une session ouverte sur spirit. L'envoi
//! est donc extrait ici, au périmètre, avec ses tests en ligne : c'est la forme
//! où « le tour part bien sur le fil, avec la session du tour » est une
//! propriété mesurée plutôt qu'une lecture à l'œil.
//!
//! Le bras du worker, lui, **est** atteignable depuis le vol V3c-2c : l'adresse
//! de spirit arrive par argument jusqu'ici, donc un test monte le worker entier
//! contre un factice — c'est l'écart É1 du plan de V3b-1, mesuré par
//! `chat::tests::mika1727_v3c2c_apres_un_setmodel_le_tour_suivant_porte_loverride`.
//! Les deux mesures ne se remplacent pas : celle d'ici tient le contrat de fil
//! d'un envoi, celle de là-bas tient l'ordre des tours d'un même worker.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne rend rien à l'écran et ne connaît pas l'`App` : il rend un
//! [`TurnReply`] que `chat.rs` convertit en `AgentResponse`. Il n'a pas non plus
//! de décodeur à lui — les jetons passent par
//! [`mika_a2a::params::attested_run_usage`], le lecteur que les deux portes de
//! `mika ask` partagent déjà (mika#1883). Une seconde lecture de la même clé est
//! comment deux surfaces finissent par répondre différemment à la même question.

use crate::tui::app::AgentResponse;
use anyhow::Result;
use mika_agent::async_db::AsyncDatabase;

/// Ce qu'un tour rendu par mika-spirit rapporte à la TUI.
///
/// `input_tokens` est `Option` et le reste : un serveur qui n'a rien attesté
/// n'a pas dépensé zéro jeton, il n'a rien dit. Rendre `0` serait
/// indiscernable d'un tour réel et c'est la confusion que mika#2331 nomme.
#[derive(Debug)]
pub(crate) struct TurnReply {
    pub content: String,
    pub input_tokens: Option<u32>,
}

/// L'endpoint A2A du mika-spirit local pour cet agent.
///
/// Même forme que `commands::ask` (`{spirit_url}/a2a/{agent}`), et pour la même
/// raison : c'est l'adresse que `budget_floor::attested_envelope_secs` sait
/// reconnaître comme locale, donc celle qui obtient l'enveloppe d'envoi
/// attestée par agent (mika#2759).
///
/// `base_url` arrive **par argument** depuis le vol V3c-2c, là où cette
/// fonction lisait `dashboard::spirit_url()`. C'est ce qui rend le bras du
/// worker mesurable : un test ne peut pas poser `MIKA_SPIRIT_URL` sans
/// `unsafe`, ni sans course avec les tests parallèles de `cargo test`.
///
/// L'adresse n'est pas normalisée au passage — pas de `trim_end_matches('/')`,
/// contrairement à `skills_remote::lire`. Ce vol déplace la lecture, il ne
/// change pas ce qu'elle rend : une barre finale compose ici la même URL
/// qu'avant le déplacement, et la corriger maintenant ferait passer pour un
/// effet de bord une décision que personne n'a prise.
fn spirit_turn_endpoint(base_url: &str, agent_name: &str) -> String {
    format!("{base_url}/a2a/{agent_name}")
}

/// L'unique site par lequel un tour de la TUI atteint mika-spirit.
///
/// `session_id` est ce qui fait tenir l'historique. La TUI garde sa session
/// locale (V2 la déplace) et la passe en `caller_session_id`
/// (`mika.caller_session_id`, mika#2070) : spirit l'adopte comme session de
/// boucle puisqu'il possède déjà la ligne, dans la même base. Les messages du
/// tour atterrissent donc sur la session que la TUI affiche, et non sur une
/// session parallèle.
///
/// Un seul paramètre de `send_message_to_agent` reste posé vide à dessein : la
/// restriction de skills. L'isolation de session reste `false` pour une raison
/// propre — un tour de TUI veut précisément voir ses tours précédents.
///
/// `model_override` **voyage depuis le vol V3b-1**, et il part **brut** : la
/// valeur telle que l'opérateur l'a tapée, alias et préfixe `provider/`
/// compris, sans qu'aucune résolution ait lieu ici. C'est mika#2304 : seul le
/// côté qui exécute détient les clés d'API et la table d'alias, donc c'est lui
/// qui résout — et qui **refuse fermement** le tour sur un modèle qu'il ne sait
/// pas servir, plutôt que de le dégrader en « pas d'override ». Un override
/// silencieusement abandonné rendrait une réponse plausible sous un modèle que
/// l'opérateur n'a pas choisi.
///
/// `callback_turn` est en revanche **un paramètre de cette fonction** depuis le
/// vol V3a-2a : la TUI est le seul appelant qui a un tour de callback à envoyer,
/// et le paramètre existe pour qu'il n'ait pas à rouvrir ce site. Le bras
/// `Message` de `chat.rs` le pose `false` — un message tapé par l'opérateur
/// n'est pas un callback. **Le bras `CallbackResult` le pose `true`**, depuis
/// le vol V3a-2b, par [`livrer_tour_de_callback`] : c'est le seul site du crate
/// à le faire.
///
/// `base_url` est l'adresse que le worker a reçue à son spawn (vol V3c-2c), et
/// non plus celle que cette porte relisait dans l'environnement : le worker en
/// tient une seule pour toute sa vie, donc ses tours ne peuvent plus partir
/// vers deux spirits différents au gré d'un `MIKA_SPIRIT_URL` qui bougerait en
/// cours de session.
pub(crate) async fn send_turn_to_spirit(
    text: &str,
    base_url: &str,
    agent_name: &str,
    session_id: &str,
    callback_turn: bool,
    model_override: Option<&str>,
) -> Result<TurnReply> {
    let endpoint = spirit_turn_endpoint(base_url, agent_name);
    send_turn_to_endpoint(text, &endpoint, session_id, callback_turn, model_override).await
}

/// Le corps de [`send_turn_to_spirit`], l'endpoint déjà résolu.
///
/// Séparé pour une seule raison : les tests visent un serveur factice, et
/// l'alternative serait de muter `MIKA_SPIRIT_URL` — un état global de
/// processus que les tests `cargo` parallèles se disputent.
///
/// `model_override` part brut, sans résolution locale : voir
/// [`send_turn_to_spirit`] pour le pourquoi (mika#2304).
async fn send_turn_to_endpoint(
    text: &str,
    endpoint: &str,
    session_id: &str,
    callback_turn: bool,
    model_override: Option<&str>,
) -> Result<TurnReply> {
    let task = mika_cli::remote_ask::send_message_to_agent(
        text,
        endpoint,
        Some(session_id),
        &[],
        model_override,
        false,
        callback_turn,
    )
    .await
    .map_err(|e| wrap_turn_error(&e, endpoint))?;

    // mika#2270 — un `Task` que le rendu ne sait pas lire est un tour PERDU, pas
    // un agent qui n'a rien à dire : le moteur a produit sa réponse, l'a payée,
    // et le canal de retour l'a laissée tomber. L'échec est donc rendu tel quel,
    // jamais aplati en chaîne vide.
    let content = mika_cli::remote_ask::render_task_parts(&task)
        .map_err(|empty| wrap_turn_error(&anyhow::Error::new(empty), endpoint))?;

    // mika#1883 — le total du tour tel que le serveur l'atteste, lu par le
    // lecteur partagé. `None` veut dire « rien attesté », et la seule lecture
    // honnête de ça est l'absence.
    //
    // La saturation plutôt qu'un `try_into` : au-delà de `u32::MAX` jetons
    // l'affichage est de toute façon faux, mais le rendre `None` dirait
    // « le serveur n'a rien attesté », ce qui est l'autre mensonge.
    let input_tokens = mika_a2a::params::attested_run_usage(&task)
        .map(|usage| usage.input.min(u64::from(u32::MAX)) as u32);

    Ok(TurnReply {
        content,
        input_tokens,
    })
}

/// La tâche de callback qu'un tour en échec doit rendre au poll suivant.
///
/// Les deux champs ne se lisent **qu'ensemble**, et qu'à un seul endroit — le
/// bras d'erreur de [`livrer_tour_de_callback_sur_endpoint`] : l'identifiant
/// seul ne dit pas à quel statut rendre la ligne, et le statut seul ne nomme
/// aucune ligne. Les passer en paire nommée est ce qui garde les deux
/// livraisons sous l'arité que ce dépôt lint (`clippy -D warnings`), là où une
/// huitième place sur [`livrer_tour_de_callback`] n'avait d'autre issue qu'un
/// `allow` — c'est-à-dire une exception locale à un seuil que tout le workspace
/// respecte.
pub(crate) struct TacheDeCallback<'a> {
    pub task_id: &'a str,
    pub statut_dorigine: &'a str,
}

/// Le tour de callback de la TUI, livré sur le fil (mika#1727, vol V3a-2b).
///
/// C'était le **dernier** appel local de la boucle dans `mika-cli` : le bras
/// `AgentRequest::CallbackResult` de `chat.rs` faisait tourner
/// `agent::run_agent` dans le processus de la TUI. Il passe désormais par le
/// seul site d'envoi du crate, avec `callback_turn: true` — l'unique appelant à
/// `true` de [`send_turn_to_endpoint`].
///
/// Elle rend un [`AgentResponse`] et non un `Result` parce que le bras
/// d'aujourd'hui convertit **les deux** issues en `Reply` : lui rendre un
/// `Result` ferait rejouer cette conversion chez l'appelant, qui n'a aucune
/// décision à prendre dessus.
///
/// # Ce que la réponse porte, et ce qu'elle ne porte plus
///
/// Sur un succès : le texte rendu par le serveur et `input_tokens` tel que le
/// serveur l'atteste. `thinking` est **`None`**, et c'est une perte réelle — un
/// `Task` A2A ne porte pas de part de pensée, donc le bloc que le tour local
/// rendait n'existe plus. Aucun avis d'inertie n'est émis pour autant :
/// personne ne *tape* un tour de callback, et un avis sans opérateur pour le
/// lire est un avis qu'on apprend à ignorer (mika#1883).
///
/// # Pourquoi la remise du statut survit à l'envoi distant
///
/// Sur **toute** erreur, la tâche revient à son statut d'origine pour que le
/// poll suivant la reprenne — la règle d'aujourd'hui, gardée telle quelle.
/// Elle coûte désormais quelque chose de visible : une erreur peut arriver
/// **après** que spirit a fait le tour, si la relecture par `context_id`
/// échoue elle aussi (mika#2036). La livraison suivante rejoue alors un tour
/// déjà exécuté. La règle préfère ce doublon à un résultat perdu, et c'est le
/// même arbitrage que `mika ask` pose sur son canal de retour : une réponse
/// générée puis abandonnée est la panne qu'on ne voit pas.
///
/// L'échec de cette remise est journalisé et **non propagé** : l'opérateur doit
/// recevoir le message d'erreur du tour, pas le remplacer par celui de la
/// remise.
///
/// `model_override` part brut, sans résolution locale : voir
/// [`send_turn_to_spirit`] pour le pourquoi (mika#2304). Un tour de callback le
/// porte comme les autres — le modèle que l'opérateur a choisi vaut pour la
/// reprise d'une tâche autant que pour ce qu'il tape.
///
/// `base_url` arrive en deuxième position, juste après la base de données,
/// comme sur
/// [`send_turn_to_spirit`] : c'est l'adresse du spawn (vol V3c-2c), et un tour
/// de callback part vers le même spirit que les tours tapés — il reprend une
/// tâche de cette session-là.
pub(crate) async fn livrer_tour_de_callback(
    db: &AsyncDatabase,
    base_url: &str,
    agent_name: &str,
    session_id: &str,
    cadrage: &str,
    tache: TacheDeCallback<'_>,
    model_override: Option<&str>,
) -> AgentResponse {
    let endpoint = spirit_turn_endpoint(base_url, agent_name);
    livrer_tour_de_callback_sur_endpoint(db, &endpoint, session_id, cadrage, tache, model_override)
        .await
}

/// Le corps de [`livrer_tour_de_callback`], l'endpoint déjà résolu.
///
/// Séparé pour la même raison que [`send_turn_to_endpoint`] : les tests visent
/// un serveur factice, et l'alternative serait de muter `MIKA_SPIRIT_URL` — un
/// état global de processus que les tests `cargo` parallèles se disputent.
///
/// `model_override` part brut, sans résolution locale : voir
/// [`send_turn_to_spirit`] pour le pourquoi (mika#2304).
async fn livrer_tour_de_callback_sur_endpoint(
    db: &AsyncDatabase,
    endpoint: &str,
    session_id: &str,
    cadrage: &str,
    tache: TacheDeCallback<'_>,
    model_override: Option<&str>,
) -> AgentResponse {
    match send_turn_to_endpoint(cadrage, endpoint, session_id, true, model_override).await {
        Ok(reply) => AgentResponse::Reply {
            content: reply.content,
            is_error: false,
            thinking: None,
            input_tokens: reply.input_tokens,
        },
        Err(e) => {
            // `mark_task_delivered` a posé `delivered` ; la remise au statut
            // d'origine est ce qui rend la tâche à
            // `get_undelivered_callback_tasks`.
            if let Err(reset_err) = db
                .update_task_status(tache.task_id, tache.statut_dorigine)
                .await
            {
                tracing::warn!(
                    task_id = %tache.task_id,
                    error = %reset_err,
                    "failed to unclaim callback task after agent error"
                );
            }
            AgentResponse::Reply {
                content: format!("Failed to process callback result: {e}"),
                is_error: true,
                thinking: None,
                input_tokens: None,
            }
        }
    }
}

/// Ce qu'un `SetModel { model }` laisse comme override pour les tours suivants.
///
/// Une fonction, et non trois lignes dans le bras, parce que c'est la seule
/// décision que ce bras prend encore depuis le vol V3b-1 : tout le reste —
/// l'alias, le préfixe de provider, la clé d'API — est résolu par spirit
/// (mika#2304). Isolée, elle se mesure sans ouvrir ni base ni session.
///
/// Une valeur blanche rend **`None`** plutôt que `Some("")` : spirit refuserait
/// fermement une clé vide, donc la poser couperait tous les tours suivants au
/// lieu de rendre son modèle par défaut. `/model` sans argument n'atteint pas ce
/// bras aujourd'hui, mais la ligne de défense se tient ici, là où la clé est
/// décidée, et non chez chaque appelant.
///
/// La valeur non vide est `trim`ée et pas autrement touchée : la normaliser
/// serait commencer à résoudre.
pub(crate) fn override_de_set_model(model: &str) -> Option<String> {
    let taille = model.trim();
    if taille.is_empty() {
        None
    } else {
        Some(taille.to_string())
    }
}

/// Aplatir l'échec d'un tour à la façon de `commands::ask::wrap_send_error`.
///
/// `{err:#}` et pas `{err}` : la chaîne porte le statut HTTP, l'échec de
/// transport, la transition d'état A2A et l'erreur serveur, et l'afficheur de
/// la TUI n'imprime que la couche de tête. L'incident OAuth Prime du
/// 2026-08-24 a coûté ~2 h de diagnostic sous ce masquage (mika#1985).
///
/// La classe de transport est **préservée** : `is_transport_failure` marche sur
/// toute la chaîne de causes, donc un appelant peut encore distinguer
/// « réessaie » de « le contrat est cassé » après ce ré-emballage.
///
/// Pas d'indice « (is it running?) » : `commands::ask` l'a écarté parce qu'il
/// suggère une cause unique là où la chaîne en nomme déjà la vraie.
fn wrap_turn_error(err: &anyhow::Error, endpoint: &str) -> anyhow::Error {
    let message = format!("le tour de la TUI vers {endpoint} a échoué : {err:#}");
    if mika_cli::remote_ask::is_transport_failure(err) {
        anyhow::Error::new(mika_cli::remote_ask::TransportClass::new(message))
    } else {
        anyhow::anyhow!("{message}")
    }
}

/// Ce qu'une image collée laisse derrière elle quand le tour part sans elle.
///
/// Le tour ne portant que du texte, les deux champs que la TUI capture pour
/// l'envoi — le type MIME et la charge base64 — n'ont plus de consommateur sur
/// le chemin d'envoi. Ils en gardent un ici, et c'est délibéré : le seul endroit
/// où ils ont encore un sens est l'avis qui dit à l'opérateur ce qui est tombé.
/// Les jeter de `tui::attachment::ImageAttachment` à la place demanderait de
/// réécrire l'encodage du collage le jour où une part fichier voyage.
pub(crate) struct DroppedImage {
    /// Le type MIME tel que le collage l'a reconnu.
    pub media_type: String,
    /// La taille de la charge base64 que le tour a transportée pour rien.
    ///
    /// Distincte de `ImageAttachment::size_bytes`, qui est la taille décodée :
    /// c'est bien l'encodage qui a voyagé, environ un tiers plus gros.
    pub payload_bytes: usize,
}

/// L'avis à afficher pour les entrées de la TUI qui n'atteignent plus le tour.
///
/// Trois inerties, un seul avis, **à chaque tour qui les exerce** — pas une
/// fois par session. Bruyant, et c'est le but : un réglage qui ne fait plus ce
/// qu'il annonce doit se voir chaque fois qu'on s'en sert, et l'avis nomme le
/// geste qui le fait taire. C'est la forme de `cli_skill_flag_inert`
/// (mika#1883), qui avertit à chaque invocation.
///
/// - `/think` → le budget de pensée n'a aucune clé de fil pour voyager, et le
///   serveur ne lit pas `thinking_level` : `config_keys.rs` le valide, personne
///   ne le convertit. Le bloc `thinking:` de la réponse disparaît par la même
///   cause — le `Task` A2A ne porte pas de part de pensée — donc les deux sont
///   dits dans le même avis.
/// - image collée → `remote_ask::build_send_params` ne bâtit que des parts
///   texte. Les images sont abandonnées, et l'avis dit **ce qui** est tombé.
///
/// `None` quand le tour n'exerce aucune des deux entrées : un avis qui
/// s'afficherait sans cause est un avis qu'on apprend à ignorer.
pub(crate) fn inertia_notice(
    thinking_budget: Option<u32>,
    dropped_images: &[DroppedImage],
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    if thinking_budget.is_some() {
        lines.push(
            "le budget de pensée n'atteint plus le tour (il ne voyage pas jusqu'à mika-spirit) \
             et la réponse ne portera plus de bloc `thinking:` — `/think off` éteint l'avis"
                .to_string(),
        );
    }
    if !dropped_images.is_empty() {
        let mut kinds: Vec<&str> = Vec::new();
        for image in dropped_images {
            let kind = image.media_type.as_str();
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        let payload: usize = dropped_images.iter().map(|image| image.payload_bytes).sum();
        lines.push(format!(
            "{} image(s) abandonnée(s) ({} ; {} de base64 porté jusqu'au worker puis jeté) : \
             un tour vers mika-spirit ne porte que du texte — envoyer le message sans pièce \
             jointe éteint l'avis",
            dropped_images.len(),
            kinds.join(", "),
            crate::tui::attachment::ImageAttachment::format_size(payload),
        ));
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!("\u{26a0} mika#1727 — {}", lines.join(" ; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::StatusCode, routing::post};
    use mika_agent::db::{Database, NewTask};
    use mika_common::dead_endpoint::DeadEndpoint;
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    /// Le dernier corps JSON-RPC reçu, pour assertion sur ce que le client a
    /// réellement envoyé.
    #[derive(Default)]
    struct Capture {
        last_body: Option<Value>,
    }

    type SharedCapture = Arc<Mutex<Capture>>;

    /// Un serveur A2A factice, sur le patron de `tests/remote_ask_integration.rs`.
    ///
    /// La route ne porte qu'un segment d'agent (`/a2a/{agent}`) : c'est la forme
    /// du spirit local, pas celle de la passerelle (`/a2a/{customer}/{agent}`).
    async fn spawn_mock<F>(handler: F) -> (SocketAddr, SharedCapture)
    where
        F: Fn(SharedCapture, Json<Value>) -> (StatusCode, Json<Value>)
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let capture: SharedCapture = Arc::new(Mutex::new(Capture::default()));
        let cap_for_handler = capture.clone();
        let app = Router::new().route(
            "/a2a/{agent}",
            post(move |body: Json<Value>| {
                let h = handler.clone();
                let cap = cap_for_handler.clone();
                async move { h(cap, body) }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });
        (addr, capture)
    }

    fn ok_task_with_text(text: &str) -> Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "id": "task-mock-1",
                "status": {
                    "state": "completed",
                    "message": {
                        "messageId": "msg-agent-1",
                        "role": "agent",
                        "parts": [{"kind": "text", "text": text}],
                        "kind": "message",
                    }
                },
                "kind": "task",
            }
        })
    }

    /// Un `Task` complété qui porte `metadata`, pour les attestations.
    fn ok_task_with_metadata(text: &str, metadata: Value) -> Value {
        let mut response = ok_task_with_text(text);
        response["result"]["metadata"] = metadata;
        response
    }

    /// **T1** — le tour part sur le fil, et c'est bien un `message/send`.
    #[tokio::test]
    async fn mika1727_v1_t1_le_tour_part_sur_le_fil() {
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("bonjour")))
        })
        .await;

        let endpoint = format!("http://{addr}/a2a/mika-prime");
        let reply = send_turn_to_endpoint("quoi de neuf", &endpoint, "sess-t1", false, None)
            .await
            .expect("un spirit qui répond un Task complété rend son texte");
        assert_eq!(reply.content, "bonjour");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        assert_eq!(body["method"], "message/send");
        let parts = &body["params"]["message"]["parts"];
        assert_eq!(parts[0]["kind"], "text");
        assert_eq!(parts[0]["text"], "quoi de neuf");
        assert_eq!(body["params"]["message"]["role"], "user");
    }

    /// **T2** — la session de la TUI voyage, donc l'historique tient.
    ///
    /// La clé est lue par la constante de `mika-a2a`, jamais par une aiguille
    /// littérale : un littéral ici serait le premier pas d'un second décodeur,
    /// ce que `mika1883_both_client_surfaces_read_the_one_reader` refuse déjà.
    #[tokio::test]
    async fn mika1727_v1_t2_la_session_voyage() {
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("ok")))
        })
        .await;

        let endpoint = format!("http://{addr}/a2a/mika-prime");
        send_turn_to_endpoint("salut", &endpoint, "sess-t2-abc", false, None)
            .await
            .expect("envoi");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        assert_eq!(
            body["params"]["metadata"][mika_cli::remote_ask::CALLER_SESSION_ID_KEY],
            "sess-t2-abc",
            "sans cette clé spirit minte sa propre session et le tour quitte \
             l'historique que la TUI affiche (mika#2070) ; corps : {body}"
        );
    }

    /// **T3** — un spirit injoignable est un échec nommé, pas un silence.
    ///
    /// mika#2569 : le port est TENU pour toute la durée de l'appel. Le motif
    /// qui liait puis libérait un port a vu un serveur factice voisin du même
    /// binaire servir l'appel censé échouer (2026-09-28, job `Check` de
    /// mika#2561). `dead` doit rester vivant jusqu'après l'appel.
    #[tokio::test]
    async fn mika1727_v1_t3_un_spirit_injoignable_est_un_echec_nomme() {
        let dead = DeadEndpoint::reserve();
        let endpoint = format!("http://{}/a2a/mika-prime", dead.addr());

        let err = send_turn_to_endpoint("salut", &endpoint, "sess-t3", false, None)
            .await
            .expect_err("rien n'écoute : l'appel doit échouer");
        let chain = format!("{err:#}");

        assert!(
            chain.contains("le tour de la TUI vers") && chain.contains(&endpoint),
            "l'échec doit nommer l'endpoint visé ; obtenu : {chain}"
        );
        assert!(
            chain.contains("unreachable"),
            "la chaîne sous-jacente doit survivre au ré-emballage ; obtenu : {chain}"
        );
        assert!(
            mika_cli::remote_ask::is_transport_failure(&err),
            "un endpoint mort est une classe transport, et le ré-emballage ne \
             doit pas la perdre ; obtenu : {chain}"
        );
    }

    /// **T4** — un `Task` illisible est un échec, jamais une réponse vide.
    ///
    /// Contrôle négatif de mika#2270 : onze appels consécutifs y ont sorti 0
    /// avec un contenu absent pendant que le verdict dormait dans le journal du
    /// serveur.
    #[tokio::test]
    async fn mika1727_v1_t4_un_task_illisible_est_un_echec() {
        let (addr, _) = spawn_mock(|_cap, _body| {
            let response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "id": "task-mock-vide",
                    "status": {"state": "completed"},
                    "kind": "task",
                }
            });
            (StatusCode::OK, Json(response))
        })
        .await;

        let endpoint = format!("http://{addr}/a2a/mika-prime");
        let outcome = send_turn_to_endpoint("salut", &endpoint, "sess-t4", false, None).await;
        let err = match outcome {
            Ok(reply) => panic!(
                "un Task sans texte lisible doit échouer, pas rendre {:?}",
                reply.content
            ),
            Err(err) => err,
        };
        assert!(
            format!("{err:#}").contains("le tour de la TUI vers"),
            "l'échec de rendu doit nommer l'endpoint comme les autres ; obtenu : {err:#}"
        );
    }

    /// **T5** — les jetons absents ne deviennent pas zéro.
    ///
    /// Les deux moitiés comptent : l'attestation présente est rendue, et
    /// l'attestation absente reste `None`. Seule la première sans la seconde
    /// laisserait passer un `unwrap_or(0)`.
    #[tokio::test]
    async fn mika1727_v1_t5_les_jetons_absents_ne_deviennent_pas_zero() {
        let (addr, _) =
            spawn_mock(|_cap, _body| (StatusCode::OK, Json(ok_task_with_text("ok")))).await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        let reply = send_turn_to_endpoint("salut", &endpoint, "sess-t5a", false, None)
            .await
            .expect("envoi");
        assert_eq!(
            reply.input_tokens, None,
            "rien d'attesté doit rester absent, jamais 0 (mika#2331)"
        );

        let (addr, _) = spawn_mock(|_cap, _body| {
            let metadata = serde_json::json!({
                mika_cli::remote_ask::RUN_USAGE_KEY: {"input": 41_000, "output": 900},
            });
            (StatusCode::OK, Json(ok_task_with_metadata("ok", metadata)))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        let reply = send_turn_to_endpoint("salut", &endpoint, "sess-t5b", false, None)
            .await
            .expect("envoi");
        assert_eq!(reply.input_tokens, Some(41_000));
    }

    /// **Critère 3 de V3a-2a — la clé de callback voyage, et son absence aussi.**
    ///
    /// Les deux moitiés vivent dans le **même** test, par deux `spawn_mock`
    /// dans la même fonction. La moitié négative n'a de valeur que comparée à la
    /// positive dans la même mesure : séparées, un `spawn_mock` qui capturerait
    /// mal le corps rendrait la négative verte pour la mauvaise raison, et rien
    /// ne le dirait.
    ///
    /// La clé est lue par la constante, jamais par un littéral — même raison
    /// qu'en T2.
    #[tokio::test]
    async fn mika1727_v3a2a_la_cle_de_callback_voyage_et_son_absence_aussi() {
        // Moitié positive : `true` pose la clé, et la pose À CÔTÉ de la session.
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("ok")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        send_turn_to_endpoint("salut", &endpoint, "sess-cb-oui", true, None)
            .await
            .expect("envoi");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        let metadata = &body["params"]["metadata"];
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLBACK_TURN_KEY],
            serde_json::Value::Bool(true),
            "le serveur lit un booléen strict : toute autre forme FAIT ÉCHOUER \
             le tour au lieu de dégrader en tour ordinaire (V3a-1) ; corps : {body}"
        );
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLER_SESSION_ID_KEY],
            "sess-cb-oui",
            "la clé de callback ne doit pas chasser celle de la session : un \
             tour de callback doit atterrir sur l'historique que la TUI \
             affiche ; corps : {body}"
        );

        // Moitié négative : le MÊME envoi à `false` ne porte pas la clé. Sans
        // cette moitié, un `insert` rendu inconditionnel passerait la première.
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("ok")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        send_turn_to_endpoint("salut", &endpoint, "sess-cb-non", false, None)
            .await
            .expect("envoi");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        let metadata = &body["params"]["metadata"];
        assert!(
            metadata
                .get(mika_cli::remote_ask::CALLBACK_TURN_KEY)
                .is_none(),
            "un tour ordinaire ne pose pas la clé, pas même à `false` ; \
             corps : {body}"
        );
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLER_SESSION_ID_KEY],
            "sess-cb-non",
            "anti-vacuité de la moitié négative : le corps capturé est bien \
             celui de ce second envoi, et non un corps resté vide ; \
             corps : {body}"
        );
    }

    /// **Critère 1 de V3b-1 — le modèle choisi voyage, et son absence aussi.**
    ///
    /// Les deux moitiés vivent dans la **même** fonction, pour la raison du test
    /// de callback ci-dessus : séparée, la moitié négative serait verte devant un
    /// `spawn_mock` qui capture mal, et rien ne le dirait.
    ///
    /// La valeur donnée porte un préfixe de provider **exprès** : elle doit
    /// arriver telle quelle sur le fil. Une valeur que ce site aurait raccourcie,
    /// résolue ou normalisée serait un second résolveur de modèle à côté de celui
    /// de spirit (mika#2304) — et le jour où les deux divergeraient, l'opérateur
    /// recevrait une réponse d'un modèle qu'il n'a pas demandé.
    #[tokio::test]
    async fn mika1727_v3b1_le_modele_choisi_voyage_et_son_absence_aussi() {
        // Moitié positive : la valeur part brute, et À CÔTÉ de la session.
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("ok")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        send_turn_to_endpoint(
            "salut",
            &endpoint,
            "sess-modele-oui",
            false,
            Some("anthropic/claude-sonnet-4-6"),
        )
        .await
        .expect("envoi");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        let metadata = &body["params"]["metadata"];
        assert_eq!(
            metadata[mika_cli::remote_ask::MODEL_OVERRIDE_KEY],
            "anthropic/claude-sonnet-4-6",
            "le modèle choisi doit partir tel que tapé : c'est spirit qui \
             résout l'alias et le préfixe, et qui détient les clés \
             (mika#2304) ; corps : {body}"
        );
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLER_SESSION_ID_KEY],
            "sess-modele-oui",
            "la clé du modèle ne doit pas chasser celle de la session : le \
             tour doit atterrir sur l'historique que la TUI affiche ; \
             corps : {body}"
        );

        // Moitié négative : le MÊME envoi sans override ne porte pas la clé.
        // Sans elle, un `insert` rendu inconditionnel passerait la première — et
        // une clé posée à `null` est refusée fermement par spirit, donc elle
        // couperait tous les tours de qui n'a jamais tapé `/model`.
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("ok")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        send_turn_to_endpoint("salut", &endpoint, "sess-modele-non", false, None)
            .await
            .expect("envoi");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        let metadata = &body["params"]["metadata"];
        assert!(
            metadata
                .get(mika_cli::remote_ask::MODEL_OVERRIDE_KEY)
                .is_none(),
            "aucun override veut dire aucune clé, pas une clé vide ni nulle \
             (mika#2331) ; corps : {body}"
        );
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLER_SESSION_ID_KEY],
            "sess-modele-non",
            "anti-vacuité de la moitié négative : le corps capturé est bien \
             celui de ce second envoi, et non un corps resté vide ; \
             corps : {body}"
        );
    }

    /// Une base en mémoire portant une tâche de callback **déjà réclamée**.
    ///
    /// Le gabarit `NewTask` vient de `tui/app.rs::couture_dun_callback_livrable`
    /// — une `long_running` en `callback`, action `resume_agent` — et la ligne
    /// est passée à `delivered`, qui est l'état où `mark_task_delivered` l'a
    /// laissée quand le bras tourne. Le statut d'**origine** n'est pas posé ici :
    /// c'est un argument du bras, et le figer dans l'assistant rendrait invisible
    /// la seule chose que le test de l'erreur mesure.
    ///
    /// Un seul assistant pour tous les tests du vol : deux copies d'un gabarit de
    /// vingt-deux champs divergent en silence, et les deux tests cesseraient de
    /// parler de la même tâche (mika#2205).
    async fn tache_de_callback_livree(db: &AsyncDatabase) -> String {
        let id = db
            .create_task(NewTask {
                agent_id: db.agent_id().to_string(),
                team_run_id: None,
                parent_task_id: None,
                depth: 1,
                label: "long_running:run_claude_pilot".to_string(),
                trigger_type: "callback".to_string(),
                cron_expr: None,
                event_source: None,
                event_offset_secs: None,
                condition_expr: None,
                next_fire_at: None,
                timeout_at: None,
                action_type: "resume_agent".to_string(),
                action_config: "{}".to_string(),
                input_context: None,
                created_by_session: Some("sess-du-pilote".to_string()),
                created_trace_id: None,
                reference_url: None,
                source: Some("self_dev".to_string()),
                metadata: None,
                r#type: None,
                dispatch_class: Some("implement".to_string()),
            })
            .await
            .expect("une ligne de callback");
        db.update_task_status(&id, "delivered")
            .await
            .expect("la réclamation de la livraison");
        id
    }

    fn base_de_callback() -> AsyncDatabase {
        AsyncDatabase::new(Database::open_in_memory().expect("une base en mémoire"))
    }

    /// **Critères 1 et 2 de V3a-2b** — le tour de callback part sur le fil avec
    /// la clé, et un succès ne touche pas à la tâche.
    ///
    /// Les deux critères tiennent dans un test parce qu'ils décrivent le **même**
    /// envoi : séparés, le second rouvrirait un serveur factice pour re-mesurer
    /// ce que le premier vient de mesurer, et les deux pourraient diverger sur ce
    /// qui a réellement été envoyé.
    ///
    /// Le critère 2 est le contrôle négatif du vol : sans lui, une remise de
    /// statut rendue inconditionnelle passerait le test de l'erreur.
    #[tokio::test]
    async fn mika1727_v3a2b_le_tour_de_callback_part_sur_le_fil_avec_la_cle() {
        let db = base_de_callback();
        let task_id = tache_de_callback_livree(&db).await;

        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("le pilote a fini")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");

        let reponse = livrer_tour_de_callback_sur_endpoint(
            &db,
            &endpoint,
            "sess-cb-v3a2b",
            "[callback: run_claude_pilot] terminé",
            TacheDeCallback {
                task_id: &task_id,
                statut_dorigine: "completed",
            },
            None,
        )
        .await;

        let AgentResponse::Reply {
            content, is_error, ..
        } = &reponse
        else {
            panic!("le bras convertit les deux issues en Reply, et rien d'autre")
        };
        assert!(!is_error, "un tour qui aboutit n'est pas une erreur");
        assert_eq!(
            content, "le pilote a fini",
            "la réponse porte le texte rendu par le serveur, et non un texte \
             composé ici"
        );

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        let metadata = &body["params"]["metadata"];
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLBACK_TURN_KEY],
            Value::Bool(true),
            "sans cette clé spirit fait un tour ordinaire : la garde #716 reste \
             désarmée et le cadrage est enregistré comme un message de \
             l'utilisateur ; corps : {body}"
        );
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLER_SESSION_ID_KEY],
            "sess-cb-v3a2b",
            "le tour doit atterrir sur l'historique que la TUI affiche ; \
             corps : {body}"
        );
        assert_eq!(
            body["params"]["message"]["parts"][0]["text"], "[callback: run_claude_pilot] terminé",
            "c'est le cadrage bâti par le bras qui part, pas le résultat brut ; \
             corps : {body}"
        );

        let tache = db
            .get_task(&task_id)
            .await
            .expect("relecture")
            .expect("la tâche existe");
        assert_eq!(
            tache.status, "delivered",
            "un succès ne rend pas la tâche : la rendre la ferait rejouer au \
             poll suivant, et le tour aurait lieu deux fois"
        );
    }

    /// **Critère 3 de V3a-2b** — sur une erreur, la tâche revient à son statut
    /// d'origine, quel qu'il soit.
    ///
    /// Les deux statuts que `mark_task_delivered` accepte sont parcourus dans la
    /// même mesure : la remise lit un argument, et un test sur un seul statut
    /// resterait vert devant un littéral codé en dur.
    #[tokio::test]
    async fn mika1727_v3a2b_un_endpoint_mort_rend_la_tache_a_son_statut_dorigine() {
        for statut_dorigine in ["completed", "failed"] {
            let db = base_de_callback();
            let task_id = tache_de_callback_livree(&db).await;

            // mika#2569 : le port est TENU pour toute la durée de l'appel, sans
            // quoi un serveur factice voisin du même binaire sert l'appel censé
            // échouer.
            let dead = DeadEndpoint::reserve();
            let endpoint = format!("http://{}/a2a/mika-prime", dead.addr());

            let reponse = livrer_tour_de_callback_sur_endpoint(
                &db,
                &endpoint,
                "sess-cb-mort",
                "[callback: run_claude_pilot] terminé",
                TacheDeCallback {
                    task_id: &task_id,
                    statut_dorigine,
                },
                None,
            )
            .await;

            let AgentResponse::Reply {
                content, is_error, ..
            } = &reponse
            else {
                panic!("le bras convertit les deux issues en Reply, et rien d'autre")
            };
            assert!(is_error, "un spirit injoignable est une erreur");
            assert!(
                content.starts_with("Failed to process callback result:"),
                "le texte d'aujourd'hui est gardé mot pour mot ; obtenu : {content}"
            );

            let tache = db
                .get_task(&task_id)
                .await
                .expect("relecture")
                .expect("la tâche existe");
            assert_eq!(
                tache.status, statut_dorigine,
                "sans la remise, la tâche reste `delivered` et aucun poll ne la \
                 reprend : le résultat du pilote est perdu en silence"
            );
        }
    }

    /// **Critère 2 de V3b-1** — le tour de callback porte le modèle choisi, et
    /// la clé de callback reste à `true`.
    ///
    /// Les deux clés sont mesurées dans le même envoi : le callback passe par le
    /// site d'envoi commun, donc le risque réel n'est pas qu'une des deux
    /// manque, c'est qu'un argument supplémentaire **déplace** l'autre dans la
    /// liste. Un test qui ne regarderait que l'override resterait vert avec un
    /// `callback_turn` parti à `false` — et spirit ferait alors un tour
    /// ordinaire, laissant la garde #716 désarmée.
    #[tokio::test]
    async fn mika1727_v3b1_le_tour_de_callback_porte_le_modele_choisi() {
        let db = base_de_callback();
        let task_id = tache_de_callback_livree(&db).await;

        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("le pilote a fini")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");

        let reponse = livrer_tour_de_callback_sur_endpoint(
            &db,
            &endpoint,
            "sess-cb-modele",
            "[callback: run_claude_pilot] terminé",
            TacheDeCallback {
                task_id: &task_id,
                statut_dorigine: "completed",
            },
            Some("groq/kimi-k2"),
        )
        .await;

        let AgentResponse::Reply { is_error, .. } = &reponse else {
            panic!("le bras convertit les deux issues en Reply, et rien d'autre")
        };
        assert!(!is_error, "un tour qui aboutit n'est pas une erreur");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        let metadata = &body["params"]["metadata"];
        assert_eq!(
            metadata[mika_cli::remote_ask::MODEL_OVERRIDE_KEY],
            "groq/kimi-k2",
            "un tour de callback porte le modèle choisi comme les autres : la \
             reprise d'une tâche tourne sous le modèle que l'opérateur a \
             posé ; corps : {body}"
        );
        assert_eq!(
            metadata[mika_cli::remote_ask::CALLBACK_TURN_KEY],
            Value::Bool(true),
            "l'argument de l'override ne doit pas avoir déplacé celui du \
             callback : sans cette clé spirit fait un tour ordinaire ; \
             corps : {body}"
        );
    }

    /// **Critère 3 de V3b-1** — l'override de `SetModel` se lit, et se tait sur
    /// une valeur blanche.
    ///
    /// La table parcourt les trois formes que `format_worker_model` peut rendre
    /// — nom nu, nom préfixé d'un provider, et la valeur blanche — plus la
    /// chaîne vide. Les deux premières attestent que **rien n'est résolu ici** :
    /// le préfixe survit, l'alias survit. Les deux dernières sont le négatif, et
    /// c'est la moitié qui compte : un `Some(String::new())` poserait une clé
    /// vide que spirit refuse fermement, donc il couperait tous les tours
    /// suivants au lieu de rendre le modèle par défaut.
    ///
    /// La sortie est ensuite **donnée au site d'envoi** : la fonction pure et le
    /// fil se mesurent dans la même fonction, sinon la paire resterait verte
    /// avec un bras qui lit la fonction et n'en fait rien.
    #[tokio::test]
    async fn mika1727_v3b1_loverride_de_set_model_se_lit_et_se_tait() {
        assert_eq!(
            override_de_set_model("claude-sonnet-4-6"),
            Some("claude-sonnet-4-6".to_string()),
            "un nom nu part tel quel : c'est spirit qui résout l'alias"
        );
        assert_eq!(
            override_de_set_model("  groq/kimi-k2  "),
            Some("groq/kimi-k2".to_string()),
            "le préfixe de provider SURVIT, et seuls les blancs de bord \
             tombent : normaliser ici serait commencer à résoudre (mika#2304)"
        );
        assert_eq!(
            override_de_set_model("   "),
            None,
            "une valeur blanche doit rendre l'absence : une clé vide est \
             refusée fermement par spirit, donc elle couperait les tours \
             suivants au lieu de rendre le modèle par défaut"
        );
        assert_eq!(
            override_de_set_model(""),
            None,
            "même raison que ci-dessus pour la chaîne vide"
        );

        // La sortie de la fonction pure est bien ce qui atteint le fil : c'est
        // le maillon que la table seule ne mesure pas.
        let (addr, capture) = spawn_mock(|cap, Json(body)| {
            cap.lock().unwrap().last_body = Some(body);
            (StatusCode::OK, Json(ok_task_with_text("ok")))
        })
        .await;
        let endpoint = format!("http://{addr}/a2a/mika-prime");
        let pose = override_de_set_model("groq/kimi-k2");
        send_turn_to_endpoint(
            "salut",
            &endpoint,
            "sess-apres-setmodel",
            false,
            pose.as_deref(),
        )
        .await
        .expect("envoi");

        let body = capture.lock().unwrap().last_body.clone().unwrap();
        assert_eq!(
            body["params"]["metadata"][mika_cli::remote_ask::MODEL_OVERRIDE_KEY],
            "groq/kimi-k2",
            "ce que la fonction pure rend est ce que le tour suivant porte ; \
             corps : {body}"
        );
    }

    /// Une image abandonnée, pour les assertions de l'avis d'inertie.
    fn dropped(media_type: &str, payload_bytes: usize) -> DroppedImage {
        DroppedImage {
            media_type: media_type.to_string(),
            payload_bytes,
        }
    }

    /// Les trois inerties de D3 sont dites, et chacune nomme son extinction.
    #[test]
    fn mika1727_v1_les_inerties_nomment_le_geste_qui_les_eteint() {
        assert_eq!(
            inertia_notice(None, &[]),
            None,
            "un tour qui n'exerce aucune inertie ne doit rien afficher"
        );

        let pensee = inertia_notice(Some(10_000), &[]).expect("un budget de pensée doit être dit");
        assert!(
            pensee.contains("/think off"),
            "l'avis doit nommer le geste qui l'éteint ; obtenu : {pensee}"
        );
        assert!(
            pensee.contains("thinking:"),
            "la disparition du bloc de pensée est dite dans le même avis ; obtenu : {pensee}"
        );

        let images = inertia_notice(
            None,
            &[
                dropped("image/png", 300 * 1024),
                dropped("image/png", 200 * 1024),
            ],
        )
        .expect("des images abandonnées doivent être dites");
        assert!(
            images.contains('2'),
            "le nombre d'images abandonnées doit être dit ; obtenu : {images}"
        );
        assert!(
            images.contains("image/png"),
            "le type de ce qui est tombé doit être dit ; obtenu : {images}"
        );
        assert_eq!(
            images.matches("image/png").count(),
            1,
            "deux images du même type se disent une fois : répéter le type ferait \
             d'un avis de dix images collées un pavé illisible ; obtenu : {images}"
        );
        assert!(
            images.contains("500KB"),
            "la charge transportée pour rien est la somme des deux ; obtenu : {images}"
        );

        let deux = inertia_notice(Some(10_000), &[dropped("image/jpeg", 1024)])
            .expect("les deux causes doivent être dites");
        assert!(
            deux.contains("/think off") && deux.contains('1') && deux.contains("image/jpeg"),
            "{deux}"
        );
    }

    /// Allowlist du scan structurel du vol V1 — **livrée vide depuis le vol
    /// V3a-2b**, et c'est la propriété (doctrine mika#2201, #2323, #2425,
    /// #2457, #2759).
    ///
    /// Elle est au niveau du module, comme [`V2A_ALLOWED`] et pour la même
    /// raison : la garde de vacuité
    /// [`mika1727_v3a2b_lallowlist_du_tour_est_livree_vide`] doit observer la
    /// constante que le scan lit réellement, et non une copie qui pourrait
    /// diverger d'elle.
    const V1_ALLOWED: &[&str] = &[];

    /// **AC1** — aucun tour de la TUI ne fait tourner la boucle ici.
    ///
    /// **Aucun site n'est plus toléré depuis le vol V3a-2b.** Le bras
    /// `CallbackResult` de `chat.rs` était le dernier appel local du crate ; il
    /// passe désormais par `chat_turn::livrer_tour_de_callback`, qui pose
    /// `mika.callback_turn` sur le fil. L'exception de fichier qui le couvrait
    /// est partie avec lui, et le compte qui la doublait aussi.
    ///
    /// Quand cette garde rougit, le remède est de router le nouvel appelant par
    /// l'envoi qui existe — `send_turn_to_spirit` pour un tour tapé,
    /// `livrer_tour_de_callback` pour un tour de callback —, **jamais**
    /// d'ajouter une ligne à [`V1_ALLOWED`].
    #[test]
    fn mika1727_v1_le_tour_de_la_tui_ne_fait_plus_tourner_la_boucle() {
        const NEEDLES: &[&str] = &["agent::run_agent(", "AgentParams"];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let src_root = scanner.src_root().to_path_buf();
        let mut violations: Vec<String> = Vec::new();

        scanner.for_each(|path, production| {
            let rel = path
                .strip_prefix(&src_root)
                .unwrap_or(path)
                .display()
                .to_string();
            if V1_ALLOWED.contains(&rel.as_str()) {
                return;
            }
            for (idx, line) in production.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if NEEDLES.iter().any(|needle| line.contains(needle)) {
                    violations.push(format!("{rel}:{} — {}", idx + 1, line.trim()));
                }
            }
        });

        assert!(
            violations.is_empty(),
            "mika#1727 V1 — la boucle d'agent tourne de nouveau dans le processus \
             de la TUI. Le remède est de router le tour par \
             `chat_turn::send_turn_to_spirit` (tour tapé) ou par \
             `livrer_tour_de_callback` (tour de callback), jamais d'élargir \
             V1_ALLOWED :\n{}",
            violations.join("\n")
        );
    }

    /// **Critère 4 de V3a-2b** — [`V1_ALLOWED`] est livrée vide, et le reste.
    ///
    /// Sans cette assertion, le jour où un appel local revient, l'issue la plus
    /// courte serait d'y réinscrire `commands/chat.rs` : la garde redeviendrait
    /// verte et le scan deviendrait ce que mika#2323 refuse — un détecteur qui
    /// ne détecte plus que ce qu'on veut bien lui montrer. Le remède d'un rouge
    /// est de router le nouvel appelant par l'envoi qui existe.
    #[test]
    fn mika1727_v3a2b_lallowlist_du_tour_est_livree_vide() {
        assert!(
            V1_ALLOWED.is_empty(),
            "mika#1727 V3a-2b — V1_ALLOWED est livrée vide et doit le rester : \
             un scan structurel s'applique à tout le crate, il ne s'exempte \
             pas. Trouvé : {V1_ALLOWED:?}"
        );
    }

    // mika#1727 vol V3a-2b — `mika1727_v1_le_site_tolere_na_quun_seul_appel_local`
    // vivait ici. Elle comptait les `agent::run_agent(` de `commands/chat.rs` et
    // en exigeait exactement **un**, le bras `CallbackResult` : l'exception
    // portait sur le fichier, donc elle n'aurait pas vu le tour utilisateur
    // régresser à côté du bras toléré. Ce vol envoie ce dernier appel sur le
    // fil ; le compte doit valoir zéro, ce que la garde principale mesure déjà
    // sur tout le crate, allowlist vide. La garder voudrait dire garder un
    // `== 1` qui est devenu un faux rouge.

    /// Anti-vacuité (mika#2103) — le balayage a réellement vu les deux fichiers
    /// du vol.
    ///
    /// Un scanner qui ne voit rien passe les deux gardes ci-dessus sans
    /// refuser quoi que ce soit : c'est la forme de garde qui passe la revue,
    /// passe la CI par construction, et laisse la régression atterrir.
    #[test]
    fn mika1727_v1_le_scan_nest_pas_vide() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let files = scanner.files();

        assert!(
            files.len() > 10,
            "le balayage de `mika-cli` doit voir tout son arbre ; vu {} fichier(s)",
            files.len()
        );
        // `commands/` est porté exprès : `Path::ends_with` compare par
        // composants, donc le suffixe nu `chat.rs` serait satisfait par
        // n'importe quel `chat.rs` de l'arbre — la forme de vérification qui
        // passe pendant qu'une moitié de la propriété est fausse.
        for expected in ["commands/chat.rs", "commands/chat_turn.rs"] {
            assert!(
                files.iter().any(|p| p.ends_with(expected)),
                "le balayage doit voir {expected} ; sinon les gardes de ce vol \
                 n'inspectent pas ce qu'elles prétendent"
            );
        }
    }

    /// Allowlist du scan structurel du vol V2a — **livrée vide**, et c'est la
    /// propriété (doctrine mika#2201, #2323, #2425, #2457, #2759).
    ///
    /// Elle est au niveau du module pour que la garde de vacuité
    /// [`mika1727_v2a_lallowlist_est_livree_vide`] observe la constante que le
    /// scan lit réellement, et non une copie qui pourrait diverger d'elle.
    const V2A_ALLOWED: &[&str] = &[];

    /// **Critère 1 du vol V2a** — `mika-cli` ne construit plus de moteur de
    /// tâches.
    ///
    /// Le scan porte sur le **geste de construction**, jamais sur le chemin de
    /// module : `commands/tasks.rs` importe légitimement neuf utilitaires de
    /// `mika_agent::task_engine` pour lire et écrire des tâches par la base, et
    /// un scan sur `task_engine::` rougirait sur ce lecteur-là en laissant
    /// passer un `TaskDispatcher` construit sous un alias.
    ///
    /// `prune_old_tasks(` n'est délibérément **pas** une aiguille : c'est une
    /// purge que n'importe quel client pourrait légitimement déclencher, pas la
    /// preuve d'un moteur local. Les quatre aiguilles retenues ne se
    /// prononcent que sur « ce processus possède un moteur ».
    ///
    /// Le remède quand ce test rougit est de router le besoin par mika-spirit,
    /// **jamais** d'élargir [`V2A_ALLOWED`].
    #[test]
    fn mika1727_v2a_la_tui_ne_construit_plus_de_moteur_de_taches() {
        const NEEDLES: &[&str] = &[
            "TaskEngine::new(",
            "TaskDispatcher {",
            "spawn_tick_loop(",
            "startup_recovery(",
        ];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let src_root = scanner.src_root().to_path_buf();
        let mut violations: Vec<String> = Vec::new();

        scanner.for_each(|path, production| {
            let rel = path
                .strip_prefix(&src_root)
                .unwrap_or(path)
                .display()
                .to_string();
            if V2A_ALLOWED.contains(&rel.as_str()) {
                return;
            }
            for (idx, line) in production.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if NEEDLES.iter().any(|needle| line.contains(needle)) {
                    violations.push(format!("{rel}:{} — {}", idx + 1, line.trim()));
                }
            }
        });

        assert!(
            violations.is_empty(),
            "mika#1727 V2a — `mika-cli` construit de nouveau un moteur de tâches \
             dans son processus. Le daemon est le seul enregistreur des \
             récurrentes d'un agent (course de mika#2351) ; le remède est de \
             passer par un endpoint de mika-spirit, jamais d'élargir \
             V2A_ALLOWED :\n{}",
            violations.join("\n")
        );
    }

    /// Anti-vacuité (mika#2103, mika#2205) du scan ci-dessus.
    ///
    /// Un balayage qui ne voit rien refuse tout aussi silencieusement qu'un
    /// balayage conforme : il passe la revue, passe la CI par construction, et
    /// laisse la régression atterrir. Les deux fichiers nommés ici sont les
    /// deux moitiés de la propriété — celui d'où le moteur est parti, et le
    /// lecteur légitime de `task_engine::` que le scan doit laisser passer sans
    /// exception.
    #[test]
    fn mika1727_v2a_le_scan_du_moteur_nest_pas_vide() {
        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let files = scanner.files();

        assert!(
            files.len() > 10,
            "le balayage de `mika-cli` doit voir tout son arbre ; vu {} fichier(s)",
            files.len()
        );
        // `commands/` est porté exprès : `Path::ends_with` compare par
        // composants, donc le suffixe nu `tasks.rs` serait satisfait par
        // n'importe quel `tasks.rs` de l'arbre.
        for expected in ["commands/chat.rs", "commands/tasks.rs"] {
            assert!(
                files.iter().any(|p| p.ends_with(expected)),
                "le balayage doit voir {expected} ; sinon la garde du vol V2a \
                 n'inspecte pas ce qu'elle prétend"
            );
        }
    }

    /// L'allowlist du vol V2a est vide, et le reste.
    ///
    /// Sans cette garde, la première régression se réparerait d'une ligne dans
    /// [`V2A_ALLOWED`] et le scan deviendrait ce que mika#2323 refuse : un
    /// recensement de ses propres exceptions. Une entrée ici n'est pas un
    /// réglage, c'est le vol V2a annulé.
    #[test]
    fn mika1727_v2a_lallowlist_est_livree_vide() {
        assert!(
            V2A_ALLOWED.is_empty(),
            "mika#1727 V2a — V2A_ALLOWED est livrée vide et doit le rester : un \
             site qui construit un moteur de tâches dans `mika-cli` se supprime, \
             il ne s'exempte pas. Trouvé : {V2A_ALLOWED:?}"
        );
    }
}
