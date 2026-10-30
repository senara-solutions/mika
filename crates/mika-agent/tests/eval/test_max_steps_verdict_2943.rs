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
//! # Les quatre tours du fichier (vol 2/2)
//!
//! Le détecteur ci-dessus est le contrôle **positif** du joint. Les trois
//! autres bornent la population sur laquelle le filet s'arme — chacun par un
//! tour réel du relevé du ticket, passé par la **même** chaîne de production
//! ([`what_the_pr_receives`]) :
//!
//! | test | tour mesuré | ce que la PR reçoit, et pourquoi |
//! |---|---|---|
//! | `…ne_laisse_jamais_la_pr_muette` | `b22e6ea0` (#2920, 28/10) | un `hold[review]` — c'est le défaut réparé |
//! | `…apres_revue_postee_najoute_rien_sur_la_pr` | `e6e7d8e0` (#2941, 27/10) | rien : la session a déjà revu (`AlreadyReviewed`) |
//! | `…un_tour_non_pr_coupe_ne_poste_rien` | la forme `check_suite` sur `main` | rien : aucune PR n'est nommée |
//! | `…avec_un_build_en_vol_laisse_le_verdict_au_callback` | `11699630` (#2920), `517380e0` (#2932) | rien : le callback du build portera le verdict (D1) |
//!
//! Les trois derniers mesurent un **silence**, et un silence est vrai de trop
//! de choses : chacun porte donc des préconditions dures qui établissent que le
//! tour a bien été coupé et que le fait de coupure a bien été posé — sans quoi
//! le défaut d'avant les rendrait tous verts.
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
use mika_agent::qa_build_callback::BUILD_MIKA_TOOL;
use mika_agent::server::deadline_verdict::{
    DEADLINE_VERDICT_LINE, DeadlineVerdictInput, DeadlineVerdictOutcome, PostReviewRequest,
    TurnConclusion, deadline_verdict_target, maybe_post_deadline_verdict,
};
use mika_agent::skills::builtin_handlers::format_pr_dedup_key;
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

/// Un événement webhook réel qui ne nomme **aucune** PR : la forme qu'émet
/// `server::webhook_queue::CHECK_SUITE_RE`, sur la branche par défaut.
///
/// Choisi plutôt qu'un texte Telegram parce que c'est un **webhook**, donc le
/// même call-site : le terme sous test est l'absence de PR dans le texte, pas
/// l'absence de webhook.
const CHECK_SUITE_ON_MAIN: &str =
    "[GitHub] Check suite success on senara-solutions/mika (branch: main)";

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
    /// Ce que l'outil écrit dans le registre de session quand il réussit.
    ///
    /// C'est le maillon qu'aucun test d'unité ne voit : en production,
    /// `run_gh` sur un `pr review` qui réussit inscrit la clé de déduplication
    /// dans `pr_reviews_posted`, et c'est cette écriture-là que le filet relit.
    /// Un champ plutôt qu'un second `impl Tool` — la fixture reste une seule
    /// forme, et ce que l'outil écrit se lit à son site de construction.
    on_success: OnSuccess,
}

/// Le crochet d'écriture au registre, reçu tel que `ToolContext` le porte : le
/// registre de la session, et l'identifiant de session **du tour** — jamais la
/// constante du fichier, qui serait supposée plutôt que mesurée.
type OnSuccess = Option<Box<dyn Fn(&DashMap<String, HashSet<String>>, &str) + Send + Sync>>;

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

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> anyhow::Result<ToolOutput> {
        self.calls.lock().unwrap().push(input);
        if let (Some(hook), Some(registry)) = (self.on_success.as_ref(), ctx.pr_reviews_posted) {
            hook(registry, ctx.session_id);
        }
        Ok(ToolOutput::success("ok"))
    }
}

/// Le registre du tour : `run_shell` (l'exploration qui brûle le budget) et
/// `run_gh` (la revue, qui dans ce corpus ne vient jamais).
fn qa_tools() -> (ToolRegistry, CallLog, CallLog) {
    qa_tools_with(None)
}

