//! L'unique site par lequel un tour de la TUI atteint mika-spirit (mika#1727, V1).
//!
//! # Pourquoi un module, et pas une fonction dans `chat.rs`
//!
//! Le worker de la TUI vit dans `chat::spawn_agent_worker`, qui exige un
//! `AppContext` complet — base de données, provider LLM, MCP, `TaskEngine`.
//! Rien de ce bras n'est atteignable depuis un test. L'envoi est donc extrait
//! ici, au périmètre, avec ses tests en ligne : c'est la seule forme où
//! « le tour part bien sur le fil, avec la session du tour » est une propriété
//! mesurée plutôt qu'une lecture à l'œil.
//!
//! # Ce que ce module ne fait pas
//!
//! Il ne rend rien à l'écran et ne connaît pas l'`App` : il rend un
//! [`TurnReply`] que `chat.rs` convertit en `AgentResponse`. Il n'a pas non plus
//! de décodeur à lui — les jetons passent par
//! [`mika_a2a::params::attested_run_usage`], le lecteur que les deux portes de
//! `mika ask` partagent déjà (mika#1883). Une seconde lecture de la même clé est
//! comment deux surfaces finissent par répondre différemment à la même question.

use anyhow::Result;

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
fn spirit_turn_endpoint(agent_name: &str) -> String {
    format!(
        "{}/a2a/{}",
        crate::commands::dashboard::spirit_url(),
        agent_name
    )
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
/// Les trois autres paramètres de `send_message_to_agent` sont posés vides à
/// dessein : pas de restriction de skills, pas d'override de modèle (`/model`
/// reste appliqué en processus en V1, voir `chat.rs`), pas d'isolation de
/// session — un tour de TUI veut précisément voir ses tours précédents.
pub(crate) async fn send_turn_to_spirit(
    text: &str,
    agent_name: &str,
    session_id: &str,
) -> Result<TurnReply> {
    let endpoint = spirit_turn_endpoint(agent_name);
    send_turn_to_endpoint(text, &endpoint, session_id).await
}

/// Le corps de [`send_turn_to_spirit`], l'endpoint déjà résolu.
///
/// Séparé pour une seule raison : les tests visent un serveur factice, et
/// l'alternative serait de muter `MIKA_SPIRIT_URL` — un état global de
/// processus que les tests `cargo` parallèles se disputent.
async fn send_turn_to_endpoint(text: &str, endpoint: &str, session_id: &str) -> Result<TurnReply> {
    let task = mika_cli::remote_ask::send_message_to_agent(
        text,
        endpoint,
        Some(session_id),
        &[],
        None,
        false,
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
        let reply = send_turn_to_endpoint("quoi de neuf", &endpoint, "sess-t1")
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
        send_turn_to_endpoint("salut", &endpoint, "sess-t2-abc")
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

        let err = send_turn_to_endpoint("salut", &endpoint, "sess-t3")
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
        let outcome = send_turn_to_endpoint("salut", &endpoint, "sess-t4").await;
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
        let reply = send_turn_to_endpoint("salut", &endpoint, "sess-t5a")
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
        let reply = send_turn_to_endpoint("salut", &endpoint, "sess-t5b")
            .await
            .expect("envoi");
        assert_eq!(reply.input_tokens, Some(41_000));
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
}
