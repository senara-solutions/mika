//! Le filet : un tour QA coupé par sa deadline POSTE un verdict (mika#2276 M2).
//!
//! # Le défaut que ce module ferme
//!
//! Mesuré sur la PR #2275, trace `921f11f0-acd4-11f1-8bc6-90c3b908c45a`,
//! 2026-09-10 05:00:55Z → 05:09:21Z. Le tour mika-qa a lu le callout de plan et
//! le diff complet, puis a lancé deux `cargo test --release` de 237,9 s et
//! 231,1 s — 469 s des ~506 s de son enveloppe — et s'est terminé sur
//! `agent deadline exceeded — exiting loop gracefully`, `steps_completed=5`.
//! Zéro verdict rédigé, donc zéro verdict posté.
//!
//! Ce qu'on voyait de l'extérieur : **Telegram notifié deux fois, PR muette.**
//! Le mécanisme exact est là : `LoopResult::DeadlineExceeded` tombe dans
//! `persist_deadline_fallback`, dont le texte générique — *« I'm sorry, that took
//! too long »* — repart sur le canal de réponse via `sender_arc.send()` comme
//! n'importe quelle vraie réponse. Le seul chemin qui pose un verdict sur une PR
//! est `run_gh pr review`, appelé par le LLM, jamais atteint. **« Le tour a
//! abouti » et « le tour a conclu » étaient deux faits différents que rien dans
//! le code ne séparait.**
//!
//! # Ce que le filet ne fait pas
//!
//! Il ne rend pas le dépassement plus rare — c'est le rôle de M1 (budget d'outil
//! par skill, `agent_loop::build_skill_tool_timeouts`), qui retire au tour QA le
//! moyen de se suicider par recompilation. Les deux sont nécessaires : M1 sans
//! ce filet laisse la prochaine cause de dépassement muette ; ce filet sans M1
//! laisse la QA échouer bruyamment à chaque revue de PR substrat.
//!
//! Il ne remplace pas non plus le fallback conversationnel : celui-ci continue
//! de partir sur le canal de notification. Le verdict s'ajoute, sur l'autre
//! canal, celui qui manquait.
//!
//! # Forme du verdict — `hold[review]`, tranché par l'architecte (Q1)
//!
//! `hold[review]` existe déjà dans `qa-review/skill.toml` et est déjà compris par
//! `verdict_handler` (« notifier l'opérateur, laisser la tâche `in_progress` »).
//! Un `block[timeout]` neuf aurait demandé une nouvelle branche dans
//! `verdict_handler.rs` — donc le gate CODEOWNERS — pour une sémantique que
//! `hold[review]` porte déjà : *ce tour n'a pas conclu, un humain regarde.*
//!
//! # Deux motifs depuis mika#2368
//!
//! Le filet a été **généralisé**, pas dupliqué. La question qu'il répond n'est
//! plus « le tour a-t-il été coupé ? » mais « ce tour devait un verdict et n'en
//! a pas posté ? » — et il y a deux façons d'y arriver :
//!
//! - [`VerdictReason::CutOffByDeadline`] — mika#2276, le cas fondateur : le tour
//!   n'a jamais atteint sa conclusion.
//! - [`VerdictReason::CallbackConcludedWithoutVerdict`] — mika#2368 : le tour de
//!   callback de build QA a **conclu**, proprement, sans rien poster. La garde
//!   positive `qa_build_callback_verdict` (mika#2355) l'a re-prompté une fois ;
//!   son budget est d'un coup, délibérément, et une fois épuisé il reste une
//!   injonction ignorée deux fois et une PR muette.
//!
//! Ce qui dépend du motif : le corps posté, la ligne de journal et le **nom
//! d'événement**. Ce qui n'en dépend pas : la résolution de cible, le registre
//! anti-double-post, la classification du 422, la discipline « ne jamais rendre
//! d'erreur ».
//!
//! Les deux noms d'événement sont distincts ([`DEADLINE_VERDICT_EVENT`] et
//! [`CALLBACK_VERDICT_EVENT`]) et chacun est **SOLE WRITER** du sien. Ce n'est
//! pas du confort de nommage : la sonde de contrôle négatif de mika#2355 est
//! littéralement `grep qa_deadline_verdict … | jq 'select(.outcome == "posted")'`
//! — *le filet mika#2276 ne doit pas se mettre à firer*. Un nom partagé
//! fusionnerait deux populations qui doivent rester comptables séparément, et un
//! `posted` du motif neuf se lirait comme une régression de l'ancien. Même
//! principe que `phantom_aged_out` / `phantom_sweep_spared` (mika#2156) ou
//! `auto_pull_no_token` / `wip_rescue_no_token` (mika#2205).

use std::collections::HashSet;
use std::future::Future;

use dashmap::DashMap;
use tracing::{info, warn};

use super::verdict::parse_pr_review_event;
use super::webhook_queue_v2::parse_pr_action_event;

/// Ligne canonique du verdict de secours. Forme tranchée en Q1 : surface
/// existante, `verdict_handler` la comprend déjà, CODEOWNERS épargné.
pub const DEADLINE_VERDICT_LINE: &str = "VERDICT: hold[review]";

/// Nom d'événement du motif [`VerdictReason::CutOffByDeadline`] (mika#2276).
///
/// **SOLE WRITER** : ce module est le seul site qui écrit ce nom, en journal
/// comme en `audit_events`. La sonde de contrôle négatif de mika#2355 en dépend.
pub const DEADLINE_VERDICT_EVENT: &str = "qa_deadline_verdict";

/// Nom d'événement du motif [`VerdictReason::CallbackConcludedWithoutVerdict`]
/// (mika#2368).
///
/// **SOLE WRITER**, et **distinct** de [`DEADLINE_VERDICT_EVENT`] à dessein :
/// voir la section « Deux motifs » du module.
pub const CALLBACK_VERDICT_EVENT: &str = "qa_callback_verdict";

