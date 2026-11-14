use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use secrecy::ExposeSecret;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::commands::chat_turn;
use crate::commands::session_remote;
use crate::commands::tasks::CallbackFeed;
use crate::commands::team_outcome::TeamOutcome;
use crate::init::{self, AppContext};
use crate::tui::app::{
    AgentRequest, AgentResponse, AgentStatus, App, ChatMessage, ChatRole, TeamRequest,
    session_message_to_chat_message,
};
use crate::tui::event::{AppEvent, EventReader};
use crate::tui::input;
use crate::tui::ui;
use mika_agent::agent;
use mika_agent::async_db::AsyncDatabase;
use mika_agent::prompt;
use mika_agent::skills::SkillRegistry;
use mika_agent::skills::assembly::assemble_agent_registry;
use mika_agent::teams::types::TeamEvent;
use mika_agent::tools;
use mika_common::config::Settings;
use mika_common::team;
use std::path::Path;

/// Holds the agent worker supervisor handle, the AppContext (for DB shutdown),
/// and the shutdown-initiated flag the supervisor reads to distinguish operator-driven
/// shutdown from worker death (mika#1149).
struct AgentWorker {
    /// Supervisor task — owns the worker's JoinHandle and forwards unexpected exits
    /// as `AgentResponse::WorkerCrashed` events.
    supervisor_handle: JoinHandle<()>,
    /// Set to `true` before tearing down the worker (Quit, agent switch, /restart)
    /// so the supervisor exits silently on the resulting `Ok(())` from the worker loop.
    shutdown_initiated: Arc<AtomicBool>,
    _ctx: AppContext,
}

/// La source d'environnement arrive en paramètre : un test de refus qui
/// muterait l'environnement du process serait `unsafe` en édition 2024, et faux
/// sous `cargo test`, qui parallélise. La même forme que
/// `tui/commands/handlers.rs::SourceEnv`.
type SourceEnv<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Le jeton que les routes de session exigent, lu **strictement**.
///
/// La seconde phrase du refus n'est pas de la politesse : `dashboard.rs:37-38`
/// accepte aussi `MIKA_DASHBOARD_TOKEN` pour la sonde, là où le tour et les
/// routes de session ne l'acceptent pas. Un shell qui ne porte que ce jeton
/// passait donc la sonde de démarrage et prenait un 401 à chaque tour ; c'est
/// ici le seul endroit du code où cet écart se dit à qui le subit.
///
/// Pas de règle de non-vacuité, délibérément : une variable posée à vide est
/// une *configuration fausse*, pas une absence. Elle part sur le fil et revient
/// en 401, dont le message nomme déjà la variable.
fn jeton_interne(lire: SourceEnv<'_>) -> Result<String> {
    lire("MIKA_INTERNAL_TOKEN").ok_or_else(|| {
        anyhow::anyhow!(
            "La TUI exige MIKA_INTERNAL_TOKEN pour ouvrir sa session sur \
             mika-spirit. Pose-la dans ~/.mika/.env — MIKA_DASHBOARD_TOKEN n'est \
             pas accepté par les routes de session, même s'il suffit à la sonde \
             du tableau de bord."
        )
    })
}

/// Ouvre — ou reprend — la session de la TUI (mika#1727, vol V2d-4a).
///
/// Tout ce qui touche le monde arrive par argument, donc un test pilote la
/// source d'environnement, la base et l'URL du banc sans construire
/// d'`AppContext` : c'est la forme de `rewind_par_spirit`. Elle prend
/// `&AsyncDatabase` et non `&AppContext` pour la même raison.
///
/// Le booléen rendu est `is_singleton_session`. Sur le chemin nominal il vient
/// **du fil** et non de `resolve_canonical_session_id` : la route rend la
/// session canonique de l'agent au lieu d'en créer une
/// (`mika-agent/src/server/sessions.rs`), donc le serveur est la seule autorité
/// sur ce fait.
async fn ouvrir_session_par_spirit(
    db: &AsyncDatabase,
    base_url: &str,
    lire_env: SourceEnv<'_>,
    agent: &str,
    session_id: Option<&str>,
    canonical_session_id: Option<&str>,
) -> Result<(String, bool)> {
    // Avant toute construction de client, et sur les **deux** bras : le premier
    // tour part sur le fil de toute façon, et un refus qui attendrait le tour
    // nommerait le transport là où la cause est la configuration du shell.
    let jeton = jeton_interne(lire_env)?;

    let Some(id) = session_id else {
        let miroir = session_remote::ouvrir(base_url, &jeton, agent)
            .await
            .map_err(|probleme| {
                anyhow::anyhow!("La TUI n'a pas pu ouvrir sa session : {probleme}")
            })?;
        return Ok((miroir.session_id, miroir.canonical));
    };

    // Le bras `--session-id` : une reprise, et jamais une ouverture. La route
    // ne prend aucun identifiant dans son corps, donc elle ne sait pas créer
    // une session à l'identifiant que l'opérateur a choisi.
    match db.get_session(id).await {
        Ok(Some(existante)) if existante.agent_id != db.agent_id() => anyhow::bail!(
            "Session '{}' belongs to agent '{}', not '{}'",
            id,
            existante.agent_id,
            db.agent_id()
        ),
        Ok(Some(_)) => Ok((id.to_string(), canonical_session_id == Some(id))),
        Ok(None) => anyhow::bail!(
            "La session '{id}' est inconnue de cette base. La route d'ouverture \
             de mika-spirit ne prend aucun identifiant : elle ne sait pas créer \
             une session à identifiant choisi. Lance la TUI sans --session-id, \
             ou donne l'identifiant d'une session qui existe."
        ),
        // Cette erreur est **avalée** sur main (`if let Ok(Some(existing))`), et
        // la session était créée en local derrière. Une base illisible ne doit
        // pas se lire comme « session inconnue » : motif mika#2331, une
        // provenance fausse coûte plus cher qu'une provenance absente.
        Err(e) => anyhow::bail!(
            "La session '{id}' n'a pas pu être lue dans la base locale : {e}. \
             Tant que la base ne répond pas, la TUI ne sait pas à qui cette \
             session appartient et ne la reprend pas."
        ),
    }
}

