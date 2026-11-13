//! mika#2506 — le chemin de **production** du geste déterministe d'itération.
//!
//! # Ce que les tests unitaires du module ne peuvent pas voir
//!
//! `server::iterate_dispatch::tests` asserte le décideur pur rang par rang
//! (V1/V2, chacun vu rouge par mutation). Il ne voit **rien** de ce qui suit :
//! une chaîne qui lirait les bons champs et n'appellerait jamais le décideur
//! serait verte là et inerte en production ; et le nombre qui voyage dans
//! l'`input` de dispatch — la seule contrainte de conception que le § 3 du plan
//! existe pour tenir — n'est composé que sur ce chemin.
//!
//! Donc : forge injectée (aucun réseau), vraie base en mémoire, **vrai**
//! `SkillRegistry` portant un handler exec long-running inoffensif, et
//! `dispatch_iteration_with_forge` traversé de bout en bout.
//!
//! # Rouge-avant (porte #2264)
//!
//! Sur le code d'avant, `server::iterate_dispatch` n'existe pas : le fichier ne
//! compile pas, ce qui est la forme la plus franche du rouge. Recettes
//! d'injection sur cette branche :
//!
//! - passer `req.issue` → le numéro de la PR dans `try_engine_dispatch_for`
//!   (`create_and_dispatch`) ⇒ `le_nombre_qui_voyage_est_le_numero_d_issue`
//!   rougit, et **elle seule** ;
//! - retirer le `metadata:` pré-estampé de la `NewTask` ⇒
//!   `la_tache_est_auto_descriptive_a_la_creation` rougit ;
//! - retirer l'appel `set_task_dispatcher_source` ⇒ la même rougit sur l'autre
//!   moitié ;
//! - rendre `EngineDispatchResult::Deferred` en `Dispatched` ⇒
//!   `un_creneau_occupe_refuse_au_lieu_de_differer` rougit ;
//! - remettre le prédicat d'avant mika#2796 — `input.to_string().contains("2504")`
//!   sur la chaîne **entière**, soit retirer le filtre d'exclusion de
//!   `champs_portant_le_nombre` ⇒
//!   `mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien`
//!   rougit, et **elle seule**. C'est le flake du run 39160328822, rendu
//!   déterministe : un UUID v4 qui se termine par `…7f2504` suffisait à faire
//!   rougir un `input` correct.

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::{Database, NewTask};
use mika_agent::server::iterate_dispatch::{
    IssueState, IssueView, IterateOutcome, IterateRefusal, IterateRequest, PrSnapshot,
    dispatch_iteration_with_forge,
};
use mika_agent::skills::SkillRegistry;

const AGENT_ID: &str = "mika-dev";
const SESSION_ID: &str = "iterate-test-session";
const ISSUE: u64 = 2503;
const PR_URL: &str = "https://github.com/senara-solutions/mika/pull/2504";

/// La base, **avec sa ligne d'agent et sa session** : `tasks.created_by_session`
/// et `audit_events.session_id` portent des FK, donc une fixture sans elles
/// refuse la création de la tâche et le test mesure une erreur FK au lieu du
/// terme qu'il visait. Motif `cb_test_db` (verdict_handler).
async fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    let async_db = AsyncDatabase::new_with_agent(db, AGENT_ID);
    async_db
        .with_db(|d| {
            d.execute_sql(
                "INSERT OR IGNORE INTO agents (id, name, home_dir) VALUES (?1, ?1, '')",
                &[&AGENT_ID as &dyn rusqlite::types::ToSql],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    async_db
        .create_session(SESSION_ID, AGENT_ID, "cli")
        .await
        .unwrap();
    async_db
}

fn request(context: &str) -> IterateRequest {
    IterateRequest {
        repo: "mika".to_string(),
        issue: ISSUE,
        iteration_context: context.to_string(),
    }
}

/// Un corps d'issue portant le callout de branche que
/// `auto_pull::extract_branch_name` lit — le seul lecteur de cette grammaire.
fn issue_body() -> String {
    "> - **Branch:** `fix/2503/some-fix`\n> - **Plan:** `docs/plans/x-plan.md`\n".to_string()
}

fn open_pr() -> PrSnapshot {
    PrSnapshot {
        number: 2504,
        url: PR_URL.to_string(),
        is_draft: false,
        mergeable: "MERGEABLE".to_string(),
    }
}

/// Un `SkillRegistry` réel portant `run_claude_pilot` comme handler exec
/// long-running, dont le script est inoffensif.
///
/// Réel plutôt que `from_test_entries` : c'est la résolution de l'outil ET celle
/// du chemin du handler (`skill_dir.join(command)`, dont l'existence est
/// vérifiée avant le spawn) que ce chemin traverse.
fn dev_pilot_registry(dir: &std::path::Path) -> SkillRegistry {
    let skill_dir = dir.join("dev-pilot");
    std::fs::create_dir_all(skill_dir.join("handlers")).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        "[skill]\nname = \"dev-pilot\"\ndescription = \"t\"\nversion = \"0.1.0\"\n\n\
         [triggers]\nkeywords = [\"code task\"]\n",
    )
    .unwrap();
    std::fs::write(skill_dir.join("system_prompt.md"), "# dev-pilot\n").unwrap();
    std::fs::write(
        skill_dir.join("tools.json"),
        r#"[{
            "name": "run_claude_pilot",
            "description": "d",
            "input_schema": {"type": "object", "properties": {}},
            "handler": {
              "type": "exec",
              "command": "handlers/run.sh",
              "long_running": true,
              "estimated_duration_secs": 7200
            }
        }]"#,
    )
    .unwrap();
    // Inoffensif : sort immédiatement, n'écrit nulle part, ne livre aucun
    // callback. Ce que le test lit est la row callback et son `input_context`,
    // que l'exécuteur écrit AVANT le spawn.
    let handler = skill_dir.join("handlers/run.sh");
    std::fs::write(&handler, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&handler, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    SkillRegistry::from_dir(dir)
}