/// Clé de `tasks.metadata` portant la PR qu'un callback de build QA devra
/// pouvoir verdicter (mika#2368 C2).
///
/// **La cible est dite, jamais dérivée par le filet.** Elle est résolue par le
/// producteur au moment du spawn (`skills::executor::execute_long_running`, qui
/// lit `LongRunningContext.originating_message`) et stampée sur la tâche
/// callback — même trajectoire que `metadata.dispatch_worktree_file`
/// (mika#2249) et `metadata.pilot_transcript_expected` (mika#2040).
///
/// Ce qui est condamné, c'est la dérivation **tardive** : celle qui se ferait au
/// moment du filet, quand l'échec n'est plus rattrapable et ne se journalise
/// nulle part. Ici la résolution a lieu au spawn, son échec est journalisé
/// sur-le-champ, et le filet **ne parse rien** — il lit un stamp.
///
/// Absence n'est jamais preuve : pas de stamp, stamp illisible, cible non
/// résoluble ⇒ zéro POST et une ligne nommant l'abstention (AC6).
pub const QA_REVIEW_PR_TARGET_KEY: &str = "qa_review_pr_target";

/// La PR qu'un tour était en train de traiter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrTarget {
    pub repo: String,
    pub pr_number: u64,
}

impl PrTarget {
    /// La forme stampée dans `tasks.metadata` sous [`QA_REVIEW_PR_TARGET_KEY`].
    ///
    /// Écrivain et lecteur vivent côte à côte, exprès : une grammaire de fil
    /// dont les deux moitiés sont séparées dérive en silence — la leçon que
    /// mika#2158 a dû engraver une fois.
    pub fn to_metadata_value(&self) -> String {
        format!("{}#{}", self.repo, self.pr_number)
    }

    /// Relit [`Self::to_metadata_value`]. `None` sur toute forme illisible —
    /// un signal qu'on ne peut pas lire n'est jamais un terme satisfait.
    pub fn from_metadata_value(raw: &str) -> Option<Self> {
        let (repo, number) = raw.trim().rsplit_once('#')?;
        if repo.is_empty() {
            return None;
        }
        Some(Self {
            repo: repo.to_string(),
            pr_number: number.parse().ok()?,
        })
    }
}

/// Pourquoi le filet poste.
///
/// Le motif décide du corps, de la ligne de journal et du nom d'événement. Il
/// était, jusqu'à mika#2368, exprimé par un `Option<DeadlineOverrun>` — c'est-à-dire
/// par le fait qu'il n'y en avait qu'un.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictReason {
    /// mika#2276 — le tour a été coupé par son enveloppe avant de conclure.
    CutOffByDeadline(crate::agent_loop::DeadlineOverrun),
    /// mika#2368 — le tour de callback de build QA a conclu sans poster de
    /// verdict, re-prompt de la garde `qa_build_callback_verdict` compris.
    CallbackConcludedWithoutVerdict,
}

impl VerdictReason {
    /// Le nom d'événement de ce motif. Voir la section « Deux motifs » du module
    /// pour pourquoi les deux ne peuvent pas être un seul.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::CutOffByDeadline(_) => DEADLINE_VERDICT_EVENT,
            Self::CallbackConcludedWithoutVerdict => CALLBACK_VERDICT_EVENT,
        }
    }
}

/// Ce que le filet demande de poster. Le call-site fournit l'exécution ; ce
/// module fournit la décision et le corps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostReviewRequest {
    pub repo: String,
    pub pr_number: u64,
    pub body: String,
}

/// Issue d'un passage du filet. Chaque variante est un fait distinct que
/// l'opérateur doit pouvoir distinguer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadlineVerdictOutcome {
    /// Le filet ne s'applique pas. `reason` est un discriminant stable — il
    /// atterrit dans le journal et se `GROUP BY`.
    NotApplicable(&'static str),
    /// Une review a déjà été postée pour cette PR dans cette session (AC3).
    /// Zéro POST.
    AlreadyReviewed,
    /// Verdict posté.
    Posted,
    /// Le POST a répondu 422 : GitHub dit que la review existe déjà. **Succès
    /// idempotent**, jamais échec de verdict (exigence architecte Q4). Le
    /// registre en mémoire ne survit pas à un redémarrage ; 422 est le filet
    /// quand il a été perdu.
    AlreadyPostedUpstream,
    /// Le POST a échoué pour une autre raison.
    Failed(String),
}

/// Entrées du filet.
pub struct DeadlineVerdictInput<'a> {
    /// Pourquoi on poste. Décide du corps, du journal et du nom d'événement.
    pub reason: VerdictReason,
    /// La PR sur laquelle poster, **déjà résolue par l'appelant**.
    ///
    /// Le filet ne parse rien : le call-site webhook (mika#2276) passe par
    /// [`deadline_verdict_target`], le call-site callback (mika#2368) lit le
    /// stamp [`QA_REVIEW_PR_TARGET_KEY`]. Aucun des deux ne devine une PR.
    pub target: PrTarget,
    pub session_id: &'a str,
    pub trace_id: &'a str,
    pub agent_id: &'a str,
    /// Le registre anti-double-post partagé (`AppState.pr_reviews_posted`).
    /// `None` hors mode serveur — le filet s'abstient alors, faute de pouvoir
    /// répondre à AC3.
    pub pr_reviews_posted: Option<&'a DashMap<String, HashSet<String>>>,
}

