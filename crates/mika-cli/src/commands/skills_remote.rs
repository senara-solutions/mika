//! Le transport de la route des skills de mika-spirit
//! (`GET /api/v1/skills?agent=<nom>`, mika#1727 vol V3c-2a-ii) : les cinq
//! miroirs du fil, un vocabulaire de refus à trois variants, et la porte
//! [`lire`]. L'URL de base et le jeton arrivent **par argument**, pour la raison
//! déjà écrite sur [`super::rewind_remote`] : un client qui résout ses propres
//! accès ne se pilote pas depuis un test. Le jeton ne se lit donc pas ici —
//! `session_remote::jeton_interne` reste la seule lecture du crate, et c'est
//! l'appelant de V3c-2b qui l'y prendra.
//!
//! **Rien n'est mis en commun avec les deux transports voisins.** Une entrée
//! d'egress déclare **un** site de construction de client, et sa ligne `data`
//! décrit ce qui part de ce site ; la moitié de ce raisonnement est écrite sur
//! l'en-tête de [`super::session_remote`], l'autre sur `rewind_remote::post`.
//!
//! **État du vol.** La porte est posée et elle n'a **aucun appelant de
//! production** : les items sans lecteur portent une attente `dead_code` que
//! V3c-2b retire en câblant la TUI sur cette route — `app.skills` cesse alors
//! d'être assemblé localement. V3c-2a-iii éclate avant cela les deux refus
//! métier, 404 agent inconnu et 401 jeton refusé, qui sortent d'ici par
//! [`SkillsRemoteIssue::Statut`] en attendant.

use serde::Deserialize;
use std::collections::BTreeMap;

// Les cinq miroirs du fil. La réponse de `mika-agent/src/server/dashboard.rs`
// est construite par `serde_json::json!` et n'a donc aucun type côté serveur :
// ces structs **sont** le contrat de fil côté client, sans règle de domaine.
// Les champs portent le nom que la route leur donne — `tools` reste le
// **compte** des outils d'une skill, `tool_names` leurs noms — parce qu'un
// miroir qui renomme oblige chacun de ses lecteurs à tenir la correspondance.

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")
)]
#[derive(Debug, Deserialize)]
pub(crate) struct SkillsMirror {
    pub(crate) agent: String,
    pub(crate) skills: Vec<SkillMirror>,
    pub(crate) skipped: Vec<SkippedMirror>,
    pub(crate) warnings: Vec<WarningMirror>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")
)]
#[derive(Debug, Deserialize)]
pub(crate) struct SkillMirror {
    pub(crate) name: String,
    pub(crate) description: String,
    /// `"built-in"` ou `"marketplace"`, tel que la route le calcule. Un `&str`
    /// de deux valeurs et non un enum : la route ne publie pas ce vocabulaire
    /// comme fermé, et un variant de plus côté serveur ferait ici un
    /// `CorpsIllisible` là où la TUI n'a qu'une étiquette à afficher.
    pub(crate) origin: String,
    pub(crate) enabled: bool,
    pub(crate) always_on: bool,
    pub(crate) tools: usize,
    pub(crate) has_override: bool,
    pub(crate) version: String,
    /// Le timeout **racine** du manifeste, celui que la route sert : elle ne
    /// connaît pas le couple provider/modèle de la session appelante et n'en
    /// invente pas un.
    pub(crate) timeout_secs: u64,
    pub(crate) keywords: Vec<String>,
    pub(crate) tool_names: Vec<String>,
    pub(crate) path: String,
    pub(crate) variants: VariantsMirror,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")
)]
#[derive(Debug, Deserialize)]
pub(crate) struct VariantsMirror {
    pub(crate) total: usize,
    /// `BTreeMap` et non `HashMap` : `/skill <nom>` rend le détail par provider
    /// dans la même ligne que le total, et un ordre d'itération qui bouge d'un
    /// affichage à l'autre se lit comme un changement du registre.
    pub(crate) providers: BTreeMap<String, usize>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")
)]
#[derive(Debug, Deserialize)]
pub(crate) struct SkippedMirror {
    pub(crate) name: String,
    pub(crate) reason: String,
}

