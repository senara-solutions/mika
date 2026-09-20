//! Non-fuite d'une valeur de secret par le canal MCP — et son contrôle
//! positif (mika#2281, UI-3 / V3–V5).
//!
//! # Ce que ce test mesure, et ce qu'il ne mesure pas
//!
//! AC2 du ticket demande qu'aucune valeur de secret n'apparaisse dans le
//! contexte LLM, les canaux durables ni les journaux. Sur le **canal MCP**,
//! cette propriété est tenue par le fournisseur, pas par Mika : le serveur
//! « 1Password Environments » ne rend jamais de valeur à son client. Il n'y
//! avait donc rien à implémenter — seulement à *vérifier*, et le substrat ne
//! permettait pas de le vérifier : il n'existait aucun serveur MCP factice,
//! aucun harnais, aucune fixture de protocole, et `EvalHarness::mcp_manager()`
//! n'avait aucun appelant.
//!
//! Le trou réel est ailleurs, et le ticket ne le nommait pas (plan mika#2281
//! R4) : `create_local_env_file` **monte un `.env`**. Ce que le fournisseur
//! garantit, c'est que le *serveur* ne rend pas les valeurs — **pas** que le
//! fichier monté soit illisible. Or les agents portent `file-reader` dans les
//! allowlists usuelles, et `secret_scrubber` ne couvre que 14 formes connues :
//! une chaîne de connexion arbitraire est persistée en clair.
//!
//! D'où les deux tests, qui n'ont de sens qu'ensemble :
//!
//! 1. **Négatif** — la sentinelle ne traverse aucun canal durable quand seul
//!    le canal MCP est employé, *et* les noms de variables, eux, traversent
//!    bien (sans cette seconde moitié, un serveur mort donnerait le même
//!    résultat vert).
//! 2. **Contrôle positif** — la **même** sentinelle, atteinte par le canal
//!    fichier, **est** trouvée. Sans lui, un test tout-vert n'établit pas la
//!    non-fuite : il établit que la sentinelle n'a traversé aucun canal, ce
//!    qui est aussi ce qu'on observe quand l'instrument ne mesure rien.
//!
//! # Indépendant de 1Password
//!
//! Le fixture reproduit le *contrat* (noms oui, valeurs non), pas le produit.
//! Ce test garde donc sa valeur si le verdict de la sonde UI-1 est négatif —
//! il porte sur le chemin MCP générique, donc sur tout serveur présent et
//! futur. Voir `docs/mcp.md` pour le verdict daté.

use std::collections::HashMap;

use anyhow::Result;
use mika_agent::mcp::McpManager;
use mika_agent::mcp::config::{McpConfig, McpServerConfig, McpTransport};
use mika_common::llm::mock::*;
use serde_json::json;

use super::harness::EvalHarness;

/// Nom du serveur MCP factice. Contraint par `McpServerConfig::validate` :
/// minuscules, chiffres, `-`/`_` simples, pas de `__`.
const SERVER: &str = "opsentinel";

/// La sentinelle : une chaîne de connexion, **délibérément hors des 14 motifs
/// de `SECRET_PATTERNS`**.
///
/// Ce choix est le cœur de la mesure. Une valeur en forme de PAT GitHub serait
/// masquée par le scrubber avant persistance, et le test mesurerait alors la
/// couverture du scrubber au lieu de la frontière du canal. Ici, si la
/// sentinelle apparaît dans un canal durable, c'est qu'elle y est bel et bien
/// arrivée — rien ne l'aurait effacée. Le contrôle positif ci-dessous le
/// démontre sur pièce.
///
/// Ni guillemet ni antislash : les assertions scannent le `Debug` de la
/// requête LLM, où une échappement changerait la chaîne recherchée.
const SENTINEL: &str = "postgres://probe:sentinel2281ff00aa55@db.invalid:5432/mika_probe";

/// Le nom de la variable qui porte la sentinelle. Délibérément **pas**
/// `MIKA_*_TOKEN` ni `GH_TOKEN` : ces deux formes-là sont scrubbées à
/// l'assignation, ce qui masquerait la fuite que le contrôle positif doit
/// rendre visible.
const VAR_NAME: &str = "DATABASE_URL";

/// Nom de l'Environment de test. Jamais un coffre réel : la consigne
/// opérateur du ticket (« JAMAIS en prod, JAMAIS sur un coffre réel ») est ici
/// tenue par construction — le serveur est un fixture de ce dépôt.
const ENVIRONMENT: &str = "mika-test-2281";

