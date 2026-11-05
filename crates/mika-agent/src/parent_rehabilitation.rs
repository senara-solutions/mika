//! Une parente `self_dev` fauchée par le reaper de groom redevient dispatchable
//! quand un plan a convergé (mika#3044).
//!
//! # Le vol, en une phrase
//!
//! Quand la garde d'état de [`crate::skills::executor::validate_dispatch_readiness`]
//! s'apprête à refuser une parente `self_dev` en `failed`, [`decide`] demande si
//! cet état terminal est exactement un reap de groom que la suite des événements
//! a démenti — dernière transition = `groom_parent_orphaned`, puis un enfant de
//! groom **de cette parente** livré après lui avec `PLAN_GROOMED`. Si oui, la
//! parente est réhabilitée `failed` → `in_progress` **après que toutes les autres
//! gardes ont passé**, puis dispatchée. Dans tous les autres cas, le refus
//! `task_not_dispatchable` d'aujourd'hui sort mot pour mot.
//!
//! Ce n'est pas rouvrir les causes de reap à tort — mika#2986 et mika#3042 les
//! retirent. C'est **borner le coût d'une erreur future** : un reap à tort ne
//! doit plus suffire à perdre un vol dont le plan a convergé. La parente fauchée
//! qui porte une PR est déjà rattrapée par `try_promote_parent_on_retry_success`
//! (`failed` → `completed`, `retry_success`) ; un plan convergé n'a pas de PR,
//! donc aucun chemin de retour. Ce module lui en donne un, symétrique.
//!
//! # Ce que ce module est, structurellement
//!
//! **UN** prédicat, lu depuis **UN** site (AC3). Frère de forme de
//! [`crate::flight_verdict_ledger`] et de [`crate::model_transmission`] : le
//! motif de la maison pour une décision à site unique, avec son écriture de
//! piste au même endroit que sa lecture.

use crate::async_db::AsyncDatabase;
use crate::task_engine::engine::GROOM_PARENT_ORPHANED_EVENT;
use crate::task_state::tasks::{Task, groom_result_convergence};
use tracing::{info, warn};

/// `tool_name` de la ligne d'audit écrite quand une parente est réhabilitée.
/// C'est une **transition** (`failed` → `in_progress`), et c'est ce qui rend le
/// vol idempotent : voir [`commit`].
pub(crate) const PARENT_REHABILITATED_EVENT: &str = "task_engine_parent_rehabilitated";

/// `tool_name` de la ligne d'audit écrite quand la réhabilitation est refusée.
/// Ce n'est **pas** une transition : `before_value` y est `None`.
pub(crate) const PARENT_REHABILITATION_REFUSED_EVENT: &str =
    "task_engine_parent_rehabilitation_refused";

/// L'état que porte une parente réhabilitée. Nommé parce que deux sites
/// l'écrivent — le `WHERE` de `promote_task_in_progress` et le retour de
/// `validate_dispatch_readiness` — et qu'ils doivent dire la même chose.
pub(crate) const REHABILITATED_STATUS: &str = "in_progress";

/// Pourquoi une parente terminale n'est **pas** réhabilitable.
///
/// Nommé, parce qu'AC3 demande un refus qui porte sa raison : *« ainsi "aucun
/// cas" et "jamais évalué" ne rendent pas les mêmes octets »*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RehabRefusal {
    /// (a) — la parente n'est pas une parente `self_dev` manuelle en `failed`.
    NotSelfDevFailedParent,
    /// (b) — sa dernière transition d'état n'est pas un reap de groom, ou une
    /// autre transition le suit.
    LastTransitionNotGroomReap,
    /// (c) — aucun enfant de groom de cette parente livré après le reap ne porte
    /// `PLAN_GROOMED` en position de verdict.
    NoGroomedChildSinceReap,
    /// Fail-closed de (b) ou de (c) : la base n'a pas répondu. *Un signal qu'on
    /// ne peut pas lire n'est jamais un terme satisfait* (mika#2277/#2279).
    AuditTrailUnreadable,
    /// La course du *compare-and-swap* est perdue : entre la décision et
    /// l'écriture, quelqu'un d'autre a bougé la parente. Voir [`commit`].
    LostRace,
    /// L'écriture de la transition a échoué. **Distinct de [`Self::LostRace`]**,
    /// et c'est le seul motif que le § 2.5 du plan n'avait pas nommé : une course
    /// perdue dit *« la parente a bougé »*, un échec d'écriture dit *« on ne sait
    /// pas »*. Les fondre rendrait le premier incomptable.
    WriteFailed,
}

