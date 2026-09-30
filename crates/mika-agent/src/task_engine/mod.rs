pub mod cron;
pub mod dispatcher;
pub mod engine;
pub mod liveness;
pub mod pilot_transcript;
pub mod process_kill;
pub mod process_liveness;
pub mod queue;
pub mod types;
pub mod worktree_activity;

pub use dispatcher::{DispatchError, TaskDispatcher};
// mika#2515 U2a — les clés de `metadata` que `classify_undelivered_verdict` lit.
// Ré-exportées plutôt que recopiées : l'écrivain possède le nom, le lecteur
// l'importe. Deux orthographes d'une même clé feraient répondre le classificateur
// et le compteur sur deux champs différents, en silence — la classe que
// `grooming_marker` a dû refermer une fois (mika#2158).
pub(crate) use dispatcher::{
    DELIVERY_ATTEMPTS_KEY, DELIVERY_QUARANTINED_AT_KEY, VERDICT_DELIVERY_DEFERRALS_KEY,
};
pub use engine::{TaskEngine, promoted_wrapper_liveness_secs, stuck_pending_reaper_grace_secs};
pub use queue::QueuedTask;
pub use types::{action_type, task_status, trigger_type};

use crate::async_db::AsyncDatabase;
use crate::db::NewTask;
use chrono::Timelike;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// Prune completed/failed/cancelled/expired tasks older than 30 days at startup
/// to prevent unbounded DB growth.
pub async fn prune_old_tasks(db: &AsyncDatabase) {
    // 30 days in seconds
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    if let Err(e) = db.prune_completed_tasks(THIRTY_DAYS_SECS).await {
        warn!("Failed to prune completed tasks: {}", e);
    }
}

/// Événement de journal (INFO) : une tentative > 1 a abouti (mika#2601).
///
/// **Régime attendu : NON VIDE, faible.** Chaque ligne est un enregistrement
/// que l'ancien code perdait jusqu'au redémarrage suivant — c'est la mesure
/// directe que le réessai mord. Sans elle, « aucune contention n'a eu lieu »
/// (le bon état) et « le classifieur est inerte » rendent des octets
/// identiques : la classe mika#2205, que ce dépôt a payée quatre fois.
pub(crate) const RECURRING_REGISTRATION_RETRIED_EVENT: &str = "recurring_registration_retried";

/// Événement de journal (ERROR) : le budget est épuisé, l'enregistrement est
/// perdu jusqu'au prochain redémarrage (mika#2601).
///
/// **Régime attendu : VIDE.** ERROR et non WARN parce que la conséquence n'est
/// **pas rattrapable** : aucune ligne n'a été créée donc rien ne tire, le
/// balayage des 60 ticks ne réenregistre aucune récurrente, et
/// `mika tasks rearm <label>` exige une ligne **morte** — il refuse
/// `RearmError::NoDeadRow`, « a rearm never creates a recurrence ex nihilo ».
/// Un WARN convient à ce qui se répare tout seul, pas à un scan mort jusqu'au
/// prochain redémarrage.
pub(crate) const RECURRING_REGISTRATION_FAILED_EVENT: &str = "recurring_registration_failed";

/// Trois tentatives, entrecoupées de 250 ms puis 1000 ms (mika#2601).
///
/// **Chaque tentative porte déjà les 5 s de `busy_timeout` de la connexion**
/// (`Database::open`), donc ce compte est un multiplicateur sur une unité de
/// 5 s, pas sur rien :
///
/// - régime nominal (verrou libre)  : 0 ms ajouté, une tentative ;
/// - le cas mesuré (un pic)         : ≤ 5,25 s, deuxième tentative servie ;
/// - pire cas par label             : 3 × 5 s + 1,25 s ≈ 16,25 s.
///
/// Pire cas sur un démarrage à 4 agents × 7 labels : ≈ 7 min 35 s, contre
/// ≈ 2 min 20 s aujourd'hui — et aujourd'hui les scans meurent pour 24 h. Un
/// démarrage de sept minutes est bruyant par lui-même, et chaque label produit
/// une ligne ERROR.
///
/// **Ce budget absorbe un pic ; il ne peut pas survivre à un `VACUUM`**, et il
/// faut le dire plutôt que de le laisser découvrir. Un verrou tenu 60 s ferait
/// échouer les trois tentatives de chaque label — on aurait payé sept minutes
/// pour rien. Le choix est donc délibéré : *absorber le pic à bas coût,
/// abandonner fort*. La population mesurée est **un** label par redémarrage,
/// c'est-à-dire un pic ; la sonde S3 est ce qui distingue les deux régimes, et
/// la cause-racine du régime soutenu a son propre suivi.
const PRODUCTION_ATTEMPTS: u32 = 3;
const PRODUCTION_BACKOFFS: [Duration; 2] =
    [Duration::from_millis(250), Duration::from_millis(1000)];

/// Budget de réessai de l'enregistrement d'une récurrente (mika#2601).
///
/// Injectable pour que le contrôle négatif d'AC3 emprunte le **même chemin de
/// code** que la production, au lieu d'en simuler un — idiome maison des points
/// d'entrée `*_with_deadline` réservés aux tests. Délibérément **pas** de
/// variable d'environnement (D1) : précédent `GH_AUTH_PROBE_TIMEOUT`, et un `0`
/// sur un budget de réessai serait un désarmement silencieux d'un filet de
/// sûreté.
#[derive(Debug, Clone)]
pub(crate) struct RecurringRetryPolicy {
    /// Nombre total de tentatives, réessais compris. Toujours ≥ 1.
    attempts: u32,
    /// Attente après la n-ième tentative ratée. Une liste plus courte que
    /// `attempts - 1` réutilise sa dernière valeur ; vide signifie aucune
    /// attente.
    backoffs: Vec<Duration>,
}

impl RecurringRetryPolicy {
    /// La politique en vigueur au démarrage : voir [`PRODUCTION_ATTEMPTS`].
    pub(crate) fn production() -> Self {
        Self {
            attempts: PRODUCTION_ATTEMPTS,
            backoffs: PRODUCTION_BACKOFFS.to_vec(),
        }
    }

