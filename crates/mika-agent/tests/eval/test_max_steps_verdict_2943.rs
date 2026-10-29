//! mika#2943 — **le contrôle négatif** (règle 13, mika#2264) : l'état que ce
//! correctif rend inatteignable, et le détecteur qui rougit s'il se reproduit.
//!
//! # L'état interdit
//!
//! > Un tour **webhook** de mika-qa, déclenché par un événement qui nomme une
//! > PR, coupé par son budget de pas d'outil, sans revue postée dans la
//! > session — et **la PR ne reçoit rien**.
//!
//! Dans les symboles de cette PR, les quatre conjoints :
//!
//! | conjoint | comment ce fichier l'établit |
//! |---|---|
//! | le tour a été coupé au plafond | `run_shell` a servi exactement `max_tool_steps_for_agent("mika-qa", …)` appels, et la sentinelle du corpus n'a jamais été consommée |
//! | l'événement nomme une PR | le texte est celui du gateway pour `pull_request.ready_for_review` sur #2920 |
//! | la session n'a pas posté de revue | le registre `pr_reviews_posted` est vide et le `run_gh` du tour n'a jamais été appelé |
//! | rien n'atteint la PR | zéro POST capturé par le poster injecté — ou un POST qui ne porte pas `hold[review]` sur **cette** PR |
//!
//! Les six tests `mika2943_*` de `server::deadline_verdict` et de
//! `qa_build_callback` sont des contrôles **positifs** : ils partent d'un
//! `TurnConclusion::MaxStepsExceeded` fabriqué à la main et attestent que le
//! filet en tire le bon verdict. Aucun d'eux ne peut voir le joint où le défaut
//! vivait — entre ce que la boucle *sait* de sa coupure et ce que le call-site
//! en *lit*. C'est très exactement le joint que ce fichier traverse, et c'est
//! pour cela qu'il rougit sur le code d'avant là où les six resteraient verts
//! (ils ne compileraient pas : leur vocabulaire est celui de la réparation).
//!
//! # Pourquoi l'assertion est une négation, et non le contrat
//!
//! Le contrat (« un `hold[review]` est posté ») est tenu ailleurs : par
//! `mika2943_a_max_steps_cut_webhook_turn_posts_exactly_one_hold`. Ce fichier
//! asserte l'absence de la conjonction ci-dessus. La différence porte : un
//! futur changement qui ferait *disparaître* la coupure du corpus — plafond
//! relevé, continuation réarrangée, garde webhook qui conclut plus tôt — rendrait
//! le contrat vert par vacuité. Les trois premiers conjoints sont donc assertés
//! comme **préconditions dures** avant la négation, qui ne peut alors jamais
//! passer pour la mauvaise raison.
//!
//! # Rouge-avant, recette exécutable par le relecteur
//!
//! 1. Ramener les sources au parent du correctif :
//!    `git checkout origin/main -- crates/mika-agent/src crates/mika-agent/tests/eval/test_deadline_verdict_2276.rs crates/mika-agent/tests/eval/test_error_verdict_2289.rs crates/mika-agent/tests/eval/grounding_assertions/mod.rs`
//! 2. Dans [`conclusion_of`] ci-dessous, rendre la ligne d'avant —
//!    `TurnConclusion::from(output.deadline_exceeded)`, celle que
//!    `handlers.rs:1558` exécutait. C'est la **seule** ligne du fichier que le
//!    rouge réécrit, parce que le code d'avant n'avait pas d'autre
//!    convertisseur à ce joint : `AgentOutput` n'y portait aucun fait de
//!    coupure par pas, et c'est le défaut même.
//! 3. `cargo test -p mika-agent --test eval mika2943_un_tour_webhook`
//! 4. Le détecteur rougit sur la conjonction : le tour est bien coupé à 30 pas,
//!    la session n'a rien posté, et `deadline_verdict_target` rend `None` —
//!    zéro POST, PR muette. C'est le symptôme de la trace `b22e6ea0`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dashmap::DashMap;
use mika_agent::agent_loop::AgentOutput;
use mika_agent::planning::policy::{MAX_TOOL_STEPS, max_tool_steps_for_agent};
use mika_agent::server::deadline_verdict::{
    DEADLINE_VERDICT_LINE, DeadlineVerdictInput, DeadlineVerdictOutcome, PostReviewRequest,
    TurnConclusion, deadline_verdict_target, maybe_post_deadline_verdict,
};
use mika_agent::tools::{Tool, ToolContext, ToolOutput, ToolRegistry, default_tools};
use mika_common::claude::ToolDefinition;
use mika_common::llm::mock::*;
use serde_json::{Value, json};