impl RehabRefusal {
    /// La raison, telle qu'elle part dans `audit_events.after_value` — le jeton
    /// sur lequel un opérateur groupe.
    pub(crate) fn as_reason(self) -> &'static str {
        match self {
            Self::NotSelfDevFailedParent => "not_self_dev_failed_parent",
            Self::LastTransitionNotGroomReap => "last_transition_not_groom_reap",
            Self::NoGroomedChildSinceReap => "no_groomed_child_since_reap",
            Self::AuditTrailUnreadable => "audit_trail_unreadable",
            Self::LostRace => "parent_rehab_lost_race",
            Self::WriteFailed => "parent_rehab_write_failed",
        }
    }

    /// La même raison en clair, pour `audit_events.reasoning`.
    fn explanation(self) -> &'static str {
        match self {
            Self::NotSelfDevFailedParent => "not a failed manual self_dev parent (AC1a)",
            Self::LastTransitionNotGroomReap => {
                "the parent's last state transition is not a groom reap, or another transition follows it (AC1b)"
            }
            Self::NoGroomedChildSinceReap => {
                "no groom child of this parent delivered PLAN_GROOMED after the reap (AC1c)"
            }
            Self::AuditTrailUnreadable => {
                "the audit trail or the groom children could not be read — fail-closed"
            }
            Self::LostRace => {
                "the guarded failed -> in_progress transition matched no row: the parent moved meanwhile"
            }
            Self::WriteFailed => "the guarded failed -> in_progress transition failed to execute",
        }
    }
}

/// Ce que [`decide`] a établi et que [`commit`] écrira — **si** (d) passe.
///
/// Un type nommé plutôt qu'un couple de `String`, parce que l'appelant le porte
/// à travers une quinzaine de gardes et un millier de lignes avant de l'écrire :
/// `Option<RehabPending>` se lit comme *« une réhabilitation est due »*, là où
/// `Option<(String, String)>` demande de se souvenir de l'ordre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RehabPending {
    /// L'instant du reap, tel que la ligne d'audit le porte.
    pub(crate) reap_at: String,
    /// L'enfant de groom dont le `result` a convergé.
    pub(crate) groom_child_id: String,
}

/// Ce que [`decide`] rend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RehabDecision {
    /// Les trois conditions (a), (b) et (c) tiennent. Reste (d) — tout le reste
    /// de `validate_dispatch_readiness` — avant que [`commit`] n'écrive.
    Eligible(RehabPending),
    /// Le refus, qui porte sa raison.
    Refused(RehabRefusal),
}

impl RehabDecision {
    /// Le libellé de la sonde — `"eligible"` ou `"refused"`.
    fn label(&self) -> &'static str {
        match self {
            Self::Eligible(_) => "eligible",
            Self::Refused(_) => "refused",
        }
    }
}

/// **LE** prédicat. Un seul appelant :
/// [`crate::skills::executor::validate_dispatch_readiness`], dans la branche de
/// sa garde d'état — le seul endroit où la décision peut **empêcher** le
/// `return Err` plutôt que l'observer après coup (doctrine mika#1646).
///
/// # Trois propriétés tenues par la forme, pas par la vigilance
///
/// - **Les conditions sont évaluées dans l'ordre (a) → (b) → (c)**, et la
///   première qui tombe nomme le refus. (d) n'est pas ici : c'est le reste de
///   `validate_dispatch_readiness`.
/// - **`decide` est `async` et fait ses deux lectures elle-même.** Un prédicat
///   pur à qui l'appelant passerait les deux lectures exigerait deux fonctions —
///   la seconde dépend de l'instant du reap rendu par la première — donc deux
///   sites de lecture dans `executor.rs`, donc exactement ce qu'AC3 refuse.
/// - **Fail-closed aux deux lectures.** Une erreur de base rend
///   [`RehabRefusal::AuditTrailUnreadable`], jamais `Eligible`. C'est le
///   contraire du frein d'ancrage mika#2786 (fail-open), et la différence est de
///   conséquence : là, ne pas freiner laisse le vol partir ; ici, réhabiliter sur
///   une lecture ratée dispatcherait sur une parente dont on ne sait rien.
///
/// La sonde `parent_rehabilitation_decided` est émise à **chaque** évaluation,
/// branche saine comprise — sans quoi *« aucun cas »* et *« jamais évalué »*
/// rendraient les mêmes octets (classe mika#2205). Son **absence** ne veut pas
/// dire « aucune évaluation » mais « binaire antérieur au vol » (mika#2340).
pub(crate) async fn decide(db: &AsyncDatabase, task: &Task) -> RehabDecision {
    let decision = evaluate(db, task).await;

    info!(
        target: "mika::otel",
        event = "parent_rehabilitation_decided",
        task_id = %task.id,
        decision = decision.label(),
        // ABSENT — jamais `""` — quand la décision est `Eligible` : « pas de
        // raison de refus » et « raison vide » ne sont pas la même lecture
        // (motif mika#2331).
        refusal = match &decision {
            RehabDecision::Refused(r) => Some(r.as_reason()),
            RehabDecision::Eligible(_) => None,
        },
        "Parent rehabilitation predicate evaluated"
    );

    decision
}