/// Un avertissement de validation. Pas de champ `level`, et ce n'est pas un
/// oubli : la route n'en sert pas, parce que tout ce qui atteint cette liste est
/// un avertissement par construction — un diagnostic n'est enregistré que sur
/// une skill restée chargée.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")
)]
#[derive(Debug, Deserialize)]
pub(crate) struct WarningMirror {
    pub(crate) name: String,
    pub(crate) messages: Vec<String>,
}

/// Le vocabulaire de refus de cette route, et **pas**
/// [`super::session_remote::SessionRemoteIssue`] : le `NotFound` de ce voisin
/// écrit « unknown agent, or unknown session », là où un 404 d'ici ne peut
/// vouloir dire qu'**agent inconnu**. Un message qui nomme une session
/// enverrait l'opérateur chercher la panne là où elle n'est pas.
///
/// Trois variants, là où les deux voisins rangent une erreur de transport dans
/// `Unexpected { status: None, … }` : `Injoignable` ne **peut pas** porter de
/// statut, et c'est le type qui le dit plutôt qu'un `Option<u16>` qu'un lecteur
/// aurait à discriminer. `CorpsIllisible` est un refus et jamais un succès — un
/// 200 dont le corps ne se désérialise pas sort par `Err`, pas en miroir à
/// champs vides.
///
/// 404 et 401 sortent par `Statut` dans ce vol, et **aucun bras mort** ne les
/// attend : V3c-2a-iii les éclate avec leurs tests.
#[derive(Debug)]
pub(crate) enum SkillsRemoteIssue {
    Injoignable {
        url: String,
        cause: String,
    },
    AgentInconnu {
        agent: String,
        url: String,
    },
    Statut {
        status: u16,
        url: String,
        body: String,
    },
    CorpsIllisible {
        url: String,
        body: String,
        cause: String,
    },
}

// `Display` à la main : rien à chaîner, aucune frontière de bibliothèque.
impl std::fmt::Display for SkillsRemoteIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Injoignable { url, cause } => {
                write!(f, "mika-spirit is unreachable at {url}: {cause}")
            }
            // Le nom vient de l'argument de `lire`, jamais du corps du 404 : le
            // serveur y renvoie une reformulation du nom qu'on vient d'envoyer,
            // et la relire coûterait une désérialisation qui peut échouer — un
            // spirit d'une autre version, un proxy — pour retrouver une chaîne
            // que l'appelant détient déjà. Et cet échec-là rendrait un message
            // sans nom d'agent, là où le nom est le seul fait utile.
            //
            // La limite, dite ici plutôt que gardée : un 404 ne vaut « agent
            // inconnu » que servi par un spirit qui monte bien cette route. Un
            // spirit qui ne la monterait pas du tout rendrait aussi un 404, et
            // ce message nommerait l'agent à tort — distinction que le serveur
            // lui-même ne fait pas, donc que le client ne peut pas faire.
            Self::AgentInconnu { agent, url } => {
                write!(
                    f,
                    "mika-spirit does not serve an agent named '{agent}' at {url}"
                )
            }
            Self::Statut { status, url, body } => {
                write!(f, "mika-spirit returned {status} for {url}: {body}")
            }
            // Le corps reçu voyage dans le message avec la cause : la cause
            // nomme le champ qui manque, mais seul le corps dit de quelle
            // réponse il vient — et c'est elle que l'opérateur doit montrer.
            Self::CorpsIllisible { url, body, cause } => write!(
                f,
                "mika-spirit answered {url} with a body this client cannot read: {cause} (body: {body})"
            ),
        }
    }
}