/// Le même registre, avec un `run_gh` qui inscrit au registre de session ce
/// qu'un `pr review` réussi y inscrirait.
fn qa_tools_with(on_gh_success: OnSuccess) -> (ToolRegistry, CallLog, CallLog) {
    let shell_calls: CallLog = Arc::new(Mutex::new(Vec::new()));
    let gh_calls: CallLog = Arc::new(Mutex::new(Vec::new()));
    let mut tools = default_tools();
    tools.register(Box::new(Recording {
        name: "run_shell",
        calls: shell_calls.clone(),
        on_success: None,
    }));
    tools.register(Box::new(Recording {
        name: "run_gh",
        calls: gh_calls.clone(),
        on_success: on_gh_success,
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

/// La forme mesurée de `e6e7d8e0` (#2941, 27/10 15:38) : le tour **poste sa
/// revue**, puis brûle le reste de son budget.
///
/// Même longueur que [`burned_budget_corpus`] à l'appel près — le premier pas
/// est le `gh pr review`, les `ceiling - 1` suivants sont l'exploration. Le
/// tour est donc coupé exactement de la même façon ; la seule différence est
/// qu'une revue est déjà partie.
fn reviewed_then_burned_budget_corpus(ceiling: usize) -> Vec<MockResponse> {
    let mut responses = vec![tool_call_response(
        "run_gh",
        json!({
            "command": ["pr", "review", PR.to_string(), "--comment", "--body", "VERDICT: pass"],
            "repo": REPO,
        }),
    )];
    responses.extend((0..ceiling - 1).map(|i| {
        tool_call_response(
            "run_shell",
            json!({"command": format!("git show HEAD~{i}:docs/plans/plan.md")}),
        )
    }));
    responses.push(text_response(
        "La revue est postée ; j'ai continué à lire le plan jusqu'au plafond.",
    ));
    responses.push(text_response("(sentinelle : ne doit jamais être rendue)"));
    responses
}

/// La forme mesurée de `11699630` (#2920, 26/10 15:13) et de `517380e0`
/// (#2932, 27/10 00:58) : le tour **dispatche un build**, puis brûle le reste
/// de son budget. Le verdict reviendra par le callback de ce build.
///
/// Le nom de l'outil vient de la constante que lit
/// `qa_build_callback::build_callback_dispatched_in_turn` — un littéral recopié
/// ici laisserait le test vert si la constante changeait, et l'abstention qu'il
/// atteste ne porterait plus sur rien.
fn build_then_burned_budget_corpus(ceiling: usize) -> Vec<MockResponse> {
    let mut responses = vec![tool_call_response(
        BUILD_MIKA_TOOL,
        json!({"branch": "fix/2926"}),
    )];
    responses.extend((0..ceiling - 1).map(|i| {
        tool_call_response(
            "run_shell",
            json!({"command": format!("git show HEAD~{i}:docs/plans/plan.md")}),
        )
    }));
    responses.push(text_response(
        "Le build est lancé ; j'ai continué à lire le plan jusqu'au plafond.",
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

/// **AC2, second contrôle** — un tour **non-PR** coupé par ses pas ne poste
/// rien, et ce n'est pas un défaut.
///
/// Le ticket le dit en une phrase : *« Un tour non-PR coupé ne poste rien. »*
/// Le filet n'a pas de cible, donc il n'a rien à dire : le silence est ici le
/// comportement correct, pas le symptôme de `b22e6ea0`. Ce que ce test interdit
/// est l'inverse — qu'un filet élargi à `MaxStepsExceeded` se mette à chercher
/// une PR dans un texte qui n'en nomme pas, et en trouve une par accident
/// (le numéro d'une issue, un `#N` dans un titre de commit).
///
/// La seconde précondition porte tout : sans elle, un filet qui s'abstient
/// **parce que la boucle n'a rien posé** passerait pour un filet qui s'abstient
/// faute de cible, et le test serait vert pour la mauvaise raison.
#[tokio::test]
async fn mika2943_un_tour_non_pr_coupe_ne_poste_rien() {
    let ceiling = max_tool_steps_for_agent("mika-qa", MAX_TOOL_STEPS);
    let (tools, shell_calls, _gh_calls) = qa_tools();
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

    let trace = harness.run(CHECK_SUITE_ON_MAIN).await.expect("agent run");

    // -- Précondition 1 : le tour a bien été coupé à son plafond. -------------
    // Même mesure qu'au test AC1 : le décompte d'appels servis, et la
    // sentinelle du corpus qu'un pas de plus aurait consommée — `MockLlmProvider`
    // paniquerait alors à l'épuisement.
    let served = shell_calls.lock().unwrap().len();
    assert_eq!(
        served, ceiling,
        "précondition : le tour doit brûler exactement son plafond de pas \
         ({ceiling}), or {served} appels servis — sans coupure, ce test \
         n'atteste rien"
    );

    // -- Précondition 2 : le fait de coupure est bien posé. ------------------
    // C'est ce que le filet lit. Un `None` ici rendrait l'abstention triviale :
    // le call-site verrait `Concluded`, exactement comme avant le correctif, et
    // l'absence de POST ne dirait plus rien de l'absence de cible.
    assert!(
        trace.output.max_steps_exceeded.is_some(),
        "précondition : la boucle doit poser le fait de coupure — sans lui, \
         l'abstention mesurée plus bas serait celle du défaut, pas celle de \
         l'absence de PR"
    );

    // -- Précondition 3 : le déclencheur ne nomme aucune PR. -----------------
    assert!(
        !CHECK_SUITE_ON_MAIN.contains('#') && !CHECK_SUITE_ON_MAIN.contains("/pull/"),
        "précondition : le déclencheur ne porte ni `repo#N` ni URL de PR"
    );

    let NetDecision {
        outcome,
        posts,
        captured,
    } = what_the_pr_receives(&trace.output, CHECK_SUITE_ON_MAIN, &registry).await;

    assert!(
        outcome.is_none(),
        "un tour non-PR coupé n'arme pas le filet : il n'y a pas de PR à qui \
         dire quoi que ce soit. motif = {outcome:?}"
    );
    assert_eq!(
        posts, 0,
        "aucun POST sur un événement qui ne nomme pas de PR — capturé = {captured:?}"
    );
}

/// **AC2, premier contrôle** — une coupure qui suit une revue déjà postée
/// n'ajoute rien sur la PR.
///
/// C'est la trace `e6e7d8e0` (#2941, 27/10 15:38), que le ticket compte hors de
/// la population du défaut : le tour a été coupé, mais la PR a eu sa revue.
/// Reposer un `hold[review]` par-dessus serait une régression **pire** que le
/// silence qu'on répare — deux verdicts sur une PR réveillent `verdict_handler`
/// deux fois sur une décision déjà prise.
///
/// Le maillon que les tests d'unité du filet ne voient pas est l'écriture au
/// registre : eux la posent à la main, ici c'est la fixture `run_gh` qui
/// l'exécute au moment où le tour poste sa revue, avec la clé que
/// [`format_pr_dedup_key`] fabrique — jamais une clé recomposée, sans quoi le
/// test deviendrait un troisième lecteur du format (mika#2158).
#[tokio::test]
async fn mika2943_une_coupure_apres_revue_postee_najoute_rien_sur_la_pr() {
    let ceiling = max_tool_steps_for_agent("mika-qa", MAX_TOOL_STEPS);
    let registry: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());

    // Ce que le handler de production fait sur un `gh pr review` qui réussit.
    let (tools, shell_calls, gh_calls) = qa_tools_with(Some(Box::new(|registry, session_id| {
        registry
            .entry(session_id.to_string())
            .or_default()
            .insert(format_pr_dedup_key(Some(REPO), &PR.to_string()));
    })));

    let harness = EvalHarness::builder()
        .responses(reviewed_then_burned_budget_corpus(ceiling))
        .tools(tools)
        .agent_id("mika-qa")
        .session_id(SESSION_ID)
        .pr_reviews_posted(registry.clone())
        .build()
        .await
        .expect("harness build");

    let trace = harness.run(READY_FOR_REVIEW_2920).await.expect("agent run");

    // -- Précondition 1 : le tour a bien été coupé à son plafond. -------------
    // `ceiling - 1` parce que le premier pas est la revue. La sentinelle du
    // corpus n'a pas été consommée, sans quoi `MockLlmProvider` aurait paniqué.
    let served = shell_calls.lock().unwrap().len();
    assert_eq!(
        served,
        ceiling - 1,
        "précondition : après la revue, le tour doit brûler le reste de son \
         plafond ({} pas), or {served} appels servis",
        ceiling - 1
    );
    assert_eq!(
        gh_calls.lock().unwrap().len(),
        1,
        "précondition : la revue est postée une fois, au premier pas"
    );

    // -- Précondition 2 : le fait de coupure est bien posé. ------------------
    // Aucun build n'est en vol dans ce corpus, donc l'abstention de R4 ne peut
    // pas être la cause du silence mesuré plus bas.
    assert!(
        trace.output.max_steps_exceeded.is_some(),
        "précondition : la boucle pose le fait de coupure — sinon le filet ne \
         s'arme pas, et son silence ne dirait rien de la déduplication"
    );

    // -- Précondition 3 : le registre porte exactement une clé. --------------
    // La revue du tour, et rien d'autre. Un registre vide rendrait l'assertion
    // sans objet ; deux clés voudraient dire que la fixture écrit deux fois.
    let keys_for_session = registry
        .get(SESSION_ID)
        .map(|entry| entry.len())
        .unwrap_or(0);
    assert_eq!(
        keys_for_session, 1,
        "précondition : la fixture a inscrit la revue du tour, une fois — \
         registre = {registry:?}"
    );

    let NetDecision {
        outcome,
        posts,
        captured,
    } = what_the_pr_receives(&trace.output, READY_FOR_REVIEW_2920, &registry).await;

    // Les deux termes comptent. `posts == 0` seul serait aussi vrai d'un filet
    // jamais armé — c'est-à-dire du défaut que cette PR répare.
    assert_eq!(
        outcome,
        Some(DeadlineVerdictOutcome::AlreadyReviewed),
        "le filet s'arme bien sur la coupure, et décide de ne rien poster \
         parce que la session a déjà revu cette PR"
    );
    assert_eq!(
        posts, 0,
        "aucun second verdict sur une PR déjà revue — capturé = {captured:?}"
    );
}

/// **AC3** — une coupure avec un build en vol laisse le verdict au callback.
///
/// Les traces `11699630` (#2920, 26/10 15:13) et `517380e0` (#2932, 27/10
/// 00:58) : le tour a été coupé au plafond, mais il avait dispatché un
/// `build_mika`, et la revue est revenue par le filet de ce callback
/// (mika#2515). Le filet du webhook doit donc **se taire** — un second
/// `hold[review]` réveillerait `verdict_handler` deux fois sur une décision qui
/// n'est pas la sienne (décision D1 du plan, reconduite du vol 1).
///
/// L'abstention est assertée sur sa **cause** : le site unique qui pose
/// `max_steps_exceeded` ne le pose pas quand un build est en vol
/// (`agent_loop/mod.rs:7020-7024`). La ligne INFO `qa_max_steps_verdict_withheld`
/// qui l'accompagne (`agent_loop/mod.rs:6960-6973`) n'est **pas** assertée ici :
/// le harnais eval ne capture pas le flux `tracing`, et l'y brancher pour une
/// ligne serait une infrastructure hors périmètre. C'est le témoin opérateur,
/// que la sonde AC4 relit en production ; le test tient la cause (décision D2).
#[tokio::test]
async fn mika2943_une_coupure_avec_un_build_en_vol_laisse_le_verdict_au_callback() {
    let ceiling = max_tool_steps_for_agent("mika-qa", MAX_TOOL_STEPS);
    let registry: Arc<DashMap<String, HashSet<String>>> = Arc::new(DashMap::new());

    let (mut tools, shell_calls, gh_calls) = qa_tools();
    let build_calls: CallLog = Arc::new(Mutex::new(Vec::new()));
    tools.register(Box::new(Recording {
        name: BUILD_MIKA_TOOL,
        calls: build_calls.clone(),
        on_success: None,
    }));

    let harness = EvalHarness::builder()
        .responses(build_then_burned_budget_corpus(ceiling))
        .tools(tools)
        .agent_id("mika-qa")
        .session_id(SESSION_ID)
        .pr_reviews_posted(registry.clone())
        .build()
        .await
        .expect("harness build");

    let trace = harness.run(READY_FOR_REVIEW_2920).await.expect("agent run");

    // -- Précondition 1 : le tour a bien été coupé à son plafond. -------------
    // `ceiling - 1` parce que le premier pas est le build. La sentinelle du
    // corpus n'a pas été consommée, sans quoi `MockLlmProvider` aurait paniqué.
    let served = shell_calls.lock().unwrap().len();
    assert_eq!(
        served,
        ceiling - 1,
        "précondition : après le build, le tour doit brûler le reste de son \
         plafond ({} pas), or {served} appels servis",
        ceiling - 1
    );

    // -- Précondition 2 : le build est bien parti, et la trace le dit. -------
    // C'est le terme exact que lit `build_callback_dispatched_in_turn` : le nom
    // de l'outil **et** le succès de l'appel. Mesuré dans `trace.tool_calls`,
    // pas supposé depuis le corpus — un outil qui aurait échoué laisserait le
    // prédicat faux et l'abstention viendrait d'ailleurs.
    assert_eq!(
        build_calls.lock().unwrap().len(),
        1,
        "précondition : le build est dispatché une fois, au premier pas"
    );
    let dispatched_builds: Vec<&_> = trace
        .tool_calls
        .iter()
        .filter(|c| c.tool_name == BUILD_MIKA_TOOL && c.success)
        .collect();
    assert_eq!(
        dispatched_builds.len(),
        1,
        "précondition : la trace porte exactement un `{BUILD_MIKA_TOOL}` réussi — \
         c'est ce que `build_callback_dispatched_in_turn` lit. outils vus = {:?}",
        trace
            .tool_calls
            .iter()
            .map(|c| (c.tool_name.as_str(), c.success))
            .collect::<Vec<_>>()
    );
    assert!(
        gh_calls.lock().unwrap().is_empty(),
        "précondition : aucune revue dans ce corpus — le silence mesuré plus \
         bas n'est pas celui de la déduplication"
    );

    // -- Précondition 3 : ce n'est pas une coupure par deadline. -------------
    // L'abstention qu'on mesure est celle du plafond de pas. Un tour mort sur
    // sa deadline armerait le filet de mika#2276, et ce test n'attesterait
    // plus l'abstention de mika#2943.
    assert!(
        trace.output.deadline_exceeded.is_none(),
        "précondition : le tour est coupé par ses pas, pas par sa deadline"
    );

    // -- L'abstention, sur sa cause. -----------------------------------------
    assert!(
        trace.output.max_steps_exceeded.is_none(),
        "le site unique qui pose le fait de coupure s'en abstient quand un \
         build QA est en vol — sinon le filet du webhook poserait un second \
         `hold[review]` sur une PR dont le callback de build portera déjà le \
         verdict (traces 11699630 et 517380e0)"
    );
    assert_eq!(
        conclusion_of(&trace.output),
        TurnConclusion::Concluded,
        "sans fait de coupure, le call-site lit un tour conclu — c'est par là \
         que le filet du webhook reste muet"
    );

    let NetDecision {
        outcome,
        posts,
        captured,
    } = what_the_pr_receives(&trace.output, READY_FOR_REVIEW_2920, &registry).await;

    assert!(
        outcome.is_none(),
        "le filet du webhook ne s'arme pas : le verdict appartient au callback \
         du build. motif = {outcome:?}"
    );
    assert_eq!(
        posts, 0,
        "aucun verdict du webhook sur une PR dont le build portera le sien — \
         capturé = {captured:?}"
    );
}