/// Sème la **preuve** de grooming que la porte mika#1620 / mika#2484 exige d'un
/// dispatch `dev-pilot` : un parent `groom` sur l'issue, et un callback complété
/// portant `Outcome: PLAN_GROOMED`.
///
/// # Pourquoi le chemin nominal en a besoin — ce n'est pas un artefact de test
///
/// La porte refuse tout `dev-pilot` sur une issue portant des callouts de
/// grooming **sans preuve en base**, et la preuve est purgée à 30 jours. Elle est
/// partagée avec `block[ac]` / `block[ci]`, qui passent le même
/// `iteration_context` — donc l'élargir serait un changement de comportement de
/// la boucle autonome, refusé au § 11 du plan. Une itération sur un ticket dont
/// la preuve a expiré est donc refusée `engine_refused`, avec le `recovery` du
/// moteur ; c'est la limite nommée, et le test
/// `sans_preuve_de_grooming_le_refus_est_engine_refused` l'atteste.
///
/// La row est semée par l'API publique plutôt que par `db::tests::completed_groom_pair`,
/// qui est `pub(crate)` et hors d'atteinte d'un crate d'intégration.
async fn seed_groom_proof(db: &AsyncDatabase, issue_url: &str) {
    let parent = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: format!("ready-label: {issue_url}"),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION_ID.to_string()),
            created_trace_id: None,
            reference_url: Some(issue_url.to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("groom".to_string()),
        })
        .await
        .unwrap();
    let callback = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: Some(parent.clone()),
            depth: 0,
            label: "long_running:run_claude_pilot_groom".to_string(),
            trigger_type: "callback".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "resume_agent".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION_ID.to_string()),
            created_trace_id: None,
            reference_url: None,
            source: None,
            metadata: None,
            r#type: None,
            dispatch_class: Some("groom".to_string()),
        })
        .await
        .unwrap();
    // Complété par le chemin d'écriture de production, comme le fait
    // `completed_groom_pair`.
    assert!(
        db.update_task_completed(
            &callback,
            Some("claude-pilot completed (status: done).\nOutcome: PLAN_GROOMED\nSession: s"),
        )
        .await
        .unwrap(),
        "le callback de groom doit se compléter par le chemin de production"
    );
    // La preuve est posée ; le parent `groom` est terminé pour qu'il ne tienne
    // pas le créneau de l'issue face à l'itération qui suit.
    db.update_task_completed(&parent, None).await.unwrap();
}

/// La forme exacte d'un blocage de créneau : une autre tâche de la classe
/// `implement` portant un enfant callback actif.
async fn occupy_implement_slot(db: &AsyncDatabase) {
    let parent = db
        .create_task(NewTask {
            agent_id: AGENT_ID.to_string(),
            team_run_id: None,
            parent_task_id: None,
            depth: 0,
            label: "other-dispatch".to_string(),
            trigger_type: "manual".to_string(),
            cron_expr: None,
            event_source: None,
            event_offset_secs: None,
            condition_expr: None,
            next_fire_at: None,
            timeout_at: None,
            action_type: "none".to_string(),
            action_config: "{}".to_string(),
            input_context: None,
            created_by_session: Some(SESSION_ID.to_string()),
            created_trace_id: None,
            // Une AUTRE issue : sinon la collision serait sur l'index de dédup,
            // pas sur le créneau, et le test mesurerait le mauvais terme.
            reference_url: Some("https://github.com/senara-solutions/mika/issues/9999".to_string()),
            source: Some("self_dev".to_string()),
            metadata: None,
            r#type: Some("issue".to_string()),
            dispatch_class: Some("implement".to_string()),
        })
        .await
        .unwrap();
    db.update_task_status(&parent, "in_progress").await.unwrap();
    db.create_task(NewTask {
        agent_id: AGENT_ID.to_string(),
        team_run_id: None,
        parent_task_id: Some(parent),
        depth: 0,
        label: "long_running:run_claude_pilot".to_string(),
        trigger_type: "callback".to_string(),
        cron_expr: None,
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: "resume_agent".to_string(),
        action_config: "{}".to_string(),
        input_context: None,
        created_by_session: Some(SESSION_ID.to_string()),
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: Some("implement".to_string()),
    })
    .await
    .unwrap();
}