/// Spawn the agent worker task. Returns the worker, channels, and context info needed for App.
async fn spawn_agent_worker(
    ctx: AppContext,
    agent_name: &str,
    http_client: &reqwest::Client,
    session_id: Option<&str>,
) -> Result<(
    AgentWorker,
    mpsc::UnboundedSender<AgentRequest>,
    mpsc::UnboundedReceiver<AgentResponse>,
    String, // session_id
    String, // model
    String, // identity_name
    Arc<SkillRegistry>,
    bool, // is_singleton_session (mika#1401)
)> {
    let identity = prompt::load_identity(&ctx.home_dir);
    if let Some(s) = session_id
        && s.is_empty()
    {
        anyhow::bail!("--session-id value must not be empty");
    }
    // Resolve the canonical session for singleton agents (mika#1401). An explicit
    // --session-id wins; otherwise a singleton agent reuses its one canonical
    // session on launch (and `/clear` preserves it — see App.is_singleton_session).
    let canonical_session_id =
        prompt::resolve_canonical_session_id(&identity, ctx.async_db.agent_id());
    // mika#1727 vol V2d-4a — la session de la TUI s'ouvre sur mika-spirit, ou
    // la TUI ne démarre pas. Les deux créations locales qui vivaient ici
    // n'avaient qu'un `tracing::warn!` sur échec : la TUI continuait sans
    // session, et chaque tour prenait la panne plus loin, là où elle ne se lit
    // plus. Le `?` remonte au site d'appel du démarrage (l. 543), avant
    // l'installation du terminal.
    let (session_id, is_singleton_session) = ouvrir_session_par_spirit(
        &ctx.async_db,
        &crate::commands::dashboard::spirit_url(),
        &|nom| std::env::var(nom).ok(),
        agent_name,
        session_id,
        canonical_session_id.as_deref(),
    )
    .await?;
    let mut tool_registry = tools::default_tools();
    for tool in tools::management_tools_if_needed(
        &ctx.global_home,
        &ctx.settings,
        reqwest::Client::new(),
        ctx.github_app.clone(),
    ) {
        tool_registry.register(tool);
    }
    let tool_registry = Arc::new(tool_registry);
    let skills_dir = ctx.home_dir.join("skills");
    // Migrate legacy .disabled marker files to DB (one-shot, idempotent).
    if let Err(e) = mika_agent::skills::migrate_disabled_markers_async(
        &skills_dir,
        &ctx.async_db,
        &ctx.async_db.agent_id,
    )
    .await
    {
        tracing::warn!(error = %e, "failed to migrate .disabled markers");
    }
    // Same assembler as the server init and as the two hot reloads below
    // (mika#2889) — the TUI no longer carries its own copy of the sequence.
    let overrides = ctx
        .async_db
        .get_skill_overrides(&ctx.async_db.agent_id)
        .await
        .unwrap_or_default();
    let skill_registry = Arc::new(assemble_agent_registry(
        &skills_dir,
        &identity,
        &overrides,
        ctx.async_db.agent_id(),
    ));
    let skills_dirty = Arc::new(AtomicBool::new(false));
    let embedding_client = ctx.settings.make_embedding_client();
    let message_sender = crate::init::make_message_sender(
        &ctx.settings,
        &ctx.async_db,
        http_client,
        ctx.async_db.agent_id(),
    );

    let brave_api_key = ctx
        .settings
        .brave_api_key
        .as_ref()
        .map(|s| s.expose_secret().to_string());
    let github_token = ctx.settings.agent_github_token().map(String::from);

    // Connect to MCP servers
    let mcp_manager = crate::init::connect_mcp(&ctx.home_dir).await;

    // mika#1962 — CLI has no `AgentState`, so the CLI process start IS the
    // init boundary for the tier. Resolve once here and thread the copy; do
    // not re-read the env per turn.
    let cli_tier = mika_common::home::AgentTier::from_env();
    // mika#2290 — same boundary, same reason: the CLI process start is where
    // the posed hosting fact is read, once.
    let cli_deployment = mika_common::home::Deployment::from_env();

    // mika#1727 vol V2a — la TUI ne construit plus de moteur de tâches. Le
    // `TaskDispatcher`, le `TaskEngine`, la boucle de tick, `startup_recovery`,
    // la purge des tâches anciennes et l'enregistrement des récurrentes
    // `heartbeat` / `reflection` vivaient ici ; ils étaient un second
    // exemplaire, divergent par construction, de ce que le daemon possède
    // déjà. Le daemon (`mika-agent/src/server/mod.rs`) est désormais le seul
    // enregistreur des récurrentes de l'agent, ce qui éteint la course de
    // mika#2351. Le détecteur qui refuse le retour du geste vit dans le module
    // `tests` de `commands/chat_turn.rs`.

    let (user_tx, mut user_rx) = mpsc::unbounded_channel::<AgentRequest>();
    let (agent_tx, agent_rx) = mpsc::unbounded_channel::<AgentResponse>();
    // Cloned for the supervisor task so a worker crash can surface to the TUI
    // after the worker's own `agent_tx` has been dropped (mika#1149).
    let supervisor_agent_tx = agent_tx.clone();

    let worker_db = ctx.async_db.clone();
    let mut worker_llm = ctx.llm.clone();
    let mut worker_settings = ctx.settings.clone();
    let worker_tools = tool_registry.clone();
    let mut worker_skills = skill_registry.clone();
    let worker_home = ctx.home_dir.clone();
    let mut worker_session = session_id.clone();
    let worker_embedding = embedding_client;
    let worker_brave_key = brave_api_key;
    let worker_github_token = github_token;
    let worker_gateway_url = ctx.settings.routing_url.clone();
    let worker_internal_token = ctx
        .settings
        .internal_token
        .as_ref()
        .map(|s| s.expose_secret().to_string());
    let worker_github_app = ctx.github_app.clone();
    let worker_sender = message_sender;
    let worker_dirty = skills_dirty.clone();
    let mut worker_mcp = mcp_manager;
    let worker_global_home = ctx.global_home.clone();
    // mika#1727 V1 — l'agent visé par l'endpoint A2A du tour. C'est le nom tel
    // que l'opérateur l'a lancé, le même que `commands::ask` met dans son URL.
    let worker_agent_name = agent_name.to_string();
    let handle = tokio::spawn(async move {
        while let Some(request) = user_rx.recv().await {
            match request {
                AgentRequest::Message {
                    text,
                    images,
                    thinking_budget,
                } => {
                    // Hot-reload skills if the dirty flag was set by a previous
                    // turn. A reload is an assembly, so it goes through the single
                    // assembler (mika#2889).
                    let mut skills_reloaded = false;
                    if worker_dirty.load(Ordering::Acquire) {
                        worker_dirty.store(false, Ordering::Release);
                        let reload_identity = mika_agent::prompt::load_identity(&worker_home);
                        let overrides = worker_db
                            .get_skill_overrides(&worker_db.agent_id)
                            .await
                            .unwrap_or_default();
                        worker_skills = Arc::new(assemble_agent_registry(
                            &worker_home.join("skills"),
                            &reload_identity,
                            &overrides,
                            &worker_db.agent_id,
                        ));
                        skills_reloaded = true;
                    }

                    // mika#1727 V1 § D3 — les entrées qui cessent d'atteindre le
                    // tour se disent à voix haute, une fois par tour, chacune
                    // nommant le geste qui l'éteint (mika#1883). L'avis est émis
                    // ici, dans le worker, parce que c'est l'unique point où
                    // atterrissent les DEUX expéditeurs d'`AgentRequest::Message`
                    // (la saisie de `tui/app.rs` et le coup unique de `/think`
                    // dans `tui/commands/handlers.rs`).
                    let dropped: Vec<chat_turn::DroppedImage> = images
                        .iter()
                        .map(|img| chat_turn::DroppedImage {
                            media_type: img.media_type.clone(),
                            payload_bytes: img.base64_data.len(),
                        })
                        .collect();
                    if let Some(notice) = chat_turn::inertia_notice(thinking_budget, &dropped)
                        && agent_tx
                            .send(AgentResponse::Notice { content: notice })
                            .is_err()
                    {
                        break;
                    }

                    // mika#1727 V1 § D1 — le tour part vers mika-spirit, qui
                    // possède la boucle, la session et les outils. Plus rien de ce
                    // bras ne fait tourner `agent::run_agent`.
                    let result =
                        chat_turn::send_turn_to_spirit(&text, &worker_agent_name, &worker_session)
                            .await;

                    // If the dirty flag is set after this turn, rebuild now so we
                    // can send the updated registry to the TUI for autocomplete.
                    if worker_dirty.load(Ordering::Acquire) {
                        worker_dirty.store(false, Ordering::Release);
                        let reload_identity2 = mika_agent::prompt::load_identity(&worker_home);
                        let overrides = worker_db
                            .get_skill_overrides(&worker_db.agent_id)
                            .await
                            .unwrap_or_default();
                        worker_skills = Arc::new(assemble_agent_registry(
                            &worker_home.join("skills"),
                            &reload_identity2,
                            &overrides,
                            &worker_db.agent_id,
                        ));
                        skills_reloaded = true;
                    }

                    let updated_skills = if skills_reloaded {
                        Some(worker_skills.clone())
                    } else {
                        None
                    };

                    let response = match result {
                        Ok(reply) => AgentResponse::Reply {
                            content: reply.content,
                            is_error: false,
                            // § D3 — un tour A2A synchrone ne rend aucun bloc de
                            // pensée : la part `thinking` ne voyage pas sur le
                            // fil. L'avis d'inertie ci-dessus l'a déjà dit.
                            thinking: None,
                            input_tokens: reply.input_tokens,
                            updated_skills,
                        },
                        Err(e) => AgentResponse::Reply {
                            content: format!("{e}"),
                            is_error: true,
                            thinking: None,
                            input_tokens: None,
                            updated_skills,
                        },
                    };
                    if agent_tx.send(response).is_err() {
                        break;
                    }
                }
                AgentRequest::CallbackResult {
                    task_id,
                    label,
                    result,
                    trace_id,
                    original_status,
                    parent_task_id,
                } => {
                    // Save the callback result as a tool_result message
                    let metadata = serde_json::json!({
                        "callback_task_id": task_id,
                        "label": label,
                        "parent_task_id": parent_task_id,
                    })
                    .to_string();
                    let _ = worker_db
                        .save_message_with_metadata(
                            &worker_session,
                            "tool_result",
                            &result,
                            Some(&metadata),
                            trace_id.as_deref(),
                            false,
                        )
                        .await;

                    // Build framing message for the agent
                    let is_failed = original_status == "failed";
                    let framing = agent::build_callback_trigger_context(
                        &label,
                        &task_id,
                        parent_task_id.as_deref(),
                        &result,
                        is_failed,
                    );

                    // mika#1727 V1 — chemin par le module, et non par un import de
                    // tête : le bras `Message` n'appelle plus rien de `agent`, et
                    // un import nu de ces deux noms survivrait au vol V2 qui
                    // emporte ce bras. Le corps du bras est inchangé par ailleurs.
                    let is_onboarding = agent::check_onboarding(&worker_db).await;
                    let callback_result = agent::run_agent(&agent::AgentParams {
                        db: &worker_db,
                        tier: cli_tier,
                        deployment: cli_deployment,
                        llm: worker_llm.as_ref(),
                        tools: &worker_tools,
                        skills: &worker_skills,
                        user_message: &framing,
                        channel_type: "cli",
                        session_id: &worker_session,
                        home_dir: &worker_home,
                        is_onboarding,
                        message_sender: worker_sender.clone(),
                        skip_compaction: false,
                        embedding_client: worker_embedding.as_ref(),
                        thinking: None,
                        user_images: &[],
                        brave_api_key: worker_brave_key.as_deref(),
                        github_token: worker_github_token.as_deref(),
                        gateway_url: worker_gateway_url.as_deref(),
                        internal_token: worker_internal_token.as_deref(),
                        github_app: worker_github_app.as_deref(),
                        skills_dirty: &worker_dirty,
                        skill_nudge: None, // mika#1583: nudges apply in server mode only
                        mcp_manager: worker_mcp.as_ref(),
                        global_home_dir: Some(&worker_global_home),
                        is_callback_turn: true,
                        settings: Some(&worker_settings),
                        // mika#2304: see the sibling construction above — the
                        // chat worker's provider already carries any `--model`.
                        caller_model_override: false,
                        session_isolated: false,
                        trace_id: trace_id.clone(),
                        correlated_task_id: None,
                        internal: false,
                        pr_reviews_posted: None, // CLI mode: no session-scoped dedup needed
                        // mika#3004: see the sibling construction above — a callback
                        // turn on the CLI path has no pre-handler webhook text either.
                        webhook_event_text: None,
                        stream_ctx: None, // A2A-only injection point (mika#1731 wire, mika#1757 emission)
                    })
                    .await;

                    let response = match callback_result {
                        Ok(output) => AgentResponse::Reply {
                            content: output.text.unwrap_or_default(),
                            is_error: false,
                            thinking: output.thinking,
                            input_tokens: output.usage.as_ref().map(|u| u.input_tokens as u32),
                            updated_skills: None,
                        },
                        Err(e) => {
                            // Unclaim the task so the next poll cycle can retry it.
                            // mark_task_delivered set status to 'delivered'; reset to original
                            // status so get_undelivered_callback_tasks will pick it up again.
                            if let Err(reset_err) = worker_db
                                .update_task_status(&task_id, &original_status)
                                .await
                            {
                                tracing::warn!(
                                    task_id = %task_id,
                                    error = %reset_err,
                                    "failed to unclaim callback task after agent error"
                                );
                            }
                            AgentResponse::Reply {
                                content: format!("Failed to process callback result: {e}"),
                                is_error: true,
                                thinking: None,
                                input_tokens: None,
                                updated_skills: None,
                            }
                        }
                    };
                    if agent_tx.send(response).is_err() {
                        break;
                    }
                }
                AgentRequest::SetModel { model } => {
                    // Recreate the LLM provider with the new model.
                    // Parse provider/model format if present, otherwise set on active provider.
                    let mut updated_settings = worker_settings.clone();
                    if let Some(slash_pos) = model.find('/') {
                        let prefix = &model[..slash_pos];
                        if let Ok(provider) = prefix.parse::<mika_common::llm::ProviderKind>() {
                            let model_name = model[slash_pos + 1..].to_string();
                            updated_settings.llm_provider = provider;
                            updated_settings.set_provider_model(provider, Some(model_name));
                        } else {
                            let provider = updated_settings.llm_provider;
                            updated_settings.set_provider_model(provider, Some(model.clone()));
                        }
                    } else {
                        let provider = updated_settings.llm_provider;
                        updated_settings.set_provider_model(provider, Some(model.clone()));
                    }
                    if let Ok(new_llm) = updated_settings.make_llm_provider() {
                        worker_llm = new_llm;
                        worker_settings = updated_settings;
                    }
                }
                AgentRequest::NewSession { session_id } => {
                    worker_session = session_id;
                }
                AgentRequest::Quit => break,
            }
        }
        // Gracefully shut down MCP server connections
        if let Some(mcp) = worker_mcp.take() {
            mcp.shutdown().await;
        }
    });

    let model = ctx.settings.active_model_display();
    let identity_name = identity.name.clone();

    // Supervise the worker task (mika#1149). The supervisor awaits `handle` and
    // forwards `JoinError` (panic) or premature `Ok(())` (user_rx closed while the
    // session is still active) as `AgentResponse::WorkerCrashed`. The chat loop sets
    // `shutdown_initiated = true` before tearing the worker down so expected exits
    // (Quit, agent switch, /restart) are silenced.
    let shutdown_initiated = Arc::new(AtomicBool::new(false));
    let supervisor_shutdown = shutdown_initiated.clone();
    let supervisor_agent_id = ctx.async_db.agent_id().to_string();
    let supervisor_session_id = session_id.clone();
    let supervisor_handle = tokio::spawn(async move {
        mika_cli::supervision::supervise(handle, supervisor_shutdown, move |failure| {
            tracing::error!(
                target: "mika::otel",
                event = "agent_worker_silenced",
                agent_id = %supervisor_agent_id,
                session_id = %supervisor_session_id,
                reason = %failure.reason,
                "TUI agent worker entered failure state; pending prompt dropped"
            );
            let _ = supervisor_agent_tx.send(AgentResponse::WorkerCrashed {
                reason: failure.reason,
            });
        })
        .await;
    });

    let worker = AgentWorker {
        supervisor_handle,
        shutdown_initiated,
        _ctx: ctx,
    };

    Ok((
        worker,
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        skill_registry,
        is_singleton_session,
    ))
}