    // NOTE: aucun constructeur réservé aux tests ici, et aucun attribut de
    // compilation conditionnelle dans cet `impl`. Les scans de source de ce
    // dépôt tronquent chaque fichier au **premier** marqueur de module de
    // test textuel, indentation comprise ; un attribut posé ici couperait le
    // fichier AVANT `ensure_recurring_task` et rendrait le scan R-8 aveugle
    // sur sa propre cible — en se lisant comme un arbre propre (classe
    // mika#2103 / mika#2205). Ce commentaire évite lui-même d'écrire la
    // séquence littérale, pour la même raison (faux positif mika#2050).
    // Le constructeur à une tentative vit donc dans `mod tests`, qui voit ces
    // champs privés en tant que module descendant.

    /// Attente à observer après la tentative `attempt` (1-indexée) ratée.
    fn backoff_after(&self, attempt: u32) -> Duration {
        let idx = attempt.saturating_sub(1) as usize;
        self.backoffs
            .get(idx)
            .or_else(|| self.backoffs.last())
            .copied()
            .unwrap_or(Duration::ZERO)
    }
}

/// Ce qu'une boucle de réessai a consommé, quelle que soit son issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetryReport {
    /// Tentatives réellement effectuées (≥ 1).
    pub attempts: u32,
    /// Somme des backoffs réellement dormis.
    pub waited: Duration,
}

/// L'enregistrement n'a pas pris et le budget est épuisé (mika#2601).
#[derive(Debug, thiserror::Error)]
#[error(
    "recurring task `{label}` was not registered after {} attempt(s) \
     ({} ms of backoff, busy={busy}): {error}",
    .report.attempts,
    .report.waited.as_millis()
)]
pub(crate) struct RegistrationFailure {
    pub label: String,
    pub report: RetryReport,
    /// L'échec était-il de la contention ? `false` signifie **un autre
    /// défaut** : c'est `error` qu'il faut lire, pas le budget qu'il faut
    /// rallonger.
    pub busy: bool,
    pub error: anyhow::Error,
}

/// Rejoue `op` tant que son erreur est une contention SQLite et que le budget
/// reste (mika#2601).
///
/// Écrit sur un `Result` **générique** plutôt que sur « busy uniquement »
/// délibérément : c'est ce qui rend « une erreur non-busy n'est pas rejouée »
/// une propriété de cette fonction, testable sans base, plutôt qu'une
/// conséquence de ce que son appelant veut bien lui passer.
async fn retry_on_busy<F, Fut>(
    policy: &RecurringRetryPolicy,
    mut op: F,
) -> Result<RetryReport, (RetryReport, bool, anyhow::Error)>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let mut waited = Duration::ZERO;
    let mut attempt: u32 = 1;
    loop {
        match op().await {
            Ok(()) => {
                return Ok(RetryReport {
                    attempts: attempt,
                    waited,
                });
            }
            Err(e) => {
                let busy = crate::db::is_sqlite_busy(&e);
                if !busy || attempt >= policy.attempts.max(1) {
                    return Err((
                        RetryReport {
                            attempts: attempt,
                            waited,
                        },
                        busy,
                        e,
                    ));
                }
                let backoff = policy.backoff_after(attempt);
                if !backoff.is_zero() {
                    tokio::time::sleep(backoff).await;
                    waited += backoff;
                }
                attempt += 1;
            }
        }
    }
}

/// Register a recurring task in the DB if one with the same label doesn't already exist.
/// If it already exists but the cron expression differs, update the cron and recompute
/// the next fire time.
///
/// Used at startup to ensure built-in tasks (heartbeat, reflection) are always registered.
///
/// **Calling this function *is* the config declaring the task must run** — the
/// callers are the boot paths that already evaluated the knob or the
/// `identity.toml` toggle. So a prior *config-driven* cancel of the same label
/// (the knob-off boot cancelled the row) must not survive as a veto: mika#2271
/// reverts it before re-registering. Terminal failures (`failed` / `expired`)
/// keep blocking through the mika#1742 refuse-to-zombie guard — only the
/// deliberate `cancelled` state is cleared here.
///
/// **Depuis mika#2601 l'enregistrement réessaie sous contention SQLite.** La
/// signature publique est inchangée, donc les huit sites d'appel ont un diff
/// nul : le ticket porte sur le réessai, pas sur la forme des appelants.
pub async fn ensure_recurring_task(
    db: &AsyncDatabase,
    label: &str,
    cron_expr: &str,
    action_config: &str,
) {
    let _ = ensure_recurring_task_with_policy(
        db,
        label,
        cron_expr,
        action_config,
        &RecurringRetryPolicy::production(),
    )
    .await;
}

/// [`ensure_recurring_task`] avec son budget de réessai explicite (mika#2601).
///
/// **L'unité réessayée est l'enregistrement ENTIER**, et c'est son idempotence
/// qui l'autorise plutôt qu'un raisonnement sur une écriture partielle :
/// `revert_config_cancel_recurring_task` exclut par son `WHERE` les lignes
/// portant déjà le marqueur (second passage ⇒ `n = 0`) ;
/// `create_recurring_task_if_absent` est deux `SELECT` puis un
/// `INSERT OR IGNORE`, en autocommit, **l'écriture en dernier**, donc un échec
/// laisse la base intacte ; `update_recurring_task_cron` est un `UPDATE` vers
/// une valeur fixe.
pub(crate) async fn ensure_recurring_task_with_policy(
    db: &AsyncDatabase,
    label: &str,
    cron_expr: &str,
    action_config: &str,
    policy: &RecurringRetryPolicy,
) -> Result<(), RegistrationFailure> {
    let outcome = retry_on_busy(policy, || {
        register_recurring_task_once(db, label, cron_expr, action_config)
    })
    .await;

    match outcome {
        Ok(report) => {
            if report.attempts > 1 {
                info!(
                    event = RECURRING_REGISTRATION_RETRIED_EVENT,
                    agent_id = %db.agent_id(),
                    label,
                    cron = cron_expr,
                    attempts = report.attempts,
                    waited_ms = report.waited.as_millis() as u64,
                    "recurring task registration succeeded after retrying under SQLite contention"
                );
            }
            Ok(())
        }
        Err((report, busy, error)) => {
            error!(
                event = RECURRING_REGISTRATION_FAILED_EVENT,
                agent_id = %db.agent_id(),
                label,
                cron = cron_expr,
                attempts = report.attempts,
                waited_ms = report.waited.as_millis() as u64,
                busy,
                error = %error,
                "recurring task registration LOST — nothing will fire this label \
                 until the next restart (mika#2601)"
            );
            Err(RegistrationFailure {
                label: label.to_string(),
                report,
                busy,
                error,
            })
        }
    }
}

