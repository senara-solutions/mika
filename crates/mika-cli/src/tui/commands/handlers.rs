use std::fmt::Write;

use mika_agent::config_keys::{is_settable_key, settable_keys_display, validate_config_value};
use mika_common::agent;
use mika_common::home;
use mika_common::team;

use crate::tui::app::{
    AgentRequest, AgentStatus, App, ChatMessage, ChatRole, MessagesLayout, SelectionState,
    session_message_to_chat_message,
};
use crate::tui::commands::{COMMANDS, parse_command, resolve_thinking_level};
use crate::tui::input;

/// Error message shown when the agent worker channel is closed.
const WORKER_NOT_RESPONDING: &str = "Error: Agent worker is not responding. Try /exit and restart.";

/// Commands allowed in team mode. New commands are blocked by default (safe failure mode).
const TEAM_MODE_ALLOWED_COMMANDS: &[&str] = &[
    "help", "h", "?", "clear", "exit", "quit", "q", "export", "agents", "teams", "team", "status",
    "stat", "verbose", "v",
];

/// Dispatch a slash command string to the appropriate handler.
/// Returns Some(output) for commands that produce output, None for commands like /exit.
pub async fn dispatch(app: &mut App<'_>, input: &str) -> Option<String> {
    let (cmd_name, args) = parse_command(input);

    // Gate agent-specific commands in team mode (allowlist — new commands blocked by default)
    if app.is_team_mode() && !TEAM_MODE_ALLOWED_COMMANDS.contains(&cmd_name) {
        return Some(format!("/{cmd_name} is not available in team mode."));
    }

    match cmd_name {
        "help" | "h" | "?" => Some(handle_help()),
        "clear" => Some(handle_clear(app, args).await),
        "exit" | "quit" | "q" => {
            app.should_quit = true;
            None
        }
        "compact" => Some(handle_compact(app).await),
        "memory" | "mem" => Some(handle_memory(app, args).await),
        "reminders" | "remind" => Some(handle_reminders(app).await),
        "tasks" => Some(handle_tasks(app, args).await),
        "status" | "stat" => {
            if app.is_team_mode() {
                Some(handle_team_info(app))
            } else {
                Some(handle_status(app).await)
            }
        }
        "soul" => Some(handle_soul(app).await),
        "config" | "cfg" => Some(handle_config(app, args).await),
        "model" => Some(handle_model(app, args).await),
        "provider" => Some(handle_provider(app, args).await),
        "export" => Some(handle_export(app).await),
        "skills" => Some(handle_skills(app)),
        "skill" => Some(handle_skill(app, args)),
        "switch" | "agent" => Some(handle_switch(app, args)),
        "agents" => Some(handle_agents(app)),
        "teams" => Some(handle_teams(app)),
        "team" => {
            if app.is_team_mode() {
                Some(handle_team_info(app))
            } else {
                Some(handle_team(args))
            }
        }
        "verbose" | "v" => Some(handle_verbose(app)),
        "inbox" => Some(handle_inbox(app).await),
        "think" | "t" => handle_think(app, args).await,
        "attach" | "img" => Some(handle_attach(app, args)),
        "undo" => Some(handle_undo(app).await),
        "rewind" => Some(handle_rewind(app, args).await),
        "restart" => Some(handle_restart(app)),
        _ => Some(format!(
            "Unknown command: /{cmd_name}. Type /help for available commands."
        )),
    }
}

/// Tear down the dead agent worker and spawn a fresh one (mika#1149).
/// Refuses on a healthy worker — `/clear` is the right tool for "start a new
/// session." The actual respawn is performed by the chat loop on the next
/// iteration once `pending_restart` is set.
fn handle_restart(app: &mut App<'_>) -> String {
    if !app.worker_crashed {
        return "/restart only applies after the worker has crashed. Use /clear to start a new session on a healthy worker.".to_string();
    }
    app.pending_restart = true;
    "Restarting agent worker… (the lost prompt is not replayed — please re-send it after the worker comes back up.)".to_string()
}

fn handle_help() -> String {
    let mut out = String::from("Available commands:\n");
    for cmd in COMMANDS {
        let aliases = if cmd.aliases.is_empty() {
            String::new()
        } else {
            format!(" ({})", cmd.aliases.join(", "))
        };
        let args = cmd.args_hint.unwrap_or("");
        let _ = writeln!(
            out,
            "  /{}{} {} — {}",
            cmd.name, aliases, args, cmd.description
        );
    }
    out
}

async fn handle_clear(app: &mut App<'_>, _args: &str) -> String {
    // Le `.env` de la home et l'URL de base, comme `handle_rewind_impl` : la
    // lecture du jeton prend sa source en paramètre pour qu'un test la pilote,
    // et un chargement caché derrière ce paramètre ferait mentir sa signature.
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    let base_url = crate::commands::dashboard::spirit_url();
    clear_par_spirit(app, &base_url, &|nom| std::env::var(nom).ok()).await
}

/// `/clear` tel que mika-spirit l'applique : la nouvelle session est **ouverte**
/// par la porte `ouvrir`, et l'ancienne **close** par la porte `clore`
/// (mika#1727, vol V2d-4b). Plus rien n'écrit la table `sessions` dans le
/// processus de la TUI, ce que la garde structurelle de ce fichier tient.
///
/// L'URL de base et la source d'environnement arrivent **par argument**, comme
/// [`rewind_par_spirit`] : c'est ce qui permet aux tests de viser un factice et
/// de lire ce qui est parti sur la socket, plutôt que de relire le code qui
/// l'écrit.
///
/// **`ouvrir` avant `clore`, et l'ordre est le livrable.** L'inverse perdrait
/// l'historique le jour où `ouvrir` échoue après un `clore` réussi : la TUI
/// n'aurait plus de session, et l'ancienne serait déjà close. Dans cet ordre,
/// un `ouvrir` qui échoue ne touche à rien.
async fn clear_par_spirit(app: &mut App<'_>, base_url: &str, lire_env: SourceEnv<'_>) -> String {
    use crate::commands::session_remote;

    // Singleton agents (mika#1401) are single-session-by-nature: the canonical
    // session must never be ended or replaced. `/clear` therefore resets only the
    // visual display — the session ID and the agent worker's loaded context are
    // preserved, and the DB history (the canonical thread) is left intact. This is
    // the TUI counterpart of `end_session_unless_canonical` on the CLI exit paths.
    //
    // Ce bras ne lit pas le jeton et n'appelle rien : il n'y a rien à demander
    // au serveur pour une session qu'on ne remplace pas.
    if app.is_singleton_session {
        app.messages.clear();
        app.scroll_offset = 0;
        // Suppress the just-cleared history from re-rendering on the next poll.
        app.last_seen_msg_id = app.db.max_message_id().await.unwrap_or(0);
        app.context_tokens = None;
        app.messages_layout = MessagesLayout::default();
        app.pending_response = None;
        app.reveal_index = 0;
        app.status = AgentStatus::Idle;
        app.pending_images.clear();
        app.pending_command = None;
        app.has_new_message = false;
        app.selection_state = SelectionState::None;
        app.pending_task_count = 0;
        app.hidden_internal_count = 0;
        app.needs_redraw = true;
        return "Display cleared. This agent is single-session-by-nature (mika#1401) — the canonical session is preserved.".to_string();
    }

    // Le jeton, **avant toute construction de client**. Un refus qui attendrait
    // l'appel nommerait le transport là où la cause est la configuration du
    // shell, et il tomberait sur un `ouvrir` qui aurait déjà inscrit une session
    // que plus personne ne reprendrait.
    let jeton = match jeton_interne(lire_env) {
        Ok(jeton) => jeton,
        Err(refus) => return refus,
    };

    // La nouvelle session. Sur un `Err`, rien n'est clos et rien ne change : ni
    // `session_id`, ni l'affichage, ni le worker. C'est la moitié de la raison
    // pour laquelle `ouvrir` passe avant `clore`.
    let ouverte = match session_remote::ouvrir(base_url, &jeton, &app.agent_name).await {
        Ok(miroir) => miroir,
        Err(probleme) => return format!("Error opening a new session: {probleme}"),
    };

    // L'ancienne, et l'appel part **inconditionnellement** : le serveur est
    // seul juge de ce qui est canonique (`server/sessions.rs:162-164`), donc une
    // TUI qui trancherait ici rejouerait une autorité qu'elle vient de lui
    // rendre. Le résultat n'est lu qu'après la remise à zéro : un échec de
    // clôture ne défait rien, il se **dit**.
    let ancienne = app.session_id.clone();
    let cloture = session_remote::clore(base_url, &jeton, &app.agent_name, &ancienne).await;

    // Notify the agent worker of the new session
    if app
        .agent_tx
        .send(AgentRequest::NewSession {
            session_id: ouverte.session_id.clone(),
        })
        .is_err()
    {
        return WORKER_NOT_RESPONDING.to_string();
    }

    // Drain any pending responses from the old session to prevent ghost messages
    while app.agent_rx.try_recv().is_ok() {}

    // Update app state — session
    app.session_id = ouverte.session_id;
    // Le serveur est seule autorité sur ce fait, comme au démarrage (V2d-4a) :
    // la route rend la session canonique de l'agent au lieu d'en créer une, donc
    // une TUI peut entrer non-singleton et sortir singleton.
    app.is_singleton_session = ouverte.canonical;
    app.messages.clear();
    app.scroll_offset = 0;
    app.last_seen_msg_id = app.db.max_message_id().await.unwrap_or(0);
    app.context_tokens = None;
    app.messages_layout = MessagesLayout::default();

    // Response state
    app.pending_response = None;
    app.reveal_index = 0;
    app.status = AgentStatus::Idle;

    // Input state
    app.pending_images.clear();
    app.pending_command = None;

    // UI state
    app.has_new_message = false;
    app.selection_state = SelectionState::None;
    app.pending_task_count = 0;
    app.hidden_internal_count = 0;

    app.needs_redraw = true;

    // Les **deux** champs du miroir sont lus, et c'est ce qui retire son
    // `expect(dead_code)` : `ended: false` seul ne dit pas si mika-spirit a
    // refusé de clore la canonique — un 200 normal (mika#1401) — ou s'il n'a
    // rien clos sans le dire.
    match cloture {
        Ok(miroir) if miroir.ended => "Chat cleared and session reset.".to_string(),
        Ok(miroir) if miroir.canonical => format!(
            "Chat cleared and session reset. The previous session ({ancienne}) is this \
             agent's canonical session (mika#1401) and was kept open."
        ),
        // mika-spirit n'a pas clos et n'a pas dit pourquoi : le message ne
        // l'invente pas (motif mika#2331 — une provenance fausse coûte plus
        // cher qu'une provenance absente).
        Ok(_) => format!(
            "Chat cleared and session reset. mika-spirit did not end the previous \
             session ({ancienne}) and gave no reason."
        ),
        // La nouvelle session est **gardée** : le worker l'a déjà reçue, et la
        // défaire coûterait l'affichage pour une cause qui n'est pas la sienne.
        Err(probleme) => format!(
            "Chat cleared and session reset, but the previous session ({ancienne}) was \
             not ended: {probleme}"
        ),
    }
}

async fn handle_compact(app: &App<'_>) -> String {
    // L'enveloppe de `handle_clear`, pour sa raison : le `.env` et l'URL de base
    // sont chargés ici, et la fonction déléguée les reçoit en argument — un
    // chargement caché derrière ce paramètre ferait mentir sa signature.
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    let base_url = crate::commands::dashboard::spirit_url();
    compact_par_spirit(app, &base_url, &|nom| std::env::var(nom).ok()).await
}

/// `/compact` tel que mika-spirit l'applique : la conversation est recomposée par
/// la porte `compacter` (mika#1727, vol V2d-4d). Plus rien ne compacte dans le
/// processus de la TUI, ce que la garde structurelle de ce fichier tient.
///
/// **`&App` et non `&mut App`**, à la différence de [`clear_par_spirit`] : cette
/// commande ne modifie aucun champ. La conversation est réécrite côté serveur, et
/// la TUI la relira au prochain poll comme n'importe quelle autre écriture de la
/// base. Un `&mut` que le corps n'utilise pas annoncerait une mutation dans la
/// signature et laisserait un appelant futur en glisser une sans que la revue la
/// voie.
///
/// **Aucun seuil n'est écrit ici.** Le serveur lit `COMPACTION_THRESHOLD`
/// (`server/compact.rs:93`) et rend le fait **et** le compte ; le client qui en
/// gardait une copie figée (le `50` écrit deux fois avant ce vol) refusait ou
/// acceptait au mauvais rang dès que la constante bougeait, et affichait un
/// dénominateur faux, en silence. Le message de `compacted: false` porte donc le
/// compte rendu et **aucun dénominateur** : en afficher un redemanderait au
/// client de connaître le seuil qu'il vient de cesser de connaître.
async fn compact_par_spirit(app: &App<'_>, base_url: &str, lire_env: SourceEnv<'_>) -> String {
    use crate::commands::session_remote;

    // Le refus **local**, et il n'est pas un doublon du 409 du serveur. Tant que
    // V1 n'est pas livré, le tour de la TUI tourne dans le processus de la TUI
    // (`chat.rs:341` et `chat.rs:468` appellent `agent::run_agent`), donc il ne
    // détient **pas** le verrou d'agent que `/api/v1/compact` tente de prendre :
    // un `/compact` lancé pendant ce tour obtiendrait un 200, et la compaction
    // remplacerait par un résumé les messages que le tour local est en train de
    // relire. De ce fait-là, ce refus est le seul juge, et il le restera jusqu'à
    // V1.
    if app.status != AgentStatus::Idle {
        return "Cannot compact while agent is busy.".to_string();
    }

    // Le jeton, **avant tout appel**, pour la raison de `clear_par_spirit` : un
    // refus qui attendrait l'appel nommerait le transport là où la cause est la
    // configuration du shell. La lecture reste unique dans le crate.
    let jeton = match jeton_interne(lire_env) {
        Ok(jeton) => jeton,
        Err(refus) => return refus,
    };

    match session_remote::compacter(base_url, &jeton, &app.agent_name).await {
        // Les deux booléens sont des 200, et `messages` est le compte **d'avant**
        // dans les deux cas.
        Ok(miroir) if !miroir.compacted => {
            format!("Nothing to compact ({} messages).", miroir.messages)
        }
        // « demandée », jamais « faite » : la porte déclare que `compacted` veut
        // dire « la compaction a été demandée et n'a pas échoué », et le serveur
        // ne dit nulle part qu'une ligne `summary` a été écrite — `maybe_compact`
        // relit le seuil elle-même et peut légitimement n'avoir rien fait.
        // Affirmer « Compacted conversation », ce que disait le code d'avant,
        // attesterait un état du système que rien n'a confirmé.
        Ok(miroir) => format!(
            "Compaction requested ({} messages before). mika-spirit does not report \
             whether a summary was written.",
            miroir.messages
        ),
        // Le `Display` de `Busy` dit la cause (« another turn holds its lock ») et
        // non le remède. Devant une TUI c'est le remède qui décide de la seconde
        // qui suit, et dire que l'occupant n'est pas son propre tour évite
        // d'envoyer l'opérateur attendre la fin d'un tour qui n'existe pas — le
        // refus local ci-dessus aurait déjà intercepté ce cas-là.
        Err(session_remote::SessionRemoteIssue::Busy) => {
            "mika-spirit is busy with this agent — another client holds its turn lock. \
             Try /compact again when it is idle."
                .to_string()
        }
        // Tout le reste porte le `Display` de la porte, déjà écrit pour nommer sa
        // cause sans en inventer une (motif mika#2331).
        Err(probleme) => format!("Compaction failed: {probleme}"),
    }
}