pub async fn run(
    agent_name: &str,
    session: Option<&str>,
    model_override: Option<&str>,
    start_in_audit_mode: bool,
) -> Result<()> {
    let mut ctx = init::init_for_agent(agent_name)?;

    if let Some(model) = model_override {
        ctx.override_model(model)?;
    }

    let http_client = reqwest::Client::new();
    let (
        mut worker,
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        skill_registry,
        is_singleton_session,
    ) = spawn_agent_worker(ctx, agent_name, &http_client, session).await?;

    // Build app with shared resources
    let mut app = App::new(
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        worker._ctx.async_db.clone(),
        worker._ctx.llm.clone(),
        worker._ctx.home_dir.clone(),
        skill_registry,
        agent_name.to_string(),
        worker._ctx.global_home.clone(),
        worker._ctx.settings.llm_provider,
        start_in_audit_mode,  // audit mode launch (#773)
        is_singleton_session, // single-session-by-nature (mika#1401)
    );

    // La souscription au fil des tâches (mika#1727, vol V2b-2b-4). Le fil se
    // rouvre de lui-même, donc un daemon à terre au lancement n'est plus
    // définitif. Un jeton absent, lui, n'ouvre toujours rien : c'est le seul cas
    // où `callback_feed` reste `None`, et `poll_callback_tasks` couvre alors,
    // comme il couvre ce que le fil manque pendant une coupure. `run_team`
    // n'appelle pas ce chemin.
    if let Ok(token) = crate::commands::dashboard::auth_token() {
        app.callback_feed = Some(CallbackFeed::open_for(
            agent_name,
            crate::commands::dashboard::spirit_url(),
            token,
        ));
    }

    // Load recent conversation history so the user sees prior messages on restart.
    // In inbox mode (default), internal messages are filtered at the DB level.
    // The hidden count seeds the [N hidden] footer badge (mika#593).
    if let Ok((history, hidden_at_startup)) = worker
        ._ctx
        .async_db
        .load_recent_messages_filtered(20, app.inbox_mode)
        .await
    {
        for msg in &history {
            if let Some(chat_msg) = session_message_to_chat_message(msg) {
                app.messages.push(chat_msg);
            }
        }
        app.hidden_internal_count = hidden_at_startup;
    }

    // Load persisted thinking level ("off" resolves to None → stays default)
    app.load_thinking_level().await;

    // Surface skipped skills as a startup warning so the user knows
    let skipped_skills = app.skills.skipped();
    if !skipped_skills.is_empty() {
        use std::fmt::Write;
        let count = skipped_skills.len();
        let mut warning = format!("\u{26a0} {count} skill(s) skipped at startup:\n");
        let max_display = 5;
        for entry in skipped_skills.iter().take(max_display) {
            let _ = writeln!(warning, "  \u{2022} {}: {}", entry.name, entry.reason);
        }
        if count > max_display {
            let _ = writeln!(
                warning,
                "  ... and {} more. Run `mika skills validate` for details.",
                count - max_display
            );
        }
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: warning.trim_end().to_string(),
            rendered: None,
            channel: None,
        });
    }

    // Surface skills with validation warnings (#530)
    let validation_warnings = app.skills.validated_warnings();
    if !validation_warnings.is_empty() {
        use std::fmt::Write;
        let count = validation_warnings.len();
        let mut warning = format!("\u{26a0} {count} skill(s) loaded with validation warnings:\n");
        let max_display = 5;
        for entry in validation_warnings.iter().take(max_display) {
            // Show skill name + first diagnostic message for brevity
            let first_msg = entry
                .diagnostics
                .first()
                .map(|d| d.message.as_str())
                .unwrap_or("unknown issue");
            let _ = writeln!(warning, "  \u{2022} {}: {}", entry.skill_name, first_msg);
        }
        if count > max_display {
            let _ = writeln!(
                warning,
                "  ... and {} more. Run `mika skills validate` for details.",
                count - max_display
            );
        }
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: warning.trim_end().to_string(),
            rendered: None,
            channel: None,
        });
    }

    // Initialize cross-channel polling watermark
    app.last_seen_msg_id = worker._ctx.async_db.max_message_id().await.unwrap_or(0);

    // One-time dashboard status check at startup (no periodic polling).
    //
    // mika#1727 V1 § D4 — la même sonde répond à deux questions depuis que le
    // tour passe par mika-spirit : le tableau de bord est-il servi, et le spirit
    // répond-il **tout court**. `is_dashboard_running()` écrase les deux en un
    // `bool`, donc un daemon à terre y est indistinguable d'un tableau de bord
    // simplement désactivé. On lit l'option, et un `None` est dit AVANT le
    // premier tour plutôt que découvert par son échec.
    let dashboard_status = crate::commands::dashboard::query_dashboard_status().await;
    app.dashboard_running = dashboard_status.is_some_and(|(enabled, _, _)| enabled);
    if dashboard_status.is_none() {
        // La cause n'est pas résolue ici : la sonde rend `None` pour un daemon
        // injoignable comme pour un shell sans jeton, et en désigner une seule
        // serait une fausse provenance. Les deux sont nommées, et avec elles le
        // geste qui les lève.
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: format!(
                "\u{26a0} mika#1727 — mika-spirit n'a pas répondu sur {}. Depuis ce vol, \
                 chaque tour de la TUI y est envoyé : sans lui, le prochain message \
                 échouera en nommant l'endpoint. Démarre le daemon (`mika-spirit`), ou \
                 vérifie `MIKA_INTERNAL_TOKEN` dans ce shell — la sonde rend la même \
                 absence dans les deux cas.",
                crate::commands::dashboard::spirit_url()
            ),
            rendered: None,
            channel: None,
        });
    }

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Enter TUI mode
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Event reader (30ms tick rate for responsive progressive reveal)
    let mut events = EventReader::new(Duration::from_millis(30));

    // Main loop
    loop {
        if app.needs_redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            app.needs_redraw = false;
        }

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                input::handle_key(&mut app, key);
                app.needs_redraw = true;
            }
            Some(AppEvent::Mouse(mouse)) => {
                input::handle_mouse(&mut app, mouse);
                app.needs_redraw = true;
            }
            Some(AppEvent::Paste(text)) => {
                input::handle_paste(&mut app, &text);
                app.needs_redraw = true;
            }
            Some(AppEvent::Tick) => {
                app.tick().await;
            }
            Some(AppEvent::Resize) => {
                app.selection_state.clear();
                app.needs_redraw = true;
            }
            None => break,
        }

        // Handle agent switch
        if let Some(target_name) = app.pending_switch.take() {
            // End the old session before switching agents. No-op for singleton
            // agents — the canonical session is never ended (mika#1401).
            if !app.is_singleton_session
                && let Err(e) = worker._ctx.async_db.end_session(&app.session_id).await
            {
                tracing::warn!(error = %e, "failed to end session on agent switch");
            }

            // Silence the supervisor before tearing down the old worker — dropping
            // app.agent_tx + sending Quit would otherwise resolve as
            // `Ok(())` and trip the WorkerCrashed path (mika#1149).
            worker.shutdown_initiated.store(true, Ordering::Release);

            // mika#1150 cohort: send Quit BEFORE aborting so the worker's
            // post-loop `mcp.shutdown().await` runs on the common case
            // (worker is healthy, just switching agents). Abort is the
            // fallback if the worker is mid-LLM-call and the 2s drain
            // window expires.
            let _ = app.agent_tx.send(AgentRequest::Quit);

            let old_supervisor = std::mem::replace(
                &mut worker.supervisor_handle,
                tokio::spawn(async {}), // placeholder
            );
            if tokio::time::timeout(Duration::from_secs(2), old_supervisor)
                .await
                .is_err()
            {
                tracing::warn!(
                    target_agent = %target_name,
                    "old agent worker did not shut down within 2s during agent switch; \
                     supervisor task abandoned (mcp.shutdown handshake may have been skipped)"
                );
            }

            // Initialize the new agent
            match init::init_for_agent(&target_name) {
                Ok(new_ctx) => {
                    match spawn_agent_worker(new_ctx, &target_name, &http_client, None).await {
                        Ok((
                            new_worker,
                            new_tx,
                            new_rx,
                            new_session,
                            new_model,
                            new_identity,
                            new_skills,
                            new_is_singleton_session,
                        )) => {
                            // Update app fields
                            app.agent_tx = new_tx;
                            app.agent_rx = new_rx;
                            app.session_id = new_session;
                            // The target agent may have a different session profile
                            // (mika#1401): switch the singleton gate to match it.
                            app.is_singleton_session = new_is_singleton_session;
                            app.model = new_model.clone();
                            app.identity_name = new_identity;
                            app.db = new_worker._ctx.async_db.clone();
                            app.llm = new_worker._ctx.llm.clone();
                            app.home_dir = new_worker._ctx.home_dir.clone();
                            app.skills = new_skills;
                            app.agent_name = target_name.clone();
                            app.history = crate::tui::app::InputHistory::load(&app.home_dir);
                            // mika#1150 cohort: the new worker is healthy by
                            // construction; drop any residual lifecycle flags
                            // from the agent we just left so the operator's
                            // next /restart targets *this* agent rather than
                            // restarting healthy-agent-B because A had crashed.
                            app.worker_crashed = false;
                            app.pending_restart = false;
                            app.status = AgentStatus::Idle;

                            worker = new_worker;

                            // Load persisted thinking level from new agent's DB
                            app.load_thinking_level().await;

                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!(
                                    "Switched to agent '{target_name}' ({new_model})."
                                ),
                                rendered: None,
                                channel: None,
                            });
                        }
                        Err(e) => {
                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!("Failed to switch agent: {e}"),
                                rendered: None,
                                channel: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    app.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Failed to switch agent: {e}"),
                        rendered: None,
                        channel: None,
                    });
                }
            }
            app.scroll_offset = 0;
            app.needs_redraw = true;
        }

        // Handle /restart: tear down the crashed worker + supervisor and spin up a
        // fresh pair for the same agent (mika#1149 §S1.2). Reuses the agent-switch
        // shape but skips re-init for a different agent.
        if app.pending_restart {
            app.pending_restart = false;

            // End the old session before restarting so the dashboard shows it as
            // completed rather than ongoing. No-op for singleton agents — the
            // canonical session is never ended (mika#1401).
            if !app.is_singleton_session
                && let Err(e) = worker._ctx.async_db.end_session(&app.session_id).await
            {
                tracing::warn!(error = %e, "failed to end session on /restart");
            }

            worker.shutdown_initiated.store(true, Ordering::Release);
            let old_supervisor = std::mem::replace(
                &mut worker.supervisor_handle,
                tokio::spawn(async {}), // placeholder
            );
            if tokio::time::timeout(Duration::from_secs(2), old_supervisor)
                .await
                .is_err()
            {
                tracing::warn!(
                    "old agent worker did not shut down within 2s during /restart; \
                     supervisor task abandoned"
                );
            }

            match init::init_for_agent(agent_name) {
                Ok(new_ctx) => {
                    match spawn_agent_worker(new_ctx, agent_name, &http_client, None).await {
                        Ok((
                            new_worker,
                            new_tx,
                            new_rx,
                            new_session,
                            new_model,
                            _new_identity,
                            new_skills,
                            // Same agent restarted — the singleton gate is invariant.
                            _new_is_singleton_session,
                        )) => {
                            app.agent_tx = new_tx;
                            app.agent_rx = new_rx;
                            app.session_id = new_session;
                            app.model = new_model;
                            app.db = new_worker._ctx.async_db.clone();
                            app.llm = new_worker._ctx.llm.clone();
                            app.skills = new_skills;
                            app.worker_crashed = false;

                            worker = new_worker;

                            app.load_thinking_level().await;

                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content:
                                    "Agent worker restarted. In-flight callback caches were lost; pending background tasks will still deliver."
                                        .to_string(),
                                rendered: None,
                                channel: None,
                            });
                        }
                        Err(e) => {
                            // Worker stays absent; the WorkerCrashed banner remains.
                            app.messages.push(ChatMessage {
                                role: ChatRole::System,
                                content: format!("Failed to restart agent worker: {e}"),
                                rendered: None,
                                channel: None,
                            });
                        }
                    }
                }
                Err(e) => {
                    app.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: format!("Failed to restart agent worker: {e}"),
                        rendered: None,
                        channel: None,
                    });
                }
            }

            app.scroll_offset = 0;
            app.needs_redraw = true;
        }

        if app.should_quit {
            // Mark shutdown so the supervisor stays silent when the worker's
            // while-let loop exits via the Quit branch (mika#1149).
            worker.shutdown_initiated.store(true, Ordering::Release);
            let _ = app.agent_tx.send(AgentRequest::Quit);
            break;
        }
    }

    // Restore terminal
    restore_terminal()?;

    // Shut down event reader thread
    events.shutdown();

    // End the session so the dashboard shows duration instead of "ongoing".
    // No-op for singleton agents — the canonical session is never ended (mika#1401).
    if !app.is_singleton_session
        && let Err(e) = worker._ctx.async_db.end_session(&app.session_id).await
    {
        tracing::warn!(error = %e, "failed to end session");
    }

    // Wait for the supervisor (which owns the worker JoinHandle). It exits
    // silently because shutdown_initiated was set in the Quit branch above
    // (before agent_tx.send(Quit), which is the load-bearing ordering).
    if tokio::time::timeout(Duration::from_secs(2), worker.supervisor_handle)
        .await
        .is_err()
    {
        tracing::warn!("agent worker supervisor did not shut down within 2s; abandoning task");
    }

    // Database shutdown happens automatically via Drop on worker._ctx
    Ok(())
}

