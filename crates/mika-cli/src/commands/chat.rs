use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::commands::chat_turn;
use crate::commands::session_remote;
use crate::commands::skills_remote::{self, EtatSkills};
use crate::commands::tasks::CallbackFeed;
use crate::commands::team_outcome::TeamOutcome;
use crate::init::{self, DbContext};
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
use mika_agent::teams::types::TeamEvent;
use mika_common::config::Settings;
use mika_common::team;
use std::path::Path;

/// Holds the agent worker supervisor handle, the DbContext (for DB shutdown),
/// and the shutdown-initiated flag the supervisor reads to distinguish operator-driven
/// shutdown from worker death (mika#1149).
struct AgentWorker {
    /// Supervisor task — owns the worker's JoinHandle and forwards unexpected exits
    /// as `AgentResponse::WorkerCrashed` events.
    supervisor_handle: JoinHandle<()>,
    /// Set to `true` before tearing down the worker (Quit, agent switch, /restart)
    /// so the supervisor exits silently on the resulting `Ok(())` from the worker loop.
    shutdown_initiated: Arc<AtomicBool>,
    _ctx: DbContext,
}

// La lecture stricte du jeton et sa source vivent chez les portes depuis
// V2d-4b (`commands/session_remote.rs`) : le démarrage la partage avec
// `/rewind` et `/clear`, et la copie locale qu'elle remplace nommait
// l'ouverture de session — en français, là où l'autre copie était en anglais.
// Les deux avaient déjà divergé ; c'était l'argument de leur mise en commun.
use crate::commands::session_remote::{SourceEnv, jeton_interne};

/// Ouvre — ou reprend — la session de la TUI (mika#1727, vol V2d-4a).
///
/// Tout ce qui touche le monde arrive par argument, donc un test pilote la
/// source d'environnement, la base et l'URL du banc sans construire le
/// contexte du démarrage : c'est la forme de `rewind_par_spirit`. Elle prend
/// `&AsyncDatabase` et non `&DbContext` pour la même raison.
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
    // `map_err` et non un `?` direct : la lecture commune rend un `String`,
    // parce qu'une variable absente n'a ni statut ni URL à porter.
    let jeton = jeton_interne(lire_env).map_err(anyhow::Error::msg)?;

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

/// Lit le registre de skills de l'agent sur mika-spirit (mika#1727, vol
/// V3c-2b-A1), là où la TUI l'assemblait sur le disque.
///
/// La forme d'[`ouvrir_session_par_spirit`] : tout ce qui touche le monde arrive
/// par argument, donc le test se joue contre un factice sans variable
/// d'environnement.
///
/// **Rien n'est fatal ici**, et c'est la différence avec l'ouverture de session
/// juste au-dessus : le retour est un [`EtatSkills`] et non un `Result` qu'un
/// `?` remonterait. Un registre illisible n'empêche pas de tenir une
/// conversation — les quatre surfaces de lecture disent la cause, et la TUI
/// démarre. Un jeton absent est un refus comme un autre pour cette raison : il
/// ne peut pas arrêter un démarrage que même un spirit à terre ne doit pas
/// arrêter.
async fn lire_les_skills(base_url: &str, lire_env: SourceEnv<'_>, agent: &str) -> EtatSkills {
    let jeton = jeton_interne(lire_env)?;
    skills_remote::lire(base_url, &jeton, agent)
        .await
        .map_err(|probleme| probleme.to_string())
}

/// Clôt la session du fil sur mika-spirit, au nom de l'agent qu'on quitte
/// (mika#1727, vol V2d-4c).
///
/// Même forme que `ouvrir_session_par_spirit` ci-dessus : l'URL de base et la
/// source d'environnement arrivent **par argument**, pour qu'un test les
/// pilote. Un client qui résout ses propres accès ne se pilote pas depuis un
/// test, et un chargement caché derrière le paramètre ferait mentir la
/// signature.
///
/// Le retour est `anyhow::Result<()>` et non `Result<_, SessionRemoteIssue>` :
/// les trois appelants sont dans [`run`], qui est en `anyhow`, et un type
/// d'erreur qui mélangerait « variable absente » et « spirit a répondu 401 »
/// n'est pas `SessionRemoteIssue` — une variable absente n'a ni statut ni URL,
/// et `Unexpected { status: None, .. }` se lirait « spirit injoignable » là où
/// rien n'a été tenté (motif mika#2331).
///
/// Le miroir est lu **ici** plutôt que rendu aux appelants : aucun des trois
/// sites n'a d'écran où porter la disjonction de mika#1401 — le dernier a déjà
/// rendu le terminal, les deux autres détruisent leur worker —, et la leur
/// rendre les obligerait à rejouer trois fois la même lecture sans
/// destinataire. `/clear` la rend à l'écran parce que `/clear` **a** un écran
/// (`tui/commands/handlers.rs::clear_par_spirit`).
async fn clore_session_de_sortie(
    base_url: &str,
    lire_env: SourceEnv<'_>,
    agent: &str,
    session_id: &str,
    is_singleton_session: bool,
) -> Result<()> {
    // Avant toute lecture du jeton : la session canonique d'un agent singleton
    // ne se clôt jamais (mika#1401), donc il n'y a rien à demander au fil et
    // aucune raison d'exiger une variable d'environnement pour ne rien faire.
    if is_singleton_session {
        return Ok(());
    }

    // `map_err` et non un `?` direct : la lecture commune rend un `String`,
    // pour la raison écrite sur `jeton_interne`. Le refus nomme
    // `MIKA_INTERNAL_TOKEN` parce que c'est elle qui le nomme.
    let jeton = jeton_interne(lire_env).map_err(anyhow::Error::msg)?;

    let miroir = session_remote::clore(base_url, &jeton, agent, session_id)
        .await
        .map_err(|probleme| anyhow::anyhow!("La session n'a pas pu être close : {probleme}"))?;

    // Les deux `ended: false` sont des 200, et aucun n'est un échec : rendre
    // l'un des deux en `Err` ferait dire au `warn!` de l'appelant qu'un geste a
    // échoué alors qu'il n'avait pas lieu d'être.
    if !miroir.ended {
        if miroir.canonical {
            tracing::warn!(
                session_id = %session_id,
                "mika-spirit n'a pas clos la session canonique de l'agent, ce qui est son \
                 régime nominal (mika#1401) : elle ne se clôt jamais"
            );
        } else {
            tracing::warn!(
                session_id = %session_id,
                "mika-spirit n'a clos aucune session et n'a pas dit pourquoi"
            );
        }
    }
    Ok(())
}

