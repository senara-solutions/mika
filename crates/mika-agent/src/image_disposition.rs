//! Lecteur unique de « que fait-on de cette image ? » (mika#1784).
//!
//! # Le défaut que ce module ferme
//!
//! Al B (testeur famille, 2026-07-20) envoie une photo manuscrite avec la
//! légende « Extrais les numéros de cette photo. » Il reçoit tour à tour
//! « Sorry, I had a hiccup processing your message », puis « Je ne vois pas
//! l'image », puis de nouveau le hiccup. Deux symptômes, une seule question mal
//! posée à quatre endroits.
//!
//! La couche gateway est **innocentée par la lecture** : `routes.rs` télécharge
//! le `file_id`, encode en base64 et POSTe un tableau `images:`, et ses trois
//! échecs de téléchargement ont chacun leur propre message utilisateur, nommément
//! différent du hiccup. Le défaut est en aval, dans l'agent.
//!
//! `LlmProvider::supports_vision()` répond **par rail, jamais par modèle** :
//! `true` pour Anthropic, `true` pour tout `ProviderKind ∈ {OpenAi, OpenRouter,
//! Mistral, Google, DeepSeek}`, `false` partout ailleurs (dont `ZAi`, `Groq`,
//! `Kimi`, `Qwen`, `MiniMax`). Le prédicat est donc **à la fois trop permissif et
//! trop restrictif**, pour la même raison : la vision est une propriété du
//! modèle, et la réponse se décide au niveau du rail. Un `matches!` sur
//! `ProviderKind` ne peut pas distinguer `glm-4.5v` de `glm-5.2` — les deux
//! arrivent par la même porte.
//!
//! D'où les deux symptômes d'Al, selon la configuration effective de son tenant :
//!
//! - `supports_vision() == false` → l'image était **droppée en silence** (un
//!   `warn!` dans un fichier de log, et rien d'autre : ni au modèle, ni à
//!   l'utilisateur). Le modèle recevait la légende sans l'image et répondait
//!   honnêtement qu'il ne la voyait pas. C'est la réponse 2, à la lettre.
//! - `supports_vision() == true` à tort → l'image partait, le provider refusait
//!   la requête, `run_agent` rendait `Err` → le hiccup générique.
//!
//! Ce module ne **corrige pas** `supports_vision()` — le rendre exact demanderait
//! un catalogue de modèles multimodaux qui dérive à chaque sortie de modèle et se
//! trompe dans les deux sens. Il rend ses **deux modes d'erreur inoffensifs**, ce
//! qui est strictement plus fort qu'une frontière mieux placée.
//!
//! # Lecteur unique, et pourquoi c'est structurel
//!
//! [`decide`] est le **seul** site de `crates/mika-agent/src/` à consulter
//! `supports_vision()`, garanti par le scan de source
//! [`tests::mika1784_supports_vision_is_read_only_here`]. Le motif a déjà dû être
//! engravé deux fois dans ce dépôt — `grooming_marker` (mika#2158),
//! `live_pilot` (mika#2279) — après avoir mesuré ce que coûte un prédicat
//! recopié. Ici la divergence était **déjà installée**, et sur deux axes à la
//! fois : sur les quatre sites lisant `user_images`, deux ne consultaient pas le
//! prédicat du tout (et écrivaient `[1 image(s) attached]` en base même quand
//! l'image avait été droppée — une invitation directe à la fabrication au tour
//! suivant), et les deux qui le consultaient le posaient au **mauvais provider**
//! (`llm`, la base, alors que la requête part sur `effective_llm`, l'override
//! `[llm]` d'une skill keyword-matched, cf. `run_loop(effective_llm, …)`).
//!
//! Une garde comportementale ne verrait pas cette classe : un cinquième site
//! recopié ne rendrait aucune décision fausse, il la rendrait divergente.
//!
//! # Ce que ce module ne fait PAS
//!
//! Il ne rend aucun modèle capable de lire une image. Il rend l'incapacité
//! **dite, tracée et sans panne**.

use std::borrow::Cow;

use mika_common::home::PersonaProfile;
use mika_common::llm::error::LlmError;
use mika_common::llm::{LlmImage, LlmProvider};

