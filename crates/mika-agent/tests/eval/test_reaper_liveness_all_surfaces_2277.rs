//! Le reaper D1 ne fauche que le silence sur **toutes** les surfaces du pilote
//! (mika#2277, AC2).
//!
//! # Le défaut que ce fichier ferme, mesuré
//!
//! Le 2026-09-10 à 07:16:10Z — premier déploiement armé de mika#2275 — le
//! reaper a tué **les deux seuls pilotes en vol**, tous deux vivants et
//! productifs. Leurs logs claude-pilot avaient été écrits 3 min 40 s et 3 min
//! 31 s avant le kill ; leurs transcripts aussi. Leur worktree, lui, n'avait
//! pas bougé depuis le `checkout` : 2758 s et 2744 s, au-delà de la fenêtre de
//! 2700 s. Un pilote en phase lecture / exploration / planification ne touche
//! aucun fichier du worktree pendant que le SDK lui rend des tours en continu.
//!
//! La mécanique de mika#2275 était correcte — bonne row `pending`, bon pid,
//! kill effectif, audit écrit. **Le prédicat était faux :** « aucune écriture
//! worktree » n'est pas « pilote muet ».
//!
//! # Pourquoi quatre contrôles négatifs et pas un
//!
//! Le prédicat est un **ET de trois termes**, et un ET ne se prouve pas en
//! neutralisant le tout : un test unique qui rend les trois surfaces fraîches
//! passerait au vert sur un prédicat qui n'en lit qu'une. Chaque terme doit
//! donc être montré porteur **séparément** :
//!
//! - **N1** — la forme exacte de l'incident : transcript frais, log frais,
//!   worktree intact au-delà de la fenêtre.
//! - **N2** — transcript frais **seul**.
//! - **N3** — log frais **seul**.
//! - **N4** — signal **indisponible** (clé `pilot_transcript_expected` absente),
//!   tout le reste muet. C'est la règle de sûreté : un signal qu'on ne peut pas
//!   lire n'est jamais un terme satisfait.
//!
//! Le contrôle **positif** vit dans la même suite, et il n'est pas décoratif :
//! sans lui, les quatre négatifs seraient tous satisfaits par un reaper qui ne
//! fauche plus rien — c'est-à-dire par la panne symétrique de mika#2261.
//!
//! # Le rouge-avant, mesuré et non supposé
//!
//! Les cinq tests ont été lancés contre le prédicat worktree-seul **avant** de
//! le corriger, et les cinq étaient rouges, chacun pour sa propre raison :
//! N1/N2/N3 sur `is_same_process_alive` (le pilote vivant recevait bel et bien
//! le SIGTERM), N4 de même, et le contrôle positif sur l'assertion AC5 — il
//! fauchait déjà, mais sa row d'audit ne portait que `worktree_idle_secs`.
//! Aucun des cinq n'est donc vacuously satisfait par la nouvelle
//! implémentation.
//!
//! # mika#2875 — les deux portes posées devant l'avertissement de N4
//!
//! L'avertissement que N4 exige (`pilot_stall_signal_unavailable`) a été mesuré
//! le 2026-10-22 : **135 tirs depuis le 2026-10-07**, dont 120 sur des
//! `long_running:build_mika` — qui ne sont pas des pilotes et n'ont par
//! construction aucune des trois surfaces — et 14 sur de vrais pilotes
//! signalés **entre 1 et 41 s** après leur création. 100 % partaient dans les
//! 60 s. Le marqueur posé à ce premier tir faisait qu'une indisponibilité
//! *tardive* — le seul cas que l'instrument existe pour voir — n'était plus
//! jamais signalée.
//!
//! Deux portes ferment cela, dans cet ordre : la **population** (seuls les deux
//! labels de pilote), puis l'**âge** (rien ne se dit et rien ne se marque tant
//! que le dispatch est plus jeune que le seuil du reaper). AC1, AC2 et AC3 les
//! tiennent, et N4 a été **réparé** — antidaté — plutôt qu'excepté : ce qu'il
//! décrivait était une indisponibilité de naissance, ce qu'il décrit désormais
//! est l'indisponibilité tardive qu'il a toujours voulu dire.
//!
//! # Ce que ce fichier NE couvre pas
//!
//! Le comportement de la sonde mtime elle-même (fichier absent, répertoire,
//! mtime futur, lien symbolique) est unitaire, dans
//! `task_engine::worktree_activity`. Le rejouer ici achèterait une suite plus
//! lente et la même réponse.
//!
//! # Fire-Disposition
//!
//! **Halte-et-remontée, gate CI bloquant** pour les cinq tests. Un N rouge veut
//! dire que le reaper fauche du travail vivant — la classe que ce ticket
//! répare. Le contrôle positif rouge veut dire qu'il ne fauche plus rien, ce
//! qui est un défaut de même gravité.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use mika_agent::async_db::AsyncDatabase;
use mika_agent::db::Database;
use mika_agent::messaging::{MessageSender, SendOutcome};
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::executor::build_callback_task;
use mika_agent::task_engine::dispatcher::TaskDispatcher;
use mika_agent::task_engine::engine::TaskEngine;
use mika_agent::task_engine::process_liveness::is_same_process_alive;
use mika_agent::tools::default_tools;