/// Run the TUI in team mode. Each user message is sent as a goal to `run_team()`.
pub async fn run_team(
    team_name: &str,
    global_home: &Path,
    run_id: Option<&str>,
    start_in_audit_mode: bool,
) -> Result<()> {
    let team_dir = team::team_dir(global_home, team_name);

    let (team_tx, mut team_rx_worker) = mpsc::unbounded_channel::<TeamRequest>();
    let (response_tx, response_rx) = mpsc::unbounded_channel::<TeamEvent>();

    // Use the shared container DB for team conversation persistence
    let db_path = mika_common::home::container_db_path(global_home);
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let team_db = mika_agent::async_db::AsyncDatabase::open(&db_path)?;

    // Spawn team worker
    let worker_home = global_home.to_path_buf();
    let worker_name = team_name.to_string();
    let worker_team_db = team_db.clone();
    let worker_run_id = run_id.map(|s| s.to_string());
    let team_worker_handle = tokio::spawn(async move {
        let settings = match Settings::load(&worker_home) {
            Ok(s) => s,
            Err(e) => {
                let _ = response_tx.send(TeamEvent::RunFailed(format!(
                    "Failed to load settings: {e}"
                )));
                return;
            }
        };

        let github_app = mika_common::github_app::GitHubApp::from_settings(&settings);
        while let Some(request) = team_rx_worker.recv().await {
            match request {
                TeamRequest::Goal(goal) => {
                    let tx = response_tx.clone();
                    let callback = move |event: TeamEvent| {
                        let _ = tx.send(event);
                    };

                    match mika_agent::teams::run_team(
                        &worker_name,
                        &goal,
                        &worker_home,
                        &settings,
                        Some(Box::new(callback)),
                        worker_team_db.clone(),
                        worker_run_id.as_deref(),
                        github_app.clone(),
                        None, // CLI: no AppState for session-scoped dedup (#821)
                        // mika#1962 — CLI process start is the tier init boundary.
                        mika_common::home::AgentTier::from_env(),
                        // mika#2290 — same boundary for the posed hosting fact.
                        mika_common::home::Deployment::from_env(),
                    )
                    .await
                    {
                        Ok(run) => {
                            // mika#1940 — same classifier as `mika ask --team`.
                            let event = match crate::commands::team_outcome::classify(&run) {
                                // Routing a failure to `RunFailed` is what makes
                                // the damage structurally impossible rather than
                                // merely forbidden: the TUI persists to the DB
                                // only on the `Deliverable` branch
                                // (`tui/app.rs`, `save_message("", "assistant",
                                // …)`). Before this, a `FailedNoDelegation` run
                                // carried `deliverable = Some(retry_reply)`, so
                                // a failed run's text became a durable
                                // `assistant` turn, re-read as context by every
                                // later turn — the only one of the three defect
                                // sites whose damage outlived the session.
                                TeamOutcome::Failed {
                                    diagnostic,
                                    partial,
                                } => {
                                    let mut msg = diagnostic;
                                    if let Some(partial) = partial {
                                        msg.push('\n');
                                        msg.push_str(
                                            crate::commands::team_outcome::PARTIAL_OUTPUT_MARKER,
                                        );
                                        msg.push('\n');
                                        msg.push_str(&partial);
                                    }
                                    TeamEvent::RunFailed(msg)
                                }
                                TeamOutcome::Delivered { text } => TeamEvent::Deliverable(text),
                                TeamOutcome::CompletedWithoutDeliverable => {
                                    TeamEvent::Deliverable(String::new())
                                }
                                // Current behaviour kept deliberately: the TUI
                                // answers an empty deliverable with "Team
                                // completed with no deliverable.", which is
                                // false for a suspended run (it did not
                                // complete, it is awaiting callbacks). Fixing
                                // that needs an extra `TeamEvent` variant whose
                                // fan-out touches both CLI callbacks, the TUI
                                // and the engine's emission sites —
                                // disproportionate for a p3 this is not about.
                                // `ask.rs`, which has no interface state
                                // machine, gets the honest note today; the
                                // asymmetry is deliberate and tracked
                                // (mika#1940, scope-out (b)).
                                TeamOutcome::Pending { .. } => {
                                    TeamEvent::Deliverable(String::new())
                                }
                            };
                            let _ = response_tx.send(event);
                        }
                        Err(e) => {
                            let _ = response_tx.send(TeamEvent::RunFailed(format!("{e}")));
                        }
                    }
                }
                TeamRequest::Quit => break,
            }
        }
    });

    // Load previous run context if a reference run was provided
    let (previous_run, run_warning) = if let Some(ref_id) = run_id {
        match team_db.get_team_run_summary(ref_id).await {
            Ok(Some(summary)) => {
                let warning = if summary.run.team_name != team_name {
                    tracing::warn!(
                        run_id = ref_id,
                        expected_team = team_name,
                        actual_team = summary.run.team_name,
                        "Referenced run belongs to a different team"
                    );
                    Some(format!(
                        "Warning: Referenced run belongs to team '{}', not '{}'",
                        summary.run.team_name, team_name
                    ))
                } else {
                    None
                };
                (Some(summary), warning)
            }
            Ok(None) => {
                tracing::warn!(run_id = ref_id, "Referenced team run not found in database");
                (
                    None,
                    Some(format!(
                        "Warning: Referenced run {} not found. Starting without previous run context.",
                        ref_id
                    )),
                )
            }
            Err(e) => {
                tracing::warn!(run_id = ref_id, error = %e, "Failed to load previous run summary");
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    // Build app in team mode
    let mut app = App::new_team(
        team_tx,
        response_rx,
        team_name,
        team_dir.clone(),
        global_home.to_path_buf(),
        team_db,
        previous_run,
        start_in_audit_mode, // audit mode launch (#773)
    );

    // Inject warning message if there was a run reference issue
    if let Some(warning) = run_warning {
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: warning,
            rendered: None,
            channel: None,
        });
    }

    // Team mode: skip history loading. Team conversations are goal-driven —
    // each goal triggers a full team run. The shared container DB contains
    // regular agent messages that should not appear in the team TUI.

    // One-time dashboard status check at startup (no periodic polling).
    app.dashboard_running = crate::commands::dashboard::is_dashboard_running().await;

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original_hook(info);
    }));

    // Enter TUI mode
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Event reader (30ms tick rate for responsive progressive reveal)
    let mut events = EventReader::new(Duration::from_millis(30));

    // Main loop
    loop {
        if app.needs_redraw {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            app.needs_redraw = false;
        }

        match events.next().await {
            Some(AppEvent::Key(key)) => {
                input::handle_key(&mut app, key);
                app.needs_redraw = true;
            }
            Some(AppEvent::Mouse(mouse)) => {
                input::handle_mouse(&mut app, mouse);
                app.needs_redraw = true;
            }
            Some(AppEvent::Paste(text)) => {
                input::handle_paste(&mut app, &text);
                app.needs_redraw = true;
            }
            Some(AppEvent::Tick) => {
                app.tick().await;
            }
            Some(AppEvent::Resize) => {
                app.selection_state.clear();
                app.needs_redraw = true;
            }
            None => break,
        }

        if app.should_quit {
            if let Some(ref tx) = app.team_tx {
                let _ = tx.send(TeamRequest::Quit);
            }
            break;
        }
    }

    // Shut down team DB (ensures pending writes complete)
    app.db.shutdown();

    // Restore terminal
    restore_terminal()?;

    // Shut down event reader thread
    events.shutdown();

    // Give team worker a brief window to shut down gracefully before aborting
    let _ = tokio::time::timeout(Duration::from_secs(2), team_worker_handle).await;

    Ok(())
}