/// Une tentative d'enregistrement (mika#2601).
///
/// **Une règle, un classifieur, quatre sites :** *une erreur busy avorte la
/// tentative (donc l'enregistrement entier est rejoué) ; toute autre erreur
/// garde la disposition d'aujourd'hui.* C'est un sur-ensemble strict du
/// comportement antérieur — aucune régression possible sur les erreurs
/// non-busy, qui continuent d'être avalées exactement comme avant.
///
/// **Son nom ne doit pas se terminer par celui de l'API publique suivi d'une
/// parenthèse ouvrante, et ce n'est pas une préférence de style.** La garde de
/// classe mika#2337 recense les *sites d'enregistrement* en cherchant ce motif
/// **en sous-chaîne** sur le texte brut de l'arbre, puis exige que le 4ᵉ
/// argument de chaque site soit un littéral — c'est ainsi qu'elle lit le
/// trigger déclaré et vérifie qu'il a un bras dans `dispatch_run_skill`. Un
/// helper nommé `try_` + le nom public est apparié par cette sous-chaîne : sa
/// **définition** est écartée (la garde saute les lignes portant `fn `) mais
/// **son appel** ne l'est pas, et il passe son `action_config` en *variable*
/// par construction — donc la garde halte en nommant un enregistrement dont
/// elle ne peut pas lire le destinataire. Renommer est le seul remède :
/// déplacer ce helper dans un autre fichier n'y change rien, la garde balayant
/// tout `src/` et non le seul `task_engine/mod.rs` que son message cite.
/// Le vocabulaire retenu est donc celui de la primitive de base
/// (`create_recurring_task_if_absent`), jamais celui de l'API publique.
async fn register_recurring_task_once(
    db: &AsyncDatabase,
    label: &str,
    cron_expr: &str,
    action_config: &str,
) -> anyhow::Result<()> {
    let agent_id = db.agent_id.clone();

    // mika#2271: knob-off cancelled this label; the caller now says it must run.
    // Clear the config-cancel veto so the mika#1742 guard doesn't refuse the
    // re-registration below.
    match db.revert_config_cancel_recurring_task(label).await {
        Ok(0) => {}
        Ok(n) => {
            info!(
                agent_id = %agent_id,
                label,
                rows = n,
                "reverted config cancel on recurring task (mika#2271)"
            )
        }
        Err(e) if crate::db::is_sqlite_busy(&e) => return Err(e),
        Err(e) => {
            warn!(agent_id = %agent_id, label, error = %e, "failed to revert config cancel on recurring task")
        }
    }

    let task = NewTask {
        agent_id: agent_id.clone(),
        team_run_id: None,
        parent_task_id: None,
        depth: 0,
        label: label.to_string(),
        trigger_type: "recurring".to_string(),
        cron_expr: Some(cron_expr.to_string()),
        event_source: None,
        event_offset_secs: None,
        condition_expr: None,
        next_fire_at: None,
        timeout_at: None,
        action_type: action_type::RUN_SKILL.to_string(),
        action_config: action_config.to_string(),
        input_context: None,
        created_by_session: None,
        created_trace_id: None,
        reference_url: None,
        source: None,
        metadata: None,
        r#type: None,
        dispatch_class: None,
    };

    match db.create_recurring_task_if_absent(task).await {
        Ok(Some(id)) => {
            info!(agent_id = %agent_id, label, task_id = %id, cron = cron_expr, "registered recurring task")
        }
        Ok(None) => {
            // Task already exists — check if the cron expression changed.
            match db.get_recurring_task_cron(label).await {
                Ok(Some(existing_cron)) => {
                    if existing_cron != cron_expr {
                        let now = crate::timestamp::now();
                        match cron::next_fire_from_cron(cron_expr, &now) {
                            Ok(next_fire) => {
                                match db
                                    .update_recurring_task_cron(label, cron_expr, &next_fire)
                                    .await
                                {
                                    Ok(_) => {
                                        info!(agent_id = %agent_id, label, old_cron = %existing_cron, new_cron = cron_expr, "updated recurring task cron")
                                    }
                                    Err(e) if crate::db::is_sqlite_busy(&e) => return Err(e),
                                    Err(e) => {
                                        warn!(agent_id = %agent_id, label, error = %e, "failed to update recurring task cron")
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(agent_id = %agent_id, label, cron = cron_expr, error = %e, "failed to compute next fire time for updated cron")
                            }
                        }
                    } else {
                        debug!(agent_id = %agent_id, label, "recurring task already registered, skipping");
                    }
                }
                // Une ligne sans cron lisible : rien à comparer, rien à dire.
                Ok(None) => {}
                Err(e) if crate::db::is_sqlite_busy(&e) => return Err(e),
                // Ce bras était ENTIÈREMENT muet avant mika#2601 (`if let
                // Ok(Some(..))`) : un cron qui ne se relit pas se lisait comme
                // un cron en phase.
                Err(e) => {
                    warn!(agent_id = %agent_id, label, error = %e, "failed to read recurring task cron")
                }
            }
        }
        Err(e) if crate::db::is_sqlite_busy(&e) => return Err(e),
        Err(e) => {
            warn!(agent_id = %agent_id, label, error = %e, "failed to register recurring task")
        }
    }

    Ok(())
}

/// Pourquoi `mika tasks rearm` a refusé (mika#2446 R-4/R-5).
///
/// Chaque refus nomme ce qui le fonde : un geste opérateur refusé sans
/// raison lisible est un geste qu'on contourne au jugé — en éditant la base,
/// précisément ce que la commande existe pour rendre inutile.
#[derive(Debug, thiserror::Error)]
pub enum RearmError {
    /// Aucune ligne récurrente morte pour ce label. Jamais de création ex
    /// nihilo : créer une récurrente depuis un label inconnu serait le seul
    /// geste capable d'introduire un trigger non routable.
    #[error(
        "no dead recurring task labelled `{label}` for agent `{agent_id}` — \
         nothing to re-arm (a rearm never creates a recurrence ex nihilo)"
    )]
    NoDeadRow { agent_id: String, label: String },
    /// Une ligne active existe déjà : il n'y a rien à ressusciter.
    #[error("recurring task `{label}` is already armed — nothing to re-arm")]
    AlreadyArmed { label: String },
    /// `ensure_recurring_task` n'enregistre que des `run_skill` ; ré-armer
    /// autre chose passerait par un chemin qui réécrirait l'action.
    #[error(
        "recurring task `{label}` (task {task_id}) has action_type `{action_type}`; \
         `mika tasks rearm` only re-registers run_skill recurrences"
    )]
    NotRunSkill {
        label: String,
        task_id: String,
        action_type: String,
    },
    /// L'`action_config` de la ligne morte ne porte pas de `trigger` lisible :
    /// sans trigger, la routabilité ne peut pas être établie — refus.
    #[error(
        "recurring task `{label}` (task {task_id}) carries no readable `trigger` in its \
         action_config — routability cannot be established, refusing"
    )]
    NoTrigger { label: String, task_id: String },
    /// R-5 / AC7 — le binaire courant ne sait pas router ce trigger.
    /// Ré-armer ne produirait qu'une mort de plus par trigger inconnu.
    #[error(
        "refusing to re-arm `{label}`: trigger `{trigger}` is not routable by this binary \
         (mika {version}, git {git_hash}); routable triggers: {routable_triggers}. \
         Re-arming would only produce another unknown-trigger death — deploy a binary \
         that carries the arm first."
    )]
    NotRoutable {
        label: String,
        trigger: String,
        version: String,
        git_hash: String,
        routable_triggers: String,
    },
    /// La ligne morte n'a pas de `cron_expr` : l'opérateur ne doit pas en
    /// retaper un, donc la commande refuse plutôt que d'en inventer un.
    #[error("recurring task `{label}` (task {task_id}) has no cron_expr to re-register with")]
    NoCron { label: String, task_id: String },
    /// Le marqueur est posé et tracé, mais la ré-inscription n'a pas pris —
    /// une autre garde a refusé ; le journal porte son WARN.
    #[error(
        "re-registration of `{label}` did not take after the operator lift — \
         read the mika#1742 warning in the log"
    )]
    NotRegistered { label: String },
    #[error(transparent)]
    Db(#[from] anyhow::Error),
}