use super::process_fixtures::{kill_pid, spawn_live_child};

const AGENT_ID: &str = "mika";

/// Bien au-delà des 2700 s par défaut : la fixture n'encode pas le seuil une
/// seconde fois, `mika_common::config` le possède et l'épingle.
const SILENT_SECS: u64 = 10_000;

/// La fraîcheur mesurée sur les deux faux positifs du 2026-09-10 : le log et le
/// transcript avaient été écrits ~3 min 35 s avant le kill.
const FRESH_SECS: u64 = 215;

struct NoopSender;

#[async_trait::async_trait]
impl MessageSender for NoopSender {
    async fn send(&self, _text: &str) -> anyhow::Result<SendOutcome> {
        Ok(SendOutcome::Delivered)
    }
}

fn test_db() -> AsyncDatabase {
    let db = Database::open_in_memory().expect("open in-memory DB");
    AsyncDatabase::new_with_agent(db, AGENT_ID)
}

// -- capture des événements tracing (même forme que `test_llm_watchdog_2342`) --

/// Paires `(message, champs)` capturées, partagées entre la couche et le test.
type Captured = Arc<std::sync::Mutex<Vec<(String, std::collections::HashMap<String, String>)>>>;

struct CapturingLayer {
    events: Captured,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CapturingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = std::collections::HashMap::new();
        let mut visitor = FieldVisitor(&mut fields);
        event.record(&mut visitor);
        let message = fields.remove("message").unwrap_or_default();
        if let Ok(mut events) = self.events.lock() {
            events.push((message, fields));
        }
    }
}

struct FieldVisitor<'a>(&'a mut std::collections::HashMap<String, String>);

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

fn capture() -> (tracing::subscriber::DefaultGuard, Captured) {
    use tracing_subscriber::layer::SubscriberExt;
    let events: Captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturingLayer {
        events: Arc::clone(&events),
    });
    let guard = tracing::subscriber::set_default(subscriber);
    (guard, events)
}

/// Les événements `pilot_stall_signal_unavailable` capturés, avec leurs champs.
///
/// On filtre sur le champ `event` plutôt que sur le texte du message : c'est le
/// nom que l'opérateur grep, et c'est ce que mika#2875 tient.
fn unavailable_warnings(events: &Captured) -> Vec<std::collections::HashMap<String, String>> {
    events
        .lock()
        .expect("lock des événements capturés")
        .iter()
        .filter(|(_, fields)| {
            fields.get("event").map(String::as_str) == Some("pilot_stall_signal_unavailable")
        })
        .map(|(_, fields)| fields.clone())
        .collect()
}

