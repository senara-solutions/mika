//! mika#2727 — un tour de revue ne finit pas sans avoir **soumis** la revue.
//!
//! Ces tests pilotent **le chemin de production du défaut mesuré** : un tour de
//! conversation ouvert par un événement webhook `[GitHub] PR synchronize: …`,
//! avec `qa-review` chargé. C'est exactement par là que les deux incidents du
//! 2026-10-15 sont passés (`server::handlers::run_agent_for_message` →
//! `run_agent`, donc mode conversation), et c'est pour cela que ce fichier
//! n'utilise pas `run_silent_agent` comme son frère mika#2355 : la garde
//! mika#2355 vit sur le tour de callback, celle-ci sur le tour d'ouverture.
//!
//! Ce que chaque test épingle :
//! - **AC3a** — un tour qui n'appelle que `qa_pr_view` puis `run_gh pr list`
//!   (lecture seule) et rend son verdict en texte est re-prompté, et la revue
//!   finit postée.
//! - **AC3b — contrôle positif, dans le même fichier** — séquence identique à
//!   un argument près (`pr review --approve` au lieu de `pr list`) : **aucune**
//!   correction n'est injectée, aucun re-prompt.
//! - **AC1, la moitié « une fois »** — si le re-prompt échoue aussi, le tour
//!   sort. La garde ne boucle pas : son budget est à un coup.
//!
//! # Pourquoi le fixture déclare deux `required_tools` et non les quatre réels
//!
//! `skills/bundled/qa-review/skill.toml` déclare
//! `required_tools = ["qa_pr_view", "run_gh", "run_shell", "build_mika"]`. Les
//! quatre ne sont pas transposables ici, et pas par commodité : un `build_mika`
//! **réussi satisfait la nouvelle garde** par la réserve D3
//! ([`mika_agent::qa_review_submission::build_dispatched_in_turn`] — un tour qui
//! lance le build rend la main et attend son callback, où mika#2355 reprend).
//! Un fixture qui satisferait les quatre `required_tools` ne pourrait donc
//! jamais faire tirer cette garde : les deux contraintes ne sont pas
//! conjointement satisfiables dans un seul tour.
//!
//! Le fixture retient donc les **deux outils que le tour incriminé a réellement
//! appelés** — `qa_pr_view` et `run_gh` — ce qui est aussi la narration du
//! défaut : la garde `required_tools` est **satisfaite** à l'instant du premier
//! `EndTurn`, et aucune revue n'a atteint GitHub. Ce que `run_shell` et
//! `build_mika` ajouteraient à la garde `required_tools` est couvert ailleurs
//! (`grounding_regressions/qa_review_required_tools_*`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mika_agent::qa_review_submission::REVIEW_SUBMISSION_CORRECTION;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::index::{ResolvedSkillTool, SkillEntry};
use mika_agent::skills::manifest::{Constraints, SkillInfo, SkillManifest, ToolHandler, Triggers};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use mika_common::llm::{LlmContent, LlmRequest, LlmRole};
use serde_json::{Value, json};

use super::harness::EvalHarness;

// ---------------------------------------------------------------------------
// Le message de fil
// ---------------------------------------------------------------------------

/// La tête du second incident (`3f2c3fa4`, tour de 19:11:36Z).
const HEAD: &str = "3f2c3fa4";

/// La forme exacte que la gateway pose sur le fil pour une mise à jour de
/// branche : deux lignes, l'action après `PR `, le dépôt et le numéro collés
/// par un `#`. Lue par l'unique extracteur
/// (`server::webhook_queue_v2::parse_pr_action_event`), jamais réécrite ici.
fn pr_synchronize_event() -> String {
    "[GitHub] PR synchronize: senara-solutions/mika#2704 — fix(agent): le tour de revue \
     (branch: fix/2704/tour-de-revue)\nhttps://github.com/senara-solutions/mika/pull/2704"
        .to_string()
}

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

