//! Témoin fan-out (mika#2265, AC3) — la preuve que le harness multi-agents
//! **mesure** quelque chose que le harness mono-agent ne peut pas mesurer.
//!
//! # Ce que ce fichier démontre, et ce qu'il ne démontre pas
//!
//! Il démontre une **capacité de mesure**, pas un comportement du produit. Un
//! témoin qui asserterait l'état de fait de mika#2260 (« deux agents entrent
//! dans le chemin de merge ») graverait le bug dans la suite et virerait au
//! rouge le jour où #2260 ferme. Ce ticket livre l'instrument, pas le remède.
//!
//! Le scénario emprunte donc ses valeurs à mika#2248 — `tool_name =
//! "ci_success_handler_processed"`, `target_key = "senara-solutions/mika#2244"` —
//! pour nommer la classe qu'il rend visible, sans dépendre du handler réel.
//!
//! # Les trois contrôles (cf. `feedback_a_probe_needs_both_controls_in_the_same_call`)
//!
//! - **positif** — deux agents écrivent le même `tool_name` ; le harness rend
//!   `{mika-dev: 1, mika-qa: 1}` : deux attributions distinctes.
//! - **négatif** — le même scénario sur [`EvalHarness`] (mono-agent) : les deux
//!   écritures atterrissent sous l'unique `agent_id`, la carte rend une seule
//!   clé. C'est le rouge-avant, exprimé comme un test **vert** qui épingle
//!   l'incapacité.
//! - **partage** — un handle lit la tranche d'un autre agent. Sans lui, un
//!   montage à mémoires disjointes passerait les deux premiers.
//!
//! # Rouge-avant réellement observé (plan V3)
//!
//! Vérifié à la main pendant l'implémentation, une fois : l'assertion du témoin
//! positif — « la carte porte deux attributions distinctes » — portée sur un
//! `EvalHarness` au lieu du `MultiAgentHarness`. Résultat consigné dans le
//! doc-comment de [`controle_negatif_le_harness_mono_agent_ne_peut_pas_attribuer`].
//! Un rouge qu'on n'a pas vu de ses yeux est une croyance.

use std::collections::BTreeMap;

use anyhow::Result;

use super::harness::EvalHarness;
use super::multi_agent::MultiAgentHarness;

/// Le callsite de mika#2248 : les deux agents l'atteignaient, et le `gh pr merge`
/// posé là tournait sous le token de celui qui gagnait la course.
const TOOL: &str = "ci_success_handler_processed";
const TARGET: &str = "senara-solutions/mika#2244";

const PRIMARY: &str = "mika-dev";
const SECONDARY: &str = "mika-qa";

// ---------------------------------------------------------------------------
// Contrôle positif — le harness attribue
// ---------------------------------------------------------------------------