/// Un dispatcher armé, dont le répertoire de logs pilote pointe sur le tmpdir
/// du test.
///
/// L'armement et le répertoire passent par `Settings`, jamais par
/// `MIKA_PILOT_*` : l'env est global au process et ce binaire lance ses tests
/// en parallèle.
fn dispatcher_with_log_dir(db: &AsyncDatabase, pilot_log_dir: &Path) -> Arc<TaskDispatcher> {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let mut settings = mika_common::config::Settings::load(tmp.path()).expect("load settings");
    settings.pilot_stall_reap_enabled = Some(true);
    settings.pilot_log_dir = Some(pilot_log_dir.to_string_lossy().into_owned());
    Arc::new(TaskDispatcher {
        db: db.clone(),
        tier: mika_common::home::AgentTier::Default,
        deployment: mika_common::home::Deployment::Unknown,
        llm: mika_common::llm::dummy_provider(),
        tools: Arc::new(default_tools()),
        skills: Arc::new(SkillRegistry::empty()),
        message_sender: Some(Arc::new(NoopSender)),
        home_dir: PathBuf::from("/tmp"),
        // mika#2329 — home global inexistant : aucun STOP n'y est armé, chemin nominal.
        global_home_dir: PathBuf::from("/tmp/mika-test-global-home-absent"),
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
        auto_pull_stop_armed: AtomicBool::new(false),
        worktree_reap_stop_armed: AtomicBool::new(false),
        proactive_budget_reported: std::sync::Mutex::new(None),
    })
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

/// Âges des trois surfaces, en secondes. `None` = surface **indisponible**
/// (fichier jamais écrit, ou clé de metadata jamais estampillée).
#[derive(Clone, Copy)]
struct Surfaces {
    worktree: Option<u64>,
    transcript: Option<u64>,
    pilot_log: Option<u64>,
}

impl Surfaces {
    /// Les trois muettes : le seul état où le reaper a le droit de tirer.
    fn all_silent() -> Self {
        Self {
            worktree: Some(SILENT_SECS),
            transcript: Some(SILENT_SECS),
            pilot_log: Some(SILENT_SECS),
        }
    }
}

/// Sème un dispatch **par le chemin de production** et pose les trois surfaces.
///
/// Aucun `update_task_status` : le status est celui que `create_task` écrit
/// pour un `NewTask` construit par [`build_callback_task`] — `pending`, la row
/// qui porte réellement le pid (mika#2272).
///
/// `tool_name` est passé **au producteur du label** plutôt qu'écrit en dur :
/// c'est ce qui rend AC3 de mika#2875 testable (`build_mika`) sans qu'un
/// littéral `"long_running:build_mika"` recopié ici puisse dériver de la forme
/// que `build_callback_task` émet réellement.
#[allow(clippy::too_many_arguments)]
async fn seed_dispatch(
    db: &AsyncDatabase,
    root: &Path,
    pilot_log_dir: &Path,
    pid: i64,
    start_time: u64,
    surfaces: Surfaces,
    tool_name: &str,
) -> Result<String> {
    let input = serde_json::json!({
        "skill": "self-dev",
        "prompt": "implémente mika#2277",
        "task_id": "parent-inexistant",
        "branch": "bug/2277/task-engine-reaper-faux-positif-d1-le",
    });
    let task = build_callback_task(
        AGENT_ID.to_string(),
        None,
        tool_name,
        &input,
        3600,
        "eval-session",
        "trace-2277",
        None,
    );
    let id = db.create_task(task).await?;

    // Les écritures que `spawn_long_running_exec` fait après le spawn, dans la
    // même forme — `process_start_time` est stocké en chaîne JSON par la
    // production, le copier autrement validerait une forme qui n'existe pas.
    db.set_task_process_id(&id, Some(pid)).await?;
    db.set_task_metadata_field(&id, "process_start_time", &start_time.to_string())
        .await?;

    if let Some(idle) = surfaces.worktree {
        let worktree = seed_worktree(root, idle);
        let declaration = declare_worktree(root, &worktree);
        db.set_task_metadata_field(
            &id,
            "dispatch_worktree_file",
            &declaration.to_string_lossy(),
        )
        .await?;
    }

    if let Some(idle) = surfaces.transcript {
        // `inject_pilot_transcript_env` écrit le fichier ET estampille la clé ;
        // les deux ensemble, ou la surface est indisponible.
        let transcript = root.join(format!("{id}.jsonl"));
        std::fs::write(&transcript, b"{\"type\":\"llm_call\"}\n").expect("write transcript");
        set_mtime(&transcript, idle);
        db.set_task_metadata_field(
            &id,
            "pilot_transcript_expected",
            &transcript.to_string_lossy(),
        )
        .await?;
    }

    if let Some(idle) = surfaces.pilot_log {
        // Le log pilote n'est PAS déclaré : le moteur le dérive de
        // `<pilot_log_dir>/<task.id>.log`, exactement comme `dispatch-lib.sh`
        // le compose (`--log-dir "$_PILOT_LOG_DIR" --task-id "$LOG_ID"`).
        let log = pilot_log_dir.join(format!("{id}.log"));
        std::fs::write(&log, b"tool result\n").expect("write pilot log");
        set_mtime(&log, idle);
    }

    Ok(id)
}

/// Antidate le `fired_at` d'un dispatch de `age_secs` secondes (mika#2875).
///
/// `fired_at` et non `created_at` : c'est l'estampille que
/// `set_task_process_id` pose **au spawn** (mika#2263), donc celle que la porte
/// d'âge lit en priorité. Antidater `created_at` seul ne déplacerait rien.
async fn backdate_fired_at(db: &AsyncDatabase, task_id: &str, age_secs: u64) -> Result<()> {
    let row = task_id.to_string();
    let sql = format!(
        "UPDATE tasks SET fired_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{age_secs} seconds') \
         WHERE id = ?1"
    );
    db.with_db(move |db| {
        db.execute_sql(&sql, &[&row])?;
        Ok(())
    })
    .await?;
    Ok(())
}

async fn stall_audit_count(db: &AsyncDatabase) -> i64 {
    db.count_audit_events_by_tool_name("pilot_silent_stall")
        .await
        .expect("count audit events")
}

/// La clé de metadata que le reaper estampille après avoir signalé une surface
/// indisponible — son garde-fou « au plus une fois par dispatch » (AC4).
async fn inertia_marker(db: &AsyncDatabase, task_id: &str) -> Option<String> {
    let task = db.get_task(task_id).await.ok()??;
    let metadata: serde_json::Value = serde_json::from_str(task.metadata.as_deref()?).ok()?;
    Some(
        metadata
            .get("pilot_stall_signal_unavailable_reported")?
            .as_str()?
            .to_string(),
    )
}

/// Monte le décor commun : tmpdir, répertoire de logs pilote, base, moteur.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    pilot_log_dir: PathBuf,
    db: AsyncDatabase,
    engine: TaskEngine,
    /// Le seuil d'âge du reaper, lu sur les **mêmes** `Settings` que le moteur
    /// résout (mika#2875). Pas un `2700` recopié : la porte d'âge n'introduit
    /// aucun second réglage, donc la fixture ne doit introduire aucun second
    /// littéral.
    max_age_secs: u64,
}