fn restore_terminal() -> Result<()> {
    terminal::disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    Ok(())
}

// Les tests vivent ici, **en fin de fichier** : le balayeur de
// `mika_common::source_guard` coupe la production au premier `#[cfg(test)]`, et
// un item de production posé après en sortirait sans bruit.
#[cfg(test)]
mod tests {
    use super::*;
    // Banc **emprunté** au voisin, et non recopié : un second témoin de socket
    // ouvrirait une seconde vérité sur ce que le serveur reçoit, libre d'en
    // diverger. Il est `pub(crate)` depuis V2c-1b, pour ce motif-là.
    use crate::commands::rewind_remote::tests::fake_spirit;
    use axum::http::StatusCode;
    use mika_agent::db::Database;
    use mika_common::dead_endpoint::DeadEndpoint;

    /// Le chemin tel que le banc doit le voir : `fake_spirit` associe par le
    /// chemin **brut**. Un `format!` mal composé dans la porte tombe sur son
    /// `fallback` à 501, qu'aucun bras de `post` ne traduit.
    const ROUTE_SESSIONS: &str = "/api/v1/sessions";
    const CORPS_OUVERT: &str = r#"{"session_id":"s-du-fil","canonical":false}"#;

    /// La base de la TUI, en mémoire, comme `handlers.rs::test_app()` :
    /// `ouvrir_session_par_spirit` prend `&AsyncDatabase` et non `&AppContext`,
    /// donc la fixture n'a ni home, ni `Settings`, ni registre de skills.
    fn base_en_memoire() -> AsyncDatabase {
        AsyncDatabase::new(Database::open_in_memory().expect("une base en mémoire"))
    }