/// `tool_name` de la ligne `audit_events` écrite sur [`ImageDisposition::Withheld`].
///
/// **SOLE WRITER** : ce module est le seul site à écrire ce `tool_name`, ce qui
/// fait de la requête du bloc 3 la liste exacte plutôt qu'une approximation :
///
/// ```sql
/// SELECT after_value, count(*)
/// FROM audit_events
/// WHERE tool_name = 'image_withheld_no_vision'
/// GROUP BY 1 ORDER BY 2 DESC;
/// ```
pub const IMAGE_WITHHELD_AUDIT_TOOL_NAME: &str = "image_withheld_no_vision";

/// Ce qu'il advient des images utilisateur d'un tour.
///
/// Trois états et pas deux : « aucune image » n'est pas « image retenue », et les
/// confondre ferait écrire une ligne marqueur sur chaque tour sans image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageDisposition {
    /// Aucune image sur ce tour.
    None,
    /// Le provider qui recevra la requête déclare la vision : images transmises.
    Transmitted { count: usize },
    /// Il ne la déclare pas : images retenues.
    Withheld {
        count: usize,
        provider: String,
        model: String,
    },
}

impl ImageDisposition {
    /// Les images de ce tour partent-elles vraiment au modèle ?
    ///
    /// Existe pour que le site d'attachement ne puisse pas écrire
    /// `!matches!(d, Withheld { .. })` et inverser le sens sur la variante
    /// `None` — modèle repris de `LivePilotVerdict::is_alive` (mika#2279).
    #[must_use]
    pub fn transmits(&self) -> bool {
        matches!(self, Self::Transmitted { .. })
    }

    /// Nombre d'images reçues de l'utilisateur, transmises ou non.
    #[must_use]
    pub fn count(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Transmitted { count } | Self::Withheld { count, .. } => *count,
        }
    }
}

/// Décide du sort des images de ce tour.
///
/// `llm` **DOIT** être le provider qui recevra la requête (`effective_llm`),
/// jamais le provider de base : un tour servi par un override `[llm]` de skill
/// décidait jusqu'ici de la vision d'après un provider qui n'est pas celui qui
/// reçoit la requête, ce qui produit les deux modes d'erreur ci-dessus **sans
/// même changer de configuration**.
///
/// Fonction pure : aucune écriture, aucun log. Les effets de bord (ligne
/// marqueur, `audit_events`, `warn!`) appartiennent à l'appelant, qui est le seul
/// à tenir la base et la session.
#[must_use]
pub fn decide(images: &[LlmImage], llm: &dyn LlmProvider) -> ImageDisposition {
    if images.is_empty() {
        return ImageDisposition::None;
    }
    let count = images.len();
    if llm.supports_vision() {
        ImageDisposition::Transmitted { count }
    } else {
        ImageDisposition::Withheld {
            count,
            provider: llm.provider_name().to_string(),
            model: llm.model_name().to_string(),
        }
    }
}

/// Ce qui est persisté sur le message utilisateur, aux deux sites de persistance.
///
/// La disposition **n'est pas connaissable** à ce moment : le provider effectif
/// dépend du skill matching, qui n'a pas encore eu lieu. Donc on cesse
/// d'affirmer et on se borne au fait certain — `received`, vrai dans les deux
/// cas, muet sur ce qu'on ne peut pas savoir. Le mot `attached` affirmait la
/// transmission avant qu'elle soit décidée, et l'historique répétait ensuite
/// cette affirmation au modèle à chaque tour suivant.
#[must_use]
pub fn user_message_save_text(user_message: &str, image_count: usize) -> Cow<'_, str> {
    if image_count == 0 {
        return Cow::Borrowed(user_message);
    }
    Cow::Owned(format!("[{image_count} image(s) received]\n{user_message}"))
}