fn fixture() -> Result<Fixture> {
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().to_path_buf();
    let pilot_log_dir = root.join("var-log-claude-pilot");
    std::fs::create_dir_all(&pilot_log_dir)?;
    let db = test_db();
    let dispatcher = dispatcher_with_log_dir(&db, &pilot_log_dir);
    let max_age_secs = dispatcher.settings.effective_pilot_stall_reap_age_seconds();
    let engine = TaskEngine::new(db.clone(), dispatcher);
    Ok(Fixture {
        _tmp: tmp,
        root,
        pilot_log_dir,
        db,
        engine,
        max_age_secs,
    })
}

// ---------------------------------------------------------------------------
// Contrôle POSITIF — les trois surfaces muettes, le reaper tire
// ---------------------------------------------------------------------------

/// Worktree, transcript et log tous muets au-delà de la fenêtre, processus
/// vivant ⇒ SIGTERM émis, row `failed`, audit écrit.
///
/// Il porte aussi AC5 : la row d'audit doit nommer les **trois** âges, pas le
/// seul worktree. Cette row est la seule surface par laquelle quiconque lit ce
/// mécanisme ; sans les trois nombres, la décision n'est pas rejouable — et
/// c'est précisément en relisant `worktree_idle_secs=2758` seul que le
/// 2026-09-10 a d'abord ressemblé à un fonctionnement nominal.
#[tokio::test]
async fn controle_positif_les_trois_surfaces_muettes_sont_fauchees() -> Result<()> {
    let mut f = fixture()?;

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        Surfaces::all_silent(),
        "run_claude_pilot",
    )
    .await?;

    assert_eq!(
        stall_audit_count(&f.db).await,
        0,
        "état de départ : aucun audit avant le scan"
    );
    assert!(
        is_same_process_alive(pid_u32, start_time),
        "contrôle d'entrée : le pilote est vivant au moment du scan — sans cette ligne, \
         l'assertion de mort qui suit pourrait porter sur un processus déjà parti"
    );

    drive_scan(&mut f.engine).await;

    assert!(
        !is_same_process_alive(pid_u32, start_time),
        "les trois surfaces muettes : le processus doit être mort après le scan"
    );

    let task = f.db.get_task(&id).await?.expect("la row existe");
    assert_eq!(
        task.status, "failed",
        "la disposition armée doit transitionner la row"
    );
    assert_eq!(
        stall_audit_count(&f.db).await,
        1,
        "exactement un `pilot_silent_stall` par dispatch fauché"
    );

    // AC5 — la row d'audit porte les trois âges.
    let rows =
        f.db.get_audit_event_rows_by_tool_name("pilot_silent_stall")
            .await?;
    let reasoning = rows
        .first()
        .and_then(|(_, _, _, reasoning)| reasoning.clone())
        .expect("la row d'audit porte un `reasoning`");
    for axis in [
        "worktree_idle_secs",
        "transcript_idle_secs",
        "pilot_log_idle_secs",
    ] {
        assert!(
            reasoning.contains(axis),
            "AC5 : la row d'audit doit nommer `{axis}` ; obtenu {reasoning:?}"
        );
    }

    let _ = std::fs::remove_file(format!("/tmp/mika-cancel-reason-{pid}"));
    kill_pid(pid);
    Ok(())
}