/// Extrait la PR qu'un événement désigne, quelle que soit sa forme.
///
/// Deux grammaires, toutes deux produites par
/// `mika_gateway::github::format_event_text` :
///
/// - `[GitHub] PR review ({state}) on {repo}#{n} ...` → [`parse_pr_review_event`]
/// - `[GitHub] PR {action}: {repo}#{n} — ...` → [`parse_pr_action_event`]
///
/// La seconde est celle qui déclenche une revue QA (`review_requested`,
/// `synchronize`, `ready_for_review`) et donc la seule que le symptôme mesuré
/// emprunte ; la première est incluse parce qu'un tour déclenché par une review
/// peut lui aussi mourir sur sa deadline.
///
/// Aucune des deux regex n'est recopiée ici : une grammaire de fil dupliquée est
/// exactement ce qui a laissé deux lecteurs diverger dans mika#2158.
///
/// Reste `pub` après mika#2368 : les appelants qui ont un texte d'événement sous
/// la main (`handlers.rs`, et le producteur du stamp dans `skills::executor`)
/// l'utilisent, mais [`maybe_post_deadline_verdict`] ne l'appelle plus.
pub fn parse_pr_target(text: &str) -> Option<PrTarget> {
    if let Some(event) = parse_pr_review_event(text) {
        return Some(PrTarget {
            repo: event.repo,
            pr_number: event.pr_number,
        });
    }
    let (_action, repo, pr_number) = parse_pr_action_event(text)?;
    Some(PrTarget {
        repo: repo.to_string(),
        pr_number,
    })
}

/// La décision d'entrée du call-site webhook mika#2276 : ce tour appelle-t-il le
/// filet, et sur quelle PR ?
///
/// `Some` seulement si le tour a été **coupé** *et* portait sur une PR. Elle
/// porte les deux gardes d'entrée que [`maybe_post_deadline_verdict`] tenait
/// avant mika#2368 (`turn_completed` et `not_a_pr_event`) : elles n'ont jamais
/// été des propriétés du filet mais du chemin webhook, et le motif devenant
/// énuméré, la première ne peut plus vivre dans le filet — `overrun == None`
/// décrit exactement le périmètre du second motif.
///
/// Le call-site l'appelle **avant** de résoudre un token : sur le chemin nominal
/// (tour conclu, ou tour non-PR) rien n'est payé, rien n'est journalisé.
pub fn deadline_verdict_target(
    deadline_exceeded: Option<crate::agent_loop::DeadlineOverrun>,
    event_text: &str,
) -> Option<(VerdictReason, PrTarget)> {
    let overrun = deadline_exceeded?;
    // Un tour Telegram, un heartbeat ou un événement non-PR peut dépasser sa
    // deadline : il n'y a alors rien sur quoi poster, et ce n'est pas un défaut.
    let target = parse_pr_target(event_text)?;
    Some((VerdictReason::CutOffByDeadline(overrun), target))
}

/// Le registre anti-double-post porte-t-il déjà une review pour cette PR ?
///
/// Le format de clé appartient à `builtin_handlers::format_pr_dedup_key`, qui est
/// aussi ce qu'écrit `run_gh` sur succès de `gh pr review` — on l'appelle plutôt
/// que de le recomposer, faute de quoi la même grammaire vivrait à deux endroits
/// et dériverait en silence dans les deux sens : une clé manquée re-poste une
/// review, une clé fabriquée laisse la PR muette.
///
/// **Deux formes testées, pas une.** `make_pr_dedup_key` met `__default__` à la
/// place du dépôt quand l'appel `gh pr review` n'a pas porté de `--repo`. Un tour
/// QA qui poste sans `--repo` a bel et bien posté ; manquer cette forme ferait
/// re-poster le filet — le double-post exact qu'AC3 interdit.
fn session_has_review_for(
    registry: &DashMap<String, HashSet<String>>,
    session_id: &str,
    target: &PrTarget,
) -> bool {
    use crate::skills::builtin_handlers::format_pr_dedup_key;

    let Some(posted) = registry.get(session_id) else {
        return false;
    };
    let number = target.pr_number.to_string();
    posted.contains(&format_pr_dedup_key(Some(&target.repo), &number))
        || posted.contains(&format_pr_dedup_key(None, &number))
}

/// Le corps du verdict de secours (AC1 mika#2276, AC5 mika#2368).
///
/// Dépend du motif. Sur [`VerdictReason::CutOffByDeadline`] il porte le nombre de
/// steps accomplis — un tour coupé au step 5 et un tour coupé au step 19
/// appellent des réponses opérateur différentes, et le texte de fallback ne dit
/// ni l'un ni l'autre. Sur le motif mika#2368 il n'y a pas de steps à nommer, et
/// « a atteint la limite de son enveloppe de temps » serait simplement faux.
///
/// **AC5c** : la branche `CutOffByDeadline` est figée octet pour octet. Le test
/// [`tests::mika2368_the_deadline_body_is_frozen_byte_for_byte`] en tient la
/// chaîne attendue en dur plutôt que de la régénérer par le code sous test.
fn build_verdict_body(reason: &VerdictReason, trace_id: &str) -> String {
    match reason {
        VerdictReason::CutOffByDeadline(overrun) => format!(
            "{DEADLINE_VERDICT_LINE}\n\
             \n\
             Ce verdict est posté par le moteur, pas par le tour de revue.\n\
             \n\
             Le tour de revue QA a atteint la limite de son enveloppe de temps avant \
             d'avoir rédigé un verdict : il s'est arrêté après {steps} step(s) d'outil. \
             Aucune conclusion de revue n'a été produite — ce `hold[review]` ne dit rien \
             du contenu de la PR, seulement que la revue n'a pas abouti.\n\
             \n\
             Relancer la revue (retirer puis remettre le reviewer) suffit dans le cas \
             nominal. Si le dépassement se répète sur cette PR, le tour bute \
             probablement sur un travail trop long pour un budget de revue — regarder \
             les `run_shell` du tour avant de relancer une troisième fois.\n\
             \n\
             Trace : `{trace_id}` — chercher `agent deadline exceeded` et \
             `{DEADLINE_VERDICT_EVENT}` dans `$MIKA_SPIRIT_LOG_FILE`.\n\
             \n\
             <sub>mika#2276</sub>",
            steps = overrun.steps_completed,
        ),
        VerdictReason::CallbackConcludedWithoutVerdict => format!(
            "{DEADLINE_VERDICT_LINE}\n\
             \n\
             Ce verdict est posté par le moteur, pas par le tour de revue.\n\
             \n\
             Le tour de callback de build de la revue QA s'est terminé **sans poster \
             de verdict** : aucun `run_gh pr review` réussi n'apparaît dans son \
             historique d'outils, ni sur son premier EndTurn, ni après le re-prompt \
             du moteur. Aucune conclusion de revue n'a été produite — ce \
             `hold[review]` ne dit rien du contenu de la PR, seulement que la revue \
             n'a pas abouti.\n\
             \n\
             Relancer la revue (retirer puis remettre le reviewer) suffit dans le cas \
             nominal. Si le silence se répète sur cette PR, le tour bute sur autre \
             chose qu'un aléa — regarder les `run_gh` du tour et le corps de son \
             dernier EndTurn avant de relancer une troisième fois.\n\
             \n\
             Trace : `{trace_id}` — chercher `qa_build_callback_verdict` et \
             `{CALLBACK_VERDICT_EVENT}` dans `$MIKA_SPIRIT_LOG_FILE`.\n\
             \n\
             <sub>mika#2368</sub>"
        ),
    }
}

