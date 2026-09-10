//! Le contrôle positif qui manquait au reaper D1 : un pilote **réellement
//! vivant**, sur la **vraie forme de row**, est trouvé et tué (mika#2272).
//!
//! # Pourquoi ce fichier existe alors que `test_pilot_silent_stall_reaper.rs`
//! existe déjà
//!
//! Parce que celui-là passait au vert sur un mécanisme inerte en production.
//! Sa fixture posait `update_task_status(id, "in_progress")` sur la row du
//! dispatch — une transition que la production n'écrit **jamais** sur cette
//! row. Le test fabriquait donc la population que le code savait lire, et les
//! deux se confirmaient l'un l'autre pendant que deux pilotes zombies vivaient
//! 50 minutes sans produire un seul audit.
//!
//! La leçon n'est pas « il manquait un cas ». C'est que le synthétique avait
//! remplacé la mesure. Ce fichier n'écrit donc aucun status à la main : il
//! passe par [`build_callback_task`] + `create_task`, le **chemin d'écriture de
//! production**, et commence par asserter ce que ce chemin produit vraiment.
//!
//! # Les trois choses réelles
//!
//! 1. **Le processus** — [`spawn_live_child`] rend un `sleep 600` authentique,
//!    chef de son groupe comme les dispatches de `executor.rs`. « Tué » se lit
//!    ensuite sur `/proc`, pas sur une valeur de retour.
//! 2. **La row** — `pending`, parce que c'est là que vit un dispatch en cours.
//! 3. **La base** — [`MultiAgentHarness`] (mika#2265) : un fichier partagé, une
//!    connexion par agent, comme un container. C'est ce qui rend exprimable
//!    l'assertion d'attribution : le moteur de `mika-qa` voit la row de
//!    `mika-dev` dans la base, et ne doit pas y toucher.
//!
//! # Ce que « tué » veut dire ici, mesuré
//!
//! Le rouge-avant a été fait, pas supposé : l'appel à
//! `reap_silently_stalled_pilots` commenté dans `TaskEngine::tick`, l'assertion
//! de mort passe au rouge et le `sleep 600` survit au scan. Le reaper est donc
//! causalement responsable de la mort du processus.
//!
//! En instrumentant le chemin, une chose s'est vue qui n'était pas cherchée :
//! sur cette machine, `send_signal(pid, "TERM", true)` — `/bin/kill -TERM
//! -<pgid>` — rend **false** alors que le signal est bel et bien délivré (le
//! processus est mort au retour). `kill_process_gracefully` prend alors la
//! branche `!term_sent` et rend `!is_process_alive(pid)`, c'est-à-dire la
//! bonne réponse pour la mauvaise raison : ni la période de grâce ni
//! l'escalade SIGKILL ne sont atteintes. Un pilote qui **ignorerait** SIGTERM
//! ne serait donc pas escaladé. C'est antérieur à mika#2272 et partagé avec
//! `cancel_task` et le nettoyage d'orphelins — donc fiché à part, pas corrigé
//! ici. Le commentaire de `spawn_live_child` affirme l'inverse (« exits 0 on
//! this platform ») ; c'est cette mesure qui fait foi.
//!
//! # Fire-Disposition
//!
//! **(c) halte-et-remontée, gate CI bloquant** pour les trois tests. Un rouge
//! ici signifie soit que le reaper est redevenu inerte (la classe zombie
//! rouvre), soit qu'il fauche hors de sa population (il détruit du travail
//! vivant). Aucun des deux ne se remédie tout seul.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::executor::build_callback_task;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::task_engine::process_liveness::is_same_process_alive;
use mika_agent::tools::default_tools;

use super::multi_agent::MultiAgentHarness;
use super::process_fixtures::{kill_pid, spawn_live_child};

/// L'agent qui dispatche les pilotes. Le second agent monté n'est pas
/// décoratif : il porte le contrôle d'attribution.
const DISPATCHER_AGENT: &str = "mika-dev";
const OTHER_AGENT: &str = "mika-qa";