// ---------------------------------------------------------------------------
// N1 — LA PORTE : la forme exacte de l'incident du 2026-09-10
// ---------------------------------------------------------------------------

/// Transcript **frais**, log **frais**, worktree intact depuis plus de 2700 s
/// ⇒ **aucun** kill, **aucune** transition, **aucune** row d'audit.
///
/// C'est le test que le ticket exige mot pour mot, et le seul dont le rouge
/// reproduit l'incident : sur le prédicat worktree-seul de mika#2275, ce
/// dispatch était fauché.
#[tokio::test]
async fn n1_log_et_transcript_frais_worktree_intact_aucun_kill() -> Result<()> {
    let mut f = fixture()?;

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        Surfaces {
            worktree: Some(SILENT_SECS),
            transcript: Some(FRESH_SECS),
            pilot_log: Some(FRESH_SECS),
        },
        "run_claude_pilot",
    )
    .await?;

    drive_scan(&mut f.engine).await;

    assert!(
        is_same_process_alive(pid_u32, start_time),
        "AC2 : un pilote qui consomme des tours LLM ne doit JAMAIS recevoir de signal, \
         quel que soit le mtime de son worktree"
    );
    assert_eq!(
        f.db.get_task(&id).await?.expect("la row existe").status,
        "pending",
        "aucune disposition : la row reste sur sa surface vivante"
    );
    assert_eq!(
        stall_audit_count(&f.db).await,
        0,
        "aucune détection armée non plus — un audit ici voudrait dire que le prédicat \
         a conclu au silence puis s'est retenu, ce qui n'est pas ce que AC1 demande"
    );
    assert_eq!(
        inertia_marker(&f.db, &id).await,
        None,
        "AC4 : un dispatch écarté parce qu'une surface est ACTIVE est le fonctionnement \
         nominal et reste silencieux — seule l'indisponibilité se dit"
    );

    kill_pid(pid);
    Ok(())
}

// ---------------------------------------------------------------------------
// N2 / N3 — chaque terme porteur SÉPARÉMENT
// ---------------------------------------------------------------------------

/// Transcript frais **seul** : log et worktree muets ⇒ aucun kill.
///
/// Sans ce contrôle, un prédicat qui aurait remplacé le worktree par le log
/// (au lieu de conjoindre les trois) passerait N1 au vert.
#[tokio::test]
async fn n2_transcript_frais_seul_aucun_kill() -> Result<()> {
    let mut f = fixture()?;

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        Surfaces {
            worktree: Some(SILENT_SECS),
            transcript: Some(FRESH_SECS),
            pilot_log: Some(SILENT_SECS),
        },
        "run_claude_pilot",
    )
    .await?;

    drive_scan(&mut f.engine).await;

    assert!(
        is_same_process_alive(pid_u32, start_time),
        "le transcript est le signal le plus proche de la classe visée (« le flux SDK se \
         tait ») : un tour LLM horodaté suffit à établir la vie"
    );
    assert_eq!(
        f.db.get_task(&id).await?.expect("la row existe").status,
        "pending"
    );
    assert_eq!(stall_audit_count(&f.db).await, 0);
    assert_eq!(inertia_marker(&f.db, &id).await, None);

    kill_pid(pid);
    Ok(())
}