/// Ce qu'un `mika tasks rearm` réussi a fait (mika#2446 R-4).
#[derive(Debug, Clone)]
pub struct RearmOutcome {
    /// Le label tel que stocké (la recherche est `COLLATE NOCASE`).
    pub label: String,
    pub trigger: String,
    pub cron_expr: String,
    /// La ligne morte la plus récente — celle dont la mort est absoute.
    pub dead_task_id: String,
    pub dead_status: String,
    /// Nombre de lignes mortes marquées par cet acte (toutes celles du label).
    pub rows_marked: usize,
}

/// Ré-arme une récurrente morte sans attendre la fenêtre de grâce
/// mika#1742 et sans édition manuelle de la base (mika#2446 R-4/R-5).
///
/// Séquence, et l'ordre est porteur :
/// 1. résoudre la ligne morte la plus récente du label — absente → refus,
///    jamais de création ex nihilo ;
/// 2. lire son `trigger` et **refuser s'il n'est pas routable par ce
///    binaire** (AC7), en nommant le trigger, la version, l'empreinte git et
///    l'inventaire ;
/// 3. **tracer l'acte AVANT de le poser** (`audit_events`,
///    `tool_name = 'recurring_operator_rearm'`) : un acte non tracé est ce que
///    ce geste refuse d'être, donc un audit illisible annule tout ;
/// 4. poser le marqueur sur les lignes mortes du label ;
/// 5. ré-enregistrer via [`ensure_recurring_task`] avec le `cron_expr` et
///    l'`action_config` lus **sur la ligne morte**, puis vérifier qu'une ligne
///    active existe.
///
/// **Refusé : une exemption automatique au second décès.** Ce serait désarmer
/// mika#1742 pour toute la classe. Le ré-armement est un acte explicite,
/// imputable, et chaque décès postérieur retrouve un veto armé.
pub async fn rearm_recurring_task(
    db: &AsyncDatabase,
    label: &str,
) -> Result<RearmOutcome, RearmError> {
    if db.get_recurring_task_cron(label).await?.is_some() {
        return Err(RearmError::AlreadyArmed {
            label: label.to_string(),
        });
    }

    let target = db
        .find_recurring_rearm_target(label)
        .await?
        .ok_or_else(|| RearmError::NoDeadRow {
            agent_id: db.agent_id.clone(),
            label: label.to_string(),
        })?;

    // La recherche est insensible à la casse ; la vérification d'activité
    // ci-dessus lit le label tapé. On la refait sur l'orthographe stockée.
    if target.label != label && db.get_recurring_task_cron(&target.label).await?.is_some() {
        return Err(RearmError::AlreadyArmed {
            label: target.label.clone(),
        });
    }

    if target.action_type != action_type::RUN_SKILL {
        return Err(RearmError::NotRunSkill {
            label: target.label.clone(),
            task_id: target.task_id.clone(),
            action_type: target.action_type.clone(),
        });
    }

    let trigger = serde_json::from_str::<serde_json::Value>(&target.action_config)
        .ok()
        .and_then(|v| v.get("trigger").and_then(|t| t.as_str()).map(str::to_owned))
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| RearmError::NoTrigger {
            label: target.label.clone(),
            task_id: target.task_id.clone(),
        })?;

    // AC7 — le lecteur unique de l'inventaire pour les prédicats.
    if !dispatcher::is_routable_trigger(&trigger) {
        let attribution = dispatcher::binary_attribution();
        warn!(
            event = "recurring_operator_rearm_refused",
            label = %target.label,
            trigger = %trigger,
            binary_version = %attribution.version,
            binary_git_hash = %attribution.git_hash,
            routable_triggers = %attribution.routable_triggers,
            "mika#2446: rearm refused — trigger not routable by this binary"
        );
        return Err(RearmError::NotRoutable {
            label: target.label.clone(),
            trigger,
            version: attribution.version.to_string(),
            git_hash: attribution.git_hash.to_string(),
            routable_triggers: attribution.routable_triggers,
        });
    }

    let cron_expr = target
        .cron_expr
        .clone()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| RearmError::NoCron {
            label: target.label.clone(),
            task_id: target.task_id.clone(),
        })?;

    // Tracer AVANT de poser : si la trace ne s'écrit pas, rien n'est changé.
    let attribution = dispatcher::binary_attribution();
    db.log_audit_event(
        &format!("system-{}", db.agent_id()),
        "recurring_operator_rearm",
        &format!("label:{}", target.label),
        Some(&target.status),
        Some("rearmed"),
        Some(&format!(
            "dead_task:{} trigger:{} cron:{} dead_updated_at:{} \
             binary_version:{} binary_git_hash:{}",
            target.task_id,
            trigger,
            cron_expr,
            target.updated_at,
            attribution.version,
            attribution.git_hash,
        )),
        None,
    )
    .await?;

    let rows_marked = db.mark_recurring_operator_rearm(&target.label).await?;

    ensure_recurring_task(db, &target.label, &cron_expr, &target.action_config).await;

    if db.get_recurring_task_cron(&target.label).await?.is_none() {
        return Err(RearmError::NotRegistered {
            label: target.label.clone(),
        });
    }

    info!(
        event = "recurring_operator_rearm",
        label = %target.label,
        trigger = %trigger,
        cron = %cron_expr,
        dead_task_id = %target.task_id,
        dead_status = %target.status,
        rows_marked,
        "mika#2446: recurring task re-armed by operator"
    );

    Ok(RearmOutcome {
        label: target.label,
        trigger,
        cron_expr,
        dead_task_id: target.task_id,
        dead_status: target.status,
        rows_marked,
    })
}