/// Le corps du prédicat, séparé pour que la sonde de [`decide`] soit émise sur
/// **tous** les chemins de sortie sans un `info!` par `return`.
async fn evaluate(db: &AsyncDatabase, task: &Task) -> RehabDecision {
    // (a) — la population. Les trois mêmes colonnes que la garde de
    // `try_promote_parent_on_retry_success` (`dispatcher.rs`), parce que c'est la
    // même population : une parente de vol autonome, et rien d'autre.
    if task.status != "failed"
        || task.trigger_type != "manual"
        || task.source.as_deref() != Some("self_dev")
    {
        return RehabDecision::Refused(RehabRefusal::NotSelfDevFailedParent);
    }

    // (b) — la dernière transition d'état, quel que soit l'outil. Le `LIMIT 1` de
    // la requête fait de « aucune transition ne la suit » une propriété de la
    // MÊME ligne que « la dernière transition est le reap » : une seule lecture
    // pour les deux moitiés de (b), donc aucune fenêtre entre elles.
    let transition = match db.latest_state_transition_for_target(&task.id).await {
        Ok(t) => t,
        Err(e) => {
            warn!(
                task_id = %task.id,
                error = %e,
                "parent_rehabilitation: last state transition unreadable — fail-closed"
            );
            return RehabDecision::Refused(RehabRefusal::AuditTrailUnreadable);
        }
    };
    // Aucune transition du tout : la parente n'a pas été fauchée par ce reaper.
    let Some((tool_name, _before, after_value, reap_at)) = transition else {
        return RehabDecision::Refused(RehabRefusal::LastTransitionNotGroomReap);
    };
    // Le `WHERE` de la requête garantit déjà `after_value IS NOT NULL`, donc le
    // bord `reaper_db_error` (`engine.rs`) — même nom d'outil, `after_value` NULL,
    // un reap qui N'A PAS eu lieu — tombe du bon côté par construction. On
    // redouble avec l'égalité qu'AC1(b) exige : un refus doublement fermé coûte
    // une comparaison de chaîne.
    if tool_name != GROOM_PARENT_ORPHANED_EVENT || after_value.as_deref() != Some("failed") {
        return RehabDecision::Refused(RehabRefusal::LastTransitionNotGroomReap);
    }

    // (c) — un enfant de groom DE CETTE PARENTE, livré APRÈS le reap, dont le
    // `result` porte `PLAN_GROOMED` en position de verdict. Le marqueur n'a
    // jamais été filtré en SQL : les `result` arrivent bruts et le verdict se
    // prend ici, par le lecteur unique (doctrine mika#2590).
    let children = match db.groom_children_delivered_since(&task.id, &reap_at).await {
        Ok(c) => c,
        Err(e) => {
            warn!(
                task_id = %task.id,
                error = %e,
                "parent_rehabilitation: groom children unreadable — fail-closed"
            );
            return RehabDecision::Refused(RehabRefusal::AuditTrailUnreadable);
        }
    };
    for (child_id, result) in children {
        let Some(result) = result else { continue };
        if groom_result_convergence(&result).is_converged() {
            return RehabDecision::Eligible(RehabPending {
                reap_at,
                groom_child_id: child_id,
            });
        }
    }

    RehabDecision::Refused(RehabRefusal::NoGroomedChildSinceReap)
}