use super::harness::EvalHarness;

/// L'événement mesuré : la sortie du draft de #2920, 2026-10-28T06:42:09Z,
/// trace `b22e6ea0`. Forme exacte de `pull_request.ready_for_review` telle que
/// `mika_gateway::github::format_event_text` l'émet.
const READY_FOR_REVIEW_2920: &str = concat!(
    "[GitHub] PR ready_for_review: senara-solutions/mika#2920 — ",
    "feat(2926): le plafond de pas de mika-qa (branch: fix/2926)\n",
    "https://github.com/senara-solutions/mika/pull/2920"
);

const TRACE_ID: &str = "b22e6ea0b4d911f19f3e90c3b908c45a";
const SESSION_ID: &str = "webhook-2920-ready-for-review";
const REPO: &str = "senara-solutions/mika";
const PR: u64 = 2920;

/// Ce que `server::handlers::conclusion_of` exécute sur la branche `Ok`.
///
/// Isolé dans une fonction nommée pour une seule raison : c'est le joint où le
/// défaut vivait, donc la seule ligne que la recette rouge du module ci-dessus
/// réécrit.
///
/// * après (mika#2943) : `TurnConclusion::of_output(output)` — total sur les
///   deux bornes du tour ;
/// * avant : `TurnConclusion::from(output.deadline_exceeded)` — ne lisait qu'un
///   champ, et une coupure par pas lui arrivait comme `Concluded`.
fn conclusion_of(output: &AgentOutput) -> TurnConclusion {
    TurnConclusion::of_output(output)
}

// ---------------------------------------------------------------------------
// Le corpus
// ---------------------------------------------------------------------------

/// Le journal d'appels d'un outil : les entrées reçues, dans l'ordre.
type CallLog = Arc<Mutex<Vec<Value>>>;

/// Un outil qui enregistre ses appels et ne fait rien d'autre.
///
/// `run_shell` et `run_gh` sont des outils de skill, absents de
/// `default_tools()` : la fixture est donc le seul fournisseur du nom, et aucun
/// `gh` ni aucun shell n'est exécuté. Même forme que la fixture de
/// `test_qa_step_budget_2926`, dont ce fichier est le voisin — elle y est
/// privée, et la partager demanderait de rendre `pub` un détail de test dans un
/// module qui atteste autre chose.
struct Recording {
    name: &'static str,
    calls: CallLog,
}

#[async_trait]
impl Tool for Recording {
    fn name(&self) -> &str {
        self.name
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.to_string(),
            description: format!("{} (fixture mika#2943)", self.name),
            input_schema: json!({"type": "object"}),
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        Ok(ToolOutput::success("ok"))
    }
}

/// Le registre du tour : `run_shell` (l'exploration qui brûle le budget) et
/// `run_gh` (la revue, qui dans ce corpus ne vient jamais).
fn qa_tools() -> (ToolRegistry, CallLog, CallLog) {
    let shell_calls: CallLog = Arc::new(Mutex::new(Vec::new()));
    let gh_calls: CallLog = Arc::new(Mutex::new(Vec::new()));
    let mut tools = default_tools();
    tools.register(Box::new(Recording {
        name: "run_shell",
        calls: shell_calls.clone(),
    }));
    tools.register(Box::new(Recording {
        name: "run_gh",
        calls: gh_calls.clone(),
    }));
    (tools, shell_calls, gh_calls)
}