/// Log claude-pilot frais **seul** : transcript et worktree muets ⇒ aucun kill.
///
/// Le pendant de N2. Il compte pour de vrai : `MIKA_LOG_PILOT_TRANSCRIPTS`
/// peut être coupé, auquel cas le log est la seule surface qui reste — et un
/// prédicat qui ne lirait que le transcript rendrait ce dispatch fauchable.
#[tokio::test]
async fn n3_log_pilote_frais_seul_aucun_kill() -> Result<()> {
    let mut f = fixture()?;

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        Surfaces {
            worktree: Some(SILENT_SECS),
            transcript: Some(SILENT_SECS),
            pilot_log: Some(FRESH_SECS),
        },
        "run_claude_pilot",
    )
    .await?;

    drive_scan(&mut f.engine).await;

    assert!(
        is_same_process_alive(pid_u32, start_time),
        "le log pilote actif établit la vie tout autant que le transcript"
    );
    assert_eq!(
        f.db.get_task(&id).await?.expect("la row existe").status,
        "pending"
    );
    assert_eq!(stall_audit_count(&f.db).await, 0);
    assert_eq!(inertia_marker(&f.db, &id).await, None);

    kill_pid(pid);
    Ok(())
}

// ---------------------------------------------------------------------------
// N4 — un signal indisponible n'est jamais un terme satisfait
// ---------------------------------------------------------------------------

/// `pilot_transcript_expected` absent de la metadata, tout le reste muet ⇒
/// aucun kill, **et** l'inertie est dite (AC4).
///
/// Les deux moitiés comptent. Sans la première, couper
/// `MIKA_LOG_PILOT_TRANSCRIPTS` rendrait chaque dispatch fauchable sur deux
/// termes au lieu de trois. Sans la seconde, la même coupure **désarmerait le
/// reaper en silence** — et la leçon de mika#2272 est exactement qu'un
/// compteur à zéro peut être l'absence de mesure et non l'absence de défaut.
///
/// # L'indisponibilité décrite ici est désormais une indisponibilité TARDIVE
///
/// Le dispatch est antidaté au-delà du seuil (mika#2875). Avant cette porte,
/// une row née à l'instant suffisait : l'avertissement partait dans la seconde.
/// C'est précisément ce qui le rendait inutile — sur 135 tirs mesurés le
/// 2026-10-22, **100 % partaient dans les 60 s** suivant la création, donc sur
/// une surface qu'aucun pilote n'avait encore eu le temps d'écrire, et le
/// marqueur posé là consommait le signal avant qu'il n'ait vu son objet.
/// L'inertie que AC4 veut voir dite est celle d'un dispatch **qui a vécu** sans
/// qu'aucune surface n'apparaisse ; l'antidatage est ce qui en fait celle-là.
#[tokio::test]
async fn n4_signal_indisponible_aucun_kill_et_inertie_visible() -> Result<()> {
    let mut f = fixture()?;

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        Surfaces {
            worktree: Some(SILENT_SECS),
            transcript: None,
            pilot_log: Some(SILENT_SECS),
        },
        "run_claude_pilot",
    )
    .await?;
    backdate_fired_at(&f.db, &id, f.max_age_secs + 60).await?;

    drive_scan(&mut f.engine).await;

    assert!(
        is_same_process_alive(pid_u32, start_time),
        "un signal qu'on ne peut pas lire n'est pas un signal de silence"
    );
    assert_eq!(
        f.db.get_task(&id).await?.expect("la row existe").status,
        "pending"
    );
    assert_eq!(stall_audit_count(&f.db).await, 0);

    let marker = inertia_marker(&f.db, &id)
        .await
        .expect("AC4 : l'inertie doit être estampillée, pas silencieuse");
    assert!(
        marker.contains("transcript"),
        "AC4 : le marqueur doit NOMMER la surface manquante ; obtenu {marker:?}"
    );

    kill_pid(pid);
    Ok(())
}

// ---------------------------------------------------------------------------
// mika#2875 — les deux portes : l'âge, puis la population
// ---------------------------------------------------------------------------

/// Les trois surfaces **indisponibles** : la forme exacte des 135 lignes
/// mesurées le 2026-10-22 (`missing_surfaces = transcript,pilot_log,worktree`).
const NO_SURFACE: Surfaces = Surfaces {
    worktree: None,
    transcript: None,
    pilot_log: None,
};

