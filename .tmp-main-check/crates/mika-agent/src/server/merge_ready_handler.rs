//! L'acteur du merge : le dispatcher, et personne d'autre (mika#2248).
//!
//! [`super::ci_success_handler`] évalue et émet un signal ; ce handler le
//! consomme et merge. La séparation existe parce que le même handler tourne dans
//! deux agents — `check_suite.completed(success)` est routé vers `mika-dev`
//! (primaire) et diffusé vers `mika-qa` (secondaire, mika#1711) — et un
//! `gh pr merge` posé côté évaluation tourne sous le token de celui qui gagne la
//! course. Mesuré le 2026-09-08 sur mika#2244 : `mergedBy = mika-platform-qa`,
//! le relecteur fermant sa propre approbation.
//!
//! ## Ce qui autorise un merge ici
//!
//! Quatre conditions, toutes nécessaires, dans cet ordre :
//!
//! 1. **un signal relisible** — [`mika_common::forge_identity::parse_merge_ready_signal`]
//!    sur le texte du tour. Le signal ne peut venir que de l'évaluateur, qui l'a
//!    écrit après avoir franchi verdict, CI agrégée, périmètre et behind-main.
//!    Pas de signal → ce handler est transparent.
//! 2. **l'agent courant n'est pas le relecteur** — contrôle d'égalité de login
//!    contre le relecteur nommé DANS le signal. C'est l'énoncé d'AC1 posé à
//!    l'exécution, pas seulement dans une table de routage.
//! 3. **l'agent courant est le dispatcher** — liste blanche
//!    ([`mika_common::forge_identity::merge_disposition`]). Un agent inconnu
//!    tient, il ne merge pas.
//! 4. **le périmètre est mécanique** — revérifié ici, fail-closed, et non
//!    délégué au signal. Le signal dit « les portes étaient vertes » ; l'acteur
//!    reste responsable de ce qu'il écrit sur la forge. C'est aussi ce qui fait
//!    qu'un signal qui fuiterait par un autre chemin ne peut pas fermer une PR
//!    décision-core (AC4).
//!
//! ## Pourquoi pas un label `merge-ready` sur la forge
//!
//! Deux raisons mesurées. D'abord la permission : une écriture de label sous le
//! PAT résolu échoue (`Resource not accessible by personal access token
//! (addLabelsToLabelable)`, 29 refus mesurés le 2026-09-07, mika#2228) — le
//! signal aurait eu besoin d'un second token, et donc d'un second mode de
//! panne, sur le chemin critique du merge. Ensuite l'autorité : un label est
//! écrivable par un humain, et ferait d'un geste d'interface une autorisation de
//! merge capable de contourner la porte décision-core. Le signal reste dans la
//! trace du tour, et l'autorité reste dans le code.

use std::sync::Arc;

use mika_common::forge_identity::{
    self, MergeDisposition, MergeReadySignal, merge_disposition, parse_merge_ready_signal,
    would_merge_as_reviewer,
};
use tracing::{info, warn};

use crate::async_db::AsyncDatabase;
use crate::messaging::MessageSender;
use crate::perimeter::{self, Classification};
use crate::tools::pr_merge_with_gate::run_gh_merge;

use super::ci_success_handler::update_verdict_merge_metadata;
use super::verdict_handler::VerdictAction;

/// Pourquoi l'agent courant n'est pas l'acteur du merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldReason {
    /// L'agent courant merge sous le login qui a posé la revue.
    WouldMergeAsReviewer,
    /// L'agent courant n'est pas le dispatcher.
    NotDispatcher,
}

impl HoldReason {
    fn audit_action(self) -> &'static str {
        match self {
            Self::WouldMergeAsReviewer => "merge_ready_hold_reviewer_is_not_merge_actor",
            Self::NotDispatcher => "merge_ready_hold_not_dispatcher",
        }
    }
}

/// L'agent courant peut-il merger ce signal ? `Ok(())` ou la raison du refus.
///
/// Pure, pour que les deux refus soient testables sans forge ni base : ce sont
/// les deux assertions d'AC1 et elles doivent rester vraies par construction.
fn authorize_merge(agent_id: &str, reviewer_login: &str) -> Result<(), HoldReason> {
    if would_merge_as_reviewer(agent_id, reviewer_login) {
        return Err(HoldReason::WouldMergeAsReviewer);
    }
    if merge_disposition(agent_id) != MergeDisposition::Act {
        return Err(HoldReason::NotDispatcher);
    }
    Ok(())
}