    /// Une source qui ne porte que `MIKA_INTERNAL_TOKEN`.
    fn jeton_pose(valeur: &'static str) -> impl Fn(&str) -> Option<String> {
        move |nom| (nom == "MIKA_INTERNAL_TOKEN").then(|| valeur.to_string())
    }

    #[tokio::test]
    async fn mika1727_v2d4a_sans_jeton_interne_le_demarrage_refuse_avant_tout_appel() {
        let db = base_en_memoire();
        // Le banc est **armé** et ne verra rien : c'est ce qui distingue
        // « refusé avant tout appel » de « refusé par le serveur ».
        let (base, recues) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, CORPS_OUVERT)]).await;

        let refus = ouvrir_session_par_spirit(&db, &base, &|_| None, "mika-dev", None, None)
            .await
            .expect_err("sans jeton, le démarrage refuse");

        let message = refus.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
        let vues = recues.lock().expect("le témoin du banc");
        assert_eq!(vues.len(), 0, "aucune requête ne doit partir");
    }

    #[tokio::test]
    async fn mika1727_v2d4a_un_jeton_de_tableau_de_bord_ne_suffit_pas() {
        let db = base_en_memoire();
        let (base, recues) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, CORPS_OUVERT)]).await;
        let tableau_seul =
            |nom: &str| (nom == "MIKA_DASHBOARD_TOKEN").then(|| "jeton-tableau".to_string());

        let refus = ouvrir_session_par_spirit(&db, &base, &tableau_seul, "mika-dev", None, None)
            .await
            .expect_err("le jeton du tableau de bord n'ouvre pas de session");

        let message = refus.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
        assert!(message.contains("MIKA_DASHBOARD_TOKEN"), "{message}");
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );

        // Contrôle positif, et pas une décoration : sans lui, un refus tombé
        // pour toute autre raison — une URL mal bâtie, un banc non armé —
        // rendrait ce test et le précédent verts en mesurant autre chose que la
        // lecture du jeton.
        let les_deux = |nom: &str| match nom {
            "MIKA_DASHBOARD_TOKEN" => Some("jeton-tableau".to_string()),
            "MIKA_INTERNAL_TOKEN" => Some("jeton-interne".to_string()),
            _ => None,
        };
        let (session_id, _) =
            ouvrir_session_par_spirit(&db, &base, &les_deux, "mika-dev", None, None)
                .await
                .expect("la même source, plus le jeton interne, ouvre");
        assert_eq!(session_id, "s-du-fil");
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            1,
            "une requête attendue"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4a_louverture_poste_sur_la_route_des_sessions_avec_son_jeton() {
        let db = base_en_memoire();
        let (base, recues) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, CORPS_OUVERT)]).await;

        let (session_id, _) = ouvrir_session_par_spirit(
            &db,
            &base,
            &jeton_pose("jeton-secret"),
            "mika-dev",
            None,
            None,
        )
        .await
        .expect("un 200 au corps lisible ouvre la session");

        // Les faits se lisent sur la socket et non sur l'appel : c'est ce que le
        // serveur reçoit qui tient le contrat, pas ce qu'on croit poster.
        let vues = recues.lock().expect("le témoin du banc");
        // Le compte sans rendu `Debug` : `RequeteRecue` n'en dérive pas, et le
        // banc du voisin reste intact dans ce vol.
        assert_eq!(vues.len(), 1, "un seul appel attendu");
        let vue = &vues[0];
        assert_eq!(vue.methode, "POST");
        assert_eq!(vue.chemin, ROUTE_SESSIONS);
        assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
        let envoye: serde_json::Value = serde_json::from_str(&vue.corps).expect("un corps JSON");
        assert_eq!(envoye["agent"], "mika-dev");

        // La session de la TUI est celle du fil, et non un UUID tiré ici : c'est
        // le geste que ce vol remplace.
        assert_eq!(session_id, "s-du-fil");
    }

    #[tokio::test]
    async fn mika1727_v2d4a_canonical_vrai_donne_une_session_singleton() {
        let db = base_en_memoire();
        let (base, _) = fake_spirit(&[(
            ROUTE_SESSIONS,
            StatusCode::OK,
            r#"{"session_id":"s-canonique","canonical":true}"#,
        )])
        .await;

        // `canonical_session_id` est volontairement `None` : si la TUI lisait
        // encore son identité locale pour ce booléen, ce test serait faux.
        let (session_id, is_singleton_session) =
            ouvrir_session_par_spirit(&db, &base, &jeton_pose("jeton"), "mika-dev", None, None)
                .await
                .expect("un 200 au corps lisible ouvre la session");

        assert_eq!(session_id, "s-canonique");
        assert!(
            is_singleton_session,
            "le `canonical` du fil est la seule autorité sur ce fait"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4a_canonical_faux_ne_donne_pas_de_singleton() {
        let db = base_en_memoire();
        let (base, _) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, CORPS_OUVERT)]).await;

        let (_, is_singleton_session) =
            ouvrir_session_par_spirit(&db, &base, &jeton_pose("jeton"), "mika-dev", None, None)
                .await
                .expect("un 200 au corps lisible ouvre la session");

        assert!(!is_singleton_session);
    }

    #[tokio::test]
    async fn mika1727_v2d4a_un_401_nomme_la_variable_a_poser() {
        let db = base_en_memoire();
        let (base, _) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::UNAUTHORIZED, "")]).await;

        let refus = ouvrir_session_par_spirit(
            &db,
            &base,
            &jeton_pose("mauvais-jeton"),
            "mika-dev",
            None,
            None,
        )
        .await
        .expect_err("un 401 arrête le démarrage");

        // Un jeton posé mais faux est la panne la plus coûteuse à diagnostiquer :
        // le message doit nommer la variable, et non le transport.
        let message = refus.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
    }

    #[tokio::test]
    async fn mika1727_v2d4a_un_point_injoignable_nomme_lurl() {
        let db = base_en_memoire();
        // mika#2569 : le port est **tenu** pour toute la durée de l'appel. Le
        // motif qui liait puis libérait a vu un factice voisin du même binaire
        // servir l'appel censé échouer.
        let dead = DeadEndpoint::reserve();
        let base = format!("http://{}", dead.addr());

        let refus =
            ouvrir_session_par_spirit(&db, &base, &jeton_pose("jeton"), "mika-dev", None, None)
                .await
                .expect_err("rien n'écoute : le démarrage doit s'arrêter");

        let message = refus.to_string();
        assert!(message.contains("unreachable"), "{message}");
        assert!(
            message.contains(&format!("{base}{ROUTE_SESSIONS}")),
            "{message}"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4a_une_session_inconnue_est_refusee_en_nommant_son_identifiant() {
        let db = base_en_memoire();
        let (base, recues) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, CORPS_OUVERT)]).await;

        let refus = ouvrir_session_par_spirit(
            &db,
            &base,
            &jeton_pose("jeton"),
            "mika-dev",
            Some("s-absente"),
            None,
        )
        .await
        .expect_err("une session inconnue ne se reprend pas");

        let message = refus.to_string();
        assert!(message.contains("s-absente"), "{message}");
        // Aucune ligne n'est écrite derrière le refus : la base reste vide, et le
        // banc n'a rien vu — la route ne sait pas créer une session à
        // identifiant choisi, donc le refus est la seule issue.
        assert!(
            db.get_session("s-absente")
                .await
                .expect("la base répond")
                .is_none(),
            "le refus ne doit créer aucune ligne"
        );
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4a_une_session_connue_est_reprise_sans_ouverture() {
        let db = base_en_memoire();
        db.create_session("s-connue", db.agent_id(), "cli")
            .await
            .expect("la fixture pose la session de l'agent");
        let (base, recues) = fake_spirit(&[(ROUTE_SESSIONS, StatusCode::OK, CORPS_OUVERT)]).await;

        let (session_id, is_singleton_session) = ouvrir_session_par_spirit(
            &db,
            &base,
            &jeton_pose("jeton"),
            "mika-dev",
            Some("s-connue"),
            None,
        )
        .await
        .expect("une session de l'agent se reprend");

        // Rendue telle quelle, et surtout : `ouvrir` n'est pas appelée. Si elle
        // l'était, le fil rendrait `s-du-fil` et l'opérateur perdrait la session
        // qu'il a nommée.
        assert_eq!(session_id, "s-connue");
        assert!(!is_singleton_session);
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            0,
            "une reprise n'ouvre rien sur le fil"
        );
    }

    /// mika#1727 V2d-4a — la création de session de la TUI est passée à
    /// `session_remote::ouvrir`. Cette garde refuse son retour en local dans la
    /// moitié *production* de ce fichier.
    ///
    /// Le scan porte sur les deux gestes qui *créent une session en base dans
    /// le processus de la TUI*, et sur rien d'autre : `end_session(` y reste
    /// jusqu'à V2d-4b, qui porte la porte `clore` et sa propre garde.
    #[test]
    fn mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local() {
        // Livrée vide, et vide par construction : quand la garde rougit, le
        // remède est la porte que son message nomme, jamais une entrée ici.
        const ALLOWED: &[&str] = &[];
        assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
        const MOTIFS: &[(&str, &str)] = &[
            ("create_session(", "session_remote::ouvrir"),
            ("get_or_create_canonical_session(", "session_remote::ouvrir"),
        ];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let site = scanner.src_root().join("commands/chat.rs");
        let production = scanner.production_of(&site);

        // Sans ce témoin, une garde qui balaie un chemin faux, ou un balayeur
        // qui masquerait le fichier entier, serait verte **par cécité** —
        // l'exact défaut que `ProductionScanner` existe pour refuser
        // (mika#2398). Le témoin vise un nom de la moitié *production* : s'il
        // disparaît, la garde rougit et demande qu'on la re-vise, au lieu de
        // continuer à ne rien regarder.
        assert!(
            production.contains("async fn spawn_agent_worker("),
            "la garde n'a pas lu la moitié production de chat.rs"
        );

        // Les deux motifs sont balayés avant le refus, et non refusés l'un après
        // l'autre : un `assert!` par motif n'aurait montré que le premier site,
        // et le rouge doit nommer tout ce qu'il y a à déplacer, pas une moitié.
        let violations: Vec<String> = MOTIFS
            .iter()
            .flat_map(|(motif, porte)| {
                production
                    .lines()
                    .enumerate()
                    .filter(move |(_, l)| !l.trim_start().starts_with("//") && l.contains(motif))
                    .map(move |(i, l)| format!("{}: {} (remède : {porte})", i + 1, l.trim()))
            })
            .collect();
        assert!(
            violations.is_empty(),
            "mika#1727 V2d-4a — la TUI crée sa session dans son propre processus. \
             Le remède est de passer par la porte que chaque ligne nomme, jamais \
             une entrée d'allowlist :\n{}",
            violations.join("\n")
        );
    }
}