/// Chemin du fixture, résolu à la compilation depuis la racine du crate.
fn fixture_server_path() -> String {
    format!(
        "{}/tests/fixtures/mcp_secret_boundary_server.py",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Connecte un `McpManager` au serveur factice.
///
/// Les clés d'environnement ne commencent pas par `MIKA_` : `connect_stdio`
/// refuse ces surcharges à dessein (et le ferait en silence, avec un `warn!`),
/// ce qui priverait le fixture de sa sentinelle.
async fn connect_probe_server() -> McpManager {
    let mut env = HashMap::new();
    env.insert("MCP_PROBE_SENTINEL".to_string(), SENTINEL.to_string());
    env.insert("MCP_PROBE_VAR_NAME".to_string(), VAR_NAME.to_string());
    env.insert("MCP_PROBE_ENVIRONMENT".to_string(), ENVIRONMENT.to_string());

    let mut config = McpConfig::default();
    config.mcp_servers.insert(
        SERVER.to_string(),
        McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("python3".to_string()),
            args: Some(vec![fixture_server_path()]),
            env: Some(env),
            url: None,
            headers: None,
            enabled: true,
        },
    );

    let manager = McpManager::connect_all(&config).await;

    // `connect_all` est fail-open (`mcp/mod.rs`) : un serveur qui échoue est
    // simplement absent, et le seul signal est un `warn!` que ce test ne lit
    // pas. Sans cette assertion, une fixture cassée rendrait un manager vide
    // et TOUTES les assertions négatives ci-dessous passeraient au vert sans
    // rien avoir mesuré — la panne silencieuse que ce dépôt refuse.
    assert!(
        manager.has_connections(),
        "le serveur MCP factice ne s'est pas connecté — sans lui les assertions \
         négatives sont vides de sens. Vérifier que `python3` est dans le PATH \
         et que {} est lisible.",
        fixture_server_path()
    );

    let names: Vec<&str> = manager
        .tool_definitions()
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert!(
        names.contains(&&*format!("mcp__{SERVER}__list_environments"))
            && names.contains(&&*format!("mcp__{SERVER}__create_local_env_file")),
        "les deux outils du fixture doivent être découverts, or la liste est {names:?}"
    );

    manager
}

/// Concatène tout ce qui a été soumis au modèle — prompt système, messages,
/// résultats d'outils, définitions d'outils.
///
/// C'est **le contenu même** que `MIKA_LOG_LLM_BODIES` écrit dans le journal
/// (`llm request body`), donc la surface d'AC2 point « corps de requête ».
/// L'asserter ici plutôt que de capturer un `tracing` global est un choix :
/// un abonnement global est thread-local et ne survit pas au runtime
/// multi-thread du test, donc une capture de journal serait soit fragile, soit
/// silencieusement vide — c'est-à-dire verte pour la mauvaise raison.
fn all_llm_request_bytes(trace: &super::trace::AgentTrace) -> String {
    trace
        .captured_requests
        .iter()
        .map(|r| format!("{r:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Concatène tous les canaux durables du tour : `tool_calls.input`,
/// `tool_calls.output`, et `messages.metadata` (les `ToolCallSummary`).
async fn all_durable_bytes(
    harness: &EvalHarness,
    trace: &super::trace::AgentTrace,
) -> Result<String> {
    let mut buf = String::new();
    for call in &trace.tool_calls {
        if let Some(input) = &call.input {
            buf.push_str(input);
            buf.push('\n');
        }
        if let Some(output) = &call.output {
            buf.push_str(output);
            buf.push('\n');
        }
    }
    // `AgentTrace` ne porte pas son `trace_id` ; celui du tour `run()` est
    // celui du harnais.
    for msg in harness
        .db
        .get_messages_by_trace_id(&harness.trace_id)
        .await?
    {
        if let Some(metadata) = msg.metadata {
            buf.push_str(&metadata);
            buf.push('\n');
        }
    }
    Ok(buf)
}

/// V3 + V4 — le canal MCP ne porte jamais la valeur, et il porte bien les noms.
#[tokio::test]
async fn mika2281_mcp_channel_never_carries_the_secret_value() -> Result<()> {
    let manager = connect_probe_server().await;

    let harness = EvalHarness::builder().mcp_manager(manager).build().await?;

    // Le chemin du `.env` n'est connu qu'après `build()` (le home est un
    // TempDir), donc les réponses du mock sont posées ici et non au builder.
    let env_path = harness.home_dir.path().join(".env");
    harness.mock().clear_and_set(vec![
        tool_call_response(&format!("mcp__{SERVER}__list_environments"), json!({})),
        tool_call_response(
            &format!("mcp__{SERVER}__create_local_env_file"),
            json!({ "environment": ENVIRONMENT, "path": env_path.to_string_lossy() }),
        ),
        text_response("J'ai listé l'Environment de test et monté son .env."),
    ]);

    let trace = harness
        .run("Liste mes Environments puis monte le .env.")
        .await?;

    // --- Le canal a bien fonctionné (sinon tout le reste est creux). --------
    let called: Vec<&str> = trace.tool_names();
    assert!(
        called.contains(&&*format!("mcp__{SERVER}__list_environments"))
            && called.contains(&&*format!("mcp__{SERVER}__create_local_env_file")),
        "les deux outils MCP doivent avoir été appelés, or : {called:?}"
    );
    for call in &trace.tool_calls {
        assert!(
            call.success,
            "l'appel MCP {} a échoué : {:?}",
            call.tool_name, call.output
        );
        assert_eq!(
            call.tool_source, "mcp",
            "l'appel {} doit être routé par le troisième étage de dispatch",
            call.tool_name
        );
    }

    let durable = all_durable_bytes(&harness, &trace).await?;
    let submitted = all_llm_request_bytes(&trace);

    // Les NOMS ont traversé — la moitié positive de ce test négatif. Un
    // serveur mort ou une sentinelle jamais écrite produirait le même vert
    // sur les assertions d'absence ci-dessous.
    assert!(
        durable.contains(VAR_NAME),
        "le nom de variable doit traverser le canal MCP (il n'y a pas de \
         mutisme général) — canaux durables : {durable}"
    );
    assert!(
        submitted.contains(VAR_NAME),
        "le nom de variable doit atteindre le modèle — requêtes : {submitted}"
    );

    // --- V3 : aucun canal durable ne porte la valeur. ----------------------
    assert!(
        !durable.contains(SENTINEL),
        "V3 ÉCHOUE : la valeur de secret est persistée (tool_calls.input / \
         tool_calls.output / messages.metadata). Contenu : {durable}"
    );

    // --- V4 : rien de ce qui est soumis au modèle ne la porte. -------------
    assert!(
        !submitted.contains(SENTINEL),
        "V4 ÉCHOUE : la valeur de secret est dans un corps de requête LLM — \
         c'est exactement ce que `MIKA_LOG_LLM_BODIES` écrirait dans le \
         journal. Requêtes : {submitted}"
    );

    // Le fichier, lui, la porte : la frontière est le canal, pas la valeur.
    // C'est le fait que le contrôle positif ci-dessous exploite.
    let on_disk = std::fs::read_to_string(&env_path)?;
    assert!(
        on_disk.contains(SENTINEL),
        "le fixture doit avoir écrit la sentinelle sur le disque, sinon il n'y \
         avait rien à taire. Fichier : {on_disk}"
    );

    Ok(())
}

/// V5 — contrôle positif : la même sentinelle, par le canal fichier, EST
/// trouvée.
///
/// Ce test **doit** passer au vert en trouvant la valeur. S'il échoue, ce
/// n'est pas une fuite corrigée : c'est l'instrument du test précédent qui ne
/// mesure plus rien, et ses assertions d'absence qui sont devenues vides.
///
/// Il matérialise aussi R4 du plan : la protection du fournisseur porte sur le
/// canal MCP et **pas** sur le fichier monté. Monter un `.env` à portée d'un
/// agent qui lit des fichiers, c'est mettre les valeurs à un `read_agent_file`
/// du contexte — et le remède n'est pas un filtre, c'est de ne pas le monter
/// là (voir la procédure d'isolation, `docs/mcp.md`).
#[tokio::test]
async fn mika2281_positive_control_the_file_channel_does_carry_it() -> Result<()> {
    let manager = connect_probe_server().await;

    let harness = EvalHarness::builder().mcp_manager(manager).build().await?;

    let env_path = harness.home_dir.path().join(".env");
    harness.mock().clear_and_set(vec![
        tool_call_response(
            &format!("mcp__{SERVER}__create_local_env_file"),
            json!({ "environment": ENVIRONMENT, "path": env_path.to_string_lossy() }),
        ),
        // Le même agent, le même tour, un outil de lecture de fichier.
        tool_call_response("read_agent_file", json!({ "path": ".env" })),
        text_response("Le .env est monté et lu."),
    ]);

    let trace = harness
        .run("Monte le .env de l'Environment de test, puis lis-le.")
        .await?;

    let read_calls = trace.calls_for_tool("read_agent_file");
    assert_eq!(
        read_calls.len(),
        1,
        "un seul `read_agent_file` attendu, or : {:?}",
        trace.tool_names()
    );
    assert!(
        read_calls[0].success,
        "la lecture du .env monté a échoué : {:?}",
        read_calls[0].output
    );

    let durable = all_durable_bytes(&harness, &trace).await?;
    let submitted = all_llm_request_bytes(&trace);

    assert!(
        durable.contains(SENTINEL),
        "V5 ÉCHOUE : le canal fichier aurait dû porter la valeur jusqu'aux \
         canaux durables. Sans cette occurrence, les assertions d'absence du \
         test voisin ne prouvent rien. Contenu : {durable}"
    );
    assert!(
        submitted.contains(SENTINEL),
        "V5 ÉCHOUE : le canal fichier aurait dû porter la valeur jusqu'au \
         contexte du modèle. Requêtes : {submitted}"
    );

    Ok(())
}