/// Ce que mika-spirit sert du registre de skills d'un agent, rendu tel que la
/// route le nomme.
///
/// Le nom d'agent part par `.query(&[("agent", agent)])`, donc **reqwest
/// encode** : un nom portant un caractère réservé arrive intact sans qu'un
/// `format!` ait à le savoir. C'est la différence qu'un `format!` ne ferait pas
/// voir — il passerait sur `mika-dev` et casserait sur `mika/dev`.
///
/// Pas de `skills_url(base_url)` à part, contrairement à
/// `rewind_remote::rewind_url` : il n'y a qu'**une** route ici, donc le
/// `trim_end_matches('/')` qui absorbe un `base_url` à barre finale n'a pas deux
/// lecteurs à tenir d'accord.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "câblé au vol V3c-2b de mika#1727")
)]
pub(crate) async fn lire(
    base_url: &str,
    token: &str,
    agent: &str,
) -> Result<SkillsMirror, SkillsRemoteIssue> {
    let url = format!("{}/api/v1/skills", base_url.trim_end_matches('/'));
    // Client construit ici, au site de l'appel : c'est ce que déclare l'entrée
    // d'egress `cli-spirit-skills`, qu'un client en argument rendrait aveugle.
    // Le raisonnement entier est sur `rewind_remote::post`.
    let reponse = reqwest::Client::new()
        .get(&url)
        .query(&[("agent", agent)])
        .header("authorization", format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|cause| SkillsRemoteIssue::Injoignable {
            url: url.clone(),
            cause: cause.to_string(),
        })?;

    // Statut lu avant le corps, comme les deux transports voisins : un corps
    // d'erreur désérialisé comme un succès est la panne à ne pas produire.
    let statut = reponse.status().as_u16();
    let corps = reponse.text().await.unwrap_or_default();
    if statut != 200 {
        // Le `if` est gardé et le `match` niché dedans : à plat, un
        // `match statut { 200 => {}, … }` réécrirait le chemin de succès pour
        // deux cas d'erreur, et la revue aurait à relire un diff là où rien
        // n'a changé. Le bras `_` reste `Statut` et reste le dernier, donc il
        // est le seul à consommer `corps` et aucun `clone` n'est nécessaire.
        return Err(match statut {
            404 => SkillsRemoteIssue::AgentInconnu {
                agent: agent.to_string(),
                url,
            },
            _ => SkillsRemoteIssue::Statut {
                status: statut,
                url,
                body: corps,
            },
        });
    }
    serde_json::from_str(&corps).map_err(|cause| SkillsRemoteIssue::CorpsIllisible {
        url,
        body: corps.clone(),
        cause: cause.to_string(),
    })
}

// Les tests vivent ici, **en fin de fichier** : le scanner d'egress coupe la
// production au premier `#[cfg(test)]`, et un item de production placé après en
// sortirait — le lint d'egress ne le verrait plus. `rewind_remote.rs` porte la
// même note, répétée ici plutôt que pointée : un lecteur qui déplace un item ne
// lit pas le fichier d'à côté.
#[cfg(test)]
mod tests {
    use super::*;
    // Banc **emprunté** au voisin, et non recopié : un second témoin de socket
    // ouvrirait une seconde vérité sur ce que le serveur reçoit, libre d'en
    // diverger. Il est `pub(crate)` depuis V2c-1b, pour ce motif-là.
    use crate::commands::rewind_remote::tests::{RequeteRecue, fake_spirit};
    use axum::http::StatusCode;
    use mika_common::dead_endpoint::DeadEndpoint;
    use std::sync::{Arc, Mutex};

    const ROUTE: &str = "/api/v1/skills";
    const JETON: &str = "jeton-interne";