/// Un `run_gh` qui enregistre ce qu'on lui demande et répond succès. Le contrat
/// AC3a est « une revue EST postée » : un test ne le voit qu'en tenant
/// l'exécution, d'où l'enregistreur plutôt qu'une assertion sur le texte final.
struct RecordingRunGh {
    calls: Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl Tool for RecordingRunGh {
    fn name(&self) -> &str {
        "run_gh"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_gh".to_string(),
            description: "Recording run_gh stub (mika#2727)".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        Ok(ToolOutput::success(
            "https://github.com/senara-solutions/mika/pull/2704#pullrequestreview-1".to_string(),
        ))
    }
}

/// Stub `qa_pr_view` — l'outil du skill, premier appel de tout tour de revue.
struct StubQaPrViewTool;

#[async_trait]
impl Tool for StubQaPrViewTool {
    fn name(&self) -> &str {
        "qa_pr_view"
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "qa_pr_view".to_string(),
            description: "View PR details for QA review".to_string(),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        Ok(ToolOutput::success(format!(
            "PR #2704: fix(agent): le tour de revue\nStatus: Open, headRefOid: {HEAD}"
        )))
    }
}

fn tools_with_recording_run_gh() -> (ToolRegistry, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(StubQaPrViewTool));
    tools.register(Box::new(RecordingRunGh {
        calls: calls.clone(),
    }));
    (tools, calls)
}

// ---------------------------------------------------------------------------
// Le skill
// ---------------------------------------------------------------------------

/// `qa-review` tel qu'il compte ici : `always_on = true` (sa valeur réelle, donc
/// chargé sur tout tour mika-qa sans dépendre d'un mot-clé dans le message) et
/// les deux `required_tools` que l'en-tête du fichier justifie.
fn qa_review_skill() -> SkillEntry {
    SkillEntry {
        manifest: SkillManifest {
            skill: SkillInfo {
                name: "qa-review".to_string(),
                description: "QA PR review skill".to_string(),
                version: "0.9.1".to_string(),
                always_on: true,
                timeout_secs: 30,
                dependencies: vec![],
                max_prompt_size: None,
                data_grade: Default::default(),
            },
            triggers: Triggers {
                keywords: vec!["review".to_string(), "pr".to_string()],
            },
            llm: Default::default(),
            constraints: Constraints {
                required_tools: vec!["qa_pr_view".to_string(), "run_gh".to_string()],
                required_fetches_for_quoted_resources: false,
            },
            output: Default::default(),
            context: HashMap::new(),
            variants: Default::default(),
        },
        dir: PathBuf::from("/skills/qa-review"),
        keywords_lower: vec!["review".to_string(), "pr".to_string()],
        prompt_snippet: "You are running the qa-review skill.".to_string(),
        skill_tools: vec![ResolvedSkillTool {
            definition: ToolDefinition {
                name: "qa_pr_view".to_string(),
                description: "View PR details for QA review".to_string(),
                input_schema: json!({"type": "object"}),
            },
            handler: ToolHandler::Builtin {
                function: "qa_pr_view".to_string(),
            },
            skill_dir: PathBuf::from("/skills/qa-review"),
        }],
        enabled: true,
        has_override: false,
        provider_overrides: HashMap::new(),
        prompt_sources: SkillEntry::empty_prompt_sources(),
        model_overrides: HashMap::new(),
    }
}

fn qa_registry() -> SkillRegistry {
    SkillRegistry::from_test_entries(vec![qa_review_skill()])
}

// ---------------------------------------------------------------------------
// Lecture des requêtes capturées
// ---------------------------------------------------------------------------

/// Les messages `User` d'une requête capturée, à plat.
fn user_texts(req: &LlmRequest) -> Vec<String> {
    req.messages
        .iter()
        .filter(|m| matches!(m.role, LlmRole::User))
        .filter_map(|m| match &m.content {
            LlmContent::Text(t) => Some(t.clone()),
            LlmContent::Blocks(_) => None,
        })
        .collect()
}

/// Combien de messages `User` de **cette** requête portent la consigne.
///
/// C'est la quantité qui dit « re-prompté une fois », et non le nombre de
/// requêtes qui la portent : l'historique s'accumule, donc une injection unique
/// se voit dans toutes les requêtes suivantes. Le compte qui compte est celui
/// des injections dans la dernière requête.
fn injections(req: &LlmRequest) -> usize {
    user_texts(req)
        .iter()
        .filter(|t| t.contains(REVIEW_SUBMISSION_CORRECTION))
        .count()
}

/// Les appels `run_gh` qui sont des `pr review`, lus comme le prédicat du
/// moteur les lit — sur l'argv, jamais sur le texte de sortie.
fn pr_review_calls(calls: &[Value]) -> usize {
    calls
        .iter()
        .filter(|c| {
            let argv = c["command"].as_array();
            argv.map(|a| a.first() == Some(&json!("pr")) && a.get(1) == Some(&json!("review")))
                .unwrap_or(false)
        })
        .count()
}

// ---------------------------------------------------------------------------
// Capture de journal (mika#2732) — même forme que
// `test_context_scope_observability_2305.rs`
// ---------------------------------------------------------------------------

type Captured = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let message = fields.remove("message").unwrap_or_default();
        if let Ok(mut events) = self.events.lock() {
            events.push((message, fields));
        }
    }
}