/// La ligne `role = "system"` persistée sur [`ImageDisposition::Withheld`].
///
/// Persistée **entre la résolution d'`effective_llm` et le rechargement de
/// l'historique**, de sorte que ce rechargement la reprenne : un seul geste ferme
/// le tour courant *et* tous les suivants. Le rôle `system` est déjà le véhicule
/// des marqueurs de contexte du substrat (converti en `User` pour les providers,
/// « e.g. rewind notices ») — aucun mécanisme nouveau.
///
/// **Anglais, comme tous les marqueurs de contexte** : il s'adresse au modèle,
/// pas à l'utilisateur. À ne pas confondre avec [`image_refused_reply`], qui
/// s'adresse à l'utilisateur et suit donc `PersonaProfile`.
///
/// C'est la moitié *intent* de l'AC2, et elle est structurelle : le fait vient du
/// substrat, pas d'une règle de prompt. Cf.
/// `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
/// (mika#2120) — une consigne « n'invente pas le contenu d'une image que tu n'as
/// pas reçue » aurait exactement la durée de vie que ce retour d'expérience lui
/// prédit.
#[must_use]
pub fn withheld_context_marker(count: usize) -> String {
    let (subject, was, is, its) = if count == 1 {
        ("1 image".to_string(), "was", "is", "its")
    } else {
        (format!("{count} images"), "were", "are", "their")
    };
    format!(
        "[Image not transmitted: the active model does not read images. {subject} {was} \
         received from the user and {is} NOT available to you. Do not describe or infer \
         {its} contents; say plainly that you cannot read images.]"
    )
}

/// Le provider a-t-il refusé la requête **à cause de l'image** ?
///
/// Rend le statut HTTP quand oui, `None` sinon. L'attribution est **conjonctive
/// et étroite** :
///
/// - le tour portait au moins une image, **et**
/// - l'erreur est un [`LlmError::HttpError`] dont `status` est dans `400..=499`,
///   **429 exclu**.
///
/// Tout le reste — transport, timeout, 5xx, 429, `parse`, `provider`, `other` —
/// garde le hiccup générique inchangé.
///
/// **Pourquoi cette conjonction et pas une plus large.** Un 4xx sur un tour
/// portant une image a deux causes plausibles : le rejet multimodal et le
/// dépassement de taille. Les deux appellent le même message. Un 5xx ou un
/// timeout n'apprend rien sur l'image et l'attribuer serait une fausse
/// attribution — le défaut que ce dépôt combat sous le nom de fabrication. Un 429
/// est une limitation de débit, sans rapport. La règle : *une erreur n'est
/// attribuée à l'image que lorsque le provider a refusé la requête.*
///
/// La **décision** lit le variant (`downcast_ref`, qui traverse toute la chaîne
/// de causes `anyhow`), jamais la chaîne `error_class()` — en extraire « est-ce
/// un 4xx ? » demanderait de re-parser `"http_400"`, c'est-à-dire exactement la
/// correspondance par sous-chaîne que ce dépôt proscrit. La **trace**, elle,
/// porte `error_class()`, le vocabulaire de fil partagé (mika#2331 D3), pour que
/// le `GROUP BY` de l'opérateur tombe dans la même population que les autres
/// classificateurs.
///
/// **Angle mort assumé, nommé plutôt que découvert.** `llm/anthropic.rs` aplatit
/// toute erreur de son rail en [`LlmError::ProviderError`]. Un 400 Anthropic ne
/// serait donc pas un `HttpError` et retomberait sur le hiccup générique. C'est
/// sans effet ici : ce rail déclare la vision et la supporte réellement, donc il
/// ne produit pas la population que ce prédicat vise. Corriger l'aplatissement
/// changerait la classe d'erreur vue par tout le moteur sur ce rail — un format
/// de fil — et mérite son propre ticket.
#[must_use]
pub fn image_refusal_status(err: &anyhow::Error, image_count: usize) -> Option<u16> {
    if image_count == 0 {
        return None;
    }
    match err.downcast_ref::<LlmError>() {
        Some(LlmError::HttpError { status, .. })
            if (400..=499).contains(status) && *status != 429 =>
        {
            Some(*status)
        }
        _ => None,
    }
}