/// **AC1** — un pilote plus jeune que le seuil ne dit rien et ne marque rien.
///
/// Le dispatch est semé à son âge naturel (quelques millisecondes) : c'est la
/// population réelle de l'avertissement avant mika#2875 — 14 pilotes signalés
/// entre 1 et 41 s après leur création, avant qu'aucun d'eux n'ait eu le temps
/// d'écrire une surface.
///
/// Les deux assertions sont solidaires et aucune ne remplace l'autre. Le
/// silence seul laisserait passer une implémentation qui tait l'avertissement
/// **mais pose quand même le marqueur** — et c'est la pire des deux, parce
/// qu'elle consomme le signal sans même le dire : l'indisponibilité tardive, le
/// seul cas que cet instrument existe pour voir, serait alors perdue pour
/// toujours et en silence.
#[tokio::test]
async fn ac1_un_pilote_plus_jeune_que_le_seuil_ne_dit_rien_et_ne_marque_rien() -> Result<()> {
    let mut f = fixture()?;
    let (guard, events) = capture();

    let (pid, start_time) = spawn_live_child();
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        NO_SURFACE,
        "run_claude_pilot",
    )
    .await?;

    drive_scan(&mut f.engine).await;

    let warnings = unavailable_warnings(&events);
    assert!(
        warnings.is_empty(),
        "AC1 : un dispatch né il y a un instant n'a encore RIEN pu écrire — \
         l'avertissement ne décrit donc aucune inertie ; obtenu {warnings:?}"
    );
    assert_eq!(
        inertia_marker(&f.db, &id).await,
        None,
        "AC1 : et surtout aucun marqueur — un marqueur posé ici consommerait \
         le signal avant qu'il n'ait vu son objet (mika#2875)"
    );

    drop(guard);
    kill_pid(pid);
    Ok(())
}

/// **AC2** — le MÊME dispatch, passé le seuil, dit exactement une fois.
///
/// Deux phases sur une seule row, et c'est ce qui en fait une preuve. Deux
/// tests séparés — un jeune muet, un vieux bavard — seraient tous deux verts
/// sur une implémentation qui taît l'avertissement **définitivement** : chacun
/// regarderait une row différente, et le report du signal (ce que mika#2875
/// demande réellement : plus tard, pas jamais) ne serait attesté par personne.
///
/// La phase B passe **deux** balayages. Le premier doit dire une fois ; le
/// second ne doit rien ajouter — c'est le garde-fou de cadence (AC4 de
/// mika#2277) qui doit survivre à la porte d'âge, pas disparaître avec elle.
#[tokio::test]
async fn ac2_le_meme_dispatch_au_dela_du_seuil_dit_exactement_une_fois() -> Result<()> {
    let mut f = fixture()?;
    let (guard, events) = capture();

    let (pid, start_time) = spawn_live_child();
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        NO_SURFACE,
        "run_claude_pilot",
    )
    .await?;

    // ── Phase A : l'âge natif. Rien ne se dit, rien ne se marque.
    drive_scan(&mut f.engine).await;
    assert!(
        unavailable_warnings(&events).is_empty(),
        "phase A : le dispatch est plus jeune que le seuil"
    );
    assert_eq!(
        inertia_marker(&f.db, &id).await,
        None,
        "phase A : le marqueur reste à poser — c'est ce qui rend la phase B \
         possible"
    );

    // ── Phase B : le même dispatch, vieilli au-delà du seuil.
    backdate_fired_at(&f.db, &id, f.max_age_secs + 120).await?;
    drive_scan(&mut f.engine).await;
    drive_scan(&mut f.engine).await;

    let warnings = unavailable_warnings(&events);
    assert_eq!(
        warnings.len(),
        1,
        "AC2 : exactement UN avertissement sur deux balayages ; obtenu \
         {warnings:?}"
    );
    let fields = &warnings[0];

    assert_eq!(
        fields.get("task_id").map(String::as_str),
        Some(id.as_str()),
        "l'avertissement doit nommer CE dispatch ; obtenu {fields:?}"
    );
    assert_eq!(
        fields.get("missing_surfaces").map(String::as_str),
        Some("transcript,pilot_log,worktree"),
        "les trois surfaces manquent, et l'avertissement les nomme toutes — \
         c'est la forme mesurée en production ; obtenu {fields:?}"
    );

    // L'âge dit, et dit au-delà du seuil : sans cette assertion, un
    // avertissement émis par une porte qui compare le mauvais horodatage
    // passerait au vert.
    let age = fields
        .get("age_secs")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| panic!("l'avertissement doit porter `age_secs` ; obtenu {fields:?}"));
    assert!(
        age >= i64::try_from(f.max_age_secs).expect("le seuil tient dans i64"),
        "AC2 : l'âge rapporté ({age} s) doit être au moins le seuil \
         ({} s) — c'est la condition même de l'émission",
        f.max_age_secs
    );
    assert_eq!(
        fields
            .get("max_age_secs")
            .and_then(|s| s.parse::<u64>().ok()),
        Some(f.max_age_secs),
        "le seuil est rapporté avec l'âge : un âge sans son seuil n'est pas \
         relisible ; obtenu {fields:?}"
    );
    assert_eq!(
        fields.get("label").map(String::as_str),
        Some("long_running:run_claude_pilot"),
        "le label est rapporté : c'est ce qui rend la population lisible dans \
         le journal, et c'est ce dont l'absence a coûté le diagnostic de \
         mika#2875 ; obtenu {fields:?}"
    );

    let marker = inertia_marker(&f.db, &id)
        .await
        .expect("AC2 : le marqueur est posé à CE moment-là, pas avant");
    assert_eq!(
        marker, "transcript,pilot_log,worktree",
        "le marqueur porte les mêmes surfaces que l'avertissement"
    );

    drop(guard);
    kill_pid(pid);
    Ok(())
}