/// Spawn the agent worker task. Returns the worker, channels, and context info needed for App.
///
/// `model_override` est l'override de départ du worker (mika#1727, vol V3b-1) :
/// la valeur de `--model` telle que l'opérateur l'a tapée, ou `None`. Il n'est
/// pas lu dans `ctx` parce qu'un respawn — bascule d'agent ou `/restart` — doit
/// pouvoir repartir sans override alors que `ctx` en porte encore un.
async fn spawn_agent_worker(
    ctx: DbContext,
    agent_name: &str,
    session_id: Option<&str>,
    model_override: Option<String>,
) -> Result<(
    AgentWorker,
    mpsc::UnboundedSender<AgentRequest>,
    mpsc::UnboundedReceiver<AgentResponse>,
    String, // session_id
    String, // model
    String, // identity_name
    Arc<EtatSkills>,
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
    // mika#1727 vol V3c-2b-A1 — le registre des skills arrive par la route, et
    // non plus du disque. Les trois gestes qui vivaient ici — la migration des
    // marqueurs `.disabled`, la lecture des overrides, l'assemblage — étaient un
    // second exemplaire de ce que le serveur fait déjà à l'`init_agent` de
    // chaque agent (`mika-agent/src/server/mod.rs`), et la migration en
    // particulier n'est pas perdue : elle y est faite avant que cette route
    // puisse répondre.
    //
    // La lecture a lieu ici, donc à **chaque** respawn : la bascule d'agent et
    // `/restart` repassent par cette fonction, et le miroir qu'ils posent est
    // celui de l'agent qu'ils visent.
    let etat_skills = Arc::new(
        lire_les_skills(
            &crate::commands::dashboard::spirit_url(),
            &|nom| std::env::var(nom).ok(),
            agent_name,
        )
        .await,
    );
    // mika#1727 vol V3b-2 — la TUI ne connecte plus de serveur MCP. Elle en
    // montait un jeu au démarrage et le fermait en sortie, alors qu'aucun outil
    // de ce processus n'y touchait depuis V1 : le tour tourne chez spirit, qui
    // monte son propre `McpManager`.

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
    // mika#1727 vol V3b-1 — le modèle choisi vit ICI, dans le worker, et nulle
    // part ailleurs : c'est lui qui tient le fil des tours. La copie des
    // réglages qui occupait cette ligne ne servait qu'au bras `SetModel`, qui
    // validait un provider local et jetait le résultat ; plus aucun tour ne se
    // construit dans ce processus.
    // mika#1727 vol V3b-2 — ce qui s'affiche est ce qui part sur le fil :
    // afficher le modèle configuré alors qu'un `--model` le remplace pour tous
    // les tours serait une provenance fausse (mika#2331). La décision vit ici et
    // non dans `run` parce que les deux respawns passent par cette porte sans
    // override, et retombent donc sur le modèle configuré.
    let model = model_override
        .clone()
        .unwrap_or_else(|| ctx.settings.active_model_display());
    let mut worker_override = model_override;
    let mut worker_session = session_id.clone();
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
                    let result = chat_turn::send_turn_to_spirit(
                        &text,
                        &worker_agent_name,
                        &worker_session,
                        false,
                        worker_override.as_deref(),
                    )
                    .await;

                    let response = match result {
                        Ok(reply) => AgentResponse::Reply {
                            content: reply.content,
                            is_error: false,
                            // § D3 — un tour A2A synchrone ne rend aucun bloc de
                            // pensée : la part `thinking` ne voyage pas sur le
                            // fil. L'avis d'inertie ci-dessus l'a déjà dit.
                            thinking: None,
                            input_tokens: reply.input_tokens,
                        },
                        Err(e) => AgentResponse::Reply {
                            content: format!("{e}"),
                            is_error: true,
                            thinking: None,
                            input_tokens: None,
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

                    // mika#1727 vol V3a-2b — le tour part sur le fil, avec
                    // `mika.callback_turn` à `true`. C'était le dernier appel
                    // local de la boucle dans ce crate ; ce qu'il perd en
                    // chemin et pourquoi la remise de statut lui survit se lit
                    // sur `chat_turn::livrer_tour_de_callback`.
                    let response = chat_turn::livrer_tour_de_callback(
                        &worker_db,
                        &worker_agent_name,
                        &worker_session,
                        &framing,
                        &task_id,
                        &original_status,
                        worker_override.as_deref(),
                    )
                    .await;
                    if agent_tx.send(response).is_err() {
                        break;
                    }
                }
                AgentRequest::SetModel { model } => {
                    // mika#1727 vol V3b-1 — le bras pose l'override pour les
                    // tours suivants, et ne valide plus rien. La validation qui
                    // vivait ici construisait un provider local pour jeter son
                    // produit : elle ne pouvait prononcer que sur les clés et
                    // les alias de CE processus, et la TUI n'exécute plus aucun
                    // tour. C'est spirit qui tranche, qui a les clés, et qui
                    // refuse fermement un modèle qu'il ne sait pas servir
                    // (mika#2304) — le refus arrive donc au tour suivant, et il
                    // est visible, là où un abandon local était muet.
                    worker_override = chat_turn::override_de_set_model(&model);
                }
                AgentRequest::NewSession { session_id } => {
                    worker_session = session_id;
                }
                AgentRequest::Quit => break,
            }
        }
    });

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
        etat_skills,
        is_singleton_session,
    ))
}