    /// La forme que la route sert, partagée par les tests de ce fichier plutôt
    /// que recopiée dans chacun : deux skills — une de chaque origine, et la
    /// seconde à l'autre valeur de chaque booléen —, une ignorée et un
    /// avertissement. Recopiée par test, elle divergerait là où le contrat de
    /// fil est un.
    const CORPS: &str = r#"{
      "agent": "mika-dev",
      "skills": [
        {"name": "self-dev", "description": "orchestration", "origin": "built-in",
         "enabled": true, "always_on": false, "tools": 2, "has_override": true,
         "version": "0.3.1", "timeout_secs": 600, "keywords": ["dev", "loop"],
         "tool_names": ["run_claude_pilot", "deploy_mika"], "path": "/skills/self-dev",
         "variants": {"total": 3, "providers": {"anthropic": 2, "groq": 0}}},
        {"name": "weather", "description": "", "origin": "marketplace",
         "enabled": false, "always_on": true, "tools": 0, "has_override": false,
         "version": "1.0.0", "timeout_secs": 30, "keywords": [], "tool_names": [],
         "path": "/skills/weather", "variants": {"total": 0, "providers": {}}}
      ],
      "skipped": [{"name": "broken", "reason": "manifest parse error"}],
      "warnings": [{"name": "self-dev", "messages": ["two handlers share a keyword"]}]
    }"#;

    /// Un factice qui ne sert qu'un couple sur la route des skills, et l'appel
    /// que [`lire`] y fait. L'URL de base revient avec, parce que les messages
    /// de refus la nomment, et le témoin aussi, parce que la visée se mesure sur
    /// la socket et non sur le rendu.
    async fn appel(
        statut: StatusCode,
        corps: &str,
        agent: &str,
    ) -> (
        String,
        Arc<Mutex<Vec<RequeteRecue>>>,
        Result<SkillsMirror, SkillsRemoteIssue>,
    ) {
        let (base, recues) = fake_spirit(&[(ROUTE, statut, corps)]).await;
        let rendu = lire(&base, JETON, agent).await;
        (base, recues, rendu)
    }

    #[tokio::test]
    async fn mika1727_le_miroir_relit_tout_ce_que_la_route_sert() {
        let (_, recues, rendu) = appel(StatusCode::OK, CORPS, "mika-dev").await;
        let miroir = rendu.expect("le factice sert la forme de la route");

        assert_eq!(miroir.agent, "mika-dev");
        assert_eq!(miroir.skills.len(), 2);
        let premiere = &miroir.skills[0];
        assert_eq!(premiere.name, "self-dev");
        assert_eq!(premiere.description, "orchestration");
        assert_eq!(premiere.origin, "built-in");
        assert!(premiere.enabled);
        assert!(!premiere.always_on);
        // Le compte et les noms relus côte à côte : c'est l'écart que la route
        // publie, et un miroir qui les confondrait passerait tout autre test.
        assert_eq!(premiere.tools, 2);
        assert_eq!(premiere.tool_names, ["run_claude_pilot", "deploy_mika"]);
        assert!(premiere.has_override);
        assert_eq!(premiere.version, "0.3.1");
        assert_eq!(premiere.timeout_secs, 600);
        assert_eq!(premiere.keywords, ["dev", "loop"]);
        assert_eq!(premiere.path, "/skills/self-dev");
        assert_eq!(premiere.variants.total, 3);
        // Un provider sans modèle arrive à `0`, pas absent : la route fait la
        // distinction, donc le miroir doit la porter.
        assert_eq!(premiere.variants.providers.get("anthropic"), Some(&2));
        assert_eq!(premiere.variants.providers.get("groq"), Some(&0));

        // La seconde skill porte l'autre valeur de chaque booléen et des
        // collections vides : sans elle, un miroir à défauts constants se
        // relirait juste.
        let seconde = &miroir.skills[1];
        assert_eq!(seconde.origin, "marketplace");
        assert!(!seconde.enabled);
        assert!(seconde.always_on);
        assert!(seconde.keywords.is_empty());
        assert!(seconde.variants.providers.is_empty());

        assert_eq!(miroir.skipped.len(), 1);
        assert_eq!(miroir.skipped[0].name, "broken");
        assert_eq!(miroir.skipped[0].reason, "manifest parse error");
        assert_eq!(miroir.warnings.len(), 1);
        assert_eq!(miroir.warnings[0].name, "self-dev");
        assert_eq!(
            miroir.warnings[0].messages,
            ["two handlers share a keyword"]
        );

        // La visée est attestée dans le **même** test : un miroir relu d'une
        // requête partie ailleurs ne prouve rien.
        let vues = recues.lock().unwrap();
        assert_eq!(vues.len(), 1);
        assert_eq!(vues[0].methode, "GET");
        assert_eq!(vues[0].chemin, ROUTE);
        assert_eq!(vues[0].requete.as_deref(), Some("agent=mika-dev"));
        assert_eq!(
            vues[0].autorisation.as_deref(),
            Some(format!("Bearer {JETON}").as_str())
        );
    }

    #[tokio::test]
    async fn mika1727_un_nom_dagent_a_caractere_reserve_part_encode() {
        // `mika/dev?x` n'est pas un nom d'agent que le serveur accepterait, et
        // c'est exactement pourquoi il sert ici : la question n'est pas ce que
        // le serveur en fait, mais si le `/` et le `?` traversent le chemin et
        // la requête. Un `format!` à la place de `.query()` les y laisserait
        // passer, et ce test est le seul geste qui le verrait.
        let (_, recues, _) = appel(StatusCode::OK, CORPS, "mika/dev?x").await;
        let vues = recues.lock().unwrap();
        assert_eq!(
            vues[0].chemin, ROUTE,
            "le nom d'agent ne doit pas fuir dans le chemin"
        );
        assert_eq!(vues[0].requete.as_deref(), Some("agent=mika%2Fdev%3Fx"));
    }

    #[tokio::test]
    async fn mika1727_un_point_injoignable_na_pas_de_statut() {
        // mika#2569 : le port est **tenu** pour toute la durée de l'appel. Le
        // motif qui liait puis libérait a vu un factice voisin du même binaire
        // servir l'appel censé échouer (2026-09-28, job `Check` de mika#2561).
        let dead = DeadEndpoint::reserve();
        let base = format!("http://{}", dead.addr());

        let probleme = lire(&base, JETON, "mika-dev")
            .await
            .expect_err("rien n'écoute : l'appel doit échouer");
        // Le variant lui-même porte l'absence de statut : il n'y a aucun champ à
        // discriminer, donc rien à lire de travers.
        assert!(
            matches!(probleme, SkillsRemoteIssue::Injoignable { .. }),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(message.contains("unreachable"), "{message}");
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_404_nomme_lagent_demande_et_jamais_une_session() {
        // Le corps est celui que la route sert réellement pour ce statut, et
        // non une chaîne arbitraire : c'est ce qui rend le test capable de voir
        // qu'un futur éditeur a remis ce corps dans le message.
        let (base, _, rendu) = appel(
            StatusCode::NOT_FOUND,
            r#"{"error": "agent 'fantome' not found"}"#,
            "fantome",
        )
        .await;
        let probleme = rendu.expect_err("un 404 est une erreur");
        assert!(
            matches!(probleme, SkillsRemoteIssue::AgentInconnu { .. }),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(message.contains("fantome"), "{message}");
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
        // L'assertion sans laquelle ce variant ne prouve rien : il n'existe que
        // pour ne pas dire ce que dit le `NotFound` du voisin — « unknown
        // agent, or unknown session » — qui enverrait l'opérateur chercher la
        // panne du côté des sessions, où elle n'est pas.
        assert!(
            !message.contains("session"),
            "un 404 d'ici ne peut vouloir dire qu'agent inconnu : {message}"
        );
    }

    #[tokio::test]
    async fn mika1727_un_autre_statut_nomme_le_statut_lurl_et_le_corps() {
        let (base, _, rendu) = appel(StatusCode::INTERNAL_SERVER_ERROR, "boom", "mika-dev").await;
        let probleme = rendu.expect_err("un 500 est une erreur");
        assert!(
            matches!(probleme, SkillsRemoteIssue::Statut { status: 500, .. }),
            "{probleme:?}"
        );
        // Les trois ensemble : sans le statut l'opérateur ne sait pas quoi
        // chercher, sans l'URL contre quoi, et sans le corps ce que le serveur
        // avait déjà dit.
        let message = probleme.to_string();
        assert!(message.contains("500"), "{message}");
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
        assert!(message.contains("boom"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_un_200_illisible_nest_jamais_un_succes() {
        // Un corps plausible auquel `skills` manque : la panne d'un serveur
        // d'une autre version, pas un corps absurde.
        let (base, _, rendu) = appel(StatusCode::OK, r#"{"agent":"mika-dev"}"#, "mika-dev").await;
        // L'assertion négative d'abord : le défaut à ne jamais produire est un
        // miroir à champs vides rendu comme un succès, pas un message raté.
        assert!(
            rendu.is_err(),
            "un 200 au corps illisible ne doit jamais rendre un miroir"
        );
        let probleme = rendu.expect_err("déjà établi juste au-dessus");
        assert!(
            matches!(probleme, SkillsRemoteIssue::CorpsIllisible { .. }),
            "{probleme:?}"
        );
        let message = probleme.to_string();
        assert!(message.contains(&format!("{base}{ROUTE}")), "{message}");
        // La cause nomme le champ absent : c'est elle qui dit *où* les deux
        // versions divergent, là où « corps illisible » seul ne dirait rien.
        assert!(message.contains("skills"), "{message}");
    }
}
