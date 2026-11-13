//! mika#3004 — un tour de mika-dev déclenché par `pr_review` n'exige plus
//! l'outil de groom, et un tour de label `ready` l'exige toujours.
//!
//! **L'incident, mesuré le 2026-10-30, trace `3b771914-d485-11f1-8e41-923f9687bff8`,
//! session `cbfeed6e`.** À 17:14:34Z un tour de mika-dev est facturé
//! `webhook_turn_charged` / `trigger: pr_review` / `candidate: review_verdict`
//! sur mika#2968 — PR #2982 ouverte, plan convergé. À 17:14:38Z la porte #3 le
//! re-prompte : *« agent responded without calling required tools »*,
//! `missing_names: ["run_claude_pilot_groom"]`. À 17:14:59Z il crée la parente
//! « Groom mika#2968 », à 17:15:05Z un `deferred_dispatch_registered`, et ses
//! deux enfants rendent `auto_skipped` / `already_groomed` en trois secondes.
//! Coût : deux tours A2A pour aucun travail.
//!
//! **Ce que cette suite atteste, et ce qu'elle ne prétend pas.** Elle tient la
//! propriété *structurelle* : sur un tour dont l'événement webhook est en
//! territoire de handler, l'exigence de groom est désarmée **quoi qu'elle ait
//! armée**. Le jeton exact qui l'a armée dans l'incident n'est pas
//! reconstructible depuis GitHub — le texte intégral du tour vit dans la base de
//! l'agent, et aucun corps de revue de la PR #2982 ne porte `groom` en mot
//! plein. Ce qui *est* mesuré, c'est le véhicule : la passerelle appose le
//! **corps de la revue** au texte de l'événement
//! (`crates/mika-gateway/src/github.rs:597-607`, plafond 16 000 caractères),
//! donc n'importe quelle prose de mika-qa mentionnant un groom arme `dev-groom`
//! par mot-clé. La phrase de la fixture qui porte le jeton est donc **nominale**,
//! et aucune assertion ne dépend de son libellé — seulement de la présence du
//! jeton, dont le cas 2 prouve qu'elle est réelle.
//!
//! **Trois cas, et les trois portent.**
//! 1. Le tour `pr_review` ne prend pas le re-prompt (AC1, AC2 moitié positive).
//! 2. Le **même** brief, `webhook_event_text` absent, le prend — c'est la cause B
//!    du vol (le texte que le tour voit est celui d'APRÈS les handlers) et c'est
//!    le contrôle anti-vacuité du cas 1 : si le jeton n'armait rien, ce cas
//!    serait rouge.
//! 3. Un label `ready` sur un ticket sans PR l'exige toujours (AC2 moitié
//!    négative).

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::SkillEntry;
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::{text_response, tool_call_response};
use serde_json::json;

use super::assertions::{
    assert_exact_steps, assert_has_output, assert_tools_exclude, assert_tools_include,
    assert_tools_registered,
};
use super::harness::EvalHarness;

/// L'événement brut que la passerelle construit pour un `pull_request_review`,
/// corps de revue inclus — c'est le texte d'AVANT les handlers de `server/`, et
/// donc ce que `webhook_event_text` porte en production.
///
/// L'état (`commented`), le titre et le libellé de la dernière ligne sont
/// nominaux ; le ticket, la PR, le relecteur et la **forme** viennent de la
/// mesure. La dernière ligne est le jeton `groom` qui arme `dev-groom` par
/// mot-clé — voir l'en-tête du module.
const PR_REVIEW_3004_EVENT: &str = "[GitHub] PR review (commented) on \
     senara-solutions/mika#2968 (feat(2968): sans approbation sur la tête, pas de \
     merge-candidat) by @mika-platform-qa\n\
     https://github.com/senara-solutions/mika/pull/2982\n\n\
     VERDICT: block[ci]\n\
     DEPTH: code-level\n\
     REASON: CI check 'Canonical Token Lint' failing on PR head — code-level lint \
     failure likely repairable by a code fix.\n\n\
     - [⏭️] AC1 — fermé par le vol 1 (mika#2971) ; le plan n'a pas été soumis à un \
     nouveau groom.\n";

/// L'enrichissement que `verdict_handler` **préfixe** au texte sur un
/// `Passthrough { enrichment: Some(_) }` (`server/handlers.rs:1898-1902`).
///
/// C'est la cause B du vol, dans sa forme qui préserve le corps de la revue :
/// après ce préfixe, la ligne `[GitHub] PR review (…)` n'est plus la première,
/// donc un prédicat de préfixe interrogé sur le texte du tour répond « hors
/// territoire » sur le tour même qu'un handler vient de traiter. L'autre forme —
/// `Handled { pre_digest }` — **remplace** le texte, et elle est couverte par le
/// test unitaire `mika3004_un_tour_pr_review_desarme_lexigence_de_groom`.
const VERDICT_ENRICHMENT_3004: &str = "<verdict_handler>\n\
     VERDICT: block[ci] sur senara-solutions/mika#2968 — un correctif est déjà en vol.\n\
     Wait for the active callback to finish before taking further action.\n\
     </verdict_handler>\n\n";