/// Les skills que la route a dit ignorées, en un message de démarrage
/// (mika#1727, vol V3c-2b-A1).
///
/// Fonction pure sur `&EtatSkills` — **rien du monde en argument** —, comme ses
/// deux voisines ci-dessous. C'est cette extraction qui rend le texte mesurable
/// : en ligne dans [`run`], il ne se comparait qu'à travers un démarrage
/// complet, donc il ne se comparait pas. Le `format!` et les `writeln!` sont
/// ceux d'avant la bascule, déménagés sans se réécrire : à donnée égale, le
/// rendu est au caractère près celui d'hier.
///
/// `None` sur une lecture en échec : ce qu'il y a à dire alors est la cause, et
/// c'est [`message_de_cause`] qui la dit — une seule fois, plutôt qu'une fois
/// par surface.
fn message_des_skills_ignorees(etat: &EtatSkills) -> Option<String> {
    let skipped = &etat.as_ref().ok()?.skipped;
    if skipped.is_empty() {
        return None;
    }
    use std::fmt::Write;
    let count = skipped.len();
    let mut warning = format!("\u{26a0} {count} skill(s) skipped at startup:\n");
    let max_display = 5;
    for entry in skipped.iter().take(max_display) {
        let _ = writeln!(warning, "  \u{2022} {}: {}", entry.name, entry.reason);
    }
    if count > max_display {
        let _ = writeln!(
            warning,
            "  ... and {} more. Run `mika skills validate` for details.",
            count - max_display
        );
    }
    Some(warning.trim_end().to_string())
}

/// Les skills chargées avec un avertissement de validation, en un message de
/// démarrage (#530, porté sur le miroir au vol V3c-2b-A1).
///
/// **Le premier message de chaque skill**, comme avant la bascule : le registre
/// local portait des `diagnostics` dont on lisait `.first().message`, la route
/// sert des `messages` dont on lit `.first()`. Le fallback `"unknown issue"`
/// reste, et il reste atteignable — la route peut servir une liste vide, et
/// « absent n'est pas zéro » vaut aussi pour un texte (mika#2331).
fn message_des_avertissements(etat: &EtatSkills) -> Option<String> {
    let warnings = &etat.as_ref().ok()?.warnings;
    if warnings.is_empty() {
        return None;
    }
    use std::fmt::Write;
    let count = warnings.len();
    let mut warning = format!("\u{26a0} {count} skill(s) loaded with validation warnings:\n");
    let max_display = 5;
    for entry in warnings.iter().take(max_display) {
        // Show skill name + first diagnostic message for brevity
        let first_msg = entry
            .messages
            .first()
            .map(String::as_str)
            .unwrap_or("unknown issue");
        let _ = writeln!(warning, "  \u{2022} {}: {}", entry.name, first_msg);
    }
    if count > max_display {
        let _ = writeln!(
            warning,
            "  ... and {} more. Run `mika skills validate` for details.",
            count - max_display
        );
    }
    Some(warning.trim_end().to_string())
}

/// Pourquoi la TUI n'a pas pu lire le registre, en un message de démarrage
/// (mika#1727, vol V3c-2b-A1, critère 2).
///
/// La ligne de cause **telle qu'elle a été figée à la lecture**, sans
/// reformulation : elle nomme déjà l'URL et le geste de remédiation, et un
/// cadre ajouté ici dirait deux fois ce que les messages de
/// [`skills_remote::SkillsRemoteIssue`] disent une fois.
///
/// Sans ce message, un spirit à terre se lirait comme un agent sans skills —
/// la provenance fausse que mika#2331 refuse.
fn message_de_cause(etat: &EtatSkills) -> Option<String> {
    etat.as_ref().err().cloned()
}