struct FieldVisitor<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for FieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
}

/// Un dispatcher gardé vivant pour tout le binaire de test, afin que le nôtre ne
/// soit jamais le seul.
///
/// `tracing-core` bascule en mode « un seul dispatcher » dès qu'un unique
/// dispatcher scopé est vivant ; dans ce mode un callsite enregistré pour la
/// première fois calcule son intérêt depuis le défaut du **thread qui
/// l'enregistre**. Le callsite de `qa_turn_ended_without_review` est frappé par
/// les tours des autres tests de ce binaire, la plupart sur d'autres threads
/// sans souscripteur : celui qui y arrive le premier pendant que notre capture
/// est le seul dispatcher met en cache `Interest::never` pour tout le monde, et
/// ce thread-ci ne voit alors plus rien. Un second dispatcher vivant maintient
/// le registre en mode multi-dispatcher, où l'intérêt est dérivé de tous les
/// dispatchers vivants et tranché par événement (mika#2646).
fn keep_registry_multi_dispatcher() {
    static KEEPER: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
    KEEPER.get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
}

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    keep_registry_multi_dispatcher();
    let events: Captured = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    // Enregistrer notre dispatcher reconstruit tous les intérêts mis en cache
    // contre les dispatchers vivants, ce qui efface un `never` caché avant nous.
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

/// Les lignes `qa_turn_ended_without_review` vues depuis le début de la capture.
fn turn_ended_lines(events: &Captured) -> Vec<HashMap<String, String>> {
    let captured = events.lock().expect("capture mutex");
    captured
        .iter()
        .filter(|(_, f)| {
            f.get("event").map(String::as_str)
                == Some(mika_agent::qa_review_submission::TURN_ENDED_WITHOUT_REVIEW_EVENT)
        })
        .map(|(_, f)| f.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// AC3a — le test nommé par le ticket
// ---------------------------------------------------------------------------

/// La séquence mesurée : `qa_pr_view`, puis un `run_gh` en **lecture**
/// (`pr list --head …`), puis le verdict en texte et `EndTurn`. Les deux
/// `required_tools` sont satisfaits à cet instant — c'est tout le défaut. Après
/// mika#2727 : re-prompt nommant la soumission, puis la revue est **postée**.
#[tokio::test]
async fn ac3a_un_tour_qui_ne_lit_que_pr_list_est_re_prompte_et_finit_par_poster() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            // Pas 1 — l'outil du skill.
            tool_call_response("qa_pr_view", json!({"pr": 2704})),
            // Pas 2 — `run_gh` en lecture seule. Satisfait le terme `run_gh`
            // de `required_tools` et ne soumet rien.
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", HEAD, "--json", "number,headRefOid"]}),
            ),
            // Pas 3 — le verdict en texte, et EndTurn. La forme exacte des deux
            // incidents du 2026-10-15.
            text_response("Revue terminée : tout est vert.\n\nVERDICT: pass"),
            // Pas 4 (après re-prompt) — la revue est postée.
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2704", "--approve", "--body",
                    "## PLAN-AC VERIFICATION\n…\n\nVERDICT: pass"]}),
            ),
            // Pas 5 — EndTurn, la garde est satisfaite.
            text_response("VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&pr_synchronize_event()).await.unwrap();

    // AC3a, moitié qui compte — une revue a été POSTÉE.
    let calls = gh_calls.lock().unwrap();
    assert_eq!(
        pr_review_calls(&calls),
        1,
        "exactement un `run_gh pr review`, vu {calls:?}"
    );
    drop(calls);

    // La garde a tiré une fois, et après le premier EndTurn.
    let requests = &trace.captured_requests;
    assert_eq!(
        requests.len(),
        5,
        "qa_pr_view → pr list → texte → [correction] → pr review → texte = 5 appels LLM"
    );
    assert_eq!(
        injections(&requests[2]),
        0,
        "la consigne n'existe pas avant le premier EndTurn"
    );
    assert_eq!(
        injections(&requests[3]),
        1,
        "la consigne est injectée une fois, juste après le premier EndTurn"
    );
    assert_eq!(
        injections(&requests[4]),
        1,
        "et une seule fois : la garde ne re-prompte pas sur un tour déjà satisfait"
    );
    assert_eq!(
        trace.llm_call_count, 5,
        "un appel de plus que la séquence sans garde (cf. ac3b, qui en fait 3)"
    );

    // La consigne nomme ce qu'il faut faire, pas seulement ce qui manque.
    let consigne = user_texts(&requests[3])
        .into_iter()
        .find(|t| t.contains(REVIEW_SUBMISSION_CORRECTION))
        .expect("la consigne est dans un message User");
    for terme in ["run_gh", "pr review", "VERDICT:", "pr list"] {
        assert!(
            consigne.contains(terme),
            "la consigne doit nommer `{terme}` ; reçue : {consigne}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3b — contrôle positif, dans le même fichier
// ---------------------------------------------------------------------------

/// Séquence identique à AC3a à **un argument près** : `pr review --approve` au
/// lieu de `pr list`. Rien ne doit être injecté. C'est la moitié qui dit que la
/// garde lit la soumission et non le nom de l'outil : les deux tours appellent
/// `run_gh` exactement une fois, avec le même nom, et seul celui qui n'a rien
/// soumis est re-prompté.
#[tokio::test]
async fn ac3b_un_tour_qui_poste_sa_revue_nest_pas_re_prompte() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("qa_pr_view", json!({"pr": 2704})),
            // Le seul écart avec AC3a.
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2704", "--approve", "--body",
                    "## PLAN-AC VERIFICATION\n…\n\nVERDICT: pass"]}),
            ),
            text_response("Revue soumise.\n\nVERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&pr_synchronize_event()).await.unwrap();

    let calls = gh_calls.lock().unwrap();
    assert_eq!(pr_review_calls(&calls), 1, "la revue est postée une fois");
    drop(calls);

    let requests = &trace.captured_requests;
    assert_eq!(
        requests.len(),
        3,
        "qa_pr_view → pr review → texte = 3 appels LLM, aucun re-prompt"
    );
    assert_eq!(
        trace.llm_call_count, 3,
        "la séquence sans garde : c'est le témoin du `+1` d'AC3a"
    );
    for (i, req) in requests.iter().enumerate() {
        assert_eq!(
            injections(req),
            0,
            "aucune consigne ne doit apparaître (requête {i})"
        );
    }
}