/// **AC3** — un `long_running:build_mika` ne produit aucun avertissement.
///
/// 120 des 135 lignes mesurées le 2026-10-22 étaient des builds. Un build n'a
/// par construction ni transcript, ni log pilote, ni worktree déclaré : ce
/// n'est pas une anomalie d'observabilité, c'est sa définition. L'instrument
/// mesurait donc une population qu'il ne nomme pas (classe mika#2205).
///
/// Il est antidaté **bien au-delà** du seuil, et c'est délibéré : sans cela, la
/// porte d'âge seule suffirait à le taire, et ce test serait vert sur une
/// implémentation qui n'a aucune porte de population. Son contrôle positif est
/// `ac2_le_meme_dispatch_au_dela_du_seuil_dit_exactement_une_fois`, qui sème la
/// même absence de surface au même âge sous un label de pilote et obtient un
/// avertissement.
///
/// La seconde moitié est le contrôle négatif que le ticket exige mot pour mot :
/// *« ne change pas ce que le reaper fait des pilotes »* — et donc ici, des
/// non-pilotes. Taire l'avertissement ne doit pas se payer d'une disposition.
#[tokio::test]
async fn ac3_un_build_mika_ne_produit_aucun_pilot_stall_signal_unavailable() -> Result<()> {
    let mut f = fixture()?;
    let (guard, events) = capture();

    let (pid, start_time) = spawn_live_child();
    let pid_u32 = u32::try_from(pid).expect("pid tient dans u32");
    let id = seed_dispatch(
        &f.db,
        &f.root,
        &f.pilot_log_dir,
        pid,
        start_time,
        NO_SURFACE,
        "build_mika",
    )
    .await?;
    backdate_fired_at(&f.db, &id, f.max_age_secs * 4).await?;

    // Contrôle d'entrée : la row est bien dans la population du reaper. Sans
    // cette ligne, un filtre en amont (requête, statut, pid) rendrait AC3 vert
    // par vacuité.
    let seeded = f.db.get_task(&id).await?.expect("la row existe");
    assert_eq!(seeded.label, "long_running:build_mika");
    assert_eq!(seeded.status, "pending");
    assert!(seeded.process_id.is_some());

    drive_scan(&mut f.engine).await;

    let warnings = unavailable_warnings(&events);
    assert!(
        warnings.is_empty(),
        "AC3 : un build n'est pas un pilote et ne doit pas peupler cet \
         avertissement, quel que soit son âge ; obtenu {warnings:?}"
    );
    assert_eq!(
        inertia_marker(&f.db, &id).await,
        None,
        "AC3 : et aucun marqueur non plus — la row sort de la population, elle \
         n'y entre pas pour en être silencieusement retirée"
    );

    // Contrôle négatif : le reaper ne fait rien de plus ni de moins qu'avant.
    assert!(
        is_same_process_alive(pid_u32, start_time),
        "AC3 : un build silencieux n'est pas fauché — la porte retire un \
         avertissement, jamais une protection"
    );
    assert_eq!(
        f.db.get_task(&id).await?.expect("la row existe").status,
        "pending",
        "AC3 : aucune disposition sur la row du build"
    );
    assert_eq!(stall_audit_count(&f.db).await, 0);

    drop(guard);
    kill_pid(pid);
    Ok(())
}