/// Un POST refusé en 422 est-il un « la review existe déjà » ?
///
/// `run_gh_subprocess` rend l'erreur `gh` en texte (stderr), pas un code HTTP
/// structuré : la classification se fait donc sur la chaîne, à contrecœur mais
/// sans autre prise. On reste **étroit** — `422` seul serait trop large, un 422
/// peut aussi signaler un corps invalide. Deux formes suffisent parce que ce
/// sont celles que `gh` produit : le code accompagné du libellé HTTP, et le
/// message que l'API renvoie quand l'auteur a déjà une review en attente.
pub fn is_idempotent_already_posted(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("http 422")
        || lower.contains("unprocessable entity")
        || lower.contains("was submitted too quickly")
        || lower.contains("already")
            && (lower.contains("pending review") || lower.contains("reviewed"))
}

/// Poste le verdict de secours sur la PR que l'appelant a **résolue**, si et
/// seulement si aucune review n'a déjà été postée pour elle dans cette session.
///
/// Depuis mika#2368 la décision « ce tour doit-il un verdict ? » appartient à
/// l'appelant et se dit par [`VerdictReason`] — voir [`deadline_verdict_target`]
/// pour le chemin webhook, et le câblage du dispatcher pour le chemin callback.
///
/// `poster` exécute le POST. L'injection existe parce que le contrat qu'AC2
/// demande d'asserter est *« un verdict EST posté »*, ce qu'un test ne peut voir
/// qu'en tenant l'exécution ; le call-site de production y branche
/// `run_gh_subprocess`.
///
/// Le filet ne renvoie jamais d'erreur : un échec de POST est une variante
/// d'issue, journalisée. Faire échouer le traitement du webhook parce que le
/// filet a échoué remplacerait un silence par une panne.
pub async fn maybe_post_deadline_verdict<F, Fut>(
    input: DeadlineVerdictInput<'_>,
    poster: F,
) -> DeadlineVerdictOutcome
where
    F: FnOnce(PostReviewRequest) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let event = input.reason.event_name();
    let target = &input.target;

    let Some(registry) = input.pr_reviews_posted else {
        // Sans le registre, AC3 est indécidable. Le double-post est une
        // régression pire que le silence (table de disposition, ligne 4) : on
        // s'abstient, et on le dit.
        warn!(
            event,
            agent_id = %input.agent_id,
            trace_id = %input.trace_id,
            repo = %target.repo,
            pr = target.pr_number,
            outcome = "no_registry",
            "verdict de secours dû sur une PR, mais le registre anti-double-post \
             est absent — rien n'a été posté"
        );
        return DeadlineVerdictOutcome::NotApplicable("no_registry");
    };

    if session_has_review_for(registry, input.session_id, target) {
        info!(
            event,
            agent_id = %input.agent_id,
            trace_id = %input.trace_id,
            repo = %target.repo,
            pr = target.pr_number,
            outcome = "already_reviewed",
            "la review était déjà postée dans cette session — pas de second verdict"
        );
        return DeadlineVerdictOutcome::AlreadyReviewed;
    }

    let request = PostReviewRequest {
        repo: target.repo.clone(),
        pr_number: target.pr_number,
        body: build_verdict_body(&input.reason, input.trace_id),
    };

    match poster(request).await {
        Ok(_) => {
            // Inscrire au registre : un second passage du filet dans la même
            // session (une re-revue relancée à la main pendant que la première
            // agonise) ne doit pas re-poster.
            registry
                .entry(input.session_id.to_string())
                .or_default()
                .insert(crate::skills::builtin_handlers::format_pr_dedup_key(
                    Some(&target.repo),
                    &target.pr_number.to_string(),
                ));
            // Deux `warn!` et non un paramétré : `steps_completed` n'existe que
            // sur le motif deadline, et le rendre `Option` changerait sa forme
            // dans le JSON (`5` devient `Some(5)`) — un champ que les sondes
            // mika#2276 lisent. AC5c exige la ligne à l'identique.
            match &input.reason {
                VerdictReason::CutOffByDeadline(overrun) => warn!(
                    event,
                    agent_id = %input.agent_id,
                    trace_id = %input.trace_id,
                    repo = %target.repo,
                    pr = target.pr_number,
                    steps_completed = overrun.steps_completed,
                    outcome = "posted",
                    "tour de revue coupé par sa deadline — verdict hold[review] posté \
                     par le moteur"
                ),
                VerdictReason::CallbackConcludedWithoutVerdict => warn!(
                    event,
                    agent_id = %input.agent_id,
                    trace_id = %input.trace_id,
                    repo = %target.repo,
                    pr = target.pr_number,
                    outcome = "posted",
                    "callback de build QA conclu sans verdict — verdict hold[review] \
                     posté par le moteur"
                ),
            }
            DeadlineVerdictOutcome::Posted
        }
        Err(e) if is_idempotent_already_posted(&e) => {
            info!(
                event,
                agent_id = %input.agent_id,
                trace_id = %input.trace_id,
                repo = %target.repo,
                pr = target.pr_number,
                outcome = "already_posted_upstream",
                "GitHub a répondu 422 — la review existe déjà, succès idempotent"
            );
            DeadlineVerdictOutcome::AlreadyPostedUpstream
        }
        Err(e) => {
            warn!(
                event,
                agent_id = %input.agent_id,
                trace_id = %input.trace_id,
                repo = %target.repo,
                pr = target.pr_number,
                outcome = "post_failed",
                error = %e,
                "échec du POST du verdict de secours"
            );
            DeadlineVerdictOutcome::Failed(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::DeadlineOverrun;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const REVIEW_REQUESTED: &str = "[GitHub] PR review_requested: senara-solutions/mika#2275 — fix(mika#2272) (branch: fix/2272)\nhttps://github.com/senara-solutions/mika/pull/2275\nRequested reviewer: @mika-platform-qa";

    fn overrun(steps: usize) -> Option<DeadlineOverrun> {
        Some(DeadlineOverrun {
            steps_completed: steps,
        })
    }

    /// Le motif mika#2276, tel que [`deadline_verdict_target`] le construit.
    fn cut_off(steps: usize) -> VerdictReason {
        VerdictReason::CutOffByDeadline(DeadlineOverrun {
            steps_completed: steps,
        })
    }

    /// La cible que `REVIEW_REQUESTED` désigne — résolue par le lecteur unique,
    /// jamais écrite à la main, pour que le test casse si la grammaire bouge.
    fn requested_target() -> PrTarget {
        parse_pr_target(REVIEW_REQUESTED).expect("PR target")
    }

    #[test]
    fn parses_the_review_requested_shape_that_triggers_a_qa_turn() {
        let target = parse_pr_target(REVIEW_REQUESTED).expect("PR target");
        assert_eq!(target.repo, "senara-solutions/mika");
        assert_eq!(target.pr_number, 2275);
    }

    #[test]
    fn parses_the_pr_review_submitted_shape_too() {
        let text = "[GitHub] PR review (approved) on senara-solutions/mika#2275 (fix) by @someone\nhttps://github.com/senara-solutions/mika/pull/2275#pullrequestreview-1\n\nVERDICT: pass";
        let target = parse_pr_target(text).expect("PR target");
        assert_eq!(target.repo, "senara-solutions/mika");
        assert_eq!(target.pr_number, 2275);
    }

    #[test]
    fn a_non_pr_event_has_no_target() {
        assert!(parse_pr_target("Coucou, tu peux regarder mon agenda ?").is_none());
        assert!(
            parse_pr_target("[GitHub] Check suite success on senara-solutions/mika (branch: main)")
                .is_none()
        );
    }

    /// mika#2276 AC1/AC2 — le contrat central : **deadline dépassé ⇒ verdict posté.**
    #[tokio::test]
    async fn deadline_on_a_pr_turn_posts_a_verdict() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let captured = Arc::new(std::sync::Mutex::new(None::<PostReviewRequest>));
        let calls = Arc::new(AtomicUsize::new(0));

        let sink = captured.clone();
        let counter = calls.clone();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                reason: cut_off(5),
                target: requested_target(),
                session_id: "qa-session",
                trace_id: "921f11f0-acd4-11f1-8bc6-90c3b908c45a",
                agent_id: "mika-qa",
                pr_reviews_posted: Some(&registry),
            },
            move |req| {
                counter.fetch_add(1, Ordering::SeqCst);
                *sink.lock().unwrap() = Some(req);
                async { Ok("ok".to_string()) }
            },
        )
        .await;

        assert_eq!(outcome, DeadlineVerdictOutcome::Posted);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "exactement un POST");

        let req = captured.lock().unwrap().clone().expect("un POST capturé");
        assert_eq!(req.repo, "senara-solutions/mika");
        assert_eq!(req.pr_number, 2275);
        assert!(
            req.body.starts_with(DEADLINE_VERDICT_LINE),
            "le corps doit s'ouvrir sur la ligne canonique, got: {}",
            req.body
        );
        // AC1 : motif, steps, trace_id.
        assert!(
            req.body.contains("5 step(s)"),
            "steps manquants: {}",
            req.body
        );
        assert!(
            req.body.contains("921f11f0-acd4-11f1-8bc6-90c3b908c45a"),
            "trace_id manquant: {}",
            req.body
        );
        assert!(
            !req.body.contains("block[timeout]"),
            "Q1 : pas de verdict neuf — hold[review] seulement"
        );
    }

    /// mika#2276 AC3 — un tour qui a DÉJÀ posté sa review puis dépasse sa
    /// deadline ne poste pas un second verdict.
    #[tokio::test]
    async fn a_turn_that_already_reviewed_posts_nothing() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        registry
            .entry("qa-session".to_string())
            .or_default()
            .insert(
                // La clé telle que `run_gh` l'écrit après un `pr review --repo`.
                "senara-solutions/mika|2275".to_string(),
            );
        let calls = Arc::new(AtomicUsize::new(0));

        let counter = calls.clone();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                reason: cut_off(12),
                target: requested_target(),
                session_id: "qa-session",
                trace_id: "trace",
                agent_id: "mika-qa",
                pr_reviews_posted: Some(&registry),
            },
            move |_req| {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok(String::new()) }
            },
        )
        .await;

        assert_eq!(outcome, DeadlineVerdictOutcome::AlreadyReviewed);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "AC3 : zéro POST");
    }

    /// mika#2276 AC3 — la même chose quand le tour a posté SANS `--repo`.
    ///
    /// `make_pr_dedup_key` écrit alors `__default__|{n}`. Manquer cette forme
    /// ferait re-poster le filet sur une PR déjà reviewée, soit le double-post
    /// que la table de disposition classe pire que le silence.
    #[tokio::test]
    async fn a_review_posted_without_repo_flag_still_suppresses_the_net() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        registry
            .entry("qa-session".to_string())
            .or_default()
            .insert("__default__|2275".to_string());
        let calls = Arc::new(AtomicUsize::new(0));

        let counter = calls.clone();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                reason: cut_off(3),
                target: requested_target(),
                session_id: "qa-session",
                trace_id: "trace",
                agent_id: "mika-qa",
                pr_reviews_posted: Some(&registry),
            },
            move |_req| {
                counter.fetch_add(1, Ordering::SeqCst);
                async { Ok(String::new()) }
            },
        )
        .await;

        assert_eq!(outcome, DeadlineVerdictOutcome::AlreadyReviewed);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// mika#2276 AC3 (exigence architecte Q4) — un POST répondant **422**
    /// s'interprète en succès idempotent, jamais en échec de verdict.
    #[tokio::test]
    async fn a_422_reads_as_idempotent_success_and_is_not_retried() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let calls = Arc::new(AtomicUsize::new(0));

        let counter = calls.clone();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                reason: cut_off(5),
                target: requested_target(),
                session_id: "qa-session",
                trace_id: "trace",
                agent_id: "mika-qa",
                pr_reviews_posted: Some(&registry),
            },
            move |_req| {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    Err("gh exit code 1: HTTP 422: Unprocessable Entity (https://api.github.com/repos/x/y/pulls/1/reviews)".to_string())
                }
            },
        )
        .await;

        assert_eq!(outcome, DeadlineVerdictOutcome::AlreadyPostedUpstream);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "un seul POST — un 422 ne se réessaie pas"
        );
    }

    #[test]
    fn a_422_classifier_stays_narrow() {
        assert!(is_idempotent_already_posted(
            "gh: HTTP 422: Unprocessable Entity"
        ));
        assert!(is_idempotent_already_posted(
            "was submitted too quickly (HTTP 422)"
        ));
        // Un vrai échec ne doit pas se déguiser en succès.
        assert!(!is_idempotent_already_posted("gh: HTTP 403: Forbidden"));
        assert!(!is_idempotent_already_posted("gh CLI not found"));
        assert!(!is_idempotent_already_posted(
            "gh: HTTP 404: Not Found (pull request 4220 not found)"
        ));
    }

    /// Un échec de POST autre que 422 reste un échec — il ne doit pas se
    /// silencer en succès idempotent, sinon la panne redevient invisible, ce
    /// que ce ticket existe pour empêcher.
    #[tokio::test]
    async fn a_genuine_post_failure_is_reported_as_such() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                reason: cut_off(5),
                target: requested_target(),
                session_id: "qa-session",
                trace_id: "trace",
                agent_id: "mika-qa",
                pr_reviews_posted: Some(&registry),
            },
            |_req| async { Err("gh exit code 1: HTTP 403: Forbidden".to_string()) },
        )
        .await;

        assert!(matches!(outcome, DeadlineVerdictOutcome::Failed(_)));
    }

    /// Un tour qui a conclu normalement ne déclenche rien — le filet est
    /// strictement additif sur le chemin nominal.
    ///
    /// mika#2368 : la garde est passée de la première ligne du filet à
    /// [`deadline_verdict_target`], parce que « `overrun == None` » décrit
    /// désormais exactement le périmètre du second motif et ne peut donc plus
    /// être un refus. L'assertion n'est pas affaiblie : elle porte maintenant
    /// sur le fait que le call-site **n'appelle pas** le filet, ce qui est un
    /// zéro POST plus fort qu'un `NotApplicable`.
    #[test]
    fn a_completed_turn_never_reaches_the_net() {
        assert!(
            deadline_verdict_target(None, REVIEW_REQUESTED).is_none(),
            "un tour conclu ne doit produire aucun motif deadline"
        );
    }

    /// Un tour non-PR qui dépasse sa deadline (Telegram, heartbeat) n'a rien sur
    /// quoi poster — et ce n'est pas un défaut.
    #[test]
    fn a_non_pr_deadline_overrun_never_reaches_the_net() {
        assert!(
            deadline_verdict_target(overrun(9), "Salut, tu peux me résumer ma semaine ?").is_none()
        );
    }

    /// Le chemin nominal du call-site mika#2276, dans l'autre sens : un tour
    /// coupé sur une PR produit bien le motif deadline et la bonne cible.
    #[test]
    fn a_cut_off_pr_turn_produces_the_deadline_reason_and_its_target() {
        let (reason, target) =
            deadline_verdict_target(overrun(5), REVIEW_REQUESTED).expect("motif + cible");
        assert_eq!(reason, cut_off(5));
        assert_eq!(reason.event_name(), DEADLINE_VERDICT_EVENT);
        assert_eq!(target, requested_target());
    }

    /// Le filet s'inscrit lui-même au registre : une re-revue relancée à la main
    /// pendant que la première agonise ne produit pas deux verdicts.
    #[tokio::test]
    async fn the_net_registers_its_own_post_so_it_cannot_fire_twice() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let counter = calls.clone();
            let _ = maybe_post_deadline_verdict(
                DeadlineVerdictInput {
                    reason: cut_off(5),
                    target: requested_target(),
                    session_id: "qa-session",
                    trace_id: "trace",
                    agent_id: "mika-qa",
                    pr_reviews_posted: Some(&registry),
                },
                move |_req| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async { Ok(String::new()) }
                },
            )
            .await;
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "le second passage doit voir sa propre inscription"
        );
    }

    // -----------------------------------------------------------------------
    // mika#2368 — le second motif
    // -----------------------------------------------------------------------

    /// **AC5c** — le corps du motif mika#2276 est figé **octet pour octet**.
    ///
    /// La chaîne attendue est en dur, pas re-générée par le code sous test :
    /// une comparaison contre `build_verdict_body(...)` passerait quelle que
    /// soit la dérive, ce qui est précisément ce qu'AC5c interdit.
    #[test]
    fn mika2368_the_deadline_body_is_frozen_byte_for_byte() {
        let body = build_verdict_body(&cut_off(5), "921f11f0-acd4-11f1-8bc6-90c3b908c45a");
        let expected = "VERDICT: hold[review]\n\
             \n\
             Ce verdict est posté par le moteur, pas par le tour de revue.\n\
             \n\
             Le tour de revue QA a atteint la limite de son enveloppe de temps avant \
             d'avoir rédigé un verdict : il s'est arrêté après 5 step(s) d'outil. \
             Aucune conclusion de revue n'a été produite — ce `hold[review]` ne dit rien \
             du contenu de la PR, seulement que la revue n'a pas abouti.\n\
             \n\
             Relancer la revue (retirer puis remettre le reviewer) suffit dans le cas \
             nominal. Si le dépassement se répète sur cette PR, le tour bute \
             probablement sur un travail trop long pour un budget de revue — regarder \
             les `run_shell` du tour avant de relancer une troisième fois.\n\
             \n\
             Trace : `921f11f0-acd4-11f1-8bc6-90c3b908c45a` — chercher `agent deadline exceeded` et \
             `qa_deadline_verdict` dans `$MIKA_SPIRIT_LOG_FILE`.\n\
             \n\
             <sub>mika#2276</sub>";
        assert_eq!(
            body, expected,
            "AC5c : le corps mika#2276 doit rester identique octet pour octet"
        );
    }

    /// **AC5** — le corps du motif neuf dit ce qui s'est passé, et pas ce qui
    /// ne s'est pas passé : ni deadline, ni steps.
    #[test]
    fn mika2368_the_callback_body_names_its_own_cause() {
        let body = build_verdict_body(&VerdictReason::CallbackConcludedWithoutVerdict, "tr-2368");
        assert!(body.starts_with(DEADLINE_VERDICT_LINE));
        assert!(body.contains("sans poster"));
        assert!(body.contains("run_gh pr review"));
        assert!(body.contains("tr-2368"));
        assert!(body.contains(CALLBACK_VERDICT_EVENT));
        assert!(body.contains("mika#2368"));
        assert!(
            !body.contains("enveloppe de temps"),
            "le motif callback ne doit pas parler de deadline — c'est le mensonge \
             que C1 existe pour retirer"
        );
        assert!(
            !body.contains("step(s) d'outil"),
            "ce motif ne porte pas de steps"
        );
    }

    /// **AC5b** — le filet ne peut produire ni `pass` ni `block[*]`, sur
    /// **tous** ses chemins et pour les **deux** motifs.
    ///
    /// Asserté sur la constante *et* en passant le corps produit au parseur de
    /// `server::verdict` : la constante seule ne prouve pas ce que la machine
    /// d'état lira, et c'est elle qui merge sur `pass`.
    #[test]
    fn mika2368_the_net_can_never_produce_pass_or_block() {
        assert_eq!(DEADLINE_VERDICT_LINE, "VERDICT: hold[review]");

        for reason in [
            cut_off(5),
            cut_off(0),
            VerdictReason::CallbackConcludedWithoutVerdict,
        ] {
            let body = build_verdict_body(&reason, "trace");
            for forbidden in [
                "VERDICT: pass",
                "block[ac]",
                "block[ci]",
                "block[security]",
                "block[pipeline]",
            ] {
                assert!(
                    !body.contains(forbidden),
                    "le corps du filet ne doit jamais contenir {forbidden} — \
                     `verdict_handler` route `pass` vers un MERGE, et le filet \
                     mergerait une PR dont aucun diff n'a été revu"
                );
            }
            // Ce que la machine d'état lira réellement.
            let parsed = crate::server::verdict::parse_verdict(&body);
            assert!(
                matches!(&parsed, crate::server::verdict::Verdict::Hold(kind) if kind == "review"),
                "le parseur de verdict_handler doit lire hold[review] et rien \
                 d'autre, got {parsed:?}"
            );
        }
    }

    /// **C2** — le stamp fait un aller-retour, et toute forme illisible rend
    /// `None` plutôt qu'une cible devinée.
    #[test]
    fn mika2368_the_target_stamp_round_trips_and_fails_safe() {
        let target = requested_target();
        let raw = target.to_metadata_value();
        assert_eq!(raw, "senara-solutions/mika#2275");
        assert_eq!(PrTarget::from_metadata_value(&raw), Some(target));

        for unreadable in [
            "",
            "senara-solutions/mika",
            "senara-solutions/mika#",
            "senara-solutions/mika#abc",
            "#2275",
            "senara-solutions/mika#-1",
        ] {
            assert!(
                PrTarget::from_metadata_value(unreadable).is_none(),
                "{unreadable:?} doit être illisible, jamais deviné"
            );
        }
    }

    /// **AC5** — le contrat central du motif neuf : exactement un POST, le bon
    /// corps, la bonne cible.
    #[tokio::test]
    async fn mika2368_a_callback_concluded_without_verdict_posts_exactly_one_hold() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let captured = Arc::new(std::sync::Mutex::new(None::<PostReviewRequest>));
        let calls = Arc::new(AtomicUsize::new(0));

        let sink = captured.clone();
        let counter = calls.clone();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                reason: VerdictReason::CallbackConcludedWithoutVerdict,
                target: requested_target(),
                session_id: "callback-session",
                trace_id: "trace-2368",
                agent_id: "mika-qa",
                pr_reviews_posted: Some(&registry),
            },
            move |req| {
                counter.fetch_add(1, Ordering::SeqCst);
                *sink.lock().unwrap() = Some(req);
                async { Ok("ok".to_string()) }
            },
        )
        .await;

        assert_eq!(outcome, DeadlineVerdictOutcome::Posted);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "exactement un POST");
        let req = captured.lock().unwrap().clone().expect("un POST capturé");
        assert_eq!(req.repo, "senara-solutions/mika");
        assert_eq!(req.pr_number, 2275);
        assert!(req.body.starts_with(DEADLINE_VERDICT_LINE));
    }

    /// **AC7** — un tour qui a posté son propre verdict n'en reçoit pas un
    /// second, sur le motif neuf comme sur l'ancien, et pour les deux formes
    /// de clé que `run_gh` écrit.
    #[tokio::test]
    async fn mika2368_a_callback_that_already_reviewed_gets_no_second_verdict() {
        for key in ["senara-solutions/mika|2275", "__default__|2275"] {
            let registry: DashMap<String, HashSet<String>> = DashMap::new();
            registry
                .entry("callback-session".to_string())
                .or_default()
                .insert(key.to_string());
            let calls = Arc::new(AtomicUsize::new(0));

            let counter = calls.clone();
            let outcome = maybe_post_deadline_verdict(
                DeadlineVerdictInput {
                    reason: VerdictReason::CallbackConcludedWithoutVerdict,
                    target: requested_target(),
                    session_id: "callback-session",
                    trace_id: "trace-2368",
                    agent_id: "mika-qa",
                    pr_reviews_posted: Some(&registry),
                },
                move |_req| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async { Ok(String::new()) }
                },
            )
            .await;

            assert_eq!(
                outcome,
                DeadlineVerdictOutcome::AlreadyReviewed,
                "clé {key}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0, "AC7 : zéro POST ({key})");
        }
    }

    /// **AC6** — sans registre, le filet s'abstient et le dit, quel que soit le
    /// motif. Terme d'abstention hérité de mika#2276 et conservé tel quel.
    #[tokio::test]
    async fn mika2368_no_registry_abstains_on_both_reasons() {
        for reason in [cut_off(5), VerdictReason::CallbackConcludedWithoutVerdict] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let outcome = maybe_post_deadline_verdict(
                DeadlineVerdictInput {
                    reason,
                    target: requested_target(),
                    session_id: "s",
                    trace_id: "trace",
                    agent_id: "mika-qa",
                    pr_reviews_posted: None,
                },
                move |_req| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async { Ok(String::new()) }
                },
            )
            .await;
            assert_eq!(
                outcome,
                DeadlineVerdictOutcome::NotApplicable("no_registry")
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }

    /// **C5** — les deux noms d'événement sont distincts. Les fusionner
    /// casserait la sonde de contrôle négatif de mika#2355 sans casser aucune
    /// autre assertion.
    #[test]
    fn mika2368_the_two_event_names_are_distinct() {
        assert_ne!(DEADLINE_VERDICT_EVENT, CALLBACK_VERDICT_EVENT);
        assert_eq!(cut_off(1).event_name(), "qa_deadline_verdict");
        assert_eq!(
            VerdictReason::CallbackConcludedWithoutVerdict.event_name(),
            "qa_callback_verdict"
        );
    }

    /// **T8 / C5 — SOLE WRITER.** Chacun des deux noms est écrit littéralement
    /// à un seul endroit de la production : ici, dans les constantes.
    ///
    /// Un test **de source**, parce qu'un test comportemental ne peut pas voir
    /// cette classe : un second écrivain ne rendrait aucune décision fausse, il
    /// rendrait les deux populations inséparables. Toutes les assertions
    /// resteraient vertes pendant que les sondes cesseraient de discriminer.
    #[test]
    fn mika2368_each_event_name_has_exactly_one_writer_in_production() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sites: Vec<(String, String)> = Vec::new();

        fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    // Le périmètre est la **production** : un test qui assert
                    // sur la valeur d'une constante n'est pas un écrivain, et
                    // l'inclure ferait rougir le garde pour la raison inverse
                    // de celle qu'il surveille.
                    let production = match text.find("\n#[cfg(test)]") {
                        Some(at) => &text[..at],
                        None => &text[..],
                    };
                    let rel = path.to_string_lossy().to_string();
                    for line in production.lines() {
                        let trimmed = line.trim_start();
                        // Les commentaires et la prose de doc citent les noms
                        // abondamment — c'est du texte, pas un écrivain.
                        if trimmed.starts_with("//") {
                            continue;
                        }
                        for name in [DEADLINE_VERDICT_EVENT, CALLBACK_VERDICT_EVENT] {
                            if line.contains(&format!("\"{name}\"")) {
                                out.push((name.to_string(), rel.clone()));
                            }
                        }
                    }
                }
            }
        }

        walk(&root, &mut sites);

        for name in [DEADLINE_VERDICT_EVENT, CALLBACK_VERDICT_EVENT] {
            let writers: Vec<&String> = sites
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, p)| p)
                .collect();
            assert_eq!(
                writers.len(),
                1,
                "{name} doit avoir exactement un écrivain littéral (la constante \
                 de ce module) ; trouvé : {writers:?}"
            );
            assert!(
                writers[0].ends_with("deadline_verdict.rs"),
                "{name} doit être écrit dans ce module, pas dans {:?}",
                writers[0]
            );
        }
    }
}