const ISSUE_URL: &str = "https://github.com/senara-solutions/mika/issues/2503";

/// Le chemin nominal, **sans token GitHub**, et ce n'est pas une facilité.
///
/// `validate_dispatch_readiness` fait un vrai appel réseau pour sa porte de
/// grooming (`fetch_issue_body`) et pour son garde `blockedBy`. Sans token les
/// deux **échouent-ouvert**, ce qui est le comportement de production documenté
/// (« Fail-open when no `github_token` configured ») — et c'est la seule façon
/// d'exercer hors ligne la moitié qui suit la porte. C'est la même contrainte
/// que `test_dispatch_no_grooming_marker_guard.rs` a dû nommer : `fetch_issue_body`
/// n'est pas injectable.
///
/// Ce que ça ne dispense PAS de faire : avec un token, la porte s'applique, et
/// c'est `sans_preuve_de_grooming_le_refus_est_engine_refused` qui le mesure.
/// Les deux moitiés sont couvertes, chacune là où elle est atteignable.
async fn run_nominal(db: &AsyncDatabase, skills: &SkillRegistry, context: &str) -> IterateOutcome {
    let req = request(context);
    dispatch_iteration_with_forge(
        db,
        skills,
        None,
        SESSION_ID,
        "test-trace",
        &req,
        |_repo, _n| async {
            Ok(Some(IssueView {
                state: IssueState::Open,
                body: issue_body(),
            }))
        },
        |_repo, _branch| async { Ok(vec![open_pr()]) },
    )
    .await
}

/// Les clés de l'`input` de dispatch dont la valeur est **frappée par le moteur**
/// et ne dérive en rien de la requête de l'opérateur.
///
/// mika#2796 : exclues du contrôle négatif, parce qu'un UUID v4 porte ~20
/// chiffres décimaux répartis en segments, et qu'une sous-chaîne décimale de 4
/// chiffres donnée y tombe environ **une fois sur 4 400** (15 fenêtres
/// utiles × 16⁻⁴ : les segments 3 et 4 commencent par `4` et `[89ab]`). Le CI requis de la PR
/// mika#2787 (run 39160328822, 2026-10-18) a rougi sur
/// `task_id = "fcc4cf43-9b62-4b0d-8281-06046a7f2504"` alors que l'`input` était
/// correct — le prédicat d'alors lisait la chaîne entière.
///
/// L'exclusion ne dissimule aucune fuite : la valeur de `task_id` est vérifiée
/// **exactement** par `le_nombre_qui_voyage_est_le_numero_d_issue`
/// (`assert_eq!` contre le `task_id` rendu par l'outcome), donc le seul champ
/// retiré du balayage est aussi le seul dont la valeur est pinnée au byte.
const CLES_NON_DERIVEES_DE_LA_REQUETE: [&str; 1] = ["task_id"];

/// Rend, sous la forme `clé=valeur`, les champs de l'`input` de dispatch dont la
/// valeur porte `nombre` — hors [`CLES_NON_DERIVEES_DE_LA_REQUETE`]. Vide =
/// aucune fuite.
///
/// # Trois propriétés, chacune porteuse
///
/// - **Le balayage porte sur les sous-arbres, pas sur les scalaires.** C'est le
///   `to_string()` du sous-arbre qui est lu, donc un champ qui deviendrait un
///   objet reste couvert sans qu'on y pense : c'est la lecture du « tout champ
///   libre » de mika#2796.
/// - **Le défaut est d'INCLURE.** La liste est une **exclusion**, pas une liste
///   de champs à regarder : un cinquième champ ajouté au `json!` de production
///   (`server/verdict_handler.rs`) entre dans le contrôle le jour où il est
///   écrit. Une liste d'inclusion (`prompt` + `iteration_context`) aurait la
///   forme que mika#2309 condamne — un contrôle qui cesse de couvrir ce qu'il
///   devait couvrir, sans rougir.
/// - **L'exclusion ne peut pas devenir muette en silence.** Chaque clé exclue
///   doit être présente, sinon panique : un renommage en production ferait
///   sinon re-balayer l'UUID, et le flake reviendrait sans que rien ne le dise.
fn champs_portant_le_nombre(input: &serde_json::Value, nombre: &str) -> Vec<String> {
    let obj = input
        .as_object()
        .expect("l'`input` de dispatch est un objet JSON");
    for cle in CLES_NON_DERIVEES_DE_LA_REQUETE {
        assert!(
            obj.contains_key(cle),
            "la clé exclue `{cle}` a disparu de l'`input` de dispatch : l'exclusion \
             ne désigne plus rien, le balayage reviendrait sur un identifiant \
             aléatoire et le flake de mika#2796 avec lui. Remède : retirer la clé \
             de CLES_NON_DERIVEES_DE_LA_REQUETE si le champ a disparu, ou l'y \
             renommer s'il a été renommé. Input vu : {input}"
        );
    }
    obj.iter()
        .filter(|(cle, _)| !CLES_NON_DERIVEES_DE_LA_REQUETE.contains(&cle.as_str()))
        .filter(|(_, valeur)| valeur.to_string().contains(nombre))
        .map(|(cle, valeur)| format!("{cle}={valeur}"))
        .collect()
}

