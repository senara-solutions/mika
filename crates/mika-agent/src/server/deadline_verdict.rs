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

use std::collections::HashSet;
use std::future::Future;

use dashmap::DashMap;
use tracing::{info, warn};

use super::verdict::parse_pr_review_event;
use super::webhook_queue_v2::parse_pr_action_event;

/// Ligne canonique du verdict de secours. Forme tranchée en Q1 : surface
/// existante, `verdict_handler` la comprend déjà, CODEOWNERS épargné.
pub const DEADLINE_VERDICT_LINE: &str = "VERDICT: hold[review]";

/// Nom d'événement pour le grep opérateur. Écrit à un seul site.
pub const DEADLINE_VERDICT_EVENT: &str = "qa_deadline_verdict";

/// La PR qu'un tour était en train de traiter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrTarget {
    pub repo: String,
    pub pr_number: u64,
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
    /// `steps_completed` du tour, `None` si le tour n'a pas dépassé sa deadline.
    /// C'est [`crate::agent_loop::AgentOutput::deadline_exceeded`] transposé.
    pub overrun: Option<crate::agent_loop::DeadlineOverrun>,
    /// Le texte de l'événement d'origine, tel que le gateway l'a formaté.
    pub event_text: &'a str,
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

/// Le corps du verdict de secours (AC1).
///
/// Porte le motif, le nombre de steps accomplis et le `trace_id` — un tour coupé
/// au step 5 et un tour coupé au step 19 appellent des réponses opérateur
/// différentes, et le texte de fallback ne dit ni l'un ni l'autre.
fn build_verdict_body(overrun: crate::agent_loop::DeadlineOverrun, trace_id: &str) -> String {
    format!(
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
    )
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

/// Poste le verdict de secours si — et seulement si — le tour a été coupé, qu'il
/// traitait une PR, et qu'aucune review n'a déjà été postée pour elle.
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
    let Some(overrun) = input.overrun else {
        return DeadlineVerdictOutcome::NotApplicable("turn_completed");
    };
    let Some(target) = parse_pr_target(input.event_text) else {
        // Un tour Telegram, un heartbeat ou un événement non-PR peut dépasser
        // sa deadline : il n'y a alors rien sur quoi poster, et ce n'est pas un
        // défaut.
        return DeadlineVerdictOutcome::NotApplicable("not_a_pr_event");
    };
    let Some(registry) = input.pr_reviews_posted else {
        // Sans le registre, AC3 est indécidable. Le double-post est une
        // régression pire que le silence (table de disposition, ligne 4) : on
        // s'abstient, et on le dit.
        warn!(
            event = DEADLINE_VERDICT_EVENT,
            agent_id = %input.agent_id,
            trace_id = %input.trace_id,
            repo = %target.repo,
            pr = target.pr_number,
            outcome = "no_registry",
            "tour coupé par sa deadline sur une PR, mais le registre anti-double-post \
             est absent — verdict de secours non posté"
        );
        return DeadlineVerdictOutcome::NotApplicable("no_registry");
    };

    if session_has_review_for(registry, input.session_id, &target) {
        info!(
            event = DEADLINE_VERDICT_EVENT,
            agent_id = %input.agent_id,
            trace_id = %input.trace_id,
            repo = %target.repo,
            pr = target.pr_number,
            outcome = "already_reviewed",
            "tour coupé par sa deadline mais la review était déjà postée — \
             pas de second verdict"
        );
        return DeadlineVerdictOutcome::AlreadyReviewed;
    }

    let request = PostReviewRequest {
        repo: target.repo.clone(),
        pr_number: target.pr_number,
        body: build_verdict_body(overrun, input.trace_id),
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
            warn!(
                event = DEADLINE_VERDICT_EVENT,
                agent_id = %input.agent_id,
                trace_id = %input.trace_id,
                repo = %target.repo,
                pr = target.pr_number,
                steps_completed = overrun.steps_completed,
                outcome = "posted",
                "tour de revue coupé par sa deadline — verdict hold[review] posté \
                 par le moteur"
            );
            DeadlineVerdictOutcome::Posted
        }
        Err(e) if is_idempotent_already_posted(&e) => {
            info!(
                event = DEADLINE_VERDICT_EVENT,
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
                event = DEADLINE_VERDICT_EVENT,
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
                overrun: overrun(5),
                event_text: REVIEW_REQUESTED,
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
                overrun: overrun(12),
                event_text: REVIEW_REQUESTED,
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
                overrun: overrun(3),
                event_text: REVIEW_REQUESTED,
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
                overrun: overrun(5),
                event_text: REVIEW_REQUESTED,
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
                overrun: overrun(5),
                event_text: REVIEW_REQUESTED,
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
    #[tokio::test]
    async fn a_completed_turn_never_posts() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();

        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                overrun: None,
                event_text: REVIEW_REQUESTED,
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

        assert_eq!(
            outcome,
            DeadlineVerdictOutcome::NotApplicable("turn_completed")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// Un tour non-PR qui dépasse sa deadline (Telegram, heartbeat) n'a rien sur
    /// quoi poster — et ce n'est pas un défaut.
    #[tokio::test]
    async fn a_non_pr_deadline_overrun_posts_nothing() {
        let registry: DashMap<String, HashSet<String>> = DashMap::new();
        let outcome = maybe_post_deadline_verdict(
            DeadlineVerdictInput {
                overrun: overrun(9),
                event_text: "Salut, tu peux me résumer ma semaine ?",
                session_id: "telegram-session",
                trace_id: "trace",
                agent_id: "mika",
                pr_reviews_posted: Some(&registry),
            },
            |_req| async { Ok(String::new()) },
        )
        .await;

        assert_eq!(
            outcome,
            DeadlineVerdictOutcome::NotApplicable("not_a_pr_event")
        );
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
                    overrun: overrun(5),
                    event_text: REVIEW_REQUESTED,
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
}
