//! Un outil MCP testimony n'est jamais **publié** au modèle (mika#2814).
//!
//! # Ce que ce test mesure, et pourquoi là
//!
//! mika#1959 a mis le troisième étage de dispatch sous la garde L4 de la
//! doctrine non-transit (mika#1798) : un outil MCP dont le serveur déclare
//! `data_grade = "testimony"` est refusé **avant** `McpManager::call_tool`.
//! Mais le refus portait sur l'**appel**, pas sur la **publication** — l'outil
//! restait dans `tool_definitions()`, donc dans le lot envoyé au modèle, qui
//! pouvait encore le *proposer*. La doctrine interdit les deux :
//!
//! > Mika may NEVER access **nor propose accessing** testimony-grade data.
//!
//! La mesure porte donc sur les **octets soumis au modèle**, pas sur un
//! compteur interne ni sur le texte de la réponse. Un filtre écrit et non
//! câblé laisse le compteur juste et la publication intacte ; seule la requête
//! LLM dit la vérité sur ce que le modèle a vu.
//!
//! # Pourquoi les deux tests n'ont de sens qu'ensemble
//!
//! Le premier est une assertion d'**absence**, et une absence est aussi ce
//! qu'on observe quand l'instrument ne mesure rien : fixture cassée, serveur
//! non connecté, harnais qui ne publie aucun outil. Trois précautions la
//! rendent décisive :
//!
//! 1. `has_connections()` est asserté — le serveur **est** là (la fixture
//!    tourne, le protocole a abouti) ;
//! 2. `is_mcp_tool()` et `tool_data_grades()` sont assertés — l'inventaire de
//!    refus survit, donc la garde L4 n'a pas été désarmée par le filtre
//!    (décision D3 du plan : seules les *définitions* sont amputées) ;
//! 3. le **contrôle positif** relance la *même* fixture en `operational` et
//!    exige que les deux noms traversent. Sans lui, un harnais qui ne publie
//!    jamais d'outils MCP rendrait le premier test vert pour la mauvaise
//!    raison.
//!
//! # La fixture est réutilisée, pas dupliquée
//!
//! `mcp_secret_boundary_server.py` (mika#2281) expose deux outils et ne lit
//! que des variables d'environnement facultatives. Ce qui est mesuré ici est
//! le *grade déclaré côté client*, jamais une propriété du serveur : une
//! seconde fixture n'ajouterait qu'un second chemin à maintenir.

use anyhow::Result;
use mika_agent::mcp::McpManager;
use mika_agent::mcp::config::{McpConfig, McpServerConfig, McpTransport};
use mika_agent::skills::manifest::DataGrade;
use mika_common::llm::mock::*;

use super::harness::EvalHarness;

/// Nom du serveur MCP factice. Contraint par `McpServerConfig::validate` :
/// minuscules, chiffres, `-`/`_` simples, pas de `__`.
///
/// « journal » parce que c'est l'exemple même de la doctrine — le journal
/// intime est la donnée testimony canonique de mika#1798.
const SERVER: &str = "journal";