// ───────────────────────────────────────────────────────────────────────────
// V5 — le nombre qui voyage est le numéro d'ISSUE (R2, AC2)
// ───────────────────────────────────────────────────────────────────────────

/// **Le terme que le § 3 du plan existe pour tenir.**
///
/// `dispatch-lib.sh` consomme `prompt: "<repo>#<N>"` comme un numéro d'**issue**
/// (`gh issue view "$ISSUE_NUM"`, puis `derive-branch-name --issue "$ISSUE_NUM"`).
/// La PR est résolue **uniquement** pour vérifier les préconditions.
///
/// Contrôle négatif porteur : le numéro de PR (`2504`) ne doit apparaître dans
/// aucun champ **dérivé de la requête** de l'`input` de dispatch. Sans lui,
/// l'assertion positive serait satisfaite par un `prompt` contenant les deux.
///
/// Le `task_id` est exclu du balayage (mika#2796) : son UUID est frappé par le
/// moteur, ne dérive en rien de la requête, et sa valeur est vérifiée
/// **exactement** plus bas. Une doc qui promettrait « nulle part » est ce qui
/// fait qu'on « répare » l'assertion dans le mauvais sens à la lecture suivante.
#[tokio::test]
async fn le_nombre_qui_voyage_est_le_numero_d_issue() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;

    seed_groom_proof(&db, ISSUE_URL).await;

    let outcome = run_nominal(&db, &skills, "rebase puis corrige le lint SIGPIPE").await;
    let IterateOutcome::Dispatched {
        task_id,
        callback_task_id,
        pr_url,
    } = outcome
    else {
        panic!("attendu Dispatched, obtenu {outcome:?}");
    };
    assert_eq!(pr_url, PR_URL, "la PR vérifiée est rendue à l'opérateur");

    // L'`input` de dispatch est ce que `build_callback_task` a sérialisé dans
    // `input_context` — écrit AVANT le spawn, donc lisible sans attendre le
    // handler.
    let callback = db.get_task(&callback_task_id).await.unwrap().unwrap();
    let input: serde_json::Value =
        serde_json::from_str(&callback.input_context.expect("input_context écrit")).unwrap();

    assert_eq!(
        input["prompt"].as_str(),
        Some("mika#2503"),
        "le `prompt` porte le numéro d'ISSUE, et la forme NUE du dépôt \
         (mika#1593 : la forme owner-qualifiée route silencieusement le dispatch \
          en mode free-text sans worktree)"
    );
    assert_eq!(input["skill"].as_str(), Some("dev-pilot"));
    assert_eq!(input["task_id"].as_str(), Some(task_id.as_str()));
    // L'égalité ci-dessus compare deux sorties du même `create_task` : elle ne
    // prouve pas que la valeur est frappée par le moteur. C'est la prémisse de
    // l'exclusion de `CLES_NON_DERIVEES_DE_LA_REQUETE` (mika#2796), donc elle
    // est vérifiée ici plutôt que supposée.
    assert!(
        uuid::Uuid::parse_str(&task_id).is_ok(),
        "le `task_id` exclu du contrôle négatif doit être un UUID frappé par le \
         moteur, pas une valeur dérivée de la requête : {task_id}"
    );
    assert_eq!(
        input["iteration_context"].as_str(),
        Some("rebase puis corrige le lint SIGPIPE"),
        "le contexte fourni voyage verbatim — la voie sans contexte CRASHE la \
         session (self-dev Rule 4)"
    );

    // Contrôle négatif : aucun champ DÉRIVÉ DE LA REQUÊTE ne porte le numéro de
    // PR. Le `task_id` est exclu (mika#2796) — son UUID n'a aucun rapport avec
    // la requête, et sa valeur est vérifiée exactement deux assertions plus
    // haut.
    let porteurs = champs_portant_le_nombre(&input, "2504");
    assert!(
        porteurs.is_empty(),
        "le numéro de PR ne doit apparaître dans aucun champ dérivé de la requête \
         — `dispatch-lib` le lirait comme un numéro d'issue : {porteurs:?} \
         (input complet : {input})"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// mika#2796 — le prédicat du contrôle négatif, mesuré
// ───────────────────────────────────────────────────────────────────────────
//
// `champs_portant_le_nombre` est un **détecteur** : son chemin de succès est
// « aucune violation trouvée ». Un détecteur dont nul ne mesure ni le silence ni
// la capacité à rougir est une garde qui ne refuse rien. Les trois tests qui
// suivent sont synchrones — ni base ni tokio : le sujet est une
// `serde_json::Value`.

/// **Le rouge-avant de mika#2796, figé.**
///
/// L'`input` exact du run 39160328822 (CI requis de la PR mika#2787,
/// 2026-10-18), UUID `fcc4cf43-9b62-4b0d-8281-06046a7f2504` verbatim. Les quatre
/// champs sont ceux que `server/verdict_handler.rs` compose, et l'`input` est
/// **correct** : `prompt` porte le numéro d'issue, `iteration_context` le texte
/// de l'opérateur. Seul le `task_id` se termine par `…7f2504`.
///
/// Sous le prédicat d'avant — `!input.to_string().contains("2504")` — ce test
/// échoue. C'est le flake, rendu déterministe et nommé.
#[test]
fn mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien() {
    let input = serde_json::json!({
        "skill": "dev-pilot",
        "prompt": "mika#2503",
        "task_id": "fcc4cf43-9b62-4b0d-8281-06046a7f2504",
        "iteration_context": "rebase puis corrige le lint SIGPIPE",
    });

    assert_eq!(
        champs_portant_le_nombre(&input, "2504"),
        Vec::<String>::new(),
        "un UUID qui porte par hasard les chiffres du numéro de PR n'est pas une \
         fuite : la valeur du `task_id` est frappée par `create_task` et ne dérive \
         en rien de la requête (mika#2796)"
    );
}

/// **Le contrôle reste capable de rougir** — et il nomme le champ coupable.
///
/// Deux cas, tous deux sur la forme de production : le numéro de PR semé dans le
/// champ libre `iteration_context`, puis dans `prompt` (la fuite réelle que
/// mika#2506 existe pour refuser — `dispatch-lib` lirait `mika#2504` comme un
/// numéro d'issue).
///
/// L'égalité porte sur la liste **entière**, pas sur sa non-vacuité : un prédicat
/// qui rendrait tous les champs rougirait aussi « correctement » sans rien
/// discriminer, et le message d'échec ne dirait plus lequel transporte le nombre.
///
/// Les deux `task_id` sont **neutres** — aucun ne porte `2504` — et c'est
/// délibéré : le sujet de ce test est la capacité à rougir, pas l'exclusion.
/// Un UUID pathologique ici lui ferait mesurer les deux, et le rouge-avant V2 du
/// plan cesserait de discriminer (il rougirait ce test *et* celui du flake).
/// La charge de l'exclusion appartient à
/// `mika2796_un_task_id_portant_les_chiffres_du_numero_de_pr_ne_declenche_rien`,
/// seul.
#[test]
fn mika2796_le_controle_rougit_quand_le_nombre_voyage_dans_un_champ_libre() {
    let dans_le_champ_libre = serde_json::json!({
        "skill": "dev-pilot",
        "prompt": "mika#2503",
        "task_id": "7b1e0a6d-4c88-4f1a-9ea7-30d55bc1ef80",
        "iteration_context": "reprends la PR 2504 et corrige le lint",
    });
    assert_eq!(
        champs_portant_le_nombre(&dans_le_champ_libre, "2504"),
        vec!["iteration_context=\"reprends la PR 2504 et corrige le lint\"".to_string()],
        "le champ libre est balayé, et le message nomme le champ — pas seulement \
         le blob"
    );

    let dans_le_prompt = serde_json::json!({
        "skill": "dev-pilot",
        "prompt": "mika#2504",
        "task_id": "fcc4cf43-9b62-4b0d-8281-06046a7f2503",
        "iteration_context": "rebase puis corrige le lint SIGPIPE",
    });
    assert_eq!(
        champs_portant_le_nombre(&dans_le_prompt, "2504"),
        vec!["prompt=\"mika#2504\"".to_string()],
        "la fuite que mika#2506 existe pour refuser doit être trouvée, et nommée \
         `prompt=`"
    );
}

/// **L'assertion auto-nettoyante.**
///
/// Si `task_id` est renommée ou retirée du `json!` de production, l'exclusion ne
/// désigne plus rien : le balayage reviendrait sur l'UUID et le flake de
/// mika#2796 avec lui, sans que rien ne le dise. Ce test fait rougir le
/// renommage à la place — c'est ce qui empêche l'exception de devenir muette.
#[test]
#[should_panic(expected = "la clé exclue `task_id` a disparu")]
fn mika2796_lexclusion_du_task_id_rougit_si_la_cle_disparait() {
    let input = serde_json::json!({
        "skill": "dev-pilot",
        "prompt": "mika#2503",
        // Renommée : exactement ce que ferait un refactor de production.
        "dispatch_task_uuid": "fcc4cf43-9b62-4b0d-8281-06046a7f2504",
        "iteration_context": "rebase puis corrige le lint SIGPIPE",
    });

    let _ = champs_portant_le_nombre(&input, "2504");
}

// ───────────────────────────────────────────────────────────────────────────
// V6 — la tâche est auto-descriptive à la création (R5, AC6)
// ───────────────────────────────────────────────────────────────────────────

/// `dispatcher_source = 'operator'` et `metadata.claude_pilot.pr_url` présents
/// **avant tout callback**.
///
/// Le pré-estampage rend la tâche lisible dès sa création — c'est très exactement
/// la corrélation que l'opérateur a dû faire à la main le 2026-09-23 — et il arme
/// déterministe le chemin du parent-completer (mika#1162, prédicat
/// `pr_url IS NOT NULL`) plutôt que de faire dépendre la résolution de la tâche
/// de la découverte de la PR en aval.
#[tokio::test]
async fn la_tache_est_auto_descriptive_a_la_creation() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;

    seed_groom_proof(&db, ISSUE_URL).await;

    let outcome = run_nominal(&db, &skills, "corrige la CI").await;
    let IterateOutcome::Dispatched { task_id, .. } = outcome else {
        panic!("attendu Dispatched, obtenu {outcome:?}");
    };

    let parent = db.get_task(&task_id).await.unwrap().unwrap();

    assert_eq!(
        parent.dispatcher_source.as_deref(),
        Some("operator"),
        "la priorité opérateur de `promote_pending_deferred_if_idle` en dépend : \
         un opérateur qui demande une itération ne doit pas être affamé derrière \
         les wrappers de la boucle"
    );

    let metadata: serde_json::Value =
        serde_json::from_str(&parent.metadata.expect("metadata pré-estampée")).unwrap();
    assert_eq!(
        metadata["claude_pilot"]["pr_url"].as_str(),
        Some(PR_URL),
        "la PR itérée est lisible par `mika tasks get` sans attendre la ligne \
         `PR:` du callback"
    );

    // La tâche entre dans l'index de dédup ACTIF par son `reference_url` : c'est
    // ce qui fait qu'un second `mika iterate` collisionne au lieu d'ouvrir un
    // second pilote.
    assert_eq!(
        parent.reference_url.as_deref(),
        Some("https://github.com/senara-solutions/mika/issues/2503")
    );
    assert_eq!(parent.dispatch_class.as_deref(), Some("implement"));
    // `mark_parent_dispatched` (mika#2335) a tourné : le parent est la row que
    // l'opérateur lit, et elle doit dire qu'un dispatch est parti.
    assert_eq!(parent.status, "in_progress");
    assert!(
        parent.fired_at.is_some(),
        "mika#2335 — un parent dispatché porte `fired_at`, sans quoi un opérateur \
         lit « jamais démarrée » sur un pilote qui travaille"
    );
}