/// La forme mesurée de `b22e6ea0` : le tour lit, puis enchaîne des `run_shell`
/// au-delà de son plafond, et la revue n'arrive jamais.
///
/// Exactement `ceiling` appels d'outil, puis le texte que la continuation rend,
/// puis une sentinelle. `MockLlmProvider` panique à l'épuisement : si la boucle
/// servait un pas de plus, la continuation consommerait la sentinelle et
/// l'assertion de précondition sur le décompte rougirait — le test ne peut pas
/// passer pour la mauvaise raison.
fn burned_budget_corpus(ceiling: usize) -> Vec<MockResponse> {
    let mut responses: Vec<MockResponse> = (0..ceiling)
        .map(|i| {
            tool_call_response(
                "run_shell",
                json!({"command": format!("git show HEAD~{i}:docs/plans/plan.md")}),
            )
        })
        .collect();
    responses.push(text_response(
        "J'ai lu le diff et le plan, la revue n'est pas encore rédigée.",
    ));
    responses.push(text_response("(sentinelle : ne doit jamais être rendue)"));
    responses
}

// ---------------------------------------------------------------------------
// La chaîne de production, en un terme
// ---------------------------------------------------------------------------

/// Ce que la chaîne a décidé, et ce qu'elle a émis.
struct NetDecision {
    /// `None` = `deadline_verdict_target` n'a pas armé le filet.
    outcome: Option<DeadlineVerdictOutcome>,
    /// Le nombre de POST que le poster injecté a vus passer.
    posts: usize,
    /// La dernière requête capturée, telle que la PR l'aurait reçue.
    captured: Option<PostReviewRequest>,
}

/// Ce que le call-site webhook exécute, et ce que la PR en reçoit.
///
/// La chaîne entière de production — `conclusion_of` → `deadline_verdict_target`
/// → `maybe_post_deadline_verdict` — plus le poster qui capture. Un terme
/// partagé plutôt qu'une copie par test : quatre copies de la chaîne
/// divergeraient, et un test qui n'appellerait plus `conclusion_of` cesserait
/// de traverser le joint où le défaut vivait sans que rien ne le dise
/// (classe mika#2158).
///
/// Pas d'`expect()` sur l'`Option` que `deadline_verdict_target` rend : le refus
/// du call-site est ce que deux des quatre tests de ce fichier **attendent** (le
/// tour non-PR, le build en vol), et un dépliage y paniquerait sur le chemin
/// nominal. Le `None` remonte dans [`NetDecision::outcome`].
async fn what_the_pr_receives(
    output: &AgentOutput,
    event_text: &str,
    registry: &DashMap<String, HashSet<String>>,
) -> NetDecision {
    let captured = Arc::new(Mutex::new(None::<PostReviewRequest>));
    let posts = Arc::new(AtomicUsize::new(0));

    let outcome = match deadline_verdict_target(conclusion_of(output), event_text) {
        // Le refus du call-site : c'est par ici que le défaut sortait.
        None => None,
        Some((reason, target)) => {
            let sink = captured.clone();
            let counter = posts.clone();
            Some(
                maybe_post_deadline_verdict(
                    DeadlineVerdictInput {
                        reason,
                        target,
                        session_id: SESSION_ID,
                        trace_id: TRACE_ID,
                        agent_id: "mika-qa",
                        pr_reviews_posted: Some(registry),
                    },
                    move |req| {
                        counter.fetch_add(1, Ordering::SeqCst);
                        *sink.lock().unwrap() = Some(req);
                        async { Ok("submitted".to_string()) }
                    },
                )
                .await,
            )
        }
    };

    NetDecision {
        outcome,
        posts: posts.load(Ordering::SeqCst),
        captured: captured.lock().unwrap().take(),
    }
}

// ---------------------------------------------------------------------------
// Le détecteur
// ---------------------------------------------------------------------------