// ---------------------------------------------------------------------------
// AC1 — « re-prompté UNE fois »
// ---------------------------------------------------------------------------

/// Le re-prompt échoue aussi : le modèle re-rend du texte sans rien poster. Le
/// tour doit **sortir**, pas boucler — le budget `intent_guard_retries` est à un
/// coup, et un tour de revue qui bouclerait brûlerait les 20 pas du loop sur un
/// défaut que la garde ne peut pas corriger seule. C'est la population que AC2
/// (`qa_turn_ended_without_review`) rend visible à l'opérateur, épinglée plus
/// bas par `mika2732_…` ; le complémentaire qui l'arme est
/// `qa_review_submission::submission_unmet_after_retry`.
///
/// `#[serial]` : ce tour frappe le **même callsite** que le test de capture
/// mika#2732. Sans la sérialisation, ce test-ci peut enregistrer le callsite
/// depuis un thread sans souscripteur pendant que la capture est le seul
/// dispatcher vivant, et y cacher `Interest::never` pour tout le binaire — la
/// capture voit alors zéro ligne, par intermittence (mika#2646). Le voisin doit
/// porter l'attribut autant que le captureur : un seul des deux ne suffit pas.
#[tokio::test]
#[serial_test::serial]
async fn ac1_le_re_prompt_est_a_un_coup_la_garde_ne_boucle_pas() {
    let (tools, gh_calls) = tools_with_recording_run_gh();
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("qa_pr_view", json!({"pr": 2704})),
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", HEAD, "--json", "number"]}),
            ),
            text_response("Revue terminée : tout est vert.\n\nVERDICT: pass"),
            // Le re-prompt échoue : encore du texte, toujours rien de posté.
            text_response("Comme je le disais, VERDICT: pass"),
        ])
        .tools(tools)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();

    let trace = harness.run(&pr_synchronize_event()).await.unwrap();

    let calls = gh_calls.lock().unwrap();
    assert_eq!(
        pr_review_calls(&calls),
        0,
        "ce tour ne poste rien : c'est la population d'AC2"
    );
    drop(calls);

    let requests = &trace.captured_requests;
    assert_eq!(
        requests.len(),
        4,
        "qa_pr_view → pr list → texte → [correction] → texte = 4 appels LLM, \
         puis le tour SORT"
    );
    assert_eq!(
        injections(&requests[3]),
        1,
        "une seule injection, même sur un tour qui ne se corrige pas"
    );
}

// ---------------------------------------------------------------------------
// AC2 (mika#2732) — le tour muet le dit, le tour sain se tait
// ---------------------------------------------------------------------------