/// Contrôle négatif du dédup : un second `mika iterate` pendant que le premier
/// tourne est refusé, il n'ouvre pas un second pilote sur la même branche.
#[tokio::test]
async fn un_second_iterate_sur_la_meme_issue_collisionne() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;

    seed_groom_proof(&db, ISSUE_URL).await;

    let first = run_nominal(&db, &skills, "corrige la CI").await;
    assert!(matches!(first, IterateOutcome::Dispatched { .. }));

    let second = run_nominal(&db, &skills, "corrige la CI encore").await;
    match second {
        IterateOutcome::Refused { reason, detail } => {
            assert_eq!(reason, IterateRefusal::SlotBusy);
            assert!(
                detail.contains("créneau"),
                "le refus doit nommer le blocage : {detail}"
            );
        }
        other => panic!("attendu un refus, obtenu {other:?}"),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// U6 — `slot_busy` refuse, il ne diffère pas (R4, AC5)
// ───────────────────────────────────────────────────────────────────────────

/// **Divergence délibérée avec `try_engine_dispatch_for`**, qui rend `Deferred`
/// et enregistre un wrapper.
///
/// Un geste différé part quelques minutes plus tard, sans personne qui regarde,
/// alors que l'opérateur a tapé une commande en attendant une réponse. Et
/// l'échappatoire existe déjà et se nomme : le refus la cite.
#[tokio::test]
async fn un_creneau_occupe_refuse_au_lieu_de_differer() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;

    occupy_implement_slot(&db).await;

    let outcome = run_nominal(&db, &skills, "corrige la CI").await;
    match outcome {
        IterateOutcome::Refused { reason, detail } => {
            assert_eq!(reason, IterateRefusal::SlotBusy);
            assert!(
                detail.contains("promote-deferred"),
                "un refus qui ne nomme pas sa levée est un refus qu'on contourne \
                 au jugé : {detail}"
            );
        }
        other => panic!("un créneau occupé doit REFUSER, pas différer : {other:?}"),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Les refus, vus sur le chemin de production
// ───────────────────────────────────────────────────────────────────────────

/// Rang 1 sur le chemin réel : **aucune** sonde de forge n'est appelée.
///
/// C'est la propriété que l'ordre achète, et elle n'est pas visible du décideur
/// pur : là, la forge est déjà lue.
#[tokio::test]
async fn un_contexte_vide_refuse_sans_toucher_la_forge() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;
    let req = request("   ");

    let outcome = dispatch_iteration_with_forge(
        &db,
        &skills,
        Some("fake-token"),
        SESSION_ID,
        "t",
        &req,
        |_r, _n| async { panic!("rang 1 doit refuser AVANT toute lecture de l'issue") },
        |_r, _b| async { panic!("rang 1 doit refuser AVANT toute lecture des PR") },
    )
    .await;

    match outcome {
        IterateOutcome::Refused { reason, .. } => {
            assert_eq!(reason, IterateRefusal::MissingContext)
        }
        other => panic!("attendu missing_context, obtenu {other:?}"),
    }
    // Et rien n'a été créé.
    assert!(db.list_active_tasks().await.unwrap().is_empty());
}

/// Une issue sans callout `> - **Branch:**` n'a pas de branche à interroger :
/// zéro PR trouvable, donc `no_open_pr` — et **la liste des PR n'est jamais
/// demandée**, ce qui est la limite nommée du § 10 et la Halte 1.
#[tokio::test]
async fn sans_callout_de_branche_le_refus_est_no_open_pr() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;
    let req = request("corrige la CI");

    let outcome = dispatch_iteration_with_forge(
        &db,
        &skills,
        Some("fake-token"),
        SESSION_ID,
        "t",
        &req,
        |_r, _n| async {
            Ok(Some(IssueView {
                state: IssueState::Open,
                body: "## Description\n\nrien ici\n".to_string(),
            }))
        },
        |_r, _b| async { panic!("sans branche dérivée, aucune PR ne doit être demandée") },
    )
    .await;

    match outcome {
        IterateOutcome::Refused { reason, detail } => {
            assert_eq!(reason, IterateRefusal::NoOpenPr);
            assert!(
                detail.contains("Branch:"),
                "le détail doit nommer le callout manquant — c'est le remède, et \
                 c'est ce qui évite d'élargir la recherche de PR par réflexe \
                 (Halte 1) : {detail}"
            );
        }
        other => panic!("attendu no_open_pr, obtenu {other:?}"),
    }
    assert!(db.list_active_tasks().await.unwrap().is_empty());
}