async fn handle_memory(app: &mut App<'_>, args: &str) -> String {
    if args.starts_with("search") {
        let query = args.strip_prefix("search").unwrap_or("").trim();
        if query.is_empty() {
            return "Usage: /memory search <query>".to_string();
        }
        return handle_memory_search(app, query).await;
    }

    // Show core memory blocks
    match app.db.get_all_core_memory().await {
        Ok(entries) => {
            if entries.is_empty() {
                return "No core memory entries.".to_string();
            }
            let mut out = String::from("Core Memory:\n");
            for entry in &entries {
                let _ = writeln!(
                    out,
                    "  [{}] {} ({} tokens)",
                    entry.key, entry.value, entry.token_count
                );
            }
            if let Ok(total) = app.db.total_core_memory_tokens().await {
                let _ = writeln!(out, "\nTotal: {total}/2000 tokens");
            }
            out
        }
        Err(e) => format!("Failed to load core memory: {e}"),
    }
}

async fn handle_memory_search(app: &mut App<'_>, query: &str) -> String {
    let mut out = String::new();
    let mut found = false;

    // Search across all memory layers concurrently
    let (people, commitments, preferences, events) = tokio::join!(
        app.db.search_people(query),
        app.db.search_commitments(query),
        app.db.search_preferences(query),
        app.db.search_events(query),
    );

    if let Ok(people) = people
        && !people.is_empty()
    {
        found = true;
        let _ = writeln!(out, "People:");
        for p in &people {
            let rel = p.relationship.as_deref().unwrap_or("unknown");
            let _ = writeln!(out, "  {} ({})", p.canonical_name, rel);
        }
    }
    if let Ok(commitments) = commitments
        && !commitments.is_empty()
    {
        found = true;
        let _ = writeln!(out, "Commitments:");
        for c in &commitments {
            let due = c.due_date.as_deref().unwrap_or("no due date");
            let _ = writeln!(out, "  [{}] {} ({})", c.status, c.description, due);
        }
    }
    if let Ok(preferences) = preferences
        && !preferences.is_empty()
    {
        found = true;
        let _ = writeln!(out, "Preferences:");
        for p in &preferences {
            let _ = writeln!(out, "  {}: {}", p.category, p.value);
        }
    }
    if let Ok(events) = events
        && !events.is_empty()
    {
        found = true;
        let _ = writeln!(out, "Events:");
        for e in &events {
            let date = e.event_date.as_deref().unwrap_or("no date");
            let _ = writeln!(out, "  {} ({})", e.description, date);
        }
    }

    if !found {
        format!("No results for '{query}'.")
    } else {
        out
    }
}

async fn handle_reminders(app: &mut App<'_>) -> String {
    let tasks = match app.db.get_user_visible_tasks().await {
        Ok(t) => t,
        Err(_) => return "Failed to load reminders.".to_string(),
    };

    if tasks.is_empty() {
        return "No active reminders.".to_string();
    }

    let mut out = String::new();
    let _ = writeln!(out, "Active reminders:");
    for t in &tasks {
        let short_id = &t.id[..12.min(t.id.len())];
        let fire_at = t
            .next_fire_at
            .as_ref()
            .map(|s| mika_agent::db::format_ts(s))
            .unwrap_or_else(|| "unknown".to_string());
        let _ = writeln!(out, "  {}: {} (fires: {})", short_id, t.label, fire_at);
    }
    out
}

async fn handle_tasks(app: &mut App<'_>, args: &str) -> String {
    if let Some(id) = args.strip_prefix("cancel").map(|s| s.trim()) {
        if id.is_empty() {
            return "Usage: /tasks cancel <id>".to_string();
        }
        return match app.db.cancel_task(id).await {
            Ok(true) => format!("Cancelled task {id}."),
            Ok(false) => format!("Task {id} not found or already completed."),
            Err(e) => format!("Failed to cancel task: {e}"),
        };
    }

    let tasks = match app.db.get_schedulable_tasks().await {
        Ok(t) => t,
        Err(e) => return format!("Failed to load tasks: {e}"),
    };

    let pending: Vec<_> = tasks
        .iter()
        .filter(|t| t.status == "pending" || t.status == "in_progress")
        .collect();

    if pending.is_empty() {
        return "No pending tasks.".to_string();
    }

    let mut out = String::new();
    let _ = writeln!(out, "Pending tasks ({}):", pending.len());
    for t in &pending {
        let short_id = &t.id[..12.min(t.id.len())];
        let when = t
            .next_fire_at
            .as_ref()
            .map(|s| mika_agent::db::format_ts(s))
            .unwrap_or_else(|| t.trigger_type.clone());
        let _ = writeln!(
            out,
            "  {}: [{}] \"{}\" ({})",
            short_id, t.action_type, t.label, when
        );
    }
    out.push_str("  Use /tasks cancel <id> to cancel.");
    out
}

async fn handle_status(app: &mut App<'_>) -> String {
    let mut out = String::from("Status:\n");

    // Run all DB queries concurrently
    let (count, size, tokens, version) = tokio::join!(
        app.db.count_messages(),
        app.db.db_size_bytes(),
        app.db.total_core_memory_tokens(),
        app.db.schema_version(),
    );

    if let Ok(count) = count {
        let _ = writeln!(out, "  Messages: {count}");
    }
    if let Ok(size) = size {
        let size_kb = size / 1024;
        let _ = writeln!(out, "  DB size: {size_kb} KB");
    }
    if let Ok(tokens) = tokens {
        let _ = writeln!(out, "  Core memory: {tokens}/2000 tokens");
    }
    if let Ok(version) = version {
        let _ = writeln!(out, "  Schema: v{version}");
    }
    let _ = writeln!(out, "  Model: {}", app.model);
    let _ = writeln!(
        out,
        "  Session: {}",
        &app.session_id[..8.min(app.session_id.len())]
    );

    out
}

async fn handle_soul(app: &mut App<'_>) -> String {
    let soul_path = app.home_dir.join("soul.md");
    match tokio::fs::read_to_string(&soul_path).await {
        Ok(content) if !content.trim().is_empty() => content,
        Ok(_) => "soul.md is empty.".to_string(),
        Err(_) => "No soul.md found. Create one at ~/.mika/soul.md".to_string(),
    }
}

async fn handle_config(app: &mut App<'_>, args: &str) -> String {
    if let Some(rest) = args.strip_prefix("set") {
        return handle_config_set(app, rest.trim()).await;
    }

    let config_path = app.home_dir.join("config").join("local.toml");

    let mut out = String::from("Configuration:\n");
    let _ = writeln!(out, "  Model: {}", app.model);
    let _ = writeln!(out, "  Home: {}", app.home_dir.display());
    let _ = writeln!(
        out,
        "  Session: {}",
        &app.session_id[..8.min(app.session_id.len())]
    );

    // Show config file path without dumping contents (may contain secrets)
    if tokio::fs::metadata(&config_path).await.is_ok() {
        let _ = writeln!(out, "  Config file: {}", config_path.display());
    } else {
        let _ = writeln!(out, "  Config file: (using defaults)");
    }

    // Show customer config entries
    if let Ok(configs) = app.db.list_customer_config().await
        && !configs.is_empty()
    {
        let _ = writeln!(out);
        let _ = writeln!(out, "Customer settings:");
        for (key, value) in &configs {
            let _ = writeln!(out, "  {key}: {value}");
        }
    }

    out
}

async fn handle_config_set(app: &mut App<'_>, args: &str) -> String {
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].trim().is_empty() {
        return format!(
            "Usage: /config set <key> <value>\nSettable keys: {}",
            settable_keys_display()
        );
    }
    let key = parts[0];
    let value = parts[1].trim();

    if !is_settable_key(key) {
        return format!(
            "Unknown config key: {key}\nSettable keys: {}",
            settable_keys_display()
        );
    }

    if let Err(msg) = validate_config_value(key, value) {
        return msg;
    }

    match app.db.set_customer_config(key, value).await {
        Ok(()) => format!("Set {key} = {value}"),
        Err(e) => format!("Failed to set {key}: {e}"),
    }
}

#[cfg(test)]
fn resolve_model_name(input: &str) -> Option<(&'static str, &'static str)> {
    let lower = input.to_lowercase();
    for &(alias, full_id, display) in crate::cli::MODEL_ALIASES {
        if lower == alias || lower == full_id {
            return Some((full_id, display));
        }
    }
    None
}

async fn handle_model(app: &mut App<'_>, args: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        match crate::commands::model::list_models(&app.global_home, &app.home_dir).await {
            Ok(result) => {
                let mut out = format!(
                    "Current model: {}\n\nAvailable models for {}:",
                    result.current_model, result.provider
                );
                if result.models.is_empty() {
                    let _ = write!(
                        out,
                        "\n  (no models available \u{2014} type a model name directly)"
                    );
                } else {
                    for m in &result.models {
                        let marker = if m.current { " (current)" } else { "" };
                        let _ = write!(out, "\n  {}{}", m.id, marker);
                    }
                }
                let _ = write!(out, "\n\nAliases:");
                for a in &result.aliases {
                    let _ = write!(out, "\n  {} \u{2014} {}", a.alias, a.display);
                }
                let _ = write!(out, "\n\nUsage: /model <name> or /model <alias>");
                out
            }
            Err(e) => format!("Error: {e}"),
        }
    } else {
        match crate::commands::model::switch_model(&app.global_home, &app.home_dir, args) {
            Ok(result) => {
                let target_provider: mika_common::llm::ProviderKind =
                    result.provider.parse().unwrap_or(app.provider);
                // mika#1727 vol V3b-2 — `app.provider` ne bouge pas, même quand
                // `switch_model` a changé de provider dans `config.toml` : un
                // override ne re-dispatche jamais vers un autre provider
                // (mika#1591), donc les tours de cette TUI restent servis par
                // celui que mika-spirit a chargé. Le modèle, lui, part bien :
                // `format_worker_model` rend la valeur que le fil porte.
                let display_id =
                    crate::commands::provider::format_display_model(target_provider, &result.model);
                let worker_model =
                    crate::commands::provider::format_worker_model(target_provider, &result.model);
                app.model = display_id.clone();
                if app
                    .agent_tx
                    .send(AgentRequest::SetModel {
                        model: worker_model,
                    })
                    .is_err()
                {
                    return WORKER_NOT_RESPONDING.to_string();
                }
                app.needs_redraw = true;

                let mut out = format!("Switched to {display_id}.");
                if result.provider_changed {
                    out.push_str(&format!(
                        " (switched provider to {} in config.toml \u{2014} mika-spirit takes it \
                         at its next restart, for every turn of the agent)",
                        result.provider
                    ));
                }
                for w in &result.warnings {
                    out.push_str(&format!(" (warning: {w})"));
                }
                out
            }
            Err(e) => format!("{e}"),
        }
    }
}

async fn handle_provider(app: &mut App<'_>, args: &str) -> String {
    use mika_common::llm::ProviderKind;

    let args = args.trim();
    if args.is_empty() {
        // List providers using shared logic
        let providers = crate::commands::provider::list_providers(
            &app.global_home,
            &app.home_dir,
            app.provider,
        );
        let mut out = format!("Current provider: {}\n\nAvailable providers:", app.provider);
        for p in &providers {
            let marker = if p.current { " (current)" } else { "" };
            let _ = write!(out, "\n  {} — {}{}", p.name, p.model, marker);
        }
        let _ = write!(out, "\n\nUsage: /provider <name>");
        return out;
    }

    // Handle subcommands: "set model <name>", "set api_key <value>", "set base_url <value>"
    if args == "set" {
        return "Usage: /provider set <model|api_key|base_url> <value>".to_string();
    }
    if let Some(rest) = args.strip_prefix("set ") {
        let rest = rest.trim();
        if let Some((field, value)) = rest.split_once(' ') {
            let value = value.trim();
            match crate::commands::provider::set_provider_field(
                &app.global_home,
                &app.home_dir,
                field,
                value,
            ) {
                Ok(result) => {
                    // If setting model, also update the display and agent worker
                    if field == "model" {
                        let display_id =
                            crate::commands::provider::format_display_model(app.provider, value);
                        let worker_model =
                            crate::commands::provider::format_worker_model(app.provider, value);
                        app.model = display_id;
                        if app
                            .agent_tx
                            .send(AgentRequest::SetModel {
                                model: worker_model,
                            })
                            .is_err()
                        {
                            return WORKER_NOT_RESPONDING.to_string();
                        }
                        app.needs_redraw = true;
                    }
                    if field == "api_key" || field == "api-key" {
                        format!(
                            "Set {} in .env (restart required for changes to take effect)",
                            result.field
                        )
                    } else {
                        format!("Set {} = \"{}\"", result.field, result.value)
                    }
                }
                Err(e) => format!("Error: {e}"),
            }
        } else {
            "Usage: /provider set <model|api_key|base_url> <value>".to_string()
        }
    } else {
        // Switch provider using shared logic (prefetch_models=false, TUI does fire-and-forget)
        match crate::commands::provider::switch_provider(
            &app.global_home,
            &app.home_dir,
            match args.parse::<ProviderKind>() {
                Ok(p) => p,
                Err(e) => return e,
            },
            false,
        )
        .await
        {
            Ok(result) => {
                // mika#1727 vol V3b-2 — cette commande écrit, et rien de plus :
                // aucun `SetModel` ne part, ni `app.provider` ni `app.model` ne
                // bouge. Un override préfixé d'un provider que le spirit **en
                // cours** n'a pas configuré est refusé fermement et couperait
                // tous les tours suivants (mika#1591), et des champs qui
                // annonceraient un provider que le fil n'atteint pas seraient
                // une fausse provenance (motif mika#2331). La bascule atteint
                // les tours au prochain démarrage de mika-spirit, qui relit
                // `config.toml` — d'où une réponse qui nomme ce redémarrage.
                let new_provider: ProviderKind = result.provider.parse().unwrap_or(app.provider);

                // Fire-and-forget model cache warm (TUI keeps running, unlike CLI)
                let home = app.home_dir.clone();
                let global = app.global_home.clone();
                tokio::spawn(async move {
                    let _ =
                        crate::commands::provider::fetch_models(&global, &home, new_provider).await;
                });

                let mut out = format!(
                    "Wrote llm_provider = \"{new_provider}\" (model: {}) to config.toml. \
                     mika-spirit takes it at its next restart, for every turn of the agent. \
                     This session keeps running on {}.",
                    result.model, app.provider
                );
                for w in &result.warnings {
                    let _ = write!(out, "\n{w}");
                }
                out
            }
            Err(e) => format!("{e}"),
        }
    }
}