/// Bien au-delà des 2700 s par défaut, pour que la fixture n'encode pas le
/// seuil une seconde fois — `mika_common::config` le possède et l'épingle.
const STALE_SECS: u64 = 10_000;

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

/// Un dispatcher dont l'armement est posé par `Settings`, pas par
/// `MIKA_PILOT_STALL_REAP_*` : l'env est global au process et ce binaire lance
/// ses tests en parallèle.
///
/// `armed: None` laisse **le défaut de production** décider — c'est la seule
/// forme qui teste la cause n° 2 plutôt que de la contourner.
///
/// `pilot_log_dir` (mika#2277) points at a directory the caller owns: the
/// claude-pilot session log is **derived** as `<pilot_log_dir>/<task-id>.log`,
/// so it must survive the whole test rather than the `TempDir` created here.
fn dispatcher_for(
    db: &AsyncDatabase,
    armed: Option<bool>,
    pilot_log_dir: &Path,
) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let mut settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    settings.pilot_stall_reap_enabled = armed;
    settings.pilot_log_dir = Some(pilot_log_dir.to_string_lossy().into_owned());
    Arc::new(TaskDispatcher {
        db: db.clone(),
        tier: mika_common::home::AgentTier::Default,
        llm: mika_common::llm::dummy_provider(),
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from("/tmp"),
        embedding_client: None,
        brave_api_key: None,
        gateway_url: None,
        internal_token: None,
        github_token: None,
        github_app: None,
        skills_dirty: Arc::new(AtomicBool::new(false)),
        agent_lock: None,
        cli_mode: true,
        settings,
        pr_reviews_posted: None,
    })
}

fn engine_for(db: &AsyncDatabase, armed: Option<bool>, pilot_log_dir: &Path) -> TaskEngine {
    TaskEngine::new(db.clone(), dispatcher_for(db, armed, pilot_log_dir))
}

/// Un répertoire tenant lieu de `/var/log/claude-pilot`, sous le tmp du test
/// (mika#2277).
fn pilot_log_dir(root: &Path) -> PathBuf {
    let dir = root.join("var-log-claude-pilot");
    std::fs::create_dir_all(&dir).expect("mkdir pilot log dir");
    dir
}

/// Un cycle de scan complet (`DB_SCAN_INTERVAL_TICKS` = 60).
async fn drive_scan(engine: &mut TaskEngine) {
    for _ in 0..60 {
        engine.tick().await;
    }
}

fn set_mtime(path: &Path, secs_ago: u64) {
    let when = SystemTime::now() - Duration::from_secs(secs_ago);
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when))
        .expect("backdate mtime");
}

/// Un worktree dont la dernière écriture remonte à `idle_secs`.
fn seed_worktree(root: &Path, idle_secs: u64) -> PathBuf {
    let worktree = root.join("worktree");
    let src = worktree.join("crates").join("mika-agent").join("src");
    std::fs::create_dir_all(&src).expect("mkdir worktree");
    let file = src.join("engine.rs");
    std::fs::write(&file, b"fn main() {}").expect("write file");
    // Antidater toute la chaîne : un répertoire porte son propre mtime, en
    // laisser un à « maintenant » ferait passer l'arbre pour fraîchement écrit.
    for p in [
        file.as_path(),
        src.as_path(),
        &worktree.join("crates").join("mika-agent"),
        &worktree.join("crates"),
        worktree.as_path(),
    ] {
        set_mtime(p, idle_secs);
    }
    worktree
}

/// Le fichier que `dispatch-lib.sh` écrit pour déclarer son worktree.
fn declare_worktree(root: &Path, worktree: &Path) -> PathBuf {
    let file = root.join("declaration.path");
    std::fs::write(&file, format!("{}\n", worktree.display())).expect("write declaration");
    file
}