/// Check if heartbeat is enabled for the agent from identity.toml config.
/// Returns `true` (default) unless `[heartbeat] enabled = false`.
pub async fn heartbeat_enabled_for_agent(home_dir: &Path) -> bool {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    identity
        .heartbeat
        .as_ref()
        .map(|c| c.enabled)
        .unwrap_or(true)
}

/// Build a UTC cron expression for reflection from identity.toml config + customer timezone.
/// Returns `None` if reflection is disabled or not configured.
pub async fn reflection_cron_for_agent(home_dir: &Path, db: &AsyncDatabase) -> Option<String> {
    let identity = crate::prompt::load_identity_async(home_dir).await;
    let config = identity.reflection.as_ref().filter(|c| c.enabled)?;
    let local_time = config.parse_time()?;

    let tz_str = if let Some(ref tz) = config.timezone {
        tz.clone()
    } else {
        db.get_customer_config("timezone")
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| "UTC".to_string())
    };
    let tz: chrono_tz::Tz = match tz_str.parse() {
        Ok(tz) => tz,
        Err(_) => {
            warn!(timezone = %tz_str, "invalid timezone in customer config, skipping reflection registration");
            return None;
        }
    };

    // Convert local time to UTC: pick today's date, attach the local time,
    // convert to UTC, extract hour/minute.
    // NOTE: DST drift — the UTC offset is computed from today's date. For timezones with
    // daylight saving time, the reflection may fire ~1 hour early or late after a DST
    // transition until the next restart. This is acceptable for daily reflections.
    let today = chrono::Utc::now().with_timezone(&tz).date_naive();
    let local_dt = today.and_time(local_time);
    let utc_dt = local_dt.and_local_timezone(tz).earliest()?;
    let utc_time = utc_dt.with_timezone(&chrono::Utc).time();

    Some(format!("0 {} {} * * *", utc_time.minute(), utc_time.hour()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    const FEEDER_LABEL: &str = "auto_pull_groomed";
    const FEEDER_CRON: &str = "0 */20 * * * *";
    const FEEDER_CONFIG: &str = r#"{"trigger":"auto_pull_groomed"}"#;

    fn test_async_db() -> AsyncDatabase {
        AsyncDatabase::new(Database::open_in_memory().unwrap())
    }

    async fn statuses_for(db: &AsyncDatabase, label: &str) -> Vec<String> {
        db.get_tasks_by_status(vec![
            "recurring_active".to_string(),
            "pending".to_string(),
            "in_progress".to_string(),
            "cancelled".to_string(),
            "failed".to_string(),
            "expired".to_string(),
        ])
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.label == label)
        .map(|t| t.status)
        .collect()
    }

    /// **Porte mika#2271 — test négatif.** Invariant : *un cycle knob-off →
    /// knob-on ré-inscrit le feeder*. Le knob-off boot annule la task
    /// récurrente ; le knob-on boot rappelle `ensure_recurring_task`, ce qui
    /// **est** la config déclarant que la task doit tourner. La garde
    /// refuse-to-zombie (mika#1742) ne doit pas transformer ce cancel
    /// délibéré en veto permanent.
    ///
    /// Sans le fix, le second `ensure_recurring_task` est refusé par la garde
    /// et le seul statut restant est `cancelled` — la boucle n'est plus
    /// réalimentée (symptôme mesuré le 2026-09-09).
    #[tokio::test]
    async fn knob_off_then_on_reregisters_the_feeder() {
        let db = test_async_db();

        // Boot 1 — knob absent : le feeder s'inscrit.
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;
        assert_eq!(
            statuses_for(&db, FEEDER_LABEL).await,
            vec!["recurring_active".to_string()],
            "boot initial : le feeder doit être inscrit"
        );

        // Boot 2 — MIKA_DEV_AUTO_PULL=0 : la branche knob-off annule la row.
        db.cancel_recurring_task_by_label(FEEDER_LABEL)
            .await
            .unwrap();
        assert_eq!(
            statuses_for(&db, FEEDER_LABEL).await,
            vec!["cancelled".to_string()],
            "knob-off : la row doit être annulée"
        );

        // Boot 3 — knob retiré : le feeder doit revenir.
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert!(
            statuses.iter().any(|s| s == "recurring_active"),
            "knob-on : le feeder doit être RÉ-INSCRIT (recurring_active), \
             pas laissé cancelled — statuts observés : {statuses:?}"
        );
    }

    /// Contrôle positif de la garde : un `failed` récent bloque toujours la
    /// ré-inscription. L'exemption mika#2271 ne vise que le cancel de config —
    /// elle ne doit pas désarmer la protection anti-zombie de mika#1742.
    #[tokio::test]
    async fn recent_failed_still_blocks_reregistration() {
        let db = test_async_db();
        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let label = FEEDER_LABEL.to_string();
        db.with_db(move |d| {
            d.conn.execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE label = ?1",
                rusqlite::params![label],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        ensure_recurring_task(&db, FEEDER_LABEL, FEEDER_CRON, FEEDER_CONFIG).await;

        let statuses = statuses_for(&db, FEEDER_LABEL).await;
        assert_eq!(
            statuses,
            vec!["failed".to_string()],
            "un échec terminal récent doit toujours bloquer la ré-inscription \
             (mika#1742) — statuts observés : {statuses:?}"
        );
    }

    // ── mika#2446 — `mika tasks rearm <label>` (AC6 / AC7) ──────────────

    const REAP_LABEL: &str = "worktree_reap";
    const REAP_CRON: &str = "0 */10 * * * *";
    const REAP_CONFIG: &str = r#"{"trigger":"worktree_reap"}"#;

    /// Inscrit `label` puis le fait mourir `failed` dans la fenêtre de grâce,
    /// de cause quelconque (non marquée) — l'état qui arme le veto mika#1742.
    async fn kill_in_window(db: &AsyncDatabase, label: &str, cron: &str, config: &str) {
        ensure_recurring_task(db, label, cron, config).await;
        let label = label.to_string();
        db.with_db(move |d| {
            d.conn.execute(
                "UPDATE tasks SET status = 'failed',
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')
                 WHERE label = ?1",
                rusqlite::params![label],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// AC6 — le ré-armement lève le veto sans attendre 24 h ni éditer la
    /// base, ré-inscrit avec le cron lu sur la ligne morte, et trace l'acte.
    #[tokio::test]
    async fn mika2446_rearm_revives_a_dead_recurrence_and_traces_the_act() {
        let db = test_async_db();
        kill_in_window(&db, REAP_LABEL, REAP_CRON, REAP_CONFIG).await;

        // Précondition : le chemin nominal (redémarrage) est refusé par le veto.
        ensure_recurring_task(&db, REAP_LABEL, REAP_CRON, REAP_CONFIG).await;
        assert_eq!(
            statuses_for(&db, REAP_LABEL).await,
            vec!["failed".to_string()]
        );

        let outcome = rearm_recurring_task(&db, REAP_LABEL)
            .await
            .expect("un trigger routable doit être ré-armé");
        assert_eq!(outcome.label, REAP_LABEL);
        assert_eq!(outcome.trigger, "worktree_reap");
        assert_eq!(outcome.cron_expr, REAP_CRON);
        assert_eq!(outcome.dead_status, "failed");
        assert_eq!(outcome.rows_marked, 1);

        let statuses = statuses_for(&db, REAP_LABEL).await;
        assert!(
            statuses.iter().any(|s| s == "recurring_active"),
            "la récurrente doit être ré-inscrite — statuts : {statuses:?}"
        );
        assert_eq!(
            db.get_recurring_task_cron(REAP_LABEL)
                .await
                .unwrap()
                .as_deref(),
            Some(REAP_CRON),
            "le cron est lu sur la ligne morte, jamais retapé"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("recurring_operator_rearm")
                .await
                .unwrap(),
            1,
            "l'acte doit être tracé dans audit_events"
        );
    }

    /// AC7 — un trigger que ce binaire ne sait pas router est refusé, en
    /// nommant le trigger et l'inventaire ; rien n'est tracé, marqué ni
    /// ré-inscrit (ré-armer ne produirait qu'une mort de plus).
    #[tokio::test]
    async fn mika2446_rearm_refuses_a_trigger_this_binary_cannot_route() {
        let db = test_async_db();
        kill_in_window(&db, "zorglub_scan", REAP_CRON, r#"{"trigger":"zorglub"}"#).await;

        let err = rearm_recurring_task(&db, "zorglub_scan")
            .await
            .expect_err("un trigger non routable doit être refusé");
        match &err {
            RearmError::NotRoutable {
                trigger,
                routable_triggers,
                ..
            } => {
                assert_eq!(trigger, "zorglub");
                assert_eq!(routable_triggers, &dispatcher::routable_triggers_csv());
            }
            other => panic!("attendu NotRoutable, obtenu {other:?}"),
        }
        let rendered = err.to_string();
        assert!(rendered.contains("zorglub"), "le refus nomme le trigger");
        assert!(
            rendered.contains("worktree_reap"),
            "le refus nomme l'inventaire routable : {rendered}"
        );

        assert_eq!(
            statuses_for(&db, "zorglub_scan").await,
            vec!["failed".to_string()]
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("recurring_operator_rearm")
                .await
                .unwrap(),
            0,
            "un refus ne trace pas d'acte"
        );
        // Aucun marqueur posé : le chemin nominal reste refusé par le veto.
        ensure_recurring_task(&db, "zorglub_scan", REAP_CRON, r#"{"trigger":"zorglub"}"#).await;
        assert_eq!(
            statuses_for(&db, "zorglub_scan").await,
            vec!["failed".to_string()]
        );
    }

    // ── mika#2601 — la boucle de réessai sous contention (V2) ───────────
    //
    // Unitaire, sans base : `retry_on_busy` est écrit sur un `Result`
    // générique précisément pour que « une erreur non-busy n'est pas
    // rejouée » soit une propriété de la fonction et non de son appelant.

    /// Une seule tentative, aucune attente — le comportement d'avant
    /// mika#2601, utilisé par le contrôle négatif d'AC3.
    ///
    /// Vit ici plutôt que dans l'`impl` de production : voir la note à cet
    /// endroit, un `#[cfg(test)]` indenté y tronquerait les scans de source.
    impl RecurringRetryPolicy {
        fn single_attempt() -> Self {
            Self {
                attempts: 1,
                backoffs: Vec::new(),
            }
        }
    }

    fn busy_err() -> anyhow::Error {
        anyhow::Error::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("database is locked".to_string()),
        ))
    }

    fn not_busy_err() -> anyhow::Error {
        anyhow::Error::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL),
            Some("NOT NULL constraint failed".to_string()),
        ))
    }

    /// Réessaie sous busy, s'arrête au premier `Ok`, et `waited_ms` est bien
    /// la somme des backoffs réellement dormis.
    #[tokio::test(start_paused = true)]
    async fn mika2601_la_boucle_reessaie_sous_busy_et_sarrete_au_premier_ok() {
        let policy = RecurringRetryPolicy::production();
        let calls = std::cell::Cell::new(0u32);

        let report = retry_on_busy(&policy, || {
            calls.set(calls.get() + 1);
            let n = calls.get();
            async move { if n < 2 { Err(busy_err()) } else { Ok(()) } }
        })
        .await
        .expect("la deuxième tentative aboutit");

        assert_eq!(calls.get(), 2, "exactement deux tentatives");
        assert_eq!(report.attempts, 2);
        assert_eq!(
            report.waited, PRODUCTION_BACKOFFS[0],
            "un seul backoff dormi : celui qui suit la première tentative"
        );
    }

    /// **Contrôle négatif porteur.** Une erreur qui n'est pas de la
    /// contention ne doit pas être rejouée — sinon « la boucle réessaie sous
    /// busy » serait indistinguable de « la boucle réessaie tout ».
    #[tokio::test(start_paused = true)]
    async fn mika2601_une_erreur_non_busy_nest_jamais_rejouee() {
        let policy = RecurringRetryPolicy::production();
        let calls = std::cell::Cell::new(0u32);

        let (report, busy, _err) = retry_on_busy(&policy, || {
            calls.set(calls.get() + 1);
            async move { Err(not_busy_err()) }
        })
        .await
        .expect_err("une erreur non-busy est rendue telle quelle");

        assert_eq!(calls.get(), 1, "une seule tentative, aucun réessai");
        assert_eq!(report.attempts, 1);
        assert_eq!(report.waited, Duration::ZERO);
        assert!(!busy, "l'échec doit être rapporté comme non-contention");
    }

    /// Le budget est borné : après N tentatives busy, on abandonne — et
    /// `waited` porte la somme des deux backoffs.
    #[tokio::test(start_paused = true)]
    async fn mika2601_le_budget_est_borne_et_labandon_est_date() {
        let policy = RecurringRetryPolicy::production();
        let calls = std::cell::Cell::new(0u32);

        let (report, busy, _err) = retry_on_busy(&policy, || {
            calls.set(calls.get() + 1);
            async move { Err(busy_err()) }
        })
        .await
        .expect_err("trois tentatives busy épuisent le budget");

        assert_eq!(calls.get(), PRODUCTION_ATTEMPTS);
        assert_eq!(report.attempts, PRODUCTION_ATTEMPTS);
        assert!(busy, "l'abandon doit être attribué à la contention");
        assert_eq!(
            report.waited,
            PRODUCTION_BACKOFFS[0] + PRODUCTION_BACKOFFS[1],
            "les deux backoffs ont été dormis, et pas un troisième"
        );
    }

    /// Une politique à une seule tentative ne dort jamais — c'est le
    /// comportement d'avant mika#2601, et le contrôle négatif d'AC3.
    #[tokio::test(start_paused = true)]
    async fn mika2601_une_politique_a_une_tentative_ne_reessaie_pas() {
        let policy = RecurringRetryPolicy::single_attempt();
        let calls = std::cell::Cell::new(0u32);

        let (report, busy, _err) = retry_on_busy(&policy, || {
            calls.set(calls.get() + 1);
            async move { Err(busy_err()) }
        })
        .await
        .expect_err("aucun réessai disponible");

        assert_eq!(calls.get(), 1);
        assert_eq!(report.attempts, 1);
        assert_eq!(report.waited, Duration::ZERO);
        assert!(busy);
    }

    // ── mika#2601 — AC3 : contention réelle, deux agents (V3) ───────────

    /// Ouvre une base **sur fichier** et abaisse son `busy_timeout`.
    ///
    /// **Pourquoi l'abaisser, et il faut le dire :** sans ça les 5 s de
    /// production absorberaient les ~600 ms de verrou et **le réessai ne
    /// serait pas exercé du tout** — il faudrait tenir le verrou > 5 s, soit
    /// un test de six secondes sur chaque `cargo test`. Ce que ce montage
    /// mesure est le *mécanisme* (une erreur busy est classée, réessayée, et
    /// l'enregistrement atterrit) ; l'arithmétique de production vit dans le
    /// doc-comment de [`PRODUCTION_ATTEMPTS`], pas ici.
    async fn open_registrant(path: &std::path::Path, agent_id: &str) -> AsyncDatabase {
        let db = Database::open(path).expect("ouverture de la base de test");
        // `PRAGMA busy_timeout = N` REND UNE LIGNE : `execute_sql` (qui passe
        // par `Connection::execute`) échouerait sur `ExecuteReturnedResults`.
        db.query_scalar::<i64>("PRAGMA busy_timeout = 200", &[])
            .expect("le pragma doit se poser");
        let handle = AsyncDatabase::new_with_agent(db, agent_id);
        // **Obligatoire, et hors contention.** `tasks.agent_id REFERENCES
        // agents(id)` et la migration ne seede que `'mika'` ; or
        // `create_recurring_task_if_absent` insère en `INSERT OR IGNORE`, qui
        // **avale une violation de FK** et rend `Ok(None)` — indistinguable de
        // « la ligne existait déjà ». Sans cet enregistrement le montage
        // mesurerait une FK manquante en croyant mesurer un verrou.
        handle
            .register_agent(agent_id, agent_id, "")
            .await
            .expect("enregistrement de l'agent de test");
        handle
    }

    /// Prend le verrou d'écriture pour `hold`, puis le relâche.
    ///
    /// En WAL les deux `SELECT` de `create_recurring_task_if_absent`
    /// **aboutissent** (les lecteurs ne sont pas bloqués) et l'échec tombe sur
    /// l'`INSERT` — très exactement la forme mesurée en production.
    ///
    /// **Rend la main une fois le verrou RÉELLEMENT pris.** Sans ce
    /// rendez-vous, le registrant court contre le `BEGIN IMMEDIATE` du thread
    /// et peut écrire avant lui : le test passerait alors sans avoir jamais
    /// rencontré de contention, c'est-à-dire en ne mesurant rien.
    fn hold_write_lock(path: std::path::PathBuf, hold: Duration) -> std::thread::JoinHandle<()> {
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            let conn = rusqlite::Connection::open(&path).expect("connexion du bloqueur");
            conn.execute_batch("PRAGMA journal_mode = WAL;")
                .expect("wal");
            conn.execute_batch("BEGIN IMMEDIATE;")
                .expect("le bloqueur prend le verrou d'écriture");
            acquired_tx.send(()).expect("signal de prise du verrou");
            std::thread::sleep(hold);
            conn.execute_batch("COMMIT;")
                .expect("relâchement du verrou");
        });
        acquired_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("le bloqueur doit avoir pris le verrou avant qu'on enregistre");
        handle
    }

    async fn active_recurring_labels(db: &AsyncDatabase, label: &str) -> Vec<String> {
        db.get_tasks_by_status(vec!["recurring_active".to_string()])
            .await
            .unwrap()
            .into_iter()
            .filter(|t| t.label == label && t.agent_id == db.agent_id)
            .map(|t| t.agent_id)
            .collect()
    }

    /// AC3 — deux agents enregistrent concurremment pendant qu'un tiers tient
    /// le verrou d'écriture ; les deux doivent finir `recurring_active`.
    #[tokio::test]
    async fn mika2601_un_enregistrement_concurrent_sous_verrou_aboutit_pour_chaque_agent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mika.db");
        // Première ouverture : migration, hors contention.
        drop(Database::open(&path).expect("migration initiale"));

        // Les deux registrants sont ouverts et enregistrés AVANT que le verrou
        // soit pris : ce qu'on met sous contention est l'enregistrement de la
        // récurrente, rien d'autre.
        let db_a = open_registrant(&path, "agent-a").await;
        let db_b = open_registrant(&path, "agent-b").await;
        let blocker = hold_write_lock(path.clone(), Duration::from_millis(600));
        let policy = RecurringRetryPolicy::production();

        let (ra, rb) = tokio::join!(
            ensure_recurring_task_with_policy(
                &db_a,
                FEEDER_LABEL,
                FEEDER_CRON,
                FEEDER_CONFIG,
                &policy
            ),
            ensure_recurring_task_with_policy(
                &db_b,
                FEEDER_LABEL,
                FEEDER_CRON,
                FEEDER_CONFIG,
                &policy
            ),
        );
        blocker.join().expect("le bloqueur se termine");

        assert!(
            ra.is_ok(),
            "agent-a : l'enregistrement doit aboutir — {:?}",
            ra.err().map(|e| e.to_string())
        );
        assert!(
            rb.is_ok(),
            "agent-b : l'enregistrement doit aboutir — {:?}",
            rb.err().map(|e| e.to_string())
        );
        assert_eq!(
            active_recurring_labels(&db_a, FEEDER_LABEL).await,
            vec!["agent-a".to_string()],
            "agent-a doit porter une ligne recurring_active"
        );
        assert_eq!(
            active_recurring_labels(&db_b, FEEDER_LABEL).await,
            vec!["agent-b".to_string()],
            "agent-b doit porter une ligne recurring_active"
        );
    }

    /// **Contrôle négatif d'AC3 : « sans le réessai, le test rougit ».**
    ///
    /// Le *même* montage via [`ensure_recurring_task_with_policy`] à une seule
    /// tentative — pas une simulation : le même chemin de code, un budget
    /// différent. Aucune ligne `recurring_active`, et l'appel rend son
    /// [`RegistrationFailure`] en nommant la contention.
    #[tokio::test]
    async fn mika2601_sans_reessai_lenregistrement_est_perdu() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mika.db");
        drop(Database::open(&path).expect("migration initiale"));

        let db = open_registrant(&path, "agent-a").await;
        let blocker = hold_write_lock(path.clone(), Duration::from_millis(600));
        let res = ensure_recurring_task_with_policy(
            &db,
            FEEDER_LABEL,
            FEEDER_CRON,
            FEEDER_CONFIG,
            &RecurringRetryPolicy::single_attempt(),
        )
        .await;
        blocker.join().expect("le bloqueur se termine");

        let failure = res.expect_err("sans réessai, l'enregistrement est perdu");
        assert_eq!(failure.label, FEEDER_LABEL);
        assert_eq!(failure.report.attempts, 1);
        assert!(
            failure.busy,
            "l'échec doit être attribué à la contention, pas à un autre défaut : {}",
            failure.error
        );
        assert!(
            active_recurring_labels(&db, FEEDER_LABEL).await.is_empty(),
            "aucune ligne recurring_active ne doit exister"
        );
    }

    /// Jamais de création ex nihilo, et rien à ré-armer sur une ligne vivante.
    #[tokio::test]
    async fn mika2446_rearm_refuses_unknown_and_already_armed_labels() {
        let db = test_async_db();
        assert!(matches!(
            rearm_recurring_task(&db, REAP_LABEL).await,
            Err(RearmError::NoDeadRow { .. })
        ));
        assert!(statuses_for(&db, REAP_LABEL).await.is_empty());

        ensure_recurring_task(&db, REAP_LABEL, REAP_CRON, REAP_CONFIG).await;
        assert!(matches!(
            rearm_recurring_task(&db, REAP_LABEL).await,
            Err(RearmError::AlreadyArmed { .. })
        ));
    }
}