/// **Contrôle négatif mika#2943** — l'état interdit du module ci-dessus n'est
/// pas atteignable.
///
/// Ce test ne dit pas « le filet poste » ; il dit « la conjonction *coupé &
/// aucune revue de la session & rien sur la PR* n'arrive pas ». Les trois
/// premiers termes sont des préconditions dures, le quatrième est la négation.
#[tokio::test]
async fn mika2943_un_tour_webhook_coupe_par_ses_pas_ne_laisse_jamais_la_pr_muette() {
    let ceiling = max_tool_steps_for_agent("mika-qa", MAX_TOOL_STEPS);
    let (tools, shell_calls, gh_calls) = qa_tools();

    // Le registre de session, branché comme en production : le MÊME `Arc` est
    // passé au tour et relu par le filet. C'est ce qui rend l'absence de revue
    // mesurée plutôt que supposée — un `run_gh pr review` du tour l'aurait
    // rempli, et le filet s'abstiendrait alors (contrôle d'AC2).
    let registry: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());

    let harness = EvalHarness::builder()
        .responses(burned_budget_corpus(ceiling))
        .tools(tools)
        .agent_id("mika-qa")
        .session_id(SESSION_ID)
        .pr_reviews_posted(registry.clone())
        .build()
        .await
        .expect("harness build");

    let trace = harness.run(READY_FOR_REVIEW_2920).await.expect("agent run");

    // -- Précondition 1 : le tour a bien été coupé à son plafond. -------------
    // Établie sans nommer la réparation : le décompte d'appels de la fixture et
    // la sentinelle non consommée disent la coupure dans un vocabulaire que les
    // deux révisions partagent. Si ce terme était lu dans
    // `output.max_steps_exceeded`, le détecteur serait vert par vacuité sur le
    // code d'avant, où ce champ n'existe pas.
    let served = shell_calls.lock().unwrap().len();
    assert_eq!(
        served, ceiling,
        "précondition : le tour doit brûler exactement son plafond de pas \
         ({ceiling}), or {served} appels servis — sans coupure, ce test \
         n'atteste rien"
    );

    // -- Précondition 2 : la session n'a posté aucune revue. ------------------
    assert!(
        gh_calls.lock().unwrap().is_empty(),
        "précondition : aucun `run_gh` dans ce corpus — c'est la PR sans revue \
         de la trace b22e6ea0"
    );
    assert!(
        registry.is_empty(),
        "précondition : le registre de session est vide, donc la \
         déduplication de mika#2276 ne peut pas être la cause d'un silence"
    );

    // -- Précondition 3 : l'événement nomme bien une PR. ---------------------
    // Sans elle, un filet qui s'abstiendrait faute de cible passerait pour un
    // filet correct.
    assert!(
        READY_FOR_REVIEW_2920.contains(&format!("{REPO}#{PR}")),
        "précondition : le déclencheur nomme la PR cible"
    );

    // -- Ce que la PR reçoit réellement, par le chemin de production. --------
    let NetDecision {
        outcome,
        posts,
        captured,
    } = what_the_pr_receives(&trace.output, READY_FOR_REVIEW_2920, &registry).await;

    // -- La négation. --------------------------------------------------------
    // « Rien n'atteint la PR » couvre les trois formes du silence : aucun motif,
    // aucun POST, ou un POST qui part ailleurs. Un verdict posté sur une autre
    // PR laisse #2920 tout aussi muette que zéro POST, et serait une régression
    // pire — c'est pourquoi le terme porte sur le contenu capturé, pas sur le
    // compteur seul.
    let reached_the_pr = captured.as_ref().is_some_and(|req| {
        req.repo == REPO
            && req.pr_number == PR
            && req.body.starts_with(DEADLINE_VERDICT_LINE)
            && req.body.contains(&format!("{ceiling} step(s)"))
    });

    assert!(
        reached_the_pr,
        "ÉTAT INTERDIT (mika#2943) : un tour webhook de mika-qa coupé à \
         {ceiling} pas sur {REPO}#{PR}, sans revue postée dans la session, et \
         la PR n'a rien reçu. motif = {outcome:?}, POST = {posts}, capturé = {:?}. \
         C'est la trace b22e6ea0 du 2026-10-28, et les cinq tours de la mesure \
         qui la précèdent.",
        captured.as_ref().map(|r| (
            r.repo.clone(),
            r.pr_number,
            r.body.lines().next().unwrap_or("").to_string()
        )),
    );

    // Le corollaire de la négation : le filet a conclu, et une seule fois. Zéro
    // POST est le défaut ; deux POST réveilleraient `verdict_handler` sur une
    // décision qui n'est pas la sienne.
    assert_eq!(outcome, Some(DeadlineVerdictOutcome::Posted));
    assert_eq!(
        posts, 1,
        "exactement un POST — ni zéro (le défaut), ni deux (la régression)"
    );
}