/// Sème un dispatch **par le chemin de production** et rend son id.
///
/// Aucun `update_task_status` : le status de la row est celui que
/// `create_task` écrit pour un `NewTask` construit par
/// [`build_callback_task`]. Si un jour ce n'est plus `pending`, c'est
/// [`la_row_porteuse_du_pid_est_pending`] qui doit le dire, pas ce helper qui
/// doit le corriger.
///
/// Les **trois** surfaces sont muettes (mika#2277) : depuis que le prédicat est
/// une conjonction, silencer le seul worktree ne décrit plus un pilote bloqué —
/// ça décrit la forme des faux positifs du 2026-09-10, que
/// `test_reaper_liveness_all_surfaces_2277.rs` épingle comme **non** fauchable.
#[allow(clippy::too_many_arguments)]
async fn seed_live_dispatch(
    db: &AsyncDatabase,
    agent_id: &str,
    session_id: &str,
    pid: i64,
    start_time: u64,
    declaration_file: &Path,
    root: &Path,
    log_dir: &Path,
) -> Result<String> {
    let input = serde_json::json!({
        "skill": "self-dev",
        "prompt": "implémente mika#2272",
        "task_id": "parent-inexistant",
        "branch": "fix/2272/reaper-scan-pending-row-arm",
    });
    let task = build_callback_task(
        agent_id.to_string(),
        None,
        "run_claude_pilot",
        &input,
        3600,
        session_id,
        "trace-2272",
    );
    let id = db.create_task(task).await?;

    // Les trois écritures que `spawn_long_running_exec` fait après le spawn,
    // dans le même ordre et sous la même forme — `process_start_time` est
    // stocké en chaîne JSON par la production, le copier autrement validerait
    // une forme qui n'existe pas.
    db.set_task_process_id(&id, Some(pid)).await?;
    db.set_task_metadata_field(&id, "process_start_time", &start_time.to_string())
        .await?;
    db.set_task_metadata_field(
        &id,
        "dispatch_worktree_file",
        &declaration_file.to_string_lossy(),
    )
    .await?;

    // Les deux surfaces ajoutées par mika#2277, muettes elles aussi : le
    // transcript déclaré (`inject_pilot_transcript_env` écrit le fichier ET
    // estampille la clé) et le log claude-pilot dérivé.
    let transcript = root.join(format!("{id}.jsonl"));
    std::fs::write(&transcript, b"{\"type\":\"llm_call\"}\n")?;
    set_mtime(&transcript, STALE_SECS);
    db.set_task_metadata_field(
        &id,
        "pilot_transcript_expected",
        &transcript.to_string_lossy(),
    )
    .await?;

    let log = log_dir.join(format!("{id}.log"));
    std::fs::write(&log, b"tool result\n")?;
    set_mtime(&log, STALE_SECS);

    Ok(id)
}

// ---------------------------------------------------------------------------
// La cause n° 1, épinglée à la source
// ---------------------------------------------------------------------------