/// Chemin de la fixture, résolu à la compilation depuis la racine du crate.
fn fixture_server_path() -> String {
    format!(
        "{}/tests/fixtures/mcp_secret_boundary_server.py",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Le nom namespacé du premier outil de la fixture.
fn outil_liste() -> String {
    format!("mcp__{SERVER}__list_environments")
}

/// Le nom namespacé du second outil de la fixture.
fn outil_monte() -> String {
    format!("mcp__{SERVER}__create_local_env_file")
}

/// Connecte un `McpManager` à la fixture, au grade demandé.
///
/// Le grade est le **seul** paramètre : les deux tests partagent tout le reste,
/// de sorte que la différence observée ne puisse venir que de lui.
async fn connect_fixture(data_grade: DataGrade) -> McpManager {
    let mut config = McpConfig::default();
    config.mcp_servers.insert(
        SERVER.to_string(),
        McpServerConfig {
            transport: McpTransport::Stdio,
            command: Some("python3".to_string()),
            args: Some(vec![fixture_server_path()]),
            env: None,
            url: None,
            headers: None,
            enabled: true,
            data_grade,
        },
    );

    let manager = McpManager::connect_all(&config).await;

    // `connect_all` est fail-open : un serveur qui échoue est simplement
    // absent, et le seul signal est un `warn!` que ce test ne lit pas. Sans
    // cette assertion, une fixture cassée rendrait un manager vide et
    // l'assertion d'absence passerait au vert sans rien avoir mesuré.
    assert!(
        manager.has_connections(),
        "le serveur MCP factice ne s'est pas connecté — sans lui l'assertion \
         d'absence est vide de sens. Vérifier que `python3` est dans le PATH \
         et que {} est lisible.",
        fixture_server_path()
    );

    manager
}

/// Concatène tout ce qui a été soumis au modèle — prompt système, messages,
/// résultats d'outils, **définitions d'outils**.
///
/// C'est la surface de la mesure : la dernière de ces quatre populations est
/// celle que mika#2814 ferme, et elle n'apparaît nulle part ailleurs.
fn all_llm_request_bytes(trace: &super::trace::AgentTrace) -> String {
    trace
        .captured_requests
        .iter()
        .map(|r| format!("{r:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// AC1 + AC3 — testimony : le modèle ne voit pas les outils, et L4 garde son
/// inventaire.
#[tokio::test]
async fn mika2814_un_outil_mcp_testimony_natteint_jamais_le_modele() -> Result<()> {
    let manager = connect_fixture(DataGrade::Testimony).await;

    // --- La publication est amputée au niveau du manager. ------------------
    let publies: Vec<&str> = manager
        .tool_definitions()
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert!(
        publies.is_empty(),
        "les deux outils d'un serveur testimony ne doivent pas être publiés, \
         or le manager en porte : {publies:?}"
    );

    // --- AC3 : la garde L4 n'est pas désarmée pour autant. -----------------
    // Un appel halluciné sur un nom non publié doit être refusé comme
    // `TOOL_TESTIMONY_GRADE_FORBIDDEN`, pas comme « outil inconnu ».
    for outil in [outil_liste(), outil_monte()] {
        assert!(
            manager.is_mcp_tool(&outil),
            "{outil} doit rester dans le routage : sans lui, un appel halluciné \
             tomberait dans « outil inconnu » au lieu du refus L4 de mika#1959"
        );
        assert_eq!(
            manager.tool_data_grades().get(&outil),
            Some(&DataGrade::Testimony),
            "{outil} doit garder son grade dans la map que L4 consomme"
        );
    }

    // --- AC1 : la mesure sur les octets soumis au modèle. ------------------
    let harness = EvalHarness::builder().mcp_manager(manager).build().await?;
    harness
        .mock()
        .clear_and_set(vec![text_response("Je n'ai pas d'outil pour cela.")]);

    let trace = harness
        .run("De quoi disposes-tu pour lire mon journal ?")
        .await?;

    let submitted = all_llm_request_bytes(&trace);
    for outil in [outil_liste(), outil_monte()] {
        assert!(
            !submitted.contains(&outil),
            "AC1 ÉCHOUE : {outil} est dans ce qui a été soumis au modèle — il \
             peut donc encore être proposé, ce que la doctrine mika#1798 \
             interdit au même titre que l'accès. Requêtes : {submitted}"
        );
    }

    Ok(())
}

/// AC2 — contrôle positif : la **même** fixture en `operational` traverse.
///
/// Ce test doit passer au vert en **trouvant** les deux noms. S'il échoue, ce
/// n'est pas un filtre devenu plus sûr : c'est le filtre devenu aveugle au
/// grade, et l'assertion d'absence du test voisin devenue creuse.
#[tokio::test]
async fn mika2814_controle_positif_un_serveur_operational_est_publie() -> Result<()> {
    let manager = connect_fixture(DataGrade::Operational).await;

    let publies: Vec<&str> = manager
        .tool_definitions()
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert!(
        publies.contains(&outil_liste().as_str()) && publies.contains(&outil_monte().as_str()),
        "AC2 ÉCHOUE : un serveur operational doit publier ses deux outils, or \
         le manager porte {publies:?}"
    );

    let harness = EvalHarness::builder().mcp_manager(manager).build().await?;
    harness
        .mock()
        .clear_and_set(vec![text_response("J'ai deux outils pour cela.")]);

    let trace = harness
        .run("De quoi disposes-tu pour lire mon journal ?")
        .await?;

    let submitted = all_llm_request_bytes(&trace);
    for outil in [outil_liste(), outil_monte()] {
        assert!(
            submitted.contains(&outil),
            "AC2 ÉCHOUE : {outil} devrait atteindre le modèle — sans cette \
             occurrence, l'absence mesurée par le test voisin ne prouve rien. \
             Requêtes : {submitted}"
        );
    }

    Ok(())
}