/// Le contrôle négatif d'AC2 : un label `ready` sur un ticket qui ne porte
/// aucune PR. Hors territoire de handler (le marqueur de label est son propre
/// axe) et hors domaine Webhook Fallthrough, donc aucune des deux propriétés du
/// tour ne désarme quoi que ce soit — et c'est précisément la population où
/// l'exigence de groom est le travail.
const READY_LABEL_3004_EVENT: &str = "[GitHub] Issue labeled ready on \
     senara-solutions/mika#2675 — feat(2675): un ticket sans PR liée\n\
     https://github.com/senara-solutions/mika/issues/2675\n\n\
     Le plan reste à produire : ce ticket attend son groom.\n";

/// Outil factice qui réussit toujours — même forme que celui de
/// `test_required_tools_gate.rs`.
struct SuccessTool {
    name: String,
}

#[async_trait]
impl Tool for SuccessTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: format!("Stub success tool: {}", self.name),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success("ok".to_string()))
    }
}

/// Un `SkillEntry` à l'image de `skills/bundled/dev-groom/skill.toml` : mot-clé
/// `groom`, `required_tools = ["run_claude_pilot_groom"]`.
///
/// `collect_required_tools` n'honore que les appariements
/// `MatchReason::Keyword`, et `dev-groom` est le **seul** skill empaqueté à
/// déclarer cet outil (`grep -rn run_claude_pilot_groom skills/bundled/*/skill.toml`).
fn dev_groom_shaped_skill() -> SkillEntry {
    let keywords = ["groom", "groom ticket", "/mika-groom-ticket", "groom issue"];
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "dev-groom-shaped".to_string(),
                description: "dev-groom-shaped test skill (mika#3004)".to_string(),
                version: "0.1.0".to_string(),
                always_on: false,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: keywords.iter().map(|s| s.to_string()).collect(),
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec!["run_claude_pilot_groom".to_string()],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/dev-groom-shaped"),
        keywords_lower: keywords.iter().map(|s| s.to_lowercase()).collect(),
        prompt_snippet: String::new(),
        skill_tools: vec![],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

/// Le registre d'outils du tour : `run_claude_pilot_groom` y est **servi**.
///
/// Sans lui, la branche 3 (`NonServi`) de `raison_de_desarmement` répondrait la
/// première et les trois cas seraient verts pour la mauvaise raison — le cas 2
/// est ce qui redevient rouge si quelqu'un le retire.
fn tools_with_groom() -> mika_agent::tools::ToolRegistry {
    let mut tools = default_tools();
    tools.register(Box::new(SuccessTool {
        name: "run_claude_pilot_groom".to_string(),
    }));
    tools.register(Box::new(SuccessTool {
        name: "acknowledge_note".to_string(),
    }));
    tools
}

/// Le brief du tour : l'enrichissement du handler, puis l'événement.
fn turn_text_3004() -> String {
    format!("{VERDICT_ENRICHMENT_3004}{PR_REVIEW_3004_EVENT}")
}

/// AC1 + AC2 (moitié positive) — le tour `pr_review` de la fixture n'exige pas
/// `run_claude_pilot_groom` : il accuse, répond, et la porte #3 ne le re-prompte
/// pas. Deux requêtes LLM, pas trois.
#[tokio::test]
async fn mika3004_un_tour_pr_review_ne_prend_pas_le_re_prompt_de_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("acknowledge_note", json!({})),
            text_response(
                "La PR #2982 est déjà ouverte sur mika#2968 et son plan a convergé : \
                 rien à groomer, j'attends la fin du callback actif.",
            ),
        ])
        .tools(tools_with_groom())
        .skills(SkillRegistry::from_test_entries(vec![
            dev_groom_shaped_skill(),
        ]))
        .webhook_event_text(PR_REVIEW_3004_EVENT)
        .build()
        .await
        .unwrap();

    let trace = harness.run(&turn_text_3004()).await.unwrap();

    assert_has_output(&trace);
    // L'outil est bien SERVI : sans cette ligne, la branche 3 (`NonServi`) de
    // `raison_de_desarmement` pourrait répondre la première et le cas serait vert
    // pour la mauvaise raison.
    assert_tools_registered(&trace, &["run_claude_pilot_groom"]);
    // Et il n'est jamais appelé — c'est l'issue qu'AC1 demande.
    assert_tools_exclude(&trace, &["run_claude_pilot_groom"]);
    // 1 (acknowledge_note) + 1 (texte final) = 2. Un re-prompt ferait 3.
    assert_exact_steps(&trace, 2);
}