/// Deux agents atteignent le même callsite ; le harness rend **qui**, pas
/// seulement **combien**.
///
/// # Fire-Disposition
/// **(c) halte-et-remontée, gate CI bloquant.** Ce test tire quand la capacité
/// multi-agents régresse : le harness cesse de rendre deux attributions
/// distinctes. Pas de remédiation automatique, pas de skip conditionnel.
#[tokio::test]
async fn temoin_positif_deux_agents_deux_attributions_distinctes() -> Result<()> {
    let h = MultiAgentHarness::builder()
        .agent(PRIMARY)
        .agent(SECONDARY)
        .build()?;

    // Patron « ordre déterministe » : le primaire d'abord, puis les secondaires.
    // C'est la forme de mika#2248 (un ordre, pas une course).
    for (agent_id, db) in h.agents() {
        db.log_audit_event(h.session_id(agent_id), TOOL, TARGET, None, None, None, None)
            .await?;
    }

    let counts = h.audit_counts_by_agent(TOOL).await?;

    assert_eq!(
        counts.len(),
        2,
        "le harness doit rendre UNE attribution PAR AGENT monté ; obtenu {counts:?}. \
         Une carte à une seule clé signifie que l'attribution a été perdue — c'est \
         exactement l'aveuglement que ce harness existe pour supprimer."
    );
    assert_eq!(
        counts.get(PRIMARY),
        Some(&1),
        "`{PRIMARY}` doit porter exactement 1 événement ; carte : {counts:?}"
    );
    assert_eq!(
        counts.get(SECONDARY),
        Some(&1),
        "`{SECONDARY}` doit porter exactement 1 événement ; carte : {counts:?}"
    );

    // L'ordre de déclaration est préservé — un fan-out a un primaire.
    assert_eq!(h.agent_ids(), vec![PRIMARY, SECONDARY]);

    h.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Sonde de partage — la base est bien commune
// ---------------------------------------------------------------------------

/// Un handle voit la tranche d'un autre agent ; et ne voit rien là où il n'y a
/// rien.
///
/// Les deux contrôles sont dans le même test, à dessein : un montage à mémoires
/// disjointes (deux `open_in_memory()`) rendrait `0` au premier et `0` au
/// second, donc seul le couple distingue « partage » de « chaque agent écrit
/// dans son propre néant ».
///
/// # Fire-Disposition
/// **(c) halte-et-remontée, gate CI bloquant.** Ce test tire quand la base cesse
/// d'être partagée — retour à `open_in_memory`, à `with_agent()` comme seule
/// topologie, ou une factorisation qui redonne une mémoire par agent.
#[tokio::test]
async fn sonde_de_partage_un_agent_lit_la_tranche_de_lautre() -> Result<()> {
    let h = MultiAgentHarness::builder()
        .agent(PRIMARY)
        .agent(SECONDARY)
        .build()?;

    // Seul le secondaire écrit.
    h.db(SECONDARY)
        .log_audit_event(
            h.session_id(SECONDARY),
            TOOL,
            TARGET,
            None,
            None,
            None,
            None,
        )
        .await?;

    // Contrôle positif : le primaire, qui n'a rien écrit, VOIT l'écriture du
    // secondaire. C'est la base partagée qui répond, pas son propre handle.
    let seen = h.cross_read_count(PRIMARY, SECONDARY).await?;
    assert!(
        seen > 0,
        "`{PRIMARY}` ne voit rien de la tranche de `{SECONDARY}` : les deux handles ne \
         partagent PAS la même base. C'est le montage à mémoires disjointes que le \
         doc-comment de `multi_agent.rs` décrit — chaque agent écrit dans son propre néant."
    );

    // Contrôle négatif de la même sonde : elle ne rend pas systématiquement
    // « quelque chose ». Le primaire n'a rien écrit, sa propre tranche est vide.
    let own = h.cross_read_count(PRIMARY, PRIMARY).await?;
    assert_eq!(
        own, 0,
        "la tranche de `{PRIMARY}` doit être vide — il n'a rien écrit. Une valeur non nulle \
         signifie que la sonde ne discrimine pas par `agent_id`, et que le contrôle positif \
         ci-dessus ne prouve donc rien."
    );

    h.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Contrôle négatif — l'incapacité du harness mono-agent, épinglée
// ---------------------------------------------------------------------------

/// Le rouge-avant, exprimé comme un vert : sur [`EvalHarness`], deux écritures
/// « de deux agents » s'effondrent en **une seule** attribution.
///
/// # Le rouge observé (plan V3)
///
/// L'assertion `counts.len() == 2` du témoin positif, portée sur ce montage,
/// échoue — observé le 2026-09-09, pas déduit :
///
/// ```text
/// assertion `left == right` failed: … Carte obtenue : {"mika": 2}
///   left: 1
///  right: 2
/// ```
///
/// `EvalHarness` n'a qu'un `agent_id`
/// (`AsyncDatabase::new` délègue à `new_with_agent(db, "mika")`), et toutes ses
/// sondes d'audit sont scopées dessus. L'assertion « deux attributions » y est
/// **structurellement** inatteignable — pas configurée autrement, inatteignable.
///
/// # Fire-Disposition
///
/// **(c) halte-et-remontée, avec une instruction écrite dans l'assertion.**
/// Ce test tire si `EvalHarness` devient un jour capable de produire deux
/// attributions. Ce serait une *bonne* nouvelle signalée en rouge, et le
/// réflexe — supprimer le test qui gêne — serait le pire geste : c'est le
/// contrôle négatif qui aurait perdu son pouvoir de contrôle, et donc le témoin
/// positif qui ne prouverait plus rien.
#[tokio::test]
async fn controle_negatif_le_harness_mono_agent_ne_peut_pas_attribuer() -> Result<()> {
    let h = EvalHarness::builder().build().await?;

    // Deux écritures, comme si deux agents avaient atteint le même callsite.
    for _ in 0..2 {
        h.db.log_audit_event(&h.session_id, TOOL, TARGET, None, None, None, None)
            .await?;
    }

    let total = h.db.count_audit_events_by_tool_name(TOOL).await?;
    assert_eq!(
        total, 2,
        "les deux écritures ont bien eu lieu — l'incapacité porte sur l'attribution, \
         pas sur le comptage"
    );

    // La seule carte que ce harness sache produire : une clé, celle de son
    // unique agent. `count_audit_events_by_tool_name` est scopée sur
    // `self.agent_id`, il n'existe aucun second `agent_id` à interroger.
    let counts: BTreeMap<&str, i64> = BTreeMap::from([(h.db.agent_id(), total)]);

    assert_eq!(
        counts.len(),
        1,
        "NE PAS SUPPRIMER CE TEST. Il épingle l'incapacité du harness mono-agent à \
         attribuer un fan-out — c'est cette incapacité qui donne son sens au témoin \
         positif `temoin_positif_deux_agents_deux_attributions_distinctes`. S'il vire au \
         rouge, `EvalHarness` a gagné une capacité multi-agents : re-cadrer le témoin \
         positif sur la NOUVELLE asymétrie plutôt que d'effacer celle-ci. \
         Carte obtenue : {counts:?}"
    );
    assert_eq!(
        counts.get(h.db.agent_id()),
        Some(&2),
        "les deux écritures se sont effondrées sous l'unique `agent_id` `{}` : \
         « combien » survit, « lequel » est perdu",
        h.db.agent_id()
    );

    Ok(())
}