/// **Rouge-avant de la cause n° 1.** La row qui porte le `process_id` d'un
/// dispatch vivant est `pending`, et ce n'est pas une opinion du test : elle
/// sort du constructeur de production.
///
/// Mesure de production qui a motivé mika#2272 (2026-09-09, sur toutes les rows
/// ayant jamais porté un `process_id`) : 876 `delivered`, 19 `cancelled`,
/// 1 `failed`, 1 `pending`, **zéro `in_progress`**. Un reaper filtrant
/// `in_progress` ne regardait pas une petite population, il en regardait une
/// vide.
///
/// Ce test échoue si quelqu'un remet un status synthétique dans la fixture, ou
/// si le cycle de vie change sans que le prédicat du reaper suive.
#[tokio::test]
async fn la_row_porteuse_du_pid_est_pending() -> Result<()> {
    let h = MultiAgentHarness::builder()
        .agent(DISPATCHER_AGENT)
        .build()?;
    let db = h.db(DISPATCHER_AGENT);

    let tmp = tempfile::tempdir()?;
    let log_dir = pilot_log_dir(tmp.path());
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let (pid, start_time) = spawn_live_child();
    let id = seed_live_dispatch(
        db,
        DISPATCHER_AGENT,
        h.session_id(DISPATCHER_AGENT),
        pid,
        start_time,
        &declaration,
        tmp.path(),
        &log_dir,
    )
    .await?;

    let task = db.get_task(&id).await?.expect("la row sémée");
    assert_eq!(
        task.status, "pending",
        "le chemin de production écrit `pending` sur la row du dispatch ; \
         c'est le parent que #525 fait passer `in_progress`, pas cet enfant"
    );
    assert_eq!(task.process_id, Some(pid), "et c'est elle qui porte le pid");

    // Et la conséquence directe : la requête historique ne la voit pas, la
    // nouvelle si. Les deux contrôles dans le même appel — sans le premier,
    // « la nouvelle requête trouve la row » ne dirait pas que l'ancienne avait
    // tort.
    let ancienne = db.get_active_callback_tasks_with_pid().await?;
    assert!(
        ancienne.iter().all(|t| t.id != id),
        "contrôle négatif : la requête `in_progress` de #959 ne rend pas cette row — \
         c'est exactement pourquoi mika#2261 n'a jamais tiré"
    );
    let nouvelle = db.get_live_dispatch_callback_tasks_with_pid().await?;
    assert!(
        nouvelle.iter().any(|t| t.id == id),
        "contrôle positif : la requête de mika#2272 la rend"
    );

    kill_pid(pid);
    h.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Le contrôle positif : trouvé ET tué, sur défaut de production
// ---------------------------------------------------------------------------

/// **Le test que mika#2272 devait produire.** Un pilote authentiquement vivant
/// et silencieux, sur sa vraie row, avec l'armement **par défaut** — trouvé,
/// tué, transitionné, audité.
///
/// Le contrôle d'attribution passe en premier, et il ne peut pas passer après :
/// une fois la row fauchée, `mika-qa` n'aurait plus rien à ne pas toucher. Il
/// mesure ce qu'un harness mono-agent ne peut pas exprimer — l'autre agent voit
/// la row dans la base partagée et s'en abstient parce qu'elle n'est pas de sa
/// tranche, pas parce qu'il ne la voit pas.
#[tokio::test]
async fn un_pilote_vivant_et_muet_est_trouve_et_tue() -> Result<()> {
    let h = MultiAgentHarness::builder()
        .agent(DISPATCHER_AGENT)
        .agent(OTHER_AGENT)
        .build()?;

    let tmp = tempfile::tempdir()?;
    let log_dir = pilot_log_dir(tmp.path());
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let reason_path = format!("/tmp/mika-cancel-reason-{pid}");
    let _ = std::fs::remove_file(&reason_path);

    let id = seed_live_dispatch(
        h.db(DISPATCHER_AGENT),
        DISPATCHER_AGENT,
        h.session_id(DISPATCHER_AGENT),
        pid,
        start_time,
        &declaration,
        tmp.path(),
        &log_dir,
    )
    .await?;

    assert_eq!(
        h.audit_counts_by_agent("pilot_silent_stall").await?,
        [
            (DISPATCHER_AGENT.to_string(), 0),
            (OTHER_AGENT.to_string(), 0)
        ]
        .into_iter()
        .collect(),
        "état de départ : aucun audit sous aucun agent"
    );

    // --- Contrôle d'attribution : l'autre agent ne fauche pas cette row ------
    let mut autre = engine_for(h.db(OTHER_AGENT), None, &log_dir);
    drive_scan(&mut autre).await;

    assert_eq!(
        h.db(DISPATCHER_AGENT)
            .get_task(&id)
            .await?
            .expect("la row existe toujours")
            .status,
        "pending",
        "le moteur de `{OTHER_AGENT}` voit la row de `{DISPATCHER_AGENT}` dans la base \
         partagée et ne doit pas y toucher"
    );
    assert!(
        is_same_process_alive(pid_u32, start_time),
        "et il ne doit pas non plus signaler le processus"
    );
    assert_eq!(
        h.audit_counts_by_agent("pilot_silent_stall")
            .await?
            .get(OTHER_AGENT),
        Some(&0),
        "aucun audit ne doit apparaître sous `{OTHER_AGENT}`"
    );

    // --- Contrôle positif : l'agent propriétaire fauche ----------------------
    let mut proprietaire = engine_for(h.db(DISPATCHER_AGENT), None, &log_dir);
    assert!(
        is_same_process_alive(pid_u32, start_time),
        "contrôle d'entrée : le pilote est bien vivant au moment où son propre agent scanne — \
         sans cette ligne, l'assertion de mort qui suit pourrait porter sur un processus \
         déjà parti pour une raison étrangère au reaper"
    );
    drive_scan(&mut proprietaire).await;

    assert!(
        !is_same_process_alive(pid_u32, start_time),
        "LE point du ticket : un processus RÉEL, vivant et muet, doit être mort \
         après le scan. C'est la seule assertion que le reaper inerte n'aurait \
         pas pu satisfaire, et elle se lit sur /proc, pas sur une valeur de retour"
    );

    let task = h
        .db(DISPATCHER_AGENT)
        .get_task(&id)
        .await?
        .expect("la row existe");
    assert_eq!(
        task.status, "failed",
        "la disposition armée par défaut doit transitionner la row"
    );
    assert_ne!(
        task.status, "cancelled",
        "`cancelled` porte do-NOT-retry vers l'aval ; ce dispatch doit rester re-jouable"
    );

    let counts = h.audit_counts_by_agent("pilot_silent_stall").await?;
    assert_eq!(
        counts.get(DISPATCHER_AGENT),
        Some(&1),
        "exactement un `pilot_silent_stall`, sous l'agent qui a fauché ; carte : {counts:?}"
    );
    assert_eq!(
        counts.get(OTHER_AGENT),
        Some(&0),
        "et toujours aucun sous l'autre ; carte : {counts:?}"
    );

    let reason = std::fs::read_to_string(&reason_path)
        .expect("le discriminant doit être écrit AVANT le signal");
    assert!(
        reason.contains("REAPED_PILOT_SILENT_STALL"),
        "attendu le discriminant silent-stall, obtenu {reason:?}"
    );

    let _ = std::fs::remove_file(&reason_path);
    kill_pid(pid);
    h.shutdown();
    Ok(())
}

// ---------------------------------------------------------------------------
// Le contrôle négatif apparié : l'observation reste possible, explicitement
// ---------------------------------------------------------------------------

/// Désarmé **explicitement**, le même dispatch est vu et laissé intact.
///
/// Apparié au test précédent : sans lui, « armé par défaut » et « le désarmement
/// ne marche plus » rendraient le même vert. C'est aussi ce qui rend l'armement
/// réversible sans rebuild — la garde que mika#2272 met à la place de la
/// condition de bascule de #2249.
#[tokio::test]
async fn desarme_explicitement_il_observe_sans_faucher() -> Result<()> {
    let h = MultiAgentHarness::builder()
        .agent(DISPATCHER_AGENT)
        .build()?;
    let db = h.db(DISPATCHER_AGENT);

    let tmp = tempfile::tempdir()?;
    let log_dir = pilot_log_dir(tmp.path());
    let worktree = seed_worktree(tmp.path(), STALE_SECS);
    let declaration = declare_worktree(tmp.path(), &worktree);

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_live_dispatch(
        db,
        DISPATCHER_AGENT,
        h.session_id(DISPATCHER_AGENT),
        pid,
        start_time,
        &declaration,
        tmp.path(),
        &log_dir,
    )
    .await?;

    let mut engine = engine_for(db, Some(false), &log_dir);
    drive_scan(&mut engine).await;

    assert_eq!(
        db.count_audit_events_by_tool_name("pilot_silent_stall")
            .await?,
        1,
        "contrôle positif : la détection est inconditionnelle, la row d'audit est écrite"
    );
    assert_eq!(
        db.get_task(&id).await?.expect("la row existe").status,
        "pending",
        "contrôle négatif : désarmé, il ne transitionne pas"
    );
    assert!(
        is_same_process_alive(pid_u32, start_time),
        "contrôle négatif : désarmé, il ne signale pas le processus"
    );

    kill_pid(pid);
    h.shutdown();
    Ok(())
}