/// Écrit la réhabilitation — **la dernière étape avant le `Ok`** de
/// `validate_dispatch_readiness`, après la claim mika#1948.
///
/// Rend `Ok(())` quand la parente est désormais [`REHABILITATED_STATUS`], et
/// `Err(refus)` sinon. Les deux issues écrivent leur ligne d'audit ici, au même
/// site que le prédicat (AC3).
///
/// # Pourquoi l'écriture est séparée de la décision
///
/// AC1(d) : *« la réhabilitation n'est écrite qu'après [les autres gardes] : si
/// l'une refuse, la parente reste `failed` »*. Entre [`decide`] et cet appel,
/// `validate_dispatch_readiness` a traversé une dizaine de gardes et plusieurs
/// allers-retours GitHub à 10 s chacun. Le `WHERE` de `promote_task_in_progress`
/// n'est donc pas une redite du prédicat, c'est un **compare-and-swap** : un
/// autre écrivain a pu bouger la ligne, et dans ce cas on refuse
/// ([`RehabRefusal::LostRace`]) plutôt que de dispatcher sur une parente dont
/// l'état a changé sous nos pieds.
///
/// # L'idempotence, et pourquoi elle ne demande ni compteur ni table
///
/// La ligne de succès **est** une transition (`failed` → `in_progress`). Après
/// réhabilitation, la dernière transition de la parente est donc
/// [`PARENT_REHABILITATED_EVENT`] et plus le reap : (b) refuse toute seconde
/// réhabilitation sur le même reap, et (a) refuse avant elle, la parente étant
/// désormais `in_progress`. **Une parente est réhabilitée au plus une fois par
/// reap.**
///
/// La ligne de refus, elle, porte `before_value = None`. Ce n'est pas une
/// économie, c'est le D1 appliqué à lui-même : avec un `before_value`, une ligne
/// de refus deviendrait « la dernière transition » de la parente et
/// empoisonnerait la lecture (b) du dispatch suivant.
pub(crate) async fn commit(
    db: &AsyncDatabase,
    task_id: &str,
    pending: &RehabPending,
) -> std::result::Result<(), RehabRefusal> {
    let RehabPending {
        reap_at,
        groom_child_id,
    } = pending;
    match db.promote_task_in_progress(task_id).await {
        Ok(true) => {
            let reasoning = format!(
                "groom_reap_overturned (reap_at: {reap_at}, groom_child: {groom_child_id})"
            );
            write_audit_line(
                db,
                task_id,
                PARENT_REHABILITATED_EVENT,
                Some("failed"),
                REHABILITATED_STATUS,
                &reasoning,
            )
            .await;
            info!(
                task_id = %task_id,
                reap_at = %reap_at,
                groom_child = %groom_child_id,
                "parent rehabilitated: groom reap overturned by a converged plan"
            );
            Ok(())
        }
        Ok(false) => {
            refuse(db, task_id, RehabRefusal::LostRace).await;
            Err(RehabRefusal::LostRace)
        }
        Err(e) => {
            warn!(
                task_id = %task_id,
                error = %e,
                "parent_rehabilitation: the guarded transition failed to execute"
            );
            refuse(db, task_id, RehabRefusal::WriteFailed).await;
            Err(RehabRefusal::WriteFailed)
        }
    }
}

/// Écrit la ligne de refus. Appelée par [`commit`] pour les deux refus d'écriture
/// et par l'appelant du prédicat pour les refus de [`decide`] — un seul site
/// d'écriture, deux moments.
pub(crate) async fn refuse(db: &AsyncDatabase, task_id: &str, refusal: RehabRefusal) {
    write_audit_line(
        db,
        task_id,
        PARENT_REHABILITATION_REFUSED_EVENT,
        // `None`, et c'est le D1 appliqué à lui-même : une ligne de refus porte un
        // jugement, pas une transition.
        None,
        refusal.as_reason(),
        refusal.explanation(),
    )
    .await;
}

/// Le site **unique** d'écriture de la piste de ce module.
///
/// `session_id = system-<agent>` et `target_key` = l'id de parente **nu** : les
/// deux conventions du reaper (`engine.rs`), pour qu'un opérateur joigne les
/// deux lignes sur la même clé.
///
/// Un échec d'écriture de la piste ne défait pas la transition : elle est déjà
/// commise, et la perdre en plus de la ligne serait pire. Le `warn!` nomme ce
/// qui manquera à la lecture.
async fn write_audit_line(
    db: &AsyncDatabase,
    task_id: &str,
    tool_name: &str,
    before_value: Option<&str>,
    after_value: &str,
    reasoning: &str,
) {
    let session_id = format!("system-{}", db.agent_id());
    let trace_id = mika_common::trace::generate_trace_id();
    if let Err(e) = db
        .log_audit_event(
            &session_id,
            tool_name,
            task_id,
            before_value,
            Some(after_value),
            Some(reasoning),
            Some(&trace_id),
        )
        .await
    {
        warn!(
            task_id = %task_id,
            tool_name = %tool_name,
            error = %e,
            "parent_rehabilitation: audit line not written"
        );
    }
}