pub async fn run(
    agent_name: &str,
    session: Option<&str>,
    model_override: Option<&str>,
    start_in_audit_mode: bool,
) -> Result<()> {
    let ctx = init::init_db_only_for_agent(agent_name)?;

    // mika#1727 vol V3b-2 — la validation de `--model` qui vivait ici bâtissait
    // un provider local pour le jeter, et n'avait d'avis que sur les clés et les
    // alias de CE processus (le pourquoi se lit sur `commands::ask::run`). Ce
    // qui reste local est la seule part indépendante du provider — une valeur
    // blanche — et son refus prend la forme et le message d'`ask.rs` : une
    // faute de frappe se lit au lancement, pas au premier message.
    if let Some(model) = model_override
        && model.trim().is_empty()
    {
        anyhow::bail!(crate::commands::ask::MODEL_MUST_NOT_BE_EMPTY);
    }

    // L'override de départ part au worker tel que l'opérateur l'a tapé, et non
    // tel que `ctx` l'a résolu : c'est spirit qui résout (mika#2304).
    let (
        mut worker,
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        etat_skills,
        is_singleton_session,
    ) = spawn_agent_worker(ctx, agent_name, session, model_override.map(String::from)).await?;

    // Build app with shared resources
    let mut app = App::new(
        user_tx,
        agent_rx,
        session_id,
        model,
        identity_name,
        worker._ctx.async_db.clone(),
        worker._ctx.home_dir.clone(),
        etat_skills,
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

    // Ce que l'état des skills a à dire au démarrage (mika#1727, vol V3c-2b-A1).
    // La cause **en premier** : sur un échec de lecture, elle est le seul des
    // trois messages qui ait quelque chose à dire, et elle explique l'absence
    // des deux autres.
    for message in [
        message_de_cause(&app.skills),
        message_des_skills_ignorees(&app.skills),
        message_des_avertissements(&app.skills),
    ]
    .into_iter()
    .flatten()
    {
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: message,
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
        // mika#1727 V2d-4a — le jeton n'est plus une cause possible à ce point du
        // démarrage : l'écran n'existe que si `ouvrir_session_par_spirit` a lu
        // `MIKA_INTERNAL_TOKEN`, et sur le chemin nominal spirit a répondu à
        // l'ouverture. Nommer encore la variable enverrait l'opérateur vérifier
        // sa configuration pour une panne survenue depuis — l'inverse de ce que
        // ce message doit faire.
        app.messages.push(ChatMessage {
            role: ChatRole::System,
            content: format!(
                "\u{26a0} mika#1727 — mika-spirit n'a pas répondu sur {} à la sonde du \
                 tableau de bord, alors que la session de la TUI s'y est ouverte. \
                 Chaque tour y est envoyé : s'il reste muet, le prochain message \
                 échouera en nommant l'endpoint. Vérifie que le daemon \
                 (`mika-spirit`) tourne toujours.",
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
            // La session du fil se clôt sur mika-spirit avant le changement
            // d'agent (mika#1727, vol V2d-4c). Le cas singleton est tranché
            // **dans** la clôture : le laisser aussi ici en ferait un second
            // juge du même fait.
            //
            // L'agent qu'on quitte est lu ici et non dans la clôture :
            // `app.agent_name` est réaffecté une soixantaine de lignes plus
            // bas, et une clôture au nom du nouvel agent clôrait une session
            // qui n'est pas la sienne. La capture ramène à deux lignes une
            // distance qu'aucun test de `run()` ne peut surveiller.
            let agent_quitte = app.agent_name.clone();
            if let Err(e) = clore_session_de_sortie(
                &crate::commands::dashboard::spirit_url(),
                &|nom| std::env::var(nom).ok(),
                &agent_quitte,
                &app.session_id,
                app.is_singleton_session,
            )
            .await
            {
                tracing::warn!(error = %e, agent = %agent_quitte,
                    "la session n'a pas été close sur spirit au changement d'agent");
            }

            // Silence the supervisor before tearing down the old worker — dropping
            // app.agent_tx + sending Quit would otherwise resolve as
            // `Ok(())` and trip the WorkerCrashed path (mika#1149).
            worker.shutdown_initiated.store(true, Ordering::Release);

            // mika#1150 cohort: send Quit BEFORE aborting so the worker's loop
            // ends by itself on the common case (worker is healthy, just
            // switching agents). Abort is the fallback if the worker is
            // mid-turn and the 2s drain window expires. The post-loop
            // `mcp.shutdown().await` this ordering also served left with MCP at
            // V3b-2; draining a turn in flight is reason enough on its own.
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
                     supervisor task abandoned"
                );
            }

            // Initialize the new agent
            match init::init_db_only_for_agent(&target_name) {
                Ok(new_ctx) => {
                    // mika#1727 — une bascule d'agent repart sans override : le
                    // modèle choisi l'avait été pour l'agent qu'on quitte, et le
                    // nouveau n'a pas forcément de quoi le servir. Le vol V3b-2
                    // ne porte pas sa survie : elle est hors périmètre, et ce
                    // bras retombe donc sur le modèle que `config.toml` déclare.
                    match spawn_agent_worker(new_ctx, &target_name, None, None).await {
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

            // La session se clôt sur mika-spirit avant le redémarrage, pour que
            // le dashboard la montre terminée plutôt qu'en cours (mika#1727,
            // vol V2d-4c). Un échec ne retient pas le `/restart` : il se
            // journalise, et le geste continue.
            if let Err(e) = clore_session_de_sortie(
                &crate::commands::dashboard::spirit_url(),
                &|nom| std::env::var(nom).ok(),
                &app.agent_name,
                &app.session_id,
                app.is_singleton_session,
            )
            .await
            {
                tracing::warn!(error = %e, "la session n'a pas été close sur spirit au /restart");
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

            match init::init_db_only_for_agent(agent_name) {
                Ok(new_ctx) => {
                    // mika#1727 — un `/restart` repart sans override, comme un
                    // démarrage sans `--model`. Le vol V3b-2 ne porte pas sa
                    // survie : elle est hors périmètre, nommément.
                    match spawn_agent_worker(new_ctx, agent_name, None, None).await {
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

    // La session se clôt sur mika-spirit, pour que le dashboard montre une
    // durée plutôt qu'« en cours » (mika#1727, vol V2d-4c). La sortie ne
    // dépend jamais de spirit : le terminal est déjà rendu et le lecteur
    // d'événements arrêté, donc un échec ne peut que se journaliser.
    if let Err(e) = clore_session_de_sortie(
        &crate::commands::dashboard::spirit_url(),
        &|nom| std::env::var(nom).ok(),
        &app.agent_name,
        &app.session_id,
        app.is_singleton_session,
    )
    .await
    {
        tracing::warn!(error = %e, "la session n'a pas été close sur spirit à la sortie");
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

    /// Le chemin de fin, écrit **en entier** et non composé depuis
    /// `ROUTE_SESSIONS` : le composer ici rejouerait le `format!` de la porte,
    /// et une faute dans celui-ci passerait inaperçue des deux côtés. Le banc
    /// associe par le chemin brut, donc un chemin faux tombe sur son `fallback`
    /// à 501 — qu'aucun bras de `post` ne traduit.
    const ROUTE_FIN: &str = "/api/v1/sessions/s-du-fil/end";
    const CORPS_CLOS: &str = r#"{"ended":true,"canonical":false}"#;

    /// La base de la TUI, en mémoire, comme `handlers.rs::test_app()` :
    /// `ouvrir_session_par_spirit` prend `&AsyncDatabase` et non le contexte,
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

    #[tokio::test]
    async fn mika1727_v2d4c_la_cloture_de_sortie_poste_sur_la_route_de_fin_avec_son_jeton() {
        let (base, recues) = fake_spirit(&[(ROUTE_FIN, StatusCode::OK, CORPS_CLOS)]).await;

        clore_session_de_sortie(
            &base,
            &jeton_pose("jeton-secret"),
            "agent-quitte",
            "s-du-fil",
            false,
        )
        .await
        .expect("un 200 au corps lisible clôt la session");

        // Les faits se lisent sur la socket et non sur l'appel : c'est ce que le
        // serveur reçoit qui tient le contrat, pas ce qu'on croit poster.
        let vues = recues.lock().expect("le témoin du banc");
        assert_eq!(vues.len(), 1, "un seul appel attendu");
        let vue = &vues[0];
        assert_eq!(vue.methode, "POST");
        assert_eq!(vue.chemin, ROUTE_FIN);
        assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
        let envoye: serde_json::Value = serde_json::from_str(&vue.corps).expect("un corps JSON");
        // L'agent **passé**, et c'est tout l'enjeu du critère 3 : au changement
        // d'agent, celui qui part sur le fil est l'agent quitté. Un corps qui
        // porterait l'autre clôrait une session qui n'est pas la sienne, et le
        // contrôle d'appartenance du serveur ne le rattraperait pas.
        assert_eq!(envoye["agent"], "agent-quitte");
    }

    #[tokio::test]
    async fn mika1727_v2d4c_un_singleton_ne_clot_rien_et_ne_lit_pas_le_jeton() {
        // Le banc est **armé** et ne verra rien.
        let (base, recues) = fake_spirit(&[(ROUTE_FIN, StatusCode::OK, CORPS_CLOS)]).await;

        // La source d'environnement est **vide**, et c'est le montage : avec un
        // jeton posé, ce test ne saurait pas distinguer « on n'a pas lu » de
        // « on a lu puis renoncé ». Vide, il atteste l'ordre du corps — le
        // singleton tranche avant toute lecture.
        clore_session_de_sortie(&base, &|_| None, "mika-dev", "s-du-fil", true)
            .await
            .expect("la canonique ne se clôt jamais, et ce n'est pas un échec");

        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4c_sans_jeton_la_cloture_refuse_avant_tout_appel_en_nommant_la_variable() {
        let (base, recues) = fake_spirit(&[(ROUTE_FIN, StatusCode::OK, CORPS_CLOS)]).await;

        let refus = clore_session_de_sortie(&base, &|_| None, "mika-dev", "s-du-fil", false)
            .await
            .expect_err("sans jeton, la clôture refuse");

        let message = refus.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );

        // Contrôle positif, et pas une décoration : sans lui, un refus tombé
        // pour toute autre raison — un chemin mal composé, un banc non armé —
        // rendrait ce test vert en mesurant autre chose que la lecture du jeton.
        clore_session_de_sortie(&base, &jeton_pose("jeton"), "mika-dev", "s-du-fil", false)
            .await
            .expect("le même montage, plus le jeton, atteint la socket");
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            1,
            "une requête attendue"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4c_un_401_est_rendu_en_erreur() {
        let (base, recues) = fake_spirit(&[(ROUTE_FIN, StatusCode::UNAUTHORIZED, "")]).await;

        let refus = clore_session_de_sortie(
            &base,
            &jeton_pose("mauvais-jeton"),
            "mika-dev",
            "s-du-fil",
            false,
        )
        .await
        .expect_err("un 401 est rendu à l'appelant");

        // Le message nomme la variable à poser parce que `Denied` la nomme : un
        // jeton posé mais faux est la panne la plus coûteuse à diagnostiquer.
        let message = refus.to_string();
        assert!(message.contains("MIKA_INTERNAL_TOKEN"), "{message}");
        // La socket **a** été touchée, et c'est ce qui sépare ce refus de celui
        // du test précédent : les deux rendent une erreur, un seul a appelé.
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            1,
            "une requête attendue"
        );
    }

    #[tokio::test]
    async fn mika1727_v2d4c_un_point_injoignable_est_rendu_en_erreur() {
        // mika#2569 : le port est **tenu** pour toute la durée de l'appel. Le
        // motif qui liait puis libérait a vu un factice voisin du même binaire
        // servir l'appel censé échouer.
        let dead = DeadEndpoint::reserve();
        let base = format!("http://{}", dead.addr());

        let refus =
            clore_session_de_sortie(&base, &jeton_pose("jeton"), "mika-dev", "s-du-fil", false)
                .await
                .expect_err("rien n'écoute : la clôture est rendue en erreur");

        let message = refus.to_string();
        assert!(message.contains("unreachable"), "{message}");
        assert!(message.contains(&format!("{base}{ROUTE_FIN}")), "{message}");
    }

    #[tokio::test]
    async fn mika1727_v2d4c_une_session_canonique_refusee_nest_pas_un_echec() {
        let (base, recues) = fake_spirit(&[(
            ROUTE_FIN,
            StatusCode::OK,
            r#"{"ended":false,"canonical":true}"#,
        )])
        .await;

        // `is_singleton_session` est **faux** ici : le serveur est la seule
        // autorité sur la canonicité, et le client peut l'ignorer — c'est le cas
        // d'une session reprise par `--session-id`. Un `ended: false` canonique
        // en 200 est le régime nominal de mika#1401 : la session ne se clôt
        // jamais, et le geste n'a pas échoué. Le rendre en `Err` ferait dire au
        // `warn!` de l'appelant qu'une sortie a mal tourné.
        clore_session_de_sortie(&base, &jeton_pose("jeton"), "mika-dev", "s-du-fil", false)
            .await
            .expect("un refus sur la canonique n'est pas un échec");

        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            1,
            "l'appel a bien eu lieu"
        );
    }

    // --- mika#1727 vol V3c-2b-A1 : la source des skills est la route ---

    /// Le chemin tel que le banc doit le voir, écrit **en entier** comme
    /// `ROUTE_SESSIONS` : le banc associe par le chemin brut, hors chaîne de
    /// requête, et un chemin faux tombe sur son `fallback` à 501.
    const ROUTE_SKILLS: &str = "/api/v1/skills";

    /// **Critère 4** — la lecture vise la route, avec son jeton et son agent.
    ///
    /// Les trois faits se mesurent sur la socket et non sur le rendu : un agent
    /// qui n'atteindrait pas la chaîne de requête donnerait le miroir d'un autre
    /// agent — plausible, et faux.
    #[tokio::test]
    async fn mika1727_v3c2b_la_lecture_des_skills_vise_la_route_avec_son_jeton() {
        let (base, recues) = fake_spirit(&[(
            ROUTE_SKILLS,
            StatusCode::OK,
            crate::commands::skills_remote::tests::CORPS,
        )])
        .await;

        let etat = lire_les_skills(&base, &jeton_pose("jeton-secret"), "mika-dev").await;

        let miroir = etat.expect("un 200 au corps lisible donne le miroir");
        assert_eq!(miroir.skills.len(), 2);

        let vues = recues.lock().expect("le témoin du banc");
        assert_eq!(vues.len(), 1, "un seul appel attendu");
        let vue = &vues[0];
        assert_eq!(vue.methode, "GET");
        assert_eq!(vue.chemin, ROUTE_SKILLS);
        assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-secret"));
        assert_eq!(vue.requete.as_deref(), Some("agent=mika-dev"));
    }

    /// **Critère 4** — sans jeton, la lecture refuse **avant tout appel**, et
    /// le refus est la cause que portera `app.skills`.
    #[tokio::test]
    async fn mika1727_v3c2b_sans_jeton_la_lecture_des_skills_refuse_avant_tout_appel() {
        // Le banc est **armé** et ne verra rien : c'est ce qui distingue
        // « refusé avant tout appel » de « refusé par le serveur ».
        let (base, recues) = fake_spirit(&[(
            ROUTE_SKILLS,
            StatusCode::OK,
            crate::commands::skills_remote::tests::CORPS,
        )])
        .await;

        let cause = lire_les_skills(&base, &|_| None, "mika-dev")
            .await
            .expect_err("sans jeton, la lecture refuse");

        assert!(cause.contains("MIKA_INTERNAL_TOKEN"), "{cause}");
        assert_eq!(
            recues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );
    }

    /// **Critère 4** — un refus de la route devient une cause **en une ligne**.
    ///
    /// L'assertion sur l'absence de `\n` n'est pas cosmétique : cette chaîne est
    /// poussée telle quelle dans un message `System` et rendue par `/skills`, où
    /// une cause multiligne déplacerait tout le rendu.
    #[tokio::test]
    async fn mika1727_v3c2b_un_refus_de_la_route_devient_une_cause_en_une_ligne() {
        let (base, _) = fake_spirit(&[(ROUTE_SKILLS, StatusCode::NOT_FOUND, "")]).await;

        let cause = lire_les_skills(&base, &jeton_pose("jeton"), "mika-dev")
            .await
            .expect_err("un 404 est un refus");

        // Un 404 sur cette route ne peut vouloir dire qu'agent inconnu, et le
        // message nomme l'agent **demandé**.
        assert!(cause.contains("mika-dev"), "{cause}");
        assert!(
            !cause.contains('\n'),
            "la cause tient sur une ligne : {cause}"
        );
    }

    /// **Critères 1 et 3** — les trois messages de démarrage, en littéral, sur
    /// le miroir de référence.
    ///
    /// Littéraux parce que ce sont des messages que l'opérateur lit au
    /// démarrage : le compte, la puce, le `:` séparateur et la troncature à cinq
    /// étaient ceux du registre local, et ils restent les mêmes à donnée égale.
    #[test]
    fn mika1727_v3c2b_les_messages_de_demarrage_rendent_le_miroir() {
        let etat: EtatSkills = Ok(crate::commands::skills_remote::tests::miroir_de_test());

        assert_eq!(
            message_des_skills_ignorees(&etat).as_deref(),
            Some(
                "\u{26a0} 1 skill(s) skipped at startup:\n  \u{2022} broken: manifest parse error"
            )
        );
        assert_eq!(
            message_des_avertissements(&etat).as_deref(),
            Some(
                "\u{26a0} 1 skill(s) loaded with validation warnings:\n  \u{2022} self-dev: two handlers share a keyword"
            )
        );
        // Un miroir lu n'a aucune cause à dire.
        assert_eq!(message_de_cause(&etat), None);
    }

    /// **Critères 2 et 3** — sur une lecture en échec, le démarrage dit la
    /// cause et **rien d'autre** ; sur un miroir lu et vide, il ne dit rien.
    ///
    /// Les deux moitiés tiennent dans un test parce que c'est leur
    /// complémentarité qui est la propriété : chacun des trois messages se tait
    /// exactement là où les deux autres parlent, et un message « 0 skill(s)
    /// skipped » sur une lecture qui n'a pas eu lieu serait la fausse provenance
    /// que mika#2331 refuse.
    #[test]
    fn mika1727_v3c2b_le_demarrage_dit_la_cause_et_jamais_un_compte_invente() {
        let echec: EtatSkills = Err("spirit injoignable".to_string());

        assert_eq!(
            message_de_cause(&echec).as_deref(),
            Some("spirit injoignable")
        );
        assert_eq!(message_des_skills_ignorees(&echec), None);
        assert_eq!(message_des_avertissements(&echec), None);

        let vide: EtatSkills = Ok(crate::commands::skills_remote::SkillsMirror::default());
        assert_eq!(message_de_cause(&vide), None);
        assert_eq!(message_des_skills_ignorees(&vide), None);
        assert_eq!(message_des_avertissements(&vide), None);
    }

    /// mika#1727 — la session de la TUI s'ouvre et se clôt sur mika-spirit.
    /// Cette garde refuse le retour en local des trois gestes correspondants
    /// dans la moitié *production* de ce fichier : la création est passée à
    /// `session_remote::ouvrir` au vol V2d-4a, la clôture à
    /// `session_remote::clore` au vol V2d-4c.
    ///
    /// Le nom de la garde n'a pas changé avec le second vol : c'est une garde
    /// **par fichier**, pas par vol, et la renommer casserait la trace des deux
    /// vols qui la citent.
    ///
    /// Le scan porte sur ces trois gestes et sur rien d'autre. `maybe_compact(`
    /// n'en est pas : `/compact` compacte encore en local, et une garde qui
    /// refuserait un geste que rien ne remplace ne laisserait d'autre issue
    /// qu'une entrée d'allowlist — exactement ce qu'elle existe pour éviter.
    #[test]
    fn mika1727_v2d4a_la_tui_ne_cree_plus_sa_session_en_local() {
        // Livrée vide, et vide par construction : quand la garde rougit, le
        // remède est la porte que son message nomme, jamais une entrée ici.
        const ALLOWED: &[&str] = &[];
        assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
        const MOTIFS: &[(&str, &str)] = &[
            ("create_session(", "session_remote::ouvrir"),
            ("get_or_create_canonical_session(", "session_remote::ouvrir"),
            ("end_session(", "session_remote::clore"),
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
        // Contre-témoin : `production_of` **masque** les régions `#[cfg(test)]`
        // sans jamais tronquer, donc la moitié test doit être absente alors que
        // tout ce qui la suit reste visible. Jusqu'à V2d-4c, aucun motif de
        // cette garde n'apparaissait dans la moitié test et un masquage perdu
        // y serait resté invisible ; les tests de la clôture de sortie
        // l'exercent désormais. Sans cette assertion, un masquage perdu ferait
        // rougir un *motif*, et on aurait « réparé » la garde par une entrée
        // d'allowlist. La cible est un nom de test antérieur à ce vol.
        assert!(
            !production.contains("fn mika1727_v2d4a_un_point_injoignable_nomme_lurl"),
            "la garde n'a pas masqué la moitié test de chat.rs"
        );

        // Les trois motifs sont balayés avant le refus, et non refusés l'un
        // après l'autre : un `assert!` par motif n'aurait montré que le premier
        // site, et le rouge doit nommer tout ce qu'il y a à déplacer, pas une
        // moitié.
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
            "mika#1727 — la TUI touche la table `sessions` dans son propre \
             processus. Le remède est de passer par la porte que chaque ligne \
             nomme, jamais une entrée d'allowlist :\n{}",
            violations.join("\n")
        );
    }

    /// **Critère 4 de V3b-1, étendu au critère 4 de V3b-2** — le tour de la TUI
    /// ne construit plus son provider, n'en garde plus de copie des réglages, et
    /// depuis V3b-2 ne porte plus ni contexte complet, ni MCP, ni rechargement
    /// de skills à chaud. **Critère 8 de V3c-2b-A1** : la TUI n'assemble plus
    /// ses skills sur le disque — ni registre, ni migration des marqueurs, ni
    /// lecture des overrides — et la réponse du worker ne porte plus de registre.
    ///
    /// **Son nom ne change pas avec le second vol** : c'est une garde par
    /// *fichier*, pas par vol — l'argument de la garde voisine ci-dessus — et la
    /// renommer casserait la trace des deux vols qui la citent. Le nom couvre
    /// moins bien `connect_mcp`, `McpManager` et `skills_dirty` que les trois
    /// autres motifs neufs, et cette tension est dite ici plutôt que réparée
    /// par un renommage qui coûterait la trace.
    ///
    /// La garde est **par fichier**, et non par crate, parce que
    /// `make_llm_provider` a quatre lecteurs légitimes ailleurs dans
    /// `mika-cli` — `init.rs`, `commands/provider.rs`, `commands/doctor.rs`,
    /// `commands/agents.rs` — qui construisent un provider pour autre chose
    /// qu'un tour de chat. Une garde par crate aurait demandé une allowlist de
    /// quatre entrées, et une allowlist peuplée est exactement ce qui ne refuse
    /// plus rien (mika#2309) : le cinquième lecteur s'y serait ajouté sans
    /// qu'un œil s'y arrête.
    #[test]
    fn mika1727_v3b1_le_tour_de_la_tui_ne_construit_plus_son_provider() {
        // Livrée vide, et vide par construction : quand la garde rougit, le
        // remède est la porte que son message nomme, jamais une entrée ici.
        const ALLOWED: &[&str] = &[];
        assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
        const MOTIFS: &[(&str, &str)] = &[
            (
                "worker_settings",
                "l'override vit dans le worker, pas dans une copie des réglages",
            ),
            (
                "make_llm_provider(",
                "le provider est construit par spirit, qui détient les clés",
            ),
            (
                "init_for_agent",
                "`init_db_only_for_agent` : aucun tour ici",
            ),
            ("AppContext", "`DbContext` : le worker ne lit que la base"),
            ("connect_mcp", "spirit monte son propre `McpManager`"),
            ("McpManager", "spirit monte son propre `McpManager`"),
            (
                "override_model(",
                "l'override voyage sous `mika.model_override`",
            ),
            (
                "skills_dirty",
                "plus rien ne marque un registre à réassembler : la TUI n'en assemble plus",
            ),
            (
                "assemble_agent_registry",
                "le registre arrive par la route, pas par le disque",
            ),
            (
                "migrate_disabled_markers_async",
                "spirit migre les marqueurs à l'`init_agent`",
            ),
            (
                "get_skill_overrides",
                "les overrides sont déjà appliqués dans le miroir",
            ),
            (
                "updated_skills",
                "plus de rechargement à chaud : la route est relue au respawn",
            ),
        ];
        // Deux sites depuis V3b-2, chacun avec SON témoin : un témoin générique
        // comme `pub async fn run(` passerait sur le mauvais fichier, et une
        // garde qui balaie un chemin faux — ou dont le masquage des régions de
        // test a dérapé — est verte par cécité (mika#2398). Celui d'`ask.rs` est
        // hors du périmètre de ce vol, donc sa disparition dit bien « la garde ne
        // vise plus le bon fichier » et non « le vol a bougé ».
        const SITES: &[(&str, &str)] = &[
            ("commands/chat.rs", "async fn spawn_agent_worker("),
            ("commands/ask.rs", "--task-id value must not be empty"),
        ];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let mut violations: Vec<String> = Vec::new();
        for (fichier, temoin) in SITES {
            let production = scanner.production_of(&scanner.src_root().join(fichier));
            assert!(
                production.contains(temoin),
                "la garde n'a pas lu la moitié production de {fichier}"
            );
            violations.extend(MOTIFS.iter().flat_map(|(motif, porte)| {
                production
                    .lines()
                    .enumerate()
                    .filter(move |(_, l)| !l.trim_start().starts_with("//") && l.contains(motif))
                    .map(move |(i, l)| {
                        format!("{fichier}:{}: {} (remède : {porte})", i + 1, l.trim())
                    })
            }));
        }

        // Contre-témoin sur `chat.rs` **seulement**. Il y est indispensable : la
        // table de motifs ci-dessus vit dans la moitié test de ce fichier et les
        // contient en clair, donc un masquage perdu la ferait se dénoncer
        // elle-même, et le réflexe serait de « réparer » par une entrée
        // d'allowlist — précisément le piège. Sur `ask.rs` il n'y a rien à
        // contre-témoigner : sa moitié test ne contient aucun des douze motifs,
        // un masquage perdu n'y produirait aucun rouge, et l'assertion y serait
        // du théâtre.
        let chat = scanner.production_of(&scanner.src_root().join("commands/chat.rs"));
        assert!(
            !chat.contains("fn mika1727_v2d4a_un_point_injoignable_nomme_lurl"),
            "la garde n'a pas masqué la moitié test de chat.rs"
        );

        assert!(
            violations.is_empty(),
            "mika#1727 vol V3b-1 — le tour de la TUI se remet à décider du \
             modèle dans son propre processus. Le remède est la porte que \
             chaque ligne nomme, jamais une entrée d'allowlist :\n{}",
            violations.join("\n")
        );
    }
}