/// Le message utilisateur servi quand [`image_refusal_status`] a attribué.
///
/// Deux écritures choisies sur `PersonaProfile`, par `match` exhaustif **sans
/// bras `_ =>`** : le compilateur force toute nouvelle persona à décider. Motif
/// déjà tranché par mika#2290 pour la phrase d'hébergement, et **aucune règle
/// n'est dérivée de la locale du compte** (arbitrage de Prime, 2026-09-09).
///
/// Al étant un testeur famille francophone, servir de l'anglais technique sur le
/// tour d'échec fait partie du défaut qu'il a subi, pas d'un détail cosmétique.
#[must_use]
pub fn image_refused_reply(persona: PersonaProfile) -> &'static str {
    match persona {
        PersonaProfile::Operator => {
            "I couldn't process that image — the model serving this conversation rejected the \
             request. I can't read images right now, so I won't guess at what it shows. \
             Describe it in text and I'll help from there."
        }
        PersonaProfile::Family => {
            "Je n'arrive pas à lire cette image — je ne sais pas encore regarder les photos. \
             Je préfère te le dire plutôt que d'inventer ce qu'il y a dessus. \
             Si tu m'écris ce qu'elle contient, je t'aide tout de suite."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mika_common::llm::mock::MockLlmProvider;

    fn img() -> LlmImage {
        LlmImage {
            media_type: "image/jpeg".to_string(),
            data: "AAAA".to_string(),
        }
    }

    fn provider(supports_vision: bool) -> MockLlmProvider {
        MockLlmProvider::builder()
            .supports_vision(supports_vision)
            .provider_name("zai")
            .model_name("glm-5.2")
            .build()
    }

    // ── decide(), les trois variantes ──

    #[test]
    fn no_image_is_none_whatever_the_provider_declares() {
        for vision in [true, false] {
            assert_eq!(decide(&[], &provider(vision)), ImageDisposition::None);
        }
    }

    #[test]
    fn a_vision_provider_transmits() {
        let d = decide(&[img(), img()], &provider(true));
        assert_eq!(d, ImageDisposition::Transmitted { count: 2 });
        assert!(d.transmits());
        assert_eq!(d.count(), 2);
    }

    #[test]
    fn a_non_vision_provider_withholds_and_names_itself() {
        let d = decide(&[img()], &provider(false));
        assert_eq!(
            d,
            ImageDisposition::Withheld {
                count: 1,
                provider: "zai".to_string(),
                model: "glm-5.2".to_string(),
            }
        );
        // Le site d'attachement ne doit attacher que sur `Transmitted`.
        assert!(!d.transmits());
    }

    /// `None` ne transmet pas non plus — la propriété qui rend `transmits()`
    /// utilisable comme condition d'attachement sans réintroduire le bug.
    #[test]
    fn none_does_not_transmit() {
        assert!(!ImageDisposition::None.transmits());
        assert_eq!(ImageDisposition::None.count(), 0);
    }

    // ── le texte persisté sur le message utilisateur ──

    /// La garde du cœur du point (3) : aucune affirmation de transmission ne peut
    /// plus être émise avant que la disposition soit connue.
    #[test]
    fn mika1784_the_persisted_user_message_never_claims_attachment() {
        let text = user_message_save_text("Extrais les numéros de cette photo.", 1);
        assert!(text.starts_with("[1 image(s) received]\n"));
        assert!(
            !text.contains("attached"),
            "l'historique affirmait une pièce jointe que le modèle n'a jamais reçue — \
             c'est l'invitation à la fabrication que l'AC2 demande de fermer"
        );
    }

    #[test]
    fn without_images_the_user_message_is_untouched_and_not_reallocated() {
        let text = user_message_save_text("bonjour", 0);
        assert_eq!(text, "bonjour");
        assert!(matches!(text, Cow::Borrowed(_)));
    }

    // ── la ligne marqueur ──

    #[test]
    fn the_marker_names_the_fact_and_forbids_inference() {
        let one = withheld_context_marker(1);
        assert!(one.contains("1 image was received"), "{one}");
        assert!(one.contains("is NOT available to you"), "{one}");
        assert!(one.contains("Do not describe or infer"), "{one}");

        let three = withheld_context_marker(3);
        assert!(three.contains("3 images were received"), "{three}");
        assert!(three.contains("are NOT available to you"), "{three}");
    }

    // ── attribution du bloc 2 : les sept classes d'erreur ──

    fn anyhowed(e: LlmError) -> anyhow::Error {
        // Enveloppé dans une chaîne de causes, comme en production : `downcast_ref`
        // doit la traverser, pas seulement lire la couche supérieure.
        anyhow::Error::new(e).context("agent loop failed")
    }

    #[test]
    fn mika1784_only_a_provider_refusal_on_an_image_turn_is_attributed() {
        let refusal = anyhowed(LlmError::HttpError {
            status: 400,
            message: "model does not support image input".into(),
            retryable: false,
        });
        assert_eq!(image_refusal_status(&refusal, 1), Some(400));
        assert_eq!(image_refusal_status(&refusal, 3), Some(400));
    }

    /// Les négatifs, un par classe. Chacun doit **conserver** le hiccup
    /// générique : attribuer un timeout ou un 5xx à l'image serait une fausse
    /// attribution, c'est-à-dire le défaut que ce dépôt combat sous le nom de
    /// fabrication.
    #[test]
    fn mika1784_every_other_error_class_keeps_the_generic_hiccup() {
        let cases: Vec<(&str, LlmError)> = vec![
            (
                "http_429 — limitation de débit, sans rapport avec l'image",
                LlmError::HttpError {
                    status: 429,
                    message: "slow down".into(),
                    retryable: true,
                },
            ),
            (
                "http_500 — le provider est tombé, il n'apprend rien sur l'image",
                LlmError::HttpError {
                    status: 500,
                    message: "upstream".into(),
                    retryable: true,
                },
            ),
            (
                "http_503",
                LlmError::HttpError {
                    status: 503,
                    message: "unavailable".into(),
                    retryable: true,
                },
            ),
            (
                "transport_timeout — la signature de mika#2179",
                LlmError::Transport("failed to read response body: operation timed out".into()),
            ),
            (
                "transport",
                LlmError::Transport("connection refused".into()),
            ),
            ("parse", LlmError::ParseError("bad json".into())),
            (
                "provider — dont l'aplatissement du rail Anthropic",
                LlmError::ProviderError("upstream said no".into()),
            ),
            ("unsupported", LlmError::UnsupportedFeature("vision".into())),
        ];
        for (label, err) in cases {
            assert_eq!(
                image_refusal_status(&anyhowed(err), 1),
                None,
                "attribué à tort : {label}"
            );
        }
    }

    /// Le second terme de la conjonction, seul : un 4xx sur un tour **sans**
    /// image n'a rien à voir avec une image.
    #[test]
    fn mika1784_a_refusal_without_images_is_not_attributed() {
        let refusal = anyhowed(LlmError::HttpError {
            status: 400,
            message: "bad request".into(),
            retryable: false,
        });
        assert_eq!(image_refusal_status(&refusal, 0), None);
    }

    /// Une erreur qui n'est pas un `LlmError` du tout (classe `other`).
    #[test]
    fn a_non_llm_error_is_not_attributed() {
        let err = anyhow::anyhow!("disk full");
        assert_eq!(image_refusal_status(&err, 1), None);
    }

    /// Les bornes exactes de la fenêtre 4xx, 429 troué au milieu.
    #[test]
    fn the_attributed_window_is_400_to_499_with_429_punched_out() {
        let attributed = |status: u16| {
            image_refusal_status(
                &anyhowed(LlmError::HttpError {
                    status,
                    message: String::new(),
                    retryable: false,
                }),
                1,
            )
            .is_some()
        };
        assert!(!attributed(399));
        assert!(attributed(400));
        assert!(attributed(413), "dépassement de taille — même message");
        assert!(attributed(428));
        assert!(!attributed(429));
        assert!(attributed(430));
        assert!(attributed(499));
        assert!(!attributed(500));
    }

    // ── registre ──

    #[test]
    fn mika1784_the_two_registers_are_distinct_and_neither_fabricates() {
        let operator = image_refused_reply(PersonaProfile::Operator);
        let family = image_refused_reply(PersonaProfile::Family);
        assert_ne!(operator, family);
        // Le tenant famille est francophone : lui servir de l'anglais technique
        // sur le tour d'échec fait partie du défaut qu'Al a subi.
        assert!(
            family.contains("Je n'arrive pas à lire cette image"),
            "{family}"
        );
        // AC1 branche B : ni « hiccup », ni fabrication.
        for reply in [operator, family] {
            assert!(!reply.to_lowercase().contains("hiccup"), "{reply}");
        }
    }

    // ── les gardes structurelles ──

    /// `decide()` est appelée avec le provider qui REÇOIT la requête.
    ///
    /// C'est le point (4) du diagnostic, et **une garde comportementale ne peut
    /// pas le voir** : passer `llm` au lieu d'`effective_llm` rend une décision
    /// parfaitement plausible — simplement celle d'un autre provider — et toutes
    /// les assertions existantes restent vertes, parce que dans un harness la
    /// base et l'effectif sont le même objet. Le défaut n'apparaît que chez un
    /// tenant dont un tour est servi par un override `[llm]` de skill, où il
    /// reproduit les deux modes d'erreur d'Al **sans même changer de
    /// configuration**.
    ///
    /// Monter un vrai override de bout en bout demanderait un provider HTTP réel
    /// construit depuis `Settings`, donc un appel sortant : hors de proportion
    /// pour épingler quel identifiant est passé en second argument.
    ///
    /// Note de périmètre : `run_team_agent` résout son propre `effective_llm` au
    /// même motif et n'est **pas** concerné — `user_images` est un champ
    /// d'`AgentParams` seul, et ni le mode team ni le mode silent ne portent
    /// d'images utilisateur.
    #[test]
    fn mika1784_decide_is_called_with_the_provider_that_receives_the_request() {
        let agent_loop =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent_loop/mod.rs");
        let content = std::fs::read_to_string(&agent_loop)
            .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", agent_loop.display()));

        let call_sites: Vec<(usize, &str)> = content
            .lines()
            .enumerate()
            .filter(|(_, l)| l.contains("image_disposition::decide("))
            .filter(|(_, l)| !l.trim_start().starts_with("//"))
            .map(|(n, l)| (n + 1, l.trim()))
            .collect();

        assert_eq!(
            call_sites.len(),
            1,
            "un seul site de décision est attendu dans la boucle ; trouvés : {call_sites:?}"
        );

        // L'argument est sur la ligne suivante quand rustfmt coupe l'appel.
        let (line_no, _) = call_sites[0];
        let window: String = content
            .lines()
            .skip(line_no - 1)
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            window.contains("effective_llm"),
            "mika#1784 point (4) — `decide()` doit recevoir `effective_llm`, le provider \
             qui recevra vraiment la requête (`run_loop(effective_llm, …)`), et jamais \
             `llm`, le provider de base. Les deux ne diffèrent que sur un tour servi par \
             un override `[llm]` de skill — c'est-à-dire précisément la population qu'aucun \
             test ne couvre.\nSite trouvé : {window}"
        );
    }

    // ── la garde du lecteur unique ──

    /// `supports_vision()` ne se lit qu'ici.
    ///
    /// Une garde **comportementale** ne verrait pas cette classe : un cinquième
    /// site recopié ne rendrait aucune décision fausse, il la rendrait
    /// divergente — et c'est exactement ce qui était installé avant mika#1784
    /// (deux sites qui ne consultaient pas le prédicat, deux qui le posaient au
    /// mauvais provider). Même motif que
    /// `grooming_marker::tests::no_grooming_regex_outside_this_module`
    /// (mika#2158) et `live_pilot` (mika#2279).
    ///
    /// Aucune liste d'exemptions : une allowlist née vide est l'endroit où l'on
    /// range la prochaine violation. Un site supplémentaire est un
    /// halt-and-surface — appelez [`decide`].
    #[test]
    fn mika1784_supports_vision_is_read_only_here() {
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_module = src_root.join("image_disposition.rs");

        let mut offenders = Vec::new();
        let mut stack = vec![src_root.clone()];
        let mut scanned = 0usize;

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("la garde doit pouvoir lire {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("entrée de répertoire lisible").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path == this_module {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                    panic!("la garde doit pouvoir lire {}: {e}", path.display())
                });
                scanned += 1;
                for (n, line) in content.lines().enumerate() {
                    let code = line.trim_start();
                    // Les lignes de doc et de commentaire ont le droit de nommer
                    // le prédicat — c'est ainsi qu'on explique pourquoi il ne se
                    // lit qu'ici.
                    if code.starts_with("//") {
                        continue;
                    }
                    if code.contains("supports_vision") {
                        offenders.push(format!(
                            "{}:{}: {}",
                            path.strip_prefix(&src_root).unwrap_or(&path).display(),
                            n + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }

        assert!(
            scanned > 0,
            "la garde n'a scanné aucun fichier — chemin cassé"
        );
        assert!(
            offenders.is_empty(),
            "mika#1784 — `supports_vision()` se lit hors de `image_disposition.rs`. \
             Le prédicat répond par rail et non par modèle : recopié, il diverge sans \
             qu'aucune assertion ne rougisse. Appelez `image_disposition::decide` avec le \
             provider qui RECEVRA la requête (`effective_llm`).\n{}",
            offenders.join("\n")
        );
    }
}