/// Consomme un signal merge-ready et merge, sous l'identité de l'agent courant.
///
/// `github_token` est le token résolu pour l'agent courant (PAT > App) : c'est
/// lui qui décide de `mergedBy`, et c'est pour cela que l'autorisation
/// ci-dessus porte sur l'agent, pas sur le contenu du signal.
///
/// Retourne `Passthrough` quand il n'y a pas de signal, ou quand l'agent courant
/// n'est pas l'acteur — le pré-digest de l'évaluateur dit déjà au LLM de ne pas
/// merger à la main, et le redoubler n'ajouterait qu'un second texte à suivre.
pub async fn try_handle_merge_ready(
    text: &str,
    db: &AsyncDatabase,
    github_token: Option<&str>,
    message_sender: Option<&Arc<dyn MessageSender>>,
    session_id: &str,
    trace_id: &str,
) -> VerdictAction {
    let signal = match parse_merge_ready_signal(text) {
        Some(s) => s,
        None => return VerdictAction::Passthrough { enrichment: None },
    };

    let agent_id = db.agent_id().to_string();
    let target_key = format!(
        "pr:{}#{}@{}",
        signal.repo, signal.pr_number, signal.head_sha
    );

    if let Err(reason) = authorize_merge(&agent_id, &signal.reviewer_login) {
        info!(
            event = reason.audit_action(),
            pr_number = signal.pr_number,
            repo = %signal.repo,
            agent_id = %agent_id,
            reviewer = %signal.reviewer_login,
            dispatcher = forge_identity::DISPATCHER_AGENT,
            "Merge-ready signal seen but this agent is not the merge actor — holding (mika#2248)"
        );
        if let Err(e) = db
            .log_audit_event(
                session_id,
                reason.audit_action(),
                &target_key,
                Some("merge_ready_signaled"),
                Some("held_for_dispatcher"),
                Some(&format!(
                    "agent_id={agent_id} reviewer={} dispatcher={}",
                    signal.reviewer_login,
                    forge_identity::DISPATCHER_AGENT,
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, action = reason.audit_action(), "Failed to log merge-ready hold audit event");
        }
        return VerdictAction::Passthrough { enrichment: None };
    }

    let token = match github_token {
        Some(t) => t,
        None => {
            warn!(
                pr_number = signal.pr_number,
                repo = %signal.repo,
                "Merge-ready signal but no GitHub token for the dispatcher — cannot merge"
            );
            return VerdictAction::Handled {
                pre_digest: format_error_pre_digest(
                    &signal,
                    "no GitHub token configured for this agent",
                ),
            };
        }
    };

    // Porte périmètre, revérifiée par l'acteur et fail-closed (AC4). L'évaluateur
    // l'a déjà consultée ; la redire ici n'est pas une redondance mais la règle
    // « celui qui écrit sur la forge répond de ce qu'il écrit » — et la seule
    // défense si un signal arrivait par un chemin que l'évaluateur n'a pas posé.
    //
    // Le fetch est borné : un `gh` pendu ici tiendrait le tour ouvert, et la
    // porte qui protège le mieux est celle qui rend une réponse. Le dépassement
    // de délai est un échec comme un autre — donc décision-core.
    let fetch_future = perimeter::fetch::fetch_pr_files(signal.pr_number, &signal.repo, token);
    let fetch_outcome =
        match tokio::time::timeout(std::time::Duration::from_secs(60), fetch_future).await {
            Ok(inner) => inner.map_err(|e| e.to_string()),
            Err(_) => Err("perimeter file fetch timed out after 60s".to_string()),
        };
    let perimeter_verdict = match fetch_outcome {
        Ok(files) => perimeter::classify_pr_files(&files),
        Err(e) => {
            warn!(
                error = %e,
                pr_number = signal.pr_number,
                repo = %signal.repo,
                "Merge actor: perimeter fetch failed — fail-closed to DECISION-CORE (mika#2248)"
            );
            perimeter::PrClassification {
                verdict: Classification::DecisionCore,
                mechanical_files: Vec::new(),
                decision_core_files: vec![format!("<fetch-error: {e}>")],
            }
        }
    };

    if perimeter_verdict.verdict == Classification::DecisionCore {
        info!(
            event = "merge_ready_human_gate_required",
            pr_number = signal.pr_number,
            repo = %signal.repo,
            summary = %perimeter_verdict.summary(),
            decision_core_files = ?perimeter_verdict.decision_core_files,
            "Forge-gate at the merge actor: DECISION-CORE zone(s) — operator must merge manually"
        );
        if let Err(e) = db
            .log_audit_event(
                session_id,
                "merge_ready_human_gate_required",
                &target_key,
                Some("merge_ready_signaled"),
                Some("held_for_operator"),
                Some(&format!(
                    "agent_id={agent_id} gate=forge-gate decision_core_files={}",
                    perimeter_verdict.decision_core_files.join(","),
                )),
                Some(trace_id),
            )
            .await
        {
            warn!(error = %e, "Failed to log merge_ready_human_gate_required audit event");
        }
        notify(
            message_sender,
            &format!(
                "PR #{} on {} — merge-ready signal held at the merge actor: the PR touches \
                 DECISION-CORE zone(s). {}. Operator must merge manually.",
                signal.pr_number,
                signal.repo,
                perimeter_verdict.summary(),
            ),
        )
        .await;
        return VerdictAction::Handled {
            pre_digest: format_decision_core_hold_pre_digest(&signal, &perimeter_verdict.summary()),
        };
    }

    let pr_url = format!(
        "https://github.com/{}/pull/{}",
        signal.repo, signal.pr_number
    );

    let merge_future = run_gh_merge(
        signal.pr_number,
        &signal.repo,
        "squash",
        true,  // delete_branch
        false, // not auto — the evaluator already aggregated every required check
        token,
    );
    let merge_result =
        match tokio::time::timeout(std::time::Duration::from_secs(60), merge_future).await {
            Ok(inner) => inner,
            Err(_) => {
                warn!(
                    pr_number = signal.pr_number,
                    "gh pr merge timed out after 60s at the merge actor"
                );
                Err("gh pr merge timed out after 60s".to_string())
            }
        };

    match merge_result {
        Ok(_output) => {
            if let Ok(Some(task)) = db.find_active_task_by_pr_url(&pr_url).await
                && let Err(e) = update_verdict_merge_metadata(
                    db,
                    &task.id,
                    &task.metadata,
                    signal.pr_number,
                    &pr_url,
                    "merge_initiated",
                )
                .await
            {
                warn!(error = %e, task_id = %task.id, "Failed to update task metadata after merge");
            }

            if let Err(e) = db
                .log_audit_event(
                    session_id,
                    "ci_success_merge",
                    &format!("task:{}", signal.task_id),
                    Some("merge_ready_signaled"),
                    Some("merge_initiated"),
                    Some(&format!(
                        "trigger=merge_ready_signal actor={agent_id} reviewer={} pr_url={pr_url} task_id={}",
                        signal.reviewer_login, signal.task_id,
                    )),
                    Some(trace_id),
                )
                .await
            {
                warn!(error = %e, "Failed to log ci_success_merge audit event");
            }

            notify(
                message_sender,
                &format!(
                    "PR #{} on {} — CI checks all green + VERDICT: pass from @{}. \
                     Squash-merge initiated by the dispatcher `{agent_id}` (branch deletion requested).",
                    signal.pr_number, signal.repo, signal.reviewer_login,
                ),
            )
            .await;

            info!(
                event = "merge_ready_merge_initiated",
                pr_number = signal.pr_number,
                repo = %signal.repo,
                agent_id = %agent_id,
                reviewer = %signal.reviewer_login,
                task_id = %signal.task_id,
                "Merge actor: squash-merge initiated under the dispatcher's identity (mika#2248)"
            );

            VerdictAction::Handled {
                pre_digest: format_success_pre_digest(&signal, &agent_id),
            }
        }
        Err(e) => {
            let lower = e.to_lowercase();
            if lower.contains("already merged")
                || lower.contains("already been merged")
                || lower.contains("pull request is closed")
            {
                info!(
                    pr_number = signal.pr_number,
                    "PR already finalized before the merge actor ran — acknowledging"
                );
                return VerdictAction::Handled {
                    pre_digest: format_already_merged_pre_digest(&signal),
                };
            }
            warn!(
                error = %e,
                pr_number = signal.pr_number,
                agent_id = %agent_id,
                "Merge actor: structural merge failed"
            );
            VerdictAction::Handled {
                pre_digest: format_error_pre_digest(&signal, &e),
            }
        }
    }
}

async fn notify(sender: Option<&Arc<dyn MessageSender>>, message: &str) {
    let Some(sender) = sender else { return };
    match sender.send(message).await {
        Ok(crate::messaging::SendOutcome::Delivered) => {}
        Ok(crate::messaging::SendOutcome::Failed { reason }) => {
            warn!(reason = %reason, "Merge-ready notification delivery failed");
        }
        Ok(crate::messaging::SendOutcome::NoChannel) => {
            warn!("Merge-ready notification skipped — no reply channel (chat_id=0)");
        }
        Err(e) => {
            warn!(error = %e, "Failed to send merge-ready notification");
        }
    }
}

// ---------------------------------------------------------------------------
// Pre-digest message formatting
// ---------------------------------------------------------------------------
//
// IMPORTANT: every pre-digest below avoids the completion-claim guard's
// vocabulary (merged, deployed, complete/completed, shipped) — pinned by the
// tests at the bottom of this file.

/// Le merge a été lancé par le dispatcher.
fn format_success_pre_digest(signal: &MergeReadySignal, agent_id: &str) -> String {
    format!(
        "<merge_ready_handler>\n\
         Merge-ready signal consumed for {}#{} (head {}).\n\n\
         Squash-merge has been initiated under this agent's identity (`{agent_id}`), not the \
         reviewer's (@{}) — branch deletion requested.\n\n\
         Task: {}\n\n\
         Do NOT call pr_merge_with_gate — the merge action is already in progress.\n\
         Update the task status to reflect the outcome, then notify the user.\n\
         </merge_ready_handler>",
        signal.repo, signal.pr_number, signal.head_sha, signal.reviewer_login, signal.task_id,
    )
}

/// La PR était déjà finalisée avant que l'acteur n'agisse.
fn format_already_merged_pre_digest(signal: &MergeReadySignal) -> String {
    format!(
        "<merge_ready_handler>\n\
         Merge-ready signal for {}#{} — the PR was already finalized before this agent acted.\n\n\
         Task: {}\n\n\
         Do NOT call pr_merge_with_gate — no action needed.\n\
         Update the task status if not already done, then notify the user.\n\
         </merge_ready_handler>",
        signal.repo, signal.pr_number, signal.task_id,
    )
}

/// Le périmètre est décision-core : la PR reste à l'opérateur (AC4).
fn format_decision_core_hold_pre_digest(
    signal: &MergeReadySignal,
    perimeter_summary: &str,
) -> String {
    format!(
        "<merge_ready_handler>\n\
         Merge-ready signal for {}#{}, but the PR touches DECISION-CORE zone(s).\n\n\
         Auto-merge BLOCKED by forge-gate at the merge actor. {perimeter_summary}. \
         Operator must merge manually.\n\n\
         Do NOT call pr_merge_with_gate for this PR — it blocks on the same gate. \
         Notify the operator that a manual review-and-merge is required.\n\
         </merge_ready_handler>",
        signal.repo, signal.pr_number,
    )
}

/// Le merge a échoué, ou le token manquait.
fn format_error_pre_digest(signal: &MergeReadySignal, error: &str) -> String {
    format!(
        "<merge_ready_handler>\n\
         Merge-ready signal for {}#{} — this agent is the merge actor but the merge did not go \
         through.\n\n\
         Error: {error}\n\n\
         Investigate the error and decide the next action. Do NOT assume the PR closed.\n\
         </merge_ready_handler>",
        signal.repo, signal.pr_number,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    static COMPLETION_CLAIM_RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?i)\b(merged|deployed|completed?|shipped)\b")
                .expect("completion claim regex")
        });

    /// Base en mémoire portée par `agent_id`, avec la ligne d'agent que la
    /// contrainte de clé étrangère de `sessions` exige.
    fn db_for_agent(agent_id: &str, session_id: &str) -> AsyncDatabase {
        let db = crate::db::Database::open_in_memory().expect("in-memory db");
        db.register_agent(agent_id, agent_id, "")
            .expect("register agent");
        db.create_session(session_id, agent_id, "github")
            .expect("create session");
        AsyncDatabase::new_with_agent(db, agent_id)
    }

    fn sample_signal() -> MergeReadySignal {
        MergeReadySignal {
            repo: "senara-solutions/mika".to_string(),
            pr_number: 2244,
            head_sha: "abc123def456".to_string(),
            branch: "fix/2244/x".to_string(),
            task_id: "task-9".to_string(),
            reviewer_login: forge_identity::REVIEWER_FORGE_LOGIN.to_string(),
        }
    }

    // ---- AC1/AC2: le relecteur n'est jamais l'acteur ----

    #[test]
    fn mika2248_le_relecteur_ne_peut_pas_merger_sa_propre_approbation() {
        // La forme exacte mesurée sur mika#2244 : l'agent qui tourne EST le
        // relecteur nommé dans le signal.
        assert_eq!(
            authorize_merge(
                forge_identity::REVIEWER_AGENT,
                forge_identity::REVIEWER_FORGE_LOGIN
            ),
            Err(HoldReason::WouldMergeAsReviewer),
        );
    }

    #[test]
    fn mika2248_le_dispatcher_est_lacteur() {
        // Le contrôle négatif dans le même appel : sans lui, un `authorize_merge`
        // qui refuserait tout passerait le test ci-dessus.
        assert_eq!(
            authorize_merge(
                forge_identity::DISPATCHER_AGENT,
                forge_identity::REVIEWER_FORGE_LOGIN
            ),
            Ok(()),
        );
    }

    #[test]
    fn mika2248_un_agent_tiers_tient_meme_sans_collision_de_login() {
        // `mika` n'est pas le relecteur — la première ceinture ne le retient pas.
        // C'est la liste blanche qui le retient, et c'est pour cela qu'elle est
        // une liste blanche : un nouvel agent n'hérite pas du droit de merger.
        assert_eq!(
            authorize_merge("mika", forge_identity::REVIEWER_FORGE_LOGIN),
            Err(HoldReason::NotDispatcher),
        );
        assert_eq!(
            authorize_merge("mika-arch", "samidarko"),
            Err(HoldReason::NotDispatcher),
        );
    }

    #[tokio::test]
    async fn mika2248_le_relecteur_natteint_jamais_le_merge() {
        // Bout en bout sur la moitié observable : un signal valide, l'agent
        // relecteur, et un token présent. Le handler doit rendre Passthrough
        // AVANT tout appel `gh` — donc sans réseau, et sans jamais pouvoir
        // écrire `mergedBy`.
        let db = db_for_agent(forge_identity::REVIEWER_AGENT, "s-2248");

        let text = format!(
            "<ci_success_handler>\n{}\n</ci_success_handler>",
            sample_signal()
        );
        let action =
            try_handle_merge_ready(&text, &db, Some("fake-token"), None, "s-2248", "trace-2248")
                .await;

        assert!(
            matches!(action, VerdictAction::Passthrough { enrichment: None }),
            "le relecteur doit tenir sans agir, got {action:?}"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name("ci_success_merge")
                .await
                .expect("count"),
            0,
            "aucune ligne de merge ne doit exister sous l'identité du relecteur"
        );
        assert_eq!(
            db.count_audit_events_by_tool_name(HoldReason::WouldMergeAsReviewer.audit_action())
                .await
                .expect("count"),
            1,
            "le refus doit être greppable dans l'audit — sinon AC1 n'est mesurable que sur la forge"
        );
    }

    #[tokio::test]
    async fn sans_signal_le_handler_est_transparent() {
        let db = db_for_agent(forge_identity::DISPATCHER_AGENT, "s-none");
        let action = try_handle_merge_ready(
            "[GitHub] Check suite success on senara-solutions/mika (branch: main)",
            &db,
            Some("fake-token"),
            None,
            "s-none",
            "trace-none",
        )
        .await;
        assert!(matches!(
            action,
            VerdictAction::Passthrough { enrichment: None }
        ));
    }

    // ---- Pré-digests ----

    #[test]
    fn les_pre_digests_evitent_le_vocabulaire_du_garde_de_completion() {
        let signal = sample_signal();
        for text in [
            format_success_pre_digest(&signal, forge_identity::DISPATCHER_AGENT),
            format_already_merged_pre_digest(&signal),
            format_decision_core_hold_pre_digest(&signal, "DECISION-CORE (1 file: foo.rs)"),
            format_error_pre_digest(&signal, "gh exit code 1"),
        ] {
            assert!(
                !COMPLETION_CLAIM_RE.is_match(&text),
                "pré-digest contenant un mot déclencheur du garde de completion : {text}"
            );
        }
    }

    #[test]
    fn le_pre_digest_de_succes_distingue_lacteur_du_relecteur() {
        let signal = sample_signal();
        let text = format_success_pre_digest(&signal, forge_identity::DISPATCHER_AGENT);
        assert!(text.contains(forge_identity::DISPATCHER_AGENT));
        assert!(text.contains(forge_identity::REVIEWER_FORGE_LOGIN));
        assert!(text.contains("Do NOT call pr_merge_with_gate"));
    }

    #[test]
    fn le_hold_decision_core_route_vers_loperateur() {
        let text =
            format_decision_core_hold_pre_digest(&sample_signal(), "DECISION-CORE (1 file: a.rs)");
        assert!(text.contains("Operator must merge manually"));
        assert!(text.contains("BLOCKED by forge-gate"));
    }
}