/// Une forge muette sur les PR n'est **jamais** lue comme « zéro PR » — ce
/// serait une affirmation fausse, et elle enverrait l'opérateur chercher une PR
/// qui existe.
#[tokio::test]
async fn une_forge_muette_sur_les_pr_nest_pas_zero_pr() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;
    let req = request("corrige la CI");

    let outcome = dispatch_iteration_with_forge(
        &db,
        &skills,
        Some("fake-token"),
        SESSION_ID,
        "t",
        &req,
        |_r, _n| async {
            Ok(Some(IssueView {
                state: IssueState::Open,
                body: issue_body(),
            }))
        },
        |_r, _b| async { Err("HTTP 502".to_string()) },
    )
    .await;

    match outcome {
        IterateOutcome::Refused { reason, detail } => {
            assert_eq!(reason, IterateRefusal::IssueUnresolvable);
            assert!(
                detail.contains("502"),
                "le détail porte la cause : {detail}"
            );
        }
        other => panic!("attendu issue_unresolvable, obtenu {other:?}"),
    }
}

/// Une issue fermée est refusée avant qu'une PR ne soit cherchée (cas mika#988,
/// dont l'auto-skip en aval se saborderait).
#[tokio::test]
async fn une_issue_fermee_refuse_avant_de_chercher_une_pr() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;
    let req = request("corrige la CI");

    let outcome = dispatch_iteration_with_forge(
        &db,
        &skills,
        Some("fake-token"),
        SESSION_ID,
        "t",
        &req,
        |_r, _n| async {
            Ok(Some(IssueView {
                state: IssueState::Closed,
                body: issue_body(),
            }))
        },
        |_r, _b| async { panic!("une issue fermée ne doit pas déclencher de lecture de PR") },
    )
    .await;

    match outcome {
        IterateOutcome::Refused { reason, .. } => {
            assert_eq!(reason, IterateRefusal::IssueUnresolvable)
        }
        other => panic!("attendu issue_unresolvable, obtenu {other:?}"),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// La limite nommée : la porte de grooming n'est PAS élargie
// ───────────────────────────────────────────────────────────────────────────

/// Le même chemin **avec** un token : la porte de grooming et le garde
/// `blockedBy` s'activent alors et sollicitent le réseau.
async fn run_with_token(
    db: &AsyncDatabase,
    skills: &SkillRegistry,
    context: &str,
) -> IterateOutcome {
    let req = request(context);
    dispatch_iteration_with_forge(
        db,
        skills,
        Some("fake-token"),
        SESSION_ID,
        "test-trace",
        &req,
        |_repo, _n| async {
            Ok(Some(IssueView {
                state: IssueState::Open,
                body: issue_body(),
            }))
        },
        |_repo, _branch| async { Ok(vec![open_pr()]) },
    )
    .await
}

/// Un refus de la porte de readiness du moteur se présente en `engine_refused`,
/// **jamais** en `slot_busy`.
///
/// # Pourquoi c'est porteur, et pas un détail de vocabulaire
///
/// `slot_busy` enverrait l'opérateur vers `mika tasks promote-deferred`, qui ne
/// fait **rien** pour un refus de la porte de grooming : un refus qui nomme le
/// mauvais remède est la classe de défaut que ce ticket même ferme (mika#1971).
/// Le motif du moteur voyage verbatim, parce qu'il porte son propre `recovery`.
///
/// # Ce que ce test atteste hors ligne, et ce qu'il n'atteste PAS
///
/// La porte contacte l'API GitHub (`fetch_issue_body`), qui n'est pas injectable
/// et n'est pas joignable ici. Ce qui est **mesuré** est donc : *un refus de la
/// porte, quel qu'il soit, ressort en `engine_refused` avec le motif du moteur*.
/// La variante précise `dispatch_grooming_not_verified` — la plus probable en
/// production sur un ticket dont la preuve a plus de 30 jours — n'est pas
/// atteignable sans réseau ; c'est la même limite que
/// `test_dispatch_no_grooming_marker_guard.rs` a dû nommer, et elle est la
/// sonde S1 du plan.
///
/// C'est aussi là que la limite du § 11 est mesurée plutôt qu'écrite : la porte
/// est partagée avec `block[ac]` / `block[ci]`, donc lui faire lire
/// `iteration_context` serait un changement de comportement de la boucle
/// autonome. **Suivi nommé.**
#[tokio::test]
async fn un_refus_de_la_porte_moteur_est_engine_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;

    let outcome = run_with_token(&db, &skills, "corrige la CI").await;
    match outcome {
        IterateOutcome::Refused { reason, detail } => {
            assert_eq!(
                reason,
                IterateRefusal::EngineRefused,
                "un refus de la porte n'est PAS un créneau occupé : `slot_busy` \
                 enverrait l'opérateur vers `promote-deferred`, qui ne répare rien"
            );
            assert!(
                detail.contains("dispatch readiness check failed"),
                "le motif du moteur doit voyager verbatim — il porte son propre \
                 `recovery` : {detail}"
            );
        }
        other => panic!("attendu engine_refused, obtenu {other:?}"),
    }
}