/// Le contrôle de la cause B, et l'anti-vacuité du cas ci-dessus — c'est l'état
/// mesuré le 2026-10-30 à 17:14:38Z, trace `3b771914`.
///
/// Même brief, même registre, même skill : seul `webhook_event_text` manque.
/// Le texte du tour commence alors par `<verdict_handler>`, le prédicat de
/// préfixe répond « hors territoire », la branche 5 ne tire pas et la porte #3
/// re-prompte. Si ce cas passait au vert, c'est que le jeton `groom` de la
/// fixture n'arme rien et que le cas précédent ne mesure rien.
#[tokio::test]
async fn mika3004_le_meme_brief_sans_texte_devenement_prend_le_re_prompt() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("acknowledge_note", json!({})),
            text_response("Rien à groomer, j'attends la fin du callback actif."),
            // Après le re-prompt de la porte #3.
            tool_call_response("run_claude_pilot_groom", json!({})),
            text_response("Groom lancé."),
        ])
        .tools(tools_with_groom())
        .skills(SkillRegistry::from_test_entries(vec![
            dev_groom_shaped_skill(),
        ]))
        .build()
        .await
        .unwrap();

    let trace = harness.run(&turn_text_3004()).await.unwrap();

    assert_has_output(&trace);
    // L'exigence a tenu : le tour serait mort au deuxième texte sans le
    // re-prompt de la porte #3, et l'outil de groom n'aurait jamais été atteint.
    assert_tools_include(&trace, &["run_claude_pilot_groom"]);
    // 1 + 1 (texte refusé) + 1 (groom) + 1 (texte final) = 4.
    assert_exact_steps(&trace, 4);
}

/// AC2 (moitié négative) — un tour déclenché par le label `ready` sur un ticket
/// sans PR exige toujours `run_claude_pilot_groom`.
///
/// `webhook_event_text` **et** le brief portent le même texte : c'est exactement
/// la forme d'un tour qu'aucun handler n'a réécrit, donc le cas sépare « la
/// branche 5 lit le bon territoire » de « la branche 5 a désarmé l'exigence en
/// bloc ».
///
/// **Cinq requêtes, et seule la première paire appartient à ce vol.** La porte
/// #3 re-prompte au deuxième texte — c'est la propriété mesurée ici, et
/// `assert_tools_include` en est la preuve indépendante du compte : sans ce
/// re-prompt le tour serait mort à la requête 2 et n'aurait jamais atteint
/// l'appel de groom scripté en troisième réponse. Un **second** re-prompt suit,
/// propre à la population des briefs préfixés `[GitHub] ` du label `ready`
/// (famille `webhook_zero_tools` / `webhook_ready_label_dispatch`) ; il est
/// insensible au contenu du texte final — vérifié sur deux libellés, l'un mince
/// et l'autre auto-portant — et ce vol n'y touche pas. Le contraste qui porte
/// AC2 est donc **2 contre 5** à registre d'outils et skill identiques.
#[tokio::test]
async fn mika3004_un_label_ready_sans_pr_exige_toujours_le_groom() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("acknowledge_note", json!({})),
            text_response("Ticket vu, je regarde."),
            // Après le re-prompt de la porte #3.
            tool_call_response("run_claude_pilot_groom", json!({})),
            text_response(
                "J'ai lancé le groom de senara-solutions/mika#2675 via \
                 run_claude_pilot_groom : le ticket porte le label `ready` et aucune \
                 PR ne lui est liée, donc son plan reste à produire. Le verdict \
                 arrivera par callback.",
            ),
            // Après le second re-prompt, étranger à ce vol (voir le doc-comment).
            text_response("Groom lancé sur senara-solutions/mika#2675."),
        ])
        .tools(tools_with_groom())
        .skills(SkillRegistry::from_test_entries(vec![
            dev_groom_shaped_skill(),
        ]))
        .webhook_event_text(READY_LABEL_3004_EVENT)
        .build()
        .await
        .unwrap();

    let trace = harness.run(READY_LABEL_3004_EVENT).await.unwrap();

    assert_has_output(&trace);
    assert_tools_registered(&trace, &["run_claude_pilot_groom"]);
    // La preuve du forçage, indépendante du compte de requêtes.
    assert_tools_include(&trace, &["run_claude_pilot_groom"]);
    // 1 + 1 (refusé par la porte #3) + 1 (groom) + 1 (refusé par la seconde
    // garde) + 1 (texte final) = 5.
    assert_exact_steps(&trace, 5);
}