async fn handle_export(app: &mut App<'_>) -> String {
    if app.messages.is_empty() {
        return "Nothing to export.".to_string();
    }

    // Team mode exports to the team directory instead of agent home
    let exports_dir = if let Some(ref team_dir) = app.team_dir {
        team_dir.join("exports")
    } else {
        app.home_dir.join("exports")
    };
    if let Err(e) = tokio::fs::create_dir_all(&exports_dir).await {
        return format!("Failed to create exports directory: {e}");
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
    let (label, filename) = if app.is_team_mode() {
        let name = app.team_name.as_deref().unwrap_or("team");
        (name.to_string(), format!("team-{name}-{timestamp}.md"))
    } else {
        let short_session = &app.session_id[..8.min(app.session_id.len())];
        (
            app.session_id.clone(),
            format!("session-{short_session}-{timestamp}.md"),
        )
    };
    let filepath = exports_dir.join(&filename);

    let mut content = String::from("# Mika Conversation Export\n\n");
    if app.is_team_mode() {
        let _ = writeln!(content, "Team: {label}");
    } else {
        let _ = writeln!(content, "Session: {label}");
        let _ = writeln!(content, "Model: {}", app.model);
    }
    let _ = writeln!(
        content,
        "Exported: {}\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    );
    let _ = writeln!(content, "---\n");

    for msg in &app.messages {
        let channel_prefix = msg
            .channel
            .as_ref()
            .map(|ch| format!("[{ch}] "))
            .unwrap_or_default();
        match msg.role {
            ChatRole::User => {
                let _ = writeln!(content, "**{channel_prefix}You:** {}\n", msg.content);
            }
            ChatRole::Assistant => {
                let _ = writeln!(
                    content,
                    "**{channel_prefix}{}:** {}\n",
                    app.identity_name, msg.content
                );
            }
            ChatRole::System => {
                let _ = writeln!(content, "*System: {}*\n", msg.content);
            }
            ChatRole::Thinking => {
                // Thinking is ephemeral — not exported.
            }
            ChatRole::Command => {
                // Command output is ephemeral (status, help text, etc.)
                // and not part of the conversation — intentionally excluded.
            }
        }
    }

    // Use create_new to prevent symlink attacks (atomic, fails if path exists)
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&filepath)
        .await
    {
        Ok(mut file) => {
            use tokio::io::AsyncWriteExt;
            match file.write_all(content.as_bytes()).await {
                Ok(()) => format!("Exported to {}", filepath.display()),
                Err(e) => format!("Export failed: {e}"),
            }
        }
        Err(e) => format!("Export failed: {e}"),
    }
}

fn handle_skills(app: &App<'_>) -> String {
    let skills = app.skills.skills();
    let skipped = app.skills.skipped();

    if skills.is_empty() && skipped.is_empty() {
        return "No skills loaded.".to_string();
    }

    let max_name_width = skills
        .iter()
        .map(|e| e.manifest.skill.name.len())
        .chain(skipped.iter().map(|s| s.name.len()))
        .max()
        .unwrap_or(0);

    let mut always_on: Vec<&_> = skills
        .iter()
        .filter(|e| e.manifest.skill.always_on)
        .collect();
    let mut on_demand: Vec<&_> = skills
        .iter()
        .filter(|e| !e.manifest.skill.always_on)
        .collect();

    // Within each group: enabled first, disabled last
    always_on.sort_by_key(|e| !e.enabled);
    on_demand.sort_by_key(|e| !e.enabled);

    let mut out = if skipped.is_empty() {
        format!("Loaded skills ({}):\n", skills.len())
    } else {
        format!(
            "Loaded skills ({}, {} skipped):\n",
            skills.len(),
            skipped.len()
        )
    };

    let format_entry = |out: &mut String, entry: &mika_agent::skills::index::SkillEntry| {
        let tool_count = entry.skill_tools.len();
        let tool_info = if tool_count == 0 {
            "\u{2014}".to_string()
        } else if tool_count == 1 {
            "1 tool".to_string()
        } else {
            format!("{tool_count} tools")
        };
        let disabled = if entry.enabled { "" } else { " [disabled]" };
        let overridden = if entry.has_override {
            " [override]"
        } else {
            ""
        };
        let variants = {
            let count = entry.variant_count();
            if count > 0 {
                format!(" [variants: {count}]")
            } else {
                String::new()
            }
        };
        let _ = writeln!(
            out,
            "  \u{25cf} {:<width$}  {:<9}  {}{}{}{}",
            entry.manifest.skill.name,
            tool_info,
            entry.manifest.skill.description,
            disabled,
            overridden,
            variants,
            width = max_name_width
        );
    };

    if !always_on.is_empty() {
        let _ = writeln!(out, "\n  ALWAYS ON");
        for entry in &always_on {
            format_entry(&mut out, entry);
        }
    }

    if !on_demand.is_empty() {
        let _ = writeln!(out, "\n  ON DEMAND");
        for entry in &on_demand {
            format_entry(&mut out, entry);
        }
    }

    if !skipped.is_empty() {
        let _ = writeln!(out, "\n  SKIPPED");
        for entry in skipped {
            let _ = writeln!(
                out,
                "  \u{2717} {:<width$}  {}",
                entry.name,
                entry.reason,
                width = max_name_width
            );
        }
    }

    out
}

fn handle_switch(app: &mut App<'_>, args: &str) -> String {
    let name = agent::normalize_agent_name(args);
    if name.is_empty() {
        return "Usage: /switch <agent-name>".to_string();
    }
    if let Err(e) = agent::validate_agent_name(&name) {
        return format!("Invalid agent name: {e}");
    }
    if name == app.agent_name {
        return format!("Already using agent '{name}'.");
    }

    if !agent::agent_exists(&app.global_home, &name) {
        return format!(
            "Agent '{name}' not found. Create it with `mika agents create {name}` first."
        );
    }

    if app.status != AgentStatus::Idle {
        return "Cannot switch while agent is busy. Wait for the current response to finish."
            .to_string();
    }

    // Signal the current worker to stop
    let _ = app.agent_tx.send(AgentRequest::Quit);
    app.pending_switch = Some(name.clone());
    format!("Switching to agent '{name}'...")
}

fn handle_agents(app: &App<'_>) -> String {
    let agents = agent::list_agents(&app.global_home);
    let active = home::read_active_agent(&app.global_home);

    if agents.is_empty() {
        return "No agents found.".to_string();
    }

    let mut out = String::from("Agents:\n");
    for name in &agents {
        let current = if *name == app.agent_name {
            " (current)"
        } else {
            ""
        };
        let default = if *name == active { " (default)" } else { "" };
        let _ = writeln!(out, "  {name}{current}{default}");
    }
    out
}

fn handle_skill(app: &App<'_>, args: &str) -> String {
    let name = args.trim();
    if name.is_empty() {
        return "Usage: /skill <name>".to_string();
    }

    let skills = app.skills.skills();
    let found = skills
        .iter()
        .find(|s| s.manifest.skill.name.eq_ignore_ascii_case(name));

    match found {
        Some(entry) => {
            let m = &entry.manifest;
            let keywords = if m.triggers.keywords.is_empty() {
                "none".to_string()
            } else {
                m.triggers.keywords.join(", ")
            };
            let mut out = String::new();
            let _ = writeln!(out, "Skill: {}", m.skill.name);
            let _ = writeln!(out, "  Description: {}", m.skill.description);
            let _ = writeln!(
                out,
                "  Version: {}",
                if m.skill.version.is_empty() {
                    "unset"
                } else {
                    &m.skill.version
                }
            );
            let _ = writeln!(out, "  Always on: {}", m.skill.always_on);
            let _ = writeln!(out, "  Enabled: {}", entry.enabled);
            let _ = writeln!(out, "  Timeout: {}s", m.skill.timeout_secs);
            let _ = writeln!(out, "  Keywords: {keywords}");
            if !entry.skill_tools.is_empty() {
                let tool_names: Vec<&str> = entry
                    .skill_tools
                    .iter()
                    .map(|t| t.definition.name.as_str())
                    .collect();
                let _ = writeln!(out, "  Tools: {}", tool_names.join(", "));
            }
            let _ = writeln!(out, "  Path: {}", entry.dir.display());
            let variant_total = entry.variant_count();
            if variant_total > 0 {
                let providers = entry.variant_providers();
                let provider_list: Vec<_> = providers.iter().copied().collect();
                let mut variant_details = Vec::new();
                for p in &provider_list {
                    let model_count = entry.variant_models(p).len();
                    if model_count > 0 {
                        variant_details.push(format!("{p} ({model_count} models)"));
                    } else {
                        variant_details.push(p.to_string());
                    }
                }
                let _ = writeln!(
                    out,
                    "  Variants: {} total ({})",
                    variant_total,
                    variant_details.join(", ")
                );
            }
            out
        }
        None => {
            format!("No skill found with name '{name}'. Use /skills to list all loaded skills.")
        }
    }
}

fn handle_teams(app: &App<'_>) -> String {
    let teams = team::list_teams(&app.global_home);

    if teams.is_empty() {
        return "No teams found. Use `mika teams create <name>` to create one.".to_string();
    }

    let mut out = String::from("Teams:\n");
    for name in &teams {
        let agent_count = match team::load_team(&app.global_home, name) {
            Ok(def) => def.agents.len(),
            Err(_) => 0,
        };
        let _ = writeln!(out, "  {name} ({agent_count} agents)");
    }
    out
}

/// Show team info for both `/team` and `/status` in team mode.
fn handle_verbose(app: &mut App<'_>) -> String {
    if !app.is_team_mode() {
        return "Verbose mode is only available in team mode.".to_string();
    }
    app.verbose_mode = !app.verbose_mode;
    let state = if app.verbose_mode { "on" } else { "off" };
    format!("Verbose mode: {state}")
}

/// Toggle between inbox mode (hide internal messages) and audit mode (show all).
async fn handle_inbox(app: &mut App<'_>) -> String {
    app.inbox_mode = !app.inbox_mode;

    // Reload messages from DB with the new filter setting.
    // Discard the hidden count — /inbox toggle resets the counter to track
    // only messages arriving during the new mode session.
    app.messages.clear();
    if let Ok((recent, _hidden_count)) = app
        .db
        .load_recent_messages_filtered(20, app.inbox_mode)
        .await
    {
        for msg in &recent {
            if let Some(chat_msg) = session_message_to_chat_message(msg) {
                app.messages.push(chat_msg);
            }
        }
    }

    // Reset hidden count — will be refreshed on next poll tick
    app.hidden_internal_count = 0;
    app.scroll_offset = 0;
    app.messages_layout = MessagesLayout::default();

    if app.inbox_mode {
        "Switched to inbox mode (internal messages hidden).".to_string()
    } else {
        "Switched to audit mode (all messages visible).".to_string()
    }
}

fn handle_team_info(app: &App<'_>) -> String {
    let team_name = app.team_name.as_deref().unwrap_or("unknown");
    let mut out = String::new();
    let _ = writeln!(out, "Team: {team_name}");
    match team::load_team(&app.global_home, team_name) {
        Ok(def) => {
            let _ = writeln!(out, "  Orchestrator: {}", def.team.orchestrator);
            let _ = writeln!(out, "  Max iterations: {}", def.flow.max_iterations);
            let _ = writeln!(out, "  Agents:");
            for a in &def.agents {
                let _ = writeln!(out, "    {} — {}", a.name, a.role);
            }
        }
        Err(e) => {
            let _ = writeln!(out, "  (failed to load team definition: {e})");
        }
    }
    out
}

fn handle_team(args: &str) -> String {
    if args.is_empty() {
        return "Usage: /team <name> \"<goal>\"".to_string();
    }

    // Parse: /team <name> <goal>
    let (name, goal) = match args.split_once(char::is_whitespace) {
        Some((n, g)) => (n.trim(), g.trim().trim_matches('"')),
        None => return "Usage: /team <name> \"<goal>\"".to_string(),
    };

    if goal.is_empty() {
        return "Usage: /team <name> \"<goal>\"".to_string();
    }

    format!(
        "Team runs are long-running operations. Use the CLI instead: \
         mika ask --team {name} \"{goal}\""
    )
}

async fn handle_think(app: &mut App<'_>, args: &str) -> Option<String> {
    let args = args.trim();

    // No args: show current level and usage
    if args.is_empty() {
        let current = match app.thinking_level {
            Some((budget, level)) => format!("Thinking level: {level} ({budget} tokens)"),
            None => "Thinking: off".to_string(),
        };
        return Some(format!(
            "{current}\nUsage: /think [low|medium|high|off] [prompt]"
        ));
    }

    // /think off — disable persistent thinking
    if args.eq_ignore_ascii_case("off") {
        app.thinking_level = None;
        if let Err(e) = app.db.set_customer_config("thinking_level", "off").await {
            tracing::warn!(error = %e, "failed to persist thinking level");
        }
        return Some("Thinking: off".to_string());
    }

    // Parse: first word might be a level
    let (first, rest) = match args.split_once(char::is_whitespace) {
        Some((f, r)) => (f, r.trim()),
        None => (args, ""),
    };

    // Resolve budget, level, and prompt text in one step
    let (budget, level, prompt) = match resolve_thinking_level(first) {
        Some((b, l)) if rest.is_empty() => {
            // /think high — set persistent level, no prompt
            app.thinking_level = Some((b, l));
            if let Err(e) = app.db.set_customer_config("thinking_level", l).await {
                tracing::warn!(error = %e, "failed to persist thinking level");
            }
            return Some(format!(
                "Thinking level: {l} ({b} tokens). All messages will use extended thinking."
            ));
        }
        Some((b, l)) => (b, l, rest),
        None => (10_000, "medium", args),
    };

    if app.status != AgentStatus::Idle {
        return Some("Agent is busy. Wait for the current response to finish.".to_string());
    }

    app.messages.push(ChatMessage {
        role: ChatRole::User,
        content: format!("[think:{level}] {prompt}"),
        rendered: None,
        channel: None,
    });

    let images = std::mem::take(&mut app.pending_images);
    let _ = app.agent_tx.send(AgentRequest::Message {
        text: prompt.to_string(),
        images,
        thinking_budget: Some(budget),
    });
    app.status = AgentStatus::Thinking;
    app.scroll_offset = 0;
    app.needs_redraw = true;
    None
}

fn handle_attach(app: &mut App<'_>, args: &str) -> String {
    let path = args.trim();
    if path.is_empty() {
        return "Usage: /attach <path-to-image>".to_string();
    }
    match input::try_load_image_file(path) {
        Some(attachment) => {
            let label = attachment.label.clone();
            let size = attachment.size_display();
            if let Some(err) = app.attach_image(attachment) {
                return err;
            }
            format!("Attached: {label} ({size})")
        }
        None => {
            "Failed to load image. Supported formats: png, jpg, gif, webp (max 10MB).".to_string()
        }
    }
}

async fn handle_undo(app: &mut App<'_>) -> String {
    handle_rewind_impl(app, 1, None).await
}

async fn handle_rewind(app: &mut App<'_>, args: &str) -> String {
    let args = args.trim();

    if args.is_empty() {
        return "Usage: /rewind <count> or /rewind to <message_id>".to_string();
    }

    // /rewind to <message_id>
    if let Some(id_str) = args
        .strip_prefix("to ")
        .or_else(|| args.strip_prefix("to\t"))
    {
        let id_str = id_str.trim();
        match id_str.parse::<i64>() {
            Ok(msg_id) if msg_id > 0 => return handle_rewind_impl(app, 0, Some(msg_id)).await,
            _ => return format!("Invalid message ID: '{id_str}'. Must be a positive integer."),
        }
    }

    // /rewind <count>
    match args.parse::<usize>() {
        Ok(count) if count > 0 => handle_rewind_impl(app, count, None).await,
        Ok(_) => "Rewind count must be at least 1.".to_string(),
        Err(_) => {
            format!("Invalid argument: '{args}'. Usage: /rewind <count> or /rewind to <message_id>")
        }
    }
}

// La lecture stricte du jeton et sa source vivent chez les portes depuis
// V2d-4b (`commands/session_remote.rs`) : `/rewind` et `/clear` la partagent
// avec le démarrage de la TUI, et la copie locale qu'elle remplace nommait le
// rembobinage dans un refus que trois appelants se partagent désormais.
//
// Refuser **avant** l'appel reste le geste : les routes sont montées sous
// `require_internal_token`, donc un jeton absent y prendrait un 401 dont le
// message nomme le transport là où la cause est la configuration du shell.
// `commands/iterate.rs:72-79` a tranché l'inverse pour une route qui *lance*
// un dispatch — un aller-retour pour un 401 lisible n'y coûte qu'un appel sans
// effet. Ici l'appel qui suivrait n'est pas sans effet.
use crate::commands::session_remote::{SourceEnv, jeton_interne};

async fn handle_rewind_impl(
    app: &mut App<'_>,
    exchange_count: usize,
    after_message_id: Option<i64>,
) -> String {
    if app.status != AgentStatus::Idle {
        return "Cannot rewind while agent is busy.".to_string();
    }

    // Le `.env` de la home est chargé ici et non dans `jeton_interne` : la
    // lecture prend sa source en paramètre pour qu'un test la pilote, et un
    // chargement caché derrière ce paramètre ferait mentir sa signature.
    if let Ok(home) = mika_common::home::resolve_home_dir() {
        mika_common::dotenv::load_dotenv(&home);
    }
    let base_url = crate::commands::dashboard::spirit_url();
    rewind_par_spirit(
        app,
        &base_url,
        &|nom| std::env::var(nom).ok(),
        exchange_count,
        after_message_id,
    )
    .await
}

/// Le rembobinage tel que mika-spirit l'applique : l'ancrage, l'aperçu et
/// l'exécution passent par les trois portes de [`crate::commands::rewind_remote`].
///
/// L'URL de base et la source d'environnement arrivent **par argument**, comme
/// `commands/chat_turn.rs::send_turn_to_endpoint` le fait pour le tour : c'est ce
/// qui permet aux tests de viser un factice et de lire ce qui est parti sur la
/// socket, plutôt que de relire le code qui l'écrit.
async fn rewind_par_spirit(
    app: &mut App<'_>,
    base_url: &str,
    lire_env: SourceEnv<'_>,
    exchange_count: usize,
    after_message_id: Option<i64>,
) -> String {
    use crate::commands::rewind_remote;
    use mika_agent::rewind;

    let jeton = match jeton_interne(lire_env) {
        Ok(jeton) => jeton,
        Err(refus) => return refus,
    };

    // L'ancrage. `/rewind to <id>` le tient déjà et n'appelle rien ; `/undo` et
    // `/rewind <n>` le demandent à `resolve`, qui peut rendre une session
    // antérieure. Un `Err` est rendu par son `Display`, qui nomme déjà la cause —
    // dont le 404 « no exchanges to rewind, or unknown agent », qui couvre ce que
    // le `Ok(None)` local rendait avant ce vol.
    let (session, ancrage) = if let Some(id) = after_message_id {
        (app.session_id.clone(), id)
    } else {
        match rewind_remote::resolve(
            base_url,
            &jeton,
            &app.agent_name,
            &app.session_id,
            exchange_count,
        )
        .await
        {
            Ok(ancrage) => (ancrage.session_id, ancrage.after_message_id),
            Err(probleme) => return format!("Error finding exchanges: {probleme}"),
        }
    };

    // L'aperçu. `format_preview` reste la **seule** mise en forme d'un aperçu, de
    // part et d'autre de la porte : c'est tout l'intérêt de la conversion vers le
    // type de domaine que le vol V2c-1a-2a a payée.
    let apercu_rendu =
        match rewind_remote::preview(base_url, &jeton, &app.agent_name, &session, ancrage).await {
            Ok(apercu) => rewind::format_preview(&apercu),
            Err(rewind_remote::RewindRemoteIssue::Blocked { reason, preview }) => {
                let mut sortie = format!("Rewind blocked: {reason}");
                // L'aperçu du refus quand le serveur l'a joint : il dit *ce qui*
                // aurait été défait, ce que la seule raison ne dit pas.
                if let Some(apercu) = preview {
                    sortie.push_str("\n\n");
                    sortie.push_str(&rewind::format_preview(&apercu));
                }
                return sortie;
            }
            Err(probleme) => return format!("Error previewing rewind: {probleme}"),
        };

    // Cloné et non emprunté : `app` est repris en `&mut` juste après pour le
    // rechargement, et un emprunt immuable traversant l'`await` d'`execute`
    // ferait porter au lecteur la charge de vérifier qu'il meurt à temps.
    let origine = (session != app.session_id).then(|| app.session_id.clone());

    let resultat = match rewind_remote::execute(
        base_url,
        &jeton,
        &app.agent_name,
        &session,
        ancrage,
        origine.as_deref(),
    )
    .await
    {
        Ok(resultat) => resultat,
        Err(probleme) => return format!("Rewind failed: {probleme}"),
    };

    // Reload messages from DB for both same-session and cross-session
    // rewinds. The DB may contain intermediate assistant messages (tool_use)
    // that are never in app.messages, so counting-based truncation is unreliable.
    // This mirrors the startup loader in chat.rs.
    //
    // C'est **spirit** qui a écrit cette base, et la TUI qui la relit : les deux
    // processus partagent le fichier, et le déplacement de la session hors du
    // processus de la TUI est un vol ultérieur de mika#1727. Jusque-là, ce
    // rechargement est précisément ce qui rend le rembobinage distant visible ici.
    app.messages.clear();
    if let Ok((recent, _hidden_count)) = app
        .db
        .load_recent_messages_filtered(20, app.inbox_mode)
        .await
    {
        for msg in &recent {
            if let Some(chat_msg) = session_message_to_chat_message(msg) {
                app.messages.push(chat_msg);
            }
        }
    }
    app.scroll_offset = 0;
    app.messages_layout = MessagesLayout::default();

    let mut output = apercu_rendu;
    output.push_str(&format!(
        "\n\nRewind complete: {} messages deleted, {} reversals applied",
        resultat.messages_deleted, resultat.reversals_applied
    ));
    // `reversals_skipped` et `reversal_descriptions` disent **ce que le
    // rembobinage n'a pas su défaire**. Un effet mémoire non inversé qu'on tait
    // est exactement le mensonge que `reversal_action_from_label` refuse déjà de
    // produire côté aperçu ; le taire ici le produirait une porte plus loin.
    if resultat.reversals_skipped > 0 {
        output.push_str(&format!(", {} skipped", resultat.reversals_skipped));
    }
    output.push('.');
    if resultat.tasks_deleted > 0 || resultat.tasks_cancelled > 0 {
        output.push_str(&format!(
            "\nTasks: {} deleted, {} cancelled.",
            resultat.tasks_deleted, resultat.tasks_cancelled
        ));
    }
    if !resultat.reversal_descriptions.is_empty() {
        output.push_str("\n\nReversals:");
        for description in &resultat.reversal_descriptions {
            output.push_str(&format!("\n  - {description}"));
        }
    }
    if !resultat.warnings.is_empty() {
        output.push_str("\n\nWarnings:");
        for w in &resultat.warnings {
            output.push_str(&format!("\n  - {w}"));
        }
    }
    if !resultat.rewind_trace_id.is_empty() {
        output.push_str(&format!("\nTrace: {}", resultat.rewind_trace_id));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use mika_agent::async_db::AsyncDatabase;
    use mika_agent::db::Database;
    use mika_agent::skills::SkillRegistry;
    use mika_common::llm::ProviderKind;
    use tokio::sync::mpsc;

    /// Test helper: construct a minimal App with captured agent_tx receiver.
    /// Returns (App, agent_rx, temp_dir) — temp_dir must be kept alive for the test duration.
    async fn test_app() -> (
        App<'static>,
        mpsc::UnboundedReceiver<AgentRequest>,
        tempfile::TempDir,
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let home_dir = temp_dir.path().to_path_buf();
        let global_home = temp_dir.path().to_path_buf();

        let db = Database::open_in_memory().unwrap();
        let async_db = AsyncDatabase::new(db);

        // Create a session for the app
        let session_id = uuid::Uuid::new_v4().to_string();
        async_db
            .create_session(&session_id, "mika", "cli")
            .await
            .unwrap();

        let (agent_tx, agent_rx) = mpsc::unbounded_channel();
        let (_response_tx, response_rx) = mpsc::unbounded_channel();

        let skills = Arc::new(SkillRegistry::empty());

        let app = App::new(
            agent_tx,
            response_rx,
            session_id,
            "claude-sonnet-4-6".to_string(),
            "Mika".to_string(),
            async_db,
            home_dir,
            skills,
            "mika".to_string(),
            global_home,
            ProviderKind::Anthropic,
            false, // start_in_audit_mode
            false, // is_singleton_session (mika#1401)
        );

        (app, agent_rx, temp_dir)
    }

    /// Drain all pending requests from the agent channel.
    fn drain_requests(rx: &mut mpsc::UnboundedReceiver<AgentRequest>) -> Vec<AgentRequest> {
        let mut requests = Vec::new();
        while let Ok(req) = rx.try_recv() {
            requests.push(req);
        }
        requests
    }

    // --- Existing tests ---

    #[test]
    fn test_handle_help_contains_all_commands() {
        let output = handle_help();
        assert!(output.contains("/help"));
        assert!(output.contains("/clear"));
        assert!(output.contains("/exit"));
        assert!(output.contains("/memory"));
        assert!(output.contains("/status"));
        assert!(output.contains("/skills"));
    }

    #[test]
    fn test_handle_help_contains_new_commands() {
        let output = handle_help();
        assert!(output.contains("/think"));
        assert!(output.contains("/attach"));
        assert!(output.contains("/undo"));
        assert!(output.contains("/rewind"));
    }

    #[test]
    fn test_handle_help_contains_config_args() {
        let output = handle_help();
        assert!(output.contains("/config"));
        assert!(output.contains("set <key> <value>"));
    }

    #[test]
    fn test_settable_config_keys_allowlist() {
        assert!(is_settable_key("chat_id"));
        assert!(is_settable_key("timezone"));
        assert!(!is_settable_key("api_key"));
        assert!(!is_settable_key("db_path"));
    }

    #[test]
    fn test_team_mode_allowed_commands() {
        for cmd in &[
            "help", "h", "?", "clear", "exit", "quit", "q", "export", "agents", "teams", "team",
            "status", "stat",
        ] {
            assert!(
                TEAM_MODE_ALLOWED_COMMANDS.contains(cmd),
                "Expected '{cmd}' to be allowed in team mode"
            );
        }
    }

    #[test]
    fn test_team_mode_blocks_agent_commands() {
        for cmd in &[
            "compact", "memory", "model", "provider", "think", "soul", "config", "skills", "agent",
            "attach", "undo", "rewind",
        ] {
            assert!(
                !TEAM_MODE_ALLOWED_COMMANDS.contains(cmd),
                "Expected '{cmd}' to be blocked in team mode"
            );
        }
    }

    // --- /clear tests ---

    /// Les chemins tels que le banc doit les voir : `fake_spirit` associe par le
    /// chemin **brut**. Un `format!` mal composé dans la porte tombe sur son
    /// `fallback` à 501, qu'aucun bras de `post` ne traduit.
    const ROUTE_SESSIONS: &str = "/api/v1/sessions";
    const CORPS_OUVERT: &str = r#"{"session_id":"s-du-fil","canonical":false}"#;
    const CORPS_CLOS: &str = r#"{"ended":true,"canonical":false}"#;

    /// Monte le banc sur `reponses` et fait passer un `/clear` dessus.
    async fn par_le_banc(
        app: &mut App<'_>,
        reponses: &[(&str, axum::http::StatusCode, String)],
        lire_env: SourceEnv<'_>,
    ) -> (String, Arc<Mutex<Vec<RequeteRecue>>>) {
        let empruntes = reponses
            .iter()
            .map(|(chemin, statut, corps)| (*chemin, *statut, corps.as_str()))
            .collect::<Vec<_>>();
        let (base, vues) = fake_spirit(&empruntes).await;
        let sortie = clear_par_spirit(app, &base, lire_env).await;
        (sortie, vues)
    }

    /// Le chemin nominal : les deux routes servies, et le jeton posé. C'est ce
    /// que les tests existants de `/clear` attendaient de la base locale ;
    /// chacun garde son intention, seule la façon d'y arriver change.
    ///
    /// Le chemin de clôture est composé depuis `app.session_id` **avant**
    /// l'appel : c'est l'ancienne session qui le porte, et le banc associe par
    /// chemin brut.
    async fn clear_nominal(app: &mut App<'_>) -> (String, Arc<Mutex<Vec<RequeteRecue>>>) {
        let fin = format!("{ROUTE_SESSIONS}/{}/end", app.session_id);
        let reponses = [
            (
                ROUTE_SESSIONS,
                axum::http::StatusCode::OK,
                CORPS_OUVERT.to_string(),
            ),
            (
                fin.as_str(),
                axum::http::StatusCode::OK,
                CORPS_CLOS.to_string(),
            ),
        ];
        par_le_banc(app, &reponses, &avec_jeton).await
    }

    #[tokio::test]
    async fn test_clear_creates_new_session() {
        let (mut app, _rx, _tmp) = test_app().await;
        let old_session = app.session_id.clone();

        // Add a message to verify it gets cleared
        app.messages.push(ChatMessage {
            role: ChatRole::User,
            content: "hello".to_string(),
            rendered: None,
            channel: None,
        });

        let (output, _) = clear_nominal(&mut app).await;

        assert!(output.contains("session reset"));
        assert_ne!(app.session_id, old_session, "session_id should change");
        assert!(app.messages.is_empty(), "messages should be cleared");
        assert_eq!(app.scroll_offset, 0);
        assert_eq!(app.last_seen_msg_id, 0);
        assert!(app.context_tokens.is_none());
        assert!(app.needs_redraw);
    }

    #[tokio::test]
    async fn test_clear_sends_new_session_request() {
        let (mut app, mut rx, _tmp) = test_app().await;

        clear_nominal(&mut app).await;

        let requests = drain_requests(&mut rx);
        assert_eq!(requests.len(), 1, "should send exactly one request");
        match &requests[0] {
            AgentRequest::NewSession { session_id } => {
                assert_eq!(
                    session_id, &app.session_id,
                    "session_id should match app's new session"
                );
            }
            _ => panic!("expected NewSession request"),
        }
    }

    /// L'intention est la même qu'avant ce vol — « l'ancienne session est
    /// close » — mais le fait se lit désormais **sur la socket** : la clôture
    /// est un geste de mika-spirit, et `ended_at` en base locale n'en serait
    /// plus qu'un reflet, écrit par un autre processus.
    #[tokio::test]
    async fn test_clear_ends_old_session() {
        let (mut app, _rx, _tmp) = test_app().await;
        let old_session = app.session_id.clone();

        let (_, vues) = clear_nominal(&mut app).await;

        let vues = vues.lock().expect("le témoin du banc");
        let fin = vues
            .iter()
            .find(|v| v.chemin == format!("{ROUTE_SESSIONS}/{old_session}/end"))
            .expect("la route de clôture est atteinte, sur l'ancienne session");
        assert_eq!(fin.methode, "POST");
        // L'agent part dans le corps : sans lui, le serveur résoudrait *son*
        // agent par défaut et clorait la session d'un tiers (`clore`, l. 275).
        assert!(fin.corps.contains(r#""agent":"mika""#), "{}", fin.corps);
    }

    // --- /clear singleton tests (mika#1401) ---

    #[tokio::test]
    async fn test_clear_singleton_preserves_session() {
        let (mut app, mut rx, _tmp) = test_app().await;
        app.is_singleton_session = true;
        let session_before = app.session_id.clone();

        app.messages.push(ChatMessage {
            role: ChatRole::User,
            content: "hello".to_string(),
            rendered: None,
            channel: None,
        });

        // Le banc est **armé** et la source est **vide** : si le bras singleton
        // lisait le jeton ou appelait une porte, il refuserait au lieu
        // d'effacer, et le témoin montrerait une requête (critère 6).
        let fin = format!("{ROUTE_SESSIONS}/{}/end", app.session_id);
        let reponses = [
            (
                ROUTE_SESSIONS,
                axum::http::StatusCode::OK,
                CORPS_OUVERT.to_string(),
            ),
            (
                fin.as_str(),
                axum::http::StatusCode::OK,
                CORPS_CLOS.to_string(),
            ),
        ];
        let (output, vues) = par_le_banc(&mut app, &reponses, &|_| None).await;
        assert_eq!(
            vues.lock().expect("le témoin du banc").len(),
            0,
            "un singleton n'appelle rien, même sans jeton"
        );

        // Session ID is preserved (single-session-by-nature).
        assert_eq!(
            app.session_id, session_before,
            "singleton session_id must not change on /clear"
        );
        assert!(output.contains("single-session-by-nature"));
        // Display is still cleared.
        assert!(
            app.messages.is_empty(),
            "display messages should be cleared"
        );
        assert_eq!(app.scroll_offset, 0);
        assert!(app.needs_redraw);
        // No NewSession request is sent — the worker keeps its context.
        let requests = drain_requests(&mut rx);
        assert!(
            requests.is_empty(),
            "singleton /clear must not send a NewSession request"
        );
    }

    #[tokio::test]
    async fn test_clear_singleton_does_not_end_session() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.is_singleton_session = true;
        let session_id = app.session_id.clone();

        // Celui-ci garde son assertion sur `ended_at` : elle mesure que *rien*
        // n'a touché la base locale, et c'est encore vrai — et la seule chose
        // qu'un témoin de socket ne dirait pas.
        let (_, _) = par_le_banc(
            &mut app,
            &[(
                ROUTE_SESSIONS,
                axum::http::StatusCode::OK,
                CORPS_OUVERT.to_string(),
            )],
            &|_| None,
        )
        .await;

        // The canonical session's ended_at must stay NULL.
        let session = app.db.get_session(&session_id).await.unwrap();
        assert!(
            session.map(|s| s.ended_at.is_none()).unwrap_or(false),
            "singleton session must not be ended on /clear"
        );
    }

    // --- /model tests ---

    #[test]
    fn test_model_alias_resolves() {
        let result = resolve_model_name("opus");
        assert!(result.is_some());
        let (full_id, _display) = result.unwrap();
        assert!(full_id.contains("opus"), "should resolve to an opus model");
    }

    #[test]
    fn test_model_unknown_returns_none() {
        let result = resolve_model_name("nonexistent-model-xyz");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_model_no_args_shows_current() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_model(&mut app, "").await;

        assert!(output.contains("Current model:"));
        assert!(output.contains("claude-sonnet-4-6"));
        assert!(output.contains("Usage: /model"));
    }

    #[tokio::test]
    async fn test_model_direct_name_sets_model() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Switch to DeepSeek first (easier to test with fake API key)
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();
        let _ = handle_provider(&mut app, "deepseek").await;
        // Le canal est déjà vide depuis V3b-2 — la bascule n'envoie rien ; le
        // drain garde la mesure ci-dessous close sur le seul `/model`.
        drain_requests(&mut rx);

        // Use a direct model name (not an alias)
        let output = handle_model(&mut app, "deepseek-reasoner").await;

        assert!(
            output.contains("Switched to deepseek/deepseek-reasoner"),
            "should accept direct model name, got: {output}"
        );
        assert_eq!(app.model, "deepseek/deepseek-reasoner");

        // Should have sent SetModel to the worker
        let requests = drain_requests(&mut rx);
        assert!(requests.iter().any(
            |r| matches!(r, AgentRequest::SetModel { model } if model == "deepseek/deepseek-reasoner")
        ));
    }

    // --- /provider tests ---

    #[tokio::test]
    async fn test_provider_no_args_shows_list() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "").await;

        assert!(output.contains("Current provider:"));
        assert!(output.contains("anthropic"));
        assert!(output.contains("openai"));
        assert!(output.contains("Usage: /provider"));
    }

    #[tokio::test]
    async fn test_provider_already_using() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "anthropic").await;

        assert!(output.contains("Already using"));
    }

    #[tokio::test]
    async fn test_provider_invalid_name() {
        let (mut app, _rx, _tmp) = test_app().await;
        let original_provider = app.provider;

        let output = handle_provider(&mut app, "nonexistent-provider").await;

        // Should return an error message (from ProviderKind parse error)
        // Should return an error, not a success
        assert!(
            !output.contains("Switched"),
            "should not switch to invalid provider, got: {output}"
        );
        assert_eq!(
            app.provider, original_provider,
            "provider should not change"
        );
    }

    #[tokio::test]
    async fn test_provider_set_subcommand_usage() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "set").await;

        assert!(
            output.contains("Usage:"),
            "expected usage message, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_provider_set_unknown_field() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_provider(&mut app, "set unknown_field value").await;

        assert!(output.contains("Unknown field"));
    }

    // --- /clear comprehensive state reset tests ---

    #[tokio::test]
    async fn test_clear_resets_all_state_fields() {
        let (mut app, _rx, _tmp) = test_app().await;

        // Set all fields to non-default values
        app.pending_response = Some("partial response text".to_string());
        app.reveal_index = 42;
        app.has_new_message = true;
        app.selection_state = SelectionState::Selected {
            start: crate::tui::app::MessagePosition {
                message_idx: 0,
                text_pos: crate::tui::app::TextPosition {
                    line: 0,
                    char_offset: 0,
                },
            },
            end: crate::tui::app::MessagePosition {
                message_idx: 0,
                text_pos: crate::tui::app::TextPosition {
                    line: 0,
                    char_offset: 5,
                },
            },
        };
        app.pending_command = Some("/model opus".to_string());
        app.pending_task_count = 3;
        app.executing_task_count = 1;
        app.queued_task_count = 2;
        app.status = AgentStatus::Thinking;

        clear_nominal(&mut app).await;

        assert!(
            app.pending_response.is_none(),
            "pending_response should be None"
        );
        assert_eq!(app.reveal_index, 0, "reveal_index should be 0");
        assert!(!app.has_new_message, "has_new_message should be false");
        assert!(
            matches!(app.selection_state, SelectionState::None),
            "selection_state should be None"
        );
        assert!(
            app.pending_command.is_none(),
            "pending_command should be None"
        );
        assert_eq!(app.pending_task_count, 0, "pending_task_count should be 0");
        assert_eq!(
            app.executing_task_count, 1,
            "executing_task_count should be preserved (agent-scoped, not session-scoped)"
        );
        assert_eq!(
            app.queued_task_count, 2,
            "queued_task_count should be preserved (agent-scoped, not session-scoped)"
        );
        assert_eq!(app.status, AgentStatus::Idle, "status should be Idle");
    }

    #[tokio::test]
    async fn test_clear_while_thinking() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Thinking;

        clear_nominal(&mut app).await;

        assert_eq!(app.status, AgentStatus::Idle);
    }

    #[tokio::test]
    async fn test_clear_while_responding() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Responding(42);
        app.pending_response = Some("partial output being revealed".to_string());
        app.reveal_index = 10;

        clear_nominal(&mut app).await;

        assert_eq!(app.status, AgentStatus::Idle);
        assert!(app.pending_response.is_none());
        assert_eq!(app.reveal_index, 0);
    }

    #[tokio::test]
    async fn test_clear_with_pending_images() {
        use crate::tui::attachment::ImageAttachment;

        let (mut app, _rx, _tmp) = test_app().await;
        app.pending_images.push(ImageAttachment {
            base64_data: "dGVzdA==".to_string(),
            media_type: "image/png".to_string(),
            size_bytes: 4,
            label: "test.png".to_string(),
        });

        clear_nominal(&mut app).await;

        assert!(
            app.pending_images.is_empty(),
            "pending_images should be cleared"
        );
    }

    #[tokio::test]
    async fn test_clear_preserves_preferences() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.thinking_level = Some((8192, "medium"));
        let original_model = app.model.clone();
        let original_provider = app.provider;

        clear_nominal(&mut app).await;

        assert_eq!(
            app.thinking_level,
            Some((8192, "medium")),
            "thinking_level should be preserved"
        );
        assert_eq!(app.model, original_model, "model should be preserved");
        assert_eq!(
            app.provider, original_provider,
            "provider should be preserved"
        );
    }

    // --- /skills skipped section tests ---

    #[tokio::test]
    async fn test_skills_shows_skipped_section() {
        use mika_agent::skills::index::SkippedSkill;

        let (mut app, _rx, _tmp) = test_app().await;
        app.skills = Arc::new(SkillRegistry::with_skipped(vec![
            SkippedSkill {
                name: "broken-skill".to_string(),
                reason: "broken symlink".to_string(),
            },
            SkippedSkill {
                name: "bad-manifest".to_string(),
                reason: "invalid TOML: unexpected key".to_string(),
            },
        ]));

        let output = handle_skills(&app);

        assert!(output.contains("SKIPPED"), "should have SKIPPED section");
        assert!(
            output.contains("broken-skill"),
            "should show broken skill name"
        );
        assert!(
            output.contains("broken symlink"),
            "should show broken skill reason"
        );
        assert!(
            output.contains("bad-manifest"),
            "should show bad manifest name"
        );
        assert!(
            output.contains("invalid TOML"),
            "should show parse error reason"
        );
        assert!(
            output.contains("2 skipped"),
            "header should show skipped count"
        );
    }

    #[tokio::test]
    async fn test_skills_hides_skipped_when_none() {
        let (app, _rx, _tmp) = test_app().await;

        let output = handle_skills(&app);

        // Empty registry — no skills, no skipped
        assert!(
            !output.contains("SKIPPED"),
            "should not have SKIPPED section"
        );
    }

    // --- Channel send failure tests ---

    #[tokio::test]
    async fn test_clear_with_closed_channel_returns_error() {
        let (mut app, rx, _tmp) = test_app().await;
        // Drop the receiver to close the channel
        drop(rx);

        let (output, _) = clear_nominal(&mut app).await;

        assert!(
            output.contains("not responding"),
            "should report worker error: {output}"
        );
    }

    // --- #442: provider/model config state fixes ---

    #[tokio::test]
    async fn test_provider_switch_reads_configured_model() {
        let (mut app, _rx, tmp) = test_app().await;

        // Pre-configure deepseek_model in config.toml
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\ndeepseek_model = \"deepseek-reasoner\"\n",
        )
        .unwrap();

        // Also need a fake API key for validation to pass
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_provider(&mut app, "deepseek").await;

        // Should use the configured model, not the default
        assert!(
            output.contains("deepseek-reasoner"),
            "should show configured model, got: {output}"
        );
        // mika#1727 vol V3b-2 — ce test tient la lecture du modèle configuré, et
        // l'`App` ne bascule plus ; le fait à part entière est tenu par
        // `test_provider_switch_writes_config_and_sends_no_setmodel`.
        assert_eq!(app.provider, ProviderKind::Anthropic);
    }

    #[tokio::test]
    async fn test_provider_switch_persists_default_model() {
        let (mut app, _rx, tmp) = test_app().await;

        // Set up API key but no deepseek_model
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "llm_provider = \"anthropic\"\n").unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let _output = handle_provider(&mut app, "deepseek").await;

        // Config should now have deepseek_model persisted
        let config_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config_content.contains("deepseek_model"),
            "should persist default model, config: {config_content}"
        );
    }

    #[tokio::test]
    async fn test_provider_switch_warns_max_tokens() {
        let (mut app, _rx, tmp) = test_app().await;

        // Set up: llm_max_tokens exceeds DeepSeek's limit.
        //
        // mika#2296 T4b — this test holds the BEHAVIOUR (a budget above the
        // provider's ceiling still warns), not the value. Its fixture was 16384
        // only because the ceiling was 8192; at 65_536 that fixture no longer
        // exceeds anything and the warning would legitimately stop firing. It
        // moves with the constant rather than being discovered red, which is why
        // it is a distinct test from the `max_output_tokens` pin. 131072 is the
        // hard bound of `validation.rs:110` — the largest value still legal.
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\nllm_max_tokens = 131072\n",
        )
        .unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_provider(&mut app, "deepseek").await;

        assert!(
            output.contains("exceeds deepseek's limit"),
            "should warn about max_tokens, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_model_cross_provider_alias_switches_provider() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set up: start on Anthropic, switch via deepseek alias
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        assert_eq!(app.provider, ProviderKind::Anthropic);
        let output = handle_model(&mut app, "deepseek").await;

        // mika#1727 vol V3b-2 (critère 3) — la bascule est persistée et le
        // modèle part sur le fil, mais `app.provider` ne bouge pas : un override
        // ne re-dispatche jamais vers un autre provider (mika#1591).
        assert_eq!(
            app.provider,
            ProviderKind::Anthropic,
            "le provider de l'App ne suit pas la persistance"
        );
        assert!(
            output.contains("switched provider to deepseek"),
            "should note provider switch, got: {output}"
        );
        assert!(
            output.contains("next restart"),
            "la réponse nomme ce que la persistance couvre, got: {output}"
        );

        // Le `SetModel` porte exactement ce que `format_worker_model` rend pour
        // le provider cible — l'alias `deepseek` résout `deepseek/deepseek-chat`.
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "deepseek/deepseek-chat")
            ),
            "SetModel should carry the target provider's prefixed model"
        );
    }

    #[tokio::test]
    async fn test_model_direct_with_provider_prefix() {
        let (mut app, _rx, tmp) = test_app().await;

        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_model(&mut app, "deepseek/deepseek-reasoner").await;

        // Le provider cible se lit dans le modèle préfixé, pas sur l'`App`
        // (mika#1727 vol V3b-2).
        assert_eq!(app.provider, ProviderKind::Anthropic);
        assert!(
            output.contains("deepseek/deepseek-reasoner"),
            "should set full model id, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_model_no_args_shows_provider_models() {
        let (mut app, _rx, _tmp) = test_app().await;

        let output = handle_model(&mut app, "").await;

        // Anthropic uses hardcoded models
        assert!(
            output.contains("Available models for anthropic"),
            "should show provider models, got: {output}"
        );
        assert!(
            output.contains("claude-sonnet-4-6"),
            "should list Anthropic models, got: {output}"
        );
        assert!(output.contains("Aliases:"), "should show aliases section");
    }

    #[test]
    fn test_max_output_tokens_known_providers() {
        // Verify key providers have sensible limits
        // mika#2296: was 8_192 (the R1-era figure). See the derivation at
        // `ProviderKind::max_output_tokens` and `llm::tests`.
        assert_eq!(ProviderKind::DeepSeek.max_output_tokens(), 65_536);
        assert_eq!(ProviderKind::Anthropic.max_output_tokens(), 128_000);
        assert_eq!(ProviderKind::OpenAi.max_output_tokens(), 16_384);
        assert_eq!(ProviderKind::Groq.max_output_tokens(), 8_192);
    }

    // --- #451: provider switch worker propagation ---

    /// **Le test négatif du vol V3b-2 (critère 2, mika#1727).**
    ///
    /// C'est l'ancien `test_provider_switch_worker_gets_provider_prefix`
    /// retourné, et non un test neuf à côté : le fait qu'il tenait — un
    /// `SetModel` préfixé `deepseek/` part au worker — est précisément celui
    /// que le vol retire. Un override vers un provider que le spirit **en
    /// cours** n'a pas configuré est refusé fermement (mika#1591) et couperait
    /// tous les tours suivants. Le préfixe reste tenu, pour les gestes qui le
    /// portent encore, par `test_provider_set_model_still_sends_setmodel` et
    /// `test_model_switch_worker_gets_provider_prefix`.
    #[tokio::test]
    async fn test_provider_switch_writes_config_and_sends_no_setmodel() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Setup: start on Anthropic, switch to DeepSeek
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        let output = handle_provider(&mut app, "deepseek").await;

        assert!(
            drain_requests(&mut rx).is_empty(),
            "rien ne part au worker : ni SetModel, ni autre chose"
        );
        assert_eq!(app.provider, ProviderKind::Anthropic);
        assert_eq!(app.model, "claude-sonnet-4-6");

        let config = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(
            config.contains("llm_provider = \"deepseek\""),
            "la bascule est écrite dans config.toml, got: {config}"
        );
        assert!(
            output.contains("next restart"),
            "la réponse nomme le redémarrage de mika-spirit, got: {output}"
        );
    }

    /// **É9 — `/provider set model <x>` pose toujours l'override (mika#1727).**
    ///
    /// Ce bras-là ne change pas de provider, donc mika#1591 ne le concerne pas :
    /// le préfixe qu'il envoie est celui d'`app.provider`, c'est-à-dire du
    /// provider que le spirit en cours a chargé. Sans ce test, le retrait du
    /// `SetModel` de la bascule serait indiscernable d'un retrait des deux.
    #[tokio::test]
    async fn test_provider_set_model_still_sends_setmodel() {
        let (mut app, mut rx, tmp) = test_app().await;

        // test_app uses global_home == agent_home, so .env isn't read
        std::fs::write(
            tmp.path().join("config.toml"),
            "llm_provider = \"anthropic\"\nanthropic_api_key = \"sk-ant-fake-key\"\n",
        )
        .unwrap();

        let output = handle_provider(&mut app, "set model claude-opus-4-6").await;

        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(|r| matches!(
                r,
                AgentRequest::SetModel { model } if model == "anthropic/claude-opus-4-6"
            )),
            "SetModel carries app.provider's prefix"
        );
        assert_eq!(app.model, "claude-opus-4-6");
        assert!(
            output.contains("anthropic_model"),
            "should name the key written, got: {output}"
        );
    }

    #[tokio::test]
    async fn test_provider_switch_back_to_anthropic_writes_config_only() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set API keys via process env (test_app uses global_home == agent_home,
        // so .env file isn't loaded by load_for_agent).
        let env_path = tmp.path().join(".env");
        std::fs::write(&env_path, "MIKA_DEEPSEEK_API_KEY=sk-fake-key-for-test\n").unwrap();

        // Switch to DeepSeek first
        let _ = handle_provider(&mut app, "deepseek").await;
        drain_requests(&mut rx);

        // Now switch back to Anthropic — need API key in config since .env isn't read
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"deepseek\"\nanthropic_model = \"claude-sonnet-4-6\"\nanthropic_api_key = \"sk-ant-fake-key\"\n",
        )
        .unwrap();
        let _output = handle_provider(&mut app, "anthropic").await;

        // mika#1727 vol V3b-2 — la bascule de retour écrit, et n'envoie rien non
        // plus : la règle ne dépend pas du provider visé.
        assert!(drain_requests(&mut rx).is_empty());
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config.contains("llm_provider = \"anthropic\""),
            "la bascule de retour est écrite, got: {config}"
        );
    }

    #[tokio::test]
    async fn test_model_switch_worker_gets_provider_prefix() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Need API key in config (test_app uses global_home == agent_home, .env not read)
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\nanthropic_api_key = \"sk-ant-fake-key\"\n",
        )
        .unwrap();

        // Direct model name on Anthropic — worker should get "anthropic/" prefix
        let output = handle_model(&mut app, "claude-opus-4-6").await;

        assert!(
            output.contains("Switched to"),
            "should switch model, got: {output}"
        );

        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "anthropic/claude-opus-4-6")
            ),
            "SetModel should have anthropic/ prefix"
        );
        // Display should be bare for Anthropic
        assert_eq!(app.model, "claude-opus-4-6");
    }

    #[tokio::test]
    async fn test_provider_listing_shows_configured_model() {
        let (mut app, _rx, tmp) = test_app().await;

        // Configure a non-default model for DeepSeek
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"anthropic\"\ndeepseek_model = \"deepseek-reasoner\"\n",
        )
        .unwrap();

        let output = handle_provider(&mut app, "").await;

        // Should show the configured model, not the default "deepseek-chat"
        assert!(
            output.contains("deepseek-reasoner"),
            "should show configured model for deepseek, got: {output}"
        );
    }

    #[test]
    fn test_format_helpers() {
        use crate::commands::provider::{format_display_model, format_worker_model};
        use mika_common::llm::ProviderKind;

        // format_worker_model always includes prefix
        assert_eq!(
            format_worker_model(ProviderKind::Anthropic, "claude-sonnet-4-6"),
            "anthropic/claude-sonnet-4-6"
        );
        assert_eq!(
            format_worker_model(ProviderKind::DeepSeek, "deepseek-chat"),
            "deepseek/deepseek-chat"
        );

        // format_display_model: no prefix for Anthropic, prefix for others
        assert_eq!(
            format_display_model(ProviderKind::Anthropic, "claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            format_display_model(ProviderKind::DeepSeek, "deepseek-chat"),
            "deepseek/deepseek-chat"
        );
    }

    #[tokio::test]
    async fn test_model_alias_on_openrouter_uses_provider_prefix() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set up: user is on OpenRouter
        app.provider = ProviderKind::OpenRouter;
        let config_path = tmp.path().join("config.toml");
        std::fs::write(
            &config_path,
            "llm_provider = \"openrouter\"\nopenrouter_model = \"anthropic/claude-sonnet-4\"\n",
        )
        .unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "MIKA_OPENROUTER_API_KEY=sk-or-fake-key-for-test\n",
        )
        .unwrap();

        // Using "sonnet" alias on OpenRouter should set the model to
        // "anthropic/claude-sonnet-4-6" (valid OpenRouter model name),
        // NOT "claude-sonnet-4-6" (invalid bare name).
        let output = handle_model(&mut app, "sonnet").await;

        // Should stay on OpenRouter (not switch to Anthropic)
        assert_eq!(
            app.provider,
            ProviderKind::OpenRouter,
            "should stay on OpenRouter, got: {:?}",
            app.provider
        );
        assert!(
            output.contains("openrouter/anthropic/claude-sonnet-4-6"),
            "should show openrouter model with anthropic prefix, got: {output}"
        );

        // Worker should receive the full prefixed model
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "openrouter/anthropic/claude-sonnet-4-6")
            ),
            "SetModel should have openrouter/anthropic/ prefix"
        );

        // Config should have openrouter_model with anthropic prefix
        let config_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config_content.contains("anthropic/claude-sonnet-4-6"),
            "should persist model with anthropic prefix, config: {config_content}"
        );
    }

    #[tokio::test]
    async fn test_model_on_openrouter_keeps_slash_model_name() {
        let (mut app, mut rx, tmp) = test_app().await;

        // Set up: user is on OpenRouter
        app.provider = ProviderKind::OpenRouter;
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "llm_provider = \"openrouter\"\n").unwrap();
        let env_path = tmp.path().join(".env");
        std::fs::write(
            &env_path,
            "MIKA_OPENROUTER_API_KEY=sk-or-fake-key-for-test\n",
        )
        .unwrap();

        // When on OpenRouter, "qwen/qwen-plus" should be treated as an OpenRouter model
        // name, NOT as a cross-provider switch to the Qwen provider.
        let output = handle_model(&mut app, "qwen/qwen-plus").await;

        // Should stay on OpenRouter, not switch to Qwen
        assert_eq!(
            app.provider,
            ProviderKind::OpenRouter,
            "should stay on OpenRouter, got: {:?}",
            app.provider
        );
        assert!(
            output.contains("openrouter/qwen/qwen-plus"),
            "should show openrouter model, got: {output}"
        );

        // Worker should receive openrouter/qwen/qwen-plus (not qwen/qwen-plus)
        let requests = drain_requests(&mut rx);
        assert!(
            requests.iter().any(
                |r| matches!(r, AgentRequest::SetModel { model } if model == "openrouter/qwen/qwen-plus")
            ),
            "SetModel should have openrouter/ prefix"
        );

        // Config should have openrouter_model, not qwen_model
        let config_content = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            config_content.contains("openrouter_model"),
            "should persist to openrouter_model, config: {config_content}"
        );
        assert!(
            !config_content.contains("qwen_model"),
            "should NOT create qwen_model, config: {config_content}"
        );
    }

    // --- /restart command tests (mika#1149) ---

    #[tokio::test]
    async fn test_restart_refuses_on_healthy_worker() {
        let (mut app, _rx, _td) = test_app().await;
        // Default state: worker_crashed = false. /restart must refuse.
        let output = handle_restart(&mut app);
        assert!(
            !app.pending_restart,
            "must not arm restart on healthy worker"
        );
        assert!(
            output.contains("only applies after"),
            "refusal message should mention the precondition: {output}"
        );
        assert!(
            output.contains("/clear"),
            "refusal should redirect to /clear: {output}"
        );
    }

    #[tokio::test]
    async fn test_restart_arms_pending_restart_after_crash() {
        let (mut app, _rx, _td) = test_app().await;
        app.worker_crashed = true;
        let output = handle_restart(&mut app);
        assert!(app.pending_restart, "must arm pending_restart");
        assert!(
            output.contains("Restarting"),
            "confirmation should announce restart: {output}"
        );
        assert!(
            output.contains("not replayed"),
            "confirmation should warn the prompt is lost: {output}"
        );
    }

    // --- Le rembobinage par mika-spirit (mika#1727, vol V2c-1b) ---

    /// T1 — sans jeton, le refus nomme la variable à poser.
    #[test]
    fn mika1727_v2c1b_sans_jeton_interne_le_refus_nomme_la_variable() {
        let refus = jeton_interne(&|_| None).expect_err("une source vide ne rend aucun jeton");
        assert!(
            refus.contains("MIKA_INTERNAL_TOKEN"),
            "le refus doit nommer la variable à poser : {refus}"
        );
    }

    /// T2 — `MIKA_DASHBOARD_TOKEN` seul ne suffit pas.
    #[test]
    fn mika1727_v2c1b_un_jeton_de_tableau_de_bord_ne_suffit_pas() {
        // La source ne rend que `MIKA_DASHBOARD_TOKEN` : l'environnement d'un
        // opérateur qui a configuré le tableau de bord et rien d'autre. Le repli de
        // `dashboard::auth_token()` l'accepterait ici, pour prendre un 401 sur
        // `resolve`. La source est un paramètre, donc rien de tout cela ne mute
        // l'environnement du processus de test.
        let tableau_de_bord =
            |nom: &str| (nom == "MIKA_DASHBOARD_TOKEN").then(|| "jeton-tb".to_string());
        assert!(
            jeton_interne(&tableau_de_bord).is_err(),
            "le jeton du tableau de bord ne passe pas require_internal_token"
        );
        // Contrôle positif : la même lecture rend bien le jeton quand il est posé,
        // sans quoi le refus ci-dessus serait vrai par construction.
        assert_eq!(
            jeton_interne(
                &|nom: &str| (nom == "MIKA_INTERNAL_TOKEN").then(|| "jeton-interne".to_string())
            )
            .expect("un MIKA_INTERNAL_TOKEN posé se lit"),
            "jeton-interne"
        );
    }

    /// Le factice des trois portes est **emprunté** à `rewind_remote` (§ 2.7) :
    /// un second témoin de socket ouvrirait une seconde vérité sur « ce que le
    /// serveur a vu », dans un ticket qui existe pour retirer des duplications.
    use crate::commands::rewind_remote::tests::{RequeteRecue, fake_spirit};
    use std::sync::Mutex;

    /// Une source qui pose le jeton interne, pour les tests qui veulent voir les
    /// appels partir.
    fn avec_jeton(nom: &str) -> Option<String> {
        (nom == "MIKA_INTERNAL_TOKEN").then(|| "jeton-interne".to_string())
    }

    const APERCU_SERVI: &str = r#"{"messages":[{"id":41,"role":"user","content_snippet":"la question"}],
        "reversals":[],"tasks":[],"warnings":[],"blocked":false,"block_reason":null}"#;

    const RESULTAT_SERVI: &str = r#"{"messages_deleted":2,"reversals_applied":3,"reversals_skipped":1,
        "tasks_deleted":0,"tasks_cancelled":0,"warnings":[],
        "reversal_descriptions":["la fiche de Vincent"],"rewind_trace_id":"rw-7"}"#;

    /// Les chemins des trois portes, dans l'ordre où la couture les traverse.
    fn trois_portes(ancrage: &str) -> Vec<(&'static str, axum::http::StatusCode, String)> {
        vec![
            (
                "/api/v1/rewind/resolve",
                axum::http::StatusCode::OK,
                ancrage.to_string(),
            ),
            (
                "/api/v1/rewind/preview",
                axum::http::StatusCode::OK,
                APERCU_SERVI.to_string(),
            ),
            (
                "/api/v1/rewind/execute",
                axum::http::StatusCode::OK,
                RESULTAT_SERVI.to_string(),
            ),
        ]
    }

    /// Monte le factice sur `reponses` et fait passer un rembobinage dessus.
    async fn par_la_couture(
        app: &mut App<'_>,
        reponses: &[(&str, axum::http::StatusCode, String)],
        lire_env: SourceEnv<'_>,
        exchange_count: usize,
        after_message_id: Option<i64>,
    ) -> (String, Arc<Mutex<Vec<RequeteRecue>>>) {
        let empruntes = reponses
            .iter()
            .map(|(chemin, statut, corps)| (*chemin, *statut, corps.as_str()))
            .collect::<Vec<_>>();
        let (base, vues) = fake_spirit(&empruntes).await;
        let sortie =
            rewind_par_spirit(app, &base, lire_env, exchange_count, after_message_id).await;
        (sortie, vues)
    }

    /// T3 — sans jeton, aucune requête n'atteint le factice.
    #[tokio::test]
    async fn mika1727_v2c1b_sans_jeton_aucune_requete_ne_part() {
        let (mut app, _rx, _tmp) = test_app().await;
        let (sortie, vues) =
            par_la_couture(&mut app, &trois_portes("{}"), &|_| None, 1, None).await;
        assert!(
            sortie.contains("MIKA_INTERNAL_TOKEN"),
            "la sortie doit nommer la variable : {sortie}"
        );
        // Le refus tombe **avant** l'appel, et c'est la socket qui l'atteste : un
        // message exact ne dirait pas qu'aucun `resolve` n'est parti.
        assert_eq!(
            vues.lock().unwrap().len(),
            0,
            "aucune requête ne doit partir"
        );
    }

    /// T4 — `/undo` traverse les trois portes dans l'ordre, sous son agent, et
    /// rend les compteurs du résultat servi.
    #[tokio::test]
    async fn mika1727_v2c1b_undo_traverse_les_trois_portes_dans_lordre() {
        let (mut app, _rx, _tmp) = test_app().await;
        let ancrage = format!(
            r#"{{"session_id":"{}","after_message_id":40,"trace_ids":[]}}"#,
            app.session_id
        );
        let (sortie, vues) =
            par_la_couture(&mut app, &trois_portes(&ancrage), &avec_jeton, 1, None).await;

        let vues = vues.lock().unwrap();
        let chemins = vues.iter().map(|v| v.chemin.as_str()).collect::<Vec<_>>();
        assert_eq!(
            chemins,
            [
                "/api/v1/rewind/resolve",
                "/api/v1/rewind/preview",
                "/api/v1/rewind/execute"
            ],
            "les trois portes, dans cet ordre"
        );
        for vue in vues.iter() {
            assert!(
                vue.corps.contains(r#""agent":"mika""#),
                "l'agent de la TUI part dans les trois corps : {}",
                vue.corps
            );
        }
        assert!(sortie.contains("2 messages deleted"), "{sortie}");
        assert!(sortie.contains("3 reversals applied"), "{sortie}");
        // Les trois champs que l'attente `dead_code` de `ResultMirror` couvrait.
        assert!(sortie.contains("1 skipped"), "{sortie}");
        assert!(sortie.contains("la fiche de Vincent"), "{sortie}");
        assert!(sortie.contains("Trace: rw-7"), "{sortie}");
    }

    /// T5 — `/rewind to <id>` n'appelle pas `resolve`, et le corps d'`execute`
    /// ne porte **pas** de clé `originating_session_id`.
    #[tokio::test]
    async fn mika1727_v2c1b_rewind_to_nappelle_pas_resolve() {
        let (mut app, _rx, _tmp) = test_app().await;
        let (_sortie, vues) =
            par_la_couture(&mut app, &trois_portes("{}"), &avec_jeton, 0, Some(40)).await;

        let vues = vues.lock().unwrap();
        assert_eq!(vues.len(), 2, "aperçu puis exécution, et rien avant");
        assert_eq!(vues[0].chemin, "/api/v1/rewind/preview");
        assert_eq!(vues[1].chemin, "/api/v1/rewind/execute");
        // Lu sur la socket : un `None` côté Rust et une clé `null` sur le fil sont
        // indiscernables depuis l'appelant, et c'est cette distinction que le
        // commentaire d'`execute` déclare load-bearing.
        assert!(
            !vues[1].corps.contains("originating_session_id"),
            "pas de clé d'origine quand la session ne bouge pas : {}",
            vues[1].corps
        );
    }

    /// T6 — quand `resolve` rend une autre session, l'origine part avec celle de
    /// la TUI.
    #[tokio::test]
    async fn mika1727_v2c1b_une_autre_session_fait_partir_lorigine() {
        let (mut app, _rx, _tmp) = test_app().await;
        let ancrage = r#"{"session_id":"s-anterieure","after_message_id":40,"trace_ids":[]}"#;
        let (_sortie, vues) =
            par_la_couture(&mut app, &trois_portes(ancrage), &avec_jeton, 2, None).await;

        let vues = vues.lock().unwrap();
        let execute = vues
            .iter()
            .find(|v| v.chemin == "/api/v1/rewind/execute")
            .expect("la porte execute est atteinte");
        assert!(
            execute
                .corps
                .contains(&format!(r#""originating_session_id":"{}""#, app.session_id)),
            "l'origine est la session de la TUI : {}",
            execute.corps
        );
        // Et la session visée est bien celle que `resolve` a rendue, sans quoi
        // l'assertion ci-dessus passerait sur un rembobinage de la session courante.
        assert!(
            execute.corps.contains(r#""session_id":"s-anterieure""#),
            "{}",
            execute.corps
        );
    }

    /// T7 — un 422 de `preview` dit le refus et la raison du serveur, et rien
    /// n'est exécuté.
    #[tokio::test]
    async fn mika1727_v2c1b_un_refus_de_lapercu_nexecute_rien() {
        let (mut app, _rx, _tmp) = test_app().await;
        let mut reponses = trois_portes("{}");
        // La forme que le serveur sert sur un refus : un aperçu dont `blocked`
        // est vrai, sous un 422. Un corps d'erreur libre serait rendu
        // `Unexpected` par la porte — un diagnostic de version, pas un refus.
        reponses[1] = (
            "/api/v1/rewind/preview",
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"messages":[],"reversals":[],"tasks":[],"warnings":[],
                "blocked":true,"block_reason":"un verrou de session"}"#
                .to_string(),
        );
        let (sortie, vues) = par_la_couture(&mut app, &reponses, &avec_jeton, 0, Some(40)).await;

        assert!(sortie.contains("Rewind blocked"), "{sortie}");
        assert!(
            sortie.contains("un verrou de session"),
            "la raison du serveur est rendue : {sortie}"
        );
        let vues = vues.lock().unwrap();
        assert!(
            vues.iter().all(|v| v.chemin != "/api/v1/rewind/execute"),
            "un refus d'aperçu n'exécute rien"
        );
    }

    /// T8 — la garde structurelle : la TUI ne rembobine plus la base en local.
    ///
    /// Le scan porte sur les trois gestes qui *lisent ou écrivent le
    /// rembobinage en local*, et sur rien d'autre : `rewind::` reste importé
    /// pour `format_preview`, le formateur légitime, et `app.db` reste lu pour
    /// le rechargement.
    #[test]
    fn mika1727_v2c1b_la_tui_ne_rembobine_plus_la_base_en_local() {
        // Livrée vide, et vide par construction : quand la garde rougit, le
        // remède est la porte que son message nomme, jamais une entrée ici.
        const ALLOWED: &[&str] = &[];
        assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
        const MOTIFS: &[(&str, &str)] = &[
            ("find_recent_exchanges(", "rewind_remote::resolve"),
            ("preview_rewind(", "rewind_remote::preview"),
            ("execute_rewind(", "rewind_remote::execute"),
        ];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let site = scanner.src_root().join("tui/commands/handlers.rs");
        let production = scanner.production_of(&site);

        // Sans ce témoin, une garde qui balaie un chemin faux, ou un balayeur
        // qui masquerait le fichier entier, serait verte **par cécité** —
        // l'exact défaut que `ProductionScanner` existe pour refuser
        // (mika#2398). Le témoin vise un nom de la moitié *production* : s'il
        // disparaît, la garde rougit et demande qu'on la re-vise, au lieu de
        // continuer à ne rien regarder.
        assert!(
            production.contains("async fn handle_rewind_impl("),
            "la garde n'a pas lu la moitié production de handlers.rs"
        );

        for (motif, porte) in MOTIFS {
            let violations: Vec<String> = production
                .lines()
                .enumerate()
                .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(motif))
                .map(|(i, l)| format!("{}: {}", i + 1, l.trim()))
                .collect();
            assert!(
                violations.is_empty(),
                "mika#1727 V2c-1b — `{motif}` rembobine la base dans le processus de \
                 la TUI. Le remède est de passer par `{porte}`, jamais une entrée \
                 d'allowlist :\n{}",
                violations.join("\n")
            );
        }
    }

    // --- `/clear` par mika-spirit (mika#1727, vol V2d-4b) ---

    /// T-A — critère 2 : les deux portes, dans cet ordre, sous le jeton et sous
    /// l'agent de la TUI ; la session vient du fil, et le worker en reçoit une.
    #[tokio::test]
    async fn mika1727_v2d4b_clear_ouvre_puis_clot_dans_cet_ordre() {
        let (mut app, mut rx, _tmp) = test_app().await;
        let ancienne = app.session_id.clone();

        let (_, vues) = clear_nominal(&mut app).await;

        let vues = vues.lock().expect("le témoin du banc");
        let chemins = vues.iter().map(|v| v.chemin.as_str()).collect::<Vec<_>>();
        // L'ordre est le livrable de ce vol : `clore` d'abord perdrait
        // l'historique le jour où `ouvrir` échoue.
        assert_eq!(
            chemins,
            [ROUTE_SESSIONS, &format!("{ROUTE_SESSIONS}/{ancienne}/end")],
            "ouvrir puis clore, et rien d'autre"
        );
        for vue in vues.iter() {
            assert_eq!(vue.methode, "POST");
            assert_eq!(vue.autorisation.as_deref(), Some("Bearer jeton-interne"));
            assert!(vue.corps.contains(r#""agent":"mika""#), "{}", vue.corps);
        }

        // La session de la TUI est celle du fil, et non un UUID tiré ici : c'est
        // le geste que ce vol remplace.
        assert_eq!(app.session_id, "s-du-fil");
        let requests = drain_requests(&mut rx);
        assert_eq!(requests.len(), 1, "un seul NewSession");
        match &requests[0] {
            AgentRequest::NewSession { session_id } => assert_eq!(session_id, "s-du-fil"),
            _ => panic!("expected NewSession request"),
        }
    }

    /// T-B — critère 3 : sans jeton, le refus tombe **avant tout appel**, et
    /// c'est la socket qui l'atteste — un message exact ne dirait pas qu'aucun
    /// `ouvrir` n'est parti.
    #[tokio::test]
    async fn mika1727_v2d4b_sans_jeton_clear_refuse_avant_tout_appel() {
        let (mut app, mut rx, _tmp) = test_app().await;
        let ancienne = app.session_id.clone();
        app.messages.push(ChatMessage {
            role: ChatRole::User,
            content: "hello".to_string(),
            rendered: None,
            channel: None,
        });

        let fin = format!("{ROUTE_SESSIONS}/{ancienne}/end");
        let reponses = [
            (
                ROUTE_SESSIONS,
                axum::http::StatusCode::OK,
                CORPS_OUVERT.to_string(),
            ),
            (
                fin.as_str(),
                axum::http::StatusCode::OK,
                CORPS_CLOS.to_string(),
            ),
        ];
        let (sortie, vues) = par_le_banc(&mut app, &reponses, &|_| None).await;

        assert!(sortie.contains("MIKA_INTERNAL_TOKEN"), "{sortie}");
        assert_eq!(
            vues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );
        assert_eq!(app.session_id, ancienne, "la session ne bouge pas");
        assert_eq!(app.messages.len(), 1, "l'affichage reste");
        assert!(
            drain_requests(&mut rx).is_empty(),
            "le worker ne reçoit rien"
        );
    }

    /// T-C — critère 4 : un `ouvrir` refusé ne clôt rien.
    #[tokio::test]
    async fn mika1727_v2d4b_un_ouvrir_refuse_ne_clot_rien() {
        let (mut app, mut rx, _tmp) = test_app().await;
        let ancienne = app.session_id.clone();

        // Les deux routes sont servies ; seule l'ouverture refuse. Sans la
        // seconde armée, « aucun `end` n'est parti » serait vrai par
        // construction du banc.
        let fin = format!("{ROUTE_SESSIONS}/{ancienne}/end");
        let reponses = [
            (
                ROUTE_SESSIONS,
                axum::http::StatusCode::UNAUTHORIZED,
                String::new(),
            ),
            (
                fin.as_str(),
                axum::http::StatusCode::OK,
                CORPS_CLOS.to_string(),
            ),
        ];
        let (sortie, vues) = par_le_banc(&mut app, &reponses, &avec_jeton).await;

        // Un jeton posé mais faux est la panne la plus coûteuse à diagnostiquer :
        // le message nomme la variable, et non le transport.
        assert!(sortie.contains("MIKA_INTERNAL_TOKEN"), "{sortie}");
        let vues = vues.lock().expect("le témoin du banc");
        // Le **chemin** de la vue unique, et non son seul compte : un `len() == 1`
        // serait vert si la TUI avait appelé `clore` et rien d'autre.
        let chemins = vues.iter().map(|v| v.chemin.as_str()).collect::<Vec<_>>();
        assert_eq!(chemins, [ROUTE_SESSIONS], "l'ouverture, et rien après");
        assert_eq!(app.session_id, ancienne, "la session ne bouge pas");
        assert!(
            drain_requests(&mut rx).is_empty(),
            "le worker ne reçoit rien"
        );
    }

    /// T-D — critère 5 : une clôture en échec garde la nouvelle session, et le
    /// message nomme la cause.
    #[tokio::test]
    async fn mika1727_v2d4b_une_cloture_en_echec_garde_la_nouvelle_session() {
        let (mut app, mut rx, _tmp) = test_app().await;
        let ancienne = app.session_id.clone();

        // Le 500 des trois routes sur échec d'écriture : l'ancienne session est
        // là, le serveur l'a vue, et il n'a pas pu la clore.
        let fin = format!("{ROUTE_SESSIONS}/{ancienne}/end");
        let reponses = [
            (
                ROUTE_SESSIONS,
                axum::http::StatusCode::OK,
                CORPS_OUVERT.to_string(),
            ),
            (
                fin.as_str(),
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "disk full".to_string(),
            ),
        ];
        let (sortie, _) = par_le_banc(&mut app, &reponses, &avec_jeton).await;

        assert!(sortie.contains("not ended"), "{sortie}");
        assert!(sortie.contains(&ancienne), "{sortie}");
        // La cause vient du `Display` de `SessionRemoteIssue`, et non d'une
        // paraphrase écrite ici.
        assert!(sortie.contains("500"), "{sortie}");

        // Rien n'est défait : la nouvelle session est gardée, et le worker l'a.
        assert_eq!(app.session_id, "s-du-fil");
        let requests = drain_requests(&mut rx);
        assert_eq!(requests.len(), 1, "un seul NewSession");
        match &requests[0] {
            AgentRequest::NewSession { session_id } => assert_eq!(session_id, "s-du-fil"),
            _ => panic!("expected NewSession request"),
        }
    }

    /// T-E — la garde structurelle : la TUI ne touche plus la table `sessions`
    /// en local, et depuis V2d-4d elle ne compacte plus la conversation en
    /// local.
    ///
    /// Elle n'est **pas fondue** avec celle de V2c-1b. Les deux refusent des
    /// gestes différents pour des raisons différentes, et une garde fondue
    /// rougirait sous un seul message : celui qui la lirait au prochain vol ne
    /// saurait plus laquelle des deux portes lui est demandée.
    ///
    /// `maybe_compact(` est devenu un motif à V2d-4d, et pas avant : la porte
    /// `compacter` existait depuis V2d-3b mais rien ne l'atteignait, et une
    /// garde qui refuse un geste que rien ne remplace encore ne laisse d'autre
    /// issue qu'une entrée d'allowlist — exactement ce qu'elle existe pour
    /// éviter. Le motif arrive donc dans le commit qui câble
    /// `compact_par_spirit`, et son rouge-avant nommait la seule ligne de
    /// production qui appelait `maybe_compact`.
    ///
    /// Le nom de la garde reste `mika1727_v2d4b_…` : c'est une garde **par
    /// fichier**, pas par vol, et la renommer casserait la trace des trois vols
    /// qui la citent pour un gain nul.
    #[test]
    fn mika1727_v2d4b_la_tui_ne_clot_plus_la_base_en_local() {
        // Livrée vide, et vide par construction : quand la garde rougit, le
        // remède est la porte que son message nomme, jamais une entrée ici.
        const ALLOWED: &[&str] = &[];
        assert!(ALLOWED.is_empty(), "l'allowlist de cette garde reste vide");
        const MOTIFS: &[(&str, &str)] = &[
            ("create_session(", "session_remote::ouvrir"),
            ("end_session(", "session_remote::clore"),
            ("maybe_compact(", "session_remote::compacter"),
        ];

        let scanner =
            mika_common::source_guard::ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR"));
        let site = scanner.src_root().join("tui/commands/handlers.rs");
        let production = scanner.production_of(&site);

        // Témoin : sans lui, une garde qui balaie un chemin faux serait verte
        // **par cécité** — l'exact défaut que `ProductionScanner` existe pour
        // refuser (mika#2398).
        assert!(
            production.contains("async fn handle_clear("),
            "la garde n'a pas lu la moitié production de handlers.rs"
        );
        // Contre-témoin : `production_of` **masque** les régions `#[cfg(test)]`
        // sans jamais tronquer, donc la moitié test doit être absente alors que
        // tout ce qui la suit reste visible. Sans cette assertion, les deux
        // `create_session(` et `end_session(` des tests ci-dessus feraient
        // rougir la garde, et on l'aurait « réparée » par une allowlist.
        assert!(
            !production.contains("fn test_clear_singleton_does_not_end_session"),
            "la garde n'a pas masqué la moitié test de handlers.rs"
        );

        for (motif, porte) in MOTIFS {
            let violations: Vec<String> = production
                .lines()
                .enumerate()
                .filter(|(_, l)| !l.trim_start().starts_with("//") && l.contains(motif))
                .map(|(i, l)| format!("{}: {}", i + 1, l.trim()))
                .collect();
            // Le message dit « ce que mika-spirit décide » et non « la table
            // `sessions` » : depuis V2d-4d les trois motifs ne visent plus la
            // même table, et un rouge qui nommerait `sessions` sur
            // `maybe_compact(` enverrait son lecteur chercher la panne au
            // mauvais endroit.
            assert!(
                violations.is_empty(),
                "mika#1727 V2d-4b/V2d-4d — `{motif}` fait dans le processus de la TUI \
                 ce que mika-spirit décide. Le remède est de passer par `{porte}`, \
                 jamais une entrée d'allowlist :\n{}",
                violations.join("\n")
            );
        }
    }

    // --- `/compact` par mika-spirit (mika#1727, vol V2d-4d) ---

    /// La route telle que le banc doit la voir : `fake_spirit` associe par le
    /// chemin **brut**, et son `fallback` à 501 n'est traduit par aucun bras de
    /// `post`.
    const ROUTE_COMPACT: &str = "/api/v1/compact";

    /// Monte le banc sur `reponses` et fait passer un `/compact` dessus — le
    /// modèle de `par_le_banc` (l. 1635), pour `compact_par_spirit`. `app` est
    /// pris par référence partagée : c'est ce que la fonction mesurée demande.
    async fn par_le_banc_compact(
        app: &App<'_>,
        reponses: &[(&str, axum::http::StatusCode, String)],
        lire_env: SourceEnv<'_>,
    ) -> (String, Arc<Mutex<Vec<RequeteRecue>>>) {
        let empruntes = reponses
            .iter()
            .map(|(chemin, statut, corps)| (*chemin, *statut, corps.as_str()))
            .collect::<Vec<_>>();
        let (base, vues) = fake_spirit(&empruntes).await;
        let sortie = compact_par_spirit(app, &base, lire_env).await;
        (sortie, vues)
    }

    /// Le banc armé sur la seule route de compaction, en 200.
    fn compaction_servie(corps: &str) -> [(&'static str, axum::http::StatusCode, String); 1] {
        [(ROUTE_COMPACT, axum::http::StatusCode::OK, corps.to_string())]
    }

    /// Les deux comptes sont choisis pour ne pouvoir venir d'aucun comptage
    /// local : la base de `test_app()` ne porte ni sept ni soixante-quatre
    /// messages, donc un comptage résiduel côté client rendrait un autre nombre.
    const RIEN_A_COMPACTER: &str = r#"{"compacted":false,"messages":7}"#;
    const COMPACTION_DEMANDEE: &str = r#"{"compacted":true,"messages":64}"#;

    /// T-A — critère 1 : un agent qui n'est pas au repos refuse **sans aucun
    /// appel**. Le banc est monté *et* le jeton posé : sans les deux, « aucun
    /// appel » ne se distinguerait ni de « aucun serveur » ni de « aucun
    /// jeton ».
    #[tokio::test]
    async fn mika1727_v2d4d_un_agent_occupe_refuse_sans_aucun_appel() {
        let (mut app, _rx, _tmp) = test_app().await;
        app.status = AgentStatus::Thinking;

        let (sortie, vues) =
            par_le_banc_compact(&app, &compaction_servie(COMPACTION_DEMANDEE), &avec_jeton).await;

        assert!(
            sortie.contains("busy"),
            "le refus local dit l'occupation : {sortie}"
        );
        assert_eq!(
            vues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );
    }

    /// T-B — critère 1 : sans jeton, le refus tombe **avant tout appel** et
    /// nomme la variable à poser.
    ///
    /// Le contrôle positif est obligatoire : sans lui, un refus tombé pour
    /// toute autre raison — un agent qui ne serait pas au repos, un banc muet —
    /// rendrait ce test vert en mesurant autre chose que l'absence de jeton.
    #[tokio::test]
    async fn mika1727_v2d4d_sans_jeton_compact_refuse_avant_tout_appel_en_nommant_la_variable() {
        let (app, _rx, _tmp) = test_app().await;
        let reponses = compaction_servie(COMPACTION_DEMANDEE);

        let (sortie, vues) = par_le_banc_compact(&app, &reponses, &|_| None).await;
        assert!(sortie.contains("MIKA_INTERNAL_TOKEN"), "{sortie}");
        assert_eq!(
            vues.lock().expect("le témoin du banc").len(),
            0,
            "aucune requête ne doit partir"
        );

        // Contrôle positif : le même montage, le jeton posé, atteint la socket.
        let (_, vues) = par_le_banc_compact(&app, &reponses, &avec_jeton).await;
        assert_eq!(
            vues.lock().expect("le témoin du banc").len(),
            1,
            "le même montage avec jeton doit atteindre la socket"
        );
    }

    /// T-C — critère 2 : la route, la méthode, le jeton et l'agent, lus sur la
    /// socket.
    #[tokio::test]
    async fn mika1727_v2d4d_compact_poste_sur_la_route_de_compaction_avec_son_jeton_et_son_agent() {
        let (app, _rx, _tmp) = test_app().await;

        let (_, vues) =
            par_le_banc_compact(&app, &compaction_servie(COMPACTION_DEMANDEE), &avec_jeton).await;

        let vues = vues.lock().expect("le témoin du banc");
        assert_eq!(vues.len(), 1, "un seul appel, et sur la seule route");
        assert_eq!(vues[0].chemin, ROUTE_COMPACT);
        assert_eq!(vues[0].methode, "POST");
        assert_eq!(
            vues[0].autorisation.as_deref(),
            Some("Bearer jeton-interne")
        );
        // Le corps est **désérialisé** plutôt que cherché par sous-chaîne : le
        // serveur lit un champ, pas un ordre de clés, et un `contains` serait
        // vert sur un `agent` enfoui dans une autre valeur.
        let corps: serde_json::Value =
            serde_json::from_str(&vues[0].corps).expect("le corps posté est du JSON");
        assert_eq!(corps["agent"], serde_json::json!(app.agent_name));
    }

    /// T-D — critères 2 et 3 : « rien à compacter » porte le compte **rendu** et
    /// aucun seuil.
    #[tokio::test]
    async fn mika1727_v2d4d_rien_a_compacter_porte_le_compte_rendu_et_aucun_seuil() {
        let (app, _rx, _tmp) = test_app().await;

        let (sortie, _) =
            par_le_banc_compact(&app, &compaction_servie(RIEN_A_COMPACTER), &avec_jeton).await;

        assert!(sortie.contains("Nothing to compact"), "{sortie}");
        assert!(sortie.contains('7'), "le compte servi est rendu : {sortie}");
        // La moitié exécutable du critère 3 : plus aucun dénominateur, donc plus
        // aucun seuil reconnu côté client.
        assert!(
            !sortie.contains("50"),
            "aucun seuil n'est écrit côté client : {sortie}"
        );
    }

    /// T-E — critère 2 : la compaction est dite **demandée**, jamais affirmée
    /// faite. La grounding rule attestée par assertion, et non par relecture :
    /// une rédaction future qui reviendrait au passé rougira ici.
    #[tokio::test]
    async fn mika1727_v2d4d_une_compaction_est_demandee_jamais_affirmee_faite() {
        let (app, _rx, _tmp) = test_app().await;

        let (sortie, _) =
            par_le_banc_compact(&app, &compaction_servie(COMPACTION_DEMANDEE), &avec_jeton).await;

        assert!(sortie.contains("requested"), "{sortie}");
        assert!(
            sortie.contains("64"),
            "le compte d'avant est rendu : {sortie}"
        );
        let minuscule = sortie.to_lowercase();
        assert!(!minuscule.contains("compacted"), "{sortie}");
        assert!(!minuscule.contains("compactée"), "{sortie}");
    }

    /// T-F — critère 2 : le 409 dit l'occupation **et** le nouvel essai. Le
    /// `Display` de `Busy` dit la cause ; devant une TUI, c'est le remède qui
    /// décide de la seconde qui suit.
    #[tokio::test]
    async fn mika1727_v2d4d_un_409_dit_loccupation_et_le_nouvel_essai() {
        let (app, _rx, _tmp) = test_app().await;
        let reponses = [(
            ROUTE_COMPACT,
            axum::http::StatusCode::CONFLICT,
            String::new(),
        )];

        let (sortie, vues) = par_le_banc_compact(&app, &reponses, &avec_jeton).await;

        assert!(sortie.contains("busy"), "{sortie}");
        assert!(
            sortie.contains("again"),
            "le remède est dit, pas seulement la cause : {sortie}"
        );
        // La socket a bien été touchée : sans cette assertion, un refus local
        // rendrait ce test vert sans qu'aucun 409 soit jamais reçu.
        assert_eq!(
            vues.lock().expect("le témoin du banc").len(),
            1,
            "le 409 vient du serveur"
        );
    }

    /// T-G — critère 2 : toute autre erreur porte le `Display` du problème.
    ///
    /// Un 500 du banc plutôt qu'un point injoignable : c'est le montage déjà
    /// employé l. 3047-3068, aucun critère n'exige ici un `DeadEndpoint`, et il
    /// ne coûte aucun import.
    #[tokio::test]
    async fn mika1727_v2d4d_une_autre_erreur_porte_le_display_du_probleme() {
        let (app, _rx, _tmp) = test_app().await;
        let reponses = [(
            ROUTE_COMPACT,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "disk full".to_string(),
        )];

        let (sortie, _) = par_le_banc_compact(&app, &reponses, &avec_jeton).await;

        assert!(sortie.contains("Compaction failed"), "{sortie}");
        // La cause vient du `Display` de `SessionRemoteIssue`, et non d'une
        // paraphrase écrite ici.
        assert!(sortie.contains("500"), "{sortie}");
    }
}