/// Les deux séquences voisines, **sous une seule capture**, parce que c'est la
/// paire qui est le livrable.
///
/// Tour A est celui d'`ac1_…` : le re-prompt échoue aussi, rien n'est posté. Il
/// doit produire la ligne. Tour B est celui d'`ac3b_…` : la revue est postée, à
/// un argument près du précédent. Il ne doit rien produire. Un test qui
/// n'asserterait que la présence passerait sur un `warn!` inconditionnel —
/// c'est-à-dire sur un instrument qui crie à chaque tour de revue et qu'un
/// opérateur apprend en une semaine à ne plus lire (motif mika#2272).
///
/// Le compte est **exactement un** sur les deux tours réunis, et les valeurs
/// `repo`/`pr` sont assertées, pas seulement la présence des champs : c'est le
/// contrôle de non-vacuité demandé par l'AC2.4(c) du ticket, porté ici par le
/// couple dépôt/numéro puisque la tête part en vol 2.
///
/// **Pas de champ de tête, et c'est la décision D1/D2 du plan.** Le fil ne porte
/// que la branche ; l'émettre s'appelle `branch` et demande d'étendre l'unique
/// lecteur et ses trois appelants. Un champ `head` portant une branche se relit
/// comme un SHA manquant (motif mika#2331), donc rien plutôt que faux : ce test
/// n'asserte **pas** l'absence d'un champ futur, il n'en nomme aucun.
///
/// `#[serial]` : voir `ac1_…`, qui le porte pour la même raison (mika#2646).
#[tokio::test]
#[serial_test::serial]
async fn mika2732_un_tour_de_revue_muet_emet_sa_ligne_et_le_tour_sain_non() {
    let (_guard, events) = capture();

    // --- Tour A : le re-prompt échoue aussi (séquence d'`ac1_…`). ---
    let (tools_a, gh_a) = tools_with_recording_run_gh();
    let harness_a = EvalHarness::builder()
        .responses(vec![
            tool_call_response("qa_pr_view", json!({"pr": 2704})),
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "list", "--head", HEAD, "--json", "number"]}),
            ),
            text_response("Revue terminée : tout est vert.\n\nVERDICT: pass"),
            text_response("Comme je le disais, VERDICT: pass"),
        ])
        .tools(tools_a)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();
    harness_a.run(&pr_synchronize_event()).await.unwrap();
    assert_eq!(
        pr_review_calls(&gh_a.lock().unwrap()),
        0,
        "le tour A doit vraiment ne rien soumettre, sinon il ne prouve rien"
    );

    let after_a = turn_ended_lines(&events);
    assert_eq!(
        after_a.len(),
        1,
        "le tour muet doit émettre exactement une ligne ; vu {after_a:?}"
    );
    let ligne = &after_a[0];
    assert_eq!(
        ligne.get("repo").map(String::as_str),
        Some("senara-solutions/mika"),
        "le dépôt vient du lecteur unique et doit porter sa valeur : {ligne:?}"
    );
    assert_eq!(
        ligne.get("pr").map(String::as_str),
        Some("2704"),
        "le numéro de PR est la clé par laquelle l'opérateur retrouve le tour : {ligne:?}"
    );
    for champ in ["trace_id", "agent_id"] {
        assert!(
            ligne.get(champ).is_some_and(|v| !v.is_empty()),
            "`{champ}` doit être renseigné — c'est par là qu'on rejoint la trace : {ligne:?}"
        );
    }

    // --- Tour B : la revue est postée (séquence d'`ac3b_…`). ---
    let (tools_b, gh_b) = tools_with_recording_run_gh();
    let harness_b = EvalHarness::builder()
        .responses(vec![
            tool_call_response("qa_pr_view", json!({"pr": 2704})),
            tool_call_response(
                "run_gh",
                json!({"command": ["pr", "review", "2704", "--approve", "--body",
                    "## PLAN-AC VERIFICATION\n…\n\nVERDICT: pass"]}),
            ),
            text_response("Revue soumise.\n\nVERDICT: pass"),
        ])
        .tools(tools_b)
        .skills(qa_registry())
        .build()
        .await
        .unwrap();
    harness_b.run(&pr_synchronize_event()).await.unwrap();
    assert_eq!(
        pr_review_calls(&gh_b.lock().unwrap()),
        1,
        "le tour B doit vraiment poster sa revue, sinon le contrôle négatif est vide"
    );

    let after_b = turn_ended_lines(&events);
    assert_eq!(
        after_b.len(),
        1,
        "le tour sain n'ajoute rien : toujours une seule ligne sur les deux tours ; \
         vu {after_b:?}"
    );
}