/// Contrôle négatif de l'ordre côté moteur : un créneau occupé est refusé
/// **avant** la porte de readiness (check 3 précède check 5), donc le motif est
/// `slot_busy` même avec un token, là où le test précédent obtient
/// `engine_refused`.
///
/// Sans ce test, « le créneau refuse » serait indistinguable de « la porte
/// refuse tout », et les deux motifs seraient interchangeables.
#[tokio::test]
async fn un_creneau_occupe_precede_la_porte_moteur() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;
    occupy_implement_slot(&db).await;

    let outcome = run_with_token(&db, &skills, "corrige la CI").await;
    match outcome {
        IterateOutcome::Refused { reason, .. } => assert_eq!(
            reason,
            IterateRefusal::SlotBusy,
            "le créneau est testé avant la porte : le motif doit rester `slot_busy`"
        ),
        other => panic!("attendu slot_busy, obtenu {other:?}"),
    }
}

// ───────────────────────────────────────────────────────────────────────────
// R9 — la surface opérateur
// ───────────────────────────────────────────────────────────────────────────

/// Chaque invocation laisse une ligne d'audit, succès **et** refus, sous le même
/// `tool_name` avec l'issue dans `after_value`.
///
/// Une seule requête `GROUP BY after_value` donne donc les deux comptes,
/// soustractibles — motif `ready_label_outcome` (mika#2323).
#[tokio::test]
async fn chaque_invocation_laisse_une_ligne_daudit() {
    let tmp = tempfile::tempdir().unwrap();
    let skills = dev_pilot_registry(tmp.path());
    let db = test_db().await;
    seed_groom_proof(&db, ISSUE_URL).await;

    // Un refus…
    let req = request("");
    let _ = dispatch_iteration_with_forge(
        &db,
        &skills,
        Some("fake-token"),
        SESSION_ID,
        "t",
        &req,
        |_r, _n| async { unreachable!() },
        |_r, _b| async { unreachable!() },
    )
    .await;
    // …puis un succès.
    let _ = run_nominal(&db, &skills, "corrige la CI").await;

    let events = db
        .list_audit_events_paginated_with_count(
            AGENT_ID,
            Some("operator_iterate_dispatch"),
            None,
            50,
            0,
        )
        .await
        .unwrap()
        .0;

    let outcomes: Vec<Option<String>> = events.iter().map(|e| e.after_value.clone()).collect();
    assert!(
        outcomes.contains(&Some("missing_context".to_string())),
        "le refus doit être audité : {outcomes:?}"
    );
    assert!(
        outcomes.contains(&Some("dispatched".to_string())),
        "le succès doit être audité sous le MÊME nom, sinon les deux populations \
         ne sont pas soustractibles : {outcomes:?}"
    );
    for e in &events {
        assert_eq!(
            e.target_key.as_str(),
            "issue:senara-solutions/mika#2503",
            "la clé porte l'issue owner-qualifiée, pour qu'une requête par ticket \
             soit exacte"
        );
    }
}
